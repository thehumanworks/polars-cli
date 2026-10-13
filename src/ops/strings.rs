//! String column transforms mirroring Nushell's `polars str-*`, `contains`, and case commands.

use anyhow::{Context, bail};
use polars::prelude::{DataFrame, Expr, IntoLazy, Null, lit};

use super::{
    col,
    column::{MAX_OFFSET, Target, resolve_required, transform},
    is_schema_less_empty,
};

pub fn uppercase(df: DataFrame, target: &Target) -> anyhow::Result<DataFrame> {
    transform(df, target, "uppercase", |source| {
        Ok(source.string("uppercase")?.str().to_uppercase())
    })
}

pub fn lowercase(df: DataFrame, target: &Target) -> anyhow::Result<DataFrame> {
    transform(df, target, "lowercase", |source| {
        Ok(source.string("lowercase")?.str().to_lowercase())
    })
}

pub fn lengths(df: DataFrame, target: &Target, bytes: bool) -> anyhow::Result<DataFrame> {
    transform(df, target, "str-lengths", |source| {
        let expr = source.string("str-lengths")?.str();
        Ok(if bytes {
            expr.len_bytes()
        } else {
            expr.len_chars()
        })
    })
}

pub fn replace(
    df: DataFrame,
    target: &Target,
    pattern: &str,
    replacement: &str,
    all: bool,
    literal: bool,
) -> anyhow::Result<DataFrame> {
    let command = if all {
        "str-replace-all"
    } else {
        "str-replace"
    };
    if !literal {
        regex::Regex::new(pattern)
            .with_context(|| format!("{command}: invalid regex pattern {pattern:?}"))?;
    }
    transform(df, target, command, |source| {
        let expr = source.string(command)?.str();
        Ok(if all {
            expr.replace_all(lit(pattern), lit(replacement), literal)
        } else {
            expr.replace(lit(pattern), lit(replacement), literal)
        })
    })
}

pub fn slice(
    df: DataFrame,
    target: &Target,
    start: i64,
    length: Option<u64>,
) -> anyhow::Result<DataFrame> {
    let length = length.map_or_else(|| lit(Null {}), lit);
    let start = start.clamp(-MAX_OFFSET, MAX_OFFSET);
    transform(df, target, "str-slice", |source| {
        Ok(source
            .string("str-slice")?
            .str()
            .slice(lit(start), length.clone()))
    })
}

pub fn split(df: DataFrame, target: &Target, separator: &str) -> anyhow::Result<DataFrame> {
    transform(df, target, "str-split", |source| {
        Ok(source.string("str-split")?.str().split(lit(separator)))
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StripSide {
    Both,
    Start,
    End,
}

/// Strip characters from strings; `None` strips whitespace.
pub fn strip_chars(
    df: DataFrame,
    target: &Target,
    characters: Option<&str>,
    side: StripSide,
) -> anyhow::Result<DataFrame> {
    let matches = characters.map_or_else(|| lit(Null {}), lit);
    transform(df, target, "str-strip-chars", |source| {
        let expr = source.string("str-strip-chars")?.str();
        Ok(match side {
            StripSide::Both => expr.strip_chars(matches.clone()),
            StripSide::Start => expr.strip_chars_start(matches.clone()),
            StripSide::End => expr.strip_chars_end(matches.clone()),
        })
    })
}

pub fn contains(
    df: DataFrame,
    target: &Target,
    pattern: &str,
    literal: bool,
) -> anyhow::Result<DataFrame> {
    if !literal {
        regex::Regex::new(pattern)
            .with_context(|| format!("contains: invalid regex pattern {pattern:?}"))?;
    }
    transform(df, target, "contains", |source| {
        let expr = source.string("contains")?.str();
        Ok(if literal {
            expr.contains_literal(lit(pattern))
        } else {
            expr.contains(lit(pattern), true)
        })
    })
}

/// Join every value of each selected string column into one value, producing one row.
pub fn join(
    df: DataFrame,
    columns: &[String],
    delimiter: &str,
    ignore_nulls: bool,
) -> anyhow::Result<DataFrame> {
    if is_schema_less_empty(&df) {
        return Ok(df);
    }
    let names = resolve_required(&df, columns, "str-join")?;
    let exprs = names
        .iter()
        .map(|name| {
            let dtype = df.schema().get(name.as_str()).cloned().unwrap_or_default();
            if !(dtype.is_string() || dtype.is_categorical() || dtype.is_null()) {
                bail!("str-join requires a string column, but {name:?} is {dtype}");
            }
            Ok(col(name)
                .cast(polars::prelude::DataType::String)
                .str()
                .join(delimiter, ignore_nulls))
        })
        .collect::<anyhow::Result<Vec<Expr>>>()?;
    df.lazy().select(exprs).collect().context("str-join failed")
}
