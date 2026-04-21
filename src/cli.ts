#!/usr/bin/env node
import { Command } from 'commander';
import { createRequire } from 'node:module';
import { ingestClaude } from './sources/claude.js';
import { ingestCodex } from './sources/codex.js';
import { dailyReport } from './reports/daily.js';
import { monthlyReport } from './reports/monthly.js';
import { sessionReport } from './reports/session.js';
import { filterByDate, parseSince, parseUntil } from './reports/shared.js';
import { renderJson, renderTable, renderWarnings, ReportKind } from './render.js';
import type { Source, UsageEvent, IngestStats } from './types.js';

const require = createRequire(import.meta.url);
const pkg = require('../package.json') as { version: string };

interface GlobalOpts {
  source?: 'claude' | 'codex' | 'all';
  since?: string;
  until?: string;
  json?: boolean;
  claudeDir?: string;
  codexDir?: string;
}

async function runReport(kind: ReportKind, opts: GlobalOpts): Promise<void> {
  const source: 'all' | Source = opts.source ?? 'all';
  let since: Date | undefined;
  let until: Date | undefined;
  try {
    since = parseSince(opts.since);
    until = parseUntil(opts.until);
  } catch (err) {
    process.stderr.write(`error: ${(err as Error).message}\n`);
    process.exit(1);
  }

  const mergedStats: IngestStats = { skippedLines: 0, unknownModels: new Set() };
  const events: UsageEvent[] = [];

  try {
    if (source === 'claude' || source === 'all') {
      const run = await ingestClaude({ flag: opts.claudeDir, env: process.env });
      events.push(...run.events);
      mergedStats.skippedLines += run.stats.skippedLines;
    }
    if (source === 'codex' || source === 'all') {
      const run = await ingestCodex({ flag: opts.codexDir, env: process.env });
      events.push(...run.events);
      mergedStats.skippedLines += run.stats.skippedLines;
    }
  } catch (err) {
    process.stderr.write(`error: ${(err as Error).message}\n`);
    process.exit(2);
  }

  const filtered = filterByDate(events, since, until);
  const unknown = mergedStats.unknownModels;
  const rows =
    kind === 'daily'
      ? dailyReport(filtered, unknown)
      : kind === 'monthly'
        ? monthlyReport(filtered, unknown)
        : sessionReport(filtered, unknown);

  const showSource = source === 'all';
  if (opts.json) {
    process.stdout.write(renderJson(rows, kind, showSource) + '\n');
  } else {
    process.stdout.write(renderTable(rows, kind, showSource) + '\n');
  }
  for (const w of renderWarnings(unknown, mergedStats.skippedLines)) {
    process.stderr.write(w + '\n');
  }
}

function attachCommonFlags(cmd: Command): Command {
  return cmd
    .option('--source <source>', 'claude | codex | all', 'all')
    .option('--since <date>', 'inclusive lower bound (YYYY-MM-DD, local time)')
    .option('--until <date>', 'inclusive upper bound (YYYY-MM-DD, local time)')
    .option('--json', 'emit machine-readable JSON instead of a table', false)
    .option('--claude-dir <paths>', 'one or more comma-separated Claude project roots')
    .option('--codex-dir <paths>', 'one or more comma-separated Codex session roots');
}

const program = new Command();
program
  .name('aiusage')
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

program.parseAsync(process.argv).catch((err) => {
  process.stderr.write(`error: ${(err as Error).message}\n`);
  process.exit(2);
});
