use crate::ingest::run::local_day;
use crate::pricing::cost_of;
use crate::repo::RepoIdentity;
use crate::store::queries::{QueryFilter, RepoFilterSpec, NO_REPO_SENTINEL};
use crate::types::{Source, UsageEvent};
use anyhow::{Context, Result};
use chrono::{Datelike, Duration, Local, NaiveDate, Utc};
use comfy_table::{presets::UTF8_FULL, Cell, ContentArrangement, Row, Table};
use rusqlite::{params_from_iter, types::Value, Connection};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareDimension {
    Source,
    Repo,
    Model,
    Session,
}

impl CompareDimension {
    pub const ALL: [Self; 4] = [Self::Source, Self::Repo, Self::Model, Self::Session];

    pub fn as_str(self) -> &'static str {
        match self {
            CompareDimension::Source => "source",
            CompareDimension::Repo => "repo",
            CompareDimension::Model => "model",
            CompareDimension::Session => "session",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CompareWindow {
    pub label: String,
    pub start: String,
    pub end: String,
}

impl CompareWindow {
    fn contains_day(&self, day: &str) -> bool {
        self.start.as_str() <= day && day <= self.end.as_str()
    }
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct UsageTotals {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub total_tokens: u64,
    pub cost_usd: f64,
}

impl UsageTotals {
    fn add_row(&mut self, row: &EventRow) {
        self.input += row.input;
        self.output += row.output;
        self.cache_read += row.cache_read;
        self.cache_write += row.cache_write;
        self.total_tokens += row.input + row.output + row.cache_read + row.cache_write;
        self.cost_usd += row.cost_usd;
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DeltaTotals {
    pub current: UsageTotals,
    pub baseline: UsageTotals,
    pub token_delta: i128,
    pub token_pct_delta: Option<f64>,
    pub cost_delta: f64,
    pub cost_pct_delta: Option<f64>,
}

impl DeltaTotals {
    fn new(current: UsageTotals, baseline: UsageTotals) -> Self {
        let token_delta = current.total_tokens as i128 - baseline.total_tokens as i128;
        let cost_delta = current.cost_usd - baseline.cost_usd;
        Self {
            token_pct_delta: pct_delta(current.total_tokens as f64, baseline.total_tokens as f64),
            cost_pct_delta: pct_delta(current.cost_usd, baseline.cost_usd),
            current,
            baseline,
            token_delta,
            cost_delta,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BreakdownRow {
    pub key: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
    pub delta: DeltaTotals,
}

#[derive(Debug, Clone, Serialize)]
pub struct Breakdown {
    pub dimension: String,
    pub positive: Vec<BreakdownRow>,
    pub negative: Vec<BreakdownRow>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompareReport {
    pub windows: CompareWindows,
    pub summary: DeltaTotals,
    pub breakdowns: BTreeMap<String, Breakdown>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompareWindows {
    pub current: CompareWindow,
    pub baseline: CompareWindow,
}

pub fn resolve_windows(
    current: Option<&str>,
    baseline: Option<&str>,
    now: chrono::DateTime<Utc>,
) -> Result<CompareWindows> {
    let local_today = now.with_timezone(&Local).date_naive();
    match (current, baseline) {
        (None, None) => {
            let current = month_to_date_window(local_today, "this-month");
            let baseline = previous_month_comparable(&current, "last-month")?;
            Ok(CompareWindows { current, baseline })
        }
        (Some(cur), Some(base)) => Ok(CompareWindows {
            current: parse_window(cur, local_today)?,
            baseline: parse_window(base, local_today)?,
        }),
        (Some(cur), None) => {
            let current = parse_window(cur, local_today)?;
            let baseline = match cur {
                "today" => parse_window("yesterday", local_today)?,
                "this-week" => parse_window("last-week", local_today)?,
                "this-month" => parse_window("last-month", local_today)?,
                "this-year" => parse_window("last-year", local_today)?,
                _ => previous_period(&current)?,
            };
            Ok(CompareWindows { current, baseline })
        }
        (None, Some(_)) => anyhow::bail!("baseline window requires a current window"),
    }
}

fn parse_window(raw: &str, today: NaiveDate) -> Result<CompareWindow> {
    if let Some((start, end)) = raw.split_once("..") {
        let start = parse_day(start)?;
        let end = parse_day(end)?;
        if end < start {
            anyhow::bail!("comparison range end must be on or after start");
        }
        return Ok(window(raw, start, end));
    }

    match raw {
        "today" => Ok(window(raw, today, today)),
        "yesterday" => {
            let d = today - Duration::days(1);
            Ok(window(raw, d, d))
        }
        "this-week" => {
            let start = today - Duration::days(today.weekday().num_days_from_monday() as i64);
            Ok(window(raw, start, today))
        }
        "last-week" => {
            let this_start = today - Duration::days(today.weekday().num_days_from_monday() as i64);
            let start = this_start - Duration::days(7);
            Ok(window(raw, start, start + Duration::days(6)))
        }
        "this-month" => Ok(month_to_date_window(today, raw)),
        "last-month" => {
            let current = month_to_date_window(today, "this-month");
            previous_month_comparable(&current, raw)
        }
        "this-year" => Ok(window(
            raw,
            NaiveDate::from_ymd_opt(today.year(), 1, 1).unwrap(),
            today,
        )),
        "last-year" => {
            let start = NaiveDate::from_ymd_opt(today.year() - 1, 1, 1).unwrap();
            let day = today.ordinal().min(days_in_year(today.year() - 1));
            let end = NaiveDate::from_yo_opt(today.year() - 1, day).unwrap();
            Ok(window(raw, start, end))
        }
        _ => anyhow::bail!(
            "invalid comparison window '{raw}'; use a preset or YYYY-MM-DD..YYYY-MM-DD"
        ),
    }
}

fn parse_day(raw: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .with_context(|| format!("invalid date '{raw}', expected YYYY-MM-DD"))
}

fn month_to_date_window(today: NaiveDate, label: &str) -> CompareWindow {
    let start = NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap();
    window(label, start, today)
}

fn previous_month_comparable(current: &CompareWindow, label: &str) -> Result<CompareWindow> {
    let current_start = parse_day(&current.start)?;
    let current_end = parse_day(&current.end)?;
    let (year, month) = if current_start.month() == 1 {
        (current_start.year() - 1, 12)
    } else {
        (current_start.year(), current_start.month() - 1)
    };
    let start = NaiveDate::from_ymd_opt(year, month, 1).unwrap();
    let span_days = (current_end - current_start).num_days();
    let end_day = (1 + span_days as u32).min(days_in_month(year, month));
    let end = NaiveDate::from_ymd_opt(year, month, end_day).unwrap();
    Ok(window(label, start, end))
}

fn previous_period(current: &CompareWindow) -> Result<CompareWindow> {
    let start = parse_day(&current.start)?;
    let end = parse_day(&current.end)?;
    let span = (end - start).num_days() + 1;
    let baseline_end = start - Duration::days(1);
    let baseline_start = baseline_end - Duration::days(span - 1);
    Ok(window("previous", baseline_start, baseline_end))
}

fn days_in_month(year: i32, month: u32) -> u32 {
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    (NaiveDate::from_ymd_opt(next_year, next_month, 1).unwrap() - Duration::days(1)).day()
}

fn days_in_year(year: i32) -> u32 {
    NaiveDate::from_ymd_opt(year, 12, 31).unwrap().ordinal()
}

fn window(label: &str, start: NaiveDate, end: NaiveDate) -> CompareWindow {
    CompareWindow {
        label: label.to_owned(),
        start: start.format("%Y-%m-%d").to_string(),
        end: end.format("%Y-%m-%d").to_string(),
    }
}

pub fn compare_from_db(
    conn: &Connection,
    windows: CompareWindows,
    filter: QueryFilter,
    dimensions: &[CompareDimension],
    top: usize,
) -> Result<CompareReport> {
    let current = load_rows(conn, &windows.current, &filter)?;
    let baseline = load_rows(conn, &windows.baseline, &filter)?;
    Ok(build_report_from_rows(
        windows, &current, &baseline, dimensions, top,
    ))
}

pub fn compare_from_events(
    resolved: &[(UsageEvent, RepoIdentity)],
    windows: CompareWindows,
    repo_filter: &Option<RepoFilterSpec>,
    dimensions: &[CompareDimension],
    top: usize,
    unknown: &mut HashSet<String>,
) -> CompareReport {
    let mut current = Vec::new();
    let mut baseline = Vec::new();
    for (event, repo) in resolved {
        if !matches_repo(repo, repo_filter) {
            continue;
        }
        let day = local_day(&event.timestamp);
        let cost = cost_of(event, Some(unknown));
        let row = EventRow {
            source: event.source,
            session_id: event.session_id.clone(),
            repo_key: repo.key.clone(),
            repo_label: repo.display_name.clone(),
            model: event.model.clone(),
            input: event.input_tokens,
            output: event.output_tokens,
            cache_read: event.cache_read_tokens,
            cache_write: event.cache_write_tokens,
            cost_usd: cost,
        };
        if windows.current.contains_day(&day) {
            current.push(row.clone());
        }
        if windows.baseline.contains_day(&day) {
            baseline.push(row);
        }
    }
    build_report_from_rows(windows, &current, &baseline, dimensions, top)
}

#[derive(Debug, Clone)]
struct EventRow {
    source: Source,
    session_id: String,
    repo_key: Option<String>,
    repo_label: String,
    model: String,
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
    cost_usd: f64,
}

fn load_rows(
    conn: &Connection,
    window: &CompareWindow,
    filter: &QueryFilter,
) -> Result<Vec<EventRow>> {
    let repo_clause = match &filter.repo {
        None => "",
        Some(RepoFilterSpec::NoRepo) => "AND e.repo IS NULL",
        Some(RepoFilterSpec::DisplayName(_)) => {
            "AND e.repo IN (SELECT key FROM repos WHERE display_name = ?)"
        }
        Some(RepoFilterSpec::KeyPrefix(_)) => "AND e.repo IS NOT NULL AND e.repo LIKE ?",
    };
    let source_clause = if filter.source.is_some() {
        "AND e.source = ?"
    } else {
        ""
    };
    let sql = format!(
        r#"SELECT e.source, e.session_id, e.repo,
                  COALESCE(r.display_name, '{no_repo}'),
                  e.model, e.input, e.output, e.cache_read, e.cache_write, e.cost_usd
             FROM events e
             LEFT JOIN repos r ON r.key = e.repo
             WHERE e.day >= ? AND e.day <= ? {source_clause} {repo_clause}"#,
        no_repo = RepoIdentity::NO_REPO_DISPLAY,
    );
    let mut params = vec![
        Value::Text(window.start.clone()),
        Value::Text(window.end.clone()),
    ];
    if let Some(source) = filter.source {
        params.push(Value::Text(source.as_str().to_owned()));
    }
    match &filter.repo {
        None | Some(RepoFilterSpec::NoRepo) => {}
        Some(RepoFilterSpec::DisplayName(name)) => params.push(Value::Text(name.clone())),
        Some(RepoFilterSpec::KeyPrefix(prefix)) => params.push(Value::Text(format!("{prefix}%"))),
    }
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(params.iter()), |row| {
        let source_raw: String = row.get(0)?;
        let source = source_raw.parse::<Source>().map_err(|err| {
            rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                err,
            )))
        })?;
        Ok(EventRow {
            source,
            session_id: row.get(1)?,
            repo_key: row.get(2)?,
            repo_label: row.get(3)?,
            model: row.get(4)?,
            input: row.get::<_, i64>(5)? as u64,
            output: row.get::<_, i64>(6)? as u64,
            cache_read: row.get::<_, i64>(7)? as u64,
            cache_write: row.get::<_, i64>(8)? as u64,
            cost_usd: row.get(9)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn build_report_from_rows(
    windows: CompareWindows,
    current_rows: &[EventRow],
    baseline_rows: &[EventRow],
    dimensions: &[CompareDimension],
    top: usize,
) -> CompareReport {
    let current_total = totals_for_rows(current_rows);
    let baseline_total = totals_for_rows(baseline_rows);
    let summary = DeltaTotals::new(current_total, baseline_total);
    let mut breakdowns = BTreeMap::new();
    for dimension in dimensions {
        breakdowns.insert(
            dimension.as_str().to_owned(),
            build_breakdown(*dimension, current_rows, baseline_rows, top),
        );
    }
    CompareReport {
        windows,
        summary,
        breakdowns,
    }
}

fn totals_for_rows(rows: &[EventRow]) -> UsageTotals {
    let mut totals = UsageTotals::default();
    for row in rows {
        totals.add_row(row);
    }
    totals
}

fn build_breakdown(
    dimension: CompareDimension,
    current_rows: &[EventRow],
    baseline_rows: &[EventRow],
    top: usize,
) -> Breakdown {
    let mut buckets: HashMap<String, (String, Option<Source>, UsageTotals, UsageTotals)> =
        HashMap::new();
    for row in current_rows {
        let (key, label, source) = dimension_key(dimension, row);
        buckets
            .entry(key)
            .or_insert_with(|| {
                (
                    label,
                    source,
                    UsageTotals::default(),
                    UsageTotals::default(),
                )
            })
            .2
            .add_row(row);
    }
    for row in baseline_rows {
        let (key, label, source) = dimension_key(dimension, row);
        buckets
            .entry(key)
            .or_insert_with(|| {
                (
                    label,
                    source,
                    UsageTotals::default(),
                    UsageTotals::default(),
                )
            })
            .3
            .add_row(row);
    }

    let mut rows: Vec<BreakdownRow> = buckets
        .into_iter()
        .map(|(key, (label, source, current, baseline))| BreakdownRow {
            key,
            label,
            source,
            delta: DeltaTotals::new(current, baseline),
        })
        .collect();
    rows.sort_by(|a, b| {
        b.delta
            .cost_delta
            .abs()
            .partial_cmp(&a.delta.cost_delta.abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let positive = rows
        .iter()
        .filter(|row| row.delta.cost_delta >= 0.0)
        .take(top)
        .cloned()
        .collect();
    let negative = rows
        .into_iter()
        .filter(|row| row.delta.cost_delta < 0.0)
        .take(top)
        .collect();
    Breakdown {
        dimension: dimension.as_str().to_owned(),
        positive,
        negative,
    }
}

fn dimension_key(dimension: CompareDimension, row: &EventRow) -> (String, String, Option<Source>) {
    match dimension {
        CompareDimension::Source => (
            row.source.as_str().to_owned(),
            row.source.as_str().to_owned(),
            Some(row.source),
        ),
        CompareDimension::Repo => {
            let key = row
                .repo_key
                .clone()
                .unwrap_or_else(|| NO_REPO_SENTINEL.to_owned());
            (key, row.repo_label.clone(), None)
        }
        CompareDimension::Model => (row.model.clone(), row.model.clone(), None),
        CompareDimension::Session => (
            format!("{}\u{1f}{}", row.source.as_str(), row.session_id),
            format!("{}:{}", row.source.as_str(), short_id(&row.session_id)),
            Some(row.source),
        ),
    }
}

fn matches_repo(repo: &RepoIdentity, filter: &Option<RepoFilterSpec>) -> bool {
    match filter {
        None => true,
        Some(RepoFilterSpec::NoRepo) => repo.key.is_none(),
        Some(RepoFilterSpec::DisplayName(name)) => repo.key.is_some() && repo.display_name == *name,
        Some(RepoFilterSpec::KeyPrefix(prefix)) => repo
            .key
            .as_deref()
            .is_some_and(|key| key.starts_with(prefix)),
    }
}

fn pct_delta(current: f64, baseline: f64) -> Option<f64> {
    if baseline.abs() < f64::EPSILON {
        None
    } else {
        Some(((current - baseline) / baseline) * 100.0)
    }
}

fn short_id(value: &str) -> String {
    value.chars().take(8).collect()
}

pub fn render_human(report: &CompareReport) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "tokctl compare: {} {}..{} vs {} {}..{}\n\n",
        report.windows.current.label,
        report.windows.current.start,
        report.windows.current.end,
        report.windows.baseline.label,
        report.windows.baseline.start,
        report.windows.baseline.end,
    ));
    let mut summary = Table::new();
    summary
        .load_preset(UTF8_FULL)
        .set_content_arrangement(ContentArrangement::Dynamic);
    summary.set_header(
        ["metric", "current", "baseline", "delta", "%"]
            .iter()
            .map(|s| Cell::new(*s)),
    );
    summary.add_row(Row::from(vec![
        Cell::new("tokens"),
        Cell::new(fmt_u(report.summary.current.total_tokens)),
        Cell::new(fmt_u(report.summary.baseline.total_tokens)),
        Cell::new(format_signed_i(report.summary.token_delta)),
        Cell::new(fmt_pct(report.summary.token_pct_delta)),
    ]));
    summary.add_row(Row::from(vec![
        Cell::new("cost"),
        Cell::new(fmt_cost(report.summary.current.cost_usd)),
        Cell::new(fmt_cost(report.summary.baseline.cost_usd)),
        Cell::new(format_signed_cost(report.summary.cost_delta)),
        Cell::new(fmt_pct(report.summary.cost_pct_delta)),
    ]));
    out.push_str(&summary.to_string());
    out.push('\n');

    for breakdown in report.breakdowns.values() {
        out.push('\n');
        out.push_str(&format!("{}\n", breakdown.dimension));
        out.push_str(&render_driver_table("up", &breakdown.positive));
        if !breakdown.negative.is_empty() {
            out.push('\n');
            out.push_str(&render_driver_table("down", &breakdown.negative));
        }
    }
    out
}

fn render_driver_table(title: &str, rows: &[BreakdownRow]) -> String {
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .set_content_arrangement(ContentArrangement::Dynamic);
    table.set_header(
        [title, "current", "baseline", "delta", "%"]
            .iter()
            .map(|s| Cell::new(*s)),
    );
    for row in rows {
        table.add_row(Row::from(vec![
            Cell::new(&row.label),
            Cell::new(fmt_cost(row.delta.current.cost_usd)),
            Cell::new(fmt_cost(row.delta.baseline.cost_usd)),
            Cell::new(format_signed_cost(row.delta.cost_delta)),
            Cell::new(fmt_pct(row.delta.cost_pct_delta)),
        ]));
    }
    table.to_string()
}

pub fn render_json(report: &CompareReport) -> String {
    serde_json::to_string_pretty(report).unwrap_or_else(|_| "{}".into())
}

fn fmt_u(n: u64) -> String {
    crate::render::fmt_num(n)
}

fn fmt_cost(n: f64) -> String {
    format!("${n:.2}")
}

fn format_signed_i(n: i128) -> String {
    if n >= 0 {
        format!("+{}", fmt_u(n as u64))
    } else {
        format!("-{}", fmt_u(n.unsigned_abs() as u64))
    }
}

fn format_signed_cost(n: f64) -> String {
    if n >= 0.0 {
        format!("+${n:.2}")
    } else {
        format!("-${:.2}", n.abs())
    }
}

fn fmt_pct(value: Option<f64>) -> String {
    value
        .map(|v| format!("{v:+.1}%"))
        .unwrap_or_else(|| "n/a".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::schema::DDL;
    use chrono::TimeZone;

    #[test]
    fn default_windows_are_month_to_date_vs_previous_month() {
        let now = Utc.with_ymd_and_hms(2026, 4, 25, 12, 0, 0).unwrap();
        let windows = resolve_windows(None, None, now).unwrap();
        assert_eq!(windows.current.start, "2026-04-01");
        assert_eq!(windows.current.end, "2026-04-25");
        assert_eq!(windows.baseline.start, "2026-03-01");
        assert_eq!(windows.baseline.end, "2026-03-25");
    }

    #[test]
    fn explicit_range_parses() {
        let today = Utc.with_ymd_and_hms(2026, 4, 25, 12, 0, 0).unwrap();
        let windows = resolve_windows(
            Some("2026-04-01..2026-04-15"),
            Some("2026-03-01..2026-03-15"),
            today,
        )
        .unwrap();
        assert_eq!(windows.current.end, "2026-04-15");
    }

    #[test]
    fn zero_baseline_pct_is_none() {
        assert_eq!(pct_delta(5.0, 0.0), None);
        assert_eq!(pct_delta(15.0, 10.0), Some(50.0));
    }

    #[test]
    fn db_compare_ranks_source_delta() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(DDL).unwrap();
        conn.execute(
            "INSERT INTO events
             (file_path, source, ts, day, month, session_id, project_path, repo, model,
              input, output, cache_read, cache_write, cost_usd)
             VALUES
             ('/a', 'codex', 1, '2026-04-02', '2026-04', 's1', NULL, NULL, 'gpt-5.4', 1, 0, 0, 0, 10.0),
             ('/b', 'codex', 1, '2026-03-02', '2026-03', 's1', NULL, NULL, 'gpt-5.4', 1, 0, 0, 0, 1.0),
             ('/c', 'claude', 1, '2026-04-02', '2026-04', 's2', NULL, NULL, 'claude-sonnet-4-6', 1, 0, 0, 0, 2.0)",
            [],
        )
        .unwrap();
        let windows = CompareWindows {
            current: window(
                "cur",
                parse_day("2026-04-01").unwrap(),
                parse_day("2026-04-30").unwrap(),
            ),
            baseline: window(
                "base",
                parse_day("2026-03-01").unwrap(),
                parse_day("2026-03-31").unwrap(),
            ),
        };
        let report = compare_from_db(
            &conn,
            windows,
            QueryFilter::default(),
            &[CompareDimension::Source],
            5,
        )
        .unwrap();
        let source = report.breakdowns.get("source").unwrap();
        assert_eq!(source.positive[0].label, "codex");
    }

    #[test]
    fn json_contains_windows() {
        let windows = CompareWindows {
            current: window(
                "cur",
                parse_day("2026-04-01").unwrap(),
                parse_day("2026-04-01").unwrap(),
            ),
            baseline: window(
                "base",
                parse_day("2026-03-01").unwrap(),
                parse_day("2026-03-01").unwrap(),
            ),
        };
        let report = build_report_from_rows(windows, &[], &[], &[CompareDimension::Source], 5);
        let json = render_json(&report);
        assert!(json.contains("\"windows\""));
    }
}
