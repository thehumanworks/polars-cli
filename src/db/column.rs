//! Typed column builders shared by the PostgreSQL and MySQL backends.

use anyhow::{Context, bail, ensure};
use polars::chunked_array::builder::get_list_builder;
use polars::prelude::*;

/// Datetimes and durations keep the microsecond precision of PostgreSQL and MySQL.
pub(super) const TIME_UNIT: TimeUnit = TimeUnit::Microseconds;

/// The widest decimal Polars supports.
const MAX_PRECISION: usize = 38;

const TOO_WIDE: &str = "a decimal value needs more than 38 digits; cast the column in the query, for example to a float or text";

/// One result column, filled value by value in its final Polars type.
pub(super) enum ColumnBuilder {
    Null(PlSmallStr, usize),
    Boolean(BooleanChunkedBuilder),
    Int8(PrimitiveChunkedBuilder<Int8Type>),
    Int16(PrimitiveChunkedBuilder<Int16Type>),
    Int32(PrimitiveChunkedBuilder<Int32Type>),
    Int64(PrimitiveChunkedBuilder<Int64Type>),
    UInt8(PrimitiveChunkedBuilder<UInt8Type>),
    UInt16(PrimitiveChunkedBuilder<UInt16Type>),
    UInt32(PrimitiveChunkedBuilder<UInt32Type>),
    UInt64(PrimitiveChunkedBuilder<UInt64Type>),
    Float32(PrimitiveChunkedBuilder<Float32Type>),
    Float64(PrimitiveChunkedBuilder<Float64Type>),
    String(StringChunkedBuilder),
    Binary(BinaryChunkedBuilder),
    Date(PrimitiveChunkedBuilder<Int32Type>),
    Datetime(PrimitiveChunkedBuilder<Int64Type>, Option<TimeZone>),
    Duration(PrimitiveChunkedBuilder<Int64Type>),
    Time(PrimitiveChunkedBuilder<Int64Type>),
    Decimal(DecimalBuilder),
    List(Box<ListBuilder>),
}

impl ColumnBuilder {
    /// A builder for `dtype`. A decimal column's scale follows its values, so the scale in
    /// `dtype` is ignored.
    pub(super) fn new(name: &str, dtype: &DataType) -> anyhow::Result<Self> {
        let name = PlSmallStr::from(name);
        Ok(match dtype {
            DataType::Null => Self::Null(name, 0),
            DataType::Boolean => Self::Boolean(BooleanChunkedBuilder::new(name, 0)),
            DataType::Int8 => Self::Int8(PrimitiveChunkedBuilder::new(name, 0)),
            DataType::Int16 => Self::Int16(PrimitiveChunkedBuilder::new(name, 0)),
            DataType::Int32 => Self::Int32(PrimitiveChunkedBuilder::new(name, 0)),
            DataType::Int64 => Self::Int64(PrimitiveChunkedBuilder::new(name, 0)),
            DataType::UInt8 => Self::UInt8(PrimitiveChunkedBuilder::new(name, 0)),
            DataType::UInt16 => Self::UInt16(PrimitiveChunkedBuilder::new(name, 0)),
            DataType::UInt32 => Self::UInt32(PrimitiveChunkedBuilder::new(name, 0)),
            DataType::UInt64 => Self::UInt64(PrimitiveChunkedBuilder::new(name, 0)),
            DataType::Float32 => Self::Float32(PrimitiveChunkedBuilder::new(name, 0)),
            DataType::Float64 => Self::Float64(PrimitiveChunkedBuilder::new(name, 0)),
            DataType::String => Self::String(StringChunkedBuilder::new(name, 0)),
            DataType::Binary => Self::Binary(BinaryChunkedBuilder::new(name, 0)),
            DataType::Date => Self::Date(PrimitiveChunkedBuilder::new(name, 0)),
            DataType::Datetime(TIME_UNIT, zone) => {
                Self::Datetime(PrimitiveChunkedBuilder::new(name, 0), zone.clone())
            }
            DataType::Duration(TIME_UNIT) => Self::Duration(PrimitiveChunkedBuilder::new(name, 0)),
            DataType::Time => Self::Time(PrimitiveChunkedBuilder::new(name, 0)),
            DataType::Decimal(_, _) => Self::Decimal(DecimalBuilder {
                name,
                scale: 0,
                values: Vec::new(),
            }),
            DataType::List(inner) => Self::List(Box::new(ListBuilder {
                inner: Self::new(&name, inner)?,
                name,
                lengths: Vec::new(),
            })),
            other => bail!("internal error: no column builder for {other}"),
        })
    }

    pub(super) fn push(&mut self, value: AnyValue<'_>) -> anyhow::Result<()> {
        if value.is_null() {
            self.push_null();
            return Ok(());
        }
        match (self, value) {
            (Self::Boolean(builder), AnyValue::Boolean(value)) => builder.append_value(value),
            (Self::Int8(builder), AnyValue::Int8(value)) => builder.append_value(value),
            (Self::Int16(builder), AnyValue::Int16(value)) => builder.append_value(value),
            (Self::Int32(builder), AnyValue::Int32(value)) => builder.append_value(value),
            (Self::Int64(builder), AnyValue::Int64(value)) => builder.append_value(value),
            (Self::UInt8(builder), AnyValue::UInt8(value)) => builder.append_value(value),
            (Self::UInt16(builder), AnyValue::UInt16(value)) => builder.append_value(value),
            (Self::UInt32(builder), AnyValue::UInt32(value)) => builder.append_value(value),
            (Self::UInt64(builder), AnyValue::UInt64(value)) => builder.append_value(value),
            (Self::Float32(builder), AnyValue::Float32(value)) => builder.append_value(value),
            (Self::Float64(builder), AnyValue::Float64(value)) => builder.append_value(value),
            (Self::String(builder), AnyValue::String(value)) => builder.append_value(value),
            (Self::String(builder), AnyValue::StringOwned(value)) => builder.append_value(value),
            (Self::Binary(builder), AnyValue::Binary(value)) => builder.append_value(value),
            (Self::Date(builder), AnyValue::Date(days)) => builder.append_value(days),
            (Self::Datetime(builder, _), AnyValue::Datetime(micros, TIME_UNIT, _)) => {
                builder.append_value(micros)
            }
            (Self::Duration(builder), AnyValue::Duration(micros, TIME_UNIT)) => {
                builder.append_value(micros)
            }
            (Self::Time(builder), AnyValue::Time(nanos)) => builder.append_value(nanos),
            (Self::Decimal(builder), AnyValue::Decimal(mantissa, _, scale)) => {
                builder.push(mantissa, scale)?
            }
            (_, value) => bail!(
                "internal error: a {} value does not match its column",
                value.dtype()
            ),
        }
        Ok(())
    }

    fn push_null(&mut self) {
        match self {
            Self::Null(_, len) => *len += 1,
            Self::Boolean(builder) => builder.append_null(),
            Self::Int8(builder) => builder.append_null(),
            Self::Int16(builder) => builder.append_null(),
            Self::Int32(builder) | Self::Date(builder) => builder.append_null(),
            Self::Int64(builder)
            | Self::Datetime(builder, _)
            | Self::Duration(builder)
            | Self::Time(builder) => builder.append_null(),
            Self::UInt8(builder) => builder.append_null(),
            Self::UInt16(builder) => builder.append_null(),
            Self::UInt32(builder) => builder.append_null(),
            Self::UInt64(builder) => builder.append_null(),
            Self::Float32(builder) => builder.append_null(),
            Self::Float64(builder) => builder.append_null(),
            Self::String(builder) => builder.append_null(),
            Self::Binary(builder) => builder.append_null(),
            Self::Decimal(builder) => builder.values.push(None),
            Self::List(builder) => builder.lengths.push(None),
        }
    }

    /// Append one list whose elements `fill` pushes; `fill` returns how many it pushed.
    pub(super) fn push_list(
        &mut self,
        fill: impl FnOnce(&mut ColumnBuilder) -> anyhow::Result<usize>,
    ) -> anyhow::Result<()> {
        let Self::List(builder) = self else {
            bail!("internal error: a list value does not match its column");
        };
        let length = fill(&mut builder.inner)?;
        builder.lengths.push(Some(length));
        Ok(())
    }

    pub(super) fn finish(self) -> anyhow::Result<Series> {
        Ok(match self {
            Self::Null(name, len) => Series::full_null(name, len, &DataType::Null),
            Self::Boolean(builder) => builder.finish().into_series(),
            Self::Int8(builder) => builder.finish().into_series(),
            Self::Int16(builder) => builder.finish().into_series(),
            Self::Int32(builder) => builder.finish().into_series(),
            Self::Int64(builder) => builder.finish().into_series(),
            Self::UInt8(builder) => builder.finish().into_series(),
            Self::UInt16(builder) => builder.finish().into_series(),
            Self::UInt32(builder) => builder.finish().into_series(),
            Self::UInt64(builder) => builder.finish().into_series(),
            Self::Float32(builder) => builder.finish().into_series(),
            Self::Float64(builder) => builder.finish().into_series(),
            Self::String(builder) => builder.finish().into_series(),
            Self::Binary(builder) => builder.finish().into_series(),
            Self::Date(builder) => builder.finish().into_date().into_series(),
            Self::Datetime(builder, zone) => builder
                .finish()
                .into_datetime(TIME_UNIT, zone)
                .into_series(),
            Self::Duration(builder) => builder.finish().into_duration(TIME_UNIT).into_series(),
            Self::Time(builder) => builder.finish().into_time().into_series(),
            Self::Decimal(builder) => builder.finish()?,
            Self::List(builder) => builder.finish()?,
        })
    }
}

/// Decimal values, all brought to the largest scale seen so far.
pub(super) struct DecimalBuilder {
    name: PlSmallStr,
    scale: usize,
    values: Vec<Option<i128>>,
}

impl DecimalBuilder {
    fn push(&mut self, mantissa: i128, scale: usize) -> anyhow::Result<()> {
        if scale > self.scale {
            let factor = power_of_ten(scale - self.scale)?;
            for value in self.values.iter_mut().flatten() {
                *value = fit(value.checked_mul(factor))?;
            }
            self.scale = scale;
        }
        let value = fit(mantissa.checked_mul(power_of_ten(self.scale - scale)?))?;
        self.values.push(Some(value));
        Ok(())
    }

    fn finish(self) -> anyhow::Result<Series> {
        Ok(
            Int128Chunked::from_iter_options(self.name, self.values.into_iter())
                .into_decimal(MAX_PRECISION, self.scale)?
                .into_series(),
        )
    }
}

fn power_of_ten(exponent: usize) -> anyhow::Result<i128> {
    u32::try_from(exponent)
        .ok()
        .filter(|exponent| *exponent as usize <= MAX_PRECISION)
        .and_then(|exponent| 10_i128.checked_pow(exponent))
        .context(TOO_WIDE)
}

/// Keep a mantissa only when it has at most [`MAX_PRECISION`] digits.
fn fit(mantissa: Option<i128>) -> anyhow::Result<i128> {
    let limit = 10_u128.pow(MAX_PRECISION as u32);
    mantissa
        .filter(|mantissa| mantissa.unsigned_abs() < limit)
        .context(TOO_WIDE)
}

/// Parse decimal text such as `-12.340` exactly, keeping its scale.
pub(super) fn decimal(text: &str) -> anyhow::Result<AnyValue<'static>> {
    let (negative, digits) = match text.strip_prefix('-') {
        Some(digits) => (true, digits),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    let (whole, fraction) = digits.split_once('.').unwrap_or((digits, ""));
    ensure!(
        !(whole.is_empty() && fraction.is_empty())
            && whole
                .bytes()
                .chain(fraction.bytes())
                .all(|byte| byte.is_ascii_digit()),
        "invalid decimal value {text:?}"
    );
    let mut mantissa = 0_i128;
    for digit in whole.bytes().chain(fraction.bytes()) {
        mantissa = fit(mantissa
            .checked_mul(10)
            .and_then(|mantissa| mantissa.checked_add(i128::from(digit - b'0'))))?;
    }
    let mantissa = if negative { -mantissa } else { mantissa };
    Ok(AnyValue::Decimal(mantissa, MAX_PRECISION, fraction.len()))
}

/// List values stored as their elements, flattened, and each list's length.
pub(super) struct ListBuilder {
    name: PlSmallStr,
    inner: ColumnBuilder,
    lengths: Vec<Option<usize>>,
}

impl ListBuilder {
    fn finish(self) -> anyhow::Result<Series> {
        let elements = self.inner.finish()?;
        let mut builder = get_list_builder(
            elements.dtype(),
            elements.len(),
            self.lengths.len(),
            self.name,
        );
        let mut offset = 0;
        for length in self.lengths {
            match length {
                Some(length) => {
                    builder.append_series(&elements.slice(offset as i64, length))?;
                    offset += length;
                }
                None => builder.append_null(),
            }
        }
        Ok(builder.finish().into_series())
    }
}

#[cfg(test)]
mod tests {
    use polars::prelude::{AnyValue, DataType};

    use super::{ColumnBuilder, decimal};

    #[test]
    fn decimal_text_keeps_its_scale() {
        assert_eq!(
            decimal("-12.340").unwrap(),
            AnyValue::Decimal(-12_340, 38, 3)
        );
        assert_eq!(decimal("7").unwrap(), AnyValue::Decimal(7, 38, 0));
        assert!(decimal("1e5").is_err());
        assert!(decimal("").is_err());
        assert!(decimal(&"9".repeat(39)).is_err());
    }

    #[test]
    fn decimal_columns_take_the_largest_scale() {
        let mut column = ColumnBuilder::new("amount", &DataType::Decimal(38, 0)).unwrap();
        column.push(decimal("1.5").unwrap()).unwrap();
        column.push(AnyValue::Null).unwrap();
        column.push(decimal("2.25").unwrap()).unwrap();
        let series = column.finish().unwrap();
        assert_eq!(series.dtype(), &DataType::Decimal(38, 2));
        assert_eq!(
            series
                .cast(&DataType::String)
                .unwrap()
                .str()
                .unwrap()
                .get(0),
            Some("1.50")
        );
        assert_eq!(series.null_count(), 1);
    }

    #[test]
    fn list_columns_keep_null_and_empty_lists() {
        let mut column =
            ColumnBuilder::new("tags", &DataType::List(Box::new(DataType::String))).unwrap();
        column
            .push_list(|inner| {
                inner.push(AnyValue::String("a"))?;
                inner.push(AnyValue::Null)?;
                Ok(2)
            })
            .unwrap();
        column.push(AnyValue::Null).unwrap();
        column.push_list(|_| Ok(0)).unwrap();
        let series = column.finish().unwrap();
        assert_eq!(series.dtype(), &DataType::List(Box::new(DataType::String)));
        let lists = series.list().unwrap();
        assert_eq!(lists.get_as_series(0).unwrap().len(), 2);
        assert!(lists.get_as_series(1).is_none());
        assert_eq!(lists.get_as_series(2).unwrap().len(), 0);
    }
}
