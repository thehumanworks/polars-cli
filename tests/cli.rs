use anyhow::Result;
use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::{Value, json};
use tempfile::TempDir;

fn cmd() -> Result<Command> {
    Ok(Command::cargo_bin("pl")?)
}

fn write_fixture(dir: &TempDir, name: &str, contents: &str) -> Result<std::path::PathBuf> {
    let path = dir.path().join(name);
    std::fs::write(&path, contents)?;
    Ok(path)
}

#[test]
fn cli_converts_csv_to_json() -> Result<()> {
    let dir = TempDir::new()?;
    let path = write_fixture(&dir, "records.csv", "name,count\nalpha,1\nbeta,2\n")?;

    let assert = cmd()?.arg(&path).arg("--to").arg("json").assert().success();

    let stdout = serde_json::from_slice::<Value>(&assert.get_output().stdout)?;

    assert_eq!(
        stdout,
        json!([
            {"name": "alpha", "count": 1},
            {"name": "beta", "count": 2}
        ])
    );

    Ok(())
}

#[test]
fn cli_reads_csv_from_stdin_when_input_is_omitted() -> Result<()> {
    let assert = cmd()?
        .arg("--from")
        .arg("csv")
        .arg("--to")
        .arg("json")
        .write_stdin("name,count\nalpha,1\nbeta,2\n")
        .assert()
        .success();

    let stdout = serde_json::from_slice::<Value>(&assert.get_output().stdout)?;

    assert_eq!(
        stdout,
        json!([
            {"name": "alpha", "count": 1},
            {"name": "beta", "count": 2}
        ])
    );

    Ok(())
}

#[test]
fn cli_reads_markdown_from_stdin_when_input_is_dash() -> Result<()> {
    let assert = cmd()?
        .arg("-")
        .arg("--from")
        .arg("markdown")
        .arg("--to")
        .arg("json")
        .write_stdin("| name | count |\n| --- | ---: |\n| alpha | 1 |\n| beta | 2 |\n")
        .assert()
        .success();

    let stdout = serde_json::from_slice::<Value>(&assert.get_output().stdout)?;

    assert_eq!(
        stdout,
        json!([
            {"name": "alpha", "count": 1},
            {"name": "beta", "count": 2}
        ])
    );

    Ok(())
}

#[test]
fn cli_requires_from_when_reading_stdin() -> Result<()> {
    cmd()?
        .arg("--to")
        .arg("json")
        .write_stdin("name,count\nalpha,1\n")
        .assert()
        .failure()
        .stderr(predicate::str::contains("stdin input requires --from"));

    Ok(())
}

#[test]
fn cli_converts_markdown_to_json() -> Result<()> {
    let dir = TempDir::new()?;
    let path = write_fixture(
        &dir,
        "table.md",
        "| name | count |\n| --- | ---: |\n| alpha | 1 |\n| beta | 2 |\n",
    )?;

    let assert = cmd()?.arg(&path).arg("--to").arg("json").assert().success();

    let stdout = serde_json::from_slice::<Value>(&assert.get_output().stdout)?;

    assert_eq!(
        stdout,
        json!([
            {"name": "alpha", "count": 1},
            {"name": "beta", "count": 2}
        ])
    );

    Ok(())
}

#[test]
fn cli_converts_html_document_to_markdown() -> Result<()> {
    let dir = TempDir::new()?;
    let path = write_fixture(&dir, "page.html", "<h1>Hello</h1><p>World</p>")?;

    cmd()?
        .arg(&path)
        .arg("--to")
        .arg("markdown")
        .assert()
        .success()
        .stdout(predicate::eq("# Hello\n\nWorld"));

    Ok(())
}

#[test]
fn cli_ignores_style_tag_content_when_converting_html_to_markdown() -> Result<()> {
    let dir = TempDir::new()?;
    let path = write_fixture(
        &dir,
        "page.html",
        "<style>body { color: red; }</style><h1>Hello</h1><p>World</p>",
    )?;

    cmd()?
        .arg(&path)
        .arg("--to")
        .arg("markdown")
        .assert()
        .success()
        .stdout(predicate::eq("# Hello\n\nWorld"));

    Ok(())
}

#[test]
fn cli_ignores_title_tag_content_when_converting_html_to_markdown() -> Result<()> {
    let dir = TempDir::new()?;
    let path = write_fixture(
        &dir,
        "page.html",
        "<html><head><title>Demo Title</title></head><body><h1>Hello</h1><p>World</p></body></html>",
    )?;

    cmd()?
        .arg(&path)
        .arg("--to")
        .arg("markdown")
        .assert()
        .success()
        .stdout(predicate::eq("# Hello\n\nWorld"));

    Ok(())
}

#[test]
fn cli_ignores_common_non_content_tags_when_converting_html_to_markdown() -> Result<()> {
    let dir = TempDir::new()?;
    let path = write_fixture(
        &dir,
        "page.html",
        "<html><head><title>Demo Title</title><meta name=\"description\" content=\"ignored\"><link rel=\"stylesheet\" href=\"app.css\"></head><body><script>console.log('ignored')</script><template><p>Hidden</p></template><h1>Hello</h1><p>World</p></body></html>",
    )?;

    cmd()?
        .arg(&path)
        .arg("--to")
        .arg("markdown")
        .assert()
        .success()
        .stdout(predicate::eq("# Hello\n\nWorld"));

    Ok(())
}

#[test]
fn cli_converts_html_table_to_json_when_forced_to_table() -> Result<()> {
    let dir = TempDir::new()?;
    let path = write_fixture(
        &dir,
        "table.html",
        "<table><thead><tr><th>name</th><th>count</th></tr></thead><tbody><tr><td>alpha</td><td>1</td></tr><tr><td>beta</td><td>2</td></tr></tbody></table>",
    )?;

    let assert = cmd()?
        .arg(&path)
        .arg("--from")
        .arg("table")
        .arg("--to")
        .arg("json")
        .assert()
        .success();

    let stdout = serde_json::from_slice::<Value>(&assert.get_output().stdout)?;

    assert_eq!(
        stdout,
        json!([
            {"name": "alpha", "count": 1},
            {"name": "beta", "count": 2}
        ])
    );

    Ok(())
}

#[test]
fn cli_converts_html_table_to_markdown_when_forced_to_table() -> Result<()> {
    let dir = TempDir::new()?;
    let path = write_fixture(
        &dir,
        "table.html",
        "<table><thead><tr><th>name</th><th>count</th></tr></thead><tbody><tr><td>alpha</td><td>1</td></tr><tr><td>beta</td><td>2</td></tr></tbody></table>",
    )?;

    cmd()?
        .arg(&path)
        .arg("--from")
        .arg("table")
        .arg("--to")
        .arg("markdown")
        .assert()
        .success()
        .stdout(predicate::eq(
            "| name | count |\n| --- | --- |\n| alpha | 1 |\n| beta | 2 |",
        ));

    Ok(())
}

#[test]
fn cli_rejects_html_document_to_json() -> Result<()> {
    let dir = TempDir::new()?;
    let path = write_fixture(&dir, "page.html", "<h1>Hello</h1><p>World</p>")?;

    cmd()?
        .arg(&path)
        .arg("--to")
        .arg("json")
        .assert()
        .failure()
        .stderr(predicate::str::contains("Use --from table"));

    Ok(())
}

#[test]
fn cli_converts_toml_to_json_lines_with_inferred_format() -> Result<()> {
    let dir = TempDir::new()?;
    let path = write_fixture(
        &dir,
        "records.toml",
        "[[rows]]\nname = \"alpha\"\ncount = 1\n\n[[rows]]\nname = \"beta\"\ncount = 2\n",
    )?;

    let assert = cmd()?
        .arg(&path)
        .arg("--to")
        .arg("jsonl")
        .assert()
        .success();

    let rows = String::from_utf8(assert.get_output().stdout.clone())?
        .lines()
        .map(serde_json::from_str::<Value>)
        .collect::<std::result::Result<Vec<_>, _>>()?;

    assert_eq!(
        rows,
        vec![
            json!({"name": "alpha", "count": 1}),
            json!({"name": "beta", "count": 2}),
        ]
    );

    Ok(())
}

#[test]
fn cli_converts_csv_to_markdown() -> Result<()> {
    let dir = TempDir::new()?;
    let path = write_fixture(&dir, "records.csv", "name,count\nalpha,1\nbeta,2\n")?;

    cmd()?
        .arg(&path)
        .arg("--to")
        .arg("markdown")
        .assert()
        .success()
        .stdout(predicate::eq(
            "| name | count |\n| --- | --- |\n| alpha | 1 |\n| beta | 2 |",
        ));

    Ok(())
}

#[test]
fn cli_converts_one_column_csv_to_text_lines() -> Result<()> {
    let dir = TempDir::new()?;
    let path = write_fixture(&dir, "lines.csv", "line\nalpha\nbeta\n")?;

    cmd()?
        .arg(&path)
        .arg("--to")
        .arg("text")
        .assert()
        .success()
        .stdout(predicate::eq("alpha\nbeta"));

    Ok(())
}

#[test]
fn cli_rejects_multi_column_csv_to_text_lines() -> Result<()> {
    let dir = TempDir::new()?;
    let path = write_fixture(&dir, "records.csv", "name,count\nalpha,1\nbeta,2\n")?;

    cmd()?
        .arg(&path)
        .arg("--to")
        .arg("text")
        .assert()
        .failure()
        .stderr(predicate::str::contains("exactly one column"));

    Ok(())
}

#[test]
fn cli_converts_csv_to_html() -> Result<()> {
    let dir = TempDir::new()?;
    let path = write_fixture(&dir, "records.csv", "name,count\nalpha,1\nbeta,2\n")?;

    cmd()?
        .arg(&path)
        .arg("--to")
        .arg("html")
        .assert()
        .success()
        .stdout(predicate::eq(
            "<table><thead><tr><th>name</th><th>count</th></tr></thead><tbody><tr><td>alpha</td><td>1</td></tr><tr><td>beta</td><td>2</td></tr></tbody></table>",
        ));

    Ok(())
}

#[test]
fn cli_converts_xml_to_json() -> Result<()> {
    let dir = TempDir::new()?;
    let path = write_fixture(
        &dir,
        "records.xml",
        "<rows><row><field name=\"name\">alpha</field><field name=\"count\">1</field></row><row><field name=\"name\">beta</field><field name=\"count\">2</field></row></rows>",
    )?;

    let assert = cmd()?.arg(&path).arg("--to").arg("json").assert().success();

    let stdout = serde_json::from_slice::<Value>(&assert.get_output().stdout)?;

    assert_eq!(
        stdout,
        json!([
            {"name": "alpha", "count": 1},
            {"name": "beta", "count": 2}
        ])
    );

    Ok(())
}

#[test]
fn cli_accepts_cargo_style_toml_by_flattening_nested_tables() -> Result<()> {
    let dir = TempDir::new()?;
    let path = write_fixture(
        &dir,
        "cargo-like.toml",
        "[package]\nname = \"demo\"\nedition = \"2024\"\n\n[dependencies]\nanyhow = \"1\"\n\n[dependencies.polars]\nversion = \"0.53.0\"\nfeatures = [\"csv\", \"json\"]\n\n[[bin]]\nname = \"demo\"\npath = \"src/main.rs\"\n",
    )?;

    let assert = cmd()?.arg(&path).arg("--to").arg("json").assert().success();

    let stdout = serde_json::from_slice::<Value>(&assert.get_output().stdout)?;

    assert_eq!(
        stdout,
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
fn cli_requires_an_explicit_input_format_for_unknown_extensions() -> Result<()> {
    let dir = TempDir::new()?;
    let path = write_fixture(&dir, "records.data", "name,count\nalpha,1\n")?;

    cmd()?
        .arg(&path)
        .arg("--to")
        .arg("json")
        .assert()
        .failure()
        .stderr(predicate::str::contains("unsupported input format"));

    Ok(())
}

#[test]
fn polars_alias_reports_version() -> Result<()> {
    Command::cargo_bin("polars")?
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("polars").or(predicate::str::contains("pl")));

    Ok(())
}

#[test]
fn cli_runs_sql_query_against_single_csv_positional() -> Result<()> {
    let dir = TempDir::new()?;
    let path = write_fixture(&dir, "people.csv", "name,age\nalice,30\nbob,25\n")?;

    let assert = cmd()?
        .arg(&path)
        .arg("--sql")
        .arg("SELECT name FROM people WHERE age >= 30 ORDER BY name")
        .arg("--to")
        .arg("json")
        .assert()
        .success();

    let stdout = serde_json::from_slice::<Value>(&assert.get_output().stdout)?;
    assert_eq!(stdout, json!([{"name": "alice"}]));
    Ok(())
}

#[test]
fn cli_sql_joins_csv_and_json_positional_inputs() -> Result<()> {
    let dir = TempDir::new()?;
    let people = write_fixture(&dir, "people.csv", "id,name\n1,alice\n2,bob\n")?;
    let scores = write_fixture(
        &dir,
        "scores.json",
        "[{\"id\": 1, \"score\": 90}, {\"id\": 2, \"score\": 70}]",
    )?;

    let assert = cmd()?
        .arg(&people)
        .arg(&scores)
        .arg("--sql")
        .arg(
            "SELECT people.name, scores.score FROM people \
             JOIN scores ON people.id = scores.id ORDER BY people.name",
        )
        .arg("--to")
        .arg("json")
        .assert()
        .success();

    let stdout = serde_json::from_slice::<Value>(&assert.get_output().stdout)?;
    assert_eq!(
        stdout,
        json!([
            {"name": "alice", "score": 90},
            {"name": "bob", "score": 70}
        ])
    );
    Ok(())
}

#[test]
fn cli_sql_accepts_explicit_table_aliases() -> Result<()> {
    let dir = TempDir::new()?;
    let path = write_fixture(&dir, "raw.csv", "city,pop\namsterdam,900\nutrecht,360\n")?;

    let assert = cmd()?
        .arg("--sql")
        .arg("SELECT city FROM cities WHERE pop > 500 ORDER BY city")
        .arg("--to")
        .arg("json")
        .arg("--table")
        .arg(format!("cities={}", path.display()))
        .assert()
        .success();

    let stdout = serde_json::from_slice::<Value>(&assert.get_output().stdout)?;
    assert_eq!(stdout, json!([{"city": "amsterdam"}]));
    Ok(())
}

#[test]
fn cli_sql_registers_every_file_in_a_directory_positional() -> Result<()> {
    let dir = TempDir::new()?;
    write_fixture(&dir, "people.csv", "id,name\n1,alice\n2,bob\n")?;
    write_fixture(
        &dir,
        "scores.json",
        "[{\"id\": 1, \"score\": 90}, {\"id\": 2, \"score\": 70}]",
    )?;

    let assert = cmd()?
        .arg(dir.path())
        .arg("--sql")
        .arg(
            "SELECT people.name, scores.score FROM people \
             JOIN scores ON people.id = scores.id ORDER BY people.name",
        )
        .arg("--to")
        .arg("json")
        .assert()
        .success();

    let stdout = serde_json::from_slice::<Value>(&assert.get_output().stdout)?;
    assert_eq!(
        stdout,
        json!([
            {"name": "alice", "score": 90},
            {"name": "bob", "score": 70}
        ])
    );
    Ok(())
}

#[test]
fn cli_sql_supports_polars_read_csv_table_function_without_inputs() -> Result<()> {
    let dir = TempDir::new()?;
    let path = write_fixture(&dir, "records.csv", "id,name\n1,alice\n2,bob\n")?;

    let assert = cmd()?
        .arg("--sql")
        .arg(format!(
            "SELECT name FROM read_csv('{}') WHERE id = 2",
            path.display()
        ))
        .arg("--to")
        .arg("json")
        .assert()
        .success();

    let stdout = serde_json::from_slice::<Value>(&assert.get_output().stdout)?;
    assert_eq!(stdout, json!([{"name": "bob"}]));
    Ok(())
}

#[test]
fn cli_sql_defaults_to_markdown_output() -> Result<()> {
    let dir = TempDir::new()?;
    let path = write_fixture(&dir, "people.csv", "name,age\nalice,30\n")?;

    cmd()?
        .arg(&path)
        .arg("--sql")
        .arg("SELECT name FROM people")
        .assert()
        .success()
        .stdout(predicate::eq("| name |\n| --- |\n| alice |"));

    Ok(())
}
