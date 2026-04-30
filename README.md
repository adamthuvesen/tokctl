# tokctl

A terminal dashboard for what you actually spent on **Claude**, **Codex**, and **Cursor** — built on the JSONL/CSV logs already on your disk. SQLite-cached, keyboard-driven, local-only.

```sh
tokctl ui
```

That's the whole pitch. Open it, drill into a day, repo, or session — three levels deep, all the way down to the individual turn — and see where the money went.

---

## Install

```sh
cargo install --path .
# or
cargo build --release && ln -sf "$(pwd)/target/release/tokctl" ~/.local/bin/tokctl
```

Rust 1.75+, edition 2021. SQLite is bundled — no system `libsqlite` needed.

First run ingests your local Claude/Codex JSONL and any Cursor CSV cache into `$XDG_DATA_HOME/tokctl/cache.db`. Subsequent runs are incremental.

---

## UI

| Keys | Action |
| --- | --- |
| `j` `k` / arrows | move within the focused area |
| `[` / `]` | previous / next section |
| `h` `←` / `l` `→` | focus sidebar / focus main |
| `Enter` | drill (sidebar→main; main pushes one level: section → sessions → events) |
| `Esc` / `Backspace` | pop drill (one level), close overlay, cancel filter |
| `g` `g` / `G` | top / bottom |
| `Ctrl-d` / `Ctrl-u` | half page |
| `Tab` | cycle main-pane tabs |
| `t` | jump to Provider section |
| `d` `w` `m` `y` | bucket: day / week / month / year |
| `T` `W` `M` `z` `a` | window: today / week / month / year / all |
| `1` `2` `3` `4` | source: all / claude / codex / cursor |
| `s` / `e` | sort cycle / compact-vs-expanded |
| `/` | fuzzy filter |
| `r` | re-query cache (no ingest) |
| `i` | row details |
| `y` `Y` | yank row key / summary (needs `clipboard` feature) |
| `?` | help overlay |
| `q` / `Ctrl-c` | quit |

UI preferences persist to `<cache_dir>/ui_state.json` (delete to reset). Default builds include [`arboard`](https://crates.io/crates/arboard); on headless Linux `cargo install --path . --no-default-features` and `y` becomes a no-op.

**Drill levels.** The main pane is a stack: pick a row in any section (except `Provider`) and `Enter` drills one level deeper. Repos / Days / Models drill into their session list; Sessions drills directly into per-turn events; inside a sessions drill another `Enter` drills into events. The breadcrumb shows where you are (`Repos › tokctl › 72a0a659…`); `Esc` or `←` pops one level at a time.

---

## Cursor sync

Cursor doesn't write JSONL, so `tokctl` reads CSV exports from a local cache directory. You can drop exports there manually, or let `tokctl` fetch them:

```sh
tokctl cursor login    # one-time, stores a session token
tokctl cursor status   # check what's cached and when it was synced
tokctl cursor sync     # pull latest CSVs into the local cache
```

Normal `tokctl ui` runs will refresh Cursor automatically when credentials are configured. If sync fails, ingest falls back to whatever CSVs are already cached.

---

## CLI

Same data, no UI. Useful for scripts, diffs, or piping into something else.

```sh
tokctl daily                              # by calendar day
tokctl monthly                            # by YYYY-MM
tokctl session                            # by session id
tokctl repo                               # by git repo, cost desc
tokctl repo tokctl                        # sessions inside one repo
tokctl compare                            # this month-to-date vs comparable previous month
tokctl compare this-week last-week --by repo
tokctl doctor                             # roots, cache, pricing, sync readiness

# filter / pivot any of the above
tokctl daily --source claude
tokctl daily --since 2026-01-01 --until 2026-03-31
tokctl daily --repo tokctl                # name, path prefix, or "(no-repo)"
tokctl daily --group-by repo

# output
tokctl daily --json
tokctl daily --claude-dir /a,/b --codex-dir ~/.codex/sessions

# cache
tokctl daily --rebuild                    # wipe DB, full re-ingest
tokctl daily --no-cache                   # in-memory only, skip SQLite
tokctl export-db                          # print cache path (does not create DB)
```

`tokctl doctor` is the one to run if anything looks off — it checks roots, discovered inputs, cache health, pricing coverage, and Cursor sync readiness without mutating local data.

---

## Where data lives

| | Default | Override |
| --- | --- | --- |
| Claude projects | `~/.claude/projects/`, `~/.config/claude/projects/` | `--claude-dir` (csv), `TOKCTL_CLAUDE_DIR`, `CLAUDE_CONFIG_DIR` (each + `/projects`) |
| Codex sessions | `~/.codex/sessions/` | `--codex-dir` (csv), `TOKCTL_CODEX_DIR`, `CODEX_HOME` (+ `/sessions`) |
| Cursor CSV cache | `~/.config/tokctl/cursor-cache/`, `~/.config/tokscale/cursor-cache/` | `--cursor-dir` (csv), `TOKCTL_CURSOR_DIR` |
| SQLite cache | `$XDG_DATA_HOME/tokctl/cache.db` or `~/.local/share/tokctl/cache.db` | `TOKCTL_CACHE_DIR` |

Desktop **Claude.app** and **Codex.app** write to the same JSONL locations as the CLIs. `~/Library/Application Support/{Claude,Codex}/` is UI-only and not parsed.

---

## How it works (briefly)

- **Repo attribution:** nearest `.git` ancestor of each event's project path (Claude) or `cwd` (Codex), symlink-normalized. No match → `(no-repo)`.
- **Ingest:** parallel parse, serial writes. Files ≥ 1 MB are mmapped; recently modified files bypass mmap via a safety window in the ingest plan.
- **Pricing:** static table in [`src/pricing.rs`](src/pricing.rs) for Claude/Codex; Cursor preserves the row cost from the CSV when present. Unknown priced models count as **$0** with a trailing warning. PRs welcome for new rates.
- **Schema:** when you change anything that affects stored cost, bump `SCHEMA_VERSION` in [`src/store/schema.rs`](src/store/schema.rs) in the same commit so the next run rebuilds aggregates. `--rebuild` forces a full reset.

### Poking at SQLite directly

```sh
sqlite3 "$(tokctl export-db)"
```

Tables: `events` (token rows; `repo` = canonical key), `files`, `repos` (`display_name`, `origin_url`, `first_seen`), `meta`. Joins: `events.file_path = files.path`, `events.repo = repos.key`.


