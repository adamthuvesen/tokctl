use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
pub const STATE_VERSION: u32 = 3;

/// Top-level navigation entries — one per sidebar row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Section {
    Repos,
    Days,
    Models,
    Sessions,
    /// Time-bucketed costs with per-source (Claude/Codex/Cursor) columns.
    #[serde(alias = "trend")]
    Provider,
}

impl Section {
    /// Sidebar top-to-bottom order.
    pub const ALL: [Section; 5] = [
        Section::Days,
        Section::Models,
        Section::Provider,
        Section::Repos,
        Section::Sessions,
    ];

    pub fn next(self) -> Self {
        let i = Section::ALL.iter().position(|s| *s == self).unwrap_or(0);
        Section::ALL[(i + 1) % Section::ALL.len()]
    }

    pub fn prev(self) -> Self {
        let i = Section::ALL.iter().position(|s| *s == self).unwrap_or(0);
        Section::ALL[(i + Section::ALL.len() - 1) % Section::ALL.len()]
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Section::Repos => "repos",
            Section::Days => "days",
            Section::Models => "models",
            Section::Sessions => "sessions",
            Section::Provider => "provider",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Section::Repos => "REPOS",
            Section::Days => "DAYS",
            Section::Models => "MODELS",
            Section::Sessions => "SESSIONS",
            Section::Provider => "PROVIDER",
        }
    }

    /// Human-friendly label for the sidebar row.
    pub fn label(self) -> &'static str {
        match self {
            Section::Repos => "Repos",
            Section::Days => "Days",
            Section::Models => "Models",
            Section::Sessions => "Sessions",
            Section::Provider => "Provider",
        }
    }

    /// Tab labels for this section. Empty slice = single-lens (no tab row drawn).
    pub fn tabs(self) -> &'static [&'static str] {
        match self {
            Section::Repos => &["Costs", "Provider"],
            _ => &[],
        }
    }

    /// Whether rows in this section represent individually scoped drillable
    /// entities. Provider buckets aggregate across sources/sessions and have
    /// nothing meaningful to drill into; everything else does.
    pub fn rows_drill_to_sessions(self) -> bool {
        !matches!(self, Section::Provider | Section::Sessions)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
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
    RecentAsc,
    AlphaDesc,
    AlphaAsc,
}

impl Sort {
    pub fn next(self) -> Self {
        match self {
            Sort::CostDesc => Sort::CostAsc,
            Sort::CostAsc => Sort::RecentDesc,
            Sort::RecentDesc => Sort::RecentAsc,
            Sort::RecentAsc => Sort::AlphaDesc,
            Sort::AlphaDesc => Sort::AlphaAsc,
            Sort::AlphaAsc => Sort::CostDesc,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Sort::CostDesc => "cost↓",
            Sort::CostAsc => "cost↑",
            Sort::RecentDesc => "recent↓",
            Sort::RecentAsc => "recent↑",
            Sort::AlphaDesc => "alpha↓",
            Sort::AlphaAsc => "alpha↑",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
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

/// Which area currently owns keyboard focus. `Sidebar` = section list,
/// `Main` = the active section's content (or its drill view).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Sidebar,
    Main,
}

/// What the drill is showing. The kind determines which data slice fills the
/// main pane and which row affordances apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrillKind {
    /// Sessions scoped by some upstream section (repo / day / model).
    Sessions { from_section: Section },
    /// Per-turn events for a single `(source, session_id)`.
    Events { source: crate::types::Source },
}

/// One level on the drill stack. `key` is the stable identifier of the row
/// that was drilled into (repo key, day bucket, session id, …); `label` is
/// the human-readable breadcrumb chunk; `cursor` is the row-cursor inside
/// this drill's view, captured so we can restore it when a deeper drill is
/// popped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drill {
    pub kind: DrillKind,
    pub key: String,
    pub label: String,
    pub cursor: usize,
}

#[derive(Debug, Clone, Default)]
pub struct FilterState {
    pub active: bool,
    pub query: String,
}

#[derive(Debug, Clone)]
pub struct AppState {
    pub current_section: Section,
    pub time_window: TimeWindow,
    pub source_filter: SourceFilter,
    pub sort: Sort,
    pub trend_granularity: TrendGranularity,
    pub help_open: bool,
    pub detail_open: bool,
    pub focus: Focus,
    /// Stack of drill levels. Empty = at the section root. Each push deepens
    /// the view (Section → Sessions → Events). Not persisted.
    pub drill_stack: Vec<Drill>,
    /// Active tab index per section (only meaningful when `Section::tabs()` is non-empty).
    pub tab_per_section: BTreeMap<Section, u8>,
    /// Last-selected row key per section (persisted; falls back to first row when stale).
    pub section_selection: BTreeMap<Section, String>,
    /// Live row-cursor index per section (not persisted).
    pub section_index: BTreeMap<Section, usize>,
    /// Sidebar row cursor (live, not persisted — derived from `current_section`).
    pub sidebar_index: usize,
    pub filter: FilterState,
    pub flash: Option<String>,
    /// Whether tables show expanded columns. Default false = compact.
    pub expanded: bool,
    /// One-shot intro flash on first launch under v3. Persisted so it shows once.
    pub seen_v3_intro: bool,
    /// Set once a source-filter no-op flash has fired in the current events
    /// drill, so the hint shows on first press only. Reset on every drill push/pop.
    pub source_filter_hint_fired: bool,
}

impl Default for AppState {
    fn default() -> Self {
        let sidebar_index = 0;
        Self {
            current_section: Section::Days,
            time_window: TimeWindow::Month,
            source_filter: SourceFilter::All,
            sort: Sort::CostDesc,
            trend_granularity: TrendGranularity::Monthly,
            help_open: false,
            detail_open: false,
            focus: Focus::Main,
            drill_stack: Vec::new(),
            tab_per_section: BTreeMap::new(),
            section_selection: BTreeMap::new(),
            section_index: BTreeMap::new(),
            sidebar_index,
            filter: FilterState::default(),
            flash: None,
            expanded: false,
            seen_v3_intro: false,
            source_filter_hint_fired: false,
        }
    }
}

impl AppState {
    pub fn active_tab_index(&self) -> u8 {
        *self
            .tab_per_section
            .get(&self.current_section)
            .unwrap_or(&0)
    }

    pub fn active_tab_label(&self) -> Option<&'static str> {
        let tabs = self.current_section.tabs();
        if tabs.is_empty() {
            None
        } else {
            let i = (self.active_tab_index() as usize).min(tabs.len() - 1);
            Some(tabs[i])
        }
    }

    /// True iff the main pane is currently showing a drilled view.
    pub fn drill_active(&self) -> bool {
        !self.drill_stack.is_empty()
    }

    /// Number of drill levels currently on the stack (0 at section root).
    pub fn drill_depth(&self) -> usize {
        self.drill_stack.len()
    }

    /// Deepest drill on the stack, i.e. the one whose contents the main
    /// pane currently renders. `None` when at the section root.
    pub fn deepest_drill(&self) -> Option<&Drill> {
        self.drill_stack.last()
    }

    /// Mutable access to the deepest drill, used to update its cursor as the
    /// user navigates within its view.
    pub fn deepest_drill_mut(&mut self) -> Option<&mut Drill> {
        self.drill_stack.last_mut()
    }

    pub fn current_index(&self) -> usize {
        if let Some(d) = self.deepest_drill() {
            return d.cursor;
        }
        *self.section_index.get(&self.current_section).unwrap_or(&0)
    }

    pub fn set_current_index(&mut self, idx: usize) {
        if let Some(d) = self.deepest_drill_mut() {
            d.cursor = idx;
        } else {
            self.section_index.insert(self.current_section, idx);
        }
    }
}
