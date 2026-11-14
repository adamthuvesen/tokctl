# AGENTS.md: tokctl

`tokctl` is a Rust CLI and ratatui dashboard for Claude, Codex, and Cursor usage.
Read [docs/architecture.md](docs/architecture.md) before changing a subsystem.

## Find the owner

- Source formats and token meaning: `src/sources/`, `src/types.rs`.
- Discovery and file replacement: `src/discovery.rs`, `src/ingest/`.
- Connection ownership, transactions, migrations, and all SQL: `src/store/`.
- Date boundaries and prices: `src/calendar.rs`, `src/pricing.rs`.
- CLI requests and output: `src/cli/`, `src/compare.rs`, `src/render.rs`.
- Dashboard state, visible selection, and refresh: `src/tui/`.
- Cursor network adapter: `src/cursor_sync.rs`.

## Verify

```sh
just                    # discover commands
just test sources::     # focused test name filter
just verify-cli         # CLI and report contracts
just verify-ui          # TUI behavior and actual PTY interaction
just check              # fast lane: format, lint, tests (local, pre-commit, CI)
just fix                # auto-fix: cargo fmt in place
just slow               # slow lane: no-default-features tests, release build
pre-commit install      # once per clone: runs just check before each commit
```

Prove changes with `just verify` (`verify-tokctl` skill at `.agents/skills/verify-tokctl/`).
Use `tokctl ui --demo --cache-dir PATH` for public-safe application checks.

## Contracts that need judgment

Keep provider token semantics, explicit Cursor costs, and public JSON fields stable unless a change explicitly migrates them.
The architecture owns mechanical guarantees. Do not restore a separate in-memory report implementation or append-only parser path.
A pricing or parser change derives its fingerprint automatically. A schema change still needs a transactional migration.
Read related tests and the architecture notes before changing these contracts.

Report local verification separately from live Cursor, clipboard, host-terminal, and remote CI evidence.

## Enforced Rules

Cite each enforcer as `path::name`; `enforced_rules_table_cites_real_enforcers` checks that the file defines it.

| Rule | Enforced by | Added |
| --- | --- | --- |
| SQL has one connection owner | `clippy.toml::disallowed-types`, `tests/repo_rules.rs::connection_lint_allows_stay_in_store` | 2026-11-04 |
| A file has one provider claim | `tests/ingest_safety.rs::conflicting_provider_claims_fail_before_any_ingest_mutation` | 2026-11-04 |
| Missing or failed roots never authorize deletion | `tests/ingest_safety.rs::absent_unselected_and_failed_roots_preserve_snapshots`, `tests/ingest_safety.rs::complete_other_provider_root_cannot_purge_unavailable_source` | 2026-11-04 |
| A failed replacement keeps the last valid snapshot | `tests/ingest_safety.rs::malformed_only_replacement_preserves_nonempty_valid_snapshot` | 2026-11-04 |
| Report values and warnings describe one snapshot | `tests/ingest_safety.rs::read_batches_pin_one_snapshot_and_readonly_queries_reject_changed_provenance`, `tests/cli_behavior.rs::diagnostic_failure_prevents_partial_report_stdout` | 2026-11-04 |
| Displayed and activated rows agree | `src/tui/input/tests.rs::filtered_highlight_copy_details_totals_and_drill_use_the_same_repo` | 2026-11-04 |
| Demo overwrite needs a synthetic cache | `src/demo.rs::overwrite_refuses_normal_cache_without_changing_rows`, `tests/ingest_safety.rs::normal_ingestion_and_rebuild_remove_demo_overwrite_authority` | 2026-11-04 |
| Doctor is read-only and offline | `tests/cli_behavior.rs::doctor_json_does_not_create_cache_or_config`, `tests/repo_rules.rs::doctor_stays_offline` | 2026-11-06 |
| Only the Cursor sync adapter uses the network or spawns processes | `tests/repo_rules.rs::network_and_process_calls_stay_in_the_cursor_sync_owner` | 2026-11-07 |
| Fixtures hold no real home paths, user names, session IDs, or emails | `tests/repo_rules.rs::fixtures_contain_no_real_identities` | 2026-11-06 |
| Local, pre-commit, and CI run one fast lane | `tests/repo_rules.rs::fast_lane_runs_in_pre_commit_and_ci` | 2026-11-04 |
| `verify control` must observe FAIL before a verify pass counts | `.agents/skills/verify-tokctl/verify::control` | 2026-11-06 |
| The verify helper passes basedpyright strict as its own project, and ruff lints the extensionless script by name | `justfile::lint`, `.agents/skills/verify-tokctl/pyrightconfig.json::"extends"`, `pyproject.toml::typeCheckingMode` | 2026-11-07 |
| No crate, module, or Python file turns a check off: no inner `#![allow]`, no `[lints]` allow, no bare or file-wide `noqa`, no bare or reasonless `pyright: ignore`, no weaker `pyright:` mode | `tests/repo_rules.rs::no_check_is_silenced_for_a_whole_file_or_crate` | 2026-11-09 |
| A Claude message id counts once across all session files | `tests/cli_behavior.rs::a_claude_message_id_in_two_session_files_counts_once` | 2026-11-07 |
| This table cites real enforcers | `tests/repo_rules.rs::enforced_rules_table_cites_real_enforcers` | 2026-11-06 |
