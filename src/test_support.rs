//! Shared fixtures for unit and integration tests (`cfg(test)` or `test-fixtures` feature).

use chrono::Utc;
use rusqlite::Connection;

use crate::repo::RepoIdentity;
use crate::store::schema::DDL;
use crate::store::writes::{insert_events, upsert_repo, EventRow, RepoRow};
use crate::types::{Source, UsageEvent};

fn fixture_ts(offset_ms: i64) -> chrono::DateTime<Utc> {
    let base: chrono::DateTime<Utc> = "2026-04-22T12:00:00Z".parse().unwrap();
    base + chrono::Duration::milliseconds(offset_ms)
}

/// In-memory SQLite connection with schema applied.
pub fn fresh_conn() -> Connection {
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch(DDL).unwrap();
    c
}

pub fn event_row(
    ts: i64,
    session: &str,
    repo: Option<&str>,
    tokens: u64,
    cost: f64,
    source: Source,
) -> EventRow {
    EventRow {
        file_path: format!("/{session}.jsonl"),
        source,
        ts,
        day: "2026-04-22".into(),
        month: "2026-04".into(),
        session_id: session.into(),
        project_path: repo.map(str::to_owned),
        repo: repo.map(str::to_owned),
        model: "claude-sonnet-4-6".into(),
        input: tokens,
        output: 0,
        cache_read: 0,
        cache_write: 0,
        cost_usd: cost,
    }
}

/// Standard four-event fixture used by store and parity tests.
pub fn seed_standard_events(conn: &mut Connection) {
    let tx = conn.transaction().unwrap();
    upsert_repo(
        &tx,
        &RepoRow {
            key: "/u/dev/alpha".into(),
            display_name: "alpha".into(),
            origin_url: None,
            first_seen: 1,
        },
    )
    .unwrap();
    upsert_repo(
        &tx,
        &RepoRow {
            key: "/u/dev/beta".into(),
            display_name: "beta".into(),
            origin_url: None,
            first_seen: 1,
        },
    )
    .unwrap();
    insert_events(
        &tx,
        &[
            event_row(1, "sA", Some("/u/dev/alpha"), 100, 1.00, Source::Claude),
            event_row(2, "sA", Some("/u/dev/alpha"), 50, 0.50, Source::Claude),
            event_row(3, "sB", Some("/u/dev/beta"), 10, 5.00, Source::Codex),
            event_row(4, "sC", None, 20, 0.10, Source::Claude),
        ],
    )
    .unwrap();
    tx.commit().unwrap();
}

/// Same events as [`seed_standard_events`], as [`UsageEvent`] with explicit costs
/// so in-memory aggregation matches stored `cost_usd` in SQL tests.
pub fn sample_usage_events() -> Vec<UsageEvent> {
    vec![
        usage_from_row(0, "sA", Some("/u/dev/alpha"), 100, 1.00, Source::Claude),
        usage_from_row(1, "sA", Some("/u/dev/alpha"), 50, 0.50, Source::Claude),
        usage_from_row(2, "sB", Some("/u/dev/beta"), 10, 5.00, Source::Codex),
        usage_from_row(3, "sC", None, 20, 0.10, Source::Claude),
    ]
}

/// Resolved pairs with repo keys matching the SQL seed (no filesystem/git required).
pub fn sample_resolved_pairs() -> Vec<(UsageEvent, RepoIdentity)> {
    let events = sample_usage_events();
    vec![
        (
            events[0].clone(),
            RepoIdentity {
                key: Some("/u/dev/alpha".into()),
                display_name: "alpha".into(),
                origin_url: None,
            },
        ),
        (
            events[1].clone(),
            RepoIdentity {
                key: Some("/u/dev/alpha".into()),
                display_name: "alpha".into(),
                origin_url: None,
            },
        ),
        (
            events[2].clone(),
            RepoIdentity {
                key: Some("/u/dev/beta".into()),
                display_name: "beta".into(),
                origin_url: None,
            },
        ),
        (
            events[3].clone(),
            RepoIdentity {
                key: None,
                display_name: RepoIdentity::NO_REPO_DISPLAY.into(),
                origin_url: None,
            },
        ),
    ]
}

fn usage_from_row(
    offset_ms: i64,
    session: &str,
    project_path: Option<&str>,
    tokens: u64,
    cost: f64,
    source: Source,
) -> UsageEvent {
    UsageEvent {
        source,
        timestamp: fixture_ts(offset_ms),
        session_id: session.into(),
        project_path: project_path.map(str::to_owned),
        model: "claude-sonnet-4-6".into(),
        input_tokens: tokens,
        output_tokens: 0,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        explicit_cost_usd: Some(cost),
    }
}
