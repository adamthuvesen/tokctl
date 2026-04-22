use crate::types::{AggregateRow, Source, SourceLabel};
use anyhow::Result;
use chrono::{DateTime, TimeZone, Utc};
use rusqlite::{params_from_iter, types::Value, Connection};

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

/// Best-effort basename for a raw `project_path`. Handles both real absolute
/// paths and Claude's dash-encoded form.
fn basename_of(s: &str) -> &str {
    if s.starts_with('/') {
        s.rsplit('/').next().filter(|x| !x.is_empty()).unwrap_or(s)
    } else if s.starts_with('-') {
        s.rsplit('-').next().filter(|x| !x.is_empty()).unwrap_or(s)
    } else {
        s
    }
}

fn source_label(f: &QueryFilter) -> SourceLabel {
    match f.source {
        Some(s) => SourceLabel::Source(s),
        None => SourceLabel::All,
    }
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
             e.session_id,
             e.source,
             MAX(r.display_name)  AS repo_display,
             MAX(e.project_path)  AS project_path,
             MAX(e.ts) AS latest_ts,
             SUM(e.input)       AS input_tokens,
             SUM(e.output)      AS output_tokens,
             SUM(e.cache_read)  AS cache_read_tokens,
             SUM(e.cache_write) AS cache_write_tokens,
             SUM(e.input + e.output + e.cache_read + e.cache_write) AS total_tokens,
             SUM(e.cost_usd)    AS cost_usd
           FROM events e
           LEFT JOIN repos r ON r.key = e.repo
           WHERE 1=1 {src} {ts} {repo}
           GROUP BY e.source, e.session_id
           ORDER BY latest_ts DESC"#,
        src = source_clause(&filter),
        ts = time_clause(),
        repo = repo_clause(&filter),
    );
    let params = build_params(&filter);
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(params.iter()), |row| {
        let session_id: String = row.get(0)?;
        let src_str: String = row.get(1)?;
        let source = match src_str.as_str() {
            "claude" => Source::Claude,
            _ => Source::Codex,
        };
        let repo_display: Option<String> = row.get(2)?;
        let project_path: Option<String> = row.get(3)?;
        let latest_ms: i64 = row.get(4)?;
        let latest_ts: DateTime<Utc> = Utc
            .timestamp_millis_opt(latest_ms)
            .single()
            .unwrap_or_else(Utc::now);
        // Prefer the resolved repo display name; fall back to a basename of
        // the raw project_path so rows stay scannable even without a repo.
        let shown =
            repo_display.or_else(|| project_path.as_deref().map(basename_of).map(String::from));
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
    Ok(rows.filter_map(std::result::Result::ok).collect())
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
             COUNT(DISTINCT e.session_id) AS sessions,
             SUM(e.input)       AS input_tokens,
             SUM(e.output)      AS output_tokens,
             SUM(e.cache_read)  AS cache_read_tokens,
             SUM(e.cache_write) AS cache_write_tokens,
             SUM(e.input + e.output + e.cache_read + e.cache_write) AS total_tokens,
             SUM(e.cost_usd)    AS cost_usd
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
    let mut stmt = conn.prepare(&sql)?;
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
        })
    })?;
    Ok(rows.filter_map(std::result::Result::ok).collect())
}

/// Resolve a user-supplied repo name (`--repo <name>` or `tokctl repo <name>`)
/// into a concrete [`RepoFilterSpec`]. Ambiguity — multiple repos with the
/// same display name, none of which match a path prefix — returns an error
/// listing the candidates.
///
/// The literal value `(no-repo)` maps to [`RepoFilterSpec::NoRepo`].
pub fn resolve_repo_filter(conn: &Connection, name: &str) -> Result<RepoFilterSpec> {
    if name == crate::repo::RepoIdentity::NO_REPO_DISPLAY {
        return Ok(RepoFilterSpec::NoRepo);
    }
    // Path-like input: always treat as a key prefix. Keeps behaviour
    // predictable when a user passes a full canonical path.
    if name.starts_with('/') {
        return Ok(RepoFilterSpec::KeyPrefix(name.to_owned()));
    }

    let mut stmt = conn.prepare("SELECT key FROM repos WHERE display_name = ?1")?;
    let keys: Vec<String> = stmt
        .query_map([name], |row| row.get::<_, String>(0))?
        .filter_map(std::result::Result::ok)
        .collect();

    match keys.len() {
        0 => Ok(RepoFilterSpec::DisplayName(name.to_owned())),
        1 => Ok(RepoFilterSpec::DisplayName(name.to_owned())),
        _ => anyhow::bail!(
            "repo name '{}' is ambiguous — matches {} repos: {}. \
             Pass a path prefix (e.g. /Users/you/dev/…) to disambiguate.",
            name,
            keys.len(),
            keys.join(", ")
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::schema::DDL;
    use crate::store::writes::{insert_events, upsert_repo, EventRow, RepoRow};

    fn fresh_conn() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(DDL).unwrap();
        c
    }

    fn evt(
        ts: i64,
        session: &str,
        repo: Option<&str>,
        tokens: u64,
        cost: f64,
        source: Source,
    ) -> EventRow {
        EventRow {
            file_path: format!("/{session}.jsonl"),
            source,
            ts,
            day: "2026-04-22".into(),
            month: "2026-04".into(),
            session_id: session.into(),
            project_path: repo.map(str::to_owned),
            repo: repo.map(str::to_owned),
            model: "claude-sonnet-4-6".into(),
            input: tokens,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            cost_usd: cost,
        }
    }

    fn seed(conn: &mut Connection) {
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
        upsert_repo(
            &tx,
            &RepoRow {
                key: "/u/dev/beta".into(),
                display_name: "beta".into(),
                origin_url: None,
                first_seen: 1,
            },
        )
        .unwrap();
        insert_events(
            &tx,
            &[
                evt(1, "sA", Some("/u/dev/alpha"), 100, 1.00, Source::Claude),
                evt(2, "sA", Some("/u/dev/alpha"), 50, 0.50, Source::Claude),
                evt(3, "sB", Some("/u/dev/beta"), 10, 5.00, Source::Codex),
                evt(4, "sC", None, 20, 0.10, Source::Claude),
            ],
        )
        .unwrap();
        tx.commit().unwrap();
    }

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
