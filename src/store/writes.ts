import type { DB } from './db.js';

export interface FileManifestRow {
  path: string;
  source: 'claude' | 'codex';
  project: string | null;
  size: number;
  mtime_ns: bigint | number;
  last_offset: number;
  n_events: number;
  session_id: string | null;
  model: string | null;
}

export interface EventRow {
  file_path: string;
  source: 'claude' | 'codex';
  ts: number;
  day: string;
  month: string;
  session_id: string;
  project_path: string | null;
  model: string;
  input: number;
  output: number;
  cache_read: number;
  cache_write: number;
  cost_usd: number;
}

export function upsertFileManifest(db: DB, row: FileManifestRow): void {
  const stmt = db.prepare(`
    INSERT INTO files (path, source, project, size, mtime_ns, last_offset, n_events, session_id, model)
    VALUES (@path, @source, @project, @size, @mtime_ns, @last_offset, @n_events, @session_id, @model)
    ON CONFLICT(path) DO UPDATE SET
      source      = excluded.source,
      project     = excluded.project,
      size        = excluded.size,
      mtime_ns    = excluded.mtime_ns,
      last_offset = excluded.last_offset,
      n_events    = excluded.n_events,
      session_id  = COALESCE(excluded.session_id, files.session_id),
      model       = COALESCE(excluded.model, files.model)
  `);
  stmt.run(row);
}

export function deleteFileAndEvents(db: DB, filePath: string): void {
  db.prepare('DELETE FROM events WHERE file_path = ?').run(filePath);
  db.prepare('DELETE FROM files WHERE path = ?').run(filePath);
}

export function insertEvents(db: DB, rows: EventRow[]): number {
  if (rows.length === 0) return 0;
  const stmt = db.prepare(`
    INSERT INTO events
      (file_path, source, ts, day, month, session_id, project_path, model, input, output, cache_read, cache_write, cost_usd)
    VALUES
      (@file_path, @source, @ts, @day, @month, @session_id, @project_path, @model, @input, @output, @cache_read, @cache_write, @cost_usd)
  `);
  let inserted = 0;
  for (const row of rows) {
    stmt.run(row);
    inserted += 1;
  }
  return inserted;
}

export function loadFileManifest(db: DB): Map<string, FileManifestRow> {
  const rows = db
    .prepare(
      'SELECT path, source, project, size, mtime_ns, last_offset, n_events, session_id, model FROM files',
    )
    .all() as FileManifestRow[];
  const map = new Map<string, FileManifestRow>();
  for (const r of rows) map.set(r.path, r);
  return map;
}
