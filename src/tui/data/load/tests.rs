use super::super::cache::DataCache;
use super::*;
use crate::store::schema::DDL;
use crate::store::writes::{insert_events, EventRow};
use crate::tui::data::{LeftMemoKey, TrendMemoKey};
use crate::tui::state::{AppState, TimeWindow, TrendGranularity};

#[test]
fn day_bounds_parses() {
    let (a, b) = bucket_bounds("2026-04-22", TrendGranularity::Daily).unwrap();
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
            selection: crate::tui::data::AggregateSelection::Model(vec!["fixture".into()]),
            model_stats: None,
        },
        LeftRow {
            label: "alpha".into(),
            key: "alpha".into(),
            sessions: 1,
            total_tokens: 0,
            cost: 1.0,
            is_no_repo: false,
            latest_ts: None,
            selection: crate::tui::data::AggregateSelection::Model(vec!["fixture".into()]),
            model_stats: None,
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
            selection: crate::tui::data::AggregateSelection::Model(vec!["fixture".into()]),
            model_stats: None,
        },
        LeftRow {
            label: "codex-new".into(),
            key: "codex-new".into(),
            sessions: 1,
            total_tokens: 0,
            cost: 1.0,
            is_no_repo: false,
            latest_ts: Some("2026-04-19T09:00:00Z".parse().unwrap()),
            selection: crate::tui::data::AggregateSelection::Model(vec!["fixture".into()]),
            model_stats: None,
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
        state.models_by_family,
    )
}

fn fixture_conn_with_events() -> Store {
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
    cache.refresh_all(&conn, &state);
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
    cache.refresh_all(&conn, &state);
    assert!(!cache.left_memo.is_empty());
    cache.clear_memos();
    assert!(cache.left_memo.is_empty());
    assert!(cache.trend_memo.is_empty());
    assert!(cache.sparkline_memo.is_none());
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
    cache.refresh_all(&conn, &state);
    let days_key = left_memo_key(&state);

    // Visit Models — should add a second memo entry without evicting Days.
    state.current_section = Section::Models;
    cache.refresh_all(&conn, &state);
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
    cache.refresh_all(&conn, &state);
    let trend_key = TrendMemoKey(
        state.time_window,
        state.source_filter,
        state.trend_granularity,
    );
    assert!(cache.trend_memo.contains_key(&trend_key));

    // Switch to Repos and back — the same trend key must still be warm.
    state.current_section = Section::Repos;
    cache.refresh_all(&conn, &state);
    assert!(
        cache.trend_memo.contains_key(&trend_key),
        "trend memo is keyed without section, so a section switch must not evict it"
    );
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
            cost_provenance: "source".into(),
            message_id: None,
            explicit_cost_usd: Some(0.0),
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
                cost_provenance: "source".into(),
                message_id: None,
                explicit_cost_usd: Some(0.1),
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
                cost_provenance: "source".into(),
                message_id: None,
                explicit_cost_usd: Some(0.1),
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
                cost_provenance: "source".into(),
                message_id: None,
                explicit_cost_usd: Some(0.1),
            },
        ],
    )
    .unwrap();
    tx.commit().unwrap();

    let rows = load_sessions_for(
        &conn,
        QueryScope {
            models: Some(vec!["model-x".into()]),
            ..Default::default()
        },
    )
    .unwrap();

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].project.as_deref(), Some("zeta"));
    assert_eq!(rows[0].latest_ts.timestamp_millis(), 3);
}

#[test]
fn refresh_failure_is_visible_not_empty_data() {
    let conn = Store::ephemeral().unwrap();
    conn.execute_batch("DROP TABLE events").unwrap();
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
    let conn = Store::ephemeral().unwrap();
    conn.execute_batch("DROP TABLE events").unwrap();
    let mut cache = DataCache::default();

    cache.refresh_all(&conn, &AppState::default());
    assert!(cache.refresh_error.is_some());

    conn.execute_batch(DDL).unwrap();
    cache.refresh_all(&conn, &AppState::default());

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
    cache.refresh_all(&conn, &state);
    assert!(!cache.left.is_empty());

    conn.execute("DROP TABLE events", []).unwrap();
    cache.refresh_all(&conn, &state);

    assert!(cache.refresh_error.is_some());
    assert!(!cache.left.is_empty());
}

#[test]
fn refresh_does_not_ingest_events_or_files() {
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

fn mk_conn() -> Store {
    let c = Store::ephemeral().unwrap();
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
        cost_provenance: "source".into(),
        message_id: None,
        explicit_cost_usd: Some(cost),
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
    let rows = load_trend(&conn, &state, now, base_filter(&state, now).unwrap()).unwrap();
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
    let rows = load_trend(
        &conn,
        &state,
        Utc::now(),
        base_filter(&state, Utc::now()).unwrap(),
    )
    .unwrap();
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
        ..Default::default()
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
        ..Default::default()
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
    let rows = load_trend(
        &conn,
        &state,
        Utc::now(),
        base_filter(&state, Utc::now()).unwrap(),
    )
    .unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].is_current);
}

#[test]
fn all_bucket_drills_preserve_source_time_bounds_and_full_parent_totals() {
    let calendar = crate::calendar::Calendar::system().unwrap();
    for (granularity, bucket) in [
        (TrendGranularity::Daily, "2026-04-22"),
        (TrendGranularity::Weekly, "2026-W17"),
        (TrendGranularity::Monthly, "2026-04"),
        (TrendGranularity::Yearly, "2026"),
    ] {
        let (start, end) = bucket_bounds(bucket, granularity).unwrap();
        let mut store = Store::ephemeral().unwrap();
        let events = [
            (start + 1, Source::Claude, 100.0),
            (start + 20, Source::Claude, 2.0),
            (end - 1, Source::Claude, 3.0),
            (end, Source::Claude, 200.0),
            (start + 30, Source::Codex, 300.0),
        ]
        .into_iter()
        .map(|(ts, source, cost)| {
            let (day, month) = calendar.local_day_month(Utc.timestamp_millis_opt(ts).unwrap());
            let mut row = mk_event(ts, &day, &month, source, cost);
            row.session_id = "repeat".into();
            row
        })
        .collect::<Vec<_>>();
        let tx = store.transaction().unwrap();
        insert_events(&tx, &events).unwrap();
        tx.commit().unwrap();
        let mut scope = QueryScope {
            source: Some(Source::Claude),
            since_ms: Some(start + 10),
            until_ms: Some(end),
            ..Default::default()
        };
        let rows = load_left_axis(&store, Section::Days, scope.clone(), granularity).unwrap();
        assert_eq!(rows.len(), 1, "{granularity:?}");
        assert_eq!(rows[0].key, bucket);
        rows[0].selection.apply_to(&mut scope);
        assert_eq!(scope.since_ms, Some(start + 10));
        assert_eq!(scope.until_ms, Some(end));
        let sessions = load_sessions_for(&store, scope.clone()).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].source, Source::Claude);
        assert_eq!(sessions[0].cost, rows[0].cost);
        let events =
            load_events_for(&store, sessions[0].source, &sessions[0].session_id, scope).unwrap();
        assert_eq!(
            events.len(),
            2,
            "{granularity:?} excludes the next bucket and includes its last millisecond"
        );
        assert_eq!(events.iter().map(|row| row.cost).sum::<f64>(), rows[0].cost);
    }
}

#[test]
fn model_and_exact_repo_scopes_survive_session_event_drills() {
    let mut store = Store::ephemeral().unwrap();
    let mut events = Vec::new();
    for (ts, repo, model, source, cost) in [
        (1000, "/repo", "model-a", Source::Claude, 2.0),
        (2000, "/repo-child", "model-a", Source::Claude, 20.0),
        (3000, "/repo", "model-b", Source::Claude, 30.0),
        (4000, "/repo", "model-a", Source::Codex, 40.0),
    ] {
        let mut row = mk_event(ts, "1970-01-01", "1970-01", source, cost);
        row.repo = Some(repo.into());
        row.model = model.into();
        row.session_id = "repeat".into();
        events.push(row);
    }
    let tx = store.transaction().unwrap();
    insert_events(&tx, &events).unwrap();
    tx.commit().unwrap();
    let scope = QueryScope {
        repo: Some(RepoFilterSpec::KeyExact("/repo".into())),
        models: Some(vec!["model-a".into()]),
        source: Some(Source::Claude),
        ..Default::default()
    };
    let sessions = load_sessions_for(&store, scope.clone()).unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].cost, 2.0);
    let events = load_events_for(&store, sessions[0].source, "repeat", scope).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].cost, 2.0);
}

#[test]
fn calendar_rollover_invalidates_memos_without_data_changes() {
    let calendar = crate::calendar::Calendar::system().unwrap();
    let today = NaiveDate::from_ymd_opt(2026, 4, 22).unwrap();
    let start = calendar.day_start(today).unwrap();
    let tomorrow = calendar.day_start(today.succ_opt().unwrap()).unwrap();
    let mut store = Store::ephemeral().unwrap();
    let tx = store.transaction().unwrap();
    insert_events(
        &tx,
        &[mk_event(
            start.timestamp_millis() + 1000,
            "2026-04-22",
            "2026-04",
            Source::Claude,
            2.0,
        )],
    )
    .unwrap();
    tx.commit().unwrap();
    let state = AppState {
        time_window: TimeWindow::D1,
        trend_granularity: TrendGranularity::Daily,
        ..AppState::default()
    };
    let mut cache = DataCache::default();
    cache.refresh_at(&store, &state, start + chrono::Duration::hours(12));
    assert_eq!(cache.left.len(), 1);
    let generation = cache.status.generation;
    cache.refresh_at(&store, &state, tomorrow + chrono::Duration::hours(12));
    assert!(cache.refresh_error.is_none());
    assert!(cache.left.is_empty());
    assert!(cache.trend.is_empty());
    assert_eq!(cache.status.generation, generation);
}

#[test]
fn generation_changes_invalidate_rows_when_file_mtime_does_not_change() {
    let store = fixture_conn_with_events();
    let state = AppState {
        time_window: TimeWindow::All,
        ..AppState::default()
    };
    let mut cache = DataCache::default();
    cache.refresh_all(&store, &state);
    let old_mtime = cache.status.mtime_ns;
    assert!(!cache.left.is_empty());
    store.execute_batch("DELETE FROM events; INSERT OR REPLACE INTO meta(key,value) VALUES ('generation','77');").unwrap();
    cache.refresh_all(&store, &state);
    assert!(cache.refresh_error.is_none());
    assert!(cache.left.is_empty());
    assert_eq!(cache.status.mtime_ns, old_mtime);
    assert_eq!(cache.status.generation, 77);
}

#[test]
fn failed_scope_refresh_keeps_the_data_and_its_previous_scope() {
    let store = fixture_conn_with_events();
    let mut state = AppState {
        current_section: Section::Days,
        time_window: TimeWindow::All,
        ..AppState::default()
    };
    let mut cache = DataCache::default();
    cache.refresh_all(&store, &state);
    let cost = cache.left.iter().map(|row| row.cost).sum::<f64>();
    store.execute_batch("DROP TABLE events").unwrap();
    state.current_section = Section::Models;
    state.time_window = TimeWindow::D1;
    cache.refresh_all(&store, &state);
    assert!(cache.refresh_error.is_some());
    assert_eq!(
        cache.loaded_state.as_ref().unwrap().current_section,
        Section::Days
    );
    cache.restore_loaded_state(&mut state);
    assert_eq!(state.current_section, Section::Days);
    assert_eq!(state.time_window, TimeWindow::All);
    assert_eq!(cache.left.iter().map(|row| row.cost).sum::<f64>(), cost);
    assert!(state.flash.as_ref().unwrap().contains("previous scope"));
}

#[test]
fn malformed_bucket_labels_return_an_error() {
    for granularity in [
        TrendGranularity::Daily,
        TrendGranularity::Weekly,
        TrendGranularity::Monthly,
        TrendGranularity::Yearly,
    ] {
        assert!(bucket_bounds("invalid", granularity).is_err());
    }
}

#[test]
fn widening_the_global_window_keeps_the_bucket_and_releases_the_previous_window() {
    let calendar = crate::calendar::Calendar::system().unwrap();
    let mut store = Store::ephemeral().unwrap();
    let mut events = Vec::new();
    for (day, cost) in [
        ("2026-04-01", 1.0),
        ("2026-04-22", 2.0),
        ("2026-04-30", 3.0),
        ("2026-05-01", 50.0),
    ] {
        let date = NaiveDate::parse_from_str(day, "%Y-%m-%d").unwrap();
        let ts = calendar.day_start(date).unwrap().timestamp_millis() + 1000;
        let mut row = mk_event(ts, day, &day[..7], Source::Claude, cost);
        row.session_id = "repeat".into();
        events.push(row);
    }
    let tx = store.transaction().unwrap();
    insert_events(&tx, &events).unwrap();
    tx.commit().unwrap();
    let now = calendar
        .day_start(NaiveDate::from_ymd_opt(2026, 4, 22).unwrap())
        .unwrap();
    let mut state = AppState {
        time_window: TimeWindow::D1,
        trend_granularity: TrendGranularity::Monthly,
        sort: Sort::AlphaAsc,
        ..AppState::default()
    };
    let mut cache = DataCache::default();
    cache.refresh_at(&store, &state, now);
    assert_eq!(cache.visible(&state).left[0].key, "2026-04");
    let drill = crate::tui::input::drill_target_for_current(&state, &cache).unwrap();
    state.push_drill(drill);
    cache.refresh_at(&store, &state, now);
    assert_eq!(cache.sessions[0].cost, 5.0);
    state.apply(crate::tui::state::Action::SetWindow(TimeWindow::All));
    cache.refresh_at(&store, &state, now);
    assert!(cache.refresh_error.is_none());
    assert_eq!(cache.sessions[0].cost, 6.0);
}

#[test]
fn failed_rollover_preserves_resolved_scope_and_does_not_retry_every_tick() {
    let calendar = crate::calendar::Calendar::system().unwrap();
    let date = NaiveDate::from_ymd_opt(2026, 4, 22).unwrap();
    let start = calendar.day_start(date).unwrap();
    let tomorrow = calendar.day_start(date.succ_opt().unwrap()).unwrap();
    let mut store = Store::ephemeral().unwrap();
    let tx = store.transaction().unwrap();
    insert_events(
        &tx,
        &[mk_event(
            start.timestamp_millis() + 1000,
            "2026-04-22",
            "2026-04",
            Source::Claude,
            2.0,
        )],
    )
    .unwrap();
    tx.commit().unwrap();
    let state = AppState {
        time_window: TimeWindow::D1,
        ..AppState::default()
    };
    let mut cache = DataCache::default();
    cache.refresh_at(&store, &state, start);
    let scope = cache.loaded_scope.clone();
    store.execute_batch("DROP TABLE events").unwrap();
    assert!(cache.calendar_changed(tomorrow));
    cache.refresh_at(&store, &state, tomorrow);
    assert!(cache.refresh_error.is_some());
    assert_eq!(cache.loaded_scope, scope);
    assert_eq!(cache.status.last_query, start);
    assert!(!cache.calendar_changed(tomorrow + chrono::Duration::seconds(1)));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(180, 24)).unwrap();
    terminal
        .draw(|frame| crate::tui::view::draw(frame, &state, &cache))
        .unwrap();
    let screen = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(screen.contains("snapshot 2026-04-22"));
}

#[test]
fn configured_roots_scope_root_rows_provider_totals_and_session_event_drills() {
    let calendar = crate::calendar::Calendar::system().unwrap();
    let now = calendar
        .day_start(NaiveDate::from_ymd_opt(2026, 4, 22).unwrap())
        .unwrap();
    let mut store = Store::ephemeral().unwrap();
    let events = [
        ("/configured/session.jsonl", 2.0),
        ("/outside/session.jsonl", 20.0),
        ("/configured-other/session.jsonl", 200.0),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (path, cost))| {
        let mut row = mk_event(
            now.timestamp_millis() + index as i64 + 1,
            "2026-04-22",
            "2026-04",
            Source::Claude,
            cost,
        );
        row.file_path = path.into();
        row.session_id = "repeat".into();
        row
    })
    .collect::<Vec<_>>();
    let tx = store.transaction().unwrap();
    insert_events(&tx, &events).unwrap();
    tx.commit().unwrap();
    let mut state = AppState {
        time_window: TimeWindow::All,
        ..AppState::default()
    };
    let mut cache = DataCache::default();
    cache.refresh_at(&store, &state, now);
    assert_eq!(cache.left.iter().map(|row| row.cost).sum::<f64>(), 222.0);
    cache.source_roots = Some(vec!["/configured".into()]);
    cache.refresh_at(&store, &state, now);
    assert!(cache.refresh_error.is_none());
    assert_eq!(cache.left.iter().map(|row| row.cost).sum::<f64>(), 2.0);
    assert_eq!(
        cache.trend.iter().map(|row| row.total_cost).sum::<f64>(),
        2.0
    );
    assert_eq!(cache.sparkline.iter().sum::<f64>(), 2.0);
    let drill = crate::tui::input::drill_target_for_current(&state, &cache).unwrap();
    state.push_drill(drill);
    cache.refresh_at(&store, &state, now);
    assert_eq!(cache.sessions.len(), 1);
    assert_eq!(cache.sessions[0].cost, 2.0);
    let drill = crate::tui::input::drill_target_for_current(&state, &cache).unwrap();
    state.push_drill(drill);
    cache.refresh_at(&store, &state, now);
    assert_eq!(cache.events.len(), 1);
    assert_eq!(cache.events[0].cost, 2.0);
    assert_eq!(
        cache.loaded_scope.as_ref().unwrap().roots,
        cache.source_roots
    );
    state.apply(crate::tui::state::Action::JumpToSection(Section::Provider));
    cache.refresh_at(&store, &state, now);
    assert_eq!(
        cache.trend.iter().map(|row| row.total_cost).sum::<f64>(),
        2.0
    );
    cache.source_roots = None;
    cache.refresh_at(&store, &state, now);
    assert_eq!(
        cache.trend.iter().map(|row| row.total_cost).sum::<f64>(),
        222.0
    );
}

#[test]
fn retained_provenance_diagnostics_follow_the_loaded_dashboard_scope() {
    let mut store = Store::ephemeral().unwrap();
    let tx = store.transaction().unwrap();
    insert_events(
        &tx,
        &[crate::test_support::event_row(
            1,
            "old",
            None,
            1,
            2.0,
            Source::Claude,
        )],
    )
    .unwrap();
    tx.execute(
        "INSERT INTO files(path,source,size,mtime_ns) VALUES('/old.jsonl','claude',1,1)",
        [],
    )
    .unwrap();
    tx.commit().unwrap();
    let mut state = AppState {
        time_window: TimeWindow::All,
        ..AppState::default()
    };
    let mut cache = DataCache {
        current_file_errors: 1,
        ..DataCache::default()
    };
    cache.refresh_all(&store, &state);
    assert!(cache.refresh_error.is_none());
    assert_eq!(cache.diagnostics.legacy_cost_rows, 1);
    assert_eq!(cache.diagnostics.stale_parser_files, 1);
    assert_eq!(cache.diagnostics.current_file_errors, 1);
    state.source_filter = crate::tui::state::SourceFilter::Codex;
    cache.refresh_all(&store, &state);
    assert!(cache.refresh_error.is_none());
    assert_eq!(cache.diagnostics.legacy_cost_rows, 0);
    assert_eq!(cache.diagnostics.stale_parser_files, 0);
    assert_eq!(cache.diagnostics.current_file_errors, 1);
}

#[test]
fn refresh_and_scope_changes_preserve_cursor_sync_failure() {
    let store = Store::ephemeral().unwrap();
    let mut state = AppState {
        time_window: TimeWindow::All,
        ..AppState::default()
    };
    let failure = "Cursor sync failed. Using cached exports: synthetic failure";
    let mut cache = DataCache {
        cursor_sync_failure: Some(failure.into()),
        ..DataCache::default()
    };
    cache.refresh_all(&store, &state);
    assert!(cache.refresh_error.is_none());
    assert_eq!(cache.cursor_sync_failure.as_deref(), Some(failure));
    state.source_filter = crate::tui::state::SourceFilter::Claude;
    cache.refresh_all(&store, &state);
    assert!(cache.refresh_error.is_none());
    assert_eq!(
        cache.visible(&state).cursor_sync_failure.as_deref(),
        Some(failure)
    );
}

fn model_event(ts_ms: i64, session: &str, source: Source, model: &str, cost: f64) -> EventRow {
    let mut row = mk_event(ts_ms, "2026-04-22", "2026-04", source, cost);
    row.session_id = session.into();
    row.model = model.into();
    row
}

fn models_store() -> Store {
    let mut store = mk_conn();
    let tx = store.transaction().unwrap();
    insert_events(
        &tx,
        &[
            model_event(
                1_000,
                "dated",
                Source::Claude,
                "claude-opus-4-7-20250101",
                3.0,
            ),
            model_event(2_000, "plain", Source::Claude, "claude-opus-4-7", 2.0),
            model_event(3_000, "sonnet", Source::Claude, "claude-sonnet-4-6", 1.0),
            model_event(4_000, "mystery", Source::Claude, "mystery-model", 0.0),
        ],
    )
    .unwrap();
    tx.commit().unwrap();
    store
}

fn models_state(by_family: bool) -> AppState {
    AppState {
        current_section: Section::Models,
        time_window: TimeWindow::All,
        models_by_family: by_family,
        ..AppState::default()
    }
}

#[test]
fn models_section_groups_by_normalized_id_and_marks_unpriced() {
    let store = models_store();
    let mut cache = DataCache::default();
    cache.refresh_all(&store, &models_state(false));
    assert!(cache.refresh_error.is_none());
    let keys: Vec<_> = cache.left.iter().map(|row| row.key.as_str()).collect();
    assert_eq!(
        keys,
        ["claude-opus-4-7", "claude-sonnet-4-6", "mystery-model"]
    );
    let opus = cache.left[0].model_stats.as_ref().unwrap();
    assert_eq!(cache.left[0].cost, 5.0);
    assert_eq!(
        opus.members,
        ["claude-opus-4-7", "claude-opus-4-7-20250101"]
    );
    assert!(opus.priced);
    assert!(!cache.left[2].model_stats.as_ref().unwrap().priced);
    assert!(cache.left_previous_cost.is_none());
}

#[test]
fn models_family_toggle_regroups_and_gets_its_own_memo() {
    let store = models_store();
    let mut cache = DataCache::default();
    cache.refresh_all(&store, &models_state(false));
    cache.refresh_all(&store, &models_state(true));
    let keys: Vec<_> = cache.left.iter().map(|row| row.key.as_str()).collect();
    assert_eq!(keys, ["claude-opus", "claude-sonnet", "mystery-model"]);
    assert_eq!(cache.left_memo.len(), 2);
}

#[test]
fn models_drill_includes_sessions_of_every_raw_id_in_the_group() {
    let store = models_store();
    let mut cache = DataCache::default();
    let mut state = models_state(false);
    cache.refresh_all(&store, &state);
    let drill = crate::tui::input::drill_target_for_current(&state, &cache).unwrap();
    assert_eq!(drill.label, "claude-opus-4-7");
    state.push_drill(drill);
    cache.refresh_all(&store, &state);
    let mut sessions: Vec<_> = cache
        .sessions
        .iter()
        .map(|s| s.session_id.as_str())
        .collect();
    sessions.sort_unstable();
    assert_eq!(sessions, ["dated", "plain"]);
}

#[test]
fn models_section_reports_the_previous_window_cost() {
    let now: DateTime<Utc> = "2026-04-22T12:00:00Z".parse().unwrap();
    let day = 86_400_000;
    let mut store = mk_conn();
    let tx = store.transaction().unwrap();
    insert_events(
        &tx,
        &[
            model_event(
                now.timestamp_millis() - day,
                "now",
                Source::Claude,
                "claude-opus-4-7",
                6.0,
            ),
            model_event(
                now.timestamp_millis() - 9 * day,
                "before",
                Source::Claude,
                "claude-opus-4-7",
                4.0,
            ),
        ],
    )
    .unwrap();
    tx.commit().unwrap();
    let state = AppState {
        time_window: TimeWindow::D7,
        ..models_state(false)
    };
    let mut cache = DataCache::default();
    cache.refresh_at(&store, &state, now);
    assert!(cache.refresh_error.is_none());
    assert_eq!(cache.left.iter().map(|r| r.cost).sum::<f64>(), 6.0);
    assert_eq!(cache.left_previous_cost, Some(4.0));
}

#[test]
fn model_trend_has_one_zero_filled_column_per_day_so_rows_line_up() {
    let now: DateTime<Utc> = "2026-04-22T12:00:00Z".parse().unwrap();
    let day = 86_400_000;
    let mut store = mk_conn();
    let tx = store.transaction().unwrap();
    insert_events(
        &tx,
        &[
            model_event(
                now.timestamp_millis(),
                "a-today",
                Source::Claude,
                "claude-opus-4-7",
                2.0,
            ),
            model_event(
                now.timestamp_millis() - 5 * day,
                "a-old",
                Source::Claude,
                "claude-opus-4-7",
                1.0,
            ),
            model_event(
                now.timestamp_millis() - 2 * day,
                "b-mid",
                Source::Claude,
                "claude-sonnet-4-6",
                3.0,
            ),
        ],
    )
    .unwrap();
    tx.commit().unwrap();
    store
        .execute_batch(
            "UPDATE events SET day = strftime('%Y-%m-%d', ts / 1000, 'unixepoch', 'localtime')",
        )
        .unwrap();
    let state = AppState {
        time_window: TimeWindow::D7,
        ..models_state(false)
    };
    let mut cache = DataCache::default();
    cache.refresh_at(&store, &state, now);
    let trend = |key: &str| {
        cache
            .left
            .iter()
            .find(|row| row.key == key)
            .and_then(|row| row.model_stats.as_ref())
            .unwrap()
            .daily_cost
            .clone()
    };
    assert_eq!(
        trend("claude-opus-4-7"),
        [0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 2.0]
    );
    assert_eq!(
        trend("claude-sonnet-4-6"),
        [0.0, 0.0, 0.0, 0.0, 3.0, 0.0, 0.0]
    );
}

#[test]
fn long_windows_sum_days_into_equal_buckets() {
    let today = NaiveDate::from_ymd_opt(2026, 4, 22).unwrap();
    let days = vec![(today, 1.0), (today - chrono::Duration::days(29), 2.0)];
    let out = super::bucket_daily_cost(&days, today, 30);
    assert_eq!(out.len(), 14);
    assert_eq!(out[0], 2.0);
    assert_eq!(out[13], 1.0);
    assert_eq!(out.iter().sum::<f64>(), 3.0);
}
