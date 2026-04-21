import fs from 'node:fs';
import fsp from 'node:fs/promises';
import type { UsageEvent } from '../types.js';
import { claudeLineHasSignal, parseClaudeLine } from '../sources/claude-parse.js';
import { parseCodexLine, type CodexParseContext } from '../sources/codex-parse.js';

export interface FileRangeResult {
  events: UsageEvent[];
  messageIds: string[]; // Claude dedupe keys, index-aligned with events
  skippedLines: number;
  // Last full-line byte boundary we consumed. Caller writes this to manifest.
  consumedToOffset: number;
  // Codex session context to persist on the manifest for future tail-reads.
  sessionId: string | null;
  projectPath: string | null;
  currentModel: string | null;
}

async function readRange(
  filePath: string,
  fromOffset: number,
  toOffset: number,
): Promise<Buffer> {
  if (toOffset <= fromOffset) return Buffer.alloc(0);
  const fh = await fsp.open(filePath, 'r');
  try {
    const length = toOffset - fromOffset;
    const buf = Buffer.allocUnsafe(length);
    let read = 0;
    while (read < length) {
      const { bytesRead } = await fh.read(buf, read, length - read, fromOffset + read);
      if (bytesRead === 0) break;
      read += bytesRead;
    }
    return buf.subarray(0, read);
  } finally {
    await fh.close();
  }
}

// Splits a buffer into complete lines. If `requireCompleteLastLine` is true and
// the buffer does not end in \n, the trailing partial line is discarded and its
// byte length is returned so the caller can rewind its offset.
function splitLines(buf: Buffer): { lines: string[]; trailingBytes: number } {
  const text = buf.toString('utf8');
  if (text.length === 0) return { lines: [], trailingBytes: 0 };
  const endsWithNewline = text.endsWith('\n');
  const parts = text.split('\n');
  if (endsWithNewline) {
    // Drop the empty string after the final \n.
    parts.pop();
    return { lines: parts, trailingBytes: 0 };
  }
  const trailing = parts.pop() ?? '';
  return { lines: parts, trailingBytes: Buffer.byteLength(trailing, 'utf8') };
}

export async function ingestClaudeRange(opts: {
  filePath: string;
  projectPath: string | null;
  fromOffset: number;
  toOffset: number;
}): Promise<FileRangeResult> {
  const buf = await readRange(opts.filePath, opts.fromOffset, opts.toOffset);
  const { lines, trailingBytes } = splitLines(buf);

  const events: UsageEvent[] = [];
  const messageIds: string[] = [];
  let skipped = 0;

  for (const line of lines) {
    if (!line) continue;
    if (!claudeLineHasSignal(line)) continue;
    const parsed = parseClaudeLine(line, { projectPath: opts.projectPath });
    if (!parsed) {
      // Signal-matched but parse failed — count as skipped.
      skipped += 1;
      continue;
    }
    events.push(parsed.event);
    messageIds.push(parsed.messageId ?? '');
  }

  return {
    events,
    messageIds,
    skippedLines: skipped,
    consumedToOffset: opts.toOffset - trailingBytes,
    sessionId: null,
    projectPath: opts.projectPath,
    currentModel: null,
  };
}

export async function ingestCodexRange(opts: {
  filePath: string;
  fromOffset: number;
  toOffset: number;
  initialCtx: CodexParseContext;
}): Promise<FileRangeResult> {
  const buf = await readRange(opts.filePath, opts.fromOffset, opts.toOffset);
  const { lines, trailingBytes } = splitLines(buf);

  const events: UsageEvent[] = [];
  const ctx: CodexParseContext = { ...opts.initialCtx };
  let skipped = 0;

  for (const line of lines) {
    if (!line) continue;
    try {
      const parsed = parseCodexLine(line, ctx);
      if (parsed?.event) events.push(parsed.event);
    } catch {
      skipped += 1;
    }
  }

  return {
    events,
    messageIds: events.map(() => ''),
    skippedLines: skipped,
    consumedToOffset: opts.toOffset - trailingBytes,
    sessionId: ctx.sessionId,
    projectPath: ctx.projectPath,
    currentModel: ctx.currentModel,
  };
}

// Helper for legacy / --no-cache: read entire file as stream.
export async function getFileSize(filePath: string): Promise<number> {
  const st = await fsp.stat(filePath);
  return st.size;
}

export function readStream(filePath: string, fromOffset = 0): fs.ReadStream {
  return fs.createReadStream(filePath, { encoding: 'utf-8', start: fromOffset });
}
