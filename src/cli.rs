use std::{
    io::{IsTerminal, Read, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, bail};
use clap::{ArgAction, Args, CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::{Shell, generate};
use polars::{
    frame::DataFrame,
    io::avro::AvroCompression,
    prelude::{DataType, QuantileMethod},
};
use serde_json::Value as JsonValue;

use crate::{
    ops::{
        self, JoinKind, JoinOptions, KeepStrategy,
        aggregate::{self, Aggregation},
        column::{
            self, BinOptions, Cumulative, FillStrategy, Horizontal, MathOp, Quantiles,
            ReplaceOptions, Rolling, Target,
        },
        reshape::{self, PivotOptions, RollingBy, SampleSize},
        strings::{self, StripSide},
        temporal::{self, AsDatetime, DatePart, DatetimeSpec, RangeSpec},
    },
    read::{
        InputFormat, ReadOptions, detect_input_format, detect_input_format_from_bytes,
        read_bytes_with_options,
    },
    selector,
    sql::SqlEngine,
    transformer::{
        JsonFormat, html_document_to_markdown, to_arrow, to_avro, to_delimited, to_html, to_json,
        to_markdown, to_parquet, to_pretty_json, to_repr, to_table, to_text_lines, to_toml, to_xml,
        to_yaml,
    },
    value::{
        parse_dtype, parse_f64_list, parse_index_list, parse_list, parse_scalar, parse_string_list,
        parse_time_unit, parse_time_zone, split_top_level_commas,
    },
};

const EXAMPLES: &str = r#"Examples:
  pl open people.csv | pl filter 'age >= 18' | pl select name age
  pl filter 'total > 100' --input orders.csv --to csv
  pl select name --data '[{"name":"Ada","age":36}]' --raw
  pl query 'SELECT team, COUNT(*) AS n FROM df GROUP BY team' -i people.yaml
  pl open events.csv | pl get-year created_at --as year | pl value-counts year
  pl open sales.parquet | pl pivot --on month --index region --values revenue
  pl completions bash

Column arguments accept names, *, ^regex$, type:GROUP (numeric, string, ...), and index:N.
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    Csv,
    Tsv,
    Json,
    #[value(alias = "ndjson")]
    Jsonl,
    #[value(alias = "yml")]
    Yaml,
    Toml,
    Markdown,
    Table,
    Text,
    Html,
    Xml,
    Parquet,
    #[value(alias = "ipc", alias = "feather")]
    Arrow,
    Avro,
    Repr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum KeepArg {
    First,
    Last,
    Any,
    None,
}

impl From<KeepArg> for KeepStrategy {
    fn from(value: KeepArg) -> Self {
        match value {
            KeepArg::First => Self::First,
            KeepArg::Last => Self::Last,
            KeepArg::Any => Self::Any,
            KeepArg::None => Self::None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum JoinType {
    Inner,
    Left,
    Right,
    Full,
    Cross,
    Semi,
    Anti,
}

impl From<JoinType> for JoinKind {
    fn from(value: JoinType) -> Self {
        match value {
            JoinType::Inner => Self::Inner,
            JoinType::Left => Self::Left,
            JoinType::Right => Self::Right,
            JoinType::Full => Self::Full,
            JoinType::Cross => Self::Cross,
            JoinType::Semi => Self::Semi,
            JoinType::Anti => Self::Anti,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum AvroCompressionArg {
    Deflate,
    Snappy,
}

impl From<AvroCompressionArg> for AvroCompression {
    fn from(value: AvroCompressionArg) -> Self {
        match value {
            AvroCompressionArg::Deflate => Self::Deflate,
            AvroCompressionArg::Snappy => Self::Snappy,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum FillStrategyArg {
    Forward,
    Backward,
    Min,
    Max,
    Mean,
    Zero,
    One,
}

impl From<FillStrategyArg> for FillStrategy {
    fn from(value: FillStrategyArg) -> Self {
        match value {
            FillStrategyArg::Forward => Self::Forward,
            FillStrategyArg::Backward => Self::Backward,
            FillStrategyArg::Min => Self::Min,
            FillStrategyArg::Max => Self::Max,
            FillStrategyArg::Mean => Self::Mean,
            FillStrategyArg::Zero => Self::Zero,
            FillStrategyArg::One => Self::One,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum CumulativeArg {
    Sum,
    Min,
    Max,
    Count,
    Product,
}

impl From<CumulativeArg> for Cumulative {
    fn from(value: CumulativeArg) -> Self {
        match value {
            CumulativeArg::Sum => Self::Sum,
            CumulativeArg::Min => Self::Min,
            CumulativeArg::Max => Self::Max,
            CumulativeArg::Count => Self::Count,
            CumulativeArg::Product => Self::Product,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum RollingArg {
    Sum,
    Min,
    Max,
    Mean,
    Median,
    Std,
    Var,
}

impl From<RollingArg> for Rolling {
    fn from(value: RollingArg) -> Self {
        match value {
            RollingArg::Sum => Self::Sum,
            RollingArg::Min => Self::Min,
            RollingArg::Max => Self::Max,
            RollingArg::Mean => Self::Mean,
            RollingArg::Median => Self::Median,
            RollingArg::Std => Self::Std,
            RollingArg::Var => Self::Var,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum HorizontalArg {
    All,
    Any,
    Min,
    Max,
    Sum,
    Mean,
}

impl From<HorizontalArg> for Horizontal {
    fn from(value: HorizontalArg) -> Self {
        match value {
            HorizontalArg::All => Self::All,
            HorizontalArg::Any => Self::Any,
            HorizontalArg::Min => Self::Min,
            HorizontalArg::Max => Self::Max,
            HorizontalArg::Sum => Self::Sum,
            HorizontalArg::Mean => Self::Mean,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum QuantileMethodArg {
    Nearest,
    Lower,
    Higher,
    Midpoint,
    Linear,
}

impl From<QuantileMethodArg> for QuantileMethod {
    fn from(value: QuantileMethodArg) -> Self {
        match value {
            QuantileMethodArg::Nearest => Self::Nearest,
            QuantileMethodArg::Lower => Self::Lower,
            QuantileMethodArg::Higher => Self::Higher,
            QuantileMethodArg::Midpoint => Self::Midpoint,
            QuantileMethodArg::Linear => Self::Linear,
        }
    }
}

/// Columns to transform in place, or with `--as`, into a new column.
#[derive(Debug, Clone, Args)]
struct TargetArgs {
    /// Columns: names, *, ^regex$, type:GROUP, or index:N. Defaults to every column.
    #[arg(value_name = "COLUMN")]
    columns: Vec<String>,

    /// Write results to NAME instead of replacing the column; {} expands to the column name.
    #[arg(long = "as", value_name = "NAME")]
    alias: Option<String>,
}

impl TargetArgs {
    fn target(&self) -> Target {
        Target::new(self.columns.clone(), self.alias.clone())
    }
}

/// Columns to aggregate or inspect.
#[derive(Debug, Clone, Args)]
struct ColumnArgs {
    /// Columns: names, *, ^regex$, type:GROUP, or index:N. Defaults to every column.
    #[arg(value_name = "COLUMN")]
    columns: Vec<String>,
}

#[derive(Debug, Clone, Args)]
struct OpenArgs {
    /// Path, -, or a structured inline literal.
    #[arg(allow_hyphen_values = true, value_name = "INPUT")]
    source: Option<String>,

    /// File type; overrides extension and content detection like --from.
    #[arg(short = 't', long = "type", value_enum, value_name = "FORMAT")]
    file_type: Option<InputFormat>,

    /// CSV/TSV field delimiter: one character, or \t / tab.
    #[arg(short = 'd', long, value_name = "CHAR")]
    delimiter: Option<String>,

    /// CSV/TSV input has no header row; columns become column_1, column_2, ...
    #[arg(long)]
    no_input_header: bool,

    /// Rows used to infer CSV/TSV column types; 0 reads every column as text.
    #[arg(long, value_name = "ROWS")]
    infer_schema: Option<usize>,

    /// Skip this many CSV/TSV lines before the header.
    #[arg(long, value_name = "ROWS")]
    skip_rows: Option<usize>,

    /// Read only these columns. Repeat or comma-delimit.
    #[arg(long, value_delimiter = ',', value_name = "COLUMN")]
    columns: Vec<String>,

    /// Column types: NAME:TYPE[,NAME:TYPE...] or a JSON object such as '{"id":"str"}'.
    #[arg(short = 's', long, value_name = "SCHEMA")]
    schema: Option<String>,

    /// Truncate CSV/TSV rows that have more fields than the header.
    #[arg(long)]
    truncate_ragged_lines: bool,

    /// Read a JSON object of equal-length arrays as columns.
    #[arg(short = 'c', long)]
    as_columns: bool,
}

#[derive(Debug, Clone, Args)]
struct SaveArgs {
    /// Destination file. Its extension selects the format unless --type is given.
    path: PathBuf,

    /// Output format.
    #[arg(short = 't', long = "type", value_enum, value_name = "FORMAT")]
    file_type: Option<OutputFormat>,

    /// CSV field delimiter: one character, or \t / tab.
    #[arg(long, value_name = "CHAR")]
    csv_delimiter: Option<String>,

    /// Omit the CSV/TSV header row.
    #[arg(long)]
    csv_no_header: bool,

    /// Avro block compression.
    #[arg(long, value_enum)]
    avro_compression: Option<AvroCompressionArg>,
}

#[derive(Debug, Clone, Args)]
struct RangeArgs {
    /// First date or datetime, such as 2024-01-01.
    #[arg(short = 's', long, allow_hyphen_values = true)]
    start: String,

    /// Last date or datetime.
    #[arg(short = 'e', long, allow_hyphen_values = true)]
    end: String,

    /// Step as a Polars duration: 1d, 12h, 1w, 1mo, 1y, ...
    #[arg(long, default_value = "1d")]
    interval: String,

    /// Which bounds are included: both, left, right, or none.
    #[arg(short = 'c', long, default_value = "both")]
    closed: String,
}

impl RangeArgs {
    fn spec(&self) -> anyhow::Result<RangeSpec> {
        Ok(RangeSpec {
            start: self.start.clone(),
            end: self.end.clone(),
            interval: self.interval.clone(),
            closed: temporal::parse_closed(&self.closed)?,
        })
    }
}

#[derive(Debug, Clone, Args)]
struct DatetimeArgs {
    /// Time unit: ns, us, or ms.
    #[arg(long, value_name = "UNIT")]
    time_unit: Option<String>,

    /// Time zone such as UTC or America/New_York.
    #[arg(long, value_name = "ZONE")]
    time_zone: Option<String>,
}

impl DatetimeArgs {
    fn spec(&self) -> anyhow::Result<DatetimeSpec> {
        Ok(DatetimeSpec {
            time_unit: self.time_unit.as_deref().map(parse_time_unit).transpose()?,
            time_zone: parse_time_zone(self.time_zone.as_deref())?,
        })
    }
}

#[derive(Debug, Clone, Subcommand)]
enum MathCommand {
    /// Absolute value.
    Abs(TargetArgs),
    /// Cosine.
    Cos(TargetArgs),
    /// Sine.
    Sin(TargetArgs),
    /// e raised to the value.
    Exp(TargetArgs),
    /// Logarithm, natural unless --base is given.
    Log {
        /// Logarithm base.
        #[arg(long)]
        base: Option<f64>,
        #[command(flatten)]
        target: TargetArgs,
    },
    /// Natural logarithm of 1 + x.
    Log1p(TargetArgs),
    /// Sign: -1, 0, or 1.
    Sign(TargetArgs),
    /// Square root.
    Sqrt(TargetArgs),
    /// Dot product of two columns, as a one-row `dot` column.
    Dot { left: String, right: String },
    /// Bitwise AND of every value in each column.
    BitwiseAnd(ColumnArgs),
    /// Bitwise OR of every value in each column.
    BitwiseOr(ColumnArgs),
    /// Bitwise XOR of every value in each column.
    BitwiseXor(ColumnArgs),
    /// Number of set bits.
    BitwiseCountOnes(TargetArgs),
    /// Number of unset bits.
    BitwiseCountZeros(TargetArgs),
    /// Number of leading set bits.
    BitwiseLeadingOnes(TargetArgs),
    /// Number of leading unset bits.
    BitwiseLeadingZeros(TargetArgs),
    /// Number of trailing set bits.
    BitwiseTrailingOnes(TargetArgs),
    /// Number of trailing unset bits.
    BitwiseTrailingZeros(TargetArgs),
}

#[derive(Debug, Clone, Subcommand)]
enum Commands {
    /// Read a file, inline value, or stdin into a dataframe.
    #[command(visible_alias = "into-df")]
    Open(OpenArgs),

    /// Write the input to a file; prints nothing.
    Save(SaveArgs),

    /// Keep rows matching a Polars SQL predicate.
    #[command(visible_aliases = ["where", "filter-with"])]
    Filter {
        /// SQL predicate evaluated against the input frame.
        predicate: String,
    },

    /// Select columns or Polars SQL expressions.
    #[command(visible_alias = "get")]
    Select {
        /// Columns, selectors, or SQL expressions. Quote expressions containing shell metacharacters.
        #[arg(required = true, num_args = 1..)]
        expressions: Vec<String>,
    },

    /// Add or replace columns with NAME=SQL_EXPRESSION assignments.
    #[command(name = "with-column", visible_alias = "mutate")]
    WithColumn {
        #[arg(required = true, num_args = 1.., value_name = "NAME=EXPRESSION")]
        assignments: Vec<String>,
    },

    /// Sort by columns or SQL expressions. The sort is stable.
    #[command(name = "sort-by", visible_alias = "sort")]
    SortBy {
        #[arg(required = true, num_args = 1..)]
        expressions: Vec<String>,

        /// Sort every expression descending. Per-expression :asc/:desc suffixes override this.
        #[arg(short = 'r', long, visible_alias = "reverse")]
        descending: bool,

        /// Place nulls last for every expression.
        #[arg(short = 'n', long)]
        nulls_last: bool,

        /// Accepted for Nushell compatibility; ties always keep input order.
        #[arg(short = 'm', long)]
        maintain_order: bool,
    },

    /// Return the first N rows.
    #[command(visible_alias = "head")]
    First {
        #[arg(default_value_t = 1)]
        rows: usize,
    },

    /// Return the last N rows.
    #[command(visible_alias = "tail")]
    Last {
        #[arg(default_value_t = 1)]
        rows: usize,
    },

    /// Return N rows beginning at OFFSET; negative offsets count from the end.
    Slice {
        #[arg(allow_negative_numbers = true)]
        offset: i64,
        rows: usize,
    },

    /// Return rows at the given indices; negative indices count from the end.
    Take {
        /// Row indices, space- or comma-separated, or a JSON array.
        #[arg(required = true, num_args = 1.., allow_negative_numbers = true)]
        indices: Vec<String>,
    },

    /// Return a random sample of rows.
    Sample {
        /// Number of rows.
        #[arg(short = 'n', long, conflicts_with = "fraction")]
        n_rows: Option<usize>,

        /// Fraction of rows.
        #[arg(short = 'f', long)]
        fraction: Option<f64>,

        /// Seed for a reproducible sample.
        #[arg(short = 's', long)]
        seed: Option<u64>,

        /// Sample with replacement.
        #[arg(short = 'e', long)]
        replace: bool,

        /// Shuffle the sampled rows.
        #[arg(short = 'u', long)]
        shuffle: bool,
    },

    /// Remove columns by name or selector.
    Drop {
        #[arg(required = true, num_args = 1..)]
        columns: Vec<String>,
    },

    /// Rename columns with OLD=NEW mappings or OLD NEW pairs.
    Rename {
        #[arg(required = true, num_args = 1.., value_name = "OLD=NEW")]
        mappings: Vec<String>,
    },

    /// Remove duplicate rows, optionally considering only selected columns.
    #[command(visible_aliases = ["distinct", "drop-duplicates"])]
    Unique {
        #[arg(num_args = 0..)]
        columns: Vec<String>,

        /// Columns to consider; same as positional columns.
        #[arg(short = 's', long)]
        subset: Vec<String>,

        #[arg(long, value_enum, default_value_t = KeepArg::First)]
        keep: KeepArg,

        /// Keep the last duplicate (same as --keep last).
        #[arg(short = 'l', long)]
        last: bool,

        /// Accepted for Nushell compatibility; row order is always kept.
        #[arg(short = 'm', long, visible_alias = "maintain")]
        maintain_order: bool,
    },

    /// Remove rows containing nulls, optionally restricted to selected columns.
    #[command(name = "drop-nulls")]
    DropNulls {
        #[arg(num_args = 0..)]
        columns: Vec<String>,
    },

    /// Reverse row order.
    Reverse,

    /// Shift values down by N rows (up when negative), filling the gap with nulls or --fill.
    Shift {
        #[arg(allow_negative_numbers = true)]
        periods: i64,

        /// Value for the vacated rows.
        #[arg(short = 'f', long, allow_hyphen_values = true)]
        fill: Option<String>,

        #[command(flatten)]
        target: TargetArgs,
    },

    /// Add a row index column.
    #[command(name = "with-row-index")]
    WithRowIndex {
        /// Name of the index column.
        #[arg(short = 'n', long, default_value = "index")]
        name: String,

        /// First index value.
        #[arg(long, default_value_t = 0)]
        offset: u32,
    },

    /// Group by keys and evaluate SQL aggregate expressions; groups keep first-appearance order.
    #[command(name = "group-by")]
    GroupBy {
        #[arg(required = true, num_args = 1..)]
        keys: Vec<String>,

        /// SQL aggregation such as 'SUM(amount) AS total'. Repeat for multiple aggregations.
        #[arg(short = 'a', long = "agg", required = true)]
        aggregations: Vec<String>,

        /// Accepted for Nushell compatibility; group order is always kept.
        #[arg(short = 'm', long)]
        maintain_order: bool,
    },

    /// Join the input frame with another file or inline value.
    Join {
        /// Right-hand path or structured inline literal.
        #[arg(allow_hyphen_values = true)]
        right: String,

        /// Same-name join keys. Repeat or use comma-delimited values.
        #[arg(long, value_delimiter = ',')]
        on: Vec<String>,

        /// Left-side keys when names differ.
        #[arg(long = "left-on", value_delimiter = ',')]
        left_on: Vec<String>,

        /// Right-side keys when names differ.
        #[arg(long = "right-on", value_delimiter = ',')]
        right_on: Vec<String>,

        #[arg(long = "how", value_enum, default_value_t = JoinType::Inner)]
        kind: JoinType,

        /// Suffix for right-hand columns whose names clash.
        #[arg(short = 's', long, default_value = "_right")]
        suffix: String,

        /// Match null keys with each other.
        #[arg(long)]
        nulls_equal: bool,

        /// Merge differently named key pairs into the left key column.
        #[arg(long)]
        coalesce_columns: bool,
    },

    /// Join with another input wherever a SQL predicate holds (inequality joins).
    #[command(name = "join-where")]
    JoinWhere {
        /// Right-hand path or structured inline literal.
        #[arg(allow_hyphen_values = true)]
        right: String,

        /// Predicate over both inputs; clashing right-hand columns take the suffix.
        predicate: String,

        /// Suffix for right-hand columns whose names clash.
        #[arg(short = 's', long, default_value = "_right")]
        suffix: String,
    },

    /// Vertically concatenate two or more inputs.
    Concat {
        /// Paths or structured inline literals. With one value, stdin is the first frame.
        #[arg(required = true, num_args = 1.., value_name = "INPUT")]
        sources: Vec<String>,

        /// Align columns by name, filling missing columns with nulls.
        #[arg(long)]
        diagonal: bool,

        /// Cast clashing column types to a common supertype.
        #[arg(long)]
        to_supertypes: bool,
    },

    /// Append another input as new columns, or as rows with --col.
    Append {
        /// Path or structured inline literal.
        #[arg(allow_hyphen_values = true)]
        other: String,

        /// Append rows instead of columns.
        #[arg(short = 'c', long)]
        col: bool,
    },

    /// Run a Polars SQL statement; the input frame is named df.
    #[command(visible_alias = "sql")]
    Query { statement: String },

    /// List column names.
    Columns,

    /// Show column names and data types.
    Schema,

    /// Show row and column counts.
    Shape,

    /// Count rows, or non-null values in the given columns.
    Count(ColumnArgs),

    /// Count rows as a `len` column.
    Len,

    /// Count null values per column.
    #[command(name = "count-null")]
    CountNull(ColumnArgs),

    /// Count distinct values per column.
    #[command(name = "n-unique")]
    NUnique(ColumnArgs),

    /// Render the input in a chosen format.
    To { format: OutputFormat },

    /// Cast columns to a data type such as i64, f64, str, bool, date, or datetime<ms, UTC>.
    Cast {
        dtype: String,

        #[command(flatten)]
        target: TargetArgs,

        /// Turn values that cannot be cast into nulls instead of failing.
        #[arg(long)]
        non_strict: bool,
    },

    /// Replace nulls with VALUE, or by --strategy.
    #[command(name = "fill-null")]
    FillNull {
        /// VALUE then COLUMNs; with --strategy every argument is a COLUMN.
        #[arg(value_name = "VALUE|COLUMN", allow_negative_numbers = true)]
        args: Vec<String>,

        #[arg(long = "as", value_name = "NAME")]
        alias: Option<String>,

        /// Fill strategy instead of a value.
        #[arg(long, value_enum)]
        strategy: Option<FillStrategyArg>,

        /// Maximum consecutive nulls to fill with forward/backward.
        #[arg(long)]
        limit: Option<u32>,
    },

    /// Replace NaN values in float columns.
    #[command(name = "fill-nan")]
    FillNan {
        #[arg(allow_negative_numbers = true)]
        value: String,

        #[command(flatten)]
        target: TargetArgs,
    },

    /// Turn list columns into one row per element.
    #[command(visible_alias = "flatten")]
    Explode {
        /// List columns. Defaults to every list column.
        columns: Vec<String>,

        /// Explode an empty list into a null instead of dropping it.
        #[arg(long)]
        empty_as_null: bool,

        /// Explode a null list into a null instead of dropping it.
        #[arg(long)]
        keep_nulls: bool,
    },

    /// Expand struct columns into one column per field.
    Unnest {
        /// Struct columns. Defaults to every struct column.
        columns: Vec<String>,

        /// Prefix fields with the column name and this separator.
        #[arg(short = 's', long)]
        separator: Option<String>,
    },

    /// Aggregate each column into a single list value.
    Implode(ColumnArgs),

    /// Pivot from long to wide format.
    Pivot {
        /// Columns whose values become new column names.
        #[arg(long, required = true)]
        on: Vec<String>,

        /// Columns that identify output rows.
        #[arg(long)]
        index: Vec<String>,

        /// Columns whose values fill the cells.
        #[arg(long)]
        values: Vec<String>,

        /// Aggregation for cells with several values: first, last, sum, min, max, mean, median, count.
        #[arg(short = 'a', long)]
        aggregate: Option<String>,

        /// Separator in generated names when there are several value columns.
        #[arg(short = 'p', long, default_value = "_")]
        separator: String,
    },

    /// Unpivot from wide to long format.
    #[command(visible_alias = "melt")]
    Unpivot {
        /// Columns kept as identifiers.
        #[arg(long)]
        index: Vec<String>,

        /// Columns to unpivot. Defaults to every non-index column.
        #[arg(long)]
        on: Vec<String>,

        /// Name of the variable column.
        #[arg(short = 'r', long)]
        variable_name: Option<String>,

        /// Name of the value column.
        #[arg(short = 'l', long)]
        value_name: Option<String>,
    },

    /// One-hot encode columns into dummy variables.
    Dummies {
        columns: Vec<String>,

        /// Drop the first category of each column.
        #[arg(short = 'd', long)]
        drop_first: bool,

        /// Do not create a dummy column for nulls.
        #[arg(short = 'n', long)]
        drop_nulls: bool,

        /// Separator between the column name and the value.
        #[arg(short = 's', long)]
        separator: Option<String>,
    },

    /// Descriptive statistics per column.
    Summary {
        /// Quantiles to report, comma-separated.
        #[arg(short = 'q', long, default_value = "0.25,0.5,0.75")]
        quantiles: String,
    },

    /// Count distinct values (or rows, for several columns) in first-appearance order.
    #[command(name = "value-counts")]
    ValueCounts {
        #[command(flatten)]
        columns: ColumnArgs,

        /// Name of the count column.
        #[arg(short = 'c', long)]
        column: Option<String>,

        /// Sort by count, highest first.
        #[arg(short = 's', long)]
        sort: bool,

        /// Report proportions instead of counts.
        #[arg(short = 'n', long)]
        normalize: bool,
    },

    /// Sum each column.
    Sum(ColumnArgs),
    /// Minimum of each column.
    Min(ColumnArgs),
    /// Maximum of each column.
    Max(ColumnArgs),
    /// Mean of each column.
    Mean(ColumnArgs),
    /// Median of each column.
    Median(ColumnArgs),
    /// Standard deviation of each column.
    Std {
        #[command(flatten)]
        columns: ColumnArgs,
        /// Delta degrees of freedom.
        #[arg(long, default_value_t = 1)]
        ddof: u8,
    },
    /// Variance of each column.
    Var {
        #[command(flatten)]
        columns: ColumnArgs,
        /// Delta degrees of freedom.
        #[arg(long, default_value_t = 1)]
        ddof: u8,
    },
    /// Quantile of each column.
    Quantile {
        /// Quantile between 0 and 1.
        quantile: f64,
        #[command(flatten)]
        columns: ColumnArgs,
        /// Interpolation method.
        #[arg(long, value_enum, default_value_t = QuantileMethodArg::Nearest)]
        method: QuantileMethodArg,
    },
    /// Entropy of each column, -sum(pk * log(pk)).
    Entropy {
        #[command(flatten)]
        columns: ColumnArgs,
        /// Logarithm base.
        #[arg(long, default_value_t = std::f64::consts::E)]
        base: f64,
        /// Normalize pk to sum to 1.
        #[arg(long, default_value_t = true, action = ArgAction::Set)]
        normalize: bool,
    },
    /// Whether every value of a boolean column is true.
    #[command(name = "all-true")]
    AllTrue(ColumnArgs),
    /// Whether every value of a boolean column is false.
    #[command(name = "all-false")]
    AllFalse(ColumnArgs),
    /// Row index of each column's maximum.
    #[command(name = "arg-max")]
    ArgMax(ColumnArgs),
    /// Row index of each column's minimum.
    #[command(name = "arg-min")]
    ArgMin(ColumnArgs),
    /// Row indices where a boolean column is true.
    #[command(name = "arg-true")]
    ArgTrue(ColumnArgs),
    /// Row index of the first occurrence of each distinct value or row.
    #[command(name = "arg-unique")]
    ArgUnique(ColumnArgs),
    /// Row indices that sort the selected columns.
    #[command(name = "arg-sort")]
    ArgSort {
        #[command(flatten)]
        columns: ColumnArgs,
        /// Sort descending.
        #[arg(short = 'r', long)]
        reverse: bool,
        /// Place nulls last.
        #[arg(short = 'n', long)]
        nulls_last: bool,
        /// Return at most this many indices.
        #[arg(short = 'l', long)]
        limit: Option<usize>,
        /// Accepted for Nushell compatibility; ties always keep input order.
        #[arg(short = 'm', long)]
        maintain_order: bool,
    },
    /// Row indices where a SQL predicate is true.
    #[command(name = "arg-where")]
    ArgWhere { predicate: String },

    /// Null mask: true where a value is null.
    #[command(name = "is-null")]
    IsNull(TargetArgs),
    /// Mask: true where a value is not null.
    #[command(name = "is-not-null")]
    IsNotNull(TargetArgs),
    /// Mask of rows that occur more than once (over the selected columns).
    #[command(name = "is-duplicated")]
    IsDuplicated(TargetArgs),
    /// Mask of rows that occur exactly once (over the selected columns).
    #[command(name = "is-unique")]
    IsUnique(TargetArgs),
    /// Mask: true where a value is in VALUES (comma-separated or a JSON array).
    #[command(name = "is-in")]
    IsIn {
        #[arg(allow_negative_numbers = true)]
        values: String,
        #[command(flatten)]
        target: TargetArgs,
        /// Treat null as equal to null.
        #[arg(long)]
        nulls_equal: bool,
    },
    /// Invert a boolean column.
    Not(TargetArgs),
    /// Set VALUE where a SQL predicate holds.
    Set {
        #[arg(allow_negative_numbers = true)]
        value: String,
        #[command(flatten)]
        target: TargetArgs,
        /// SQL predicate selecting the rows to set.
        #[arg(short = 'm', long, required = true)]
        mask: String,
    },
    /// Set VALUE at the given row indices.
    #[command(name = "set-with-idx")]
    SetWithIdx {
        #[arg(allow_negative_numbers = true)]
        value: String,
        #[command(flatten)]
        target: TargetArgs,
        /// Row indices, comma-separated or a JSON array.
        #[arg(long, required = true, allow_hyphen_values = true)]
        indices: String,
    },

    /// Uppercase strings.
    Uppercase(TargetArgs),
    /// Lowercase strings.
    Lowercase(TargetArgs),
    /// String lengths in characters, or bytes with --bytes.
    #[command(name = "str-lengths")]
    StrLengths {
        #[command(flatten)]
        target: TargetArgs,
        #[arg(short = 'b', long)]
        bytes: bool,
    },
    /// Replace the first regex match in each string.
    #[command(name = "str-replace")]
    StrReplace {
        #[arg(short = 'p', long, allow_hyphen_values = true)]
        pattern: String,
        #[arg(short = 'r', long, allow_hyphen_values = true)]
        replace: String,
        #[command(flatten)]
        target: TargetArgs,
        /// Treat the pattern as literal text.
        #[arg(long)]
        literal: bool,
    },
    /// Replace every regex match in each string.
    #[command(name = "str-replace-all")]
    StrReplaceAll {
        #[arg(short = 'p', long, allow_hyphen_values = true)]
        pattern: String,
        #[arg(short = 'r', long, allow_hyphen_values = true)]
        replace: String,
        #[command(flatten)]
        target: TargetArgs,
        /// Treat the pattern as literal text.
        #[arg(long)]
        literal: bool,
    },
    /// Substring from START (negative counts from the end), optionally of --length characters.
    #[command(name = "str-slice")]
    StrSlice {
        #[arg(allow_negative_numbers = true)]
        start: i64,
        #[command(flatten)]
        target: TargetArgs,
        #[arg(short = 'l', long)]
        length: Option<u64>,
    },
    /// Split strings on a separator into lists.
    #[command(name = "str-split")]
    StrSplit {
        #[arg(allow_hyphen_values = true)]
        separator: String,
        #[command(flatten)]
        target: TargetArgs,
    },
    /// Strip characters from both ends of strings.
    #[command(name = "str-strip-chars")]
    StrStripChars {
        /// PATTERN (the characters to strip) then COLUMNs; with --whitespace every argument is a COLUMN.
        #[arg(value_name = "PATTERN|COLUMN")]
        args: Vec<String>,
        #[arg(long = "as", value_name = "NAME")]
        alias: Option<String>,
        /// Strip whitespace instead of PATTERN.
        #[arg(short = 'w', long)]
        whitespace: bool,
        /// Strip from the start only.
        #[arg(short = 's', long, conflicts_with = "end")]
        start: bool,
        /// Strip from the end only.
        #[arg(short = 'e', long)]
        end: bool,
    },
    /// Mask: true where a string matches a regex.
    Contains {
        #[arg(allow_hyphen_values = true)]
        pattern: String,
        #[command(flatten)]
        target: TargetArgs,
        /// Treat the pattern as literal text.
        #[arg(long)]
        literal: bool,
    },
    /// Join all strings of each column into one value.
    #[command(name = "str-join")]
    StrJoin {
        #[command(flatten)]
        columns: ColumnArgs,
        #[arg(short = 'd', long, default_value = "", allow_hyphen_values = true)]
        delimiter: String,
        #[arg(short = 'n', long)]
        ignore_nulls: bool,
    },
    /// Concatenate columns row-wise into a new string column.
    #[command(name = "concat-str")]
    ConcatStr {
        #[arg(allow_hyphen_values = true)]
        separator: String,
        #[arg(required = true, num_args = 1..)]
        columns: Vec<String>,
        #[arg(long = "as", value_name = "NAME", default_value = "concat_str")]
        alias: String,
        /// Skip nulls instead of producing null.
        #[arg(long)]
        ignore_nulls: bool,
    },

    /// Year of each date.
    #[command(name = "get-year")]
    GetYear(TargetArgs),
    /// Month of each date.
    #[command(name = "get-month")]
    GetMonth(TargetArgs),
    /// ISO week of each date.
    #[command(name = "get-week")]
    GetWeek(TargetArgs),
    /// ISO weekday of each date (Monday = 1).
    #[command(name = "get-weekday")]
    GetWeekday(TargetArgs),
    /// Day of month of each date.
    #[command(name = "get-day")]
    GetDay(TargetArgs),
    /// Day of year of each date.
    #[command(name = "get-ordinal")]
    GetOrdinal(TargetArgs),
    /// Hour of each datetime.
    #[command(name = "get-hour")]
    GetHour(TargetArgs),
    /// Minute of each datetime.
    #[command(name = "get-minute")]
    GetMinute(TargetArgs),
    /// Second of each datetime.
    #[command(name = "get-second")]
    GetSecond(TargetArgs),
    /// Nanosecond of each datetime.
    #[command(name = "get-nanosecond")]
    GetNanosecond(TargetArgs),
    /// A named part of each date: year, quarter, month, week, weekday, day, ordinal, hour, ...
    Datepart {
        part: String,
        #[command(flatten)]
        target: TargetArgs,
    },
    /// Format dates with a strftime pattern.
    Strftime {
        format: String,
        #[command(flatten)]
        target: TargetArgs,
    },
    /// Parse strings as dates with a strftime pattern.
    #[command(name = "as-date")]
    AsDate {
        format: String,
        #[command(flatten)]
        target: TargetArgs,
        /// Allow the pattern to match part of the string.
        #[arg(short = 'n', long)]
        not_exact: bool,
    },
    /// Parse strings as datetimes with a strftime pattern.
    #[command(name = "as-datetime")]
    AsDatetime {
        format: String,
        #[command(flatten)]
        target: TargetArgs,
        /// Allow the pattern to match part of the string.
        #[arg(short = 'n', long)]
        not_exact: bool,
        /// Drop time zone information from the result.
        #[arg(long)]
        naive: bool,
        /// Ambiguous local times: raise, earliest, latest, or null.
        #[arg(short = 'a', long, default_value = "raise")]
        ambiguous: String,
        #[command(flatten)]
        datetime: DatetimeArgs,
    },
    /// Convert datetimes to another time zone (null makes them naive UTC).
    #[command(name = "convert-time-zone")]
    ConvertTimeZone {
        time_zone: String,
        #[command(flatten)]
        target: TargetArgs,
    },
    /// Set or remove the time zone of datetimes without converting them (null removes it).
    #[command(name = "replace-time-zone")]
    ReplaceTimeZone {
        time_zone: String,
        #[command(flatten)]
        target: TargetArgs,
        /// Ambiguous local times: raise, earliest, latest, or null.
        #[arg(short = 'a', long, default_value = "raise")]
        ambiguous: String,
        /// Nonexistent local times: raise or null.
        #[arg(short = 'n', long, default_value = "raise")]
        nonexistent: String,
    },
    /// Truncate dates to a multiple of a duration such as 1mo or 15m.
    Truncate {
        every: String,
        #[command(flatten)]
        target: TargetArgs,
    },
    /// Generate a date column; reads no input.
    #[command(name = "date-range")]
    DateRange {
        #[command(flatten)]
        range: RangeArgs,
        #[arg(long = "as", value_name = "NAME", default_value = "date")]
        alias: String,
    },
    /// Generate a datetime column; reads no input.
    #[command(name = "datetime-range")]
    DatetimeRange {
        #[command(flatten)]
        range: RangeArgs,
        #[command(flatten)]
        datetime: DatetimeArgs,
        #[arg(long = "as", value_name = "NAME", default_value = "datetime")]
        alias: String,
    },
    /// Add a list column of dates between --start and --end (columns or literal dates).
    #[command(name = "date-ranges")]
    DateRanges {
        #[command(flatten)]
        range: RangeArgs,
        #[arg(long = "as", value_name = "NAME", default_value = "date_range")]
        alias: String,
    },
    /// Add a list column of datetimes between --start and --end (columns or literals).
    #[command(name = "datetime-ranges")]
    DatetimeRanges {
        #[command(flatten)]
        range: RangeArgs,
        #[command(flatten)]
        datetime: DatetimeArgs,
        #[arg(long = "as", value_name = "NAME", default_value = "datetime_range")]
        alias: String,
    },

    /// Math functions on numeric columns.
    Math {
        #[command(subcommand)]
        command: MathCommand,
    },
    /// Running sum, min, max, count, or product.
    Cumulative {
        #[arg(value_enum)]
        kind: CumulativeArg,
        #[command(flatten)]
        target: TargetArgs,
        /// Accumulate from the last row.
        #[arg(short = 'r', long)]
        reverse: bool,
    },
    /// Rolling window statistics, or a rolling group-by with --index-column.
    Rolling {
        /// sum, min, max, mean, median, std, or var.
        #[arg(value_enum)]
        kind: Option<RollingArg>,
        /// Window size in rows.
        window: Option<usize>,
        #[command(flatten)]
        target: TargetArgs,
        /// Values required in a window before producing a result. Defaults to the window size.
        #[arg(long)]
        min_periods: Option<usize>,
        /// Label each window at its center.
        #[arg(long)]
        center: bool,
        /// Sorted time or integer column to roll over (group-by mode).
        #[arg(long)]
        index_column: Option<String>,
        /// Window length in group-by mode, such as 7d or 2i.
        #[arg(short = 'p', long)]
        period: Option<String>,
        /// Window offset in group-by mode. Defaults to -period.
        #[arg(long, allow_hyphen_values = true)]
        offset: Option<String>,
        /// Closed window side in group-by mode: right, left, both, or none.
        #[arg(short = 'c', long, default_value = "right")]
        closed: String,
        /// Extra group keys in group-by mode.
        #[arg(short = 'g', long = "group-by", value_delimiter = ',')]
        group_by: Vec<String>,
        /// SQL aggregation in group-by mode, such as 'SUM(x) AS total'. Repeatable.
        #[arg(short = 'a', long = "agg")]
        aggregations: Vec<String>,
    },
    /// Row-wise reduction across columns: all, any, min, max, sum, or mean.
    Horizontal {
        #[arg(value_enum)]
        op: HorizontalArg,
        #[arg(required = true, num_args = 1..)]
        columns: Vec<String>,
        /// Output column name. Defaults to the operation name.
        #[arg(long = "as", value_name = "NAME")]
        alias: Option<String>,
        /// Propagate nulls instead of ignoring them (sum and mean).
        #[arg(short = 'n', long)]
        nulls: bool,
    },
    /// Bin values at break points into categories.
    Cut {
        /// Break points, comma-separated or a JSON array.
        #[arg(allow_negative_numbers = true, allow_hyphen_values = true)]
        breaks: String,
        #[command(flatten)]
        target: TargetArgs,
        /// Category labels (one more than the break points).
        #[arg(short = 'l', long)]
        labels: Option<String>,
        /// Make intervals left-closed.
        #[arg(short = 'c', long)]
        left_closed: bool,
        /// Output a struct with the bin's right endpoint.
        #[arg(short = 'b', long)]
        include_breaks: bool,
    },
    /// Bin values into quantile-based categories.
    Qcut {
        /// Number of equal-probability bins, or quantiles comma-separated / as a JSON array.
        quantiles: String,
        #[command(flatten)]
        target: TargetArgs,
        /// Category labels (one per bin).
        #[arg(short = 'l', long)]
        labels: Option<String>,
        /// Make intervals left-closed.
        #[arg(short = 'c', long)]
        left_closed: bool,
        /// Output a struct with the bin's right endpoint.
        #[arg(short = 'b', long)]
        include_breaks: bool,
        /// Drop duplicate quantile edges instead of failing.
        #[arg(short = 'd', long)]
        allow_duplicates: bool,
    },
    /// Map values: OLD NEW lists (comma-separated or JSON arrays) or --map '{"old":"new"}'.
    Replace {
        /// OLD NEW then COLUMNs; with --map every argument is a COLUMN.
        #[arg(value_name = "OLD NEW|COLUMN", allow_negative_numbers = true)]
        args: Vec<String>,
        #[arg(long = "as", value_name = "NAME")]
        alias: Option<String>,
        /// JSON object mapping old values to new values.
        #[arg(long)]
        map: Option<String>,
        /// Fail when a value has no replacement.
        #[arg(short = 's', long)]
        strict: bool,
        /// Value for unmatched entries.
        #[arg(short = 'd', long, allow_hyphen_values = true)]
        default: Option<String>,
        /// Data type of the result.
        #[arg(short = 't', long)]
        return_dtype: Option<String>,
    },
    /// Parse strings as integers.
    Integer {
        #[command(flatten)]
        target: TargetArgs,
        /// Radix between 2 and 36.
        #[arg(long, default_value_t = 10)]
        base: u32,
        /// Integer type of the result.
        #[arg(long, default_value = "i64")]
        dtype: String,
        /// Fail on unparsable values instead of producing null.
        #[arg(short = 's', long)]
        strict: bool,
    },
    /// Parse strings as decimals with SCALE digits after the point.
    Decimal {
        scale: usize,
        #[command(flatten)]
        target: TargetArgs,
    },
    /// Encode struct columns as JSON strings.
    #[command(name = "struct-json-encode")]
    StructJsonEncode(TargetArgs),
    /// Mask: true where a list contains ELEMENT.
    #[command(name = "list-contains")]
    ListContains {
        #[arg(allow_negative_numbers = true)]
        element: String,
        #[command(flatten)]
        target: TargetArgs,
    },

    /// Generate a shell completion script.
    Completions { shell: Shell },
}

#[derive(Debug, Clone, Parser)]
#[command(
    author,
    version,
    about = "A Bash-native, composable Polars CLI",
    long_about = "Read, transform, query, and render structured data with Polars. Commands consume stdin by default, making `pl` pipelines behave naturally in Bash.",
    after_help = EXAMPLES,
    override_usage = "pl [OPTIONS] [INPUT] [COMMAND]\n       pl [OPTIONS] <COMMAND> [ARGS]...",
    subcommand_precedence_over_arg = true
)]
struct Cli {
    /// Input path/literal before a command, or legacy conversion/SQL inputs.
    #[arg(value_name = "INPUT")]
    inputs: Vec<String>,

    #[command(subcommand)]
    command: Option<Commands>,

    /// Input path, -, or structured inline literal. Available before or after subcommands.
    #[arg(
        short = 'i',
        long,
        global = true,
        value_name = "INPUT",
        conflicts_with = "data"
    )]
    input: Option<String>,

    /// Explicit inline input literal. Available before or after subcommands.
    #[arg(long, global = true, value_name = "DATA", conflicts_with = "input")]
    data: Option<String>,

    /// Force the input format instead of inferring it from the extension or content.
    #[arg(long, value_enum, global = true)]
    from: Option<InputFormat>,

    /// Force the output format. Defaults to table on a terminal and JSONL in a pipe.
    #[arg(long, value_enum, global = true)]
    to: Option<OutputFormat>,

    /// Write output to a file. Its extension selects the format when --to is omitted.
    #[arg(short = 'o', long, global = true)]
    output: Option<PathBuf>,

    /// Emit a single-column dataframe as newline-delimited values.
    #[arg(long, global = true)]
    raw: bool,

    /// Pretty-print JSON array output.
    #[arg(long, global = true)]
    pretty: bool,

    /// Omit the header row for CSV or TSV output.
    #[arg(long, global = true)]
    no_header: bool,

    /// Field delimiter for CSV output: one character, or \t / tab.
    #[arg(long, global = true, value_name = "CHAR")]
    output_delimiter: Option<String>,

    /// Legacy mode: run a Polars SQL query against positional inputs.
    #[arg(long)]
    sql: Option<String>,

    /// Legacy mode: register NAME=PATH with --sql. Repeatable.
    #[arg(long = "table", value_name = "NAME=PATH")]
    tables: Vec<String>,
}

#[derive(Debug)]
enum LoadedInput {
    Frame(DataFrame),
    HtmlDocument(String),
}

/// Rendered output: text is newline-terminated on emission, binary is written verbatim.
enum Rendered {
    Text(String),
    Binary(Vec<u8>),
}

/// Rendering options that are not part of the format itself.
#[derive(Debug, Clone, Copy)]
struct RenderOptions {
    header: bool,
    delimiter: Option<u8>,
    pretty: bool,
    avro_compression: Option<AvroCompression>,
}

/// Stack size for the thread that parses arguments and runs the command.
///
/// Building the definitions for over a hundred subcommands takes a few MiB of stack in
/// unoptimized builds, more than the 1 MiB main-thread stack on Windows. The reservation is
/// virtual memory, so a generous fixed size costs nothing until it is used.
const RUN_STACK_SIZE: usize = 32 * 1024 * 1024;

pub fn run() -> anyhow::Result<()> {
    // Corrupt binary input can panic inside Polars' decoders; `read` turns those panics into
    // errors, so their default panic message would only be noise.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if !crate::read::is_decoding_binary() {
            default_hook(info);
        }
    }));
    std::thread::Builder::new()
        .name("pl".to_owned())
        .stack_size(RUN_STACK_SIZE)
        .spawn(run_on_current_thread)
        .context("failed to start the command thread")?
        .join()
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}

fn run_on_current_thread() -> anyhow::Result<()> {
    let cli = Cli::parse();

    if let Some(Commands::Completions { shell }) = cli.command {
        let mut command = Cli::command();
        let binary_name = std::env::args()
            .next()
            .and_then(|path| {
                Path::new(&path)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| "pl".to_owned());
        generate(shell, &mut command, binary_name, &mut std::io::stdout());
        return Ok(());
    }

    if cli.command.is_some() && cli.sql.is_some() {
        bail!("--sql is legacy root mode; use the query subcommand in a pipeline");
    }
    if cli.command.is_some() && !cli.tables.is_empty() {
        bail!("--table is legacy root mode and can only be used with --sql");
    }

    if let Some(statement) = cli.sql.as_deref() {
        return run_legacy_sql(&cli, statement);
    }

    match cli.command.clone() {
        None => run_legacy_conversion(&cli),
        Some(command) => run_command(&cli, command),
    }
}

fn run_command(cli: &Cli, command: Commands) -> anyhow::Result<()> {
    let frame = || require_frame(load_primary(cli, None, &ReadOptions::default())?);
    let emit = |frame: DataFrame| emit_frame(cli, frame, None, None);

    match command {
        Commands::Open(args) => run_open(cli, &args),
        Commands::Save(args) => run_save(cli, &args),
        Commands::Filter { predicate } => emit(ops::filter(frame()?, &predicate)?),
        Commands::Select { expressions } => emit(ops::select(frame()?, &expressions)?),
        Commands::WithColumn { assignments } => emit(ops::with_columns(frame()?, &assignments)?),
        Commands::SortBy {
            expressions,
            descending,
            nulls_last,
            maintain_order: _,
        } => emit(ops::sort_by(
            frame()?,
            &expressions,
            descending,
            nulls_last,
        )?),
        Commands::First { rows } => emit(ops::first(frame()?, rows)),
        Commands::Last { rows } => emit(ops::last(frame()?, rows)),
        Commands::Slice { offset, rows } => emit(ops::slice(frame()?, offset, rows)),
        Commands::Take { indices } => {
            let indices = indices
                .iter()
                .map(|value| parse_index_list(value))
                .collect::<anyhow::Result<Vec<_>>>()?
                .concat();
            emit(reshape::take(frame()?, &indices)?)
        }
        Commands::Sample {
            n_rows,
            fraction,
            seed,
            replace,
            shuffle,
        } => {
            let size = match (n_rows, fraction) {
                (Some(rows), None) => SampleSize::Rows(rows),
                (None, Some(fraction)) => SampleSize::Fraction(fraction),
                _ => bail!("sample requires --n-rows or --fraction"),
            };
            emit(reshape::sample(frame()?, size, seed, replace, shuffle)?)
        }
        Commands::Drop { columns } => emit(ops::drop_columns(frame()?, &columns)?),
        Commands::Rename { mappings } => emit(ops::rename_columns(frame()?, &mappings)?),
        Commands::Unique {
            mut columns,
            subset,
            keep,
            last,
            maintain_order: _,
        } => {
            columns.extend(selector::split_list_arguments(&subset));
            let keep = if last { KeepArg::Last } else { keep };
            emit(ops::unique(frame()?, &columns, keep.into())?)
        }
        Commands::DropNulls { columns } => emit(ops::drop_nulls(frame()?, &columns)?),
        Commands::Reverse => emit(ops::reverse(frame()?)),
        Commands::Shift {
            periods,
            fill,
            target,
        } => {
            let fill = fill.as_deref().map(parse_scalar);
            emit(column::shift(
                frame()?,
                &target.target(),
                periods,
                fill.as_ref(),
            )?)
        }
        Commands::WithRowIndex { name, offset } => {
            emit(reshape::with_row_index(frame()?, &name, offset)?)
        }
        Commands::GroupBy {
            keys,
            aggregations,
            maintain_order: _,
        } => emit(ops::group_by(frame()?, &keys, &aggregations)?),
        Commands::Join {
            right,
            on,
            left_on,
            right_on,
            kind,
            suffix,
            nulls_equal,
            coalesce_columns,
        } => {
            let left = frame()?;
            let right = require_frame(load_token(&right, None, false, &ReadOptions::default())?)?;
            let (left_keys, right_keys) = resolve_join_keys(&on, &left_on, &right_on)?;
            emit(ops::join(
                left,
                right,
                kind.into(),
                &left_keys,
                &right_keys,
                &JoinOptions {
                    suffix: &suffix,
                    nulls_equal,
                    coalesce: coalesce_columns,
                },
            )?)
        }
        Commands::JoinWhere {
            right,
            predicate,
            suffix,
        } => {
            let left = frame()?;
            let right = require_frame(load_token(&right, None, false, &ReadOptions::default())?)?;
            emit(reshape::join_where(left, right, &predicate, &suffix)?)
        }
        Commands::Concat {
            sources,
            diagonal,
            to_supertypes,
        } => run_concat(cli, &sources, diagonal, to_supertypes),
        Commands::Append { other, col } => {
            let left = frame()?;
            let right = require_frame(load_token(&other, None, false, &ReadOptions::default())?)?;
            emit(reshape::append(left, right, col)?)
        }
        Commands::Query { statement } => emit(ops::query(frame()?, &statement)?),
        Commands::Columns => emit(ops::columns(&frame()?)?),
        Commands::Schema => emit(ops::schema(&frame()?)?),
        Commands::Shape => emit(ops::shape(&frame()?)?),
        Commands::Count(args) if args.columns.is_empty() => emit(ops::count(&frame()?)?),
        Commands::Count(args) => emit(aggregate::aggregate(
            frame()?,
            &args.columns,
            Aggregation::Count,
        )?),
        Commands::Len => emit(aggregate::len_frame(&frame()?)?),
        Commands::CountNull(args) => emit(aggregate::aggregate(
            frame()?,
            &args.columns,
            Aggregation::CountNull,
        )?),
        Commands::NUnique(args) => emit(aggregate::aggregate(
            frame()?,
            &args.columns,
            Aggregation::NUnique,
        )?),
        Commands::To { format } => {
            let loaded = load_primary(cli, None, &ReadOptions::default())?;
            emit_loaded(cli, loaded, Some(format), None)
        }
        Commands::Cast {
            dtype,
            target,
            non_strict,
        } => emit(column::cast(
            frame()?,
            &target.target(),
            &parse_dtype(&dtype)?,
            !non_strict,
        )?),
        Commands::FillNull {
            args,
            alias,
            strategy,
            limit,
        } => {
            let (value, columns) = match strategy {
                Some(_) => (None, args),
                None => {
                    let mut args = args.into_iter();
                    let value = args
                        .next()
                        .context("fill-null requires a VALUE or --strategy")?;
                    (Some(parse_scalar(&value)), args.collect())
                }
            };
            emit(column::fill_null(
                frame()?,
                &Target::new(columns, alias),
                value.as_ref(),
                strategy.map(Into::into),
                limit,
            )?)
        }
        Commands::FillNan { value, target } => emit(column::fill_nan(
            frame()?,
            &target.target(),
            &parse_scalar(&value),
        )?),
        Commands::Explode {
            columns,
            empty_as_null,
            keep_nulls,
        } => emit(reshape::explode(
            frame()?,
            &columns,
            empty_as_null,
            keep_nulls,
        )?),
        Commands::Unnest { columns, separator } => {
            emit(reshape::unnest(frame()?, &columns, separator.as_deref())?)
        }
        Commands::Implode(args) => emit(aggregate::aggregate(
            frame()?,
            &args.columns,
            Aggregation::Implode,
        )?),
        Commands::Pivot {
            on,
            index,
            values,
            aggregate,
            separator,
        } => emit(reshape::pivot(
            frame()?,
            &PivotOptions {
                on: selector::split_list_arguments(&on),
                index: selector::split_list_arguments(&index),
                values: selector::split_list_arguments(&values),
                aggregate,
                separator,
            },
        )?),
        Commands::Unpivot {
            index,
            on,
            variable_name,
            value_name,
        } => emit(reshape::unpivot(
            frame()?,
            &selector::split_list_arguments(&index),
            &selector::split_list_arguments(&on),
            variable_name.as_deref(),
            value_name.as_deref(),
        )?),
        Commands::Dummies {
            columns,
            drop_first,
            drop_nulls,
            separator,
        } => emit(reshape::dummies(
            frame()?,
            &columns,
            separator.as_deref(),
            drop_first,
            drop_nulls,
        )?),
        Commands::Summary { quantiles } => {
            emit(aggregate::summary(&frame()?, &parse_f64_list(&quantiles)?)?)
        }
        Commands::ValueCounts {
            columns,
            column,
            sort,
            normalize,
        } => emit(aggregate::value_counts(
            frame()?,
            &columns.columns,
            column.as_deref(),
            sort,
            normalize,
        )?),
        Commands::Sum(args) => run_aggregate(cli, &args.columns, Aggregation::Sum),
        Commands::Min(args) => run_aggregate(cli, &args.columns, Aggregation::Min),
        Commands::Max(args) => run_aggregate(cli, &args.columns, Aggregation::Max),
        Commands::Mean(args) => run_aggregate(cli, &args.columns, Aggregation::Mean),
        Commands::Median(args) => run_aggregate(cli, &args.columns, Aggregation::Median),
        Commands::Std { columns, ddof } => {
            run_aggregate(cli, &columns.columns, Aggregation::Std(ddof))
        }
        Commands::Var { columns, ddof } => {
            run_aggregate(cli, &columns.columns, Aggregation::Var(ddof))
        }
        Commands::Quantile {
            quantile,
            columns,
            method,
        } => run_aggregate(
            cli,
            &columns.columns,
            Aggregation::Quantile(quantile, method.into()),
        ),
        Commands::Entropy {
            columns,
            base,
            normalize,
        } => run_aggregate(
            cli,
            &columns.columns,
            Aggregation::Entropy { base, normalize },
        ),
        Commands::AllTrue(args) => run_aggregate(cli, &args.columns, Aggregation::AllTrue),
        Commands::AllFalse(args) => run_aggregate(cli, &args.columns, Aggregation::AllFalse),
        Commands::ArgMax(args) => run_aggregate(cli, &args.columns, Aggregation::ArgMax),
        Commands::ArgMin(args) => run_aggregate(cli, &args.columns, Aggregation::ArgMin),
        Commands::ArgTrue(args) => emit(aggregate::arg_true(frame()?, &args.columns)?),
        Commands::ArgUnique(args) => emit(aggregate::arg_unique(frame()?, &args.columns)?),
        Commands::ArgSort {
            columns,
            reverse,
            nulls_last,
            limit,
            maintain_order: _,
        } => emit(aggregate::arg_sort(
            frame()?,
            &columns.columns,
            reverse,
            nulls_last,
            limit,
        )?),
        Commands::ArgWhere { predicate } => emit(aggregate::arg_where(frame()?, &predicate)?),
        Commands::IsNull(target) => emit(column::is_null(frame()?, &target.target(), false)?),
        Commands::IsNotNull(target) => emit(column::is_null(frame()?, &target.target(), true)?),
        Commands::IsDuplicated(target) => emit(reshape::duplicate_mask(
            frame()?,
            &target.columns,
            false,
            target.alias.as_deref(),
        )?),
        Commands::IsUnique(target) => emit(reshape::duplicate_mask(
            frame()?,
            &target.columns,
            true,
            target.alias.as_deref(),
        )?),
        Commands::IsIn {
            values,
            target,
            nulls_equal,
        } => emit(column::is_in(
            frame()?,
            &target.target(),
            &parse_list(&values)?,
            nulls_equal,
        )?),
        Commands::Not(target) => emit(column::not(frame()?, &target.target())?),
        Commands::Set {
            value,
            target,
            mask,
        } => {
            let frame = frame()?;
            if frame.width() == 0 {
                return emit(frame);
            }
            let mask = ops::sql_mask(&frame, &mask)?;
            emit(column::set_where(
                frame,
                &target.target(),
                &parse_scalar(&value),
                mask,
                "set",
            )?)
        }
        Commands::SetWithIdx {
            value,
            target,
            indices,
        } => {
            let frame = frame()?;
            if frame.width() == 0 {
                return emit(frame);
            }
            let mask = column::index_mask(frame.height(), &parse_index_list(&indices)?)?;
            emit(column::set_where(
                frame,
                &target.target(),
                &parse_scalar(&value),
                mask,
                "set-with-idx",
            )?)
        }
        Commands::Uppercase(target) => emit(strings::uppercase(frame()?, &target.target())?),
        Commands::Lowercase(target) => emit(strings::lowercase(frame()?, &target.target())?),
        Commands::StrLengths { target, bytes } => {
            emit(strings::lengths(frame()?, &target.target(), bytes)?)
        }
        Commands::StrReplace {
            pattern,
            replace,
            target,
            literal,
        } => emit(strings::replace(
            frame()?,
            &target.target(),
            &pattern,
            &replace,
            false,
            literal,
        )?),
        Commands::StrReplaceAll {
            pattern,
            replace,
            target,
            literal,
        } => emit(strings::replace(
            frame()?,
            &target.target(),
            &pattern,
            &replace,
            true,
            literal,
        )?),
        Commands::StrSlice {
            start,
            target,
            length,
        } => emit(strings::slice(frame()?, &target.target(), start, length)?),
        Commands::StrSplit { separator, target } => {
            emit(strings::split(frame()?, &target.target(), &separator)?)
        }
        Commands::StrStripChars {
            args,
            alias,
            whitespace,
            start,
            end,
        } => {
            let (pattern, columns) = if whitespace {
                (None, args)
            } else {
                let mut args = args.into_iter();
                let pattern = args
                    .next()
                    .context("str-strip-chars requires a PATTERN or --whitespace")?;
                (Some(pattern), args.collect())
            };
            let side = match (start, end) {
                (true, _) => StripSide::Start,
                (_, true) => StripSide::End,
                _ => StripSide::Both,
            };
            emit(strings::strip_chars(
                frame()?,
                &Target::new(columns, alias),
                pattern.as_deref(),
                side,
            )?)
        }
        Commands::Contains {
            pattern,
            target,
            literal,
        } => emit(strings::contains(
            frame()?,
            &target.target(),
            &pattern,
            literal,
        )?),
        Commands::StrJoin {
            columns,
            delimiter,
            ignore_nulls,
        } => emit(strings::join(
            frame()?,
            &columns.columns,
            &delimiter,
            ignore_nulls,
        )?),
        Commands::ConcatStr {
            separator,
            columns,
            alias,
            ignore_nulls,
        } => emit(column::concat_str_columns(
            frame()?,
            &columns,
            &separator,
            ignore_nulls,
            &alias,
        )?),
        Commands::GetYear(target) => run_date_part(cli, &target, DatePart::Year),
        Commands::GetMonth(target) => run_date_part(cli, &target, DatePart::Month),
        Commands::GetWeek(target) => run_date_part(cli, &target, DatePart::Week),
        Commands::GetWeekday(target) => run_date_part(cli, &target, DatePart::Weekday),
        Commands::GetDay(target) => run_date_part(cli, &target, DatePart::Day),
        Commands::GetOrdinal(target) => run_date_part(cli, &target, DatePart::Ordinal),
        Commands::GetHour(target) => run_date_part(cli, &target, DatePart::Hour),
        Commands::GetMinute(target) => run_date_part(cli, &target, DatePart::Minute),
        Commands::GetSecond(target) => run_date_part(cli, &target, DatePart::Second),
        Commands::GetNanosecond(target) => run_date_part(cli, &target, DatePart::Nanosecond),
        Commands::Datepart { part, target } => run_date_part(cli, &target, DatePart::parse(&part)?),
        Commands::Strftime { format, target } => {
            emit(temporal::strftime(frame()?, &target.target(), &format)?)
        }
        Commands::AsDate {
            format,
            target,
            not_exact,
        } => emit(temporal::as_date(
            frame()?,
            &target.target(),
            &format,
            not_exact,
        )?),
        Commands::AsDatetime {
            format,
            target,
            not_exact,
            naive,
            ambiguous,
            datetime,
        } => {
            let datetime = datetime.spec()?;
            emit(temporal::as_datetime(
                frame()?,
                &target.target(),
                &AsDatetime {
                    format,
                    not_exact,
                    naive,
                    time_unit: datetime.time_unit,
                    time_zone: datetime.time_zone,
                    ambiguous,
                },
            )?)
        }
        Commands::ConvertTimeZone { time_zone, target } => emit(temporal::convert_time_zone(
            frame()?,
            &target.target(),
            parse_time_zone(Some(&time_zone))?,
        )?),
        Commands::ReplaceTimeZone {
            time_zone,
            target,
            ambiguous,
            nonexistent,
        } => emit(temporal::replace_time_zone(
            frame()?,
            &target.target(),
            parse_time_zone(Some(&time_zone))?,
            &ambiguous,
            temporal::parse_nonexistent(&nonexistent)?,
        )?),
        Commands::Truncate { every, target } => {
            emit(temporal::truncate(frame()?, &target.target(), &every)?)
        }
        Commands::DateRange { range, alias } => {
            reject_input(cli, "date-range")?;
            emit(temporal::date_range_frame(&range.spec()?, &alias)?)
        }
        Commands::DatetimeRange {
            range,
            datetime,
            alias,
        } => {
            reject_input(cli, "datetime-range")?;
            emit(temporal::datetime_range_frame(
                &range.spec()?,
                &datetime.spec()?,
                &alias,
            )?)
        }
        Commands::DateRanges { range, alias } => emit(temporal::date_ranges_column(
            frame()?,
            &range.spec()?,
            &alias,
        )?),
        Commands::DatetimeRanges {
            range,
            datetime,
            alias,
        } => emit(temporal::datetime_ranges_column(
            frame()?,
            &range.spec()?,
            &datetime.spec()?,
            &alias,
        )?),
        Commands::Math { command } => run_math(cli, command),
        Commands::Cumulative {
            kind,
            target,
            reverse,
        } => emit(column::cumulative(
            frame()?,
            &target.target(),
            kind.into(),
            reverse,
        )?),
        Commands::Rolling {
            kind,
            window,
            target,
            min_periods,
            center,
            index_column,
            period,
            offset,
            closed,
            group_by,
            aggregations,
        } => match index_column {
            Some(index_column) => {
                if kind.is_some() || window.is_some() || !target.columns.is_empty() {
                    bail!(
                        "rolling --index-column takes --period and --agg instead of TYPE, WINDOW, and COLUMNs"
                    );
                }
                let period = period.context("rolling --index-column requires --period")?;
                emit(reshape::rolling_by(
                    frame()?,
                    &RollingBy {
                        index_column,
                        period,
                        offset,
                        closed: temporal::parse_closed(&closed)?,
                        group_by,
                        aggregations,
                    },
                )?)
            }
            None => {
                let (Some(kind), Some(window)) = (kind, window) else {
                    bail!(
                        "rolling requires TYPE and WINDOW, or --index-column with --period and --agg"
                    );
                };
                emit(column::rolling(
                    frame()?,
                    &target.target(),
                    kind.into(),
                    window,
                    min_periods,
                    center,
                )?)
            }
        },
        Commands::Horizontal {
            op,
            columns,
            alias,
            nulls,
        } => {
            let name = alias.unwrap_or_else(|| {
                op.to_possible_value()
                    .map(|value| value.get_name().to_owned())
                    .unwrap_or_default()
            });
            emit(column::horizontal(
                frame()?,
                &columns,
                op.into(),
                nulls,
                &name,
            )?)
        }
        Commands::Cut {
            breaks,
            target,
            labels,
            left_closed,
            include_breaks,
        } => emit(column::cut(
            frame()?,
            &target.target(),
            parse_f64_list(&breaks)?,
            &BinOptions {
                labels: labels.as_deref().map(parse_string_list).transpose()?,
                left_closed,
                include_breaks,
            },
        )?),
        Commands::Qcut {
            quantiles,
            target,
            labels,
            left_closed,
            include_breaks,
            allow_duplicates,
        } => {
            let quantiles = match quantiles.trim().parse::<usize>() {
                Ok(bins) => Quantiles::Bins(bins),
                Err(_) => Quantiles::Probabilities(parse_f64_list(&quantiles)?),
            };
            emit(column::qcut(
                frame()?,
                &target.target(),
                &quantiles,
                &BinOptions {
                    labels: labels.as_deref().map(parse_string_list).transpose()?,
                    left_closed,
                    include_breaks,
                },
                allow_duplicates,
            )?)
        }
        Commands::Replace {
            args,
            alias,
            map,
            strict,
            default,
            return_dtype,
        } => {
            let (old, new, columns) = match map {
                Some(map) => {
                    let (old, new) = parse_replace_map(&map)?;
                    (old, new, args)
                }
                None => {
                    if args.len() < 2 {
                        bail!("replace requires OLD and NEW values, or --map");
                    }
                    let mut args = args.into_iter();
                    let old = parse_list(&args.next().unwrap_or_default())?;
                    let new = parse_list(&args.next().unwrap_or_default())?;
                    (old, new, args.collect())
                }
            };
            emit(column::replace(
                frame()?,
                &Target::new(columns, alias),
                &ReplaceOptions {
                    old,
                    new,
                    strict,
                    default: default.as_deref().map(parse_scalar),
                    return_dtype: return_dtype.as_deref().map(parse_dtype).transpose()?,
                },
            )?)
        }
        Commands::Integer {
            target,
            base,
            dtype,
            strict,
        } => emit(column::integer(
            frame()?,
            &target.target(),
            base,
            Some(parse_dtype(&dtype)?),
            strict,
        )?),
        Commands::Decimal { scale, target } => {
            emit(column::decimal(frame()?, &target.target(), scale)?)
        }
        Commands::StructJsonEncode(target) => {
            emit(column::struct_json_encode(frame()?, &target.target())?)
        }
        Commands::ListContains { element, target } => emit(column::list_contains(
            frame()?,
            &target.target(),
            &parse_scalar(&element),
        )?),
        Commands::Completions { .. } => unreachable!("handled before input loading"),
    }
}

fn run_aggregate(cli: &Cli, columns: &[String], aggregation: Aggregation) -> anyhow::Result<()> {
    let frame = require_frame(load_primary(cli, None, &ReadOptions::default())?)?;
    emit_frame(
        cli,
        aggregate::aggregate(frame, columns, aggregation)?,
        None,
        None,
    )
}

fn run_date_part(cli: &Cli, target: &TargetArgs, part: DatePart) -> anyhow::Result<()> {
    let frame = require_frame(load_primary(cli, None, &ReadOptions::default())?)?;
    emit_frame(
        cli,
        temporal::date_part(frame, &target.target(), part)?,
        None,
        None,
    )
}

fn run_math(cli: &Cli, command: MathCommand) -> anyhow::Result<()> {
    let elementwise = |target: TargetArgs, op: MathOp| -> anyhow::Result<()> {
        let frame = require_frame(load_primary(cli, None, &ReadOptions::default())?)?;
        emit_frame(cli, column::math(frame, &target.target(), op)?, None, None)
    };
    match command {
        MathCommand::Abs(target) => elementwise(target, MathOp::Abs),
        MathCommand::Cos(target) => elementwise(target, MathOp::Cos),
        MathCommand::Sin(target) => elementwise(target, MathOp::Sin),
        MathCommand::Exp(target) => elementwise(target, MathOp::Exp),
        MathCommand::Log { base, target } => elementwise(target, MathOp::Log(base)),
        MathCommand::Log1p(target) => elementwise(target, MathOp::Log1p),
        MathCommand::Sign(target) => elementwise(target, MathOp::Sign),
        MathCommand::Sqrt(target) => elementwise(target, MathOp::Sqrt),
        MathCommand::BitwiseCountOnes(target) => elementwise(target, MathOp::CountOnes),
        MathCommand::BitwiseCountZeros(target) => elementwise(target, MathOp::CountZeros),
        MathCommand::BitwiseLeadingOnes(target) => elementwise(target, MathOp::LeadingOnes),
        MathCommand::BitwiseLeadingZeros(target) => elementwise(target, MathOp::LeadingZeros),
        MathCommand::BitwiseTrailingOnes(target) => elementwise(target, MathOp::TrailingOnes),
        MathCommand::BitwiseTrailingZeros(target) => elementwise(target, MathOp::TrailingZeros),
        MathCommand::BitwiseAnd(args) => run_aggregate(cli, &args.columns, Aggregation::BitwiseAnd),
        MathCommand::BitwiseOr(args) => run_aggregate(cli, &args.columns, Aggregation::BitwiseOr),
        MathCommand::BitwiseXor(args) => run_aggregate(cli, &args.columns, Aggregation::BitwiseXor),
        MathCommand::Dot { left, right } => {
            let frame = require_frame(load_primary(cli, None, &ReadOptions::default())?)?;
            emit_frame(cli, aggregate::dot(frame, &left, &right)?, None, None)
        }
    }
}

fn run_open(cli: &Cli, args: &OpenArgs) -> anyhow::Result<()> {
    if let (Some(file_type), Some(from)) = (args.file_type, cli.from)
        && file_type != from
    {
        bail!("open --type and --from disagree; pass only one");
    }
    let options = ReadOptions {
        delimiter: args.delimiter.as_deref().map(parse_delimiter).transpose()?,
        no_header: args.no_input_header,
        infer_schema_length: args.infer_schema,
        skip_rows: args.skip_rows,
        columns: (!args.columns.is_empty()).then(|| args.columns.clone()),
        schema: args.schema.as_deref().map(parse_schema_spec).transpose()?,
        truncate_ragged_lines: args.truncate_ragged_lines,
    };
    let forced = args.file_type.or(cli.from);
    let loaded = load_primary_with_format(cli, args.source.as_deref(), forced, &options)?;
    let loaded = match (loaded, args.as_columns) {
        (LoadedInput::Frame(frame), true) => LoadedInput::Frame(as_columns(frame)?),
        (loaded, _) => loaded,
    };
    emit_loaded(cli, loaded, None, None)
}

/// Turn a one-row frame of equal-length list columns into a frame with one row per element.
fn as_columns(frame: DataFrame) -> anyhow::Result<DataFrame> {
    if frame.width() == 0 {
        return Ok(frame);
    }
    if frame.height() != 1
        || !frame
            .columns()
            .iter()
            .all(|column| matches!(column.dtype(), DataType::List(_)))
    {
        bail!("--as-columns expects one JSON object whose values are arrays of equal length");
    }
    let names = frame
        .get_column_names()
        .into_iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    reshape::explode(frame, &names, false, true)
        .context("--as-columns expects one JSON object whose values are arrays of equal length")
}

fn run_save(cli: &Cli, args: &SaveArgs) -> anyhow::Result<()> {
    if cli.output.is_some() {
        bail!("save writes to its PATH; drop --output");
    }
    if args.path == Path::new("-") {
        bail!("save needs a file path; use `pl to FORMAT` to write stdout");
    }
    let format = match args.file_type.or(cli.to) {
        Some(format) => format,
        None => output_format_from_path(&args.path).with_context(|| {
            format!(
                "cannot infer the save format from {}; use --type",
                args.path.display()
            )
        })?,
    };
    if cli.raw && format != OutputFormat::Text {
        bail!("--raw conflicts with non-text output");
    }
    if cli.pretty && format != OutputFormat::Json {
        bail!("--pretty is only valid with JSON output");
    }
    let delimiter = match (&args.csv_delimiter, &cli.output_delimiter) {
        (Some(delimiter), _) | (None, Some(delimiter)) => Some(parse_delimiter(delimiter)?),
        (None, None) => None,
    };
    let options = RenderOptions {
        header: !(args.csv_no_header || cli.no_header),
        delimiter,
        pretty: cli.pretty,
        avro_compression: args.avro_compression.map(Into::into),
    };
    let rendered = match load_primary(cli, None, &ReadOptions::default())? {
        LoadedInput::Frame(mut frame) => render_frame(&mut frame, format, &options)?,
        LoadedInput::HtmlDocument(document) => {
            if format != OutputFormat::Markdown {
                bail!(
                    "HTML document input only supports Markdown output. Use --from table to parse its first <table>"
                );
            }
            Rendered::Text(html_document_to_markdown(&document)?)
        }
    };
    write_rendered(Some(&args.path), rendered, format)
}

fn reject_input(cli: &Cli, command: &str) -> anyhow::Result<()> {
    if cli.input.is_some() || cli.data.is_some() || !cli.inputs.is_empty() {
        bail!("{command} generates data and does not read input");
    }
    Ok(())
}

fn run_legacy_conversion(cli: &Cli) -> anyhow::Result<()> {
    if !cli.tables.is_empty() {
        bail!("--table can only be used together with --sql");
    }
    if cli.inputs.len() > 1 {
        bail!("only one positional input is supported outside of --sql mode");
    }
    let loaded = load_primary(cli, None, &ReadOptions::default())?;
    emit_loaded(cli, loaded, None, None)
}

fn run_legacy_sql(cli: &Cli, statement: &str) -> anyhow::Result<()> {
    let mut engine = SqlEngine::new();

    for input in &cli.inputs {
        let path = Path::new(input);
        if !path.exists() {
            bail!("legacy --sql positional input does not exist: {input}");
        }
        if path.is_dir() {
            engine.register_directory(path)?;
        } else {
            engine.register_path(path, None)?;
        }
    }

    if cli.input.is_some() || cli.data.is_some() {
        let frame = require_frame(load_primary(cli, None, &ReadOptions::default())?)?;
        use polars::prelude::IntoLazy;
        engine.register_frame("df", frame.lazy());
    }

    for entry in &cli.tables {
        let (name, path) = parse_table_spec(entry)?;
        engine.register_path(path, Some(name))?;
    }

    let frame = engine.execute_collect(statement)?;
    emit_frame(cli, frame, None, Some(OutputFormat::Markdown))
}

fn run_concat(
    cli: &Cli,
    inputs: &[String],
    diagonal: bool,
    to_supertypes: bool,
) -> anyhow::Result<()> {
    let mut frames = Vec::new();
    if cli.input.is_some() || cli.data.is_some() || inputs.len() == 1 {
        frames.push(require_frame(load_primary(
            cli,
            None,
            &ReadOptions::default(),
        )?)?);
    }
    for input in inputs {
        frames.push(require_frame(load_token(
            input,
            None,
            false,
            &ReadOptions::default(),
        )?)?);
    }
    emit_frame(
        cli,
        reshape::concat_frames(frames, diagonal, to_supertypes)?,
        None,
        None,
    )
}

fn resolve_join_keys(
    on: &[String],
    left_on: &[String],
    right_on: &[String],
) -> anyhow::Result<(Vec<String>, Vec<String>)> {
    if !on.is_empty() {
        if !left_on.is_empty() || !right_on.is_empty() {
            bail!("use either --on or --left-on/--right-on, not both");
        }
        return Ok((on.to_vec(), on.to_vec()));
    }
    if left_on.is_empty() != right_on.is_empty() {
        bail!("--left-on and --right-on must be supplied together");
    }
    Ok((left_on.to_vec(), right_on.to_vec()))
}

fn load_primary(
    cli: &Cli,
    command_input: Option<&str>,
    options: &ReadOptions,
) -> anyhow::Result<LoadedInput> {
    load_primary_with_format(cli, command_input, cli.from, options)
}

fn load_primary_with_format(
    cli: &Cli,
    command_input: Option<&str>,
    format: Option<InputFormat>,
    options: &ReadOptions,
) -> anyhow::Result<LoadedInput> {
    let sources = usize::from(command_input.is_some())
        + usize::from(cli.input.is_some())
        + usize::from(cli.data.is_some())
        + usize::from(!cli.inputs.is_empty());
    if sources > 1 {
        bail!("input was provided more than once; choose a positional input, --input, or --data");
    }
    if cli.inputs.len() > 1 {
        bail!(
            "commands accept at most one leading positional input; use concat for multiple inputs"
        );
    }
    if let Some(data) = cli.data.as_deref() {
        return load_literal(data, format, "inline data", options);
    }
    if let Some(input) = cli
        .input
        .as_deref()
        .or(command_input)
        .or_else(|| cli.inputs.first().map(String::as_str))
    {
        return load_token(input, format, true, options);
    }
    load_stdin(format, options)
}

fn load_token(
    input: &str,
    forced_format: Option<InputFormat>,
    allow_stdin: bool,
    options: &ReadOptions,
) -> anyhow::Result<LoadedInput> {
    if input == "-" {
        if !allow_stdin {
            bail!(
                "'-' is not valid for this secondary input; pipe it as the primary input instead"
            );
        }
        return load_stdin(forced_format, options);
    }

    if let Some(path) = input.strip_prefix('@') {
        return load_path(Path::new(path), forced_format, options);
    }

    let path = Path::new(input);
    if path.exists() {
        return load_path(path, forced_format, options);
    }
    if looks_like_path(input) && !looks_like_structured_literal(input) {
        bail!("input path does not exist: {input}. Use --data to force a literal string");
    }
    load_literal(input, forced_format, "inline input", options)
}

fn load_path(
    path: &Path,
    forced_format: Option<InputFormat>,
    options: &ReadOptions,
) -> anyhow::Result<LoadedInput> {
    let bytes =
        std::fs::read(path).with_context(|| format!("failed to read input {}", path.display()))?;
    let format = match forced_format {
        Some(format) => format,
        None => detect_input_format(path).or_else(|_| detect_input_format_from_bytes(&bytes))?,
    };
    load_bytes(bytes, format, &path.display().to_string(), options)
}

fn load_literal(
    input: &str,
    forced_format: Option<InputFormat>,
    label: &str,
    options: &ReadOptions,
) -> anyhow::Result<LoadedInput> {
    let bytes = input.as_bytes().to_vec();
    let format = forced_format
        .map(Ok)
        .unwrap_or_else(|| detect_input_format_from_bytes(&bytes))?;
    load_bytes(bytes, format, label, options)
}

fn load_stdin(
    forced_format: Option<InputFormat>,
    options: &ReadOptions,
) -> anyhow::Result<LoadedInput> {
    if std::io::stdin().is_terminal() {
        bail!("no input provided; pipe data on stdin or use --input/--data");
    }
    let mut bytes = Vec::new();
    std::io::stdin()
        .read_to_end(&mut bytes)
        .context("failed to read stdin")?;
    if bytes.is_empty() {
        return Ok(LoadedInput::Frame(DataFrame::empty()));
    }
    let format = forced_format
        .map(Ok)
        .unwrap_or_else(|| detect_input_format_from_bytes(&bytes))?;
    load_bytes(bytes, format, "stdin", options)
}

fn load_bytes(
    bytes: Vec<u8>,
    format: InputFormat,
    label: &str,
    options: &ReadOptions,
) -> anyhow::Result<LoadedInput> {
    if format == InputFormat::Html {
        if !options.is_default() {
            bail!("read options do not apply to HTML document input");
        }
        let document = String::from_utf8(bytes)
            .with_context(|| format!("HTML input {label} is not valid UTF-8"))?;
        return Ok(LoadedInput::HtmlDocument(document));
    }
    Ok(LoadedInput::Frame(read_bytes_with_options(
        &bytes, format, label, options,
    )?))
}

fn require_frame(input: LoadedInput) -> anyhow::Result<DataFrame> {
    match input {
        LoadedInput::Frame(frame) => Ok(frame),
        LoadedInput::HtmlDocument(_) => bail!(
            "HTML document input is not tabular. Use --from table for its first <table>, or convert it with `pl to markdown`"
        ),
    }
}

fn emit_loaded(
    cli: &Cli,
    input: LoadedInput,
    command_format: Option<OutputFormat>,
    default_format: Option<OutputFormat>,
) -> anyhow::Result<()> {
    match input {
        LoadedInput::Frame(frame) => emit_frame(cli, frame, command_format, default_format),
        LoadedInput::HtmlDocument(document) => {
            let format = resolve_output_format(cli, command_format, Some(OutputFormat::Markdown))?;
            if format != OutputFormat::Markdown {
                bail!(
                    "HTML document input only supports Markdown output. Use --from table to parse its first <table>"
                );
            }
            write_rendered(
                cli.output.as_deref(),
                Rendered::Text(html_document_to_markdown(&document)?),
                format,
            )
        }
    }
}

fn emit_frame(
    cli: &Cli,
    mut frame: DataFrame,
    command_format: Option<OutputFormat>,
    default_format: Option<OutputFormat>,
) -> anyhow::Result<()> {
    let format = resolve_output_format(cli, command_format, default_format)?;
    if cli.pretty && format != OutputFormat::Json {
        bail!("--pretty is only valid with JSON output");
    }
    let options = RenderOptions {
        header: !cli.no_header,
        delimiter: cli
            .output_delimiter
            .as_deref()
            .map(parse_delimiter)
            .transpose()?,
        pretty: cli.pretty,
        avro_compression: None,
    };
    let rendered = render_frame(&mut frame, format, &options)?;
    write_rendered(cli.output.as_deref(), rendered, format)
}

fn render_frame(
    frame: &mut DataFrame,
    format: OutputFormat,
    options: &RenderOptions,
) -> anyhow::Result<Rendered> {
    if options.delimiter.is_some() && !matches!(format, OutputFormat::Csv | OutputFormat::Tsv) {
        bail!("an output delimiter only applies to CSV or TSV output");
    }
    let text = match format {
        OutputFormat::Csv => {
            to_delimited(frame, options.delimiter.unwrap_or(b','), options.header)?
        }
        OutputFormat::Tsv => {
            to_delimited(frame, options.delimiter.unwrap_or(b'\t'), options.header)?
        }
        OutputFormat::Json if options.pretty => to_pretty_json(frame)?,
        OutputFormat::Json => to_json(frame, Some(JsonFormat::Json))?,
        OutputFormat::Jsonl => to_json(frame, Some(JsonFormat::JsonLines))?,
        OutputFormat::Yaml => to_yaml(frame)?,
        OutputFormat::Toml => to_toml(frame)?,
        OutputFormat::Markdown => to_markdown(frame)?,
        OutputFormat::Table => to_table(frame)?,
        OutputFormat::Text => to_text_lines(frame)?,
        OutputFormat::Html => to_html(frame)?,
        OutputFormat::Xml => to_xml(frame)?,
        OutputFormat::Repr => to_repr(frame),
        OutputFormat::Parquet => return Ok(Rendered::Binary(to_parquet(frame)?)),
        OutputFormat::Arrow => return Ok(Rendered::Binary(to_arrow(frame)?)),
        OutputFormat::Avro => {
            return Ok(Rendered::Binary(to_avro(frame, options.avro_compression)?));
        }
    };
    Ok(Rendered::Text(text))
}

fn resolve_output_format(
    cli: &Cli,
    command_format: Option<OutputFormat>,
    default_format: Option<OutputFormat>,
) -> anyhow::Result<OutputFormat> {
    // A global --to is the explicit override when both forms are present.
    let explicit = cli.to.or(command_format);
    if cli.raw {
        if explicit.is_some_and(|format| format != OutputFormat::Text) {
            bail!("--raw conflicts with non-text output; omit --to or use --to text");
        }
        return Ok(OutputFormat::Text);
    }
    if let Some(format) = explicit {
        return Ok(format);
    }
    if let Some(path) = cli.output.as_deref()
        && path != Path::new("-")
    {
        return output_format_from_path(path).with_context(|| {
            format!(
                "cannot infer output format from {}; use --to",
                path.display()
            )
        });
    }
    if cli.pretty {
        return Ok(OutputFormat::Json);
    }
    if let Some(format) = default_format {
        return Ok(format);
    }
    Ok(if std::io::stdout().is_terminal() {
        OutputFormat::Table
    } else {
        OutputFormat::Jsonl
    })
}

fn output_format_from_path(path: &Path) -> Option<OutputFormat> {
    match path
        .extension()
        .and_then(|extension| extension.to_str())?
        .to_ascii_lowercase()
        .as_str()
    {
        "csv" => Some(OutputFormat::Csv),
        "tsv" | "tab" => Some(OutputFormat::Tsv),
        "json" => Some(OutputFormat::Json),
        "jsonl" | "ndjson" => Some(OutputFormat::Jsonl),
        "yaml" | "yml" => Some(OutputFormat::Yaml),
        "toml" => Some(OutputFormat::Toml),
        "md" | "markdown" => Some(OutputFormat::Markdown),
        "table" => Some(OutputFormat::Table),
        "txt" | "text" => Some(OutputFormat::Text),
        "html" | "htm" => Some(OutputFormat::Html),
        "xml" => Some(OutputFormat::Xml),
        "parquet" | "pq" => Some(OutputFormat::Parquet),
        "arrow" | "ipc" | "feather" => Some(OutputFormat::Arrow),
        "avro" => Some(OutputFormat::Avro),
        _ => None,
    }
}

fn write_rendered(
    path: Option<&Path>,
    rendered: Rendered,
    format: OutputFormat,
) -> anyhow::Result<()> {
    let to_file = path.filter(|path| *path != Path::new("-"));
    let bytes = match rendered {
        Rendered::Text(text) => terminate_rendered_output(text).into_bytes(),
        Rendered::Binary(bytes) => {
            if to_file.is_none() && std::io::stdout().is_terminal() {
                bail!(
                    "refusing to write binary {} output to a terminal; redirect stdout or use --output",
                    format
                        .to_possible_value()
                        .map(|value| value.get_name().to_owned())
                        .unwrap_or_default()
                );
            }
            bytes
        }
    };
    match to_file {
        Some(path) => std::fs::write(path, bytes)
            .with_context(|| format!("failed to write output {}", path.display())),
        None => {
            let mut stdout = std::io::stdout().lock();
            match stdout.write_all(&bytes) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
                Err(error) => Err(error).context("failed to write stdout"),
            }
        }
    }
}

/// Make non-empty rendered output a POSIX text stream without inventing a blank record.
///
/// Formats that already terminate the last record (JSONL, CSV, `--raw`) are left unchanged.
/// Empty output stays empty so a zero-row JSONL wire stays schema-less.
fn terminate_rendered_output(mut output: String) -> String {
    if !output.is_empty() && !output.ends_with('\n') {
        output.push('\n');
    }
    output
}

fn parse_table_spec(spec: &str) -> anyhow::Result<(&str, &Path)> {
    let (name, path) = spec
        .split_once('=')
        .with_context(|| format!("--table expects NAME=PATH but got {spec:?}"))?;
    if name.is_empty() || path.is_empty() {
        bail!("--table expects a non-empty name and path, got {spec:?}");
    }
    Ok((name, Path::new(path)))
}

/// Parse a single-byte field delimiter; `\t` and `tab` mean a tab.
fn parse_delimiter(value: &str) -> anyhow::Result<u8> {
    match value {
        "\\t" | "tab" => return Ok(b'\t'),
        _ => {}
    }
    match value.as_bytes() {
        [byte] => Ok(*byte),
        _ => bail!("a delimiter must be a single ASCII character, got {value:?}"),
    }
}

/// Parse `NAME:TYPE[,NAME:TYPE...]` or a JSON object of names to types.
fn parse_schema_spec(spec: &str) -> anyhow::Result<Vec<(String, DataType)>> {
    let trimmed = spec.trim();
    if trimmed.starts_with('{') {
        let object = serde_json::from_str::<serde_json::Map<String, JsonValue>>(trimmed)
            .context("--schema JSON must be an object of column names to type names")?;
        return object
            .into_iter()
            .map(|(name, dtype)| match dtype {
                JsonValue::String(dtype) => Ok((name, parse_dtype(&dtype)?)),
                other => bail!("--schema type for {name:?} must be a string, got {other}"),
            })
            .collect();
    }
    let fields = split_top_level_commas(trimmed);
    fields
        .into_iter()
        .filter(|field| !field.trim().is_empty())
        .map(|field| {
            let (name, dtype) = field
                .split_once(':')
                .with_context(|| format!("--schema expects NAME:TYPE but got {field:?}"))?;
            Ok((name.trim().to_owned(), parse_dtype(dtype)?))
        })
        .collect()
}

/// Parse a `--map` JSON object into parallel old/new value lists.
fn parse_replace_map(map: &str) -> anyhow::Result<(Vec<JsonValue>, Vec<JsonValue>)> {
    let object = serde_json::from_str::<serde_json::Map<String, JsonValue>>(map)
        .context("--map must be a JSON object such as '{\"old\":\"new\"}'")?;
    let mut old = Vec::with_capacity(object.len());
    let mut new = Vec::with_capacity(object.len());
    for (key, value) in object {
        if value.is_array() || value.is_object() {
            bail!("--map values must be scalars, got {value}");
        }
        old.push(parse_scalar(&key));
        new.push(value);
    }
    Ok((old, new))
}

fn looks_like_structured_literal(input: &str) -> bool {
    let input = input.trim_start();
    input.starts_with(['{', '[', '<', '|'])
        || input.starts_with("---")
        || input.contains('\n')
        || input.contains('\t')
}

fn looks_like_path(input: &str) -> bool {
    input.contains('/')
        || input.contains('\\')
        || input.starts_with('.')
        || input.starts_with('~')
        || Path::new(input).extension().is_some()
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::{
        Cli, RUN_STACK_SIZE, parse_delimiter, parse_schema_spec, terminate_rendered_output,
    };

    #[test]
    fn command_definition_is_consistent() {
        std::thread::Builder::new()
            .stack_size(RUN_STACK_SIZE)
            .spawn(|| Cli::command().debug_assert())
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn terminate_rendered_output_leaves_empty_output_empty() {
        assert_eq!(terminate_rendered_output(String::new()), "");
    }

    #[test]
    fn terminate_rendered_output_appends_a_newline_when_missing() {
        assert_eq!(terminate_rendered_output("[]".to_owned()), "[]\n");
    }

    #[test]
    fn terminate_rendered_output_does_not_add_a_second_newline() {
        assert_eq!(terminate_rendered_output("Ada\n".to_owned()), "Ada\n");
    }

    #[test]
    fn parses_delimiters_and_schema_specs() {
        assert_eq!(parse_delimiter(";").unwrap(), b';');
        assert_eq!(parse_delimiter("\\t").unwrap(), b'\t');
        assert!(parse_delimiter("ab").is_err());
        let schema = parse_schema_spec("id:str, at:datetime<ms, UTC>").unwrap();
        assert_eq!(schema.len(), 2);
        assert_eq!(schema[0].0, "id");
        assert_eq!(parse_schema_spec(r#"{"n":"i32"}"#).unwrap().len(), 1);
    }
}
