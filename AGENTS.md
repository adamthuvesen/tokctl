# Agent instructions (tokctl)

## Project

`tokctl` is a **local-only Node CLI** (TypeScript, ESM) that reads Claude/Codex JSONL session logs and prints token/cost reports. Persistence is a **SQLite** cache (`better-sqlite3`); default runs use incremental ingest + SQL reports; **`--no-cache`** uses the in-memory path without touching the DB.

## Layout

- `src/cli.ts` — entry, `--rebuild` / `--no-cache` routing
- `src/ingest/` — discovery, ingest plan, `runIngest`
- `src/store/` — SQLite schema, writes, queries
- `src/sources/` — full-file parsers used by `--no-cache` and shared parse helpers
- `src/legacy/inMemory.ts` — aggregations for `--no-cache`
- `openspec/specs/` — behavior specs (usage-ingest, cache-store, cli, usage-reports)

## Commands

```sh
npm run build      # tsc -> dist/
npm test           # vitest
npm run typecheck  # tsc --noEmit
```

## Change discipline

- Prefer the **smallest** diff that satisfies requirements; match existing patterns in `src/`.
- Do not add network calls or shipping of secrets; cache path is local only.
- For behavior changes, align with **OpenSpec** under `openspec/specs/` when applicable.

## Global policy

Where this file is silent, follow the repository maintainer’s global **AGENTS.md** / Cursor rules for commits (no push without ask), security, and workflow.
