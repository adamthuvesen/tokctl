## Why

Cold ingest on a realistic corpus (2.1 GB, ~4,800 JSONL files) takes ~7 seconds in the release build — on par with the TypeScript version it replaced, which is disappointing for a Rust rewrite. The bottleneck is a sequential single-threaded loop that parses every line through `serde_json::Value` (dynamic JSON navigation) and reads each file via `BufReader::take`. On modern multi-core machines this leaves 7/8 cores idle. Three targeted optimisations can push cold ingest to roughly **1.5–2 seconds** without changing behaviour or output.

## What Changes

- Parse discovered files concurrently using `rayon`'s work-stealing thread pool (default: `num_cpus` threads), then serialise only the database writes. Expected ~3–4× speedup on cold ingest.
- Replace `serde_json::Value` navigation in both Claude and Codex parsers with `#[derive(Deserialize)]` structs that use `#[serde(default)]` and `Option<T>` for forward-compatibility. Expected ~1.5–2× parser speedup and lower allocations per line.
- For files above a size threshold (default: 1 MB), use `memmap2::Mmap` instead of `BufReader::take` to avoid a userspace copy. Marginal but free on large session files.
- Add a hidden `--threads <N>` flag to override the parallelism setting (useful for benchmarking and constrained environments).
- Add criterion-based benchmarks covering the parser hot path so future regressions are caught.

**Non-breaking:** All output (report data, JSON schema, warnings, exit codes) remains identical to the pre-change behaviour. The SQLite schema is unchanged.

## Capabilities

### New Capabilities

- `ingest-perf`: Performance-oriented ingest path — parallel file parsing, typed JSONL deserialization, memory-mapped reads for large files, and the `--threads` override.

### Modified Capabilities

_(None to delta — `ingest-run`, `jsonl-parsing`, `ingest-plan`, and `file-range` specs live under the still-active `rust-rewrite` change. This change adds a new capability that composes with them without changing their observable contracts.)_

## Impact

- **Added dependencies**: `rayon` (parallel iteration), `memmap2` (memory-mapped file reads), `criterion` (dev-only, benchmarks).
- **Source files touched**: `src/ingest/run.rs` (parallel loop), `src/ingest/file_range.rs` (mmap path), `src/sources/claude.rs` + `src/sources/codex.rs` (typed structs), `src/cli.rs` (`--threads` flag), new `benches/parse.rs`.
- **Binary size**: +~200 KB expected from `rayon` + `memmap2`. Still well under 4 MB release.
- **Correctness invariant**: `--no-cache` output and cached output must remain byte-for-byte identical to the pre-change TypeScript reference on the project's test fixtures.
- **Determinism**: Event ordering in the database may change because files are parsed in arbitrary completion order, but all reports group and sort deterministically by `(day|month|session, ts)` so user-visible output is unaffected.
