use crate::repo::project_basename;
use crate::types::{AggregateRow, Source, SourceLabel};
use anyhow::Result;
use chrono::{DateTime, TimeZone, Utc};
use rusqlite::{
    params_from_iter,
    types::{Type, Value},
    Connection,
};
use std::str::FromStr;

/// Sentinel string used in SQL to represent the "(no-repo)" bucket when
/// repo IS NULL. Chosen to be unambiguous against any real repo key.
pub const NO_REPO_SENTINEL: &str = "<NO_REPO>";

#[derive(Debug, Clone)]
pub enum RepoFilterSpec {
    /// Only events with `repo IS NULL`.
    NoRepo,
    /// Only events whose repo has the given display name.
    DisplayName(String),
    /// Only events whose repo key starts with the given path prefix.
    KeyPrefix(String),
}

#[derive(Debug, Clone, Default)]
pub struct QueryFilter {
    pub source: Option<Source>,
    pub since_ms: Option<i64>,
    pub until_ms: Option<i64>,
    pub repo: Option<RepoFilterSpec>,
}

fn source_clause(f: &QueryFilter) -> &'static str {
    if f.source.is_some() {
        "AND e.source = ?"
    } else {
        ""
    }
}

fn time_clause() -> &'static str {
    "AND (? IS NULL OR e.ts >= ?) AND (? IS NULL OR e.ts <= ?)"
}

fn repo_clause(f: &QueryFilter) -> &'static str {
    match &f.repo {
        None => "",
        Some(RepoFilterSpec::NoRepo) => "AND e.repo IS NULL",
        Some(RepoFilterSpec::DisplayName(_)) => {
            "AND e.repo IN (SELECT key FROM repos WHERE display_name = ?)"
        }
        Some(RepoFilterSpec::KeyPrefix(_)) => "AND e.repo IS NOT NULL AND e.repo LIKE ?",
    }
}

fn build_params(f: &QueryFilter) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    if let Some(src) = f.source {
        out.push(Value::Text(src.as_str().to_owned()));
    }
    let opt_int = |v: Option<i64>| v.map(Value::Integer).unwrap_or(Value::Null);
    // time_clause() binds each of since/until twice (NULL-check, then compare).
    out.push(opt_int(f.since_ms));
    out.push(opt_int(f.since_ms));
    out.push(opt_int(f.until_ms));
    out.push(opt_int(f.until_ms));
    match &f.repo {
        None | Some(RepoFilterSpec::NoRepo) => {}
        Some(RepoFilterSpec::DisplayName(n)) => out.push(Value::Text(n.clone())),
        Some(RepoFilterSpec::KeyPrefix(p)) => {
            // LIKE with % suffix. SQLite LIKE is case-insensitive for ASCII
            // by default; repo keys are OS paths so we keep defaults.
            let mut pat = p.clone();
            pat.push('%');
            out.push(Value::Text(pat));
        }
    }
    out
}

fn source_label(f: &QueryFilter) -> SourceLabel {
    match f.source {
        Some(s) => SourceLabel::Source(s),
        None => SourceLabel::All,
    }
}

pub(crate) fn parse_source_column(raw: String, col: usize) -> rusqlite::Result<Source> {
    Source::from_str(&raw).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(
            col,
            Type::Text,
            Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, err)),
        )
    })
}

pub fn daily_report(conn: &Connection, filter: QueryFilter) -> Result<Vec<AggregateRow>> {
    let sql = format!(
        r#"SELECT
             e.day AS key,
             SUM(e.input)       AS input_tokens,
             SUM(e.output)      AS output_tokens,
             SUM(e.cache_read)  AS cache_read_tokens,
             SUM(e.cache_write) AS cache_write_tokens,
             SUM(e.input + e.output + e.cache_read + e.cache_write) AS total_tokens,
             SUM(e.cost_usd)    AS cost_usd
           FROM events e
           WHERE 1=1 {src} {ts} {repo}
           GROUP BY key
           ORDER BY key ASC"#,
        src = source_clause(&filter),
        ts = time_clause(),
        repo = repo_clause(&filter),
    );
    run_bucket_query(conn, &sql, &filter, source_label(&filter))
}

pub fn monthly_report(conn: &Connection, filter: QueryFilter) -> Result<Vec<AggregateRow>> {
    let sql = format!(
        r#"SELECT
             e.month AS key,
             SUM(e.input)       AS input_tokens,
             SUM(e.output)      AS output_tokens,
             SUM(e.cache_read)  AS cache_read_tokens,
             SUM(e.cache_write) AS cache_write_tokens,
             SUM(e.input + e.output + e.cache_read + e.cache_write) AS total_tokens,
             SUM(e.cost_usd)    AS cost_usd
           FROM events e
           WHERE 1=1 {src} {ts} {repo}
           GROUP BY key
           ORDER BY key ASC"#,
        src = source_clause(&filter),
        ts = time_clause(),
        repo = repo_clause(&filter),
    );
    run_bucket_query(conn, &sql, &filter, source_label(&filter))
}

fn run_bucket_query(
    conn: &Connection,
    sql: &str,
    filter: &QueryFilter,
    label: SourceLabel,
) -> Result<Vec<AggregateRow>> {
    let params = build_params(filter);
    let mut stmt = conn.prepare_cached(sql)?;
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
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn session_report(conn: &Connection, filter: QueryFilter) -> Result<Vec<AggregateRow>> {
    let sql = format!(
        r#"WITH filtered AS (
             SELECT e.*
             FROM events e
             WHERE 1=1 {src} {ts} {repo}
           )
           SELECT
             e.session_id,
             e.source,
             MAX(r.display_name)  AS repo_display,
             (
               SELECT fp.project_path
               FROM filtered fp
               WHERE fp.source = e.source
                 AND fp.session_id = e.session_id
                 AND fp.project_path IS NOT NULL
               ORDER BY fp.id ASC
               LIMIT 1
             ) AS project_path,
             MAX(e.ts) AS latest_ts,
             SUM(e.input)       AS input_tokens,
             SUM(e.output)      AS output_tokens,
             SUM(e.cache_read)  AS cache_read_tokens,
             SUM(e.cache_write) AS cache_write_tokens,
             SUM(e.input + e.output + e.cache_read + e.cache_write) AS total_tokens,
             SUM(e.cost_usd)    AS cost_usd
           FROM filtered e
           LEFT JOIN repos r ON r.key = e.repo
           GROUP BY e.source, e.session_id
           ORDER BY latest_ts DESC"#,
        src = source_clause(&filter),
        ts = time_clause(),
        repo = repo_clause(&filter),
    );
    let params = build_params(&filter);
    let mut stmt = conn.prepare_cached(&sql)?;
    let rows = stmt.query_map(params_from_iter(params.iter()), |row| {
        let session_id: String = row.get(0)?;
        let src_str: String = row.get(1)?;
        let source = parse_source_column(src_str, 1)?;
        let repo_display: Option<String> = row.get(2)?;
        let project_path: Option<String> = row.get(3)?;
        let latest_ms: i64 = row.get(4)?;
        let latest_ts: DateTime<Utc> = Utc
            .timestamp_millis_opt(latest_ms)
            .single()
            .unwrap_or_else(Utc::now);
        // Prefer the resolved repo display name; fall back to a basename of
        // the raw project_path so rows stay scannable even without a repo.
        let shown = repo_display.or_else(|| {
            project_path
                .as_deref()
                .map(project_basename)
                .map(String::from)
        });
        Ok(AggregateRow {
            key: session_id,
            source: SourceLabel::Source(source),
            project_path: shown,
            latest_timestamp: Some(latest_ts),
            input_tokens: row.get::<_, i64>(5)? as u64,
            output_tokens: row.get::<_, i64>(6)? as u64,
            cache_read_tokens: row.get::<_, i64>(7)? as u64,
            cache_write_tokens: row.get::<_, i64>(8)? as u64,
            total_tokens: row.get::<_, i64>(9)? as u64,
            cost_usd: row.get(10)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Row shape for the repo-grouped report.
#[derive(Debug, Clone)]
pub struct RepoAggregateRow {
    /// Canonical repo key, or the NO_REPO_SENTINEL for the `(no-repo)` row.
    pub key: String,
    pub display_name: String,
    pub origin_url: Option<String>,
    pub sessions: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub total_tokens: u64,
    pub cost_usd: f64,
    pub latest_timestamp: DateTime<Utc>,
}

impl RepoAggregateRow {
    pub fn is_no_repo(&self) -> bool {
        self.key == NO_REPO_SENTINEL
    }
}

pub fn repo_report(conn: &Connection, filter: QueryFilter) -> Result<Vec<RepoAggregateRow>> {
    let sql = format!(
        r#"SELECT
             COALESCE(e.repo, '{no_repo}') AS key,
             COALESCE(r.display_name, '{no_repo_name}') AS display_name,
             r.origin_url,
             COUNT(DISTINCT e.source || char(31) || e.session_id) AS sessions,
             SUM(e.input)       AS input_tokens,
             SUM(e.output)      AS output_tokens,
             SUM(e.cache_read)  AS cache_read_tokens,
             SUM(e.cache_write) AS cache_write_tokens,
             SUM(e.input + e.output + e.cache_read + e.cache_write) AS total_tokens,
             SUM(e.cost_usd)    AS cost_usd,
             MAX(e.ts)          AS latest_ts
           FROM events e
           LEFT JOIN repos r ON r.key = e.repo
           WHERE 1=1 {src} {ts} {repo}
           GROUP BY COALESCE(e.repo, '{no_repo}')
           ORDER BY cost_usd DESC"#,
        no_repo = NO_REPO_SENTINEL,
        no_repo_name = crate::repo::RepoIdentity::NO_REPO_DISPLAY,
        src = source_clause(&filter),
        ts = time_clause(),
        repo = repo_clause(&filter),
    );
    let params = build_params(&filter);
    let mut stmt = conn.prepare_cached(&sql)?;
    let rows = stmt.query_map(params_from_iter(params.iter()), |row| {
        Ok(RepoAggregateRow {
            key: row.get(0)?,
            display_name: row.get(1)?,
            origin_url: row.get(2)?,
            sessions: row.get::<_, i64>(3)? as u64,
            input_tokens: row.get::<_, i64>(4)? as u64,
            output_tokens: row.get::<_, i64>(5)? as u64,
            cache_read_tokens: row.get::<_, i64>(6)? as u64,
            cache_write_tokens: row.get::<_, i64>(7)? as u64,
            total_tokens: row.get::<_, i64>(8)? as u64,
            cost_usd: row.get(9)?,
            latest_timestamp: Utc
                .timestamp_millis_opt(row.get(10)?)
                .single()
                .unwrap_or_else(Utc::now),
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Resolve a user-supplied repo name (`--repo <name>` or `tokctl repo <name>`)
/// into a concrete [`RepoFilterSpec`]. Ambiguity — multiple repos with the
/// same display name, none of which match a path prefix — returns an error
/// listing the candidates.
///
/// The literal value `(no-repo)` maps to [`RepoFilterSpec::NoRepo`].
/// Time bucket granularity for period-style aggregations (TUI days axis, etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeriodGranularity {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

impl PeriodGranularity {
    fn bucket_expr(self) -> &'static str {
        match self {
            PeriodGranularity::Daily => "e.day",
            PeriodGranularity::Weekly => "strftime('%Y-W%W', e.day)",
            PeriodGranularity::Monthly => "e.month",
            PeriodGranularity::Yearly => "substr(e.day, 1, 4)",
        }
    }
}

/// Row returned by [`period_buckets`] and [`model_buckets`].
#[derive(Debug, Clone)]
pub struct BucketAggregateRow {
    pub key: String,
    pub sessions: u64,
    pub cost_usd: f64,
    pub total_tokens: u64,
    pub latest_ts_ms: i64,
}

pub fn period_buckets(
    conn: &Connection,
    filter: QueryFilter,
    granularity: PeriodGranularity,
) -> Result<Vec<BucketAggregateRow>> {
    let sql = format!(
        r#"SELECT {bucket} AS bucket,
                  COUNT(DISTINCT e.source || char(31) || e.session_id) AS sessions,
                  SUM(e.cost_usd) AS cost,
                  SUM(e.input + e.output + e.cache_read + e.cache_write) AS total_tokens,
                  MAX(e.ts) AS latest_ts
             FROM events e
             WHERE 1=1 {src} {ts}
             GROUP BY bucket
             ORDER BY bucket DESC"#,
        bucket = granularity.bucket_expr(),
        src = source_clause(&filter),
        ts = time_clause(),
    );
    run_bucket_aggregate_query(conn, &sql, &filter)
}

/// Model-grouped buckets for the TUI models section (no repo filter — matches prior TUI SQL).
pub fn model_buckets(conn: &Connection, filter: QueryFilter) -> Result<Vec<BucketAggregateRow>> {
    let sql = format!(
        r#"SELECT e.model AS model,
                  COUNT(DISTINCT e.source || char(31) || e.session_id) AS sessions,
                  SUM(e.cost_usd) AS cost,
                  SUM(e.input + e.output + e.cache_read + e.cache_write) AS total_tokens,
                  MAX(e.ts) AS latest_ts
             FROM events e
             WHERE 1=1 {src} {ts}
             GROUP BY model
             ORDER BY cost DESC"#,
        src = source_clause(&filter),
        ts = time_clause(),
    );
    run_bucket_aggregate_query(conn, &sql, &filter)
}

fn run_bucket_aggregate_query(
    conn: &Connection,
    sql: &str,
    filter: &QueryFilter,
) -> Result<Vec<BucketAggregateRow>> {
    let params = build_params(filter);
    let mut stmt = conn.prepare_cached(sql)?;
    let rows = stmt.query_map(params_from_iter(params.iter()), |row| {
        Ok(BucketAggregateRow {
            key: row.get(0)?,
            sessions: row.get::<_, i64>(1)? as u64,
            cost_usd: row.get(2)?,
            total_tokens: row.get::<_, i64>(3)? as u64,
            latest_ts_ms: row.get(4)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Per-day, per-source costs for provider-style trend charts.
pub fn daily_cost_by_source(
    conn: &Connection,
    since_ms: Option<i64>,
) -> Result<Vec<(String, Source, f64, u64)>> {
    let sql = r#"SELECT
                   e.day,
                   e.source,
                   SUM(e.cost_usd) AS cost,
                   SUM(e.input + e.output + e.cache_read + e.cache_write) AS tokens
                 FROM events e
                 WHERE (?1 IS NULL OR e.ts >= ?1)
                 GROUP BY e.day, e.source"#;
    let mut stmt = conn.prepare_cached(sql)?;
    let since = since_ms.map_or(Value::Null, Value::Integer);
    let rows = stmt.query_map([since], |row| {
        let src_str: String = row.get(1)?;
        let source = parse_source_column(src_str, 1)?;
        Ok((
            row.get::<_, String>(0)?,
            source,
            row.get::<_, f64>(2)?,
            row.get::<_, i64>(3)? as u64,
        ))
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

pub fn sparkline_costs(conn: &Connection, days: u32) -> Result<Vec<f64>> {
    let sql = "SELECT day, SUM(cost_usd) FROM events GROUP BY day ORDER BY day DESC LIMIT ?1";
    let mut stmt = conn.prepare_cached(sql)?;
    let rows = stmt.query_map([days as i64], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
    })?;
    let mut pairs: Vec<(String, f64)> = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    pairs.reverse();
    Ok(pairs.into_iter().map(|(_, c)| c).collect())
}

pub fn resolve_repo_filter(conn: &Connection, name: &str) -> Result<RepoFilterSpec> {
    if name == crate::repo::RepoIdentity::NO_REPO_DISPLAY {
        return Ok(RepoFilterSpec::NoRepo);
    }
    // Path-like input: always treat as a key prefix. Keeps behaviour
    // predictable when a user passes a full canonical path.
    if name.starts_with('/') {
        return Ok(RepoFilterSpec::KeyPrefix(name.to_owned()));
    }

    let mut stmt = conn.prepare_cached("SELECT key FROM repos WHERE display_name = ?1")?;
    let keys: Vec<String> = stmt
        .query_map([name], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    if keys.len() > 1 {
        anyhow::bail!(
            "repo name '{}' is ambiguous — matches {} repos: {}. \
             Pass a path prefix (e.g. /Users/you/dev/…) to disambiguate.",
            name,
            keys.len(),
            keys.join(", ")
        );
    }
    Ok(RepoFilterSpec::DisplayName(name.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::writes::{insert_events, upsert_repo, EventRow, RepoRow};
    use crate::test_support::{event_row as evt, fresh_conn, seed_standard_events as seed};

    #[test]
    fn repo_report_orders_by_cost_desc() {
        let mut conn = fresh_conn();
        seed(&mut conn);
        let rows = repo_report(&conn, QueryFilter::default()).unwrap();
        assert_eq!(rows[0].display_name, "beta"); // 5.00 cost
        assert_eq!(rows[1].display_name, "alpha"); // 1.50
                                                   // no-repo bucket present
        assert!(rows.iter().any(|r| r.is_no_repo()));
        assert_eq!(rows.iter().find(|r| r.is_no_repo()).unwrap().sessions, 1);
    }

    #[test]
    fn repo_report_sessions_counts_distinct() {
        let mut conn = fresh_conn();
        seed(&mut conn);
        let rows = repo_report(&conn, QueryFilter::default()).unwrap();
        let alpha = rows.iter().find(|r| r.display_name == "alpha").unwrap();
        assert_eq!(alpha.sessions, 1); // two events, same session
    }

    #[test]
    fn repo_report_counts_source_session_pairs() {
        let mut conn = fresh_conn();
        let tx = conn.transaction().unwrap();
        upsert_repo(
            &tx,
            &RepoRow {
                key: "/u/dev/alpha".into(),
                display_name: "alpha".into(),
                origin_url: None,
                first_seen: 1,
            },
        )
        .unwrap();
        insert_events(
            &tx,
            &[
                evt(1, "same", Some("/u/dev/alpha"), 100, 1.00, Source::Claude),
                evt(2, "same", Some("/u/dev/alpha"), 100, 1.00, Source::Codex),
            ],
        )
        .unwrap();
        tx.commit().unwrap();

        let rows = repo_report(&conn, QueryFilter::default()).unwrap();
        let alpha = rows.iter().find(|r| r.display_name == "alpha").unwrap();
        assert_eq!(alpha.sessions, 2);
    }

    #[test]
    fn repo_filter_display_name() {
        let mut conn = fresh_conn();
        seed(&mut conn);
        let filter = QueryFilter {
            repo: Some(RepoFilterSpec::DisplayName("alpha".into())),
            ..Default::default()
        };
        let rows = daily_report(&conn, filter).unwrap();
        let total: u64 = rows.iter().map(|r| r.input_tokens).sum();
        assert_eq!(total, 150);
    }

    #[test]
    fn repo_filter_key_prefix() {
        let mut conn = fresh_conn();
        seed(&mut conn);
        let filter = QueryFilter {
            repo: Some(RepoFilterSpec::KeyPrefix("/u/dev/be".into())),
            ..Default::default()
        };
        let rows = session_report(&conn, filter).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].key, "sB");
    }

    #[test]
    fn repo_filter_no_repo() {
        let mut conn = fresh_conn();
        seed(&mut conn);
        let filter = QueryFilter {
            repo: Some(RepoFilterSpec::NoRepo),
            ..Default::default()
        };
        let rows = session_report(&conn, filter).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].key, "sC");
    }

    #[test]
    fn session_report_uses_first_non_null_project_path() {
        let mut conn = fresh_conn();
        let tx = conn.transaction().unwrap();
        insert_events(
            &tx,
            &[
                EventRow {
                    file_path: "/a.jsonl".into(),
                    source: Source::Claude,
                    ts: 1,
                    day: "2026-04-22".into(),
                    month: "2026-04".into(),
                    session_id: "same".into(),
                    project_path: None,
                    repo: None,
                    model: "claude-sonnet-4-6".into(),
                    input: 1,
                    output: 0,
                    cache_read: 0,
                    cache_write: 0,
                    cost_usd: 0.1,
                },
                EventRow {
                    file_path: "/a.jsonl".into(),
                    source: Source::Claude,
                    ts: 2,
                    day: "2026-04-22".into(),
                    month: "2026-04".into(),
                    session_id: "same".into(),
                    project_path: Some("/tmp/zeta".into()),
                    repo: None,
                    model: "claude-sonnet-4-6".into(),
                    input: 1,
                    output: 0,
                    cache_read: 0,
                    cache_write: 0,
                    cost_usd: 0.1,
                },
                EventRow {
                    file_path: "/a.jsonl".into(),
                    source: Source::Claude,
                    ts: 3,
                    day: "2026-04-22".into(),
                    month: "2026-04".into(),
                    session_id: "same".into(),
                    project_path: Some("/tmp/alpha".into()),
                    repo: None,
                    model: "claude-sonnet-4-6".into(),
                    input: 1,
                    output: 0,
                    cache_read: 0,
                    cache_write: 0,
                    cost_usd: 0.1,
                },
            ],
        )
        .unwrap();
        tx.commit().unwrap();

        let rows = session_report(&conn, QueryFilter::default()).unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].project_path.as_deref(), Some("zeta"));
        assert_eq!(rows[0].latest_timestamp.unwrap().timestamp_millis(), 3);
    }

    #[test]
    fn resolve_repo_filter_no_repo_sentinel() {
        let mut conn = fresh_conn();
        seed(&mut conn);
        assert!(matches!(
            resolve_repo_filter(&conn, "(no-repo)").unwrap(),
            RepoFilterSpec::NoRepo
        ));
    }

    #[test]
    fn resolve_repo_filter_path_prefix() {
        let mut conn = fresh_conn();
        seed(&mut conn);
        assert!(matches!(
            resolve_repo_filter(&conn, "/u/dev/alpha").unwrap(),
            RepoFilterSpec::KeyPrefix(_)
        ));
    }

    #[test]
    fn resolve_repo_filter_ambiguous_errors() {
        let mut conn = fresh_conn();
        seed(&mut conn);
        // Two repos with the same display name.
        let tx = conn.transaction().unwrap();
        upsert_repo(
            &tx,
            &RepoRow {
                key: "/elsewhere/alpha".into(),
                display_name: "alpha".into(),
                origin_url: None,
                first_seen: 1,
            },
        )
        .unwrap();
        tx.commit().unwrap();
        let err = resolve_repo_filter(&conn, "alpha").unwrap_err();
        assert!(err.to_string().contains("ambiguous"));
    }
}
