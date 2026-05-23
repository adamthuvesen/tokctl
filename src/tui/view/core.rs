use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::tui::data::DataCache;
use crate::tui::shell::{draw_main_frame, draw_sidebar};
use crate::tui::state::{AppState, DrillKind, Focus, Section};
use crate::tui::theme::Palette;
use crate::tui::widgets::filter::apply_filter_sessions;
use crate::tui::MIN_WIDTH;

use super::chrome::{draw_detail, draw_filter_prompt, draw_footer, draw_header, draw_help};
use super::layout::{main_panel_dimensions, pad_top, SIDEBAR_WIDTH};
use super::tables::{draw_events_table, draw_left_table, draw_sessions_table, draw_trend_table};

pub fn draw(frame: &mut Frame<'_>, state: &AppState, cache: &DataCache) {
    let palette = Palette::default();
    let area = frame.area();

    if area.width < MIN_WIDTH {
        let msg = Paragraph::new(Line::from(Span::styled(
            format!(
                "terminal too narrow — resize to ≥{} cols (current: {})",
                MIN_WIDTH, area.width
            ),
            palette.dim_text(),
        )));
        frame.render_widget(msg, area);
        return;
    }

    let (panel_w, panel_h) = main_panel_dimensions(state, cache);

    // Cap to available space; reserve 2 for header + 2 for footer.
    let body_avail = area.height.saturating_sub(4);
    // Sidebar needs 3 header rows + (n_sections - 1) * 2 + 1 per item = 12 rows
    // to show all sections without truncation. Panel content may be shorter.
    let sidebar_min_h = 3 + (Section::ALL.len() as u16 - 1) * 2 + 1;
    let body_height = panel_h.min(body_avail).max(sidebar_min_h);

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(body_height),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .split(area);

    draw_header(frame, outer[0], state, cache, &palette);

    // Sidebar fixed; main hugs content; remainder is empty filler.
    let main_avail = area.width.saturating_sub(SIDEBAR_WIDTH);
    let main_width = panel_w.min(main_avail);

    let body_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(SIDEBAR_WIDTH),
            Constraint::Length(main_width),
            Constraint::Min(0),
        ])
        .split(outer[1]);

    draw_sidebar(frame, body_cols[0], state, &palette);
    draw_main(frame, body_cols[1], state, cache, &palette);

    draw_footer(frame, outer[3], state, cache, &palette);

    if state.help_open {
        draw_help(frame, area, &palette);
    }

    if state.detail_open {
        draw_detail(frame, area, state, cache, &palette);
    }

    if state.filter.active {
        draw_filter_prompt(frame, area, state, &palette);
    }
}

// -- Main pane dispatch -----------------------------------------------------

fn draw_main(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    cache: &DataCache,
    palette: &Palette,
) {
    let focused = state.focus == Focus::Main;

    // Drilled view supersedes the section's normal renderer. Title is the
    // cumulative breadcrumb across every level on the stack.
    if state.drill_active() {
        let title = breadcrumb_title(state);
        let inner = draw_main_frame(frame, area, &title, &[], 0, focused, palette);
        draw_drill(frame, pad_top(inner, 1), state, cache, palette);
        return;
    }

    let title = main_pane_title(state);
    let tabs = state.current_section.tabs();
    let active_tab = state.active_tab_index() as usize;
    let inner = draw_main_frame(frame, area, &title, tabs, active_tab, focused, palette);
    let padded = pad_top(inner, 1);

    match state.current_section {
        Section::Repos => match active_tab {
            0 => draw_left_table(frame, padded, state, cache, palette),
            _ => draw_trend_table(frame, padded, state, cache, palette),
        },
        Section::Days | Section::Models => {
            draw_left_table(frame, padded, state, cache, palette);
        }
        Section::Sessions => {
            let (rows, _) = apply_filter_sessions(&cache.sessions, state);
            if rows.is_empty() {
                frame.render_widget(
                    Paragraph::new(Line::from(Span::styled(
                        "no rows in this window",
                        palette.dim_text(),
                    ))),
                    padded,
                );
            } else {
                draw_sessions_table(frame, padded, state, &rows, palette);
            }
        }
        Section::Provider => {
            draw_trend_table(frame, padded, state, cache, palette);
        }
    }
}

/// Inset the top of a rect by `n` rows for breathing space.
fn main_pane_title(state: &AppState) -> String {
    if state.current_section == Section::Days {
        match state.trend_granularity {
            crate::tui::state::TrendGranularity::Daily => "DAYS".into(),
            crate::tui::state::TrendGranularity::Weekly => "WEEKS".into(),
            crate::tui::state::TrendGranularity::Monthly => "MONTHS".into(),
            crate::tui::state::TrendGranularity::Yearly => "YEARS".into(),
        }
    } else {
        state.current_section.title().to_owned()
    }
}

// -- Breadcrumb / drill renderer --------------------------------------------

/// Build the cumulative breadcrumb title for the bordered panel:
/// `SECTION › label_1 [› label_2]`. Labels are kept short by the caller
/// (session ids are pre-truncated to ~8 chars). Long repo/day labels still
/// fit in practice; we don't ellipsize here, the panel border will clip.
pub(crate) fn breadcrumb_title(state: &AppState) -> String {
    let mut out = state.current_section.title().to_owned();
    for d in &state.drill_stack {
        out.push_str(" › ");
        out.push_str(&d.label);
    }
    out
}

fn draw_drill(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    cache: &DataCache,
    palette: &Palette,
) {
    if area.height < 2 {
        return;
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(area);

    // Breadcrumb hint row inside the panel — drives the eye to the back
    // affordance even when the title is long.
    let mut spans: Vec<Span<'static>> = vec![Span::styled(
        state.current_section.title(),
        palette.dim_text(),
    )];
    for d in &state.drill_stack {
        spans.push(Span::styled(" › ", palette.dim_text()));
        spans.push(Span::styled(
            d.label.clone(),
            Style::default().add_modifier(Modifier::BOLD),
        ));
    }
    spans.push(Span::raw("   "));
    spans.push(Span::styled("[esc/← back]", palette.dim_text()));
    frame.render_widget(Paragraph::new(Line::from(spans)), rows[0]);

    match state
        .deepest_drill()
        .map(|d| d.kind)
        .expect("drill_active true => deepest_drill some")
    {
        DrillKind::Sessions { .. } => {
            draw_sessions_table(frame, rows[1], state, &cache.sessions, palette);
        }
        DrillKind::Events { .. } => {
            draw_events_table(frame, rows[1], state, &cache.events, palette);
        }
    }
}
