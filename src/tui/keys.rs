use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::time::{Duration, Instant};

use crate::tui::state::{Action, AppState, SourceFilter, TimeWindow, TrendGranularity};

const GG_WINDOW: Duration = Duration::from_millis(500);

pub fn map_key(state: &AppState, k: KeyEvent, last_g: &mut Option<Instant>) -> Action {
    // Filter mode swallows most keys.
    if state.filter.active {
        return match k.code {
            KeyCode::Esc => Action::FilterCancel,
            KeyCode::Enter => Action::FilterCommit,
            KeyCode::Backspace => Action::FilterBackspace,
            KeyCode::Char(c) => Action::FilterChar(c),
            _ => Action::None,
        };
    }

    if k.modifiers.contains(KeyModifiers::CONTROL) {
        return match k.code {
            KeyCode::Char('c') => Action::Quit,
            KeyCode::Char('d') => Action::PageDown,
            KeyCode::Char('u') => Action::PageUp,
            _ => Action::None,
        };
    }

    // Two-key gg -> Top.
    if let KeyCode::Char('g') = k.code {
        let now = Instant::now();
        if let Some(prev) = *last_g {
            if now.duration_since(prev) <= GG_WINDOW {
                *last_g = None;
                return Action::Top;
            }
        }
        *last_g = Some(now);
        return Action::None;
    } else {
        *last_g = None;
    }

    // Trend-overlay scoped overrides.
    if state.trend_open {
        match k.code {
            KeyCode::Char('d') => return Action::SetTrendGranularity(TrendGranularity::Daily),
            KeyCode::Char('w') => return Action::SetTrendGranularity(TrendGranularity::Weekly),
            KeyCode::Char('m') => return Action::SetTrendGranularity(TrendGranularity::Monthly),
            KeyCode::Char('y') => return Action::SetTrendGranularity(TrendGranularity::Yearly),
            _ => {}
        }
    }

    match k.code {
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Char('?') => Action::ToggleHelp,
        KeyCode::Char('t') => Action::ToggleTrend,
        KeyCode::Char('r') => Action::Refresh,
        KeyCode::Char('e') => Action::ToggleExpand,
        KeyCode::Char('s') => Action::CycleSort,
        KeyCode::Char('i') => Action::ToggleDetail,
        KeyCode::Tab => Action::CycleAxis,
        KeyCode::Char('/') => Action::FilterOpen,
        KeyCode::Char('y') => Action::Yank,
        KeyCode::Char('Y') => Action::YankSummary,
        KeyCode::Enter => Action::Drill,
        KeyCode::Esc | KeyCode::Backspace => Action::Pop,
        KeyCode::Char('G') => Action::Bottom,
        KeyCode::Left | KeyCode::Char('h') => Action::FocusLeft,
        KeyCode::Right | KeyCode::Char('l') => Action::FocusRight,
        KeyCode::Up | KeyCode::Char('k') => Action::MoveUp,
        KeyCode::Down | KeyCode::Char('j') => Action::MoveDown,
        KeyCode::Char('T') => Action::SetWindow(TimeWindow::Today),
        KeyCode::Char('w') => Action::SetWindow(TimeWindow::Week),
        KeyCode::Char('m') => Action::SetWindow(TimeWindow::Month),
        KeyCode::Char('z') => Action::SetWindow(TimeWindow::Year),
        KeyCode::Char('a') => Action::SetWindow(TimeWindow::All),
        KeyCode::Char('1') => Action::SetSource(SourceFilter::All),
        KeyCode::Char('2') => Action::SetSource(SourceFilter::Claude),
        KeyCode::Char('3') => Action::SetSource(SourceFilter::Codex),
        KeyCode::Char('4') => Action::SetSource(SourceFilter::Cursor),
        _ => Action::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    #[test]
    fn maps_detail_and_summary_copy() {
        let state = AppState::default();
        let mut last_g = None;
        let detail = map_key(
            &state,
            KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE),
            &mut last_g,
        );
        assert!(matches!(detail, Action::ToggleDetail));
        let summary = map_key(
            &state,
            KeyEvent::new(KeyCode::Char('Y'), KeyModifiers::SHIFT),
            &mut last_g,
        );
        assert!(matches!(summary, Action::YankSummary));
    }
}
