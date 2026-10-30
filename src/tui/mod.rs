pub mod data;
pub mod format;
mod input;
pub mod keys;
mod projection;
pub mod shell;
pub mod state;
pub mod theme;
pub mod view;
pub mod widgets;

use anyhow::{Context, Result};
use crossterm::{
    event::{self, Event as CtEvent, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io::{stdout, IsTerminal, Stdout};

use std::time::{Duration, Instant};

use crate::store::Store;

pub use state::{AppState, Focus, Section, SourceFilter, TimeWindow};

/// Tick cadence for the footer clock / sparkline refresh.
const TICK_MS: u64 = 33;
/// Minimum width before we bail out of the sidebar/main shell.
pub const MIN_WIDTH: u16 = 80;

#[derive(Debug)]
pub enum Event {
    Input(CtEvent),
    Tick,
}

/// Raw-mode + alternate-screen guard. `Drop` restores the terminal even on
/// panic; combined with the panic hook installed in `run`, a crash never
/// leaves the user stranded in raw mode.
pub struct TerminalGuard {
    restore: fn(),
}

impl TerminalGuard {
    pub fn enter() -> Result<Self> {
        Self::enter_with(
            || enable_raw_mode().context("enabling raw mode"),
            || execute!(stdout(), EnterAlternateScreen).context("entering alternate screen"),
            Self::restore,
        )
    }

    fn enter_with(
        enable: impl FnOnce() -> Result<()>,
        enter: impl FnOnce() -> Result<()>,
        restore: fn(),
    ) -> Result<Self> {
        enable()?;
        let guard = Self { restore };
        enter()?;
        Ok(guard)
    }

    pub fn restore() {
        let _ = disable_raw_mode();
        let _ = execute!(stdout(), LeaveAlternateScreen);
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        (self.restore)();
    }
}

type PanicHook = dyn Fn(&std::panic::PanicHookInfo<'_>) + Send + Sync + 'static;

struct PanicHookGuard(std::sync::Arc<PanicHook>);

impl Drop for PanicHookGuard {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            let previous = self.0.clone();
            std::panic::set_hook(Box::new(move |info| previous(info)));
        }
    }
}

fn install_panic_hook() -> PanicHookGuard {
    let previous: std::sync::Arc<PanicHook> = std::panic::take_hook().into();
    let default = previous.clone();
    std::panic::set_hook(Box::new(move |info| {
        TerminalGuard::restore();
        default(info);
    }));
    PanicHookGuard(previous)
}

pub fn require_terminal() -> Result<()> {
    if !stdout().is_terminal() || !std::io::stdin().is_terminal() {
        anyhow::bail!(
            "tokctl ui requires an interactive terminal. \
             Use `tokctl repo` or `tokctl daily` for scripted / piped output."
        );
    }
    Ok(())
}

pub fn run_with_store_path(
    cache_path: &std::path::Path,
    roots: Option<Vec<std::path::PathBuf>>,
) -> Result<()> {
    require_terminal()?;
    crate::calendar::Calendar::system()?;

    let conn = Store::open(cache_path)
        .with_context(|| format!("opening cache at {}", cache_path.display()))?;

    let state_path = cache_path
        .parent()
        .map(|p| p.join("ui_state.json"))
        .unwrap_or_else(|| std::path::PathBuf::from("ui_state.json"));

    let mut state = state::load(&state_path);
    if !state.seen_v3_intro {
        state.flash = Some("Tab cycles tabs · sections live in the sidebar".into());
        state.seen_v3_intro = true;
    }

    let _panic_hook = install_panic_hook();
    let _guard = TerminalGuard::enter()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout())).context("terminal init")?;
    terminal.clear().context("clearing terminal")?;

    let result = event_loop(&mut terminal, conn, state, state_path.clone(), roots);
    drop(terminal);
    drop(_guard);
    if let Ok(Some(warning)) = &result {
        eprintln!("tokctl ui: {warning}");
    }
    result.map(|_| ())
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    conn: Store,
    mut state: AppState,
    state_path: std::path::PathBuf,
    roots: Option<Vec<std::path::PathBuf>>,
) -> Result<Option<String>> {
    let dirty_state_initial = state.flash.is_some();
    let mut cache = data::DataCache {
        source_roots: roots,
        ..Default::default()
    };
    cache.refresh_all(&conn, &state);
    input::clamp_cursor_to_visible_rows(&mut state, &cache);
    cache.remember_state(&state);

    let mut last_save = Instant::now();
    let mut dirty_state = dirty_state_initial;
    let mut last_g_press: Option<Instant> = None;
    // Render-side dirty flag: distinct from `dirty_state` (which drives
    // ui_state.json saves). Skip terminal.draw() unless something visible
    // actually changed. Idle CPU drops to ~0% — we only redraw on input
    // or when the footer clock minute rolls over.
    let mut dirty_render = true;
    let mut last_drawn_minute: Option<chrono::DateTime<chrono::Local>> = None;

    loop {
        if dirty_render {
            terminal
                .draw(|f| view::draw(f, &state, &cache))
                .context("render frame")?;
            dirty_render = false;
            last_drawn_minute = Some(current_minute());
        }

        let ev = if event::poll(Duration::from_millis(TICK_MS)).context("polling terminal input")? {
            Event::Input(event::read().context("reading terminal input")?)
        } else {
            Event::Tick
        };
        match ev {
            Event::Tick => {
                // Only the footer clock changes without input. It's
                // %H:%M precision, so one redraw per displayed minute.
                if cache.calendar_changed(chrono::Utc::now()) {
                    cache.refresh_all(&conn, &state);
                    cache.restore_loaded_state(&mut state);
                    input::clamp_cursor_to_visible_rows(&mut state, &cache);
                    dirty_render = true;
                }
                let now_minute = current_minute();
                if last_drawn_minute != Some(now_minute) {
                    dirty_render = true;
                }
            }
            Event::Input(CtEvent::Resize(_, _)) => {
                terminal.clear().ok();
                dirty_render = true;
            }
            Event::Input(CtEvent::Key(k)) if k.kind == KeyEventKind::Press => {
                let query_before = cache.query_for(&state, chrono::Utc::now())?;
                let action = keys::map_key(&state, k, &mut last_g_press);
                let is_yank = matches!(action, state::Action::Yank);
                let is_yank_summary = matches!(action, state::Action::YankSummary);
                let is_manual_refresh = matches!(action, state::Action::Refresh);
                // Resolve the drill row BEFORE state.apply() so we can read
                // the currently-focused row from the cache. The state's hint
                // tells us whether a Drill action is even legal here.
                let drill_target =
                    if matches!(action, state::Action::Drill) && state.can_push_drill() {
                        input::drill_target_for_current(&state, &cache)
                    } else {
                        None
                    };
                let outcome = state.apply(action);
                if outcome.quit {
                    break;
                }
                if let Some(d) = drill_target {
                    state.push_drill(d);
                    dirty_state = true;
                }
                // Manual refresh always re-queries; clear the memos first
                // so fresh data is fetched, not the cached snapshot.
                if is_manual_refresh {
                    cache.clear_memos();
                }
                if is_manual_refresh
                    || query_before != cache.query_for(&state, chrono::Utc::now())?
                {
                    cache.refresh_all(&conn, &state);
                    cache.restore_loaded_state(&mut state);
                }
                // Clamp the cursor to the visible row count so j/k past the
                // end of the list (or after the cache shrinks) doesn't drift
                // off-screen. Done after refresh so we clamp against fresh
                // data, not stale.
                input::clamp_cursor_to_visible_rows(&mut state, &cache);
                cache.remember_state(&state);
                if outcome.dirty {
                    dirty_state = true;
                }
                if is_yank {
                    let key = input::yank_key(&state, &cache);
                    if let Some(k) = key {
                        state.flash = Some(if copy_to_clipboard(&k) {
                            "copied key".into()
                        } else {
                            "clipboard unavailable".into()
                        });
                    }
                }
                if is_yank_summary {
                    let summary = input::yank_summary(&state, &cache);
                    if let Some(summary) = summary {
                        state.flash = Some(if copy_to_clipboard(&summary) {
                            "copied summary".into()
                        } else {
                            "clipboard unavailable".into()
                        });
                    }
                }
                // Any keystroke we routed through `apply` is potentially
                // visible — repaint on the next loop turn. Cheaper than
                // tracking which sub-paths set what.
                dirty_render = true;
            }
            _ => {}
        }

        if dirty_state && last_save.elapsed() >= Duration::from_millis(500) {
            if let Err(error) = state::save(&state_path, &state) {
                state.flash = Some(format!("could not save preferences: {error}"));
                dirty_render = true;
            }
            dirty_state = false;
            last_save = Instant::now();
        }
    }

    // Final save on clean exit.
    let warning = state::save(&state_path, &state)
        .err()
        .map(|error| format!("could not save preferences: {error}"));
    Ok(warning)
}

fn current_minute() -> chrono::DateTime<chrono::Local> {
    use chrono::{Local, Timelike};
    Local::now()
        .with_second(0)
        .and_then(|t| t.with_nanosecond(0))
        .unwrap_or_else(Local::now)
}

#[cfg(feature = "clipboard")]
fn copy_to_clipboard(s: &str) -> bool {
    arboard::Clipboard::new()
        .and_then(|mut cb| cb.set_text(s.to_owned()))
        .is_ok()
}

#[cfg(not(feature = "clipboard"))]
fn copy_to_clipboard(_: &str) -> bool {
    false
}

#[cfg(test)]
mod terminal_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn setup_failure_after_raw_mode_restores_terminal() {
        static RESTORED: AtomicUsize = AtomicUsize::new(0);
        let result = TerminalGuard::enter_with(
            || Ok(()),
            || anyhow::bail!("alternate screen unavailable"),
            || {
                RESTORED.fetch_add(1, Ordering::SeqCst);
            },
        );
        assert!(result.is_err());
        assert_eq!(RESTORED.load(Ordering::SeqCst), 1);
    }
}

#[cfg(test)]
mod terminal_panic_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn unwinding_restores_terminal() {
        static RESTORED: AtomicUsize = AtomicUsize::new(0);
        let result = std::panic::catch_unwind(|| {
            let _guard = TerminalGuard::enter_with(
                || Ok(()),
                || Ok(()),
                || {
                    RESTORED.fetch_add(1, Ordering::SeqCst);
                },
            )
            .unwrap();
            panic!("render failed");
        });
        assert!(result.is_err());
        assert_eq!(RESTORED.load(Ordering::SeqCst), 1);
    }
}
