# `pl` — Bash-native Polars pipelines

`pl` reads structured data into a Polars dataframe, applies one operation, and
writes the result back to the shell. Commands consume standard input by default,
so dataframe operations compose with ordinary Bash pipes:

```bash
pl open people.csv \
  | pl filter 'age >= 18' \
  | pl select name age \
  | pl sort-by age:desc \
  | pl to csv
```

The command vocabulary covers Nushell's Polars plugin, but the interface is
designed for Bash and other Unix shells: stdin/stdout composition, direct and
inline inputs, raw scalar output, deterministic non-interactive formats, useful
exit statuses, and generated shell completions. [docs/nushell-parity.md](docs/nushell-parity.md)
maps every Nushell `polars` command to its `pl` form.

## Install

```bash
cargo install --path .
```

This installs both `pl` and the longer `polars` alias. For local development:

```bash
cargo build --release
./target/release/pl --help
```

## Supply input

Every dataframe command accepts the same input forms. Global options may appear
before or after the subcommand.

```bash
# stdin
cat people.csv | pl filter 'age >= 18'

# explicit path
pl filter 'age >= 18' --input people.csv
pl --input people.csv filter 'age >= 18'

# leading direct input
pl people.csv filter 'age >= 18'

# open as the first pipeline stage
pl open people.csv | pl filter 'age >= 18'

# structured literal
pl select name --data '[{"name":"Ada","age":36}]' --raw
pl '[{"name":"Ada","age":36}]' select name --raw

# an explicit stdin token
pl open - < people.csv

# force a token to be treated as a path
pl open @records.data --from csv
```

`--data` is the unambiguous choice for literals that resemble paths or begin
with option syntax. `--from FORMAT` overrides extension and content detection.

`open` (alias `into-df`) also takes the reader options of Nushell's `polars open`:

```bash
pl open export.txt --type csv --delimiter ';' --skip-rows 2 --no-input-header
pl open big.csv --columns id,total --schema 'id:str,total:f64'
pl open zips.csv --infer-schema 0          # every column as text
pl open ragged.csv --truncate-ragged-lines
pl open columns.json --as-columns          # {"a":[1,2],"b":[3,4]} as two rows
```

`--delimiter`, `--no-input-header`, `--infer-schema`, `--skip-rows`, and
`--truncate-ragged-lines` apply to CSV/TSV only and are rejected for other
formats. `--columns` and `--schema` apply to every tabular format; `--schema`
also accepts a JSON object such as `'{"id":"str"}'`.

### Input formats

| Format | Names | Notes |
| --- | --- | --- |
| CSV | `csv` | Header row is required. |
| TSV | `tsv` | Header row is required. |
| JSON | `json` | Accepts an object, an array, or a scalar; scalars become a `value` column. |
| newline-delimited JSON | `jsonl`, `ndjson` | The default wire format between piped `pl` processes. |
| YAML | `yaml`, `yml` | Converted through the same row model as JSON. |
| TOML | `toml` | Arrays of tables become rows; nested tables are flattened where needed. |
| Markdown table | `markdown` | Reads the first table. |
| HTML table | `table` | Reads the first `<table>`. |
| HTML document | `html` | Document conversion is intentionally limited to Markdown output. |
| XML | `xml` | Repeated child elements become rows; nested fields are flattened. |
| text | `text` | Each line becomes a row in a `line` column. |
| Parquet | `parquet` | Binary; `.parquet` or `.pq`. |
| Arrow IPC | `arrow`, `ipc`, `feather` | Binary; `.arrow`, `.ipc`, or `.feather`. |
| Avro | `avro` | Binary; `.avro`. |

Known file extensions are authoritative. Stdin, inline values, and files with
unknown extensions are content-sniffed; Parquet, Arrow, and Avro are recognized
by their magic bytes. Use `--from` whenever the content is ambiguous, such as a
one-column CSV.

## Output and stream contract

```bash
pl people.csv select name age --to json
pl people.csv select name --raw
pl people.csv select name age --output people.yaml
pl people.csv select name age | pl to markdown
```

Output selection follows this precedence:

1. `--to FORMAT` (which overrides `pl to FORMAT` when both are supplied);
2. `pl to FORMAT`;
3. the extension of `--output PATH`;
4. a readable aligned table when stdout is a terminal;
5. JSONL when stdout is redirected or piped.

Supported outputs are `csv`, `tsv`, `json`, `jsonl`/`ndjson`, `yaml`/`yml`,
`toml`, `markdown`, `table`, `text`, `html`, `xml`, `parquet`, `arrow`
(`ipc`, `feather`), `avro`, and `repr` (Polars' own display format).

Binary formats are written verbatim to files and pipes, but `pl` refuses to
write them to a terminal. `save` writes a file and prints nothing:

```bash
pl open people.csv | pl save people.parquet
pl open people.csv | pl save people.avro --avro-compression snappy
pl open people.csv | pl save people.txt --type csv --csv-delimiter '|' --csv-no-header
pl open people.csv | pl to csv --output-delimiter ';'
```

Non-empty rendered output is a POSIX text stream: it ends in exactly one newline.
Formats that already terminate the last record (JSONL, CSV/TSV, `--raw`) are not
given a second one. Input is not start/end-trimmed, because leading blank text
lines and trailing spaces in `--raw` values are data.

`--raw` requires exactly one column and emits one newline-terminated value per row, which makes it
suitable for `read`, `mapfile`, `while read`, and command substitution.
`--pretty` formats JSON arrays. `--no-header` applies to CSV and TSV. `-o -`
means stdout.

The pipeline wire format is ordinary JSONL, so intermediate output can also be
consumed by `jq`, `awk`, or another process. A zero-row JSONL stream contains no
schema by definition; downstream `pl` transforms therefore propagate it as an
empty dataframe. Row counts remain correct, while a CSV header cannot be
reconstructed after that empty wire boundary.

### Typed pipelines

JSON carries dates, datetimes, and decimals as text. Date and time commands
accept ISO-8601 text, so most pipelines need nothing extra. When exact types
matter between stages, use Arrow (or Parquet) as the wire format; input format
detection recognizes it automatically, and even zero-row results keep their
schema:

```bash
pl open events.csv \
  | pl as-datetime '%d/%m/%Y %H:%M' at --time-zone UTC --to arrow \
  | pl convert-time-zone Europe/Paris at --to arrow \
  | pl filter "at >= '2024-01-01'" \
  | pl to csv
```

## Commands

| Area | Commands |
| --- | --- |
| Read/write | `open`/`into-df`, `save`, `to` |
| Rows | `filter`/`where`/`filter-with`, `first`/`head`, `last`/`tail`, `slice`, `take`, `sample`, `sort-by`/`sort`, `unique`/`distinct`/`drop-duplicates`, `drop-nulls`, `reverse`, `shift`, `with-row-index` |
| Columns | `select`/`get`, `with-column`/`mutate`, `drop`, `rename`, `cast`, `fill-null`, `fill-nan`, `set`, `set-with-idx`, `replace`, `cut`, `qcut`, `integer`, `decimal` |
| Aggregate | `group-by`, `sum`, `min`, `max`, `mean`, `median`, `std`, `var`, `quantile`, `entropy`, `count`, `len`, `count-null`, `n-unique`, `implode`, `all-true`, `all-false`, `value-counts`, `summary` |
| Combine | `join`, `join-where`, `concat`, `append` |
| Reshape | `pivot`, `unpivot`/`melt`, `explode`/`flatten`, `unnest`, `dummies` |
| Masks and indices | `is-null`, `is-not-null`, `is-duplicated`, `is-unique`, `is-in`, `not`, `arg-max`, `arg-min`, `arg-true`, `arg-unique`, `arg-sort`, `arg-where` |
| Strings | `uppercase`, `lowercase`, `contains`, `str-lengths`, `str-replace`, `str-replace-all`, `str-slice`, `str-split`, `str-strip-chars`, `str-join`, `concat-str` |
| Dates and times | `get-year`, `get-month`, `get-week`, `get-weekday`, `get-day`, `get-ordinal`, `get-hour`, `get-minute`, `get-second`, `get-nanosecond`, `datepart`, `strftime`, `as-date`, `as-datetime`, `convert-time-zone`, `replace-time-zone`, `truncate`, `date-range`, `datetime-range`, `date-ranges`, `datetime-ranges` |
| Numbers | `math abs\|cos\|sin\|exp\|log\|log1p\|sign\|sqrt\|dot\|bitwise-*`, `cumulative`, `rolling`, `horizontal` |
| Nested | `list-contains`, `struct-json-encode` |
| SQL | `query`/`sql` |
| Inspect | `columns`, `schema`, `shape` |
| Shell | `completions` |

Run `pl COMMAND --help` for arguments and aliases.

### Column selectors

`select`, `drop`, `unique`, `drop-nulls`, and every `COLUMN...` argument of the
column, aggregation, mask, and reshape commands accept selectors. Join keys,
`group-by` and `sort-by` keys, and options that name a single column stay plain
names or SQL expressions. An exact column name always wins, so headers that look
like selectors stay addressable.

| Selector | Selects |
| --- | --- |
| `name` | that column (missing names are an error) |
| `*` | every column |
| `'^regex$'` | columns whose names match, as in Polars |
| `type:GROUP[,GROUP...]` | `numeric`, `integer`, `signed-integer`, `unsigned-integer`, `float`, `decimal`, `string`, `boolean`, `temporal`, `date`, `datetime`, `duration`, `time`, `list`, `array`, `struct`, `nested`, `categorical`, `enum`, `binary`, `null`, or an exact dtype such as `type:i64` or `type:datetime<ms, UTC>` |
| `index:N[,N...]` | columns by position; negative positions count from the end |

```bash
pl orders.csv select type:numeric
pl orders.csv drop '^tmp_.*$'
pl orders.csv cast f64 type:integer
pl orders.csv select index:0,-1
```

### Column commands

Column ("series") commands such as `uppercase`, `cast`, `fill-null`,
`cumulative`, `get-year`, or `math sqrt` take optional `COLUMN...` selectors.
Without them every column is used, so a one-column input behaves exactly like a
Nushell series. Results replace their source columns in place, keeping the
frame's shape and column order. `--as NAME` adds the result as a new column
instead; with several columns, `{}` in NAME expands to each source name.

```bash
pl people.csv uppercase name
pl people.csv str-lengths name --as name_length
pl people.csv uppercase type:string --as '{}_upper'
pl events.csv get-year created_at --as year
pl prices.csv rolling mean 7 close --as close_7d
pl readings.csv fill-null --strategy forward
```

Values on the command line are parsed as JSON scalars when they are valid JSON
(`0`, `1.5`, `true`, `null`, `'"42"'`) and are plain strings otherwise. Value
lists are comma-separated or a JSON array: `pl is-in 'a,b'`, `pl cut '[0, 10, 100]'`.

### Deterministic output

Bash scripts compare, diff, and cache output, so `pl` never depends on hash
order: `group-by`, `value-counts`, `pivot`, and `unique` keep first-appearance
order, `sort-by` and `arg-sort` are stable, and `join` and `join-where` order
matches by left row, then right row (right joins by right row; unmatched right
rows of a full join come last). `sample` is reproducible with `--seed`.

## SQL expressions

`filter`, `select`, `with-column`, `sort-by`, and `group-by` use Polars SQL
expressions. Simple column names are quoted automatically. Pass complex
expressions as one shell argument:

```bash
pl orders.csv filter "status = 'paid' AND quantity > 0"
pl orders.csv select id customer 'price * quantity AS gross'
pl orders.csv with-column 'gross=price * quantity'
pl orders.csv sort-by gross:desc customer:asc
pl orders.csv group-by customer --agg 'SUM(price * quantity) AS revenue'
```

Use SQL identifier quotes for names containing spaces or punctuation:

```bash
pl people.csv select '"full name" AS name'
```

SQL covers Nushell's expression builders (`polars col`, `lit`, `as`, `when`,
`otherwise`, `expr-not`, `is-in`, `concat-str`, `over`):

```bash
pl orders.csv with-column "size=CASE WHEN quantity > 10 THEN 'bulk' ELSE 'unit' END"
pl orders.csv with-column 'customer_total=SUM(price * quantity) OVER (PARTITION BY customer)'
pl orders.csv filter "NOT (status IN ('void', 'test'))"
pl orders.csv select "'2024' AS batch" id
```

`query` registers the current input frame as `df`:

```bash
pl orders.csv query "
  SELECT customer, SUM(price * quantity) AS revenue
  FROM df
  WHERE status = 'paid'
  GROUP BY customer
  ORDER BY revenue DESC
" --to json
```

Each Bash stage materializes a dataframe and serializes JSONL. For large jobs,
a single `query` is usually faster and preserves Polars' optimizer across the
whole operation.

## Joins and concatenation

```bash
# Same-name keys; duplicate right-side non-key names gain a _right suffix.
pl customers.csv join orders.json --on customer_id --how left

# Differently named keys.
pl customers.csv join orders.json \
  --left-on customer_id \
  --right-on buyer_id \
  --how inner

# Multiple same-name keys can be repeated or comma-delimited.
pl left.csv join right.csv --on tenant_id,id

# Vertical concatenation.
pl concat january.csv february.csv march.csv --to csv
cat january.csv | pl concat february.csv | pl to csv
```

Join kinds are `inner`, `left`, `right`, `full`, `cross`, `semi`, and `anti`.
Full and right joins coalesce same-name key columns, and `--coalesce-columns`
does the same for differently named key pairs. `--suffix` (default `_right`)
renames clashing right-hand columns, and `--nulls-equal` lets null keys match.

```bash
# Inequality and range joins with a SQL predicate.
pl people.json join-where offers.json 'cash >= price'

# Rows that do or do not have a match.
pl customers.csv join orders.csv --on customer_id --how semi
pl customers.csv join orders.csv --on customer_id --how anti

# Align differently shaped inputs by column name.
pl concat --diagonal january.json february.json

# Side by side (Nushell `polars append`), or stacked with --col.
pl left.csv append right.csv
```

## Reshape and summarize

```bash
pl sales.csv pivot --on month --index region --values revenue --aggregate sum
pl wide.csv unpivot --index id --variable-name metric --value-name value
pl orders.json explode items | pl unnest items --separator .
pl people.csv value-counts team --sort
pl people.csv summary --quantiles 0.1,0.5,0.9
pl people.csv group-by team --agg 'AVG(age) AS mean_age' --agg 'COUNT(*) AS n'
pl prices.csv rolling --index-column day --period 7d --agg 'AVG(close) AS weekly'
pl date-range --start 2024-01-01 --end 2024-01-31 --interval 1w
```

## Replacing common jq/yq pipelines

`pl` is dataframe-first. It is a strong replacement when the input is a table or
a top-level collection of similarly shaped records. It is not a recursive,
general-purpose tree editor for arbitrary nested documents.

```bash
# jq -r '.[] | select(.active) | .name' people.json
pl open people.json \
  | pl filter active \
  | pl select name --raw

# jq '[.[] | {name, total: (.price * .quantity)}]' orders.json
pl open orders.json \
  | pl with-column 'total=price * quantity' \
  | pl select name total \
  | pl to json

# yq -o=json '.[] | select(.enabled)' services.yaml
pl open services.yaml \
  | pl filter enabled \
  | pl to json

# Convert and reshape without separate yq, jq, and csv tools.
pl services.yaml filter enabled \
  | pl select name port \
  | pl to csv
```

For a one-column result, prefer `--raw` over parsing JSON in a shell loop:

```bash
while IFS= read -r name; do
  printf 'service=%s\n' "$name"
done < <(pl services.yaml filter enabled | pl select name --raw)
```

## Shell completion

```bash
# Current Bash session
eval "$(pl completions bash)"

# Or generate scripts for bash, zsh, fish, elvish, or PowerShell.
pl completions zsh > ~/.zfunc/_pl
```

## Legacy compatibility

The original root conversion and multi-table SQL forms remain available:

```bash
pl records.csv --to json
pl people.csv scores.json \
  --sql 'SELECT people.name, scores.score FROM people JOIN scores USING (id)' \
  --to json
pl --sql 'SELECT * FROM cities' --table cities=raw.csv
```

Legacy `--sql` defaults to Markdown output. New pipelines should normally use
`query`, where the current frame is always named `df`.

## Development

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```
