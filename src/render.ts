import Table from 'cli-table3';
import type { AggregateRow } from './store/rowTypes.js';
import type { Source } from './types.js';

export type ReportKind = 'daily' | 'monthly' | 'session';

export interface RenderOptions {
  kind: ReportKind;
  sourceFilter: 'all' | Source;
  asJson: boolean;
}

const fmtNum = (n: number): string => n.toLocaleString('en-US');
const fmtCost = (n: number): string => `$${n.toFixed(2)}`;

export function toJsonRows(rows: AggregateRow[], kind: ReportKind, showSource: boolean): unknown[] {
  return rows.map((r) => {
    const base: Record<string, unknown> = {};
    const keyName = kind === 'daily' ? 'date' : kind === 'monthly' ? 'month' : 'session_id';
    base[keyName] = r.key;
    if (kind === 'session') {
      base.source = r.source;
      base.project_path = r.projectPath ?? null;
      base.latest_timestamp = r.latestTimestamp?.toISOString() ?? null;
    } else if (showSource) {
      base.source = r.source ?? 'all';
    }
    base.input = r.inputTokens;
    base.output = r.outputTokens;
    base.cache_read = r.cacheReadTokens;
    base.cache_write = r.cacheWriteTokens;
    base.totalTokens = r.totalTokens;
    base.costUsd = Number(r.costUsd.toFixed(4));
    return base;
  });
}

export function renderJson(rows: AggregateRow[], kind: ReportKind, showSource: boolean): string {
  return JSON.stringify(toJsonRows(rows, kind, showSource), null, 2);
}

export function renderTable(
  rows: AggregateRow[],
  kind: ReportKind,
  showSource: boolean,
): string {
  const keyHeader = kind === 'daily' ? 'date' : kind === 'monthly' ? 'month' : 'session';
  const head: string[] =
    kind === 'session'
      ? ['session', 'source', 'project', 'last_activity', 'input', 'output', 'cache_read', 'cache_write', 'total', 'cost_usd']
      : showSource
        ? [keyHeader, 'source', 'input', 'output', 'cache_read', 'cache_write', 'total', 'cost_usd']
        : [keyHeader, 'input', 'output', 'cache_read', 'cache_write', 'total', 'cost_usd'];

  const table = new Table({ head });
  for (const r of rows) {
    if (kind === 'session') {
      table.push([
        r.key.slice(0, 8),
        r.source ?? '',
        r.projectPath ?? '',
        r.latestTimestamp?.toISOString().replace('T', ' ').slice(0, 19) ?? '',
        fmtNum(r.inputTokens),
        fmtNum(r.outputTokens),
        fmtNum(r.cacheReadTokens),
        fmtNum(r.cacheWriteTokens),
        fmtNum(r.totalTokens),
        fmtCost(r.costUsd),
      ]);
    } else if (showSource) {
      table.push([
        r.key,
        r.source ?? 'all',
        fmtNum(r.inputTokens),
        fmtNum(r.outputTokens),
        fmtNum(r.cacheReadTokens),
        fmtNum(r.cacheWriteTokens),
        fmtNum(r.totalTokens),
        fmtCost(r.costUsd),
      ]);
    } else {
      table.push([
        r.key,
        fmtNum(r.inputTokens),
        fmtNum(r.outputTokens),
        fmtNum(r.cacheReadTokens),
        fmtNum(r.cacheWriteTokens),
        fmtNum(r.totalTokens),
        fmtCost(r.costUsd),
      ]);
    }
  }

  if (rows.length > 0) {
    const tot = rows.reduce(
      (acc, r) => {
        acc.input += r.inputTokens;
        acc.output += r.outputTokens;
        acc.cacheRead += r.cacheReadTokens;
        acc.cacheWrite += r.cacheWriteTokens;
        acc.total += r.totalTokens;
        acc.cost += r.costUsd;
        return acc;
      },
      { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0, cost: 0 },
    );
    if (kind === 'session') {
      table.push(['TOTAL', '', '', '', fmtNum(tot.input), fmtNum(tot.output), fmtNum(tot.cacheRead), fmtNum(tot.cacheWrite), fmtNum(tot.total), fmtCost(tot.cost)]);
    } else if (showSource) {
      table.push(['TOTAL', '', fmtNum(tot.input), fmtNum(tot.output), fmtNum(tot.cacheRead), fmtNum(tot.cacheWrite), fmtNum(tot.total), fmtCost(tot.cost)]);
    } else {
      table.push(['TOTAL', fmtNum(tot.input), fmtNum(tot.output), fmtNum(tot.cacheRead), fmtNum(tot.cacheWrite), fmtNum(tot.total), fmtCost(tot.cost)]);
    }
  }

  return table.toString();
}

export function renderWarnings(unknownModels: Set<string>, skippedLines: number): string[] {
  const warnings: string[] = [];
  if (unknownModels.size > 0) {
    const list = Array.from(unknownModels).sort().join(', ');
    warnings.push(`warning: no price for model(s): ${list} (cost treated as 0)`);
  }
  if (skippedLines > 0) {
    warnings.push(`warning: skipped ${skippedLines} malformed JSONL line(s)`);
  }
  return warnings;
}
