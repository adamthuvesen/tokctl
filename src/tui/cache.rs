use chrono::{DateTime, Utc};
use rusqlite::Connection;
use std::collections::BTreeMap;

use super::load::{
    base_filter, left_selected_key, load_cache_status, load_events_for, load_left_axis,
    load_sessions_for, load_sparkline, load_trend, sort_event_rows, sort_left_rows,
    sort_session_rows,
};
use super::rows::{
    CacheStatus, EventRow, LeftMemoKey, LeftRow, RefreshError, RefreshScope, SessionRow,
    TrendMemoKey, TrendRow,
};
use crate::store::queries::QueryFilter;
use crate::tui::state::{AppState, DrillKind, Section};

#[derive(Debug, Default, Clone)]
pub struct DataCache {
    pub left: Vec<LeftRow>,
    pub sessions: Vec<SessionRow>,
    pub events: Vec<EventRow>,
    pub sparkline: Vec<f64>,
    pub trend: Vec<TrendRow>,
    pub status: CacheStatus,
    pub refresh_error: Option<RefreshError>,
    pub(crate) left_memo: BTreeMap<LeftMemoKey, Vec<LeftRow>>,
    pub(crate) trend_memo: BTreeMap<TrendMemoKey, Vec<TrendRow>>,
    pub(crate) sparkline_memo: Option<Vec<f64>>,
    pub(crate) memo_mtime_ns: Option<i64>,
}

impl DataCache {
    pub fn refresh_all(&mut self, conn: &Connection, state: &AppState) {
        self.refresh_for(conn, state, crate::tui::state::RefreshMask::all());
    }

    pub fn clear_memos(&mut self) {
        self.left_memo.clear();
        self.trend_memo.clear();
        self.sparkline_memo = None;
        self.memo_mtime_ns = None;
    }

    fn set_refresh_error(&mut self, scope: RefreshScope, err: impl std::fmt::Display) {
        self.refresh_error = Some(RefreshError::new(scope, err));
    }

    fn clear_refresh_error(&mut self, scope: RefreshScope) {
        if self
            .refresh_error
            .as_ref()
            .is_some_and(|e| e.scope == scope)
        {
            self.refresh_error = None;
        }
    }

    pub fn refresh_for(
        &mut self,
        conn: &Connection,
        state: &AppState,
        mask: crate::tui::state::RefreshMask,
    ) {
        let now = Utc::now();
        let filter = base_filter(state, now);

        self.refresh_status(conn, now);
        self.invalidate_memos_if_source_files_changed();

        if mask.left {
            self.refresh_left(conn, state, &filter);
        }
        if mask.sessions {
            self.refresh_sessions(conn, state, &filter);
        }
        if mask.events {
            self.refresh_events(conn, state, &filter);
        }
        if mask.sparkline {
            self.refresh_sparkline(conn);
        }
        if mask.trend {
            self.refresh_trend(conn, state, now);
        }
    }

    fn refresh_status(&mut self, conn: &Connection, now: DateTime<Utc>) {
        match load_cache_status(conn, now) {
            Ok(status) => {
                self.status = status;
                self.clear_refresh_error(RefreshScope::Status);
            }
            Err(err) => self.set_refresh_error(RefreshScope::Status, err),
        }
    }

    fn invalidate_memos_if_source_files_changed(&mut self) {
        if self.memo_mtime_ns != self.status.mtime_ns {
            self.left_memo.clear();
            self.trend_memo.clear();
            self.sparkline_memo = None;
            self.memo_mtime_ns = self.status.mtime_ns;
        }
    }

    fn refresh_left(&mut self, conn: &Connection, state: &AppState, filter: &QueryFilter) {
        // Sessions section at root shows `SessionRow` data in the main pane, not `LeftRow`.
        if state.current_section == Section::Sessions && !state.drill_active() {
            match load_sessions_for(conn, Section::Sessions, None, filter.clone()) {
                Ok(rows) => {
                    self.sessions = rows;
                    sort_session_rows(&mut self.sessions, state.sort);
                    self.left.clear();
                    self.clear_refresh_error(RefreshScope::Sessions);
                    self.clear_refresh_error(RefreshScope::Left);
                }
                Err(err) => self.set_refresh_error(RefreshScope::Sessions, err),
            }
            return;
        }

        let key = LeftMemoKey(
            state.current_section,
            state.time_window,
            state.source_filter,
            state.trend_granularity,
        );
        if let Some(cached) = self.left_memo.get(&key) {
            self.left = cached.clone();
            sort_left_rows(&mut self.left, state.current_section, state.sort);
            self.clear_refresh_error(RefreshScope::Left);
            return;
        }

        match load_left_axis(
            conn,
            state.current_section,
            filter.clone(),
            state.trend_granularity,
        ) {
            Ok(rows) => {
                self.left = rows;
                self.left_memo.insert(key, self.left.clone());
                sort_left_rows(&mut self.left, state.current_section, state.sort);
                self.clear_refresh_error(RefreshScope::Left);
            }
            Err(err) => self.set_refresh_error(RefreshScope::Left, err),
        }
    }

    fn refresh_sessions(&mut self, conn: &Connection, state: &AppState, filter: &QueryFilter) {
        if matches!(
            state.deepest_drill().map(|d| d.kind),
            Some(DrillKind::Events { .. })
        ) {
            return;
        }

        if state.current_section == Section::Sessions && !state.drill_active() {
            return;
        }

        let (scope_section, sel_key): (Section, Option<String>) = match state.deepest_drill() {
            Some(d) => match d.kind {
                DrillKind::Sessions { from_section } => (from_section, Some(d.key.clone())),
                DrillKind::Events { .. } => {
                    (state.current_section, left_selected_key(state, &self.left))
                }
            },
            None => (state.current_section, left_selected_key(state, &self.left)),
        };

        match load_sessions_for(conn, scope_section, sel_key.as_deref(), filter.clone()) {
            Ok(rows) => {
                self.sessions = rows;
                sort_session_rows(&mut self.sessions, state.sort);
                self.clear_refresh_error(RefreshScope::Sessions);
            }
            Err(err) => self.set_refresh_error(RefreshScope::Sessions, err),
        }
    }

    fn refresh_events(&mut self, conn: &Connection, state: &AppState, filter: &QueryFilter) {
        match state.deepest_drill() {
            Some(d) => match d.kind {
                DrillKind::Events { source } => {
                    match load_events_for(conn, source, &d.key, filter.clone()) {
                        Ok(rows) => {
                            self.events = rows;
                            sort_event_rows(&mut self.events, state.sort);
                            self.clear_refresh_error(RefreshScope::Events);
                        }
                        Err(err) => self.set_refresh_error(RefreshScope::Events, err),
                    }
                }
                _ => {
                    self.events.clear();
                    self.clear_refresh_error(RefreshScope::Events);
                }
            },
            None => {
                self.events.clear();
                self.clear_refresh_error(RefreshScope::Events);
            }
        }
    }

    fn refresh_sparkline(&mut self, conn: &Connection) {
        self.sparkline = match self.sparkline_memo.as_ref() {
            Some(cached) => {
                let rows = cached.clone();
                self.clear_refresh_error(RefreshScope::Sparkline);
                rows
            }
            None => match load_sparkline(conn, 30) {
                Ok(fresh) => {
                    self.sparkline_memo = Some(fresh.clone());
                    self.clear_refresh_error(RefreshScope::Sparkline);
                    fresh
                }
                Err(err) => {
                    self.set_refresh_error(RefreshScope::Sparkline, err);
                    self.sparkline.clone()
                }
            },
        };
    }

    fn refresh_trend(&mut self, conn: &Connection, state: &AppState, now: DateTime<Utc>) {
        let key = TrendMemoKey(
            state.time_window,
            state.source_filter,
            state.trend_granularity,
        );
        self.trend = match self.trend_memo.get(&key) {
            Some(cached) => {
                let rows = cached.clone();
                self.clear_refresh_error(RefreshScope::Trend);
                rows
            }
            None => match load_trend(conn, state, now) {
                Ok(fresh) => {
                    self.trend_memo.insert(key, fresh.clone());
                    self.clear_refresh_error(RefreshScope::Trend);
                    fresh
                }
                Err(err) => {
                    self.set_refresh_error(RefreshScope::Trend, err);
                    self.trend.clone()
                }
            },
        };
    }
}
