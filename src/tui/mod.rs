pub mod data;
pub mod format;
mod input;
pub mod keys;
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
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crate::store::{open_store, store_path};

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
    restored: bool,
}

impl TerminalGuard {
    pub fn enter() -> Result<Self> {
        enable_raw_mode().context("enabling raw mode")?;
        execute!(stdout(), EnterAlternateScreen).context("entering alternate screen")?;
        Ok(Self { restored: false })
    }

    pub fn restore() {
        let _ = disable_raw_mode();
        let _ = execute!(stdout(), LeaveAlternateScreen);
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        if !self.restored {
            Self::restore();
            self.restored = true;
        }
    }
}

fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        TerminalGuard::restore();
        default(info);
    }));
}

/// Public entry point wired to `tokctl ui`.
pub fn run() -> Result<()> {
    if !stdout().is_terminal() {
        anyhow::bail!(
            "tokctl ui requires an interactive terminal. \
             Use `tokctl repo` or `tokctl daily` for scripted / piped output."
        );
    }

    let cache_path = store_path();
    let conn = open_store(&cache_path)
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

    install_panic_hook();
    let _guard = TerminalGuard::enter()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout())).context("terminal init")?;
    terminal.clear().ok();

    let result = event_loop(&mut terminal, conn, state, state_path.clone());
    TerminalGuard::restore();
    result
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    conn: rusqlite::Connection,
    mut state: AppState,
    state_path: std::path::PathBuf,
) -> Result<()> {
    let dirty_state_initial = state.flash.is_some();
    let (tx, rx) = mpsc::channel::<Event>();

    // Input thread.
    let input_tx = tx.clone();
    thread::spawn(move || loop {
        if let Ok(ev) = event::read() {
            if input_tx.send(Event::Input(ev)).is_err() {
                break;
            }
        }
    });

    // Tick thread.
    let tick_tx = tx.clone();
    thread::spawn(move || loop {
        thread::sleep(Duration::from_millis(TICK_MS));
        if tick_tx.send(Event::Tick).is_err() {
            break;
        }
    });

    let mut cache = data::DataCache::default();
    cache.refresh_all(&conn, &state);

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

        let ev = rx.recv().context("event channel closed")?;
        match ev {
            Event::Tick => {
                // Only the footer clock changes without input. It's
                // %H:%M precision, so one redraw per displayed minute.
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
                if outcome.needs_refresh {
                    cache.refresh_for(&conn, &state, outcome.refresh);
                }
                // Clamp the cursor to the visible row count so j/k past the
                // end of the list (or after the cache shrinks) doesn't drift
                // off-screen. Done after refresh so we clamp against fresh
                // data, not stale.
                input::clamp_cursor_to_visible_rows(&mut state, &cache);
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
            state::save(&state_path, &state).ok();
            dirty_state = false;
            last_save = Instant::now();
        }
    }

    // Final save on clean exit.
    state::save(&state_path, &state).ok();
    Ok(())
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
