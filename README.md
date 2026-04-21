# tokctl

Local-only CLI that reports token usage and cost across **Claude Code**, **Claude Desktop**, **Codex CLI**, and **Codex Desktop** on macOS/Linux. Reads JSONL session logs from disk; no network calls. A small SQLite cache keeps warm runs well under 200 ms.

## Install

```sh
npm install
npm run build
npm link   # makes `tokctl` available on your PATH
```

`npm install` builds a native addon (`better-sqlite3`), so Xcode command-line tools or equivalent are required. Remove them with `npm rebuild` if you ever need to force a rebuild.

## Usage

```sh
tokctl daily               # tokens + cost grouped by date
tokctl monthly             # grouped by YYYY-MM
tokctl session             # grouped by session id

# filter one tool
tokctl daily --source claude
tokctl daily --source codex

# narrow to a date range
tokctl monthly --since 2026-01-01 --until 2026-03-31

# JSON output for scripts
tokctl daily --json

# multiple or alternate directories
tokctl daily --claude-dir /path/a,/path/b
tokctl daily --codex-dir $CODEX_HOME/sessions

# cache controls
tokctl daily --rebuild     # delete the cache DB and re-ingest from scratch
tokctl daily --no-cache    # bypass the cache for this invocation
tokctl export-db           # print the absolute path of the cache DB
```

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

## Default paths

| Source | Default roots | Env overrides |
|---|---|---|
| Claude | `~/.claude/projects/`, `~/.config/claude/projects/` | `TOKCTL_CLAUDE_DIR` (csv), `CLAUDE_CONFIG_DIR` (csv, ccusage-compatible) |
| Codex  | `~/.codex/sessions/` | `TOKCTL_CODEX_DIR` (csv), `CODEX_HOME` (single path, `/sessions` appended) |
| Cache  | `$XDG_CACHE_HOME/tokctl/tokctl.db` (or `~/.cache/tokctl/tokctl.db`) | `TOKCTL_CACHE_DIR` |

**macOS Desktop apps are covered automatically** — both `/Applications/Claude.app` and `/Applications/Codex.app` write their session JSONL to the same paths as the CLIs. The Electron data under `~/Library/Application Support/{Claude,Codex}/` holds only UI metadata (no token buckets) and is deliberately not parsed.

## How fast is it?

On a reference dataset (Mac, 968 MB of Claude JSONL + 1.3 GB of Codex JSONL):

| command | before cache | cold | warm |
|---|---|---|---|
| `daily --source codex` | 3.94 s | 3.6 s | **~95 ms** |
| `daily --source claude` | 2.49 s | 4.9 s | **~175 ms** |
| `daily` (both) | 5.22 s | 9.3 s | **~160 ms** |

"Cold" = first run ever or after `--rebuild`. "Warm" = a typical repeat run; only today's open session file is scanned for new bytes. The cache is ~5-20 MB after a year of heavy use.

## Prices

Model prices are a hand-maintained table at [`src/pricing.ts`](src/pricing.ts). Unknown models contribute `0` to the cost and are listed in a trailing warning line. Open a PR to that file when a new model shows up. **Changing a price requires a cache rebuild** — bump `SCHEMA_VERSION` in [`src/store/schema.ts`](src/store/schema.ts) in the same commit so the next run rebuilds with the new prices.

## Debugging / ad-hoc queries

```sh
sqlite3 "$(tokctl export-db)"
```

Useful tables: `events` (one row per token-bearing turn), `files` (per-JSONL manifest), `meta` (schema version). All joins are on `events.file_path = files.path`.

## Scope

v1 intentionally does not do: Cursor, per-repo roll-ups beyond what Claude's folder layout gives for free, weekly narrative, or git-activity correlation. Those live in follow-up OpenSpec changes.
