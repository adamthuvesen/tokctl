//! SQL report paths must match in-memory aggregations on the same fixture.

use std::collections::HashSet;

use tokctl::reports::in_memory::{
    daily_in_memory, filter_by_repo, monthly_in_memory, repo_in_memory, session_in_memory,
};
use tokctl::store::queries::{
    daily_report, monthly_report, repo_report, session_report, QueryFilter, RepoFilterSpec,
};
use tokctl::store::queries::{RepoAggregateRow, NO_REPO_SENTINEL};
use tokctl::test_support::{
    fresh_conn, sample_resolved_pairs, sample_usage_events, seed_standard_events,
};
use tokctl::types::{AggregateRow, Source, SourceLabel};

const EPS: f64 = 1e-9;

fn assert_aggregate_parity(sql: &[AggregateRow], mem: &[AggregateRow]) {
    let mut sql_by_key: std::collections::BTreeMap<_, _> =
        sql.iter().map(|r| (r.key.clone(), r)).collect();
    assert_eq!(
        sql.len(),
        mem.len(),
        "row count mismatch: sql={} mem={}",
        sql.len(),
        mem.len()
    );
    for m in mem {
        let s = sql_by_key
            .remove(&m.key)
            .unwrap_or_else(|| panic!("missing sql row for key {}", m.key));
        assert_eq!(s.input_tokens, m.input_tokens, "key={}", m.key);
        assert_eq!(s.output_tokens, m.output_tokens, "key={}", m.key);
        assert_eq!(s.cache_read_tokens, m.cache_read_tokens, "key={}", m.key);
        assert_eq!(s.cache_write_tokens, m.cache_write_tokens, "key={}", m.key);
        assert_eq!(s.total_tokens, m.total_tokens, "key={}", m.key);
        assert!((s.cost_usd - m.cost_usd).abs() < EPS, "key={}", m.key);
    }
    assert!(sql_by_key.is_empty());
}

fn assert_repo_parity(sql: &[RepoAggregateRow], mem: &[RepoAggregateRow]) {
    let mut sql_by_key: std::collections::BTreeMap<_, _> =
        sql.iter().map(|r| (r.key.clone(), r)).collect();
    assert_eq!(sql.len(), mem.len());
    for m in mem {
        let s = sql_by_key.remove(&m.key).expect("missing sql repo row");
        assert_eq!(s.display_name, m.display_name);
        assert_eq!(s.sessions, m.sessions);
        assert_eq!(s.total_tokens, m.total_tokens);
        assert!((s.cost_usd - m.cost_usd).abs() < EPS);
    }
}

#[test]
fn daily_report_matches_in_memory() {
    let mut conn = fresh_conn();
    seed_standard_events(&mut conn);
    let sql = daily_report(&conn, QueryFilter::default()).unwrap();

    let events = sample_usage_events();
    let mut unknown = HashSet::new();
    let mem = daily_in_memory(&events, SourceLabel::All, &mut unknown);

    assert_aggregate_parity(&sql, &mem);
}

#[test]
fn monthly_report_matches_in_memory() {
    let mut conn = fresh_conn();
    seed_standard_events(&mut conn);
    let sql = monthly_report(&conn, QueryFilter::default()).unwrap();

    let events = sample_usage_events();
    let mut unknown = HashSet::new();
    let mem = monthly_in_memory(&events, SourceLabel::All, &mut unknown);

    assert_aggregate_parity(&sql, &mem);
}

#[test]
fn session_report_matches_in_memory() {
    let mut conn = fresh_conn();
    seed_standard_events(&mut conn);
    let sql = session_report(&conn, QueryFilter::default()).unwrap();

    let events = sample_usage_events();
    let mut unknown = HashSet::new();
    let mem = session_in_memory(&events, &mut unknown);

    // Both order by latest activity desc; compare as sets keyed by session id.
    let mut sql_map: std::collections::BTreeMap<_, _> =
        sql.iter().map(|r| (r.key.clone(), r)).collect();
    assert_eq!(sql.len(), mem.len());
    for m in mem {
        let s = sql_map.remove(&m.key).expect("missing session");
        assert_eq!(s.input_tokens, m.input_tokens);
        assert!((s.cost_usd - m.cost_usd).abs() < EPS);
    }
}

#[test]
fn repo_report_matches_in_memory() {
    let mut conn = fresh_conn();
    seed_standard_events(&mut conn);
    let sql = repo_report(&conn, QueryFilter::default()).unwrap();

    let resolved = sample_resolved_pairs();
    let mut unknown = HashSet::new();
    let mem = repo_in_memory(&resolved, &None, &mut unknown);

    assert_repo_parity(&sql, &mem);
}

#[test]
fn daily_report_source_filter_parity() {
    let mut conn = fresh_conn();
    seed_standard_events(&mut conn);
    let filter = QueryFilter {
        source: Some(Source::Claude),
        ..Default::default()
    };
    let sql = daily_report(&conn, filter.clone()).unwrap();

    let events: Vec<_> = sample_usage_events()
        .into_iter()
        .filter(|e| e.source == Source::Claude)
        .collect();
    let mut unknown = HashSet::new();
    let mem = daily_in_memory(&events, SourceLabel::Source(Source::Claude), &mut unknown);

    assert_aggregate_parity(&sql, &mem);
}

#[test]
fn repo_filter_display_name_parity() {
    let mut conn = fresh_conn();
    seed_standard_events(&mut conn);
    let filter = QueryFilter {
        repo: Some(RepoFilterSpec::DisplayName("alpha".into())),
        ..Default::default()
    };
    let sql = daily_report(&conn, filter.clone()).unwrap();

    let resolved = sample_resolved_pairs();
    let spec = Some(RepoFilterSpec::DisplayName("alpha".into()));
    let filtered: Vec<_> = tokctl::reports::in_memory::filter_by_repo(&resolved, &spec);
    let mut unknown = HashSet::new();
    let mem = daily_in_memory(&filtered, SourceLabel::All, &mut unknown);

    assert_aggregate_parity(&sql, &mem);
}

#[test]
fn repo_filter_no_repo_session_parity() {
    let mut conn = fresh_conn();
    seed_standard_events(&mut conn);
    let filter = QueryFilter {
        repo: Some(RepoFilterSpec::NoRepo),
        ..Default::default()
    };
    let sql = session_report(&conn, filter).unwrap();

    let resolved = sample_resolved_pairs();
    let spec = Some(RepoFilterSpec::NoRepo);
    let mut unknown = HashSet::new();
    let mem = session_in_memory(&filter_by_repo(&resolved, &spec), &mut unknown);

    assert_eq!(sql.len(), mem.len());
    assert_eq!(sql[0].key, "sC");
    assert_eq!(mem[0].key, "sC");
}

#[test]
fn no_repo_sentinel_matches_display_name() {
    assert_eq!(NO_REPO_SENTINEL, "<NO_REPO>");
}
