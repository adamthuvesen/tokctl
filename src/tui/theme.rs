use ratatui::style::{Color, Modifier, Style};

/// Fixed palette with named semantic roles. All color literals live here;
/// no ad-hoc Color::Rgb values appear in view.rs.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub fg: Color,
    pub accent: Color,
    pub dim: Color,
    pub warn: Color,
    pub info: Color,
    pub cost_low: Color,
    pub cost_mid: Color,
    pub cost_high: Color,
    /// Background for the selected table row (soft lavender).
    pub selected_bg: Color,
    /// Foreground for the selected table row (dark violet).
    pub selected_fg: Color,
    /// Filled portion of proportion bars (violet-500).
    pub bar_filled: Color,
    /// Empty remainder of proportion bars (zinc-300).
    pub bar_empty: Color,
    /// Border color for inactive panes (very faint on light bg).
    pub border_inactive: Color,
    /// Background for the active main-pane tab pill.
    pub tab_active_bg: Color,
    /// Foreground for the active main-pane tab pill.
    pub tab_active_fg: Color,
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            fg: Color::Reset,
            accent: Color::Rgb(0x7c, 0x3a, 0xed),   // violet-600
            dim: Color::Rgb(0x71, 0x71, 0x7a),      // zinc-500
            warn: Color::Rgb(0xd9, 0x77, 0x06),     // amber-600
            info: Color::Rgb(0x08, 0x91, 0xb2),     // cyan-600
            cost_low: Color::Rgb(0x15, 0x80, 0x3d), // green-700
            cost_mid: Color::Rgb(0xca, 0x8a, 0x04), // yellow-600
            cost_high: Color::Rgb(0xdc, 0x26, 0x26), // red-600
            selected_bg: Color::Rgb(0xed, 0xe9, 0xfe), // violet-100 (soft lavender)
            selected_fg: Color::Rgb(0x3b, 0x07, 0x64), // violet-950 (dark)
            bar_filled: Color::Rgb(0x8b, 0x5c, 0xf6), // violet-500
            bar_empty: Color::Rgb(0xd4, 0xd4, 0xd8), // zinc-300
            border_inactive: Color::Rgb(0xd4, 0xd4, 0xd8), // zinc-300
            tab_active_bg: Color::Rgb(0xed, 0xe9, 0xfe), // violet-100 lavender pill
            tab_active_fg: Color::Rgb(0x3b, 0x07, 0x64), // violet-950
        }
    }
}

impl Palette {
    /// Active pane border: accent color + bold (for border title weight).
    pub fn active_border(&self) -> Style {
        Style::default()
            .fg(self.accent)
            .add_modifier(Modifier::BOLD)
    }

    /// Inactive pane border: very faint, recedes on light backgrounds.
    pub fn inactive_border(&self) -> Style {
        Style::default().fg(self.border_inactive)
    }

    /// Soft lavender highlight for selected rows — no bold, no inversion.
    pub fn selected_row(&self) -> Style {
        Style::default().bg(self.selected_bg).fg(self.selected_fg)
    }

    pub fn dim_text(&self) -> Style {
        Style::default().fg(self.dim)
    }
    pub fn accent_text(&self) -> Style {
        Style::default().fg(self.accent)
    }
    pub fn warn_text(&self) -> Style {
        Style::default().fg(self.warn)
    }
    pub fn info_text(&self) -> Style {
        Style::default().fg(self.info)
    }

    /// Filled portion of proportion bars.
    pub fn bar_filled_style(&self) -> Style {
        Style::default().fg(self.bar_filled)
    }

    /// Empty remainder of proportion bars.
    pub fn bar_empty_style(&self) -> Style {
        Style::default().fg(self.bar_empty)
    }

    /// Active tab pill: bold violet text on lavender bg.
    pub fn tab_active_style(&self) -> Style {
        Style::default()
            .bg(self.tab_active_bg)
            .fg(self.tab_active_fg)
            .add_modifier(Modifier::BOLD)
    }

    /// Inactive tab label: dim text, no background.
    pub fn tab_inactive_style(&self) -> Style {
        Style::default().fg(self.dim)
    }

    /// Interpolate the cost gradient. `ratio` is clamped to `[0, 1]`.
    pub fn cost_color(&self, ratio: f64) -> Color {
        let r = ratio.clamp(0.0, 1.0);
        let (a, b, t) = if r < 0.5 {
            (self.cost_low, self.cost_mid, r * 2.0)
        } else {
            (self.cost_mid, self.cost_high, (r - 0.5) * 2.0)
        };
        lerp(a, b, t)
    }
}

fn lerp(a: Color, b: Color, t: f64) -> Color {
    match (a, b) {
        (Color::Rgb(ar, ag, ab), Color::Rgb(br, bg, bb)) => {
            Color::Rgb(lerp_u8(ar, br, t), lerp_u8(ag, bg, t), lerp_u8(ab, bb, t))
        }
        _ => a,
    }
}

fn lerp_u8(a: u8, b: u8, t: f64) -> u8 {
    let af = a as f64;
    let bf = b as f64;
    (af + (bf - af) * t).round().clamp(0.0, 255.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gradient_endpoints_match_palette() {
        let p = Palette::default();
        assert_eq!(p.cost_color(0.0), p.cost_low);
        assert_eq!(p.cost_color(1.0), p.cost_high);
    }

    #[test]
    fn gradient_mid_matches_mid() {
        let p = Palette::default();
        assert_eq!(p.cost_color(0.5), p.cost_mid);
    }

    #[test]
    fn gradient_clamps() {
        let p = Palette::default();
        assert_eq!(p.cost_color(-1.0), p.cost_low);
        assert_eq!(p.cost_color(5.0), p.cost_high);
    }

    #[test]
    fn new_palette_roles_exist() {
        let p = Palette::default();
        // selected_bg is lavender (violet-100)
        assert_eq!(p.selected_bg, Color::Rgb(0xed, 0xe9, 0xfe));
        // selected_fg is dark violet
        assert_eq!(p.selected_fg, Color::Rgb(0x3b, 0x07, 0x64));
        // bar_filled is violet-500
        assert_eq!(p.bar_filled, Color::Rgb(0x8b, 0x5c, 0xf6));
        // dim is zinc-500
        assert_eq!(p.dim, Color::Rgb(0x71, 0x71, 0x7a));
    }

    #[test]
    fn selected_row_uses_lavender_no_bold() {
        let p = Palette::default();
        let s = p.selected_row();
        assert_eq!(s.bg, Some(p.selected_bg));
        assert_eq!(s.fg, Some(p.selected_fg));
        assert!(!s.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn inactive_border_uses_border_inactive_not_dim() {
        let p = Palette::default();
        let s = p.inactive_border();
        assert_eq!(s.fg, Some(p.border_inactive));
        // Should NOT be the dim color
        assert_ne!(s.fg, Some(p.dim));
    }
}
