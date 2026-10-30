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
- Cursor network adapter: `src/cursor_sync.rs`. Doctor remains read-only and offline.

## Verify

```sh
just                    # discover commands
just test sources::     # focused test name filter
just verify-cli         # CLI and report contracts
just verify-ui          # TUI behavior and actual PTY interaction
just check              # format, lint, both feature sets, release build
```

Rustup reads the pinned toolchain from `rust-toolchain.toml`. The supported minimum is in `Cargo.toml`.
Use locked dependencies. CI runs `just check` on Linux and macOS.

Tests must use synthetic logs, isolated homes, and temporary caches.
Keep real credentials and usage logs out of fixtures and benchmark output.
Use `tokctl ui --demo --cache-dir PATH` for public-safe application checks.
Demo overwrite requires a cache still marked as synthetic.

## Contracts that need judgment

Keep provider token semantics, explicit Cursor costs, and public JSON fields stable unless a change explicitly migrates them.
The architecture owns mechanical guarantees. Do not restore a separate in-memory report implementation or append-only parser path.
A pricing or parser change derives its fingerprint automatically. A schema change still needs a transactional migration.
Preserve the last valid file snapshot when a source fails. Missing roots must not authorize deletion.
Read related tests and the architecture notes before changing these contracts.

Report local verification separately from live Cursor, clipboard, host-terminal, and remote CI evidence.
