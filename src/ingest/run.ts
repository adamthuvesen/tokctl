import { costOf, normalizeModelId } from '../pricing.js';
import type { DB } from '../store/db.js';
import { withTx } from '../store/db.js';
import {
  deleteFileAndEvents,
  insertEvents,
  loadFileManifest,
  upsertFileManifest,
  type EventRow,
  type FileManifestRow,
} from '../store/writes.js';
import type { UsageEvent } from '../types.js';
import {
  discoverClaude,
  discoverCodex,
  planIngest,
  type DiscoveredFile,
  type Discovery,
  type IngestPlan,
} from './plan.js';
import { ingestClaudeRange, ingestCodexRange } from './fileRange.js';

export interface IngestStats {
  filesScanned: number;
  filesSkipped: number;
  filesTailed: number;
  filesFullParsed: number;
  filesPurged: number;
  eventsInserted: number;
  skippedLines: number;
  unknownModels: Set<string>;
  warnings: string[];
}

export interface RunIngestOptions {
  db: DB;
  claudeRoots: string[];
  codexRoots: string[];
  includeClaude: boolean;
  includeCodex: boolean;
  safetyWindowMs?: number;
  now?: number;
}

function pad2(n: number): string {
  return n < 10 ? `0${n}` : String(n);
}
function localDay(d: Date): string {
  return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())}`;
}
function localMonth(d: Date): string {
  return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}`;
}

function eventToRow(ev: UsageEvent, filePath: string, unknown: Set<string>): EventRow {
  const cost = costOf(ev, unknown);
  return {
    file_path: filePath,
    source: ev.source,
    ts: ev.timestamp.getTime(),
    day: localDay(ev.timestamp),
    month: localMonth(ev.timestamp),
    session_id: ev.sessionId,
    project_path: ev.projectPath ?? null,
    model: ev.model,
    input: ev.inputTokens,
    output: ev.outputTokens,
    cache_read: ev.cacheReadTokens,
    cache_write: ev.cacheWriteTokens,
    cost_usd: cost,
  };
}

async function handlePlan(
  db: DB,
  plan: IngestPlan,
  manifest: Map<string, FileManifestRow>,
  stats: IngestStats,
  unknown: Set<string>,
): Promise<void> {
  const rowsToInsert: EventRow[] = [];
  const manifestUpserts: FileManifestRow[] = [];
  const purgeSet = new Set(plan.toPurge);

  // Full parses
  for (const file of plan.toFullParse) {
    stats.filesFullParsed += 1;
    // Drop any prior events for this file (truncation / rotation case)
    if (manifest.has(file.path)) purgeSet.add(file.path);

    if (file.source === 'claude') {
      const res = await ingestClaudeRange({
        filePath: file.path,
        projectPath: file.project,
        fromOffset: 0,
        toOffset: file.size,
      });
      stats.skippedLines += res.skippedLines;
      for (const ev of res.events) rowsToInsert.push(eventToRow(ev, file.path, unknown));
      manifestUpserts.push({
        path: file.path,
        source: 'claude',
        project: file.project,
        size: file.size,
        mtime_ns: file.mtimeNs,
        last_offset: res.consumedToOffset,
        n_events: res.events.length,
        session_id: null,
        model: null,
      });
    } else {
      const res = await ingestCodexRange({
        filePath: file.path,
        fromOffset: 0,
        toOffset: file.size,
        initialCtx: { sessionId: null, projectPath: null, currentModel: null },
      });
      stats.skippedLines += res.skippedLines;
      for (const ev of res.events) rowsToInsert.push(eventToRow(ev, file.path, unknown));
      manifestUpserts.push({
        path: file.path,
        source: 'codex',
        project: res.projectPath,
        size: file.size,
        mtime_ns: file.mtimeNs,
        last_offset: res.consumedToOffset,
        n_events: res.events.length,
        session_id: res.sessionId,
        model: res.currentModel,
      });
    }
  }

  // Tail reads
  for (const { file, fromOffset } of plan.toTail) {
    stats.filesTailed += 1;
    const existing = manifest.get(file.path);

    if (file.source === 'claude') {
      const res = await ingestClaudeRange({
        filePath: file.path,
        projectPath: file.project,
        fromOffset,
        toOffset: file.size,
      });
      stats.skippedLines += res.skippedLines;
      for (const ev of res.events) rowsToInsert.push(eventToRow(ev, file.path, unknown));
      const nEvents = (existing?.n_events ?? 0) + res.events.length;
      manifestUpserts.push({
        path: file.path,
        source: 'claude',
        project: file.project,
        size: file.size,
        mtime_ns: file.mtimeNs,
        last_offset: res.consumedToOffset,
        n_events: nEvents,
        session_id: null,
        model: null,
      });
    } else {
      // Restore session context from manifest for Codex tail-reads.
      const initialCtx = {
        sessionId: existing?.session_id ?? null,
        projectPath: existing?.project ?? null,
        currentModel: existing?.model ?? null,
      };
      const res = await ingestCodexRange({
        filePath: file.path,
        fromOffset,
        toOffset: file.size,
        initialCtx,
      });
      stats.skippedLines += res.skippedLines;
      for (const ev of res.events) rowsToInsert.push(eventToRow(ev, file.path, unknown));
      const nEvents = (existing?.n_events ?? 0) + res.events.length;
      manifestUpserts.push({
        path: file.path,
        source: 'codex',
        project: res.projectPath,
        size: file.size,
        mtime_ns: file.mtimeNs,
        last_offset: res.consumedToOffset,
        n_events: nEvents,
        session_id: res.sessionId,
        model: res.currentModel,
      });
    }
  }

  stats.filesSkipped = plan.toSkip.length;
  stats.filesPurged = purgeSet.size;
  stats.filesScanned =
    stats.filesSkipped + stats.filesTailed + stats.filesFullParsed;
  stats.eventsInserted = rowsToInsert.length;

  // All writes in one transaction.
  withTx(db, () => {
    for (const p of purgeSet) deleteFileAndEvents(db, p);
    for (const m of manifestUpserts) upsertFileManifest(db, m);
    insertEvents(db, rowsToInsert);
  });
}

export async function runIngest(opts: RunIngestOptions): Promise<IngestStats> {
  const stats: IngestStats = {
    filesScanned: 0,
    filesSkipped: 0,
    filesTailed: 0,
    filesFullParsed: 0,
    filesPurged: 0,
    eventsInserted: 0,
    skippedLines: 0,
    unknownModels: new Set(),
    warnings: [],
  };

  const manifest = loadFileManifest(opts.db);

  // Scope manifest to sources we're about to scan — we don't want to purge the
  // other source's rows just because --source=codex was passed.
  const scopedManifest = new Map<string, FileManifestRow>();
  for (const [p, row] of manifest) {
    if (row.source === 'claude' && !opts.includeClaude) continue;
    if (row.source === 'codex' && !opts.includeCodex) continue;
    scopedManifest.set(p, row);
  }

  let discovery: Discovery = { files: [], unchangedPaths: new Set() };

  if (opts.includeClaude) {
    const d = await discoverClaude(opts.claudeRoots, {
      manifest: scopedManifest,
      safetyWindowMs: opts.safetyWindowMs,
      now: opts.now,
    });
    discovery = mergeDiscovery(discovery, d);
  }
  if (opts.includeCodex) {
    const d = await discoverCodex(opts.codexRoots, {
      manifest: scopedManifest,
      safetyWindowMs: opts.safetyWindowMs,
      now: opts.now,
    });
    discovery = mergeDiscovery(discovery, d);
  }

  const plan = planIngest({
    manifest: scopedManifest,
    discovery,
    safetyWindowMs: opts.safetyWindowMs,
    now: opts.now,
  });

  await handlePlan(opts.db, plan, scopedManifest, stats, stats.unknownModels);

  return stats;
}

function mergeDiscovery(a: Discovery, b: Discovery): Discovery {
  return {
    files: [...a.files, ...b.files],
    unchangedPaths: new Set([...a.unchangedPaths, ...b.unchangedPaths]),
  };
}

// Utility for consumers that want to know about unknown-cost events from the DB
// (rather than what the current run inserted).
export function collectUnknownModelsFromDb(db: DB, sourceFilter: 'claude' | 'codex' | null): Set<string> {
  const sql = `
    SELECT DISTINCT model FROM events
    WHERE cost_usd = 0
      AND (input + output + cache_read + cache_write) > 0
      ${sourceFilter ? 'AND source = ?' : ''}
  `;
  const stmt = db.prepare(sql);
  const rows = (sourceFilter ? stmt.all(sourceFilter) : stmt.all()) as Array<{ model: string }>;
  const out = new Set<string>();
  for (const r of rows) {
    // We already stored the exact model string; don't show the normalized key.
    if (!out.has(r.model) && !hasKnownPrice(r.model)) out.add(r.model);
  }
  return out;
}

// Local mirror of pricing.hasPrice without the circular import.
import { hasPrice as _hasPrice } from '../pricing.js';
function hasKnownPrice(model: string): boolean {
  return _hasPrice(model) || _hasPrice(normalizeModelId(model));
}
