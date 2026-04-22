## Why

`tokctl` is currently a Node.js CLI requiring Node ≥20 at runtime, which creates a friction point for distribution and usage on machines where Node isn't installed or pinned to the right version. Rewriting in Rust produces a single self-contained binary with no runtime dependency, near-instant startup, and stronger compile-time correctness guarantees — while the codebase is still small enough (~2,300 lines) that a full rewrite is practical.

## What Changes

- **BREAKING**: Distribution changes from `npm install -g tokctl` to a pre-compiled binary (or `cargo install tokctl`). The Node/npm package is retired.
- All source files under `src/` are replaced with an equivalent Rust crate (`src/` or a `crates/tokctl/` layout).
- Build tooling switches from `tsc` / `tsx` / `vitest` to `cargo build` / `cargo test`.
- The SQLite schema, incremental ingest logic, JSONL parsers, pricing table, and CLI flags are preserved exactly — behaviour is unchanged from the user's perspective.
- The `--no-cache` / in-memory code path is retained (as a first-class path, not legacy).
- `package.json`, `tsconfig.json`, and all TypeScript source files are removed.

## Capabilities

### New Capabilities

- `cli`: Command-line interface — flags, subcommands, routing (replaces `src/cli.ts`). Covers `--source`, `--since`, `--until`, `--json`, `--rebuild`, `--no-cache`, `--claude-dir`, `--codex-dir` and the `daily` / `monthly` / `session` report subcommands.
- `file-discovery`: Filesystem traversal that locates Claude and Codex JSONL session files under configurable root directories (replaces `src/sources/claude.ts`, `src/sources/codex.ts`, `src/paths.ts`).
- `jsonl-parsing`: Line-by-line JSONL parsers for Claude and Codex session formats, extracting `UsageEvent` records (replaces `src/sources/claude-parse.ts`, `src/sources/codex-parse.ts`).
- `ingest-plan`: Incremental ingest planner that compares discovered files against the SQLite manifest to produce skip / tail / full-parse / purge decisions (replaces `src/ingest/plan.ts`, `src/ingest/fileRange.ts`).
- `ingest-run`: Ingest orchestration that executes the ingest plan and writes events + manifest rows to SQLite (replaces `src/ingest/run.ts`).
- `cache-store`: SQLite persistence layer — schema, writes, and report queries (replaces `src/store/`).
- `pricing`: Model-to-cost lookup table used to compute dollar amounts from token counts (replaces `src/pricing.ts`).
- `render`: Table and JSON output formatting for daily, monthly, and session reports (replaces `src/render.ts`).
- `in-memory-reports`: Aggregation path for `--no-cache` runs that computes reports directly from parsed events without touching SQLite (replaces `src/legacy/inMemory.ts`).

### Modified Capabilities

_(None — this is a full rewrite with no existing spec files to delta.)_

## Impact

- **Removed dependencies**: `better-sqlite3`, `cli-table3`, `commander`, `tsx`, `typescript`, `vitest`, all `@types/*` packages.
- **Added dependencies**: `clap` (CLI), `rusqlite` (SQLite, bundled feature), `comfy-table` (table rendering), `serde` + `serde_json` (JSONL parsing), `chrono` (date handling), `anyhow` (error handling), `walkdir` (directory traversal).
- **Build artifacts**: `dist/` directory and npm packaging replaced by a single native binary produced by `cargo build --release`.
- **Test infrastructure**: `vitest` test suite replaced by `cargo test` with Rust unit + integration tests. Existing JSONL fixtures in `tests/fixtures/` are reused as-is.
- **CI**: Any existing Node-based CI steps need updating to use the Rust toolchain.
