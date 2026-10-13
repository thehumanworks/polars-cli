//! Aggregations that reduce a frame to one row, plus index (`arg-*`), `value-counts`, and
//! `summary` commands.

use anyhow::{Context, bail};
use polars::prelude::{
    AnyValue, Column, DataFrame, DataType, Expr, IntoLazy, IntoSeries, NewChunkedArray, Null,
    QuantileMethod, Series, SortMultipleOptions, StringChunked, arg_where as arg_true_of,
    as_struct, len, lit,
};

use super::{col, column::resolve_required, is_schema_less_empty, sql_mask};
use crate::selector;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Aggregation {
    Sum,
    Min,
    Max,
    Mean,
    Median,
    Std(u8),
    Var(u8),
    Quantile(f64, QuantileMethod),
    NUnique,
    Count,
    CountNull,
    Entropy { base: f64, normalize: bool },
    Implode,
    AllTrue,
    AllFalse,
    ArgMax,
    ArgMin,
    BitwiseAnd,
    BitwiseOr,
    BitwiseXor,
}

impl Aggregation {
    fn name(self) -> &'static str {
        match self {
            Self::Sum => "sum",
            Self::Min => "min",
            Self::Max => "max",
            Self::Mean => "mean",
            Self::Median => "median",
            Self::Std(_) => "std",
            Self::Var(_) => "var",
            Self::Quantile(..) => "quantile",
            Self::NUnique => "n-unique",
            Self::Count => "count",
            Self::CountNull => "count-null",
            Self::Entropy { .. } => "entropy",
            Self::Implode => "implode",
            Self::AllTrue => "all-true",
            Self::AllFalse => "all-false",
            Self::ArgMax => "arg-max",
            Self::ArgMin => "arg-min",
            Self::BitwiseAnd => "math bitwise-and",
            Self::BitwiseOr => "math bitwise-or",
            Self::BitwiseXor => "math bitwise-xor",
        }
    }

    /// Whether the aggregation is defined for a column of this type.
    fn supports(self, dtype: &DataType) -> bool {
        let numeric = dtype.is_numeric() || dtype.is_bool() || dtype.is_null();
        match self {
            Self::Sum
            | Self::Mean
            | Self::Median
            | Self::Std(_)
            | Self::Var(_)
            | Self::Quantile(..)
            | Self::Entropy { .. } => numeric,
            Self::Min | Self::Max | Self::ArgMax | Self::ArgMin => {
                numeric || dtype.is_string() || dtype.is_temporal()
            }
            Self::AllTrue | Self::AllFalse => dtype.is_bool() || dtype.is_null(),
            Self::BitwiseAnd | Self::BitwiseOr | Self::BitwiseXor => {
                dtype.is_integer() || dtype.is_bool()
            }
            Self::NUnique | Self::Count | Self::CountNull | Self::Implode => true,
        }
    }

    fn expr(self, column: Expr) -> Expr {
        match self {
            Self::Sum => column.sum(),
            Self::Min => column.min(),
            Self::Max => column.max(),
            Self::Mean => column.mean(),
            Self::Median => column.median(),
            Self::Std(ddof) => column.std(ddof),
            Self::Var(ddof) => column.var(ddof),
            Self::Quantile(quantile, method) => column.quantile(lit(quantile), method),
            Self::NUnique => column.n_unique(),
            Self::Count => column.count(),
            Self::CountNull => column.null_count(),
            Self::Entropy { base, normalize } => column.entropy(base, normalize),
            Self::Implode => column.implode(),
            Self::AllTrue => column.all(true),
            Self::AllFalse => column.not().all(true),
            Self::ArgMax => column.arg_max(),
            Self::ArgMin => column.arg_min(),
            Self::BitwiseAnd => column.bitwise_and(),
            Self::BitwiseOr => column.bitwise_or(),
            Self::BitwiseXor => column.bitwise_xor(),
        }
    }
}

/// Reduce each selected column (default: every column) to one value, producing one row.
///
/// Explicitly named columns must support the aggregation. When every column is implied,
/// unsupported columns produce null, matching Polars' `DataFrame.sum()` and friends.
pub fn aggregate(
    df: DataFrame,
    columns: &[String],
    aggregation: Aggregation,
) -> anyhow::Result<DataFrame> {
    if is_schema_less_empty(&df) {
        return Ok(df);
    }
    if let Aggregation::Quantile(quantile, _) = aggregation
        && !(0.0..=1.0).contains(&quantile)
    {
        bail!("quantile must be between 0 and 1");
    }
    let command = aggregation.name();
    let explicit = !columns.is_empty();
    let names = resolve_required(&df, columns, command)?;
    let exprs = names
        .iter()
        .map(|name| {
            let dtype = df.schema().get(name.as_str()).cloned().unwrap_or_default();
            if aggregation.supports(&dtype) {
                Ok(aggregation.expr(col(name)).alias(name.as_str()))
            } else if explicit {
                bail!("{command} does not support {name:?} ({dtype})")
            } else {
                Ok(lit(Null {}).cast(dtype).alias(name.as_str()))
            }
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    df.lazy()
        .select(exprs)
        .collect()
        .with_context(|| format!("{command} failed"))
}

/// The dot product of two numeric columns, as a one-row `dot` column.
pub fn dot(df: DataFrame, left: &str, right: &str) -> anyhow::Result<DataFrame> {
    if is_schema_less_empty(&df) {
        return Ok(df);
    }
    for name in [left, right] {
        let dtype = df
            .schema()
            .get(name)
            .with_context(|| format!("column {name:?} does not exist"))?;
        if !dtype.is_numeric() {
            bail!("math dot requires numeric columns, but {name:?} is {dtype}");
        }
    }
    df.lazy()
        .select([col(left).dot(col(right)).alias("dot")])
        .collect()
        .context("math dot failed")
}

/// Count rows, as Nushell's `polars len`.
pub fn len_frame(df: &DataFrame) -> anyhow::Result<DataFrame> {
    DataFrame::new_infer_height(vec![Column::new("len".into(), [df.height() as u64])])
        .map_err(Into::into)
}

/// Count each distinct value (or row, for several columns) in first-appearance order.
pub fn value_counts(
    df: DataFrame,
    columns: &[String],
    count_name: Option<&str>,
    sort: bool,
    normalize: bool,
) -> anyhow::Result<DataFrame> {
    if is_schema_less_empty(&df) {
        return Ok(df);
    }
    let names = resolve_required(&df, columns, "value-counts")?;
    let count_name = count_name.unwrap_or(if normalize { "proportion" } else { "count" });
    let height = df.height();
    if names.iter().any(|name| name == count_name) {
        bail!(
            "value-counts output column {count_name:?} collides with a key; use --column to rename it"
        );
    }
    let mut frame = df
        .lazy()
        .group_by_stable(names.iter().map(|name| col(name)).collect::<Vec<_>>())
        .agg([len().cast(DataType::UInt64).alias(count_name)]);
    if sort {
        frame = frame.sort(
            [count_name],
            SortMultipleOptions::default()
                .with_order_descending(true)
                .with_maintain_order(true),
        );
    }
    let mut counts = frame.collect().context("value-counts failed")?;
    if normalize {
        // Divide in Rust: Polars divides by a scalar via its reciprocal, so 3/5 would print
        // as 0.6000000000000001.
        let total = height.max(1) as f64;
        let proportions = counts
            .column(count_name)?
            .u64()?
            .into_iter()
            .map(|count| count.map(|count| count as f64 / total))
            .collect::<Vec<_>>();
        counts.with_column(Column::new(count_name.into(), proportions))?;
    }
    Ok(counts)
}

/// Descriptive statistics in Nushell's `polars summary` layout.
pub fn summary(df: &DataFrame, quantiles: &[f64]) -> anyhow::Result<DataFrame> {
    if quantiles
        .iter()
        .any(|quantile| !(0.0..=1.0).contains(quantile))
    {
        bail!("summary quantiles must be between 0 and 1");
    }
    let mut labels = vec!["count", "sum", "mean", "median", "std", "min"]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    labels.extend(
        quantiles
            .iter()
            .map(|quantile| format!("{}%", quantile * 100.0)),
    );
    labels.push("max".to_owned());

    let mut columns = vec![Column::new(
        "descriptor".into(),
        StringChunked::from_slice("descriptor".into(), &labels).into_series(),
    )];
    for column in df.columns() {
        let series = column.as_materialized_series();
        let numeric = series.dtype().is_numeric() || series.dtype().is_bool();
        let stat = |value: Option<f64>| if numeric { value } else { None };
        let mut values = vec![
            Some(series.len() as f64),
            stat(series.sum::<f64>().ok()),
            stat(series.mean()),
            stat(series.median()),
            stat(series.std(0)),
            stat(series.min::<f64>().ok().flatten()),
        ];
        for quantile in quantiles {
            values.push(stat(quantile_value(series, *quantile)));
        }
        values.push(stat(series.max::<f64>().ok().flatten()));
        let name = format!("{} ({})", series.name(), series.dtype());
        columns.push(Column::new(name.into(), values));
    }
    DataFrame::new_infer_height(columns).map_err(Into::into)
}

fn quantile_value(series: &Series, quantile: f64) -> Option<f64> {
    let scalar = series
        .quantile_reduce(quantile, QuantileMethod::default())
        .ok()?;
    match scalar.value() {
        AnyValue::Null => None,
        value => value.extract::<f64>(),
    }
}

/// Row indices where a boolean column is true.
pub fn arg_true(df: DataFrame, columns: &[String]) -> anyhow::Result<DataFrame> {
    if is_schema_less_empty(&df) {
        return Ok(df);
    }
    let name = single_column(&df, columns, "arg-true")?;
    if !df.schema().get(&name).is_some_and(DataType::is_bool) {
        bail!("arg-true requires a boolean column, but {name:?} is not boolean");
    }
    df.lazy()
        .select([arg_true_of(col(&name)).alias("arg_true")])
        .collect()
        .context("arg-true failed")
}

/// Row indices where a SQL predicate is true.
pub fn arg_where(df: DataFrame, predicate: &str) -> anyhow::Result<DataFrame> {
    if is_schema_less_empty(&df) {
        return Ok(df);
    }
    let mask = sql_mask(&df, predicate)?;
    DataFrame::new_infer_height(vec![Column::new("mask".into(), mask.into_series())])?
        .lazy()
        .select([arg_true_of(col("mask")).alias("arg_where")])
        .collect()
        .context("arg-where failed")
}

/// Index of the first occurrence of each distinct value (or row, for several columns).
pub fn arg_unique(df: DataFrame, columns: &[String]) -> anyhow::Result<DataFrame> {
    if is_schema_less_empty(&df) {
        return Ok(df);
    }
    let names = resolve_required(&df, columns, "arg-unique")?;
    let key = row_key(&names);
    df.lazy()
        .select([key.arg_unique().alias("arg_unique")])
        .collect()
        .context("arg-unique failed")
}

/// Row indices that would sort the frame by the selected columns (stable).
pub fn arg_sort(
    df: DataFrame,
    columns: &[String],
    descending: bool,
    nulls_last: bool,
    limit: Option<usize>,
) -> anyhow::Result<DataFrame> {
    if is_schema_less_empty(&df) {
        return Ok(df);
    }
    let names = resolve_required(&df, columns, "arg-sort")?;
    let index = "__pl_arg_sort";
    let mut sorted = df
        .lazy()
        .with_row_index(index, None)
        .sort(
            names.iter().map(String::as_str).collect::<Vec<_>>(),
            SortMultipleOptions::default()
                .with_order_descending(descending)
                .with_nulls_last(nulls_last)
                .with_maintain_order(true),
        )
        .select([col(index).alias("arg_sort")]);
    // A limit beyond the index range cannot cut anything off.
    if let Some(limit) = limit.and_then(|limit| u32::try_from(limit).ok()) {
        sorted = sorted.limit(limit);
    }
    sorted.collect().context("arg-sort failed")
}

/// One expression identifying a row across columns: the column itself, or a struct of several.
pub(crate) fn row_key(names: &[String]) -> Expr {
    match names {
        [name] => col(name),
        _ => as_struct(names.iter().map(|name| col(name)).collect()),
    }
}

/// Resolve selectors to exactly one column, defaulting to the only column of the frame.
pub(crate) fn single_column(
    df: &DataFrame,
    columns: &[String],
    command: &str,
) -> anyhow::Result<String> {
    let names = selector::resolve_or_all(df.schema(), columns)?;
    match names.as_slice() {
        [name] => Ok(name.clone()),
        [] => bail!("{command}: the column selection matched no columns"),
        _ => bail!(
            "{command} needs exactly one column; name it explicitly (got {})",
            names.join(", ")
        ),
    }
}
