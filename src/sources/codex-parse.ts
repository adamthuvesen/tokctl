import type { UsageEvent } from '../types.js';

export interface CodexParseContext {
  sessionId: string | null;
  projectPath: string | null;
  currentModel: string | null;
}

export function codexLineHasSignal(line: string): boolean {
  // Every row we care about carries one of these three markers. We can't use
  // just "token_count" because session_meta and turn_context rows are needed to
  // populate the parser context.
  return (
    line.includes('"token_count"') ||
    line.includes('"session_meta"') ||
    line.includes('"turn_context"')
  );
}

export interface ParsedCodex {
  kind: 'session_meta' | 'turn_context' | 'token_count';
  event?: UsageEvent;
}

export function parseCodexLine(
  line: string,
  ctx: CodexParseContext,
): ParsedCodex | null {
  if (!codexLineHasSignal(line)) return null;

  let raw: unknown;
  try {
    raw = JSON.parse(line);
  } catch {
    return null;
  }
  if (!raw || typeof raw !== 'object') return null;
  const row = raw as Record<string, unknown>;
  const type = row.type;
  const payload = row.payload as Record<string, unknown> | undefined;
  if (!payload || typeof payload !== 'object') return null;

  if (type === 'session_meta') {
    if (typeof payload.id === 'string') ctx.sessionId = payload.id;
    if (typeof payload.cwd === 'string') ctx.projectPath = payload.cwd;
    return { kind: 'session_meta' };
  }

  if (type === 'turn_context') {
    if (typeof payload.model === 'string') ctx.currentModel = payload.model;
    return { kind: 'turn_context' };
  }

  if (type !== 'event_msg') return null;
  if (payload.type !== 'token_count') return null;
  const info = payload.info as Record<string, unknown> | undefined;
  if (!info) return { kind: 'token_count' };
  const last = info.last_token_usage as Record<string, unknown> | undefined;
  if (!last) return { kind: 'token_count' };

  const input = Number(last.input_tokens ?? 0);
  const output = Number(last.output_tokens ?? 0);
  const cacheRead = Number(last.cached_input_tokens ?? 0);
  const reasoning = Number(last.reasoning_output_tokens ?? 0);
  const totalOutput = output + reasoning;
  if (input + totalOutput + cacheRead === 0) return { kind: 'token_count' };

  const timestampStr = typeof row.timestamp === 'string' ? row.timestamp : null;
  const sessionId = ctx.sessionId;
  if (!timestampStr || !sessionId) return { kind: 'token_count' };
  const timestamp = new Date(timestampStr);
  if (Number.isNaN(timestamp.getTime())) return { kind: 'token_count' };

  const event: UsageEvent = {
    source: 'codex',
    timestamp,
    sessionId,
    projectPath: ctx.projectPath ?? undefined,
    model: ctx.currentModel ?? 'unknown',
    inputTokens: input,
    outputTokens: totalOutput,
    cacheReadTokens: cacheRead,
    cacheWriteTokens: 0,
  };
  return { kind: 'token_count', event };
}
