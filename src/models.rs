//! `tokctl models`: token usage and cost per model over a rolling window.

use crate::format::{fmt_compact, fmt_wide_num};
use crate::pricing;
use crate::store::queries::{model_usage, ModelUsageRow, QueryFilter};
use crate::store::Store;
use crate::types::Source;
use crate::windows::TimeWindow;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use comfy_table::modifiers::UTF8_ROUND_CORNERS;
use comfy_table::presets::UTF8_FULL_CONDENSED;
use comfy_table::{Attribute, Cell, CellAlignment, Color, ContentArrangement, Row, Table};
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelsBy {
    Model,
    Family,
}

/// Display grouping of one model line across versions. It is not a pricing
/// concept: each member keeps its own price.
///
/// - `claude-<tier>-<version>` -> `claude-<tier>` (a leading `claude-3-5-` style
///   version is skipped: `claude-3-5-haiku` -> `claude-haiku`)
/// - `gpt-<version>-<name>[-...]` -> `gpt-<name>`
/// - anything else is its own family
pub fn model_family(model: &str) -> String {
    let id = pricing::normalize_model_id(model);
    if let Some(rest) = id.strip_prefix("claude-") {
        let tier = rest
            .split('-')
            .find(|part| !part.is_empty() && !part.bytes().all(|b| b.is_ascii_digit()));
        if let Some(tier) = tier {
            return format!("claude-{tier}");
        }
    }
    if let Some(rest) = id.strip_prefix("gpt-") {
        if let Some((version, name)) = rest.split_once('-') {
            let name = name.split('-').next().unwrap_or_default();
            if version.starts_with(|c: char| c.is_ascii_digit()) && !name.is_empty() {
                return format!("gpt-{name}");
            }
        }
    }
    id.to_owned()
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub cost_usd: f64,
    pub events: u64,
    /// Cache reads and prompt tokens of rows whose source reports cache data.
    /// Cursor exports carry none, so they stay out of the cache-hit ratio.
    cache_read_measured: u64,
    prompt_measured: u64,
}

impl Usage {
    fn add(&mut self, row: &ModelUsageRow) -> Result<()> {
        let add = |left: u64, right: u64| {
            left.checked_add(right)
                .context("model token total exceeds supported range")
        };
        self.input = add(self.input, row.input)?;
        self.output = add(self.output, row.output)?;
        self.cache_read = add(self.cache_read, row.cache_read)?;
        self.cache_write = add(self.cache_write, row.cache_write)?;
        self.cost_usd += row.cost_usd;
        self.events = add(self.events, row.events)?;
        if row.source != Source::Cursor {
            self.cache_read_measured = add(self.cache_read_measured, row.cache_read)?;
            let prompt = add(add(row.input, row.cache_read)?, row.cache_write)?;
            self.prompt_measured = add(self.prompt_measured, prompt)?;
        }
        Ok(())
    }

    pub fn total_tokens(&self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_write
    }

    /// Share of prompt tokens served from cache, over Claude and Codex rows only;
    /// `None` without such prompt tokens.
    pub fn cache_hit(&self) -> Option<f64> {
        (self.prompt_measured > 0)
            .then(|| self.cache_read_measured as f64 / self.prompt_measured as f64)
    }
}

#[derive(Debug, Clone)]
pub struct ModelRow {
    /// `None` when rows of every source merge into one.
    pub source: Option<Source>,
    pub model: String,
    /// Raw model ids behind this row, sorted.
    pub members: Vec<String>,
    pub latest_ts_ms: i64,
    pub usage: Usage,
    /// False when no member model has a price; its cost is unknown, not zero.
    pub priced: bool,
    /// Fraction of the period's cost; 0 when the period has no cost.
    pub share: f64,
}

#[derive(Debug, Clone)]
pub struct UnpricedModel {
    pub model: String,
    pub tokens: u64,
}

#[derive(Debug, Clone)]
pub struct ModelsReport {
    pub window: TimeWindow,
    pub since_ms: Option<i64>,
    pub rows: Vec<ModelRow>,
    pub total: Usage,
    /// The window of the same length just before this one; `None` for `all` and when it
    /// holds no events.
    pub previous_total: Option<Usage>,
    pub unpriced: Vec<UnpricedModel>,
}

fn sum_usage(rows: &[ModelUsageRow]) -> Result<Usage> {
    let mut total = Usage::default();
    for row in rows {
        total.add(row)?;
    }
    Ok(total)
}

/// Cursor cost is explicit in its exports, so only Claude and Codex rows can lack a price.
fn is_unpriced(row: &ModelUsageRow) -> bool {
    row.source != Source::Cursor && !pricing::has_price(&row.model)
}

/// `base` carries no time bounds: `window` sets them, and the previous window derives from it.
/// `split_sources` keeps one row per (source, model); otherwise a model merges across sources.
pub fn models_report(
    store: &Store,
    base: QueryFilter,
    window: TimeWindow,
    by: ModelsBy,
    split_sources: bool,
    now: DateTime<Utc>,
) -> Result<ModelsReport> {
    store.read_snapshot(|store| {
        let since_ms = window.since_ms(now)?;
        let mut current = base.clone();
        current.intersect_time(since_ms, None);
        let current_rows = model_usage(store, current)?;

        let previous_total = match window.previous_bounds_ms(now)? {
            Some((start, end)) => {
                let mut previous = base;
                previous.intersect_time(Some(start), Some(end));
                Some(sum_usage(&model_usage(store, previous)?)?).filter(|usage| usage.events > 0)
            }
            None => None,
        };

        let total = sum_usage(&current_rows)?;
        let mut groups: BTreeMap<(Option<&'static str>, String), (ModelRow, usize, usize)> =
            BTreeMap::new();
        let mut unpriced: BTreeMap<String, u64> = BTreeMap::new();
        for row in &current_rows {
            let name = match by {
                ModelsBy::Model => pricing::normalize_model_id(&row.model).to_owned(),
                ModelsBy::Family => model_family(&row.model),
            };
            let (group, members, unpriced_members) = groups
                .entry((split_sources.then(|| row.source.as_str()), name.clone()))
                .or_insert_with(|| {
                    let empty = ModelRow {
                        source: split_sources.then_some(row.source),
                        model: name,
                        members: Vec::new(),
                        latest_ts_ms: row.latest_ts_ms,
                        usage: Usage::default(),
                        priced: true,
                        share: 0.0,
                    };
                    (empty, 0, 0)
                });
            group.usage.add(row)?;
            group.latest_ts_ms = group.latest_ts_ms.max(row.latest_ts_ms);
            if !group.members.contains(&row.model) {
                group.members.push(row.model.clone());
            }
            *members += 1;
            if is_unpriced(row) {
                *unpriced_members += 1;
                let tokens = unpriced
                    .entry(pricing::normalize_model_id(&row.model).to_owned())
                    .or_default();
                *tokens += row.input + row.output + row.cache_read + row.cache_write;
            }
        }

        let mut rows: Vec<ModelRow> = groups
            .into_values()
            .map(|(mut row, members, unpriced_members)| {
                row.members.sort();
                row.priced = unpriced_members < members;
                row.share = if total.cost_usd > 0.0 {
                    row.usage.cost_usd / total.cost_usd
                } else {
                    0.0
                };
                row
            })
            .collect();
        rows.sort_by(|a, b| {
            b.usage
                .cost_usd
                .total_cmp(&a.usage.cost_usd)
                .then(b.usage.total_tokens().cmp(&a.usage.total_tokens()))
                .then_with(|| a.model.cmp(&b.model))
                .then_with(|| {
                    a.source
                        .map(Source::as_str)
                        .cmp(&b.source.map(Source::as_str))
                })
        });

        let mut unpriced: Vec<UnpricedModel> = unpriced
            .into_iter()
            .map(|(model, tokens)| UnpricedModel { model, tokens })
            .collect();
        unpriced.sort_by(|a, b| b.tokens.cmp(&a.tokens).then_with(|| a.model.cmp(&b.model)));

        Ok(ModelsReport {
            window,
            since_ms,
            rows,
            total,
            previous_total,
            unpriced,
        })
    })
}

const BAR_CELLS: usize = 10;
const PARTIAL_BLOCKS: [char; 7] = ['▏', '▎', '▍', '▌', '▋', '▊', '▉'];

/// A left-aligned bar of `BAR_CELLS` cells in eighths of a cell, padded with spaces.
fn share_bar(share: f64) -> String {
    let eighths = (share.clamp(0.0, 1.0) * (BAR_CELLS * 8) as f64).round() as usize;
    let (full, rest) = (eighths / 8, eighths % 8);
    let mut bar = "█".repeat(full);
    if rest > 0 {
        bar.push(PARTIAL_BLOCKS[rest - 1]);
    }
    let used = full + usize::from(rest > 0);
    bar.push_str(&" ".repeat(BAR_CELLS - used));
    bar
}

fn fmt_cost(n: f64) -> String {
    let cents = (n * 100.0).round() as u128;
    format!("${}.{:02}", fmt_wide_num(cents / 100), cents % 100)
}

fn fmt_percent(fraction: Option<f64>) -> String {
    fraction.map_or_else(|| "-".into(), |f| format!("{:.1}%", f * 100.0))
}

fn right(text: impl ToString) -> Cell {
    Cell::new(text.to_string()).set_alignment(CellAlignment::Right)
}

fn usage_cells(usage: &Usage, cost: String) -> Vec<Cell> {
    vec![
        right(fmt_compact(usage.input)),
        right(fmt_compact(usage.output)),
        right(fmt_compact(usage.cache_read)),
        right(fmt_compact(usage.cache_write)),
        right(fmt_compact(usage.total_tokens())),
        right(cost),
    ]
}

pub fn render_human(report: &ModelsReport) -> String {
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL_CONDENSED)
        .apply_modifier(UTF8_ROUND_CORNERS)
        .set_content_arrangement(ContentArrangement::Dynamic);
    let headers = [
        "source",
        "model",
        "input",
        "output",
        "cache read",
        "cache write",
        "total",
        "cost",
        "share of cost",
        "cache hit",
    ];
    table.set_header(headers.iter().enumerate().map(|(i, name)| {
        let cell = Cell::new(*name).add_attribute(Attribute::Bold);
        if i >= 2 && i != 8 {
            cell.set_alignment(CellAlignment::Right)
        } else {
            cell
        }
    }));

    for row in &report.rows {
        let mut cells = vec![
            Cell::new(row.source.map_or("all", Source::as_str)),
            Cell::new(&row.model),
        ];
        let cost = if row.priced {
            fmt_cost(row.usage.cost_usd)
        } else {
            "unpriced".into()
        };
        cells.extend(usage_cells(&row.usage, cost));
        if !row.priced {
            cells[7] = cells[7].clone().fg(Color::Yellow);
        }
        cells.push(if row.priced && report.total.cost_usd > 0.0 {
            Cell::new(format!(
                "{:>5.1}% {}",
                row.share * 100.0,
                share_bar(row.share)
            ))
            .fg(Color::Cyan)
        } else {
            Cell::new("    -")
        });
        cells.push(right(fmt_percent(row.usage.cache_hit())));
        table.add_row(Row::from(cells));
    }

    if !report.rows.is_empty() {
        let mut cells = vec![Cell::new("TOTAL"), Cell::new("")];
        cells.extend(usage_cells(&report.total, fmt_cost(report.total.cost_usd)));
        cells.push(Cell::new(""));
        cells.push(right(fmt_percent(report.total.cache_hit())));
        table.add_row(Row::from(
            cells
                .into_iter()
                .map(|c| c.add_attribute(Attribute::Bold))
                .collect::<Vec<_>>(),
        ));
    }

    let mut out = table.to_string();
    if report.rows.is_empty() {
        out.push_str(&format!(
            "\nNo usage in the last {}.",
            report.window.as_str()
        ));
    }
    if let Some(previous) = &report.previous_total {
        out.push('\n');
        out.push_str(&period_line(report.window, &report.total, previous));
    } else if report.window != TimeWindow::All {
        out.push_str(&format!("\nprevious {}: no data", report.window.as_str()));
    }
    if !report.unpriced.is_empty() {
        let list = report
            .unpriced
            .iter()
            .map(|m| format!("{} ({} tokens)", m.model, fmt_compact(m.tokens)))
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!(
            "\nwarning: no price for {list}; cost excludes them"
        ));
    }
    out
}

/// Cost change against the previous window as `+12.3%`; `n/a` without a previous cost.
pub fn signed_percent(current: f64, previous: f64) -> String {
    if previous > 0.0 {
        let delta = current - previous;
        let sign = if delta < 0.0 { "-" } else { "+" };
        format!("{sign}{:.1}%", delta.abs() / previous * 100.0)
    } else {
        "n/a".into()
    }
}

fn period_line(window: TimeWindow, total: &Usage, previous: &Usage) -> String {
    let delta = total.cost_usd - previous.cost_usd;
    let sign = if delta < 0.0 { "-" } else { "+" };
    let percent = signed_percent(total.cost_usd, previous.cost_usd);
    format!(
        "vs previous {}: {} -> {} ({sign}{}, {percent})",
        window.as_str(),
        fmt_cost(previous.cost_usd),
        fmt_cost(total.cost_usd),
        fmt_cost(delta.abs()),
    )
}

fn usage_json(usage: &Usage) -> Value {
    json!({
        "input": usage.input,
        "output": usage.output,
        "cache_read": usage.cache_read,
        "cache_write": usage.cache_write,
        "totalTokens": usage.total_tokens(),
        "costUsd": round_cost(usage.cost_usd),
        "cache_hit": usage.cache_hit(),
    })
}

fn round_cost(cost: f64) -> f64 {
    (cost * 10_000.0).round() / 10_000.0
}

pub fn render_json(report: &ModelsReport) -> String {
    let rows: Vec<Value> = report
        .rows
        .iter()
        .map(|row| {
            let mut value = usage_json(&row.usage);
            let object = value.as_object_mut().expect("usage_json builds an object");
            object.insert("source".into(), json!(row.source.map(Source::as_str)));
            object.insert("model".into(), json!(row.model));
            object.insert("share".into(), json!(row.share));
            object.insert("priced".into(), json!(row.priced));
            value
        })
        .collect();
    let unpriced: Vec<Value> = report
        .unpriced
        .iter()
        .map(|m| json!({ "model": m.model, "tokens": m.tokens }))
        .collect();
    let body = json!({
        "window": report.window.as_str(),
        "since_ms": report.since_ms,
        "rows": rows,
        "total": usage_json(&report.total),
        "previous_total": report.previous_total.as_ref().map(usage_json),
        "unpriced": unpriced,
    });
    serde_json::to_string_pretty(&body).unwrap_or_else(|_| "{}".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY_MS: i64 = 86_400_000;

    fn now() -> DateTime<Utc> {
        "2026-04-22T12:00:00Z".parse().unwrap()
    }

    /// Whole days before the fixed `now()`; noon keeps every case clear of local midnight.
    fn days_ago(days: i64) -> i64 {
        now().timestamp_millis() - days * DAY_MS
    }

    #[allow(clippy::too_many_arguments)]
    fn insert(
        store: &Store,
        id: &str,
        source: &str,
        model: &str,
        days: i64,
        input: u64,
        cache_read: u64,
        cost: f64,
    ) {
        store
            .execute(
                "INSERT INTO events
                 (file_path, source, ts, day, month, session_id, project_path, repo, model,
                  input, output, cache_read, cache_write, cost_usd)
                 VALUES (?1, ?2, ?3, '2026-04-01', '2026-04', ?1, NULL, NULL, ?4, ?5, 0, ?6, 0, ?7)",
                rusqlite::params![
                    id,
                    source,
                    days_ago(days),
                    model,
                    input as i64,
                    cache_read as i64,
                    cost
                ],
            )
            .unwrap();
    }

    fn report(store: &Store, window: TimeWindow, by: ModelsBy) -> ModelsReport {
        models_report(store, QueryFilter::default(), window, by, true, now()).unwrap()
    }

    #[test]
    fn families_group_versions_of_the_priced_model_ids() {
        let family = |id: &str| model_family(id);
        assert_eq!(family("claude-opus-5-5"), "claude-opus");
        assert_eq!(family("claude-opus-5"), "claude-opus");
        assert_eq!(family("claude-sonnet-4-6-20250101"), "claude-sonnet");
        assert_eq!(family("claude-fable-5-2"), "claude-fable");
        assert_eq!(family("claude-3-5-haiku"), "claude-haiku");
        assert_eq!(family("claude-3-opus"), "claude-opus");
        assert_eq!(family("claude-opus-5-5[1m]"), "claude-opus");
        assert_eq!(family("gpt-6.1-sol"), "gpt-sol");
        assert_eq!(family("gpt-6-sol"), "gpt-sol");
        assert_eq!(family("gpt-5.3-codex"), "gpt-codex");
        assert_eq!(family("gpt-5-codex-mini"), "gpt-codex");
        assert_eq!(family("gpt-5.1-codex-max"), "gpt-codex");
        assert_eq!(family("gpt-5.4-mini"), "gpt-mini");
        // No name part, or not a known shape: the family is the id itself.
        assert_eq!(family("gpt-5.4"), "gpt-5.4");
        assert_eq!(family("composer-2.5"), "composer-2.5");
    }

    #[test]
    fn rows_sort_by_cost_and_shares_sum_to_one() {
        let store = Store::ephemeral().unwrap();
        insert(&store, "a", "claude", "claude-sonnet-4-6", 1, 100, 0, 1.0);
        insert(&store, "b", "codex", "gpt-5.4", 1, 100, 0, 6.0);
        insert(&store, "c", "claude", "claude-opus-4-7", 2, 100, 0, 3.0);
        insert(&store, "d", "claude", "claude-opus-4-7[1m]", 3, 100, 0, 1.0);
        insert(
            &store,
            "e",
            "claude",
            "claude-opus-4-7-20260101",
            3,
            100,
            0,
            1.0,
        );
        let report = report(&store, TimeWindow::D7, ModelsBy::Model);

        let order: Vec<&str> = report.rows.iter().map(|r| r.model.as_str()).collect();
        assert_eq!(order, ["gpt-5.4", "claude-opus-4-7", "claude-sonnet-4-6"]);
        assert_eq!(report.rows[1].usage.input, 300, "suffixed ids join one row");
        let shares: f64 = report.rows.iter().map(|r| r.share).sum();
        assert!((shares - 1.0).abs() < 1e-9, "{shares}");
        assert!((report.rows[1].share - 5.0 / 12.0).abs() < 1e-9);
    }

    #[test]
    fn family_grouping_merges_versions_within_a_source() {
        let store = Store::ephemeral().unwrap();
        insert(&store, "a", "claude", "claude-opus-5-5", 1, 100, 0, 2.0);
        insert(&store, "b", "claude", "claude-opus-5", 1, 50, 0, 1.0);
        insert(&store, "c", "claude", "claude-sonnet-5", 1, 10, 0, 0.5);
        let report = report(&store, TimeWindow::D7, ModelsBy::Family);

        assert_eq!(report.rows.len(), 2);
        assert_eq!(report.rows[0].model, "claude-opus");
        assert_eq!(report.rows[0].usage.input, 150);
        assert!((report.rows[0].usage.cost_usd - 3.0).abs() < 1e-9);
    }

    #[test]
    fn unpriced_model_is_reported_not_costed_as_zero() {
        let store = Store::ephemeral().unwrap();
        insert(&store, "a", "claude", "claude-sonnet-4-6", 1, 100, 0, 1.0);
        insert(&store, "b", "codex", "gpt-99-ultra", 1, 700, 0, 0.0);
        // Cursor carries explicit costs, so an unknown Cursor model is not unpriced.
        insert(&store, "c", "cursor", "composer-9", 1, 5, 0, 0.4);
        let report = report(&store, TimeWindow::D7, ModelsBy::Model);

        let unpriced: Vec<(&str, u64)> = report
            .unpriced
            .iter()
            .map(|m| (m.model.as_str(), m.tokens))
            .collect();
        assert_eq!(unpriced, [("gpt-99-ultra", 700)]);
        let ultra = report
            .rows
            .iter()
            .find(|r| r.model == "gpt-99-ultra")
            .unwrap();
        assert!(!ultra.priced);
        let text = render_human(&report);
        assert!(text.contains("unpriced"), "{text}");
        assert!(
            text.contains("no price for gpt-99-ultra (700 tokens)"),
            "{text}"
        );
        assert!(
            report
                .rows
                .iter()
                .find(|r| r.model == "composer-9")
                .unwrap()
                .priced
        );
    }

    #[test]
    fn previous_total_covers_the_window_just_before() {
        let store = Store::ephemeral().unwrap();
        insert(&store, "now", "claude", "claude-sonnet-4-6", 1, 10, 0, 4.0);
        insert(
            &store,
            "prev",
            "claude",
            "claude-sonnet-4-6",
            10,
            10,
            0,
            2.0,
        );
        insert(
            &store,
            "older",
            "claude",
            "claude-sonnet-4-6",
            20,
            10,
            0,
            100.0,
        );
        let report = report(&store, TimeWindow::D7, ModelsBy::Model);

        assert!((report.total.cost_usd - 4.0).abs() < 1e-9);
        let previous = report.previous_total.as_ref().unwrap();
        assert!((previous.cost_usd - 2.0).abs() < 1e-9);
        assert!(render_human(&report).contains("vs previous 7d: $2.00 -> $4.00 (+$2.00, +100.0%)"));

        let all = self::report(&store, TimeWindow::All, ModelsBy::Model);
        assert!(all.previous_total.is_none());
        assert!(!render_human(&all).contains("vs previous"));
        assert!(render_json(&all).contains("\"previous_total\": null"));
    }

    #[test]
    fn empty_previous_window_reports_no_data_not_zero() {
        let store = Store::ephemeral().unwrap();
        insert(&store, "now", "claude", "claude-sonnet-4-6", 1, 10, 0, 4.0);
        let report = report(&store, TimeWindow::D7, ModelsBy::Model);
        assert!(report.previous_total.is_none());
        let text = render_human(&report);
        assert!(text.contains("previous 7d: no data"));
        assert!(!text.contains("vs previous"));
        assert!(render_json(&report).contains("\"previous_total\": null"));
    }

    #[test]
    fn cache_hit_is_none_without_prompt_tokens() {
        let none = Usage::default();
        assert_eq!(none.cache_hit(), None);
        let mut some = Usage::default();
        let mut add = |source, input, cache_read| {
            let row = ModelUsageRow {
                source,
                model: "m".into(),
                input,
                output: 0,
                cache_read,
                cache_write: 0,
                cost_usd: 0.0,
                latest_ts_ms: 0,
                events: 1,
            };
            some.add(&row).unwrap();
        };
        add(Source::Claude, 25, 75);
        add(Source::Cursor, 900, 0);
        assert_eq!(some.cache_hit(), Some(0.75));
        let mut cursor_only = Usage::default();
        let row = ModelUsageRow {
            source: Source::Cursor,
            model: "m".into(),
            input: 5,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            cost_usd: 1.0,
            latest_ts_ms: 0,
            events: 1,
        };
        cursor_only.add(&row).unwrap();
        assert_eq!(cursor_only.cache_hit(), None);
    }

    #[test]
    fn share_bar_has_fixed_width() {
        for share in [0.0, 0.013, 0.5, 0.999, 1.0] {
            assert_eq!(share_bar(share).chars().count(), BAR_CELLS, "{share}");
        }
        assert_eq!(share_bar(1.0), "█".repeat(BAR_CELLS));
    }
}
