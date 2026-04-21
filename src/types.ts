export type Source = 'claude' | 'codex';

export interface UsageEvent {
  source: Source;
  timestamp: Date;
  sessionId: string;
  projectPath?: string;
  model: string;
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheWriteTokens: number;
}

export interface IngestStats {
  skippedLines: number;
  unknownModels: Set<string>;
}
