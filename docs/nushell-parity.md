# Nushell `polars` plugin parity

This table maps every command of Nushell's `polars` plugin to its `pl` equivalent.
`pl` keeps a Bash-shaped interface, so the mapping follows a few consistent rules
instead of copying Nushell's value types:

- **Dataframes travel through pipes.** Each `pl` stage reads its input from stdin, a
  path, `--input`, or `--data`, and writes one result. JSONL is the default wire
  format; `--to arrow` (or `parquet`) keeps exact types between stages.
- **Expressions are Polars SQL.** Nushell builds expressions from `polars col`,
  `polars lit`, `polars when`, and friends. In `pl`, the same expressions are SQL
  strings passed to `select`, `with-column`, `filter`, `sort-by`, and `group-by --agg`.
- **Series commands take column arguments.** Nushell applies `polars uppercase` and
  similar commands to a one-column dataframe or an expression. In `pl` they take
  `COLUMN...` selectors (default: every column), replace those columns in place, and
  accept `--as NAME` to add a new column instead. On a one-column input this is
  exactly the Nushell behavior.
- **Databases are read with `pl db`.** The plugin has no database reader (Nushell
  reads SQLite with `query db`). `pl db QUERY` runs a read-only query on
  PostgreSQL, MySQL/MariaDB, or SQLite and emits the result as a frame; see
  [ADR 0003](adr/0003-database-sources.md).
- **Selectors are column arguments.** `polars selector …` values become selector
  tokens: `*`, `^regex$`, `type:GROUP`, and `index:N`. See
  [Column selectors](../README.md#column-selectors).

| Nushell | `pl` | Notes |
| --- | --- | --- |
| `polars open` | `pl open` | `-t/--type`, `-d/--delimiter`, `--infer-schema`, `--skip-rows`, `--columns`, `-s/--schema`, `--truncate-ragged-lines`. Nushell's `--no-header` is `--no-input-header`, because `--no-header` already controls CSV output. Reads CSV, TSV, JSON, JSONL/NDJSON, Parquet, Arrow IPC, and Avro, plus YAML, TOML, Markdown, HTML tables, XML, and text. Cloud URLs, hive partitioning, and `--eager` (every stage is eager) are not provided; databases are read with `pl db`. |
| `polars save` | `pl save PATH` | `-t/--type`, `--csv-delimiter`, `--csv-no-header`, `--avro-compression deflate\|snappy`. Prints nothing. `--output PATH` and `pl to FORMAT` remain available. |
| `polars into-df` | `pl into-df` | Alias of `open`. `--as-columns` reads `{"a":[…],"b":[…]}` as columns; `--schema` is `open -s`. |
| `polars into-nu` | `pl to FORMAT` | `--rows N` → `pl first N`; `--tail` → `pl last N`; `--index` → `pl with-row-index`. |
| `polars into-repr` | `pl to repr` | Polars' native display format. |
| `polars into-lazy`, `polars collect`, `polars cache` | — | Every `pl` process collects its result. Use one `pl query` when a whole plan should be optimized together. |
| `polars profile` | — | Not applicable to separate processes; time a stage with `time pl …`. |
| `polars store-get`, `store-ls`, `store-rm` | — | Files are the store: `pl save frame.arrow`, then `pl open frame.arrow`. |
| `polars into-schema`, `polars into-dtype` | `pl schema` | Types are spelled as strings wherever they are needed: `i64`, `str`, `datetime<ms, UTC>`, `list<f64>`, `decimal<10, 2>`. `pl schema` output can be pasted back. |
| `polars map-batches` | — | Closures do not cross process boundaries. Use SQL expressions, or pipe JSONL through `jq`, `awk`, or another program. |
| `polars columns` | `pl columns` | |
| `polars schema` | `pl schema` | |
| `polars shape` | `pl shape` | |
| `polars len` | `pl len` | One-row `len` column. |
| `polars count` | `pl count COLUMN...` | Non-null counts per column. Without columns, `pl count` keeps its row-count behavior. |
| `polars count-null` | `pl count-null` | |
| `polars n-unique` | `pl n-unique` | |
| `polars first` | `pl first [N]` | Alias `head`. |
| `polars last` | `pl last [N]` | Alias `tail`. |
| `polars slice` | `pl slice OFFSET N` | Negative offsets count from the end. |
| `polars take` | `pl take INDEX...` | Indices may be space- or comma-separated, or a JSON array; negative indices count from the end. |
| `polars sample` | `pl sample` | `-n/--n-rows`, `-f/--fraction`, `-s/--seed`, `-e/--replace`, `-u/--shuffle`. |
| `polars filter` | `pl filter PREDICATE` | SQL predicate. Alias `where`. |
| `polars filter-with` | `pl filter-with PREDICATE` | Alias of `filter`. |
| `polars unique` | `pl unique` | `-s/--subset`, `-l/--last`, `--keep first\|last\|any\|none`. Row order is always kept, so `--maintain-order` is accepted and has no further effect. |
| `polars drop-duplicates` | `pl drop-duplicates` | Alias of `unique`. |
| `polars drop-nulls` | `pl drop-nulls [COLUMN...]` | |
| `polars reverse` | `pl reverse` | |
| `polars sort-by` | `pl sort-by EXPR[:asc\|:desc]...` | `-r/--reverse`, `-n/--nulls-last`. Sorting is always stable, so `--maintain-order` is accepted and has no further effect. |
| `polars shift` | `pl shift N [COLUMN...]` | `-f/--fill VALUE`. Without columns, every column shifts. |
| `polars select` | `pl select EXPR...` | Columns, selectors, or SQL expressions. Alias `get`. |
| `polars get` | `pl get COLUMN...` | Alias of `select`. |
| `polars with-column` | `pl with-column NAME=EXPR...` | Alias `mutate`. |
| `polars drop` | `pl drop COLUMN...` | Accepts selectors, so `polars drop (polars selector string)` is `pl drop type:string`. |
| `polars rename` | `pl rename OLD NEW...` or `OLD=NEW...` | |
| `polars cast` | `pl cast DTYPE [COLUMN...]` | Strict by default; `--non-strict` turns failures into nulls. |
| `polars fill-null` | `pl fill-null VALUE [COLUMN...]` | Also `--strategy forward\|backward\|min\|max\|mean\|zero\|one` and `--limit`. Without columns, only columns compatible with VALUE are filled. |
| `polars fill-nan` | `pl fill-nan VALUE [COLUMN...]` | Float columns. |
| `polars explode` | `pl explode [COLUMN...]` | `--empty-as-null`, `--keep-nulls`. Defaults to every list column. |
| `polars flatten` | `pl flatten [COLUMN...]` | Alias of `explode`. |
| `polars unnest` | `pl unnest [COLUMN...]` | `-s/--separator`. Defaults to every struct column. |
| `polars implode` | `pl implode [COLUMN...]` | |
| `polars append` | `pl append OTHER` | Adds OTHER's columns (clashing names gain `_x`); `-c/--col` appends rows. |
| `polars concat` | `pl concat INPUT...` | `--diagonal`, `--to-supertypes`. With one INPUT, stdin is the first frame. |
| `polars join` | `pl join RIGHT --on KEY` | `--left-on`/`--right-on`, `--how inner\|left\|right\|full\|cross\|semi\|anti`, `-s/--suffix`, `--nulls-equal`, `--coalesce-columns`. Same-name keys are always merged; `--coalesce-columns` also merges differently named key pairs. |
| `polars join-where` | `pl join-where RIGHT PREDICATE` | SQL predicate; output is ordered by left row, then right row. |
| `polars query` | `pl query SQL` | The input is `df`. Alias `sql`. |
| `polars group-by` + `polars agg` | `pl group-by KEY... --agg 'SUM(x) AS s'` | Groups keep first-appearance order, so `--maintain-order` is accepted and has no further effect. |
| `polars agg-groups` | `pl with-row-index \| pl group-by k --agg 'ARRAY_AGG("index") AS groups'` | |
| `polars sum`, `min`, `max`, `mean`, `median` | `pl sum [COLUMN...]`, … | One row. Without columns, unsupported columns become null, as in Polars. |
| `polars std`, `polars var` | `pl std`, `pl var` | `--ddof` (default 1). |
| `polars quantile` | `pl quantile Q [COLUMN...]` | `--method nearest\|lower\|higher\|midpoint\|linear`. |
| `polars entropy` | `pl entropy [COLUMN...]` | `--base`, `--normalize true\|false`. |
| `polars all-true`, `polars all-false` | `pl all-true`, `pl all-false` | |
| `polars arg-max`, `polars arg-min` | `pl arg-max`, `pl arg-min` | One row of indices per column. |
| `polars arg-true` | `pl arg-true [COLUMN]` | `arg_true` column. |
| `polars arg-unique` | `pl arg-unique [COLUMN...]` | `arg_unique` column; several columns compare whole rows. |
| `polars arg-sort` | `pl arg-sort [COLUMN...]` | `-r/--reverse`, `-n/--nulls-last`, `-l/--limit`. |
| `polars arg-where` | `pl arg-where PREDICATE` | SQL predicate; `arg_where` column. |
| `polars value-counts` | `pl value-counts [COLUMN...]` | `-c/--column NAME`, `-s/--sort`, `-n/--normalize`. Values keep first-appearance order unless sorted. |
| `polars summary` | `pl summary` | Same layout: `descriptor` plus `NAME (dtype)` columns; `-q/--quantiles`. |
| `polars pivot` | `pl pivot --on COL --index COL --values COL` | `-a/--aggregate first\|last\|sum\|min\|max\|mean\|median\|count`, `-p/--separator`. Rows and new columns keep first-appearance order. |
| `polars unpivot` | `pl unpivot --index COL [--on COL]` | `--variable-name`, `--value-name`. Alias `melt`. |
| `polars dummies` | `pl dummies [COLUMN...]` | `-d/--drop-first`, `-n/--drop-nulls`, `-s/--separator SEP`. |
| `polars is-null`, `polars is-not-null` | `pl is-null`, `pl is-not-null` | Masks replace the column in place, or use `--as NAME`. |
| `polars is-duplicated`, `polars is-unique` | `pl is-duplicated`, `pl is-unique` | Row-level over the selected columns. Outputs only the mask, as Nushell does, unless `--as NAME` adds it to the frame. |
| `polars is-in` | `pl is-in VALUES [COLUMN...]` | VALUES is comma-separated or a JSON array; `--nulls-equal`. In SQL: `x IN (1, 2)`. |
| `polars not`, `polars expr-not` | `pl not [COLUMN...]` | In SQL: `NOT predicate`. |
| `polars set` | `pl set VALUE [COLUMN...] --mask PREDICATE` | The mask is a SQL predicate. |
| `polars set-with-idx` | `pl set-with-idx VALUE [COLUMN...] --indices 0,2` | |
| `polars col` | column names, selectors | `polars col a` is `a`; `polars col '*'` is `*`; regex names use `^…$`. |
| `polars lit` | SQL literals | `pl select "'label' AS kind" name`. |
| `polars as` | `AS` in SQL, or `--as NAME` | |
| `polars when`, `polars otherwise` | `CASE WHEN … THEN … ELSE … END` | `pl with-column "size=CASE WHEN n > 10 THEN 'big' ELSE 'small' END"`. |
| `polars over` | SQL window functions | `pl with-column 'total=SUM(x) OVER (PARTITION BY g)'`. |
| `polars horizontal` | `pl horizontal all\|any\|min\|max\|sum\|mean COLUMN...` | Adds a column named after the operation, or `--as NAME`; `-n/--nulls` propagates nulls. |
| `polars concat-str` | `pl concat-str SEP COLUMN... --as NAME` | `--ignore-nulls`. In SQL: `CONCAT_WS(sep, a, b)`. |
| `polars replace` | `pl replace OLD NEW [COLUMN...]` or `--map JSON` | `-s/--strict`, `-d/--default`, `-t/--return-dtype`. |
| `polars cut` | `pl cut BREAKS [COLUMN...]` | `-l/--labels`, `-c/--left-closed`, `-b/--include-breaks`. |
| `polars qcut` | `pl qcut N\|QUANTILES [COLUMN...]` | `-l/--labels`, `-c/--left-closed`, `-b/--include-breaks`, `-d/--allow-duplicates`. |
| `polars cumulative` | `pl cumulative sum\|min\|max\|count\|product [COLUMN...]` | `-r/--reverse`. |
| `polars rolling` | `pl rolling TYPE WINDOW [COLUMN...]` | TYPE is `sum`, `min`, `max`, `mean`, `median`, `std`, or `var`; `--min-periods`, `--center`. The lazyframe form is `pl rolling --index-column t --period 7d --agg 'SUM(x) AS s'` with `--offset`, `-c/--closed`, `-g/--group-by`. |
| `polars math abs`, `cos`, `sin`, `exp`, `log`, `log1p`, `sign`, `sqrt` | `pl math abs`, … | `pl math log --base B`. |
| `polars math dot` | `pl math dot LEFT RIGHT` | One-row `dot` column. |
| `polars math bitwise-and`, `bitwise-or`, `bitwise-xor` | `pl math bitwise-and`, … | Aggregations. |
| `polars math bitwise-count-ones`, `-count-zeros`, `-leading-ones`, `-leading-zeros`, `-trailing-ones`, `-trailing-zeros` | `pl math bitwise-count-ones`, … | |
| `polars integer` | `pl integer [COLUMN...]` | `--base`, `--dtype`, `-s/--strict`. Nushell's positional base and dtype are flags here, because COLUMN follows. |
| `polars decimal` | `pl decimal SCALE [COLUMN...]` | |
| `polars uppercase`, `polars lowercase` | `pl uppercase`, `pl lowercase` | |
| `polars contains` | `pl contains PATTERN [COLUMN...]` | Regex; `--literal`. |
| `polars str-lengths` | `pl str-lengths [COLUMN...]` | `-b/--bytes`. |
| `polars str-replace`, `polars str-replace-all` | `pl str-replace -p PATTERN -r REPLACEMENT` | `--literal`. |
| `polars str-slice` | `pl str-slice START [COLUMN...]` | `-l/--length`; negative START counts from the end. |
| `polars str-split` | `pl str-split SEPARATOR [COLUMN...]` | Produces list columns. |
| `polars str-strip-chars` | `pl str-strip-chars CHARS [COLUMN...]` | `-s/--start`, `-e/--end`, `-w/--whitespace`. |
| `polars str-join` | `pl str-join [COLUMN...]` | `-d/--delimiter`, `-n/--ignore-nulls`. |
| `polars struct-json-encode` | `pl struct-json-encode [COLUMN...]` | |
| `polars list-contains` | `pl list-contains ELEMENT [COLUMN...]` | |
| `polars get-year`, `get-month`, `get-week`, `get-weekday`, `get-day`, `get-ordinal`, `get-hour`, `get-minute`, `get-second`, `get-nanosecond` | same names | ISO-8601 strings are parsed, so JSONL pipelines work without a cast. |
| `polars datepart` | `pl datepart PART [COLUMN...]` | year, quarter, month, week, weekday, day, ordinal, hour, minute, second, millisecond, microsecond, nanosecond. |
| `polars strftime` | `pl strftime FORMAT [COLUMN...]` | |
| `polars as-date` | `pl as-date FORMAT [COLUMN...]` | `-n/--not-exact`. |
| `polars as-datetime` | `pl as-datetime FORMAT [COLUMN...]` | `-n/--not-exact`, `--naive`, `-a/--ambiguous`, `--time-unit`, `--time-zone`. |
| `polars convert-time-zone` | `pl convert-time-zone ZONE [COLUMN...]` | `null` converts to naive UTC. |
| `polars replace-time-zone` | `pl replace-time-zone ZONE [COLUMN...]` | `-a/--ambiguous`, `-n/--nonexistent`; `null` removes the zone. |
| `polars truncate` | `pl truncate EVERY [COLUMN...]` | Durations such as `1d`, `15m`, `1mo`. |
| `polars date-range`, `polars datetime-range` | `pl date-range --start … --end …` | `--interval`, `-c/--closed`, `--as`; `datetime-range` adds `--time-unit` and `--time-zone`. Reads no input. |
| `polars date-ranges`, `polars datetime-ranges` | `pl date-ranges --start COL --end COL` | Adds a list column (`--as`); bounds are columns or literals. |
| `polars selector all` | `*` | |
| `polars selector by-name` | column names | |
| `polars selector by-dtype` | `type:i64,f64` | Any dtype string, such as `type:datetime<ms, UTC>` or `type:array<i64, 3>`. |
| `polars selector by-index` | `index:0,-1` | |
| `polars selector first`, `polars selector last` | `index:0`, `index:-1` | |
| `polars selector exclude`, `polars selector not` | `pl drop SELECTOR`, or `pl select '* EXCLUDE (a, b)'` | |
| `polars selector matches` | `'^.*PATTERN.*$'` | Selector regexes are anchored with `^…$`, as in Polars. |
| `polars selector starts-with`, `ends-with`, `contains` | `'^prefix.*$'`, `'^.*suffix$'`, `'^.*text.*$'` | |
| `polars selector alpha`, `alphanumeric`, `digit` | `'^\p{Alphabetic}+$'`, `'^[\p{Alphabetic}\p{Nd}]+$'`, `'^\p{Nd}+$'` | Use `[[:alpha:]]`-style classes for ASCII only. |
| `polars selector numeric`, `integer`, `signed-integer`, `unsigned-integer`, `float`, `decimal`, `string`, `boolean`, `temporal`, `date`, `datetime`, `duration`, `list`, `array`, `struct`, `nested`, `categorical`, `enum`, `binary` | `type:numeric`, `type:integer`, … | `selector string --include-categorical` is `type:string,categorical`. |
| `polars selector object`, `polars selector empty` | — | `pl` has no object columns; an empty selection is never needed in a pipe. |
