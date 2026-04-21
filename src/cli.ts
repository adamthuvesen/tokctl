#!/usr/bin/env node
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { Command } from 'commander';
import { createRequire } from 'node:module';
import { ingestClaude } from './sources/claude.js';
import { ingestCodex } from './sources/codex.js';
import { parseSince, parseUntil } from './dates.js';
import { closeStore, openStore } from './store/db.js';
import { resolveCachePath } from './store/path.js';
import {
  dailyReportFromDb,
  monthlyReportFromDb,
  sessionReportFromDb,
  type QueryFilter,
} from './store/queries.js';
import { collectUnknownModelsFromDb, runIngest } from './ingest/run.js';
import {
  dailyInMemory,
  filterByDate,
  monthlyInMemory,
  sessionInMemory,
} from './legacy/inMemory.js';
import { renderJson, renderTable, renderWarnings, type ReportKind } from './render.js';
import type { Source, UsageEvent } from './types.js';

const require = createRequire(import.meta.url);
const pkg = require('../package.json') as { version: string };

interface GlobalOpts {
  source?: 'claude' | 'codex' | 'all';
  since?: string;
  until?: string;
  json?: boolean;
  claudeDir?: string;
  codexDir?: string;
  rebuild?: boolean;
  // Commander stores `--no-cache` under the key `cache` (truthy by default,
  // falsy when the flag is passed). We look at process.argv directly for a
  // stable signal.
}

function noCacheRequested(): boolean {
  return process.argv.includes('--no-cache');
}

function resolveDefaultClaudeRoots(env: NodeJS.ProcessEnv): string[] {
  return [
    path.join(os.homedir(), '.claude', 'projects'),
    path.join(os.homedir(), '.config', 'claude', 'projects'),
  ];
}

function resolveDefaultCodexRoots(env: NodeJS.ProcessEnv): string[] {
  return [path.join(os.homedir(), '.codex', 'sessions')];
}

interface ResolvedRoots {
  roots: string[];
  userSupplied: boolean;
}

function resolveRootsFor(
  flag: string | undefined,
  tokctlEnv: string | undefined,
  toolEnv: string | undefined,
  toolEnvSuffix: string | undefined,
  defaults: string[],
): ResolvedRoots {
  const expand = (p: string): string => (p.startsWith('~/') ? path.join(os.homedir(), p.slice(2)) : p);
  const split = (v: string): string[] =>
    v.split(',').map((s) => s.trim()).filter(Boolean).map(expand);

  if (flag && flag.trim()) return { roots: split(flag), userSupplied: true };
  if (tokctlEnv && tokctlEnv.trim()) return { roots: split(tokctlEnv), userSupplied: true };
  if (toolEnv && toolEnv.trim()) {
    const parts = split(toolEnv);
    return {
      roots: toolEnvSuffix ? parts.map((p) => path.join(p, toolEnvSuffix)) : parts,
      userSupplied: true,
    };
  }
  return { roots: defaults, userSupplied: false };
}

function existingRoots(resolved: ResolvedRoots): string[] {
  const out: string[] = [];
  for (const r of resolved.roots) {
    try {
      const st = fs.statSync(r);
      if (st.isDirectory()) out.push(r);
      else if (resolved.userSupplied) throw new Error(`not a directory: ${r}`);
    } catch (err) {
      if (resolved.userSupplied) throw err;
      // silent for defaults
    }
  }
  return out;
}

async function runReportFromCacheAsync(kind: ReportKind, opts: GlobalOpts): Promise<void> {
  const source: Source | 'all' = opts.source ?? 'all';
  let since: Date | undefined;
  let until: Date | undefined;
  try {
    since = parseSince(opts.since);
    until = parseUntil(opts.until);
  } catch (err) {
    process.stderr.write(`error: ${(err as Error).message}\n`);
    process.exit(1);
  }

  const cachePath = resolveCachePath(process.env);
  if (opts.rebuild) {
    try {
      fs.unlinkSync(cachePath);
    } catch {
      // ignore
    }
  }

  const claudeRoots = resolveRootsFor(
    opts.claudeDir,
    process.env.TOKCTL_CLAUDE_DIR,
    process.env.CLAUDE_CONFIG_DIR,
    'projects',
    resolveDefaultClaudeRoots(process.env),
  );
  const codexRoots = resolveRootsFor(
    opts.codexDir,
    process.env.TOKCTL_CODEX_DIR,
    process.env.CODEX_HOME,
    'sessions',
    resolveDefaultCodexRoots(process.env),
  );

  let claudeExisting: string[];
  let codexExisting: string[];
  try {
    claudeExisting = existingRoots(claudeRoots);
    codexExisting = existingRoots(codexRoots);
  } catch (err) {
    process.stderr.write(`error: ${(err as Error).message}\n`);
    process.exit(2);
  }

  const db = openStore({
    path: cachePath,
    onRebuild: (reason) => {
      process.stderr.write(`notice: rebuilding cache (${reason})\n`);
    },
  });

  const includeClaude = source === 'all' || source === 'claude';
  const includeCodex = source === 'all' || source === 'codex';

  let stats: Awaited<ReturnType<typeof runIngest>>;
  try {
    stats = await runIngest({
      db,
      claudeRoots: claudeExisting,
      codexRoots: codexExisting,
      includeClaude,
      includeCodex,
    });
  } catch (err) {
    closeStore(db);
    process.stderr.write(`error: ${(err as Error).message}\n`);
    process.exit(2);
  }

  const filter: QueryFilter = {
    source: source === 'all' ? null : source,
    sinceMs: since?.getTime() ?? null,
    untilMs: until?.getTime() ?? null,
  };

  const rows =
    kind === 'daily'
      ? dailyReportFromDb(db, filter)
      : kind === 'monthly'
        ? monthlyReportFromDb(db, filter)
        : sessionReportFromDb(db, filter);

  const showSource = source === 'all';
  if (opts.json) {
    process.stdout.write(renderJson(rows, kind, showSource) + '\n');
  } else {
    process.stdout.write(renderTable(rows, kind, showSource) + '\n');
  }

  const unknownDb = collectUnknownModelsFromDb(db, source === 'all' ? null : source);
  for (const m of stats.unknownModels) unknownDb.add(m);
  for (const w of renderWarnings(unknownDb, stats.skippedLines)) {
    process.stderr.write(w + '\n');
  }

  closeStore(db);
}

async function runReportNoCache(kind: ReportKind, opts: GlobalOpts): Promise<void> {
  const source: Source | 'all' = opts.source ?? 'all';
  let since: Date | undefined;
  let until: Date | undefined;
  try {
    since = parseSince(opts.since);
    until = parseUntil(opts.until);
  } catch (err) {
    process.stderr.write(`error: ${(err as Error).message}\n`);
    process.exit(1);
  }

  const skippedLines = { n: 0 };
  const unknownModels = new Set<string>();
  const events: UsageEvent[] = [];

  try {
    if (source === 'claude' || source === 'all') {
      const run = await ingestClaude({ flag: opts.claudeDir, env: process.env });
      events.push(...run.events);
      skippedLines.n += run.stats.skippedLines;
    }
    if (source === 'codex' || source === 'all') {
      const run = await ingestCodex({ flag: opts.codexDir, env: process.env });
      events.push(...run.events);
      skippedLines.n += run.stats.skippedLines;
    }
  } catch (err) {
    process.stderr.write(`error: ${(err as Error).message}\n`);
    process.exit(2);
  }

  const filtered = filterByDate(events, since, until);
  const sourceLabel: Source | 'all' = source === 'all' ? 'all' : source;
  const rows =
    kind === 'daily'
      ? dailyInMemory(filtered, sourceLabel, unknownModels)
      : kind === 'monthly'
        ? monthlyInMemory(filtered, sourceLabel, unknownModels)
        : sessionInMemory(filtered, unknownModels);

  const showSource = source === 'all';
  if (opts.json) {
    process.stdout.write(renderJson(rows, kind, showSource) + '\n');
  } else {
    process.stdout.write(renderTable(rows, kind, showSource) + '\n');
  }
  for (const w of renderWarnings(unknownModels, skippedLines.n)) {
    process.stderr.write(w + '\n');
  }
}

async function runReport(kind: ReportKind, opts: GlobalOpts): Promise<void> {
  const noCache = noCacheRequested();
  if (opts.rebuild && noCache) {
    process.stderr.write('error: --rebuild and --no-cache are mutually exclusive\n');
    process.exit(1);
  }
  if (noCache) {
    await runReportNoCache(kind, opts);
  } else {
    await runReportFromCacheAsync(kind, opts);
  }
}

function attachCommonFlags(cmd: Command): Command {
  return cmd
    .option('--source <source>', 'claude | codex | all', 'all')
    .option('--since <date>', 'inclusive lower bound (YYYY-MM-DD, local time)')
    .option('--until <date>', 'inclusive upper bound (YYYY-MM-DD, local time)')
    .option('--json', 'emit machine-readable JSON instead of a table', false)
    .option('--claude-dir <paths>', 'one or more comma-separated Claude project roots')
    .option('--codex-dir <paths>', 'one or more comma-separated Codex session roots')
    .option('--rebuild', 'delete the cache DB before running', false)
    .option('--no-cache', 'bypass the cache for this invocation (sets cache=false)');
}

const program = new Command();
program
  .name('tokctl')
  .description('Token usage and cost report for Claude Code, Claude Desktop, Codex CLI, and Codex Desktop.')
  .version(pkg.version);

attachCommonFlags(program.command('daily').description('aggregate by date (YYYY-MM-DD, local time)'))
  .action(async (opts: GlobalOpts) => {
    await runReport('daily', opts);
  });

attachCommonFlags(program.command('monthly').description('aggregate by month (YYYY-MM, local time)'))
  .action(async (opts: GlobalOpts) => {
    await runReport('monthly', opts);
  });

attachCommonFlags(program.command('session').description('aggregate by session id (sorted by latest activity)'))
  .action(async (opts: GlobalOpts) => {
    await runReport('session', opts);
  });

program
  .command('export-db')
  .description('print the absolute path of the cache DB (does not create it)')
  .action(() => {
    process.stdout.write(resolveCachePath(process.env) + '\n');
  });

program.parseAsync(process.argv).catch((err) => {
  process.stderr.write(`error: ${(err as Error).message}\n`);
  process.exit(2);
});
