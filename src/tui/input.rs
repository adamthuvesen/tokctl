//! Keyboard routing helpers: drill targets, cursor clamp, yank.

use chrono::Local;

use crate::tui::data;
use crate::tui::state::{self, AppState, DrillKind, Section};

pub fn yank_key(state: &AppState, cache: &data::DataCache) -> Option<String> {
    let visible = cache.visible(state);
    let cache = &visible;
    match state.deepest_drill() {
        Some(d) => match d.kind {
            DrillKind::Sessions { .. } => cache
                .sessions
                .get(
                    state
                        .current_index()
                        .min(cache.sessions.len().saturating_sub(1)),
                )
                .map(|r| format!("{}:{}", r.source.as_str(), r.session_id)),
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
            Section::Provider | Section::Repos if state.is_provider_view() => cache
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
                .map(|r| format!("{}:{}", r.source.as_str(), r.session_id)),
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
    let visible = cache.visible(state);
    let cache = &visible;
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
            Section::Provider | Section::Repos if state.is_provider_view() => cache
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
    let visible = cache.visible(state);
    let index = state.current_index();
    let mut scope = state
        .deepest_drill()
        .map(|drill| drill.scope.clone())
        .unwrap_or_default();
    let (kind, key, label) = match state.next_drill_kind_hint()? {
        state::NextDrill::Sessions => {
            let row = visible.left.get(index)?;
            row.selection.apply_to(&mut scope);
            (
                DrillKind::Sessions {
                    from_section: state.current_section,
                },
                row.key.clone(),
                row.label.clone(),
            )
        }
        state::NextDrill::Events => {
            let row = visible.sessions.get(index)?;
            scope.session = Some(crate::store::queries::SessionKey {
                source: row.source,
                id: row.session_id.clone(),
            });
            (
                DrillKind::Events { source: row.source },
                row.session_id.clone(),
                short_session_label(&row.session_id),
            )
        }
    };
    Some(state::Drill {
        kind,
        key,
        label,
        cursor: 0,
        scope,
        parent_filter: Default::default(),
    })
}

pub fn clamp_cursor_to_visible_rows(state: &mut AppState, cache: &data::DataCache) {
    let visible = cache.visible(state);
    let cache = &visible;
    let rows = match state.deepest_drill().map(|d| d.kind) {
        Some(DrillKind::Sessions { .. }) => cache.sessions.len(),
        Some(DrillKind::Events { .. }) => cache.events.len(),
        None => match state.current_section {
            Section::Provider | Section::Repos if state.is_provider_view() => cache.trend.len(),
            Section::Sessions => cache.sessions.len(),
            _ => cache.left.len(),
        },
    };
    if !state.drill_active() && !state.section_index.contains_key(&state.current_section) {
        if let Some(saved) = state.section_selection.get(&state.current_section) {
            let keys = visible_keys(state, cache);
            let index = keys.iter().position(|key| key == saved).or_else(|| {
                (state.current_section == Section::Sessions)
                    .then(|| {
                        cache
                            .sessions
                            .iter()
                            .position(|row| &row.session_id == saved)
                    })
                    .flatten()
            });
            if let Some(index) = index {
                state.set_current_index(index);
            }
        }
    }
    state.set_current_index(state.current_index().min(rows.saturating_sub(1)));
    if !state.drill_active() {
        if let Some(key) = visible_keys(state, cache).get(state.current_index()) {
            state
                .section_selection
                .insert(state.current_section, key.clone());
        }
    }
}

fn visible_keys(state: &AppState, cache: &data::VisibleRows) -> Vec<String> {
    if state.is_provider_view() {
        cache.trend.iter().map(|row| row.bucket.clone()).collect()
    } else if state.current_section == Section::Sessions {
        cache
            .sessions
            .iter()
            .map(|row| format!("{}:{}", row.source.as_str(), row.session_id))
            .collect()
    } else {
        cache.left.iter().map(|row| row.key.clone()).collect()
    }
}

fn short_session_label(session_id: &str) -> String {
    let mut s: String = session_id.chars().take(8).collect();
    if session_id.chars().count() > 8 {
        s.push('…');
    }
    s
}

#[cfg(test)]
mod tests;
