use crate::types::{AggregateRow, Source, SourceLabel};
use anyhow::Result;
use chrono::{DateTime, TimeZone, Utc};
use rusqlite::{params_from_iter, types::Value, Connection};

#[derive(Debug, Clone, Copy, Default)]
pub struct QueryFilter {
    pub source: Option<Source>,
    pub since_ms: Option<i64>,
    pub until_ms: Option<i64>,
}

fn build_params(f: QueryFilter) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    if let Some(src) = f.source {
        out.push(Value::Text(src.as_str().to_owned()));
    }
    out.push(match f.since_ms {
        Some(v) => Value::Integer(v),
        None => Value::Null,
    });
    out.push(match f.since_ms {
        Some(v) => Value::Integer(v),
        None => Value::Null,
    });
    out.push(match f.until_ms {
        Some(v) => Value::Integer(v),
        None => Value::Null,
    });
    out.push(match f.until_ms {
        Some(v) => Value::Integer(v),
        None => Value::Null,
    });
    out
}

fn source_clause(f: QueryFilter) -> &'static str {
    if f.source.is_some() {
        "AND source = ?"
    } else {
        ""
    }
}

fn time_clause() -> &'static str {
    "AND (? IS NULL OR ts >= ?) AND (? IS NULL OR ts <= ?)"
}

fn source_label(f: QueryFilter) -> SourceLabel {
    match f.source {
        Some(s) => SourceLabel::Source(s),
        None => SourceLabel::All,
    }
}

pub fn daily_report(conn: &Connection, filter: QueryFilter) -> Result<Vec<AggregateRow>> {
    let sql = format!(
        r#"SELECT
             day AS key,
             SUM(input)       AS input_tokens,
             SUM(output)      AS output_tokens,
             SUM(cache_read)  AS cache_read_tokens,
             SUM(cache_write) AS cache_write_tokens,
             SUM(input + output + cache_read + cache_write) AS total_tokens,
             SUM(cost_usd)    AS cost_usd
           FROM events
           WHERE 1=1 {src} {ts}
           GROUP BY key
           ORDER BY key ASC"#,
        src = source_clause(filter),
        ts = time_clause(),
    );
    run_bucket_query(conn, &sql, filter, source_label(filter))
}

pub fn monthly_report(conn: &Connection, filter: QueryFilter) -> Result<Vec<AggregateRow>> {
    let sql = format!(
        r#"SELECT
             month AS key,
             SUM(input)       AS input_tokens,
             SUM(output)      AS output_tokens,
             SUM(cache_read)  AS cache_read_tokens,
             SUM(cache_write) AS cache_write_tokens,
             SUM(input + output + cache_read + cache_write) AS total_tokens,
             SUM(cost_usd)    AS cost_usd
           FROM events
           WHERE 1=1 {src} {ts}
           GROUP BY key
           ORDER BY key ASC"#,
        src = source_clause(filter),
        ts = time_clause(),
    );
    run_bucket_query(conn, &sql, filter, source_label(filter))
}

fn run_bucket_query(
    conn: &Connection,
    sql: &str,
    filter: QueryFilter,
    label: SourceLabel,
) -> Result<Vec<AggregateRow>> {
    let params = build_params(filter);
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params_from_iter(params.iter()), |row| {
        Ok(AggregateRow {
            key: row.get(0)?,
            source: label,
            project_path: None,
            latest_timestamp: None,
            input_tokens: row.get::<_, i64>(1)? as u64,
            output_tokens: row.get::<_, i64>(2)? as u64,
            cache_read_tokens: row.get::<_, i64>(3)? as u64,
            cache_write_tokens: row.get::<_, i64>(4)? as u64,
            total_tokens: row.get::<_, i64>(5)? as u64,
            cost_usd: row.get(6)?,
        })
    })?;
    Ok(rows.filter_map(std::result::Result::ok).collect())
}

pub fn session_report(conn: &Connection, filter: QueryFilter) -> Result<Vec<AggregateRow>> {
    let sql = format!(
        r#"SELECT
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
           WHERE 1=1 {src} {ts}
           GROUP BY source, session_id
           ORDER BY latest_ts DESC"#,
        src = source_clause(filter),
        ts = time_clause(),
    );
    let params = build_params(filter);
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(params.iter()), |row| {
        let session_id: String = row.get(0)?;
        let src_str: String = row.get(1)?;
        let source = match src_str.as_str() {
            "claude" => Source::Claude,
            _ => Source::Codex,
        };
        let project_path: Option<String> = row.get(2)?;
        let latest_ms: i64 = row.get(3)?;
        let latest_ts: DateTime<Utc> = Utc
            .timestamp_millis_opt(latest_ms)
            .single()
            .unwrap_or_else(Utc::now);
        Ok(AggregateRow {
            key: session_id,
            source: SourceLabel::Source(source),
            project_path,
            latest_timestamp: Some(latest_ts),
            input_tokens: row.get::<_, i64>(4)? as u64,
            output_tokens: row.get::<_, i64>(5)? as u64,
            cache_read_tokens: row.get::<_, i64>(6)? as u64,
            cache_write_tokens: row.get::<_, i64>(7)? as u64,
            total_tokens: row.get::<_, i64>(8)? as u64,
            cost_usd: row.get(9)?,
        })
    })?;
    Ok(rows.filter_map(std::result::Result::ok).collect())
}
