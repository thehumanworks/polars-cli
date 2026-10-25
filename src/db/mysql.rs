//! MySQL and MariaDB: binary-protocol result values decoded into Polars types.

use std::str::FromStr;

use anyhow::{Context, bail, ensure};
use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime};
use futures_util::TryStreamExt;
use polars::prelude::{AnyValue, DataFrame, DataType, TimeZone};
use sqlx::mysql::{MySql, MySqlConnectOptions, MySqlRow, MySqlTypeInfo};
use sqlx::{
    AssertSqlSafe, Column, ConnectOptions, Connection, Decode, Executor, Row, SqlSafeStr,
    Statement, TypeInfo,
};

use super::column::{ColumnBuilder, TIME_UNIT, decimal};

/// URL parameters the driver understands. `time-zone` is absent because sessions always
/// use UTC, which lets TIMESTAMP columns be typed as UTC datetimes.
const PARAMETERS: &[&str] = &[
    "sslmode",
    "ssl-mode",
    "sslca",
    "ssl-ca",
    "sslcert",
    "ssl-cert",
    "sslkey",
    "ssl-key",
    "charset",
    "collation",
    "statement-cache-capacity",
    "socket",
];

/// Days from 0001-01-01 to the Unix epoch.
const EPOCH_DAYS_FROM_CE: i32 = 719_163;

pub(super) async fn query(url: &str, statement: &str) -> anyhow::Result<DataFrame> {
    super::check_parameters(url, PARAMETERS)?;
    let options = MySqlConnectOptions::from_str(url)
        .context("invalid MySQL URL")?
        // Keep the server's SQL mode, so `||` and quoting mean what they mean in the mysql client.
        .pipes_as_concat(false)
        .no_engine_substitution(false)
        .timezone(String::from("+00:00"));
    let target = format!(
        "mysql://{}@{}:{}/{}",
        options.get_username(),
        options.get_host(),
        options.get_port(),
        options.get_database().unwrap_or_default()
    );
    let mut connection = super::connect(&target, options.connect())
        .await
        .map_err(|error| {
            // sqlx is built without RSA password exchange, which caching_sha2_password needs
            // on unencrypted connections; its own message names sqlx's cargo features.
            if format!("{error:#}").contains("RSA auth backend disabled") {
                error.context(
                    "MySQL needs an encrypted connection to check this password; use ssl-mode=PREFERRED (the default) or REQUIRED with a server that supports TLS",
                )
            } else {
                error
            }
        })?;
    let mut transaction = connection.begin_with("START TRANSACTION READ ONLY").await?;
    let prepared = transaction
        .prepare(AssertSqlSafe(statement).into_sql_str())
        .await
        .context("MySQL rejected the query")?;
    let columns = prepared.columns();
    super::check_columns(columns.iter().map(|column| column.name()))?;
    let mut decoders = columns
        .iter()
        .map(|column| {
            let kind = Kind::of(column.type_info()).with_context(|| {
                format!(
                    "column {:?} has MySQL type {}, which pl db cannot read; cast it in the query, for example with CAST(column AS CHAR)",
                    column.name(),
                    column.type_info().name()
                )
            })?;
            let builder = ColumnBuilder::new(column.name(), &kind.dtype())?;
            Ok((kind, builder))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    let mut height = 0;
    let mut rows = prepared.query().fetch(&mut *transaction);
    while let Some(row) = rows
        .try_next()
        .await
        .context("failed to read the query result")?
    {
        for (index, (kind, builder)) in decoders.iter_mut().enumerate() {
            kind.push(&row, index, builder)
                .with_context(|| format!("failed to read column {:?}", columns[index].name()))?;
        }
        height += 1;
    }
    drop(rows);
    transaction.rollback().await?;
    connection.close().await?;

    let series = decoders
        .into_iter()
        .map(|(_, builder)| builder.finish())
        .collect::<anyhow::Result<Vec<_>>>()?;
    super::frame(height, series)
}

/// How a MySQL value maps to a Polars value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Boolean,
    Int8,
    Int16,
    Int32,
    Int64,
    UInt8,
    UInt16,
    UInt32,
    UInt64,
    Float32,
    Float64,
    Decimal,
    Date,
    Datetime,
    Timestamp,
    Time,
    Text,
    Binary,
    Bit,
    Null,
}

impl Kind {
    /// The mapping for a type, or `None` when `pl db` cannot read it.
    fn of(ty: &MySqlTypeInfo) -> Option<Self> {
        Some(match ty.name() {
            // BOOL and BOOLEAN are TINYINT(1); MySQL treats every nonzero value as true.
            "BOOLEAN" => Self::Boolean,
            "TINYINT" => Self::Int8,
            "SMALLINT" | "YEAR" => Self::Int16,
            "MEDIUMINT" | "INT" => Self::Int32,
            "BIGINT" => Self::Int64,
            "TINYINT UNSIGNED" => Self::UInt8,
            "SMALLINT UNSIGNED" => Self::UInt16,
            "MEDIUMINT UNSIGNED" | "INT UNSIGNED" => Self::UInt32,
            "BIGINT UNSIGNED" => Self::UInt64,
            "FLOAT" => Self::Float32,
            "DOUBLE" => Self::Float64,
            "DECIMAL" => Self::Decimal,
            "DATE" => Self::Date,
            "DATETIME" => Self::Datetime,
            "TIMESTAMP" => Self::Timestamp,
            "TIME" => Self::Time,
            "CHAR" | "VARCHAR" | "TINYTEXT" | "TEXT" | "MEDIUMTEXT" | "LONGTEXT" | "ENUM"
            | "SET" | "JSON" => Self::Text,
            "BINARY" | "VARBINARY" | "TINYBLOB" | "BLOB" | "MEDIUMBLOB" | "LONGBLOB"
            | "GEOMETRY" => Self::Binary,
            "BIT" => Self::Bit,
            "NULL" => Self::Null,
            _ => return None,
        })
    }

    fn dtype(self) -> DataType {
        match self {
            Self::Boolean => DataType::Boolean,
            Self::Int8 => DataType::Int8,
            Self::Int16 => DataType::Int16,
            Self::Int32 => DataType::Int32,
            Self::Int64 => DataType::Int64,
            Self::UInt8 => DataType::UInt8,
            Self::UInt16 => DataType::UInt16,
            Self::UInt32 => DataType::UInt32,
            Self::UInt64 | Self::Bit => DataType::UInt64,
            Self::Float32 => DataType::Float32,
            Self::Float64 => DataType::Float64,
            Self::Decimal => DataType::Decimal(38, 0),
            Self::Date => DataType::Date,
            Self::Datetime => DataType::Datetime(TIME_UNIT, None),
            Self::Timestamp => DataType::Datetime(TIME_UNIT, Some(TimeZone::UTC)),
            // TIME is a signed duration of up to 838 hours, not a time of day.
            Self::Time => DataType::Duration(TIME_UNIT),
            Self::Text => DataType::String,
            Self::Binary => DataType::Binary,
            Self::Null => DataType::Null,
        }
    }

    fn push(self, row: &MySqlRow, index: usize, builder: &mut ColumnBuilder) -> anyhow::Result<()> {
        let value = match self {
            Self::Boolean => get::<i8>(row, index)?.map(|value| AnyValue::Boolean(value != 0)),
            Self::Int8 => get(row, index)?.map(AnyValue::Int8),
            Self::Int16 => get(row, index)?.map(AnyValue::Int16),
            Self::Int32 => get(row, index)?.map(AnyValue::Int32),
            Self::Int64 => get(row, index)?.map(AnyValue::Int64),
            Self::UInt8 => get(row, index)?.map(AnyValue::UInt8),
            Self::UInt16 => get(row, index)?.map(AnyValue::UInt16),
            Self::UInt32 => get(row, index)?.map(AnyValue::UInt32),
            Self::UInt64 => get(row, index)?.map(AnyValue::UInt64),
            Self::Float32 => get(row, index)?.map(AnyValue::Float32),
            Self::Float64 => get(row, index)?.map(AnyValue::Float64),
            Self::Decimal => get::<&str>(row, index)?.map(decimal).transpose()?,
            Self::Date => get::<&[u8]>(row, index)?
                .map(datetime)
                .transpose()?
                .flatten()
                .map(|datetime| {
                    AnyValue::Date(datetime.date().num_days_from_ce() - EPOCH_DAYS_FROM_CE)
                }),
            Self::Datetime | Self::Timestamp => get::<&[u8]>(row, index)?
                .map(datetime)
                .transpose()?
                .flatten()
                .map(|datetime| {
                    AnyValue::Datetime(datetime.and_utc().timestamp_micros(), TIME_UNIT, None)
                }),
            Self::Time => get::<&[u8]>(row, index)?
                .map(time)
                .transpose()?
                .map(|micros| AnyValue::Duration(micros, TIME_UNIT)),
            Self::Text => get::<&str>(row, index)?.map(AnyValue::String),
            Self::Binary => get::<&[u8]>(row, index)?.map(AnyValue::Binary),
            Self::Bit => get::<&[u8]>(row, index)?.map(bit).transpose()?,
            Self::Null => None,
        };
        builder.push(value.unwrap_or(AnyValue::Null))
    }
}

/// Decode a value without sqlx's type check; [`Kind::of`] already chose the decoding.
fn get<'r, T: Decode<'r, MySql>>(row: &'r MySqlRow, index: usize) -> anyhow::Result<Option<T>> {
    Ok(row.try_get_unchecked::<Option<T>, _>(index)?)
}

/// Decode a binary DATE, DATETIME, or TIMESTAMP: a length byte, then the date and time
/// parts. MySQL's zero date has no parts; it means "no date" and becomes null.
fn datetime(bytes: &[u8]) -> anyhow::Result<Option<NaiveDateTime>> {
    let parts = bytes.get(1..).context("truncated date value")?;
    if parts.is_empty() {
        return Ok(None);
    }
    let [year_low, year_high, month, day, time @ ..] = parts else {
        bail!("truncated date value");
    };
    let year = u16::from_le_bytes([*year_low, *year_high]);
    let date = NaiveDate::from_ymd_opt(year.into(), (*month).into(), (*day).into())
        .with_context(|| {
            format!(
                "the date {year:04}-{month:02}-{day:02} is invalid; replace it in the query, for example with NULLIF(column, '{year:04}-{month:02}-{day:02}')"
            )
        })?;
    let (hour, minute, second, micros) = match time {
        [] => (0, 0, 0, 0),
        [hour, minute, second] => (*hour, *minute, *second, 0),
        [hour, minute, second, micros @ ..] if micros.len() == 4 => (
            *hour,
            *minute,
            *second,
            u32::from_le_bytes([micros[0], micros[1], micros[2], micros[3]]),
        ),
        _ => bail!("malformed datetime value"),
    };
    let time = NaiveTime::from_hms_micro_opt(hour.into(), minute.into(), second.into(), micros)
        .context("malformed datetime value")?;
    Ok(Some(date.and_time(time)))
}

/// Decode a binary TIME into signed microseconds: a length byte, a sign, days, hours,
/// minutes, seconds, and microseconds. A zero time has no parts.
fn time(bytes: &[u8]) -> anyhow::Result<i64> {
    let parts = bytes.get(1..).context("truncated time value")?;
    if parts.is_empty() {
        return Ok(0);
    }
    let [
        negative,
        d0,
        d1,
        d2,
        d3,
        hours,
        minutes,
        seconds,
        micros @ ..,
    ] = parts
    else {
        bail!("truncated time value");
    };
    let micros = match micros {
        [] => 0,
        [m0, m1, m2, m3] => u32::from_le_bytes([*m0, *m1, *m2, *m3]),
        _ => bail!("malformed time value"),
    };
    let days = i64::from(u32::from_le_bytes([*d0, *d1, *d2, *d3]));
    let seconds =
        ((days * 24 + i64::from(*hours)) * 60 + i64::from(*minutes)) * 60 + i64::from(*seconds);
    let total = seconds * 1_000_000 + i64::from(micros);
    Ok(if *negative == 0 { total } else { -total })
}

/// A BIT(n) value: up to eight big-endian bytes.
fn bit(bytes: &[u8]) -> anyhow::Result<AnyValue<'static>> {
    ensure!(bytes.len() <= 8, "BIT value wider than 64 bits");
    Ok(AnyValue::UInt64(
        bytes
            .iter()
            .fold(0, |value, byte| value << 8 | u64::from(*byte)),
    ))
}

#[cfg(test)]
mod tests {
    use super::{bit, datetime, time};

    #[test]
    fn binary_datetimes_decode_and_zero_dates_are_null() {
        assert_eq!(datetime(&[0]).unwrap(), None);
        let date = datetime(&[4, 0xE8, 0x07, 2, 29]).unwrap().unwrap();
        assert_eq!(date.to_string(), "2024-02-29 00:00:00");
        let precise = datetime(&[11, 0xE8, 0x07, 2, 29, 13, 5, 9, 0x40, 0xE2, 0x01, 0x00])
            .unwrap()
            .unwrap();
        assert_eq!(precise.to_string(), "2024-02-29 13:05:09.123456");
        assert!(datetime(&[4, 0xE8, 0x07, 2, 30]).is_err());
    }

    #[test]
    fn binary_times_are_signed_durations() {
        assert_eq!(time(&[0]).unwrap(), 0);
        // -1 day 02:03:04 = -(26h 3m 4s)
        let micros = time(&[8, 1, 1, 0, 0, 0, 2, 3, 4]).unwrap();
        assert_eq!(micros, -((26 * 3600 + 3 * 60 + 4) * 1_000_000));
    }

    #[test]
    fn bits_are_big_endian() {
        assert_eq!(
            bit(&[0x01, 0x02]).unwrap(),
            polars::prelude::AnyValue::UInt64(258)
        );
    }
}
