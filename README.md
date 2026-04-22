# tokctl

Local-only CLI that reports token usage and cost across **Claude** and **Codex** on macOS/Linux. Reads JSONL session logs from disk; no network calls. A small SQLite cache keeps warm runs fast.

## Install

```sh
cargo install --path .
# or build a release binary in place:
cargo build --release
# then use ./target/release/tokctl, or symlink it:
ln -sf "$(pwd)/target/release/tokctl" ~/.local/bin/tokctl
```

Requires a Rust toolchain (stable, edition 2021). SQLite is statically linked via `rusqlite`'s `bundled` feature — no system libraries needed.

## Usage

```sh
tokctl daily               # tokens + cost grouped by date
tokctl monthly             # grouped by YYYY-MM
tokctl session             # grouped by session id
tokctl repo                # grouped by git repo (ordered by cost)
tokctl repo tokctl         # drill down: sessions inside one repo

# filter one tool
tokctl daily --source claude
tokctl daily --source codex

# narrow to a date range
tokctl monthly --since 2026-01-01 --until 2026-03-31

# filter to a single repo (display name, path prefix, or `(no-repo)`)
tokctl daily --repo tokctl
tokctl session --repo /Users/me/dev/api     # disambiguate duplicate names
tokctl monthly --repo "(no-repo)"           # sessions outside any git repo

# pivot an existing report by a different axis
tokctl daily --group-by repo                # same as `tokctl repo`
tokctl monthly --group-by session

# JSON output for scripts
tokctl daily --json
tokctl repo --json

# multiple or alternate directories
tokctl daily --claude-dir /path/a,/path/b
tokctl daily --codex-dir $CODEX_HOME/sessions

# cache controls
tokctl daily --rebuild     # delete the cache DB and re-ingest from scratch
tokctl daily --no-cache    # bypass the cache for this invocation
tokctl export-db           # print the absolute path of the cache DB
```

### Repo resolution

For every event, `tokctl` looks up its originating path (Claude's dash-encoded project folder, Codex's `cwd`) and walks upward for the nearest `.git` ancestor. That canonical path is the *repo key*; its basename is the display name. Symlinks are resolved so the same repo reached through different paths groups together. Events with no git ancestor fall into an explicit `(no-repo)` bucket — they are never merged with a real repo.

### Example table output

```
┌────────────┬─────────┬────────┬────────────┬─────────────┬──────────┬──────────┐
│ date       │ input   │ output │ cache_read │ cache_write │ total    │ cost_usd │
├────────────┼─────────┼────────┼────────────┼─────────────┼──────────┼──────────┤
│ 2026-04-19 │  12,304 │  3,410 │     98,210 │      47,670 │  161,594 │     1.24 │
│ 2026-04-20 │   8,812 │  2,140 │    123,400 │      31,200 │  165,552 │     0.91 │
└────────────┴─────────┴────────┴────────────┴─────────────┴──────────┴──────────┘
```

### Example `--json` output

```json
[
  { "date": "2026-04-19", "input": 12304, "output": 3410, "cache_read": 98210, "cache_write": 47670, "totalTokens": 161594, "costUsd": 1.24 },
  { "date": "2026-04-20", "input": 8812,  "output": 2140, "cache_read": 123400, "cache_write": 31200, "totalTokens": 165552, "costUsd": 0.91 }
]
```

## Interactive UI

```sh
tokctl ui
```

A keyboard-driven two-pane dashboard over the cache — your chosen pivot axis on the left, its sessions on the right. Read-only; never mutates `cache.db`. Refuses to launch on a non-TTY (pipes, CI).

```
┌─ tokctl  2026-04-22 16:30  last 30 days · $84.12 · 1.24M tok ──────────────[?]─┐
│  [ REPOS ]                             │  SESSIONS                             │
│  ▸ tokctl                     $41.22   │  3m ago    claude  tokctl       $8.20 │
│    cortex                     $18.40   │  2h ago    codex   my-project    $3.10│
│    (no-repo)                  $ 9.03   │  yesterday claude  api-service   $1.40│
│  ▁▂▂▃▇▅▃▂▁▂▃▅▇▆▃  cost/day last 30d · window:month · source:all               │
│  j/k move  ↵ drill  h/l pane  / filter  Tab axis  t trend  T/w/m/Y/a window   │
└────────────────────────────────────────────────────────────────────────────────┘
```

`Tab` cycles the left-pane axis (`repo → day → model → session`), and the right pane always shows the sessions belonging to the left-pane selection.

Keybindings:

| Keys | Action |
|---|---|
| `h`/`l` `←`/`→` | move focus between panes |
| `j`/`k` `↓`/`↑` | move selection |
| `g g` / `G` | top / bottom |
| `Ctrl-d` / `Ctrl-u` | half page down / up |
| `Enter` | drill right |
| `Esc` / `Backspace` | cancel filter / pop overlay / pane-left |
| `/` | fuzzy filter within the focused pane |
| `Tab` | cycle left-pane axis (repo → day → model → session) |
| `s` | cycle sort (cost↓, cost↑, recent↓, alpha↑) |
| `t` | toggle trend overlay; inside: `d`/`w`/`m`/`y` granularity |
| `T` `w` `m` `Y` `a` | time window: today / week / month / year / all |
| `1` / `2` / `3` | source filter: all / claude / codex |
| `r` | re-query the cache (no ingest, no JSONL re-parse) |
| `y` | yank focused row's key to clipboard |
| `?` | toggle help overlay |
| `q` / `Ctrl-c` | quit |

Note: the spec originally mapped lowercase `y` to both *yank* and *year window*; we resolved the conflict by keeping `y` for yank (the more frequent action) and using capital `Y` for the year window.

UI preferences (pivot axis, time window, source filter, sort, trend granularity, pane widths, last selection) live in `<cache_dir>/ui_state.json` next to `cache.db`. Delete the file to reset; an unknown `version` is ignored and the file is rewritten with defaults on the next clean exit.

### Clipboard feature

Yank (`y`) uses the `clipboard` cargo feature, which pulls in [`arboard`](https://crates.io/crates/arboard). It's **on by default**. On minimal Linux containers lacking X11/Wayland headers, opt out:

```sh
cargo install --path . --no-default-features
# or for a dev build
cargo build --no-default-features
```

Without the feature, `y` is a no-op.

## Default paths

| Source | Default roots | Env overrides |
|---|---|---|
| Claude | `~/.claude/projects/`, `~/.config/claude/projects/` | `TOKCTL_CLAUDE_DIR` (csv), `CLAUDE_CONFIG_DIR` (csv, ccusage-compatible) |
| Codex  | `~/.codex/sessions/` | `TOKCTL_CODEX_DIR` (csv), `CODEX_HOME` (single path, `/sessions` appended) |
| Cache  | `$XDG_DATA_HOME/tokctl/cache.db` (or `~/.local/share/tokctl/cache.db`) | `TOKCTL_CACHE_DIR` |

**macOS Desktop apps are covered automatically** — both `/Applications/Claude.app` and `/Applications/Codex.app` write their session JSONL to the same paths as the CLIs. The Electron data under `~/Library/Application Support/{Claude,Codex}/` holds only UI metadata (no token buckets) and is deliberately not parsed.

## Prices

Model prices live in [`src/pricing.rs`](src/pricing.rs). Unknown models contribute `0` to the cost and are listed in a trailing warning line. Open a PR to that file when a new model shows up. **Changing a price requires a cache rebuild** — bump `SCHEMA_VERSION` in [`src/store/schema.rs`](src/store/schema.rs) in the same commit so the next run rebuilds with the new prices.

## Debugging / ad-hoc queries

```sh
sqlite3 "$(tokctl export-db)"
```

Useful tables: `events` (one row per token-bearing turn; `events.repo` carries the canonical repo key), `files` (per-JSONL manifest), `repos` (one row per resolved repo with `display_name`, optional `origin_url`, and `first_seen`), `meta` (schema version). Joins: `events.file_path = files.path`, `events.repo = repos.key`.

**Schema migrations.** The schema is version-stamped in `meta.schema_version`. Upgrades from `v2 → v3` (the repo-rollup change) run in place on first open: the `repo` column and `repos` table are added and backfilled from existing `project_path` values without re-parsing JSONL. Changing a price or any other pricing-affecting value requires a cache rebuild — bump `SCHEMA_VERSION` in [`src/store/schema.rs`](src/store/schema.rs), or run with `--rebuild` as a fallback if an in-place migration ever fails.

## Development

```sh
cargo test          # run unit + integration tests
cargo clippy        # lint
cargo fmt           # format
cargo bench         # parser microbenchmarks (criterion)
```

## Scope

v1 intentionally does not do: Cursor, weekly narrative, or git-history correlation (commit ↔ session matching). Per-repo roll-ups are supported via `tokctl repo`; identity is derived locally from `.git` ancestor walks and never over the network. The interactive `tokctl ui` is in-scope and ships as of this release; **mouse support and auto-refresh / file-watch are explicitly out of scope** — refresh is manual (`r`) and navigation stays keyboard-first.
