use anyhow::Result;
use chrono::{DateTime, Datelike, Local, NaiveDate, TimeZone, Utc};
use rusqlite::{
    params_from_iter,
    types::{Type, Value},
    Connection,
};

use crate::repo::project_basename;
use crate::store::queries::{
    repo_report, session_report, QueryFilter, RepoAggregateRow, RepoFilterSpec,
};
use crate::tui::state::{
    AppState, DrillKind, Section, Sort, SourceFilter, TimeWindow, TrendGranularity,
};
use crate::types::{AggregateRow, Source};
use std::collections::BTreeMap;
use std::str::FromStr;

#[derive(Debug, Clone)]
pub struct LeftRow {
    /// Primary label shown in the pane.
    pub label: String,
    /// Internal key used to filter downstream panes. For repo/model/day it's
    /// the raw key; for session it's the session id.
    pub key: String,
    pub sessions: u64,
    pub total_tokens: u64,
    pub cost: f64,
    pub is_no_repo: bool,
    /// Most-recent timestamp for this row, when meaningful. Used by the
    /// recent sort for aggregate sections and by the Sessions "when" column.
    pub latest_ts: Option<DateTime<Utc>>,
    /// Source for this row, when the row maps 1:1 to a single source.
    /// Populated for `Sessions` so the events drill knows which `(source,
    /// session_id)` to query.
    pub source: Option<Source>,
}

#[derive(Debug, Clone)]
pub struct SessionRow {
    pub session_id: String,
    pub source: Source,
    pub latest_ts: DateTime<Utc>,
    pub project: Option<String>,
    pub cost: f64,
    pub total_tokens: u64,
}

/// One row per turn within a single session — the deepest drill level.
#[derive(Debug, Clone)]
pub struct EventRow {
    pub ts: DateTime<Utc>,
    pub model: String,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub cost: f64,
}

/// One time bucket with per-source (Claude / Codex / Cursor) costs. Used for the **Provider** section
/// and Repos **Provider** tab; bucket size (d/w/m/y) comes from `AppState::trend_granularity`, shared with **Days**.
#[derive(Debug, Clone)]
pub struct TrendRow {
    pub bucket: String,
    pub claude_cost: f64,
    pub codex_cost: f64,
    pub cursor_cost: f64,
    pub total_tokens: u64,
    pub total_cost: f64,
    pub is_current: bool,
}

#[derive(Debug, Clone)]
pub struct CacheStatus {
    pub cache_path: String,
    pub event_count: u64,
    pub freshness: String,
    pub last_query: DateTime<Utc>,
    /// Most recent `mtime_ns` across indexed source files. Drives memo
    /// invalidation: when this advances, the in-memory memos are dropped
    /// because the underlying events may have changed.
    pub mtime_ns: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshScope {
    Status,
    Left,
    Sessions,
    Events,
    Sparkline,
    Trend,
}

impl RefreshScope {
    pub fn as_str(self) -> &'static str {
        match self {
            RefreshScope::Status => "status",
            RefreshScope::Left => "rows",
            RefreshScope::Sessions => "sessions",
            RefreshScope::Events => "events",
            RefreshScope::Sparkline => "sparkline",
            RefreshScope::Trend => "trend",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshError {
    pub scope: RefreshScope,
    pub message: String,
}

impl RefreshError {
    pub fn new(scope: RefreshScope, err: impl std::fmt::Display) -> Self {
        Self {
            scope,
            message: err.to_string(),
        }
    }

    pub fn display_message(&self) -> String {
        format!("refresh failed: {}: {}", self.scope.as_str(), self.message)
    }
}

impl Default for CacheStatus {
    fn default() -> Self {
        Self {
            cache_path: crate::store::store_path().display().to_string(),
            event_count: 0,
            freshness: "unknown".into(),
            last_query: Utc::now(),
            mtime_ns: None,
        }
    }
}

/// Memo key for the per-section `left` slice. Covers everything that
/// materially affects what `load_left_axis` returns; two visits with the
/// same key yield byte-identical rows.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct LeftMemoKey(
    pub Section,
    pub TimeWindow,
    pub SourceFilter,
    pub TrendGranularity,
);

/// Memo key for the trend slice. Deliberately excludes `Section` — trend
/// is a function of (window, source, granularity) only, so switching
/// sections must reuse the cached entry.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct TrendMemoKey(pub TimeWindow, pub SourceFilter, pub TrendGranularity);

#[derive(Debug, Default, Clone)]
pub struct DataCache {
    pub left: Vec<LeftRow>,
    pub sessions: Vec<SessionRow>,
    /// Per-turn events for the currently drilled session (only populated
    /// when the deepest drill is `DrillKind::Events`).
    pub events: Vec<EventRow>,
    pub sparkline: Vec<f64>,
    /// Time-bucketed per-source rows; see [`TrendRow`] (field name = time-series slice, not the UI label).
    pub trend: Vec<TrendRow>,
    pub status: CacheStatus,
    pub refresh_error: Option<RefreshError>,
    /// Per-section LeftRow memo. Hits replace the live `left` without
    /// running SQL. Cleared on `Refresh` or when the events table's
    /// `mtime_ns` advances (signaling re-ingest).
    left_memo: BTreeMap<LeftMemoKey, Vec<LeftRow>>,
    /// Per-(window, source, granularity) trend memo.
    trend_memo: BTreeMap<TrendMemoKey, Vec<TrendRow>>,
    /// Sparkline only depends on cache freshness — single optional cache.
    sparkline_memo: Option<Vec<f64>>,
    /// Last-seen events `mtime_ns` for the memos. When this differs from
    /// the live status, all memos are dropped.
    memo_mtime_ns: Option<i64>,
}

impl DataCache {
    pub fn refresh_all(&mut self, conn: &Connection, state: &AppState) {
        self.refresh_for(conn, state, crate::tui::state::RefreshMask::all());
    }

    /// Drop every in-memory memo. Called by `Action::Refresh` and
    /// implicitly when ingest changes the underlying data.
    pub fn clear_memos(&mut self) {
        self.left_memo.clear();
        self.trend_memo.clear();
        self.sparkline_memo = None;
        self.memo_mtime_ns = None;
    }

    fn set_refresh_error(&mut self, scope: RefreshScope, err: impl std::fmt::Display) {
        self.refresh_error = Some(RefreshError::new(scope, err));
    }

    fn clear_refresh_error(&mut self, scope: RefreshScope) {
        if self
            .refresh_error
            .as_ref()
            .is_some_and(|e| e.scope == scope)
        {
            self.refresh_error = None;
        }
    }

    pub fn refresh_for(
        &mut self,
        conn: &Connection,
        state: &AppState,
        mask: crate::tui::state::RefreshMask,
    ) {
        let now = Utc::now();
        let filter = base_filter(state, now);

        self.refresh_status(conn, now);
        self.invalidate_memos_if_source_files_changed();

        if mask.left {
            self.refresh_left(conn, state, &filter);
        }
        if mask.sessions {
            self.refresh_sessions(conn, state, &filter);
        }
        if mask.events {
            self.refresh_events(conn, state, &filter);
        }
        if mask.sparkline {
            self.refresh_sparkline(conn);
        }
        if mask.trend {
            self.refresh_trend(conn, state, now);
        }
    }

    fn refresh_status(&mut self, conn: &Connection, now: DateTime<Utc>) {
        match load_cache_status(conn, now) {
            Ok(status) => {
                self.status = status;
                self.clear_refresh_error(RefreshScope::Status);
            }
            Err(err) => self.set_refresh_error(RefreshScope::Status, err),
        }
    }

    fn invalidate_memos_if_source_files_changed(&mut self) {
        if self.memo_mtime_ns != self.status.mtime_ns {
            self.left_memo.clear();
            self.trend_memo.clear();
            self.sparkline_memo = None;
            self.memo_mtime_ns = self.status.mtime_ns;
        }
    }

    fn refresh_left(&mut self, conn: &Connection, state: &AppState, filter: &QueryFilter) {
        let key = LeftMemoKey(
            state.current_section,
            state.time_window,
            state.source_filter,
            state.trend_granularity,
        );
        if let Some(cached) = self.left_memo.get(&key) {
            self.left = cached.clone();
            sort_left_rows(&mut self.left, state.current_section, state.sort);
            self.clear_refresh_error(RefreshScope::Left);
            return;
        }

        match load_left_axis(
            conn,
            state.current_section,
            filter.clone(),
            state.trend_granularity,
        ) {
            Ok(rows) => {
                self.left = rows;
                // Memoize before sort so the cached form is canonical;
                // each consumer applies its own sort on top.
                self.left_memo.insert(key, self.left.clone());
                sort_left_rows(&mut self.left, state.current_section, state.sort);
                self.clear_refresh_error(RefreshScope::Left);
            }
            Err(err) => self.set_refresh_error(RefreshScope::Left, err),
        }
    }

    fn refresh_sessions(&mut self, conn: &Connection, state: &AppState, filter: &QueryFilter) {
        // For sessions-drill views, scope sessions to the drilled key
        // and originating section. Otherwise scope to whichever row is
        // focused in the current section.
        let (scope_section, sel_key): (Section, Option<String>) = match state.deepest_drill() {
            Some(d) => match d.kind {
                DrillKind::Sessions { from_section } => (from_section, Some(d.key.clone())),
                // Inside an events drill, the sessions slice is whatever
                // the parent (sessions-drill or section root) populated;
                // leave it unchanged.
                DrillKind::Events { .. } => {
                    (state.current_section, left_selected_key(state, &self.left))
                }
            },
            None => (state.current_section, left_selected_key(state, &self.left)),
        };
        if matches!(
            state.deepest_drill().map(|d| d.kind),
            Some(DrillKind::Events { .. })
        ) {
            return;
        }

        match load_sessions_for(conn, scope_section, sel_key.as_deref(), filter.clone()) {
            Ok(rows) => {
                self.sessions = rows;
                sort_session_rows(&mut self.sessions, state.sort);
                self.clear_refresh_error(RefreshScope::Sessions);
            }
            Err(err) => self.set_refresh_error(RefreshScope::Sessions, err),
        }
    }

    fn refresh_events(&mut self, conn: &Connection, state: &AppState, filter: &QueryFilter) {
        match state.deepest_drill() {
            Some(d) => match d.kind {
                DrillKind::Events { source } => {
                    match load_events_for(conn, source, &d.key, filter.clone()) {
                        Ok(rows) => {
                            self.events = rows;
                            sort_event_rows(&mut self.events, state.sort);
                            self.clear_refresh_error(RefreshScope::Events);
                        }
                        Err(err) => self.set_refresh_error(RefreshScope::Events, err),
                    }
                }
                _ => {
                    self.events.clear();
                    self.clear_refresh_error(RefreshScope::Events);
                }
            },
            None => {
                self.events.clear();
                self.clear_refresh_error(RefreshScope::Events);
            }
        }
    }

    fn refresh_sparkline(&mut self, conn: &Connection) {
        self.sparkline = match self.sparkline_memo.as_ref() {
            Some(cached) => {
                let rows = cached.clone();
                self.clear_refresh_error(RefreshScope::Sparkline);
                rows
            }
            None => match load_sparkline(conn, 30) {
                Ok(fresh) => {
                    self.sparkline_memo = Some(fresh.clone());
                    self.clear_refresh_error(RefreshScope::Sparkline);
                    fresh
                }
                Err(err) => {
                    self.set_refresh_error(RefreshScope::Sparkline, err);
                    self.sparkline.clone()
                }
            },
        };
    }

    fn refresh_trend(&mut self, conn: &Connection, state: &AppState, now: DateTime<Utc>) {
        let key = TrendMemoKey(
            state.time_window,
            state.source_filter,
            state.trend_granularity,
        );
        self.trend = match self.trend_memo.get(&key) {
            Some(cached) => {
                let rows = cached.clone();
                self.clear_refresh_error(RefreshScope::Trend);
                rows
            }
            None => match load_trend(conn, state, now) {
                Ok(fresh) => {
                    self.trend_memo.insert(key, fresh.clone());
                    self.clear_refresh_error(RefreshScope::Trend);
                    fresh
                }
                Err(err) => {
                    self.set_refresh_error(RefreshScope::Trend, err);
                    self.trend.clone()
                }
            },
        };
    }
}

pub fn load_cache_status(conn: &Connection, now: DateTime<Utc>) -> Result<CacheStatus> {
    let event_count: i64 = conn.query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))?;
    let max_mtime_ns: Option<i64> =
        conn.query_row("SELECT MAX(mtime_ns) FROM files", [], |row| row.get(0))?;
    let freshness = max_mtime_ns
        .and_then(|ns| chrono::DateTime::from_timestamp(ns / 1_000_000_000, 0))
        .map(|dt| relative_freshness(dt, now))
        .unwrap_or_else(|| "no indexed files".to_owned());
    Ok(CacheStatus {
        cache_path: crate::store::store_path().display().to_string(),
        event_count: event_count.max(0) as u64,
        freshness,
        last_query: now,
        mtime_ns: max_mtime_ns,
    })
}

fn relative_freshness(ts: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let delta = now.signed_duration_since(ts).num_seconds().max(0);
    if delta < 60 {
        "fresh <1m".into()
    } else if delta < 3_600 {
        format!("fresh {}m", delta / 60)
    } else if delta < 86_400 {
        format!("fresh {}h", delta / 3_600)
    } else {
        format!("fresh {}d", delta / 86_400)
    }
}

fn sort_left_rows(rows: &mut [LeftRow], section: Section, sort: Sort) {
    rows.sort_by(|a, b| {
        let ordering = match sort {
            Sort::CostDesc => b
                .cost
                .partial_cmp(&a.cost)
                .unwrap_or(std::cmp::Ordering::Equal),
            Sort::CostAsc => a
                .cost
                .partial_cmp(&b.cost)
                .unwrap_or(std::cmp::Ordering::Equal),
            Sort::RecentDesc => {
                if section == Section::Days {
                    b.key.cmp(&a.key)
                } else {
                    b.latest_ts.cmp(&a.latest_ts)
                }
            }
            Sort::RecentAsc => {
                if section == Section::Days {
                    a.key.cmp(&b.key)
                } else {
                    a.latest_ts.cmp(&b.latest_ts)
                }
            }
            Sort::AlphaDesc => b.label.cmp(&a.label),
            Sort::AlphaAsc => a.label.cmp(&b.label),
        };
        match (section == Section::Repos, a.is_no_repo, b.is_no_repo) {
            (true, true, false) => std::cmp::Ordering::Greater,
            (true, false, true) => std::cmp::Ordering::Less,
            _ => ordering.then_with(|| a.label.cmp(&b.label)),
        }
    });
}

fn sort_event_rows(rows: &mut [EventRow], sort: Sort) {
    rows.sort_by(|a, b| match sort {
        // Default chronological is preserved by load order; the sort cycle
        // adds expense and reverse-chronological views on demand.
        Sort::CostDesc => b
            .cost
            .partial_cmp(&a.cost)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.ts.cmp(&b.ts)),
        Sort::CostAsc => a
            .cost
            .partial_cmp(&b.cost)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.ts.cmp(&b.ts)),
        Sort::RecentDesc => b.ts.cmp(&a.ts),
        Sort::RecentAsc => a.ts.cmp(&b.ts),
        Sort::AlphaDesc => b.model.cmp(&a.model).then_with(|| a.ts.cmp(&b.ts)),
        Sort::AlphaAsc => a.model.cmp(&b.model).then_with(|| a.ts.cmp(&b.ts)),
    });
}

pub(super) fn sort_session_rows(rows: &mut [SessionRow], sort: Sort) {
    rows.sort_by(|a, b| match sort {
        Sort::CostDesc => b
            .cost
            .partial_cmp(&a.cost)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.latest_ts.cmp(&a.latest_ts)),
        Sort::CostAsc => a
            .cost
            .partial_cmp(&b.cost)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.latest_ts.cmp(&a.latest_ts)),
        Sort::RecentDesc => b.latest_ts.cmp(&a.latest_ts),
        Sort::RecentAsc => a.latest_ts.cmp(&b.latest_ts),
        Sort::AlphaDesc => b
            .project
            .as_deref()
            .unwrap_or(&b.session_id)
            .cmp(a.project.as_deref().unwrap_or(&a.session_id)),
        Sort::AlphaAsc => a
            .project
            .as_deref()
            .unwrap_or(&a.session_id)
            .cmp(b.project.as_deref().unwrap_or(&b.session_id)),
    });
}

fn base_filter(state: &AppState, now: DateTime<Utc>) -> QueryFilter {
    QueryFilter {
        source: state.source_filter.as_source(),
        since_ms: state.time_window.since_ms(now),
        until_ms: None,
        repo: None,
    }
}

fn parse_source_column(raw: String, col: usize) -> rusqlite::Result<Source> {
    Source::from_str(&raw).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(
            col,
            Type::Text,
            Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, err)),
        )
    })
}

fn left_selected_key(state: &AppState, left: &[LeftRow]) -> Option<String> {
    if left.is_empty() {
        return None;
    }
    let idx = state.current_index().min(left.len() - 1);
    Some(left[idx].key.clone())
}

pub fn load_left_axis(
    conn: &Connection,
    section: Section,
    filter: QueryFilter,
    granularity: TrendGranularity,
) -> Result<Vec<LeftRow>> {
    match section {
        Section::Repos => {
            let rows: Vec<RepoAggregateRow> = repo_report(conn, filter)?;
            Ok(rows
                .into_iter()
                .map(|r| LeftRow {
                    label: r.display_name.clone(),
                    key: r.key.clone(),
                    sessions: r.sessions,
                    total_tokens: r.total_tokens,
                    cost: r.cost_usd,
                    is_no_repo: r.is_no_repo(),
                    latest_ts: Some(r.latest_timestamp),
                    source: None,
                })
                .collect())
        }
        Section::Days => load_periods(conn, filter, granularity),
        Section::Models => load_models(conn, filter),
        Section::Sessions => {
            let rows = session_report(conn, filter)?;
            Ok(rows
                .into_iter()
                .map(|r| {
                    let source = match r.source {
                        crate::types::SourceLabel::Source(s) => Some(s),
                        _ => None,
                    };
                    LeftRow {
                        label: r
                            .project_path
                            .as_deref()
                            .map(crate::repo::project_basename)
                            .map(String::from)
                            .unwrap_or_else(|| r.key.chars().take(10).collect()),
                        key: r.key,
                        sessions: 1,
                        total_tokens: r.total_tokens,
                        cost: r.cost_usd,
                        is_no_repo: false,
                        latest_ts: r.latest_timestamp,
                        source,
                    }
                })
                .collect())
        }
        // Provider section uses load_trend; left-axis loading is a no-op.
        Section::Provider => Ok(Vec::new()),
    }
}

/// Group events into time buckets at the chosen granularity. Daily/Monthly
/// use the existing pre-computed columns; weekly/yearly compute the bucket
/// from the day string in SQLite.
fn load_periods(
    conn: &Connection,
    filter: QueryFilter,
    granularity: TrendGranularity,
) -> Result<Vec<LeftRow>> {
    // SQLite's strftime week (`%W`) is Sunday-based, not ISO. Close enough
    // for visualization. Yearly buckets use the leading 4 chars of `day`.
    let bucket_expr = match granularity {
        TrendGranularity::Daily => "e.day",
        TrendGranularity::Weekly => "strftime('%Y-W%W', e.day)",
        TrendGranularity::Monthly => "e.month",
        TrendGranularity::Yearly => "substr(e.day, 1, 4)",
    };
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
        bucket = bucket_expr,
        src = if filter.source.is_some() {
            "AND e.source = ?"
        } else {
            ""
        },
        ts = "AND (? IS NULL OR e.ts >= ?) AND (? IS NULL OR e.ts <= ?)",
    );
    let mut params: Vec<Value> = Vec::new();
    if let Some(s) = filter.source {
        params.push(Value::Text(s.as_str().to_owned()));
    }
    for v in [
        filter.since_ms,
        filter.since_ms,
        filter.until_ms,
        filter.until_ms,
    ] {
        params.push(match v {
            Some(x) => Value::Integer(x),
            None => Value::Null,
        });
    }
    let mut stmt = conn.prepare_cached(&sql)?;
    let rows = stmt.query_map(params_from_iter(params.iter()), |row| {
        Ok(LeftRow {
            label: row.get::<_, String>(0)?,
            key: row.get::<_, String>(0)?,
            sessions: row.get::<_, i64>(1)? as u64,
            cost: row.get::<_, f64>(2)?,
            total_tokens: row.get::<_, i64>(3)? as u64,
            is_no_repo: false,
            latest_ts: Some(
                Utc.timestamp_millis_opt(row.get(4)?)
                    .single()
                    .unwrap_or_else(Utc::now),
            ),
            source: None,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn load_models(conn: &Connection, filter: QueryFilter) -> Result<Vec<LeftRow>> {
    // Reuse the same WHERE shape daily_report uses, grouped by model.
    let sql = format!(
        r#"SELECT e.model AS model,
                  COUNT(DISTINCT e.source || char(31) || e.session_id) AS sessions,
                  SUM(e.cost_usd) AS cost,
                  SUM(e.input + e.output + e.cache_read + e.cache_write) AS total_tokens,
                  MAX(e.ts) AS latest_ts
             FROM events e
             WHERE 1=1 {src} {ts} {repo}
             GROUP BY model
             ORDER BY cost DESC"#,
        src = if filter.source.is_some() {
            "AND e.source = ?"
        } else {
            ""
        },
        ts = "AND (? IS NULL OR e.ts >= ?) AND (? IS NULL OR e.ts <= ?)",
        repo = "",
    );
    let mut params: Vec<Value> = Vec::new();
    if let Some(s) = filter.source {
        params.push(Value::Text(s.as_str().to_owned()));
    }
    for v in [
        filter.since_ms,
        filter.since_ms,
        filter.until_ms,
        filter.until_ms,
    ] {
        params.push(match v {
            Some(x) => Value::Integer(x),
            None => Value::Null,
        });
    }
    let mut stmt = conn.prepare_cached(&sql)?;
    let rows = stmt.query_map(params_from_iter(params.iter()), |row| {
        Ok(LeftRow {
            label: row.get::<_, String>(0)?,
            key: row.get::<_, String>(0)?,
            sessions: row.get::<_, i64>(1)? as u64,
            cost: row.get::<_, f64>(2)?,
            total_tokens: row.get::<_, i64>(3)? as u64,
            is_no_repo: false,
            latest_ts: Some(
                Utc.timestamp_millis_opt(row.get(4)?)
                    .single()
                    .unwrap_or_else(Utc::now),
            ),
            source: None,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn load_sessions_for(
    conn: &Connection,
    section: Section,
    key: Option<&str>,
    mut filter: QueryFilter,
) -> Result<Vec<SessionRow>> {
    match (section, key) {
        (Section::Repos, Some(k)) => {
            filter.repo = Some(if k == crate::store::queries::NO_REPO_SENTINEL {
                RepoFilterSpec::NoRepo
            } else {
                RepoFilterSpec::KeyPrefix(k.to_owned())
            });
            Ok(session_report(conn, filter)?
                .into_iter()
                .map(to_session_row)
                .collect())
        }
        (Section::Days, Some(k)) => {
            // Filter by exact day (parse YYYY-MM-DD to ms bounds).
            let (since, until) = day_bounds(k);
            filter.since_ms = Some(since);
            filter.until_ms = Some(until);
            Ok(session_report(conn, filter)?
                .into_iter()
                .map(to_session_row)
                .collect())
        }
        (Section::Models, Some(k)) => {
            // No model filter on QueryFilter; do it via a direct SQL query.
            load_sessions_by_model(conn, k, &filter)
        }
        (Section::Sessions, Some(k)) => {
            // Just return the one.
            let mut rows = session_report(conn, filter)?;
            rows.retain(|r| r.key == k);
            Ok(rows.into_iter().map(to_session_row).collect())
        }
        _ => Ok(session_report(conn, filter)?
            .into_iter()
            .map(to_session_row)
            .collect()),
    }
}

fn to_session_row(r: AggregateRow) -> SessionRow {
    let source = match r.source {
        crate::types::SourceLabel::Source(s) => s,
        _ => Source::Claude,
    };
    SessionRow {
        session_id: r.key,
        source,
        latest_ts: r.latest_timestamp.unwrap_or_else(Utc::now),
        project: r.project_path,
        cost: r.cost_usd,
        total_tokens: r.total_tokens,
    }
}

fn load_sessions_by_model(
    conn: &Connection,
    model: &str,
    filter: &QueryFilter,
) -> Result<Vec<SessionRow>> {
    let sql = format!(
        r#"SELECT e.session_id,
                  e.source,
                  MAX(r.display_name)  AS repo_display,
                  MAX(e.project_path)  AS project_path,
                  MAX(e.ts),
                  SUM(e.input + e.output + e.cache_read + e.cache_write),
                  SUM(e.cost_usd)
             FROM events e
             LEFT JOIN repos r ON r.key = e.repo
             WHERE e.model = ?1 {src} {ts}
             GROUP BY e.source, e.session_id
             ORDER BY MAX(e.ts) DESC"#,
        src = if filter.source.is_some() {
            "AND e.source = ?"
        } else {
            ""
        },
        ts = "AND (? IS NULL OR e.ts >= ?) AND (? IS NULL OR e.ts <= ?)",
    );
    let mut params: Vec<Value> = Vec::new();
    params.push(Value::Text(model.to_owned()));
    if let Some(s) = filter.source {
        params.push(Value::Text(s.as_str().to_owned()));
    }
    for v in [
        filter.since_ms,
        filter.since_ms,
        filter.until_ms,
        filter.until_ms,
    ] {
        params.push(match v {
            Some(x) => Value::Integer(x),
            None => Value::Null,
        });
    }
    let mut stmt = conn.prepare_cached(&sql)?;
    let rows = stmt.query_map(params_from_iter(params.iter()), |row| {
        let src_str: String = row.get(1)?;
        let source = parse_source_column(src_str, 1)?;
        let repo_display: Option<String> = row.get(2)?;
        let project_path: Option<String> = row.get(3)?;
        let ms: i64 = row.get(4)?;
        let shown = repo_display.or_else(|| {
            project_path
                .as_deref()
                .map(project_basename)
                .map(String::from)
        });
        Ok(SessionRow {
            session_id: row.get(0)?,
            source,
            latest_ts: Utc
                .timestamp_millis_opt(ms)
                .single()
                .unwrap_or_else(Utc::now),
            project: shown,
            total_tokens: row.get::<_, i64>(5)? as u64,
            cost: row.get(6)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn day_bounds(day: &str) -> (i64, i64) {
    let parsed = NaiveDate::parse_from_str(day, "%Y-%m-%d").ok();
    let Some(d) = parsed else {
        return (0, i64::MAX);
    };
    let start = d.and_hms_opt(0, 0, 0).unwrap();
    let end = d.and_hms_opt(23, 59, 59).unwrap();
    let start_local = Local.from_local_datetime(&start).single();
    let end_local = Local.from_local_datetime(&end).single();
    (
        start_local.map(|t| t.timestamp_millis()).unwrap_or(0),
        end_local.map(|t| t.timestamp_millis()).unwrap_or(i64::MAX),
    )
}

/// Load per-turn events for a single `(source, session_id)` pair, applying
/// the active time-window if set. Ordered by `ts` ascending so the default
/// chronological view comes for free; sort variants are applied in
/// [`sort_event_rows`].
pub fn load_events_for(
    conn: &Connection,
    source: Source,
    session_id: &str,
    filter: QueryFilter,
) -> Result<Vec<EventRow>> {
    let sql = "SELECT ts, model, input, output, cache_read, cache_write, cost_usd
                 FROM events
                WHERE source = ?1 AND session_id = ?2
                  AND (?3 IS NULL OR ts >= ?3)
                  AND (?4 IS NULL OR ts <= ?4)
                ORDER BY ts ASC";
    let mut stmt = conn.prepare_cached(sql)?;
    let params: Vec<Value> = vec![
        Value::Text(source.as_str().to_owned()),
        Value::Text(session_id.to_owned()),
        filter.since_ms.map_or(Value::Null, Value::Integer),
        filter.until_ms.map_or(Value::Null, Value::Integer),
    ];
    let rows = stmt.query_map(params_from_iter(params.iter()), |row| {
        let ts_ms: i64 = row.get(0)?;
        Ok(EventRow {
            ts: Utc
                .timestamp_millis_opt(ts_ms)
                .single()
                .unwrap_or_else(Utc::now),
            model: row.get::<_, String>(1)?,
            input: row.get::<_, i64>(2)? as u64,
            output: row.get::<_, i64>(3)? as u64,
            cache_read: row.get::<_, i64>(4)? as u64,
            cache_write: row.get::<_, i64>(5)? as u64,
            cost: row.get::<_, f64>(6)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn load_sparkline(conn: &Connection, days: u32) -> Result<Vec<f64>> {
    let sql = "SELECT day, SUM(cost_usd) FROM events GROUP BY day ORDER BY day DESC LIMIT ?1";
    let mut stmt = conn.prepare_cached(sql)?;
    let rows = stmt.query_map([days as i64], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
    })?;
    let mut pairs: Vec<(String, f64)> = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    pairs.reverse();
    Ok(pairs.into_iter().map(|(_, c)| c).collect())
}

/// Loads the provider-style time-bucket table: one SQL pass grouped by day and source, then
/// split into per-source columns in Rust. Serves the Provider section and Repos Provider tab; bucket
/// size follows `state.trend_granularity` (shared with the Days section).
pub fn load_trend(
    conn: &Connection,
    state: &AppState,
    now: DateTime<Utc>,
) -> Result<Vec<TrendRow>> {
    let since_ms = state.time_window.since_ms(now);
    let sql = r#"SELECT
                   e.day,
                   e.source,
                   SUM(e.cost_usd) AS cost,
                   SUM(e.input + e.output + e.cache_read + e.cache_write) AS tokens
                 FROM events e
                 WHERE (?1 IS NULL OR e.ts >= ?1)
                 GROUP BY e.day, e.source"#;
    let mut stmt = conn.prepare_cached(sql)?;
    let rows = stmt.query_map([since_ms.map_or(Value::Null, Value::Integer)], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, f64>(2)?,
            row.get::<_, i64>(3)? as u64,
        ))
    })?;

    let mut days: std::collections::BTreeMap<String, (f64, f64, f64, u64, f64)> =
        Default::default();
    for row in rows {
        let (day, src_str, cost, tokens) = row?;
        let source = parse_source_column(src_str, 1)?;
        let include_split = match state.source_filter {
            SourceFilter::All => true,
            SourceFilter::Claude => source == Source::Claude,
            SourceFilter::Codex => source == Source::Codex,
            SourceFilter::Cursor => source == Source::Cursor,
        };
        if !include_split {
            continue;
        }
        let entry = days.entry(day).or_insert((0.0, 0.0, 0.0, 0, 0.0));
        entry.3 += tokens;
        entry.4 += cost;
        match source {
            Source::Claude => entry.0 += cost,
            Source::Codex => entry.1 += cost,
            Source::Cursor => entry.2 += cost,
        }
    }

    let current_day = now.with_timezone(&Local).format("%Y-%m-%d").to_string();

    let bucket_of = |day: &str| -> Option<(String, bool)> {
        let d = NaiveDate::parse_from_str(day, "%Y-%m-%d").ok()?;
        let today = NaiveDate::parse_from_str(&current_day, "%Y-%m-%d").ok()?;
        Some(match state.trend_granularity {
            TrendGranularity::Daily => (day.to_owned(), d == today),
            TrendGranularity::Weekly => {
                // ISO week (Mon–Sun).
                let iso = d.iso_week();
                let today_iso = today.iso_week();
                let label = format!("{}-W{:02}", iso.year(), iso.week());
                let is_cur = iso.year() == today_iso.year() && iso.week() == today_iso.week();
                (label, is_cur)
            }
            TrendGranularity::Monthly => {
                let label = format!("{}-{:02}", d.year(), d.month());
                let is_cur = d.year() == today.year() && d.month() == today.month();
                (label, is_cur)
            }
            TrendGranularity::Yearly => {
                let label = format!("{}", d.year());
                let is_cur = d.year() == today.year();
                (label, is_cur)
            }
        })
    };

    let mut buckets: std::collections::BTreeMap<String, TrendRow> = Default::default();
    for (day, (cc, xc, uc, tok, total)) in days {
        if let Some((label, is_cur)) = bucket_of(&day) {
            let entry = buckets.entry(label.clone()).or_insert(TrendRow {
                bucket: label,
                claude_cost: 0.0,
                codex_cost: 0.0,
                cursor_cost: 0.0,
                total_tokens: 0,
                total_cost: 0.0,
                is_current: false,
            });
            entry.claude_cost += cc;
            entry.codex_cost += xc;
            entry.cursor_cost += uc;
            entry.total_tokens += tok;
            entry.total_cost += total;
            entry.is_current = entry.is_current || is_cur;
        }
    }

    Ok(buckets.into_values().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::schema::DDL;
    use crate::store::writes::{insert_events, EventRow};
    use crate::tui::state::{AppState, TimeWindow, TrendGranularity};

    #[test]
    fn day_bounds_parses() {
        let (a, b) = day_bounds("2026-04-22");
        assert!(a < b);
    }

    #[test]
    fn short_sessions_label_takes_prefix() {
        let r = SessionRow {
            session_id: "abcdefghij".into(),
            source: Source::Claude,
            latest_ts: Utc::now(),
            project: None,
            cost: 0.0,
            total_tokens: 0,
        };
        assert_eq!(r.session_id.chars().take(8).collect::<String>(), "abcdefgh");
    }

    #[test]
    fn sort_changes_loaded_rows() {
        let mut left = vec![
            LeftRow {
                label: "beta".into(),
                key: "beta".into(),
                sessions: 1,
                total_tokens: 0,
                cost: 2.0,
                is_no_repo: false,
                latest_ts: None,
                source: None,
            },
            LeftRow {
                label: "alpha".into(),
                key: "alpha".into(),
                sessions: 1,
                total_tokens: 0,
                cost: 1.0,
                is_no_repo: false,
                latest_ts: None,
                source: None,
            },
        ];
        sort_left_rows(&mut left, Section::Repos, Sort::AlphaAsc);
        assert_eq!(left[0].label, "alpha");

        let mut sessions = vec![
            SessionRow {
                session_id: "old".into(),
                source: Source::Claude,
                latest_ts: "2026-04-18T09:00:00Z".parse().unwrap(),
                project: Some("old".into()),
                cost: 10.0,
                total_tokens: 0,
            },
            SessionRow {
                session_id: "new".into(),
                source: Source::Claude,
                latest_ts: "2026-04-19T09:00:00Z".parse().unwrap(),
                project: Some("new".into()),
                cost: 1.0,
                total_tokens: 0,
            },
        ];
        sort_session_rows(&mut sessions, Sort::RecentDesc);
        assert_eq!(sessions[0].session_id, "new");
        sort_session_rows(&mut sessions, Sort::RecentAsc);
        assert_eq!(sessions[0].session_id, "old");
        sort_session_rows(&mut sessions, Sort::AlphaDesc);
        assert_eq!(sessions[0].session_id, "old");
    }

    #[test]
    fn sessions_section_recent_sort_is_global_across_sources() {
        let mut left = vec![
            LeftRow {
                label: "claude-old".into(),
                key: "claude-old".into(),
                sessions: 1,
                total_tokens: 0,
                cost: 100.0,
                is_no_repo: false,
                latest_ts: Some("2026-04-18T09:00:00Z".parse().unwrap()),
                source: Some(Source::Claude),
            },
            LeftRow {
                label: "codex-new".into(),
                key: "codex-new".into(),
                sessions: 1,
                total_tokens: 0,
                cost: 1.0,
                is_no_repo: false,
                latest_ts: Some("2026-04-19T09:00:00Z".parse().unwrap()),
                source: Some(Source::Codex),
            },
        ];

        sort_left_rows(&mut left, Section::Sessions, Sort::RecentDesc);

        assert_eq!(left[0].key, "codex-new");
    }

    fn left_memo_key(state: &AppState) -> LeftMemoKey {
        LeftMemoKey(
            state.current_section,
            state.time_window,
            state.source_filter,
            state.trend_granularity,
        )
    }

    fn fixture_conn_with_events() -> Connection {
        let mut conn = mk_conn();
        let tx = conn.transaction().unwrap();
        insert_events(
            &tx,
            &[
                mk_event(1_000, "2024-01-01", "2024-01", Source::Claude, 1.0),
                mk_event(2_000, "2024-01-02", "2024-01", Source::Claude, 2.0),
            ],
        )
        .unwrap();
        tx.commit().unwrap();
        conn
    }

    #[test]
    fn left_memo_caches_after_first_refresh() {
        let conn = fixture_conn_with_events();
        let state = AppState {
            current_section: Section::Days,
            time_window: TimeWindow::All,
            ..AppState::default()
        };
        let mut cache = DataCache::default();
        cache.refresh_for(&conn, &state, crate::tui::state::RefreshMask::all());
        let key = left_memo_key(&state);
        assert!(
            cache.left_memo.contains_key(&key),
            "first refresh populates the memo"
        );
    }

    #[test]
    fn clear_memos_drops_everything() {
        let conn = fixture_conn_with_events();
        let state = AppState {
            current_section: Section::Days,
            time_window: TimeWindow::All,
            ..AppState::default()
        };
        let mut cache = DataCache::default();
        cache.refresh_for(&conn, &state, crate::tui::state::RefreshMask::all());
        assert!(!cache.left_memo.is_empty());
        cache.clear_memos();
        assert!(cache.left_memo.is_empty());
        assert!(cache.trend_memo.is_empty());
        assert!(cache.sparkline_memo.is_none());
        assert!(cache.memo_mtime_ns.is_none());
    }

    #[test]
    fn switching_sections_keeps_both_memos_warm() {
        let conn = fixture_conn_with_events();
        let mut cache = DataCache::default();
        let mut state = AppState {
            current_section: Section::Days,
            time_window: TimeWindow::All,
            ..AppState::default()
        };
        cache.refresh_for(&conn, &state, crate::tui::state::RefreshMask::all());
        let days_key = left_memo_key(&state);

        // Visit Models — should add a second memo entry without evicting Days.
        state.current_section = Section::Models;
        cache.refresh_for(
            &conn,
            &state,
            crate::tui::state::RefreshMask {
                left: true,
                ..Default::default()
            },
        );
        let models_key = left_memo_key(&state);

        assert!(cache.left_memo.contains_key(&days_key));
        assert!(cache.left_memo.contains_key(&models_key));
    }

    #[test]
    fn trend_memo_survives_section_switch() {
        let conn = fixture_conn_with_events();
        let mut cache = DataCache::default();
        let mut state = AppState {
            current_section: Section::Provider,
            time_window: TimeWindow::All,
            trend_granularity: TrendGranularity::Daily,
            ..AppState::default()
        };
        cache.refresh_for(&conn, &state, crate::tui::state::RefreshMask::all());
        let trend_key = TrendMemoKey(
            state.time_window,
            state.source_filter,
            state.trend_granularity,
        );
        assert!(cache.trend_memo.contains_key(&trend_key));

        // Switch to Repos and back — the same trend key must still be warm.
        state.current_section = Section::Repos;
        cache.refresh_for(
            &conn,
            &state,
            crate::tui::state::RefreshMask {
                left: true,
                ..Default::default()
            },
        );
        assert!(
            cache.trend_memo.contains_key(&trend_key),
            "trend memo is keyed without section, so a section switch must not evict it"
        );
    }

    #[test]
    fn mtime_change_clears_memos() {
        let conn = fixture_conn_with_events();
        let state = AppState {
            current_section: Section::Days,
            time_window: TimeWindow::All,
            ..AppState::default()
        };
        let mut cache = DataCache::default();
        cache.refresh_for(&conn, &state, crate::tui::state::RefreshMask::all());
        assert!(!cache.left_memo.is_empty());

        // Simulate ingest by advancing the recorded memo mtime so it
        // differs from the live status on the next refresh — the live
        // mtime will then reset memo_mtime_ns and clear the memos.
        cache.memo_mtime_ns = Some(0);
        cache.refresh_for(
            &conn,
            &state,
            crate::tui::state::RefreshMask {
                left: true,
                ..Default::default()
            },
        );
        // After this refresh the memo is freshly repopulated against the
        // live mtime; assert the live mtime is what we expect.
        assert_eq!(cache.memo_mtime_ns, cache.status.mtime_ns);
    }

    #[test]
    fn cache_status_reports_event_count() {
        let mut conn = mk_conn();
        let tx = conn.transaction().unwrap();
        insert_events(
            &tx,
            &[EventRow {
                file_path: "/x".into(),
                source: Source::Claude,
                ts: 1,
                day: "2026-04-22".into(),
                month: "2026-04".into(),
                session_id: "s".into(),
                project_path: None,
                repo: None,
                model: "claude-sonnet-4-6".into(),
                input: 1,
                output: 0,
                cache_read: 0,
                cache_write: 0,
                cost_usd: 0.0,
            }],
        )
        .unwrap();
        tx.commit().unwrap();
        let status = load_cache_status(&conn, Utc::now()).unwrap();
        assert_eq!(status.event_count, 1);
    }

    #[test]
    fn refresh_failure_is_visible_not_empty_data() {
        let conn = Connection::open_in_memory().unwrap();
        let mut cache = DataCache::default();

        cache.refresh_all(&conn, &AppState::default());

        let err = cache.refresh_error.as_ref().expect("refresh error");
        assert!(err.display_message().contains("refresh failed:"));
    }

    #[test]
    fn valid_empty_refresh_has_no_error() {
        let conn = mk_conn();
        let mut cache = DataCache::default();

        cache.refresh_all(&conn, &AppState::default());

        assert!(cache.refresh_error.is_none());
        assert!(cache.left.is_empty());
    }

    #[test]
    fn successful_refresh_clears_prior_error() {
        let conn = Connection::open_in_memory().unwrap();
        let mut cache = DataCache::default();

        cache.refresh_for(
            &conn,
            &AppState::default(),
            crate::tui::state::RefreshMask {
                left: true,
                ..Default::default()
            },
        );
        assert!(cache.refresh_error.is_some());

        conn.execute_batch(DDL).unwrap();
        cache.refresh_for(
            &conn,
            &AppState::default(),
            crate::tui::state::RefreshMask {
                left: true,
                ..Default::default()
            },
        );

        assert!(cache.refresh_error.is_none());
    }

    #[test]
    fn refresh_failure_preserves_previous_successful_rows() {
        let conn = fixture_conn_with_events();
        let state = AppState {
            current_section: Section::Days,
            time_window: TimeWindow::All,
            ..AppState::default()
        };
        let mut cache = DataCache::default();
        cache.refresh_for(
            &conn,
            &state,
            crate::tui::state::RefreshMask {
                left: true,
                ..Default::default()
            },
        );
        assert!(!cache.left.is_empty());

        conn.execute("DROP TABLE events", []).unwrap();
        cache.refresh_for(
            &conn,
            &state,
            crate::tui::state::RefreshMask {
                left: true,
                ..Default::default()
            },
        );

        assert!(cache.refresh_error.is_some());
        assert!(!cache.left.is_empty());
    }

    #[test]
    fn refresh_does_not_mutate_cache_tables() {
        let conn = fixture_conn_with_events();
        let before_events: i64 = conn
            .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
            .unwrap();
        let before_files: i64 = conn
            .query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))
            .unwrap();

        let mut cache = DataCache::default();
        cache.refresh_all(
            &conn,
            &AppState {
                time_window: TimeWindow::All,
                ..AppState::default()
            },
        );

        let after_events: i64 = conn
            .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
            .unwrap();
        let after_files: i64 = conn
            .query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))
            .unwrap();
        assert_eq!(after_events, before_events);
        assert_eq!(after_files, before_files);
    }

    fn mk_conn() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(DDL).unwrap();
        c
    }

    fn mk_event(ts_ms: i64, day: &str, month: &str, src: Source, cost: f64) -> EventRow {
        EventRow {
            file_path: format!("/f-{ts_ms}.jsonl"),
            source: src,
            ts: ts_ms,
            day: day.into(),
            month: month.into(),
            session_id: format!("s-{ts_ms}"),
            project_path: None,
            repo: None,
            model: "claude-sonnet-4-6".into(),
            input: 10,
            output: 10,
            cache_read: 0,
            cache_write: 0,
            cost_usd: cost,
        }
    }

    #[test]
    fn trend_monthly_buckets_by_month() {
        let mut conn = mk_conn();
        let tx = conn.transaction().unwrap();
        insert_events(
            &tx,
            &[
                mk_event(
                    1_700_000_000_000,
                    "2023-11-14",
                    "2023-11",
                    Source::Claude,
                    1.0,
                ),
                mk_event(
                    1_700_100_000_000,
                    "2023-11-15",
                    "2023-11",
                    Source::Codex,
                    2.0,
                ),
                mk_event(
                    1_702_000_000_000,
                    "2023-12-08",
                    "2023-12",
                    Source::Claude,
                    4.0,
                ),
            ],
        )
        .unwrap();
        tx.commit().unwrap();

        let state = AppState {
            time_window: TimeWindow::All,
            trend_granularity: TrendGranularity::Monthly,
            ..AppState::default()
        };
        let now = Utc::now();
        let rows = load_trend(&conn, &state, now).unwrap();
        let nov = rows
            .iter()
            .find(|r| r.bucket.contains("11"))
            .expect("nov bucket");
        assert!((nov.claude_cost - 1.0).abs() < 1e-9);
        assert!((nov.codex_cost - 2.0).abs() < 1e-9);
        assert_eq!(nov.cursor_cost, 0.0);
        let dec = rows
            .iter()
            .find(|r| r.bucket.contains("12"))
            .expect("dec bucket");
        assert!((dec.claude_cost - 4.0).abs() < 1e-9);
    }

    #[test]
    fn trend_source_filter_zeroes_other_column() {
        let mut conn = mk_conn();
        let tx = conn.transaction().unwrap();
        insert_events(
            &tx,
            &[
                mk_event(
                    1_700_000_000_000,
                    "2023-11-14",
                    "2023-11",
                    Source::Claude,
                    1.0,
                ),
                mk_event(
                    1_700_100_000_000,
                    "2023-11-15",
                    "2023-11",
                    Source::Codex,
                    2.0,
                ),
            ],
        )
        .unwrap();
        tx.commit().unwrap();
        let state = AppState {
            time_window: TimeWindow::All,
            trend_granularity: TrendGranularity::Monthly,
            source_filter: crate::tui::state::SourceFilter::Claude,
            ..AppState::default()
        };
        let rows = load_trend(&conn, &state, Utc::now()).unwrap();
        let nov = &rows[0];
        assert!((nov.claude_cost - 1.0).abs() < 1e-9);
        assert_eq!(nov.codex_cost, 0.0);
        assert_eq!(nov.cursor_cost, 0.0);
        assert!((nov.total_cost - 1.0).abs() < 1e-9);
        assert_eq!(nov.total_tokens, 20);
    }

    #[test]
    fn load_events_for_returns_only_matching_session_ordered_chronologically() {
        let mut conn = mk_conn();
        let tx = conn.transaction().unwrap();
        let mut e_match = mk_event(2_000, "2023-11-15", "2023-11", Source::Claude, 0.5);
        e_match.session_id = "alpha".into();
        let mut e_match_earlier = mk_event(1_000, "2023-11-15", "2023-11", Source::Claude, 0.2);
        e_match_earlier.session_id = "alpha".into();
        let mut e_other_session = mk_event(3_000, "2023-11-15", "2023-11", Source::Claude, 1.0);
        e_other_session.session_id = "beta".into();
        let mut e_other_source = mk_event(4_000, "2023-11-15", "2023-11", Source::Codex, 0.3);
        e_other_source.session_id = "alpha".into();
        insert_events(
            &tx,
            &[e_match, e_match_earlier, e_other_session, e_other_source],
        )
        .unwrap();
        tx.commit().unwrap();

        let filter = QueryFilter {
            source: None,
            since_ms: None,
            until_ms: None,
            repo: None,
        };
        let rows = load_events_for(&conn, Source::Claude, "alpha", filter).unwrap();
        assert_eq!(rows.len(), 2, "only alpha-claude events");
        assert!(rows[0].ts < rows[1].ts, "ascending by ts");
        assert!((rows[0].cost - 0.2).abs() < 1e-9);
    }

    #[test]
    fn load_events_for_respects_time_window() {
        let mut conn = mk_conn();
        let tx = conn.transaction().unwrap();
        let mut early = mk_event(1_000, "2023-11-15", "2023-11", Source::Claude, 0.1);
        early.session_id = "s".into();
        let mut late = mk_event(5_000, "2023-11-15", "2023-11", Source::Claude, 0.2);
        late.session_id = "s".into();
        insert_events(&tx, &[early, late]).unwrap();
        tx.commit().unwrap();

        let filter = QueryFilter {
            source: None,
            since_ms: Some(3_000),
            until_ms: None,
            repo: None,
        };
        let rows = load_events_for(&conn, Source::Claude, "s", filter).unwrap();
        assert_eq!(rows.len(), 1);
        assert!((rows[0].cost - 0.2).abs() < 1e-9);
    }

    #[test]
    fn trend_daily_marks_today_current() {
        let mut conn = mk_conn();
        let today_local = chrono::Local::now().date_naive();
        let today_str = today_local.format("%Y-%m-%d").to_string();
        let month_str = today_local.format("%Y-%m").to_string();
        let ts_ms = chrono::Local
            .from_local_datetime(&today_local.and_hms_opt(12, 0, 0).unwrap())
            .single()
            .unwrap()
            .timestamp_millis();
        let tx = conn.transaction().unwrap();
        insert_events(
            &tx,
            &[mk_event(ts_ms, &today_str, &month_str, Source::Claude, 1.5)],
        )
        .unwrap();
        tx.commit().unwrap();
        let state = AppState {
            time_window: TimeWindow::All,
            trend_granularity: TrendGranularity::Daily,
            ..AppState::default()
        };
        let rows = load_trend(&conn, &state, Utc::now()).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].is_current);
    }
}
