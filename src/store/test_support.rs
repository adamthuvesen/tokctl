//! Shared fixtures for unit and integration tests (`cfg(test)` or `test-fixtures` feature).

use crate::store::Store;
use chrono::Utc;

use crate::store::writes::{insert_events, upsert_repo, EventRow, RepoRow};
use crate::types::Source;

fn fixture_ts(offset_ms: i64) -> chrono::DateTime<Utc> {
    let base: chrono::DateTime<Utc> = "2026-04-22T12:00:00Z".parse().unwrap();
    base + chrono::Duration::milliseconds(offset_ms)
}

/// In-memory SQLite connection with schema applied.
pub fn fresh_conn() -> Store {
    Store::ephemeral().unwrap()
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
        ts: fixture_ts(ts - 1).timestamp_millis(),
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
        explicit_cost_usd: None,
        cost_provenance: "legacy".into(),
        message_id: None,
    }
}

/// Standard four-event fixture used by store and parity tests.
pub fn seed_standard_events(conn: &mut Store) {
    let tx = conn.connection_mut().transaction().unwrap();
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
