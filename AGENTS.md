# Agent instructions (tokctl)

## Project

`tokctl` is a **local-only Rust CLI** (edition 2021) that reads Claude/Codex JSONL session logs and prints token/cost reports. Persistence is a **SQLite** cache (`rusqlite`, bundled feature); default runs use incremental ingest + SQL reports; **`--no-cache`** uses the in-memory path without touching the DB.

## Layout

- `src/main.rs` — thin entry; delegates to `cli::main_exit`
- `src/lib.rs` — library crate (also consumed by `benches/`)
- `src/cli.rs` — `clap` parser, routing, `--rebuild` / `--no-cache` dispatch
- `src/sources/` — Claude and Codex JSONL parsers (typed `serde` deserialization)
- `src/discovery.rs` + `src/paths.rs` — filesystem scanning + root resolution
- `src/ingest/` — ingest plan, byte-range reads (mmap for ≥ 1 MB), parallel runner
- `src/store/` — SQLite schema, writes, queries
- `src/legacy/in_memory.rs` — aggregations for `--no-cache`
- `src/render.rs` — table + JSON output
- `src/pricing.rs` — static model price table
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
