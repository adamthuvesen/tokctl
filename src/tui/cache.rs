use chrono::{DateTime, NaiveDate, Utc};
use std::collections::BTreeMap;

use super::load::{
    base_filter, load_cache_status, load_events_for, load_left_axis, load_sessions_for, load_trend,
};
use super::rows::{
    CacheStatus, EventRow, LeftMemoKey, LeftRow, RefreshError, RefreshScope, SessionRow,
    TrendMemoKey, TrendRow,
};
use crate::store::queries::{scoped_sparkline, QueryScope};
use crate::store::Store;
use crate::tui::state::{AppState, DrillKind, Section, TrendGranularity};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ViewQuery {
    Repos(QueryScope),
    Buckets(QueryScope, TrendGranularity),
    Models(QueryScope),
    Sessions(QueryScope),
    Events(QueryScope),
    Provider(QueryScope, TrendGranularity),
}

#[derive(Debug, Default, Clone)]
pub struct DataCache {
    pub source_roots: Option<Vec<std::path::PathBuf>>,
    pub left: Vec<LeftRow>,
    pub sessions: Vec<SessionRow>,
    pub events: Vec<EventRow>,
    pub sparkline: Vec<f64>,
    pub trend: Vec<TrendRow>,
    pub status: CacheStatus,
    pub refresh_error: Option<RefreshError>,
    pub loaded_state: Option<AppState>,
    pub loaded_scope: Option<QueryScope>,
    pub(crate) left_memo: BTreeMap<LeftMemoKey, Vec<LeftRow>>,
    pub(crate) trend_memo: BTreeMap<TrendMemoKey, Vec<TrendRow>>,
    pub(crate) sparkline_memo: Option<Vec<f64>>,
    pub(crate) memo_generation: Option<u64>,
    pub(crate) memo_day: Option<NaiveDate>,
    pub(crate) last_attempt_day: Option<NaiveDate>,
    pub(crate) sparkline_scope: Option<QueryScope>,
}

impl DataCache {
    pub(crate) fn query_for(
        &self,
        state: &AppState,
        now: DateTime<Utc>,
    ) -> anyhow::Result<ViewQuery> {
        let mut scope = self.scope_for(state, now)?;
        Ok(match state.deepest_drill().map(|drill| drill.kind) {
            Some(DrillKind::Events { source }) => {
                scope.session = Some(crate::store::queries::SessionKey {
                    source,
                    id: state.deepest_drill().unwrap().key.clone(),
                });
                ViewQuery::Events(scope)
            }
            Some(DrillKind::Sessions { .. }) => ViewQuery::Sessions(scope),
            None if state.is_provider_view() => ViewQuery::Provider(scope, state.trend_granularity),
            None => match state.current_section {
                Section::Repos => ViewQuery::Repos(scope),
                Section::Days => ViewQuery::Buckets(scope, state.trend_granularity),
                Section::Models => ViewQuery::Models(scope),
                Section::Sessions => ViewQuery::Sessions(scope),
                Section::Provider => unreachable!(),
            },
        })
    }
    pub fn refresh_all(&mut self, store: &Store, state: &AppState) {
        self.refresh_at(store, state, Utc::now());
    }

    pub fn clear_memos(&mut self) {
        self.left_memo.clear();
        self.trend_memo.clear();
        self.sparkline_memo = None;
        self.sparkline_scope = None;
        self.memo_generation = None;
        self.memo_day = None;
    }

    pub(crate) fn scope_for(
        &self,
        state: &AppState,
        now: DateTime<Utc>,
    ) -> anyhow::Result<QueryScope> {
        let mut current = base_filter(state, now)?;
        current.roots = self.source_roots.clone();
        let Some(drill) = state.deepest_drill() else {
            return Ok(current);
        };
        let mut scope = drill.scope.clone();
        scope.roots = current.roots;
        scope.intersect_time(current.since_ms, current.until_ms);
        if !matches!(drill.kind, DrillKind::Events { .. }) {
            scope.source = current.source;
        }
        Ok(scope)
    }

    /// Publish a complete snapshot only after every query for the view succeeds.
    pub(crate) fn refresh_at(&mut self, store: &Store, state: &AppState, now: DateTime<Utc>) {
        match crate::calendar::Calendar::system() {
            Ok(calendar) => self.last_attempt_day = Some(calendar.local_date(now)),
            Err(error) => {
                self.refresh_error = Some(RefreshError::new(RefreshScope::Status, error));
                return;
            }
        }
        let mut next = self.clone();
        match store.read_snapshot(|store| {
            next.load_snapshot(store, state, now)
                .map_err(anyhow::Error::new)
        }) {
            Ok(()) => {
                next.loaded_state = Some(state.clone());
                next.refresh_error = None;
                *self = next;
            }
            Err(error) => {
                self.refresh_error = Some(
                    error
                        .downcast_ref::<RefreshError>()
                        .cloned()
                        .unwrap_or_else(|| RefreshError::new(RefreshScope::Status, error)),
                );
            }
        }
    }

    pub(crate) fn calendar_changed(&self, now: DateTime<Utc>) -> bool {
        crate::calendar::Calendar::system()
            .is_ok_and(|calendar| self.last_attempt_day != Some(calendar.local_date(now)))
    }

    pub(crate) fn restore_loaded_state(&self, state: &mut AppState) {
        if let (Some(error), Some(loaded)) = (&self.refresh_error, &self.loaded_state) {
            *state = loaded.clone();
            state.flash = Some(format!(
                "{} · showing previous scope",
                error.display_message()
            ));
        }
    }

    pub(crate) fn remember_state(&mut self, state: &AppState) {
        if self.loaded_state.is_some() || self.refresh_error.is_none() {
            self.loaded_state = Some(state.clone());
        }
    }

    fn load_snapshot(
        &mut self,
        store: &Store,
        state: &AppState,
        now: DateTime<Utc>,
    ) -> Result<(), RefreshError> {
        let status = load_cache_status(store, now)
            .map_err(|e| RefreshError::new(RefreshScope::Status, e))?;
        let day = crate::calendar::Calendar::system()
            .map_err(|e| RefreshError::new(RefreshScope::Status, e))?
            .local_date(now);
        if self.memo_generation != Some(status.generation)
            || self.memo_day != Some(day)
            || self
                .loaded_scope
                .as_ref()
                .is_some_and(|scope| scope.roots != self.source_roots)
        {
            self.clear_memos();
        }
        self.memo_generation = Some(status.generation);
        self.memo_day = Some(day);
        self.status = status;
        let scope = self
            .scope_for(state, now)
            .map_err(|e| RefreshError::new(RefreshScope::Left, e))?;
        self.loaded_scope = Some(scope.clone());
        match state.deepest_drill().map(|d| d.kind) {
            Some(DrillKind::Events { source }) => {
                let drill = state.deepest_drill().unwrap();
                self.events = load_events_for(store, source, &drill.key, scope.clone())
                    .map_err(|e| RefreshError::new(RefreshScope::Events, e))?;
            }
            Some(DrillKind::Sessions { .. }) => {
                self.sessions = load_sessions_for(store, scope.clone())
                    .map_err(|e| RefreshError::new(RefreshScope::Sessions, e))?;
            }
            None if state.current_section == Section::Sessions => {
                self.sessions = load_sessions_for(store, scope.clone())
                    .map_err(|e| RefreshError::new(RefreshScope::Sessions, e))?;
                self.left.clear();
            }
            None if state.is_provider_view() => {}
            None => {
                let key = LeftMemoKey(
                    state.current_section,
                    state.time_window,
                    state.source_filter,
                    state.trend_granularity,
                );
                self.left = match self.left_memo.get(&key) {
                    Some(rows) => rows.clone(),
                    None => {
                        let rows = load_left_axis(
                            store,
                            state.current_section,
                            scope.clone(),
                            state.trend_granularity,
                        )
                        .map_err(|e| RefreshError::new(RefreshScope::Left, e))?;
                        self.left_memo.insert(key, rows.clone());
                        rows
                    }
                };
            }
        }
        let key = TrendMemoKey(
            state.time_window,
            state.source_filter,
            state.trend_granularity,
        );
        self.trend = match self.trend_memo.get(&key) {
            Some(rows) => rows.clone(),
            None => {
                let mut trend_scope = base_filter(state, now)
                    .map_err(|e| RefreshError::new(RefreshScope::Trend, e))?;
                trend_scope.roots = self.source_roots.clone();
                let rows = load_trend(store, state, now, trend_scope)
                    .map_err(|e| RefreshError::new(RefreshScope::Trend, e))?;
                self.trend_memo.insert(key, rows.clone());
                rows
            }
        };
        if self.sparkline_scope.as_ref() != Some(&scope) || self.sparkline_memo.is_none() {
            let rows = scoped_sparkline(store, scope.clone(), 30)
                .map_err(|e| RefreshError::new(RefreshScope::Sparkline, e))?;
            self.sparkline_memo = Some(rows);
            self.sparkline_scope = Some(scope);
        }
        self.sparkline = self.sparkline_memo.clone().unwrap_or_default();
        Ok(())
    }
}
