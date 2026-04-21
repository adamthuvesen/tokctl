import { costOf } from '../pricing.js';
import type { AggregateRow } from '../store/rowTypes.js';
import type { Source, UsageEvent } from '../types.js';

function pad2(n: number): string {
  return n < 10 ? `0${n}` : String(n);
}

export function localDateKey(d: Date): string {
  return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())}`;
}

export function localMonthKey(d: Date): string {
  return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}`;
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

function aggregateByKey(
  events: UsageEvent[],
  keyFor: (e: UsageEvent) => string,
  sourceLabel: Source | 'all',
  unknown: Set<string>,
): AggregateRow[] {
  const map = new Map<string, AggregateRow>();
  for (const e of events) {
    const key = keyFor(e);
    let row = map.get(key);
    if (!row) {
      row = {
        key,
        source: sourceLabel,
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
    row.costUsd += costOf(e, unknown);
  }
  return Array.from(map.values()).sort((a, b) => a.key.localeCompare(b.key));
}

export function dailyInMemory(
  events: UsageEvent[],
  sourceLabel: Source | 'all',
  unknown: Set<string>,
): AggregateRow[] {
  return aggregateByKey(events, (e) => localDateKey(e.timestamp), sourceLabel, unknown);
}

export function monthlyInMemory(
  events: UsageEvent[],
  sourceLabel: Source | 'all',
  unknown: Set<string>,
): AggregateRow[] {
  return aggregateByKey(events, (e) => localMonthKey(e.timestamp), sourceLabel, unknown);
}

export function sessionInMemory(
  events: UsageEvent[],
  unknown: Set<string>,
): AggregateRow[] {
  const map = new Map<string, AggregateRow>();
  for (const e of events) {
    const key = `${e.source}:${e.sessionId}`;
    let row = map.get(key);
    if (!row) {
      row = {
        key: e.sessionId,
        source: e.source,
        projectPath: e.projectPath,
        latestTimestamp: e.timestamp,
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
    row.costUsd += costOf(e, unknown);
    if (!row.latestTimestamp || e.timestamp > row.latestTimestamp) {
      row.latestTimestamp = e.timestamp;
    }
    if (!row.projectPath && e.projectPath) row.projectPath = e.projectPath;
  }
  return Array.from(map.values()).sort(
    (a, b) => (b.latestTimestamp?.getTime() ?? 0) - (a.latestTimestamp?.getTime() ?? 0),
  );
}
