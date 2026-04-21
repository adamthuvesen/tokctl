import { describe, it, expect } from 'vitest';
import { dailyReport } from '../src/reports/daily.js';
import { monthlyReport } from '../src/reports/monthly.js';
import { sessionReport } from '../src/reports/session.js';
import { filterByDate, parseSince, parseUntil } from '../src/reports/shared.js';
import type { UsageEvent } from '../src/types.js';

function ev(partial: Partial<UsageEvent> & { timestamp: Date }): UsageEvent {
  return {
    source: 'claude',
    sessionId: 'sess-1',
    model: 'claude-sonnet-4-6',
    inputTokens: 100,
    outputTokens: 50,
    cacheReadTokens: 1000,
    cacheWriteTokens: 500,
    ...partial,
  };
}

describe('daily report', () => {
  it('aggregates same local date into a single row, sorted ascending', () => {
    // Use mid-day UTC so the tz shift to any sane local zone stays on the same day.
    const events = [
      ev({ timestamp: new Date('2026-03-01T10:00:00Z') }),
      ev({ timestamp: new Date('2026-03-01T14:00:00Z') }),
      ev({ timestamp: new Date('2026-02-27T12:00:00Z') }),
    ];
    const rows = dailyReport(events, new Set());
    expect(rows).toHaveLength(2);
    expect(rows[0]!.key < rows[1]!.key).toBe(true);
    const same = rows.find((r) => r.inputTokens === 200);
    expect(same).toBeDefined();
  });

  it('respects --source filter semantics (filter happens upstream, here we only aggregate)', () => {
    const events = [
      ev({ timestamp: new Date('2026-03-01T10:00:00Z'), source: 'claude', inputTokens: 10 }),
      ev({ timestamp: new Date('2026-03-01T10:00:00Z'), source: 'codex', inputTokens: 20 }),
    ];
    const rows = dailyReport(events, new Set());
    expect(rows).toHaveLength(1);
    expect(rows[0]!.inputTokens).toBe(30);
  });
});

describe('monthly report', () => {
  it('collapses all events in a month into one row', () => {
    const events = [
      ev({ timestamp: new Date('2026-03-02T10:00:00Z') }),
      ev({ timestamp: new Date('2026-03-15T10:00:00Z') }),
      ev({ timestamp: new Date('2026-03-31T10:00:00Z') }),
      ev({ timestamp: new Date('2026-04-01T10:00:00Z') }),
    ];
    const rows = monthlyReport(events, new Set());
    expect(rows).toHaveLength(2);
    expect(rows[0]!.key).toMatch(/^2026-03$/);
    expect(rows[1]!.key).toMatch(/^2026-04$/);
  });
});

describe('session report', () => {
  it('groups by sessionId and sorts by latest activity desc', () => {
    const events = [
      ev({ sessionId: 'a', timestamp: new Date('2026-03-01T10:00:00Z') }),
      ev({ sessionId: 'a', timestamp: new Date('2026-03-02T10:00:00Z') }),
      ev({ sessionId: 'b', timestamp: new Date('2026-03-05T10:00:00Z') }),
    ];
    const rows = sessionReport(events, new Set());
    expect(rows).toHaveLength(2);
    expect(rows[0]!.key).toBe('b');
    expect(rows[1]!.key).toBe('a');
  });

  it('keeps source per row and separates same id across sources', () => {
    const events = [
      ev({ sessionId: 'x', source: 'claude', timestamp: new Date('2026-03-01T10:00:00Z') }),
      ev({ sessionId: 'x', source: 'codex', timestamp: new Date('2026-03-01T10:00:00Z') }),
    ];
    const rows = sessionReport(events, new Set());
    expect(rows).toHaveLength(2);
  });
});

describe('date filtering', () => {
  it('since excludes strictly earlier events', () => {
    const since = parseSince('2026-04-01');
    const events = [
      ev({ timestamp: new Date('2026-03-30T12:00:00Z') }),
      ev({ timestamp: new Date('2026-04-02T12:00:00Z') }),
    ];
    const filtered = filterByDate(events, since, undefined);
    expect(filtered).toHaveLength(1);
  });

  it('until excludes events after end-of-day', () => {
    const until = parseUntil('2026-04-15');
    const events = [
      ev({ timestamp: new Date('2026-04-14T12:00:00Z') }),
      ev({ timestamp: new Date('2026-04-17T12:00:00Z') }),
    ];
    const filtered = filterByDate(events, undefined, until);
    expect(filtered).toHaveLength(1);
  });

  it('rejects malformed date strings', () => {
    expect(() => parseSince('2026/04/01')).toThrow(/YYYY-MM-DD/);
    expect(() => parseUntil('not-a-date')).toThrow(/YYYY-MM-DD/);
  });
});
