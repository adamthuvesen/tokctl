import { costOf } from '../pricing.js';
import type { Source, UsageEvent } from '../types.js';

export interface AggregateRow {
  key: string;
  source?: Source;
  projectPath?: string;
  latestTimestamp?: Date;
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheWriteTokens: number;
  totalTokens: number;
  costUsd: number;
}

export function filterByDate(
  events: UsageEvent[],
  since?: Date,
  until?: Date,
): UsageEvent[] {
  return events.filter((e) => {
    if (since && e.timestamp < since) return false;
    if (until && e.timestamp > until) return false;
    return true;
  });
}

export function parseSince(value: string | undefined): Date | undefined {
  if (!value) return undefined;
  if (!/^\d{4}-\d{2}-\d{2}$/.test(value)) {
    throw new Error(`--since must be YYYY-MM-DD, got "${value}"`);
  }
  const d = new Date(`${value}T00:00:00`);
  if (Number.isNaN(d.getTime())) {
    throw new Error(`--since not a valid date: "${value}"`);
  }
  return d;
}

export function parseUntil(value: string | undefined): Date | undefined {
  if (!value) return undefined;
  if (!/^\d{4}-\d{2}-\d{2}$/.test(value)) {
    throw new Error(`--until must be YYYY-MM-DD, got "${value}"`);
  }
  const d = new Date(`${value}T23:59:59.999`);
  if (Number.isNaN(d.getTime())) {
    throw new Error(`--until not a valid date: "${value}"`);
  }
  return d;
}

export function aggregate(
  events: UsageEvent[],
  keyFor: (e: UsageEvent) => string,
  unknownModels: Set<string>,
): Map<string, AggregateRow> {
  const map = new Map<string, AggregateRow>();
  for (const e of events) {
    const key = keyFor(e);
    let row = map.get(key);
    if (!row) {
      row = {
        key,
        inputTokens: 0,
        outputTokens: 0,
        cacheReadTokens: 0,
        cacheWriteTokens: 0,
        totalTokens: 0,
        costUsd: 0,
      };
      map.set(key, row);
    }
    row.inputTokens += e.inputTokens;
    row.outputTokens += e.outputTokens;
    row.cacheReadTokens += e.cacheReadTokens;
    row.cacheWriteTokens += e.cacheWriteTokens;
    row.totalTokens += e.inputTokens + e.outputTokens + e.cacheReadTokens + e.cacheWriteTokens;
    row.costUsd += costOf(e, unknownModels);
    if (!row.latestTimestamp || e.timestamp > row.latestTimestamp) {
      row.latestTimestamp = e.timestamp;
    }
  }
  return map;
}

function pad2(n: number): string {
  return n < 10 ? `0${n}` : String(n);
}

export function localDateKey(d: Date): string {
  return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())}`;
}

export function localMonthKey(d: Date): string {
  return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}`;
}
