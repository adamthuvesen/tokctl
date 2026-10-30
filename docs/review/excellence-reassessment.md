# Excellence reassessment

Date: 2026-11-04. Starting revision: `833515f`.
Implementation commits: `ce61dca` and `bbf1bcb`.
This supersedes the finished-state judgment in the earlier refactor report.

## Verdict

The first refactor was stronger, but its excellent-state claim was premature.
Fresh adversarial review found nine concrete correctness and feedback gaps.
This iteration fixed those gaps, then fixed a counter-validation bug introduced during implementation.
The final scope has no unresolved demonstrated findings from independent review.

The repository now meets the local agentic-development standard: clear owners, enforceable boundaries, meaningful failure tests, and discoverable verification.
That verdict covers the code and verified environments. It does not claim that every external integration has been exercised.

## Changes and evidence

| Problem | Implemented boundary | Proof |
| --- | --- | --- |
| A Cursor scan could authorize deletion of an unavailable Claude root | Scan completeness retains its provider | Cross-provider purge regression in `tests/ingest_safety.rs` |
| The same path could change provider ownership silently | Conflicting discovery and manifest claims fail before ingestion writes | Same-run and existing-manifest conflict fixtures |
| A malformed-only replacement erased valid usage | Preserve an existing nonempty snapshot, report the failed replacement | New and existing malformed-file cases |
| Equal-size replacements with preserved mtime remained stale | Persist identity and ctime, and validate discovery against the read descriptor | Atomic replacement and discovery/read race fixtures |
| An offline symlink changed report scope | Persist requested-to-canonical roots per provider | CLI fixture retains previous usage after the target disappears |
| Retained stale values appeared current | Scoped `ReportDiagnostics` joins the report read transaction | CLI, dashboard, and diagnostic-failure fixtures |
| UI sync ignored a configured Cursor root | Reports and UI share sync target resolution | Root-resolution tests and caller trace |
| Sync failures disappeared inside the dashboard | Carry failure state into persistent dashboard warnings | Failure survives refresh and filtering in rendering tests |
| Codex repeated notifications added usage twice | Compute subsequent usage from cumulative counter deltas | Repeated, changed-last, skipped-notification, and reset fixtures |
| Impossible cache subsets and oversized counters corrupted accounting | Validate subsets and ranges before advancing counter state | Invalid initial, intermediate, and reset-baseline fixtures |

Four storage regressions were demonstrated failing before their fixes.
Reviewers also reproduced the original defects through isolated CLI harnesses.
An independent reviewer caught an invalid-counter recovery bug during implementation.
That finding is fixed, with explicit tests for the first, intermediate, and reset paths.

These tests remain necessary because filesystem changes, SQLite transactions, terminal state, and external record values cannot be proven by Rust types alone.
Architecture owns the normal path. Tests exercise the boundary failures.

## Measurement semantics

Codex maintains cumulative token fields and can emit notifications without additional usage.
Its context estimates can change latest-usage fields without changing accumulated usage.
The parser therefore counts subsequent component-wise cumulative deltas.
The first notification retains latest-request usage, preserving the first request without charging inherited cumulative history.
Records from older formats without cumulative counters retain latest-request behavior.
Counter decreases are diagnosed and establish a valid new baseline. No negative delta becomes usage.

This is an intentional correction to reported Codex totals. It is not a price-table change.
The parser fingerprint causes existing sources to be reparsed automatically.
Unavailable sources retain their previous values with visible stale-provenance warnings.

Primary evidence: OpenAI's [token protocol](https://github.com/openai/codex/blob/27065a91ccbf5ba8532844be3777fcf83b77820d/codex-rs/protocol/src/protocol.rs) defines cumulative field-wise addition.
The [session implementation](https://github.com/openai/codex/blob/27065a91ccbf5ba8532844be3777fcf83b77820d/codex-rs/core/src/session/mod.rs) restores token state for resumed or forked history.
The parser's first-request rule preserves tokctl's existing file-level reporting contract rather than importing full lineage totals.

Schema 8 adds nullable file identity and ctime plus source-root alias mappings.
Older manifests reparse because their identity is unknown. Migrations retain existing events.
Non-Unix systems conservatively reparse files until a platform-specific identity implementation exists.

## Real-log verification

A private temporary snapshot contained 1,130 Claude files and 670 Codex files, totaling 3,374,102,716 bytes.
No files changed during capture. No copy errors occurred.
Original logs and the normal application cache were not modified.
No prompts, session identifiers, repository names, or raw log records were added to this repository.

The same snapshot ran through the preceding implementation and the revised implementation.
For the revised implementation, disk, ephemeral, and unchanged-cache daily reports matched exactly for both providers.
Claude totals remained unchanged.

An independent numeric scan calculated Codex's expected ordinary input, cache reads, cache writes, and output.
All four totals matched the revised CLI output.
It found 303 unchanged cumulative notifications and no counter resets.
The corrected total was 52,829,876 tokens lower than the preceding implementation's total.
This is a deterministic accounting correction on that snapshot, not a performance or billing claim.

There were no malformed-record, file-read, or stale-provenance warnings in the revised snapshot reports.
One internal model remained unpriced and produced an explicit warning.
Its cost was not invented. Cost reports remain estimates.

Timing was not compared here. The repeated run had different cache warmth and concurrent builds.
The earlier synthetic timing characterization remains historical evidence only.

## Verification

The final shared `just check` recipe passed on:

- macOS arm64 with Rust 1.99.1.
- macOS arm64 with Rust 1.95.
- Ubuntu 24.04 arm64 in a local container with Rust 1.99.1.
- Ubuntu 24.04 arm64 in a local container with Rust 1.95.

Each run checks formatting, strict Clippy across all targets, both feature configurations, and a release build.
Each feature configuration passes 257 unit tests, 36 integration tests, and two compile-fail doctests.
The actual PTY test passes on both operating systems, with and without clipboard support.
The container uses the same `just` version installed by CI.

Independent storage and interface reviews rechecked the fixes.
The interface review found no need for an additional snapshot abstraction: current public queries enforce snapshot entry within the store boundary.
Unused query and ingest warning accumulation were removed. Scoped diagnostics now own that behavior.

## Remaining limits

- Live Cursor authentication and upstream service availability were not exercised. Fake API tests cover local validation and publication behavior.
- Physical clipboard interaction and every terminal host remain unverified. Headless Linux and macOS PTY behavior are verified.
- GitHub-hosted CI was not triggered because no branch was pushed. Local Linux and macOS runs cover its recipe.
- Static model prices and explicit unknown-price warnings remain the existing product contract. Reports are not billing records.
- Legacy formats without cumulative usage cannot identify repeated notifications reliably. Their established latest-request semantics remain explicit.

These are integration or product limits, not concealed passing-check claims.
There are no known remaining implementation blockers for the requested local scope.
