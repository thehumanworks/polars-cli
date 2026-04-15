use std::{
    collections::BTreeMap,
    io::{Cursor, Read},
    path::Path,
};

use anyhow::{Context, bail};
use clap::ValueEnum;
use polars::{
    frame::DataFrame,
    io::SerReader,
    prelude::{Column, CsvReadOptions, JsonReader},
};
use pulldown_cmark::{Options as MarkdownOptions, Parser as MarkdownParser, html};
use roxmltree::{Document, Node};
use scraper::{Html, Selector};
use serde_json::{Map as JsonMap, Value as JsonValue};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum InputFormat {
    Csv,
    Json,
    Text,
    Toml,
    Table,
    Markdown,
    Html,
    Xml,
}

pub fn read_file(path: impl AsRef<Path>, format: Option<InputFormat>) -> anyhow::Result<DataFrame> {
    let path = path.as_ref();
    let format = match format {
        Some(format) => format,
        None => detect_input_format(path)?,
    };

    match format {
        InputFormat::Csv => read_csv_file(path),
        InputFormat::Json => read_json_file(path),
        InputFormat::Text => read_text_file(path),
        InputFormat::Toml => read_toml_file(path),
        InputFormat::Table => read_table_file(path),
        InputFormat::Markdown => read_markdown_file(path),
        InputFormat::Html => bail!(
            "HTML document input does not map directly to tabular data. Use --to markdown for document conversion, or use --from table to parse the first HTML <table>"
        ),
        InputFormat::Xml => read_xml_file(path),
    }
}

pub fn read_stdin(format: InputFormat) -> anyhow::Result<DataFrame> {
    let mut input = Vec::new();
    std::io::stdin()
        .read_to_end(&mut input)
        .context("failed to read stdin")?;
    read_bytes(&input, format, "stdin")
}

pub fn detect_input_format(path: &Path) -> anyhow::Result<InputFormat> {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref()
    {
        Some("csv") => Ok(InputFormat::Csv),
        Some("json" | "jsonl" | "ndjson") => Ok(InputFormat::Json),
        Some("txt" | "text") => Ok(InputFormat::Text),
        Some("toml") => Ok(InputFormat::Toml),
        Some("md" | "markdown") => Ok(InputFormat::Markdown),
        Some("html" | "htm") => Ok(InputFormat::Html),
        Some("xml") => Ok(InputFormat::Xml),
        _ => bail!(
            "unsupported input format for {}. Use --from to specify csv, json, text, toml, table, markdown, html, or xml",
            path.display()
        ),
    }
}

pub fn read_csv_file(path: impl AsRef<Path>) -> anyhow::Result<DataFrame> {
    let path = path.as_ref();
    let input = std::fs::read(path)
        .with_context(|| format!("failed to read CSV input {}", path.display()))?;
    read_csv_bytes(&input)
}

pub fn read_json_file(path: impl AsRef<Path>) -> anyhow::Result<DataFrame> {
    let path = path.as_ref();
    let input = std::fs::read(path)
        .with_context(|| format!("failed to read JSON input {}", path.display()))?;
    read_json_bytes(&input)
}

pub fn read_text_file(path: impl AsRef<Path>) -> anyhow::Result<DataFrame> {
    let path = path.as_ref();
    let input = std::fs::read(path)
        .with_context(|| format!("failed to read text input {}", path.display()))?;
    read_text_bytes(&input, &path.display().to_string())
}

pub fn read_toml_file(path: impl AsRef<Path>) -> anyhow::Result<DataFrame> {
    let path = path.as_ref();
    let input = std::fs::read(path)
        .with_context(|| format!("failed to read TOML input {}", path.display()))?;
    read_toml_bytes(&input, &path.display().to_string())
}

pub fn read_markdown_file(path: impl AsRef<Path>) -> anyhow::Result<DataFrame> {
    let path = path.as_ref();
    let input = std::fs::read(path)
        .with_context(|| format!("failed to read Markdown input {}", path.display()))?;
    read_markdown_bytes(&input, &path.display().to_string())
}

pub fn read_table_file(path: impl AsRef<Path>) -> anyhow::Result<DataFrame> {
    let path = path.as_ref();
    let input = std::fs::read(path)
        .with_context(|| format!("failed to read table input {}", path.display()))?;
    read_table_bytes(&input, &path.display().to_string())
}

pub fn read_xml_file(path: impl AsRef<Path>) -> anyhow::Result<DataFrame> {
    let path = path.as_ref();
    let input = std::fs::read(path)
        .with_context(|| format!("failed to read XML input {}", path.display()))?;
    read_xml_bytes(&input, &path.display().to_string())
}

fn toml_value_to_json_rows(value: toml::Value) -> anyhow::Result<Vec<JsonValue>> {
    let value = serde_json::to_value(value)?;

    let JsonValue::Object(map) = value else {
        bail!("TOML input must be a top-level table");
    };

    if let Some(rows) = row_array_candidate(&map) {
        return Ok(rows);
    }

    if map.values().all(is_scalar_cell) {
        return Ok(vec![JsonValue::Object(map)]);
    }

    Ok(vec![JsonValue::Object(flatten_json_object(map))])
}

fn read_bytes(input: &[u8], format: InputFormat, label: &str) -> anyhow::Result<DataFrame> {
    match format {
        InputFormat::Csv => read_csv_bytes(input),
        InputFormat::Json => read_json_bytes(input),
        InputFormat::Text => read_text_bytes(input, label),
        InputFormat::Toml => read_toml_bytes(input, label),
        InputFormat::Table => read_table_bytes(input, label),
        InputFormat::Markdown => read_markdown_bytes(input, label),
        InputFormat::Html => bail!(
            "HTML document input does not map directly to tabular data. Use --to markdown for document conversion, or use --from table to parse the first HTML <table>"
        ),
        InputFormat::Xml => read_xml_bytes(input, label),
    }
}

fn read_csv_bytes(input: &[u8]) -> anyhow::Result<DataFrame> {
    polars::prelude::CsvReader::new(Cursor::new(input))
        .with_options(CsvReadOptions::default().with_has_header(true))
        .finish()
        .map_err(Into::into)
}

fn read_json_bytes(input: &[u8]) -> anyhow::Result<DataFrame> {
    let mut cursor = Cursor::new(input);
    JsonReader::new(&mut cursor).finish().map_err(Into::into)
}

fn read_text_bytes(input: &[u8], label: &str) -> anyhow::Result<DataFrame> {
    let text = input_as_utf8(input, label)?;
    let lines: Vec<String> = text.lines().map(ToOwned::to_owned).collect();
    let col = Column::new("line".into(), lines);

    DataFrame::new_infer_height(vec![col]).map_err(Into::into)
}

fn read_toml_bytes(input: &[u8], label: &str) -> anyhow::Result<DataFrame> {
    let raw = input_as_utf8(input, label)?;
    let value: toml::Value =
        toml::from_str(raw).with_context(|| format!("failed to parse TOML input {label}"))?;
    let rows = toml_value_to_json_rows(value)?;
    json_rows_to_dataframe(rows)
}

fn read_markdown_bytes(input: &[u8], label: &str) -> anyhow::Result<DataFrame> {
    let markdown = input_as_utf8(input, label)?;
    let mut options = MarkdownOptions::empty();
    options.insert(MarkdownOptions::ENABLE_TABLES);
    let parser = MarkdownParser::new_ext(markdown, options);
    let mut html_output = String::new();
    html::push_html(&mut html_output, parser);
    let (columns, rows) = html_table_to_json_rows(&html_output)?;

    json_rows_to_dataframe_with_order(rows, &columns)
}

fn read_table_bytes(input: &[u8], label: &str) -> anyhow::Result<DataFrame> {
    let html = input_as_utf8(input, label)?;
    let (columns, rows) = html_table_to_json_rows(html)?;

    json_rows_to_dataframe_with_order(rows, &columns)
}

fn read_xml_bytes(input: &[u8], label: &str) -> anyhow::Result<DataFrame> {
    let xml = input_as_utf8(input, label)?;
    let rows = xml_to_json_rows(xml)?;

    json_rows_to_dataframe(rows)
}

fn json_rows_to_dataframe(rows: Vec<JsonValue>) -> anyhow::Result<DataFrame> {
    if rows.is_empty() {
        bail!("input did not contain any rows");
    }

    let json = serde_json::to_vec(&rows)?;
    let mut cursor = Cursor::new(json);

    JsonReader::new(&mut cursor).finish().map_err(Into::into)
}

fn json_rows_to_dataframe_with_order(
    rows: Vec<JsonValue>,
    column_order: &[String],
) -> anyhow::Result<DataFrame> {
    let df = json_rows_to_dataframe(rows)?;
    if column_order.is_empty() {
        return Ok(df);
    }

    df.select(column_order.iter().map(String::as_str))
        .map_err(Into::into)
}

fn input_as_utf8<'a>(input: &'a [u8], label: &str) -> anyhow::Result<&'a str> {
    std::str::from_utf8(input).with_context(|| format!("input {label} is not valid UTF-8"))
}

fn html_table_to_json_rows(input: &str) -> anyhow::Result<(Vec<String>, Vec<JsonValue>)> {
    let document = Html::parse_document(input);
    let table_selector = parse_selector("table")?;
    let row_selector = parse_selector("tr")?;
    let cell_selector = parse_selector("th, td")?;
    let header_cell_selector = parse_selector("th")?;
    let table = document
        .select(&table_selector)
        .next()
        .context("HTML/Markdown input must contain a table")?;

    let mut header: Option<Vec<String>> = None;
    let mut rows = Vec::new();

    for row in table.select(&row_selector) {
        let cells: Vec<String> = row
            .select(&cell_selector)
            .map(|cell| normalize_cell_text(cell.text().collect::<Vec<_>>().join(" ")))
            .collect();

        if cells.is_empty() {
            continue;
        }

        let has_header_cells = row.select(&header_cell_selector).next().is_some();
        if header.is_none() && has_header_cells {
            header = Some(cells);
            continue;
        }

        rows.push(cells);
    }

    table_rows_to_json_rows(header, rows)
}

fn xml_to_json_rows(input: &str) -> anyhow::Result<Vec<JsonValue>> {
    let document = Document::parse(input).context("failed to parse XML input")?;
    let root = document.root_element();
    let children: Vec<_> = root.children().filter(|node| node.is_element()).collect();

    if children.is_empty() {
        bail!("XML input must contain element children under the root");
    }

    if child_elements_share_tag_name(&children) {
        let rows = children
            .into_iter()
            .map(xml_row_to_json_object)
            .collect::<anyhow::Result<Vec<_>>>()?;
        return Ok(rows.into_iter().map(JsonValue::Object).collect());
    }

    let mut flattened = JsonMap::new();
    for child in children {
        flatten_xml_node(&mut flattened, child.tag_name().name().to_owned(), child);
    }

    Ok(vec![JsonValue::Object(flattened)])
}

fn table_rows_to_json_rows(
    header: Option<Vec<String>>,
    rows: Vec<Vec<String>>,
) -> anyhow::Result<(Vec<String>, Vec<JsonValue>)> {
    if rows.is_empty() {
        bail!("table input must contain at least one data row");
    }

    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    let mut columns = header.unwrap_or_default();
    if columns.len() < width {
        columns.extend((columns.len()..width).map(|idx| format!("column_{}", idx + 1)));
    }

    let json_rows = rows
        .into_iter()
        .map(|row| {
            let mut object = JsonMap::new();
            for (index, column_name) in columns.iter().enumerate() {
                let value = row
                    .get(index)
                    .map(|cell| infer_json_scalar(cell))
                    .unwrap_or(JsonValue::Null);
                object.insert(column_name.clone(), value);
            }
            JsonValue::Object(object)
        })
        .collect();

    Ok((columns, json_rows))
}

fn xml_row_to_json_object(node: Node<'_, '_>) -> anyhow::Result<JsonMap<String, JsonValue>> {
    let children: Vec<_> = node.children().filter(|child| child.is_element()).collect();
    if children.is_empty() {
        bail!("XML row elements must contain child fields");
    }

    let mut object = JsonMap::new();
    let counts = child_tag_counts(&children);
    let mut seen = BTreeMap::new();

    for child in children {
        if child.has_tag_name("field") {
            let name = child
                .attribute("name")
                .context("XML field elements must include a name attribute")?;
            flatten_xml_field(&mut object, name.to_owned(), child);
            continue;
        }

        let child_name = child.tag_name().name().to_owned();
        let key = indexed_xml_key(&child_name, &counts, &mut seen);
        flatten_xml_field(&mut object, key, child);
    }

    Ok(object)
}

fn flatten_xml_field(out: &mut JsonMap<String, JsonValue>, path: String, node: Node<'_, '_>) {
    let children: Vec<_> = node.children().filter(|child| child.is_element()).collect();
    if children.is_empty() {
        out.insert(path, infer_json_scalar(node.text().unwrap_or("").trim()));
        return;
    }

    let counts = child_tag_counts(&children);
    let mut seen = BTreeMap::new();

    for child in children {
        if child.has_tag_name("field") {
            let child_name = child.attribute("name").unwrap_or("field");
            flatten_xml_field(out, join_path(&path, child_name), child);
            continue;
        }

        let child_name = child.tag_name().name().to_owned();
        let indexed_name = indexed_xml_key(&child_name, &counts, &mut seen);
        flatten_xml_field(out, join_path(&path, &indexed_name), child);
    }
}

fn flatten_xml_node(out: &mut JsonMap<String, JsonValue>, path: String, node: Node<'_, '_>) {
    flatten_xml_field(out, path, node);
}

fn child_elements_share_tag_name(children: &[Node<'_, '_>]) -> bool {
    children
        .first()
        .map(|first| {
            let name = first.tag_name().name();
            children.iter().all(|child| child.tag_name().name() == name)
        })
        .unwrap_or(false)
}

fn child_tag_counts(children: &[Node<'_, '_>]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for child in children {
        let key = if child.has_tag_name("field") {
            child.attribute("name").unwrap_or("field").to_owned()
        } else {
            child.tag_name().name().to_owned()
        };
        *counts.entry(key).or_insert(0) += 1;
    }
    counts
}

fn indexed_xml_key(
    name: &str,
    counts: &BTreeMap<String, usize>,
    seen: &mut BTreeMap<String, usize>,
) -> String {
    if counts.get(name).copied().unwrap_or(0) <= 1 {
        return name.to_owned();
    }

    let index = seen.entry(name.to_owned()).or_insert(0);
    let key = format!("{name}.{index}");
    *index += 1;
    key
}

fn parse_selector(selector: &str) -> anyhow::Result<Selector> {
    Selector::parse(selector)
        .map_err(|error| anyhow::anyhow!("invalid HTML selector {selector:?}: {error}"))
}

fn normalize_cell_text(text: String) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn infer_json_scalar(text: &str) -> JsonValue {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return JsonValue::Null;
    }

    if trimmed.eq_ignore_ascii_case("true") {
        return JsonValue::Bool(true);
    }

    if trimmed.eq_ignore_ascii_case("false") {
        return JsonValue::Bool(false);
    }

    if !has_leading_zero_integer(trimmed) {
        if let Ok(value) = trimmed.parse::<i64>() {
            return JsonValue::Number(value.into());
        }

        if let Ok(value) = trimmed.parse::<u64>() {
            return JsonValue::Number(value.into());
        }
    }

    if (trimmed.contains('.') || trimmed.contains('e') || trimmed.contains('E'))
        && let Ok(value) = trimmed.parse::<f64>()
        && let Some(number) = serde_json::Number::from_f64(value)
    {
        return JsonValue::Number(number);
    }

    JsonValue::String(trimmed.to_owned())
}

fn has_leading_zero_integer(text: &str) -> bool {
    let digits = text.strip_prefix('-').unwrap_or(text);
    digits.len() > 1 && digits.starts_with('0') && digits.chars().all(|char| char.is_ascii_digit())
}

fn row_array_candidate(map: &JsonMap<String, JsonValue>) -> Option<Vec<JsonValue>> {
    if map.len() != 1 {
        return None;
    }

    match map.values().next()? {
        JsonValue::Array(rows) if rows.iter().all(JsonValue::is_object) => Some(rows.clone()),
        _ => None,
    }
}

fn flatten_json_object(map: JsonMap<String, JsonValue>) -> JsonMap<String, JsonValue> {
    let mut flattened = JsonMap::new();
    for (key, value) in map {
        flatten_json_value(&mut flattened, key, value);
    }
    flattened
}

fn flatten_json_value(out: &mut JsonMap<String, JsonValue>, path: String, value: JsonValue) {
    match value {
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::String(_) => {
            out.insert(path, value);
        }
        JsonValue::Array(values) if values.iter().all(is_scalar_cell) => {
            out.insert(path, JsonValue::Array(values));
        }
        JsonValue::Array(values) => {
            for (index, value) in values.into_iter().enumerate() {
                flatten_json_value(out, format!("{path}.{index}"), value);
            }
        }
        JsonValue::Object(map) if map.is_empty() => {
            out.insert(path, JsonValue::Null);
        }
        JsonValue::Object(map) => {
            for (key, value) in map {
                flatten_json_value(out, join_path(&path, &key), value);
            }
        }
    }
}

fn join_path(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        key.to_owned()
    } else {
        format!("{prefix}.{key}")
    }
}

fn is_scalar_cell(value: &JsonValue) -> bool {
    match value {
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::String(_) => true,
        JsonValue::Array(values) => values.iter().all(is_scalar_cell),
        JsonValue::Object(_) => false,
    }
}
