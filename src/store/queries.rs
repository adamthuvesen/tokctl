use super::Store;
use crate::repo::project_basename;
use crate::types::{AggregateRow, Source, SourceLabel};
use anyhow::Result;
use chrono::{DateTime, TimeZone, Utc};
use rusqlite::{
    params_from_iter,
    types::{Type, Value},
    Connection,
};
use std::collections::HashSet;
use std::path::PathBuf;
use std::str::FromStr;

/// Sentinel string used in SQL to represent the "(no-repo)" bucket when
/// repo IS NULL. Chosen to be unambiguous against any real repo key.
pub const NO_REPO_SENTINEL: &str = "<NO_REPO>";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepoFilterSpec {
    /// Only events with `repo IS NULL`.
    NoRepo,
    /// Only events whose repo has the given display name.
    DisplayName(String),
    /// Only events whose repo key starts with the given path prefix.
    KeyPrefix(String),
    KeyExact(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionKey {
    pub source: Source,
    pub id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QueryScope {
    pub source: Option<Source>,
    pub since_ms: Option<i64>,
    /// Exclusive upper bound.
    pub until_ms: Option<i64>,
    pub repo: Option<RepoFilterSpec>,
    /// Raw model ids; one display row can group several (`claude-x` and `claude-x-20250101`).
    pub models: Option<Vec<String>>,
    pub session: Option<SessionKey>,
    pub roots: Option<Vec<PathBuf>>,
}

impl QueryScope {
    pub fn intersect_time(&mut self, since: Option<i64>, until: Option<i64>) {
        self.since_ms = match (self.since_ms, since) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
        self.until_ms = match (self.until_ms, until) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
    }
}

pub type QueryFilter = QueryScope;

/// Filters plus the duplicate-message rule, for queries over `events e`.
pub(crate) fn scope_clause(filter: &QueryScope) -> (String, Vec<Value>) {
    let (mut clause, mut params) = filter_clause(filter);
    clause.push_str(&counted_clause(filter, &mut params));
    (clause, params)
}

/// The same Claude message id in two session files (a resumed or forked session) counts once.
/// The copy in the lowest file path within the selected roots wins, so the result depends on
/// neither ingest order nor roots the report does not read.
fn counted_clause(filter: &QueryScope, params: &mut Vec<Value>) -> String {
    let roots = match &filter.roots {
        Some(roots) => format!(" AND {}", root_clause("other", roots, params)),
        None => String::new(),
    };
    format!(
        " AND (e.message_id IS NULL OR NOT EXISTS (SELECT 1 FROM events other \
         WHERE other.message_id = e.message_id AND other.source = 'claude' \
         AND other.file_path < e.file_path{roots}))"
    )
}

fn root_clause(alias: &str, roots: &[PathBuf], params: &mut Vec<Value>) -> String {
    let mut root_clauses = Vec::new();
    for root in roots {
        let root = root.to_string_lossy();
        let prefix = format!("{}/", root.trim_end_matches('/'));
        root_clauses.push(format!(
            "({alias}.file_path = ? OR substr({alias}.file_path,1,length(?)) = ?)"
        ));
        params.extend([
            Value::Text(root.into_owned()),
            Value::Text(prefix.clone()),
            Value::Text(prefix),
        ]);
    }
    if root_clauses.is_empty() {
        "0".into()
    } else {
        format!("({})", root_clauses.join(" OR "))
    }
}

fn filter_clause(filter: &QueryScope) -> (String, Vec<Value>) {
    let mut clauses = Vec::new();
    let mut params = Vec::new();
    if let Some(source) = filter.source {
        clauses.push("e.source = ?".to_owned());
        params.push(Value::Text(source.as_str().into()));
    }
    if let Some(since) = filter.since_ms {
        clauses.push("e.ts >= ?".into());
        params.push(Value::Integer(since));
    }
    if let Some(until) = filter.until_ms {
        clauses.push("e.ts < ?".into());
        params.push(Value::Integer(until));
    }
    match &filter.repo {
        None => {}
        Some(RepoFilterSpec::NoRepo) => clauses.push("e.repo IS NULL".into()),
        Some(RepoFilterSpec::DisplayName(name)) => {
            clauses.push("e.repo IN (SELECT key FROM repos WHERE display_name = ?)".into());
            params.push(Value::Text(name.clone()));
        }
        Some(RepoFilterSpec::KeyExact(key)) => {
            clauses.push("e.repo = ?".into());
            params.push(Value::Text(key.clone()));
        }
        Some(RepoFilterSpec::KeyPrefix(prefix)) => {
            clauses.push("substr(e.repo,1,length(?)) = ?".into());
            params.extend([Value::Text(prefix.clone()), Value::Text(prefix.clone())]);
        }
    }
    if let Some(models) = &filter.models {
        let marks = vec!["?"; models.len()].join(",");
        clauses.push(format!("e.model IN ({marks})"));
        params.extend(models.iter().cloned().map(Value::Text));
    }
    if let Some(session) = &filter.session {
        clauses.push("e.source = ? AND e.session_id = ?".into());
        params.extend([
            Value::Text(session.source.as_str().into()),
            Value::Text(session.id.clone()),
        ]);
    }
    if let Some(roots) = &filter.roots {
        clauses.push(root_clause("e", roots, &mut params));
    }
    (
        if clauses.is_empty() {
            String::new()
        } else {
            format!("AND {}", clauses.join(" AND "))
        },
        params,
    )
}

fn build_params(filter: &QueryScope) -> Vec<Value> {
    scope_clause(filter).1
}

fn source_label(f: &QueryFilter) -> SourceLabel {
    match f.source {
        Some(s) => SourceLabel::Source(s),
        None => SourceLabel::All,
    }
}

fn read_cost(row: &rusqlite::Row<'_>, column: usize) -> rusqlite::Result<f64> {
    let cost: f64 = row.get(column)?;
    if !cost.is_finite() || cost < 0.0 {
        return Err(rusqlite::Error::FromSqlConversionFailure(
            column,
            Type::Real,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "cost total must be finite and non-negative",
            )),
        ));
    }
    Ok(cost)
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

pub fn daily_report(store: &Store, filter: QueryFilter) -> Result<Vec<AggregateRow>> {
    if store.connection().is_autocommit() {
        return store.read_snapshot(|store| daily_report(store, filter));
    }
    let conn = store.connection();
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
           WHERE 1=1 {scope}
           GROUP BY key
           ORDER BY key ASC"#,
        scope = scope_clause(&filter).0,
    );
    run_bucket_query(conn, &sql, &filter, source_label(&filter))
}

pub fn monthly_report(store: &Store, filter: QueryFilter) -> Result<Vec<AggregateRow>> {
    if store.connection().is_autocommit() {
        return store.read_snapshot(|store| monthly_report(store, filter));
    }
    let conn = store.connection();
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
           WHERE 1=1 {scope}
           GROUP BY key
           ORDER BY key ASC"#,
        scope = scope_clause(&filter).0,
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
            cost_usd: read_cost(row, 6)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn session_report(store: &Store, filter: QueryFilter) -> Result<Vec<AggregateRow>> {
    if store.connection().is_autocommit() {
        return store.read_snapshot(|store| session_report(store, filter));
    }
    let conn = store.connection();
    let sql = format!(
        r#"WITH filtered AS (
             SELECT e.*
             FROM events e
             WHERE 1=1 {scope}
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
        scope = scope_clause(&filter).0,
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
            cost_usd: read_cost(row, 10)?,
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

pub fn repo_report(store: &Store, filter: QueryFilter) -> Result<Vec<RepoAggregateRow>> {
    if store.connection().is_autocommit() {
        return store.read_snapshot(|store| repo_report(store, filter));
    }
    let conn = store.connection();
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
           WHERE 1=1 {scope}
           GROUP BY COALESCE(e.repo, '{no_repo}')
           ORDER BY cost_usd DESC"#,
        no_repo = NO_REPO_SENTINEL,
        no_repo_name = crate::repo::RepoIdentity::NO_REPO_DISPLAY,
        scope = scope_clause(&filter).0,
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
            cost_usd: read_cost(row, 9)?,
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
pub use crate::calendar::PeriodGranularity;

impl PeriodGranularity {
    fn bucket_expr(self) -> &'static str {
        match self {
            PeriodGranularity::Daily => "e.day",
            PeriodGranularity::Weekly => "strftime('%G-W%V', e.day)",
            PeriodGranularity::Monthly => "e.month",
            PeriodGranularity::Yearly => "substr(e.day, 1, 4)",
        }
    }
}

/// Row returned by [`period_buckets`].
#[derive(Debug, Clone)]
pub struct BucketAggregateRow {
    pub key: String,
    pub sessions: u64,
    pub cost_usd: f64,
    pub total_tokens: u64,
    pub latest_ts_ms: i64,
}

pub fn period_buckets(
    store: &Store,
    filter: QueryFilter,
    granularity: PeriodGranularity,
) -> Result<Vec<BucketAggregateRow>> {
    if store.connection().is_autocommit() {
        return store.read_snapshot(|store| period_buckets(store, filter, granularity));
    }
    let conn = store.connection();
    let sql = format!(
        r#"SELECT {bucket} AS bucket,
                  COUNT(DISTINCT e.source || char(31) || e.session_id) AS sessions,
                  SUM(e.cost_usd) AS cost,
                  SUM(e.input + e.output + e.cache_read + e.cache_write) AS total_tokens,
                  MAX(e.ts) AS latest_ts
             FROM events e
             WHERE 1=1 {scope}
             GROUP BY bucket
             ORDER BY bucket DESC"#,
        bucket = granularity.bucket_expr(),
        scope = scope_clause(&filter).0,
    );
    run_bucket_aggregate_query(conn, &sql, &filter)
}

/// Token and cost sums for one (source, model) pair.
#[derive(Debug, Clone)]
pub struct ModelUsageRow {
    pub source: Source,
    pub model: String,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub cost_usd: f64,
    pub latest_ts_ms: i64,
    pub events: u64,
}

pub fn model_usage(store: &Store, filter: QueryFilter) -> Result<Vec<ModelUsageRow>> {
    if store.connection().is_autocommit() {
        return store.read_snapshot(|store| model_usage(store, filter));
    }
    let sql = format!(
        r#"SELECT e.source, e.model,
                  SUM(e.input), SUM(e.output), SUM(e.cache_read), SUM(e.cache_write),
                  SUM(e.cost_usd), MAX(e.ts), COUNT(*)
             FROM events e
             WHERE 1=1 {scope}
             GROUP BY e.source, e.model"#,
        scope = scope_clause(&filter).0,
    );
    let mut stmt = store.connection().prepare_cached(&sql)?;
    let rows = stmt.query_map(params_from_iter(build_params(&filter).iter()), |row| {
        Ok(ModelUsageRow {
            source: parse_source_column(row.get(0)?, 0)?,
            model: row.get(1)?,
            input: row.get::<_, i64>(2)? as u64,
            output: row.get::<_, i64>(3)? as u64,
            cache_read: row.get::<_, i64>(4)? as u64,
            cache_write: row.get::<_, i64>(5)? as u64,
            cost_usd: read_cost(row, 6)?,
            latest_ts_ms: row.get(7)?,
            events: row.get::<_, i64>(8)? as u64,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
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
            cost_usd: read_cost(row, 2)?,
            total_tokens: row.get::<_, i64>(3)? as u64,
            latest_ts_ms: row.get(4)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Per-day, per-source costs for provider-style trend charts.
pub fn daily_cost_by_source(
    store: &Store,
    filter: QueryScope,
) -> Result<Vec<(String, Source, f64, u64)>> {
    if store.connection().is_autocommit() {
        return store.read_snapshot(|store| daily_cost_by_source(store, filter));
    }
    let conn = store.connection();
    let sql = format!(
        r#"SELECT
                   e.day,
                   e.source,
                   SUM(e.cost_usd) AS cost,
                   SUM(e.input + e.output + e.cache_read + e.cache_write) AS tokens
                 FROM events e
                 WHERE 1=1 {}
                 GROUP BY e.day, e.source"#,
        scope_clause(&filter).0
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    let parameters = build_params(&filter);
    let rows = stmt.query_map(params_from_iter(parameters), |row| {
        let src_str: String = row.get(1)?;
        let source = parse_source_column(src_str, 1)?;
        Ok((
            row.get::<_, String>(0)?,
            source,
            read_cost(row, 2)?,
            row.get::<_, i64>(3)? as u64,
        ))
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

pub fn resolve_repo_filter_scoped(
    store: &Store,
    name: &str,
    scope: &QueryScope,
) -> Result<RepoFilterSpec> {
    if store.connection().is_autocommit() {
        return store.read_snapshot(|store| resolve_repo_filter_scoped(store, name, scope));
    }
    if name == crate::repo::RepoIdentity::NO_REPO_DISPLAY {
        return Ok(RepoFilterSpec::NoRepo);
    }
    if name.starts_with('/') {
        return Ok(RepoFilterSpec::KeyPrefix(name.to_owned()));
    }
    let (clause, mut parameters) = scope_clause(scope);
    parameters.push(Value::Text(name.into()));
    let sql = format!("SELECT DISTINCT e.repo FROM events e JOIN repos r ON r.key=e.repo WHERE 1=1 {clause} AND r.display_name=? ORDER BY e.repo");
    let mut statement = store.connection().prepare_cached(&sql)?;
    let keys = statement
        .query_map(params_from_iter(parameters), |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    anyhow::ensure!(
        keys.len() <= 1,
        "repo name '{}' is ambiguous, matches {} repos: {}. Pass a path prefix to disambiguate.",
        name,
        keys.len(),
        keys.join(", ")
    );
    Ok(RepoFilterSpec::DisplayName(name.to_owned()))
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReportDiagnostics {
    pub unknown_models: HashSet<String>,
    pub skipped_records: usize,
    pub current_file_errors: usize,
    pub legacy_cost_rows: u64,
    pub stale_pricing_rows: u64,
    pub stale_parser_files: u64,
}

pub fn report_diagnostics(store: &Store, scope: &QueryScope) -> Result<ReportDiagnostics> {
    report_diagnostics_for_scopes(store, std::slice::from_ref(scope))
}

pub fn report_diagnostics_for_scopes(
    store: &Store,
    scopes: &[QueryScope],
) -> Result<ReportDiagnostics> {
    if store.connection().is_autocommit() {
        return store.read_snapshot(|store| report_diagnostics_for_scopes(store, scopes));
    }
    let mut event_clauses = Vec::new();
    let mut event_parameters = Vec::new();
    let mut file_clauses = Vec::new();
    let mut file_parameters = Vec::new();
    for scope in scopes {
        let (clause, values) = scope_clause(scope);
        event_clauses.push(format!("(1=1 {clause})"));
        event_parameters.extend(values);
        let (clause, values) = diagnostic_file_clause(scope);
        file_clauses.push(format!("(1=1 {clause})"));
        file_parameters.extend(values);
    }
    let events = if event_clauses.is_empty() {
        "0".into()
    } else {
        event_clauses.join(" OR ")
    };
    let files = if file_clauses.is_empty() {
        "0".into()
    } else {
        file_clauses.join(" OR ")
    };
    let connection = store.connection();
    let mut statement = connection.prepare_cached(&format!(
        "SELECT DISTINCT e.model FROM events e WHERE e.source <> 'cursor' AND ({events})"
    ))?;
    let unknown_models = statement
        .query_map(params_from_iter(&event_parameters), |row| {
            row.get::<_, String>(0)
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .filter(|model| !crate::pricing::has_price(model))
        .collect();
    let mut cost_parameters = vec![Value::Text(crate::pricing::signature())];
    cost_parameters.extend(event_parameters);
    let (legacy_cost_rows, stale_pricing_rows) = connection.query_row(
        &format!("SELECT COALESCE(SUM(e.cost_provenance='legacy'),0), COALESCE(SUM(e.cost_provenance NOT IN ('legacy','source',?)),0) FROM events e WHERE {events}"),
        params_from_iter(cost_parameters),
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let skipped_records = connection.query_row(
        &format!("SELECT COALESCE(SUM(d.skipped_records),0) FROM ingest_diagnostics d JOIN files f ON f.path=d.file_path WHERE {files}"),
        params_from_iter(&file_parameters),
        |row| row.get(0),
    )?;
    let mut statement = connection.prepare_cached(&format!(
        "SELECT f.source,p.signature FROM files f LEFT JOIN parser_provenance p ON p.path=f.path WHERE {files}"
    ))?;
    let signatures = statement.query_map(params_from_iter(file_parameters), |row| {
        Ok((
            parse_source_column(row.get(0)?, 0)?,
            row.get::<_, Option<String>>(1)?,
        ))
    })?;
    let mut stale_parser_files = 0;
    for signature in signatures {
        let (source, signature) = signature?;
        if signature.as_deref() != Some(super::db::parser_signature(source).as_str()) {
            stale_parser_files += 1;
        }
    }
    Ok(ReportDiagnostics {
        unknown_models,
        skipped_records,
        legacy_cost_rows,
        stale_pricing_rows,
        stale_parser_files,
        ..Default::default()
    })
}

fn diagnostic_file_clause(scope: &QueryScope) -> (String, Vec<Value>) {
    let files = QueryScope {
        source: scope.source,
        roots: scope.roots.clone(),
        ..Default::default()
    };
    let (clause, mut parameters) = filter_clause(&files);
    let mut file_clause = clause
        .replace("e.source", "f.source")
        .replace("e.file_path", "f.path");
    if scope.since_ms.is_some()
        || scope.until_ms.is_some()
        || scope.repo.is_some()
        || scope.models.is_some()
        || scope.session.is_some()
    {
        let (clause, values) = scope_clause(scope);
        file_clause.push_str(&format!(
            " AND EXISTS(SELECT 1 FROM events e WHERE e.file_path=f.path {clause})"
        ));
        parameters.extend(values);
    }
    (file_clause, parameters)
}

pub fn skipped_records(store: &Store, scope: &QueryScope) -> Result<usize> {
    Ok(report_diagnostics(store, scope)?.skipped_records)
}

#[derive(Debug, Clone)]
pub struct EventReportRow {
    pub ts: DateTime<Utc>,
    pub model: String,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub cost: f64,
}

#[derive(Debug)]
pub struct StoreStatus {
    pub event_count: u64,
    pub max_mtime_ns: Option<i64>,
    pub generation: u64,
}

pub fn store_status(store: &Store) -> Result<StoreStatus> {
    if store.connection().is_autocommit() {
        return store.read_snapshot(store_status);
    }
    let conn = store.connection();
    Ok(StoreStatus {
        event_count: conn.query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))?,
        max_mtime_ns: conn.query_row("SELECT MAX(mtime_ns) FROM files", [], |row| row.get(0))?,
        generation: conn.query_row(
            "SELECT COALESCE((SELECT CAST(value AS INTEGER) FROM meta WHERE key='generation'),0)",
            [],
            |row| row.get(0),
        )?,
    })
}

pub fn event_rows(store: &Store, scope: QueryScope) -> Result<Vec<EventReportRow>> {
    if store.connection().is_autocommit() {
        return store.read_snapshot(|store| event_rows(store, scope));
    }
    let (clause, parameters) = scope_clause(&scope);
    let sql = format!("SELECT e.ts,e.model,e.input,e.output,e.cache_read,e.cache_write,e.cost_usd FROM events e WHERE 1=1 {clause} ORDER BY e.ts,e.id");
    let mut statement = store.connection().prepare_cached(&sql)?;
    let rows = statement.query_map(params_from_iter(parameters), |row| {
        let millis = row.get(0)?;
        let ts = Utc
            .timestamp_millis_opt(millis)
            .single()
            .ok_or_else(|| rusqlite::Error::IntegralValueOutOfRange(0, millis))?;
        Ok(EventReportRow {
            ts,
            model: row.get(1)?,
            input: row.get(2)?,
            output: row.get(3)?,
            cache_read: row.get(4)?,
            cache_write: row.get(5)?,
            cost: read_cost(row, 6)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn scoped_sparkline(store: &Store, scope: QueryScope, days: u32) -> Result<Vec<f64>> {
    if store.connection().is_autocommit() {
        return store.read_snapshot(|store| scoped_sparkline(store, scope, days));
    }
    let mut rows = daily_report(store, scope)?;
    if rows.len() > days as usize {
        rows.drain(..rows.len() - days as usize);
    }
    Ok(rows.into_iter().map(|row| row.cost_usd).collect())
}

#[derive(Debug, Clone)]
pub(crate) struct ComparisonRow {
    pub source: Source,
    pub session_id: String,
    pub repo_key: Option<String>,
    pub repo_label: String,
    pub model: String,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub cost_usd: f64,
}

pub(crate) fn comparison_rows(
    store: &Store,
    start: &str,
    end: &str,
    scope: &QueryScope,
) -> Result<Vec<ComparisonRow>> {
    let (clause, mut parameters) = scope_clause(scope);
    parameters.extend([Value::Text(start.into()), Value::Text(end.into())]);
    let aggregate_sql = format!("SELECT COALESCE(SUM(e.input+e.output+e.cache_read+e.cache_write),0),COALESCE(SUM(e.cost_usd),0) FROM events e WHERE 1=1 {clause} AND e.day >= ? AND e.day <= ?");
    store
        .connection()
        .query_row(&aggregate_sql, params_from_iter(parameters.iter()), |row| {
            let _: u64 = row.get(0)?;
            read_cost(row, 1)
        })?;
    let sql = format!("SELECT e.source,e.session_id,e.repo,COALESCE(r.display_name,'(no-repo)'),e.model,e.input,e.output,e.cache_read,e.cache_write,e.cost_usd FROM events e LEFT JOIN repos r ON r.key=e.repo WHERE 1=1 {clause} AND e.day >= ? AND e.day <= ?");
    let mut statement = store.connection().prepare_cached(&sql)?;
    let rows = statement.query_map(params_from_iter(parameters), |row| {
        Ok(ComparisonRow {
            source: parse_source_column(row.get(0)?, 0)?,
            session_id: row.get(1)?,
            repo_key: row.get(2)?,
            repo_label: row.get(3)?,
            model: row.get(4)?,
            input: row.get(5)?,
            output: row.get(6)?,
            cache_read: row.get(7)?,
            cache_write: row.get(8)?,
            cost_usd: read_cost(row, 9)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
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
                    explicit_cost_usd: None,
                    cost_provenance: "legacy".into(),
                    message_id: None,
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
                    explicit_cost_usd: None,
                    cost_provenance: "legacy".into(),
                    message_id: None,
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
                    explicit_cost_usd: None,
                    cost_provenance: "legacy".into(),
                    message_id: None,
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
            resolve_repo_filter_scoped(&conn, "(no-repo)", &QueryScope::default()).unwrap(),
            RepoFilterSpec::NoRepo
        ));
    }

    #[test]
    fn resolve_repo_filter_path_prefix() {
        let mut conn = fresh_conn();
        seed(&mut conn);
        assert!(matches!(
            resolve_repo_filter_scoped(&conn, "/u/dev/alpha", &QueryScope::default()).unwrap(),
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
        insert_events(
            &tx,
            &[evt(
                5,
                "second",
                Some("/elsewhere/alpha"),
                1,
                1.0,
                Source::Claude,
            )],
        )
        .unwrap();
        tx.commit().unwrap();
        let err = resolve_repo_filter_scoped(&conn, "alpha", &QueryScope::default()).unwrap_err();
        assert!(err.to_string().contains("ambiguous"));
    }
    #[test]
    fn diagnostics_follow_event_scope_and_keep_malformed_only_files_in_root_scope() {
        let mut store = fresh_conn();
        let mut selected = evt(1, "selected", Some("/repo"), 1, 1.0, Source::Claude);
        selected.file_path = "/selected/usage.jsonl".into();
        selected.model = "unknown-selected-model".into();
        let mut other = evt(2, "other", None, 1, 1.0, Source::Codex);
        other.file_path = "/other/usage.jsonl".into();
        other.model = "unknown-other-model".into();
        other.cost_provenance = "old-price".into();
        let tx = store.transaction().unwrap();
        insert_events(&tx, &[selected.clone(), other]).unwrap();
        tx.execute_batch("INSERT INTO files(path,source,size,mtime_ns) VALUES('/selected/usage.jsonl','claude',1,1),('/selected/malformed.jsonl','claude',1,1),('/other/usage.jsonl','codex',1,1); INSERT INTO ingest_diagnostics(file_path,skipped_records) VALUES('/selected/usage.jsonl',2),('/selected/malformed.jsonl',3),('/other/usage.jsonl',7);").unwrap();
        tx.commit().unwrap();
        store.execute_batch("BEGIN").unwrap();
        let scope = QueryScope {
            source: Some(Source::Claude),
            roots: Some(vec![PathBuf::from("/selected")]),
            ..Default::default()
        };
        let diagnostics = report_diagnostics(&store, &scope).unwrap();
        assert_eq!(
            diagnostics.unknown_models,
            HashSet::from([selected.model.clone()])
        );
        assert_eq!(diagnostics.skipped_records, 5);
        assert_eq!(diagnostics.legacy_cost_rows, 1);
        assert_eq!(diagnostics.stale_pricing_rows, 0);
        assert_eq!(diagnostics.stale_parser_files, 2);
        let selected_scope = QueryScope {
            since_ms: Some(selected.ts),
            until_ms: Some(selected.ts + 1),
            repo: Some(RepoFilterSpec::KeyExact("/repo".into())),
            models: Some(vec![selected.model.clone()]),
            session: Some(SessionKey {
                source: Source::Claude,
                id: selected.session_id,
            }),
            ..scope.clone()
        };
        let diagnostics = report_diagnostics(&store, &selected_scope).unwrap();
        assert_eq!(diagnostics.skipped_records, 2);
        assert_eq!(diagnostics.legacy_cost_rows, 1);
        assert_eq!(diagnostics.stale_parser_files, 1);
        for excluded in [
            QueryScope {
                since_ms: Some(selected.ts + 1),
                ..selected_scope.clone()
            },
            QueryScope {
                repo: Some(RepoFilterSpec::NoRepo),
                ..selected_scope.clone()
            },
            QueryScope {
                models: Some(vec!["different-model".into()]),
                ..selected_scope.clone()
            },
            QueryScope {
                session: Some(SessionKey {
                    source: Source::Claude,
                    id: "different-session".into(),
                }),
                ..selected_scope
            },
        ] {
            assert_eq!(
                report_diagnostics(&store, &excluded).unwrap(),
                ReportDiagnostics::default()
            );
        }
        let both = report_diagnostics_for_scopes(&store, &[scope.clone(), scope]).unwrap();
        assert_eq!(both.skipped_records, 5);
        assert_eq!(both.legacy_cost_rows, 1);
        assert_eq!(both.stale_parser_files, 2);
        assert_eq!(
            report_diagnostics(
                &store,
                &QueryScope {
                    source: Some(Source::Codex),
                    ..Default::default()
                }
            )
            .unwrap()
            .stale_pricing_rows,
            1
        );
    }
}
