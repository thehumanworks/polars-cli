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
        .stdout(predicate::eq("| name |\n| --- |\n| alice |"));

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
        .stdout(predicate::eq("[{\"name\":\"Ada\"}]"));
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
    assert_eq!(String::from_utf8(output.stdout)?, "[]");
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
        .stdout(predicate::eq("[{\"name\":\"Ada\"}]"));
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
