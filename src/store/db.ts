import fs from 'node:fs';
import path from 'node:path';
import Database from 'better-sqlite3';
import { DDL, SCHEMA_VERSION } from './schema.js';

export type DB = Database.Database;

export interface OpenStoreOptions {
  path: string;
  readonly?: boolean;
  onRebuild?: (reason: string) => void;
}

function initSchema(db: DB): void {
  db.pragma('journal_mode = WAL');
  db.pragma('synchronous = NORMAL');
  db.exec(DDL);
  db.prepare('INSERT OR REPLACE INTO meta (key, value) VALUES (?, ?)').run(
    'schema_version',
    String(SCHEMA_VERSION),
  );
}

function readSchemaVersion(db: DB): number | null {
  try {
    const row = db
      .prepare("SELECT value FROM meta WHERE key = 'schema_version'")
      .get() as { value: string } | undefined;
    if (!row) return null;
    const n = Number.parseInt(row.value, 10);
    return Number.isFinite(n) ? n : null;
  } catch {
    return null;
  }
}

function tryOpen(filePath: string, readonly: boolean): DB | null {
  try {
    const db = new Database(filePath, { readonly, fileMustExist: false });
    // Touch the schema to ensure the file is a real sqlite DB.
    db.prepare('SELECT 1').get();
    return db;
  } catch {
    return null;
  }
}

export function openStore(opts: OpenStoreOptions): DB {
  fs.mkdirSync(path.dirname(opts.path), { recursive: true });

  let db = tryOpen(opts.path, opts.readonly ?? false);
  if (!db) {
    if (fs.existsSync(opts.path)) {
      fs.unlinkSync(opts.path);
      opts.onRebuild?.('cache file was unreadable');
    }
    db = new Database(opts.path, { readonly: opts.readonly ?? false });
  }

  // Ensure meta exists so we can read schema_version safely.
  try {
    db.exec('CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL)');
  } catch {
    // If even this fails, the file is hopeless — rebuild.
    db.close();
    fs.unlinkSync(opts.path);
    opts.onRebuild?.('cache file was corrupt');
    db = new Database(opts.path, { readonly: opts.readonly ?? false });
  }

  const current = readSchemaVersion(db);
  if (current !== SCHEMA_VERSION) {
    db.close();
    if (fs.existsSync(opts.path)) fs.unlinkSync(opts.path);
    opts.onRebuild?.(
      current === null
        ? 'initializing cache schema'
        : `schema version mismatch (${current} → ${SCHEMA_VERSION})`,
    );
    db = new Database(opts.path, { readonly: opts.readonly ?? false });
    initSchema(db);
  } else {
    // Ensure pragmas and DDL are applied even on reopens.
    db.pragma('journal_mode = WAL');
    db.pragma('synchronous = NORMAL');
  }

  return db;
}

export function closeStore(db: DB): void {
  try {
    db.close();
  } catch {
    // ignore
  }
}

export function withTx<T>(db: DB, fn: () => T): T {
  db.exec('BEGIN IMMEDIATE');
  try {
    const out = fn();
    db.exec('COMMIT');
    return out;
  } catch (err) {
    try {
      db.exec('ROLLBACK');
    } catch {
      // already rolled back
    }
    throw err;
  }
}
