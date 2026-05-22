use super::*;
use tempfile::TempDir;

#[test]
fn section_next_wraps() {
    assert_eq!(Section::Sessions.next(), Section::Days);
    assert_eq!(Section::Provider.next(), Section::Repos);
}

#[test]
fn section_prev_wraps() {
    assert_eq!(Section::Days.prev(), Section::Sessions);
    assert_eq!(Section::Repos.prev(), Section::Provider);
}

#[test]
fn repos_section_has_two_tabs() {
    assert_eq!(Section::Repos.tabs(), &["Costs", "Provider"]);
    assert!(Section::Days.tabs().is_empty());
}

#[test]
fn rows_drill_to_sessions_excludes_provider_and_sessions() {
    assert!(Section::Repos.rows_drill_to_sessions());
    assert!(Section::Days.rows_drill_to_sessions());
    assert!(Section::Models.rows_drill_to_sessions());
    // Sessions rows drill *to events*, not to a sessions list.
    assert!(!Section::Sessions.rows_drill_to_sessions());
    assert!(!Section::Provider.rows_drill_to_sessions());
}

#[test]
fn sort_cycle_pairs_desc_then_asc() {
    let mut sort = Sort::CostDesc;
    let mut seen = Vec::new();
    for _ in 0..6 {
        seen.push(sort.as_str());
        sort = sort.next();
    }

    assert_eq!(
        seen,
        vec!["cost↓", "cost↑", "recent↓", "recent↑", "alpha↓", "alpha↑"]
    );
    assert_eq!(sort, Sort::CostDesc);
}

#[test]
fn next_section_action_changes_section_and_marks_dirty() {
    let mut s = AppState::default();
    let out = s.apply(Action::NextSection);
    assert_eq!(s.current_section, Section::Models);
    assert!(out.dirty);
    assert!(out.needs_refresh);
}

#[test]
fn cycle_tab_advances_repos_tab() {
    let mut s = AppState {
        current_section: Section::Repos,
        sidebar_index: Section::ALL
            .iter()
            .position(|s| *s == Section::Repos)
            .unwrap_or(0),
        ..AppState::default()
    };
    assert_eq!(s.active_tab_index(), 0);
    let out = s.apply(Action::CycleTab);
    assert_eq!(s.active_tab_index(), 1);
    assert!(out.dirty);
    // wraps
    s.apply(Action::CycleTab);
    assert_eq!(s.active_tab_index(), 0);
}

#[test]
fn cycle_tab_noop_for_single_lens_section() {
    let mut s = AppState {
        current_section: Section::Days,
        ..AppState::default()
    };
    let out = s.apply(Action::CycleTab);
    assert!(!out.dirty);
    assert_eq!(s.active_tab_index(), 0);
}

#[test]
fn drill_set_and_pop() {
    let mut s = AppState {
        current_section: Section::Repos,
        ..AppState::default()
    };
    s.push_drill(Drill {
        kind: DrillKind::Sessions {
            from_section: Section::Repos,
        },
        key: "tokctl".into(),
        label: "tokctl".into(),
        cursor: 0,
    });
    assert_eq!(s.drill_depth(), 1);
    let out = s.apply(Action::PopDrill);
    assert_eq!(s.drill_depth(), 0);
    assert!(out.dirty);
}

#[test]
fn jump_to_provider_switches_section() {
    let mut s = AppState::default();
    let out = s.apply(Action::JumpToSection(Section::Provider));
    assert_eq!(s.current_section, Section::Provider);
    assert!(out.dirty);
}

#[test]
fn pop_cascades_drill_then_focus() {
    let mut s = AppState {
        current_section: Section::Repos,
        ..AppState::default()
    };
    s.push_drill(Drill {
        kind: DrillKind::Sessions {
            from_section: Section::Repos,
        },
        key: "x".into(),
        label: "x".into(),
        cursor: 0,
    });
    s.apply(Action::Pop);
    assert_eq!(s.drill_depth(), 0);
    assert_eq!(s.focus, Focus::Main);
    s.apply(Action::Pop);
    assert_eq!(s.focus, Focus::Sidebar);
}

#[test]
fn move_in_sidebar_changes_section() {
    let mut s = AppState {
        focus: Focus::Sidebar,
        ..AppState::default()
    };
    s.apply(Action::MoveDown);
    assert_eq!(s.current_section, Section::Models);
    s.apply(Action::MoveUp);
    assert_eq!(s.current_section, Section::Days);
}

#[test]
fn sidebar_move_only_refreshes_left() {
    let mut s = AppState {
        focus: Focus::Sidebar,
        ..AppState::default()
    };
    let out = s.apply(Action::MoveDown);
    assert_eq!(s.current_section, Section::Models);
    assert!(out.needs_refresh, "section change triggers a refresh");
    assert!(out.refresh.left, "left rebinds to the new section's data");
    // Trend / sparkline / sessions / events don't depend on section.
    assert!(!out.refresh.sessions);
    assert!(!out.refresh.trend);
    assert!(!out.refresh.sparkline);
    assert!(!out.refresh.events);
}

#[test]
fn sidebar_bottom_jumps_and_refreshes_left_only() {
    let mut s = AppState {
        focus: Focus::Sidebar,
        ..AppState::default()
    };
    let out = s.apply(Action::Bottom);
    assert_eq!(s.current_section, Section::Sessions);
    assert!(out.needs_refresh);
    assert!(out.refresh.left);
    assert!(!out.refresh.trend);
    assert!(!out.refresh.sparkline);
}

#[test]
fn roundtrip_persists_fields() {
    let dir = TempDir::new().unwrap();
    let p = dir.path().join("ui_state.json");
    let mut s = AppState {
        current_section: Section::Provider,
        time_window: TimeWindow::Today,
        source_filter: SourceFilter::Claude,
        trend_granularity: TrendGranularity::Weekly,
        seen_v3_intro: true,
        ..AppState::default()
    };
    s.tab_per_section.insert(Section::Repos, 1);
    s.section_selection
        .insert(Section::Repos, "tokctl".to_owned());
    save(&p, &s).unwrap();
    let loaded = load(&p);
    // current_section is intentionally not restored — TUI always boots on Days.
    assert_eq!(loaded.current_section, Section::Days);
    assert_eq!(loaded.time_window, TimeWindow::Today);
    assert_eq!(loaded.source_filter, SourceFilter::Claude);
    assert_eq!(loaded.trend_granularity, TrendGranularity::Weekly);
    assert_eq!(loaded.tab_per_section.get(&Section::Repos), Some(&1u8));
    assert_eq!(
        loaded
            .section_selection
            .get(&Section::Repos)
            .map(String::as_str),
        Some("tokctl")
    );
    assert!(loaded.seen_v3_intro);
}

#[test]
fn load_accepts_legacy_trend_section_name() {
    let dir = TempDir::new().unwrap();
    let p = dir.path().join("ui_state.json");
    std::fs::write(
        &p,
        format!(
            r#"{{"version":{},"current_section":"trend","trend_granularity":"daily"}}"#,
            STATE_VERSION
        ),
    )
    .unwrap();
    let loaded = load(&p);
    // The legacy "trend" alias must not break deserialization of sibling fields,
    // even though current_section itself is no longer restored on load.
    assert_eq!(loaded.current_section, Section::Days);
    assert_eq!(loaded.trend_granularity, TrendGranularity::Daily);
}

#[test]
fn v2_file_resets_to_defaults() {
    let dir = TempDir::new().unwrap();
    let p = dir.path().join("ui_state.json");
    // Simulate an old v2 file shape.
    std::fs::write(
        &p,
        br#"{"version":2,"left_axis":"model","time_window":"today"}"#,
    )
    .unwrap();
    let loaded = load(&p);
    let default = AppState::default();
    assert_eq!(loaded.current_section, default.current_section);
    assert_eq!(loaded.time_window, default.time_window);
}

#[test]
fn unknown_version_is_ignored() {
    let dir = TempDir::new().unwrap();
    let p = dir.path().join("ui_state.json");
    std::fs::write(&p, br#"{"version":999,"current_section":"trend"}"#).unwrap();
    let loaded = load(&p);
    assert_eq!(loaded.current_section, AppState::default().current_section);
}

#[test]
fn missing_file_is_default() {
    let dir = TempDir::new().unwrap();
    let p = dir.path().join("nope.json");
    let loaded = load(&p);
    assert_eq!(loaded.time_window, TimeWindow::Month);
}

#[test]
fn detail_toggle_and_pop_preserve_section() {
    let mut s = AppState::default();
    s.section_index.insert(Section::Repos, 3);
    s.apply(Action::ToggleDetail);
    assert!(s.detail_open);
    s.apply(Action::Pop);
    assert!(!s.detail_open);
    assert_eq!(s.section_index.get(&Section::Repos), Some(&3));
}

#[test]
fn drill_is_not_persisted() {
    let dir = TempDir::new().unwrap();
    let p = dir.path().join("ui_state.json");
    let mut s = AppState {
        current_section: Section::Repos,
        ..AppState::default()
    };
    s.push_drill(Drill {
        kind: DrillKind::Sessions {
            from_section: Section::Repos,
        },
        key: "tokctl".into(),
        label: "tokctl".into(),
        cursor: 0,
    });
    save(&p, &s).unwrap();
    let loaded = load(&p);
    assert_eq!(loaded.drill_depth(), 0);
}

// ---- new tests for the drill stack ---------------------------------

#[test]
fn next_drill_kind_root_repos_is_sessions() {
    let s = AppState {
        current_section: Section::Repos,
        ..AppState::default()
    };
    assert!(matches!(
        s.next_drill_kind_hint(),
        Some(DrillKind::Sessions {
            from_section: Section::Repos
        })
    ));
}

#[test]
fn next_drill_kind_root_sessions_is_events() {
    let s = AppState {
        current_section: Section::Sessions,
        ..AppState::default()
    };
    assert!(matches!(
        s.next_drill_kind_hint(),
        Some(DrillKind::Events { .. })
    ));
}

#[test]
fn next_drill_kind_root_provider_is_none() {
    let s = AppState {
        current_section: Section::Provider,
        ..AppState::default()
    };
    assert!(s.next_drill_kind_hint().is_none());
}

#[test]
fn next_drill_kind_inside_sessions_drill_is_events() {
    let mut s = AppState {
        current_section: Section::Repos,
        ..AppState::default()
    };
    s.push_drill(Drill {
        kind: DrillKind::Sessions {
            from_section: Section::Repos,
        },
        key: "tokctl".into(),
        label: "tokctl".into(),
        cursor: 0,
    });
    assert!(matches!(
        s.next_drill_kind_hint(),
        Some(DrillKind::Events { .. })
    ));
}

#[test]
fn next_drill_kind_inside_events_is_terminal() {
    let mut s = AppState {
        current_section: Section::Sessions,
        ..AppState::default()
    };
    s.push_drill(Drill {
        kind: DrillKind::Events {
            source: crate::types::Source::Claude,
        },
        key: "abc".into(),
        label: "abc".into(),
        cursor: 0,
    });
    assert!(s.next_drill_kind_hint().is_none());
}

#[test]
fn esc_pops_one_level_at_a_time() {
    let mut s = AppState {
        current_section: Section::Repos,
        ..AppState::default()
    };
    s.push_drill(Drill {
        kind: DrillKind::Sessions {
            from_section: Section::Repos,
        },
        key: "tokctl".into(),
        label: "tokctl".into(),
        cursor: 0,
    });
    s.push_drill(Drill {
        kind: DrillKind::Events {
            source: crate::types::Source::Claude,
        },
        key: "abc".into(),
        label: "abc".into(),
        cursor: 0,
    });
    assert_eq!(s.drill_depth(), 2);
    s.apply(Action::PopDrill);
    assert_eq!(s.drill_depth(), 1);
    s.apply(Action::PopDrill);
    assert_eq!(s.drill_depth(), 0);
}

#[test]
fn drill_action_signals_correct_refresh() {
    let mut s = AppState {
        current_section: Section::Repos,
        ..AppState::default()
    };
    let out = s.apply(Action::Drill);
    assert!(out.refresh.sessions);
    assert!(!out.refresh.events);

    let mut s = AppState {
        current_section: Section::Sessions,
        ..AppState::default()
    };
    let out = s.apply(Action::Drill);
    assert!(out.refresh.events);
    assert!(!out.refresh.sessions);

    let mut s = AppState {
        current_section: Section::Provider,
        ..AppState::default()
    };
    let out = s.apply(Action::Drill);
    assert!(!out.refresh.events);
    assert!(!out.refresh.sessions);
}

#[test]
fn cursor_lives_inside_each_drill() {
    let mut s = AppState {
        current_section: Section::Repos,
        ..AppState::default()
    };
    s.push_drill(Drill {
        kind: DrillKind::Sessions {
            from_section: Section::Repos,
        },
        key: "tokctl".into(),
        label: "tokctl".into(),
        cursor: 0,
    });
    s.set_current_index(3);
    assert_eq!(s.current_index(), 3);
    s.push_drill(Drill {
        kind: DrillKind::Events {
            source: crate::types::Source::Claude,
        },
        key: "abc".into(),
        label: "abc".into(),
        cursor: 0,
    });
    // The new deepest drill starts at 0, even though the parent had cursor 3.
    assert_eq!(s.current_index(), 0);
    s.set_current_index(7);
    assert_eq!(s.current_index(), 7);
    // Pop, parent cursor restored.
    s.apply(Action::PopDrill);
    assert_eq!(s.current_index(), 3);
}

#[test]
fn source_filter_inside_events_drill_skips_event_refetch_with_flash() {
    let mut s = AppState {
        current_section: Section::Sessions,
        ..AppState::default()
    };
    s.push_drill(Drill {
        kind: DrillKind::Events {
            source: crate::types::Source::Claude,
        },
        key: "abc".into(),
        label: "abc".into(),
        cursor: 0,
    });
    let out = s.apply(Action::SetSource(SourceFilter::Codex));
    // Filter persisted; the events slice is NOT re-fetched (the drilled
    // session is fixed to one source). Parent slices that genuinely
    // depend on source still refresh so popping back is correct.
    assert_eq!(s.source_filter, SourceFilter::Codex);
    assert!(!out.refresh.events);
    assert!(!out.refresh.sessions);
    assert!(out.refresh.left);
    assert_eq!(
        s.flash.as_deref(),
        Some("source filter ignored inside session")
    );
    // Subsequent change does NOT re-flash.
    s.flash = None;
    let _ = s.apply(Action::SetSource(SourceFilter::Cursor));
    assert!(s.flash.is_none());
}

// ---- new matrix tests for refresh_mask_for ------------------------------

#[test]
fn toggle_expand_does_no_refresh() {
    let mut s = AppState::default();
    let out = s.apply(Action::ToggleExpand);
    assert!(s.expanded);
    assert!(!out.refresh.any());
    assert!(!out.needs_refresh);
    // Still dirty so the redraw happens.
    assert!(out.dirty);
}

#[test]
fn cycle_sort_at_root_only_refreshes_left() {
    let mut s = AppState {
        current_section: Section::Repos,
        ..AppState::default()
    };
    let out = s.apply(Action::CycleSort);
    assert!(out.refresh.left);
    assert!(!out.refresh.sessions);
    assert!(!out.refresh.events);
    assert!(!out.refresh.trend);
}

#[test]
fn cycle_sort_in_sessions_drill_refreshes_sessions_too() {
    let mut s = AppState {
        current_section: Section::Repos,
        ..AppState::default()
    };
    s.push_drill(Drill {
        kind: DrillKind::Sessions {
            from_section: Section::Repos,
        },
        key: "tokctl".into(),
        label: "tokctl".into(),
        cursor: 0,
    });
    let out = s.apply(Action::CycleSort);
    assert!(out.refresh.left);
    assert!(out.refresh.sessions);
    assert!(!out.refresh.events);
}

#[test]
fn cycle_sort_in_events_drill_refreshes_events_too() {
    let mut s = AppState {
        current_section: Section::Sessions,
        ..AppState::default()
    };
    s.push_drill(Drill {
        kind: DrillKind::Events {
            source: crate::types::Source::Claude,
        },
        key: "abc".into(),
        label: "abc".into(),
        cursor: 0,
    });
    let out = s.apply(Action::CycleSort);
    assert!(out.refresh.left);
    assert!(out.refresh.events);
    assert!(!out.refresh.sessions);
}

#[test]
fn set_window_at_root_refreshes_left_trend_sparkline_only() {
    let mut s = AppState::default();
    let out = s.apply(Action::SetWindow(TimeWindow::Today));
    assert!(out.refresh.left);
    assert!(out.refresh.trend);
    assert!(out.refresh.sparkline);
    assert!(!out.refresh.sessions);
    assert!(!out.refresh.events);
}

#[test]
fn set_window_in_sessions_drill_refreshes_sessions_too() {
    let mut s = AppState {
        current_section: Section::Repos,
        ..AppState::default()
    };
    s.push_drill(Drill {
        kind: DrillKind::Sessions {
            from_section: Section::Repos,
        },
        key: "tokctl".into(),
        label: "tokctl".into(),
        cursor: 0,
    });
    let out = s.apply(Action::SetWindow(TimeWindow::Today));
    assert!(out.refresh.left);
    assert!(out.refresh.trend);
    assert!(out.refresh.sparkline);
    assert!(out.refresh.sessions);
    assert!(!out.refresh.events);
}

#[test]
fn set_window_in_events_drill_refreshes_events_too() {
    let mut s = AppState {
        current_section: Section::Sessions,
        ..AppState::default()
    };
    s.push_drill(Drill {
        kind: DrillKind::Events {
            source: crate::types::Source::Claude,
        },
        key: "abc".into(),
        label: "abc".into(),
        cursor: 0,
    });
    let out = s.apply(Action::SetWindow(TimeWindow::Today));
    assert!(out.refresh.left);
    assert!(out.refresh.trend);
    assert!(out.refresh.sparkline);
    assert!(out.refresh.events);
    assert!(!out.refresh.sessions);
}

#[test]
fn set_granularity_refreshes_left_and_trend() {
    let mut s = AppState::default();
    let out = s.apply(Action::SetTrendGranularity(TrendGranularity::Daily));
    assert!(out.refresh.left);
    assert!(out.refresh.trend);
    assert!(!out.refresh.sparkline);
    assert!(!out.refresh.sessions);
}

#[test]
fn cursor_move_at_root_refreshes_nothing() {
    let mut s = AppState {
        focus: Focus::Main,
        ..AppState::default()
    };
    let out = s.apply(Action::MoveDown);
    assert!(!out.needs_refresh, "no SQL on plain cursor moves");
    assert!(out.dirty, "still need to redraw to move the highlight");
}

#[test]
fn refresh_action_invalidates_everything() {
    let mut s = AppState::default();
    let out = s.apply(Action::Refresh);
    assert!(out.refresh.left);
    assert!(out.refresh.sessions);
    assert!(out.refresh.events);
    assert!(out.refresh.trend);
    assert!(out.refresh.sparkline);
}

#[test]
fn pop_drill_refreshes_nothing() {
    let mut s = AppState {
        current_section: Section::Repos,
        ..AppState::default()
    };
    s.push_drill(Drill {
        kind: DrillKind::Sessions {
            from_section: Section::Repos,
        },
        key: "tokctl".into(),
        label: "tokctl".into(),
        cursor: 0,
    });
    let out = s.apply(Action::PopDrill);
    assert!(!out.needs_refresh);
    assert!(out.dirty);
}
