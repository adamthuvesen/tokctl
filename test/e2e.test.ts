import { describe, it, expect, beforeAll, beforeEach, afterEach } from 'vitest';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import path from 'node:path';
import fs from 'node:fs';
import os from 'node:os';

const exec = promisify(execFile);
const CLI = path.resolve('dist/cli.js');
const FIX_CLAUDE = path.resolve('test/fixtures/claude');
const FIX_CODEX = path.resolve('test/fixtures/codex');

let tmpDir = '';

beforeAll(() => {
  if (!fs.existsSync(CLI)) {
    throw new Error(`dist/cli.js missing — run \`npm run build\` before \`npm test\`.`);
  }
});

beforeEach(() => {
  tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'tokctl-e2e-'));
});

afterEach(() => {
  try {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  } catch {
    // ignore
  }
});

async function run(args: string[]) {
  return exec('node', [CLI, ...args], {
    env: {
      ...process.env,
      TOKCTL_CLAUDE_DIR: '',
      TOKCTL_CODEX_DIR: '',
      CLAUDE_CONFIG_DIR: '',
      CODEX_HOME: '',
      TOKCTL_CACHE_DIR: tmpDir,
      HOME: '/tmp',
    },
  });
}

describe('e2e — daily --json --source all', () => {
  it('returns a JSON array covering both sources from fixtures', async () => {
    const { stdout } = await run([
      'daily',
      '--json',
      '--source', 'all',
      '--claude-dir', FIX_CLAUDE,
      '--codex-dir', FIX_CODEX,
    ]);
    const rows = JSON.parse(stdout) as Array<Record<string, unknown>>;
    expect(Array.isArray(rows)).toBe(true);
    const dates = rows.map((r) => r.date);
    expect(rows.length).toBeGreaterThanOrEqual(1);
    const sorted = [...dates].sort();
    expect(dates).toEqual(sorted);
    for (const r of rows) {
      expect(r).toHaveProperty('input');
      expect(r).toHaveProperty('output');
      expect(r).toHaveProperty('cache_read');
      expect(r).toHaveProperty('cache_write');
      expect(r).toHaveProperty('totalTokens');
      expect(r).toHaveProperty('costUsd');
    }
  });
});

describe('e2e — session --json', () => {
  it('rolls up per session and sorts by latest activity desc', async () => {
    const { stdout } = await run([
      'session',
      '--json',
      '--claude-dir', FIX_CLAUDE,
      '--codex-dir', FIX_CODEX,
    ]);
    const rows = JSON.parse(stdout) as Array<Record<string, unknown>>;
    expect(rows.length).toBe(2);
    const latest = rows.map((r) => new Date(r.latest_timestamp as string).getTime());
    for (let i = 1; i < latest.length; i++) {
      expect(latest[i - 1]).toBeGreaterThanOrEqual(latest[i]!);
    }
  });
});

describe('e2e — daily --since/--until', () => {
  it('excludes events outside the window', async () => {
    const { stdout } = await run([
      'daily',
      '--json',
      '--since', '2026-04-20',
      '--until', '2026-04-20',
      '--claude-dir', FIX_CLAUDE,
      '--codex-dir', FIX_CODEX,
    ]);
    const rows = JSON.parse(stdout) as Array<Record<string, unknown>>;
    for (const r of rows) {
      expect(r.date).toBe('2026-04-20');
    }
  });
});

describe('e2e — error codes', () => {
  it('exits 2 when an explicit dir is missing', async () => {
    await expect(
      run(['daily', '--claude-dir', '/does/not/exist', '--codex-dir', FIX_CODEX]),
    ).rejects.toMatchObject({ code: 2 });
  });

  it('exits 1 on malformed --since', async () => {
    await expect(
      run(['daily', '--since', 'nope', '--claude-dir', FIX_CLAUDE, '--codex-dir', FIX_CODEX]),
    ).rejects.toMatchObject({ code: 1 });
  });

  it('exits 1 when --rebuild and --no-cache are both passed', async () => {
    await expect(
      run(['daily', '--rebuild', '--no-cache', '--claude-dir', FIX_CLAUDE, '--codex-dir', FIX_CODEX]),
    ).rejects.toMatchObject({ code: 1 });
  });
});

describe('e2e — cache behavior', () => {
  it('creates the cache DB on first run and reuses it on second run', async () => {
    const dbPath = path.join(tmpDir, 'tokctl.db');
    expect(fs.existsSync(dbPath)).toBe(false);

    const { stdout: first } = await run([
      'daily', '--json', '--claude-dir', FIX_CLAUDE, '--codex-dir', FIX_CODEX,
    ]);
    expect(fs.existsSync(dbPath)).toBe(true);

    const { stdout: second } = await run([
      'daily', '--json', '--claude-dir', FIX_CLAUDE, '--codex-dir', FIX_CODEX,
    ]);
    expect(JSON.parse(second)).toEqual(JSON.parse(first));
  });

  it('--rebuild nukes and recreates the DB', async () => {
    const dbPath = path.join(tmpDir, 'tokctl.db');
    await run(['daily', '--json', '--claude-dir', FIX_CLAUDE, '--codex-dir', FIX_CODEX]);
    const sizeBefore = fs.statSync(dbPath).size;
    const mtimeBefore = fs.statSync(dbPath).mtimeMs;

    // Wait a few ms so mtime can advance.
    await new Promise((r) => setTimeout(r, 10));
    await run([
      'daily', '--json', '--rebuild',
      '--claude-dir', FIX_CLAUDE, '--codex-dir', FIX_CODEX,
    ]);
    const mtimeAfter = fs.statSync(dbPath).mtimeMs;
    expect(mtimeAfter).toBeGreaterThan(mtimeBefore);
    // Size should be similar (same data set).
    expect(Math.abs(fs.statSync(dbPath).size - sizeBefore)).toBeLessThan(8192);
  });

  it('--no-cache does not create the DB', async () => {
    const dbPath = path.join(tmpDir, 'tokctl.db');
    await run([
      'daily', '--json', '--no-cache',
      '--claude-dir', FIX_CLAUDE, '--codex-dir', FIX_CODEX,
    ]);
    expect(fs.existsSync(dbPath)).toBe(false);
  });

  it('tail-read picks up new events appended to a fixture file', async () => {
    // Copy fixtures to a writable temp dir so we can append.
    const liveClaude = path.join(tmpDir, 'fix-claude');
    fs.cpSync(FIX_CLAUDE, liveClaude, { recursive: true });

    // First run indexes everything currently there.
    const firstRes = await run([
      'daily', '--json',
      '--claude-dir', liveClaude, '--codex-dir', FIX_CODEX,
    ]);
    const firstRows = JSON.parse(firstRes.stdout) as Array<Record<string, number>>;
    const firstTotalInput = firstRows.reduce((s, r) => s + (r.input ?? 0), 0);

    // Append one new assistant usage row to an existing claude session file.
    const sessFile = path.join(liveClaude, '-Users-dev-tokctl', 'sess-a.jsonl');
    const appended = {
      type: 'assistant',
      timestamp: '2026-04-21T00:00:00.000Z',
      sessionId: 'sess-a',
      message: {
        id: 'm-append-1',
        model: 'claude-sonnet-4-6',
        role: 'assistant',
        usage: {
          input_tokens: 1234,
          output_tokens: 56,
          cache_read_input_tokens: 0,
          cache_creation_input_tokens: 0,
        },
      },
    };
    fs.appendFileSync(sessFile, JSON.stringify(appended) + '\n');

    const secondRes = await run([
      'daily', '--json',
      '--claude-dir', liveClaude, '--codex-dir', FIX_CODEX,
    ]);
    const secondRows = JSON.parse(secondRes.stdout) as Array<Record<string, number>>;
    const secondTotalInput = secondRows.reduce((s, r) => s + (r.input ?? 0), 0);

    expect(secondTotalInput - firstTotalInput).toBe(1234);
  });
});

describe('e2e — export-db', () => {
  it('prints the resolved cache path and does not create it', async () => {
    const dbPath = path.join(tmpDir, 'tokctl.db');
    expect(fs.existsSync(dbPath)).toBe(false);
    const { stdout } = await run(['export-db']);
    expect(stdout.trim()).toBe(dbPath);
    expect(fs.existsSync(dbPath)).toBe(false);
  });
});
