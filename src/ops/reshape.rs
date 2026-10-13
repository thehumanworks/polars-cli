//! Frame-level reshaping and row operations: append, take, sample, explode, unnest, pivot,
//! unpivot, dummies, join-where, row indices, duplicate masks, and rolling group-by.

use std::collections::BTreeSet;

use anyhow::{Context, bail};
use polars::prelude::{
    ClosedWindow, DataFrame, DataFrameOps, DataType, ExplodeOptions, Expr, IdxCa, IdxSize,
    IntoLazy, LazyFrame, PlSmallStr, RollingGroupOptions, Selector, SortMultipleOptions, UnionArgs,
    UniqueKeepStrategy, UnpivotArgsDSL, by_name, concat, element,
};

use super::temporal::parse_duration as temporal_duration;
use super::{
    aggregate::row_key,
    col,
    column::{Source, resolve_required},
    is_schema_less_empty,
};
use crate::selector;

fn names_selector(names: &[String]) -> Selector {
    by_name(names.iter().map(String::as_str), true, false)
}

/// Append another frame: as new columns (default, Nushell semantics) or as rows (`vertical`).
///
/// Horizontal appends rename clashing right-hand columns with an `_x` suffix.
pub fn append(left: DataFrame, right: DataFrame, vertical: bool) -> anyhow::Result<DataFrame> {
    if is_schema_less_empty(&right) {
        return Ok(left);
    }
    if is_schema_less_empty(&left) {
        return Ok(right);
    }
    if vertical {
        return super::concat(vec![left, right]);
    }
    if left.height() != right.height() {
        bail!(
            "append needs equal row counts to add columns (left {}, right {}); use --col to append rows",
            left.height(),
            right.height()
        );
    }
    let mut taken = left
        .get_column_names()
        .into_iter()
        .map(ToString::to_string)
        .collect::<BTreeSet<_>>();
    let mut columns = left.columns().to_vec();
    for column in right.columns() {
        let mut name = column.name().to_string();
        while taken.contains(&name) {
            name.push_str("_x");
        }
        taken.insert(name.clone());
        let mut column = column.clone();
        column.rename(name.into());
        columns.push(column);
    }
    DataFrame::new_infer_height(columns).map_err(Into::into)
}

/// Gather rows by index; negative indices count from the end.
pub fn take(df: DataFrame, indices: &[i64]) -> anyhow::Result<DataFrame> {
    if is_schema_less_empty(&df) {
        if indices.is_empty() {
            return Ok(df);
        }
        bail!("take cannot select rows from empty input");
    }
    let height = df.height() as i64;
    let positions = indices
        .iter()
        .map(|index| {
            let position = if *index < 0 { height + index } else { *index };
            if position < 0 || position >= height {
                bail!("row index {index} is out of range for {height} row(s)");
            }
            Ok(position as IdxSize)
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let positions = IdxCa::from_vec("take".into(), positions);
    df.take(&positions).map_err(Into::into)
}

#[derive(Debug, Clone, Copy)]
pub enum SampleSize {
    Rows(usize),
    Fraction(f64),
}

pub fn sample(
    df: DataFrame,
    size: SampleSize,
    seed: Option<u64>,
    replace: bool,
    shuffle: bool,
) -> anyhow::Result<DataFrame> {
    if is_schema_less_empty(&df) {
        return Ok(df);
    }
    let rows = match size {
        SampleSize::Rows(rows) => rows,
        SampleSize::Fraction(fraction) => {
            if !fraction.is_finite() || fraction < 0.0 || (!replace && fraction > 1.0) {
                bail!("sample --fraction must be between 0 and 1 unless --replace is set");
            }
            (df.height() as f64 * fraction) as usize
        }
    };
    if !replace && rows > df.height() {
        bail!(
            "cannot sample {rows} row(s) from {} without --replace",
            df.height()
        );
    }
    df.sample_n_literal(rows, replace, shuffle, seed)
        .context("sample failed")
}

pub fn explode(
    df: DataFrame,
    columns: &[String],
    empty_as_null: bool,
    keep_nulls: bool,
) -> anyhow::Result<DataFrame> {
    if is_schema_less_empty(&df) {
        return Ok(df);
    }
    let names = if columns.is_empty() {
        nested_columns(&df, |dtype| {
            matches!(dtype, DataType::List(_) | DataType::Array(_, _))
        })
    } else {
        resolve_required(&df, columns, "explode")?
    };
    if names.is_empty() {
        return Ok(df);
    }
    df.lazy()
        .explode(
            names_selector(&names),
            ExplodeOptions {
                empty_as_null,
                keep_nulls,
            },
        )
        .collect()
        .context("explode failed")
}

pub fn unnest(
    df: DataFrame,
    columns: &[String],
    separator: Option<&str>,
) -> anyhow::Result<DataFrame> {
    if is_schema_less_empty(&df) {
        return Ok(df);
    }
    let names = if columns.is_empty() {
        nested_columns(&df, |dtype| matches!(dtype, DataType::Struct(_)))
    } else {
        resolve_required(&df, columns, "unnest")?
    };
    if names.is_empty() {
        return Ok(df);
    }
    df.lazy()
        .unnest(names_selector(&names), separator.map(PlSmallStr::from))
        .collect()
        .context("unnest failed")
}

fn nested_columns(df: &DataFrame, predicate: impl Fn(&DataType) -> bool) -> Vec<String> {
    df.schema()
        .iter()
        .filter(|(_, dtype)| predicate(dtype))
        .map(|(name, _)| name.to_string())
        .collect()
}

#[derive(Debug, Clone, Default)]
pub struct PivotOptions {
    pub on: Vec<String>,
    pub index: Vec<String>,
    pub values: Vec<String>,
    pub aggregate: Option<String>,
    pub separator: String,
}

/// Pivot from long to wide. Output rows and generated columns keep first-appearance order.
pub fn pivot(df: DataFrame, options: &PivotOptions) -> anyhow::Result<DataFrame> {
    if is_schema_less_empty(&df) {
        return Ok(df);
    }
    if options.on.is_empty() {
        bail!("pivot requires --on");
    }
    if options.index.is_empty() && options.values.is_empty() {
        bail!("pivot requires --index, --values, or both");
    }
    let schema = df.schema().clone();
    let on = resolve_required(&df, &options.on, "pivot --on")?;
    let explicit_index = selector::resolve(&schema, &options.index)?;
    let explicit_values = selector::resolve(&schema, &options.values)?;
    let remaining = |exclude: &[&Vec<String>]| {
        schema
            .iter_names()
            .map(ToString::to_string)
            .filter(|name| !exclude.iter().any(|set| set.contains(name)))
            .collect::<Vec<_>>()
    };
    let index = if options.index.is_empty() {
        remaining(&[&on, &explicit_values])
    } else {
        explicit_index
    };
    let values = if options.values.is_empty() {
        remaining(&[&on, &index])
    } else {
        explicit_values
    };
    if values.is_empty() {
        bail!("pivot has no value columns left after --on and --index");
    }
    let aggregate = match options.aggregate.as_deref().map(str::to_ascii_lowercase) {
        None => element().item(true),
        Some(name) => match name.as_str() {
            "first" => element().first(),
            "last" => element().last(),
            "sum" => element().sum(),
            "min" => element().min(),
            "max" => element().max(),
            "mean" => element().mean(),
            "median" => element().median(),
            "count" | "len" | "length" => element().len(),
            "item" | "element" => element().item(true),
            other => bail!(
                "unknown pivot aggregate {other:?}; expected first, last, sum, min, max, mean, median, count, or item"
            ),
        },
    };
    let on_columns = df.select(on.iter().map(String::as_str))?.unique_stable(
        None,
        UniqueKeepStrategy::First,
        None,
    )?;
    df.lazy()
        .pivot(
            names_selector(&on),
            on_columns.into(),
            names_selector(&index),
            names_selector(&values),
            aggregate,
            true,
            options.separator.as_str().into(),
        )
        .collect()
        .context("pivot failed; use --aggregate when a cell has several values")
}

pub fn unpivot(
    df: DataFrame,
    index: &[String],
    on: &[String],
    variable_name: Option<&str>,
    value_name: Option<&str>,
) -> anyhow::Result<DataFrame> {
    if is_schema_less_empty(&df) {
        return Ok(df);
    }
    let schema = df.schema().clone();
    let index = selector::resolve(&schema, index)?;
    let on = if on.is_empty() {
        None
    } else {
        Some(names_selector(&selector::resolve(&schema, on)?))
    };
    df.lazy()
        .unpivot(UnpivotArgsDSL {
            on,
            index: names_selector(&index),
            variable_name: variable_name.map(PlSmallStr::from),
            value_name: value_name.map(PlSmallStr::from),
        })
        .collect()
        .context("unpivot failed")
}

pub fn dummies(
    df: DataFrame,
    columns: &[String],
    separator: Option<&str>,
    drop_first: bool,
    drop_nulls: bool,
) -> anyhow::Result<DataFrame> {
    if is_schema_less_empty(&df) {
        return Ok(df);
    }
    let names = resolve_required(&df, columns, "dummies")?;
    df.columns_to_dummies(
        names.iter().map(String::as_str).collect(),
        separator,
        drop_first,
        drop_nulls,
    )
    .context("dummies failed")
}

/// Join on an arbitrary SQL predicate. Output is ordered by left row, then right row.
pub fn join_where(
    left: DataFrame,
    right: DataFrame,
    predicate: &str,
    suffix: &str,
) -> anyhow::Result<DataFrame> {
    if is_schema_less_empty(&left) || is_schema_less_empty(&right) {
        return Ok(DataFrame::empty());
    }
    let predicate = polars::sql::sql_expr(predicate)
        .with_context(|| format!("invalid join-where predicate {predicate:?}"))?;
    let left_index = "__pl_left_row";
    let right_index = "__pl_right_row";
    left.lazy()
        .with_row_index(left_index, None)
        .join_builder()
        .with(right.lazy().with_row_index(right_index, None))
        .suffix(suffix)
        .join_where(vec![predicate])
        .sort(
            [left_index, right_index],
            SortMultipleOptions::default().with_maintain_order(true),
        )
        .drop(names_selector(&[
            left_index.to_owned(),
            right_index.to_owned(),
        ]))
        .collect()
        .context("join-where failed")
}

/// Concatenate frames vertically, optionally aligning columns by name and casting to supertypes.
pub fn concat_frames(
    frames: Vec<DataFrame>,
    diagonal: bool,
    to_supertypes: bool,
) -> anyhow::Result<DataFrame> {
    if !diagonal && !to_supertypes {
        return super::concat(frames);
    }
    if frames.len() < 2 {
        bail!("concat requires at least two inputs");
    }
    let frames = frames
        .into_iter()
        .filter(|frame| !is_schema_less_empty(frame))
        .map(IntoLazy::lazy)
        .collect::<Vec<LazyFrame>>();
    if frames.is_empty() {
        return Ok(DataFrame::empty());
    }
    concat(
        frames,
        UnionArgs {
            diagonal,
            to_supertypes,
            maintain_order: true,
            ..Default::default()
        },
    )?
    .collect()
    .context("concat inputs could not be combined")
}

pub fn with_row_index(df: DataFrame, name: &str, offset: IdxSize) -> anyhow::Result<DataFrame> {
    if df.get_column_index(name).is_some() {
        bail!("column {name:?} already exists; choose another --name");
    }
    df.with_row_index(name.into(), Some(offset))
        .map_err(Into::into)
}

/// Row-level duplicate/unique mask over the selected columns (default: every column).
///
/// Without `alias` the output is just the mask (`is_duplicated`/`is_unique`), as in Nushell;
/// with `alias` the mask is added to the frame.
pub fn duplicate_mask(
    df: DataFrame,
    columns: &[String],
    unique: bool,
    alias: Option<&str>,
) -> anyhow::Result<DataFrame> {
    if is_schema_less_empty(&df) {
        return Ok(df);
    }
    let command = if unique { "is-unique" } else { "is-duplicated" };
    let names = resolve_required(&df, columns, command)?;
    let key = row_key(&names);
    let mask = if unique {
        key.is_unique()
    } else {
        key.is_duplicated()
    };
    let lazy = df.lazy();
    match alias {
        Some(name) => lazy.with_column(mask.alias(name)),
        None => lazy.select([mask.alias(command.replace('-', "_"))]),
    }
    .collect()
    .with_context(|| format!("{command} failed"))
}

#[derive(Debug, Clone)]
pub struct RollingBy {
    pub index_column: String,
    pub period: String,
    pub offset: Option<String>,
    pub closed: ClosedWindow,
    pub group_by: Vec<String>,
    pub aggregations: Vec<String>,
}

/// Rolling group-by over a sorted time or integer index column, with SQL aggregations.
pub fn rolling_by(df: DataFrame, options: &RollingBy) -> anyhow::Result<DataFrame> {
    if is_schema_less_empty(&df) {
        return Ok(df);
    }
    if options.aggregations.is_empty() {
        bail!("rolling --index-column requires at least one --agg SQL aggregation");
    }
    let index = options.index_column.as_str();
    let dtype = df
        .schema()
        .get(index)
        .cloned()
        .with_context(|| format!("column {index:?} does not exist"))?;
    let index_expr = if dtype.is_string() {
        Source {
            df: &df,
            name: index,
            dtype: &dtype,
        }
        .temporal("rolling")?
    } else {
        col(index)
    };
    let period = temporal_duration(&options.period)?;
    let offset = match &options.offset {
        Some(offset) => temporal_duration(offset)?,
        None => temporal_duration(&format!("-{}", options.period.trim()))?,
    };
    let aggregations = options
        .aggregations
        .iter()
        .map(|aggregation| {
            polars::sql::sql_expr(aggregation)
                .with_context(|| format!("invalid --agg expression {aggregation:?}"))
        })
        .collect::<anyhow::Result<Vec<Expr>>>()?;
    let group_by = options
        .group_by
        .iter()
        .map(|name| col(name))
        .collect::<Vec<_>>();
    df.lazy()
        .with_column(index_expr.alias(index))
        .rolling(
            col(index),
            group_by,
            RollingGroupOptions {
                index_column: index.into(),
                period,
                offset,
                closed_window: options.closed,
            },
        )
        .agg(aggregations)
        .collect()
        .context("rolling failed (the index column must be sorted ascending; use sort-by first)")
}
