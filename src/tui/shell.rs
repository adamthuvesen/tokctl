//! Sidebar/main shell rendering primitives.
//!
//! - [`draw_sidebar`] draws the left "Sections" list.
//! - [`draw_main_frame`] draws the main pane border, with an optional welded tab row.
//!
//! The tab-bar trick replaces the rounded border's `╭─...─╮` top edge with
//! tab labels, then welds the active tab to the inner pane by patching the
//! corner glyphs (`┘` / `└`) on the seam line. The result reads as one
//! continuous border instead of two stacked rectangles.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Widget},
    Frame,
};

use crate::tui::state::{AppState, Focus, Section};
use crate::tui::theme::Palette;

/// Draw the left sidebar — claude-squad inspired: floating pill header,
/// no surrounding border, numbered + spaced entries with a full-width
/// lavender highlight on the active section.
pub fn draw_sidebar(frame: &mut Frame<'_>, area: Rect, state: &AppState, palette: &Palette) {
    if area.width < 6 || area.height < 4 {
        return;
    }

    // -- Pill header ------------------------------------------------------
    // A short violet block with `Sections` inside, padded one column from
    // the sidebar edge. No rounded border around the sidebar.
    let pill_style = Style::default()
        .bg(palette.accent)
        .fg(Color::White)
        .add_modifier(Modifier::BOLD);
    let pill_line = Line::from(vec![Span::raw(" "), Span::styled(" Sections ", pill_style)]);
    let pill_rect = Rect {
        x: area.x,
        y: area.y + 1,
        width: area.width,
        height: 1,
    };
    frame.render_widget(Paragraph::new(pill_line), pill_rect);

    // -- Items ------------------------------------------------------------
    // Each item gets one row + one blank row between, starting two rows
    // below the pill (so there's breathing room).
    let items_top = area.y + 3;
    let item_inset_x = area.x + 1;
    let item_width = area.width.saturating_sub(2);
    let active = state.focus == Focus::Sidebar;

    for (i, section) in Section::ALL.iter().enumerate() {
        let row_y = items_top + (i as u16 * 2);
        if row_y >= area.y + area.height {
            break;
        }
        let row_rect = Rect {
            x: item_inset_x,
            y: row_y,
            width: item_width,
            height: 1,
        };
        let is_current = *section == state.current_section;
        let is_cursor = active && state.sidebar_index == i && !is_current;

        // Full-width lavender background on the active section.
        if is_current {
            frame.render_widget(Block::default().style(palette.selected_row()), row_rect);
        }

        let num_style = if is_current {
            palette.selected_row().add_modifier(Modifier::BOLD)
        } else if is_cursor {
            palette.accent_text()
        } else {
            palette.dim_text()
        };
        let label_style = if is_current {
            palette.selected_row().add_modifier(Modifier::BOLD)
        } else if is_cursor {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };

        // `▌` accent gutter on the active section when sidebar has focus.
        let gutter = if is_current && active {
            Span::styled("▌", palette.accent_text())
        } else {
            Span::raw(" ")
        };

        let line = Line::from(vec![
            gutter,
            Span::styled(format!("{}.", i + 1), num_style),
            Span::raw(" "),
            Span::styled(section.label().to_owned(), label_style),
        ]);
        frame.render_widget(Paragraph::new(line), row_rect);
    }
}

/// Draw the main pane border. If `tabs` is non-empty a tab row is drawn at
/// the top-left, welded to the rounded border via corner-glyph swaps. If
/// `tabs` is empty, a normal block with `title` is drawn.
///
/// Returns the inner content rect (already inside the border + below the
/// tab row, when present).
pub fn draw_main_frame(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    tabs: &[&str],
    active_tab: usize,
    focused: bool,
    palette: &Palette,
) -> Rect {
    if tabs.is_empty() {
        // Single-lens section: just a normal titled block.
        let block = main_block(focused, palette).title(Span::styled(
            if focused {
                format!("[ {title} ]")
            } else {
                format!(" {title} ")
            },
            if focused {
                palette.active_border()
            } else {
                palette.dim_text()
            },
        ));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        return inner;
    }

    // Tabbed layout: render the block first, then patch the top edge.
    let block = main_block(focused, palette);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Now overdraw the top border row with tab labels.
    if area.height < 2 || area.width < 4 {
        return inner;
    }
    let border_style = if focused {
        palette.active_border()
    } else {
        palette.inactive_border()
    };
    TabRow {
        tabs,
        active_tab,
        border_style,
        active_style: palette.tab_active_style(),
        inactive_style: palette.tab_inactive_style(),
    }
    .render(area, frame.buffer_mut());

    inner
}

fn main_block<'a>(focused: bool, palette: &Palette) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(if focused {
            BorderType::Thick
        } else {
            BorderType::Rounded
        })
        .border_style(if focused {
            palette.active_border()
        } else {
            palette.inactive_border()
        })
}

struct TabRow<'a> {
    tabs: &'a [&'a str],
    active_tab: usize,
    border_style: Style,
    active_style: Style,
    inactive_style: Style,
}

impl<'a> Widget for TabRow<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Top row of `area` is the border row we're overwriting. Draw labels
        // starting at x = area.x + 2 (one past the corner + 1 gutter).
        let row_y = area.y;
        let max_x = area.x + area.width.saturating_sub(1);
        let mut x = area.x + 2;
        for (i, label) in self.tabs.iter().enumerate() {
            let is_active = i == self.active_tab;
            // Wider pill: 2 spaces of padding inside, 2 cols gap between tabs.
            let label_text = format!("  {label}  ");
            let label_w = label_text.chars().count() as u16;
            if x + label_w + 2 > max_x {
                break;
            }
            // Active tab welds: replace the border characters that bracket
            // the label with `┘` and `└`, and use the active style for the
            // label cells. Inactive tabs render as plain text on the border.
            if is_active {
                if x > area.x {
                    let cell = &mut buf[(x.saturating_sub(1), row_y)];
                    cell.set_symbol("┘");
                    cell.set_style(self.border_style);
                }
                for (offset, ch) in label_text.chars().enumerate() {
                    let cx = x + offset as u16;
                    if cx > max_x {
                        break;
                    }
                    let cell = &mut buf[(cx, row_y)];
                    cell.set_symbol(&ch.to_string());
                    cell.set_style(self.active_style);
                }
                let trailing_x = x + label_w;
                if trailing_x <= max_x {
                    let cell = &mut buf[(trailing_x, row_y)];
                    cell.set_symbol("└");
                    cell.set_style(self.border_style);
                }
                x = trailing_x + 2;
            } else {
                for (offset, ch) in label_text.chars().enumerate() {
                    let cx = x + offset as u16;
                    if cx > max_x {
                        break;
                    }
                    let cell = &mut buf[(cx, row_y)];
                    cell.set_symbol(&ch.to_string());
                    cell.set_style(self.inactive_style);
                }
                x += label_w + 2;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    fn render_main(width: u16, height: u16, tabs: &[&str], active: usize) -> String {
        let backend = TestBackend::new(width, height);
        let mut term = Terminal::new(backend).unwrap();
        let palette = Palette::default();
        term.draw(|f| {
            let area = Rect::new(0, 0, width, height);
            let _inner = draw_main_frame(f, area, "REPOS", tabs, active, true, &palette);
        })
        .unwrap();
        let buf = term.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..height {
            for x in 0..width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn no_tabs_draws_titled_block() {
        let s = render_main(20, 4, &[], 0);
        // Top-left should be a thick border corner.
        assert!(s.contains("REPOS"));
        let first_line: &str = s.lines().next().unwrap();
        assert!(first_line.starts_with("┏") || first_line.starts_with("┎"));
    }

    #[test]
    fn two_tabs_render_with_weld_glyphs() {
        let s = render_main(40, 4, &["Costs", "Provider"], 0);
        // Tab labels appear on the top border row.
        let first_line: &str = s.lines().next().unwrap();
        assert!(first_line.contains("Costs"));
        assert!(first_line.contains("Provider"));
        // Active tab weld glyphs.
        assert!(first_line.contains("┘") || first_line.contains("└"));
    }

    #[test]
    fn narrow_width_truncates_gracefully() {
        // Should not panic; second tab may not fit.
        let s = render_main(14, 4, &["Costs", "Provider"], 0);
        assert!(s.contains("Costs"));
    }

    #[test]
    fn single_tab_still_renders() {
        let s = render_main(30, 4, &["Costs"], 0);
        assert!(s.contains("Costs"));
    }
}
