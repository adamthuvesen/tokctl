pub mod data;
pub mod format;
pub mod keys;
pub mod shell;
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
                        drill_target_for_current(&state, &cache)
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
                clamp_cursor_to_visible_rows(&mut state, &cache);
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

fn yank_key(state: &AppState, cache: &data::DataCache) -> Option<String> {
    match state.deepest_drill() {
        Some(d) => match d.kind {
            state::DrillKind::Sessions { .. } => cache
                .sessions
                .get(
                    state
                        .current_index()
                        .min(cache.sessions.len().saturating_sub(1)),
                )
                .map(|r| r.session_id.clone()),
            state::DrillKind::Events { source } => cache
                .events
                .get(
                    state
                        .current_index()
                        .min(cache.events.len().saturating_sub(1)),
                )
                .map(|r| format!("{}/{}@{}", source.as_str(), d.key, r.ts.to_rfc3339())),
        },
        None => match state.current_section {
            Section::Provider => cache
                .trend
                .get(
                    state
                        .current_index()
                        .min(cache.trend.len().saturating_sub(1)),
                )
                .map(|r| r.bucket.clone()),
            _ => cache
                .left
                .get(
                    state
                        .current_index()
                        .min(cache.left.len().saturating_sub(1)),
                )
                .map(|r| r.key.clone()),
        },
    }
}

fn yank_summary(state: &AppState, cache: &data::DataCache) -> Option<String> {
    match state.deepest_drill() {
        Some(d) => match d.kind {
            state::DrillKind::Sessions { .. } => cache
                .sessions
                .get(
                    state
                        .current_index()
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
            state::DrillKind::Events { .. } => cache
                .events
                .get(
                    state
                        .current_index()
                        .min(cache.events.len().saturating_sub(1)),
                )
                .map(|r| {
                    let when = r.ts.with_timezone(&chrono::Local).format("%H:%M:%S");
                    format!(
                        "{} {} in={} out={} cost={}",
                        when,
                        r.model,
                        r.input,
                        r.output,
                        crate::tui::format::fmt_cost(r.cost)
                    )
                }),
        },
        None => match state.current_section {
            Section::Provider => cache
                .trend
                .get(
                    state
                        .current_index()
                        .min(cache.trend.len().saturating_sub(1)),
                )
                .map(|r| {
                    format!(
                        "{} · {} tokens · {}",
                        r.bucket,
                        r.total_tokens,
                        crate::tui::format::fmt_cost(r.total_cost)
                    )
                }),
            _ => cache
                .left
                .get(
                    state
                        .current_index()
                        .min(cache.left.len().saturating_sub(1)),
                )
                .map(|r| {
                    format!(
                        "{} · {} sessions · {} tokens · {}",
                        r.label,
                        r.sessions,
                        r.total_tokens,
                        crate::tui::format::fmt_cost(r.cost)
                    )
                }),
        },
    }
}

/// Build the next drill target from the focused row in the current view.
/// The kind of drill we're about to push is defined by the state machine
/// (via `next_drill_kind_hint`); here we just attach the row's key + label
/// (and the source for events) by reading the appropriate cache slice.
fn drill_target_for_current(state: &AppState, cache: &data::DataCache) -> Option<state::Drill> {
    let kind = state.next_drill_kind_hint()?;
    match kind {
        state::DrillKind::Sessions { from_section } => {
            if cache.left.is_empty() {
                return None;
            }
            let idx = state.current_index().min(cache.left.len() - 1);
            let row = &cache.left[idx];
            Some(state::Drill {
                kind: state::DrillKind::Sessions { from_section },
                key: row.key.clone(),
                label: row.label.clone(),
                cursor: 0,
            })
        }
        state::DrillKind::Events { .. } => {
            // Events drills come from a session row — either Section::Sessions
            // at the root or a sessions-drill view. The session row tells us
            // the actual source.
            match state.deepest_drill() {
                None => {
                    if cache.left.is_empty() {
                        return None;
                    }
                    let idx = state.current_index().min(cache.left.len() - 1);
                    let row = &cache.left[idx];
                    // Section::Sessions LeftRows now carry the real source.
                    // Old fallback (Claude) only kicks in if upstream parsing
                    // produced an "all" SourceLabel, which shouldn't happen
                    // for a single-session aggregation.
                    let source = row.source.unwrap_or(crate::types::Source::Claude);
                    Some(state::Drill {
                        kind: state::DrillKind::Events { source },
                        key: row.key.clone(),
                        label: short_session_label(&row.key),
                        cursor: 0,
                    })
                }
                Some(_sessions_drill) => {
                    if cache.sessions.is_empty() {
                        return None;
                    }
                    let idx = state.current_index().min(cache.sessions.len() - 1);
                    let row = &cache.sessions[idx];
                    Some(state::Drill {
                        kind: state::DrillKind::Events { source: row.source },
                        key: row.session_id.clone(),
                        label: short_session_label(&row.session_id),
                        cursor: 0,
                    })
                }
            }
        }
    }
}

/// Returns the local time truncated to the current minute. Used to detect
/// footer-clock rollover so we redraw at most once per displayed minute
/// rather than once per Tick (~30/sec).
fn current_minute() -> chrono::DateTime<chrono::Local> {
    use chrono::{Local, Timelike};
    Local::now()
        .with_second(0)
        .and_then(|t| t.with_nanosecond(0))
        .unwrap_or_else(Local::now)
}

/// Cap the focused-row cursor at `rows.len() - 1` for whichever cache
/// slice is currently rendered. Without this, repeated `j` past the end
/// makes `current_index()` drift past the data and the highlight vanishes
/// off-screen even with a stateful viewport.
fn clamp_cursor_to_visible_rows(state: &mut state::AppState, cache: &data::DataCache) {
    let rows = match state.deepest_drill().map(|d| d.kind) {
        Some(state::DrillKind::Sessions { .. }) => cache.sessions.len(),
        Some(state::DrillKind::Events { .. }) => cache.events.len(),
        None => match state.current_section {
            state::Section::Provider => cache.trend.len(),
            _ => cache.left.len(),
        },
    };
    if rows == 0 {
        state.set_current_index(0);
        return;
    }
    let cur = state.current_index();
    if cur >= rows {
        state.set_current_index(rows - 1);
    }
}

/// Compact label used in the breadcrumb for a session id. Keeps the first
/// 8 chars and ellipsizes — enough to disambiguate while staying narrow.
fn short_session_label(session_id: &str) -> String {
    let mut s: String = session_id.chars().take(8).collect();
    if session_id.chars().count() > 8 {
        s.push('…');
    }
    s
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
