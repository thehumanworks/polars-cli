# ADR 0003: Database sources through `pl db`

## Status

Accepted. Reverses the part of ADR 0002 that kept network access and credential
handling out of `pl`; that exclusion still holds for every command except `pl db`.

## Context

ADR 0002 left out cloud URLs and the plugin's object store because they "would
add network and credential handling", and settled on "files are the store". The
data people want to analyse often lives in PostgreSQL or MySQL instead. Getting it
into `pl` meant a second client and a text hop:

```bash
psql "$DATABASE_URL" --csv -c 'SELECT …' | pl --from csv …
```

That works, but it loses types (CSV has no decimals, timestamps with zones,
binary, or lists, and an empty result has no schema). It also needs `psql` or
`mysql` installed, and it splits one task between tools with different quoting,
TLS, and credential conventions. Python Polars solves this with
`read_database_uri`; the Rust crate has no database reader, and Polars SQL's table
functions read only files (`read_csv`, `read_parquet`, `read_ipc`, `read_json`).

The Polars authors' own CLI, `pola-rs/polars-cli` (archived in 2025 for lack of
maintainers), took the SQL-first route: one `SQLContext`, sources pulled in by
those table functions, and a `.save`/`.open` session file. It never read
databases; its request for SQLite queries (issue #64) was still open when it was
archived.

## Decisions

### One command owns the network

`pl db QUERY` runs one statement on a database and emits its result like any
other command that produces a frame: a table on a terminal, JSONL in a pipe, and
`--to`/`--output` as usual. It is the only command that opens network
connections or reads credentials. Other inputs (`--input`, `join`, `concat`, and
the legacy `--table`) do not accept database URLs, so every place a script touches
the network or a credential is a visible `pl db`.

The code lives in `src/db/` behind the `database` cargo feature. The feature is
on by default, so release archives, `cargo install`, and the Nix package include
it. `cargo build --no-default-features` builds the offline `pl` that ADR 0002
described.

### The database runs the query; Polars runs everything after it

QUERY is in the database's own SQL dialect and is sent unchanged, so the filters,
joins, and aggregations the server can do run where the data and its indexes are.
The result becomes a Polars frame, and later stages (pivots, rolling windows,
joins with local files, conversion to Parquet) run in Polars. `pl` does not
translate Polars SQL into a server dialect, and it never reads whole tables
implicitly.

Polars SQL stays the engine across sources, as in the archived CLI, without a new
mechanism. Piped into `pl query`, a `pl db` result is `df`, and the table
functions join it with files in one Polars plan. A second database's result joins
the same way after `pl db … --to arrow > file.arrow`. Database tables are not
registered in Polars SQL directly: that would put credentials into another
command, and it would hide which part of the plan runs on the server.

### Read-only, one statement

- PostgreSQL runs the statement in `BEGIN READ ONLY` and MySQL/MariaDB in
  `START TRANSACTION READ ONLY`; both roll back. A transaction, unlike a session
  setting, holds through transaction-pooling proxies such as PgBouncer. Before
  the transaction ends, `pl db` closes the prepared statements it created, because
  a pooler hands the server connection to other clients.
- SQLite opens the file read-only and never creates it.
- The statement is prepared before it runs. PostgreSQL and MySQL refuse to
  prepare more than one statement; for SQLite, `pl db` fails as soon as a second
  statement appears. A statement that returns no columns (INSERT, UPDATE, DDL) is
  rejected before it executes.

This guards against accidents. It is not a security boundary: give the database
role only the privileges it needs.

### Credentials stay in the environment and out of output

- The URL comes from `--url` or `PL_DATABASE_URL`, and `--help` never shows the
  variable's value. Arguments are visible to other users in `ps` and are saved in
  shell history, so URLs that contain passwords belong in the variable.
- PostgreSQL also follows libpq's conventions: `PGPASSWORD`, `PGHOST` and the
  other `PG*` variables, and `~/.pgpass`, so a URL does not need a password.
- Messages name the server as `scheme://user@host:port/database` and never repeat
  the URL or the password.
- Nothing is stored, cached, or logged. Each invocation connects, runs, and
  disconnects.

### TLS follows each database's own client

TLS uses rustls with the operating system's trust store, on the same ring crypto
backend that Polars already links. Modes and their defaults match `psql` and
`mysql`:

- PostgreSQL: `sslmode=disable|prefer|require|verify-ca|verify-full` (default
  `prefer`), with `sslrootcert`, `sslcert`, and `sslkey`.
- MySQL: `ssl-mode=DISABLED|PREFERRED|REQUIRED|VERIFY_CA|VERIFY_IDENTITY` (default
  `PREFERRED`), with `ssl-ca`, `ssl-cert`, and `ssl-key`.

Use `verify-full` / `VERIFY_IDENTITY` for servers across a network. Unknown URL
parameters are errors, because the driver would otherwise ignore them and a
misspelt `sslmode` would silently weaken the connection.

MySQL 8's `caching_sha2_password` needs either TLS or RSA key exchange to check a
password that the server has not cached. `pl` is built without RSA (the `rsa`
crate carries RUSTSEC-2023-0071), so such logins need a TLS-capable server, which
the default `PREFERRED` mode then uses.

### Types come from the statement, and lossy values are errors

Each column is typed from the prepared statement's metadata before any row
arrives, so an empty result keeps its schema. Values are decoded straight into
Polars builders:

| Polars | PostgreSQL | MySQL/MariaDB |
| --- | --- | --- |
| `bool` | `boolean` | `BOOL`/`BOOLEAN` (nonzero is true) |
| `i8` | — | `TINYINT` |
| `i16` | `smallint` | `SMALLINT`, `YEAR` |
| `i32` | `integer` | `MEDIUMINT`, `INT` |
| `i64` | `bigint` | `BIGINT` |
| `u8` … `u64` | `oid` (`u32`) | the `UNSIGNED` integer types; `BIT(n)` (`u64`) |
| `f32`, `f64` | `real`, `double precision` | `FLOAT`, `DOUBLE` |
| `decimal[38, s]` | `numeric` | `DECIMAL` |
| `str` | text types, `json`, `jsonb`, `xml`, `uuid`, enums, `citext` | `CHAR`, `VARCHAR`, the `TEXT` types, `ENUM`, `SET`, `JSON` |
| `binary` | `bytea` | `BINARY`, `VARBINARY`, the `BLOB` types, `GEOMETRY` |
| `date` | `date` | `DATE` |
| `time` | `time` | — |
| `datetime[μs]` | `timestamp` | `DATETIME` |
| `datetime[μs, UTC]` | `timestamptz` | `TIMESTAMP` |
| `duration[μs]` | `interval` | `TIME`, which is a signed duration rather than a time of day |
| `list[…]` | one-dimensional arrays of the types above | — |

A decimal column's scale `s` is the largest scale among its values, and
PostgreSQL domains read as their base type.

SQLite types each value rather than each column, so a column takes the widest
storage class among its values (integer < real < text < blob), converted the way
SQLite's `CAST` would convert it. Columns with no values keep their declared type.

A value that Polars cannot represent exactly is an error that names the column
and suggests a cast in the query. Examples are numerics wider than 38 digits, NaN
numerics, infinite dates, intervals with months, multi-dimensional arrays, and
types outside the table such as `inet`. Values are never converted silently. The
one exception is MySQL's zero date (`0000-00-00`), which becomes null: MySQL uses
it to mean "no date".

Sessions run in UTC (PostgreSQL `TimeZone`, MySQL `time_zone`). That makes
`now()` and `CURRENT_DATE` independent of the server's configuration, which keeps
output deterministic as ADR 0002 requires, and it lets MySQL `TIMESTAMP` columns
be typed as UTC. MySQL keeps the server's `sql_mode`, so `||` means what it means
in the `mysql` client.

### The driver is sqlx

sqlx 0.9 (`postgres`, `mysql`, `sqlite-bundled`; no macros, migrations, or `Any`
driver) provides libpq-compatible URL, TLS, and `.pgpass` handling for PostgreSQL,
the MySQL protocol, and a bundled SQLite. The release binaries therefore need no
system libraries on Linux, macOS, or Windows. tokio and rustls were already in the
dependency tree through Polars. Each invocation runs a single-threaded runtime,
and rows stream into the column builders, so memory use is about the size of the
result frame.

## Consequences

- `pl` can open network connections, but only in `pl db`. Auditing a script for
  network access or credential use means looking for `pl db`.
- Release binaries are about 5% larger (on x86_64 Linux, 101 MB to 106 MB
  stripped), because the drivers and a bundled SQLite are compiled in.
- In JSONL, the default wire format, decimals and timestamps travel as text, as
  ADR 0002 describes. Pipelines that compute on them should take
  `pl db … --to arrow`, or cast in the query. JSON, JSONL, CSV, TSV, YAML, and
  TOML output now carry binary values as lowercase hex, because Polars cannot
  write binary to JSON or CSV.
- The URL parameter allow-lists must grow when sqlx adds parameters.
- Not provided: writes, bind parameters (QUERY is the caller's own SQL, sent as
  written; scripts that interpolate untrusted input into it must quote it
  themselves), other databases (SQL Server, Oracle, ClickHouse, DuckDB), query
  cancellation on Ctrl-C beyond closing the connection, and RSA password exchange
  for MySQL.
- `tests/db.rs` always runs its SQLite tests. It runs the PostgreSQL and MySQL
  tests when `PL_TEST_POSTGRES_URL` and `PL_TEST_MYSQL_URL` name servers, and CI
  provides both.
