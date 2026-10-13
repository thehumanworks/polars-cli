//! Parsing for command-line values: data types, scalar literals, and value lists.
//!
//! Scalars are parsed as JSON when they are valid JSON scalars (`42`, `1.5`, `true`, `null`,
//! `"quoted"`) and are otherwise taken as plain strings, so `pl fill-null unknown` and
//! `pl fill-null '"42"'` both behave predictably in Bash.

use anyhow::{Context, bail};
use polars::prelude::{
    AnyValue, Categories, DataType, Expr, Null, PlSmallStr, Series, StrptimeOptions, TimeUnit,
    TimeZone, lit,
};
use serde_json::Value as JsonValue;

/// Parse a data type name such as `i64`, `str`, `datetime<ms, UTC>`, or `list<f64>`.
///
/// The Polars display forms printed by `pl schema` (`datetime[μs]`, `list[i64]`) are accepted
/// as well, so a schema can be copied back into `--schema` or `cast`.
pub fn parse_dtype(input: &str) -> anyhow::Result<DataType> {
    let text = input.trim();
    let (head, args) = split_dtype_args(text)?;
    let head = head.trim().to_ascii_lowercase();
    let dtype = match (head.as_str(), args) {
        ("list", Some(inner)) => DataType::List(Box::new(parse_dtype(inner)?)),
        ("array", Some(args)) => {
            let (inner, width) = args
                .rsplit_once(',')
                .with_context(|| format!("array dtype expects array<TYPE, WIDTH>, got {text:?}"))?;
            let width = width.trim().parse::<usize>().with_context(|| {
                format!("array width must be a non-negative integer in {text:?}")
            })?;
            DataType::Array(Box::new(parse_dtype(inner)?), width)
        }
        ("datetime", args) => {
            let mut parts = args.map(split_args).unwrap_or_default().into_iter();
            let unit = parts
                .next()
                .map(|unit| parse_time_unit(&unit))
                .transpose()?
                .unwrap_or(TimeUnit::Microseconds);
            let zone = parts.next().filter(|zone| zone != "*" && !zone.is_empty());
            if parts.next().is_some() {
                bail!("datetime dtype expects datetime<UNIT[, TIMEZONE]>, got {text:?}");
            }
            DataType::Datetime(unit, parse_time_zone(zone.as_deref())?)
        }
        ("duration", args) => {
            let unit = args
                .map(parse_time_unit)
                .transpose()?
                .unwrap_or(TimeUnit::Microseconds);
            DataType::Duration(unit)
        }
        ("decimal", Some(args)) => {
            let parts = split_args(args);
            let [precision, scale] = parts.as_slice() else {
                bail!("decimal dtype expects decimal<PRECISION, SCALE>, got {text:?}");
            };
            let precision = precision
                .parse::<usize>()
                .with_context(|| format!("invalid decimal precision in {text:?}"))?;
            let scale = scale
                .parse::<usize>()
                .with_context(|| format!("invalid decimal scale in {text:?}"))?;
            if !(1..=38).contains(&precision) || scale > precision {
                bail!("decimal precision must be 1..=38 and scale must not exceed it in {text:?}");
            }
            DataType::Decimal(precision, scale)
        }
        ("decimal", None) => {
            bail!("decimal dtype requires a precision and scale, such as decimal<10, 2>")
        }
        (name, None) => match name {
            "i8" | "int8" => DataType::Int8,
            "i16" | "int16" => DataType::Int16,
            "i32" | "int32" => DataType::Int32,
            "i64" | "int64" | "int" | "integer" | "long" => DataType::Int64,
            "u8" | "uint8" => DataType::UInt8,
            "u16" | "uint16" => DataType::UInt16,
            "u32" | "uint32" => DataType::UInt32,
            "u64" | "uint64" => DataType::UInt64,
            "u128" | "uint128" => DataType::UInt128,
            "f32" | "float32" => DataType::Float32,
            "f64" | "float64" | "float" | "double" => DataType::Float64,
            "str" | "string" | "utf8" | "text" => DataType::String,
            "bool" | "boolean" => DataType::Boolean,
            "date" => DataType::Date,
            "time" => DataType::Time,
            "binary" => DataType::Binary,
            "null" => DataType::Null,
            "cat" | "categorical" => DataType::from_categories(Categories::global()),
            _ => bail!("unknown data type {text:?}"),
        },
        _ => bail!("unknown data type {text:?}"),
    };
    Ok(dtype)
}

/// Split on commas that are not nested inside `<…>` or `[…]` type arguments.
pub fn split_top_level_commas(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    for (index, character) in text.char_indices() {
        match character {
            '<' | '[' => depth += 1,
            '>' | ']' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(&text[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    parts.push(&text[start..]);
    parts
}

fn split_dtype_args(text: &str) -> anyhow::Result<(&str, Option<&str>)> {
    for (open, close) in [('<', '>'), ('[', ']')] {
        if let Some(start) = text.find(open) {
            if !text.ends_with(close) {
                bail!("unbalanced type arguments in {text:?}");
            }
            return Ok((&text[..start], Some(&text[start + 1..text.len() - 1])));
        }
    }
    Ok((text, None))
}

fn split_args(args: &str) -> Vec<String> {
    args.split(',')
        .map(|part| part.trim().to_owned())
        .filter(|part| !part.is_empty())
        .collect()
}

pub fn parse_time_unit(unit: &str) -> anyhow::Result<TimeUnit> {
    match unit.trim().to_ascii_lowercase().as_str() {
        "ns" => Ok(TimeUnit::Nanoseconds),
        "us" | "μs" | "µs" => Ok(TimeUnit::Microseconds),
        "ms" => Ok(TimeUnit::Milliseconds),
        other => bail!("unknown time unit {other:?}; expected ns, us, or ms"),
    }
}

/// Parse an optional time zone; `null` and empty strings mean "no time zone".
pub fn parse_time_zone(zone: Option<&str>) -> anyhow::Result<Option<TimeZone>> {
    let zone = zone
        .map(str::trim)
        .filter(|zone| !zone.is_empty() && !zone.eq_ignore_ascii_case("null"));
    TimeZone::opt_try_new(zone.map(PlSmallStr::from))
        .with_context(|| format!("invalid time zone {:?}", zone.unwrap_or_default()))
}

/// Cast an expression of type `from` to `to`.
///
/// Text becomes dates, datetimes, and times through Polars' format-inferring parsers, which
/// accept the common ISO-8601 spellings (`2024-01-05 10:00:00`, `2024-01-05T10:00:00Z`) that a
/// plain cast rejects. Everything else is an ordinary cast.
pub fn cast_expr(expr: Expr, from: &DataType, to: &DataType, strict: bool) -> Expr {
    let options = StrptimeOptions {
        format: None,
        strict,
        exact: true,
        cache: true,
    };
    match (from, to) {
        (DataType::String, DataType::Date) => expr.str().to_date(options),
        (DataType::String, DataType::Datetime(unit, zone)) => {
            expr.str()
                .to_datetime(Some(*unit), zone.clone(), options, lit("raise"))
        }
        (DataType::String, DataType::Time) => expr.str().to_time(options),
        _ if strict => expr.strict_cast(to.clone()),
        _ => expr.cast(to.clone()),
    }
}

/// Parse a scalar literal: JSON scalars keep their type, everything else is a string.
pub fn parse_scalar(input: &str) -> JsonValue {
    match serde_json::from_str::<JsonValue>(input.trim()) {
        Ok(value) if !value.is_array() && !value.is_object() => value,
        _ => JsonValue::String(input.to_owned()),
    }
}

/// Parse a list of scalars from a JSON array or a comma-separated string.
///
/// Use the JSON array form for values that contain commas: `'["a,b", "c"]'`.
pub fn parse_list(input: &str) -> anyhow::Result<Vec<JsonValue>> {
    let trimmed = input.trim();
    if trimmed.starts_with('[') {
        let value = serde_json::from_str::<JsonValue>(trimmed)
            .with_context(|| format!("expected a JSON array of scalars but got {input:?}"))?;
        let JsonValue::Array(items) = value else {
            bail!("expected a JSON array of scalars but got {input:?}");
        };
        if items.iter().any(|item| item.is_array() || item.is_object()) {
            bail!("list values must be scalars, got {input:?}");
        }
        return Ok(items);
    }
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    Ok(trimmed
        .split(',')
        .map(|item| parse_scalar(item.trim()))
        .collect())
}

/// Parse a list of numbers from a JSON array or a comma-separated string.
pub fn parse_f64_list(input: &str) -> anyhow::Result<Vec<f64>> {
    parse_list(input)?
        .into_iter()
        .map(|value| {
            value
                .as_f64()
                .with_context(|| format!("expected numbers but got {value} in {input:?}"))
        })
        .collect()
}

/// Parse a list of strings from a JSON array or a comma-separated string.
pub fn parse_string_list(input: &str) -> anyhow::Result<Vec<String>> {
    Ok(parse_list(input)?
        .into_iter()
        .map(|value| match value {
            JsonValue::String(text) => text,
            other => other.to_string(),
        })
        .collect())
}

/// Parse a list of row indices from a JSON array or a comma-separated string.
pub fn parse_index_list(input: &str) -> anyhow::Result<Vec<i64>> {
    parse_list(input)?
        .into_iter()
        .map(|value| {
            value
                .as_i64()
                .with_context(|| format!("expected integer indices but got {value} in {input:?}"))
        })
        .collect()
}

/// A literal expression for a parsed scalar.
pub fn scalar_expr(value: &JsonValue) -> anyhow::Result<Expr> {
    Ok(match value {
        JsonValue::Null => lit(Null {}),
        JsonValue::Bool(value) => lit(*value),
        JsonValue::Number(number) => {
            if let Some(value) = number.as_i64() {
                lit(value)
            } else if let Some(value) = number.as_u64() {
                lit(value)
            } else {
                lit(number.as_f64().context("number is not representable")?)
            }
        }
        JsonValue::String(value) => lit(value.as_str()),
        other => bail!("expected a scalar value but got {other}"),
    })
}

/// Build a series from parsed scalars, letting Polars pick a common supertype.
pub fn list_series(name: &str, values: &[JsonValue]) -> anyhow::Result<Series> {
    let values = values
        .iter()
        .map(json_to_any_value)
        .collect::<anyhow::Result<Vec<_>>>()?;
    Series::from_any_values(name.into(), &values, false).map_err(Into::into)
}

fn json_to_any_value(value: &JsonValue) -> anyhow::Result<AnyValue<'static>> {
    Ok(match value {
        JsonValue::Null => AnyValue::Null,
        JsonValue::Bool(value) => AnyValue::Boolean(*value),
        JsonValue::Number(number) => {
            if let Some(value) = number.as_i64() {
                AnyValue::Int64(value)
            } else if let Some(value) = number.as_u64() {
                AnyValue::UInt64(value)
            } else {
                AnyValue::Float64(number.as_f64().context("number is not representable")?)
            }
        }
        JsonValue::String(value) => AnyValue::StringOwned(value.as_str().into()),
        other => bail!("expected a scalar value but got {other}"),
    })
}

#[cfg(test)]
mod tests {
    use polars::prelude::{DataType, TimeUnit};
    use serde_json::json;

    use super::{parse_dtype, parse_list, parse_scalar};

    #[test]
    fn parses_scalar_and_parameterized_dtypes() {
        assert_eq!(parse_dtype("i64").unwrap(), DataType::Int64);
        assert_eq!(parse_dtype("STR").unwrap(), DataType::String);
        assert_eq!(
            parse_dtype("list<f64>").unwrap(),
            DataType::List(Box::new(DataType::Float64))
        );
        assert_eq!(
            parse_dtype("datetime[μs]").unwrap(),
            DataType::Datetime(TimeUnit::Microseconds, None)
        );
        assert!(matches!(
            parse_dtype("datetime<ms, UTC>").unwrap(),
            DataType::Datetime(TimeUnit::Milliseconds, Some(_))
        ));
        assert_eq!(
            parse_dtype("decimal<10, 2>").unwrap(),
            DataType::Decimal(10, 2)
        );
        assert!(parse_dtype("nope").is_err());
        assert!(parse_dtype("decimal").is_err());
    }

    #[test]
    fn splits_only_on_top_level_commas() {
        assert_eq!(
            super::split_top_level_commas("a:i64, b:datetime<ms, UTC>,c:array<i64, 3>"),
            ["a:i64", " b:datetime<ms, UTC>", "c:array<i64, 3>"]
        );
    }

    #[test]
    fn parses_scalars_as_json_or_plain_strings() {
        assert_eq!(parse_scalar("42"), json!(42));
        assert_eq!(parse_scalar("null"), json!(null));
        assert_eq!(parse_scalar("\"42\""), json!("42"));
        assert_eq!(parse_scalar("n/a"), json!("n/a"));
    }

    #[test]
    fn parses_comma_and_json_lists() {
        assert_eq!(
            parse_list("a, 2,true").unwrap(),
            [json!("a"), json!(2), json!(true)]
        );
        assert_eq!(
            parse_list(r#"["a,b", 1]"#).unwrap(),
            [json!("a,b"), json!(1)]
        );
        assert!(parse_list("[[1]]").is_err());
    }
}
