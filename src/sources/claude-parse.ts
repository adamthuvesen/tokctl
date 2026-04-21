import type { UsageEvent } from '../types.js';

export interface ClaudeParseContext {
  projectPath: string | null;
}

// Fast substring pre-filter. Lines that miss both markers cannot carry usage.
export function claudeLineHasSignal(line: string): boolean {
  return line.includes('"type":"assistant"') && line.includes('"usage"');
}

export interface ParsedClaude {
  event: UsageEvent;
  messageId: string | null;
}

export function parseClaudeLine(
  line: string,
  ctx: ClaudeParseContext,
): ParsedClaude | null {
  if (!claudeLineHasSignal(line)) return null;

  let raw: unknown;
  try {
    raw = JSON.parse(line);
  } catch {
    return null;
  }
  if (!raw || typeof raw !== 'object') return null;
  const row = raw as Record<string, unknown>;
  if (row.type !== 'assistant') return null;

  const message = row.message as Record<string, unknown> | undefined;
  if (!message) return null;
  const usage = message.usage as Record<string, unknown> | undefined;
  if (!usage) return null;

  const input = Number(usage.input_tokens ?? 0);
  const output = Number(usage.output_tokens ?? 0);
  const cacheRead = Number(usage.cache_read_input_tokens ?? 0);
  const cacheWrite = Number(usage.cache_creation_input_tokens ?? 0);
  if (input + output + cacheRead + cacheWrite === 0) return null;

  const model = typeof message.model === 'string' ? message.model : 'unknown';
  const sessionId = typeof row.sessionId === 'string' ? row.sessionId : '';
  const timestampStr = typeof row.timestamp === 'string' ? row.timestamp : null;
  if (!sessionId || !timestampStr) return null;
  const timestamp = new Date(timestampStr);
  if (Number.isNaN(timestamp.getTime())) return null;

  const messageId = typeof message.id === 'string' ? message.id : null;

  return {
    event: {
      source: 'claude',
      timestamp,
      sessionId,
      projectPath: ctx.projectPath ?? undefined,
      model,
      inputTokens: input,
      outputTokens: output,
      cacheReadTokens: cacheRead,
      cacheWriteTokens: cacheWrite,
    },
    messageId,
  };
}
