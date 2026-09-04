use std::path::{Path, PathBuf};

use anyhow::Result;
use polars::df;
use polars_cli::read::{
    InputFormat, detect_input_format, detect_input_format_from_bytes, read_bytes,
    read_markdown_file, read_table_file, read_text_file, read_toml_file, read_xml_file,
};
use polars_cli::sql::SqlEngine;
use polars_cli::transformer::{
    JsonFormat, html_document_to_markdown, to_csv, to_html, to_json, to_markdown, to_table,
    to_text_lines, to_toml, to_tsv, to_xml, to_yaml,
};
use serde_json::{Value, json};
use tempfile::TempDir;

fn write_fixture(dir: &TempDir, name: &str, contents: &str) -> Result<PathBuf> {
    let path = dir.path().join(name);
    std::fs::write(&path, contents)?;
    Ok(path)
}

fn as_json(df: &mut polars::frame::DataFrame) -> Result<Value> {
    let json = to_json(df, Some(JsonFormat::Json))?;
    Ok(serde_json::from_str(&json)?)
}

#[test]
fn detects_input_format_from_file_extension() -> Result<()> {
    assert_eq!(
        detect_input_format(Path::new("data.csv"))?,
        InputFormat::Csv
    );
    assert_eq!(
        detect_input_format(Path::new("notes.txt"))?,
        InputFormat::Text
    );
    assert_eq!(
        detect_input_format(Path::new("config.toml"))?,
        InputFormat::Toml
    );
    assert_eq!(
        detect_input_format(Path::new("table.md"))?,
        InputFormat::Markdown
    );
    assert_eq!(
        detect_input_format(Path::new("table.markdown"))?,
        InputFormat::Markdown
    );
    assert_eq!(
        detect_input_format(Path::new("page.html"))?,
        InputFormat::Html
    );
    assert_eq!(
        detect_input_format(Path::new("table.xml"))?,
        InputFormat::Xml
    );

    Ok(())
}

#[test]
fn read_text_file_creates_a_single_line_column() -> Result<()> {
    let dir = TempDir::new()?;
    let path = write_fixture(&dir, "notes.txt", "alpha\nbeta\n")?;

    let mut df = read_text_file(&path)?;

    assert_eq!(
        as_json(&mut df)?,
        json!([
            {"line": "alpha"},
            {"line": "beta"}
        ])
    );

    Ok(())
}

#[test]
fn read_text_bytes_keep_leading_blank_lines_and_trailing_spaces() -> Result<()> {
    let mut df = read_bytes(b"\n  Ada  \n", InputFormat::Text, "fixture")?;

    assert_eq!(
        as_json(&mut df)?,
        json!([
            {"line": ""},
            {"line": "  Ada  "}
        ])
    );

    Ok(())
}

#[test]
fn read_markdown_file_parses_the_first_table() -> Result<()> {
    let dir = TempDir::new()?;
    let path = write_fixture(
        &dir,
        "table.md",
        "# Example\n\n| name | count | enabled |\n| --- | ---: | :---: |\n| alpha | 1 | true |\n| beta | 2 | false |\n",
    )?;

    let mut df = read_markdown_file(&path)?;

    assert_eq!(
        as_json(&mut df)?,
        json!([
            {"name": "alpha", "count": 1, "enabled": true},
            {"name": "beta", "count": 2, "enabled": false}
        ])
    );

    Ok(())
}

#[test]
fn read_table_file_parses_the_first_html_table() -> Result<()> {
    let dir = TempDir::new()?;
    let path = write_fixture(
        &dir,
        "table.html",
        "<html><body><table><thead><tr><th>name</th><th>count</th></tr></thead><tbody><tr><td>alpha</td><td>1</td></tr><tr><td>beta</td><td>2</td></tr></tbody></table></body></html>",
    )?;

    let mut df = read_table_file(&path)?;

    assert_eq!(
        as_json(&mut df)?,
        json!([
            {"name": "alpha", "count": 1},
            {"name": "beta", "count": 2}
        ])
    );

    Ok(())
}

#[test]
fn read_xml_file_parses_named_fields_rows() -> Result<()> {
    let dir = TempDir::new()?;
    let path = write_fixture(
        &dir,
        "table.xml",
        "<rows><row><field name=\"name\">alpha</field><field name=\"count\">1</field></row><row><field name=\"name\">beta</field><field name=\"count\">2</field></row></rows>",
    )?;

    let mut df = read_xml_file(&path)?;

    assert_eq!(
        as_json(&mut df)?,
        json!([
            {"name": "alpha", "count": 1},
            {"name": "beta", "count": 2}
        ])
    );

    Ok(())
}

#[test]
fn csv_output_can_omit_the_header_row() -> Result<()> {
    let mut df = df!(
        "id" => &[1, 2],
        "name" => &["alpha", "beta"],
    )?;

    let rendered = to_csv(&mut df, Some(false))?;

    assert_eq!(
        rendered.lines().collect::<Vec<_>>(),
        vec!["1,alpha", "2,beta"]
    );

    Ok(())
}

#[test]
fn markdown_output_renders_a_pipe_table() -> Result<()> {
    let mut df = df!(
        "id" => &[1, 2],
        "name" => &["alpha", "beta"],
    )?;

    let rendered = to_markdown(&mut df)?;

    assert_eq!(
        rendered,
        "| id | name |\n| --- | --- |\n| 1 | alpha |\n| 2 | beta |"
    );

    Ok(())
}

#[test]
fn html_document_to_markdown_skips_style_tag_content() -> Result<()> {
    let rendered =
        html_document_to_markdown("<style>body { color: red; }</style><h1>Hello</h1><p>World</p>")?;

    assert_eq!(rendered, "# Hello\n\nWorld");

    Ok(())
}

#[test]
fn html_document_to_markdown_skips_title_tag_content() -> Result<()> {
    let rendered = html_document_to_markdown(
        "<html><head><title>Demo Title</title></head><body><h1>Hello</h1><p>World</p></body></html>",
    )?;

    assert_eq!(rendered, "# Hello\n\nWorld");

    Ok(())
}

#[test]
fn html_document_to_markdown_skips_common_non_content_tags() -> Result<()> {
    let rendered = html_document_to_markdown(
        "<html><head><title>Demo Title</title><meta name=\"description\" content=\"ignored\"><link rel=\"stylesheet\" href=\"app.css\"></head><body><script>console.log('ignored')</script><template><p>Hidden</p></template><h1>Hello</h1><p>World</p></body></html>",
    )?;

    assert_eq!(rendered, "# Hello\n\nWorld");

    Ok(())
}

#[test]
fn text_output_renders_single_column_rows_as_lines() -> Result<()> {
    let mut df = df!("line" => &["alpha", "beta"])?;

    let rendered = to_text_lines(&mut df)?;

    assert_eq!(rendered, "alpha\nbeta\n");

    Ok(())
}

#[test]
fn text_output_rejects_multi_column_dataframes() -> Result<()> {
    let mut df = df!(
        "id" => &[1, 2],
        "name" => &["alpha", "beta"],
    )?;

    let error = to_text_lines(&mut df).unwrap_err().to_string();

    assert!(error.contains("exactly one column"));

    Ok(())
}

#[test]
fn html_output_renders_a_table() -> Result<()> {
    let mut df = df!(
        "id" => &[1, 2],
        "name" => &["alpha", "beta"],
    )?;

    let rendered = to_html(&mut df)?;

    assert_eq!(
        rendered,
        "<table><thead><tr><th>id</th><th>name</th></tr></thead><tbody><tr><td>1</td><td>alpha</td></tr><tr><td>2</td><td>beta</td></tr></tbody></table>"
    );

    Ok(())
}

#[test]
fn xml_output_renders_named_fields_rows() -> Result<()> {
    let mut df = df!(
        "id" => &[1, 2],
        "name" => &["alpha", "beta"],
    )?;

    let rendered = to_xml(&mut df)?;

    assert_eq!(
        rendered,
        "<rows><row><field name=\"id\">1</field><field name=\"name\">alpha</field></row><row><field name=\"id\">2</field><field name=\"name\">beta</field></row></rows>"
    );

    Ok(())
}

#[test]
fn json_output_supports_json_lines() -> Result<()> {
    let mut df = df!(
        "id" => &[1, 2],
        "name" => &["alpha", "beta"],
    )?;

    let rendered = to_json(&mut df, Some(JsonFormat::JsonLines))?;
    let rows = rendered
        .lines()
        .map(serde_json::from_str::<Value>)
        .collect::<std::result::Result<Vec<_>, _>>()?;

    assert_eq!(
        rows,
        vec![
            json!({"id": 1, "name": "alpha"}),
            json!({"id": 2, "name": "beta"}),
        ]
    );

    Ok(())
}

#[test]
fn read_toml_file_supports_flat_scalar_tables() -> Result<()> {
    let dir = TempDir::new()?;
    let path = write_fixture(
        &dir,
        "config.toml",
        "name = \"alpha\"\ncount = 3\nenabled = true\n",
    )?;

    let mut df = read_toml_file(&path)?;

    assert_eq!(
        as_json(&mut df)?,
        json!([
            {"name": "alpha", "count": 3, "enabled": true}
        ])
    );

    Ok(())
}

#[test]
fn read_toml_file_supports_a_single_array_of_tables() -> Result<()> {
    let dir = TempDir::new()?;
    let path = write_fixture(
        &dir,
        "records.toml",
        "[[rows]]\nname = \"alpha\"\ncount = 1\n\n[[rows]]\nname = \"beta\"\ncount = 2\n",
    )?;

    let mut df = read_toml_file(&path)?;

    assert_eq!(
        as_json(&mut df)?,
        json!([
            {"name": "alpha", "count": 1},
            {"name": "beta", "count": 2}
        ])
    );

    Ok(())
}

#[test]
fn sql_engine_runs_select_over_a_registered_csv_file() -> Result<()> {
    let dir = TempDir::new()?;
    let path = write_fixture(&dir, "people.csv", "name,age\nalice,30\nbob,25\n")?;

    let mut engine = SqlEngine::new();
    engine.register_path(&path, None)?;

    let mut df = engine.execute_collect("SELECT name FROM people WHERE age >= 30 ORDER BY name")?;

    assert_eq!(as_json(&mut df)?, json!([{"name": "alice"}]));
    Ok(())
}

#[test]
fn sql_engine_uses_explicit_alias_over_file_stem() -> Result<()> {
    let dir = TempDir::new()?;
    let path = write_fixture(&dir, "raw.csv", "city,pop\namsterdam,900\nutrecht,360\n")?;

    let mut engine = SqlEngine::new();
    engine.register_path(&path, Some("cities"))?;

    let mut df = engine.execute_collect("SELECT city FROM cities WHERE pop > 500 ORDER BY city")?;

    assert_eq!(as_json(&mut df)?, json!([{"city": "amsterdam"}]));
    Ok(())
}

#[test]
fn sql_engine_joins_csv_and_json_tables() -> Result<()> {
    let dir = TempDir::new()?;
    let people = write_fixture(&dir, "people.csv", "id,name\n1,alice\n2,bob\n")?;
    let scores = write_fixture(
        &dir,
        "scores.json",
        "[{\"id\": 1, \"score\": 90}, {\"id\": 2, \"score\": 70}]",
    )?;

    let mut engine = SqlEngine::new();
    engine.register_path(&people, None)?;
    engine.register_path(&scores, None)?;

    let mut df = engine.execute_collect(
        "SELECT people.name, scores.score FROM people \
         JOIN scores ON people.id = scores.id ORDER BY people.name",
    )?;

    assert_eq!(
        as_json(&mut df)?,
        json!([
            {"name": "alice", "score": 90},
            {"name": "bob", "score": 70}
        ])
    );
    Ok(())
}

#[test]
fn sql_engine_registers_every_supported_file_in_a_directory() -> Result<()> {
    let dir = TempDir::new()?;
    write_fixture(&dir, "left.csv", "id,name\n1,alice\n2,bob\n")?;
    write_fixture(
        &dir,
        "right.json",
        "[{\"id\": 1, \"score\": 90}, {\"id\": 2, \"score\": 70}]",
    )?;

    let mut engine = SqlEngine::new();
    engine.register_directory(dir.path())?;

    let mut df = engine.execute_collect(
        "SELECT left.name, right.score FROM left \
         JOIN right ON left.id = right.id ORDER BY left.name",
    )?;

    assert_eq!(
        as_json(&mut df)?,
        json!([
            {"name": "alice", "score": 90},
            {"name": "bob", "score": 70}
        ])
    );
    Ok(())
}

#[test]
fn sql_engine_supports_polars_read_csv_table_function() -> Result<()> {
    let dir = TempDir::new()?;
    let path = write_fixture(&dir, "records.csv", "id,name\n1,alice\n2,bob\n")?;

    let mut engine = SqlEngine::new();
    let query = format!(
        "SELECT name FROM read_csv('{}') WHERE id = 2",
        path.display()
    );
    let mut df = engine.execute_collect(&query)?;

    assert_eq!(as_json(&mut df)?, json!([{"name": "bob"}]));
    Ok(())
}

#[test]
fn sql_engine_supports_group_by_aggregates() -> Result<()> {
    let dir = TempDir::new()?;
    let path = write_fixture(
        &dir,
        "orders.csv",
        "product,amount\napple,5\napple,3\npear,2\n",
    )?;

    let mut engine = SqlEngine::new();
    engine.register_path(&path, None)?;

    let mut df = engine.execute_collect(
        "SELECT product, SUM(amount) AS total FROM orders GROUP BY product ORDER BY product",
    )?;

    assert_eq!(
        as_json(&mut df)?,
        json!([
            {"product": "apple", "total": 8},
            {"product": "pear", "total": 2}
        ])
    );
    Ok(())
}

#[test]
fn read_toml_file_flattens_nested_tables_into_a_single_row() -> Result<()> {
    let dir = TempDir::new()?;
    let path = write_fixture(
        &dir,
        "cargo-like.toml",
        "[package]\nname = \"demo\"\nedition = \"2024\"\n\n[dependencies]\nanyhow = \"1\"\n\n[dependencies.polars]\nversion = \"0.53.0\"\nfeatures = [\"csv\", \"json\"]\n\n[[bin]]\nname = \"demo\"\npath = \"src/main.rs\"\n",
    )?;

    let mut df = read_toml_file(&path)?;

    assert_eq!(
        as_json(&mut df)?,
        json!([
            {
                "package.name": "demo",
                "package.edition": "2024",
                "dependencies.anyhow": "1",
                "dependencies.polars.version": "0.53.0",
                "dependencies.polars.features": ["csv", "json"],
                "bin.0.name": "demo",
                "bin.0.path": "src/main.rs"
            }
        ])
    );

    Ok(())
}

#[test]
fn detects_new_input_formats_from_extensions() -> Result<()> {
    assert_eq!(
        detect_input_format(Path::new("data.tsv"))?,
        InputFormat::Tsv
    );
    assert_eq!(
        detect_input_format(Path::new("events.ndjson"))?,
        InputFormat::Jsonl
    );
    assert_eq!(
        detect_input_format(Path::new("config.yml"))?,
        InputFormat::Yaml
    );
    Ok(())
}

#[test]
fn sniffs_pipeline_formats_from_content() -> Result<()> {
    assert_eq!(
        detect_input_format_from_bytes(b"name,age\nAda,36\n")?,
        InputFormat::Csv
    );
    assert_eq!(
        detect_input_format_from_bytes(b"name\tage\nAda\t36\n")?,
        InputFormat::Tsv
    );
    assert_eq!(
        detect_input_format_from_bytes(b"{\"id\":1}\n{\"id\":2}\n")?,
        InputFormat::Jsonl
    );
    assert_eq!(
        detect_input_format_from_bytes(b"- name: Ada\n  age: 36\n")?,
        InputFormat::Yaml
    );
    Ok(())
}

#[test]
fn reads_json_objects_scalars_json_lines_yaml_and_tsv() -> Result<()> {
    let mut object = read_bytes(b"{\"name\":\"Ada\",\"age\":36}", InputFormat::Json, "test")?;
    assert_eq!(as_json(&mut object)?, json!([{"name": "Ada", "age": 36}]));

    let mut scalar = read_bytes(b"42", InputFormat::Json, "test")?;
    assert_eq!(as_json(&mut scalar)?, json!([{"value": 42}]));

    let mut jsonl = read_bytes(b"{\"id\":1}\n{\"id\":2}\n", InputFormat::Jsonl, "test")?;
    assert_eq!(as_json(&mut jsonl)?, json!([{"id": 1}, {"id": 2}]));

    let mut yaml = read_bytes(
        b"- name: Ada\n  age: 36\n- name: Lin\n  age: 28\n",
        InputFormat::Yaml,
        "test",
    )?;
    assert_eq!(
        as_json(&mut yaml)?,
        json!([
            {"name": "Ada", "age": 36},
            {"name": "Lin", "age": 28}
        ])
    );

    let mut tsv = read_bytes(b"name\tage\nAda\t36\n", InputFormat::Tsv, "test")?;
    assert_eq!(as_json(&mut tsv)?, json!([{"name": "Ada", "age": 36}]));
    Ok(())
}

#[test]
fn renders_tsv_yaml_toml_and_plain_tables() -> Result<()> {
    let mut frame = df!(
        "name" => &["Ada", "Lin"],
        "age" => &[36, 28],
    )?;

    assert_eq!(
        to_tsv(&mut frame, Some(true))?,
        "name\tage\nAda\t36\nLin\t28\n"
    );
    assert_eq!(
        to_yaml(&mut frame)?,
        "- name: Ada\n  age: 36\n- name: Lin\n  age: 28\n"
    );
    assert_eq!(
        to_toml(&mut frame)?,
        "[[rows]]\nname = \"Ada\"\nage = 36\n\n[[rows]]\nname = \"Lin\"\nage = 28\n"
    );
    assert_eq!(
        to_table(&mut frame)?,
        "name  age\n----  ---\nAda   36\nLin   28"
    );
    Ok(())
}

#[test]
fn sniffs_quoted_and_multiline_csv_content() -> Result<()> {
    let quoted = b"name,note\nAda,\"hello, world\"\n";
    assert_eq!(detect_input_format_from_bytes(quoted)?, InputFormat::Csv);

    let multiline = b"name,note\nAda,\"hello\nworld\"\n";
    assert_eq!(detect_input_format_from_bytes(multiline)?, InputFormat::Csv);
    Ok(())
}
