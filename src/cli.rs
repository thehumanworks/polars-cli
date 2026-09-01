use std::{
    io::{IsTerminal, Read, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, bail};
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::{Shell, generate};
use polars::frame::DataFrame;

use crate::{
    ops::{self, JoinKind, KeepStrategy},
    read::{InputFormat, detect_input_format, detect_input_format_from_bytes, read_bytes},
    sql::SqlEngine,
    transformer::{
        JsonFormat, html_document_to_markdown, to_csv, to_html, to_json, to_markdown,
        to_pretty_json, to_table, to_text_lines, to_toml, to_tsv, to_xml, to_yaml,
    },
};

const EXAMPLES: &str = r#"Examples:
  pl open people.csv | pl filter 'age >= 18' | pl select name age
  pl filter 'total > 100' --input orders.csv --to csv
  pl select name --data '[{"name":"Ada","age":36}]' --raw
  pl query 'SELECT team, COUNT(*) AS n FROM df GROUP BY team' -i people.yaml
  pl completions bash
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
    Full,
    Cross,
}

impl From<JoinType> for JoinKind {
    fn from(value: JoinType) -> Self {
        match value {
            JoinType::Inner => Self::Inner,
            JoinType::Left => Self::Left,
            JoinType::Full => Self::Full,
            JoinType::Cross => Self::Cross,
        }
    }
}

#[derive(Debug, Clone, Subcommand)]
enum Commands {
    /// Read a file, inline value, or stdin into a dataframe.
    Open {
        /// Path, -, or a structured inline literal.
        #[arg(allow_hyphen_values = true, value_name = "INPUT")]
        source: Option<String>,
    },

    /// Keep rows matching a Polars SQL predicate.
    #[command(visible_alias = "where")]
    Filter {
        /// SQL predicate evaluated against the input frame.
        predicate: String,
    },

    /// Select columns or Polars SQL expressions.
    #[command(visible_alias = "get")]
    Select {
        /// Column names or SQL expressions. Quote expressions containing shell metacharacters.
        #[arg(required = true, num_args = 1..)]
        expressions: Vec<String>,
    },

    /// Add or replace columns with NAME=SQL_EXPRESSION assignments.
    #[command(name = "with-column", visible_alias = "mutate")]
    WithColumn {
        #[arg(required = true, num_args = 1.., value_name = "NAME=EXPRESSION")]
        assignments: Vec<String>,
    },

    /// Sort by columns or SQL expressions.
    #[command(name = "sort-by", visible_alias = "sort")]
    SortBy {
        #[arg(required = true, num_args = 1..)]
        expressions: Vec<String>,

        /// Sort every expression descending. Per-expression :asc/:desc suffixes override this.
        #[arg(short = 'r', long)]
        descending: bool,
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
    Slice { offset: i64, rows: usize },

    /// Remove columns by name.
    Drop {
        #[arg(required = true, num_args = 1..)]
        columns: Vec<String>,
    },

    /// Rename columns with OLD=NEW mappings.
    Rename {
        #[arg(required = true, num_args = 1.., value_name = "OLD=NEW")]
        mappings: Vec<String>,
    },

    /// Remove duplicate rows, optionally considering only selected columns.
    #[command(visible_alias = "distinct")]
    Unique {
        #[arg(num_args = 0..)]
        columns: Vec<String>,

        #[arg(long, value_enum, default_value_t = KeepArg::First)]
        keep: KeepArg,
    },

    /// Remove rows containing nulls, optionally restricted to selected columns.
    #[command(name = "drop-nulls")]
    DropNulls {
        #[arg(num_args = 0..)]
        columns: Vec<String>,
    },

    /// Reverse row order.
    Reverse,

    /// Group by keys and evaluate SQL aggregate expressions.
    #[command(name = "group-by")]
    GroupBy {
        #[arg(required = true, num_args = 1..)]
        keys: Vec<String>,

        /// SQL aggregation such as 'SUM(amount) AS total'. Repeat for multiple aggregations.
        #[arg(short = 'a', long = "agg", required = true)]
        aggregations: Vec<String>,
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
    },

    /// Vertically concatenate two or more inputs.
    Concat {
        /// Paths or structured inline literals. With one value, stdin is the first frame.
        #[arg(required = true, num_args = 1.., value_name = "INPUT")]
        sources: Vec<String>,
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

    /// Count rows.
    Count,

    /// Render the input in a chosen format.
    To { format: OutputFormat },

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

pub fn run() -> anyhow::Result<()> {
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
        Some(Commands::Open { source }) => {
            let loaded = load_primary(&cli, source.as_deref())?;
            emit_loaded(&cli, loaded, None, None)
        }
        Some(Commands::Filter { predicate }) => {
            let frame = require_frame(load_primary(&cli, None)?)?;
            emit_frame(&cli, ops::filter(frame, &predicate)?, None, None)
        }
        Some(Commands::Select { expressions }) => {
            let frame = require_frame(load_primary(&cli, None)?)?;
            emit_frame(&cli, ops::select(frame, &expressions)?, None, None)
        }
        Some(Commands::WithColumn { assignments }) => {
            let frame = require_frame(load_primary(&cli, None)?)?;
            emit_frame(&cli, ops::with_columns(frame, &assignments)?, None, None)
        }
        Some(Commands::SortBy {
            expressions,
            descending,
        }) => {
            let frame = require_frame(load_primary(&cli, None)?)?;
            emit_frame(
                &cli,
                ops::sort_by(frame, &expressions, descending)?,
                None,
                None,
            )
        }
        Some(Commands::First { rows }) => {
            let frame = require_frame(load_primary(&cli, None)?)?;
            emit_frame(&cli, ops::first(frame, rows), None, None)
        }
        Some(Commands::Last { rows }) => {
            let frame = require_frame(load_primary(&cli, None)?)?;
            emit_frame(&cli, ops::last(frame, rows), None, None)
        }
        Some(Commands::Slice { offset, rows }) => {
            let frame = require_frame(load_primary(&cli, None)?)?;
            emit_frame(&cli, ops::slice(frame, offset, rows), None, None)
        }
        Some(Commands::Drop { columns }) => {
            let frame = require_frame(load_primary(&cli, None)?)?;
            emit_frame(&cli, ops::drop_columns(frame, &columns)?, None, None)
        }
        Some(Commands::Rename { mappings }) => {
            let frame = require_frame(load_primary(&cli, None)?)?;
            emit_frame(&cli, ops::rename_columns(frame, &mappings)?, None, None)
        }
        Some(Commands::Unique { columns, keep }) => {
            let frame = require_frame(load_primary(&cli, None)?)?;
            emit_frame(&cli, ops::unique(frame, &columns, keep.into())?, None, None)
        }
        Some(Commands::DropNulls { columns }) => {
            let frame = require_frame(load_primary(&cli, None)?)?;
            emit_frame(&cli, ops::drop_nulls(frame, &columns)?, None, None)
        }
        Some(Commands::Reverse) => {
            let frame = require_frame(load_primary(&cli, None)?)?;
            emit_frame(&cli, ops::reverse(frame), None, None)
        }
        Some(Commands::GroupBy { keys, aggregations }) => {
            let frame = require_frame(load_primary(&cli, None)?)?;
            emit_frame(
                &cli,
                ops::group_by(frame, &keys, &aggregations)?,
                None,
                None,
            )
        }
        Some(Commands::Join {
            right,
            on,
            left_on,
            right_on,
            kind,
        }) => {
            let left = require_frame(load_primary(&cli, None)?)?;
            let right = require_frame(load_token(&right, None, false)?)?;
            let (left_keys, right_keys) = resolve_join_keys(&on, &left_on, &right_on)?;
            emit_frame(
                &cli,
                ops::join(left, right, kind.into(), &left_keys, &right_keys)?,
                None,
                None,
            )
        }
        Some(Commands::Concat { sources }) => run_concat(&cli, &sources),
        Some(Commands::Query { statement }) => {
            let frame = require_frame(load_primary(&cli, None)?)?;
            emit_frame(&cli, ops::query(frame, &statement)?, None, None)
        }
        Some(Commands::Columns) => {
            let frame = require_frame(load_primary(&cli, None)?)?;
            emit_frame(&cli, ops::columns(&frame)?, None, None)
        }
        Some(Commands::Schema) => {
            let frame = require_frame(load_primary(&cli, None)?)?;
            emit_frame(&cli, ops::schema(&frame)?, None, None)
        }
        Some(Commands::Shape) => {
            let frame = require_frame(load_primary(&cli, None)?)?;
            emit_frame(&cli, ops::shape(&frame)?, None, None)
        }
        Some(Commands::Count) => {
            let frame = require_frame(load_primary(&cli, None)?)?;
            emit_frame(&cli, ops::count(&frame)?, None, None)
        }
        Some(Commands::To { format }) => {
            let loaded = load_primary(&cli, None)?;
            emit_loaded(&cli, loaded, Some(format), None)
        }
        Some(Commands::Completions { .. }) => unreachable!("handled before input loading"),
    }
}

fn run_legacy_conversion(cli: &Cli) -> anyhow::Result<()> {
    if !cli.tables.is_empty() {
        bail!("--table can only be used together with --sql");
    }
    if cli.inputs.len() > 1 {
        bail!("only one positional input is supported outside of --sql mode");
    }
    let loaded = load_primary(cli, None)?;
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
        let frame = require_frame(load_primary(cli, None)?)?;
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

fn run_concat(cli: &Cli, inputs: &[String]) -> anyhow::Result<()> {
    let mut frames = Vec::new();
    if cli.input.is_some() || cli.data.is_some() || inputs.len() == 1 {
        frames.push(require_frame(load_primary(cli, None)?)?);
    }
    for input in inputs {
        frames.push(require_frame(load_token(input, None, false)?)?);
    }
    emit_frame(cli, ops::concat(frames)?, None, None)
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

fn load_primary(cli: &Cli, command_input: Option<&str>) -> anyhow::Result<LoadedInput> {
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
        return load_literal(data, cli.from, "inline data");
    }
    if let Some(input) = cli
        .input
        .as_deref()
        .or(command_input)
        .or_else(|| cli.inputs.first().map(String::as_str))
    {
        return load_token(input, cli.from, true);
    }
    load_stdin(cli.from)
}

fn load_token(
    input: &str,
    forced_format: Option<InputFormat>,
    allow_stdin: bool,
) -> anyhow::Result<LoadedInput> {
    if input == "-" {
        if !allow_stdin {
            bail!(
                "'-' is not valid for this secondary input; pipe it as the primary input instead"
            );
        }
        return load_stdin(forced_format);
    }

    if let Some(path) = input.strip_prefix('@') {
        return load_path(Path::new(path), forced_format);
    }

    let path = Path::new(input);
    if path.exists() {
        return load_path(path, forced_format);
    }
    if looks_like_path(input) && !looks_like_structured_literal(input) {
        bail!("input path does not exist: {input}. Use --data to force a literal string");
    }
    load_literal(input, forced_format, "inline input")
}

fn load_path(path: &Path, forced_format: Option<InputFormat>) -> anyhow::Result<LoadedInput> {
    let bytes =
        std::fs::read(path).with_context(|| format!("failed to read input {}", path.display()))?;
    let format = match forced_format {
        Some(format) => format,
        None => detect_input_format(path).or_else(|_| detect_input_format_from_bytes(&bytes))?,
    };
    load_bytes(bytes, format, &path.display().to_string())
}

fn load_literal(
    input: &str,
    forced_format: Option<InputFormat>,
    label: &str,
) -> anyhow::Result<LoadedInput> {
    let bytes = input.as_bytes().to_vec();
    let format = forced_format
        .map(Ok)
        .unwrap_or_else(|| detect_input_format_from_bytes(&bytes))?;
    load_bytes(bytes, format, label)
}

fn load_stdin(forced_format: Option<InputFormat>) -> anyhow::Result<LoadedInput> {
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
    load_bytes(bytes, format, "stdin")
}

fn load_bytes(bytes: Vec<u8>, format: InputFormat, label: &str) -> anyhow::Result<LoadedInput> {
    if format == InputFormat::Html {
        let document = String::from_utf8(bytes)
            .with_context(|| format!("HTML input {label} is not valid UTF-8"))?;
        return Ok(LoadedInput::HtmlDocument(document));
    }
    Ok(LoadedInput::Frame(read_bytes(&bytes, format, label)?))
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
            write_output(cli.output.as_deref(), html_document_to_markdown(&document)?)
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

    let output = match format {
        OutputFormat::Csv => to_csv(&mut frame, Some(!cli.no_header))?,
        OutputFormat::Tsv => to_tsv(&mut frame, Some(!cli.no_header))?,
        OutputFormat::Json if cli.pretty => to_pretty_json(&mut frame)?,
        OutputFormat::Json => to_json(&mut frame, Some(JsonFormat::Json))?,
        OutputFormat::Jsonl => to_json(&mut frame, Some(JsonFormat::JsonLines))?,
        OutputFormat::Yaml => to_yaml(&mut frame)?,
        OutputFormat::Toml => to_toml(&mut frame)?,
        OutputFormat::Markdown => to_markdown(&mut frame)?,
        OutputFormat::Table => to_table(&mut frame)?,
        OutputFormat::Text => to_text_lines(&mut frame)?,
        OutputFormat::Html => to_html(&mut frame)?,
        OutputFormat::Xml => to_xml(&mut frame)?,
    };
    write_output(cli.output.as_deref(), output)
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
        _ => None,
    }
}

fn write_output(path: Option<&Path>, output: String) -> anyhow::Result<()> {
    match path {
        Some(path) if path != Path::new("-") => std::fs::write(path, output)
            .with_context(|| format!("failed to write output {}", path.display())),
        _ => {
            let mut stdout = std::io::stdout().lock();
            match stdout.write_all(output.as_bytes()) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
                Err(error) => Err(error).context("failed to write stdout"),
            }
        }
    }
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
