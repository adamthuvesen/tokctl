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
    widgets::{Block, BorderType, Borders, Cell, Clear, Paragraph, Row, Sparkline, Table},
    Frame,
};

use crate::tui::data::{DataCache, LeftRow, SessionRow, TrendRow};
use crate::tui::format::{fmt_cost, fmt_num, fmt_tokens_short, relative_time};
use crate::tui::state::{AppState, PaneId, SourceFilter};
use crate::tui::theme::Palette;
use crate::tui::MIN_WIDTH;

const BAR_WIDTH: usize = 20;

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

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(8),
            Constraint::Length(2),
        ])
        .split(area);

    draw_header(frame, outer[0], state, cache, &palette);

    if state.trend_open {
        draw_trend_overlay(frame, outer[1], state, cache, &palette);
    } else {
        draw_panes(frame, outer[1], state, cache, &palette);
    }

    draw_footer(frame, outer[2], state, cache, &palette);

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

fn draw_header(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    cache: &DataCache,
    palette: &Palette,
) {
    let clock = Local::now().format("%Y-%m-%d %H:%M").to_string();
    let total_cost: f64 = cache.left.iter().map(|r| r.cost).sum();
    let total_tokens: u64 = cache.sessions.iter().map(|r| r.total_tokens).sum();

    let window = state.time_window.as_str();
    let line1 = Line::from(vec![
        Span::styled("tokctl ", Style::default().add_modifier(Modifier::BOLD)),
        Span::styled(clock, palette.dim_text()),
        Span::raw("  "),
        Span::styled(
            format!(
                "last {} · {} · {} tok",
                window,
                fmt_cost(total_cost),
                fmt_num(total_tokens),
            ),
            palette.dim_text(),
        ),
        Span::raw("  "),
        Span::styled("[?]", palette.accent_text()),
    ]);
    let line2 = Line::from(Span::styled(
        context_text(state, cache, area.width.saturating_sub(1) as usize),
        palette.dim_text(),
    ));
    let p = Paragraph::new(vec![line1, line2]);
    frame.render_widget(p, area);
}

fn context_text(state: &AppState, cache: &DataCache, width: usize) -> String {
    let selection = selected_context(state, cache);
    let filter = if state.filter.query.is_empty() {
        String::new()
    } else {
        format!(" filter:{}", state.filter.query)
    };
    truncate_chars(
        &format!(
            "axis:{} · {} · window:{} · source:{} · sort:{}{}",
            state.left_axis.chip(),
            selection,
            state.time_window.as_str(),
            state.source_filter.as_str(),
            state.sort.as_str(),
            filter
        ),
        width,
    )
}

fn selected_context(state: &AppState, cache: &DataCache) -> String {
    let left = cache
        .left
        .get(state.left_index.min(cache.left.len().saturating_sub(1)))
        .map(|row| row.label.as_str())
        .unwrap_or("none");
    let session = cache
        .sessions
        .get(
            state
                .sessions_index
                .min(cache.sessions.len().saturating_sub(1)),
        )
        .map(|row| {
            row.project
                .as_deref()
                .map(str::to_owned)
                .unwrap_or_else(|| row.session_id.chars().take(8).collect())
        });
    match session {
        Some(session) => format!("{left} > {session}"),
        None => left.to_owned(),
    }
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

fn draw_panes(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    cache: &DataCache,
    palette: &Palette,
) {
    let ws = state.pane_widths;
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(ws[0]), Constraint::Percentage(ws[1])])
        .split(area);

    let (left_rows, _scores_l) = apply_filter_left(&cache.left, state, PaneId::Left);
    draw_left_pane(frame, cols[0], state, &left_rows, palette);

    let (sess_rows, _scores_s) = apply_filter_sessions(&cache.sessions, state);
    draw_sessions_pane(frame, cols[1], state, &sess_rows, palette);
}

/// Build the pane block. Active pane = Thick border in accent; inactive = Rounded in border_inactive.
fn pane_block<'a>(title: &'a str, active: bool, palette: &Palette) -> Block<'a> {
    let (border_style, border_type, title_style) = if active {
        (
            palette.active_border(),
            BorderType::Thick,
            palette.active_border(),
        )
    } else {
        (
            palette.inactive_border(),
            BorderType::Rounded,
            palette.dim_text(),
        )
    };
    let chip = if active {
        format!("[ {title} ]")
    } else {
        format!(" {title} ")
    };
    Block::default()
        .borders(Borders::ALL)
        .border_type(border_type)
        .border_style(border_style)
        .title(Span::styled(chip, title_style))
}

fn draw_left_pane(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    rows: &[LeftRow],
    palette: &Palette,
) {
    let active = state.focus == PaneId::Left;
    let block = pane_block(state.left_axis.title(), active, palette);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if rows.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "no rows in this window",
                palette.dim_text(),
            ))),
            inner,
        );
        return;
    }

    let max_cost = rows
        .iter()
        .filter(|r| !r.is_no_repo)
        .map(|r| r.cost)
        .fold(0f64, f64::max)
        .max(0.0001);

    let selected = state.left_index.min(rows.len().saturating_sub(1));

    if state.expanded {
        draw_left_expanded(frame, inner, selected, active, rows, max_cost, palette);
    } else {
        draw_left_compact(frame, inner, selected, active, rows, max_cost, palette);
    }
}

/// Compact layout: name | tokens | cost  (default, lower density)
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
            let mut row = Row::new(vec![
                label_cell,
                Cell::from(format!("{:>8}", fmt_tokens_short(r.total_tokens))),
                Cell::from(format!("{:>10}", fmt_cost(r.cost))).style(cost_style),
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
            Constraint::Min(20),
            Constraint::Length(8),
            Constraint::Length(10),
        ],
    )
    .header(
        Row::new(vec!["name", "     tok", "      cost"])
            .style(palette.dim_text()),
    );

    frame.render_widget(table, inner);
}

/// Expanded layout: name | sessions | tokens | cost
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
            let mut row = Row::new(vec![
                label_cell,
                Cell::from(format!("{:>5}", fmt_num(r.sessions))),
                Cell::from(format!("{:>8}", fmt_tokens_short(r.total_tokens))),
                Cell::from(format!("{:>10}", fmt_cost(r.cost))).style(cost_style),
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
            Constraint::Min(16),
            Constraint::Length(5),
            Constraint::Length(8),
            Constraint::Length(10),
        ],
    )
    .header(
        Row::new(vec!["name", " sess", "     tok", "      cost"])
            .style(palette.dim_text()),
    );

    frame.render_widget(table, inner);
}

/// Build a label cell with an accent ▌ gutter mark when the row is selected.
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

fn draw_sessions_pane(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    rows: &[SessionRow],
    palette: &Palette,
) {
    let active = state.focus == PaneId::Sessions;
    let block = pane_block("SESSIONS", active, palette);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if rows.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "no sessions in this scope",
                palette.dim_text(),
            ))),
            inner,
        );
        return;
    }

    let max_cost = rows.iter().map(|r| r.cost).fold(0f64, f64::max).max(0.0001);
    let now = Utc::now();
    let selected = state.sessions_index.min(rows.len().saturating_sub(1));

    let table_rows: Vec<Row> = rows
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
            let mut row = Row::new(vec![
                Cell::from(relative_time(r.latest_ts, now)),
                Cell::from(r.source.as_str().to_owned()).style(src_style),
                proj_cell,
                Cell::from(format!("{:>8}", fmt_tokens_short(r.total_tokens))),
                Cell::from(format!("{:>10}", fmt_cost(r.cost))).style(cost_style),
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
            Constraint::Min(12),
            Constraint::Length(8),
            Constraint::Length(10),
        ],
    )
    .header(
        Row::new(vec!["when", "src", "project", "     tok", "      cost"])
            .style(palette.dim_text()),
    );

    frame.render_widget(table, inner);
}

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

    // Row 0: sparkline
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

    // Row 1: status chips + separator + 6 key hints
    let mut spans = vec![
        Span::styled(
            format!(" window:{} ", state.time_window.as_str()),
            palette.accent_text(),
        ),
        Span::styled(
            format!(" source:{} ", state.source_filter.as_str()),
            palette.dim_text(),
        ),
        Span::raw("  "),
    ];

    if let Some(flash) = &state.flash {
        spans.push(Span::styled(format!("{flash}  "), palette.accent_text()));
    }

    spans.push(Span::styled("│  ", palette.dim_text()));

    let hints: &[(&str, &str)] = &[
        ("j/k", "move"),
        ("←↵", "drill"),
        ("h/l", "pane"),
        ("/", "filter"),
        ("?", "help"),
        ("q", "quit"),
    ];
    for (idx, (key, desc)) in hints.iter().enumerate() {
        spans.push(Span::styled(format!("[{key}]"), palette.accent_text()));
        spans.push(Span::styled(format!(" {desc}"), palette.dim_text()));
        if idx + 1 < hints.len() {
            spans.push(Span::styled("  ·  ", palette.dim_text()));
        }
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), rows[1]);
}

fn draw_help(frame: &mut Frame<'_>, area: Rect, palette: &Palette) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(palette.active_border())
        .title(Span::styled(" HELP ", palette.active_border()));
    let help_area = centered(area, 72, 26);
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
        hint("h/l  ←/→", "move focus between panes"),
        hint("j/k  ↓/↑", "move selection"),
        hint("gg / G", "top / bottom"),
        hint("Ctrl-d/u", "half page down / up"),
        hint("Enter", "drill right"),
        hint("Esc / ←", "cancel / pop"),
        blank.clone(),
        section("View"),
        hint("/", "fuzzy filter"),
        hint("Tab", "cycle left-pane axis"),
        hint("e", "toggle compact / expanded"),
        hint("s", "cycle sort"),
        hint("t", "trend overlay (d/w/m/y inside)"),
        hint("T w m z a", "window: today/week/month/year/all"),
        hint("1 2 3 4", "source: all/claude/codex/cursor"),
        hint("i", "row details"),
        hint("r", "refresh (no ingest)"),
        blank.clone(),
        section("Copy / Export"),
        hint("y", "yank row key to clipboard"),
        hint("Y", "yank row summary to clipboard"),
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
    match state.focus {
        PaneId::Left => cache
            .left
            .get(state.left_index.min(cache.left.len().saturating_sub(1)))
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
        PaneId::Sessions => cache
            .sessions
            .get(
                state
                    .sessions_index
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

fn draw_trend_overlay(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    cache: &DataCache,
    palette: &Palette,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(palette.active_border())
        .title(Span::styled(
            format!(
                " TREND · {} · source:{} · [t/esc to close] ",
                state.trend_granularity.as_str(),
                state.source_filter.as_str()
            ),
            palette.active_border(),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = &cache.trend;
    if rows.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "no data for this window",
                palette.dim_text(),
            ))),
            inner,
        );
        return;
    }

    let max_total: f64 = rows
        .iter()
        .map(|r| r.total_cost)
        .fold(0f64, f64::max)
        .max(0.0001);

    // Determine which source columns have any non-zero data AND are not filtered out.
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

    // Build header cells dynamically.
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

    // Build body rows.
    let mut body: Vec<Row> = rows
        .iter()
        .map(|r| {
            trend_row(r, max_total, show_claude, show_codex, show_cursor, state, palette)
        })
        .collect();

    // Separator + TOTAL row.
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

    let sep_cells_count = 3
        + show_claude as usize
        + show_codex as usize
        + show_cursor as usize;
    let rule = "─".repeat(inner.width.saturating_sub(2) as usize);
    let mut sep_cells = vec![
        Cell::from(rule).style(palette.dim_text()),
    ];
    for _ in 1..sep_cells_count {
        sep_cells.push(Cell::from(""));
    }
    body.push(Row::new(sep_cells));

    let mut total_cells = vec![
        Cell::from("TOTAL").style(Style::default().add_modifier(Modifier::BOLD)),
    ];
    if show_claude {
        total_cells.push(Cell::from(fmt_cost(cc_sum)).style(palette.accent_text()));
    }
    if show_codex {
        total_cells.push(Cell::from(fmt_cost(xc_sum)).style(palette.warn_text()));
    }
    if show_cursor {
        total_cells.push(Cell::from(fmt_cost(uc_sum)).style(palette.info_text()));
    }
    total_cells.push(Cell::from(format!("{:>8}", fmt_tokens_short(tok_sum))).style(palette.dim_text()));
    total_cells.push(Cell::from(format!("{:>10}", fmt_cost(tot_sum))).style(Style::default().add_modifier(Modifier::BOLD)));
    total_cells.push(Cell::from(""));
    body.push(Row::new(total_cells));

    // Build constraints dynamically based on visible source columns.
    let mut constraints = vec![Constraint::Length(20)]; // bucket label
    if show_claude {
        constraints.push(Constraint::Length(10));
    }
    if show_codex {
        constraints.push(Constraint::Length(10));
    }
    if show_cursor {
        constraints.push(Constraint::Length(10));
    }
    constraints.push(Constraint::Length(8));  // tokens
    constraints.push(Constraint::Length(10)); // total
    constraints.push(Constraint::Min(BAR_WIDTH as u16 + 2)); // proportion

    let table = Table::new(body, constraints).header(header);
    frame.render_widget(table, inner);
}

fn trend_row<'a>(
    r: &'a TrendRow,
    max_total: f64,
    show_claude: bool,
    show_codex: bool,
    show_cursor: bool,
    _state: &AppState,
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
        cells.push(cost_cell_or_dash(r.claude_cost, palette.accent_text(), palette.dim_text()));
    }
    if show_codex {
        cells.push(cost_cell_or_dash(r.codex_cost, palette.warn_text(), palette.dim_text()));
    }
    if show_cursor {
        cells.push(cost_cell_or_dash(r.cursor_cost, palette.info_text(), palette.dim_text()));
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

/// Return a dim `—` cell for zero cost, otherwise a styled cost cell.
fn cost_cell_or_dash(cost: f64, value_style: Style, dim_style: Style) -> Cell<'static> {
    if cost < 0.001 {
        Cell::from("—").style(dim_style)
    } else {
        Cell::from(format!("{:>10}", fmt_cost(cost))).style(value_style)
    }
}

/// Render a two-tone proportion bar: violet filled + zinc empty, fixed width.
fn render_bar(ratio: f64, width: usize, palette: &Palette) -> Line<'static> {
    let filled = (ratio * width as f64).round() as usize;
    let filled = filled.min(width);
    let empty = width - filled;
    let mut spans = Vec::new();
    if filled > 0 {
        spans.push(Span::styled(
            "█".repeat(filled),
            palette.bar_filled_style(),
        ));
    }
    if empty > 0 {
        spans.push(Span::styled(
            "░".repeat(empty),
            palette.bar_empty_style(),
        ));
    }
    Line::from(spans)
}

fn apply_filter_left(rows: &[LeftRow], state: &AppState, pane: PaneId) -> (Vec<LeftRow>, Vec<u32>) {
    if !should_filter(state, pane) {
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
    if !should_filter(state, PaneId::Sessions) {
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

fn should_filter(state: &AppState, pane: PaneId) -> bool {
    !state.filter.query.is_empty() && state.focus == pane
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::data::{CacheStatus, DataCache};
    use crate::tui::state::{Sort, SourceFilter, TimeWindow};

    fn cache() -> DataCache {
        DataCache {
            left: vec![LeftRow {
                label: "tokctl".into(),
                key: "/dev/tokctl".into(),
                sessions: 2,
                total_tokens: 1234,
                cost: 4.2,
                is_no_repo: false,
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
            },
            ..DataCache::default()
        }
    }

    #[test]
    fn context_includes_filters_and_breadcrumb() {
        let state = AppState {
            source_filter: SourceFilter::Cursor,
            time_window: TimeWindow::Month,
            sort: Sort::RecentDesc,
            ..AppState::default()
        };
        let text = context_text(&state, &cache(), 120);
        assert!(text.contains("source:cursor"));
        assert!(text.contains("sort:recent"));
        assert!(text.contains("tokctl > tokctl"));
    }

    #[test]
    fn context_truncates_to_width() {
        let text = context_text(&AppState::default(), &cache(), 10);
        assert!(text.chars().count() <= 10);
        assert!(text.ends_with('…'));
    }

    #[test]
    fn detail_lines_include_session_identity() {
        let state = AppState {
            focus: PaneId::Sessions,
            ..AppState::default()
        };
        let lines = detail_lines(&state, &cache());
        assert!(lines.iter().any(|line| line.contains("session-abcdef")));
    }

    #[test]
    fn render_bar_fills_correctly() {
        let p = Palette::default();
        // ratio 1.0 → all filled
        let bar = render_bar(1.0, 20, &p);
        let text: String = bar.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "█".repeat(20));

        // ratio 0.0 → all empty
        let bar = render_bar(0.0, 20, &p);
        let text: String = bar.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "░".repeat(20));
    }

    #[test]
    fn render_bar_spans_total_width() {
        let p = Palette::default();
        // half-filled bar should have total width == BAR_WIDTH
        let bar = render_bar(0.5, BAR_WIDTH, &p);
        let total: usize = bar.spans.iter().map(|s| s.content.chars().count()).sum();
        assert_eq!(total, BAR_WIDTH);
    }
}
