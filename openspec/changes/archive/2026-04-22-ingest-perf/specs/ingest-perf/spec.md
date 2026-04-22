## ADDED Requirements

### Requirement: Parallel file parsing
The ingest runner SHALL parse the files in `plan.to_full_parse` and `plan.to_tail` concurrently using a work-stealing thread pool (default: physical core count). Parsing SHALL produce a `Vec<EventRow>` per file without touching the SQLite connection.

#### Scenario: Multiple files parsed concurrently
- **WHEN** the plan contains 100 files to full-parse
- **THEN** the files are parsed using up to `num_cpus` threads in parallel

#### Scenario: Single-file plan still works
- **WHEN** the plan contains exactly one file
- **THEN** that file is parsed correctly (no deadlock, no spin-up overhead that dominates)

### Requirement: Serial database writes
All SQLite writes SHALL execute on a single thread after parsing completes. Per-file transactional atomicity SHALL be preserved — a parse failure for one file MUST NOT affect events written for another file.

#### Scenario: Write order is serial
- **WHEN** N files are parsed in parallel
- **THEN** their inserts and manifest upserts run sequentially inside N independent transactions

#### Scenario: Parse failure isolated
- **WHEN** parsing one file errors
- **THEN** already-parsed files still commit their events; only the failing file's events are dropped

### Requirement: Output parity with pre-change behaviour
For any corpus of JSONL files, the ingest runner with parallelism enabled SHALL produce report output (daily, monthly, session, table and JSON) that is byte-for-byte identical to the pre-change serial implementation.

#### Scenario: Cached path parity
- **WHEN** `tokctl daily --rebuild` is run against the project's test fixtures with the new parallel path and the pre-change serial path
- **THEN** the `--json` outputs are identical when sorted by key

#### Scenario: Warning counts stable
- **WHEN** the corpus contains malformed lines and unknown models
- **THEN** the stderr warning messages match the pre-change output exactly

### Requirement: Typed JSONL deserialization
Both Claude and Codex JSONL parsers SHALL deserialize lines into `#[derive(Deserialize)]` structs rather than navigating `serde_json::Value` at runtime. Unknown JSON fields SHALL be silently ignored; missing optional fields SHALL default via `#[serde(default)]`.

#### Scenario: Unknown fields ignored
- **WHEN** a line contains a JSON field not present in the deserialization struct
- **THEN** parsing succeeds and the unknown field is discarded

#### Scenario: Missing optional fields defaulted
- **WHEN** a line omits an optional field (e.g. `message.id` in Claude)
- **THEN** the field deserializes to `None` without error

#### Scenario: Forward-compatible with future format tweaks
- **WHEN** a future schema adds a new field (e.g. `reasoning_tokens_v2`) to the usage block
- **THEN** existing fields continue to parse without modification

### Requirement: Memory-mapped reads for large files
Files larger than a configurable size threshold (default: 1,048,576 bytes / 1 MB) SHALL be read via `memmap2::Mmap`. Files at or below the threshold SHALL continue to use buffered reads.

#### Scenario: Large file uses mmap
- **WHEN** a file is 10 MB
- **THEN** the runner uses mmap to access its byte range

#### Scenario: Small file uses buffered reader
- **WHEN** a file is 500 KB
- **THEN** the runner uses the existing `BufReader` path

#### Scenario: mmap wrapped in narrow unsafe block
- **WHEN** mmap is used
- **THEN** the `unsafe` call site is confined to a single helper function with a documented invariant comment

### Requirement: `--threads N` override
The CLI SHALL accept a hidden global flag `--threads <N>` that sets the rayon thread pool size. When omitted, the pool uses rayon's default (physical core count). `N = 1` SHALL force fully serial execution.

#### Scenario: --threads 1 serializes ingest
- **WHEN** `tokctl daily --threads 1 --rebuild` is invoked
- **THEN** files are parsed one at a time

#### Scenario: --threads is hidden from --help
- **WHEN** the user runs `tokctl --help` or `tokctl daily --help`
- **THEN** `--threads` does not appear in the output

#### Scenario: Invalid --threads value errors
- **WHEN** `--threads 0` or `--threads -1` is passed
- **THEN** the CLI exits non-zero with a clear error message

### Requirement: Criterion benchmarks for the parser hot path
The crate SHALL include criterion-based benchmarks at `benches/parse.rs` exercising `parse_claude_line` and `parse_codex_line` against representative fixture data. Benchmarks SHALL be runnable via `cargo bench` without modification.

#### Scenario: cargo bench runs to completion
- **WHEN** `cargo bench --bench parse` is invoked
- **THEN** it compiles, runs both benchmarks, and reports timings

#### Scenario: Benchmarks use the public parser API
- **WHEN** the parser signature changes in a backward-compatible way
- **THEN** the benchmarks still compile without modification
