use crate::ingest::run::{local_day, local_month};
use crate::pricing::cost_of;
use crate::types::{AggregateRow, Source, SourceLabel, UsageEvent};
use chrono::{DateTime, Utc};
use std::collections::{HashMap, HashSet};

pub fn filter_by_date(
    events: &[UsageEvent],
    since: Option<DateTime<Utc>>,
    until: Option<DateTime<Utc>>,
) -> Vec<UsageEvent> {
    events
        .iter()
        .filter(|e| {
            if let Some(s) = since {
                if e.timestamp < s {
                    return false;
                }
            }
            if let Some(u) = until {
                if e.timestamp > u {
                    return false;
                }
            }
            true
        })
        .cloned()
        .collect()
}

fn aggregate_by<K: Fn(&UsageEvent) -> String>(
    events: &[UsageEvent],
    key_for: K,
    label: SourceLabel,
    unknown: &mut HashSet<String>,
) -> Vec<AggregateRow> {
    let mut map: HashMap<String, AggregateRow> = HashMap::new();
    for e in events {
        let key = key_for(e);
        let row = map.entry(key.clone()).or_insert_with(|| AggregateRow {
            key,
            source: label,
            project_path: None,
            latest_timestamp: None,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            total_tokens: 0,
            cost_usd: 0.0,
        });
        row.input_tokens += e.input_tokens;
        row.output_tokens += e.output_tokens;
        row.cache_read_tokens += e.cache_read_tokens;
        row.cache_write_tokens += e.cache_write_tokens;
        row.total_tokens +=
            e.input_tokens + e.output_tokens + e.cache_read_tokens + e.cache_write_tokens;
        row.cost_usd += cost_of(e, Some(unknown));
    }
    let mut rows: Vec<AggregateRow> = map.into_values().collect();
    rows.sort_by(|a, b| a.key.cmp(&b.key));
    rows
}

pub fn daily_in_memory(
    events: &[UsageEvent],
    label: SourceLabel,
    unknown: &mut HashSet<String>,
) -> Vec<AggregateRow> {
    aggregate_by(events, |e| local_day(&e.timestamp), label, unknown)
}

pub fn monthly_in_memory(
    events: &[UsageEvent],
    label: SourceLabel,
    unknown: &mut HashSet<String>,
) -> Vec<AggregateRow> {
    aggregate_by(events, |e| local_month(&e.timestamp), label, unknown)
}

pub fn session_in_memory(
    events: &[UsageEvent],
    unknown: &mut HashSet<String>,
) -> Vec<AggregateRow> {
    // Grouped by (source, session_id)
    let mut map: HashMap<(Source, String), AggregateRow> = HashMap::new();
    for e in events {
        let key = (e.source, e.session_id.clone());
        let row = map.entry(key).or_insert_with(|| AggregateRow {
            key: e.session_id.clone(),
            source: SourceLabel::Source(e.source),
            project_path: e.project_path.clone(),
            latest_timestamp: Some(e.timestamp),
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            total_tokens: 0,
            cost_usd: 0.0,
        });
        row.input_tokens += e.input_tokens;
        row.output_tokens += e.output_tokens;
        row.cache_read_tokens += e.cache_read_tokens;
        row.cache_write_tokens += e.cache_write_tokens;
        row.total_tokens +=
            e.input_tokens + e.output_tokens + e.cache_read_tokens + e.cache_write_tokens;
        row.cost_usd += cost_of(e, Some(unknown));

        if row.project_path.is_none() && e.project_path.is_some() {
            row.project_path = e.project_path.clone();
        }
        match row.latest_timestamp {
            Some(t) if e.timestamp > t => row.latest_timestamp = Some(e.timestamp),
            None => row.latest_timestamp = Some(e.timestamp),
            _ => {}
        }
    }
    let mut rows: Vec<AggregateRow> = map.into_values().collect();
    rows.sort_by(|a, b| {
        b.latest_timestamp
            .unwrap_or(DateTime::<Utc>::MIN_UTC)
            .cmp(&a.latest_timestamp.unwrap_or(DateTime::<Utc>::MIN_UTC))
    });
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(source: Source, sid: &str, model: &str, ts: &str, tokens: u64) -> UsageEvent {
        UsageEvent {
            source,
            timestamp: ts.parse().unwrap(),
            session_id: sid.into(),
            project_path: None,
            model: model.into(),
            input_tokens: tokens,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
        }
    }

    #[test]
    fn filter_excludes_before_since() {
        let events = vec![
            event(Source::Claude, "s", "m", "2026-01-09T00:00:00Z", 1),
            event(Source::Claude, "s", "m", "2026-01-11T00:00:00Z", 1),
        ];
        let since: DateTime<Utc> = "2026-01-10T00:00:00Z".parse().unwrap();
        let filtered = filter_by_date(&events, Some(since), None);
        assert_eq!(filtered.len(), 1);
    }

    #[test]
    fn sessions_group_by_source_plus_id() {
        let events = vec![
            event(
                Source::Claude,
                "sess-a",
                "claude-sonnet-4-6",
                "2026-04-18T09:00:00Z",
                100,
            ),
            event(
                Source::Codex,
                "sess-a",
                "gpt-5.4",
                "2026-04-18T09:00:00Z",
                100,
            ),
        ];
        let mut unknown = HashSet::new();
        let rows = session_in_memory(&events, &mut unknown);
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn daily_sorted_ascending() {
        let events = vec![
            event(
                Source::Claude,
                "s",
                "claude-sonnet-4-6",
                "2026-04-20T09:00:00Z",
                100,
            ),
            event(
                Source::Claude,
                "s",
                "claude-sonnet-4-6",
                "2026-04-18T09:00:00Z",
                100,
            ),
        ];
        let mut unknown = HashSet::new();
        let rows = daily_in_memory(&events, SourceLabel::Source(Source::Claude), &mut unknown);
        assert_eq!(rows.len(), 2);
        assert!(rows[0].key < rows[1].key);
    }
}
