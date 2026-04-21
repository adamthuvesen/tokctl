import fs from 'node:fs';
import fsp from 'node:fs/promises';
import path from 'node:path';
import readline from 'node:readline';
import os from 'node:os';
import { resolveRoots } from '../paths.js';
import type { IngestStats, UsageEvent } from '../types.js';
import type { SourceRun } from './claude.js';

export interface CodexSourceOptions {
  flag?: string;
  env: NodeJS.ProcessEnv;
}

async function walkJsonl(root: string): Promise<string[]> {
  const out: string[] = [];
  async function recurse(dir: string): Promise<void> {
    let entries: fs.Dirent[];
    try {
      entries = await fsp.readdir(dir, { withFileTypes: true });
    } catch {
      return;
    }
    for (const e of entries) {
      const full = path.join(dir, e.name);
      if (e.isDirectory()) {
        await recurse(full);
      } else if (e.isFile() && e.name.endsWith('.jsonl')) {
        out.push(full);
      }
    }
  }
  await recurse(root);
  return out;
}

async function readRollout(
  file: string,
  events: UsageEvent[],
  stats: IngestStats,
): Promise<void> {
  // Per-file state: session_meta sets sessionId + projectPath; turn_context sets the active model;
  // token_count rows carry per-turn deltas in `last_token_usage`.
  let sessionId = '';
  let projectPath: string | undefined;
  let currentModel = 'unknown';

  const stream = fs.createReadStream(file, { encoding: 'utf-8' });
  const rl = readline.createInterface({ input: stream, crlfDelay: Infinity });

  for await (const line of rl) {
    if (!line.trim()) continue;
    let row: Record<string, unknown>;
    try {
      row = JSON.parse(line) as Record<string, unknown>;
    } catch {
      stats.skippedLines += 1;
      continue;
    }

    const type = row.type;
    const payload = row.payload as Record<string, unknown> | undefined;
    if (!payload || typeof payload !== 'object') continue;

    if (type === 'session_meta') {
      if (typeof payload.id === 'string') sessionId = payload.id;
      if (typeof payload.cwd === 'string') projectPath = payload.cwd;
      continue;
    }

    if (type === 'turn_context') {
      if (typeof payload.model === 'string') currentModel = payload.model;
      continue;
    }

    if (type !== 'event_msg') continue;
    if (payload.type !== 'token_count') continue;
    const info = payload.info as Record<string, unknown> | undefined;
    if (!info) continue;
    const last = info.last_token_usage as Record<string, unknown> | undefined;
    if (!last) continue;

    const input = Number(last.input_tokens ?? 0);
    const output = Number(last.output_tokens ?? 0);
    const cacheRead = Number(last.cached_input_tokens ?? 0);
    // Codex has no cache-write concept; OpenAI handles prompt caching automatically.
    const reasoning = Number(last.reasoning_output_tokens ?? 0);
    const totalOutput = output + reasoning;
    if (input + totalOutput + cacheRead === 0) continue;

    const timestampStr = typeof row.timestamp === 'string' ? row.timestamp : null;
    if (!timestampStr || !sessionId) continue;
    const timestamp = new Date(timestampStr);
    if (Number.isNaN(timestamp.getTime())) continue;

    events.push({
      source: 'codex',
      timestamp,
      sessionId,
      projectPath,
      model: currentModel,
      inputTokens: input,
      outputTokens: totalOutput,
      cacheReadTokens: cacheRead,
      cacheWriteTokens: 0,
    });
  }
}

export async function ingestCodex(options: CodexSourceOptions): Promise<SourceRun> {
  const stats: IngestStats = { skippedLines: 0, unknownModels: new Set() };
  const events: UsageEvent[] = [];

  const { roots, userSupplied } = resolveRoots({
    flag: options.flag,
    aiusageEnv: options.env.AIUSAGE_CODEX_DIR,
    toolEnv: options.env.CODEX_HOME,
    toolEnvSuffix: 'sessions',
    defaults: [path.join(os.homedir(), '.codex', 'sessions')],
  });

  for (const root of roots) {
    const exists = await fsp.stat(root).then((s) => s.isDirectory()).catch(() => false);
    if (!exists) {
      if (userSupplied) {
        throw new Error(`Codex directory not readable: ${root}`);
      }
      continue;
    }
    const files = await walkJsonl(root);
    for (const file of files) {
      await readRollout(file, events, stats);
    }
  }
  return { events, stats };
}
