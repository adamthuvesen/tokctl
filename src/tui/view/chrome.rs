use chrono::Local;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Sparkline},
    Frame,
};

use super::layout::centered;
use crate::tui::data::VisibleRows;
use crate::tui::format::{fmt_cost, fmt_num};
use crate::tui::state::{AppState, DrillKind, Section};
use crate::tui::theme::Palette;
use crate::tui::widgets::filter::should_filter;

pub(super) fn draw_header(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    cache: &VisibleRows,
    palette: &Palette,
) {
    let clock = Local::now().format("%Y-%m-%d %H:%M").to_string();
    let (total_cost, total_tokens, scope_suffix) = header_scope(state, cache);

    let window_label = if cache.refresh_error.is_some() {
        format!(
            "snapshot {} · last {}",
            cache
                .status
                .last_query
                .with_timezone(&Local)
                .format("%Y-%m-%d %H:%M"),
            state.time_window.as_str()
        )
    } else {
        format!("last {}", state.time_window.as_str())
    };
    let line1 = Line::from(vec![
        Span::styled("tokctl ", Style::default().add_modifier(Modifier::BOLD)),
        Span::styled(clock, palette.dim_text()),
        Span::raw("  "),
        Span::styled(
            format!(
                "{}{} · {} · {} tok{}",
                window_label,
                scope_suffix,
                fmt_cost(total_cost),
                crate::format::fmt_wide_num(total_tokens),
                previous_window_change(state, cache, total_cost),
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

/// Cost change against the previous window of the same length, for the Models section only.
/// Omitted for `all`, inside a drill, and while a filter changes the total.
fn previous_window_change(state: &AppState, cache: &VisibleRows, total_cost: f64) -> String {
    let Some(previous) = cache.left_previous_cost else {
        return String::new();
    };
    if state.current_section != Section::Models || state.drill_active() || should_filter(state) {
        return String::new();
    }
    format!(
        " · {} vs prev {}",
        crate::models::signed_percent(total_cost, previous),
        state.time_window.as_str()
    )
}

/// Pick the totals shown in the header so they match what's actually on screen.
/// Most-specific scope wins; the returned suffix is appended after the time
/// window (e.g. " · acme-repo", " · filter:foo", or both).
pub(crate) fn header_scope(state: &AppState, cache: &VisibleRows) -> (f64, u128, String) {
    let suffix = match state.deepest_drill() {
        Some(drill) => format!(" · {}", drill.label),
        None => String::new(),
    };
    let suffix = if should_filter(state) {
        format!("{suffix} · filter:{}", state.filter.query)
    } else {
        suffix
    };
    let (cost, tokens) = match state.deepest_drill().map(|d| d.kind) {
        Some(DrillKind::Events { .. }) => {
            cache.events.iter().fold((0.0, 0), |(cost, tokens), row| {
                (
                    cost + row.cost,
                    tokens
                        + u128::from(row.input)
                        + u128::from(row.output)
                        + u128::from(row.cache_read)
                        + u128::from(row.cache_write),
                )
            })
        }
        Some(DrillKind::Sessions { .. }) => sum_sessions(&cache.sessions),
        None if state.current_section == Section::Sessions => sum_sessions(&cache.sessions),
        None if state.is_provider_view() => {
            cache.trend.iter().fold((0.0, 0), |(cost, tokens), row| {
                (cost + row.total_cost, tokens + row.total_tokens)
            })
        }
        None => cache.left.iter().fold((0.0, 0), |(cost, tokens), row| {
            (cost + row.cost, tokens + u128::from(row.total_tokens))
        }),
    };
    (cost, tokens, suffix)
}

fn sum_sessions(rows: &[crate::tui::data::SessionRow]) -> (f64, u128) {
    rows.iter().fold((0.0_f64, 0_u128), |(c, t), r| {
        (c + r.cost, t + u128::from(r.total_tokens))
    })
}

pub(crate) fn context_text(state: &AppState, width: usize) -> String {
    let source = match state.deepest_drill().map(|drill| drill.kind) {
        Some(DrillKind::Events { source }) => source.as_str(),
        _ => state.source_filter.as_str(),
    };
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
            source,
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

pub(super) fn draw_footer(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    cache: &VisibleRows,
    palette: &Palette,
) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    if let Some(status) = diagnostic_status(&cache.diagnostics) {
        frame.render_widget(
            Paragraph::new(Span::styled(status, palette.warn_text())),
            rows[0],
        );
    } else if cache.sparkline.iter().copied().fold(0f64, f64::max) <= 0.0 {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "no daily costs in this scope",
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
    if state.current_section == Section::Models && !state.drill_active() {
        view_group.push(("f", "family"));
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

pub(crate) fn diagnostic_status(
    diagnostics: &crate::store::queries::ReportDiagnostics,
) -> Option<String> {
    let mut parts = Vec::new();
    let stale_cost_rows = diagnostics.legacy_cost_rows + diagnostics.stale_pricing_rows;
    if stale_cost_rows > 0 || diagnostics.stale_parser_files > 0 {
        parts.push(format!(
            "retained stale: {stale_cost_rows} cost rows, {} parser files",
            diagnostics.stale_parser_files
        ));
    }
    if diagnostics.skipped_records > 0 {
        parts.push(format!("{} malformed records", diagnostics.skipped_records));
    }
    if diagnostics.current_file_errors > 0 {
        parts.push(format!(
            "{} input failures",
            diagnostics.current_file_errors
        ));
    }
    if !diagnostics.unknown_models.is_empty() {
        parts.push(format!(
            "{} models without prices",
            diagnostics.unknown_models.len()
        ));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

pub(crate) struct FooterMessage {
    pub(crate) text: String,
    pub(crate) is_error: bool,
}

pub(crate) fn footer_messages(state: &AppState, cache: &VisibleRows) -> Vec<FooterMessage> {
    let mut messages = Vec::new();
    if let Some(failure) = &cache.cursor_sync_failure {
        messages.push(FooterMessage {
            text: failure.clone(),
            is_error: true,
        });
    }
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

pub(super) fn draw_help(frame: &mut Frame<'_>, area: Rect, palette: &Palette) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(palette.active_border())
        .title(Span::styled(" HELP ", palette.active_border()));
    let help_area = centered(area, 72, 30);
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
        hint("f", "Models: group by model / family"),
        hint("s", "cycle sort"),
        hint("/", "fuzzy filter"),
        hint("T W M z a", "window: 1d/7d/30d/365d/all"),
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

pub(super) fn draw_detail(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    cache: &VisibleRows,
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

pub(crate) fn detail_lines(state: &AppState, cache: &VisibleRows) -> Vec<String> {
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
                        format!(
                            "tokens: {}",
                            crate::format::fmt_wide_num(u128::from(row.total_tokens))
                        ),
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
        Section::Provider | Section::Repos if state.is_provider_view() => cache
            .trend
            .get(
                state
                    .current_index()
                    .min(cache.trend.len().saturating_sub(1)),
            )
            .map(|row| {
                vec![
                    format!("bucket: {}", row.bucket),
                    format!("tokens: {}", crate::format::fmt_wide_num(row.total_tokens)),
                    format!("total: {}", fmt_cost(row.total_cost)),
                    format!("claude: {}", fmt_cost(row.claude_cost)),
                    format!("codex: {}", fmt_cost(row.codex_cost)),
                    format!("cursor: {}", fmt_cost(row.cursor_cost)),
                ]
            })
            .unwrap_or_else(|| vec!["no row selected".into()]),
        Section::Sessions => cache
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
                    format!(
                        "tokens: {}",
                        crate::format::fmt_wide_num(u128::from(row.total_tokens))
                    ),
                    format!("cost: {}", fmt_cost(row.cost)),
                    "copy: y key, Y summary".into(),
                ]
            })
            .unwrap_or_else(|| vec!["no session selected".into()]),
        _ => cache
            .left
            .get(
                state
                    .current_index()
                    .min(cache.left.len().saturating_sub(1)),
            )
            .map(|row| {
                if let Some(stats) = &row.model_stats {
                    return model_detail_lines(row, stats);
                }
                vec![
                    format!("name: {}", row.label),
                    format!("key: {}", row.key),
                    format!("sessions: {}", fmt_num(row.sessions)),
                    format!(
                        "tokens: {}",
                        crate::format::fmt_wide_num(u128::from(row.total_tokens))
                    ),
                    format!("cost: {}", fmt_cost(row.cost)),
                    "copy: y key, Y summary".into(),
                ]
            })
            .unwrap_or_else(|| vec!["no row selected".into()]),
    }
}

fn model_detail_lines(
    row: &crate::tui::data::LeftRow,
    stats: &crate::tui::data::ModelStats,
) -> Vec<String> {
    vec![
        format!("model: {}", row.label),
        format!("raw ids: {}", stats.members.join(", ")),
        format!(
            "tokens: {}",
            crate::format::fmt_wide_num(u128::from(row.total_tokens))
        ),
        if stats.priced {
            format!(
                "cost: {} ({:.1}% of window)",
                fmt_cost(row.cost),
                stats.share * 100.0
            )
        } else {
            "cost: unpriced (no price for this model)".into()
        },
        format!(
            "cache hit: {}",
            stats
                .cache_hit
                .map_or_else(|| "-".into(), |hit| format!("{:.1}%", hit * 100.0))
        ),
        "copy: y key, Y summary".into(),
    ]
}

pub(super) fn draw_filter_prompt(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    palette: &Palette,
) {
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
