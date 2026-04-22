## 1. Baseline and Benchmarks

- [x] 1.1 Record a baseline cold-ingest timing on the real corpus: `time tokctl daily --rebuild` (capture user / system / wall) and save the number in the PR description
- [x] 1.2 Add `criterion` to `[dev-dependencies]` in `Cargo.toml`
- [x] 1.3 Create `benches/parse.rs` with `claude_parse_line` and `codex_parse_line` benchmarks exercising the current `parse_claude_line` and `parse_codex_line` functions against fixture data
- [x] 1.4 Run `cargo bench` once to record the pre-change parser baseline (ns/iter)

## 2. Typed JSONL Deserialization

- [x] 2.1 Define `#[derive(Deserialize)]` structs for Claude rows in `src/sources/claude.rs` (`ClaudeLine`, `ClaudeMessage`, `ClaudeUsage`) with borrowed `&str` fields where safe and `#[serde(default)]` + `Option<T>` elsewhere
- [x] 2.2 Rewrite `parse_claude_line` to use `serde_json::from_str::<ClaudeLine>(line)` followed by field extraction from the typed struct
- [x] 2.3 Define equivalent structs for Codex rows (`CodexRow`, `CodexPayload`, `TokenCountInfo`, `TokenUsage`)
- [x] 2.4 Rewrite `parse_codex_line` to use typed deserialization; preserve the session_meta / turn_context / event_msg branching logic
- [x] 2.5 Run `cargo test` — all existing parser tests must pass unchanged
- [x] 2.6 Run `cargo bench` and confirm ≥1.5× parser speedup; record the result

## 3. Parallel File Parsing

- [x] 3.1 Add `rayon` to `[dependencies]` in `Cargo.toml`
- [x] 3.2 In `src/ingest/run.rs`, split `execute_plan` into two phases: (a) parallel parse producing `Vec<(DiscoveredFile, ParseResult)>`, (b) serial SQLite writes consuming that vec
- [x] 3.3 Use `rayon::prelude::ParallelIterator` over `plan.to_full_parse` and a separate `par_iter` over `plan.to_tail`
- [x] 3.4 Ensure `IngestStats` accumulation is thread-safe — collect per-file stats then fold, do not share a mutable struct across threads
- [x] 3.5 `cargo test` — all tests still pass; manually verify the ingest integration test
- [x] 3.6 End-to-end parity check: run `tokctl daily --json --rebuild` on the test fixtures with old and new binaries; confirm identical output

## 4. Memory-mapped Reads

- [x] 4.1 Add `memmap2` to `[dependencies]`
- [x] 4.2 In `src/ingest/file_range.rs`, add a private helper `fn map_file(path: &Path) -> Result<Mmap>` that wraps the single `unsafe` block with a documented invariant comment
- [x] 4.3 Introduce a size threshold constant `const MMAP_THRESHOLD: u64 = 1_048_576`
- [x] 4.4 Branch `ingest_claude_range` and `ingest_codex_range` on the byte-range length: ≥ threshold → mmap path, < threshold → existing `BufReader::take` path
- [x] 4.5 mmap path must still honour `from_offset` / `to_offset` by slicing the mapped region
- [x] 4.6 `cargo test` and re-run the fixture parity check

## 5. `--threads` Override

- [x] 5.1 Add a hidden global flag `--threads <N>` to the `Cli` struct in `src/cli.rs` using `#[arg(long, hide = true)]`
- [x] 5.2 If `--threads` is set, call `rayon::ThreadPoolBuilder::new().num_threads(n).build_global()` early in `run()` before any `par_iter`
- [x] 5.3 Validate `N >= 1`; emit a clear error otherwise
- [x] 5.4 Unit test: invoking with `--threads 1` still produces correct output
- [x] 5.5 Confirm `tokctl --help` does not mention `--threads`

## 6. Validation and Docs

- [x] 6.1 Run the real-corpus timing again: `time ./target/release/tokctl daily --rebuild` and confirm ≤ 2.5s wall time
- [x] 6.2 Run with `--threads 1` and confirm output still matches (sanity check that parallelism didn't introduce a data race)
- [x] 6.3 Run `cargo clippy -- -D warnings` and fix anything new
- [x] 6.4 Run `cargo fmt --check`
- [x] 6.5 Add a short "Performance" paragraph to `README.md` with before/after numbers
- [x] 6.6 Final `cargo test` + `cargo bench` — all green, record final benchmark deltas in the PR description
