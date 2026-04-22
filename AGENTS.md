# Agent instructions (tokctl)

## Project

`tokctl` is a **local-only Rust CLI** (edition 2021) that reads Claude/Codex JSONL session logs and prints token/cost reports. Persistence is a **SQLite** cache (`rusqlite`, bundled feature); default runs use incremental ingest + SQL reports; **`--no-cache`** uses the in-memory path without touching the DB. `tokctl ui` launches an interactive ratatui dashboard over the same cache.

Spec-driven changes live under `openspec/` — proposals and deltas in `openspec/changes/`, canonical capability specs in `openspec/specs/`. Keep specs in sync when capability behavior changes.

## Layout

- `src/main.rs` — thin entry; delegates to `cli::main_exit`
- `src/lib.rs` — library crate (also consumed by `benches/`)
- `src/cli.rs` — `clap` parser, routing, `--rebuild` / `--no-cache` / `ui` dispatch
- `src/types.rs` + `src/dates.rs` — shared types (source kind, usage, options) and `--since` / window parsing
- `src/sources/` — Claude and Codex JSONL parsers (typed `serde` deserialization)
- `src/discovery.rs` + `src/paths.rs` — filesystem scanning + root resolution
- `src/repo.rs` — git-aware repo identity resolution (repo key, display name, remote) for per-repo rollups
- `src/ingest/` — ingest plan, byte-range reads (mmap for ≥ 1 MB), parallel runner
- `src/store/` — SQLite schema, writes, queries
- `src/legacy/in_memory.rs` — aggregations for `--no-cache`
- `src/render.rs` — table + JSON output
- `src/pricing.rs` — static model price table
- `src/tui/` — ratatui dashboard: `mod.rs` event loop, `state.rs`, `data.rs` (read-only queries), `view.rs`, `keys.rs`, `format.rs`, `theme.rs`
- `benches/parse.rs` — criterion microbenchmarks for the parser hot path

## Commands

```sh
cargo build           # debug build
cargo build --release # optimized binary at target/release/tokctl
cargo test            # unit + integration tests
cargo clippy          # lint (CI runs with -D warnings)
cargo fmt             # format
cargo bench           # parser benches
```

## Change discipline

- Prefer the **smallest** diff that satisfies requirements; match existing patterns in `src/`.
- Do not add network calls or shipping of secrets; the cache path is local only.
- Parse in parallel, write serially — rusqlite connections are not `Sync`. The split is in `src/ingest/run.rs`.
- mmap is only safe because the ingest plan's safety window routes recently-modified files to full-parse. If you change that invariant, audit `src/ingest/file_range.rs::map_file`.

## Global policy

Where this file is silent, follow the repository maintainer's global **AGENTS.md** / Cursor rules for commits (no push without ask), security, and workflow.
