# Repository Guidelines

## Project Structure & Module Organization

`src/cli.rs` contains the CLI entrypoint logic and argument routing. `src/read.rs` owns input parsing and format inference. `src/transformer.rs` owns output rendering and document conversion helpers. `src/ops.rs` holds the SQL-backed frame operations; `src/ops/` holds column transforms (`column.rs`, `strings.rs`, `temporal.rs`), aggregations (`aggregate.rs`), and reshaping (`reshape.rs`). `src/selector.rs` resolves column selectors and `src/value.rs` parses dtypes and literal values. `src/bin/pl.rs` and `src/bin/polars.rs` are thin wrappers around the shared CLI runner in `src/lib.rs`.

`docs/nushell-parity.md` maps every Nushell `polars` command to `pl`; keep it current when commands change. `docs/adr/` records interface decisions.

Integration tests live in `tests/cli.rs` and cover the installed binary behavior. Library-level behavior and format conversions are covered in `tests/library.rs`. Sample inputs such as `test.csv` and `index.html` are useful local fixtures when checking manual conversions.

## Build, Test, and Development Commands

- `cargo run -- --help` runs the default `pl` binary locally.
- `cargo run -- --from csv --to json < test.csv` is a quick stdin smoke test.
- `cargo run --bin polars -- --version` verifies the alias binary.
- `cargo build` compiles all targets.
- `cargo test` runs unit, integration, and doc tests.
- `cargo clippy --all-targets --all-features -- -D warnings` enforces lint cleanliness.
- `cargo fmt --check` verifies formatting.
- `cargo install --path . --force --bin pl --bin polars` installs the two binaries into `~/.cargo/bin`.

Polars debug builds are large. Set `CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0` when disk space is tight.

## Coding Style & Naming Conventions

Use standard Rust formatting via `cargo fmt`; default rustfmt conventions are the style guide here. Keep modules focused by responsibility: parsing in `read.rs`, rendering in `transformer.rs`, orchestration in `cli.rs`. Use `snake_case` for functions/tests/modules and `PascalCase` for enums and types. Prefer `anyhow` for CLI-facing errors and `clap` derive attributes for argument parsing.

## Testing Guidelines

Add integration coverage for user-visible CLI behavior in `tests/cli.rs` and smaller behavior checks in `tests/library.rs`. Test names should describe observable behavior, for example `cli_converts_html_document_to_markdown` or `html_document_to_markdown_skips_common_non_content_tags`. Prefer exact stdout/stderr assertions, `TempDir` fixtures, and narrow commands such as `cargo test --test cli <name> -- --exact` during development.

## Commit & Pull Request Guidelines

Current history uses short, imperative, scope-first commit messages, e.g. `file conversions: md, toml, csv, json, html and xml`. Follow that pattern: concise, lowercase, and specific to the change. In pull requests, include:

- What changed and why
- Commands run for validation
- Any format-semantics changes, especially around `html` vs `table` behavior or binary naming (`pl`, `polars`)

## Binary Names & PATH Notes

This repo intentionally builds `pl` and `polars`. On macOS, `/usr/bin/pl` may already exist. Prefer `cargo run`, `cargo run --bin polars`, or ensure your PATH/shims point `pl` to this project’s installed binary.
