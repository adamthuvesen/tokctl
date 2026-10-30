# Architecture

`tokctl` reports Claude, Codex, and Cursor usage from local JSONL and CSV files.
SQLite owns cached facts and report queries. `--no-cache` uses the same store in memory.
Cursor sync is the explicit network adapter. Configured automatic sync also runs before normal ingestion.

## Ownership and dependency direction

| Owner | Responsibility | Main files |
| --- | --- | --- |
| CLI | Validate requests, resolve roots, prepare a store, render results | `src/cli.rs`, `src/cli/` |
| Source parsers | Decode provider records, normalize token semantics, reject invalid usage | `src/sources/`, `src/types.rs` |
| Discovery | Report files and scan completeness for each source root | `src/discovery.rs`, `src/paths.rs` |
| Ingestion | Read coherent files, plan replacements, publish file snapshots | `src/ingest/`, `src/store/ingest_run.rs` |
| Store | Own connections, migrations, locks, atomic writes, and report SQL | `src/store/` |
| Calendar | Resolve timezone identity, local dates, ISO weeks, and half-open bounds | `src/calendar.rs`, `src/dates.rs` |
| Pricing | Own rates and their derived fingerprint | `src/pricing.rs` |
| TUI | Own navigation, visible rows, terminal lifetime, and displayed snapshots | `src/tui/` |
| Adapters | Fetch Cursor exports, seed demo data, inspect local health | `src/cursor_sync.rs`, `src/demo.rs`, `src/doctor/`, `src/store/inspection.rs` |

`Store` keeps its connection private. Production SQL stays within its module boundary.
The ingest runner (`src/store/ingest_run.rs`) and doctor cache inspection (`src/store/inspection.rs`) live in the store because they run SQL. `clippy.toml` rejects `rusqlite::Connection` outside `src/store/`.
Their files remain beside the related domain code for navigation.
Test fixtures have separate access. Production callers cannot obtain a raw connection.

## Ingest

Discovery returns files and root completeness. A missing or unreadable root is not an empty successful scan.
Deletion reconciliation requires a complete selected root from the same provider.
A file has one provider owner. Conflicting discovery or manifest claims fail before ingestion changes snapshots.
Unavailable roots preserve cached rows and produce diagnostics.
The store remembers requested root aliases, so an unavailable symlink retains its last resolved scope.

Changed files are read through buffered descriptors. There is no mmap or provider-specific append path.
Discovery, read validation, and the manifest share file identity, ctime, size, and mtime.
This detects equal-size replacements with preserved mtime. Missing legacy identity forces a complete parse.
Non-Unix systems conservatively reparse files because the current identity implementation is Unix-specific.
Each changed file is parsed completely so deduplication and cumulative provider counters retain their full context.
Unchanged files use manifest and parser fingerprints to avoid parsing.

Parsing runs in bounded parallel batches. SQLite writes run serially.
A file replacement publishes its events, manifest, parser provenance, diagnostics, and repository metadata in one transaction.
A read or parse failure preserves the previous file snapshot.
A malformed-only replacement also preserves an existing nonempty snapshot.
A new malformed-only file retains diagnostics. Partial valid files remain reportable with visible malformed-record counts.
An ingest lock serializes discovery, planning, and publication across processes.

Tokens preserve provider measurement semantics. Codex cache reads and writes are separated from ordinary input.
Codex cumulative counters determine subsequent usage deltas. Unchanged counters emit no usage.
The first notification retains its latest request, without importing an inherited cumulative total.
Legacy records without cumulative counters retain latest-request behavior.
Counter decreases produce diagnostics and establish a new baseline. Invalid records cannot advance that baseline.
Cursor explicit CSV costs remain authoritative. Unknown priced models retain zero cost with visible warnings.
These reports are estimates, not billing records.

## Store and compatibility

Schema migrations run transactionally. Supported older caches retain their rows.
Schema 8 adds nullable file identity and ctime, plus source-root alias mappings.
Unsupported schemas fail without deleting the database. `--rebuild` clears cache data transactionally.

Events retain original explicit costs and cost provenance. Parser, price, and calendar fingerprints derive from authoritative definitions.
A rate change reprices estimated costs. Explicit provider costs stay unchanged.
A calendar change updates date projections. Parser changes trigger full file replacement when sources are available.
Legacy rows without recoverable cost provenance remain marked stale until their source can be parsed.

A read snapshot pins multi-query results. Writable snapshots coordinate with ingestion and refresh projections before reading.
Read-only snapshots reject changed provenance inside their transaction.
Reports prepare output and scoped diagnostics in the same snapshot before writing stdout.
`ReportDiagnostics` owns unknown-model, malformed-record, and stale-provenance reporting.
Current input failures are run-level diagnostics because some failures have no usable event identity.

Public reports share `QueryScope`: source, source roots, repository, model, source-qualified session, and timestamp bounds.
Repository identity matches exactly. User path prefixes are literal, including `%` and `_`.
Time ranges are half-open internally. CLI `--until YYYY-MM-DD` remains inclusive of that local date.
Weeks use ISO week years. Calendar rules handle daylight-saving transitions and skipped midnight.
A skipped whole local date produces an explicit error.

Disk and ephemeral stores use identical ingest and query code.
There is no separate in-memory aggregator to maintain.
Report JSON field names remain compatible. Source-qualified session keys distinguish equal IDs from different providers.

## TUI

`state/` owns user choices. Typed aggregate selections retain repository, model, source, and exact bucket bounds.
A drill stores its structural parent scope. The current global source and time window intersect that scope at query time.
Changing a window can widen a drill only within its parent bucket.
Configured source roots also constrain Provider views, sparklines, and every drill.

`projection.rs` builds the visible row order. Rendering, selection, details, copying, and visible totals consume that projection.
A displayed row and an activated row therefore have one authoritative identity.
Copied session keys use `source:id` to avoid provider collisions.

`cache.rs` publishes a complete successful snapshot. Failed refreshes retain the previous data and report its scope and error.
The cache observes store generation and calendar day. Typed view queries determine when navigation requires new data.
The event loop polls terminal events directly. Terminal setup and teardown have one owner, including partial setup failure.

## External boundaries

Cursor downloads are parsed before atomic publication. Malformed exports cannot replace a valid local export.
Credentials use private atomic files. Reading status or running doctor does not migrate credentials.
Credential-bearing requests use the fixed Cursor service endpoints.
Normal sync failures preserve the last valid local export. CLI reports print the warning.
The TUI retains the warning across refresh and navigation, explicitly identifying cached exports.

Demo seeding requires an isolated cache. Overwrite accepts only a marked synthetic cache.
Normal ingestion or rebuild removes that marker before the cache can contain ordinary usage.
Doctor inspects local state without network requests or repair writes.

## Making a change

| Change | Start here | Focused verification |
| --- | --- | --- |
| Provider record or token meaning | `sources/`, `types.rs` | `just test sources::`, `cargo test --locked --test ingest_safety` |
| Store, migration, or query | `store/`, `ingest/` | `just test store::`, `cargo test --locked --test ingest_safety`, `just verify-cli` |
| Time or pricing | `calendar.rs`, `dates.rs`, `pricing.rs` | `just test calendar::`, `just test pricing::`, `just verify-cli` |
| TUI behavior | `state/`, `projection.rs`, `cache.rs`, `input.rs` | `just verify-ui` |
| Cursor or demo | `cursor_sync.rs`, `demo.rs` | `just test cursor_sync::`, `just test demo::` |

Run `just check` for the fast lane and `just verify` for the slow lane. CI runs both on Linux and macOS, including the declared MSRV.
The PTY test launches the actual binary with synthetic data. It checks navigation, resize, exit, and terminal restoration.
Parser benchmarks measure parsing only. `just bench CACHE_DB` measures query refresh against an explicit nonempty cache.
The refresh benchmark fails on stale provenance, empty data, or query errors.

Tests use isolated homes and synthetic source files. Local success does not verify live Cursor authentication, clipboard access, or every terminal host.
