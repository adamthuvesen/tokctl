use crate::store::queries::{comparison_rows, ComparisonRow as EventRow};
use crate::store::queries::{QueryFilter, NO_REPO_SENTINEL};
use crate::store::Store;
use crate::types::Source;
pub use crate::windows::{resolve_windows, CompareWindow, CompareWindows};
use anyhow::{Context, Result};
use comfy_table::{presets::UTF8_FULL, Cell, ContentArrangement, Row, Table};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};

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
    fn add_row(&mut self, row: &EventRow) -> Result<()> {
        let add = |left: u64, right: u64| {
            left.checked_add(right)
                .context("comparison token total exceeds supported range")
        };
        self.input = add(self.input, row.input)?;
        self.output = add(self.output, row.output)?;
        self.cache_read = add(self.cache_read, row.cache_read)?;
        self.cache_write = add(self.cache_write, row.cache_write)?;
        let total = add(
            add(row.input, row.output)?,
            add(row.cache_read, row.cache_write)?,
        )?;
        self.total_tokens = add(self.total_tokens, total)?;
        self.cost_usd += row.cost_usd;
        anyhow::ensure!(
            self.cost_usd.is_finite(),
            "comparison cost total must be finite"
        );
        Ok(())
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

pub fn compare_from_db(
    conn: &Store,
    windows: CompareWindows,
    filter: QueryFilter,
    dimensions: &[CompareDimension],
    top: usize,
) -> Result<CompareReport> {
    conn.read_snapshot(|conn| {
        let current = load_rows(conn, &windows.current, &filter)?;
        let baseline = load_rows(conn, &windows.baseline, &filter)?;
        build_report_from_rows(windows, &current, &baseline, dimensions, top)
    })
}

fn load_rows(conn: &Store, window: &CompareWindow, filter: &QueryFilter) -> Result<Vec<EventRow>> {
    comparison_rows(conn, &window.start, &window.end, filter)
}

fn build_report_from_rows(
    windows: CompareWindows,
    current_rows: &[EventRow],
    baseline_rows: &[EventRow],
    dimensions: &[CompareDimension],
    top: usize,
) -> Result<CompareReport> {
    let current_total = totals_for_rows(current_rows)?;
    let baseline_total = totals_for_rows(baseline_rows)?;
    let summary = DeltaTotals::new(current_total, baseline_total);
    let mut breakdowns = BTreeMap::new();
    for dimension in dimensions {
        breakdowns.insert(
            dimension.as_str().to_owned(),
            build_breakdown(*dimension, current_rows, baseline_rows, top)?,
        );
    }
    Ok(CompareReport {
        windows,
        summary,
        breakdowns,
    })
}

fn totals_for_rows(rows: &[EventRow]) -> Result<UsageTotals> {
    let mut totals = UsageTotals::default();
    for row in rows {
        totals.add_row(row)?;
    }
    Ok(totals)
}

fn build_breakdown(
    dimension: CompareDimension,
    current_rows: &[EventRow],
    baseline_rows: &[EventRow],
    top: usize,
) -> Result<Breakdown> {
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
            .add_row(row)?;
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
            .add_row(row)?;
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
    Ok(Breakdown {
        dimension: dimension.as_str().to_owned(),
        positive,
        negative,
    })
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
    crate::format::fmt_num(n)
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
    use crate::windows::window;

    #[test]
    fn zero_baseline_pct_is_none() {
        assert_eq!(pct_delta(5.0, 0.0), None);
        assert_eq!(pct_delta(15.0, 10.0), Some(50.0));
    }

    #[test]
    fn db_compare_ranks_source_delta() {
        let conn = Store::ephemeral().unwrap();
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
                crate::windows::parse_day("2026-04-01").unwrap(),
                crate::windows::parse_day("2026-04-30").unwrap(),
            ),
            baseline: window(
                "base",
                crate::windows::parse_day("2026-03-01").unwrap(),
                crate::windows::parse_day("2026-03-31").unwrap(),
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
                crate::windows::parse_day("2026-04-01").unwrap(),
                crate::windows::parse_day("2026-04-01").unwrap(),
            ),
            baseline: window(
                "base",
                crate::windows::parse_day("2026-03-01").unwrap(),
                crate::windows::parse_day("2026-03-01").unwrap(),
            ),
        };
        let report =
            build_report_from_rows(windows, &[], &[], &[CompareDimension::Source], 5).unwrap();
        let json = render_json(&report);
        assert!(json.contains("\"windows\""));
    }
}
