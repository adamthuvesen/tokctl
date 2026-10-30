use super::data::{
    sort_event_rows, sort_left_rows, sort_session_rows, CacheStatus, DataCache, EventRow, LeftRow,
    RefreshError, SessionRow, TrendRow,
};
use super::state::{AppState, DrillKind, Section, Sort};
use super::widgets::filter::{apply_filter_left, apply_filter_sessions, filter_rows};

#[derive(Debug, Default, Clone)]
pub struct VisibleRows {
    pub left: Vec<LeftRow>,
    pub sessions: Vec<SessionRow>,
    pub events: Vec<EventRow>,
    pub trend: Vec<TrendRow>,
    pub sparkline: Vec<f64>,
    pub status: CacheStatus,
    pub refresh_error: Option<RefreshError>,
}

impl DataCache {
    /// All consumers use this projection, including rendering and row actions.
    pub fn visible(&self, state: &AppState) -> VisibleRows {
        let mut visible = VisibleRows {
            status: self.status.clone(),
            refresh_error: self.refresh_error.clone(),
            sparkline: self.sparkline.clone(),
            ..VisibleRows::default()
        };
        match state.deepest_drill().map(|drill| drill.kind) {
            Some(DrillKind::Events { .. }) => {
                visible.events = filter_rows(&self.events, state, |row| &row.model).0;
                sort_event_rows(&mut visible.events, state.sort);
            }
            Some(DrillKind::Sessions { .. }) => {
                visible.sessions = display_session_rows(&self.sessions, state)
            }
            None if state.current_section == Section::Sessions => {
                visible.sessions = display_session_rows(&self.sessions, state)
            }
            None if state.is_provider_view() => {
                visible.trend = display_trend_rows(&self.trend, state.sort);
                visible.trend = filter_rows(&visible.trend, state, |row| &row.bucket).0;
            }
            None => {
                let (mut rows, scores) = apply_filter_left(&self.left, state);
                if scores.is_empty() {
                    sort_left_rows(&mut rows, state.current_section, state.sort);
                }
                visible.left = rows;
            }
        }
        visible
    }
}

pub(crate) fn display_session_rows(
    rows: &[super::data::SessionRow],
    state: &AppState,
) -> Vec<super::data::SessionRow> {
    let (mut rows, scores) = apply_filter_sessions(rows, state);
    if scores.is_empty() {
        sort_session_rows(&mut rows, state.sort);
    }
    rows
}

pub(crate) fn display_trend_rows(rows: &[TrendRow], sort: Sort) -> Vec<TrendRow> {
    let mut rows = rows.to_vec();
    rows.sort_by(|a, b| match sort {
        Sort::CostDesc => b
            .total_cost
            .total_cmp(&a.total_cost)
            .then_with(|| b.bucket.cmp(&a.bucket)),
        Sort::CostAsc => a
            .total_cost
            .total_cmp(&b.total_cost)
            .then_with(|| b.bucket.cmp(&a.bucket)),
        Sort::RecentDesc | Sort::AlphaDesc => b.bucket.cmp(&a.bucket),
        Sort::RecentAsc | Sort::AlphaAsc => a.bucket.cmp(&b.bucket),
    });
    rows
}
