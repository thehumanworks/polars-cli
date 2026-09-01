use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use polars::frame::DataFrame;
use polars::prelude::{IntoLazy, LazyFrame};
use polars::sql::SQLContext;

use crate::read::{detect_input_format, read_file};

pub struct SqlEngine {
    ctx: SQLContext,
    registered: Vec<String>,
}

impl SqlEngine {
    pub fn new() -> Self {
        Self {
            ctx: SQLContext::new(),
            registered: Vec::new(),
        }
    }

    pub fn register_frame(&mut self, name: &str, frame: LazyFrame) {
        self.ctx.register(name, frame);
        if !self.registered.iter().any(|existing| existing == name) {
            self.registered.push(name.to_owned());
        }
    }

    pub fn register_path(
        &mut self,
        path: impl AsRef<Path>,
        alias: Option<&str>,
    ) -> anyhow::Result<String> {
        let path = path.as_ref();
        if path.is_dir() {
            bail!(
                "{} is a directory; use register_directory to register its contents",
                path.display()
            );
        }

        let df = read_file(path, None)
            .with_context(|| format!("failed to read table input {}", path.display()))?;

        let name = match alias {
            Some(alias) => alias.to_owned(),
            None => table_name_from_path(path)?,
        };

        self.register_frame(&name, df.lazy());
        Ok(name)
    }

    pub fn register_directory(&mut self, path: impl AsRef<Path>) -> anyhow::Result<Vec<String>> {
        let path = path.as_ref();
        if !path.is_dir() {
            bail!("{} is not a directory", path.display());
        }

        let mut entries: Vec<PathBuf> = fs::read_dir(path)
            .with_context(|| format!("failed to read directory {}", path.display()))?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|entry| entry.is_file())
            .filter(|entry| !is_hidden(entry))
            .filter(|entry| {
                detect_input_format(entry)
                    .is_ok_and(|format| format != crate::read::InputFormat::Html)
            })
            .collect();
        entries.sort();

        let mut registered = Vec::new();
        for entry in entries {
            let name = self.register_path(&entry, None)?;
            registered.push(name);
        }
        Ok(registered)
    }

    pub fn registered_tables(&self) -> &[String] {
        &self.registered
    }

    pub fn execute_collect(&mut self, query: &str) -> anyhow::Result<DataFrame> {
        let lf = self
            .ctx
            .execute(query)
            .with_context(|| format!("failed to plan SQL query: {query}"))?;
        lf.collect()
            .with_context(|| format!("failed to execute SQL query: {query}"))
    }
}

impl Default for SqlEngine {
    fn default() -> Self {
        Self::new()
    }
}

fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.starts_with('.'))
        .unwrap_or(false)
}

fn table_name_from_path(path: &Path) -> anyhow::Result<String> {
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .with_context(|| format!("{} has no usable file name", path.display()))?;
    if stem.is_empty() {
        bail!("{} resolved to an empty table name", path.display());
    }
    Ok(sanitize_identifier(stem))
}

fn sanitize_identifier(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for (index, character) in raw.chars().enumerate() {
        let keep = if index == 0 {
            character.is_ascii_alphabetic() || character == '_'
        } else {
            character.is_ascii_alphanumeric() || character == '_'
        };
        if keep {
            out.push(character);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "table".to_owned()
    } else {
        out
    }
}
