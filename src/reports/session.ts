import type { UsageEvent } from '../types.js';
import { AggregateRow } from './shared.js';
import { costOf } from '../pricing.js';

export function sessionReport(events: UsageEvent[], unknownModels: Set<string>): AggregateRow[] {
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
    row.costUsd += costOf(e, unknownModels);
    if (!row.latestTimestamp || e.timestamp > row.latestTimestamp) {
      row.latestTimestamp = e.timestamp;
    }
    if (!row.projectPath && e.projectPath) row.projectPath = e.projectPath;
  }
  return Array.from(map.values()).sort(
    (a, b) => (b.latestTimestamp?.getTime() ?? 0) - (a.latestTimestamp?.getTime() ?? 0),
  );
}
