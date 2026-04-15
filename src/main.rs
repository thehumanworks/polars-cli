use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::{Parser, ValueEnum};
use polars_cli::read::{InputFormat, read_file, read_stdin};
use polars_cli::transformer::{
    JsonFormat, to_csv, to_html, to_json, to_markdown, to_text_lines, to_xml,
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
    /// Path to the input file.
    input: Option<PathBuf>,

    /// Force the input format instead of inferring it from the file extension.
    #[arg(long, value_enum)]
    from: Option<InputFormat>,

    /// Output format written to stdout.
    #[arg(long, value_enum)]
    to: OutputFormat,

    /// Omit the header row when rendering CSV output.
    #[arg(long)]
    no_header: bool,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let mut df = read_cli_input(&cli)?;
    let output = match cli.to {
        OutputFormat::Csv => to_csv(&mut df, Some(!cli.no_header))?,
        OutputFormat::Json => to_json(&mut df, Some(JsonFormat::Json))?,
        OutputFormat::Jsonl => to_json(&mut df, Some(JsonFormat::JsonLines))?,
        OutputFormat::Markdown => to_markdown(&mut df)?,
        OutputFormat::Text => to_text_lines(&mut df)?,
        OutputFormat::Html => to_html(&mut df)?,
        OutputFormat::Xml => to_xml(&mut df)?,
    };

    print!("{output}");

    Ok(())
}

fn read_cli_input(cli: &Cli) -> anyhow::Result<polars::frame::DataFrame> {
    match cli.input.as_deref() {
        Some(path) if path != Path::new("-") => read_file(path, cli.from),
        _ => {
            let format = cli
                .from
                .context("stdin input requires --from to specify the input format")?;
            read_stdin(format)
        }
    }
}
