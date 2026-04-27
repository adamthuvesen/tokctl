use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::time::{Duration, Instant};

use crate::tui::state::{
    Action, AppState, Focus, Section, SourceFilter, TimeWindow, TrendGranularity,
};

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

    // Provider / Days scoped overrides: d/w/m/y choose bucket granularity.
    // (The two sections share `trend_granularity`.) Window-setting on
    // lowercase w/m is shadowed in these sections — use uppercase W/M.
    if matches!(state.current_section, Section::Provider | Section::Days) && state.drill.is_none() {
        match k.code {
            KeyCode::Char('d') => return Action::SetTrendGranularity(TrendGranularity::Daily),
            KeyCode::Char('y') => return Action::SetTrendGranularity(TrendGranularity::Yearly),
            KeyCode::Char('w') => return Action::SetTrendGranularity(TrendGranularity::Weekly),
            KeyCode::Char('m') => return Action::SetTrendGranularity(TrendGranularity::Monthly),
            _ => {}
        }
    }

    match k.code {
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Char('?') => Action::ToggleHelp,
        // `t` jumps to Provider section (was: toggle modal).
        KeyCode::Char('t') => Action::JumpToSection(Section::Provider),
        KeyCode::Char('r') => Action::Refresh,
        KeyCode::Char('e') => Action::ToggleExpand,
        KeyCode::Char('s') => Action::CycleSort,
        KeyCode::Char('i') => Action::ToggleDetail,
        KeyCode::Tab => Action::CycleTab,
        KeyCode::Char('/') => Action::FilterOpen,
        KeyCode::Char('y') => Action::Yank,
        KeyCode::Char('Y') => Action::YankSummary,
        KeyCode::Char('[') => Action::PrevSection,
        KeyCode::Char(']') => Action::NextSection,
        KeyCode::Enter => {
            if state.focus == Focus::Sidebar {
                Action::FocusMain
            } else {
                Action::Drill
            }
        }
        KeyCode::Esc | KeyCode::Backspace => Action::Pop,
        KeyCode::Char('G') => Action::Bottom,
        KeyCode::Left | KeyCode::Char('h') => Action::PopDrill,
        KeyCode::Right | KeyCode::Char('l') => Action::FocusMain,
        KeyCode::Up | KeyCode::Char('k') => Action::MoveUp,
        KeyCode::Down | KeyCode::Char('j') => Action::MoveDown,
        // Time window — uppercase + a/z to avoid clashing with d/w/m/y in Provider.
        KeyCode::Char('T') => Action::SetWindow(TimeWindow::Today),
        KeyCode::Char('W') => Action::SetWindow(TimeWindow::Week),
        KeyCode::Char('M') => Action::SetWindow(TimeWindow::Month),
        KeyCode::Char('z') => Action::SetWindow(TimeWindow::Year),
        KeyCode::Char('a') => Action::SetWindow(TimeWindow::All),
        KeyCode::Char('1') => Action::SetSource(SourceFilter::All),
        KeyCode::Char('2') => Action::SetSource(SourceFilter::Claude),
        KeyCode::Char('3') => Action::SetSource(SourceFilter::Codex),
        KeyCode::Char('4') => Action::SetSource(SourceFilter::Cursor),
        // Outside Provider: w/m/y can fall through to time window for back-compat.
        KeyCode::Char('w') => Action::SetWindow(TimeWindow::Week),
        KeyCode::Char('m') => Action::SetWindow(TimeWindow::Month),
        _ => Action::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn maps_detail_and_summary_copy() {
        let state = AppState::default();
        let mut last_g = None;
        let detail = map_key(&state, key(KeyCode::Char('i')), &mut last_g);
        assert!(matches!(detail, Action::ToggleDetail));
        let summary = map_key(
            &state,
            KeyEvent::new(KeyCode::Char('Y'), KeyModifiers::SHIFT),
            &mut last_g,
        );
        assert!(matches!(summary, Action::YankSummary));
    }

    #[test]
    fn tab_cycles_tabs() {
        let state = AppState::default();
        let mut last_g = None;
        let action = map_key(&state, key(KeyCode::Tab), &mut last_g);
        assert!(matches!(action, Action::CycleTab));
    }

    #[test]
    fn brackets_jump_sections() {
        let state = AppState::default();
        let mut last_g = None;
        assert!(matches!(
            map_key(&state, key(KeyCode::Char(']')), &mut last_g),
            Action::NextSection
        ));
        assert!(matches!(
            map_key(&state, key(KeyCode::Char('[')), &mut last_g),
            Action::PrevSection
        ));
    }

    #[test]
    fn t_jumps_to_provider_section() {
        let state = AppState::default();
        let mut last_g = None;
        let action = map_key(&state, key(KeyCode::Char('t')), &mut last_g);
        assert!(matches!(action, Action::JumpToSection(Section::Provider)));
    }

    #[test]
    fn enter_drills_in_main() {
        let state = AppState {
            focus: Focus::Main,
            ..AppState::default()
        };
        let mut last_g = None;
        let action = map_key(&state, key(KeyCode::Enter), &mut last_g);
        assert!(matches!(action, Action::Drill));
    }

    #[test]
    fn enter_focuses_main_from_sidebar() {
        let state = AppState {
            focus: Focus::Sidebar,
            ..AppState::default()
        };
        let mut last_g = None;
        let action = map_key(&state, key(KeyCode::Enter), &mut last_g);
        assert!(matches!(action, Action::FocusMain));
    }

    #[test]
    fn h_pops_drill() {
        let state = AppState::default();
        let mut last_g = None;
        let action = map_key(&state, key(KeyCode::Char('h')), &mut last_g);
        assert!(matches!(action, Action::PopDrill));
    }

    #[test]
    fn esc_pops() {
        let state = AppState::default();
        let mut last_g = None;
        let action = map_key(&state, key(KeyCode::Esc), &mut last_g);
        assert!(matches!(action, Action::Pop));
    }

    #[test]
    fn d_in_provider_sets_daily_granularity() {
        let state = AppState {
            current_section: Section::Provider,
            ..AppState::default()
        };
        let mut last_g = None;
        let action = map_key(&state, key(KeyCode::Char('d')), &mut last_g);
        assert!(matches!(
            action,
            Action::SetTrendGranularity(TrendGranularity::Daily)
        ));
    }

    #[test]
    fn uppercase_t_sets_today_window() {
        let state = AppState::default();
        let mut last_g = None;
        let action = map_key(
            &state,
            KeyEvent::new(KeyCode::Char('T'), KeyModifiers::SHIFT),
            &mut last_g,
        );
        assert!(matches!(action, Action::SetWindow(TimeWindow::Today)));
    }
}
