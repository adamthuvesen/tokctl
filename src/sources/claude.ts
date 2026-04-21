import fs from 'node:fs';
import fsp from 'node:fs/promises';
import path from 'node:path';
import readline from 'node:readline';
import os from 'node:os';
import { resolveRoots } from '../paths.js';
import type { IngestStats, UsageEvent } from '../types.js';

export interface ClaudeSourceOptions {
  flag?: string;
  env: NodeJS.ProcessEnv;
}

export interface SourceRun {
  events: UsageEvent[];
  stats: IngestStats;
}

function decodeProjectSlug(folderName: string): string | undefined {
  // Claude Code encodes a cwd as -<component>-<component>-... (leading dash, dashes as separators).
  // Round-trip isn't perfectly reversible — paths containing literal dashes collide with separators —
  // but for typical cwds like /Users/foo/dev/repo the decode is accurate enough to be useful.
  if (!folderName.startsWith('-')) return undefined;
  return '/' + folderName.slice(1).replace(/-/g, '/');
}

async function walkJsonl(root: string): Promise<Array<{ file: string; projectPath?: string }>> {
  const results: Array<{ file: string; projectPath?: string }> = [];
  let rootEntries: fs.Dirent[];
  try {
    rootEntries = await fsp.readdir(root, { withFileTypes: true });
  } catch {
    return results;
  }
  for (const entry of rootEntries) {
    if (!entry.isDirectory()) continue;
    const projectDir = path.join(root, entry.name);
    const projectPath = decodeProjectSlug(entry.name);
    const files = await fsp.readdir(projectDir, { withFileTypes: true }).catch(() => []);
    for (const f of files) {
      if (f.isFile() && f.name.endsWith('.jsonl')) {
        results.push({ file: path.join(projectDir, f.name), projectPath });
      }
    }
  }
  return results;
}

function parseUsageRow(raw: unknown, projectPath: string | undefined): UsageEvent | null {
  if (!raw || typeof raw !== 'object') return null;
  const row = raw as Record<string, unknown>;
  if (row.type !== 'assistant') return null;
  const message = row.message as Record<string, unknown> | undefined;
  if (!message || typeof message !== 'object') return null;
  const usage = message.usage as Record<string, unknown> | undefined;
  if (!usage || typeof usage !== 'object') return null;
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
  return {
    source: 'claude',
    timestamp,
    sessionId,
    projectPath,
    model,
    inputTokens: input,
    outputTokens: output,
    cacheReadTokens: cacheRead,
    cacheWriteTokens: cacheWrite,
  };
}

async function readJsonl(
  file: string,
  projectPath: string | undefined,
  events: UsageEvent[],
  stats: IngestStats,
  seenIds: Set<string>,
): Promise<void> {
  const stream = fs.createReadStream(file, { encoding: 'utf-8' });
  const rl = readline.createInterface({ input: stream, crlfDelay: Infinity });
  for await (const line of rl) {
    if (!line.trim()) continue;
    let parsed: unknown;
    try {
      parsed = JSON.parse(line);
    } catch {
      stats.skippedLines += 1;
      continue;
    }
    const ev = parseUsageRow(parsed, projectPath);
    if (!ev) continue;
    // Claude dedupes usage rows by message id — avoid double-counting resumed sessions.
    const id =
      (parsed as { message?: { id?: string } }).message?.id ??
      `${ev.sessionId}:${ev.timestamp.getTime()}`;
    if (seenIds.has(id)) continue;
    seenIds.add(id);
    events.push(ev);
  }
}

export async function ingestClaude(options: ClaudeSourceOptions): Promise<SourceRun> {
  const stats: IngestStats = { skippedLines: 0, unknownModels: new Set() };
  const events: UsageEvent[] = [];
  const seenIds = new Set<string>();

  const { roots, userSupplied } = resolveRoots({
    flag: options.flag,
    aiusageEnv: options.env.AIUSAGE_CLAUDE_DIR,
    toolEnv: options.env.CLAUDE_CONFIG_DIR,
    toolEnvSuffix: 'projects',
    defaults: [path.join(os.homedir(), '.claude', 'projects'), path.join(os.homedir(), '.config', 'claude', 'projects')],
  });

  for (const root of roots) {
    const exists = await fsp.stat(root).then((s) => s.isDirectory()).catch(() => false);
    if (!exists) {
      if (userSupplied) {
        throw new Error(`Claude directory not readable: ${root}`);
      }
      continue;
    }
    const files = await walkJsonl(root);
    for (const { file, projectPath } of files) {
      await readJsonl(file, projectPath, events, stats, seenIds);
    }
  }
  return { events, stats };
}
