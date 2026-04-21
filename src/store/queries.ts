import type { DB } from './db.js';
import type { AggregateRow } from './rowTypes.js';

export interface QueryFilter {
  source: 'claude' | 'codex' | null;
  sinceMs: number | null;
  untilMs: number | null;
}

function sourceClause(source: 'claude' | 'codex' | null): string {
  return source ? 'AND source = @source' : '';
}

function timeClause(): string {
  return `
    AND (@since IS NULL OR ts >= @since)
    AND (@until IS NULL OR ts <= @until)
  `;
}

interface RawDailyRow {
  key: string;
  source: 'claude' | 'codex';
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  total_tokens: number;
  cost_usd: number;
}

function toAggregateRow(r: RawDailyRow): AggregateRow {
  return {
    key: r.key,
    source: r.source,
    inputTokens: r.input_tokens,
    outputTokens: r.output_tokens,
    cacheReadTokens: r.cache_read_tokens,
    cacheWriteTokens: r.cache_write_tokens,
    totalTokens: r.total_tokens,
    costUsd: r.cost_usd,
  };
}

function buildFilterParams(f: QueryFilter): Record<string, unknown> {
  return {
    source: f.source,
    since: f.sinceMs,
    until: f.untilMs,
  };
}

export function dailyReportFromDb(db: DB, filter: QueryFilter): AggregateRow[] {
  const sql = `
    SELECT
      day AS key,
      ${filter.source ? `'${filter.source}' AS source` : "'all' AS source"},
      SUM(input)       AS input_tokens,
      SUM(output)      AS output_tokens,
      SUM(cache_read)  AS cache_read_tokens,
      SUM(cache_write) AS cache_write_tokens,
      SUM(input + output + cache_read + cache_write) AS total_tokens,
      SUM(cost_usd)    AS cost_usd
    FROM events
    WHERE 1=1
      ${sourceClause(filter.source)}
      ${timeClause()}
    GROUP BY key
    ORDER BY key ASC
  `;
  const rows = db.prepare(sql).all(buildFilterParams(filter)) as RawDailyRow[];
  return rows.map(toAggregateRow);
}

export function monthlyReportFromDb(db: DB, filter: QueryFilter): AggregateRow[] {
  const sql = `
    SELECT
      month AS key,
      ${filter.source ? `'${filter.source}' AS source` : "'all' AS source"},
      SUM(input)       AS input_tokens,
      SUM(output)      AS output_tokens,
      SUM(cache_read)  AS cache_read_tokens,
      SUM(cache_write) AS cache_write_tokens,
      SUM(input + output + cache_read + cache_write) AS total_tokens,
      SUM(cost_usd)    AS cost_usd
    FROM events
    WHERE 1=1
      ${sourceClause(filter.source)}
      ${timeClause()}
    GROUP BY key
    ORDER BY key ASC
  `;
  const rows = db.prepare(sql).all(buildFilterParams(filter)) as RawDailyRow[];
  return rows.map(toAggregateRow);
}

interface RawSessionRow {
  session_id: string;
  source: 'claude' | 'codex';
  project_path: string | null;
  latest_ts: number;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  total_tokens: number;
  cost_usd: number;
}

export function sessionReportFromDb(db: DB, filter: QueryFilter): AggregateRow[] {
  const sql = `
    SELECT
      session_id,
      source,
      MAX(project_path) AS project_path,
      MAX(ts) AS latest_ts,
      SUM(input)       AS input_tokens,
      SUM(output)      AS output_tokens,
      SUM(cache_read)  AS cache_read_tokens,
      SUM(cache_write) AS cache_write_tokens,
      SUM(input + output + cache_read + cache_write) AS total_tokens,
      SUM(cost_usd)    AS cost_usd
    FROM events
    WHERE 1=1
      ${sourceClause(filter.source)}
      ${timeClause()}
    GROUP BY source, session_id
    ORDER BY latest_ts DESC
  `;
  const rows = db.prepare(sql).all(buildFilterParams(filter)) as RawSessionRow[];
  return rows.map((r) => ({
    key: r.session_id,
    source: r.source,
    projectPath: r.project_path ?? undefined,
    latestTimestamp: new Date(r.latest_ts),
    inputTokens: r.input_tokens,
    outputTokens: r.output_tokens,
    cacheReadTokens: r.cache_read_tokens,
    cacheWriteTokens: r.cache_write_tokens,
    totalTokens: r.total_tokens,
    costUsd: r.cost_usd,
  }));
}
