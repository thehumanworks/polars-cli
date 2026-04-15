use std::{
    io::Read,
    path::{Path, PathBuf},
};

use anyhow::{Context, bail};
use clap::{Parser, ValueEnum};

use crate::read::{InputFormat, read_file, read_stdin};
use crate::sql::SqlEngine;
use crate::transformer::{
    JsonFormat, html_document_to_markdown, to_csv, to_html, to_json, to_markdown, to_text_lines,
    to_xml,
};

#[derive(Debug, Clone, Copy, ValueEnum)]
enum OutputFormat {
    Csv,
    Json,
    Jsonl,
    Markdown,
    Text,
    Html,
    Xml,
}

#[derive(Debug, Parser)]
#[command(
    author,
    version,
    about = "Read tabular data with Polars and render it to stdout"
)]
struct Cli {
    /// Paths to input files or directories. Zero or one for conversion mode; zero or more for
    /// SQL mode (each positional becomes a registered table, directories are expanded).
    inputs: Vec<PathBuf>,

    /// Force the input format instead of inferring it from the file extension.
    #[arg(long, value_enum)]
    from: Option<InputFormat>,

    /// Output format written to stdout. Defaults to markdown when --sql is used.
    #[arg(long, value_enum)]
    to: Option<OutputFormat>,

    /// Omit the header row when rendering CSV output.
    #[arg(long)]
    no_header: bool,

    /// Run a Polars SQL query against the provided inputs instead of converting them.
    #[arg(long)]
    sql: Option<String>,

    /// Register an input file under an explicit table name: --table name=path.
    /// Repeatable. Only meaningful with --sql.
    #[arg(long = "table", value_name = "NAME=PATH")]
    tables: Vec<String>,
}

pub fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();

    if let Some(query) = cli.sql.as_deref() {
        return run_sql(&cli, query);
    }

    if !cli.tables.is_empty() {
        bail!("--table can only be used together with --sql");
    }

    let to = cli
        .to
        .context("--to is required when not running a SQL query")?;
    let inputs = &cli.inputs;
    if inputs.len() > 1 {
        bail!("only one positional input is supported outside of --sql mode");
    }
    let input = inputs.first().cloned();

    let format = resolve_cli_input_format(input.as_deref(), cli.from)?;
    let output = match format {
        InputFormat::Html => {
            if !matches!(to, OutputFormat::Markdown) {
                anyhow::bail!(
                    "HTML document input only supports --to markdown. Use --from table to parse the first HTML <table>"
                );
            }
            html_document_to_markdown(&read_cli_text_input(input.as_deref())?)?
        }
        _ => {
            let mut df = read_cli_input(input.as_deref(), format)?;
            render_dataframe(&mut df, to, cli.no_header)?
        }
    };

    print!("{output}");

    Ok(())
}

fn run_sql(cli: &Cli, query: &str) -> anyhow::Result<()> {
    let to = cli.to.unwrap_or(OutputFormat::Markdown);
    let mut engine = SqlEngine::new();

    for path in &cli.inputs {
        if path.is_dir() {
            engine.register_directory(path)?;
        } else {
            engine.register_path(path, None)?;
        }
    }

    for entry in &cli.tables {
        let (name, path) = parse_table_spec(entry)?;
        engine.register_path(path, Some(name))?;
    }

    let mut df = engine.execute_collect(query)?;
    let output = render_dataframe(&mut df, to, cli.no_header)?;

    print!("{output}");
    Ok(())
}

fn render_dataframe(
    df: &mut polars::frame::DataFrame,
    to: OutputFormat,
    no_header: bool,
) -> anyhow::Result<String> {
    Ok(match to {
        OutputFormat::Csv => to_csv(df, Some(!no_header))?,
        OutputFormat::Json => to_json(df, Some(JsonFormat::Json))?,
        OutputFormat::Jsonl => to_json(df, Some(JsonFormat::JsonLines))?,
        OutputFormat::Markdown => to_markdown(df)?,
        OutputFormat::Text => to_text_lines(df)?,
        OutputFormat::Html => to_html(df)?,
        OutputFormat::Xml => to_xml(df)?,
    })
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

fn resolve_cli_input_format(
    input: Option<&Path>,
    from: Option<InputFormat>,
) -> anyhow::Result<InputFormat> {
    match input {
        Some(path) if path != Path::new("-") => match from {
            Some(format) => Ok(format),
            None => crate::read::detect_input_format(path),
        },
        _ => from.context("stdin input requires --from to specify the input format"),
    }
}

fn read_cli_input(
    input: Option<&Path>,
    format: InputFormat,
) -> anyhow::Result<polars::frame::DataFrame> {
    match input {
        Some(path) if path != Path::new("-") => read_file(path, Some(format)),
        _ => read_stdin(format),
    }
}

fn read_cli_text_input(input: Option<&Path>) -> anyhow::Result<String> {
    match input {
        Some(path) if path != Path::new("-") => std::fs::read_to_string(path)
            .with_context(|| format!("failed to read input {}", path.display())),
        _ => {
            let mut input = String::new();
            std::io::stdin()
                .read_to_string(&mut input)
                .context("failed to read stdin")?;
            Ok(input)
        }
    }
}
