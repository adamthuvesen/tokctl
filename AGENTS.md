# AGENTS.md — tokctl

`tokctl` is a local-only Rust CLI (edition 2021) that reads Claude/Codex JSONL
session logs and prints token/cost reports, backed by a SQLite cache with an
optional `--no-cache` in-memory path and a `tokctl ui` ratatui dashboard.

User-level guidance (tone, principles, git etiquette) lives in `~/.claude/CLAUDE.md`
and `~/dotfiles/agents/AGENTS.md` and is *not* duplicated here. This file is for
project-specific facts.

## Layout

```
src/
├── cli/        clap parser, routing, command handlers, pipeline
├── sources/    Claude + Codex JSONL parsers
├── ingest/     ingest plan, byte-range reads, parallel runner
├── store/      SQLite schema, writes, queries
├── tui/        ratatui dashboard
└── reports/    in-memory aggregations for --no-cache
```

Full module-by-module map is in [docs/architecture.md](docs/architecture.md).

## Quickstart

```sh
cargo build           # debug build
cargo build --release # optimized binary at target/release/tokctl
cargo test            # unit + integration tests
cargo clippy          # lint (CI runs with -D warnings)
cargo fmt             # format
cargo bench           # parser benches
```

## Critical Conventions

Non-obvious rules; verify against the code before relying on them.

- **Parse in parallel, write serially.** rusqlite connections are not `Sync`;
  the split is in [src/ingest/run.rs](src/ingest/run.rs).
- **mmap safety window.** mmap (for files ≥ 1 MB) is only safe because the
  ingest plan routes recently-modified files to full-parse. Changing that
  invariant means auditing [src/ingest/file_range.rs](src/ingest/file_range.rs)
  (`map_file`).
- **Smallest diff that satisfies the requirement.** Match existing patterns in `src/`.
- **No network.** The cache path is local only; never commit secrets, `.env`, or AI-attribution lines.

## Read The Docs First

Before editing a subsystem, read the matching doc:

- **Architecture / module map** → [docs/architecture.md](docs/architecture.md)
- **Ingest / mmap invariant** → [docs/architecture.md#ingest](docs/architecture.md#ingest)
- **TUI dashboard** → [docs/architecture.md#tui](docs/architecture.md#tui)

If a doc disagrees with code, fix the doc in the same change.

## Index

Start in [docs/architecture.md](docs/architecture.md).
