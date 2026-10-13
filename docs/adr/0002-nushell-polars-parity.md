# ADR 0002: Nushell `polars` parity in a Bash CLI

## Status

Accepted

## Context

`pl` should offer what Nushell's `polars` plugin offers while remaining an
ordinary Unix filter. The plugin keeps dataframes, lazy frames, expressions,
selectors, and group-by objects as in-memory values that commands pass to each
other. Between Bash processes there are only bytes and arguments, so a literal
port is impossible and a careless one would make scripts fragile. Where breadth
and stability conflict, stability wins.

## Decisions

### Expressions are Polars SQL

Nushell's expression builders (`polars col`, `lit`, `as`, `when`, `otherwise`,
`expr-not`, `is-in`, `concat-str`, `over`) map to Polars SQL in `select`,
`with-column`, `filter`, `sort-by`, and `group-by --agg`. SQL is already the
expression language of `pl`, it is one shell argument, and Polars SQL supports
`CASE`, `IN`, window functions, and the date and string functions involved.
Operations SQL cannot express (regex replacement, rolling windows, cumulative
functions, binning, pivots, inequality joins) are dedicated commands.

### Series commands take column selectors

Nushell applies series commands to a one-column dataframe or an expression. In
`pl` they take optional `COLUMN...` selectors, defaulting to every column, and
replace each column in place so the frame keeps its shape. `--as NAME` writes a
new column instead, with `{}` expanding to the source name for several columns.
On a one-column input this is exactly Nushell's behavior, and on wider frames it
needs no `select` round trip.

Row-level masks (`is-duplicated`, `is-unique`) cannot replace one column, so
they output only the mask as Nushell does, unless `--as` adds it to the frame.
Aggregations (`sum`, `std`, `n-unique`, ...) reduce each selected column to one
row; when no column is named, unsupported columns become null, as in Polars,
while explicitly named unsupported columns are errors.

### Selectors are argument syntax

`polars selector …` values become tokens: `*`, `^regex$` (the Polars
convention), `type:GROUP` for dtype groups or exact dtypes, and `index:N`. An
exact column name always wins, so existing headers that happen to look like
selectors keep working. Missing exact names are errors; patterns that match
nothing select nothing, as in Polars.

### Output is deterministic

Scripts diff, cache, and test output. Polars' hash-based group-by and unstable
sort do not guarantee order, and `group-by` did return groups in a different
order on each run. Now `group-by`, `value-counts`, `pivot`, and `unique` keep
first-appearance order; `sort-by` and `arg-sort` are stable; `join` and
`join-where` order matches by left row, then right row (right joins by right
row; unmatched right rows of a full join come last).

### JSONL stays the default wire; Arrow is the typed wire

The JSONL default and the stream contract of ADR 0001 are unchanged. Parquet,
Arrow IPC, and Avro are new input and output formats, recognized on stdin by
magic bytes, so `--to arrow` gives exact types (and schemas of zero-row results)
between stages. Binary output is never written to a terminal. Because JSON
carries dates as text, temporal commands parse ISO-8601 strings, and casts from
text to temporal types use Polars' format-inferring parsers.

### Names follow Nushell unless they would break existing syntax

- `open --no-header` is spelled `--no-input-header`, because the global
  `--no-header` already means "omit the CSV output header".
- `integer --base/--dtype` and `math log --base` are flags rather than optional
  positionals, because `COLUMN...` follows them.
- Global short options remain `-i` and `-o`; commands never reuse them.
- Positionals that accept values starting with `-` are single-valued, so they
  cannot swallow later flags.

### Not ported

Lazy-frame plumbing (`into-lazy`, `collect`, `cache`, `profile`), the plugin
object store (`store-*`), closures (`map-batches`), cloud URLs, and hive
partitioning have no meaning across processes or would add network and
credential handling. Files are the store; one `pl query` keeps a whole plan in
Polars' optimizer.

### The command runs on a dedicated thread

Building over a hundred subcommand definitions needs a few MiB of stack in
unoptimized builds, more than the 1 MiB main-thread stack on Windows. `pl` runs
argument parsing and the command on a thread with a fixed 32 MiB stack
reservation.

## Consequences

Existing invocations keep their output, with these deliberate changes:

- `group-by` returns groups in first-appearance order instead of a random order.
- `sort-by` is guaranteed stable; null placement is unchanged.
- `join` output follows left rows, then right rows, instead of hash order.
- `select`, `drop`, `unique`, and `drop-nulls` accept selectors. Tokens that
  previously failed (`type:…`, `^…$`, `index:…`) now select columns, and exact
  column names containing spaces work without SQL quoting.
- `count COLUMN...` counts non-null values; plain `count` is unchanged.
- `rename OLD NEW` pairs are accepted alongside `OLD=NEW`.
- New command names can shadow a leading positional input that is an
  extensionless file with the same name; use `--input` or `./name`.
