//! Column selectors shared by every command that accepts column names.
//!
//! A selector token is resolved against the input schema in this order:
//!
//! 1. an exact column name always wins, so headers that look like selectors stay addressable;
//! 2. `*` selects every column;
//! 3. `^...$` is a Polars-style regular expression over column names;
//! 4. `type:GROUP[,GROUP...]` selects columns by data type, such as `type:numeric` or `type:i64`;
//! 5. `index:N[,N...]` selects columns by position; negative positions count from the end.
//!
//! Exact names that do not exist are errors. Pattern selectors that match nothing select
//! nothing, mirroring Polars and Nushell selectors.

use anyhow::{Context, bail};
use polars::prelude::{DataType, Schema};
use regex::Regex;

use crate::value::{parse_dtype, split_top_level_commas};

const TYPE_PREFIX: &str = "type:";
const INDEX_PREFIX: &str = "index:";

/// Type selector groups, named after Nushell's `polars selector` subcommands.
pub const TYPE_GROUPS: &[&str] = &[
    "numeric",
    "integer",
    "signed-integer",
    "unsigned-integer",
    "float",
    "decimal",
    "string",
    "boolean",
    "temporal",
    "date",
    "datetime",
    "duration",
    "time",
    "list",
    "array",
    "struct",
    "nested",
    "categorical",
    "enum",
    "binary",
    "null",
];

/// True when `token` uses selector syntax rather than naming a column or SQL expression.
pub fn is_selector_syntax(token: &str) -> bool {
    token == "*"
        || is_regex_selector(token)
        || token.starts_with(TYPE_PREFIX)
        || token.starts_with(INDEX_PREFIX)
}

/// Resolve selector tokens to column names, preserving token order and removing duplicates.
pub fn resolve(schema: &Schema, specs: &[String]) -> anyhow::Result<Vec<String>> {
    let mut selected = Vec::new();
    for spec in specs {
        for name in resolve_one(schema, spec)? {
            if !selected.contains(&name) {
                selected.push(name);
            }
        }
    }
    Ok(selected)
}

/// Resolve selectors, defaulting to every column when no selector is supplied.
pub fn resolve_or_all(schema: &Schema, specs: &[String]) -> anyhow::Result<Vec<String>> {
    if specs.is_empty() {
        Ok(schema.iter_names().map(ToString::to_string).collect())
    } else {
        resolve(schema, specs)
    }
}

fn resolve_one(schema: &Schema, spec: &str) -> anyhow::Result<Vec<String>> {
    if schema.contains(spec) {
        return Ok(vec![spec.to_owned()]);
    }
    let names = || schema.iter_names().map(ToString::to_string);
    if spec == "*" {
        return Ok(names().collect());
    }
    if is_regex_selector(spec) {
        let pattern =
            Regex::new(spec).with_context(|| format!("invalid column regex selector {spec:?}"))?;
        return Ok(names().filter(|name| pattern.is_match(name)).collect());
    }
    if let Some(groups) = spec.strip_prefix(TYPE_PREFIX) {
        let matchers = split_top_level_commas(groups)
            .into_iter()
            .map(|group| TypeMatcher::parse(group.trim()))
            .collect::<anyhow::Result<Vec<_>>>()?;
        return Ok(schema
            .iter()
            .filter(|(_, dtype)| matchers.iter().any(|matcher| matcher.matches(dtype)))
            .map(|(name, _)| name.to_string())
            .collect());
    }
    if let Some(indices) = spec.strip_prefix(INDEX_PREFIX) {
        let width = schema.len();
        let all = names().collect::<Vec<_>>();
        return indices
            .split(',')
            .map(|index| {
                let index = index.trim();
                let position = index.parse::<i64>().with_context(|| {
                    format!("index selector expects integers but got {index:?}")
                })?;
                let resolved = if position < 0 {
                    width as i64 + position
                } else {
                    position
                };
                if resolved < 0 || resolved >= width as i64 {
                    bail!("column index {position} is out of range for {width} column(s)");
                }
                Ok(all[resolved as usize].clone())
            })
            .collect();
    }
    bail!("column {spec:?} does not exist")
}

fn is_regex_selector(token: &str) -> bool {
    token.len() > 1 && token.starts_with('^') && token.ends_with('$')
}

enum TypeMatcher {
    Group(&'static str),
    Exact(DataType),
}

impl TypeMatcher {
    fn parse(group: &str) -> anyhow::Result<Self> {
        let normalized = group.to_ascii_lowercase().replace('_', "-");
        let alias = match normalized.as_str() {
            "str" | "utf8" => "string",
            "bool" => "boolean",
            "int" => "integer",
            "cat" => "categorical",
            "signed" => "signed-integer",
            "unsigned" => "unsigned-integer",
            other => other,
        };
        if let Some(group) = TYPE_GROUPS.iter().find(|candidate| **candidate == alias) {
            return Ok(Self::Group(group));
        }
        parse_dtype(group).map(Self::Exact).map_err(|_| {
            anyhow::anyhow!(
                "unknown type selector {group:?}; use a dtype such as i64 or one of: {}",
                TYPE_GROUPS.join(", ")
            )
        })
    }

    fn matches(&self, dtype: &DataType) -> bool {
        match self {
            Self::Exact(expected) => expected == dtype,
            Self::Group(group) => match *group {
                "numeric" => dtype.is_numeric(),
                "integer" => dtype.is_integer(),
                "signed-integer" => dtype.is_signed_integer(),
                "unsigned-integer" => dtype.is_unsigned_integer(),
                "float" => dtype.is_float(),
                "decimal" => dtype.is_decimal(),
                "string" => dtype.is_string(),
                "boolean" => dtype.is_bool(),
                "temporal" => dtype.is_temporal(),
                "date" => matches!(dtype, DataType::Date),
                "datetime" => matches!(dtype, DataType::Datetime(_, _)),
                "duration" => matches!(dtype, DataType::Duration(_)),
                "time" => matches!(dtype, DataType::Time),
                "list" => matches!(dtype, DataType::List(_)),
                "array" => matches!(dtype, DataType::Array(_, _)),
                "struct" => matches!(dtype, DataType::Struct(_)),
                "nested" => dtype.is_nested(),
                "categorical" => dtype.is_categorical(),
                "enum" => dtype.is_enum(),
                "binary" => matches!(dtype, DataType::Binary),
                "null" => matches!(dtype, DataType::Null),
                _ => false,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use polars::prelude::{DataType, Field, Schema};

    use super::{is_selector_syntax, resolve, resolve_or_all};

    fn schema() -> Schema {
        Schema::from_iter([
            Field::new("id".into(), DataType::Int64),
            Field::new("name".into(), DataType::String),
            Field::new("score".into(), DataType::Float64),
            Field::new("type:odd".into(), DataType::String),
        ])
    }

    fn specs(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn resolves_names_regex_types_and_indices_in_order() {
        let schema = schema();
        assert_eq!(
            resolve(&schema, &specs(&["score", "^i.$", "type:string"])).unwrap(),
            ["score", "id", "name", "type:odd"]
        );
        assert_eq!(
            resolve(&schema, &specs(&["index:-1,0"])).unwrap(),
            ["type:odd", "id"]
        );
        assert_eq!(
            resolve(&schema, &specs(&["type:numeric"])).unwrap(),
            ["id", "score"]
        );
        assert_eq!(resolve(&schema, &specs(&["type:i64"])).unwrap(), ["id"]);
        assert_eq!(
            resolve(&schema, &specs(&["type:datetime<ms, UTC>,f64"])).unwrap(),
            ["score"]
        );
    }

    #[test]
    fn exact_names_win_over_selector_syntax() {
        assert_eq!(
            resolve(&schema(), &specs(&["type:odd"])).unwrap(),
            ["type:odd"]
        );
    }

    #[test]
    fn missing_names_and_bad_selectors_are_errors() {
        let schema = schema();
        assert!(resolve(&schema, &specs(&["missing"])).is_err());
        assert!(resolve(&schema, &specs(&["index:9"])).is_err());
        assert!(resolve(&schema, &specs(&["type:nonsense"])).is_err());
        assert!(resolve(&schema, &specs(&["^nothing$"])).unwrap().is_empty());
    }

    #[test]
    fn empty_selection_defaults_to_every_column() {
        assert_eq!(resolve_or_all(&schema(), &[]).unwrap().len(), 4);
        assert!(is_selector_syntax("type:float"));
        assert!(!is_selector_syntax("price * 2"));
    }
}
