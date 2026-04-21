import fsp from 'node:fs/promises';
import type { BigIntStats, Dirent } from 'node:fs';
import path from 'node:path';
import type { FileManifestRow } from '../store/writes.js';

export type Source = 'claude' | 'codex';

export interface DiscoveredFile {
  path: string;
  source: Source;
  project: string | null;
  size: number;
  mtimeNs: bigint;
}

export interface Discovery {
  files: DiscoveredFile[];
  /** Paths whose containing dir looks unchanged; trust the manifest. */
  unchangedPaths: Set<string>;
}

export interface IngestPlan {
  toSkip: string[];
  toTail: Array<{ file: DiscoveredFile; fromOffset: number }>;
  toFullParse: DiscoveredFile[];
  toPurge: string[];
}

export interface PlanInput {
  manifest: Map<string, FileManifestRow>;
  discovery: Discovery;
  safetyWindowMs?: number;
  now?: number;
}

function manifestMtime(row: FileManifestRow): bigint {
  return typeof row.mtime_ns === 'bigint' ? row.mtime_ns : BigInt(row.mtime_ns);
}

export function planIngest(input: PlanInput): IngestPlan {
  const safetyWindowMs = input.safetyWindowMs ?? 60 * 60 * 1000;
  const now = input.now ?? Date.now();
  const safetyThresholdNs = BigInt(now - safetyWindowMs) * 1_000_000n;

  const plan: IngestPlan = { toSkip: [], toTail: [], toFullParse: [], toPurge: [] };
  const touched = new Set<string>();

  for (const d of input.discovery.files) {
    touched.add(d.path);
    const row = input.manifest.get(d.path);

    if (!row) {
      plan.toFullParse.push(d);
      continue;
    }

    const rowMtime = manifestMtime(row);

    if (d.size < row.last_offset) {
      plan.toFullParse.push(d);
      continue;
    }

    if (d.size === row.size && d.mtimeNs === rowMtime) {
      plan.toSkip.push(d.path);
      continue;
    }

    if (d.size > row.last_offset) {
      plan.toTail.push({ file: d, fromOffset: row.last_offset });
    } else {
      plan.toSkip.push(d.path);
    }
  }

  for (const p of input.discovery.unchangedPaths) {
    if (touched.has(p)) continue;
    const row = input.manifest.get(p);
    if (!row) continue;
    const rowMtime = manifestMtime(row);
    if (rowMtime >= safetyThresholdNs) {
      // Spec: recent files are always re-stat'd even if dir looks unchanged.
      // Caller guarantees this case doesn't arrive via unchangedPaths — if it
      // does anyway, demote to a full-parse-safe skip: keep the data as-is.
      plan.toSkip.push(p);
    } else {
      plan.toSkip.push(p);
    }
    touched.add(p);
  }

  for (const [p] of input.manifest) {
    if (!touched.has(p)) plan.toPurge.push(p);
  }

  return plan;
}

// --- discovery helpers ---

function decodeClaudeSlug(folderName: string): string | null {
  if (!folderName.startsWith('-')) return null;
  return '/' + folderName.slice(1).replace(/-/g, '/');
}

async function statDir(dir: string): Promise<BigIntStats | null> {
  try {
    return await fsp.stat(dir, { bigint: true });
  } catch {
    return null;
  }
}

async function listDir(dir: string): Promise<Dirent[]> {
  try {
    return await fsp.readdir(dir, { withFileTypes: true });
  } catch {
    return [];
  }
}

// Index the manifest by parent directory once per run so dir-level lookups are O(1).
export interface ManifestDirIndex {
  byParent: Map<string, { paths: string[]; maxMtime: bigint }>;
}

export function indexManifestByParent(
  manifest: Map<string, FileManifestRow>,
): ManifestDirIndex {
  const byParent = new Map<string, { paths: string[]; maxMtime: bigint }>();
  for (const [p, row] of manifest) {
    const parent = path.dirname(p);
    let entry = byParent.get(parent);
    if (!entry) {
      entry = { paths: [], maxMtime: 0n };
      byParent.set(parent, entry);
    }
    entry.paths.push(p);
    const m = manifestMtime(row);
    if (m > entry.maxMtime) entry.maxMtime = m;
  }
  return { byParent };
}

interface DiscoverOptions {
  manifest: Map<string, FileManifestRow>;
  manifestIndex?: ManifestDirIndex;
  safetyWindowMs?: number;
  now?: number;
}

function isRecentMtime(mtimeNs: bigint, safetyThresholdNs: bigint): boolean {
  return mtimeNs >= safetyThresholdNs;
}

async function walkAllFiles(
  dir: string,
  accept: (absPath: string, size: number, mtimeNs: bigint) => void | Promise<void>,
): Promise<void> {
  for (const e of await listDir(dir)) {
    const abs = path.join(dir, e.name);
    if (e.isDirectory()) {
      await walkAllFiles(abs, accept);
    } else if (e.isFile() && e.name.endsWith('.jsonl')) {
      try {
        const st = await fsp.stat(abs, { bigint: true });
        await accept(abs, Number(st.size), st.mtimeNs);
      } catch {
        // race; skip
      }
    }
  }
}

/**
 * Walks a directory with dir-mtime short-circuit. If the directory's mtime is
 * older than any manifest mtime it contains (and no file under it is "recent"),
 * skip statting individual files and mark the manifest paths under it as
 * unchanged.
 */
async function walkWithShortCircuit(
  dir: string,
  source: Source,
  project: string | null,
  opts: DiscoverOptions,
  safetyThresholdNs: bigint,
  out: Discovery,
): Promise<void> {
  const st = await statDir(dir);
  if (!st) return;
  const dirMtime = st.mtimeNs;

  const index = opts.manifestIndex;
  const entry = index?.byParent.get(dir);
  // If this dir has manifest entries and the dir mtime hasn't advanced past
  // them, skip statting files inside.
  if (entry && dirMtime <= entry.maxMtime) {
    let hasRecent = false;
    for (const p of entry.paths) {
      const row = opts.manifest.get(p);
      if (!row) continue;
      if (isRecentMtime(manifestMtime(row), safetyThresholdNs)) {
        hasRecent = true;
        break;
      }
    }
    if (!hasRecent) {
      for (const p of entry.paths) out.unchangedPaths.add(p);
      return;
    }
    // Fall through: there's a recent file; do the full walk.
  }

  // Full walk: stat every file.
  for (const e of await listDir(dir)) {
    const abs = path.join(dir, e.name);
    if (e.isDirectory()) {
      await walkWithShortCircuit(abs, source, project, opts, safetyThresholdNs, out);
    } else if (e.isFile() && e.name.endsWith('.jsonl')) {
      try {
        const fs_ = await fsp.stat(abs, { bigint: true });
        out.files.push({
          path: abs,
          source,
          project,
          size: Number(fs_.size),
          mtimeNs: fs_.mtimeNs,
        });
      } catch {
        // race; skip
      }
    }
  }
}

export async function discoverClaude(
  roots: string[],
  opts: DiscoverOptions,
): Promise<Discovery> {
  const now = opts.now ?? Date.now();
  const safetyWindowMs = opts.safetyWindowMs ?? 60 * 60 * 1000;
  const safetyThresholdNs = BigInt(now - safetyWindowMs) * 1_000_000n;
  const resolvedOpts: DiscoverOptions = {
    ...opts,
    manifestIndex: opts.manifestIndex ?? indexManifestByParent(opts.manifest),
  };

  const discovery: Discovery = { files: [], unchangedPaths: new Set() };
  for (const root of roots) {
    for (const e of await listDir(root)) {
      if (!e.isDirectory()) continue;
      const projectDir = path.join(root, e.name);
      const project = decodeClaudeSlug(e.name);
      await walkWithShortCircuit(
        projectDir,
        'claude',
        project,
        resolvedOpts,
        safetyThresholdNs,
        discovery,
      );
    }
  }
  return discovery;
}

export async function discoverCodex(
  roots: string[],
  opts: DiscoverOptions,
): Promise<Discovery> {
  const now = opts.now ?? Date.now();
  const safetyWindowMs = opts.safetyWindowMs ?? 60 * 60 * 1000;
  const safetyThresholdNs = BigInt(now - safetyWindowMs) * 1_000_000n;
  const resolvedOpts: DiscoverOptions = {
    ...opts,
    manifestIndex: opts.manifestIndex ?? indexManifestByParent(opts.manifest),
  };

  const discovery: Discovery = { files: [], unchangedPaths: new Set() };
  for (const root of roots) {
    await walkWithShortCircuit(
      root,
      'codex',
      null,
      resolvedOpts,
      safetyThresholdNs,
      discovery,
    );
  }
  return discovery;
}

export async function discoverAll(input: {
  claudeRoots: string[];
  codexRoots: string[];
  manifest: Map<string, FileManifestRow>;
  safetyWindowMs?: number;
  now?: number;
}): Promise<Discovery> {
  const [c, x] = await Promise.all([
    discoverClaude(input.claudeRoots, {
      manifest: input.manifest,
      safetyWindowMs: input.safetyWindowMs,
      now: input.now,
    }),
    discoverCodex(input.codexRoots, {
      manifest: input.manifest,
      safetyWindowMs: input.safetyWindowMs,
      now: input.now,
    }),
  ]);
  const merged: Discovery = {
    files: [...c.files, ...x.files],
    unchangedPaths: new Set<string>([...c.unchangedPaths, ...x.unchangedPaths]),
  };
  return merged;
}

// Legacy discovery path for --no-cache mode: no short-circuit, no manifest.
export async function discoverClaudeLegacy(roots: string[]): Promise<DiscoveredFile[]> {
  const out: DiscoveredFile[] = [];
  for (const root of roots) {
    for (const e of await listDir(root)) {
      if (!e.isDirectory()) continue;
      const projectDir = path.join(root, e.name);
      const project = decodeClaudeSlug(e.name);
      await walkAllFiles(projectDir, (abs, size, mtimeNs) => {
        out.push({ path: abs, source: 'claude', project, size, mtimeNs });
      });
    }
  }
  return out;
}

export async function discoverCodexLegacy(roots: string[]): Promise<DiscoveredFile[]> {
  const out: DiscoveredFile[] = [];
  for (const root of roots) {
    await walkAllFiles(root, (abs, size, mtimeNs) => {
      out.push({ path: abs, source: 'codex', project: null, size, mtimeNs });
    });
  }
  return out;
}
