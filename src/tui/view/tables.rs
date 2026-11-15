use chrono::Local;
use ratatui::{
    layout::{Constraint, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Cell, Paragraph, Row, Table, TableState},
    Frame,
};

use super::layout::{
    models_column_widths, BAR_WIDTH, LEFT_NAME_WIDTH, MODEL_BAR_WIDTH, MODEL_SHARE_TEXT_WIDTH,
    MODEL_TREND_WIDTH, SESSION_PROJECT_WIDTH,
};
use crate::tui::data::{EventRow, LeftRow, SessionRow, TrendRow, VisibleRows};
use crate::tui::format::{fmt_cost, fmt_num, fmt_tokens_short, session_when};
use crate::tui::state::{AppState, Focus, SourceFilter};
use crate::tui::theme::Palette;

pub(super) fn draw_left_table(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    cache: &VisibleRows,
    palette: &Palette,
) {
    let rows = &cache.left;
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

    if state.expanded {
        draw_left_expanded(frame, area, selected, active, rows, max_cost, palette);
    } else {
        draw_left_compact(frame, area, selected, active, rows, max_cost, palette);
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

/// Models rows: share of the window's cost, cache hit, and a daily-cost trend per row.
pub(super) fn draw_models_table(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    cache: &VisibleRows,
    palette: &Palette,
) {
    let rows = &cache.left;
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

    let max_cost = rows.iter().map(|r| r.cost).fold(0f64, f64::max).max(0.0001);
    let selected = state.current_index().min(rows.len().saturating_sub(1));
    let active = state.focus == Focus::Main;

    let table_rows: Vec<Row> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let is_selected = i == selected && active;
            let stats = r.model_stats.as_ref();
            let priced = stats.is_none_or(|s| s.priced);
            let (cost_text, cost_style) = if !priced {
                (
                    "unpriced".to_owned(),
                    if is_selected {
                        palette.selected_row()
                    } else {
                        palette.warn_text()
                    },
                )
            } else {
                (
                    fmt_cost(r.cost),
                    if is_selected {
                        palette.selected_row()
                    } else {
                        Style::default().fg(palette.cost_color(r.cost / max_cost))
                    },
                )
            };
            let share_cell = match stats {
                Some(s) if priced && cache.left.iter().any(|row| row.cost > 0.0) => {
                    let mut spans = vec![Span::raw(format!("{:>5.1}% ", s.share * 100.0))];
                    spans.extend(render_bar(s.share, MODEL_BAR_WIDTH, palette).spans);
                    Cell::from(Line::from(spans))
                }
                _ => Cell::from(format!("{:>1$}", "-", MODEL_SHARE_TEXT_WIDTH as usize - 1)),
            };
            let cache_hit = stats
                .and_then(|s| s.cache_hit)
                .map_or_else(|| "-".to_owned(), |hit| format!("{:.0}%", hit * 100.0));
            let trend = stats.map_or_else(String::new, |s| {
                trend_text(&s.daily_cost, MODEL_TREND_WIDTH as usize)
            });
            let mut row = Row::new(vec![
                label_cell(r.label.clone(), is_selected, palette),
                Cell::from(format!("{:>8}", fmt_tokens_short(r.total_tokens))),
                Cell::from(format!("{:>10}", cost_text)).style(cost_style),
                share_cell,
                Cell::from(format!("{:>6}", cache_hit)),
                Cell::from(trend).style(palette.accent_text()),
            ]);
            if is_selected {
                row = row.style(palette.selected_row());
            }
            row
        })
        .collect();

    let widths = models_column_widths().map(Constraint::Length);
    let table = Table::new(table_rows, widths).header(
        Row::new(vec![
            "model",
            "     tok",
            "      cost",
            "  share of cost",
            "   hit",
            "daily cost",
        ])
        .style(palette.dim_text()),
    );

    let mut ts = TableState::default();
    ts.select(Some(selected));
    frame.render_stateful_widget(table, area, &mut ts);
}

/// Block-character sparkline of the last `width` values, scaled to their maximum.
fn trend_text(values: &[f64], width: usize) -> String {
    const LEVELS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let tail = &values[values.len().saturating_sub(width)..];
    let max = tail.iter().copied().fold(0f64, f64::max);
    if max <= 0.0 {
        return String::new();
    }
    tail.iter()
        .map(|value| {
            if *value <= 0.0 {
                return ' ';
            }
            let level = ((value / max) * (LEVELS.len() - 1) as f64).round() as usize;
            LEVELS[level.min(LEVELS.len() - 1)]
        })
        .collect()
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

pub(super) fn draw_sessions_table(
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

    let filtered = rows;
    let max_cost = filtered
        .iter()
        .map(|r| r.cost)
        .fold(0f64, f64::max)
        .max(0.0001);
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
                Cell::from(session_when(r.latest_ts)),
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

pub(super) fn draw_events_table(
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

pub(super) fn draw_trend_table(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    cache: &VisibleRows,
    palette: &Palette,
) {
    let rows = &cache.trend;
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

    let (cc_sum, xc_sum, uc_sum, tok_sum, tot_sum): (f64, f64, f64, u128, f64) =
        rows.iter().fold((0.0, 0.0, 0.0, 0u128, 0.0), |acc, r| {
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

    let table = Table::new(body, constraints)
        .header(header)
        .highlight_style(palette.selected_row());
    let mut selection = TableState::default();
    selection.select(Some(state.current_index().min(rows.len() - 1)));
    frame.render_stateful_widget(table, area, &mut selection);
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
        Cell::from(provider_cost_text(cost)).style(dim_style)
    } else {
        Cell::from(provider_cost_text(cost)).style(value_style)
    }
}

pub(crate) fn provider_cost_text(cost: f64) -> String {
    if cost < 0.001 {
        format!("{:>10}", "—")
    } else {
        format!("{:>10}", fmt_cost(cost))
    }
}

pub(crate) fn render_bar(ratio: f64, width: usize, palette: &Palette) -> Line<'static> {
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
