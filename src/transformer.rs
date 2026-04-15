use std::{borrow::Cow, fmt::Display};

use anyhow::{Context, bail};
use polars::prelude::{AnyValue, SerWriter};
use polars::{
    frame::DataFrame,
    prelude::{CsvWriter, JsonFormat as PlJsonFormat, JsonWriter, QuoteStyle},
};

#[derive(Debug, Clone, Copy, Default)]
pub enum JsonFormat {
    #[default]
    Json,
    JsonLines,
}

impl Display for JsonFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JsonFormat::Json => write!(f, "json"),
            JsonFormat::JsonLines => write!(f, "json_lines"),
        }
    }
}

impl From<JsonFormat> for PlJsonFormat {
    fn from(value: JsonFormat) -> Self {
        match value {
            JsonFormat::Json => PlJsonFormat::Json,
            JsonFormat::JsonLines => PlJsonFormat::JsonLines,
        }
    }
}

pub fn to_csv(df: &mut DataFrame, include_header: Option<bool>) -> anyhow::Result<String> {
    let mut writer = Vec::new();
    CsvWriter::new(&mut writer)
        .include_header(include_header.unwrap_or(true))
        .with_separator(b',')
        .with_quote_style(QuoteStyle::Necessary)
        .finish(df)
        .map_err(|e| anyhow::anyhow!(e))?;
    Ok(String::from_utf8(writer)?)
}

pub fn to_json(df: &mut DataFrame, format: Option<JsonFormat>) -> anyhow::Result<String> {
    let mut writer = Vec::new();
    JsonWriter::new(&mut writer)
        .with_json_format(format.unwrap_or_default().into())
        .finish(df)
        .map_err(|e| anyhow::anyhow!(e))?;
    Ok(String::from_utf8(writer)?)
}

pub fn to_markdown(df: &mut DataFrame) -> anyhow::Result<String> {
    let (headers, rows) = dataframe_to_string_rows(df)?;
    let mut lines = Vec::with_capacity(rows.len() + 2);
    lines.push(format!(
        "| {} |",
        headers
            .iter()
            .map(|header| escape_markdown_cell(header))
            .collect::<Vec<_>>()
            .join(" | ")
    ));
    lines.push(format!("| {} |", vec!["---"; headers.len()].join(" | ")));
    lines.extend(rows.into_iter().map(|row| {
        format!(
            "| {} |",
            row.iter()
                .map(|cell| escape_markdown_cell(cell))
                .collect::<Vec<_>>()
                .join(" | ")
        )
    }));

    Ok(lines.join("\n"))
}

pub fn to_text_lines(df: &mut DataFrame) -> anyhow::Result<String> {
    if df.width() != 1 {
        bail!("plain text output requires exactly one column");
    }

    let column = df
        .select_at_idx(0)
        .context("plain text output requires exactly one column")?;
    let mut lines = Vec::with_capacity(df.height());
    for idx in 0..df.height() {
        lines.push(render_any_value(column.get(idx)?));
    }

    Ok(lines.join("\n"))
}

pub fn to_html(df: &mut DataFrame) -> anyhow::Result<String> {
    let (headers, rows) = dataframe_to_string_rows(df)?;
    let head = headers
        .iter()
        .map(|header| format!("<th>{}</th>", escape_html(header)))
        .collect::<String>();
    let body = rows
        .into_iter()
        .map(|row| {
            let cells = row
                .iter()
                .map(|cell| format!("<td>{}</td>", escape_html(cell)))
                .collect::<String>();
            format!("<tr>{cells}</tr>")
        })
        .collect::<String>();

    Ok(format!(
        "<table><thead><tr>{head}</tr></thead><tbody>{body}</tbody></table>"
    ))
}

pub fn to_xml(df: &mut DataFrame) -> anyhow::Result<String> {
    let (headers, rows) = dataframe_to_string_rows(df)?;
    let body = rows
        .into_iter()
        .map(|row| {
            let fields = headers
                .iter()
                .zip(row.iter())
                .map(|(header, cell)| {
                    format!(
                        "<field name=\"{}\">{}</field>",
                        escape_html_attribute(header),
                        escape_html(cell)
                    )
                })
                .collect::<String>();
            format!("<row>{fields}</row>")
        })
        .collect::<String>();

    Ok(format!("<rows>{body}</rows>"))
}

fn dataframe_to_string_rows(df: &DataFrame) -> anyhow::Result<(Vec<String>, Vec<Vec<String>>)> {
    if df.width() == 0 {
        bail!("cannot render an empty schema");
    }

    let headers = df
        .get_column_names_owned()
        .into_iter()
        .map(|name| name.to_string())
        .collect::<Vec<_>>();
    let mut rows = Vec::with_capacity(df.height());

    for idx in 0..df.height() {
        let row = df
            .get(idx)
            .context("failed to read a DataFrame row while rendering output")?;
        rows.push(row.into_iter().map(render_any_value).collect());
    }

    Ok((headers, rows))
}

fn render_any_value(value: AnyValue<'_>) -> String {
    match value {
        AnyValue::Null => String::new(),
        other => other.str_value().into_owned(),
    }
}

fn escape_markdown_cell(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace('\n', "<br>")
}

fn escape_html(value: &str) -> Cow<'_, str> {
    if !value.contains(['&', '<', '>', '"', '\'']) {
        return Cow::Borrowed(value);
    }

    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            other => escaped.push(other),
        }
    }

    Cow::Owned(escaped)
}

fn escape_html_attribute(value: &str) -> Cow<'_, str> {
    escape_html(value)
}
