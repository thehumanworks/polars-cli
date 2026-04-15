use anyhow::Result;
use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::{Value, json};
use tempfile::TempDir;

fn write_fixture(dir: &TempDir, name: &str, contents: &str) -> Result<std::path::PathBuf> {
    let path = dir.path().join(name);
    std::fs::write(&path, contents)?;
    Ok(path)
}

#[test]
fn cli_converts_csv_to_json() -> Result<()> {
    let dir = TempDir::new()?;
    let path = write_fixture(&dir, "records.csv", "name,count\nalpha,1\nbeta,2\n")?;

    let assert = Command::cargo_bin("polars-cli")?
        .arg(&path)
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
fn cli_reads_csv_from_stdin_when_input_is_omitted() -> Result<()> {
    let assert = Command::cargo_bin("polars-cli")?
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
    let assert = Command::cargo_bin("polars-cli")?
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
    Command::cargo_bin("polars-cli")?
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

    let assert = Command::cargo_bin("polars-cli")?
        .arg(&path)
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
fn cli_converts_toml_to_json_lines_with_inferred_format() -> Result<()> {
    let dir = TempDir::new()?;
    let path = write_fixture(
        &dir,
        "records.toml",
        "[[rows]]\nname = \"alpha\"\ncount = 1\n\n[[rows]]\nname = \"beta\"\ncount = 2\n",
    )?;

    let assert = Command::cargo_bin("polars-cli")?
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

    Command::cargo_bin("polars-cli")?
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

    Command::cargo_bin("polars-cli")?
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

    Command::cargo_bin("polars-cli")?
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

    Command::cargo_bin("polars-cli")?
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

    let assert = Command::cargo_bin("polars-cli")?
        .arg(&path)
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
fn cli_accepts_cargo_style_toml_by_flattening_nested_tables() -> Result<()> {
    let dir = TempDir::new()?;
    let path = write_fixture(
        &dir,
        "cargo-like.toml",
        "[package]\nname = \"demo\"\nedition = \"2024\"\n\n[dependencies]\nanyhow = \"1\"\n\n[dependencies.polars]\nversion = \"0.53.0\"\nfeatures = [\"csv\", \"json\"]\n\n[[bin]]\nname = \"demo\"\npath = \"src/main.rs\"\n",
    )?;

    let assert = Command::cargo_bin("polars-cli")?
        .arg(&path)
        .arg("--to")
        .arg("json")
        .assert()
        .success();

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

    Command::cargo_bin("polars-cli")?
        .arg(&path)
        .arg("--to")
        .arg("json")
        .assert()
        .failure()
        .stderr(predicate::str::contains("unsupported input format"));

    Ok(())
}
