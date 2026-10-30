---
name: verify-tokctl
description: "Drives the real tokctl CLI and TUI against offline fixtures in a temp HOME. Use to run, launch, drive, or verify tokctl, or to prove a change to it works."
---

# Verify tokctl

`verify` builds `target/debug/tokctl` with `cargo build --locked`, then drives that binary the way a user does: report commands on fixture logs, `doctor`, and the demo UI in a real PTY. It never reads your real Claude, Codex, or Cursor logs.

## Run

From the repo root:

- `just verify` after a change. It runs the drives whose paths match files changed against the merge base with `main`, plus uncommitted and untracked files. With no match it runs the smoke drive.
- `just verify all` before handing over larger work. It takes a few seconds once the build is warm.
- `just verify list` shows each drive, what it proves, and the paths it covers.
- `just verify <drive> ...` runs named drives.
- `just verify control` re-drives the smoke drive against a wrong daily total. It exits 0 only when it observes FAIL. Run it once per session before you trust a pass.

## Reading results

Each drive prints `PASS|FAIL <drive> <seconds>s <detail>`. A FAIL detail names the expected and the actual value. A drive that crashes is a FAIL. The run also fails when a drive changes the checkout: a tracked, untracked, or ignored file (`target/` included), the index, HEAD, or `.git/config`; only `.verify/` may change. The run ends with `evidence: .verify/runs/<utc>-<sha7>[-dirty]/`, which holds `build.log`, one `<drive>.<step>.json` per command (argv, env, exit, stdout, stderr), `ui.screen.txt` for the PTY drive, and `summary.json` (overall `result` and `checkout_changed`). The helper does not retry, so it never reports `FLAKY`. A FAIL is a FAIL: rerun it by name to investigate, and report both results.

## Limits

- Cursor sync stays off: the temp HOME has no Cursor credentials, and preflight refuses to run if it does. Prove `tokctl cursor login|sync` by hand against a real account only when the change touches `src/cursor_sync.rs`.
- Clipboard (`y`/`Y`), host terminal quirks, and real personal logs are out of scope. Report them separately, as AGENTS.md says.
- The drives use the debug build. `just slow` covers the release build and the `--no-default-features` tests.

## Gotchas

- If `cargo` is missing from PATH, the helper falls back to `~/.cargo/bin/cargo`. Run `. ~/.cargo/env` if that fails too.
- Do not prototype drives in zsh with `env $VARS cmd`: zsh does not word-split, so the isolation variables are lost and nothing runs. The helper builds the env as data.
- Never run `tokctl` by hand without `TOKCTL_CACHE_DIR` and the `TOKCTL_*_DIR` variables set. The defaults read your real logs and write `~/.local/share/tokctl/cache.db`.
