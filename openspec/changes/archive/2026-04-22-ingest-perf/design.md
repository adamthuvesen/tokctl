## Context

After completing the Rust rewrite (`rust-rewrite` change), measured cold-ingest performance on 2.1 GB / 4,814 JSONL files:

```
                        Cold (--rebuild)    Warm (cache hit)
Rust release            7.0s                0.07s user / 0.7s wall
TS (node dist)          6.7s                ~0.5s
```

Rust is already 7× faster on the warm path (CPU) but **at parity with TS on cold ingest**. A profile would almost certainly show time split between:

1. `serde_json::from_str` into `Value`, which allocates a `BTreeMap` of `String → Value` for every line
2. Sequential file iteration on an 8+ core machine (7 cores idle)
3. `BufReader::take` copy into userspace buffers

All three have textbook Rust mitigations. This design captures them as one coherent change.

## Goals / Non-Goals

**Goals:**
- Reduce cold-ingest wall time on the reference corpus to ≤ 2.5 seconds (≈3× speedup)
- Preserve byte-for-byte report parity with the pre-change binary across cached and `--no-cache` paths
- Keep warnings (`skipped N malformed lines`, unknown models) stable in output and count
- Keep the implementation understandable — no unsafe code, no lock-free exotica

**Non-Goals:**
- Faster SQLite writes (insert batching is already reasonable and not the bottleneck)
- Async I/O or `tokio` — the workload is CPU-bound on JSON parsing, not I/O concurrency
- Streaming the in-memory path — `--no-cache` still materialises events into a `Vec`
- GPU-based JSON parsing (simd-json would be another option but not chosen here; see Decisions)

## Decisions

### 1. `rayon` for parallel file parsing, not `tokio` or `std::thread`

The unit of parallelism is "parse one file into a `Vec<EventRow>`". That's embarrassingly parallel: each file is independent, the output is a fixed-size value, and there's no shared state needed during parsing. `rayon::par_iter` over the plan's `to_full_parse` and `to_tail` slices expresses this in one line.

**Alternatives considered:**
- `tokio` — async buys nothing here (we're not waiting on network) and adds a runtime dependency.
- `std::thread::spawn` per file — would work but rayon's work-stealing scheduler handles unbalanced file sizes better.

### 2. Per-file transactions stay serial

Parsing happens in parallel; **SQLite writes stay on the main thread** inside per-file transactions. `rusqlite::Connection` is not `Sync`, and a write pool would complicate rollback semantics. The parse-then-write pattern is:

```
par_iter(plan.to_full_parse)     → Vec<(DiscoveredFile, ParseResult)>
for each result { tx.insert; tx.commit }
```

This keeps the transactional guarantees of the original design while moving ~80% of wall time off the critical path.

### 3. Typed deserialization with `#[serde(default)]`

Replace `Value::get(...).and_then(as_u64).unwrap_or(0)` chains with:

```rust
#[derive(Deserialize)]
struct ClaudeLine<'a> {
    #[serde(rename = "type", default)]
    kind: &'a str,
    #[serde(default)]
    #[serde(rename = "sessionId")]
    session_id: Option<&'a str>,
    #[serde(default)]
    timestamp: Option<&'a str>,
    #[serde(default)]
    message: Option<ClaudeMessage<'a>>,
}
```

Borrowed `&str` fields avoid allocating new `String`s for every field on every line. `#[serde(default)]` + `Option` makes forward-compatibility identical to the current "tolerate missing keys" behaviour — unknown JSON keys are still silently ignored (serde's default).

**Alternative considered:** `simd-json` for SIMD-accelerated parsing. Rejected for now — adds a big dep, requires `mut` access to the input buffer, and the typed-struct change already captures most of the win. Could be a future follow-up if the parser is still hot after these changes.

### 4. Memory-mapped reads above a size threshold

For files > 1 MB, replace `File::open` + `BufReader::take` with `memmap2::Mmap::map(&file)`. This avoids the userspace copy done by `read(2)` and lets the OS page-in the file as needed. Below the threshold, the overhead of `mmap` setup outweighs the saving — the existing `BufReader` path is faster.

**Safety note:** `Mmap` is `unsafe` to construct because another process could truncate the file mid-read. Our ingest runs against session files that are only ever appended (or not being written at all — those get bumped into `toFullParse` by the safety window). We document the invariant and wrap the unsafe in a narrow helper.

### 5. `--threads <N>` override

A hidden flag (`#[arg(long, hide = true)]`) sets `rayon::ThreadPoolBuilder::num_threads(N)`. Useful for:
- Benchmarking the sequential baseline (`--threads 1`)
- CI runners with constrained CPUs
- Users who want to keep ingest off hot cores

Default is `rayon`'s default (physical core count).

### 6. Criterion benchmarks

Add `benches/parse.rs` with two benchmarks:
- `claude_parse_line` — parse 10,000 assistant lines from a fixture
- `codex_parse_line` — same for codex token_count rows

This gives us a regression detector. Not wired into CI in this change (follow-up).

## Risks / Trade-offs

- **Non-deterministic event IDs** → SQLite assigns rowids in insert order, which now varies by completion time. Mitigation: all report queries already `ORDER BY ts` or `ORDER BY key`. Confirmed no user-visible output depends on rowid order.
- **mmap on a file being actively written** → Would race; mitigated by the existing 1-hour safety window that routes recent files to `toFullParse` rather than `toTail`. We also document the invariant in `file_range.rs`.
- **Borrowed `&str` fields in `Deserialize`** → Requires the underlying line buffer to outlive the parse. Fine because we parse one line at a time inside a `for line in reader.lines()` scope.
- **`rayon` starts a global thread pool** → One-time cost (~ms). Negligible vs savings. Pool is reused across multiple `par_iter` calls.
- **Expected gain could under-deliver** if the corpus is small (few MB). Mitigation: the `--threads 1` override lets users revert to serial if parallel overhead dominates; acceptable tradeoff because default behaviour is still faster on any realistic corpus.

## Migration Plan

1. Land `rayon` + typed-struct parsers first (biggest, safest win). Verify parity test still passes.
2. Land `memmap2` with threshold guard and unsafe wrapper. Verify parity again.
3. Add `--threads` flag.
4. Add criterion benches.

Each step is independently revertable because all three optimisations are orthogonal.

## Open Questions

- Should `--threads` be visible in `--help`? Leaning hidden (advanced use). Can flip based on user preference during implementation.
- Should we log ingest wall time to stderr at some verbosity? Deferred — noise vs observability tradeoff not worth blocking this change.
