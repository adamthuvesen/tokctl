# tokctl

Token and cost reports from **Claude** and **Codex** JSONL session logs. **Local-only** (no network). Incremental ingest into a **SQLite** cache; warm runs stay fast.

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

# filters & pivots (report commands: daily, monthly, session, repo)
tokctl daily --source claude
tokctl monthly --since 2026-01-01 --until 2026-03-31
tokctl daily --repo tokctl                    # name, path prefix, or "(no-repo)"
tokctl daily --group-by repo                  # same rollup as `tokctl repo`

# output & inputs
tokctl daily --json
tokctl daily --claude-dir /a,/b --codex-dir ~/.codex/sessions

# cache
tokctl daily --rebuild       # wipe DB, full re-ingest
tokctl daily --no-cache      # in-memory only, no SQLite
tokctl export-db             # print cache path (does not create DB)
```

## What you get

- **Rollups:** day, month, session, or repo; `tokctl repo <name>` drills to sessions.
- **JSON** (`--json`) for scripts; tables default for the terminal.
- **Ingest:** parallel parse of JSONL, serial writes to SQLite; mmap on large files with a safety window for recently touched files.
- **Repos:** nearest `.git` ancestor from each event’s project path (Claude) or `cwd` (Codex); symlink-normalized keys; `(no-repo)` when nothing matches.
- **Cost:** static model table; unknown models count as **$0** with a trailing warning.

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

Browses the SQLite cache; **no JSONL ingest** from the UI (run a CLI report first to populate). **TTY only** — exits if stdout is not a terminal.

Two panes: axis on the left (repo → day → model → session via `Tab`), sessions for the selection on the right. Press `?` for in-app help.

```
┌─ tokctl  2026-04-22 16:30  last 30 days · $84.12 · 1.24M tok ──────────────[?]─┐
│  [ REPOS ]                             │  SESSIONS                             │
│  ▸ tokctl                     $41.22   │  3m ago    claude  tokctl       $8.20 │
│    cortex                     $18.40   │  2h ago    codex   my-project    $3.10│
│    (no-repo)                  $ 9.03   │  yesterday claude  api-service   $1.40│
│  ▁▂▂▃▇▅▃▂▁▂▃▅▇▆▃  cost/day last 30d · window:month · source:all                │
│  j/k move  ↵ drill  h/l pane  / filter  Tab axis  t trend  T/w/m/Y/a window    │
└────────────────────────────────────────────────────────────────────────────────┘
```

| Keys | Action |
| --- | --- |
| `h` `l` / arrows | panes |
| `j` `k` / arrows | move |
| `g` `g` / `G` | top / bottom |
| `Ctrl-d` / `Ctrl-u` | half page |
| `Enter` | drill |
| `Esc` / `Backspace` | back / clear filter |
| `/` | fuzzy filter |
| `Tab` | cycle left axis |
| `s` | sort cycle |
| `t` | trend overlay (`d`/`w`/`m`/`y` inside) |
| `T` `w` `m` `Y` `a` | window: today / week / month / year / all |
| `1` `2` `3` | source: all / claude / codex |
| `r` | re-query cache (no ingest) |
| `y` | yank row key (needs `clipboard` feature) |
| `?` | help overlay |
| `q` / `Ctrl-c` | quit |

Preferences live in `<cache_dir>/ui_state.json` next to `cache.db` (delete to reset).

**Clipboard:** default build includes [`arboard`](https://crates.io/crates/arboard). Headless Linux: `cargo install --path . --no-default-features` — then `y` is a no-op.

## Data locations

| | Default | Override |
| --- | --- | --- |
| Claude projects | `~/.claude/projects/`, `~/.config/claude/projects/` | `--claude-dir` (csv), `TOKCTL_CLAUDE_DIR` (csv), `CLAUDE_CONFIG_DIR` (csv → each root + `/projects`) |
| Codex sessions | `~/.codex/sessions/` | `--codex-dir` (csv), `TOKCTL_CODEX_DIR` (csv), `CODEX_HOME` (+ `/sessions`) |
| Cache file | `$XDG_DATA_HOME/tokctl/cache.db` or `~/.local/share/tokctl/cache.db` | `TOKCTL_CACHE_DIR` (directory → `<dir>/cache.db`) |

Desktop **Claude.app** / **Codex.app** use the same JSONL locations as the CLIs. `~/Library/Application Support/{Claude,Codex}/` is UI-only — not parsed.

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

No Cursor logs; no commit ↔ session linking. **UI:** keyboard-first, no mouse, no file-watch (refresh with `r`).
