use ratatui::layout::Rect;

use crate::tui::data::DataCache;
use crate::tui::state::{AppState, DrillKind, Section, SourceFilter};
pub(crate) const BAR_WIDTH: usize = 20;
pub(super) const LEFT_NAME_WIDTH: u16 = 28;
pub(super) const SESSION_PROJECT_WIDTH: u16 = 28;
pub(super) const TREND_BUCKET_WIDTH: u16 = 20;
pub(super) const PROVIDER_COL_WIDTH: u16 = 10;
pub(super) const SIDEBAR_WIDTH: u16 = 22;
const PANEL_BORDER: u16 = 2;
// 1 top border + 1 top pad + 1 bottom border.
const PANEL_CHROME_HEIGHT: u16 = 3;

// -- Main panel sizing ------------------------------------------------------

/// Returns the (width, height) the main bordered panel "wants" given the
/// currently active section and the data in the cache. The caller clamps
/// these to the available terminal size.
pub(crate) fn main_panel_dimensions(state: &AppState, cache: &DataCache) -> (u16, u16) {
    let bar_w = BAR_WIDTH as u16 + 2;

    // Drilled view always renders with a 1-row breadcrumb; pick columns
    // based on which kind of drill is on top of the stack.
    if let Some(d) = state.deepest_drill() {
        let cells: Vec<u16> = match d.kind {
            DrillKind::Sessions { .. } => {
                vec![12, 6, SESSION_PROJECT_WIDTH, 8, 10, bar_w]
            }
            DrillKind::Events { .. } => events_column_widths(bar_w),
        };
        let rows = match d.kind {
            DrillKind::Sessions { .. } => cache.sessions.len() as u16,
            DrillKind::Events { .. } => cache.events.len() as u16,
        };
        return chrome((table_width(&cells), 1 + 1 + rows.max(1)));
    }

    let tab = state.active_tab_index() as usize;
    let (cols, content_h) = match state.current_section {
        Section::Repos if tab == 0 => left_panel_size(state, cache),
        Section::Days | Section::Models => left_panel_size(state, cache),
        Section::Sessions => {
            let cells = [12u16, 6, SESSION_PROJECT_WIDTH, 8, 10, bar_w];
            let rows = cache.sessions.len() as u16;
            (table_width(&cells), 1 + rows.max(1))
        }
        Section::Repos | Section::Provider => {
            let n = active_provider_count(state, cache) as usize;
            let mut cells: Vec<u16> = vec![TREND_BUCKET_WIDTH];
            cells.extend(std::iter::repeat_n(PROVIDER_COL_WIDTH, n));
            cells.extend([8u16, 10, bar_w]);
            let rows = cache.trend.len() as u16;
            // header + rows + separator + TOTAL
            (table_width(&cells), 1 + rows.max(1) + 2)
        }
    };

    chrome((cols, content_h))
}

/// Column widths used by the per-turn events table. Mirrors the order in
/// `draw_events_table` so the panel can size itself consistently.
fn events_column_widths(bar_w: u16) -> Vec<u16> {
    // when · model · in · out · cache_r · cache_w · cost · proportion
    vec![10, 16, 7, 7, 9, 9, 10, bar_w]
}

fn left_panel_size(state: &AppState, cache: &DataCache) -> (u16, u16) {
    let bar_w = BAR_WIDTH as u16 + 2;
    let rows = cache.left.len() as u16;
    let cells: &[u16] = if state.expanded {
        &[LEFT_NAME_WIDTH, 5, 8, 10, bar_w]
    } else {
        &[LEFT_NAME_WIDTH, 8, 10, bar_w]
    };
    (table_width(cells), 1 + rows.max(1))
}

/// Sum of fixed column widths plus 1 col of spacing between each pair
/// (matches ratatui's default `Table::column_spacing(1)`).
pub(super) fn table_width(cells: &[u16]) -> u16 {
    let sum: u16 = cells.iter().copied().sum();
    let gaps = cells.len().saturating_sub(1) as u16;
    sum + gaps
}

fn chrome((w, h): (u16, u16)) -> (u16, u16) {
    (w + PANEL_BORDER, h + PANEL_CHROME_HEIGHT)
}

fn active_provider_count(state: &AppState, cache: &DataCache) -> u16 {
    let rows = &cache.trend;
    let show_claude = rows.iter().any(|r| r.claude_cost > 0.001)
        && !matches!(
            state.source_filter,
            SourceFilter::Codex | SourceFilter::Cursor
        );
    let show_codex = rows.iter().any(|r| r.codex_cost > 0.001)
        && !matches!(
            state.source_filter,
            SourceFilter::Claude | SourceFilter::Cursor
        );
    let show_cursor = rows.iter().any(|r| r.cursor_cost > 0.001)
        && !matches!(
            state.source_filter,
            SourceFilter::Claude | SourceFilter::Codex
        );
    show_claude as u16 + show_codex as u16 + show_cursor as u16
}

pub(crate) fn pad_top(area: Rect, n: u16) -> Rect {
    if area.height <= n {
        return area;
    }
    Rect {
        x: area.x,
        y: area.y + n,
        width: area.width,
        height: area.height - n,
    }
}

/// Title for the main pane border. For the Days section the title reflects
/// the active granularity (DAYS / WEEKS / MONTHS / YEARS).
pub(crate) fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let w = width.min(area.width.saturating_sub(2));
    let h = height.min(area.height.saturating_sub(2));
    Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    }
}
