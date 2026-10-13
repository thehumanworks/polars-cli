use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, bail};
use polars::{
    frame::DataFrame,
    prelude::{
        BooleanChunked, ChunkExpandAtIndex, ChunkFillNullValue, Column, Expr, IntoLazy,
        SortMultipleOptions, UniqueKeepStrategy,
    },
};

use crate::{selector, sql::SqlEngine};

pub mod aggregate;
pub mod column;
pub mod reshape;
pub mod strings;
pub mod temporal;

/// A column reference by exact name.
///
/// Unlike `polars::prelude::col`, names such as `*` or `^x$` are never treated as patterns, so
/// columns resolved by [`selector`] are always addressed literally.
pub(crate) fn col(name: &str) -> Expr {
    Expr::Column(name.into())
}

/// Evaluate a SQL predicate against the frame, treating null as false.
pub(crate) fn sql_mask(df: &DataFrame, predicate: &str) -> anyhow::Result<BooleanChunked> {
    if predicate.trim().is_empty() {
        bail!("expected a non-empty SQL predicate");
    }
    let result = query(
        df.clone(),
        &format!("SELECT ({predicate}) AS \"__pl_mask\" FROM df"),
    )?;
    let mask = result
        .column("__pl_mask")?
        .as_materialized_series()
        .bool()
        .with_context(|| format!("predicate {predicate:?} does not evaluate to a boolean"))?
        .fill_null_with_values(false)?;
    match mask.len() {
        length if length == df.height() => Ok(mask),
        1 => Ok(mask.new_from_index(0, df.height())),
        length => bail!(
            "predicate {predicate:?} produced {length} value(s) for {} row(s)",
            df.height()
        ),
    }
}

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
    Right,
    Full,
    Cross,
    Semi,
    Anti,
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
    let mut projection = Vec::with_capacity(expressions.len());
    let mut copies = Vec::new();
    for expression in expressions {
        if df.get_column_index(expression).is_some() {
            projection.push(project_column(expression, &mut copies));
        } else if expression != "*" && selector::is_selector_syntax(expression) {
            for name in selector::resolve(df.schema(), std::slice::from_ref(expression))? {
                projection.push(project_column(&name, &mut copies));
            }
        } else {
            projection.push(sql_expression(expression));
        }
    }
    if projection.is_empty() {
        return Ok(DataFrame::empty());
    }
    let mut df = df;
    for (hidden, name) in copies {
        let mut column = df.column(&name)?.clone();
        column.rename(hidden.into());
        df.with_column(column)?;
    }
    query(df, &format!("SELECT {} FROM df", projection.join(", ")))
}

/// A SQL projection item selecting the column `name` exactly.
///
/// Polars SQL reads a quoted `*` or `^…$` identifier as a pattern, so such columns are
/// projected from a hidden copy (recorded in `copies`) and renamed back.
fn project_column(name: &str, copies: &mut Vec<(String, String)>) -> String {
    if name == "*" || (name.starts_with('^') && name.ends_with('$')) {
        let hidden = format!("__pl_select_{}", copies.len());
        let item = format!(
            "{} AS {}",
            quote_identifier(&hidden),
            quote_identifier(name)
        );
        copies.push((hidden, name.to_owned()));
        item
    } else {
        quote_identifier(name)
    }
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

/// Sort by columns or SQL expressions.
///
/// The sort is stable, so ties keep their input order. Nulls sort last when ascending and
/// first when descending (SQL semantics) unless `nulls_last` forces them last.
pub fn sort_by(
    df: DataFrame,
    expressions: &[String],
    descending: bool,
    nulls_last: bool,
) -> anyhow::Result<DataFrame> {
    if expressions.is_empty() {
        bail!("sort-by requires at least one column or SQL expression");
    }
    if is_schema_less_empty(&df) {
        return Ok(df);
    }

    let mut keys = Vec::with_capacity(expressions.len());
    let mut directions = Vec::with_capacity(expressions.len());
    let mut computed = Vec::new();
    let mut temporary = Vec::new();
    for (index, expression) in expressions.iter().enumerate() {
        let (expression, direction) = parse_sort_expression(expression, descending);
        let expression = expression.trim();
        if let Ok(position) = expression.parse::<usize>() {
            // A bare integer is a 1-based column position, as in SQL `ORDER BY 2`.
            let name = position
                .checked_sub(1)
                .and_then(|index| {
                    df.get_column_names()
                        .get(index)
                        .map(|name| name.to_string())
                })
                .with_context(|| {
                    format!(
                        "sort-by position {position} is out of range for {} column(s)",
                        df.width()
                    )
                })?;
            keys.push(name);
        } else if df.get_column_index(expression).is_some() {
            keys.push(expression.to_owned());
        } else if is_simple_identifier(expression) {
            bail!("column {expression:?} does not exist");
        } else {
            let name = format!("__pl_sort_{index}");
            computed.push(format!("{expression} AS {}", quote_identifier(&name)));
            temporary.push(name.clone());
            keys.push(name);
        }
        directions.push(direction == "DESC");
    }
    let nulls = directions
        .iter()
        .map(|descending| nulls_last || !descending)
        .collect::<Vec<_>>();
    let df = if computed.is_empty() {
        df
    } else {
        query(df, &format!("SELECT *, {} FROM df", computed.join(", ")))?
    };
    let sorted = df
        .sort(
            keys,
            SortMultipleOptions::default()
                .with_order_descending_multi(directions)
                .with_nulls_last_multi(nulls)
                .with_maintain_order(true),
        )
        .context("failed to sort")?;
    Ok(sorted.drop_many(temporary.iter().map(String::as_str)))
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
    let names = selector::resolve(df.schema(), columns)?;
    Ok(df.drop_many(names.iter().map(String::as_str)))
}

pub fn rename_columns(df: DataFrame, mappings: &[String]) -> anyhow::Result<DataFrame> {
    if mappings.is_empty() {
        bail!("rename requires at least one OLD=NEW mapping");
    }
    if is_schema_less_empty(&df) {
        return Ok(df);
    }

    let pairs = parse_rename_pairs(mappings)?;
    let mut renames = BTreeMap::new();
    for (old, new) in pairs {
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
    let selected = selector::resolve(df.schema(), columns)?;
    if !columns.is_empty() && selected.is_empty() {
        bail!("unique: the column selection matched no columns");
    }
    let subset = (!selected.is_empty()).then_some(selected.as_slice());
    df.unique_stable(subset, keep.into(), None)
        .context("failed to remove duplicate rows")
}

pub fn drop_nulls(df: DataFrame, columns: &[String]) -> anyhow::Result<DataFrame> {
    if is_schema_less_empty(&df) {
        return Ok(df);
    }
    let selected = selector::resolve(df.schema(), columns)?;
    if !columns.is_empty() && selected.is_empty() {
        // No selected column can hold a null, so no row is dropped.
        return Ok(df);
    }
    let subset = (!selected.is_empty()).then_some(selected.as_slice());
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

    // Groups are emitted in order of first appearance so output is deterministic.
    let row = "__pl_group_row";
    let first = "__pl_group_first";
    let df = df.with_row_index(row.into(), None)?;
    let key_expressions = keys
        .iter()
        .map(|key| sql_expression(key))
        .collect::<Vec<_>>();
    let mut projection = key_expressions.clone();
    projection.extend(aggregations.iter().cloned());
    projection.push(format!(
        "MIN({}) AS {}",
        quote_identifier(row),
        quote_identifier(first)
    ));
    let grouped = query(
        df.clone(),
        &format!(
            "SELECT {} FROM df GROUP BY {} ORDER BY {}",
            projection.join(", "),
            key_expressions.join(", "),
            quote_identifier(first)
        ),
    );
    match grouped {
        Ok(grouped) => Ok(grouped.drop_many([first])),
        // Report errors in terms of the user's query, not the ordering helper columns.
        Err(error) => {
            let mut user_projection = key_expressions.clone();
            user_projection.extend(aggregations.iter().cloned());
            query(
                df.drop(row)?,
                &format!(
                    "SELECT {} FROM df GROUP BY {}",
                    user_projection.join(", "),
                    key_expressions.join(", ")
                ),
            )?;
            Err(error)
        }
    }
}

/// Options that refine how `join` matches keys and names its output.
#[derive(Debug, Clone, Copy)]
pub struct JoinOptions<'a> {
    /// Suffix for right-hand columns whose names clash with left-hand ones.
    pub suffix: &'a str,
    /// Match null keys with each other, which SQL equality never does.
    pub nulls_equal: bool,
    /// Merge differently named key pairs into the left key column.
    pub coalesce: bool,
}

impl Default for JoinOptions<'_> {
    fn default() -> Self {
        Self {
            suffix: "_right",
            nulls_equal: false,
            coalesce: false,
        }
    }
}

pub fn join(
    left: DataFrame,
    right: DataFrame,
    kind: JoinKind,
    left_on: &[String],
    right_on: &[String],
    options: &JoinOptions<'_>,
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
            (JoinKind::Full | JoinKind::Right, true, false) => right,
            (JoinKind::Left | JoinKind::Full | JoinKind::Anti, false, true) => left,
            _ => DataFrame::empty(),
        });
    }

    let (left_frame, right_frame) = (left, right);
    let left_names = left_frame
        .get_column_names()
        .into_iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let right_names = right_frame
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

    // Same-name keys are always merged into one output column; with `coalesce`, differently
    // named key pairs are merged into the left key column too.
    let merged_keys = left_on
        .iter()
        .zip(right_on)
        .filter(|(left, right)| left == right || options.coalesce)
        .map(|(left, right)| (left.as_str(), right.as_str()))
        .collect::<BTreeMap<_, _>>();
    let merged_right_keys = merged_keys.values().copied().collect::<BTreeSet<_>>();
    let mut output_names = left_names.iter().cloned().collect::<BTreeSet<_>>();
    let mut projection = left_names
        .iter()
        .map(|name| {
            let quoted = quote_identifier(name);
            match (kind, merged_keys.get(name.as_str())) {
                (JoinKind::Full, Some(right)) => format!(
                    "COALESCE(df.{quoted}, right_df.{}) AS {quoted}",
                    quote_identifier(right)
                ),
                (JoinKind::Right, Some(right)) => {
                    format!("right_df.{} AS {quoted}", quote_identifier(right))
                }
                _ => format!("df.{quoted} AS {quoted}"),
            }
        })
        .collect::<Vec<_>>();

    let semi_or_anti = matches!(kind, JoinKind::Semi | JoinKind::Anti);
    for name in &right_names {
        if semi_or_anti || merged_right_keys.contains(name.as_str()) {
            continue;
        }
        let mut output_name = name.clone();
        if output_names.contains(&output_name) {
            output_name.push_str(options.suffix);
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

    // SQL equality never matches nulls. For --nulls-equal, compare hidden (is-null, value)
    // pairs instead: they are equal exactly when both keys are null or both keys are equal,
    // because nulls are replaced by the same typed placeholder on both sides.
    // Hidden row numbers make the output order deterministic: Polars' hash joins do not
    // guarantee an order, while scripts that diff or cache output need one.
    let left_row = "__pl_left_row";
    let right_row = "__pl_right_row";
    projection.push(format!(
        "df.{quoted} AS {quoted}",
        quoted = quote_identifier(left_row)
    ));
    if !semi_or_anti {
        projection.push(format!(
            "right_df.{quoted} AS {quoted}",
            quoted = quote_identifier(right_row)
        ));
    }

    let mut left = left_frame.clone().lazy().with_row_index(left_row, None);
    let mut right = right_frame.clone().lazy().with_row_index(right_row, None);
    let mut conditions = Vec::with_capacity(left_on.len());
    for (index, (left_key, right_key)) in left_on.iter().zip(right_on).enumerate() {
        if !options.nulls_equal || kind == JoinKind::Cross {
            conditions.push((left_key.clone(), right_key.clone()));
            continue;
        }
        let flag = format!("__pl_join_null_{index}");
        let value = format!("__pl_join_value_{index}");
        let placeholder = |frame: &DataFrame, key: &str| -> anyhow::Result<Expr> {
            let dtype = frame.schema().get(key).cloned().unwrap_or_default();
            if dtype.is_nested() || dtype.is_categorical() || dtype.is_enum() {
                bail!("--nulls-equal does not support {dtype} key {key:?}");
            }
            Ok(polars::prelude::lit(0).strict_cast(dtype))
        };
        let left_fill = placeholder(&left_frame, left_key)?;
        let right_fill = placeholder(&right_frame, right_key)?;
        left = left.with_columns([
            col(left_key).is_null().alias(flag.as_str()),
            col(left_key).fill_null(left_fill).alias(value.as_str()),
        ]);
        right = right.with_columns([
            col(right_key).is_null().alias(flag.as_str()),
            col(right_key).fill_null(right_fill).alias(value.as_str()),
        ]);
        conditions.push((flag.clone(), flag));
        conditions.push((value.clone(), value));
    }

    let mut engine = SqlEngine::new();
    engine.register_frame("df", left);
    engine.register_frame("right_df", right);
    let join_clause = match kind {
        JoinKind::Inner => "INNER JOIN",
        JoinKind::Left => "LEFT JOIN",
        JoinKind::Right => "RIGHT JOIN",
        JoinKind::Full => "FULL OUTER JOIN",
        JoinKind::Cross => "CROSS JOIN",
        JoinKind::Semi => "LEFT SEMI JOIN",
        JoinKind::Anti => "LEFT ANTI JOIN",
    };
    let condition = if kind == JoinKind::Cross {
        String::new()
    } else {
        format!(
            " ON {}",
            conditions
                .iter()
                .map(|(left, right)| format!(
                    "df.{} = right_df.{}",
                    quote_identifier(left),
                    quote_identifier(right)
                ))
                .collect::<Vec<_>>()
                .join(" AND ")
        )
    };

    let joined = engine.execute_collect(&format!(
        "SELECT {} FROM df {join_clause} right_df{condition}",
        projection.join(", ")
    ))?;
    let order = match kind {
        _ if semi_or_anti => vec![left_row],
        JoinKind::Right => vec![right_row, left_row],
        _ => vec![left_row, right_row],
    };
    let sorted = joined
        .sort(
            order,
            SortMultipleOptions::default()
                .with_nulls_last(true)
                .with_maintain_order(true),
        )
        .context("failed to order join output")?;
    Ok(sorted.drop_many([left_row, right_row]))
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

/// Parse rename arguments: `OLD=NEW` mappings, or Nushell-style `OLD NEW` pairs.
fn parse_rename_pairs(mappings: &[String]) -> anyhow::Result<Vec<(&str, &str)>> {
    let with_equals = mappings.iter().filter(|value| value.contains('=')).count();
    if with_equals == mappings.len() {
        return mappings
            .iter()
            .map(|mapping| parse_mapping(mapping, "rename"))
            .collect();
    }
    if with_equals == 0 && mappings.len().is_multiple_of(2) {
        return Ok(mappings
            .chunks(2)
            .map(|pair| (pair[0].trim(), pair[1].trim()))
            .collect());
    }
    bail!("rename expects OLD=NEW mappings or OLD NEW pairs, not a mix");
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
