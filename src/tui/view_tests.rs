use chrono::Utc;

use super::{
    breadcrumb_title, context_text, detail_lines, display_session_rows, display_trend_rows,
    footer_messages, render_bar, BAR_WIDTH,
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
