import type { Source } from '../types.js';

// Rendered rows are source-agnostic and match what the renderer expects.
export interface AggregateRow {
  key: string;
  source?: Source | 'all';
  projectPath?: string;
  latestTimestamp?: Date;
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheWriteTokens: number;
  totalTokens: number;
  costUsd: number;
}
