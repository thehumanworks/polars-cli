use std::{borrow::Cow, fmt::Display};

use anyhow::{Context, bail};
use htmd::HtmlToMarkdown;
use polars::prelude::{AnyValue, SerWriter};
use polars::{
    frame::DataFrame,
    prelude::{CsvWriter, JsonFormat as PlJsonFormat, JsonWriter, QuoteStyle},
};

const HTML_MARKDOWN_SKIP_TAGS: &[&str] = &[
    "head", "title", "script", "style", "meta", "link", "template",
];

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
    to_delimited(df, b',', include_header.unwrap_or(true))
}

pub fn to_tsv(df: &mut DataFrame, include_header: Option<bool>) -> anyhow::Result<String> {
    to_delimited(df, b'\t', include_header.unwrap_or(true))
}

fn to_delimited(df: &mut DataFrame, separator: u8, include_header: bool) -> anyhow::Result<String> {
    if df.width() == 0 {
        return Ok(String::new());
    }
    let mut writer = Vec::new();
    CsvWriter::new(&mut writer)
        .include_header(include_header)
        .with_separator(separator)
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

pub fn to_pretty_json(df: &mut DataFrame) -> anyhow::Result<String> {
    let value = dataframe_to_json_value(df)?;
    serde_json::to_string_pretty(&value).map_err(Into::into)
}

pub fn to_yaml(df: &mut DataFrame) -> anyhow::Result<String> {
    let value = dataframe_to_json_value(df)?;
    yaml_serde::to_string(&value).map_err(Into::into)
}

pub fn to_toml(df: &mut DataFrame) -> anyhow::Result<String> {
    let value = dataframe_to_json_value(df)?;
    let value = match value {
        serde_json::Value::Array(mut rows) if rows.len() == 1 => rows.remove(0),
        serde_json::Value::Array(rows) => serde_json::json!({ "rows": rows }),
        value => value,
    };
    toml::to_string_pretty(&value).context("dataframe cannot be represented as TOML")
}

pub fn to_table(df: &mut DataFrame) -> anyhow::Result<String> {
    if df.width() == 0 {
        return Ok(String::new());
    }
    let (headers, rows) = dataframe_to_string_rows(df)?;
    let rows = rows
        .into_iter()
        .map(|row| {
            row.into_iter()
                .map(|cell| cell.replace('\n', "\\n"))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let widths = (0..headers.len())
        .map(|column| {
            rows.iter()
                .filter_map(|row| row.get(column))
                .map(|cell| cell.chars().count())
                .chain(std::iter::once(headers[column].chars().count()))
                .max()
                .unwrap_or(0)
        })
        .collect::<Vec<_>>();

    let render_row = |row: &[String]| {
        row.iter()
            .enumerate()
            .map(|(index, cell)| format!("{cell:<width$}", width = widths[index]))
            .collect::<Vec<_>>()
            .join("  ")
            .trim_end()
            .to_owned()
    };

    let mut output = Vec::with_capacity(rows.len() + 2);
    output.push(render_row(&headers));
    output.push(
        widths
            .iter()
            .map(|width| "-".repeat(*width))
            .collect::<Vec<_>>()
            .join("  "),
    );
    output.extend(rows.iter().map(|row| render_row(row)));
    Ok(output.join("\n"))
}

pub fn dataframe_to_json_value(df: &mut DataFrame) -> anyhow::Result<serde_json::Value> {
    let json = to_json(df, Some(JsonFormat::Json))?;
    serde_json::from_str(&json).context("failed to convert dataframe to a JSON value")
}

pub fn to_markdown(df: &mut DataFrame) -> anyhow::Result<String> {
    if df.width() == 0 {
        return Ok(String::new());
    }
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
    if df.width() == 0 && df.height() == 0 {
        return Ok(String::new());
    }
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

    if lines.is_empty() {
        Ok(String::new())
    } else {
        let mut output = lines.join("\n");
        output.push('\n');
        Ok(output)
    }
}

pub fn to_html(df: &mut DataFrame) -> anyhow::Result<String> {
    if df.width() == 0 {
        return Ok("<table></table>".to_owned());
    }
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
    if df.width() == 0 {
        return Ok("<rows></rows>".to_owned());
    }
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

pub fn html_document_to_markdown(input: &str) -> anyhow::Result<String> {
    HtmlToMarkdown::builder()
        .skip_tags(HTML_MARKDOWN_SKIP_TAGS.to_vec())
        .build()
        .convert(input)
        .map_err(Into::into)
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
