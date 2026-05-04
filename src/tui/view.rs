use chrono::{Local, Utc};
use nucleo_matcher::{
    pattern::{CaseMatching, Normalization, Pattern},
    Config as NucleoConfig, Matcher,
};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Cell, Clear, Paragraph, Row, Sparkline, Table, TableState,
    },
    Frame,
};

use crate::tui::data::{sort_session_rows, DataCache, EventRow, LeftRow, SessionRow, TrendRow};
use crate::tui::format::{fmt_cost, fmt_num, fmt_tokens_short, relative_time};
use crate::tui::shell::{draw_main_frame, draw_sidebar};
use crate::tui::state::{AppState, DrillKind, Focus, Section, Sort, SourceFilter};
use crate::tui::theme::Palette;
use crate::tui::MIN_WIDTH;

const BAR_WIDTH: usize = 20;
const LEFT_NAME_WIDTH: u16 = 28;
const SESSION_PROJECT_WIDTH: u16 = 28;
const TREND_BUCKET_WIDTH: u16 = 20;
const PROVIDER_COL_WIDTH: u16 = 10;
const SIDEBAR_WIDTH: u16 = 22;
const PANEL_BORDER: u16 = 2;
// 1 top border + 1 top pad + 1 bottom border.
const PANEL_CHROME_HEIGHT: u16 = 3;

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

// -- Header / context -------------------------------------------------------

fn draw_header(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    cache: &DataCache,
    palette: &Palette,
) {
    let clock = Local::now().format("%Y-%m-%d %H:%M").to_string();
    let total_cost: f64 = cache.left.iter().map(|r| r.cost).sum::<f64>()
        + cache.trend.iter().map(|r| r.total_cost).sum::<f64>();
    let total_tokens: u64 = cache.sessions.iter().map(|r| r.total_tokens).sum::<u64>()
        + cache.trend.iter().map(|r| r.total_tokens).sum::<u64>();

    let line1 = Line::from(vec![
        Span::styled("tokctl ", Style::default().add_modifier(Modifier::BOLD)),
        Span::styled(clock, palette.dim_text()),
        Span::raw("  "),
        Span::styled(
            format!(
                "last {} · {} · {} tok",
                state.time_window.as_str(),
                fmt_cost(total_cost),
                fmt_num(total_tokens),
            ),
            palette.dim_text(),
        ),
        Span::raw("  "),
        Span::styled("[?]", palette.accent_text()),
    ]);
    let line2 = Line::from(Span::styled(
        context_text(state, area.width.saturating_sub(1) as usize),
        palette.dim_text(),
    ));
    frame.render_widget(Paragraph::new(vec![line1, line2]), area);
}

fn context_text(state: &AppState, width: usize) -> String {
    let tab = match state.active_tab_label() {
        Some(label) => format!(" · tab:{}", label.to_ascii_lowercase()),
        None => String::new(),
    };
    let filter = if state.filter.query.is_empty() {
        String::new()
    } else {
        format!(" · filter:{}", state.filter.query)
    };
    truncate_chars(
        &format!(
            "section:{}{tab} · window:{} · source:{} · sort:{}{filter}",
            state.current_section.as_str(),
            state.time_window.as_str(),
            state.source_filter.as_str(),
            state.sort.as_str(),
        ),
        width,
    )
}

fn truncate_chars(value: &str, width: usize) -> String {
    let count = value.chars().count();
    if count <= width {
        return value.to_owned();
    }
    if width <= 1 {
        return "…".into();
    }
    let mut out: String = value.chars().take(width.saturating_sub(1)).collect();
    out.push('…');
    out
}

// -- Main panel sizing ------------------------------------------------------

/// Returns the (width, height) the main bordered panel "wants" given the
/// currently active section and the data in the cache. The caller clamps
/// these to the available terminal size.
fn main_panel_dimensions(state: &AppState, cache: &DataCache) -> (u16, u16) {
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
            let rows = cache.left.len() as u16;
            (table_width(&cells), 1 + rows.max(1))
        }
        Section::Repos | Section::Provider => {
            let n = active_provider_count(state, cache) as usize;
            let mut cells: Vec<u16> = vec![TREND_BUCKET_WIDTH];
            cells.extend(std::iter::repeat(PROVIDER_COL_WIDTH).take(n));
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
fn table_width(cells: &[u16]) -> u16 {
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
        Section::Days | Section::Models | Section::Sessions => {
            draw_left_table(frame, padded, state, cache, palette);
        }
        Section::Provider => {
            draw_trend_table(frame, padded, state, cache, palette);
        }
    }
}

/// Inset the top of a rect by `n` rows for breathing space.
fn pad_top(area: Rect, n: u16) -> Rect {
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
fn breadcrumb_title(state: &AppState) -> String {
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

// -- Section: left-row table (Days, Models, Sessions, Repos Costs tab) ------------

fn draw_left_table(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    cache: &DataCache,
    palette: &Palette,
) {
    let (rows, _) = apply_filter_left(&cache.left, state);
    if rows.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "no rows in this window",
                palette.dim_text(),
            ))),
            area,
        );
        return;
    }

    let max_cost = rows
        .iter()
        .filter(|r| !r.is_no_repo)
        .map(|r| r.cost)
        .fold(0f64, f64::max)
        .max(0.0001);

    let selected = state.current_index().min(rows.len().saturating_sub(1));
    let active = state.focus == Focus::Main;

    // For Sessions section we render a different (richer) layout.
    if state.current_section == Section::Sessions {
        // Adapt LeftRow → SessionRow using the real ts/source threaded
        // through from session_report (latest_ts/source on LeftRow).
        let now_fallback = Utc::now();
        let sess_rows: Vec<SessionRow> = rows
            .iter()
            .map(|r| SessionRow {
                session_id: r.key.clone(),
                source: r.source.unwrap_or(crate::types::Source::Claude),
                latest_ts: r.latest_ts.unwrap_or(now_fallback),
                project: Some(r.label.clone()),
                cost: r.cost,
                total_tokens: r.total_tokens,
            })
            .collect();
        draw_sessions_table(frame, area, state, &sess_rows, palette);
        return;
    }

    if state.expanded {
        draw_left_expanded(frame, area, selected, active, &rows, max_cost, palette);
    } else {
        draw_left_compact(frame, area, selected, active, &rows, max_cost, palette);
    }
}

fn draw_left_compact(
    frame: &mut Frame<'_>,
    inner: Rect,
    selected: usize,
    active: bool,
    rows: &[LeftRow],
    max_cost: f64,
    palette: &Palette,
) {
    let table_rows: Vec<Row> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let is_selected = i == selected && active;
            let cost_style = if is_selected {
                palette.selected_row()
            } else if r.is_no_repo {
                palette.warn_text()
            } else {
                Style::default().fg(palette.cost_color(r.cost / max_cost))
            };
            let label_cell = label_cell(r.label.clone(), is_selected, palette);
            let bar_cell = if r.is_no_repo {
                Cell::from("")
            } else {
                Cell::from(render_bar(
                    (r.cost / max_cost).clamp(0.0, 1.0),
                    BAR_WIDTH,
                    palette,
                ))
            };
            let mut row = Row::new(vec![
                label_cell,
                Cell::from(format!("{:>8}", fmt_tokens_short(r.total_tokens))),
                Cell::from(format!("{:>10}", fmt_cost(r.cost))).style(cost_style),
                bar_cell,
            ]);
            if is_selected {
                row = row.style(palette.selected_row());
            }
            row
        })
        .collect();

    let table = Table::new(
        table_rows,
        [
            Constraint::Length(LEFT_NAME_WIDTH),
            Constraint::Length(8),
            Constraint::Length(10),
            Constraint::Min(BAR_WIDTH as u16 + 2),
        ],
    )
    .header(
        Row::new(vec!["name", "     tok", "      cost", "proportion"]).style(palette.dim_text()),
    );

    let mut ts = TableState::default();
    ts.select(Some(selected));
    frame.render_stateful_widget(table, inner, &mut ts);
}

fn draw_left_expanded(
    frame: &mut Frame<'_>,
    inner: Rect,
    selected: usize,
    active: bool,
    rows: &[LeftRow],
    max_cost: f64,
    palette: &Palette,
) {
    let table_rows: Vec<Row> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let is_selected = i == selected && active;
            let cost_style = if is_selected {
                palette.selected_row()
            } else if r.is_no_repo {
                palette.warn_text()
            } else {
                Style::default().fg(palette.cost_color(r.cost / max_cost))
            };
            let label_cell = label_cell(r.label.clone(), is_selected, palette);
            let bar_cell = if r.is_no_repo {
                Cell::from("")
            } else {
                Cell::from(render_bar(
                    (r.cost / max_cost).clamp(0.0, 1.0),
                    BAR_WIDTH,
                    palette,
                ))
            };
            let mut row = Row::new(vec![
                label_cell,
                Cell::from(format!("{:>5}", fmt_num(r.sessions))),
                Cell::from(format!("{:>8}", fmt_tokens_short(r.total_tokens))),
                Cell::from(format!("{:>10}", fmt_cost(r.cost))).style(cost_style),
                bar_cell,
            ]);
            if is_selected {
                row = row.style(palette.selected_row());
            }
            row
        })
        .collect();

    let table = Table::new(
        table_rows,
        [
            Constraint::Length(LEFT_NAME_WIDTH),
            Constraint::Length(5),
            Constraint::Length(8),
            Constraint::Length(10),
            Constraint::Min(BAR_WIDTH as u16 + 2),
        ],
    )
    .header(
        Row::new(vec![
            "name",
            " sess",
            "     tok",
            "      cost",
            "proportion",
        ])
        .style(palette.dim_text()),
    );

    let mut ts = TableState::default();
    ts.select(Some(selected));
    frame.render_stateful_widget(table, inner, &mut ts);
}

fn label_cell(label: String, is_selected: bool, palette: &Palette) -> Cell<'static> {
    if is_selected {
        Cell::from(Line::from(vec![
            Span::styled("▌ ", palette.accent_text()),
            Span::raw(label),
        ]))
    } else {
        Cell::from(label)
    }
}

// -- Sessions table ---------------------------------------------------------

fn draw_sessions_table(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    rows: &[SessionRow],
    palette: &Palette,
) {
    if rows.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "no sessions in this scope",
                palette.dim_text(),
            ))),
            area,
        );
        return;
    }

    let filtered = display_session_rows(rows, state);
    let max_cost = filtered
        .iter()
        .map(|r| r.cost)
        .fold(0f64, f64::max)
        .max(0.0001);
    let now = Utc::now();
    let selected = state.current_index().min(filtered.len().saturating_sub(1));
    let active = state.focus == Focus::Main;

    let table_rows: Vec<Row> = filtered
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let is_selected = i == selected && active;
            let src_style = if is_selected {
                palette.selected_row()
            } else {
                palette.dim_text()
            };
            let cost_style = if is_selected {
                palette.selected_row()
            } else {
                Style::default().fg(palette.cost_color(r.cost / max_cost))
            };
            let proj_label = r
                .project
                .clone()
                .unwrap_or_else(|| r.session_id.chars().take(8).collect());
            let proj_cell = if is_selected {
                Cell::from(Line::from(vec![
                    Span::styled("▌ ", palette.accent_text()),
                    Span::raw(proj_label),
                ]))
            } else {
                Cell::from(proj_label)
            };
            let bar_cell = Cell::from(render_bar(
                (r.cost / max_cost).clamp(0.0, 1.0),
                BAR_WIDTH,
                palette,
            ));
            let mut row = Row::new(vec![
                Cell::from(relative_time(r.latest_ts, now)),
                Cell::from(r.source.as_str().to_owned()).style(src_style),
                proj_cell,
                Cell::from(format!("{:>8}", fmt_tokens_short(r.total_tokens))),
                Cell::from(format!("{:>10}", fmt_cost(r.cost))).style(cost_style),
                bar_cell,
            ]);
            if is_selected {
                row = row.style(palette.selected_row());
            }
            row
        })
        .collect();

    let table = Table::new(
        table_rows,
        [
            Constraint::Length(12),
            Constraint::Length(6),
            Constraint::Length(SESSION_PROJECT_WIDTH),
            Constraint::Length(8),
            Constraint::Length(10),
            Constraint::Min(BAR_WIDTH as u16 + 2),
        ],
    )
    .header(
        Row::new(vec![
            "when",
            "src",
            "project",
            "     tok",
            "      cost",
            "proportion",
        ])
        .style(palette.dim_text()),
    );

    let mut ts = TableState::default();
    ts.select(Some(selected));
    frame.render_stateful_widget(table, area, &mut ts);
}

fn display_session_rows(rows: &[SessionRow], state: &AppState) -> Vec<SessionRow> {
    let (mut filtered, scores) = apply_filter_sessions(rows, state);
    if scores.is_empty() {
        sort_session_rows(&mut filtered, state.sort);
    }
    filtered
}

// -- Events table (deepest drill) -------------------------------------------

fn draw_events_table(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    rows: &[EventRow],
    palette: &Palette,
) {
    if rows.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "no events in this scope",
                palette.dim_text(),
            ))),
            area,
        );
        return;
    }

    let max_cost = rows.iter().map(|r| r.cost).fold(0f64, f64::max).max(0.0001);
    let selected = state.current_index().min(rows.len().saturating_sub(1));
    let active = state.focus == Focus::Main;

    let table_rows: Vec<Row> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let is_selected = i == selected && active;
            let cost_style = if is_selected {
                palette.selected_row()
            } else {
                Style::default().fg(palette.cost_color(r.cost / max_cost))
            };
            let when = r.ts.with_timezone(&Local).format("%H:%M:%S").to_string();
            let when_cell = if is_selected {
                Cell::from(Line::from(vec![
                    Span::styled("▌ ", palette.accent_text()),
                    Span::raw(when),
                ]))
            } else {
                Cell::from(when)
            };
            let bar_cell = Cell::from(render_bar(
                (r.cost / max_cost).clamp(0.0, 1.0),
                BAR_WIDTH,
                palette,
            ));
            let mut row = Row::new(vec![
                when_cell,
                Cell::from(short_model(&r.model)),
                Cell::from(format!("{:>7}", fmt_tokens_short(r.input))),
                Cell::from(format!("{:>7}", fmt_tokens_short(r.output))),
                Cell::from(format!("{:>9}", fmt_tokens_short(r.cache_read))),
                Cell::from(format!("{:>9}", fmt_tokens_short(r.cache_write))),
                Cell::from(format!("{:>10}", fmt_cost(r.cost))).style(cost_style),
                bar_cell,
            ]);
            if is_selected {
                row = row.style(palette.selected_row());
            }
            row
        })
        .collect();

    let table = Table::new(
        table_rows,
        [
            Constraint::Length(10),
            Constraint::Length(16),
            Constraint::Length(7),
            Constraint::Length(7),
            Constraint::Length(9),
            Constraint::Length(9),
            Constraint::Length(10),
            Constraint::Min(BAR_WIDTH as u16 + 2),
        ],
    )
    .header(
        Row::new(vec![
            "when",
            "model",
            "     in",
            "    out",
            "  cache_r",
            "  cache_w",
            "      cost",
            "proportion",
        ])
        .style(palette.dim_text()),
    );

    // Stateful render so the viewport scrolls to keep the cursor visible
    // when the session has more turns than fit on screen.
    let mut ts = TableState::default();
    ts.select(Some(selected));
    frame.render_stateful_widget(table, area, &mut ts);
}

/// Trim a model identifier to fit the events table's narrow `model` column.
/// Common forms (`claude-sonnet-4-6`, `gpt-5.4`) already fit; very long
/// custom IDs get tail-truncated with an ellipsis.
fn short_model(model: &str) -> String {
    const W: usize = 16;
    let count = model.chars().count();
    if count <= W {
        return model.to_owned();
    }
    let mut s: String = model.chars().take(W - 1).collect();
    s.push('…');
    s
}

// -- Trend table ------------------------------------------------------------

fn draw_trend_table(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    cache: &DataCache,
    palette: &Palette,
) {
    let rows = display_trend_rows(&cache.trend, state.sort);
    if rows.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "no data for this window",
                palette.dim_text(),
            ))),
            area,
        );
        return;
    }

    let max_total: f64 = rows
        .iter()
        .map(|r| r.total_cost)
        .fold(0f64, f64::max)
        .max(0.0001);

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

    let mut header_cells = vec![Cell::from(state.trend_granularity.bucket_header())];
    if show_claude {
        header_cells.push(Cell::from("claude"));
    }
    if show_codex {
        header_cells.push(Cell::from("codex"));
    }
    if show_cursor {
        header_cells.push(Cell::from("cursor"));
    }
    header_cells.push(Cell::from("tokens"));
    header_cells.push(Cell::from("total"));
    header_cells.push(Cell::from("proportion"));
    let header = Row::new(header_cells).style(palette.dim_text());

    let mut body: Vec<Row> = rows
        .iter()
        .map(|r| trend_row(r, max_total, show_claude, show_codex, show_cursor, palette))
        .collect();

    let (cc_sum, xc_sum, uc_sum, tok_sum, tot_sum): (f64, f64, f64, u64, f64) =
        rows.iter().fold((0.0, 0.0, 0.0, 0u64, 0.0), |acc, r| {
            (
                acc.0 + r.claude_cost,
                acc.1 + r.codex_cost,
                acc.2 + r.cursor_cost,
                acc.3 + r.total_tokens,
                acc.4 + r.total_cost,
            )
        });

    let sep_cells_count = 3 + show_claude as usize + show_codex as usize + show_cursor as usize;
    let rule = "─".repeat(area.width.saturating_sub(2) as usize);
    let mut sep_cells = vec![Cell::from(rule).style(palette.dim_text())];
    for _ in 1..sep_cells_count {
        sep_cells.push(Cell::from(""));
    }
    body.push(Row::new(sep_cells));

    let mut total_cells =
        vec![Cell::from("TOTAL").style(Style::default().add_modifier(Modifier::BOLD))];
    if show_claude {
        total_cells.push(Cell::from(fmt_cost(cc_sum)).style(palette.accent_text()));
    }
    if show_codex {
        total_cells.push(Cell::from(fmt_cost(xc_sum)).style(palette.warn_text()));
    }
    if show_cursor {
        total_cells.push(Cell::from(fmt_cost(uc_sum)).style(palette.info_text()));
    }
    total_cells
        .push(Cell::from(format!("{:>8}", fmt_tokens_short(tok_sum))).style(palette.dim_text()));
    total_cells.push(
        Cell::from(format!("{:>10}", fmt_cost(tot_sum)))
            .style(Style::default().add_modifier(Modifier::BOLD)),
    );
    total_cells.push(Cell::from(""));
    body.push(Row::new(total_cells));

    let mut constraints = vec![Constraint::Length(20)];
    if show_claude {
        constraints.push(Constraint::Length(10));
    }
    if show_codex {
        constraints.push(Constraint::Length(10));
    }
    if show_cursor {
        constraints.push(Constraint::Length(10));
    }
    constraints.push(Constraint::Length(8));
    constraints.push(Constraint::Length(10));
    constraints.push(Constraint::Min(BAR_WIDTH as u16 + 2));

    let table = Table::new(body, constraints).header(header);
    frame.render_widget(table, area);
}

fn display_trend_rows(rows: &[TrendRow], sort: Sort) -> Vec<TrendRow> {
    let mut out = rows.to_vec();
    out.sort_by(|a, b| match sort {
        Sort::CostDesc => b
            .total_cost
            .partial_cmp(&a.total_cost)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.bucket.cmp(&a.bucket)),
        Sort::CostAsc => a
            .total_cost
            .partial_cmp(&b.total_cost)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.bucket.cmp(&a.bucket)),
        Sort::RecentDesc | Sort::AlphaDesc => b.bucket.cmp(&a.bucket),
        Sort::RecentAsc | Sort::AlphaAsc => a.bucket.cmp(&b.bucket),
    });
    out
}

fn trend_row<'a>(
    r: &'a TrendRow,
    max_total: f64,
    show_claude: bool,
    show_codex: bool,
    show_cursor: bool,
    palette: &Palette,
) -> Row<'a> {
    let ratio = (r.total_cost / max_total).clamp(0.0, 1.0);
    let bar = render_bar(ratio, BAR_WIDTH, palette);

    let bucket_label = if r.is_current {
        format!("▸{} (so far)", r.bucket)
    } else {
        format!("  {}", r.bucket)
    };
    let total_color = palette.cost_color(ratio);

    let mut cells = vec![Cell::from(bucket_label)];

    if show_claude {
        cells.push(cost_cell_or_dash(
            r.claude_cost,
            palette.accent_text(),
            palette.dim_text(),
        ));
    }
    if show_codex {
        cells.push(cost_cell_or_dash(
            r.codex_cost,
            palette.warn_text(),
            palette.dim_text(),
        ));
    }
    if show_cursor {
        cells.push(cost_cell_or_dash(
            r.cursor_cost,
            palette.info_text(),
            palette.dim_text(),
        ));
    }

    cells.push(
        Cell::from(format!("{:>8}", fmt_tokens_short(r.total_tokens))).style(palette.dim_text()),
    );
    cells.push(
        Cell::from(format!("{:>10}", fmt_cost(r.total_cost)))
            .style(Style::default().fg(total_color)),
    );
    cells.push(Cell::from(bar));

    Row::new(cells)
}

fn cost_cell_or_dash(cost: f64, value_style: Style, dim_style: Style) -> Cell<'static> {
    if cost < 0.001 {
        Cell::from("—").style(dim_style)
    } else {
        Cell::from(format!("{:>10}", fmt_cost(cost))).style(value_style)
    }
}

fn render_bar(ratio: f64, width: usize, palette: &Palette) -> Line<'static> {
    let filled = (ratio * width as f64).round() as usize;
    let filled = filled.min(width);
    let empty = width - filled;
    let mut spans = Vec::new();
    if filled > 0 {
        spans.push(Span::styled("█".repeat(filled), palette.bar_filled_style()));
    }
    if empty > 0 {
        spans.push(Span::styled("░".repeat(empty), palette.bar_empty_style()));
    }
    Line::from(spans)
}

// -- Footer / sparkline -----------------------------------------------------

fn draw_footer(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    cache: &DataCache,
    palette: &Palette,
) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    if cache.sparkline.iter().copied().fold(0f64, f64::max) <= 0.0 {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "no data in last 30 days",
                palette.dim_text(),
            ))),
            rows[0],
        );
    } else {
        let scaled: Vec<u64> = cache
            .sparkline
            .iter()
            .map(|c| ((c * 100.0).round() as i64).max(0) as u64)
            .collect();
        let spark = Sparkline::default()
            .data(&scaled)
            .bar_set(symbols::bar::NINE_LEVELS)
            .style(palette.accent_text());
        frame.render_widget(spark, rows[0]);
    }

    let mut spans = vec![
        Span::styled(
            format!(" window:{} ", state.time_window.as_str()),
            palette.accent_text(),
        ),
        Span::raw(" "),
        Span::styled(
            format!(" source:{} ", state.source_filter.as_str()),
            palette.dim_text(),
        ),
        Span::raw("  "),
    ];

    for message in footer_messages(state, cache) {
        let style = if message.is_error {
            palette.warn_text()
        } else {
            palette.accent_text()
        };
        spans.push(Span::styled(format!("{}  ", message.text), style));
    }

    spans.push(Span::styled("│  ", palette.dim_text()));

    // Three semantic groups, separated by `│`. No brackets around keys.
    let nav: &[(&str, &str)] = &[("j/k", "move"), ("[/]", "section")];
    let mut view_group: Vec<(&str, &str)> = Vec::new();
    let in_events_drill = matches!(
        state.deepest_drill().map(|d| d.kind),
        Some(DrillKind::Events { .. })
    );
    if !state.current_section.tabs().is_empty() && !state.drill_active() {
        view_group.push(("tab", "tabs"));
    }
    if matches!(state.current_section, Section::Provider | Section::Days) && !state.drill_active() {
        view_group.push(("d/w/m/y", "bucket"));
    }
    if in_events_drill {
        view_group.push(("s", "sort"));
        view_group.push(("i", "detail"));
        view_group.push(("y/Y", "yank"));
    } else if state.can_push_drill() {
        view_group.push(("↵", "drill"));
    }
    if !in_events_drill {
        view_group.push(("/", "filter"));
    }
    let system: &[(&str, &str)] = &[("?", "help"), ("q", "quit")];

    push_hint_group(&mut spans, nav, palette);
    spans.push(Span::styled("  │  ", palette.dim_text()));
    push_hint_group(&mut spans, &view_group, palette);
    spans.push(Span::styled("  │  ", palette.dim_text()));
    push_hint_group(&mut spans, system, palette);

    frame.render_widget(Paragraph::new(Line::from(spans)), rows[1]);
}

struct FooterMessage {
    text: String,
    is_error: bool,
}

fn footer_messages(state: &AppState, cache: &DataCache) -> Vec<FooterMessage> {
    let mut messages = Vec::new();
    if let Some(flash) = &state.flash {
        messages.push(FooterMessage {
            text: flash.clone(),
            is_error: false,
        });
    }
    if let Some(err) = &cache.refresh_error {
        messages.push(FooterMessage {
            text: err.display_message(),
            is_error: true,
        });
    }
    messages
}

fn push_hint_group(spans: &mut Vec<Span<'static>>, hints: &[(&str, &str)], palette: &Palette) {
    for (i, (key, desc)) in hints.iter().enumerate() {
        spans.push(Span::styled((*key).to_owned(), palette.accent_text()));
        spans.push(Span::raw(" "));
        spans.push(Span::styled((*desc).to_owned(), palette.dim_text()));
        if i + 1 < hints.len() {
            spans.push(Span::styled("  •  ", palette.dim_text()));
        }
    }
}

// -- Help / detail / filter overlays ----------------------------------------

fn draw_help(frame: &mut Frame<'_>, area: Rect, palette: &Palette) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(palette.active_border())
        .title(Span::styled(" HELP ", palette.active_border()));
    let help_area = centered(area, 72, 28);
    frame.render_widget(Clear, help_area);
    let inner = block.inner(help_area);
    frame.render_widget(block, help_area);

    let section = |title: &'static str| -> Line<'static> {
        Line::from(Span::styled(
            title.to_owned(),
            Style::default().add_modifier(Modifier::BOLD),
        ))
    };
    let blank = Line::from("");
    let hint = |key: &'static str, desc: &'static str| -> Line<'static> {
        Line::from(vec![
            Span::raw("  "),
            Span::styled(format!("{:<10}", key), palette.accent_text()),
            Span::styled("  ", palette.dim_text()),
            Span::styled(desc, palette.dim_text()),
        ])
    };

    let lines: Vec<Line> = vec![
        section("Navigation"),
        hint("j/k  ↓/↑", "move within focused area"),
        hint("[ / ]", "previous / next section"),
        hint("h  ←", "pop drill, then focus sidebar"),
        hint("l  →", "focus main"),
        hint(
            "Enter",
            "drill (push one level: section → sessions → events)",
        ),
        hint(
            "Esc / ←",
            "pop drill (one level), close overlay, cancel filter",
        ),
        hint("gg / G", "top / bottom"),
        hint("Ctrl-d/u", "half page down / up"),
        blank.clone(),
        section("View"),
        hint("Tab", "cycle main-pane tabs"),
        hint("t", "jump to Provider section"),
        hint("d/w/m/y", "bucket granularity (Provider / Days)"),
        hint("e", "compact / expanded"),
        hint("s", "cycle sort"),
        hint("/", "fuzzy filter"),
        hint("T W M z a", "window: today/week/month/year/all"),
        hint("1 2 3 4", "source: all/claude/codex/cursor"),
        hint("i", "row details"),
        hint("r", "refresh (no ingest)"),
        blank.clone(),
        section("Copy / Export"),
        hint("y", "yank row key"),
        hint("Y", "yank row summary"),
        blank.clone(),
        hint("?", "toggle this help"),
        hint("q / Ctrl-c", "quit"),
    ];

    frame.render_widget(Paragraph::new(lines), inner);
}

fn draw_detail(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    cache: &DataCache,
    palette: &Palette,
) {
    let detail_area = centered(area, 76, 14);
    frame.render_widget(Clear, detail_area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(palette.active_border())
        .title(Span::styled(
            " DETAILS · esc/i to close ",
            palette.active_border(),
        ));
    let inner = block.inner(detail_area);
    frame.render_widget(block, detail_area);
    let lines = detail_lines(state, cache)
        .into_iter()
        .map(Line::from)
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines), inner);
}

fn detail_lines(state: &AppState, cache: &DataCache) -> Vec<String> {
    if let Some(d) = state.deepest_drill() {
        return match d.kind {
            DrillKind::Sessions { .. } => cache
                .sessions
                .get(
                    state
                        .current_index()
                        .min(cache.sessions.len().saturating_sub(1)),
                )
                .map(|row| {
                    vec![
                        format!("session: {}", row.session_id),
                        format!("source: {}", row.source.as_str()),
                        format!(
                            "project: {}",
                            row.project.clone().unwrap_or_else(|| "(unknown)".into())
                        ),
                        format!(
                            "latest: {}",
                            row.latest_ts
                                .with_timezone(&Local)
                                .format("%Y-%m-%d %H:%M:%S")
                        ),
                        format!("tokens: {}", fmt_num(row.total_tokens)),
                        format!("cost: {}", fmt_cost(row.cost)),
                        "copy: y key, Y summary".into(),
                    ]
                })
                .unwrap_or_else(|| vec!["no session selected".into()]),
            DrillKind::Events { source } => cache
                .events
                .get(
                    state
                        .current_index()
                        .min(cache.events.len().saturating_sub(1)),
                )
                .map(|row| {
                    vec![
                        format!("session: {}", d.key),
                        format!("source: {}", source.as_str()),
                        format!("model: {}", row.model),
                        format!(
                            "when: {}",
                            row.ts.with_timezone(&Local).format("%Y-%m-%d %H:%M:%S")
                        ),
                        format!("input: {}", fmt_num(row.input)),
                        format!("output: {}", fmt_num(row.output)),
                        format!("cache read: {}", fmt_num(row.cache_read)),
                        format!("cache write: {}", fmt_num(row.cache_write)),
                        format!("cost: {}", fmt_cost(row.cost)),
                        "copy: y key, Y summary".into(),
                    ]
                })
                .unwrap_or_else(|| vec!["no event selected".into()]),
        };
    }
    match state.current_section {
        Section::Provider => cache
            .trend
            .get(
                state
                    .current_index()
                    .min(cache.trend.len().saturating_sub(1)),
            )
            .map(|row| {
                vec![
                    format!("bucket: {}", row.bucket),
                    format!("tokens: {}", fmt_num(row.total_tokens)),
                    format!("total: {}", fmt_cost(row.total_cost)),
                    format!("claude: {}", fmt_cost(row.claude_cost)),
                    format!("codex: {}", fmt_cost(row.codex_cost)),
                    format!("cursor: {}", fmt_cost(row.cursor_cost)),
                ]
            })
            .unwrap_or_else(|| vec!["no row selected".into()]),
        _ => cache
            .left
            .get(
                state
                    .current_index()
                    .min(cache.left.len().saturating_sub(1)),
            )
            .map(|row| {
                vec![
                    format!("name: {}", row.label),
                    format!("key: {}", row.key),
                    format!("sessions: {}", fmt_num(row.sessions)),
                    format!("tokens: {}", fmt_num(row.total_tokens)),
                    format!("cost: {}", fmt_cost(row.cost)),
                    "copy: y key, Y summary".into(),
                ]
            })
            .unwrap_or_else(|| vec!["no row selected".into()]),
    }
}

fn draw_filter_prompt(frame: &mut Frame<'_>, area: Rect, state: &AppState, palette: &Palette) {
    let row = Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(3),
        width: area.width,
        height: 1,
    };
    let line = Line::from(vec![
        Span::styled(" / ", palette.accent_text()),
        Span::raw(state.filter.query.clone()),
        Span::styled("_", palette.accent_text()),
    ]);
    frame.render_widget(Clear, row);
    frame.render_widget(Paragraph::new(line), row);
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let w = width.min(area.width.saturating_sub(2));
    let h = height.min(area.height.saturating_sub(2));
    Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    }
}

// -- Filtering --------------------------------------------------------------

fn apply_filter_left(rows: &[LeftRow], state: &AppState) -> (Vec<LeftRow>, Vec<u32>) {
    if !should_filter(state) {
        return (rows.to_vec(), Vec::new());
    }
    let mut matcher = Matcher::new(NucleoConfig::DEFAULT);
    let pat = Pattern::parse(
        &state.filter.query,
        CaseMatching::Ignore,
        Normalization::Smart,
    );
    let mut scored: Vec<(LeftRow, u32)> = rows
        .iter()
        .filter_map(|r| {
            let mut buf = Vec::new();
            let haystack = nucleo_matcher::Utf32Str::new(&r.label, &mut buf);
            pat.score(haystack, &mut matcher).map(|s| (r.clone(), s))
        })
        .collect();
    scored.sort_by_key(|x| std::cmp::Reverse(x.1));
    let (rows, scores): (Vec<_>, Vec<_>) = scored.into_iter().unzip();
    (rows, scores)
}

fn apply_filter_sessions(rows: &[SessionRow], state: &AppState) -> (Vec<SessionRow>, Vec<u32>) {
    if !should_filter(state) {
        return (rows.to_vec(), Vec::new());
    }
    let mut matcher = Matcher::new(NucleoConfig::DEFAULT);
    let pat = Pattern::parse(
        &state.filter.query,
        CaseMatching::Ignore,
        Normalization::Smart,
    );
    let mut scored: Vec<(SessionRow, u32)> = rows
        .iter()
        .filter_map(|r| {
            let label = r.project.clone().unwrap_or_else(|| r.session_id.clone());
            let mut buf = Vec::new();
            let haystack = nucleo_matcher::Utf32Str::new(&label, &mut buf);
            pat.score(haystack, &mut matcher).map(|s| (r.clone(), s))
        })
        .collect();
    scored.sort_by_key(|x| std::cmp::Reverse(x.1));
    let (rows, scores): (Vec<_>, Vec<_>) = scored.into_iter().unzip();
    (rows, scores)
}

fn should_filter(state: &AppState) -> bool {
    !state.filter.query.is_empty() && state.focus == Focus::Main
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::data::{CacheStatus, DataCache, RefreshError, RefreshScope};
    use crate::tui::state::{Sort, SourceFilter, TimeWindow};

    fn cache() -> DataCache {
        let mut c = DataCache::default();
        c.left = vec![LeftRow {
            label: "tokctl".into(),
            key: "/dev/tokctl".into(),
            sessions: 2,
            total_tokens: 1234,
            cost: 4.2,
            is_no_repo: false,
            latest_ts: None,
            source: None,
        }];
        c.sessions = vec![SessionRow {
            session_id: "session-abcdef".into(),
            source: crate::types::Source::Claude,
            latest_ts: Utc::now(),
            project: Some("tokctl".into()),
            cost: 1.2,
            total_tokens: 500,
        }];
        c.status = CacheStatus {
            cache_path: "/tmp/cache.db".into(),
            event_count: 7,
            freshness: "fresh 1m".into(),
            last_query: Utc::now(),
            mtime_ns: None,
        };
        c
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
}
