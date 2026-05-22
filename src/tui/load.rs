use anyhow::Result;
use chrono::{DateTime, Datelike, Local, NaiveDate, TimeZone, Utc};
use rusqlite::{params_from_iter, types::Value, Connection};

use super::rows::{CacheStatus, EventRow, LeftRow, SessionRow, TrendRow};
use crate::repo::project_basename;
use crate::store::queries::{
    daily_cost_by_source, model_buckets, parse_source_column, period_buckets, repo_report,
    session_report, sparkline_costs, BucketAggregateRow, PeriodGranularity, QueryFilter,
    RepoAggregateRow, RepoFilterSpec,
};
use crate::tui::state::{AppState, Section, Sort, SourceFilter, TrendGranularity};
use crate::types::{AggregateRow, Source};

fn period_granularity(g: TrendGranularity) -> PeriodGranularity {
    match g {
        TrendGranularity::Daily => PeriodGranularity::Daily,
        TrendGranularity::Weekly => PeriodGranularity::Weekly,
        TrendGranularity::Monthly => PeriodGranularity::Monthly,
        TrendGranularity::Yearly => PeriodGranularity::Yearly,
    }
}

fn bucket_to_left_row(r: BucketAggregateRow) -> LeftRow {
    LeftRow {
        label: r.key.clone(),
        key: r.key,
        sessions: r.sessions,
        total_tokens: r.total_tokens,
        cost: r.cost_usd,
        is_no_repo: false,
        latest_ts: Some(
            Utc.timestamp_millis_opt(r.latest_ts_ms)
                .single()
                .unwrap_or_else(Utc::now),
        ),
        source: None,
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

pub(crate) fn sort_left_rows(rows: &mut [LeftRow], section: Section, sort: Sort) {
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

pub(crate) fn sort_event_rows(rows: &mut [EventRow], sort: Sort) {
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

pub fn sort_session_rows(rows: &mut [SessionRow], sort: Sort) {
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

pub(crate) fn base_filter(state: &AppState, now: DateTime<Utc>) -> QueryFilter {
    QueryFilter {
        source: state.source_filter.as_source(),
        since_ms: state.time_window.since_ms(now),
        until_ms: None,
        repo: None,
    }
}

pub(crate) fn left_selected_key(state: &AppState, left: &[LeftRow]) -> Option<String> {
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
        Section::Days => period_buckets(conn, filter, period_granularity(granularity))
            .map(|rows| rows.into_iter().map(bucket_to_left_row).collect()),
        Section::Models => model_buckets(conn, filter)
            .map(|rows| rows.into_iter().map(bucket_to_left_row).collect()),
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
        r#"WITH filtered AS (
             SELECT e.*
             FROM events e
             WHERE e.model = ?1 {src} {ts}
           )
           SELECT e.session_id,
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
                  MAX(e.ts),
                  SUM(e.input + e.output + e.cache_read + e.cache_write),
                  SUM(e.cost_usd)
             FROM filtered e
             LEFT JOIN repos r ON r.key = e.repo
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
    sparkline_costs(conn, days)
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
    let rows = daily_cost_by_source(conn, since_ms)?;

    let mut days: std::collections::BTreeMap<String, (f64, f64, f64, u64, f64)> =
        Default::default();
    for (day, source, cost, tokens) in rows {
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
#[path = "load_tests.rs"]
mod tests;
