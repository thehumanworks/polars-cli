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

The command vocabulary follows the useful core of Nushell's Polars plugin, but
the interface is designed for Bash and other Unix shells: stdin/stdout composition,
direct and inline inputs, raw scalar output, deterministic non-interactive
formats, useful exit statuses, and generated shell completions.

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

Known file extensions are authoritative. Stdin, inline values, and files with
unknown extensions are content-sniffed. Use `--from` whenever the content is
ambiguous, such as a one-column CSV.

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
`toml`, `markdown`, `table`, `text`, `html`, and `xml`.

`--raw` requires exactly one column and emits one newline-terminated value per row, which makes it
suitable for `read`, `mapfile`, `while read`, and command substitution.
`--pretty` formats JSON arrays. `--no-header` applies to CSV and TSV. `-o -`
means stdout.

The pipeline wire format is ordinary JSONL, so intermediate output can also be
consumed by `jq`, `awk`, or another process. A zero-row JSONL stream contains no
schema by definition; downstream `pl` transforms therefore propagate it as an
empty dataframe. Row counts remain correct, while a CSV header cannot be
reconstructed after that empty wire boundary.

## Commands

| Area | Commands |
| --- | --- |
| Read | `open` |
| Rows | `filter`/`where`, `first`/`head`, `last`/`tail`, `slice`, `sort-by`/`sort`, `unique`/`distinct`, `drop-nulls`, `reverse` |
| Columns | `select`/`get`, `with-column`/`mutate`, `drop`, `rename` |
| Aggregate | `group-by` |
| Combine | `join`, `concat` |
| SQL | `query`/`sql` |
| Inspect | `columns`, `schema`, `shape`, `count` |
| Render | `to` |
| Shell | `completions` |

Run `pl COMMAND --help` for arguments and aliases.

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

Join kinds are `inner`, `left`, `full`, and `cross`. Full joins coalesce
same-name key columns.

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
