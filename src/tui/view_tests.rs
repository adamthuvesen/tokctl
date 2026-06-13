use chrono::Utc;

use super::{
    breadcrumb_title, context_text, detail_lines, display_session_rows, display_trend_rows,
    footer_messages, header_scope, provider_cost_text, render_bar, BAR_WIDTH,
};
use crate::tui::data::{
    CacheStatus, DataCache, EventRow, LeftRow, RefreshError, RefreshScope, SessionRow, TrendRow,
};
use crate::tui::state::{AppState, DrillKind, Section, Sort, SourceFilter, TimeWindow};
use crate::tui::theme::Palette;

fn cache() -> DataCache {
    DataCache {
        left: vec![LeftRow {
            label: "tokctl".into(),
            key: "/dev/tokctl".into(),
            sessions: 2,
            total_tokens: 1234,
            cost: 4.2,
            is_no_repo: false,
            latest_ts: None,
            source: None,
        }],
        sessions: vec![SessionRow {
            session_id: "session-abcdef".into(),
            source: crate::types::Source::Claude,
            latest_ts: Utc::now(),
            project: Some("tokctl".into()),
            cost: 1.2,
            total_tokens: 500,
        }],
        status: CacheStatus {
            cache_path: "/tmp/cache.db".into(),
            event_count: 7,
            freshness: "fresh 1m".into(),
            last_query: Utc::now(),
            mtime_ns: None,
        },
        ..Default::default()
    }
}

#[test]
fn context_includes_section_and_filters() {
    let state = AppState {
        current_section: Section::Provider,
        source_filter: SourceFilter::Cursor,
        time_window: TimeWindow::Month,
        sort: Sort::RecentDesc,
        ..AppState::default()
    };
    let text = context_text(&state, 200);
    assert!(text.contains("section:provider"));
    assert!(text.contains("source:cursor"));
    assert!(text.contains("sort:recent"));
}

#[test]
fn context_includes_tab_when_present() {
    let mut state = AppState {
        current_section: Section::Repos,
        ..AppState::default()
    };
    state.tab_per_section.insert(Section::Repos, 1);
    let text = context_text(&state, 200);
    assert!(text.contains("tab:provider"));
}

#[test]
fn context_truncates_to_width() {
    let text = context_text(&AppState::default(), 10);
    assert!(text.chars().count() <= 10);
    assert!(text.ends_with('…'));
}

#[test]
fn detail_lines_for_drilled_view_show_session_id() {
    let mut state = AppState {
        current_section: Section::Repos,
        ..AppState::default()
    };
    state.push_drill(crate::tui::state::Drill {
        kind: DrillKind::Sessions {
            from_section: Section::Repos,
        },
        key: "tokctl".into(),
        label: "tokctl".into(),
        cursor: 0,
    });
    let lines = detail_lines(&state, &cache());
    assert!(lines
        .iter()
        .any(|line: &String| line.contains("session-abcdef")));
}

#[test]
fn detail_lines_for_event_drill_show_model_and_when() {
    let mut state = AppState {
        current_section: Section::Sessions,
        ..AppState::default()
    };
    state.push_drill(crate::tui::state::Drill {
        kind: DrillKind::Events {
            source: crate::types::Source::Claude,
        },
        key: "abc".into(),
        label: "abc".into(),
        cursor: 0,
    });
    let mut c = cache();
    c.events = vec![EventRow {
        ts: Utc::now(),
        model: "claude-sonnet-4-6".into(),
        input: 100,
        output: 50,
        cache_read: 200,
        cache_write: 0,
        cost: 0.42,
    }];
    let lines = detail_lines(&state, &c);
    assert!(lines.iter().any(|l| l.contains("claude-sonnet-4-6")));
    assert!(lines.iter().any(|l| l.starts_with("session: abc")));
}

#[test]
fn footer_messages_include_refresh_error() {
    let mut c = cache();
    c.refresh_error = Some(RefreshError::new(
        RefreshScope::Left,
        "no such table: events",
    ));

    let messages = footer_messages(&AppState::default(), &c);

    assert!(messages.iter().any(|m| {
        m.is_error
            && m.text
                .contains("refresh failed: rows: no such table: events")
    }));
}

#[test]
fn display_session_rows_sorts_recent_globally() {
    let rows = vec![
        SessionRow {
            session_id: "expensive-old".into(),
            source: crate::types::Source::Claude,
            latest_ts: "2026-04-18T09:00:00Z".parse().unwrap(),
            project: Some("expensive-old".into()),
            cost: 100.0,
            total_tokens: 0,
        },
        SessionRow {
            session_id: "cheap-new".into(),
            source: crate::types::Source::Codex,
            latest_ts: "2026-04-19T09:00:00Z".parse().unwrap(),
            project: Some("cheap-new".into()),
            cost: 1.0,
            total_tokens: 0,
        },
    ];
    let state = AppState {
        current_section: Section::Sessions,
        sort: Sort::RecentDesc,
        ..AppState::default()
    };

    let shown = display_session_rows(&rows, &state);

    assert_eq!(shown[0].session_id, "cheap-new");
}

#[test]
fn display_trend_rows_honors_active_sort() {
    let rows = vec![
        TrendRow {
            bucket: "2025-10-14".into(),
            claude_cost: 0.0,
            codex_cost: 0.0,
            cursor_cost: 1.0,
            total_tokens: 10,
            total_cost: 1.0,
            is_current: false,
        },
        TrendRow {
            bucket: "2025-12-04".into(),
            claude_cost: 0.0,
            codex_cost: 0.0,
            cursor_cost: 12.0,
            total_tokens: 20,
            total_cost: 12.0,
            is_current: false,
        },
    ];

    assert_eq!(
        display_trend_rows(&rows, Sort::AlphaDesc)[0].bucket,
        "2025-12-04"
    );
    assert_eq!(
        display_trend_rows(&rows, Sort::CostAsc)[0].bucket,
        "2025-10-14"
    );
}

#[test]
fn breadcrumb_stacks_labels() {
    let mut state = AppState {
        current_section: Section::Repos,
        ..AppState::default()
    };
    state.push_drill(crate::tui::state::Drill {
        kind: DrillKind::Sessions {
            from_section: Section::Repos,
        },
        key: "tokctl".into(),
        label: "tokctl".into(),
        cursor: 0,
    });
    state.push_drill(crate::tui::state::Drill {
        kind: DrillKind::Events {
            source: crate::types::Source::Claude,
        },
        key: "72a0a659".into(),
        label: "72a0a659".into(),
        cursor: 0,
    });
    let title = breadcrumb_title(&state);
    assert_eq!(title, "REPOS › tokctl › 72a0a659");
}

#[test]
fn render_bar_fills_correctly() {
    let p = Palette::default();
    let bar = render_bar(1.0, 20, &p);
    let text: String = bar.spans.iter().map(|s| s.content.as_ref()).collect();
    assert_eq!(text, "█".repeat(20));

    let bar = render_bar(0.0, 20, &p);
    let text: String = bar.spans.iter().map(|s| s.content.as_ref()).collect();
    assert_eq!(text, "░".repeat(20));
}

#[test]
fn render_bar_spans_total_width() {
    let p = Palette::default();
    let bar = render_bar(0.5, BAR_WIDTH, &p);
    let total: usize = bar.spans.iter().map(|s| s.content.chars().count()).sum();
    assert_eq!(total, BAR_WIDTH);
}

#[test]
fn provider_missing_cost_aligns_with_cost_cells() {
    let missing = provider_cost_text(0.0);
    let value = provider_cost_text(1.23);

    assert_eq!(missing.chars().count(), value.chars().count());
    assert_eq!(missing, "         —");
    assert_eq!(value, "     $1.23");
}

fn header_cache() -> DataCache {
    DataCache {
        trend: vec![
            TrendRow {
                bucket: "2026-04".into(),
                claude_cost: 6.0,
                codex_cost: 3.0,
                cursor_cost: 1.0,
                total_tokens: 1_000,
                total_cost: 10.0,
                is_current: false,
            },
            TrendRow {
                bucket: "2026-05".into(),
                claude_cost: 4.0,
                codex_cost: 2.0,
                cursor_cost: 0.0,
                total_tokens: 600,
                total_cost: 6.0,
                is_current: true,
            },
        ],
        left: vec![
            LeftRow {
                label: "tokctl".into(),
                key: "/dev/tokctl".into(),
                sessions: 2,
                total_tokens: 1234,
                cost: 4.2,
                is_no_repo: false,
                latest_ts: None,
                source: None,
            },
            LeftRow {
                label: "other-repo".into(),
                key: "/dev/other-repo".into(),
                sessions: 1,
                total_tokens: 500,
                cost: 1.5,
                is_no_repo: false,
                latest_ts: None,
                source: None,
            },
        ],
        sessions: vec![
            SessionRow {
                session_id: "alpha".into(),
                source: crate::types::Source::Claude,
                latest_ts: Utc::now(),
                project: Some("tokctl".into()),
                cost: 2.0,
                total_tokens: 800,
            },
            SessionRow {
                session_id: "bravo".into(),
                source: crate::types::Source::Codex,
                latest_ts: Utc::now(),
                project: Some("zzz".into()),
                cost: 0.5,
                total_tokens: 200,
            },
        ],
        ..Default::default()
    }
}

#[test]
fn header_scope_default_sums_trend() {
    let state = AppState::default();
    let (cost, tokens, suffix) = header_scope(&state, &header_cache());
    assert_eq!(cost, 16.0);
    assert_eq!(tokens, 1_600);
    assert_eq!(suffix, "");
}

#[test]
fn header_scope_sessions_drill_sums_sessions_with_label() {
    let mut state = AppState::default();
    state.push_drill(crate::tui::state::Drill {
        kind: DrillKind::Sessions {
            from_section: Section::Repos,
        },
        key: "/dev/tokctl".into(),
        label: "tokctl".into(),
        cursor: 0,
    });
    let (cost, tokens, suffix) = header_scope(&state, &header_cache());
    assert_eq!(cost, 2.5);
    assert_eq!(tokens, 1_000);
    assert_eq!(suffix, " · tokctl");
}

#[test]
fn header_scope_events_drill_sums_event_token_columns() {
    let mut state = AppState {
        current_section: Section::Sessions,
        ..AppState::default()
    };
    state.push_drill(crate::tui::state::Drill {
        kind: DrillKind::Events {
            source: crate::types::Source::Claude,
        },
        key: "alpha".into(),
        label: "alpha".into(),
        cursor: 0,
    });
    let mut c = header_cache();
    c.events = vec![
        EventRow {
            ts: Utc::now(),
            model: "claude".into(),
            input: 100,
            output: 50,
            cache_read: 200,
            cache_write: 10,
            cost: 0.42,
        },
        EventRow {
            ts: Utc::now(),
            model: "claude".into(),
            input: 1,
            output: 2,
            cache_read: 3,
            cache_write: 4,
            cost: 0.08,
        },
    ];
    let (cost, tokens, suffix) = header_scope(&state, &c);
    assert!((cost - 0.50).abs() < 1e-9);
    assert_eq!(tokens, 100 + 50 + 200 + 10 + 1 + 2 + 3 + 4);
    assert_eq!(suffix, " · alpha");
}

#[test]
fn header_scope_fuzzy_filter_on_left_pane_shrinks_total() {
    let mut state = AppState {
        current_section: Section::Repos,
        focus: crate::tui::state::Focus::Main,
        ..AppState::default()
    };
    state.filter.query = "tokctl".into();
    let (cost, tokens, suffix) = header_scope(&state, &header_cache());
    assert_eq!(cost, 4.2);
    assert_eq!(tokens, 1_234);
    assert_eq!(suffix, " · filter:tokctl");
}

#[test]
fn header_scope_fuzzy_filter_inside_drill_appends_both_suffixes() {
    let mut state = AppState {
        focus: crate::tui::state::Focus::Main,
        ..AppState::default()
    };
    state.push_drill(crate::tui::state::Drill {
        kind: DrillKind::Sessions {
            from_section: Section::Repos,
        },
        key: "/dev/tokctl".into(),
        label: "tokctl".into(),
        cursor: 0,
    });
    state.filter.query = "tokctl".into();
    let (cost, tokens, suffix) = header_scope(&state, &header_cache());
    // Only the "tokctl"-project session matches.
    assert_eq!(cost, 2.0);
    assert_eq!(tokens, 800);
    assert_eq!(suffix, " · tokctl · filter:tokctl");
}
