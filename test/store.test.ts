import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { closeStore, openStore } from '../src/store/db.js';
import { insertEvents, upsertFileManifest, loadFileManifest } from '../src/store/writes.js';
import { dailyReportFromDb, monthlyReportFromDb, sessionReportFromDb } from '../src/store/queries.js';
import { SCHEMA_VERSION } from '../src/store/schema.js';

let tmpDir = '';
let dbPath = '';

beforeEach(() => {
  tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'tokctl-store-'));
  dbPath = path.join(tmpDir, 'tokctl.db');
});

afterEach(() => {
  try {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  } catch {
    // ignore
  }
});

function e(ov: Partial<Parameters<typeof insertEvents>[1][number]> = {}) {
  return {
    file_path: 'x.jsonl',
    source: 'claude' as const,
    ts: new Date('2026-03-10T12:00:00Z').getTime(),
    day: '2026-03-10',
    month: '2026-03',
    session_id: 'sess-1',
    project_path: '/repo',
    model: 'claude-sonnet-4-6',
    input: 100,
    output: 50,
    cache_read: 200,
    cache_write: 0,
    cost_usd: 1.23,
    ...ov,
  };
}

describe('openStore', () => {
  it('creates schema and records the version', () => {
    const db = openStore({ path: dbPath });
    const row = db.prepare("SELECT value FROM meta WHERE key='schema_version'").get() as { value: string };
    expect(Number(row.value)).toBe(SCHEMA_VERSION);
    closeStore(db);
  });

  it('rebuilds when the schema version does not match', () => {
    // Write a DB with a bogus schema version.
    const db1 = openStore({ path: dbPath });
    db1.prepare("INSERT OR REPLACE INTO meta (key,value) VALUES ('schema_version','0')").run();
    closeStore(db1);

    let rebuildReason: string | null = null;
    const db2 = openStore({ path: dbPath, onRebuild: (r) => (rebuildReason = r) });
    expect(rebuildReason).toMatch(/schema version mismatch/);
    const row = db2.prepare("SELECT value FROM meta WHERE key='schema_version'").get() as { value: string };
    expect(Number(row.value)).toBe(SCHEMA_VERSION);
    closeStore(db2);
  });

  it('rebuilds when the file is not a valid sqlite db', () => {
    fs.mkdirSync(path.dirname(dbPath), { recursive: true });
    fs.writeFileSync(dbPath, 'not a database');
    let rebuildReason: string | null = null;
    const db = openStore({ path: dbPath, onRebuild: (r) => (rebuildReason = r) });
    expect(rebuildReason).toBeTruthy();
    // Smoke check: events table exists.
    const count = db.prepare('SELECT COUNT(*) AS n FROM events').get() as { n: number };
    expect(count.n).toBe(0);
    closeStore(db);
  });
});

describe('events round-trip', () => {
  it('insert → select gives back the expected row count and sums', () => {
    const db = openStore({ path: dbPath });
    upsertFileManifest(db, {
      path: 'x.jsonl', source: 'claude', project: '/repo',
      size: 100, mtime_ns: 1n, last_offset: 100, n_events: 2,
      session_id: null, model: null,
    });
    insertEvents(db, [e({ input: 10 }), e({ input: 20, ts: new Date('2026-03-11T12:00:00Z').getTime(), day: '2026-03-11' })]);
    const rows = dailyReportFromDb(db, { source: null, sinceMs: null, untilMs: null });
    expect(rows).toHaveLength(2);
    const total = rows.reduce((s, r) => s + r.inputTokens, 0);
    expect(total).toBe(30);
    closeStore(db);
  });
});

describe('SQL report queries', () => {
  function seed(db: ReturnType<typeof openStore>) {
    upsertFileManifest(db, { path: 'f.jsonl', source: 'claude', project: null, size: 1, mtime_ns: 1n, last_offset: 1, n_events: 0, session_id: null, model: null });
    insertEvents(db, [
      e({ day: '2026-03-01', month: '2026-03', source: 'claude', session_id: 'a', ts: new Date('2026-03-01T10:00:00Z').getTime(), input: 10 }),
      e({ day: '2026-03-01', month: '2026-03', source: 'codex',  session_id: 'b', ts: new Date('2026-03-01T11:00:00Z').getTime(), input: 20 }),
      e({ day: '2026-03-02', month: '2026-03', source: 'claude', session_id: 'a', ts: new Date('2026-03-02T10:00:00Z').getTime(), input: 30 }),
      e({ day: '2026-04-15', month: '2026-04', source: 'codex',  session_id: 'c', ts: new Date('2026-04-15T10:00:00Z').getTime(), input: 40 }),
    ]);
  }

  it('daily: source=null collapses across sources, ascending by day', () => {
    const db = openStore({ path: dbPath });
    seed(db);
    const rows = dailyReportFromDb(db, { source: null, sinceMs: null, untilMs: null });
    expect(rows.map((r) => r.key)).toEqual(['2026-03-01', '2026-03-02', '2026-04-15']);
    expect(rows[0]!.inputTokens).toBe(30); // 10 + 20
    closeStore(db);
  });

  it('daily: source filter restricts to that source', () => {
    const db = openStore({ path: dbPath });
    seed(db);
    const rows = dailyReportFromDb(db, { source: 'codex', sinceMs: null, untilMs: null });
    expect(rows.map((r) => r.key)).toEqual(['2026-03-01', '2026-04-15']);
    closeStore(db);
  });

  it('daily: since/until filters', () => {
    const db = openStore({ path: dbPath });
    seed(db);
    const from = new Date('2026-03-02T00:00:00Z').getTime();
    const to = new Date('2026-03-31T23:59:59Z').getTime();
    const rows = dailyReportFromDb(db, { source: null, sinceMs: from, untilMs: to });
    expect(rows.map((r) => r.key)).toEqual(['2026-03-02']);
    closeStore(db);
  });

  it('monthly: groups by YYYY-MM', () => {
    const db = openStore({ path: dbPath });
    seed(db);
    const rows = monthlyReportFromDb(db, { source: null, sinceMs: null, untilMs: null });
    expect(rows.map((r) => r.key)).toEqual(['2026-03', '2026-04']);
    closeStore(db);
  });

  it('session: grouped by (source, session_id), sorted by latest desc', () => {
    const db = openStore({ path: dbPath });
    seed(db);
    const rows = sessionReportFromDb(db, { source: null, sinceMs: null, untilMs: null });
    expect(rows).toHaveLength(3); // a (claude), b (codex), c (codex)
    const latest = rows.map((r) => r.latestTimestamp!.getTime());
    for (let i = 1; i < latest.length; i++) {
      expect(latest[i - 1]).toBeGreaterThanOrEqual(latest[i]!);
    }
    closeStore(db);
  });
});

describe('loadFileManifest', () => {
  it('round-trips manifest rows', () => {
    const db = openStore({ path: dbPath });
    upsertFileManifest(db, {
      path: 'f.jsonl', source: 'claude', project: '/p',
      size: 500, mtime_ns: 12345n, last_offset: 500, n_events: 3,
      session_id: 'sess', model: 'm',
    });
    const m = loadFileManifest(db);
    expect(m.size).toBe(1);
    const row = m.get('f.jsonl')!;
    expect(row.size).toBe(500);
    expect(row.n_events).toBe(3);
    closeStore(db);
  });
});
