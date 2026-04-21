import { describe, it, expect } from 'vitest';
import path from 'node:path';
import { ingestClaude } from '../src/sources/claude.js';
import { ingestCodex } from '../src/sources/codex.js';

const FIX_CLAUDE = path.resolve('test/fixtures/claude');
const FIX_CODEX = path.resolve('test/fixtures/codex');

describe('Claude source', () => {
  it('reads fixture JSONL, dedupes by message id, tolerates bad lines', async () => {
    const { events, stats } = await ingestClaude({ flag: FIX_CLAUDE, env: {} });
    // m1 + m2 + m3 = 3 unique (m2 appears twice; second occurrence is deduped)
    expect(events).toHaveLength(3);
    expect(stats.skippedLines).toBeGreaterThanOrEqual(1);
    for (const ev of events) {
      expect(ev.source).toBe('claude');
      expect(ev.sessionId).toBe('sess-a');
      expect(ev.projectPath).toBe('/Users/dev/tokctl');
    }
  });

  it('errors when a user-supplied root does not exist', async () => {
    await expect(
      ingestClaude({ flag: '/does/not/exist/claude', env: {} }),
    ).rejects.toThrow(/Claude directory not readable/);
  });
});

describe('Codex source', () => {
  it('reads rollout, sums per-turn deltas, ignores session_meta and non-token rows', async () => {
    const { events, stats } = await ingestCodex({ flag: FIX_CODEX, env: {} });
    expect(events).toHaveLength(3); // 3 token_count rows
    const totalInput = events.reduce((s, e) => s + e.inputTokens, 0);
    // 200 + 400 + 50 = 650 (uses last_token_usage, not total_token_usage)
    expect(totalInput).toBe(650);
    const totalCacheRead = events.reduce((s, e) => s + e.cacheReadTokens, 0);
    expect(totalCacheRead).toBe(200); // 50 + 150 + 0
    // reasoning_output_tokens folded into outputTokens
    const totalOutput = events.reduce((s, e) => s + e.outputTokens, 0);
    expect(totalOutput).toBe(195); // (60+10) + (80+20) + (25+0)
    for (const ev of events) {
      expect(ev.source).toBe('codex');
      expect(ev.sessionId).toBe('sess-x');
      expect(ev.model).toBe('gpt-5.4');
      expect(ev.projectPath).toBe('/Users/dev/repo');
      expect(ev.cacheWriteTokens).toBe(0);
    }
    expect(stats.skippedLines).toBeGreaterThanOrEqual(1);
  });

  it('errors when a user-supplied root does not exist', async () => {
    await expect(
      ingestCodex({ flag: '/does/not/exist/codex', env: {} }),
    ).rejects.toThrow(/Codex directory not readable/);
  });

  it('does not throw when a default root is missing', async () => {
    // No flag, no env — will try real defaults. On CI they may not exist.
    // We only care that it does not throw.
    const result = await ingestCodex({ env: {} });
    expect(result).toBeDefined();
  });
});
