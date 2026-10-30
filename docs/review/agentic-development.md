# Agentic development refactor verification

Date: 2026-11-04. Baseline: `20a887f`. Implementation: `5b24f5f`.
Work happened in an isolated local worktree. No branch was pushed.

## Assessment

The original repository had useful provider fixtures and a small command surface.
Its main risks were duplicated report implementations, unsafe file reads, implicit query scopes, and mutable derived cache values.
The original passing suite did not cover several observed failures.

The refactor gives SQLite one connection owner and one report implementation.
Ingestion publishes coherent file replacements. Typed query scopes preserve provider, root, repository, model, session, and time constraints.
The TUI derives display and actions from the same visible rows.

## Implemented phases

| Phase | Concrete result | Principal evidence |
| --- | --- | --- |
| Contract fixtures | Isolated CLI homes, independent report expectations, source-qualified identities | `tests/cli_behavior.rs`, `tests/report_parity.rs` |
| Safe ingestion | Complete-root reconciliation, buffered full-file reads, bounded parsing, serialized writers, atomic replacements | `tests/ingest_safety.rs`, `src/ingest/file_range.rs` tests |
| Cache provenance | Transactional migrations, original explicit costs, derived fingerprints, automatic repricing and calendar projection | `src/store/db.rs` tests |
| Shared reporting | Private Store connection, identical disk and ephemeral paths, one QueryScope | Compile-fail doctests, report parity and scope tests |
| TUI correctness | Typed drills, visible selection, retained parent bounds, root scoping, snapshot errors, owned terminal lifetime | TUI unit tests and `tests/tui_terminal.rs` |
| External adapters | Validated atomic Cursor exports, private credentials, protected demo cache, read-only doctor | Cursor, demo, doctor, and ingest tests |
| Development path | Pinned toolchain, declared MSRV, shared just recipes, Linux/macOS CI matrix, checked benchmarks | `just check`, CI configuration, benchmark checks |
| Independent reassessment | Fresh storage and UI reviews, reproduced findings, fixes rechecked | Review results below |

## Guarantees and their enforcement

| Concern | Highest effective mechanism | Why another check remains |
| --- | --- | --- |
| Competing report logic | One Store/query implementation | Independent fixtures still verify measurement semantics that types cannot prove |
| Unowned database access | Private connection and store-only SQL | Compile-fail examples pin the public boundary against accidental exposure |
| Partial file replacement | Transaction owns events, manifest, diagnostics, provenance, and repository metadata | Failure tests verify external I/O and transactional behavior |
| Missing roots deleting data | Discovery distinguishes complete and failed scans | Filesystem failures cannot be excluded by Rust types alone |
| Parser/rate/calendar drift | Fingerprints derive from authoritative definitions | Migration fixtures verify persisted compatibility and unavailable sources |
| Mixed query generations | One read snapshot, with writer coordination and provenance validation | Concurrent-process tests cover operating-system and SQLite behavior |
| Wrong TUI row activated | One visible projection and typed selection | Interaction tests establish ordering and displayed behavior |
| Invalid external values | Boundary validation and checked numeric aggregation | Malformed fixtures show rejected data cannot produce false success |
| Terminal leakage | Owned cleanup guards and one event loop | PTY verification covers actual terminal modes, resize, and exit |
| Incorrect instructions | Ownership guide and discoverable recipes | Prose covers navigation and judgment. It does not replace enforcement |

Review remains necessary for measurement definitions, useful interfaces, and external service assumptions.
Those concerns cannot be fully reduced to syntax or local deterministic tests.

## Demonstrated current defects covered

- Missing, unreadable, or unselected roots no longer authorize cache deletion.
- Malformed Cursor headers cannot replace a valid cached snapshot.
- Changed Claude files reparse completely, including same-size rewrites and message deduplication.
- Valid JSON with spaces no longer disappears in a string prefilter.
- SQL path prefixes treat `%` and `_` literally.
- Nested TUI drills retain their bucket, repository, model, and source constraints.
- Filtering, copying, details, and drilling use the displayed row order.
- Pricing changes update estimates without manual schema bumps.
- Numeric overflow produces an error or a wide presentation total, rather than wrapped tokens or infinite report costs.
- Malformed-record diagnostics persist across unchanged-file runs and respect root scope.
- Doctor reports failed table inspection rather than inventing zero counts.
- Noninteractive UI requests fail before creating caches or starting sync.

These are current defects or targeted guarantees. They are not all historical regressions.

## Historical replay

Three fixes were replayed against the historical parent and fixed source revisions.
Each isolated harness failed against the parent revision and passed against the fix.

| Fix | Replayed behavior | Parent | Fix |
| --- | --- | --- | --- |
| `ea7c9ec` | Codex input 200 with cached input 50 must produce billable input 150 | Failed | Passed |
| `e0905b2` | Cursor malformed numeric token value must not become zero usage | Failed | Passed |
| `8bfdfe8` | An old directory timestamp must not hide a changed file | Failed | Passed |

The harnesses compiled historical parser or discovery code with isolated fixtures.
This establishes those historical mistake classes. It does not imply that every old revision was rebuilt as an application.
No other historical failures are claimed.

## Independent review

A fresh storage reviewer reproduced three defects during implementation:

1. A demo marker survived real ingestion and still authorized destructive demo overwrite.
2. Santiago's skipped midnight broke the preceding day's inclusive `--until` filter.
3. Another process could change materialized calendar buckets after a Store opened.

All three were fixed and independently rechecked.
The demo replay now rejects overwrite and preserves the ordinary row.
Santiago date filtering succeeds. Apia's skipped whole date returns an explicit error.
A two-process timezone harness confirms writable stores reproject before reading.
Read-only stores reject changed provenance within their pinned read transaction.

A separate UI reviewer reproduced stale rendering after a failed refresh.
Local actions used current state while rendering used old state.
The corrected path retains the old data scope and permits current filter, cursor, sort, and detail interactions.
The independent reviewer reran the TUI tests and found no unresolved demonstrated defect in that scope.

Final integration also checked configured roots through Provider views and nested drills.
The focused fixture changes visible tokens from 222 to 2 when a single root is selected.
A sibling path with the same string prefix remains excluded.

## Compatibility and operational tradeoffs

- Public report JSON fields, inclusive CLI dates, and provider token meanings remain stable.
- `--no-cache` now uses temporary SQLite. It does not create a persistent report database.
- Supported schema versions migrate transactionally. Unsupported versions remain untouched and return an error.
- Legacy rows without original cost provenance remain stale until their source is available.
- Session copy keys intentionally use `source:id`. Saved raw session selections upgrade when selected.
- Weeks use ISO week years. This corrects previous year-boundary grouping.
- Changed files now parse completely. Append-heavy workloads trade incremental speed for coherent provider state and deduplication.
- Demo overwrite now refuses any cache that has undergone ordinary ingestion or rebuild.
- Credential endpoint overrides were removed. Credential-bearing traffic uses the fixed Cursor endpoints.
- Missing roots and malformed records produce visible warnings. Valid records can still produce a partial report.
- `files.last_offset` remains in the persisted schema for compatibility. It no longer controls ingestion.

## Verification boundaries

All source fixtures and demo data were synthetic. No personal logs or credentials were read.
The PTY test runs the actual binary and checks filtering, drilling, resize, normal exit, alternate-screen exit, and terminal-mode restoration.
Unit tests cover cleanup during partial terminal setup and panic unwinding.

Local checks do not establish live Cursor authentication or upstream export availability.
They do not establish physical clipboard operation, every terminal emulator, Linux runtime behavior, or remote CI execution.
The Linux/macOS CI matrix is configured but cannot be claimed as executed without a push.
The user explicitly requested no push.

## Final local check results

Both complete commands exited successfully on macOS arm64:

```sh
RUSTUP_TOOLCHAIN=1.99.1 just check
RUSTUP_TOOLCHAIN=1.95 just check
```

Each command checked formatting and Clippy across all targets with warnings denied.
Each feature configuration passed 241 unit tests, 25 integration tests, and two compile-fail doctests.
Both release builds passed. No tests were ignored.
The PTY test passed with and without clipboard support.

Parser benchmark smoke checks passed for both provider workloads:

```sh
cargo bench --locked --bench parse -- --test
```

The refresh benchmark completed five runs against 1,617 synthetic demo events with zero query errors.
Empty and missing cache inputs failed without success output.
Fresh query-cache median was 2.296 ms, with a 2.129–2.947 ms range.
Populated query-cache median was 0.337 ms, with a 0.293–0.490 ms range.
These measurements exclude ingestion and terminal drawing. Filesystem and SQLite page caches may be warm.

## Performance characterization

**Inconclusive for real-workload performance.** These are synthetic measurements, not a general speed claim.
No representative personal log corpus or CPU profile was used.

The comparison used release binaries, 10,000 compact Claude records, and 2,388,890 input bytes.
Each record contained 130 tokens. Append runs added one distinct 130-token event.
Every measured run checked successful exit, JSON totals, non-null costs, and zero errors.
Each cell contains five runs. Baseline and refactored runs alternated on the same machine.
The machine had ten logical CPUs and competing system load.
The initial fixture attempts returned empty baseline reports and were rejected before any timing was reported.
The baseline prefilter required compact JSON. Both measured sides then received the same valid compact fixture.

| Scenario | Baseline median ms [range] | Refactored median ms [range] | Peak RSS median MiB [range], before → after |
| --- | --- | --- | --- |
| New persistent cache | 62.12 [55.59–65.15] | 65.89 [63.19–73.97] | 22.72 [22.64–22.91] → 24.92 [24.91–25.03] |
| Unchanged persistent cache | 11.33 [11.25–11.56] | 13.36 [13.09–13.74] | 14.41 [14.31–14.44] → 15.62 [15.59–15.67] |
| Append one event | 77.82 [75.41–84.22] | 88.48 [88.02–89.55] | 22.83 [22.58–22.86] → 24.94 [24.91–25.02] |
| Ephemeral report | 15.47 [14.63–16.29] | 38.99 [38.20–40.34] | 20.97 [20.91–21.02] → 29.97 [29.94–30.02] |

“New cache” means a missing report database, not a cold operating-system page cache.
Wall time includes process startup and `/usr/bin/time` overhead.
The shared SQLite path costs more time and memory for ephemeral reports in this fixture.
That tradeoff removes the competing aggregator and retains identical report semantics.
The data does not establish the dominant runtime limiter or a performance budget for larger deployments.
Do not extrapolate these measurements to personal history sizes or long-running app responsiveness.

## Finished-state judgment

The implemented state meets the local correctness and maintainability criteria:

- One owner controls database access, file publication, calendar rules, prices, visible selection, and terminal cleanup.
- Invalid external values and incomplete discovery have explicit outcomes.
- Required scopes compose across reports and drills.
- Local and CI verification share commands and feature configurations.
- Independent reviews have no unresolved demonstrated findings in their reviewed scopes.
- The architecture guide points to concrete owners and focused checks.

Future changes should preserve these guarantees, then verify the affected observable behavior.
Live Cursor, clipboard, Linux runtime, remote CI, and representative real-data performance remain explicitly unverified.
