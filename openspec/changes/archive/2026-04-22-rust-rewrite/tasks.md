## 1. Project Bootstrap

- [x] 1.1 Create `Cargo.toml` at the repo root with correct crate name, edition 2021, and initial dependencies (`clap`, `rusqlite` bundled, `serde`, `serde_json`, `chrono`, `anyhow`, `thiserror`, `comfy-table`, `walkdir`)
- [x] 1.2 Create `src/main.rs` with a minimal `fn main() -> anyhow::Result<()>` that compiles cleanly
- [x] 1.3 Add `[profile.release]` config: `lto = true`, `codegen-units = 1`, `strip = true`
- [x] 1.4 Add `.cargo/config.toml` if needed for target-specific linker flags
- [x] 1.5 Verify `cargo build` succeeds and `cargo test` runs (zero tests pass initially)

## 2. Core Types

- [x] 2.1 Define `UsageEvent` struct in `src/types.rs` with all fields from the spec (source, timestamp, session_id, project_path, model, token counts)
- [x] 2.2 Define `Source` enum (`Claude`, `Codex`) with `Display` and serde impls
- [x] 2.3 Define `IngestStats` struct in `src/types.rs`
- [x] 2.4 Define `AggregateRow` struct in `src/types.rs` with all fields from the render spec
- [x] 2.5 Add `#[derive(Debug, Clone)]` to all data types

## 3. Pricing Module

- [x] 3.1 Create `src/pricing.rs` with the static pricing table as a `&[(& str, PriceEntry)]` array
- [x] 3.2 Implement `normalize_model_id` that strips `-YYYYMMDD` date suffixes
- [x] 3.3 Implement `cost_of(event: &UsageEvent, unknown: &mut HashSet<String>) -> f64`
- [x] 3.4 Implement `has_price(model: &str) -> bool`
- [x] 3.5 Write unit tests: known model cost, cache token cost, unknown model returns 0, date suffix stripped

## 4. JSONL Parsers

- [x] 4.1 Create `src/sources/mod.rs` with public re-exports
- [x] 4.2 Implement `claude_line_has_signal(line: &str) -> bool` fast pre-filter
- [x] 4.3 Implement `parse_claude_line(line: &str, project_path: Option<&str>) -> Option<(UsageEvent, Option<String>)>` (returns event + message_id)
- [x] 4.4 Implement `codex_line_has_signal(line: &str) -> bool` fast pre-filter
- [x] 4.5 Implement `parse_codex_line(line: &str) -> Option<UsageEvent>`
- [x] 4.6 Write unit tests using the existing JSONL fixtures in `tests/fixtures/`; confirm event field values, zero-token skip, malformed skip, unknown fields tolerated

## 5. File Discovery

- [x] 5.1 Create `src/paths.rs` with `resolve_claude_roots` and `resolve_codex_roots` (respecting flags → env vars → defaults priority chain)
- [x] 5.2 Create `src/discovery.rs` with `DiscoveredFile` struct and `Discovery` struct
- [x] 5.3 Implement `discover_claude(roots: &[PathBuf]) -> Discovery` — walks two levels deep, collects `.jsonl` files with size + mtime_ns
- [x] 5.4 Implement `discover_codex(roots: &[PathBuf]) -> Discovery` — walks any depth under root
- [x] 5.5 Implement `mtime_ns(metadata: &Metadata) -> i64` helper converting `SystemTime` to nanoseconds
- [x] 5.6 Ensure missing/unreadable root directories are silently skipped
- [x] 5.7 Write unit tests using temp directories

## 6. Ingest Plan

- [x] 6.1 Create `src/ingest/plan.rs` with `IngestPlan` struct (toSkip, toTail, toFullParse, toPurge)
- [x] 6.2 Implement `plan_ingest(manifest, discovery, safety_window_ms, now) -> IngestPlan`
- [x] 6.3 Handle all five categorisation cases: new file, unchanged, grown+old, shrunk, disappeared
- [x] 6.4 Implement safety window logic (files modified within window → toFullParse)
- [x] 6.5 Implement directory-level skip optimisation using `unchangedPaths`
- [x] 6.6 Write unit tests covering all categorisation branches and the safety window boundary

## 7. SQLite Cache Store

- [x] 7.1 Create `src/store/mod.rs`, `src/store/schema.rs`, `src/store/db.rs`
- [x] 7.2 Define DDL constants matching the exact schema from the spec
- [x] 7.3 Implement `open_store(path: &Path) -> anyhow::Result<Connection>` — creates parent dirs, runs DDL, checks schema version
- [x] 7.4 Implement `load_file_manifest(conn: &Connection) -> HashMap<String, FileManifestRow>`
- [x] 7.5 Create `src/store/writes.rs` — implement `insert_events`, `upsert_file_manifest`, `delete_file_and_events` (all using transactions)
- [x] 7.6 Create `src/store/queries.rs` — implement `daily_report_from_db`, `monthly_report_from_db`, `session_report_from_db` with source + time filters
- [x] 7.7 Implement schema version check: write version on create, error if db version > current
- [x] 7.8 Write unit tests using in-memory SQLite (`Connection::open_in_memory()`)

## 8. File Range Ingest

- [x] 8.1 Create `src/ingest/file_range.rs`
- [x] 8.2 Implement `ingest_claude_range(file_path, project_path, from_offset, to_offset) -> RangeResult` — seeks to offset, reads lines, returns events + skipped_lines count
- [x] 8.3 Implement `ingest_codex_range(file_path, from_offset, to_offset) -> RangeResult`
- [x] 8.4 Write unit tests reading byte ranges from fixture files

## 9. Ingest Runner

- [x] 9.1 Create `src/ingest/run.rs` with `RunIngestOptions` and `run_ingest(opts) -> anyhow::Result<IngestStats>`
- [x] 9.2 Implement the full execution loop: purge → full-parse → tail, each wrapped in a per-file transaction
- [x] 9.3 Implement `local_day(ts: i64) -> String` and `local_month(ts: i64) -> String` using local timezone
- [x] 9.4 Implement `event_to_row` conversion including cost calculation
- [x] 9.5 Accumulate `IngestStats` across all processed files
- [x] 9.6 Write integration tests: ingest a fixture directory, assert event counts and manifest state

## 10. In-Memory Reports

- [x] 10.1 Create `src/legacy/in_memory.rs`
- [x] 10.2 Implement `filter_by_date(events, since, until) -> Vec<UsageEvent>`
- [x] 10.3 Implement `daily_in_memory(events, source_label, unknown) -> Vec<AggregateRow>` — aggregate by local day, sort ascending
- [x] 10.4 Implement `monthly_in_memory(events, source_label, unknown) -> Vec<AggregateRow>` — aggregate by local month, sort ascending
- [x] 10.5 Implement `session_in_memory(events, unknown) -> Vec<AggregateRow>` — aggregate by (source, session_id), sort descending by latest_timestamp
- [x] 10.6 Write unit tests including boundary-date filtering and parity check against SQL queries

## 11. Render Module

- [x] 11.1 Create `src/render.rs` with `ReportKind` enum and `RenderOptions` struct
- [x] 11.2 Implement `render_table(rows, kind, show_source) -> String` using `comfy-table`
- [x] 11.3 Add TOTAL row logic when rows is non-empty
- [x] 11.4 Implement `render_json(rows, kind, show_source) -> String` with correct per-kind JSON shape
- [x] 11.5 Implement `render_warnings(unknown_models, skipped_lines) -> Vec<String>`
- [x] 11.6 Implement `fmt_num` (thousands-separated) and `fmt_cost` (`$N.NN`) helpers
- [x] 11.7 Write unit tests for JSON shape, cost rounding (4 decimal places), TOTAL row values, empty-rows edge case

## 12. CLI Wiring

- [x] 12.1 Create `src/cli.rs` with `#[derive(Parser)]` structs for all flags and subcommands
- [x] 12.2 Wire `--source`, `--since`, `--until`, `--json`, `--claude-dir`, `--codex-dir`, `--rebuild`, `--no-cache`
- [x] 12.3 Implement `parse_since` / `parse_until` date parsing (ISO-8601 and relative formats like `7d`, `30d`)
- [x] 12.4 Implement the cached path: discover → plan → ingest → query → render
- [x] 12.5 Implement the `--no-cache` path: discover → full-parse all → in-memory aggregate → render
- [x] 12.6 Implement `--rebuild`: call `open_store` after dropping the existing DB file, then normal cached path
- [x] 12.7 Print warnings to stderr after the report (not mixed with `--json` stdout)
- [x] 12.8 Write end-to-end integration tests: run the binary against fixture directories and assert stdout contains expected rows

## 13. Cleanup and Migration

- [x] 13.1 Remove `src/` (TypeScript), `dist/`, `package.json`, `package-lock.json`, `tsconfig.json`, `node_modules/` (confirm nothing else depends on them first)
- [x] 13.2 Update `README.md` install instructions to replace npm with `cargo install` and binary download
- [x] 13.3 Run `cargo clippy -- -D warnings` and fix all warnings
- [x] 13.4 Run `cargo fmt --check` and ensure code is formatted
- [x] 13.5 Verify existing JSONL fixtures still parse correctly against the Rust parsers
- [x] 13.6 Do a final `cargo test` — all tests must pass with zero failures
