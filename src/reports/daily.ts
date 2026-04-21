import type { UsageEvent } from '../types.js';
import { aggregate, AggregateRow, localDateKey } from './shared.js';

export function dailyReport(events: UsageEvent[], unknownModels: Set<string>): AggregateRow[] {
  const map = aggregate(events, (e) => localDateKey(e.timestamp), unknownModels);
  return Array.from(map.values()).sort((a, b) => a.key.localeCompare(b.key));
}
