use crate::store::Store;
use anyhow::Result;
use chrono::{DateTime, Datelike, NaiveDate, TimeZone, Utc};

use super::rows::{AggregateSelection, CacheStatus, EventRow, LeftRow, SessionRow, TrendRow};

use crate::store::queries::{
    daily_cost_by_source, event_rows, model_buckets, period_buckets, repo_report, session_report,
    store_status, BucketAggregateRow, PeriodGranularity, QueryFilter, QueryScope, RepoAggregateRow,
    RepoFilterSpec,
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

fn bucket_to_left_row(r: BucketAggregateRow, selection: AggregateSelection) -> LeftRow {
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
        selection,
    }
}

pub fn load_cache_status(conn: &Store, now: DateTime<Utc>) -> Result<CacheStatus> {
    let status = store_status(conn)?;
    let max_mtime_ns = status.max_mtime_ns;
    let freshness = max_mtime_ns
        .and_then(|ns| chrono::DateTime::from_timestamp(ns / 1_000_000_000, 0))
        .map(|dt| relative_freshness(dt, now))
        .unwrap_or_else(|| "no indexed files".to_owned());
    Ok(CacheStatus {
        cache_path: conn
            .path()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| ":memory:".into()),
        event_count: status.event_count,
        generation: status.generation,
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

pub fn sort_left_rows(rows: &mut [LeftRow], section: Section, sort: Sort) {
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

pub fn sort_event_rows(rows: &mut [EventRow], sort: Sort) {
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

pub(crate) fn base_filter(state: &AppState, now: DateTime<Utc>) -> Result<QueryFilter> {
    Ok(QueryFilter {
        source: state.source_filter.as_source(),
        since_ms: state.time_window.since_ms(now)?,
        until_ms: None,
        ..Default::default()
    })
}

pub fn load_left_axis(
    conn: &Store,
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
                    selection: AggregateSelection::Repo(if r.is_no_repo() {
                        RepoFilterSpec::NoRepo
                    } else {
                        RepoFilterSpec::KeyExact(r.key.clone())
                    }),
                })
                .collect())
        }
        Section::Days => period_buckets(conn, filter, period_granularity(granularity))?
            .into_iter()
            .map(|row| {
                let (since_ms, until_ms) = bucket_bounds(&row.key, granularity)?;
                Ok(bucket_to_left_row(
                    row,
                    AggregateSelection::Bucket {
                        granularity,
                        since_ms,
                        until_ms,
                    },
                ))
            })
            .collect(),
        Section::Models => model_buckets(conn, filter).map(|rows| {
            rows.into_iter()
                .map(|row| {
                    let selection = AggregateSelection::Model(row.key.clone());
                    bucket_to_left_row(row, selection)
                })
                .collect()
        }),
        Section::Sessions => Ok(Vec::new()),
        // Provider section uses load_trend; left-axis loading is a no-op.
        Section::Provider => Ok(Vec::new()),
    }
}

pub fn load_sessions_for(conn: &Store, scope: QueryScope) -> Result<Vec<SessionRow>> {
    session_report(conn, scope)?
        .into_iter()
        .map(to_session_row)
        .collect()
}

pub(crate) fn bucket_bounds(bucket: &str, granularity: TrendGranularity) -> Result<(i64, i64)> {
    crate::calendar::Calendar::system()?.bucket_bounds(bucket, period_granularity(granularity))
}

fn to_session_row(r: AggregateRow) -> Result<SessionRow> {
    let source = match r.source {
        crate::types::SourceLabel::Source(s) => s,
        _ => anyhow::bail!("session aggregate has no source identity"),
    };
    Ok(SessionRow {
        session_id: r.key,
        source,
        latest_ts: r.latest_timestamp.unwrap_or_else(Utc::now),
        project: r.project_path,
        cost: r.cost_usd,
        total_tokens: r.total_tokens,
    })
}

pub fn load_events_for(
    conn: &Store,
    source: Source,
    session_id: &str,
    mut filter: QueryFilter,
) -> Result<Vec<EventRow>> {
    filter.session = Some(crate::store::queries::SessionKey {
        source,
        id: session_id.to_owned(),
    });
    Ok(event_rows(conn, filter)?
        .into_iter()
        .map(|row| EventRow {
            ts: row.ts,
            model: row.model,
            input: row.input,
            output: row.output,
            cache_read: row.cache_read,
            cache_write: row.cache_write,
            cost: row.cost,
        })
        .collect())
}

/// Loads the provider-style time-bucket table: one SQL pass grouped by day and source, then
/// split into per-source columns in Rust. Serves the Provider section and Repos Provider tab; bucket
/// size follows `state.trend_granularity` (shared with the Days section).
pub fn load_trend(
    conn: &Store,
    state: &AppState,
    now: DateTime<Utc>,
    scope: QueryScope,
) -> Result<Vec<TrendRow>> {
    let rows = daily_cost_by_source(conn, scope)?;

    let mut days: std::collections::BTreeMap<String, (f64, f64, f64, u128, f64)> =
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
        entry.3 += u128::from(tokens);
        entry.4 += cost;
        match source {
            Source::Claude => entry.0 += cost,
            Source::Codex => entry.1 += cost,
            Source::Cursor => entry.2 += cost,
        }
    }

    let current_day = crate::calendar::Calendar::system()?
        .local_date(now)
        .format("%Y-%m-%d")
        .to_string();

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
