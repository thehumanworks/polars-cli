# ADR 0001: Trailing newline on rendered output

## Status

Accepted

## Context

Interactive `pl` output often left the next shell prompt on the last payload
line. The proposed fix was:

1. always write a trailing newline on stdout, as Unix tools normally do;
2. trim leading and trailing whitespace on piped input so chaining would still
   work after that terminator was added.

The diagnosis that "newlines are omitted so chaining works" is only partly true.
The default pipe format is already JSONL, and CSV/TSV/`--raw` already terminate
the last record with `\n`. The formats that omitted a final newline were mainly
JSON, pretty JSON, table, markdown, HTML, XML, and HTML-to-markdown.

A global `trim()` on input would make a trailing newline harmless, but it would
also drop real data:

- a leading blank line in `--from text` is an empty first row;
- trailing spaces on the last `--raw` / text value live after the last
  non-space character and would be stripped;
- YAML indentation is significant if a document is ever passed with a leading
  blank that a caller expected to keep as part of a literal block.

Existing parsers already tolerate a single trailing newline: JSON/YAML/TOML
ignore surrounding whitespace, JSONL skips empty lines, and `str::lines()` does
not invent an extra text row from one final `\n`.

## Decision

Normalize at emission time only.

- `write_output` appends `\n` when the rendered payload is non-empty and does
  not already end with `\n`.
- The same rule applies to `--output` files (`-o -` is stdout).
- Empty output stays empty so a zero-row JSONL wire remains schema-less.
- Input loaders do not strip start/end whitespace as a chaining strategy.
- Format renderers keep producing payload text; stream termination is a CLI
  emission concern, not a table-renderer concern.

## Consequences

- Interactive table/JSON/markdown/HTML/XML output no longer glues the prompt to
  the last line.
- `pl | pl` pipelines keep working without a second blank JSONL/text record.
- Callers that compared stdout bytes without a final newline (tests, snapshots)
  need to expect the terminator.
- Completions still bypass `write_output` and are unchanged.
