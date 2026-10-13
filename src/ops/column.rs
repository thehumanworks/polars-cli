//! Column ("series") transforms.
//!
//! Every transform resolves its COLUMN selectors (defaulting to every column), evaluates one
//! expression per selected column, and replaces each source column in place, so frame shape and
//! column order are preserved. `--as NAME` writes the result to NAME instead, keeping the source
//! column; a `{}` in NAME expands to each source column name.
//!
//! On a one-column frame this is exactly Nushell's series semantics.

use anyhow::{Context, bail};
use polars::lazy::dsl::{max_horizontal, mean_horizontal, min_horizontal, sum_horizontal};
use polars::prelude::{
    BooleanChunked, ChunkedArray, DataFrame, DataType, Expr, FillNullStrategy, IntoLazy,
    IntoSeries, NewChunkedArray, RollingOptionsFixedWindow, StrptimeOptions, all_horizontal,
    any_horizontal, concat_str, lit, when,
};
use serde_json::Value as JsonValue;

use super::{col, is_schema_less_empty};
use crate::{
    selector,
    value::{cast_expr, list_series, scalar_expr},
};

/// Largest offset magnitude passed to Polars. Frames and strings are far shorter, so clamping
/// to it never changes a result, while i64::MIN would overflow when Polars negates it.
pub(crate) const MAX_OFFSET: i64 = 1 << 48;

/// Which columns a transform reads and where it writes.
#[derive(Debug, Clone, Default)]
pub struct Target {
    pub columns: Vec<String>,
    pub alias: Option<String>,
}

impl Target {
    pub fn new(columns: Vec<String>, alias: Option<String>) -> Self {
        Self { columns, alias }
    }
}

/// A resolved source column handed to a transform.
pub struct Source<'a> {
    pub df: &'a DataFrame,
    pub name: &'a str,
    pub dtype: &'a DataType,
}

impl Source<'_> {
    pub fn expr(&self) -> Expr {
        col(self.name)
    }

    /// The column as a string expression; categorical and all-null columns are cast to strings.
    pub fn string(&self, command: &str) -> anyhow::Result<Expr> {
        match self.dtype {
            DataType::String => Ok(self.expr()),
            DataType::Null | DataType::Categorical(..) | DataType::Enum(..) => {
                Ok(self.expr().cast(DataType::String))
            }
            other => bail!(
                "{command} requires a string column, but {:?} is {other}",
                self.name
            ),
        }
    }

    pub fn numeric(&self, command: &str) -> anyhow::Result<Expr> {
        if self.dtype.is_numeric() || self.dtype.is_bool() {
            Ok(self.expr())
        } else if self.dtype.is_null() {
            Ok(self.expr().cast(DataType::Float64))
        } else {
            bail!(
                "{command} requires a numeric column, but {:?} is {}",
                self.name,
                self.dtype
            )
        }
    }

    pub fn integer(&self, command: &str) -> anyhow::Result<Expr> {
        if self.dtype.is_integer() {
            Ok(self.expr())
        } else if self.dtype.is_null() {
            Ok(self.expr().cast(DataType::Int64))
        } else {
            bail!(
                "{command} requires an integer column, but {:?} is {}",
                self.name,
                self.dtype
            )
        }
    }

    pub fn boolean(&self, command: &str) -> anyhow::Result<Expr> {
        match self.dtype {
            DataType::Boolean => Ok(self.expr()),
            DataType::Null => Ok(self.expr().cast(DataType::Boolean)),
            other => bail!(
                "{command} requires a boolean column, but {:?} is {other}",
                self.name
            ),
        }
    }

    /// The column as a temporal expression.
    ///
    /// JSON and CSV pipeline stages carry dates as ISO-8601 text, so string columns are parsed:
    /// date-only values become `date`, anything else becomes `datetime`.
    pub fn temporal(&self, command: &str) -> anyhow::Result<Expr> {
        match self.dtype {
            DataType::Date | DataType::Datetime(_, _) | DataType::Time | DataType::Duration(_) => {
                Ok(self.expr())
            }
            // An all-null JSON column has no type; treat it as datetimes so results are null.
            DataType::Null => Ok(self.expr().cast(DataType::Datetime(
                polars::prelude::TimeUnit::Microseconds,
                None,
            ))),
            DataType::String => {
                let sample = self
                    .df
                    .column(self.name)?
                    .as_materialized_series()
                    .str()?
                    .iter()
                    .flatten()
                    .next()
                    .map(str::to_owned);
                let options = StrptimeOptions {
                    format: None,
                    strict: true,
                    exact: true,
                    cache: true,
                };
                Ok(if sample.as_deref().is_some_and(is_iso_date) {
                    self.expr().str().to_date(options)
                } else {
                    self.expr()
                        .str()
                        .to_datetime(None, None, options, lit("raise"))
                })
            }
            other => bail!(
                "{command} requires a date, datetime, or ISO-8601 string column, but {:?} is {other}",
                self.name
            ),
        }
    }

    /// The column as a datetime expression; dates and strings are converted.
    pub fn datetime(&self, command: &str) -> anyhow::Result<Expr> {
        let expr = self.temporal(command)?;
        Ok(match self.dtype {
            DataType::Datetime(_, _) | DataType::Null => expr,
            DataType::Date => expr.cast(DataType::Datetime(
                polars::prelude::TimeUnit::Microseconds,
                None,
            )),
            DataType::String => {
                let options = StrptimeOptions {
                    format: None,
                    strict: true,
                    exact: true,
                    cache: true,
                };
                self.expr()
                    .str()
                    .to_datetime(None, None, options, lit("raise"))
            }
            other => bail!(
                "{command} requires a datetime column, but {:?} is {other}",
                self.name
            ),
        })
    }
}

fn is_iso_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit())
}

/// Apply `transform` to every selected column.
pub(crate) fn transform<F>(
    df: DataFrame,
    target: &Target,
    command: &str,
    transform: F,
) -> anyhow::Result<DataFrame>
where
    F: Fn(&Source<'_>) -> anyhow::Result<Expr>,
{
    if is_schema_less_empty(&df) {
        return Ok(df);
    }
    let schema = df.schema().clone();
    let names = selector::resolve_or_all(&schema, &target.columns)?;
    if names.is_empty() {
        bail!("{command}: the column selection matched no columns");
    }
    let outputs = output_names(&names, target.alias.as_deref(), command)?;
    let exprs = names
        .iter()
        .zip(&outputs)
        .map(|(name, output)| {
            let dtype = schema
                .get(name.as_str())
                .with_context(|| format!("column {name:?} does not exist"))?;
            let source = Source {
                df: &df,
                name,
                dtype,
            };
            Ok(transform(&source)?.alias(output.as_str()))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    df.lazy()
        .with_columns(exprs)
        .collect()
        .with_context(|| format!("{command} failed"))
}

/// Output names for transformed columns: in place, a single alias, or a `{}` template.
pub(crate) fn output_names(
    names: &[String],
    alias: Option<&str>,
    command: &str,
) -> anyhow::Result<Vec<String>> {
    match alias {
        None => Ok(names.to_vec()),
        Some(alias) if alias.contains("{}") => {
            Ok(names.iter().map(|name| alias.replace("{}", name)).collect())
        }
        Some(alias) if names.len() == 1 => Ok(vec![alias.to_owned()]),
        Some(_) => bail!(
            "{command}: --as needs exactly one column, or a {{}} placeholder such as '{{}}_out' for several"
        ),
    }
}

/// Resolve explicit selectors to at least one column.
pub(crate) fn resolve_required(
    df: &DataFrame,
    columns: &[String],
    command: &str,
) -> anyhow::Result<Vec<String>> {
    let names = selector::resolve_or_all(df.schema(), columns)?;
    if names.is_empty() {
        bail!("{command}: the column selection matched no columns");
    }
    Ok(names)
}

pub fn cast(
    df: DataFrame,
    target: &Target,
    dtype: &DataType,
    strict: bool,
) -> anyhow::Result<DataFrame> {
    transform(df, target, "cast", |source| {
        Ok(cast_expr(source.expr(), source.dtype, dtype, strict))
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillStrategy {
    Forward,
    Backward,
    Min,
    Max,
    Mean,
    Zero,
    One,
}

pub fn fill_null(
    df: DataFrame,
    target: &Target,
    value: Option<&JsonValue>,
    strategy: Option<FillStrategy>,
    limit: Option<u32>,
) -> anyhow::Result<DataFrame> {
    let fill = match (value, strategy) {
        (Some(value), None) => Fill::Value(scalar_expr(value)?, value.clone()),
        (None, Some(strategy)) => Fill::Strategy(match strategy {
            FillStrategy::Forward => FillNullStrategy::Forward(limit),
            FillStrategy::Backward => FillNullStrategy::Backward(limit),
            FillStrategy::Min => FillNullStrategy::Min,
            FillStrategy::Max => FillNullStrategy::Max,
            FillStrategy::Mean => FillNullStrategy::Mean,
            FillStrategy::Zero => FillNullStrategy::Zero,
            FillStrategy::One => FillNullStrategy::One,
        }),
        (Some(_), Some(_)) => bail!("fill-null accepts a VALUE or --strategy, not both"),
        (None, None) => bail!("fill-null requires a VALUE or --strategy"),
    };
    if limit.is_some()
        && !matches!(
            strategy,
            Some(FillStrategy::Forward | FillStrategy::Backward)
        )
    {
        bail!("--limit only applies to --strategy forward or backward");
    }
    let numeric_only = matches!(
        strategy,
        Some(FillStrategy::Mean | FillStrategy::Zero | FillStrategy::One)
    );
    let Some(target) = implicit_target(&df, target, |dtype| match &fill {
        Fill::Value(_, value) => value_fits_dtype(value, dtype),
        Fill::Strategy(_) => !numeric_only || dtype.is_numeric(),
    })?
    else {
        return Ok(df);
    };
    transform(df, &target, "fill-null", |source| {
        Ok(match &fill {
            Fill::Value(expr, _) => source.expr().fill_null(expr.clone()),
            Fill::Strategy(strategy) => source.expr().fill_null_with_strategy(*strategy),
        })
    })
}

enum Fill {
    Value(Expr, JsonValue),
    Strategy(FillNullStrategy),
}

/// When no COLUMN is given, restrict an implicit "every column" selection to compatible dtypes,
/// so `pl fill-null 0` fills numeric columns and leaves text alone, as Polars does.
/// Returns `None` when nothing is compatible, which makes the command a no-op.
fn implicit_target(
    df: &DataFrame,
    target: &Target,
    compatible: impl Fn(&DataType) -> bool,
) -> anyhow::Result<Option<Target>> {
    if !target.columns.is_empty() || is_schema_less_empty(df) {
        return Ok(Some(target.clone()));
    }
    if target.alias.is_some() && df.width() != 1 {
        bail!("--as needs exactly one column; name it explicitly");
    }
    let columns = df
        .schema()
        .iter()
        .filter(|(_, dtype)| compatible(dtype))
        .map(|(name, _)| name.to_string())
        .collect::<Vec<_>>();
    Ok((!columns.is_empty()).then(|| Target {
        columns,
        alias: target.alias.clone(),
    }))
}

fn value_fits_dtype(value: &JsonValue, dtype: &DataType) -> bool {
    match value {
        JsonValue::Null => false,
        JsonValue::Bool(_) => dtype.is_bool(),
        JsonValue::Number(_) => dtype.is_numeric(),
        JsonValue::String(_) => {
            dtype.is_string() || dtype.is_categorical() || dtype.is_temporal() || dtype.is_null()
        }
        _ => false,
    }
}

pub fn fill_nan(df: DataFrame, target: &Target, value: &JsonValue) -> anyhow::Result<DataFrame> {
    let fill = scalar_expr(value)?;
    let Some(target) = implicit_target(&df, target, DataType::is_float)? else {
        return Ok(df);
    };
    transform(df, &target, "fill-nan", |source| {
        if !source.dtype.is_float() {
            bail!(
                "fill-nan requires a float column, but {:?} is {}",
                source.name,
                source.dtype
            );
        }
        Ok(source.expr().fill_nan(fill.clone()))
    })
}

pub fn shift(
    df: DataFrame,
    target: &Target,
    periods: i64,
    fill: Option<&JsonValue>,
) -> anyhow::Result<DataFrame> {
    let fill = fill.map(scalar_expr).transpose()?;
    // Any shift at least as long as the frame empties it; clamping keeps Polars from
    // overflowing when it negates i64::MIN.
    let periods = periods.clamp(-MAX_OFFSET, MAX_OFFSET);
    transform(df, target, "shift", |source| {
        Ok(match &fill {
            Some(fill) => source.expr().shift_and_fill(lit(periods), fill.clone()),
            None => source.expr().shift(lit(periods)),
        })
    })
}

pub fn is_null(df: DataFrame, target: &Target, negate: bool) -> anyhow::Result<DataFrame> {
    let command = if negate { "is-not-null" } else { "is-null" };
    transform(df, target, command, |source| {
        Ok(if negate {
            source.expr().is_not_null()
        } else {
            source.expr().is_null()
        })
    })
}

pub fn not(df: DataFrame, target: &Target) -> anyhow::Result<DataFrame> {
    transform(df, target, "not", |source| Ok(source.boolean("not")?.not()))
}

pub fn is_in(
    df: DataFrame,
    target: &Target,
    values: &[JsonValue],
    nulls_equal: bool,
) -> anyhow::Result<DataFrame> {
    let values = list_series("values", values)?;
    transform(df, target, "is-in", |source| {
        if source.dtype.is_null() {
            let column = source.expr().cast(values.dtype().clone());
            return Ok(column.is_in(lit(values.clone()).implode(), nulls_equal));
        }
        let values = values.strict_cast(source.dtype).with_context(|| {
            format!(
                "is-in values cannot be compared with {:?} ({})",
                source.name, source.dtype
            )
        })?;
        Ok(source.expr().is_in(lit(values).implode(), nulls_equal))
    })
}

#[derive(Debug, Clone)]
pub struct ReplaceOptions {
    pub old: Vec<JsonValue>,
    pub new: Vec<JsonValue>,
    pub strict: bool,
    pub default: Option<JsonValue>,
    pub return_dtype: Option<DataType>,
}

pub fn replace(
    df: DataFrame,
    target: &Target,
    options: &ReplaceOptions,
) -> anyhow::Result<DataFrame> {
    if options.old.is_empty() {
        bail!("replace requires at least one value to replace");
    }
    let new = match options.new.len() {
        1 if options.old.len() > 1 => vec![options.new[0].clone(); options.old.len()],
        length if length == options.old.len() => options.new.clone(),
        length => bail!(
            "replace received {} old value(s) but {length} new value(s)",
            options.old.len()
        ),
    };
    let old = list_series("old", &options.old)?;
    let new = list_series("new", &new)?;
    let default = options.default.as_ref().map(scalar_expr).transpose()?;
    transform(df, target, "replace", |source| {
        let (column, dtype) = if source.dtype.is_null() {
            (source.expr().cast(old.dtype().clone()), old.dtype().clone())
        } else {
            (source.expr(), source.dtype.clone())
        };
        let old = old.strict_cast(&dtype).with_context(|| {
            format!(
                "replace values cannot be compared with {:?} ({})",
                source.name, source.dtype
            )
        })?;
        let new = lit(new.clone());
        let old = lit(old);
        Ok(
            if options.strict || default.is_some() || options.return_dtype.is_some() {
                column.replace_strict(
                    old,
                    new,
                    default.clone(),
                    options
                        .return_dtype
                        .clone()
                        .map(polars::prelude::DataTypeExpr::from),
                )
            } else {
                column.replace(old, new)
            },
        )
    })
}

#[derive(Debug, Clone, Default)]
pub struct BinOptions {
    pub labels: Option<Vec<String>>,
    pub left_closed: bool,
    pub include_breaks: bool,
}

pub fn cut(
    df: DataFrame,
    target: &Target,
    breaks: Vec<f64>,
    options: &BinOptions,
) -> anyhow::Result<DataFrame> {
    if breaks.is_empty() {
        bail!("cut requires at least one break point");
    }
    check_labels(options, breaks.len() + 1, "cut")?;
    transform(df, target, "cut", |source| {
        Ok(source.numeric("cut")?.cut(
            breaks.clone(),
            options.labels.clone(),
            options.left_closed,
            options.include_breaks,
        ))
    })
}

/// Upper bound on uniform `qcut` bins; Polars materializes one probability per bin.
const MAX_QCUT_BINS: usize = 10_000;

#[derive(Debug, Clone)]
pub enum Quantiles {
    Bins(usize),
    Probabilities(Vec<f64>),
}

pub fn qcut(
    df: DataFrame,
    target: &Target,
    quantiles: &Quantiles,
    options: &BinOptions,
    allow_duplicates: bool,
) -> anyhow::Result<DataFrame> {
    let bins = match quantiles {
        Quantiles::Bins(0) => bail!("qcut requires at least one bin"),
        Quantiles::Bins(bins) if *bins > MAX_QCUT_BINS => {
            bail!("qcut supports at most {MAX_QCUT_BINS} bins")
        }
        Quantiles::Bins(bins) => *bins,
        Quantiles::Probabilities(probabilities) => {
            if probabilities
                .iter()
                .any(|value| !(0.0..=1.0).contains(value))
            {
                bail!("qcut quantiles must be between 0 and 1");
            }
            probabilities.len() + 1
        }
    };
    check_labels(options, bins, "qcut")?;
    transform(df, target, "qcut", |source| {
        let expr = source.numeric("qcut")?;
        Ok(match quantiles {
            Quantiles::Bins(bins) => expr.qcut_uniform(
                *bins,
                options.labels.clone(),
                options.left_closed,
                allow_duplicates,
                options.include_breaks,
            ),
            Quantiles::Probabilities(probabilities) => expr.qcut(
                probabilities.clone(),
                options.labels.clone(),
                options.left_closed,
                allow_duplicates,
                options.include_breaks,
            ),
        })
    })
}

fn check_labels(options: &BinOptions, bins: usize, command: &str) -> anyhow::Result<()> {
    if let Some(labels) = &options.labels
        && labels.len() != bins
    {
        bail!(
            "{command} produces {bins} bin(s) but received {} label(s)",
            labels.len()
        );
    }
    Ok(())
}

/// Set VALUE wherever `mask` is true.
pub fn set_where(
    df: DataFrame,
    target: &Target,
    value: &JsonValue,
    mask: BooleanChunked,
    command: &str,
) -> anyhow::Result<DataFrame> {
    let value = scalar_expr(value)?;
    let mask = mask.into_series();
    transform(df, target, command, |source| {
        Ok(when(lit(mask.clone()))
            .then(value.clone())
            .otherwise(source.expr()))
    })
}

/// A boolean mask that is true at the given row indices; negative indices count from the end.
pub fn index_mask(height: usize, indices: &[i64]) -> anyhow::Result<BooleanChunked> {
    let mut mask = vec![false; height];
    for index in indices {
        let position = if *index < 0 {
            height as i64 + index
        } else {
            *index
        };
        if position < 0 || position >= height as i64 {
            bail!("row index {index} is out of range for {height} row(s)");
        }
        mask[position as usize] = true;
    }
    Ok(ChunkedArray::from_slice("mask".into(), &mask))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cumulative {
    Sum,
    Min,
    Max,
    Count,
    Product,
}

pub fn cumulative(
    df: DataFrame,
    target: &Target,
    kind: Cumulative,
    reverse: bool,
) -> anyhow::Result<DataFrame> {
    transform(df, target, "cumulative", |source| {
        let expr = source.expr();
        Ok(match kind {
            Cumulative::Sum => source.numeric("cumulative sum")?.cum_sum(reverse),
            Cumulative::Min => expr.cum_min(reverse),
            Cumulative::Max => expr.cum_max(reverse),
            Cumulative::Count => expr.cum_count(reverse),
            Cumulative::Product => source.numeric("cumulative product")?.cum_prod(reverse),
        })
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rolling {
    Sum,
    Min,
    Max,
    Mean,
    Median,
    Std,
    Var,
}

pub fn rolling(
    df: DataFrame,
    target: &Target,
    kind: Rolling,
    window: usize,
    min_periods: Option<usize>,
    center: bool,
) -> anyhow::Result<DataFrame> {
    if window == 0 {
        bail!("rolling window must be at least 1");
    }
    let options = RollingOptionsFixedWindow {
        window_size: window,
        min_periods: min_periods.unwrap_or(window),
        weights: None,
        center,
        fn_params: None,
    };
    transform(df, target, "rolling", |source| {
        let expr = source.numeric("rolling")?;
        let options = options.clone();
        Ok(match kind {
            Rolling::Sum => expr.rolling_sum(options),
            Rolling::Min => expr.rolling_min(options),
            Rolling::Max => expr.rolling_max(options),
            Rolling::Mean => expr.rolling_mean(options),
            Rolling::Median => expr.rolling_median(options),
            Rolling::Std => expr.rolling_std(options),
            Rolling::Var => expr.rolling_var(options),
        })
    })
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MathOp {
    Abs,
    Cos,
    Sin,
    Exp,
    Log(Option<f64>),
    Log1p,
    Sign,
    Sqrt,
    CountOnes,
    CountZeros,
    LeadingOnes,
    LeadingZeros,
    TrailingOnes,
    TrailingZeros,
}

pub fn math(df: DataFrame, target: &Target, op: MathOp) -> anyhow::Result<DataFrame> {
    transform(df, target, "math", |source| {
        Ok(match op {
            MathOp::Abs => source.numeric("math abs")?.abs(),
            MathOp::Cos => source.numeric("math cos")?.cos(),
            MathOp::Sin => source.numeric("math sin")?.sin(),
            MathOp::Exp => source.numeric("math exp")?.exp(),
            MathOp::Log(base) => source
                .numeric("math log")?
                .log(lit(base.unwrap_or(std::f64::consts::E))),
            MathOp::Log1p => source.numeric("math log1p")?.log1p(),
            MathOp::Sign => source.numeric("math sign")?.sign(),
            MathOp::Sqrt => source.numeric("math sqrt")?.sqrt(),
            MathOp::CountOnes => source
                .integer("math bitwise-count-ones")?
                .bitwise_count_ones(),
            MathOp::CountZeros => source
                .integer("math bitwise-count-zeros")?
                .bitwise_count_zeros(),
            MathOp::LeadingOnes => source
                .integer("math bitwise-leading-ones")?
                .bitwise_leading_ones(),
            MathOp::LeadingZeros => source
                .integer("math bitwise-leading-zeros")?
                .bitwise_leading_zeros(),
            MathOp::TrailingOnes => source
                .integer("math bitwise-trailing-ones")?
                .bitwise_trailing_ones(),
            MathOp::TrailingZeros => source
                .integer("math bitwise-trailing-zeros")?
                .bitwise_trailing_zeros(),
        })
    })
}

pub fn integer(
    df: DataFrame,
    target: &Target,
    base: u32,
    dtype: Option<DataType>,
    strict: bool,
) -> anyhow::Result<DataFrame> {
    if !(2..=36).contains(&base) {
        bail!("integer base must be between 2 and 36");
    }
    if let Some(dtype) = &dtype
        && !dtype.is_integer()
    {
        bail!("integer --dtype must be an integer type, got {dtype}");
    }
    transform(df, target, "integer", |source| {
        Ok(source
            .string("integer")?
            .str()
            .to_integer(lit(base), dtype.clone(), strict))
    })
}

pub fn decimal(df: DataFrame, target: &Target, scale: usize) -> anyhow::Result<DataFrame> {
    if scale > 38 {
        bail!("decimal scale must be at most 38");
    }
    transform(df, target, "decimal", |source| {
        Ok(source.string("decimal")?.str().to_decimal(scale))
    })
}

pub fn struct_json_encode(df: DataFrame, target: &Target) -> anyhow::Result<DataFrame> {
    transform(df, target, "struct-json-encode", |source| {
        if !matches!(source.dtype, DataType::Struct(_)) {
            bail!(
                "struct-json-encode requires a struct column, but {:?} is {}",
                source.name,
                source.dtype
            );
        }
        Ok(source.expr().struct_().json_encode())
    })
}

pub fn list_contains(
    df: DataFrame,
    target: &Target,
    element: &JsonValue,
) -> anyhow::Result<DataFrame> {
    let element = scalar_expr(element)?;
    transform(df, target, "list-contains", |source| {
        if !matches!(source.dtype, DataType::List(_)) {
            bail!(
                "list-contains requires a list column, but {:?} is {}",
                source.name,
                source.dtype
            );
        }
        Ok(source.expr().list().contains(element.clone(), true))
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Horizontal {
    All,
    Any,
    Min,
    Max,
    Sum,
    Mean,
}

/// Compute a row-wise reduction across columns and add it as NAME.
pub fn horizontal(
    df: DataFrame,
    columns: &[String],
    op: Horizontal,
    propagate_nulls: bool,
    name: &str,
) -> anyhow::Result<DataFrame> {
    if is_schema_less_empty(&df) {
        return Ok(df);
    }
    let names = resolve_required(&df, columns, "horizontal")?;
    let exprs = names.iter().map(|name| col(name)).collect::<Vec<_>>();
    let ignore_nulls = !propagate_nulls;
    let expr = match op {
        Horizontal::All => all_horizontal(exprs)?,
        Horizontal::Any => any_horizontal(exprs)?,
        Horizontal::Min => min_horizontal(exprs)?,
        Horizontal::Max => max_horizontal(exprs)?,
        Horizontal::Sum => sum_horizontal(exprs, ignore_nulls)?,
        Horizontal::Mean => mean_horizontal(exprs, ignore_nulls)?,
    };
    df.lazy()
        .with_column(expr.alias(name))
        .collect()
        .context("horizontal failed")
}

/// Concatenate the string forms of columns row-wise and add the result as NAME.
pub fn concat_str_columns(
    df: DataFrame,
    columns: &[String],
    separator: &str,
    ignore_nulls: bool,
    name: &str,
) -> anyhow::Result<DataFrame> {
    if is_schema_less_empty(&df) {
        return Ok(df);
    }
    let names = resolve_required(&df, columns, "concat-str")?;
    let exprs = names
        .iter()
        .map(|name| col(name).cast(DataType::String))
        .collect::<Vec<_>>();
    df.lazy()
        .with_column(concat_str(exprs, separator, ignore_nulls).alias(name))
        .collect()
        .context("concat-str failed")
}
