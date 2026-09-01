use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, bail};
use polars::{
    frame::DataFrame,
    prelude::{Column, IntoLazy, UniqueKeepStrategy},
};

use crate::sql::SqlEngine;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeepStrategy {
    First,
    Last,
    Any,
    None,
}

impl From<KeepStrategy> for UniqueKeepStrategy {
    fn from(value: KeepStrategy) -> Self {
        match value {
            KeepStrategy::First => Self::First,
            KeepStrategy::Last => Self::Last,
            KeepStrategy::Any => Self::Any,
            KeepStrategy::None => Self::None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinKind {
    Inner,
    Left,
    Full,
    Cross,
}

pub fn query(df: DataFrame, statement: &str) -> anyhow::Result<DataFrame> {
    if is_schema_less_empty(&df) {
        return Ok(df);
    }
    let mut engine = SqlEngine::new();
    engine.register_frame("df", df.lazy());
    engine.execute_collect(statement)
}

pub fn select(df: DataFrame, expressions: &[String]) -> anyhow::Result<DataFrame> {
    if expressions.is_empty() {
        bail!("select requires at least one column or SQL expression");
    }
    if is_schema_less_empty(&df) {
        return Ok(df);
    }
    let projection = expressions
        .iter()
        .map(|expression| sql_expression(expression))
        .collect::<Vec<_>>()
        .join(", ");
    query(df, &format!("SELECT {projection} FROM df"))
}

pub fn filter(df: DataFrame, predicate: &str) -> anyhow::Result<DataFrame> {
    if predicate.trim().is_empty() {
        bail!("filter requires a non-empty SQL predicate");
    }
    if is_schema_less_empty(&df) {
        return Ok(df);
    }
    query(df, &format!("SELECT * FROM df WHERE {predicate}"))
}

pub fn with_columns(mut df: DataFrame, assignments: &[String]) -> anyhow::Result<DataFrame> {
    if assignments.is_empty() {
        bail!("with-column requires at least one NAME=EXPRESSION assignment");
    }
    if is_schema_less_empty(&df) {
        return Ok(df);
    }

    for assignment in assignments {
        let (name, expression) = parse_mapping(assignment, "with-column")?;
        let mut projection = Vec::with_capacity(df.width() + 1);
        let mut replaced = false;
        for existing in df.get_column_names() {
            if existing.as_str() == name {
                projection.push(format!("{expression} AS {}", quote_identifier(name)));
                replaced = true;
            } else {
                projection.push(quote_identifier(existing.as_str()));
            }
        }
        if !replaced {
            projection.push(format!("{expression} AS {}", quote_identifier(name)));
        }
        df = query(df, &format!("SELECT {} FROM df", projection.join(", ")))
            .with_context(|| format!("failed to evaluate assignment {assignment:?}"))?;
    }

    Ok(df)
}

pub fn sort_by(
    df: DataFrame,
    expressions: &[String],
    descending: bool,
) -> anyhow::Result<DataFrame> {
    if expressions.is_empty() {
        bail!("sort-by requires at least one column or SQL expression");
    }
    if is_schema_less_empty(&df) {
        return Ok(df);
    }

    let order = expressions
        .iter()
        .map(|expression| {
            let (expression, direction) = parse_sort_expression(expression, descending);
            format!("{} {direction}", sql_expression(expression))
        })
        .collect::<Vec<_>>()
        .join(", ");
    query(df, &format!("SELECT * FROM df ORDER BY {order}"))
}

pub fn first(df: DataFrame, rows: usize) -> DataFrame {
    df.slice(0, rows)
}

pub fn last(df: DataFrame, rows: usize) -> DataFrame {
    let rows = rows.min(df.height());
    df.slice(-(rows as i64), rows)
}

pub fn slice(df: DataFrame, offset: i64, rows: usize) -> DataFrame {
    df.slice(offset, rows)
}

pub fn drop_columns(df: DataFrame, columns: &[String]) -> anyhow::Result<DataFrame> {
    if columns.is_empty() {
        bail!("drop requires at least one column name");
    }
    if is_schema_less_empty(&df) {
        return Ok(df);
    }
    for column in columns {
        if df.get_column_index(column).is_none() {
            bail!("column {column:?} does not exist");
        }
    }
    Ok(df.drop_many(columns.iter().map(String::as_str)))
}

pub fn rename_columns(df: DataFrame, mappings: &[String]) -> anyhow::Result<DataFrame> {
    if mappings.is_empty() {
        bail!("rename requires at least one OLD=NEW mapping");
    }
    if is_schema_less_empty(&df) {
        return Ok(df);
    }

    let mut renames = BTreeMap::new();
    for mapping in mappings {
        let (old, new) = parse_mapping(mapping, "rename")?;
        if df.get_column_index(old).is_none() {
            bail!("column {old:?} does not exist");
        }
        if renames.insert(old.to_owned(), new.to_owned()).is_some() {
            bail!("column {old:?} is renamed more than once");
        }
    }

    let output_names = df
        .get_column_names()
        .iter()
        .map(|name| {
            renames
                .get(name.as_str())
                .cloned()
                .unwrap_or_else(|| name.to_string())
        })
        .collect::<Vec<_>>();
    let unique_names = output_names.iter().collect::<BTreeSet<_>>();
    if unique_names.len() != output_names.len() {
        bail!("rename would create duplicate column names");
    }

    let projection = df
        .get_column_names()
        .iter()
        .map(|name| match renames.get(name.as_str()) {
            Some(new) => format!(
                "{} AS {}",
                quote_identifier(name.as_str()),
                quote_identifier(new)
            ),
            None => quote_identifier(name.as_str()),
        })
        .collect::<Vec<_>>()
        .join(", ");
    query(df, &format!("SELECT {projection} FROM df"))
}

pub fn unique(df: DataFrame, columns: &[String], keep: KeepStrategy) -> anyhow::Result<DataFrame> {
    if is_schema_less_empty(&df) {
        return Ok(df);
    }
    let subset = (!columns.is_empty()).then_some(columns);
    df.unique_stable(subset, keep.into(), None)
        .context("failed to remove duplicate rows")
}

pub fn drop_nulls(df: DataFrame, columns: &[String]) -> anyhow::Result<DataFrame> {
    if is_schema_less_empty(&df) {
        return Ok(df);
    }
    let subset = (!columns.is_empty()).then_some(columns);
    df.drop_nulls(subset)
        .context("failed to drop rows containing null values")
}

pub fn reverse(df: DataFrame) -> DataFrame {
    df.reverse()
}

pub fn group_by(
    df: DataFrame,
    keys: &[String],
    aggregations: &[String],
) -> anyhow::Result<DataFrame> {
    if keys.is_empty() {
        bail!("group-by requires at least one key column or expression");
    }
    if aggregations.is_empty() {
        bail!("group-by requires at least one --agg SQL aggregation");
    }
    if is_schema_less_empty(&df) {
        return Ok(df);
    }

    let key_expressions = keys
        .iter()
        .map(|key| sql_expression(key))
        .collect::<Vec<_>>();
    let mut projection = key_expressions.clone();
    projection.extend(aggregations.iter().cloned());
    query(
        df,
        &format!(
            "SELECT {} FROM df GROUP BY {}",
            projection.join(", "),
            key_expressions.join(", ")
        ),
    )
}

pub fn join(
    left: DataFrame,
    right: DataFrame,
    kind: JoinKind,
    left_on: &[String],
    right_on: &[String],
) -> anyhow::Result<DataFrame> {
    if kind != JoinKind::Cross {
        if left_on.is_empty() {
            bail!("join requires --on or matching --left-on/--right-on keys");
        }
        if left_on.len() != right_on.len() {
            bail!(
                "join key counts differ: {} left key(s), {} right key(s)",
                left_on.len(),
                right_on.len()
            );
        }
    }

    let left_is_empty = is_schema_less_empty(&left);
    let right_is_empty = is_schema_less_empty(&right);
    if left_is_empty || right_is_empty {
        return Ok(match (kind, left_is_empty, right_is_empty) {
            (JoinKind::Full, true, false) => right,
            (JoinKind::Left | JoinKind::Full, false, true) => left,
            _ => DataFrame::empty(),
        });
    }

    let left_names = left
        .get_column_names()
        .into_iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let right_names = right
        .get_column_names()
        .into_iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    for key in left_on {
        if !left_names.iter().any(|name| name == key) {
            bail!("left join key {key:?} does not exist");
        }
    }
    for key in right_on {
        if !right_names.iter().any(|name| name == key) {
            bail!("right join key {key:?} does not exist");
        }
    }

    let same_name_keys = left_on
        .iter()
        .zip(right_on)
        .filter(|(left, right)| left == right)
        .map(|(left, _)| left.as_str())
        .collect::<BTreeSet<_>>();
    let mut output_names = left_names.iter().cloned().collect::<BTreeSet<_>>();
    let mut projection = left_names
        .iter()
        .map(|name| {
            if kind == JoinKind::Full && same_name_keys.contains(name.as_str()) {
                format!(
                    "COALESCE(df.{quoted}, right_df.{quoted}) AS {quoted}",
                    quoted = quote_identifier(name)
                )
            } else {
                format!("df.{quoted} AS {quoted}", quoted = quote_identifier(name))
            }
        })
        .collect::<Vec<_>>();

    for name in &right_names {
        if same_name_keys.contains(name.as_str()) {
            continue;
        }
        let mut output_name = name.clone();
        if output_names.contains(&output_name) {
            output_name.push_str("_right");
            let base = output_name.clone();
            let mut suffix = 2;
            while output_names.contains(&output_name) {
                output_name = format!("{base}_{suffix}");
                suffix += 1;
            }
        }
        output_names.insert(output_name.clone());
        projection.push(format!(
            "right_df.{source} AS {target}",
            source = quote_identifier(name),
            target = quote_identifier(&output_name)
        ));
    }

    let mut engine = SqlEngine::new();
    engine.register_frame("df", left.lazy());
    engine.register_frame("right_df", right.lazy());

    let join_clause = match kind {
        JoinKind::Inner => "INNER JOIN",
        JoinKind::Left => "LEFT JOIN",
        JoinKind::Full => "FULL OUTER JOIN",
        JoinKind::Cross => "CROSS JOIN",
    };
    let condition = if kind == JoinKind::Cross {
        String::new()
    } else {
        format!(
            " ON {}",
            left_on
                .iter()
                .zip(right_on)
                .map(|(left, right)| format!(
                    "df.{} = right_df.{}",
                    quote_identifier(left),
                    quote_identifier(right)
                ))
                .collect::<Vec<_>>()
                .join(" AND ")
        )
    };

    engine.execute_collect(&format!(
        "SELECT {} FROM df {join_clause} right_df{condition}",
        projection.join(", ")
    ))
}

pub fn concat(mut frames: Vec<DataFrame>) -> anyhow::Result<DataFrame> {
    if frames.len() < 2 {
        bail!("concat requires at least two inputs");
    }
    frames.retain(|frame| !is_schema_less_empty(frame));
    let Some(mut output) = frames.first().cloned() else {
        return Ok(DataFrame::empty());
    };
    for frame in frames.into_iter().skip(1) {
        output
            .vstack_mut_owned(frame)
            .context("concat inputs must have compatible column names and data types")?;
    }
    Ok(output)
}

pub fn columns(df: &DataFrame) -> anyhow::Result<DataFrame> {
    let names = df
        .get_column_names()
        .into_iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    DataFrame::new_infer_height(vec![Column::new("column".into(), names)]).map_err(Into::into)
}

pub fn schema(df: &DataFrame) -> anyhow::Result<DataFrame> {
    let names = df
        .columns()
        .iter()
        .map(|column| column.name().to_string())
        .collect::<Vec<_>>();
    let data_types = df
        .columns()
        .iter()
        .map(|column| column.dtype().to_string())
        .collect::<Vec<_>>();
    DataFrame::new_infer_height(vec![
        Column::new("column".into(), names),
        Column::new("dtype".into(), data_types),
    ])
    .map_err(Into::into)
}

pub fn shape(df: &DataFrame) -> anyhow::Result<DataFrame> {
    DataFrame::new_infer_height(vec![
        Column::new("rows".into(), [df.height() as u64]),
        Column::new("columns".into(), [df.width() as u64]),
    ])
    .map_err(Into::into)
}

pub fn count(df: &DataFrame) -> anyhow::Result<DataFrame> {
    DataFrame::new_infer_height(vec![Column::new("count".into(), [df.height() as u64])])
        .map_err(Into::into)
}

fn is_schema_less_empty(df: &DataFrame) -> bool {
    df.width() == 0 && df.height() == 0
}

pub fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn sql_expression(expression: &str) -> String {
    let expression = expression.trim();
    if is_simple_identifier(expression) {
        quote_identifier(expression)
    } else {
        expression.to_owned()
    }
}

fn is_simple_identifier(expression: &str) -> bool {
    let mut chars = expression.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

fn parse_mapping<'a>(value: &'a str, command: &str) -> anyhow::Result<(&'a str, &'a str)> {
    let (left, right) = value
        .split_once('=')
        .with_context(|| format!("{command} expects NAME=VALUE but got {value:?}"))?;
    let left = left.trim();
    let right = right.trim();
    if left.is_empty() || right.is_empty() {
        bail!("{command} expects non-empty values around '=' but got {value:?}");
    }
    Ok((left, right))
}

fn parse_sort_expression(value: &str, default_descending: bool) -> (&str, &'static str) {
    if let Some(expression) = value.strip_suffix(":desc") {
        (expression, "DESC")
    } else if let Some(expression) = value.strip_suffix(":asc") {
        (expression, "ASC")
    } else if default_descending {
        (value, "DESC")
    } else {
        (value, "ASC")
    }
}
