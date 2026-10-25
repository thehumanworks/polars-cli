//! PostgreSQL: binary result values decoded into Polars types.

use anyhow::{Context, bail, ensure};
use futures_util::TryStreamExt;
use polars::prelude::{AnyValue, DataFrame, DataType, TimeZone};
use sqlx::postgres::{PgConnectOptions, PgTypeInfo, PgTypeKind, PgValueFormat};
use sqlx::{
    AssertSqlSafe, Column, ConnectOptions, Connection, Executor, Row, SqlSafeStr, Statement,
    TypeInfo, ValueRef,
};

use super::column::{ColumnBuilder, TIME_UNIT};

/// URL parameters the driver understands; libpq parameters it lacks are rejected.
const PARAMETERS: &[&str] = &[
    "sslmode",
    "ssl-mode",
    "sslrootcert",
    "ssl-root-cert",
    "ssl-ca",
    "sslcert",
    "ssl-cert",
    "sslkey",
    "ssl-key",
    "statement-cache-capacity",
    "host",
    "hostaddr",
    "port",
    "dbname",
    "user",
    "password",
    "application_name",
    "options",
    "options[]",
];

/// Days and microseconds from the Unix epoch to PostgreSQL's epoch, 2000-01-01.
const EPOCH_DAYS: i32 = 10_957;
const EPOCH_MICROS: i64 = 946_684_800_000_000;
const MICROS_PER_DAY: i64 = 86_400_000_000;

const INFINITE: &str = "infinite dates and timestamps have no Polars equivalent; replace them in the query, for example with CASE WHEN isfinite(column) THEN column END";

pub(super) async fn query(url: &str, statement: &str) -> anyhow::Result<DataFrame> {
    super::check_parameters(url, PARAMETERS)?;
    let options: PgConnectOptions = url.parse().context("invalid PostgreSQL URL")?;
    let target = format!(
        "postgres://{}@{}:{}/{}",
        options.get_username(),
        options.get_host(),
        options.get_port(),
        options.get_database().unwrap_or_default()
    );
    let mut connection = super::connect(&target, options.connect()).await?;
    // A read-only transaction holds through connection poolers, unlike a session setting.
    let mut transaction = connection.begin_with("BEGIN READ ONLY").await?;
    let prepared = transaction
        .prepare(AssertSqlSafe(statement).into_sql_str())
        .await
        .context("PostgreSQL rejected the query")?;
    let columns = prepared.columns();
    super::check_columns(columns.iter().map(|column| column.name()))?;
    let mut decoders = columns
        .iter()
        .map(|column| {
            let kind = Kind::of(column.type_info()).with_context(|| {
                format!(
                    "column {:?} has PostgreSQL type {}, which pl db cannot read; cast it in the query, for example with ::text",
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
            let value = row.try_get_raw(index)?;
            let pushed = if value.is_null() {
                builder.push(AnyValue::Null)
            } else if value.format() != PgValueFormat::Binary {
                Err(anyhow::anyhow!(
                    "internal error: expected a binary-format value"
                ))
            } else {
                value
                    .as_bytes()
                    .map_err(|error| anyhow::anyhow!(error))
                    .and_then(|bytes| kind.push(bytes, builder))
            };
            pushed.with_context(|| format!("failed to read column {:?}", columns[index].name()))?;
        }
        height += 1;
    }
    drop(rows);
    // Prepared statements outlive transactions. Behind a transaction-mode pooler the server
    // connection is shared, so close them before the transaction hands it back.
    transaction.clear_cached_statements().await?;
    transaction.rollback().await?;
    connection.close().await?;

    let series = decoders
        .into_iter()
        .map(|(_, builder)| builder.finish())
        .collect::<anyhow::Result<Vec<_>>>()?;
    super::frame(height, series)
}

/// How a PostgreSQL value maps to a Polars value.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    Bool,
    Int2,
    Int4,
    Int8,
    Oid,
    Float4,
    Float8,
    Numeric,
    Text,
    Jsonb,
    Uuid,
    Bytea,
    Date,
    Time,
    Timestamp,
    Timestamptz,
    Interval,
    Array(Box<Kind>),
}

impl Kind {
    /// The mapping for a type, or `None` when `pl db` cannot read it.
    fn of(ty: &PgTypeInfo) -> Option<Self> {
        match ty.kind() {
            PgTypeKind::Simple => {}
            PgTypeKind::Domain(base) => return Self::of(base),
            // An enum's binary format is its label.
            PgTypeKind::Enum(_) => return Some(Self::Text),
            PgTypeKind::Array(element) => {
                return match Self::of(element)? {
                    Self::Array(_) => None,
                    element => Some(Self::Array(Box::new(element))),
                };
            }
            PgTypeKind::Pseudo | PgTypeKind::Composite(_) | PgTypeKind::Range(_) => return None,
        }
        // Built-in type OIDs are fixed by the PostgreSQL catalog.
        Some(match ty.oid()?.0 {
            16 => Self::Bool,
            17 => Self::Bytea,
            // "char", name, text, json, xml, unknown, bpchar, varchar
            18 | 19 | 25 | 114 | 142 | 705 | 1042 | 1043 => Self::Text,
            20 => Self::Int8,
            21 => Self::Int2,
            23 => Self::Int4,
            26 => Self::Oid,
            700 => Self::Float4,
            701 => Self::Float8,
            1082 => Self::Date,
            1083 => Self::Time,
            1114 => Self::Timestamp,
            1184 => Self::Timestamptz,
            1186 => Self::Interval,
            1700 => Self::Numeric,
            2950 => Self::Uuid,
            3802 => Self::Jsonb,
            // The citext extension sends text.
            _ if ty.name() == "citext" => Self::Text,
            _ => return None,
        })
    }

    fn dtype(&self) -> DataType {
        match self {
            Self::Bool => DataType::Boolean,
            Self::Int2 => DataType::Int16,
            Self::Int4 => DataType::Int32,
            Self::Int8 => DataType::Int64,
            Self::Oid => DataType::UInt32,
            Self::Float4 => DataType::Float32,
            Self::Float8 => DataType::Float64,
            Self::Numeric => DataType::Decimal(38, 0),
            Self::Text | Self::Jsonb | Self::Uuid => DataType::String,
            Self::Bytea => DataType::Binary,
            Self::Date => DataType::Date,
            Self::Time => DataType::Time,
            Self::Timestamp => DataType::Datetime(TIME_UNIT, None),
            Self::Timestamptz => DataType::Datetime(TIME_UNIT, Some(TimeZone::UTC)),
            Self::Interval => DataType::Duration(TIME_UNIT),
            Self::Array(element) => DataType::List(Box::new(element.dtype())),
        }
    }

    fn push(&self, bytes: &[u8], builder: &mut ColumnBuilder) -> anyhow::Result<()> {
        match self {
            Self::Array(element) => {
                builder.push_list(|elements| push_array_elements(element, bytes, elements))
            }
            _ => builder.push(self.decode(bytes)?),
        }
    }

    fn decode<'a>(&self, bytes: &'a [u8]) -> anyhow::Result<AnyValue<'a>> {
        Ok(match self {
            Self::Bool => AnyValue::Boolean(exact::<1>(bytes)? != [0]),
            Self::Int2 => AnyValue::Int16(i16::from_be_bytes(exact(bytes)?)),
            Self::Int4 => AnyValue::Int32(i32::from_be_bytes(exact(bytes)?)),
            Self::Int8 => AnyValue::Int64(i64::from_be_bytes(exact(bytes)?)),
            Self::Oid => AnyValue::UInt32(u32::from_be_bytes(exact(bytes)?)),
            Self::Float4 => AnyValue::Float32(f32::from_be_bytes(exact(bytes)?)),
            Self::Float8 => AnyValue::Float64(f64::from_be_bytes(exact(bytes)?)),
            Self::Numeric => numeric(bytes)?,
            Self::Text => AnyValue::String(std::str::from_utf8(bytes)?),
            Self::Jsonb => match bytes.split_first() {
                Some((1, json)) => AnyValue::String(std::str::from_utf8(json)?),
                _ => bail!("unsupported jsonb format version"),
            },
            Self::Uuid => AnyValue::StringOwned(uuid(exact(bytes)?).into()),
            Self::Bytea => AnyValue::Binary(bytes),
            Self::Date => {
                let days = i32::from_be_bytes(exact(bytes)?);
                ensure!(days != i32::MAX && days != i32::MIN, INFINITE);
                AnyValue::Date(days.checked_add(EPOCH_DAYS).context("date out of range")?)
            }
            Self::Time => {
                let micros = i64::from_be_bytes(exact(bytes)?);
                // PostgreSQL accepts 24:00:00; a Polars time is shorter than a day.
                ensure!(
                    (0..MICROS_PER_DAY).contains(&micros),
                    "the time 24:00:00 has no Polars equivalent; replace it in the query"
                );
                AnyValue::Time(micros * 1_000)
            }
            Self::Timestamp | Self::Timestamptz => {
                let micros = i64::from_be_bytes(exact(bytes)?);
                ensure!(micros != i64::MAX && micros != i64::MIN, INFINITE);
                let micros = micros
                    .checked_add(EPOCH_MICROS)
                    .context("timestamp out of range")?;
                AnyValue::Datetime(micros, TIME_UNIT, None)
            }
            Self::Interval => AnyValue::Duration(interval(exact(bytes)?)?, TIME_UNIT),
            Self::Array(_) => bail!("internal error: nested array value"),
        })
    }
}

/// Push the elements of a one-dimensional array and return how many there were.
fn push_array_elements(
    element: &Kind,
    mut bytes: &[u8],
    elements: &mut ColumnBuilder,
) -> anyhow::Result<usize> {
    let dimensions = i32::from_be_bytes(take(&mut bytes)?);
    let _has_nulls: [u8; 4] = take(&mut bytes)?;
    let _element_oid: [u8; 4] = take(&mut bytes)?;
    let count = match dimensions {
        0 => 0,
        1 => {
            let count = i32::from_be_bytes(take(&mut bytes)?);
            let _lower_bound: [u8; 4] = take(&mut bytes)?;
            usize::try_from(count).context("invalid array length")?
        }
        _ => bail!(
            "multi-dimensional arrays have no Polars equivalent; convert them in the query, for example with array_to_json(column)::text"
        ),
    };
    for _ in 0..count {
        let length = i32::from_be_bytes(take(&mut bytes)?);
        // A length of -1 marks a null element.
        let Ok(length) = usize::try_from(length) else {
            elements.push(AnyValue::Null)?;
            continue;
        };
        let (value, rest) = bytes
            .split_at_checked(length)
            .context("truncated array value")?;
        bytes = rest;
        elements.push(element.decode(value)?)?;
    }
    Ok(count)
}

/// Decode PostgreSQL's numeric format: base-10000 digits, a weight, a sign, and a scale.
fn numeric(bytes: &[u8]) -> anyhow::Result<AnyValue<'static>> {
    let mut rest = bytes;
    let digit_count = u16::from_be_bytes(take(&mut rest)?);
    let weight = i16::from_be_bytes(take(&mut rest)?);
    let sign = u16::from_be_bytes(take(&mut rest)?);
    let scale = u16::from_be_bytes(take(&mut rest)?);
    ensure!(
        matches!(sign, 0x0000 | 0x4000),
        "NaN and infinite numerics have no decimal equivalent; cast the column in the query, for example with ::float8"
    );
    ensure!(
        rest.len() == usize::from(digit_count) * 2,
        "malformed numeric value"
    );
    let too_wide = || "a decimal value needs more than 38 digits; cast the column in the query, for example with ::float8 or ::text";
    // The mantissa counts units of 10^-scale; digit `index` is worth 10000^(weight - index).
    let mut mantissa = 0_i128;
    for (index, digit) in rest.chunks_exact(2).enumerate() {
        let digit = i128::from(i16::from_be_bytes([digit[0], digit[1]]));
        if digit == 0 {
            continue;
        }
        let exponent = 4 * (i64::from(weight) - index as i64) + i64::from(scale);
        let term = if exponent >= 0 {
            u32::try_from(exponent)
                .ok()
                .and_then(|exponent| 10_i128.checked_pow(exponent))
                .and_then(|factor| digit.checked_mul(factor))
                .with_context(too_wide)?
        } else {
            // Digits below the scale are always zero, so this division is exact.
            let divisor = u32::try_from(-exponent)
                .ok()
                .and_then(|exponent| 10_i128.checked_pow(exponent))
                .context("malformed numeric value")?;
            ensure!(digit % divisor == 0, "malformed numeric value");
            digit / divisor
        };
        mantissa = mantissa.checked_add(term).with_context(too_wide)?;
    }
    if sign == 0x4000 {
        mantissa = -mantissa;
    }
    Ok(AnyValue::Decimal(mantissa, 38, usize::from(scale)))
}

/// Microseconds in an interval. Months have no fixed length, so they are rejected.
fn interval(bytes: [u8; 16]) -> anyhow::Result<i64> {
    let mut rest = &bytes[..];
    let micros = i64::from_be_bytes(take(&mut rest)?);
    let days = i32::from_be_bytes(take(&mut rest)?);
    let months = i32::from_be_bytes(take(&mut rest)?);
    ensure!(
        months == 0,
        "intervals with months or years have no fixed length; convert them in the query, for example with EXTRACT(EPOCH FROM column)"
    );
    i64::from(days)
        .checked_mul(MICROS_PER_DAY)
        .and_then(|days| days.checked_add(micros))
        .context("interval out of range")
}

fn uuid(bytes: [u8; 16]) -> String {
    let hex = bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

/// A fixed-width value.
fn exact<const N: usize>(bytes: &[u8]) -> anyhow::Result<[u8; N]> {
    bytes
        .try_into()
        .with_context(|| format!("expected a {N}-byte value, got {} bytes", bytes.len()))
}

/// The next `N` bytes of a variable-width value.
fn take<const N: usize>(bytes: &mut &[u8]) -> anyhow::Result<[u8; N]> {
    let (head, rest) = bytes
        .split_first_chunk::<N>()
        .context("truncated PostgreSQL value")?;
    *bytes = rest;
    Ok(*head)
}

#[cfg(test)]
mod tests {
    use polars::prelude::AnyValue;

    use super::{interval, numeric, uuid};

    fn numeric_bytes(weight: i16, sign: u16, scale: u16, digits: &[i16]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend((digits.len() as u16).to_be_bytes());
        bytes.extend(weight.to_be_bytes());
        bytes.extend(sign.to_be_bytes());
        bytes.extend(scale.to_be_bytes());
        for digit in digits {
            bytes.extend(digit.to_be_bytes());
        }
        bytes
    }

    #[test]
    fn numeric_values_decode_exactly() {
        // 12345.678 is 1 2345 . 6780 in base 10000.
        let value = numeric(&numeric_bytes(1, 0, 3, &[1, 2345, 6780])).unwrap();
        assert_eq!(value, AnyValue::Decimal(12_345_678, 38, 3));
        // -0.05 is weight -1, digit 0500.
        let value = numeric(&numeric_bytes(-1, 0x4000, 2, &[500])).unwrap();
        assert_eq!(value, AnyValue::Decimal(-5, 38, 2));
        // 10^40 does not fit 38 digits.
        assert!(numeric(&numeric_bytes(10, 0, 0, &[1])).is_err());
        assert!(numeric(&numeric_bytes(0, 0xC000, 0, &[])).is_err());
    }

    #[test]
    fn intervals_without_months_become_microseconds() {
        let mut bytes = [0_u8; 16];
        bytes[..8].copy_from_slice(&1_500_000_i64.to_be_bytes());
        bytes[8..12].copy_from_slice(&2_i32.to_be_bytes());
        assert_eq!(interval(bytes).unwrap(), 2 * 86_400_000_000 + 1_500_000);
        bytes[12..].copy_from_slice(&1_i32.to_be_bytes());
        assert!(interval(bytes).is_err());
    }

    #[test]
    fn uuids_use_the_hyphenated_form() {
        let bytes = *b"\x12\x34\x56\x78\x9a\xbc\xde\xf0\x12\x34\x56\x78\x9a\xbc\xde\xf0";
        assert_eq!(uuid(bytes), "12345678-9abc-def0-1234-56789abcdef0");
    }
}
