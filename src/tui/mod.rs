pub mod data;
pub mod format;
pub mod keys;
pub mod state;
pub mod theme;
pub mod view;

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

pub use state::{AppState, LeftAxis, SourceFilter, TimeWindow};

/// Tick cadence for the footer clock / sparkline refresh.
const TICK_MS: u64 = 33;
/// Minimum width before we bail out of the three-pane layout.
pub const MIN_WIDTH: u16 = 72;

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

    let state = state::load(&state_path);

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
    let mut dirty_state = false;
    let mut last_g_press: Option<Instant> = None;

    loop {
        terminal
            .draw(|f| view::draw(f, &state, &cache))
            .context("render frame")?;

        let ev = rx.recv().context("event channel closed")?;
        match ev {
            Event::Tick => { /* redraw-only */ }
            Event::Input(CtEvent::Resize(_, _)) => {
                terminal.clear().ok();
            }
            Event::Input(CtEvent::Key(k)) if k.kind == KeyEventKind::Press => {
                let action = keys::map_key(&state, k, &mut last_g_press);
                let is_yank = matches!(action, state::Action::Yank);
                let is_yank_summary = matches!(action, state::Action::YankSummary);
                let outcome = state.apply(action);
                if outcome.quit {
                    break;
                }
                if outcome.needs_refresh {
                    cache.refresh_for(&conn, &state, outcome.refresh);
                }
                if outcome.dirty {
                    dirty_state = true;
                }
                if is_yank {
                    let key = yank_key(&state, &cache);
                    if let Some(k) = key {
                        state.flash = Some(if copy_to_clipboard(&k) {
                            "copied key".into()
                        } else {
                            "clipboard unavailable".into()
                        });
                    }
                }
                if is_yank_summary {
                    let summary = yank_summary(&state, &cache);
                    if let Some(summary) = summary {
                        state.flash = Some(if copy_to_clipboard(&summary) {
                            "copied summary".into()
                        } else {
                            "clipboard unavailable".into()
                        });
                    }
                }
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

fn yank_key(state: &AppState, cache: &data::DataCache) -> Option<String> {
    use state::PaneId;
    match state.focus {
        PaneId::Left => cache
            .left
            .get(state.left_index.min(cache.left.len().saturating_sub(1)))
            .map(|r| r.key.clone()),
        PaneId::Sessions => cache
            .sessions
            .get(
                state
                    .sessions_index
                    .min(cache.sessions.len().saturating_sub(1)),
            )
            .map(|r| r.session_id.clone()),
    }
}

fn yank_summary(state: &AppState, cache: &data::DataCache) -> Option<String> {
    use state::PaneId;
    match state.focus {
        PaneId::Left => cache
            .left
            .get(state.left_index.min(cache.left.len().saturating_sub(1)))
            .map(|r| {
                format!(
                    "{} · {} sessions · {} tokens · {}",
                    r.label,
                    r.sessions,
                    r.total_tokens,
                    crate::tui::format::fmt_cost(r.cost)
                )
            }),
        PaneId::Sessions => cache
            .sessions
            .get(
                state
                    .sessions_index
                    .min(cache.sessions.len().saturating_sub(1)),
            )
            .map(|r| {
                format!(
                    "{}:{} · {} · {} tokens · {}",
                    r.source.as_str(),
                    r.session_id,
                    r.project.clone().unwrap_or_else(|| "(unknown)".into()),
                    r.total_tokens,
                    crate::tui::format::fmt_cost(r.cost)
                )
            }),
    }
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
