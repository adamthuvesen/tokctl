use serde::{Deserialize, Serialize};
use std::path::Path;

pub const STATE_VERSION: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LeftAxis {
    Repo,
    Day,
    Model,
    Session,
}

impl LeftAxis {
    pub fn next(self) -> Self {
        match self {
            LeftAxis::Repo => LeftAxis::Day,
            LeftAxis::Day => LeftAxis::Model,
            LeftAxis::Model => LeftAxis::Session,
            LeftAxis::Session => LeftAxis::Repo,
        }
    }
    pub fn title(self) -> &'static str {
        match self {
            LeftAxis::Repo => "REPOS",
            LeftAxis::Day => "DAYS",
            LeftAxis::Model => "MODELS",
            LeftAxis::Session => "SESSIONS",
        }
    }
    pub fn chip(self) -> &'static str {
        match self {
            LeftAxis::Repo => "repo",
            LeftAxis::Day => "day",
            LeftAxis::Model => "model",
            LeftAxis::Session => "session",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TimeWindow {
    Today,
    Week,
    Month,
    Year,
    All,
}

impl TimeWindow {
    pub fn as_str(self) -> &'static str {
        match self {
            TimeWindow::Today => "today",
            TimeWindow::Week => "week",
            TimeWindow::Month => "month",
            TimeWindow::Year => "year",
            TimeWindow::All => "all",
        }
    }
    /// Millisecond lower-bound for this window, computed against local `now`.
    pub fn since_ms(self, now: chrono::DateTime<chrono::Utc>) -> Option<i64> {
        use chrono::{Duration, Local, NaiveTime, TimeZone};
        let local = now.with_timezone(&Local);
        let start_of_day = local
            .date_naive()
            .and_time(NaiveTime::from_hms_opt(0, 0, 0).unwrap());
        let start_local = Local
            .from_local_datetime(&start_of_day)
            .single()
            .unwrap_or(local);
        match self {
            TimeWindow::Today => Some(start_local.timestamp_millis()),
            TimeWindow::Week => Some((start_local - Duration::days(6)).timestamp_millis()),
            TimeWindow::Month => Some((start_local - Duration::days(29)).timestamp_millis()),
            TimeWindow::Year => Some((start_local - Duration::days(364)).timestamp_millis()),
            TimeWindow::All => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceFilter {
    All,
    Claude,
    Codex,
    Cursor,
}

impl SourceFilter {
    pub fn as_str(self) -> &'static str {
        match self {
            SourceFilter::All => "all",
            SourceFilter::Claude => "claude",
            SourceFilter::Codex => "codex",
            SourceFilter::Cursor => "cursor",
        }
    }
    pub fn as_source(self) -> Option<crate::types::Source> {
        match self {
            SourceFilter::All => None,
            SourceFilter::Claude => Some(crate::types::Source::Claude),
            SourceFilter::Codex => Some(crate::types::Source::Codex),
            SourceFilter::Cursor => Some(crate::types::Source::Cursor),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sort {
    CostDesc,
    CostAsc,
    RecentDesc,
    AlphaAsc,
}

impl Sort {
    pub fn next(self) -> Self {
        match self {
            Sort::CostDesc => Sort::CostAsc,
            Sort::CostAsc => Sort::RecentDesc,
            Sort::RecentDesc => Sort::AlphaAsc,
            Sort::AlphaAsc => Sort::CostDesc,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Sort::CostDesc => "cost↓",
            Sort::CostAsc => "cost↑",
            Sort::RecentDesc => "recent↓",
            Sort::AlphaAsc => "alpha↑",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrendGranularity {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

impl TrendGranularity {
    pub fn as_str(self) -> &'static str {
        match self {
            TrendGranularity::Daily => "daily",
            TrendGranularity::Weekly => "weekly",
            TrendGranularity::Monthly => "monthly",
            TrendGranularity::Yearly => "yearly",
        }
    }
    pub fn bucket_header(self) -> &'static str {
        match self {
            TrendGranularity::Daily => "date",
            TrendGranularity::Weekly => "week",
            TrendGranularity::Monthly => "month",
            TrendGranularity::Yearly => "year",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneId {
    Left,
    Sessions,
}

impl PaneId {
    pub fn left(self) -> Self {
        match self {
            PaneId::Left => PaneId::Left,
            PaneId::Sessions => PaneId::Left,
        }
    }
    pub fn right(self) -> Self {
        match self {
            PaneId::Left => PaneId::Sessions,
            PaneId::Sessions => PaneId::Sessions,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SelectedKeys {
    pub repo: Option<String>,
    pub session: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct FilterState {
    pub active: bool,
    pub query: String,
}

#[derive(Debug, Clone)]
pub struct AppState {
    pub left_axis: LeftAxis,
    pub time_window: TimeWindow,
    pub source_filter: SourceFilter,
    pub sort: Sort,
    pub trend_granularity: TrendGranularity,
    pub trend_open: bool,
    pub help_open: bool,
    pub detail_open: bool,
    pub focus: PaneId,
    /// Persisted pre-trend focus so Esc/t restores exactly.
    pub focus_before_trend: PaneId,
    pub selected: SelectedKeys,
    /// Selection indices per pane (live, not persisted beyond `selected`).
    pub left_index: usize,
    pub sessions_index: usize,
    pub trend_index: usize,
    pub filter: FilterState,
    pub pane_widths: [u16; 2],
    pub flash: Option<String>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            left_axis: LeftAxis::Repo,
            time_window: TimeWindow::Month,
            source_filter: SourceFilter::All,
            sort: Sort::CostDesc,
            trend_granularity: TrendGranularity::Monthly,
            trend_open: false,
            help_open: false,
            detail_open: false,
            focus: PaneId::Left,
            focus_before_trend: PaneId::Left,
            selected: SelectedKeys::default(),
            left_index: 0,
            sessions_index: 0,
            trend_index: 0,
            filter: FilterState::default(),
            pane_widths: [50, 50],
            flash: None,
        }
    }
}

/// Bitset of which data slices need refetching after an action.
#[derive(Debug, Clone, Copy, Default)]
pub struct RefreshMask {
    pub left: bool,
    pub sessions: bool,
    pub sparkline: bool,
    pub trend: bool,
}

impl RefreshMask {
    pub fn all() -> Self {
        Self {
            left: true,
            sessions: true,
            sparkline: true,
            trend: true,
        }
    }
    pub fn any(self) -> bool {
        self.left || self.sessions || self.sparkline || self.trend
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ApplyOutcome {
    pub quit: bool,
    pub dirty: bool,
    pub needs_refresh: bool,
    pub refresh: RefreshMask,
}

#[derive(Debug, Clone)]
pub enum Action {
    Quit,
    FocusLeft,
    FocusRight,
    MoveUp,
    MoveDown,
    PageUp,
    PageDown,
    Top,
    Bottom,
    Drill,
    Pop,
    CycleAxis,
    CycleSort,
    ToggleHelp,
    ToggleDetail,
    ToggleTrend,
    SetWindow(TimeWindow),
    SetSource(SourceFilter),
    SetTrendGranularity(TrendGranularity),
    Refresh,
    FilterOpen,
    FilterChar(char),
    FilterBackspace,
    FilterCommit,
    FilterCancel,
    Yank,
    YankSummary,
    None,
}

impl AppState {
    pub fn apply(&mut self, action: Action) -> ApplyOutcome {
        let mut out = ApplyOutcome::default();
        self.flash = None;
        match action {
            Action::Quit => out.quit = true,
            Action::None => {}
            Action::FocusLeft => {
                if self.trend_open {
                    // no-op in overlay
                } else {
                    self.focus = self.focus.left();
                }
            }
            Action::FocusRight => {
                if !self.trend_open {
                    self.focus = self.focus.right();
                }
            }
            Action::MoveUp => {
                self.nudge_selection(-1);
            }
            Action::MoveDown => {
                self.nudge_selection(1);
                // mark refresh downstream
            }
            Action::PageUp => self.nudge_selection(-10),
            Action::PageDown => self.nudge_selection(10),
            Action::Top => self.set_selection(0),
            Action::Bottom => self.set_selection(usize::MAX),
            Action::Drill => {
                if !self.trend_open {
                    self.focus = self.focus.right();
                }
            }
            Action::Pop => {
                if self.filter.active {
                    self.filter.active = false;
                    self.filter.query.clear();
                } else if self.detail_open {
                    self.detail_open = false;
                } else if self.help_open {
                    self.help_open = false;
                } else if self.trend_open {
                    self.trend_open = false;
                    self.focus = self.focus_before_trend;
                    out.dirty = true;
                } else {
                    self.focus = self.focus.left();
                }
            }
            Action::CycleAxis => {
                self.left_axis = self.left_axis.next();
                self.left_index = 0;
                self.selected.repo = None;
                self.selected.session = None;
                out.refresh.left = true;
                out.refresh.sessions = true;
                out.needs_refresh = true;
                out.dirty = true;
            }
            Action::CycleSort => {
                self.sort = self.sort.next();
                out.refresh.left = true;
                out.refresh.sessions = true;
                out.needs_refresh = true;
                out.dirty = true;
            }
            Action::ToggleHelp => {
                self.help_open = !self.help_open;
            }
            Action::ToggleDetail => {
                if !self.trend_open && !self.help_open {
                    self.detail_open = !self.detail_open;
                }
            }
            Action::ToggleTrend => {
                if self.trend_open {
                    self.trend_open = false;
                    self.focus = self.focus_before_trend;
                } else {
                    self.focus_before_trend = self.focus;
                    self.trend_open = true;
                    out.refresh.trend = true;
                    out.needs_refresh = true;
                }
                out.dirty = true;
            }
            Action::SetWindow(w) => {
                if self.time_window != w {
                    self.time_window = w;
                    out.refresh = RefreshMask::all();
                    out.needs_refresh = true;
                    out.dirty = true;
                }
            }
            Action::SetSource(s) => {
                if self.source_filter != s {
                    self.source_filter = s;
                    out.refresh = RefreshMask::all();
                    out.needs_refresh = true;
                    out.dirty = true;
                }
            }
            Action::SetTrendGranularity(g) => {
                if self.trend_granularity != g {
                    self.trend_granularity = g;
                    out.refresh.trend = true;
                    out.needs_refresh = true;
                    out.dirty = true;
                }
            }
            Action::Refresh => {
                out.refresh = RefreshMask::all();
                out.needs_refresh = true;
            }
            Action::FilterOpen => {
                self.filter.active = true;
                self.filter.query.clear();
            }
            Action::FilterChar(c) => {
                if self.filter.active {
                    self.filter.query.push(c);
                }
            }
            Action::FilterBackspace => {
                if self.filter.active {
                    self.filter.query.pop();
                }
            }
            Action::FilterCommit => {
                // Leave filter active; reduce-time filter applies visually.
                self.filter.active = false;
            }
            Action::FilterCancel => {
                self.filter.active = false;
                self.filter.query.clear();
            }
            Action::Yank => {
                // event loop performs clipboard IO and sets flash.
            }
            Action::YankSummary => {
                // event loop performs clipboard IO and sets flash.
            }
        }

        // Downstream refresh from selection movement.
        match action {
            Action::MoveUp
            | Action::MoveDown
            | Action::PageUp
            | Action::PageDown
            | Action::Top
            | Action::Bottom => {
                match self.focus {
                    PaneId::Left => {
                        out.refresh.sessions = true;
                        out.needs_refresh = true;
                    }
                    PaneId::Sessions => {}
                }
                out.dirty = true;
            }
            _ => {}
        }

        out
    }

    fn nudge_selection(&mut self, delta: isize) {
        let idx = self.selection_ref_mut();
        let new = (*idx as isize).saturating_add(delta).max(0) as usize;
        *idx = new;
    }

    fn set_selection(&mut self, idx: usize) {
        *self.selection_ref_mut() = idx;
    }

    fn selection_ref_mut(&mut self) -> &mut usize {
        if self.trend_open {
            return &mut self.trend_index;
        }
        match self.focus {
            PaneId::Left => &mut self.left_index,
            PaneId::Sessions => &mut self.sessions_index,
        }
    }
}

/// On-disk schema. Kept intentionally small and tolerant.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedState {
    #[serde(default = "default_version")]
    version: u32,
    #[serde(default)]
    left_axis: Option<LeftAxis>,
    #[serde(default)]
    time_window: Option<TimeWindow>,
    #[serde(default)]
    source_filter: Option<SourceFilter>,
    #[serde(default)]
    sort: Option<Sort>,
    #[serde(default)]
    trend_granularity: Option<TrendGranularity>,
    #[serde(default)]
    selected: Option<SelectedKeys>,
}

fn default_version() -> u32 {
    STATE_VERSION
}

pub fn load(path: &Path) -> AppState {
    let Ok(bytes) = std::fs::read(path) else {
        return AppState::default();
    };
    let Ok(p) = serde_json::from_slice::<PersistedState>(&bytes) else {
        return AppState::default();
    };
    if p.version != STATE_VERSION {
        return AppState::default();
    }
    let mut s = AppState::default();
    if let Some(v) = p.left_axis {
        s.left_axis = v;
    }
    if let Some(v) = p.time_window {
        s.time_window = v;
    }
    if let Some(v) = p.source_filter {
        s.source_filter = v;
    }
    if let Some(v) = p.sort {
        s.sort = v;
    }
    if let Some(v) = p.trend_granularity {
        s.trend_granularity = v;
    }
    if let Some(v) = p.selected {
        s.selected = v;
    }
    s
}

pub fn save(path: &Path, s: &AppState) -> anyhow::Result<()> {
    let p = PersistedState {
        version: STATE_VERSION,
        left_axis: Some(s.left_axis),
        time_window: Some(s.time_window),
        source_filter: Some(s.source_filter),
        sort: Some(s.sort),
        trend_granularity: Some(s.trend_granularity),
        selected: Some(s.selected.clone()),
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(&p)?;
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn roundtrip_persists_fields() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("ui_state.json");
        let s = AppState {
            left_axis: LeftAxis::Model,
            time_window: TimeWindow::Today,
            source_filter: SourceFilter::Claude,
            trend_granularity: TrendGranularity::Weekly,
            ..AppState::default()
        };
        save(&p, &s).unwrap();
        let loaded = load(&p);
        assert_eq!(loaded.left_axis, LeftAxis::Model);
        assert_eq!(loaded.time_window, TimeWindow::Today);
        assert_eq!(loaded.source_filter, SourceFilter::Claude);
        assert_eq!(loaded.trend_granularity, TrendGranularity::Weekly);
    }

    #[test]
    fn unknown_version_falls_back_to_default() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("ui_state.json");
        std::fs::write(&p, br#"{"version":999,"left_axis":"model"}"#).unwrap();
        let loaded = load(&p);
        assert_eq!(loaded.left_axis, AppState::default().left_axis);
    }

    #[test]
    fn missing_file_is_default() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("nope.json");
        let loaded = load(&p);
        assert_eq!(loaded.time_window, TimeWindow::Month);
    }

    #[test]
    fn tab_cycles_axis_and_marks_dirty() {
        let mut s = AppState::default();
        let out = s.apply(Action::CycleAxis);
        assert_eq!(s.left_axis, LeftAxis::Day);
        assert!(out.dirty);
        assert!(out.needs_refresh);
    }

    #[test]
    fn detail_toggle_and_pop_preserve_selection() {
        let mut s = AppState {
            left_index: 3,
            ..AppState::default()
        };
        s.apply(Action::ToggleDetail);
        assert!(s.detail_open);
        s.apply(Action::Pop);
        assert!(!s.detail_open);
        assert_eq!(s.left_index, 3);
    }
}
