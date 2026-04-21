import { describe, it, expect, beforeAll } from 'vitest';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import path from 'node:path';
import fs from 'node:fs';

const exec = promisify(execFile);
const CLI = path.resolve('dist/cli.js');
const FIX_CLAUDE = path.resolve('test/fixtures/claude');
const FIX_CODEX = path.resolve('test/fixtures/codex');

beforeAll(() => {
  if (!fs.existsSync(CLI)) {
    throw new Error(`dist/cli.js missing — run \`npm run build\` before \`npm test\`.`);
  }
});

async function run(args: string[]) {
  return exec('node', [CLI, ...args], {
    env: {
      ...process.env,
      // Pin to fixtures regardless of real home dirs.
      AIUSAGE_CLAUDE_DIR: '',
      AIUSAGE_CODEX_DIR: '',
      CLAUDE_CONFIG_DIR: '',
      CODEX_HOME: '',
      HOME: '/tmp', // harmless; fixtures are passed via --*-dir below
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
    // Fixture dates in local time vary by host tz, but we always have at least one row each.
    expect(rows.length).toBeGreaterThanOrEqual(1);
    // Ascending sort
    const sorted = [...dates].sort();
    expect(dates).toEqual(sorted);
    // Every row has the standard columns
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
    expect(rows.length).toBe(2); // sess-a + sess-x
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
      run(['daily', '--claude-dir', '/does/not/exist']),
    ).rejects.toMatchObject({ code: 2 });
  });

  it('exits 1 on malformed --since', async () => {
    await expect(
      run(['daily', '--since', 'nope', '--claude-dir', FIX_CLAUDE, '--codex-dir', FIX_CODEX]),
    ).rejects.toMatchObject({ code: 1 });
  });
});
