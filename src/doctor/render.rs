use comfy_table::{presets::UTF8_FULL, Cell, ContentArrangement, Row, Table};
use std::collections::BTreeMap;

use super::{DoctorCheck, DoctorReport};

pub fn render_human(report: &DoctorReport) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "tokctl doctor: {}\nchecks: {} ok, {} warn, {} error\ncache: {}\n\n",
        report.status.as_str(),
        report.check_counts.ok,
        report.check_counts.warn,
        report.check_counts.error,
        report.summary.cache_path
    ));

    let mut grouped: BTreeMap<&str, Vec<&DoctorCheck>> = BTreeMap::new();
    for check in &report.checks {
        grouped.entry(&check.category).or_default().push(check);
    }

    for (category, checks) in grouped {
        let mut table = Table::new();
        table
            .load_preset(UTF8_FULL)
            .set_content_arrangement(ContentArrangement::Dynamic);
        table.set_header(["status", category, "action"].iter().map(|s| Cell::new(*s)));
        for check in checks {
            let mut message = check.message.clone();
            if !check.details.is_empty() {
                message.push('\n');
                message.push_str(&check.details.join("\n"));
            }
            table.add_row(Row::from(vec![
                Cell::new(check.severity.as_str()),
                Cell::new(message),
                Cell::new(check.action.clone().unwrap_or_default()),
            ]));
        }
        out.push_str(&table.to_string());
        out.push('\n');
    }
    out
}

pub fn render_json(report: &DoctorReport) -> String {
    serde_json::to_string_pretty(report).unwrap_or_else(|_| "{}".into())
}
