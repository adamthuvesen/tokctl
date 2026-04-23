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
use crate::tui::state::{AppState, PaneId};
use crate::tui::theme::Palette;
use crate::tui::MIN_WIDTH;

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
    let source = state.source_filter.as_str();
    let axis = state.left_axis.chip();

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
    let line2 = Line::from(vec![
        Span::styled(format!("axis:{axis} "), palette.dim_text()),
        Span::styled(format!("source:{source} "), palette.dim_text()),
    ]);
    let p = Paragraph::new(vec![line1, line2]);
    frame.render_widget(p, area);
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

fn pane_block<'a>(title: &'a str, active: bool, palette: &Palette) -> Block<'a> {
    let border = if active {
        palette.active_border()
    } else {
        palette.inactive_border()
    };
    let chip = if active {
        format!("[ {title} ]")
    } else {
        format!(" {title} ")
    };
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border)
        .title(Span::styled(
            chip,
            if active {
                palette.active_border()
            } else {
                palette.dim_text()
            },
        ))
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
            let mut row = Row::new(vec![
                Cell::from(r.label.clone()),
                Cell::from(fmt_num(r.sessions)),
                Cell::from(fmt_tokens_short(r.total_tokens)),
                Cell::from(fmt_cost(r.cost)).style(cost_style),
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
            Constraint::Percentage(60),
            Constraint::Length(5),
            Constraint::Length(7),
            Constraint::Length(9),
        ],
    )
    .header(Row::new(vec!["name", "sess", "tok", "cost"]).style(palette.dim_text()));

    frame.render_widget(table, inner);
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
            let mut row = Row::new(vec![
                Cell::from(relative_time(r.latest_ts, now)),
                Cell::from(r.source.as_str().to_owned()).style(src_style),
                Cell::from(
                    r.project
                        .clone()
                        .unwrap_or_else(|| r.session_id.chars().take(8).collect()),
                ),
                Cell::from(fmt_tokens_short(r.total_tokens)),
                Cell::from(fmt_cost(r.cost)).style(cost_style),
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
            Constraint::Percentage(40),
            Constraint::Length(7),
            Constraint::Length(9),
        ],
    )
    .header(Row::new(vec!["when", "src", "project", "tok", "cost"]).style(palette.dim_text()));

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

    let legend = Line::from(vec![
        Span::styled(
            format!(" window:{} ", state.time_window.as_str()),
            palette.accent_text(),
        ),
        Span::styled(
            format!(" source:{} ", state.source_filter.as_str()),
            palette.dim_text(),
        ),
        Span::raw("  "),
        Span::styled(
            "j/k move  ↵ drill  h/l pane  / filter  Tab axis  t trend  T/w/m/y/a window  s sort  q quit",
            palette.dim_text(),
        ),
    ]);
    frame.render_widget(Paragraph::new(legend), rows[1]);
}

fn draw_help(frame: &mut Frame<'_>, area: Rect, palette: &Palette) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(palette.accent_border())
        .title(Span::styled(" HELP ", palette.active_border()));
    let help_area = centered(area, 70, 20);
    frame.render_widget(Clear, help_area);
    let inner = block.inner(help_area);
    frame.render_widget(block, help_area);
    let lines = vec![
        Line::from("  h/l  ←/→          move focus between panes"),
        Line::from("  j/k  ↓/↑          move selection"),
        Line::from("  g g / G            top / bottom"),
        Line::from("  Ctrl-d / Ctrl-u    half page down / up"),
        Line::from("  Enter              drill right"),
        Line::from("  Esc / Backspace    cancel / pop"),
        Line::from("  /                  fuzzy filter"),
        Line::from("  Tab                cycle left-pane axis"),
        Line::from("  s                  cycle sort"),
        Line::from("  t                  trend overlay (d/w/m/y inside)"),
        Line::from("  T w m y a          window: today / week / month / year / all"),
        Line::from("  1 2 3              source: all / claude / codex"),
        Line::from("  r                  refresh (no ingest)"),
        Line::from("  y                  yank key to clipboard"),
        Line::from("  ?                  toggle this help"),
        Line::from("  q / Ctrl-c         quit"),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
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

impl Palette {
    fn accent_border(&self) -> Style {
        self.active_border()
    }
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
    let bar_cols: usize = (inner.width as usize).saturating_sub(60).clamp(6, 24);

    let header = Row::new(vec![
        state.trend_granularity.bucket_header(),
        "claude",
        "codex",
        "tokens",
        "total",
        "proportion",
    ])
    .style(palette.dim_text());

    let mut body: Vec<Row> = rows
        .iter()
        .map(|r| trend_row(r, max_total, bar_cols, state, palette))
        .collect();

    // Separator + TOTAL row.
    let (cc_sum, xc_sum, tok_sum, tot_sum): (f64, f64, u64, f64) =
        rows.iter().fold((0.0, 0.0, 0u64, 0.0), |acc, r| {
            (
                acc.0 + r.claude_cost,
                acc.1 + r.codex_cost,
                acc.2 + r.total_tokens,
                acc.3 + r.total_cost,
            )
        });
    let rule = "─".repeat(inner.width.saturating_sub(2) as usize);
    body.push(Row::new(vec![
        Cell::from(rule.clone()).style(palette.dim_text()),
        Cell::from(""),
        Cell::from(""),
        Cell::from(""),
        Cell::from(""),
        Cell::from(""),
    ]));
    let claude_cell = if matches!(state.source_filter, crate::tui::state::SourceFilter::Codex) {
        Cell::from("—").style(palette.dim_text())
    } else {
        Cell::from(fmt_cost(cc_sum)).style(palette.accent_text())
    };
    let codex_cell = if matches!(state.source_filter, crate::tui::state::SourceFilter::Claude) {
        Cell::from("—").style(palette.dim_text())
    } else {
        Cell::from(fmt_cost(xc_sum)).style(palette.warn_text())
    };
    body.push(Row::new(vec![
        Cell::from("TOTAL").style(Style::default().add_modifier(Modifier::BOLD)),
        claude_cell,
        codex_cell,
        Cell::from(fmt_num(tok_sum)).style(palette.dim_text()),
        Cell::from(fmt_cost(tot_sum)).style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from(""),
    ]));

    let table = Table::new(
        body,
        [
            Constraint::Length(14),
            Constraint::Length(12),
            Constraint::Length(12),
            Constraint::Length(10),
            Constraint::Length(12),
            Constraint::Min(bar_cols as u16),
        ],
    )
    .header(header);
    frame.render_widget(table, inner);
}

fn trend_row<'a>(
    r: &'a TrendRow,
    max_total: f64,
    bar_cols: usize,
    state: &AppState,
    palette: &Palette,
) -> Row<'a> {
    let ratio = (r.total_cost / max_total).clamp(0.0, 1.0);
    let bar = proportional_bar(ratio, bar_cols);
    let bucket_label = if r.is_current {
        format!("▸{} (so far)", r.bucket)
    } else {
        format!("  {}", r.bucket)
    };
    let total_color = palette.cost_color(ratio);

    let claude_cell = if matches!(state.source_filter, crate::tui::state::SourceFilter::Codex) {
        Cell::from("—").style(palette.dim_text())
    } else {
        Cell::from(fmt_cost(r.claude_cost)).style(palette.accent_text())
    };
    let codex_cell = if matches!(state.source_filter, crate::tui::state::SourceFilter::Claude) {
        Cell::from("—").style(palette.dim_text())
    } else {
        Cell::from(fmt_cost(r.codex_cost)).style(palette.warn_text())
    };

    Row::new(vec![
        Cell::from(bucket_label),
        claude_cell,
        codex_cell,
        Cell::from(fmt_tokens_short(r.total_tokens)).style(palette.dim_text()),
        Cell::from(fmt_cost(r.total_cost)).style(Style::default().fg(total_color)),
        Cell::from(bar),
    ])
}

fn proportional_bar(ratio: f64, width: usize) -> String {
    // ▁▂▃▄▅▆▇█
    let blocks = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let filled = (ratio * width as f64).round() as usize;
    let filled = filled.min(width);
    let mut s = String::with_capacity(width);
    for i in 0..width {
        if i < filled {
            // Scale the block's height to ratio for the final cell, keep full elsewhere.
            let idx = if i + 1 == filled {
                let frac = (ratio * width as f64) - filled as f64 + 1.0;
                ((frac * 7.0).round() as usize).min(7)
            } else {
                7
            };
            s.push(blocks[idx]);
        } else {
            s.push(' ');
        }
    }
    s
}

/// Apply the current fuzzy filter to the left pane's rows, returning the
/// surviving rows and their scores (for debug / tie-break inspection).
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
