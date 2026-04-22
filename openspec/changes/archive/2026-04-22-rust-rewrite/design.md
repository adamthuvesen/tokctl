## Context

`tokctl` is a local-only CLI (~2,260 lines of TypeScript) that reads Claude/Codex JSONL session logs and prints token/cost reports. It has three runtime dependencies (`commander`, `better-sqlite3`, `cli-table3`) and requires Node ≥20. The rewrite preserves all user-visible behaviour and the SQLite schema while replacing the runtime with a self-contained native binary.

The existing logic is well-factored with clear module boundaries: file discovery, JSONL parsing, incremental ingest planning, SQLite persistence, and report rendering are already separated. This maps cleanly to Rust modules.

## Goals / Non-Goals

**Goals:**
- Produce a single statically-linked binary with no runtime dependency
- Preserve CLI flag names, report formats, SQLite schema, and all observable behaviour exactly
- Replace the TypeScript test suite with equivalent `cargo test` coverage, reusing existing JSONL fixtures
- Ship idiomatic Rust: `Result`-based error handling, no `unwrap` in library code, `#[derive(Debug, Clone)]` on data types

**Non-Goals:**
- Adding new features or changing report formats
- Async I/O (the workload is not concurrency-bound; blocking I/O is simpler and faster for this use case)
- Cross-compilation or packaging automation (out of scope for this change)
- Removing the `--no-cache` path or merging it into the cached path

## Decisions

### 1. Single binary crate, not a workspace

A workspace with multiple crates (e.g. `tokctl-core`, `tokctl-cli`) adds indirection with no benefit at this scale. A single crate with `mod` boundaries matches the existing `src/` structure and keeps the build simple.

**Alternatives considered**: `cargo workspace` — rejected because the tool has a single consumer and no library API to publish.

### 2. `rusqlite` with `bundled` feature

`rusqlite` compiles SQLite from source and links it statically. This eliminates the system SQLite version dependency and guarantees the schema behaves identically on all platforms.

**Alternatives considered**: `sqlx` — rejected because async is unnecessary here and the compile-time query checking adds friction without meaningful safety benefit for this query set.

### 3. `serde_json::Value` for JSONL parsing, not typed structs

Claude and Codex JSONL lines contain deeply nested JSON with many optional and unknown fields. Deserializing into `Value` and then navigating with `.get()` chains mirrors the existing TypeScript approach and tolerates format drift without breaking. Inner fields that are always present (e.g. `sessionId`, `timestamp`) are extracted after the initial parse.

**Alternatives considered**: Fully-typed `#[derive(Deserialize)]` structs with `Option` wrappers everywhere — viable but brittle; any upstream format change is a hard error rather than a graceful skip.

### 4. `anyhow` for error handling

The CLI surface area is `main() → Result<(), anyhow::Error>`. Errors are displayed to the user and the process exits non-zero. `anyhow` handles context chaining without ceremony. Internal modules that produce typed errors use `thiserror` where the caller needs to branch on the error kind (e.g. "file not found" vs "parse error").

**Alternatives considered**: `Box<dyn Error>` — too unergonomic for context chaining. `eyre` — equivalent to `anyhow` but less common; no benefit here.

### 5. `clap` with derive API

The derive API (`#[derive(Parser)]`) produces flag and subcommand definitions that live next to their handler structs. This makes it easy to verify flag parity with the TypeScript `commander` setup.

`--no-cache` remains a flag (not a subcommand). `clap`'s `action = ArgAction::SetFalse` on a `cache: bool` field replicates the `--no-cache` → `cache = false` semantic.

### 6. `comfy-table` for table rendering

`comfy-table` supports Unicode box-drawing characters, column alignment, and a builder API similar to `cli-table3`. The Rust output will be visually identical to the TypeScript output.

### 7. `mtime_ns` stored as `INTEGER` (i64) in SQLite

SQLite's `INTEGER` type maps to `i64` in `rusqlite`. Nanosecond epoch timestamps fit in i64 until year 2262. The existing schema already stores this as `INTEGER`; no migration is needed.

### 8. No async runtime

The ingest loop reads files sequentially (tailing or full-parse), inserts into SQLite, and exits. There is no benefit to spawning async tasks here — the bottleneck is disk I/O, which is already handled efficiently by byte-range reads via `std::io::Seek`. Adding `tokio` would increase compile time and binary size for zero throughput gain.

### 9. Pricing table as a static array

Model pricing is a `&'static [(& str, PriceEntry)]` with a linear scan (or a `phf` compile-time hash map if lookup becomes a hot path). Model IDs are normalized by stripping the `-YYYYMMDD` date suffix before lookup, matching the existing `normalizeModelId` behaviour.

## Risks / Trade-offs

- **serde_json::Value navigation is verbose** → Mitigated by extracting helper functions (`get_str`, `get_u64`) that consolidate the `.get()` / `.as_*()` chains and return `Option<T>`.
- **SQLite manifest stores `mtime_ns` as nanoseconds but `std::fs::Metadata::modified()` returns a `SystemTime`** → Convert via `duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos() as i64`. Add a comment explaining the conversion.
- **Fixture tests for JSONL parsing require reading test files at known paths** → Use `include_str!` or `std::fs::read_to_string` with `env!("CARGO_MANIFEST_DIR")` to resolve fixture paths portably.
- **`--no-cache` path produces identical output to the cached path** → This invariant is currently untested end-to-end. Add an integration test that runs both paths on the same fixtures and asserts equal output.
- **Binary size** → `rusqlite` bundled + `serde_json` + `clap` will produce a ~5–8 MB release binary. Acceptable for a developer tool; no special optimization needed.

## Migration Plan

1. Remove `src/`, `dist/`, `package.json`, `tsconfig.json`, `node_modules/` once the Rust crate passes all tests.
2. Add `Cargo.toml` and `src/` (Rust) at the repo root.
3. Update `README.md` install instructions: replace `npm install -g tokctl` with `cargo install tokctl` and binary download links.
4. The SQLite cache path (`~/.local/share/tokctl/cache.db` or the resolved `TOKCTL_CACHE_DIR`) is unchanged — existing caches continue to work.
5. No data migration is required: the schema version is unchanged.

## Open Questions

- Should the binary be published to `crates.io`? If so, does the name `tokctl` need reserving? (Out of scope for this change; decide before shipping.)
- Should CI build release binaries for `x86_64-unknown-linux-musl` and `aarch64-apple-darwin`? (Nice to have; add to tasks when decided.)
