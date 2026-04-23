use anyhow::Result;
use chrono::{DateTime, Datelike, Local, NaiveDate, TimeZone, Utc};
use rusqlite::{params_from_iter, types::Value, Connection};

use crate::repo::project_basename;
use crate::store::queries::{
    repo_report, session_report, QueryFilter, RepoAggregateRow, RepoFilterSpec,
};
use crate::tui::state::{AppState, LeftAxis, SourceFilter, TrendGranularity};
use crate::types::{AggregateRow, Source};
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

#[derive(Debug, Clone)]
pub struct TrendRow {
    pub bucket: String,
    pub claude_cost: f64,
    pub codex_cost: f64,
    pub total_tokens: u64,
    pub total_cost: f64,
    pub is_current: bool,
}

#[derive(Debug, Default, Clone)]
pub struct DataCache {
    pub left: Vec<LeftRow>,
    pub sessions: Vec<SessionRow>,
    pub sparkline: Vec<f64>,
    pub trend: Vec<TrendRow>,
}

impl DataCache {
    pub fn refresh_all(&mut self, conn: &Connection, state: &AppState) {
        self.refresh_for(conn, state, crate::tui::state::RefreshMask::all());
    }

    pub fn refresh_for(
        &mut self,
        conn: &Connection,
        state: &AppState,
        mask: crate::tui::state::RefreshMask,
    ) {
        let now = Utc::now();
        let filter = base_filter(state, now);

        if mask.left {
            self.left = load_left_axis(conn, state.left_axis, filter.clone()).unwrap_or_default();
            // Repo axis: pin (no-repo) to bottom and sort by cost desc.
            if state.left_axis == LeftAxis::Repo {
                self.left
                    .sort_by(|a, b| match (a.is_no_repo, b.is_no_repo) {
                        (true, false) => std::cmp::Ordering::Greater,
                        (false, true) => std::cmp::Ordering::Less,
                        _ => b
                            .cost
                            .partial_cmp(&a.cost)
                            .unwrap_or(std::cmp::Ordering::Equal),
                    });
            }
        }
        if mask.sessions {
            let sel_key = left_selected_key(state, &self.left);
            self.sessions =
                load_sessions_for(conn, state.left_axis, sel_key.as_deref(), filter.clone())
                    .unwrap_or_default();
        }
        if mask.sparkline {
            self.sparkline = load_sparkline(conn, 30).unwrap_or_default();
        }
        if mask.trend {
            self.trend = load_trend(conn, state, now).unwrap_or_default();
        }
    }
}

fn base_filter(state: &AppState, now: DateTime<Utc>) -> QueryFilter {
    QueryFilter {
        source: state.source_filter.as_source(),
        since_ms: state.time_window.since_ms(now),
        until_ms: None,
        repo: None,
    }
}

fn left_selected_key(state: &AppState, left: &[LeftRow]) -> Option<String> {
    if left.is_empty() {
        return None;
    }
    let idx = state.left_index.min(left.len() - 1);
    Some(left[idx].key.clone())
}

pub fn load_left_axis(
    conn: &Connection,
    axis: LeftAxis,
    filter: QueryFilter,
) -> Result<Vec<LeftRow>> {
    match axis {
        LeftAxis::Repo => {
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
                })
                .collect())
        }
        LeftAxis::Day => load_days(conn, filter),
        LeftAxis::Model => load_models(conn, filter),
        LeftAxis::Session => {
            let rows = session_report(conn, filter)?;
            Ok(rows
                .into_iter()
                .map(|r| LeftRow {
                    label: r.key.chars().take(10).collect(),
                    key: r.key,
                    sessions: 1,
                    total_tokens: r.total_tokens,
                    cost: r.cost_usd,
                    is_no_repo: false,
                })
                .collect())
        }
    }
}

fn load_days(conn: &Connection, filter: QueryFilter) -> Result<Vec<LeftRow>> {
    let sql = format!(
        r#"SELECT e.day AS day,
                  COUNT(DISTINCT e.session_id) AS sessions,
                  SUM(e.cost_usd) AS cost,
                  SUM(e.input + e.output + e.cache_read + e.cache_write) AS total_tokens
             FROM events e
             WHERE 1=1 {src} {ts}
             GROUP BY day
             ORDER BY day DESC"#,
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
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(params.iter()), |row| {
        Ok(LeftRow {
            label: row.get::<_, String>(0)?,
            key: row.get::<_, String>(0)?,
            sessions: row.get::<_, i64>(1)? as u64,
            cost: row.get::<_, f64>(2)?,
            total_tokens: row.get::<_, i64>(3)? as u64,
            is_no_repo: false,
        })
    })?;
    Ok(rows.filter_map(std::result::Result::ok).collect())
}

fn load_models(conn: &Connection, filter: QueryFilter) -> Result<Vec<LeftRow>> {
    // Reuse the same WHERE shape daily_report uses, grouped by model.
    let sql = format!(
        r#"SELECT e.model AS model,
                  COUNT(DISTINCT e.session_id) AS sessions,
                  SUM(e.cost_usd) AS cost,
                  SUM(e.input + e.output + e.cache_read + e.cache_write) AS total_tokens
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
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(params.iter()), |row| {
        Ok(LeftRow {
            label: row.get::<_, String>(0)?,
            key: row.get::<_, String>(0)?,
            sessions: row.get::<_, i64>(1)? as u64,
            cost: row.get::<_, f64>(2)?,
            total_tokens: row.get::<_, i64>(3)? as u64,
            is_no_repo: false,
        })
    })?;
    Ok(rows.filter_map(std::result::Result::ok).collect())
}

pub fn load_sessions_for(
    conn: &Connection,
    axis: LeftAxis,
    key: Option<&str>,
    mut filter: QueryFilter,
) -> Result<Vec<SessionRow>> {
    match (axis, key) {
        (LeftAxis::Repo, Some(k)) => {
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
        (LeftAxis::Day, Some(k)) => {
            // Filter by exact day (parse YYYY-MM-DD to ms bounds).
            let (since, until) = day_bounds(k);
            filter.since_ms = Some(since);
            filter.until_ms = Some(until);
            Ok(session_report(conn, filter)?
                .into_iter()
                .map(to_session_row)
                .collect())
        }
        (LeftAxis::Model, Some(k)) => {
            // No model filter on QueryFilter; do it via a direct SQL query.
            load_sessions_by_model(conn, k, &filter)
        }
        (LeftAxis::Session, Some(k)) => {
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
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(params.iter()), |row| {
        let src_str: String = row.get(1)?;
        let source = Source::from_str(&src_str).unwrap_or(Source::Codex);
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
    Ok(rows.filter_map(std::result::Result::ok).collect())
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

pub fn load_sparkline(conn: &Connection, days: u32) -> Result<Vec<f64>> {
    let sql = "SELECT day, SUM(cost_usd) FROM events GROUP BY day ORDER BY day DESC LIMIT ?1";
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([days as i64], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
    })?;
    let mut pairs: Vec<(String, f64)> = rows.filter_map(std::result::Result::ok).collect();
    pairs.reverse();
    Ok(pairs.into_iter().map(|(_, c)| c).collect())
}

/// Trend data: one query grouping by `(day, source)`; cost is split per source in Rust.
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
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([since_ms.map_or(Value::Null, Value::Integer)], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, f64>(2)?,
            row.get::<_, i64>(3)? as u64,
        ))
    })?;

    let mut days: std::collections::BTreeMap<String, (f64, f64, u64, f64)> = Default::default();
    for row in rows.filter_map(std::result::Result::ok) {
        let (day, src_str, cost, tokens) = row;
        let source = Source::from_str(&src_str).unwrap_or(Source::Codex);
        let include_split = match state.source_filter {
            SourceFilter::All => true,
            SourceFilter::Claude => source == Source::Claude,
            SourceFilter::Codex => source == Source::Codex,
        };
        let entry = days.entry(day).or_insert((0.0, 0.0, 0, 0.0));
        entry.2 += tokens;
        entry.3 += cost;
        if include_split {
            match source {
                Source::Claude => entry.0 += cost,
                Source::Codex => entry.1 += cost,
            }
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
    for (day, (cc, xc, tok, total)) in days {
        if let Some((label, is_cur)) = bucket_of(&day) {
            let entry = buckets.entry(label.clone()).or_insert(TrendRow {
                bucket: label,
                claude_cost: 0.0,
                codex_cost: 0.0,
                total_tokens: 0,
                total_cost: 0.0,
                is_current: false,
            });
            entry.claude_cost += cc;
            entry.codex_cost += xc;
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
