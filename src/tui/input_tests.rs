use super::*;
use crate::store::queries::{QueryScope, RepoFilterSpec};
use crate::tui::data::{AggregateSelection, DataCache, LeftRow, SessionRow, TrendRow};
use crate::tui::state::{Action, FilterState, Sort, TimeWindow};
use crate::types::Source;

fn aggregate(label: &str, cost: f64) -> LeftRow {
    LeftRow {
        label: label.into(),
        key: format!("/repo/{label}"),
        sessions: 1,
        total_tokens: cost as u64,
        cost,
        is_no_repo: false,
        latest_ts: None,
        selection: AggregateSelection::Repo(RepoFilterSpec::KeyExact(format!("/repo/{label}"))),
    }
}

#[test]
fn filtered_highlight_copy_details_totals_and_drill_use_the_same_repo() {
    let mut state = AppState {
        current_section: Section::Repos,
        time_window: TimeWindow::All,
        ..AppState::default()
    };
    state.filter = FilterState {
        active: false,
        query: "beta".into(),
    };
    let cache = DataCache {
        left: vec![aggregate("alpha", 90.0), aggregate("beta", 10.0)],
        ..Default::default()
    };
    clamp_cursor_to_visible_rows(&mut state, &cache);
    let visible = cache.visible(&state);
    assert_eq!(visible.left[0].key, "/repo/beta");
    assert_eq!(yank_key(&state, &cache).as_deref(), Some("/repo/beta"));
    assert!(crate::tui::view::detail_lines(&state, &visible).contains(&"key: /repo/beta".into()));
    assert_eq!(crate::tui::view::header_scope(&state, &visible).0, 10.0);
    let drill = drill_target_for_current(&state, &cache).unwrap();
    assert_eq!(
        drill.scope.repo,
        Some(RepoFilterSpec::KeyExact("/repo/beta".into()))
    );
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(140, 24)).unwrap();
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
    assert!(screen.contains("▌ beta"));
    assert!(!screen.contains("alpha"));
    state.push_drill(drill);
    assert!(state.filter.query.is_empty());
    state.apply(Action::PopDrill);
    assert_eq!(state.filter.query, "beta");
}

#[test]
fn filtered_sessions_with_repeated_ids_keep_their_provider_identity() {
    let mut state = AppState {
        current_section: Section::Sessions,
        time_window: TimeWindow::All,
        ..AppState::default()
    };
    state.filter.query = "beta".into();
    let cache = DataCache {
        sessions: [Source::Claude, Source::Codex]
            .into_iter()
            .enumerate()
            .map(|(index, source)| SessionRow {
                session_id: "repeat".into(),
                source,
                latest_ts: chrono::Utc::now(),
                project: Some(if index == 0 { "alpha" } else { "beta" }.into()),
                cost: 1.0,
                total_tokens: 10,
            })
            .collect(),
        ..Default::default()
    };
    assert_eq!(yank_key(&state, &cache).as_deref(), Some("codex:repeat"));
    let drill = drill_target_for_current(&state, &cache).unwrap();
    assert_eq!(
        drill.scope.session.unwrap(),
        crate::store::queries::SessionKey {
            source: Source::Codex,
            id: "repeat".into()
        }
    );
}

#[test]
fn provider_sort_copy_details_and_repos_provider_tab_share_one_projection() {
    let mut state = AppState {
        current_section: Section::Provider,
        sort: Sort::CostAsc,
        ..AppState::default()
    };
    let cache = DataCache {
        trend: [("2026-01", 9.0), ("2026-02", 1.0)]
            .into_iter()
            .map(|(bucket, cost)| TrendRow {
                bucket: bucket.into(),
                claude_cost: cost,
                codex_cost: 0.0,
                cursor_cost: 0.0,
                total_tokens: 1,
                total_cost: cost,
                is_current: false,
            })
            .collect(),
        left: vec![aggregate("unrelated", 90.0)],
        ..Default::default()
    };
    assert_eq!(yank_key(&state, &cache).as_deref(), Some("2026-02"));
    assert_eq!(
        crate::tui::view::detail_lines(&state, &cache.visible(&state))[0],
        "bucket: 2026-02"
    );
    state.current_section = Section::Repos;
    state.tab_per_section.insert(Section::Repos, 1);
    assert_eq!(yank_key(&state, &cache).as_deref(), Some("2026-02"));
    assert!(!state.can_push_drill());
    assert!(drill_target_for_current(&state, &cache).is_none());
}

#[test]
fn event_drill_keeps_the_complete_parent_scope() {
    let mut state = AppState {
        current_section: Section::Models,
        time_window: TimeWindow::All,
        ..AppState::default()
    };
    let scope = QueryScope {
        model: Some("model-x".into()),
        repo: Some(RepoFilterSpec::KeyExact("/repo/exact".into())),
        since_ms: Some(123),
        until_ms: Some(456),
        ..Default::default()
    };
    state.push_drill(state::Drill {
        kind: DrillKind::Sessions {
            from_section: Section::Models,
        },
        key: "model-x".into(),
        label: "model-x".into(),
        cursor: 0,
        scope: scope.clone(),
        parent_filter: Default::default(),
    });
    let cache = DataCache {
        sessions: vec![SessionRow {
            session_id: "repeat".into(),
            source: Source::Codex,
            latest_ts: chrono::Utc::now(),
            project: None,
            cost: 1.0,
            total_tokens: 10,
        }],
        ..Default::default()
    };
    let drill = drill_target_for_current(&state, &cache).unwrap();
    assert_eq!(drill.scope.repo, scope.repo);
    assert_eq!(drill.scope.model, scope.model);
    assert_eq!(drill.scope.since_ms, scope.since_ms);
    assert_eq!(drill.scope.until_ms, scope.until_ms);
    assert_eq!(drill.scope.session.unwrap().source, Source::Codex);
}

#[test]
fn empty_filtered_results_disable_row_actions_and_clamp_the_cursor() {
    let mut state = AppState {
        current_section: Section::Repos,
        ..AppState::default()
    };
    state.set_current_index(50);
    state.filter.query = "missing".into();
    let cache = DataCache {
        left: vec![aggregate("alpha", 2.0)],
        ..Default::default()
    };
    clamp_cursor_to_visible_rows(&mut state, &cache);
    assert_eq!(state.current_index(), 0);
    assert!(yank_key(&state, &cache).is_none());
    assert!(yank_summary(&state, &cache).is_none());
    assert!(drill_target_for_current(&state, &cache).is_none());
}

#[test]
fn selected_identity_is_restored_from_v3_preferences() {
    let mut state = AppState {
        current_section: Section::Repos,
        ..AppState::default()
    };
    state
        .section_selection
        .insert(Section::Repos, "/repo/beta".into());
    let cache = DataCache {
        left: vec![aggregate("alpha", 90.0), aggregate("beta", 10.0)],
        ..Default::default()
    };
    clamp_cursor_to_visible_rows(&mut state, &cache);
    assert_eq!(state.current_index(), 1);
    assert_eq!(yank_key(&state, &cache).as_deref(), Some("/repo/beta"));
}

#[test]
fn query_identity_changes_for_scope_navigation_and_ignores_presentation_changes() {
    let cache = DataCache::default();
    let now = chrono::Utc::now();
    let mut state = AppState::default();
    let query = cache.query_for(&state, now).unwrap();
    state.apply(Action::CycleSort);
    state.apply(Action::ToggleExpand);
    state.filter.query = "query".into();
    assert_eq!(cache.query_for(&state, now).unwrap(), query);
    state.apply(Action::SetWindow(TimeWindow::All));
    assert_ne!(cache.query_for(&state, now).unwrap(), query);
    let query = cache.query_for(&state, now).unwrap();
    state.apply(Action::NextSection);
    assert_ne!(cache.query_for(&state, now).unwrap(), query);
}

#[test]
fn legacy_v3_session_selection_upgrades_to_a_provider_qualified_key() {
    let mut state = AppState {
        current_section: Section::Sessions,
        sort: Sort::AlphaAsc,
        ..AppState::default()
    };
    state
        .section_selection
        .insert(Section::Sessions, "beta".into());
    let cache = DataCache {
        sessions: [("alpha", Source::Claude), ("beta", Source::Codex)]
            .into_iter()
            .map(|(id, source)| SessionRow {
                session_id: id.into(),
                source,
                latest_ts: chrono::Utc::now(),
                project: None,
                cost: 1.0,
                total_tokens: 1,
            })
            .collect(),
        ..Default::default()
    };
    clamp_cursor_to_visible_rows(&mut state, &cache);
    assert_eq!(state.current_index(), 1);
    assert_eq!(
        state
            .section_selection
            .get(&Section::Sessions)
            .map(String::as_str),
        Some("codex:beta")
    );
}

#[test]
fn failed_refresh_keeps_local_filter_sort_cursor_and_detail_interactions_live() {
    let mut state = AppState {
        current_section: Section::Repos,
        time_window: TimeWindow::All,
        ..AppState::default()
    };
    let mut cache = DataCache {
        left: vec![aggregate("alpha", 90.0), aggregate("beta", 10.0)],
        loaded_state: Some(state.clone()),
        refresh_error: Some(data::RefreshError::new(
            data::RefreshScope::Left,
            "database unavailable",
        )),
        ..Default::default()
    };
    cache.restore_loaded_state(&mut state);
    state.apply(Action::CycleSort);
    state.filter.query = "beta".into();
    state.set_current_index(20);
    clamp_cursor_to_visible_rows(&mut state, &cache);
    state.apply(Action::ToggleDetail);
    assert_eq!(state.current_index(), 0);
    assert_eq!(state.sort, Sort::CostAsc);
    assert_eq!(yank_key(&state, &cache).as_deref(), Some("/repo/beta"));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(140, 24)).unwrap();
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
    assert!(screen.contains("key: /repo/beta"));
    assert!(!screen.contains("alpha"));
    assert!(cache.refresh_error.take().is_some());
}
