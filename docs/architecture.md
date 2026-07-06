# Architecture

`tokctl` is a local-only Rust CLI (edition 2021) that reads Claude/Codex JSONL
session logs and prints token/cost reports. Persistence is a SQLite cache
(`rusqlite`, bundled feature): default runs use incremental ingest + SQL
reports; `--no-cache` uses the in-memory path without touching the DB.
`tokctl ui` launches an interactive ratatui dashboard over the same cache.

## Module map

### Entry and CLI

- `src/main.rs` — thin entry; delegates to `cli::main_exit`
- `src/lib.rs` — library crate (also consumed by `benches/`)
- `src/cli.rs` — `clap` parser, routing, `--rebuild` / `--no-cache` / `ui` dispatch
- `src/cli/pipeline.rs` — shared roots → ingest/gather → cached vs `--no-cache` prepared runs
- `src/cli/workflows.rs` — command handlers (report, compare, repo, cursor)

### Shared types and discovery

- `src/types.rs` + `src/dates.rs` — shared types (source kind, usage, options) and `--since` / window parsing
- `src/discovery.rs` + `src/paths.rs` — filesystem scanning + root resolution
- `src/repo.rs` — git-aware repo identity resolution (repo key, display name, remote) for per-repo rollups

### Sources

- `src/sources/` — Claude and Codex JSONL parsers (typed `serde` deserialization)

### Ingest

- `src/ingest/` — ingest plan, byte-range reads (mmap for ≥ 1 MB), parallel runner.
  Parse in parallel, write serially, because rusqlite connections are not
  `Sync`. The split is in `src/ingest/run.rs`. mmap is only safe because the
  ingest plan's
  safety window routes recently-modified files to full-parse; that invariant
  lives in `src/ingest/file_range.rs::map_file`.

### Store and reports

- `src/store/` — SQLite schema, writes, queries
- `src/reports/in_memory.rs` — aggregations for `--no-cache`
- `src/render.rs` — table + JSON output
- `src/pricing.rs` — static model price table

### TUI

- `src/tui/` — ratatui dashboard:
  - `mod.rs` — event loop
  - `input.rs` — drill / yank / clamp
  - `data.rs` → `rows.rs` / `cache.rs` / `load.rs`
  - `state/` — `types`, `apply`, `refresh`, `persist`
  - `view/` — `core`, `layout`, `chrome`, `tables`
  - `widgets/filter.rs`, `keys.rs`, `format.rs`, `theme.rs`

### Doctor and test support

- `src/doctor/` — `mod.rs` types + `run()`, `checks.rs`, `render.rs`
- `src/test_support.rs` — shared SQL/in-memory parity fixtures (`test-fixtures` feature)
- `benches/parse.rs` — criterion microbenchmarks for the parser hot path
