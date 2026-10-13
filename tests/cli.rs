use anyhow::Result;
use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::{Value, json};
use std::process::{Command as StdCommand, Stdio};
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
fn cli_infers_csv_when_reading_stdin() -> Result<()> {
    let assert = cmd()?
        .arg("--to")
        .arg("json")
        .write_stdin("name,count\nalpha,1\n")
        .assert()
        .success();

    let stdout = serde_json::from_slice::<Value>(&assert.get_output().stdout)?;
    assert_eq!(stdout, json!([{"name": "alpha", "count": 1}]));
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
        .stdout(predicate::eq("# Hello\n\nWorld\n"));

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
        .stdout(predicate::eq("# Hello\n\nWorld\n"));

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
        .stdout(predicate::eq("# Hello\n\nWorld\n"));

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
        .stdout(predicate::eq("# Hello\n\nWorld\n"));

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
            "| name | count |\n| --- | --- |\n| alpha | 1 |\n| beta | 2 |\n",
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
            "| name | count |\n| --- | --- |\n| alpha | 1 |\n| beta | 2 |\n",
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
        .stdout(predicate::eq("alpha\nbeta\n"));

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
            "<table><thead><tr><th>name</th><th>count</th></tr></thead><tbody><tr><td>alpha</td><td>1</td></tr><tr><td>beta</td><td>2</td></tr></tbody></table>\n",
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
fn cli_sniffs_input_format_for_unknown_extensions() -> Result<()> {
    let dir = TempDir::new()?;
    let path = write_fixture(&dir, "records.data", "name,count\nalpha,1\n")?;

    let assert = cmd()?.arg(&path).arg("--to").arg("json").assert().success();
    let stdout = serde_json::from_slice::<Value>(&assert.get_output().stdout)?;
    assert_eq!(stdout, json!([{"name": "alpha", "count": 1}]));
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
        .stdout(predicate::eq("| name |\n| --- |\n| alice |\n"));

    Ok(())
}

#[test]
fn cli_defaults_to_json_lines_when_stdout_is_not_a_terminal() -> Result<()> {
    cmd()?
        .arg("--data")
        .arg(r#"[{"name":"Ada"},{"name":"Lin"}]"#)
        .assert()
        .success()
        .stdout(predicate::eq("{\"name\":\"Ada\"}\n{\"name\":\"Lin\"}\n"));
    Ok(())
}

#[test]
fn cli_accepts_inline_data_and_global_flags_after_a_subcommand() -> Result<()> {
    cmd()?
        .arg("select")
        .arg("name")
        .arg("--data")
        .arg(r#"[{"name":"Ada","age":36},{"name":"Lin","age":28}]"#)
        .arg("--raw")
        .assert()
        .success()
        .stdout(predicate::eq("Ada\nLin\n"));
    Ok(())
}

#[test]
fn cli_chains_open_filter_and_select_through_real_process_pipes() -> Result<()> {
    let dir = TempDir::new()?;
    let path = write_fixture(&dir, "people.csv", "name,age\nAda,36\nLin,28\nGrace,40\n")?;
    let binary = env!("CARGO_BIN_EXE_pl");

    let mut open = StdCommand::new(binary)
        .arg("open")
        .arg(&path)
        .stdout(Stdio::piped())
        .spawn()?;
    let open_stdout = open.stdout.take().expect("open stdout must be piped");

    let mut filter = StdCommand::new(binary)
        .arg("filter")
        .arg("age >= 30")
        .stdin(Stdio::from(open_stdout))
        .stdout(Stdio::piped())
        .spawn()?;
    let filter_stdout = filter.stdout.take().expect("filter stdout must be piped");

    let output = StdCommand::new(binary)
        .arg("select")
        .arg("name")
        .arg("--raw")
        .stdin(Stdio::from(filter_stdout))
        .output()?;

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(filter.wait()?.success());
    assert!(open.wait()?.success());
    assert_eq!(String::from_utf8(output.stdout)?, "Ada\nGrace\n");
    Ok(())
}

#[test]
fn cli_auto_detects_yaml_inline_input() -> Result<()> {
    let assert = cmd()?
        .arg("open")
        .arg("- name: Ada\n  age: 36\n- name: Lin\n  age: 28")
        .arg("--to")
        .arg("json")
        .assert()
        .success();
    let stdout = serde_json::from_slice::<Value>(&assert.get_output().stdout)?;
    assert_eq!(
        stdout,
        json!([
            {"name": "Ada", "age": 36},
            {"name": "Lin", "age": 28}
        ])
    );
    Ok(())
}

#[test]
fn cli_reads_tsv_from_stdin_without_a_format_flag() -> Result<()> {
    let assert = cmd()?
        .arg("--to")
        .arg("json")
        .write_stdin("name\tage\nAda\t36\nLin\t28\n")
        .assert()
        .success();
    let stdout = serde_json::from_slice::<Value>(&assert.get_output().stdout)?;
    assert_eq!(
        stdout,
        json!([
            {"name": "Ada", "age": 36},
            {"name": "Lin", "age": 28}
        ])
    );
    Ok(())
}

#[test]
fn cli_query_registers_the_pipeline_frame_as_df() -> Result<()> {
    let assert = cmd()?
        .arg("query")
        .arg("SELECT name FROM df WHERE age >= 30 ORDER BY name")
        .arg("--data")
        .arg(r#"[{"name":"Lin","age":28},{"name":"Ada","age":36}]"#)
        .arg("--to")
        .arg("json")
        .assert()
        .success();
    let stdout = serde_json::from_slice::<Value>(&assert.get_output().stdout)?;
    assert_eq!(stdout, json!([{"name": "Ada"}]));
    Ok(())
}

#[test]
fn cli_with_column_replaces_and_adds_columns() -> Result<()> {
    let assert = cmd()?
        .arg("with-column")
        .arg("age=age + 1")
        .arg("label=UPPER(name)")
        .arg("--data")
        .arg(r#"[{"name":"Ada","age":36}]"#)
        .arg("--to")
        .arg("json")
        .assert()
        .success();
    let stdout = serde_json::from_slice::<Value>(&assert.get_output().stdout)?;
    assert_eq!(stdout, json!([{"name": "Ada", "age": 37, "label": "ADA"}]));
    Ok(())
}

#[test]
fn cli_supports_sort_slice_drop_rename_unique_and_reverse() -> Result<()> {
    let data = r#"[{"id":1,"name":"Ada","age":36},{"id":1,"name":"Ada 2","age":37},{"id":2,"name":"Lin","age":28}]"#;

    let assert = cmd()?
        .args(["sort-by", "age:desc", "--data", data, "--to", "json"])
        .assert()
        .success();
    let sorted = serde_json::from_slice::<Value>(&assert.get_output().stdout)?;
    assert_eq!(sorted[0]["age"], 37);

    cmd()?
        .args(["slice", "1", "1", "--data", data, "--raw"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("exactly one column"));

    let assert = cmd()?
        .args(["drop", "age", "--data", data, "--to", "json"])
        .assert()
        .success();
    let dropped = serde_json::from_slice::<Value>(&assert.get_output().stdout)?;
    assert!(dropped[0].get("age").is_none());

    let assert = cmd()?
        .args(["rename", "name=person", "--data", data, "--to", "json"])
        .assert()
        .success();
    let renamed = serde_json::from_slice::<Value>(&assert.get_output().stdout)?;
    assert_eq!(renamed[0]["person"], "Ada");

    let assert = cmd()?
        .args(["unique", "id", "--data", data, "--to", "json"])
        .assert()
        .success();
    let unique = serde_json::from_slice::<Value>(&assert.get_output().stdout)?;
    assert_eq!(unique.as_array().expect("array").len(), 2);

    let assert = cmd()?
        .args(["reverse", "--data", data, "--to", "json"])
        .assert()
        .success();
    let reversed = serde_json::from_slice::<Value>(&assert.get_output().stdout)?;
    assert_eq!(reversed[0]["id"], 2);
    Ok(())
}

#[test]
fn cli_drops_null_rows_for_selected_columns() -> Result<()> {
    let assert = cmd()?
        .args([
            "drop-nulls",
            "score",
            "--data",
            r#"[{"name":"Ada","score":9},{"name":"Lin","score":null}]"#,
            "--to",
            "json",
        ])
        .assert()
        .success();
    let stdout = serde_json::from_slice::<Value>(&assert.get_output().stdout)?;
    assert_eq!(stdout, json!([{"name": "Ada", "score": 9}]));
    Ok(())
}

#[test]
fn cli_groups_with_sql_aggregations() -> Result<()> {
    let assert = cmd()?
        .args([
            "group-by",
            "team",
            "--agg",
            "SUM(score) AS total",
            "--data",
            r#"[{"team":"a","score":2},{"team":"a","score":3},{"team":"b","score":1}]"#,
            "--to",
            "json",
        ])
        .assert()
        .success();
    let mut stdout = serde_json::from_slice::<Value>(&assert.get_output().stdout)?;
    stdout
        .as_array_mut()
        .expect("array")
        .sort_by_key(|row| row["team"].as_str().map(str::to_owned));
    assert_eq!(
        stdout,
        json!([
            {"team": "a", "total": 5},
            {"team": "b", "total": 1}
        ])
    );
    Ok(())
}

#[test]
fn cli_join_coalesces_same_name_keys_and_avoids_duplicate_columns() -> Result<()> {
    let assert = cmd()?
        .arg("join")
        .arg(r#"[{"id":1,"score":9},{"id":3,"score":7}]"#)
        .args(["--on", "id", "--how", "full"])
        .arg("--data")
        .arg(r#"[{"id":1,"name":"Ada"},{"id":2,"name":"Lin"}]"#)
        .args(["--to", "json"])
        .assert()
        .success();
    let stdout = serde_json::from_slice::<Value>(&assert.get_output().stdout)?;
    let rows = stdout.as_array().expect("array");
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().all(|row| row.get("id:right_df").is_none()));
    assert!(rows.iter().any(|row| row["id"] == 3));
    Ok(())
}

#[test]
fn cli_concatenates_files_literals_and_stdin() -> Result<()> {
    let assert = cmd()?
        .arg("concat")
        .arg(r#"[{"id":1}]"#)
        .arg(r#"[{"id":2}]"#)
        .args(["--to", "json"])
        .assert()
        .success();
    let stdout = serde_json::from_slice::<Value>(&assert.get_output().stdout)?;
    assert_eq!(stdout, json!([{"id": 1}, {"id": 2}]));

    let assert = cmd()?
        .arg("concat")
        .arg(r#"[{"id":2}]"#)
        .args(["--to", "json"])
        .write_stdin("{\"id\":1}\n")
        .assert()
        .success();
    let stdout = serde_json::from_slice::<Value>(&assert.get_output().stdout)?;
    assert_eq!(stdout, json!([{"id": 1}, {"id": 2}]));
    Ok(())
}

#[test]
fn cli_reports_columns_schema_shape_and_count() -> Result<()> {
    let data = r#"[{"name":"Ada","age":36},{"name":"Lin","age":28}]"#;

    cmd()?
        .args(["columns", "--data", data, "--raw"])
        .assert()
        .success()
        .stdout(predicate::eq("name\nage\n"));

    let assert = cmd()?
        .args(["schema", "--data", data, "--to", "json"])
        .assert()
        .success();
    let schema = serde_json::from_slice::<Value>(&assert.get_output().stdout)?;
    assert_eq!(schema[0], json!({"column": "name", "dtype": "str"}));

    let assert = cmd()?
        .args(["shape", "--data", data, "--to", "json"])
        .assert()
        .success();
    let shape = serde_json::from_slice::<Value>(&assert.get_output().stdout)?;
    assert_eq!(shape, json!([{"rows": 2, "columns": 2}]));

    cmd()?
        .args(["count", "--data", data, "--raw"])
        .assert()
        .success()
        .stdout(predicate::eq("2\n"));
    Ok(())
}

#[test]
fn cli_to_command_consumes_pipeline_json_lines() -> Result<()> {
    cmd()?
        .arg("to")
        .arg("csv")
        .write_stdin("{\"name\":\"Ada\",\"age\":36}\n{\"name\":\"Lin\",\"age\":28}\n")
        .assert()
        .success()
        .stdout(predicate::eq("name,age\nAda,36\nLin,28\n"));
    Ok(())
}

#[test]
fn cli_infers_output_format_from_the_output_file_extension() -> Result<()> {
    let dir = TempDir::new()?;
    let output = dir.path().join("people.yaml");
    cmd()?
        .args(["select", "name"])
        .arg("--data")
        .arg(r#"[{"name":"Ada","age":36}]"#)
        .arg("--output")
        .arg(&output)
        .assert()
        .success()
        .stdout(predicate::str::is_empty());
    assert_eq!(std::fs::read_to_string(output)?, "- name: Ada\n");
    Ok(())
}

#[test]
fn cli_pretty_prints_json() -> Result<()> {
    cmd()?
        .args(["--data", r#"[{"name":"Ada","age":36}]"#, "--pretty"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\n  {\n"));
    Ok(())
}

#[test]
fn cli_generates_bash_completions() -> Result<()> {
    cmd()?
        .args(["completions", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::contains("_pl()"));
    Ok(())
}

#[test]
fn cli_reports_an_actionable_error_for_a_missing_path() -> Result<()> {
    cmd()?
        .args(["open", "missing.json"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("input path does not exist"))
        .stderr(predicate::str::contains("--data"));
    Ok(())
}

#[test]
fn cli_accepts_a_leading_direct_input_before_the_subcommand() -> Result<()> {
    let dir = TempDir::new()?;
    let path = write_fixture(&dir, "people.csv", "name,age\nAda,36\nLin,28\n")?;

    let assert = cmd()?
        .arg(&path)
        .args(["filter", "age >= 30", "--to", "json"])
        .assert()
        .success();
    let stdout = serde_json::from_slice::<Value>(&assert.get_output().stdout)?;
    assert_eq!(stdout, json!([{"name": "Ada", "age": 36}]));

    cmd()?
        .arg(r#"[{"name":"Ada","age":36}]"#)
        .args(["select", "name", "--raw"])
        .assert()
        .success()
        .stdout(predicate::eq("Ada\n"));
    Ok(())
}

#[test]
fn cli_treats_output_dash_as_stdout() -> Result<()> {
    cmd()?
        .args([
            "select",
            "name",
            "--data",
            r#"[{"name":"Ada"}]"#,
            "--to",
            "json",
            "--output",
            "-",
        ])
        .assert()
        .success()
        .stdout(predicate::eq("[{\"name\":\"Ada\"}]\n"));
    Ok(())
}

#[test]
fn cli_propagates_zero_row_results_through_real_process_pipes() -> Result<()> {
    let binary = env!("CARGO_BIN_EXE_pl");

    let mut filter = StdCommand::new(binary)
        .args([
            "filter",
            "age > 100",
            "--data",
            r#"[{"name":"Ada","age":36}]"#,
        ])
        .stdout(Stdio::piped())
        .spawn()?;
    let filter_stdout = filter.stdout.take().expect("filter stdout must be piped");

    let mut select = StdCommand::new(binary)
        .args(["select", "name"])
        .stdin(Stdio::from(filter_stdout))
        .stdout(Stdio::piped())
        .spawn()?;
    let select_stdout = select.stdout.take().expect("select stdout must be piped");

    let output = StdCommand::new(binary)
        .args(["to", "json"])
        .stdin(Stdio::from(select_stdout))
        .output()?;

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(select.wait()?.success());
    assert!(filter.wait()?.success());
    assert_eq!(String::from_utf8(output.stdout)?, "[]\n");
    Ok(())
}

#[test]
fn cli_counts_an_empty_pipeline_as_zero() -> Result<()> {
    cmd()?
        .args(["count", "--raw"])
        .write_stdin("")
        .assert()
        .success()
        .stdout(predicate::eq("0\n"));
    Ok(())
}

#[test]
fn cli_rejects_pretty_json_with_a_non_json_output_extension() -> Result<()> {
    let dir = TempDir::new()?;
    let output = dir.path().join("people.yaml");
    cmd()?
        .args(["--data", r#"[{"name":"Ada"}]"#, "--pretty"])
        .arg("--output")
        .arg(output)
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "--pretty is only valid with JSON output",
        ));
    Ok(())
}

#[test]
fn cli_global_to_overrides_the_to_subcommand_format() -> Result<()> {
    cmd()?
        .args(["to", "csv", "--to", "json"])
        .write_stdin("{\"name\":\"Ada\"}\n")
        .assert()
        .success()
        .stdout(predicate::eq("[{\"name\":\"Ada\"}]\n"));
    Ok(())
}

#[test]
fn cli_still_requires_join_keys_when_an_input_is_empty() -> Result<()> {
    cmd()?
        .arg("join")
        .arg(r#"[{"id":1}]"#)
        .arg("--data")
        .arg("[]")
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "join requires --on or matching --left-on/--right-on keys",
        ));
    Ok(())
}

#[test]
fn cli_terminates_json_stdout_with_a_single_newline() -> Result<()> {
    cmd()?
        .args(["--data", r#"[{"name":"Ada"}]"#, "--to", "json"])
        .assert()
        .success()
        .stdout(predicate::eq("[{\"name\":\"Ada\"}]\n"));
    Ok(())
}

#[test]
fn cli_terminates_table_markdown_html_and_xml_stdout_with_a_single_newline() -> Result<()> {
    let data = r#"[{"name":"Ada"}]"#;

    cmd()?
        .args(["--data", data, "--to", "table"])
        .assert()
        .success()
        .stdout(predicate::eq("name\n----\nAda\n"));

    cmd()?
        .args(["--data", data, "--to", "markdown"])
        .assert()
        .success()
        .stdout(predicate::eq("| name |\n| --- |\n| Ada |\n"));

    cmd()?
        .args(["--data", data, "--to", "html"])
        .assert()
        .success()
        .stdout(predicate::eq(
            "<table><thead><tr><th>name</th></tr></thead><tbody><tr><td>Ada</td></tr></tbody></table>\n",
        ));

    cmd()?
        .args(["--data", data, "--to", "xml"])
        .assert()
        .success()
        .stdout(predicate::eq(
            "<rows><row><field name=\"name\">Ada</field></row></rows>\n",
        ));
    Ok(())
}

#[test]
fn cli_pretty_json_ends_with_a_single_newline() -> Result<()> {
    let stdout = cmd()?
        .args(["--data", r#"[{"name":"Ada"}]"#, "--pretty"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(stdout)?;
    assert!(stdout.ends_with('\n'), "{stdout:?}");
    assert!(!stdout.ends_with("\n\n"), "{stdout:?}");
    Ok(())
}

#[test]
fn cli_writes_json_files_with_a_trailing_newline() -> Result<()> {
    let dir = TempDir::new()?;
    let output = dir.path().join("people.json");
    cmd()?
        .args(["--data", r#"[{"name":"Ada"}]"#, "--output"])
        .arg(&output)
        .assert()
        .success()
        .stdout(predicate::str::is_empty());
    assert_eq!(std::fs::read_to_string(output)?, "[{\"name\":\"Ada\"}]\n");
    Ok(())
}

#[test]
fn cli_keeps_significant_text_whitespace_when_reading_stdin() -> Result<()> {
    let assert = cmd()?
        .args(["--from", "text", "--to", "json"])
        .write_stdin("  Ada  \n")
        .assert()
        .success();
    assert_eq!(
        String::from_utf8(assert.get_output().stdout.clone())?,
        "[{\"line\":\"  Ada  \"}]\n"
    );
    Ok(())
}

#[test]
fn cli_keeps_a_leading_blank_text_line_when_reading_stdin() -> Result<()> {
    let assert = cmd()?
        .args(["--from", "text", "--to", "json"])
        .write_stdin("\nAda\n")
        .assert()
        .success();
    assert_eq!(
        String::from_utf8(assert.get_output().stdout.clone())?,
        "[{\"line\":\"\"},{\"line\":\"Ada\"}]\n"
    );
    Ok(())
}

#[test]
fn cli_chains_json_with_a_trailing_newline_through_a_real_pipe() -> Result<()> {
    let binary = env!("CARGO_BIN_EXE_pl");

    let mut first = StdCommand::new(binary)
        .args(["--data", r#"[{"name":"Ada","age":36}]"#, "--to", "json"])
        .stdout(Stdio::piped())
        .spawn()?;
    let first_stdout = first.stdout.take().expect("first stdout must be piped");

    let output = StdCommand::new(binary)
        .args(["select", "name", "--raw"])
        .stdin(Stdio::from(first_stdout))
        .output()?;

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(first.wait()?.success());
    assert_eq!(String::from_utf8(output.stdout)?, "Ada\n");
    Ok(())
}

const PEOPLE: &str = r#"[
  {"name":"Ada","team":"red","age":36,"score":9.5,"joined":"2021-03-05"},
  {"name":"Bob","team":"blue","age":null,"score":7.25,"joined":"2022-11-30"},
  {"name":"Cy","team":"red","age":25,"score":8.0,"joined":"2020-01-15"},
  {"name":"Di","team":"blue","age":41,"score":null,"joined":"2023-07-04"}
]"#;

/// Run `pl ARGS --data DATA --to json` and parse stdout.
fn json_for(data: &str, args: &[&str]) -> Result<Value> {
    let assert = cmd()?
        .args(args)
        .arg("--data")
        .arg(data)
        .arg("--to")
        .arg("json")
        .assert()
        .success();
    Ok(serde_json::from_slice(&assert.get_output().stdout)?)
}

/// Run `pl ARGS --data DATA` and return the error message.
fn error_for(data: &str, args: &[&str]) -> Result<String> {
    let assert = cmd()?
        .args(args)
        .arg("--data")
        .arg(data)
        .env("RUST_BACKTRACE", "0")
        .assert()
        .failure();
    Ok(String::from_utf8(assert.get_output().stderr.clone())?)
}

#[test]
fn cli_selects_columns_by_type_regex_and_index_selectors() -> Result<()> {
    assert_eq!(
        json_for(PEOPLE, &["select", "type:numeric"])?[0],
        json!({"age": 36, "score": 9.5})
    );
    assert_eq!(
        json_for(PEOPLE, &["select", "^[nt].*$", "index:-1"])?[0],
        json!({"name": "Ada", "team": "red", "joined": "2021-03-05"})
    );
    assert_eq!(
        json_for(PEOPLE, &["drop", "type:string"])?[0],
        json!({"age": 36, "score": 9.5})
    );
    Ok(())
}

#[test]
fn cli_exact_column_names_win_over_selector_syntax() -> Result<()> {
    assert_eq!(
        json_for(r#"[{"type:odd":1,"x":2}]"#, &["select", "type:odd"])?,
        json!([{"type:odd": 1}])
    );
    Ok(())
}

#[test]
fn cli_column_commands_replace_in_place_or_write_new_columns_with_as() -> Result<()> {
    let data = r#"[{"a":"x","b":"y","n":1}]"#;
    assert_eq!(
        json_for(data, &["uppercase", "a"])?,
        json!([{"a": "X", "b": "y", "n": 1}])
    );
    assert_eq!(
        json_for(data, &["uppercase", "a", "--as", "a_up"])?,
        json!([{"a": "x", "b": "y", "n": 1, "a_up": "X"}])
    );
    assert_eq!(
        json_for(data, &["uppercase", "type:string", "--as", "{}_up"])?,
        json!([{"a": "x", "b": "y", "n": 1, "a_up": "X", "b_up": "Y"}])
    );
    assert!(
        error_for(data, &["uppercase", "a", "b", "--as", "c"])?
            .contains("--as needs exactly one column")
    );
    assert!(error_for(data, &["uppercase", "n"])?.contains("requires a string column"));
    Ok(())
}

#[test]
fn cli_string_commands_replace_slice_split_strip_and_measure() -> Result<()> {
    let data = r#"[{"s":"  banana  ","t":"a-b-c"}]"#;
    assert_eq!(
        json_for(data, &["str-strip-chars", "--whitespace", "s"])?[0]["s"],
        json!("banana")
    );
    assert_eq!(
        json_for(data, &["str-strip-chars", " a", "s", "--start"])?[0]["s"],
        json!("banana  ")
    );
    assert_eq!(
        json_for(data, &["str-replace", "-p", "an", "-r", "AN", "s"])?[0]["s"],
        json!("  bANana  ")
    );
    assert_eq!(
        json_for(data, &["str-replace-all", "-p", "an", "-r", "AN", "s"])?[0]["s"],
        json!("  bANANa  ")
    );
    assert_eq!(
        json_for(
            data,
            &["str-replace-all", "-p", ".", "-r", "", "t", "--literal"]
        )?[0]["t"],
        json!("a-b-c")
    );
    assert_eq!(
        json_for(data, &["str-slice", "-3", "t"])?[0]["t"],
        json!("b-c")
    );
    assert_eq!(
        json_for(data, &["str-slice", "0", "t", "--length", "1"])?[0]["t"],
        json!("a")
    );
    assert_eq!(
        json_for(data, &["str-split", "-", "t"])?[0]["t"],
        json!(["a", "b", "c"])
    );
    assert_eq!(json_for(data, &["str-lengths", "t"])?[0]["t"], json!(5));
    assert_eq!(
        json_for(r#"[{"s":"é"}]"#, &["str-lengths", "--bytes", "s"])?[0]["s"],
        json!(2)
    );
    assert_eq!(
        json_for(
            r#"[{"s":"a"},{"s":null},{"s":"b"}]"#,
            &["str-join", "-d", "+", "--ignore-nulls"]
        )?,
        json!([{"s": "a+b"}])
    );
    Ok(())
}

#[test]
fn cli_mask_commands_produce_boolean_columns() -> Result<()> {
    assert_eq!(
        json_for(PEOPLE, &["contains", "^[AB]", "name", "--as", "ab"])?
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["ab"].clone())
            .collect::<Vec<_>>(),
        [json!(true), json!(true), json!(false), json!(false)]
    );
    assert_eq!(
        json_for(PEOPLE, &["is-in", "red,green", "team"])?[1]["team"],
        json!(false)
    );
    assert_eq!(
        json_for(PEOPLE, &["is-null", "age"])?[1]["age"],
        json!(true)
    );
    assert_eq!(
        json_for(PEOPLE, &["is-not-null", "score"])?[3]["score"],
        json!(false)
    );
    assert_eq!(
        json_for(r#"[{"ok":true},{"ok":false}]"#, &["not"])?,
        json!([{"ok": false}, {"ok": true}])
    );
    assert_eq!(
        json_for(
            r#"[{"a":1,"b":1},{"a":1,"b":2},{"a":1,"b":1}]"#,
            &["is-duplicated"]
        )?,
        json!([{"is_duplicated": true}, {"is_duplicated": false}, {"is_duplicated": true}])
    );
    assert_eq!(
        json_for(
            r#"[{"a":1,"b":1},{"a":1,"b":2}]"#,
            &["is-unique", "a", "--as", "u"]
        )?,
        json!([{"a": 1, "b": 1, "u": false}, {"a": 1, "b": 2, "u": false}])
    );
    Ok(())
}

#[test]
fn cli_temporal_commands_parse_iso_strings_from_json_pipelines() -> Result<()> {
    let data = r#"[{"d":"2024-03-05"},{"d":"2023-12-31"}]"#;
    assert_eq!(
        json_for(data, &["get-year", "d", "--as", "y"])?,
        json!([{"d": "2024-03-05", "y": 2024}, {"d": "2023-12-31", "y": 2023}])
    );
    assert_eq!(json_for(data, &["get-month", "d"])?[0]["d"], json!(3));
    assert_eq!(json_for(data, &["get-day", "d"])?[0]["d"], json!(5));
    assert_eq!(json_for(data, &["get-ordinal", "d"])?[1]["d"], json!(365));
    assert_eq!(json_for(data, &["get-weekday", "d"])?[0]["d"], json!(2));
    assert_eq!(
        json_for(data, &["datepart", "quarter", "d"])?[0]["d"],
        json!(1)
    );
    assert_eq!(
        json_for(data, &["strftime", "%d/%m/%Y", "d"])?[0]["d"],
        json!("05/03/2024")
    );
    assert_eq!(
        json_for(data, &["truncate", "1mo", "d"])?[0]["d"],
        json!("2024-03-01")
    );
    let times = r#"[{"t":"2024-03-05 13:45:30"}]"#;
    assert_eq!(json_for(times, &["get-hour", "t"])?[0]["t"], json!(13));
    assert_eq!(json_for(times, &["get-minute", "t"])?[0]["t"], json!(45));
    assert_eq!(json_for(times, &["get-second", "t"])?[0]["t"], json!(30));
    Ok(())
}

#[test]
fn cli_parses_dates_and_datetimes_with_formats_and_time_zones() -> Result<()> {
    assert_eq!(
        json_for(r#"[{"d":"05/03/2024"}]"#, &["as-date", "%d/%m/%Y", "d"])?[0]["d"],
        json!("2024-03-05")
    );
    assert!(
        error_for(r#"[{"d":"2024-03-05"}]"#, &["as-date", "%d/%m/%Y", "d"])?
            .contains("as-date failed")
    );
    let assert = cmd()?
        .args([
            "as-datetime",
            "%Y-%m-%d %H:%M",
            "t",
            "--time-zone",
            "UTC",
            "--data",
            r#"[{"t":"2024-07-01 12:00"}]"#,
            "--to",
            "arrow",
        ])
        .assert()
        .success();
    let utc = assert.get_output().stdout.clone();
    cmd()?
        .args(["convert-time-zone", "Europe/Paris", "t", "--to", "csv"])
        .write_stdin(utc.clone())
        .assert()
        .success()
        .stdout(predicate::eq("t\n2024-07-01T14:00:00.000000+0200\n"));
    cmd()?
        .args(["replace-time-zone", "null", "t", "--to", "csv"])
        .write_stdin(utc)
        .assert()
        .success()
        .stdout(predicate::eq("t\n2024-07-01T12:00:00.000000\n"));
    Ok(())
}

#[test]
fn cli_generates_date_and_datetime_ranges_without_reading_input() -> Result<()> {
    cmd()?
        .args([
            "date-range",
            "--start",
            "2024-02-27",
            "--end",
            "2024-03-01",
            "--to",
            "csv",
        ])
        .assert()
        .success()
        .stdout(predicate::eq(
            "date\n2024-02-27\n2024-02-28\n2024-02-29\n2024-03-01\n",
        ));
    cmd()?
        .args([
            "datetime-range",
            "--start",
            "2024-01-01 00:00:00",
            "--end",
            "2024-01-01 01:00:00",
            "--interval",
            "30m",
            "--closed",
            "left",
            "--as",
            "at",
            "--to",
            "csv",
        ])
        .assert()
        .success()
        .stdout(predicate::eq(
            "at\n2024-01-01T00:00:00.000000\n2024-01-01T00:30:00.000000\n",
        ));
    assert_eq!(
        json_for(
            r#"[{"a":"2024-01-01","b":"2024-01-03"}]"#,
            &[
                "date-ranges",
                "--start",
                "a",
                "--end",
                "b",
                "--interval",
                "2d"
            ]
        )?[0]["date_range"],
        json!(["2024-01-01", "2024-01-03"])
    );
    assert!(
        error_for(
            "[]",
            &["date-range", "--start", "2024-01-01", "--end", "2024-01-02"]
        )?
        .contains("does not read input")
    );
    Ok(())
}

#[test]
fn cli_aggregations_reduce_columns_to_one_row() -> Result<()> {
    assert_eq!(
        json_for(PEOPLE, &["sum"])?,
        json!([{"name": null, "team": null, "age": 102, "score": 24.75, "joined": null}])
    );
    assert_eq!(
        json_for(PEOPLE, &["mean", "age", "score"])?,
        json!([{"age": 34.0, "score": 8.25}])
    );
    assert_eq!(
        json_for(PEOPLE, &["min", "name", "age"])?,
        json!([{"name": "Ada", "age": 25}])
    );
    assert_eq!(json_for(PEOPLE, &["max", "age"])?, json!([{"age": 41}]));
    assert_eq!(
        json_for(PEOPLE, &["median", "age"])?,
        json!([{"age": 36.0}])
    );
    assert_eq!(
        json_for(PEOPLE, &["quantile", "0.5", "age", "--method", "lower"])?,
        json!([{"age": 36.0}])
    );
    assert_eq!(
        json_for(r#"[{"x":1},{"x":3}]"#, &["var"])?,
        json!([{"x": 2.0}])
    );
    assert_eq!(
        json_for(r#"[{"x":1},{"x":3}]"#, &["std", "--ddof", "0"])?,
        json!([{"x": 1.0}])
    );
    assert_eq!(
        json_for(PEOPLE, &["n-unique", "team"])?,
        json!([{"team": 2}])
    );
    assert_eq!(
        json_for(PEOPLE, &["count-null"])?,
        json!([{"name": 0, "team": 0, "age": 1, "score": 1, "joined": 0}])
    );
    assert_eq!(json_for(PEOPLE, &["count", "age"])?, json!([{"age": 3}]));
    assert_eq!(json_for(PEOPLE, &["count"])?, json!([{"count": 4}]));
    assert_eq!(json_for(PEOPLE, &["len"])?, json!([{"len": 4}]));
    assert_eq!(
        json_for(
            r#"[{"a":true,"b":false},{"a":true,"b":false}]"#,
            &["all-true"]
        )?,
        json!([{"a": true, "b": false}])
    );
    assert_eq!(
        json_for(
            r#"[{"a":true,"b":false},{"a":true,"b":false}]"#,
            &["all-false"]
        )?,
        json!([{"a": false, "b": true}])
    );
    assert_eq!(
        json_for(PEOPLE, &["implode", "name"])?,
        json!([{"name": ["Ada", "Bob", "Cy", "Di"]}])
    );
    assert!(error_for(PEOPLE, &["sum", "name"])?.contains("sum does not support \"name\""));
    Ok(())
}

#[test]
fn cli_value_counts_keep_first_appearance_order_and_exact_proportions() -> Result<()> {
    let data = r#"[{"t":"b"},{"t":"a"},{"t":"a"},{"t":"c"},{"t":"a"}]"#;
    assert_eq!(
        json_for(data, &["value-counts"])?,
        json!([{"t": "b", "count": 1}, {"t": "a", "count": 3}, {"t": "c", "count": 1}])
    );
    assert_eq!(
        json_for(data, &["value-counts", "--sort", "--normalize"])?,
        json!([
            {"t": "a", "proportion": 0.6},
            {"t": "b", "proportion": 0.2},
            {"t": "c", "proportion": 0.2}
        ])
    );
    assert_eq!(
        json_for(data, &["value-counts", "t", "--column", "n"])?[0],
        json!({"t": "b", "n": 1})
    );
    Ok(())
}

#[test]
fn cli_summary_matches_the_nushell_layout() -> Result<()> {
    let summary = json_for(r#"[{"a":1,"s":"x"},{"a":3,"s":"y"}]"#, &["summary"])?;
    let descriptors = summary
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["descriptor"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        descriptors,
        [
            "count", "sum", "mean", "median", "std", "min", "25%", "50%", "75%", "max"
        ]
    );
    assert_eq!(summary[0]["a (i64)"], json!(2.0));
    assert_eq!(summary[2]["a (i64)"], json!(2.0));
    assert_eq!(summary[4]["a (i64)"], json!(1.0));
    assert_eq!(summary[1]["s (str)"], json!(null));
    Ok(())
}

#[test]
fn cli_fill_null_and_fill_nan_fill_compatible_columns() -> Result<()> {
    assert_eq!(
        json_for(PEOPLE, &["fill-null", "0"])?[1],
        json!({"name": "Bob", "team": "blue", "age": 0, "score": 7.25, "joined": "2022-11-30"})
    );
    assert_eq!(
        json_for(r#"[{"s":null,"n":null}]"#, &["fill-null", "n/a", "s"])?,
        json!([{"s": "n/a", "n": null}])
    );
    assert_eq!(
        json_for(
            PEOPLE,
            &["fill-null", "--strategy", "forward", "age", "score"]
        )?[3],
        json!({"name": "Di", "team": "blue", "age": 41, "score": 8.0, "joined": "2023-07-04"})
    );
    let assert = cmd()?
        .args(["fill-nan", "0", "--to", "json"])
        .write_stdin("x,y\n1.5,a\nNaN,b\n")
        .assert()
        .success();
    assert_eq!(
        serde_json::from_slice::<Value>(&assert.get_output().stdout)?,
        json!([{"x": 1.5, "y": "a"}, {"x": 0.0, "y": "b"}])
    );
    Ok(())
}

#[test]
fn cli_cast_is_strict_unless_non_strict() -> Result<()> {
    assert_eq!(
        json_for(r#"[{"v":"1"},{"v":"2"}]"#, &["cast", "i64", "v"])?,
        json!([{"v": 1}, {"v": 2}])
    );
    assert!(error_for(r#"[{"v":"x"}]"#, &["cast", "i64", "v"])?.contains("cast failed"));
    assert_eq!(
        json_for(
            r#"[{"v":"x"},{"v":"2"}]"#,
            &["cast", "i64", "v", "--non-strict"]
        )?,
        json!([{"v": null}, {"v": 2}])
    );
    assert!(error_for(r#"[{"v":"1"}]"#, &["cast", "nope", "v"])?.contains("unknown data type"));
    Ok(())
}

#[test]
fn cli_shift_cumulative_and_rolling_compute_window_columns() -> Result<()> {
    let data = r#"[{"x":1},{"x":2},{"x":3},{"x":4}]"#;
    assert_eq!(
        json_for(data, &["shift", "1", "--fill", "0"])?,
        json!([{"x": 0}, {"x": 1}, {"x": 2}, {"x": 3}])
    );
    assert_eq!(
        json_for(data, &["shift", "-1"])?,
        json!([{"x": 2}, {"x": 3}, {"x": 4}, {"x": null}])
    );
    assert_eq!(
        json_for(data, &["cumulative", "sum", "x", "--as", "total"])?[3],
        json!({"x": 4, "total": 10})
    );
    assert_eq!(
        json_for(data, &["cumulative", "max", "--reverse"])?,
        json!([{"x": 4}, {"x": 4}, {"x": 4}, {"x": 4}])
    );
    assert_eq!(
        json_for(data, &["rolling", "sum", "2"])?,
        json!([{"x": null}, {"x": 3}, {"x": 5}, {"x": 7}])
    );
    assert_eq!(
        json_for(data, &["rolling", "mean", "2", "--min-periods", "1"])?,
        json!([{"x": 1.0}, {"x": 1.5}, {"x": 2.5}, {"x": 3.5}])
    );
    Ok(())
}

#[test]
fn cli_rolling_group_by_aggregates_over_an_index_column() -> Result<()> {
    assert_eq!(
        json_for(
            r#"[{"t":"2024-01-01","x":1},{"t":"2024-01-02","x":2},{"t":"2024-01-04","x":3}]"#,
            &[
                "rolling",
                "--index-column",
                "t",
                "--period",
                "2d",
                "--agg",
                "SUM(x) AS s",
            ]
        )?,
        json!([
            {"t": "2024-01-01", "s": 1},
            {"t": "2024-01-02", "s": 3},
            {"t": "2024-01-04", "s": 3}
        ])
    );
    Ok(())
}

#[test]
fn cli_pivot_and_unpivot_keep_first_appearance_order() -> Result<()> {
    let long = r#"[
      {"region":"s","month":"feb","rev":4},
      {"region":"n","month":"jan","rev":1},
      {"region":"n","month":"feb","rev":2},
      {"region":"s","month":"jan","rev":3}
    ]"#;
    let wide = json_for(
        long,
        &[
            "pivot", "--on", "month", "--index", "region", "--values", "rev",
        ],
    )?;
    assert_eq!(
        wide,
        json!([
            {"region": "s", "feb": 4, "jan": 3},
            {"region": "n", "feb": 2, "jan": 1}
        ])
    );
    assert_eq!(
        json_for(
            r#"[{"r":"n","m":"jan","v":1},{"r":"n","m":"jan","v":2}]"#,
            &["pivot", "--on", "m", "--index", "r", "--aggregate", "sum"]
        )?,
        json!([{"r": "n", "jan": 3}])
    );
    assert!(
        error_for(
            r#"[{"r":"n","m":"jan","v":1},{"r":"n","m":"jan","v":2}]"#,
            &["pivot", "--on", "m", "--index", "r"]
        )?
        .contains("use --aggregate")
    );
    assert_eq!(
        json_for(
            r#"[{"id":1,"a":10,"b":20}]"#,
            &[
                "unpivot",
                "--index",
                "id",
                "--variable-name",
                "k",
                "--value-name",
                "v"
            ]
        )?,
        json!([{"id": 1, "k": "a", "v": 10}, {"id": 1, "k": "b", "v": 20}])
    );
    Ok(())
}

#[test]
fn cli_explode_unnest_and_struct_commands_handle_nested_columns() -> Result<()> {
    let lists = r#"[{"a":1,"xs":[1,2]},{"a":2,"xs":[]},{"a":3,"xs":null}]"#;
    assert_eq!(
        json_for(lists, &["explode"])?,
        json!([{"a": 1, "xs": 1}, {"a": 1, "xs": 2}])
    );
    assert_eq!(
        json_for(lists, &["flatten", "xs", "--empty-as-null", "--keep-nulls"])?,
        json!([
            {"a": 1, "xs": 1},
            {"a": 1, "xs": 2},
            {"a": 2, "xs": null},
            {"a": 3, "xs": null}
        ])
    );
    assert_eq!(
        json_for(lists, &["list-contains", "2", "xs", "--as", "has2"])?[0]["has2"],
        json!(true)
    );
    let structs = r#"[{"id":1,"s":{"x":1,"y":"q"}}]"#;
    assert_eq!(
        json_for(structs, &["unnest"])?,
        json!([{"id": 1, "x": 1, "y": "q"}])
    );
    assert_eq!(
        json_for(structs, &["unnest", "s", "--separator", "."])?,
        json!([{"id": 1, "s.x": 1, "s.y": "q"}])
    );
    assert_eq!(
        json_for(structs, &["struct-json-encode", "s"])?,
        json!([{"id": 1, "s": "{\"x\":1,\"y\":\"q\"}"}])
    );
    assert_eq!(
        json_for(r#"{"a":[1,2],"b":["x","y"]}"#, &["into-df", "--as-columns"])?,
        json!([{"a": 1, "b": "x"}, {"a": 2, "b": "y"}])
    );
    Ok(())
}

#[test]
fn cli_dummies_one_hot_encode_selected_columns() -> Result<()> {
    assert_eq!(
        json_for(
            r#"[{"t":"red","n":1},{"t":"blue","n":2}]"#,
            &["dummies", "t", "--separator", ":"]
        )?,
        json!([{"t:blue": 0, "t:red": 1, "n": 1}, {"t:blue": 1, "t:red": 0, "n": 2}])
    );
    Ok(())
}

#[test]
fn cli_append_adds_columns_by_default_and_rows_with_col() -> Result<()> {
    let left = r#"[{"a":1},{"a":2}]"#;
    let right = r#"[{"a":9},{"a":8}]"#;
    assert_eq!(
        json_for(left, &["append", right])?,
        json!([{"a": 1, "a_x": 9}, {"a": 2, "a_x": 8}])
    );
    assert_eq!(
        json_for(left, &["append", right, "--col"])?,
        json!([{"a": 1}, {"a": 2}, {"a": 9}, {"a": 8}])
    );
    assert!(error_for(left, &["append", r#"[{"b":1}]"#])?.contains("equal row counts"));
    Ok(())
}

#[test]
fn cli_take_and_sample_select_rows() -> Result<()> {
    assert_eq!(
        json_for(PEOPLE, &["take", "0", "-1", "1"])?
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["name"].clone())
            .collect::<Vec<_>>(),
        [json!("Ada"), json!("Di"), json!("Bob")]
    );
    assert_eq!(
        json_for(PEOPLE, &["take", "[2,0]"])?[0]["name"],
        json!("Cy")
    );
    let first = json_for(PEOPLE, &["sample", "-n", "3", "--seed", "11"])?;
    let second = json_for(PEOPLE, &["sample", "-n", "3", "--seed", "11"])?;
    assert_eq!(first.as_array().unwrap().len(), 3);
    assert_eq!(first, second);
    assert_eq!(
        json_for(PEOPLE, &["sample", "--fraction", "0.5", "--seed", "1"])?
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(error_for(PEOPLE, &["take", "9"])?.contains("out of range"));
    assert!(error_for(PEOPLE, &["sample", "-n", "9"])?.contains("without --replace"));
    Ok(())
}

#[test]
fn cli_join_supports_semi_anti_right_and_custom_suffixes() -> Result<()> {
    let left = r#"[{"id":1,"v":"a"},{"id":2,"v":"b"},{"id":3,"v":"c"}]"#;
    let right = r#"[{"id":2,"v":"x"},{"id":4,"v":"y"}]"#;
    assert_eq!(
        json_for(left, &["join", right, "--on", "id", "--how", "semi"])?,
        json!([{"id": 2, "v": "b"}])
    );
    assert_eq!(
        json_for(left, &["join", right, "--on", "id", "--how", "anti"])?,
        json!([{"id": 1, "v": "a"}, {"id": 3, "v": "c"}])
    );
    assert_eq!(
        json_for(left, &["join", right, "--on", "id", "--how", "right"])?,
        json!([{"id": 2, "v": "b", "v_right": "x"}, {"id": 4, "v": null, "v_right": "y"}])
    );
    assert_eq!(
        json_for(left, &["join", right, "--on", "id", "--suffix", "_r"])?,
        json!([{"id": 2, "v": "b", "v_r": "x"}])
    );
    Ok(())
}

#[test]
fn cli_join_where_matches_inequality_predicates_in_left_row_order() -> Result<()> {
    assert_eq!(
        json_for(
            r#"[{"name":"Bob","cash":10},{"name":"Alice","cash":5}]"#,
            &[
                "join-where",
                r#"[{"item":"C","price":12},{"item":"A","price":3},{"item":"B","price":7}]"#,
                "cash > price",
            ]
        )?,
        json!([
            {"name": "Bob", "cash": 10, "item": "A", "price": 3},
            {"name": "Bob", "cash": 10, "item": "B", "price": 7},
            {"name": "Alice", "cash": 5, "item": "A", "price": 3}
        ])
    );
    Ok(())
}

#[test]
fn cli_concat_diagonal_aligns_columns_by_name() -> Result<()> {
    assert_eq!(
        json_for(r#"[{"a":1}]"#, &["concat", "--diagonal", r#"[{"b":"x"}]"#])?,
        json!([{"a": 1, "b": null}, {"a": null, "b": "x"}])
    );
    assert_eq!(
        json_for(
            r#"[{"a":1}]"#,
            &["concat", "--to-supertypes", r#"[{"a":1.5}]"#]
        )?,
        json!([{"a": 1.0}, {"a": 1.5}])
    );
    Ok(())
}

#[test]
fn cli_set_and_set_with_idx_overwrite_selected_rows() -> Result<()> {
    assert_eq!(
        json_for(PEOPLE, &["set", "0", "age", "--mask", "age > 30"])?
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["age"].clone())
            .collect::<Vec<_>>(),
        [json!(0), json!(null), json!(25), json!(0)]
    );
    assert_eq!(
        json_for(PEOPLE, &["set-with-idx", "X", "name", "--indices", "0,-1"])?
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["name"].clone())
            .collect::<Vec<_>>(),
        [json!("X"), json!("Bob"), json!("Cy"), json!("X")]
    );
    Ok(())
}

#[test]
fn cli_replace_maps_values_from_lists_or_a_json_object() -> Result<()> {
    assert_eq!(
        json_for(PEOPLE, &["replace", "red,blue", "R,B", "team"])?[1]["team"],
        json!("B")
    );
    let mapped = json_for(
        PEOPLE,
        &[
            "replace",
            "--map",
            r#"{"red":"R"}"#,
            "--default",
            "other",
            "team",
        ],
    )?;
    assert_eq!(mapped[0]["team"], json!("R"));
    assert_eq!(mapped[1]["team"], json!("other"));
    assert!(
        error_for(
            PEOPLE,
            &["replace", "--map", r#"{"red":"R"}"#, "--strict", "team"]
        )?
        .contains("replace failed")
    );
    Ok(())
}

#[test]
fn cli_cut_and_qcut_bin_numeric_columns() -> Result<()> {
    let data = r#"[{"x":1},{"x":5},{"x":9}]"#;
    assert_eq!(
        json_for(data, &["cut", "3,6", "x", "--labels", "lo,mid,hi"])?,
        json!([{"x": "lo"}, {"x": "mid"}, {"x": "hi"}])
    );
    assert_eq!(
        json_for(data, &["cut", "5", "x", "--left-closed"])?[1]["x"],
        json!("[5, inf)")
    );
    assert_eq!(
        json_for(
            data,
            &["qcut", "2", "x", "--labels", "low,high", "--as", "half"]
        )?
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["half"].clone())
        .collect::<Vec<_>>(),
        [json!("low"), json!("low"), json!("high")]
    );
    assert!(error_for(data, &["cut", "3", "x", "--labels", "a"])?.contains("label"));
    Ok(())
}

#[test]
fn cli_integer_and_decimal_parse_strings() -> Result<()> {
    assert_eq!(
        json_for(
            r#"[{"h":"ff"},{"h":"zz"}]"#,
            &["integer", "--base", "16", "h"]
        )?,
        json!([{"h": 255}, {"h": null}])
    );
    assert!(
        error_for(r#"[{"h":"zz"}]"#, &["integer", "--strict", "h"])?.contains("integer failed")
    );
    assert_eq!(
        json_for(r#"[{"d":"1.5"}]"#, &["decimal", "2", "d"])?,
        json!([{"d": "1.50"}])
    );
    Ok(())
}

#[test]
fn cli_math_subcommands_compute_elementwise_and_aggregate_values() -> Result<()> {
    assert_eq!(
        json_for(r#"[{"x":-4},{"x":9}]"#, &["math", "abs"])?,
        json!([{"x": 4}, {"x": 9}])
    );
    assert_eq!(
        json_for(r#"[{"x":4},{"x":9}]"#, &["math", "sqrt", "x", "--as", "r"])?[1]["r"],
        json!(3.0)
    );
    assert_eq!(
        json_for(r#"[{"x":8}]"#, &["math", "log", "--base", "2"])?,
        json!([{"x": 3.0}])
    );
    assert_eq!(
        json_for(r#"[{"x":-2},{"x":0}]"#, &["math", "sign"])?,
        json!([{"x": -1}, {"x": 0}])
    );
    assert_eq!(
        json_for(
            r#"[{"a":1,"b":2},{"a":3,"b":4}]"#,
            &["math", "dot", "a", "b"]
        )?,
        json!([{"dot": 14}])
    );
    assert_eq!(
        json_for(r#"[{"x":6},{"x":3}]"#, &["math", "bitwise-and"])?,
        json!([{"x": 2}])
    );
    assert_eq!(
        json_for(r#"[{"x":7}]"#, &["math", "bitwise-count-ones"])?,
        json!([{"x": 3}])
    );
    Ok(())
}

#[test]
fn cli_arg_commands_return_row_indices() -> Result<()> {
    assert_eq!(
        json_for(PEOPLE, &["arg-max", "age", "score"])?,
        json!([{"age": 3, "score": 0}])
    );
    assert_eq!(json_for(PEOPLE, &["arg-min", "age"])?, json!([{"age": 2}]));
    assert_eq!(
        json_for(PEOPLE, &["arg-where", "age > 30"])?,
        json!([{"arg_where": 0}, {"arg_where": 3}])
    );
    assert_eq!(
        json_for(r#"[{"b":false},{"b":true},{"b":true}]"#, &["arg-true"])?,
        json!([{"arg_true": 1}, {"arg_true": 2}])
    );
    assert_eq!(
        json_for(PEOPLE, &["arg-unique", "team"])?,
        json!([{"arg_unique": 0}, {"arg_unique": 1}])
    );
    assert_eq!(
        json_for(PEOPLE, &["arg-sort", "name", "--reverse", "--limit", "2"])?,
        json!([{"arg_sort": 3}, {"arg_sort": 2}])
    );
    Ok(())
}

#[test]
fn cli_horizontal_and_concat_str_combine_columns_per_row() -> Result<()> {
    assert_eq!(
        json_for(
            PEOPLE,
            &["horizontal", "sum", "age", "score", "--as", "total"]
        )?[1]["total"],
        json!(7.25)
    );
    assert_eq!(
        json_for(PEOPLE, &["horizontal", "sum", "age", "score", "--nulls"])?[1]["sum"],
        json!(null)
    );
    assert_eq!(
        json_for(r#"[{"a":1,"b":5}]"#, &["horizontal", "max", "a", "b"])?,
        json!([{"a": 1, "b": 5, "max": 5}])
    );
    assert_eq!(
        json_for(
            PEOPLE,
            &["concat-str", "-", "name", "team", "--as", "label"]
        )?[0]["label"],
        json!("Ada-red")
    );
    Ok(())
}

#[test]
fn cli_round_trips_parquet_arrow_and_avro_through_files_and_pipes() -> Result<()> {
    let dir = TempDir::new()?;
    let csv = write_fixture(
        &dir,
        "people.csv",
        "name,age,score,joined\nAda,36,9.5,2021-03-05\nBob,,7.25,2022-11-30\n",
    )?;
    let expected = "name,age,score,joined\nAda,36,9.5,2021-03-05\nBob,,7.25,2022-11-30\n";
    for (file, extra) in [
        ("out.parquet", vec![]),
        ("out.arrow", vec![]),
        ("out.avro", vec![]),
        ("snappy.avro", vec!["--avro-compression", "snappy"]),
    ] {
        let path = dir.path().join(file);
        cmd()?
            .arg("save")
            .arg(&path)
            .args(&extra)
            .arg("--input")
            .arg(&csv)
            .assert()
            .success()
            .stdout(predicate::eq(""));
        cmd()?
            .arg(&path)
            .arg("--to")
            .arg("csv")
            .assert()
            .success()
            .stdout(predicate::eq(expected));
    }

    let parquet = cmd()?
        .arg(&csv)
        .arg("--to")
        .arg("parquet")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert!(parquet.starts_with(b"PAR1"));
    cmd()?
        .args(["filter", "age > 30", "--to", "csv"])
        .write_stdin(parquet)
        .assert()
        .success()
        .stdout(predicate::eq(
            "name,age,score,joined\nAda,36,9.5,2021-03-05\n",
        ));
    Ok(())
}

#[test]
fn cli_arrow_pipelines_keep_types_and_empty_schemas() -> Result<()> {
    let typed = cmd()?
        .args([
            "as-date",
            "%Y-%m-%d",
            "d",
            "--data",
            r#"[{"d":"2024-03-05","n":1}]"#,
            "--to",
            "arrow",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    cmd()?
        .args(["schema", "--to", "csv"])
        .write_stdin(typed.clone())
        .assert()
        .success()
        .stdout(predicate::eq("column,dtype\nd,date\nn,i64\n"));
    let empty = cmd()?
        .args(["filter", "n > 5", "--to", "arrow"])
        .write_stdin(typed)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    cmd()?
        .args(["to", "csv"])
        .write_stdin(empty)
        .assert()
        .success()
        .stdout(predicate::eq("d,n\n"));
    Ok(())
}

#[test]
fn cli_open_applies_csv_read_options() -> Result<()> {
    let dir = TempDir::new()?;
    let path = write_fixture(&dir, "data.txt", "junk\n1;007;Ada\n2;010;Bob\n")?;
    let assert = cmd()?
        .arg("open")
        .arg(&path)
        .args([
            "--type",
            "csv",
            "-d",
            ";",
            "--skip-rows",
            "1",
            "--no-input-header",
            "--schema",
            "column_2:str",
            "--columns",
            "column_2,column_3",
            "--to",
            "json",
        ])
        .assert()
        .success();
    assert_eq!(
        serde_json::from_slice::<Value>(&assert.get_output().stdout)?,
        json!([
            {"column_2": "007", "column_3": "Ada"},
            {"column_2": "010", "column_3": "Bob"}
        ])
    );
    let ragged = write_fixture(&dir, "ragged.csv", "a,b\n1,2\n3,4,5\n")?;
    cmd()?
        .arg("open")
        .arg(&ragged)
        .args(["--truncate-ragged-lines", "--to", "csv"])
        .assert()
        .success()
        .stdout(predicate::eq("a,b\n1,2\n3,4\n"));
    cmd()?
        .arg("open")
        .arg(&ragged)
        .args([
            "--infer-schema",
            "0",
            "--truncate-ragged-lines",
            "--to",
            "json",
        ])
        .assert()
        .success()
        .stdout(predicate::eq(
            "[{\"a\":\"1\",\"b\":\"2\"},{\"a\":\"3\",\"b\":\"4\"}]\n",
        ));
    assert!(
        error_for(r#"[{"a":1}]"#, &["open", "--delimiter", ";"])?
            .contains("only apply to CSV/TSV input")
    );
    Ok(())
}

#[test]
fn cli_writes_csv_with_custom_delimiters() -> Result<()> {
    cmd()?
        .args([
            "--data",
            r#"[{"a":1,"b":"x"}]"#,
            "--to",
            "csv",
            "--output-delimiter",
            ";",
        ])
        .assert()
        .success()
        .stdout(predicate::eq("a;b\n1;x\n"));
    let dir = TempDir::new()?;
    let path = dir.path().join("out.csv");
    cmd()?
        .arg("save")
        .arg(&path)
        .args([
            "--csv-delimiter",
            "|",
            "--csv-no-header",
            "--data",
            r#"[{"a":1,"b":"x"}]"#,
        ])
        .assert()
        .success();
    assert_eq!(std::fs::read_to_string(path)?, "1|x\n");
    Ok(())
}

#[test]
fn cli_group_by_emits_groups_in_first_appearance_order() -> Result<()> {
    let rows = (0..2000)
        .map(|index| format!("{{\"k\":\"k{}\",\"v\":{index}}}", (index * 7919) % 53))
        .collect::<Vec<_>>()
        .join("\n");
    let mut outputs = Vec::new();
    for _ in 0..3 {
        let assert = cmd()?
            .args(["group-by", "k", "--agg", "COUNT(*) AS n", "--to", "csv"])
            .write_stdin(rows.clone())
            .assert()
            .success();
        outputs.push(String::from_utf8(assert.get_output().stdout.clone())?);
    }
    assert!(outputs.windows(2).all(|pair| pair[0] == pair[1]));
    assert!(outputs[0].starts_with("k,n\nk0,"));
    assert_eq!(
        json_for(
            r#"[{"k":"b","v":1},{"k":"a","v":2},{"k":"b","v":3}]"#,
            &["group-by", "k", "--agg", "SUM(v) AS s"]
        )?,
        json!([{"k": "b", "s": 4}, {"k": "a", "s": 2}])
    );
    Ok(())
}

#[test]
fn cli_sort_by_is_stable_and_places_nulls_on_request() -> Result<()> {
    let data = r#"[{"k":2,"id":"a"},{"k":null,"id":"b"},{"k":1,"id":"c"},{"k":2,"id":"d"},{"k":1,"id":"e"}]"#;
    let ids = |value: Value| {
        value
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["id"].as_str().unwrap().to_owned())
            .collect::<String>()
    };
    assert_eq!(ids(json_for(data, &["sort-by", "k"])?), "ceadb");
    assert_eq!(ids(json_for(data, &["sort-by", "k:desc"])?), "badce");
    assert_eq!(
        ids(json_for(data, &["sort-by", "k", "-r", "--nulls-last"])?),
        "adceb"
    );
    assert_eq!(ids(json_for(data, &["sort-by", "k * -1"])?), "adceb");
    Ok(())
}

#[test]
fn cli_rename_accepts_nushell_style_old_new_pairs() -> Result<()> {
    assert_eq!(
        json_for(r#"[{"a":1,"b":2}]"#, &["rename", "a", "x", "b", "y"])?,
        json!([{"x": 1, "y": 2}])
    );
    assert!(error_for(r#"[{"a":1,"b":2}]"#, &["rename", "a=x", "b"])?.contains("not a mix"));
    Ok(())
}

#[test]
fn cli_unique_accepts_nushell_flags_and_selectors() -> Result<()> {
    let data = r#"[{"k":1,"v":"a"},{"k":1,"v":"b"},{"k":2,"v":"c"}]"#;
    assert_eq!(
        json_for(data, &["drop-duplicates", "--subset", "k", "--last"])?,
        json!([{"k": 1, "v": "b"}, {"k": 2, "v": "c"}])
    );
    assert_eq!(
        json_for(data, &["unique", "type:integer"])?,
        json!([{"k": 1, "v": "a"}, {"k": 2, "v": "c"}])
    );
    Ok(())
}

#[test]
fn cli_with_row_index_adds_a_leading_index() -> Result<()> {
    assert_eq!(
        json_for(
            r#"[{"a":"x"},{"a":"y"}]"#,
            &["with-row-index", "--name", "i", "--offset", "1"]
        )?,
        json!([{"i": 1, "a": "x"}, {"i": 2, "a": "y"}])
    );
    Ok(())
}

#[test]
fn cli_new_commands_propagate_empty_pipelines() -> Result<()> {
    for args in [
        vec!["uppercase"],
        vec!["sum"],
        vec!["value-counts"],
        vec!["pivot", "--on", "a", "--index", "b"],
        vec!["explode"],
        vec!["cast", "i64"],
        vec!["is-duplicated"],
        vec!["get-year"],
        vec!["fill-null", "0"],
        vec!["join-where", r#"[{"a":1}]"#, "a > 0"],
    ] {
        cmd()?
            .args(&args)
            .write_stdin("")
            .assert()
            .success()
            .stdout(predicate::eq(""));
    }
    Ok(())
}

#[test]
fn cli_renders_the_polars_repr() -> Result<()> {
    cmd()?
        .args(["--data", r#"[{"a":1}]"#, "to", "repr"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("shape: (1, 1)\n"))
        .stdout(predicate::str::contains("i64"));
    Ok(())
}

#[test]
fn cli_casts_text_to_temporal_types_and_selects_parameterized_dtypes() -> Result<()> {
    let typed = cmd()?
        .args([
            "cast",
            "datetime<ms, UTC>",
            "at",
            "--data",
            r#"[{"at":"2024-01-01T05:00:00+02:00","n":1.5}]"#,
            "--to",
            "arrow",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    cmd()?
        .args(["select", "type:datetime<ms, UTC>", "--to", "csv"])
        .write_stdin(typed)
        .assert()
        .success()
        .stdout(predicate::eq("at\n2024-01-01T03:00:00.000+0000\n"));
    assert_eq!(
        json_for(
            r#"[{"d":"2024-01-01"},{"d":"bad"}]"#,
            &["cast", "date", "d", "--non-strict"]
        )?,
        json!([{"d": "2024-01-01"}, {"d": null}])
    );
    assert!(error_for(r#"[{"d":"bad"}]"#, &["cast", "date", "d"])?.contains("cast failed"));
    Ok(())
}

#[test]
fn cli_column_commands_accept_all_null_json_columns() -> Result<()> {
    let data = r#"[{"x":null,"n":1}]"#;
    for args in [
        vec!["is-in", "a,b", "x"],
        vec!["replace", "a", "b", "x"],
        vec!["get-year", "x"],
        vec!["strftime", "%Y", "x"],
        vec!["uppercase", "x"],
        vec!["math", "abs", "x"],
        vec!["cumulative", "sum", "x"],
        vec!["rolling", "sum", "1", "x"],
    ] {
        assert_eq!(
            json_for(data, &args)?,
            json!([{"x": null, "n": 1}]),
            "{args:?}"
        );
    }
    Ok(())
}

#[test]
fn cli_join_matches_null_keys_and_coalesces_key_pairs_on_request() -> Result<()> {
    let left = r#"[{"k":"0","v":"a"},{"k":null,"v":"b"}]"#;
    let right = r#"[{"k":null,"w":"x"},{"k":"0","w":"y"}]"#;
    assert_eq!(
        json_for(left, &["join", right, "--on", "k"])?,
        json!([{"k": "0", "v": "a", "w": "y"}])
    );
    assert_eq!(
        json_for(left, &["join", right, "--on", "k", "--nulls-equal"])?,
        json!([
            {"k": "0", "v": "a", "w": "y"},
            {"k": null, "v": "b", "w": "x"}
        ])
    );
    assert_eq!(
        json_for(
            r#"[{"k":1},{"k":2}]"#,
            &[
                "join",
                r#"[{"id":2,"w":"x"},{"id":4,"w":"y"}]"#,
                "--left-on",
                "k",
                "--right-on",
                "id",
                "--how",
                "full",
                "--coalesce-columns",
            ]
        )?,
        json!([{"k": 1, "w": null}, {"k": 2, "w": "x"}, {"k": 4, "w": "y"}])
    );
    Ok(())
}

#[test]
fn cli_join_output_follows_left_rows_then_right_rows() -> Result<()> {
    let right = r#"[{"id":3,"w":"x"},{"id":1,"w":"y"},{"id":1,"w":"z"}]"#;
    assert_eq!(
        json_for(
            r#"[{"id":1},{"id":2}]"#,
            &["join", right, "--on", "id", "--how", "full"]
        )?,
        json!([
            {"id": 1, "w": "y"},
            {"id": 1, "w": "z"},
            {"id": 2, "w": null},
            {"id": 3, "w": "x"}
        ])
    );
    assert_eq!(
        json_for(
            r#"[{"id":1},{"id":2}]"#,
            &["join", right, "--on", "id", "--how", "right"]
        )?,
        json!([
            {"id": 3, "w": "x"},
            {"id": 1, "w": "y"},
            {"id": 1, "w": "z"}
        ])
    );
    Ok(())
}

#[test]
fn cli_sort_by_treats_bare_integers_as_column_positions() -> Result<()> {
    assert_eq!(
        json_for(r#"[{"a":"x","b":2},{"a":"y","b":1}]"#, &["sort-by", "2"])?,
        json!([{"a": "y", "b": 1}, {"a": "x", "b": 2}])
    );
    assert!(error_for(r#"[{"a":1}]"#, &["sort-by", "3"])?.contains("position 3 is out of range"));
    Ok(())
}

#[test]
fn cli_selectors_matching_nothing_do_not_fall_back_to_every_column() -> Result<()> {
    let data = r#"[{"a":1,"b":null},{"a":1,"b":null}]"#;
    assert_eq!(
        json_for(data, &["drop-nulls", "^nothing$"])?
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(error_for(data, &["unique", "type:boolean"])?.contains("matched no columns"));
    Ok(())
}

#[test]
fn cli_selects_columns_whose_names_look_like_patterns() -> Result<()> {
    let dir = TempDir::new()?;
    let path = write_fixture(&dir, "weird.csv", "a,^a$,*\n1,2,3\n")?;
    cmd()?
        .arg(&path)
        .args(["select", "^a$", "*", "a", "--to", "csv"])
        .assert()
        .success()
        .stdout(predicate::eq("^a$,*,a\n2,3,1\n"));
    Ok(())
}

#[test]
fn cli_list_options_keep_selector_tokens_whole() -> Result<()> {
    let data = r#"[{"k":1,"v":"a","w":true},{"k":1,"v":"a","w":false}]"#;
    assert_eq!(
        json_for(data, &["unique", "--subset", "index:0,1"])?,
        json!([{"k": 1, "v": "a", "w": true}])
    );
    assert_eq!(
        json_for(
            r#"[{"id":1,"a":2,"b":"x"}]"#,
            &["unpivot", "--index", "id,b"]
        )?,
        json!([{"id": 1, "b": "x", "variable": "a", "value": 2}])
    );
    Ok(())
}

#[test]
fn cli_handles_extreme_numeric_arguments_without_panicking() -> Result<()> {
    let data = r#"[{"s":"abc","n":1},{"s":"de","n":2}]"#;
    assert_eq!(
        json_for(data, &["shift", "-9223372036854775808", "n"])?,
        json!([{"s": "abc", "n": null}, {"s": "de", "n": null}])
    );
    assert_eq!(
        json_for(data, &["str-slice", "-9223372036854775808", "s"])?,
        json!([{"s": "abc", "n": 1}, {"s": "de", "n": 2}])
    );
    assert_eq!(
        json_for(data, &["arg-sort", "n", "--limit", "4294967296"])?,
        json!([{"arg_sort": 0}, {"arg_sort": 1}])
    );
    assert!(error_for(data, &["sample", "--fraction", "NaN"])?.contains("between 0 and 1"));
    assert!(error_for(data, &["qcut", "1000000000", "n"])?.contains("at most"));
    Ok(())
}

#[test]
fn cli_reads_text_that_starts_like_a_binary_format_as_text() -> Result<()> {
    for input in ["PAR1\n", "ARROW1,x\n1,2\n", "Obj\u{1}abc\n"] {
        cmd()?
            .args(["count", "--to", "csv"])
            .write_stdin(input)
            .assert()
            .success()
            .stdout(predicate::eq("count\n1\n"));
    }
    Ok(())
}

#[test]
fn cli_reports_corrupt_binary_input_as_an_error_not_a_panic() -> Result<()> {
    let parquet = cmd()?
        .args([
            "--data",
            r#"[{"a":1,"b":"x"},{"a":2,"b":"y"}]"#,
            "--to",
            "parquet",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let dir = TempDir::new()?;
    for (index, cut) in [8usize, parquet.len() / 2, parquet.len() - 6]
        .into_iter()
        .enumerate()
    {
        let mut corrupt = parquet.clone();
        corrupt[cut] ^= 0xff;
        corrupt.truncate(corrupt.len().max(cut + 1));
        let path = dir.path().join(format!("corrupt{index}.parquet"));
        std::fs::write(&path, &corrupt)?;
        let output = cmd()?
            .arg("open")
            .arg(&path)
            .env("RUST_BACKTRACE", "0")
            .output()?;
        assert!(matches!(output.status.code(), Some(0 | 1)), "{output:?}");
        assert!(!String::from_utf8_lossy(&output.stderr).contains("panicked"));
    }
    Ok(())
}
