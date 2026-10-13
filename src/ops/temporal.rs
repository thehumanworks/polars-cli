//! Date and time transforms mirroring Nushell's `polars get-*`, `datepart`, `strftime`,
//! `as-date`, `as-datetime`, time-zone, `truncate`, and range commands.
//!
//! String columns are accepted wherever a temporal column is expected and are parsed as
//! ISO-8601, because JSONL pipeline stages carry dates as text.

use anyhow::{Context, bail};
use polars::lazy::dsl::date_range;
use polars::prelude::{
    ClosedWindow, DataFrame, DataType, Duration, Expr, IntoLazy, NonExistent, StrptimeOptions,
    TimeUnit, TimeZone, date_ranges, datetime_range, datetime_ranges, lit,
};

use super::{
    col,
    column::{Target, transform},
    is_schema_less_empty,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatePart {
    Year,
    Quarter,
    Month,
    Week,
    Weekday,
    Day,
    Ordinal,
    Hour,
    Minute,
    Second,
    Millisecond,
    Microsecond,
    Nanosecond,
}

impl DatePart {
    pub fn parse(name: &str) -> anyhow::Result<Self> {
        Ok(match name.trim().to_ascii_lowercase().as_str() {
            "year" | "y" => Self::Year,
            "quarter" | "q" => Self::Quarter,
            "month" | "mo" => Self::Month,
            "week" | "w" => Self::Week,
            "weekday" | "dow" => Self::Weekday,
            "day" | "d" => Self::Day,
            "ordinal" | "doy" => Self::Ordinal,
            "hour" | "h" => Self::Hour,
            "minute" | "m" => Self::Minute,
            "second" | "s" => Self::Second,
            "millisecond" | "ms" => Self::Millisecond,
            "microsecond" | "us" => Self::Microsecond,
            "nanosecond" | "ns" => Self::Nanosecond,
            other => bail!(
                "unknown date part {other:?}; expected year, quarter, month, week, weekday, day, ordinal, hour, minute, second, millisecond, microsecond, or nanosecond"
            ),
        })
    }
}

pub fn date_part(df: DataFrame, target: &Target, part: DatePart) -> anyhow::Result<DataFrame> {
    transform(df, target, "datepart", |source| {
        let expr = source.temporal("datepart")?.dt();
        Ok(match part {
            DatePart::Year => expr.year(),
            DatePart::Quarter => expr.quarter(),
            DatePart::Month => expr.month(),
            DatePart::Week => expr.week(),
            DatePart::Weekday => expr.weekday(),
            DatePart::Day => expr.day(),
            DatePart::Ordinal => expr.ordinal_day(),
            DatePart::Hour => expr.hour(),
            DatePart::Minute => expr.minute(),
            DatePart::Second => expr.second(),
            DatePart::Millisecond => expr.millisecond(),
            DatePart::Microsecond => expr.microsecond(),
            DatePart::Nanosecond => expr.nanosecond(),
        })
    })
}

pub fn strftime(df: DataFrame, target: &Target, format: &str) -> anyhow::Result<DataFrame> {
    transform(df, target, "strftime", |source| {
        Ok(source.temporal("strftime")?.dt().strftime(format))
    })
}

fn strptime_options(format: &str, not_exact: bool) -> StrptimeOptions {
    StrptimeOptions {
        format: Some(format.into()),
        strict: true,
        exact: !not_exact,
        cache: true,
    }
}

pub fn as_date(
    df: DataFrame,
    target: &Target,
    format: &str,
    not_exact: bool,
) -> anyhow::Result<DataFrame> {
    let options = strptime_options(format, not_exact);
    transform(df, target, "as-date", |source| {
        Ok(source.string("as-date")?.str().to_date(options.clone()))
    })
}

#[derive(Debug, Clone)]
pub struct AsDatetime {
    pub format: String,
    pub not_exact: bool,
    pub naive: bool,
    pub time_unit: Option<TimeUnit>,
    pub time_zone: Option<TimeZone>,
    pub ambiguous: String,
}

pub fn as_datetime(
    df: DataFrame,
    target: &Target,
    options: &AsDatetime,
) -> anyhow::Result<DataFrame> {
    if options.naive && options.time_zone.is_some() {
        bail!("as-datetime: --naive and --time-zone cannot be combined");
    }
    let ambiguous = parse_ambiguous(&options.ambiguous)?;
    let strptime = strptime_options(&options.format, options.not_exact);
    transform(df, target, "as-datetime", |source| {
        let parsed = source.string("as-datetime")?.str().to_datetime(
            options.time_unit,
            options.time_zone.clone(),
            strptime.clone(),
            lit(ambiguous),
        );
        Ok(if options.naive {
            parsed
                .dt()
                .replace_time_zone(None, lit("raise"), NonExistent::Raise)
        } else {
            parsed
        })
    })
}

pub fn convert_time_zone(
    df: DataFrame,
    target: &Target,
    time_zone: Option<TimeZone>,
) -> anyhow::Result<DataFrame> {
    transform(df, target, "convert-time-zone", |source| {
        let expr = source.datetime("convert-time-zone")?.dt();
        Ok(match &time_zone {
            Some(zone) => expr.convert_time_zone(zone.clone()),
            None => expr
                .convert_time_zone(TimeZone::UTC)
                .dt()
                .replace_time_zone(None, lit("raise"), NonExistent::Raise),
        })
    })
}

pub fn replace_time_zone(
    df: DataFrame,
    target: &Target,
    time_zone: Option<TimeZone>,
    ambiguous: &str,
    nonexistent: NonExistent,
) -> anyhow::Result<DataFrame> {
    let ambiguous = parse_ambiguous(ambiguous)?;
    transform(df, target, "replace-time-zone", |source| {
        Ok(source
            .datetime("replace-time-zone")?
            .dt()
            .replace_time_zone(time_zone.clone(), lit(ambiguous), nonexistent))
    })
}

pub fn truncate(df: DataFrame, target: &Target, every: &str) -> anyhow::Result<DataFrame> {
    parse_duration(every)?;
    transform(df, target, "truncate", |source| {
        Ok(source.temporal("truncate")?.dt().truncate(lit(every)))
    })
}

fn parse_ambiguous(value: &str) -> anyhow::Result<&'static str> {
    Ok(match value.trim().to_ascii_lowercase().as_str() {
        "raise" => "raise",
        "earliest" => "earliest",
        "latest" => "latest",
        "null" => "null",
        other => bail!(
            "unknown --ambiguous strategy {other:?}; expected raise, earliest, latest, or null"
        ),
    })
}

pub fn parse_nonexistent(value: &str) -> anyhow::Result<NonExistent> {
    Ok(match value.trim().to_ascii_lowercase().as_str() {
        "raise" => NonExistent::Raise,
        "null" => NonExistent::Null,
        other => bail!("unknown --nonexistent strategy {other:?}; expected raise or null"),
    })
}

pub fn parse_duration(value: &str) -> anyhow::Result<Duration> {
    Duration::try_parse(value.trim()).with_context(|| {
        format!("invalid duration {value:?}; use Polars durations such as 1d, 2h, 30m, 1mo, or 1w")
    })
}

pub fn parse_closed(value: &str) -> anyhow::Result<ClosedWindow> {
    Ok(match value.trim().to_ascii_lowercase().as_str() {
        "both" => ClosedWindow::Both,
        "left" => ClosedWindow::Left,
        "right" => ClosedWindow::Right,
        "none" => ClosedWindow::None,
        other => bail!("unknown --closed value {other:?}; expected both, left, right, or none"),
    })
}

/// Bounds and step for the range commands.
#[derive(Debug, Clone)]
pub struct RangeSpec {
    pub start: String,
    pub end: String,
    pub interval: String,
    pub closed: ClosedWindow,
}

#[derive(Debug, Clone, Default)]
pub struct DatetimeSpec {
    pub time_unit: Option<TimeUnit>,
    pub time_zone: Option<TimeZone>,
}

fn inferred_options() -> StrptimeOptions {
    StrptimeOptions {
        format: None,
        strict: true,
        exact: true,
        cache: true,
    }
}

fn date_bound(value: &str, df: Option<&DataFrame>) -> Expr {
    match df {
        Some(df) if df.get_column_index(value).is_some() => col(value).cast(DataType::Date),
        _ => lit(value.trim()).str().to_date(inferred_options()),
    }
}

fn datetime_bound(value: &str, df: Option<&DataFrame>, spec: &DatetimeSpec) -> Expr {
    match df {
        Some(df) if df.get_column_index(value).is_some() => match df.schema().get(value) {
            Some(DataType::String) => {
                col(value)
                    .str()
                    .to_datetime(spec.time_unit, None, inferred_options(), lit("raise"))
            }
            _ => col(value),
        },
        _ => lit(value.trim()).str().to_datetime(
            spec.time_unit,
            None,
            inferred_options(),
            lit("raise"),
        ),
    }
}

/// Generate a one-column frame of dates.
pub fn date_range_frame(spec: &RangeSpec, name: &str) -> anyhow::Result<DataFrame> {
    let interval = parse_duration(&spec.interval)?;
    let expr = date_range(
        Some(date_bound(&spec.start, None)),
        Some(date_bound(&spec.end, None)),
        Some(interval),
        None,
        spec.closed,
    )?;
    DataFrame::empty()
        .lazy()
        .select([expr.alias(name)])
        .collect()
        .context("date-range failed")
}

/// Generate a one-column frame of datetimes.
pub fn datetime_range_frame(
    spec: &RangeSpec,
    datetime: &DatetimeSpec,
    name: &str,
) -> anyhow::Result<DataFrame> {
    let interval = parse_duration(&spec.interval)?;
    let expr = datetime_range(
        Some(datetime_bound(&spec.start, None, datetime)),
        Some(datetime_bound(&spec.end, None, datetime)),
        Some(interval),
        None,
        spec.closed,
        datetime.time_unit,
        datetime.time_zone.clone(),
    )?;
    DataFrame::empty()
        .lazy()
        .select([expr.alias(name)])
        .collect()
        .context("datetime-range failed")
}

/// Add a list column of date ranges whose bounds are columns or literal dates.
pub fn date_ranges_column(
    df: DataFrame,
    spec: &RangeSpec,
    name: &str,
) -> anyhow::Result<DataFrame> {
    if is_schema_less_empty(&df) {
        return Ok(df);
    }
    let interval = parse_duration(&spec.interval)?;
    let expr = date_ranges(
        Some(date_bound(&spec.start, Some(&df))),
        Some(date_bound(&spec.end, Some(&df))),
        Some(interval),
        None,
        spec.closed,
    )?;
    df.lazy()
        .with_column(expr.alias(name))
        .collect()
        .context("date-ranges failed")
}

/// Add a list column of datetime ranges whose bounds are columns or literal datetimes.
pub fn datetime_ranges_column(
    df: DataFrame,
    spec: &RangeSpec,
    datetime: &DatetimeSpec,
    name: &str,
) -> anyhow::Result<DataFrame> {
    if is_schema_less_empty(&df) {
        return Ok(df);
    }
    let interval = parse_duration(&spec.interval)?;
    let start = datetime_bound(&spec.start, Some(&df), datetime);
    let end = datetime_bound(&spec.end, Some(&df), datetime);
    let expr = datetime_ranges(
        Some(start),
        Some(end),
        Some(interval),
        None,
        spec.closed,
        datetime.time_unit,
        datetime.time_zone.clone(),
    )?;
    df.lazy()
        .with_column(expr.alias(name))
        .collect()
        .context("datetime-ranges failed")
}
