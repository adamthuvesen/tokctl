# aiusage

Local-only CLI that reports token usage and cost across **Claude Code**, **Claude Desktop**, **Codex CLI**, and **Codex Desktop** on macOS/Linux. Reads JSONL session logs from disk; no network calls.

## Install

```sh
npm install
npm run build
npm link   # makes `aiusage` available on your PATH
```

## Usage

```sh
aiusage daily               # tokens + cost grouped by date
aiusage monthly             # grouped by YYYY-MM
aiusage session             # grouped by session id

# filter one tool
aiusage daily --source claude
aiusage daily --source codex

# narrow to a date range
aiusage monthly --since 2026-01-01 --until 2026-03-31

# JSON output for scripts
aiusage daily --json

# multiple or alternate directories
aiusage daily --claude-dir /path/a,/path/b
aiusage daily --codex-dir $CODEX_HOME/sessions
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
| Claude | `~/.claude/projects/`, `~/.config/claude/projects/` | `AIUSAGE_CLAUDE_DIR` (csv), `CLAUDE_CONFIG_DIR` (csv, ccusage-compatible) |
| Codex  | `~/.codex/sessions/` | `AIUSAGE_CODEX_DIR` (csv), `CODEX_HOME` (single path, `/sessions` appended) |

**macOS Desktop apps are covered automatically** — both `/Applications/Claude.app` and `/Applications/Codex.app` write their session JSONL to the same paths as the CLIs. The Electron data under `~/Library/Application Support/{Claude,Codex}/` holds only UI metadata (no token buckets) and is deliberately not parsed.

## Prices

Model prices are a hand-maintained table at [`src/pricing.ts`](src/pricing.ts). Unknown models contribute `0` to the cost and are listed in a trailing warning line. Open a PR to that file when a new model shows up.

## Scope

v1 intentionally does not do: Cursor, per-repo roll-ups beyond what Claude's folder layout gives for free, weekly narrative, or git-activity correlation. Those live in follow-up OpenSpec changes.
