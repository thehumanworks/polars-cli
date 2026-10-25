//! SQLite: values typed one by one, so each column takes the widest storage class it holds.

use std::str::FromStr;

use anyhow::{Context, bail};
use futures_util::TryStreamExt;
use polars::prelude::{DataFrame, DataType, NamedFrom, Series};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{
    AssertSqlSafe, Column, ConnectOptions, Connection, Either, Executor, Row, SqlSafeStr,
    Statement, TypeInfo, ValueRef,
};

pub(super) async fn query(url: &str, statement: &str) -> anyhow::Result<DataFrame> {
    let options = SqliteConnectOptions::from_str(url)
        .context("invalid SQLite URL")?
        .read_only(true)
        .create_if_missing(false);
    let target = options.get_filename().display().to_string();
    let mut connection = super::connect(&target, options.connect()).await?;
    let prepared = connection
        .prepare(AssertSqlSafe(statement).into_sql_str())
        .await
        .context("SQLite rejected the query")?;
    let columns = prepared.columns();
    super::check_columns(columns.iter().map(|column| column.name()))?;
    let mut values = columns
        .iter()
        .map(|column| Values::declared(column.type_info().name()))
        .collect::<Vec<_>>();

    // Unlike the server databases, sqlx's SQLite driver runs every statement in the text,
    // and only `fetch_many` shows where one ends. sqlx deprecates multi-statement SQLite
    // queries; once it rejects them itself, this check can go.
    let mut height = 0;
    let mut finished = false;
    #[allow(deprecated)]
    let mut steps = prepared.query().fetch_many(&mut connection);
    while let Some(step) = steps
        .try_next()
        .await
        .context("failed to read the query result")?
    {
        let row = match step {
            Either::Left(_) if !finished => {
                finished = true;
                continue;
            }
            Either::Right(row) if !finished => row,
            _ => bail!("the query holds more than one statement; pl db runs exactly one"),
        };
        for (index, values) in values.iter_mut().enumerate() {
            let value = row.try_get_raw(index)?;
            // A non-null value reports its own storage class, not the declared type.
            let cell = if value.is_null() {
                Cell::Null
            } else {
                match value.type_info().name() {
                    "INTEGER" => Cell::Integer(row.try_get_unchecked(index)?),
                    "REAL" => Cell::Real(row.try_get_unchecked(index)?),
                    "TEXT" => Cell::Text(row.try_get_unchecked(index)?),
                    "BLOB" => Cell::Blob(row.try_get_unchecked(index)?),
                    other => bail!(
                        "column {:?} holds an unexpected SQLite value of type {other}",
                        columns[index].name()
                    ),
                }
            };
            values.push(cell);
        }
        height += 1;
    }
    drop(steps);
    connection.close().await?;

    let series = columns
        .iter()
        .zip(values)
        .map(|(column, values)| values.finish(column.name()))
        .collect();
    super::frame(height, series)
}

/// One SQLite value.
enum Cell<'a> {
    Null,
    Integer(i64),
    Real(f64),
    Text(&'a str),
    Blob(&'a [u8]),
}

impl Cell<'_> {
    fn rank(&self) -> u8 {
        match self {
            Self::Null => 0,
            Self::Integer(_) => 1,
            Self::Real(_) => 2,
            Self::Text(_) => 3,
            Self::Blob(_) => 4,
        }
    }

    /// The value as SQLite's `CAST(value AS TEXT)` would give it.
    fn text(&self) -> Option<String> {
        match self {
            Self::Null | Self::Blob(_) => None,
            Self::Integer(value) => Some(value.to_string()),
            Self::Real(value) => Some(value.to_string()),
            Self::Text(value) => Some((*value).to_owned()),
        }
    }
}

/// A column's values in its widest storage class so far, in the order
/// null < integer < real < text < blob. Widening converts earlier values as SQLite's
/// `CAST` does: integers to reals, numbers to their text, and text to its bytes.
enum Values {
    Null(usize),
    Integer(Vec<Option<i64>>),
    Real(Vec<Option<f64>>),
    Text(Vec<Option<String>>),
    Blob(Vec<Option<Vec<u8>>>),
}

impl Values {
    /// Start from the declared type, so that an empty result still has typed columns.
    fn declared(type_name: &str) -> Self {
        match type_name {
            "INTEGER" => Self::Integer(Vec::new()),
            "REAL" => Self::Real(Vec::new()),
            "TEXT" => Self::Text(Vec::new()),
            "BLOB" => Self::Blob(Vec::new()),
            // NUMERIC, BOOLEAN, DATE, DATETIME, and expressions follow their values.
            _ => Self::Null(0),
        }
    }

    fn rank(&self) -> u8 {
        match self {
            Self::Null(_) => 0,
            Self::Integer(_) => 1,
            Self::Real(_) => 2,
            Self::Text(_) => 3,
            Self::Blob(_) => 4,
        }
    }

    fn push(&mut self, cell: Cell<'_>) {
        if cell.rank() > self.rank() {
            self.widen(cell.rank());
        }
        match self {
            Self::Null(len) => *len += 1,
            Self::Integer(values) => values.push(match cell {
                Cell::Integer(value) => Some(value),
                _ => None,
            }),
            Self::Real(values) => values.push(match cell {
                Cell::Integer(value) => Some(value as f64),
                Cell::Real(value) => Some(value),
                _ => None,
            }),
            Self::Text(values) => values.push(cell.text()),
            Self::Blob(values) => values.push(match cell {
                Cell::Blob(value) => Some(value.to_vec()),
                cell => cell.text().map(String::into_bytes),
            }),
        }
    }

    fn widen(&mut self, rank: u8) {
        let values = std::mem::replace(self, Self::Null(0));
        *self = match rank {
            1 => Self::Integer(vec![None; values.len()]),
            2 => Self::Real(values.into_reals()),
            3 => Self::Text(values.into_texts()),
            _ => Self::Blob(values.into_blobs()),
        };
    }

    fn len(&self) -> usize {
        match self {
            Self::Null(len) => *len,
            Self::Integer(values) => values.len(),
            Self::Real(values) => values.len(),
            Self::Text(values) => values.len(),
            Self::Blob(values) => values.len(),
        }
    }

    fn into_reals(self) -> Vec<Option<f64>> {
        match self {
            Self::Null(len) => vec![None; len],
            Self::Integer(values) => values
                .into_iter()
                .map(|value| value.map(|value| value as f64))
                .collect(),
            Self::Real(values) => values,
            Self::Text(_) | Self::Blob(_) => unreachable!("SQLite columns only widen"),
        }
    }

    fn into_texts(self) -> Vec<Option<String>> {
        match self {
            Self::Integer(values) => values
                .into_iter()
                .map(|value| value.map(|value| value.to_string()))
                .collect(),
            Self::Real(values) => values
                .into_iter()
                .map(|value| value.map(|value| value.to_string()))
                .collect(),
            Self::Null(len) => vec![None; len],
            Self::Text(values) => values,
            Self::Blob(_) => unreachable!("SQLite columns only widen"),
        }
    }

    fn into_blobs(self) -> Vec<Option<Vec<u8>>> {
        match self {
            Self::Blob(values) => values,
            values => values
                .into_texts()
                .into_iter()
                .map(|value| value.map(String::into_bytes))
                .collect(),
        }
    }

    fn finish(self, name: &str) -> Series {
        let name = name.into();
        match self {
            Self::Null(len) => Series::full_null(name, len, &DataType::Null),
            Self::Integer(values) => Series::new(name, values),
            Self::Real(values) => Series::new(name, values),
            Self::Text(values) => Series::new(name, values),
            Self::Blob(values) => Series::new(name, values),
        }
    }
}

#[cfg(test)]
mod tests {
    use polars::prelude::DataType;

    use super::{Cell, Values};

    #[test]
    fn mixed_integers_and_reals_become_floats() {
        let mut values = Values::Null(0);
        values.push(Cell::Null);
        values.push(Cell::Integer(2));
        values.push(Cell::Real(2.5));
        let series = values.finish("x");
        assert_eq!(series.dtype(), &DataType::Float64);
        assert_eq!(series.f64().unwrap().get(1), Some(2.0));
        assert_eq!(series.null_count(), 1);
    }

    #[test]
    fn text_absorbs_numbers_as_sqlite_casts_them() {
        let mut values = Values::declared("INTEGER");
        values.push(Cell::Integer(7));
        values.push(Cell::Text("n/a"));
        let series = values.finish("x");
        assert_eq!(series.dtype(), &DataType::String);
        assert_eq!(series.str().unwrap().get(0), Some("7"));
    }

    #[test]
    fn empty_results_keep_declared_types() {
        assert_eq!(
            Values::declared("TEXT").finish("x").dtype(),
            &DataType::String
        );
        assert_eq!(
            Values::declared("NUMERIC").finish("x").dtype(),
            &DataType::Null
        );
    }
}
