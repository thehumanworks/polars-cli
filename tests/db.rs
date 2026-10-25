//! `pl db` against SQLite, and against PostgreSQL and MySQL/MariaDB when
//! `PL_TEST_POSTGRES_URL` and `PL_TEST_MYSQL_URL` name servers to use. Each server test
//! creates its own tables, named after the test, so tests can run in parallel.
#![cfg(feature = "database")]

use std::path::Path;

use anyhow::Result;
use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::{Value, json};
use sqlx::{AssertSqlSafe, ConnectOptions, Connection, Executor};
use tempfile::TempDir;

fn pl() -> Result<Command> {
    let mut command = Command::cargo_bin("pl")?;
    command.env_remove("PL_DATABASE_URL");
    Ok(command)
}

/// Run `pl db QUERY --url URL --to json` and parse its output.
fn db_json(url: &str, query: &str) -> Result<Value> {
    let output = pl()?
        .args(["db", query, "--url", url, "--to", "json"])
        .output()?;
    assert!(
        output.status.success(),
        "pl db failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(serde_json::from_slice(&output.stdout)?)
}

/// The schema of a `pl db` result, read back from its Arrow output.
fn db_schema(url: &str, query: &str) -> Result<Value> {
    let arrow = pl()?
        .args(["db", query, "--url", url, "--to", "arrow"])
        .output()?;
    assert!(
        arrow.status.success(),
        "pl db failed: {}",
        String::from_utf8_lossy(&arrow.stderr)
    );
    let schema = pl()?
        .args(["schema", "--to", "json"])
        .write_stdin(arrow.stdout)
        .output()?;
    assert!(schema.status.success());
    let rows: Vec<Value> = serde_json::from_slice(&schema.stdout)?;
    Ok(Value::Object(
        rows.into_iter()
            .map(|row| {
                (
                    row["column"].as_str().unwrap().to_owned(),
                    row["dtype"].clone(),
                )
            })
            .collect(),
    ))
}

fn block_on<T>(future: impl Future<Output = Result<T>>) -> Result<T> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(future)
}

fn sqlite_fixture(dir: &TempDir, setup: &str) -> Result<String> {
    let path = dir.path().join("fixture.db");
    block_on(async {
        let mut connection = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true)
            .connect()
            .await?;
        connection.execute(AssertSqlSafe(setup)).await?;
        connection.close().await?;
        Ok(())
    })?;
    Ok(sqlite_url(&path))
}

fn sqlite_url(path: &Path) -> String {
    format!("sqlite://{}", path.display())
}

fn server_url(variable: &str) -> Option<String> {
    let url = std::env::var(variable).ok().filter(|url| !url.is_empty());
    if url.is_none() {
        eprintln!("skipped: set {variable} to run this test against a server");
    }
    url
}

fn postgres_setup(url: &str, setup: &str) -> Result<()> {
    block_on(async {
        let mut connection = sqlx::PgConnection::connect(url).await?;
        sqlx::raw_sql(AssertSqlSafe(setup))
            .execute(&mut connection)
            .await?;
        connection.close().await?;
        Ok(())
    })
}

fn mysql_setup(url: &str, setup: &str) -> Result<()> {
    block_on(async {
        let mut connection = sqlx::MySqlConnection::connect(url).await?;
        sqlx::raw_sql(AssertSqlSafe(setup))
            .execute(&mut connection)
            .await?;
        connection.close().await?;
        Ok(())
    })
}

#[test]
fn db_reads_sqlite_rows_with_their_storage_classes() -> Result<()> {
    let dir = TempDir::new()?;
    let url = sqlite_fixture(
        &dir,
        "CREATE TABLE items (id INTEGER, price REAL, name TEXT, note);
         INSERT INTO items VALUES (1, 2.5, 'Ada', NULL), (2, NULL, 'Grace', 'x'), (3, 4, NULL, 5);",
    )?;

    assert_eq!(
        db_json(&url, "SELECT * FROM items ORDER BY id")?,
        json!([
            {"id": 1, "price": 2.5, "name": "Ada", "note": null},
            {"id": 2, "price": null, "name": "Grace", "note": "x"},
            {"id": 3, "price": 4.0, "name": null, "note": "5"}
        ])
    );
    assert_eq!(
        db_schema(&url, "SELECT * FROM items")?,
        json!({"id": "i64", "price": "f64", "name": "str", "note": "str"})
    );
    Ok(())
}

#[test]
fn db_keeps_declared_sqlite_types_for_empty_results() -> Result<()> {
    let dir = TempDir::new()?;
    let url = sqlite_fixture(&dir, "CREATE TABLE items (id INTEGER, name TEXT);")?;

    assert_eq!(
        db_schema(&url, "SELECT id, name FROM items")?,
        json!({"id": "i64", "name": "str"})
    );
    Ok(())
}

#[test]
fn db_results_feed_pl_pipelines() -> Result<()> {
    let dir = TempDir::new()?;
    let url = sqlite_fixture(
        &dir,
        "CREATE TABLE orders (customer TEXT, total REAL);
         INSERT INTO orders VALUES ('ada', 10), ('bob', 5), ('ada', 2.5);",
    )?;
    let rows = pl()?
        .args(["db", "SELECT * FROM orders", "--url", &url])
        .output()?;
    assert!(rows.status.success());

    pl()?
        .args([
            "group-by",
            "customer",
            "--agg",
            "SUM(total) AS revenue",
            "--to",
            "csv",
        ])
        .write_stdin(rows.stdout)
        .assert()
        .success()
        .stdout("customer,revenue\nada,12.5\nbob,5.0\n");
    Ok(())
}

#[test]
fn db_results_join_local_files_in_one_polars_sql_query() -> Result<()> {
    let dir = TempDir::new()?;
    let url = sqlite_fixture(
        &dir,
        "CREATE TABLE orders (customer TEXT, total REAL);
         INSERT INTO orders VALUES ('ada', 10), ('bob', 5), ('ada', 2.5);",
    )?;
    let customers = dir.path().join("customers.csv");
    std::fs::write(&customers, "customer,region\nada,north\nbob,south\n")?;
    let orders = pl()?
        .args(["db", "SELECT * FROM orders", "--url", &url, "--to", "arrow"])
        .output()?;
    assert!(orders.status.success());

    pl()?
        .args([
            "query",
            &format!(
                "SELECT c.region, SUM(o.total) AS revenue FROM df AS o \
                 JOIN read_csv('{}') AS c ON o.customer = c.customer \
                 GROUP BY c.region ORDER BY c.region",
                customers.display()
            ),
            "--to",
            "csv",
        ])
        .write_stdin(orders.stdout)
        .assert()
        .success()
        .stdout("region,revenue\nnorth,12.5\nsouth,5.0\n");
    Ok(())
}

#[test]
fn db_reads_the_url_from_pl_database_url() -> Result<()> {
    let dir = TempDir::new()?;
    let url = sqlite_fixture(
        &dir,
        "CREATE TABLE t (n INTEGER); INSERT INTO t VALUES (7);",
    )?;

    pl()?
        .env("PL_DATABASE_URL", &url)
        .args(["db", "SELECT n FROM t", "--raw"])
        .assert()
        .success()
        .stdout("7\n");
    Ok(())
}

#[test]
fn db_reads_the_query_from_stdin_or_a_file() -> Result<()> {
    let dir = TempDir::new()?;
    let url = sqlite_fixture(
        &dir,
        "CREATE TABLE t (n INTEGER); INSERT INTO t VALUES (7);",
    )?;
    let query = dir.path().join("query.sql");
    std::fs::write(&query, "-- report\nSELECT n * 2 AS n FROM t\n")?;

    pl()?
        .args(["db", "-", "--url", &url, "--raw"])
        .write_stdin("SELECT n FROM t")
        .assert()
        .success()
        .stdout("7\n");
    pl()?
        .args([
            "db",
            &format!("@{}", query.display()),
            "--url",
            &url,
            "--raw",
        ])
        .assert()
        .success()
        .stdout("14\n");
    Ok(())
}

#[test]
fn db_never_writes_to_sqlite() -> Result<()> {
    let dir = TempDir::new()?;
    let url = sqlite_fixture(
        &dir,
        "CREATE TABLE t (n INTEGER); INSERT INTO t VALUES (1);",
    )?;

    pl()?
        .args(["db", "DELETE FROM t", "--url", &url])
        .assert()
        .failure()
        .stderr(predicate::str::contains("returns no columns"));
    pl()?
        .args(["db", "DELETE FROM t RETURNING n", "--url", &url])
        .assert()
        .failure()
        .stderr(predicate::str::contains("readonly"));
    assert_eq!(
        db_json(&url, "SELECT COUNT(*) AS n FROM t")?,
        json!([{"n": 1}])
    );
    Ok(())
}

#[test]
fn db_rejects_more_than_one_sqlite_statement() -> Result<()> {
    let dir = TempDir::new()?;
    let url = sqlite_fixture(
        &dir,
        "CREATE TABLE t (n INTEGER); INSERT INTO t VALUES (1);",
    )?;

    for query in [
        "SELECT n FROM t; SELECT 'x' AS s",
        "SELECT n FROM t; SELECT n FROM t WHERE 0",
    ] {
        pl()?
            .args(["db", query, "--url", &url])
            .assert()
            .failure()
            .stderr(predicate::str::contains("more than one statement"));
    }
    pl()?
        .args(["db", "SELECT n FROM t; -- done", "--url", &url, "--raw"])
        .assert()
        .success()
        .stdout("1\n");
    Ok(())
}

#[test]
fn db_does_not_create_missing_sqlite_files() -> Result<()> {
    let dir = TempDir::new()?;
    let path = dir.path().join("missing.db");

    pl()?
        .args(["db", "SELECT 1 AS n", "--url", &sqlite_url(&path)])
        .assert()
        .failure()
        .stderr(predicate::str::contains("failed to connect to"));
    assert!(!path.exists());
    Ok(())
}

#[test]
fn db_rejects_duplicate_result_column_names() -> Result<()> {
    let dir = TempDir::new()?;
    let url = sqlite_fixture(&dir, "CREATE TABLE t (n INTEGER);")?;

    pl()?
        .args(["db", "SELECT n, n FROM t", "--url", &url])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "more than one column named \"n\"; give each column a unique alias",
        ));
    Ok(())
}

#[test]
fn db_does_not_take_a_frame_input() -> Result<()> {
    pl()?
        .args(["db", "SELECT 1", "--url", "sqlite://x.db", "--data", "[]"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("does not take an input"));
    Ok(())
}

#[test]
fn db_errors_do_not_repeat_the_url_or_password() -> Result<()> {
    let secret = "s3cr3t-pa55";
    for url in [
        format!("redis://ada:{secret}@localhost/0"),
        format!("postgres://ada:{secret}@127.0.0.1:1/shop?sslmod=require"),
        format!("postgres://ada:{secret}@127.0.0.1:1/shop"),
        format!("mysql://ada:{secret}@127.0.0.1:1/shop"),
    ] {
        let output = pl()?.args(["db", "SELECT 1", "--url", &url]).output()?;
        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!stderr.contains(secret), "{url} leaked into: {stderr}");
    }
    Ok(())
}

#[test]
fn db_rejects_connection_parameters_the_driver_would_ignore() -> Result<()> {
    pl()?
        .args([
            "db",
            "SELECT 1",
            "--url",
            "postgres://ada@127.0.0.1:1/shop?sslmod=require",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "unsupported connection parameter \"sslmod\"",
        ));
    Ok(())
}

#[test]
fn db_help_hides_the_url_in_the_environment() -> Result<()> {
    pl()?
        .env("PL_DATABASE_URL", "postgres://ada:hunter2@db/shop")
        .args(["db", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("PL_DATABASE_URL"))
        .stdout(predicate::str::contains("hunter2").not());
    Ok(())
}

#[test]
fn db_maps_postgres_types_to_polars_types() -> Result<()> {
    let Some(url) = server_url("PL_TEST_POSTGRES_URL") else {
        return Ok(());
    };
    postgres_setup(
        &url,
        "DROP TABLE IF EXISTS pl_types;
         DROP DOMAIN IF EXISTS pl_positive;
         DROP TYPE IF EXISTS pl_mood;
         CREATE TYPE pl_mood AS ENUM ('happy', 'sad');
         CREATE DOMAIN pl_positive AS integer CHECK (VALUE > 0);
         CREATE TABLE pl_types (
             b boolean, i2 smallint, i4 integer, i8 bigint, f4 real, f8 double precision,
             amount numeric(10, 2), name text, code char(3), doc jsonb, id uuid, raw bytea,
             day date, at_time time, at timestamp, at_tz timestamptz, wait interval,
             tags text[], scores integer[], mood pl_mood, positive pl_positive
         );
         INSERT INTO pl_types VALUES (
             true, 2, 4, 8, 1.5, 2.25, 12345678.90, 'Ada', 'abc', '{\"k\": [1, 2]}',
             'a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11', '\\x0102', '2024-02-29', '13:05:09.5',
             '2024-02-29 13:05:09.123456', '2024-02-29 13:05:09+02', '1 day 02:00:00.5',
             ARRAY['a', NULL], '{}', 'sad', 7
         ), (NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL,
             NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL);",
    )?;
    let query = "SELECT * FROM pl_types ORDER BY i2";

    assert_eq!(
        db_schema(&url, query)?,
        json!({
            "b": "bool", "i2": "i16", "i4": "i32", "i8": "i64", "f4": "f32", "f8": "f64",
            "amount": "decimal[38,2]", "name": "str", "code": "str", "doc": "str",
            "id": "str", "raw": "binary", "day": "date", "at_time": "time",
            "at": "datetime[μs]", "at_tz": "datetime[μs, UTC]", "wait": "duration[μs]",
            "tags": "list[str]", "scores": "list[i32]", "mood": "str", "positive": "i32"
        })
    );
    let rows = db_json(&url, query)?;
    assert_eq!(rows[0]["amount"], json!("12345678.90"));
    assert_eq!(rows[0]["doc"], json!("{\"k\": [1, 2]}"));
    assert_eq!(rows[0]["id"], json!("a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11"));
    assert_eq!(rows[0]["at_tz"], json!("2024-02-29T11:05:09+00:00"));
    assert_eq!(rows[0]["raw"], json!("0102"));
    assert_eq!(rows[0]["tags"], json!(["a", null]));
    assert_eq!(rows[0]["scores"], json!([]));
    assert_eq!(rows[0]["mood"], json!("sad"));
    assert!(rows[1].as_object().unwrap().values().all(Value::is_null));
    Ok(())
}

#[test]
fn db_keeps_postgres_schemas_for_empty_results() -> Result<()> {
    let Some(url) = server_url("PL_TEST_POSTGRES_URL") else {
        return Ok(());
    };
    assert_eq!(
        db_schema(
            &url,
            "SELECT 1::bigint AS n, now() AS at, ARRAY[1.5]::numeric[] AS xs WHERE false"
        )?,
        json!({"n": "i64", "at": "datetime[μs, UTC]", "xs": "list[decimal[38,0]]"})
    );
    Ok(())
}

#[test]
fn db_runs_postgres_queries_read_only() -> Result<()> {
    let Some(url) = server_url("PL_TEST_POSTGRES_URL") else {
        return Ok(());
    };
    postgres_setup(
        &url,
        "DROP TABLE IF EXISTS pl_read_only; CREATE TABLE pl_read_only (n int); INSERT INTO pl_read_only VALUES (1);",
    )?;

    pl()?
        .args(["db", "DELETE FROM pl_read_only RETURNING n", "--url", &url])
        .assert()
        .failure()
        .stderr(predicate::str::contains("read-only transaction"));
    pl()?
        .args([
            "db",
            "SELECT 1 AS n; DELETE FROM pl_read_only",
            "--url",
            &url,
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("multiple commands"));
    assert_eq!(
        db_json(&url, "SELECT count(*) AS n FROM pl_read_only")?,
        json!([{"n": 1}])
    );
    Ok(())
}

#[test]
fn db_names_postgres_values_polars_cannot_hold() -> Result<()> {
    let Some(url) = server_url("PL_TEST_POSTGRES_URL") else {
        return Ok(());
    };
    for (query, message) in [
        (
            "SELECT '10.0.0.1'::inet AS ip",
            "column \"ip\" has PostgreSQL type INET, which pl db cannot read",
        ),
        (
            "SELECT 'infinity'::timestamp AS at",
            "infinite dates and timestamps",
        ),
        ("SELECT 'NaN'::numeric AS x", "NaN and infinite numerics"),
        ("SELECT interval '1 month' AS wait", "months or years"),
    ] {
        pl()?
            .args(["db", query, "--url", &url])
            .assert()
            .failure()
            .stderr(predicate::str::contains(message));
    }
    Ok(())
}

#[test]
fn db_maps_mysql_types_to_polars_types() -> Result<()> {
    let Some(url) = server_url("PL_TEST_MYSQL_URL") else {
        return Ok(());
    };
    mysql_setup(
        &url,
        "DROP TABLE IF EXISTS pl_types;
         CREATE TABLE pl_types (
             flag BOOLEAN, i1 TINYINT, u1 TINYINT UNSIGNED, i2 SMALLINT, i3 MEDIUMINT,
             i4 INT, u4 INT UNSIGNED, i8 BIGINT, u8 BIGINT UNSIGNED, f4 FLOAT, f8 DOUBLE,
             amount DECIMAL(10, 2), day DATE, at DATETIME(6), stamp TIMESTAMP NULL,
             wait TIME, yr YEAR, name VARCHAR(20), body TEXT, mood ENUM('happy', 'sad'),
             raw VARBINARY(4), bits BIT(10)
         );
         INSERT INTO pl_types VALUES (
             2, -1, 255, -2, -3, -4, 4000000000, -8, 18446744073709551615, 1.5, 2.25,
             12345678.90, '2024-02-29', '2024-02-29 13:05:09.123456', '2024-02-29 13:05:09',
             '-26:03:04', 2024, 'Ada', 'long text', 'sad', X'0102', b'1000000001'
         );
         INSERT INTO pl_types (flag) VALUES (NULL);",
    )?;
    let query = "SELECT * FROM pl_types ORDER BY flag DESC";

    assert_eq!(
        db_schema(&url, query)?,
        json!({
            "flag": "bool", "i1": "i8", "u1": "u8", "i2": "i16", "i3": "i32", "i4": "i32",
            "u4": "u32", "i8": "i64", "u8": "u64", "f4": "f32", "f8": "f64",
            "amount": "decimal[38,2]", "day": "date", "at": "datetime[μs]",
            "stamp": "datetime[μs, UTC]", "wait": "duration[μs]", "yr": "i16",
            "name": "str", "body": "str", "mood": "str", "raw": "binary", "bits": "u64"
        })
    );
    let rows = db_json(&url, query)?;
    assert_eq!(rows[0]["flag"], json!(true));
    assert_eq!(rows[0]["u8"], json!(18446744073709551615_u64));
    assert_eq!(rows[0]["amount"], json!("12345678.90"));
    assert_eq!(rows[0]["day"], json!("2024-02-29"));
    assert_eq!(rows[0]["at"], json!("2024-02-29 13:05:09.123456"));
    assert_eq!(rows[0]["bits"], json!(513));
    assert!(rows[1].as_object().unwrap().values().all(Value::is_null));
    Ok(())
}

#[test]
fn db_keeps_the_mysql_server_sql_mode() -> Result<()> {
    let Some(url) = server_url("PL_TEST_MYSQL_URL") else {
        return Ok(());
    };
    // With sqlx's default PIPES_AS_CONCAT this would be the string '10'.
    assert_eq!(db_json(&url, "SELECT 1 || 0 AS x")?, json!([{"x": 1}]));
    Ok(())
}

#[test]
fn db_runs_mysql_queries_read_only() -> Result<()> {
    let Some(url) = server_url("PL_TEST_MYSQL_URL") else {
        return Ok(());
    };
    mysql_setup(
        &url,
        "DROP TABLE IF EXISTS pl_read_only; CREATE TABLE pl_read_only (n INT); INSERT INTO pl_read_only VALUES (1);",
    )?;

    // MariaDB refuses to prepare the DELETE in a read-only transaction; MySQL prepares it,
    // and pl db refuses to run a statement that returns no columns.
    pl()?
        .args(["db", "DELETE FROM pl_read_only", "--url", &url])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("returns no columns")
                .or(predicate::str::contains("READ ONLY transaction")),
        );
    pl()?
        .args([
            "db",
            "SELECT 1 AS n; DELETE FROM pl_read_only",
            "--url",
            &url,
        ])
        .assert()
        .failure();
    assert_eq!(
        db_json(&url, "SELECT COUNT(*) AS n FROM pl_read_only")?,
        json!([{"n": 1}])
    );
    Ok(())
}
