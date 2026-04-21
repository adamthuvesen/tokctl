import type { UsageEvent } from '../types.js';
import { aggregate, AggregateRow, localMonthKey } from './shared.js';

export function monthlyReport(events: UsageEvent[], unknownModels: Set<string>): AggregateRow[] {
  const map = aggregate(events, (e) => localMonthKey(e.timestamp), unknownModels);
  return Array.from(map.values()).sort((a, b) => a.key.localeCompare(b.key));
}
