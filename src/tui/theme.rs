use ratatui::style::{Color, Modifier, Style};

/// Fixed five-role palette. No theming in v1.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub fg: Color,
    pub accent: Color,
    pub dim: Color,
    pub warn: Color,
    pub cost_low: Color,
    pub cost_mid: Color,
    pub cost_high: Color,
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            fg: Color::Reset,
            // Tuned to read on light AND dark terminal backgrounds.
            accent: Color::Rgb(0x7c, 0x3a, 0xed),   // violet-600
            dim: Color::Rgb(0x6b, 0x72, 0x80),      // slate-500
            warn: Color::Rgb(0xd9, 0x77, 0x06),     // amber-600
            cost_low: Color::Rgb(0x15, 0x80, 0x3d), // green-700
            cost_mid: Color::Rgb(0xca, 0x8a, 0x04), // yellow-600
            cost_high: Color::Rgb(0xdc, 0x26, 0x26), // red-600
        }
    }
}

impl Palette {
    pub fn active_border(&self) -> Style {
        Style::default()
            .fg(self.accent)
            .add_modifier(Modifier::BOLD)
    }
    pub fn inactive_border(&self) -> Style {
        Style::default().fg(self.dim)
    }
    pub fn selected_row(&self) -> Style {
        Style::default()
            .bg(self.accent)
            .fg(Color::Rgb(0xff, 0xff, 0xff))
            .add_modifier(Modifier::BOLD)
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
}
