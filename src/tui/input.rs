//! Keyboard routing helpers: drill targets, cursor clamp, yank.

use chrono::Local;

use crate::tui::data;
use crate::tui::state::{self, AppState, DrillKind, Section};

pub fn yank_key(state: &AppState, cache: &data::DataCache) -> Option<String> {
    match state.deepest_drill() {
        Some(d) => match d.kind {
            DrillKind::Sessions { .. } => cache
                .sessions
                .get(
                    state
                        .current_index()
                        .min(cache.sessions.len().saturating_sub(1)),
                )
                .map(|r| r.session_id.clone()),
            DrillKind::Events { source } => cache
                .events
                .get(
                    state
                        .current_index()
                        .min(cache.events.len().saturating_sub(1)),
                )
                .map(|r| format!("{}/{}@{}", source.as_str(), d.key, r.ts.to_rfc3339())),
        },
        None => match state.current_section {
            Section::Provider => cache
                .trend
                .get(
                    state
                        .current_index()
                        .min(cache.trend.len().saturating_sub(1)),
                )
                .map(|r| r.bucket.clone()),
            Section::Sessions => cache
                .sessions
                .get(
                    state
                        .current_index()
                        .min(cache.sessions.len().saturating_sub(1)),
                )
                .map(|r| r.session_id.clone()),
            _ => cache
                .left
                .get(
                    state
                        .current_index()
                        .min(cache.left.len().saturating_sub(1)),
                )
                .map(|r| r.key.clone()),
        },
    }
}

pub fn yank_summary(state: &AppState, cache: &data::DataCache) -> Option<String> {
    match state.deepest_drill() {
        Some(d) => match d.kind {
            DrillKind::Sessions { .. } => cache
                .sessions
                .get(
                    state
                        .current_index()
                        .min(cache.sessions.len().saturating_sub(1)),
                )
                .map(|r| {
                    format!(
                        "{}:{} · {} · {} tokens · {}",
                        r.source.as_str(),
                        r.session_id,
                        r.project.clone().unwrap_or_else(|| "(unknown)".into()),
                        r.total_tokens,
                        crate::tui::format::fmt_cost(r.cost)
                    )
                }),
            DrillKind::Events { .. } => cache
                .events
                .get(
                    state
                        .current_index()
                        .min(cache.events.len().saturating_sub(1)),
                )
                .map(|r| {
                    let when = r.ts.with_timezone(&Local).format("%H:%M:%S");
                    format!(
                        "{} {} in={} out={} cost={}",
                        when,
                        r.model,
                        r.input,
                        r.output,
                        crate::tui::format::fmt_cost(r.cost)
                    )
                }),
        },
        None => match state.current_section {
            Section::Provider => cache
                .trend
                .get(
                    state
                        .current_index()
                        .min(cache.trend.len().saturating_sub(1)),
                )
                .map(|r| {
                    format!(
                        "{} · {} tokens · {}",
                        r.bucket,
                        r.total_tokens,
                        crate::tui::format::fmt_cost(r.total_cost)
                    )
                }),
            Section::Sessions => cache
                .sessions
                .get(
                    state
                        .current_index()
                        .min(cache.sessions.len().saturating_sub(1)),
                )
                .map(|r| {
                    format!(
                        "{}:{} · {} · {} tokens · {}",
                        r.source.as_str(),
                        r.session_id,
                        r.project.clone().unwrap_or_else(|| "(unknown)".into()),
                        r.total_tokens,
                        crate::tui::format::fmt_cost(r.cost)
                    )
                }),
            _ => cache
                .left
                .get(
                    state
                        .current_index()
                        .min(cache.left.len().saturating_sub(1)),
                )
                .map(|r| {
                    format!(
                        "{} · {} sessions · {} tokens · {}",
                        r.label,
                        r.sessions,
                        r.total_tokens,
                        crate::tui::format::fmt_cost(r.cost)
                    )
                }),
        },
    }
}

/// Build the next drill target from the focused row in the current view.
pub fn drill_target_for_current(state: &AppState, cache: &data::DataCache) -> Option<state::Drill> {
    let kind = state.next_drill_kind_hint()?;
    match kind {
        DrillKind::Sessions { from_section } => {
            if cache.left.is_empty() {
                return None;
            }
            let idx = state.current_index().min(cache.left.len() - 1);
            let row = &cache.left[idx];
            Some(state::Drill {
                kind: DrillKind::Sessions { from_section },
                key: row.key.clone(),
                label: row.label.clone(),
                cursor: 0,
            })
        }
        DrillKind::Events { .. } => match state.deepest_drill() {
            None => {
                if state.current_section == Section::Sessions {
                    if cache.sessions.is_empty() {
                        return None;
                    }
                    let idx = state.current_index().min(cache.sessions.len() - 1);
                    let row = &cache.sessions[idx];
                    Some(state::Drill {
                        kind: DrillKind::Events { source: row.source },
                        key: row.session_id.clone(),
                        label: short_session_label(&row.session_id),
                        cursor: 0,
                    })
                } else if cache.left.is_empty() {
                    None
                } else {
                    let idx = state.current_index().min(cache.left.len() - 1);
                    let row = &cache.left[idx];
                    let source = row.source.unwrap_or(crate::types::Source::Claude);
                    Some(state::Drill {
                        kind: DrillKind::Events { source },
                        key: row.key.clone(),
                        label: short_session_label(&row.key),
                        cursor: 0,
                    })
                }
            }
            Some(_sessions_drill) => {
                if cache.sessions.is_empty() {
                    return None;
                }
                let idx = state.current_index().min(cache.sessions.len() - 1);
                let row = &cache.sessions[idx];
                Some(state::Drill {
                    kind: DrillKind::Events { source: row.source },
                    key: row.session_id.clone(),
                    label: short_session_label(&row.session_id),
                    cursor: 0,
                })
            }
        },
    }
}

pub fn clamp_cursor_to_visible_rows(state: &mut AppState, cache: &data::DataCache) {
    let rows = match state.deepest_drill().map(|d| d.kind) {
        Some(DrillKind::Sessions { .. }) => cache.sessions.len(),
        Some(DrillKind::Events { .. }) => cache.events.len(),
        None => match state.current_section {
            Section::Provider => cache.trend.len(),
            Section::Sessions => cache.sessions.len(),
            _ => cache.left.len(),
        },
    };
    if rows == 0 {
        state.set_current_index(0);
        return;
    }
    let cur = state.current_index();
    if cur >= rows {
        state.set_current_index(rows - 1);
    }
}

fn short_session_label(session_id: &str) -> String {
    let mut s: String = session_id.chars().take(8).collect();
    if session_id.chars().count() > 8 {
        s.push('…');
    }
    s
}
