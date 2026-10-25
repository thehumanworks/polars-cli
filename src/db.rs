//! Read-only SQL queries against PostgreSQL, MySQL/MariaDB, and SQLite for `pl db`.
//!
//! This is the only module that opens network connections or handles credentials
//! (ADR 0003). Each backend runs one statement in a read-only session, streams its rows
//! into typed Polars columns, and closes the connection.

mod column;
mod mysql;
mod postgres;
mod sqlite;

use std::{collections::HashSet, future::Future, time::Duration};

use anyhow::{Context, bail};
use polars::prelude::{Column, DataFrame, Series};
use url::Url;

/// How long a server may take to accept a connection and finish the handshake.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Backend {
    Postgres,
    MySql,
    Sqlite,
}

/// Run one read-only statement, written in the database's own SQL dialect, and return
/// its result set.
///
/// `url` selects the database: `postgres://`, `postgresql://`, `mysql://`, `mariadb://`,
/// or `sqlite://PATH`. Errors never repeat the URL, which may contain a password.
pub fn query(url: &str, statement: &str) -> anyhow::Result<DataFrame> {
    let backend = backend(url)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to start the database client")?;
    runtime.block_on(async {
        match backend {
            Backend::Postgres => postgres::query(url, statement).await,
            Backend::MySql => mysql::query(url, statement).await,
            Backend::Sqlite => sqlite::query(url, statement).await,
        }
    })
}

fn backend(url: &str) -> anyhow::Result<Backend> {
    let scheme = url
        .split_once(':')
        .map(|(scheme, _)| scheme.to_ascii_lowercase());
    match scheme.as_deref() {
        Some("postgres" | "postgresql") => Ok(Backend::Postgres),
        Some("mysql" | "mariadb") => Ok(Backend::MySql),
        Some("sqlite") => Ok(Backend::Sqlite),
        _ => bail!(
            "unsupported database URL; use postgres://, mysql://, mariadb://, or sqlite://PATH"
        ),
    }
}

/// Reject URL parameters the driver does not know. It would ignore them silently, so a
/// misspelt `sslmode` would quietly weaken the connection.
fn check_parameters(url: &str, supported: &[&str]) -> anyhow::Result<()> {
    let url = Url::parse(url).context("invalid database URL")?;
    for (key, _) in url.query_pairs() {
        // `options[NAME]=VALUE` (PostgreSQL) is listed as `options[]`.
        let key = match key.split_once('[') {
            Some((base, rest)) if rest.ends_with(']') => format!("{base}[]"),
            _ => key.into_owned(),
        };
        if !supported.contains(&key.as_str()) {
            bail!(
                "unsupported connection parameter {key:?}; supported parameters: {}",
                supported.join(", ")
            );
        }
    }
    Ok(())
}

/// Connect within [`CONNECT_TIMEOUT`]. `target` names the server without credentials.
async fn connect<C>(
    target: &str,
    connecting: impl Future<Output = sqlx::Result<C>>,
) -> anyhow::Result<C> {
    match tokio::time::timeout(CONNECT_TIMEOUT, connecting).await {
        Ok(connection) => connection.with_context(|| format!("failed to connect to {target}")),
        Err(_) => bail!(
            "timed out after {}s connecting to {target}",
            CONNECT_TIMEOUT.as_secs()
        ),
    }
}

/// Check the result columns before any row is fetched.
fn check_columns<'a>(names: impl IntoIterator<Item = &'a str>) -> anyhow::Result<()> {
    let mut seen = HashSet::new();
    for name in names {
        if !seen.insert(name) {
            bail!(
                "the query returns more than one column named {name:?}; give each column a unique alias"
            );
        }
    }
    if seen.is_empty() {
        bail!("the statement returns no columns; pl db reads the result of a query such as SELECT");
    }
    Ok(())
}

fn frame(height: usize, columns: Vec<Series>) -> anyhow::Result<DataFrame> {
    DataFrame::new(height, columns.into_iter().map(Column::from).collect())
        .context("failed to assemble the query result")
}
