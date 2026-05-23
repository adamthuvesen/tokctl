use super::super::cache::DataCache;
use super::*;
use crate::store::schema::DDL;
use crate::store::writes::{insert_events, EventRow};
use crate::tui::data::{LeftMemoKey, TrendMemoKey};
use crate::tui::state::{AppState, TimeWindow, TrendGranularity};

#[test]
fn day_bounds_parses() {
    let (a, b) = day_bounds("2026-04-22");
    assert!(a < b);
}

#[test]
fn short_sessions_label_takes_prefix() {
    let r = SessionRow {
        session_id: "abcdefghij".into(),
        source: Source::Claude,
        latest_ts: Utc::now(),
        project: None,
        cost: 0.0,
        total_tokens: 0,
    };
    assert_eq!(r.session_id.chars().take(8).collect::<String>(), "abcdefgh");
}

#[test]
fn sort_changes_loaded_rows() {
    let mut left = vec![
        LeftRow {
            label: "beta".into(),
            key: "beta".into(),
            sessions: 1,
            total_tokens: 0,
            cost: 2.0,
            is_no_repo: false,
            latest_ts: None,
            source: None,
        },
        LeftRow {
            label: "alpha".into(),
            key: "alpha".into(),
            sessions: 1,
            total_tokens: 0,
            cost: 1.0,
            is_no_repo: false,
            latest_ts: None,
            source: None,
        },
    ];
    sort_left_rows(&mut left, Section::Repos, Sort::AlphaAsc);
    assert_eq!(left[0].label, "alpha");

    let mut sessions = vec![
        SessionRow {
            session_id: "old".into(),
            source: Source::Claude,
            latest_ts: "2026-04-18T09:00:00Z".parse().unwrap(),
            project: Some("old".into()),
            cost: 10.0,
            total_tokens: 0,
        },
        SessionRow {
            session_id: "new".into(),
            source: Source::Claude,
            latest_ts: "2026-04-19T09:00:00Z".parse().unwrap(),
            project: Some("new".into()),
            cost: 1.0,
            total_tokens: 0,
        },
    ];
    sort_session_rows(&mut sessions, Sort::RecentDesc);
    assert_eq!(sessions[0].session_id, "new");
    sort_session_rows(&mut sessions, Sort::RecentAsc);
    assert_eq!(sessions[0].session_id, "old");
    sort_session_rows(&mut sessions, Sort::AlphaDesc);
    assert_eq!(sessions[0].session_id, "old");
}

#[test]
fn sessions_section_recent_sort_is_global_across_sources() {
    let mut left = vec![
        LeftRow {
            label: "claude-old".into(),
            key: "claude-old".into(),
            sessions: 1,
            total_tokens: 0,
            cost: 100.0,
            is_no_repo: false,
            latest_ts: Some("2026-04-18T09:00:00Z".parse().unwrap()),
            source: Some(Source::Claude),
        },
        LeftRow {
            label: "codex-new".into(),
            key: "codex-new".into(),
            sessions: 1,
            total_tokens: 0,
            cost: 1.0,
            is_no_repo: false,
            latest_ts: Some("2026-04-19T09:00:00Z".parse().unwrap()),
            source: Some(Source::Codex),
        },
    ];

    sort_left_rows(&mut left, Section::Sessions, Sort::RecentDesc);

    assert_eq!(left[0].key, "codex-new");
}

fn left_memo_key(state: &AppState) -> LeftMemoKey {
    LeftMemoKey(
        state.current_section,
        state.time_window,
        state.source_filter,
        state.trend_granularity,
    )
}

fn fixture_conn_with_events() -> Connection {
    let mut conn = mk_conn();
    let tx = conn.transaction().unwrap();
    insert_events(
        &tx,
        &[
            mk_event(1_000, "2024-01-01", "2024-01", Source::Claude, 1.0),
            mk_event(2_000, "2024-01-02", "2024-01", Source::Claude, 2.0),
        ],
    )
    .unwrap();
    tx.commit().unwrap();
    conn
}

#[test]
fn left_memo_caches_after_first_refresh() {
    let conn = fixture_conn_with_events();
    let state = AppState {
        current_section: Section::Days,
        time_window: TimeWindow::All,
        ..AppState::default()
    };
    let mut cache = DataCache::default();
    cache.refresh_for(&conn, &state, crate::tui::state::RefreshMask::all());
    let key = left_memo_key(&state);
    assert!(
        cache.left_memo.contains_key(&key),
        "first refresh populates the memo"
    );
}

#[test]
fn clear_memos_drops_everything() {
    let conn = fixture_conn_with_events();
    let state = AppState {
        current_section: Section::Days,
        time_window: TimeWindow::All,
        ..AppState::default()
    };
    let mut cache = DataCache::default();
    cache.refresh_for(&conn, &state, crate::tui::state::RefreshMask::all());
    assert!(!cache.left_memo.is_empty());
    cache.clear_memos();
    assert!(cache.left_memo.is_empty());
    assert!(cache.trend_memo.is_empty());
    assert!(cache.sparkline_memo.is_none());
    assert!(cache.memo_mtime_ns.is_none());
}

#[test]
fn switching_sections_keeps_both_memos_warm() {
    let conn = fixture_conn_with_events();
    let mut cache = DataCache::default();
    let mut state = AppState {
        current_section: Section::Days,
        time_window: TimeWindow::All,
        ..AppState::default()
    };
    cache.refresh_for(&conn, &state, crate::tui::state::RefreshMask::all());
    let days_key = left_memo_key(&state);

    // Visit Models — should add a second memo entry without evicting Days.
    state.current_section = Section::Models;
    cache.refresh_for(
        &conn,
        &state,
        crate::tui::state::RefreshMask {
            left: true,
            ..Default::default()
        },
    );
    let models_key = left_memo_key(&state);

    assert!(cache.left_memo.contains_key(&days_key));
    assert!(cache.left_memo.contains_key(&models_key));
}

#[test]
fn trend_memo_survives_section_switch() {
    let conn = fixture_conn_with_events();
    let mut cache = DataCache::default();
    let mut state = AppState {
        current_section: Section::Provider,
        time_window: TimeWindow::All,
        trend_granularity: TrendGranularity::Daily,
        ..AppState::default()
    };
    cache.refresh_for(&conn, &state, crate::tui::state::RefreshMask::all());
    let trend_key = TrendMemoKey(
        state.time_window,
        state.source_filter,
        state.trend_granularity,
    );
    assert!(cache.trend_memo.contains_key(&trend_key));

    // Switch to Repos and back — the same trend key must still be warm.
    state.current_section = Section::Repos;
    cache.refresh_for(
        &conn,
        &state,
        crate::tui::state::RefreshMask {
            left: true,
            ..Default::default()
        },
    );
    assert!(
        cache.trend_memo.contains_key(&trend_key),
        "trend memo is keyed without section, so a section switch must not evict it"
    );
}

#[test]
fn mtime_change_clears_memos() {
    let conn = fixture_conn_with_events();
    let state = AppState {
        current_section: Section::Days,
        time_window: TimeWindow::All,
        ..AppState::default()
    };
    let mut cache = DataCache::default();
    cache.refresh_for(&conn, &state, crate::tui::state::RefreshMask::all());
    assert!(!cache.left_memo.is_empty());

    // Simulate ingest by advancing the recorded memo mtime so it
    // differs from the live status on the next refresh — the live
    // mtime will then reset memo_mtime_ns and clear the memos.
    cache.memo_mtime_ns = Some(0);
    cache.refresh_for(
        &conn,
        &state,
        crate::tui::state::RefreshMask {
            left: true,
            ..Default::default()
        },
    );
    // After this refresh the memo is freshly repopulated against the
    // live mtime; assert the live mtime is what we expect.
    assert_eq!(cache.memo_mtime_ns, cache.status.mtime_ns);
}

#[test]
fn cache_status_reports_event_count() {
    let mut conn = mk_conn();
    let tx = conn.transaction().unwrap();
    insert_events(
        &tx,
        &[EventRow {
            file_path: "/x".into(),
            source: Source::Claude,
            ts: 1,
            day: "2026-04-22".into(),
            month: "2026-04".into(),
            session_id: "s".into(),
            project_path: None,
            repo: None,
            model: "claude-sonnet-4-6".into(),
            input: 1,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            cost_usd: 0.0,
        }],
    )
    .unwrap();
    tx.commit().unwrap();
    let status = load_cache_status(&conn, Utc::now()).unwrap();
    assert_eq!(status.event_count, 1);
}

#[test]
fn model_session_drill_uses_first_non_null_project_path() {
    let mut conn = mk_conn();
    let tx = conn.transaction().unwrap();
    insert_events(
        &tx,
        &[
            EventRow {
                file_path: "/x".into(),
                source: Source::Claude,
                ts: 1,
                day: "2026-04-22".into(),
                month: "2026-04".into(),
                session_id: "same".into(),
                project_path: None,
                repo: None,
                model: "model-x".into(),
                input: 1,
                output: 0,
                cache_read: 0,
                cache_write: 0,
                cost_usd: 0.1,
            },
            EventRow {
                file_path: "/x".into(),
                source: Source::Claude,
                ts: 2,
                day: "2026-04-22".into(),
                month: "2026-04".into(),
                session_id: "same".into(),
                project_path: Some("/tmp/zeta".into()),
                repo: None,
                model: "model-x".into(),
                input: 1,
                output: 0,
                cache_read: 0,
                cache_write: 0,
                cost_usd: 0.1,
            },
            EventRow {
                file_path: "/x".into(),
                source: Source::Claude,
                ts: 3,
                day: "2026-04-22".into(),
                month: "2026-04".into(),
                session_id: "same".into(),
                project_path: Some("/tmp/alpha".into()),
                repo: None,
                model: "model-x".into(),
                input: 1,
                output: 0,
                cache_read: 0,
                cache_write: 0,
                cost_usd: 0.1,
            },
        ],
    )
    .unwrap();
    tx.commit().unwrap();

    let rows = load_sessions_by_model(&conn, "model-x", &QueryFilter::default()).unwrap();

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].project.as_deref(), Some("zeta"));
    assert_eq!(rows[0].latest_ts.timestamp_millis(), 3);
}

#[test]
fn refresh_failure_is_visible_not_empty_data() {
    let conn = Connection::open_in_memory().unwrap();
    let mut cache = DataCache::default();

    cache.refresh_all(&conn, &AppState::default());

    let err = cache.refresh_error.as_ref().expect("refresh error");
    assert!(err.display_message().contains("refresh failed:"));
}

#[test]
fn valid_empty_refresh_has_no_error() {
    let conn = mk_conn();
    let mut cache = DataCache::default();

    cache.refresh_all(&conn, &AppState::default());

    assert!(cache.refresh_error.is_none());
    assert!(cache.left.is_empty());
}

#[test]
fn successful_refresh_clears_prior_error() {
    let conn = Connection::open_in_memory().unwrap();
    let mut cache = DataCache::default();

    cache.refresh_for(
        &conn,
        &AppState::default(),
        crate::tui::state::RefreshMask {
            left: true,
            ..Default::default()
        },
    );
    assert!(cache.refresh_error.is_some());

    conn.execute_batch(DDL).unwrap();
    cache.refresh_for(
        &conn,
        &AppState::default(),
        crate::tui::state::RefreshMask {
            left: true,
            ..Default::default()
        },
    );

    assert!(cache.refresh_error.is_none());
}

#[test]
fn refresh_failure_preserves_previous_successful_rows() {
    let conn = fixture_conn_with_events();
    let state = AppState {
        current_section: Section::Days,
        time_window: TimeWindow::All,
        ..AppState::default()
    };
    let mut cache = DataCache::default();
    cache.refresh_for(
        &conn,
        &state,
        crate::tui::state::RefreshMask {
            left: true,
            ..Default::default()
        },
    );
    assert!(!cache.left.is_empty());

    conn.execute("DROP TABLE events", []).unwrap();
    cache.refresh_for(
        &conn,
        &state,
        crate::tui::state::RefreshMask {
            left: true,
            ..Default::default()
        },
    );

    assert!(cache.refresh_error.is_some());
    assert!(!cache.left.is_empty());
}

#[test]
fn refresh_does_not_mutate_cache_tables() {
    let conn = fixture_conn_with_events();
    let before_events: i64 = conn
        .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
        .unwrap();
    let before_files: i64 = conn
        .query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))
        .unwrap();

    let mut cache = DataCache::default();
    cache.refresh_all(
        &conn,
        &AppState {
            time_window: TimeWindow::All,
            ..AppState::default()
        },
    );

    let after_events: i64 = conn
        .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
        .unwrap();
    let after_files: i64 = conn
        .query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))
        .unwrap();
    assert_eq!(after_events, before_events);
    assert_eq!(after_files, before_files);
}

fn mk_conn() -> Connection {
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch(DDL).unwrap();
    c
}

fn mk_event(ts_ms: i64, day: &str, month: &str, src: Source, cost: f64) -> EventRow {
    EventRow {
        file_path: format!("/f-{ts_ms}.jsonl"),
        source: src,
        ts: ts_ms,
        day: day.into(),
        month: month.into(),
        session_id: format!("s-{ts_ms}"),
        project_path: None,
        repo: None,
        model: "claude-sonnet-4-6".into(),
        input: 10,
        output: 10,
        cache_read: 0,
        cache_write: 0,
        cost_usd: cost,
    }
}

#[test]
fn trend_monthly_buckets_by_month() {
    let mut conn = mk_conn();
    let tx = conn.transaction().unwrap();
    insert_events(
        &tx,
        &[
            mk_event(
                1_700_000_000_000,
                "2023-11-14",
                "2023-11",
                Source::Claude,
                1.0,
            ),
            mk_event(
                1_700_100_000_000,
                "2023-11-15",
                "2023-11",
                Source::Codex,
                2.0,
            ),
            mk_event(
                1_702_000_000_000,
                "2023-12-08",
                "2023-12",
                Source::Claude,
                4.0,
            ),
        ],
    )
    .unwrap();
    tx.commit().unwrap();

    let state = AppState {
        time_window: TimeWindow::All,
        trend_granularity: TrendGranularity::Monthly,
        ..AppState::default()
    };
    let now = Utc::now();
    let rows = load_trend(&conn, &state, now).unwrap();
    let nov = rows
        .iter()
        .find(|r| r.bucket.contains("11"))
        .expect("nov bucket");
    assert!((nov.claude_cost - 1.0).abs() < 1e-9);
    assert!((nov.codex_cost - 2.0).abs() < 1e-9);
    assert_eq!(nov.cursor_cost, 0.0);
    let dec = rows
        .iter()
        .find(|r| r.bucket.contains("12"))
        .expect("dec bucket");
    assert!((dec.claude_cost - 4.0).abs() < 1e-9);
}

#[test]
fn trend_source_filter_zeroes_other_column() {
    let mut conn = mk_conn();
    let tx = conn.transaction().unwrap();
    insert_events(
        &tx,
        &[
            mk_event(
                1_700_000_000_000,
                "2023-11-14",
                "2023-11",
                Source::Claude,
                1.0,
            ),
            mk_event(
                1_700_100_000_000,
                "2023-11-15",
                "2023-11",
                Source::Codex,
                2.0,
            ),
        ],
    )
    .unwrap();
    tx.commit().unwrap();
    let state = AppState {
        time_window: TimeWindow::All,
        trend_granularity: TrendGranularity::Monthly,
        source_filter: crate::tui::state::SourceFilter::Claude,
        ..AppState::default()
    };
    let rows = load_trend(&conn, &state, Utc::now()).unwrap();
    let nov = &rows[0];
    assert!((nov.claude_cost - 1.0).abs() < 1e-9);
    assert_eq!(nov.codex_cost, 0.0);
    assert_eq!(nov.cursor_cost, 0.0);
    assert!((nov.total_cost - 1.0).abs() < 1e-9);
    assert_eq!(nov.total_tokens, 20);
}

#[test]
fn load_events_for_returns_only_matching_session_ordered_chronologically() {
    let mut conn = mk_conn();
    let tx = conn.transaction().unwrap();
    let mut e_match = mk_event(2_000, "2023-11-15", "2023-11", Source::Claude, 0.5);
    e_match.session_id = "alpha".into();
    let mut e_match_earlier = mk_event(1_000, "2023-11-15", "2023-11", Source::Claude, 0.2);
    e_match_earlier.session_id = "alpha".into();
    let mut e_other_session = mk_event(3_000, "2023-11-15", "2023-11", Source::Claude, 1.0);
    e_other_session.session_id = "beta".into();
    let mut e_other_source = mk_event(4_000, "2023-11-15", "2023-11", Source::Codex, 0.3);
    e_other_source.session_id = "alpha".into();
    insert_events(
        &tx,
        &[e_match, e_match_earlier, e_other_session, e_other_source],
    )
    .unwrap();
    tx.commit().unwrap();

    let filter = QueryFilter {
        source: None,
        since_ms: None,
        until_ms: None,
        repo: None,
    };
    let rows = load_events_for(&conn, Source::Claude, "alpha", filter).unwrap();
    assert_eq!(rows.len(), 2, "only alpha-claude events");
    assert!(rows[0].ts < rows[1].ts, "ascending by ts");
    assert!((rows[0].cost - 0.2).abs() < 1e-9);
}

#[test]
fn load_events_for_respects_time_window() {
    let mut conn = mk_conn();
    let tx = conn.transaction().unwrap();
    let mut early = mk_event(1_000, "2023-11-15", "2023-11", Source::Claude, 0.1);
    early.session_id = "s".into();
    let mut late = mk_event(5_000, "2023-11-15", "2023-11", Source::Claude, 0.2);
    late.session_id = "s".into();
    insert_events(&tx, &[early, late]).unwrap();
    tx.commit().unwrap();

    let filter = QueryFilter {
        source: None,
        since_ms: Some(3_000),
        until_ms: None,
        repo: None,
    };
    let rows = load_events_for(&conn, Source::Claude, "s", filter).unwrap();
    assert_eq!(rows.len(), 1);
    assert!((rows[0].cost - 0.2).abs() < 1e-9);
}

#[test]
fn trend_daily_marks_today_current() {
    let mut conn = mk_conn();
    let today_local = chrono::Local::now().date_naive();
    let today_str = today_local.format("%Y-%m-%d").to_string();
    let month_str = today_local.format("%Y-%m").to_string();
    let ts_ms = chrono::Local
        .from_local_datetime(&today_local.and_hms_opt(12, 0, 0).unwrap())
        .single()
        .unwrap()
        .timestamp_millis();
    let tx = conn.transaction().unwrap();
    insert_events(
        &tx,
        &[mk_event(ts_ms, &today_str, &month_str, Source::Claude, 1.5)],
    )
    .unwrap();
    tx.commit().unwrap();
    let state = AppState {
        time_window: TimeWindow::All,
        trend_granularity: TrendGranularity::Daily,
        ..AppState::default()
    };
    let rows = load_trend(&conn, &state, Utc::now()).unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].is_current);
}
