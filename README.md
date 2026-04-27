# tokctl

Token and cost reports from **Claude** and **Codex** JSONL session logs plus **Cursor** usage data from local CSV caches or optional remote sync. Incremental ingest into a **SQLite** cache; warm runs stay fast.

## Install

```sh
cargo install --path .
# or:
cargo build --release && ln -sf "$(pwd)/target/release/tokctl" ~/.local/bin/tokctl
```

**Rust 1.75+** (edition 2021). SQLite is bundled via `rusqlite` — no system `libsqlite`.

## CLI

```sh
# pivots
tokctl daily                 # by calendar day
tokctl monthly               # by YYYY-MM
tokctl session               # by session id
tokctl repo                  # by git repo (cost desc)
tokctl repo tokctl           # sessions inside one repo
tokctl compare               # this month-to-date vs comparable previous month
tokctl compare this-week last-week --by repo
tokctl doctor                # local setup/cache/pricing diagnostics

# filters & pivots (report commands: daily, monthly, session, repo)
tokctl daily --source claude
tokctl monthly --since 2026-01-01 --until 2026-03-31
tokctl daily --repo tokctl                    # name, path prefix, or "(no-repo)"
tokctl daily --group-by repo                  # same rollup as `tokctl repo`

# output & inputs
tokctl daily --json
tokctl daily --claude-dir /a,/b --codex-dir ~/.codex/sessions --cursor-dir ~/.config/tokctl/cursor-cache

# Cursor setup & sync
tokctl cursor login
tokctl cursor status
tokctl cursor sync

# cache
tokctl daily --rebuild       # wipe DB, full re-ingest
tokctl daily --no-cache      # in-memory only, no SQLite
tokctl export-db             # print cache path (does not create DB)
```

## What you get

- **Rollups:** day, month, session, or repo; `tokctl repo <name>` drills to sessions.
- **Diagnostics:** `tokctl doctor` checks roots, discovered inputs, cache health, pricing coverage, and Cursor sync readiness without mutating local data.
- **Comparisons:** `tokctl compare` explains cost/token deltas between two windows, with breakdowns by source, repo, model, or session.
- **JSON** (`--json`) for scripts; tables default for the terminal.
- **Ingest:** parallel parse of Claude/Codex JSONL plus Cursor CSV input, serial writes to SQLite; mmap on large JSONL files with a safety window for recently touched files.
- **Repos:** nearest `.git` ancestor from each event’s project path (Claude) or `cwd` (Codex); symlink-normalized keys; `(no-repo)` when nothing matches.
- **Cost:** static model table for Claude/Codex; Cursor preserves reported row cost when present. Unknown priced models count as **$0** with a trailing warning.
- **Cursor sync:** optional session-token workflow that can fetch Cursor usage CSVs into a local cache when no local exports already exist.

### Example (`tokctl daily`)

```
┌────────────┬─────────┬────────┬────────────┬─────────────┬──────────┬──────────┐
│ date       │ input   │ output │ cache_read │ cache_write │ total    │ cost_usd │
├────────────┼─────────┼────────┼────────────┼─────────────┼──────────┼──────────┤
│ 2026-04-19 │  12,304 │  3,410 │     98,210 │      47,670 │  161,594 │     1.24 │
│ 2026-04-20 │   8,812 │  2,140 │    123,400 │      31,200 │  165,552 │     0.91 │
└────────────┴─────────┴────────┴────────────┴─────────────┴──────────┴──────────┘
```

## Interactive UI

```sh
tokctl ui
```

Refreshes the SQLite cache from the default/env Claude, Codex, and Cursor roots, optionally refreshes Cursor from the network when credentials are configured, then launches the interactive view. **TTY only** — exits if stdout is not a terminal.

Sidebar/main shell: a left **Sections** sidebar (Repos · Days · Models · Sessions · Provider) and a right **Main** pane that fills with the active section. Some sections (Repos) host **tabs** as lenses on the same data. Pressing `Enter` on a row drills in-place with a breadcrumb header; `Esc` / `←` pops back. Press `?` for in-app help. Minimum width: 80 columns.

```
┌─ tokctl  2026-04-25 12:52  last month · $11442.57 · 5.37M tok ─────────[?]─┐
│ ┌ Sections ┐  ┌─ Costs ─└────────┘─ Provider ─────────────────────────────┐ │
│ │▌ Repos   │  │  name              tok          cost                    │ │
│ │  Days    │  │  ▌ tokctl       1.2M         $4.20                       │ │
│ │  Models  │  │   apps         850K          $2.10                       │ │
│ │  Sessions│  │   (no-repo)    240K          $0.80                       │ │
│ │ Provider │  │                                                          │ │
│ └──────────┘  └──────────────────────────────────────────────────────────┘ │
│ ▁▂▂▃▇▅▃▂▁▂▃▅▇▆▃  window:month  source:all  │  [j/k] move · [[/]] section · │
│                                              [tab] tabs · [↵] drill · …    │
└────────────────────────────────────────────────────────────────────────────┘
```

| Keys | Action |
| --- | --- |
| `j` `k` / arrows | move within focused area (sidebar OR main) |
| `[` / `]` | previous / next section |
| `h` / `←` | pop drill, then focus sidebar |
| `l` / `→` | focus main |
| `Enter` | drill (sidebar→main; main→push drill) |
| `Esc` / `Backspace` | cancel filter, close overlay, pop drill |
| `g` `g` / `G` | top / bottom |
| `Ctrl-d` / `Ctrl-u` | half page |
| `Tab` | cycle main-pane tabs |
| `t` | jump to Provider section |
| `d` `w` `m` `y` | time-bucket granularity (Provider or Days section) |
| `s` | sort cycle |
| `e` | compact / expanded |
| `/` | fuzzy filter |
| `T` `W` `M` `z` `a` | window: today / week / month / year / all |
| `1` `2` `3` `4` | source: all / claude / codex / cursor |
| `r` | re-query cache (no ingest) |
| `i` | row details |
| `y` / `Y` | yank row key / summary (needs `clipboard` feature) |
| `?` | help overlay |
| `q` / `Ctrl-c` | quit |

Preferences live in `<cache_dir>/ui_state.json` next to `cache.db` (delete to reset).

**Clipboard:** default build includes [`arboard`](https://crates.io/crates/arboard). Headless Linux: `cargo install --path . --no-default-features` — then `y` is a no-op.

## Data locations

| | Default | Override |
| --- | --- | --- |
| Claude projects | `~/.claude/projects/`, `~/.config/claude/projects/` | `--claude-dir` (csv), `TOKCTL_CLAUDE_DIR` (csv), `CLAUDE_CONFIG_DIR` (csv → each root + `/projects`) |
| Codex sessions | `~/.codex/sessions/` | `--codex-dir` (csv), `TOKCTL_CODEX_DIR` (csv), `CODEX_HOME` (+ `/sessions`) |
| Cursor usage CSV | `~/.config/tokctl/cursor-cache/`, `~/.config/tokscale/cursor-cache/` | `--cursor-dir` (csv), `TOKCTL_CURSOR_DIR` (csv) |
| Cache file | `$XDG_DATA_HOME/tokctl/cache.db` or `~/.local/share/tokctl/cache.db` | `TOKCTL_CACHE_DIR` (directory → `<dir>/cache.db`) |

Desktop **Claude.app** / **Codex.app** use the same JSONL locations as the CLIs. `~/Library/Application Support/{Claude,Codex}/` is UI-only — not parsed.
Cursor support is still CSV-cache based inside `tokctl`: whether the CSV came from manual export or `tokctl cursor sync`, ingest reads local cache files from the configured Cursor roots.
Cursor sync is optional. If configured via `tokctl cursor login`, normal report/UI flows can refresh Cursor usage into `~/.config/tokctl/cursor-cache/` before ingest. If sync fails, `tokctl` falls back to whatever local Cursor CSV cache already exists.

## Pricing and schema

Prices: [`src/pricing.rs`](src/pricing.rs). PRs welcome for new models.

When you change anything that affects stored cost, bump **`SCHEMA_VERSION`** in [`src/store/schema.rs`](src/store/schema.rs) in the **same commit** so the next run rebuilds aggregates. Use `tokctl … --rebuild` if you ever need a manual full reset.

`meta.schema_version` tracks the DB; some upgrades apply in place without re-parsing JSONL.

## SQLite

```sh
sqlite3 "$(tokctl export-db)"
```

**Tables:** `events` (token rows; `repo` = canonical key), `files`, `repos` (`display_name`, `origin_url`, `first_seen`), `meta`. **Joins:** `events.file_path = files.path`, `events.repo = repos.key`.

## Development

```sh
cargo test
cargo clippy
cargo fmt
cargo bench    # parser benches (criterion)
```

## Not in scope

No commit ↔ session linking. **UI:** keyboard-first, no mouse, no file-watch (refresh with `r`).
