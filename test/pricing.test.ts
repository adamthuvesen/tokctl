import { describe, it, expect } from 'vitest';
import { costOf, hasPrice, normalizeModelId } from '../src/pricing.js';
import type { UsageEvent } from '../src/types.js';

function event(partial: Partial<UsageEvent> & { model: string }): UsageEvent {
  return {
    source: 'claude',
    timestamp: new Date('2026-04-20T12:00:00Z'),
    sessionId: 's',
    model: partial.model,
    inputTokens: 0,
    outputTokens: 0,
    cacheReadTokens: 0,
    cacheWriteTokens: 0,
    ...partial,
  };
}

describe('pricing', () => {
  it('computes cost across all four buckets for a known model', () => {
    const e = event({
      model: 'claude-sonnet-4-6',
      inputTokens: 1_000_000,
      outputTokens: 1_000_000,
      cacheReadTokens: 1_000_000,
      cacheWriteTokens: 1_000_000,
    });
    // $3 + $15 + $0.3 + $3.75 = $22.05
    expect(costOf(e)).toBeCloseTo(22.05, 4);
  });

  it('returns 0 and records unknown model', () => {
    const unknown = new Set<string>();
    const e = event({ model: 'made-up-model', inputTokens: 9_999_999 });
    expect(costOf(e, unknown)).toBe(0);
    expect(unknown.has('made-up-model')).toBe(true);
  });

  it('returns 0 for a zero-token event even with known model', () => {
    expect(costOf(event({ model: 'claude-opus-4-7' }))).toBe(0);
  });

  it('normalizes Claude IDs with date suffix', () => {
    expect(normalizeModelId('claude-opus-4-7-20260115')).toBe('claude-opus-4-7');
    expect(hasPrice('claude-opus-4-7-20260115')).toBe(true);
  });
});
