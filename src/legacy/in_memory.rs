use crate::ingest::run::{local_day, local_month};
use crate::pricing::cost_of;
use crate::repo::{project_basename, RepoIdentity, Resolver};
use crate::store::queries::{RepoAggregateRow, RepoFilterSpec, NO_REPO_SENTINEL};
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

/// Resolve repo identity for every event and return an annotated pairing.
/// The resolver is memoized across the call.
pub fn resolve_repos(events: &[UsageEvent]) -> Vec<(UsageEvent, RepoIdentity)> {
    let mut resolver = Resolver::new();
    events
        .iter()
        .cloned()
        .map(|e| {
            let id = match &e.project_path {
                Some(pp) => resolver.resolve(pp),
                None => RepoIdentity {
                    key: None,
                    display_name: RepoIdentity::NO_REPO_DISPLAY.into(),
                    origin_url: None,
                },
            };
            (e, id)
        })
        .collect()
}

fn matches_repo_filter(id: &RepoIdentity, filter: &Option<RepoFilterSpec>) -> bool {
    match filter {
        None => true,
        Some(RepoFilterSpec::NoRepo) => id.key.is_none(),
        Some(RepoFilterSpec::DisplayName(n)) => id.key.is_some() && &id.display_name == n,
        Some(RepoFilterSpec::KeyPrefix(p)) => id.key.as_deref().is_some_and(|k| k.starts_with(p)),
    }
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
    let mut map: HashMap<(Source, String), AggregateRow> = HashMap::new();
    for e in events {
        let key = (e.source, e.session_id.clone());
        let row = map.entry(key).or_insert_with(|| AggregateRow {
            key: e.session_id.clone(),
            source: SourceLabel::Source(e.source),
            project_path: e
                .project_path
                .as_deref()
                .map(|p| project_basename(p).to_owned()),
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
            row.project_path = e
                .project_path
                .as_deref()
                .map(|p| project_basename(p).to_owned());
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

/// Filter events by repo in-memory and return the subset that passes. The
/// returned events preserve original order.
pub fn filter_by_repo(
    resolved: &[(UsageEvent, RepoIdentity)],
    spec: &Option<RepoFilterSpec>,
) -> Vec<UsageEvent> {
    resolved
        .iter()
        .filter(|(_, id)| matches_repo_filter(id, spec))
        .map(|(e, _)| e.clone())
        .collect()
}

/// Repo-grouped aggregation mirroring [`crate::store::queries::repo_report`].
pub fn repo_in_memory(
    resolved: &[(UsageEvent, RepoIdentity)],
    repo_filter: &Option<RepoFilterSpec>,
    unknown: &mut HashSet<String>,
) -> Vec<RepoAggregateRow> {
    // key used to aggregate: canonical repo key OR the no-repo sentinel
    #[derive(Default)]
    struct Bucket {
        display_name: String,
        origin_url: Option<String>,
        sessions: HashSet<(Source, String)>,
        input: u64,
        output: u64,
        cache_read: u64,
        cache_write: u64,
        total: u64,
        cost: f64,
    }
    let mut buckets: HashMap<String, Bucket> = HashMap::new();

    for (e, id) in resolved {
        if !matches_repo_filter(id, repo_filter) {
            continue;
        }
        let key = id
            .key
            .clone()
            .unwrap_or_else(|| NO_REPO_SENTINEL.to_owned());
        let b = buckets.entry(key).or_default();
        if b.display_name.is_empty() {
            b.display_name = id.display_name.clone();
            b.origin_url = id.origin_url.clone();
        }
        b.sessions.insert((e.source, e.session_id.clone()));
        b.input += e.input_tokens;
        b.output += e.output_tokens;
        b.cache_read += e.cache_read_tokens;
        b.cache_write += e.cache_write_tokens;
        b.total += e.input_tokens + e.output_tokens + e.cache_read_tokens + e.cache_write_tokens;
        b.cost += cost_of(e, Some(unknown));
    }

    let mut rows: Vec<RepoAggregateRow> = buckets
        .into_iter()
        .map(|(key, b)| RepoAggregateRow {
            key,
            display_name: b.display_name,
            origin_url: b.origin_url,
            sessions: b.sessions.len() as u64,
            input_tokens: b.input,
            output_tokens: b.output,
            cache_read_tokens: b.cache_read,
            cache_write_tokens: b.cache_write,
            total_tokens: b.total,
            cost_usd: b.cost,
        })
        .collect();
    rows.sort_by(|a, b| {
        b.cost_usd
            .partial_cmp(&a.cost_usd)
            .unwrap_or(std::cmp::Ordering::Equal)
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
            explicit_cost_usd: None,
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

    #[test]
    fn repo_in_memory_buckets_no_repo_separately() {
        let mut e1 = event(
            Source::Claude,
            "a",
            "claude-sonnet-4-6",
            "2026-04-18T09:00:00Z",
            10,
        );
        e1.project_path = None;
        let e2 = event(
            Source::Claude,
            "b",
            "claude-sonnet-4-6",
            "2026-04-18T10:00:00Z",
            20,
        );
        let resolved = resolve_repos(&[e1, e2]);
        let mut unknown = HashSet::new();
        let rows = repo_in_memory(&resolved, &None, &mut unknown);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].is_no_repo());
        assert_eq!(rows[0].sessions, 2);
    }

    #[test]
    fn repo_in_memory_counts_source_session_pairs() {
        let a = event(
            Source::Claude,
            "same",
            "claude-sonnet-4-6",
            "2026-04-18T09:00:00Z",
            10,
        );
        let b = event(Source::Codex, "same", "gpt-5.4", "2026-04-18T10:00:00Z", 20);
        let resolved = resolve_repos(&[a, b]);
        let mut unknown = HashSet::new();
        let rows = repo_in_memory(&resolved, &None, &mut unknown);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].sessions, 2);
    }
}
