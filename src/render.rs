use crate::store::queries::RepoAggregateRow;
use crate::types::{AggregateRow, ReportKind};
use comfy_table::{presets::UTF8_FULL, Cell, ContentArrangement, Row, Table};
use serde_json::{json, Map, Value};
use std::collections::HashSet;

pub fn fmt_num(n: u64) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}

fn fmt_cost(n: f64) -> String {
    format!("${n:.2}")
}

fn fmt_timestamp(ts: chrono::DateTime<chrono::Utc>) -> String {
    use chrono::Local;
    ts.with_timezone(&Local)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

pub fn render_table(rows: &[AggregateRow], kind: ReportKind, show_source: bool) -> String {
    let key_header = match kind {
        ReportKind::Daily => "date",
        ReportKind::Monthly => "month",
        ReportKind::Session => "session",
    };

    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .set_content_arrangement(ContentArrangement::Dynamic);

    let head: Vec<&str> = match kind {
        ReportKind::Session => vec![
            "session",
            "source",
            "project",
            "last_activity",
            "input",
            "output",
            "cache_read",
            "cache_write",
            "total",
            "cost_usd",
        ],
        _ => {
            if show_source {
                vec![
                    key_header,
                    "source",
                    "input",
                    "output",
                    "cache_read",
                    "cache_write",
                    "total",
                    "cost_usd",
                ]
            } else {
                vec![
                    key_header,
                    "input",
                    "output",
                    "cache_read",
                    "cache_write",
                    "total",
                    "cost_usd",
                ]
            }
        }
    };
    table.set_header(head.iter().map(|s| Cell::new(*s)));

    for r in rows {
        let cells: Vec<Cell> = match kind {
            ReportKind::Session => {
                let sess = r.key.chars().take(8).collect::<String>();
                let last = r.latest_timestamp.map(fmt_timestamp).unwrap_or_default();
                vec![
                    Cell::new(sess),
                    Cell::new(r.source.as_str()),
                    Cell::new(r.project_path.clone().unwrap_or_default()),
                    Cell::new(last),
                    Cell::new(fmt_num(r.input_tokens)),
                    Cell::new(fmt_num(r.output_tokens)),
                    Cell::new(fmt_num(r.cache_read_tokens)),
                    Cell::new(fmt_num(r.cache_write_tokens)),
                    Cell::new(fmt_num(r.total_tokens)),
                    Cell::new(fmt_cost(r.cost_usd)),
                ]
            }
            _ if show_source => vec![
                Cell::new(&r.key),
                Cell::new(r.source.as_str()),
                Cell::new(fmt_num(r.input_tokens)),
                Cell::new(fmt_num(r.output_tokens)),
                Cell::new(fmt_num(r.cache_read_tokens)),
                Cell::new(fmt_num(r.cache_write_tokens)),
                Cell::new(fmt_num(r.total_tokens)),
                Cell::new(fmt_cost(r.cost_usd)),
            ],
            _ => vec![
                Cell::new(&r.key),
                Cell::new(fmt_num(r.input_tokens)),
                Cell::new(fmt_num(r.output_tokens)),
                Cell::new(fmt_num(r.cache_read_tokens)),
                Cell::new(fmt_num(r.cache_write_tokens)),
                Cell::new(fmt_num(r.total_tokens)),
                Cell::new(fmt_cost(r.cost_usd)),
            ],
        };
        table.add_row(Row::from(cells));
    }

    if !rows.is_empty() {
        let mut totals = (0u64, 0u64, 0u64, 0u64, 0u64, 0f64);
        for r in rows {
            totals.0 += r.input_tokens;
            totals.1 += r.output_tokens;
            totals.2 += r.cache_read_tokens;
            totals.3 += r.cache_write_tokens;
            totals.4 += r.total_tokens;
            totals.5 += r.cost_usd;
        }
        let total_cells: Vec<Cell> = match kind {
            ReportKind::Session => vec![
                Cell::new("TOTAL"),
                Cell::new(""),
                Cell::new(""),
                Cell::new(""),
                Cell::new(fmt_num(totals.0)),
                Cell::new(fmt_num(totals.1)),
                Cell::new(fmt_num(totals.2)),
                Cell::new(fmt_num(totals.3)),
                Cell::new(fmt_num(totals.4)),
                Cell::new(fmt_cost(totals.5)),
            ],
            _ if show_source => vec![
                Cell::new("TOTAL"),
                Cell::new(""),
                Cell::new(fmt_num(totals.0)),
                Cell::new(fmt_num(totals.1)),
                Cell::new(fmt_num(totals.2)),
                Cell::new(fmt_num(totals.3)),
                Cell::new(fmt_num(totals.4)),
                Cell::new(fmt_cost(totals.5)),
            ],
            _ => vec![
                Cell::new("TOTAL"),
                Cell::new(fmt_num(totals.0)),
                Cell::new(fmt_num(totals.1)),
                Cell::new(fmt_num(totals.2)),
                Cell::new(fmt_num(totals.3)),
                Cell::new(fmt_num(totals.4)),
                Cell::new(fmt_cost(totals.5)),
            ],
        };
        table.add_row(Row::from(total_cells));
    }

    table.to_string()
}

pub fn render_json(rows: &[AggregateRow], kind: ReportKind, show_source: bool) -> String {
    let arr: Vec<Value> = rows
        .iter()
        .map(|r| row_to_json(r, kind, show_source))
        .collect();
    serde_json::to_string_pretty(&Value::Array(arr)).unwrap_or_else(|_| "[]".into())
}

fn row_to_json(r: &AggregateRow, kind: ReportKind, show_source: bool) -> Value {
    let mut obj = Map::new();
    let key_name = match kind {
        ReportKind::Daily => "date",
        ReportKind::Monthly => "month",
        ReportKind::Session => "session_id",
    };
    obj.insert(key_name.into(), Value::String(r.key.clone()));
    match kind {
        ReportKind::Session => {
            obj.insert("source".into(), Value::String(r.source.as_str().into()));
            obj.insert(
                "project_path".into(),
                r.project_path
                    .as_ref()
                    .map(|p| Value::String(p.clone()))
                    .unwrap_or(Value::Null),
            );
            obj.insert(
                "latest_timestamp".into(),
                r.latest_timestamp
                    .map(|t| Value::String(t.to_rfc3339()))
                    .unwrap_or(Value::Null),
            );
        }
        _ if show_source => {
            obj.insert("source".into(), Value::String(r.source.as_str().into()));
        }
        _ => {}
    }
    obj.insert("input".into(), json!(r.input_tokens));
    obj.insert("output".into(), json!(r.output_tokens));
    obj.insert("cache_read".into(), json!(r.cache_read_tokens));
    obj.insert("cache_write".into(), json!(r.cache_write_tokens));
    obj.insert("totalTokens".into(), json!(r.total_tokens));
    obj.insert(
        "costUsd".into(),
        json!((r.cost_usd * 10_000.0).round() / 10_000.0),
    );
    Value::Object(obj)
}

pub fn render_repo_table(rows: &[RepoAggregateRow]) -> String {
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .set_content_arrangement(ContentArrangement::Dynamic);
    table.set_header(
        [
            "repo",
            "sessions",
            "input",
            "output",
            "cache_read",
            "cache_write",
            "total",
            "cost_usd",
        ]
        .iter()
        .map(|s| Cell::new(*s)),
    );

    // Sort so that `(no-repo)` sinks to the bottom regardless of its cost.
    let mut sorted: Vec<&RepoAggregateRow> = rows.iter().collect();
    sorted.sort_by(|a, b| match (a.is_no_repo(), b.is_no_repo()) {
        (true, false) => std::cmp::Ordering::Greater,
        (false, true) => std::cmp::Ordering::Less,
        _ => b
            .cost_usd
            .partial_cmp(&a.cost_usd)
            .unwrap_or(std::cmp::Ordering::Equal),
    });

    for r in &sorted {
        table.add_row(Row::from(vec![
            Cell::new(&r.display_name),
            Cell::new(fmt_num(r.sessions)),
            Cell::new(fmt_num(r.input_tokens)),
            Cell::new(fmt_num(r.output_tokens)),
            Cell::new(fmt_num(r.cache_read_tokens)),
            Cell::new(fmt_num(r.cache_write_tokens)),
            Cell::new(fmt_num(r.total_tokens)),
            Cell::new(fmt_cost(r.cost_usd)),
        ]));
    }

    if !sorted.is_empty() {
        let mut totals = (0u64, 0u64, 0u64, 0u64, 0u64, 0u64, 0f64);
        for r in &sorted {
            totals.0 += r.sessions;
            totals.1 += r.input_tokens;
            totals.2 += r.output_tokens;
            totals.3 += r.cache_read_tokens;
            totals.4 += r.cache_write_tokens;
            totals.5 += r.total_tokens;
            totals.6 += r.cost_usd;
        }
        table.add_row(Row::from(vec![
            Cell::new("TOTAL"),
            Cell::new(fmt_num(totals.0)),
            Cell::new(fmt_num(totals.1)),
            Cell::new(fmt_num(totals.2)),
            Cell::new(fmt_num(totals.3)),
            Cell::new(fmt_num(totals.4)),
            Cell::new(fmt_num(totals.5)),
            Cell::new(fmt_cost(totals.6)),
        ]));
    }

    table.to_string()
}

pub fn render_repo_json(rows: &[RepoAggregateRow]) -> String {
    let mut sorted: Vec<&RepoAggregateRow> = rows.iter().collect();
    sorted.sort_by(|a, b| match (a.is_no_repo(), b.is_no_repo()) {
        (true, false) => std::cmp::Ordering::Greater,
        (false, true) => std::cmp::Ordering::Less,
        _ => b
            .cost_usd
            .partial_cmp(&a.cost_usd)
            .unwrap_or(std::cmp::Ordering::Equal),
    });
    let arr: Vec<Value> = sorted
        .iter()
        .map(|r| {
            let mut obj = Map::new();
            obj.insert("repo".into(), Value::String(r.display_name.clone()));
            obj.insert(
                "key".into(),
                if r.is_no_repo() {
                    Value::Null
                } else {
                    Value::String(r.key.clone())
                },
            );
            obj.insert(
                "origin_url".into(),
                r.origin_url
                    .as_ref()
                    .map(|s| Value::String(s.clone()))
                    .unwrap_or(Value::Null),
            );
            obj.insert("sessions".into(), json!(r.sessions));
            obj.insert("input".into(), json!(r.input_tokens));
            obj.insert("output".into(), json!(r.output_tokens));
            obj.insert("cache_read".into(), json!(r.cache_read_tokens));
            obj.insert("cache_write".into(), json!(r.cache_write_tokens));
            obj.insert("totalTokens".into(), json!(r.total_tokens));
            obj.insert(
                "costUsd".into(),
                json!((r.cost_usd * 10_000.0).round() / 10_000.0),
            );
            Value::Object(obj)
        })
        .collect();
    serde_json::to_string_pretty(&Value::Array(arr)).unwrap_or_else(|_| "[]".into())
}

pub fn render_warnings(unknown_models: &HashSet<String>, skipped_lines: usize) -> Vec<String> {
    let mut out = Vec::new();
    if !unknown_models.is_empty() {
        let mut list: Vec<&String> = unknown_models.iter().collect();
        list.sort();
        let joined = list
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        out.push(format!(
            "warning: no price for model(s): {joined} (cost treated as 0)"
        ));
    }
    if skipped_lines > 0 {
        out.push(format!(
            "warning: skipped {skipped_lines} malformed JSONL line(s)"
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Source, SourceLabel};
    use chrono::Utc;

    fn aggr(key: &str) -> AggregateRow {
        AggregateRow {
            key: key.into(),
            source: SourceLabel::All,
            project_path: None,
            latest_timestamp: None,
            input_tokens: 100,
            output_tokens: 40,
            cache_read_tokens: 500,
            cache_write_tokens: 200,
            total_tokens: 840,
            cost_usd: 1.23456789,
        }
    }

    #[test]
    fn fmt_num_adds_thousands_separators() {
        assert_eq!(fmt_num(1_234_567), "1,234,567");
        assert_eq!(fmt_num(0), "0");
        assert_eq!(fmt_num(999), "999");
    }

    #[test]
    fn json_has_correct_keys_for_daily() {
        let rows = vec![aggr("2026-04-18")];
        let out = render_json(&rows, ReportKind::Daily, false);
        assert!(out.contains("\"date\""));
        assert!(out.contains("\"costUsd\": 1.2346"));
    }

    #[test]
    fn json_session_includes_metadata() {
        let mut r = aggr("sess-a");
        r.source = SourceLabel::Source(Source::Claude);
        r.project_path = Some("/p".into());
        r.latest_timestamp = Some(Utc::now());
        let out = render_json(&[r], ReportKind::Session, false);
        assert!(out.contains("\"session_id\""));
        assert!(out.contains("\"project_path\""));
        assert!(out.contains("\"latest_timestamp\""));
    }

    #[test]
    fn empty_rows_has_no_total() {
        let t = render_table(&[], ReportKind::Daily, false);
        assert!(!t.contains("TOTAL"));
    }

    #[test]
    fn non_empty_has_total() {
        let t = render_table(&[aggr("2026-04-18")], ReportKind::Daily, false);
        assert!(t.contains("TOTAL"));
    }

    #[test]
    fn warnings_sorted_and_formatted() {
        let mut set = HashSet::new();
        set.insert("zebra".into());
        set.insert("apple".into());
        let w = render_warnings(&set, 3);
        assert!(w[0].contains("apple, zebra"));
        assert!(w[1].contains("3 malformed"));
    }
}
