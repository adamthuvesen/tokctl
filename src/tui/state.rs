use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

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

/// Bitset of which data slices need refetching after an action.
#[derive(Debug, Clone, Copy, Default)]
pub struct RefreshMask {
    pub left: bool,
    pub sessions: bool,
    pub events: bool,
    pub sparkline: bool,
    pub trend: bool,
}

impl RefreshMask {
    pub fn all() -> Self {
        Self {
            left: true,
            sessions: true,
            events: true,
            sparkline: true,
            trend: true,
        }
    }
    pub fn any(self) -> bool {
        self.left || self.sessions || self.events || self.sparkline || self.trend
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
    /// Move sidebar selection / row cursor down within the focused area.
    MoveDown,
    MoveUp,
    PageDown,
    PageUp,
    Top,
    Bottom,
    /// Jump to next section (sidebar list), regardless of focus. (`]`)
    NextSection,
    /// Jump to previous section. (`[`)
    PrevSection,
    /// Jump directly to a specific section (e.g. `t` → `Provider`).
    JumpToSection(Section),
    /// Cycle to next tab within the active section. No-op if section has < 2 tabs.
    CycleTab,
    /// Push drill from the focused row, if the section supports it.
    Drill,
    /// Pop drill (or, if not drilled, focus sidebar).
    PopDrill,
    /// Move focus to the sidebar.
    FocusSidebar,
    /// Move focus to the main pane.
    FocusMain,
    /// Cascading pop: drill → detail → help → filter → focus sidebar.
    Pop,
    CycleSort,
    ToggleHelp,
    ToggleDetail,
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
    ToggleExpand,
    DismissFlash,
    None,
}

/// Centralized refresh-mask derivation: given an action, the section before
/// it ran, and the resulting state, return the minimal set of cache slices
/// that must be re-queried. One matrix, exhaustively declared, no scattered
/// `out.refresh.X = true` lines through `apply()`.
///
/// Rules of thumb:
/// - **Section change** (any path): `left` only. `trend` and `sparkline`
///   don't depend on section, so they stay valid.
/// - **Window / source / granularity change**: `left + trend + sparkline`
///   — these three depend on the time window. Sessions/events stay
///   scoped to the active drill and don't need reload here.
/// - **Sort cycle**: `left`, plus the matching drill slice if currently
///   inside one (so the drilled-into list re-sorts).
/// - **Toggle expand**: empty. Pure layout, all data already in `left`.
/// - **Drill push**: the slice the drill needs (`sessions` or `events`).
/// - **Manual refresh (`r`)**: everything.
/// - **Cursor moves at section root**: empty. Post-redesign nothing
///   downstream depends on the focused row when not drilled.
pub fn refresh_mask_for(action: &Action, section_before: Section, after: &AppState) -> RefreshMask {
    let drilled_kind = after.deepest_drill().map(|d| d.kind);

    // Section change subsumes any other mask the action might want — at
    // root, no slice except `left` could meaningfully change yet.
    if section_before != after.current_section {
        return RefreshMask {
            left: true,
            ..RefreshMask::default()
        };
    }

    match action {
        Action::Refresh => RefreshMask::all(),

        Action::SetWindow(_) | Action::SetSource(_) => RefreshMask {
            left: true,
            trend: true,
            sparkline: true,
            ..RefreshMask::default()
        },

        Action::SetTrendGranularity(_) => RefreshMask {
            // Days section also buckets by granularity, so `left` reloads.
            left: true,
            trend: true,
            ..RefreshMask::default()
        },

        Action::CycleSort => RefreshMask {
            left: true,
            sessions: matches!(drilled_kind, Some(DrillKind::Sessions { .. })),
            events: matches!(drilled_kind, Some(DrillKind::Events { .. })),
            ..RefreshMask::default()
        },

        Action::CycleTab => RefreshMask {
            // Tabs on Repos swap between Costs (left) and Provider (trend).
            left: true,
            trend: true,
            ..RefreshMask::default()
        },

        Action::Drill => match after.next_drill_kind_hint() {
            // Note: `Drill` runs BEFORE `push_drill` in the event loop, so
            // `deepest_drill()` here still reflects the parent. We use
            // `next_drill_kind_hint` to know which slice the upcoming
            // child view will need.
            Some(DrillKind::Sessions { .. }) => RefreshMask {
                sessions: true,
                ..RefreshMask::default()
            },
            Some(DrillKind::Events { .. }) => RefreshMask {
                events: true,
                ..RefreshMask::default()
            },
            None => RefreshMask::default(),
        },

        Action::ToggleExpand
        | Action::PopDrill
        | Action::Pop
        | Action::FocusSidebar
        | Action::FocusMain
        | Action::ToggleHelp
        | Action::ToggleDetail
        | Action::FilterOpen
        | Action::FilterChar(_)
        | Action::FilterBackspace
        | Action::FilterCommit
        | Action::FilterCancel
        | Action::Yank
        | Action::YankSummary
        | Action::DismissFlash
        | Action::None
        | Action::Quit
        | Action::JumpToSection(_)
        | Action::NextSection
        | Action::PrevSection
        | Action::MoveUp
        | Action::MoveDown
        | Action::PageUp
        | Action::PageDown
        | Action::Top
        | Action::Bottom => RefreshMask::default(),
    }
}

impl AppState {
    pub fn apply(&mut self, action: Action) -> ApplyOutcome {
        let mut out = ApplyOutcome::default();
        let section_before = self.current_section;
        // Most actions clear a stale flash. Filter-mode chars and movement
        // keep it; we just clear unconditionally for simplicity — a flash
        // is a one-shot anyway.
        if !matches!(
            action,
            Action::None | Action::FilterChar(_) | Action::FilterBackspace
        ) {
            self.flash = None;
        }

        match action {
            Action::Quit => out.quit = true,
            Action::None => {}
            Action::DismissFlash => {}
            Action::FocusSidebar => {
                self.focus = Focus::Sidebar;
                out.dirty = true;
            }
            Action::FocusMain => {
                self.focus = Focus::Main;
                out.dirty = true;
            }
            Action::MoveUp => {
                self.nudge_selection(-1);
                out.dirty = true;
            }
            Action::MoveDown => {
                self.nudge_selection(1);
                out.dirty = true;
            }
            Action::PageUp => {
                self.nudge_selection(-10);
                out.dirty = true;
            }
            Action::PageDown => {
                self.nudge_selection(10);
                out.dirty = true;
            }
            Action::Top => {
                self.set_selection(0);
                out.dirty = true;
            }
            Action::Bottom => {
                self.set_selection(usize::MAX);
                out.dirty = true;
            }
            Action::NextSection => {
                self.switch_section(self.current_section.next());
                out.dirty = true;
            }
            Action::PrevSection => {
                self.switch_section(self.current_section.prev());
                out.dirty = true;
            }
            Action::JumpToSection(s) => {
                if self.current_section != s {
                    self.switch_section(s);
                    out.dirty = true;
                } else if self.drill_active() {
                    // already on the section but drilled — pop back to root
                    self.drill_stack.clear();
                    self.source_filter_hint_fired = false;
                    out.dirty = true;
                }
            }
            Action::CycleTab => {
                let tabs = self.current_section.tabs();
                if tabs.len() >= 2 {
                    let cur = self.active_tab_index();
                    let next = ((cur as usize + 1) % tabs.len()) as u8;
                    self.tab_per_section.insert(self.current_section, next);
                    out.dirty = true;
                }
            }
            Action::Drill => {
                // The actual Drill is constructed in mod.rs after it reads
                // the focused row, then pushed via `push_drill`. The mask
                // derivation below picks up `next_drill_kind_hint`.
                if self.next_drill_kind_hint().is_some() {
                    out.dirty = true;
                }
            }
            Action::PopDrill => {
                if self.drill_active() {
                    self.drill_stack.pop();
                    self.source_filter_hint_fired = false;
                    self.focus = Focus::Main;
                    out.dirty = true;
                } else {
                    self.focus = Focus::Sidebar;
                    out.dirty = true;
                }
            }
            Action::Pop => {
                if self.filter.active {
                    self.filter.active = false;
                    self.filter.query.clear();
                    out.dirty = true;
                } else if self.detail_open {
                    self.detail_open = false;
                    out.dirty = true;
                } else if self.help_open {
                    self.help_open = false;
                    out.dirty = true;
                } else if self.drill_active() {
                    self.drill_stack.pop();
                    self.source_filter_hint_fired = false;
                    self.focus = Focus::Main;
                    out.dirty = true;
                } else if self.focus == Focus::Main {
                    self.focus = Focus::Sidebar;
                    out.dirty = true;
                }
            }
            Action::CycleSort => {
                self.sort = self.sort.next();
                out.dirty = true;
            }
            Action::ToggleHelp => {
                self.help_open = !self.help_open;
                out.dirty = true;
            }
            Action::ToggleDetail => {
                if !self.help_open {
                    self.detail_open = !self.detail_open;
                    out.dirty = true;
                }
            }
            Action::SetWindow(w) => {
                if self.time_window != w {
                    self.time_window = w;
                    out.dirty = true;
                }
            }
            Action::SetSource(s) => {
                if self.source_filter != s {
                    self.source_filter = s;
                    // Inside an Events drill the source is fixed by the
                    // drilled `(source, session_id)` tuple — flip the header
                    // value but skip the refetch and surface a one-shot hint.
                    if matches!(
                        self.deepest_drill().map(|d| d.kind),
                        Some(DrillKind::Events { .. })
                    ) && !self.source_filter_hint_fired
                    {
                        self.flash = Some("source filter ignored inside session".to_owned());
                        self.source_filter_hint_fired = true;
                    }
                    out.dirty = true;
                }
            }
            Action::SetTrendGranularity(g) => {
                if self.trend_granularity != g {
                    self.trend_granularity = g;
                    out.dirty = true;
                }
            }
            Action::Refresh => {
                out.dirty = true;
            }
            Action::FilterOpen => {
                self.filter.active = true;
                self.filter.query.clear();
                out.dirty = true;
            }
            Action::FilterChar(c) => {
                if self.filter.active {
                    self.filter.query.push(c);
                    out.dirty = true;
                }
            }
            Action::FilterBackspace => {
                if self.filter.active {
                    self.filter.query.pop();
                    out.dirty = true;
                }
            }
            Action::FilterCommit => {
                self.filter.active = false;
                out.dirty = true;
            }
            Action::FilterCancel => {
                self.filter.active = false;
                self.filter.query.clear();
                out.dirty = true;
            }
            Action::Yank => {}
            Action::YankSummary => {}
            Action::ToggleExpand => {
                self.expanded = !self.expanded;
                out.dirty = true;
            }
        }

        // Centralized refresh-mask derivation. One match, exhaustively
        // declared, no scattered `out.refresh.X = true` lines.
        out.refresh = refresh_mask_for(&action, section_before, self);
        out.needs_refresh = out.refresh.any();

        out
    }

    fn switch_section(&mut self, target: Section) {
        if self.current_section == target {
            return;
        }
        self.current_section = target;
        self.sidebar_index = Section::ALL.iter().position(|s| *s == target).unwrap_or(0);
        self.drill_stack.clear();
        self.source_filter_hint_fired = false;
        self.focus = Focus::Main;
    }

    fn nudge_selection(&mut self, delta: isize) {
        if self.focus == Focus::Sidebar {
            let new = (self.sidebar_index as isize + delta).rem_euclid(Section::ALL.len() as isize)
                as usize;
            self.sidebar_index = new;
            self.current_section = Section::ALL[new];
            self.drill_stack.clear();
            self.source_filter_hint_fired = false;
        } else {
            let cur = self.current_index() as isize;
            let new = cur.saturating_add(delta).max(0) as usize;
            self.set_current_index(new);
        }
    }

    fn set_selection(&mut self, idx: usize) {
        if self.focus == Focus::Sidebar {
            let new = idx.min(Section::ALL.len() - 1);
            self.sidebar_index = new;
            self.current_section = Section::ALL[new];
            self.drill_stack.clear();
            self.source_filter_hint_fired = false;
        } else {
            self.set_current_index(idx);
        }
    }

    /// Push a drill onto the stack. The new view starts with cursor at 0.
    /// Caller is responsible for ensuring the push is legal (see
    /// [`AppState::next_drill_kind_hint`] / [`AppState::can_push_drill`]).
    pub fn push_drill(&mut self, mut drill: Drill) {
        drill.cursor = 0;
        self.drill_stack.push(drill);
        self.source_filter_hint_fired = false;
        self.focus = Focus::Main;
    }

    /// What kind of drill the next `Action::Drill` *would* push, given the
    /// current section and stack top. Returns `None` for "no drill possible
    /// from here", which lets the dispatcher both gate the push and pre-flag
    /// which cache slice needs refetching.
    pub fn next_drill_kind_hint(&self) -> Option<DrillKind> {
        match self.deepest_drill() {
            None => {
                if self.current_section == Section::Sessions {
                    // Source comes from the focused row; mod.rs fills it in.
                    Some(DrillKind::Events {
                        source: crate::types::Source::Claude,
                    })
                } else if self.current_section.rows_drill_to_sessions() {
                    Some(DrillKind::Sessions {
                        from_section: self.current_section,
                    })
                } else {
                    None
                }
            }
            Some(d) => match d.kind {
                // Sessions drill: rows are sessions → push events.
                DrillKind::Sessions { .. } => Some(DrillKind::Events {
                    source: crate::types::Source::Claude,
                }),
                // Events drill: terminal level.
                DrillKind::Events { .. } => None,
            },
        }
    }

    /// Whether `Action::Drill` would do anything in the current state.
    pub fn can_push_drill(&self) -> bool {
        self.next_drill_kind_hint().is_some()
    }
}

/// On-disk schema (v3). Tolerant to unknown / missing fields; any file with
/// a `version` other than `STATE_VERSION` is silently dropped on load.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedState {
    #[serde(default = "default_version")]
    version: u32,
    #[serde(default)]
    current_section: Option<Section>,
    #[serde(default)]
    time_window: Option<TimeWindow>,
    #[serde(default)]
    source_filter: Option<SourceFilter>,
    #[serde(default)]
    sort: Option<Sort>,
    #[serde(default)]
    trend_granularity: Option<TrendGranularity>,
    #[serde(default)]
    tab_per_section: Option<BTreeMap<Section, u8>>,
    #[serde(default)]
    section_selection: Option<BTreeMap<Section, String>>,
    #[serde(default)]
    expanded: Option<bool>,
    #[serde(default)]
    seen_v3_intro: Option<bool>,
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
    // Always start on Days regardless of persisted section.
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
    if let Some(v) = p.tab_per_section {
        s.tab_per_section = v;
    }
    if let Some(v) = p.section_selection {
        s.section_selection = v;
    }
    if let Some(v) = p.expanded {
        s.expanded = v;
    }
    if let Some(v) = p.seen_v3_intro {
        s.seen_v3_intro = v;
    }
    s
}

pub fn save(path: &Path, s: &AppState) -> anyhow::Result<()> {
    let p = PersistedState {
        version: STATE_VERSION,
        current_section: Some(s.current_section),
        time_window: Some(s.time_window),
        source_filter: Some(s.source_filter),
        sort: Some(s.sort),
        trend_granularity: Some(s.trend_granularity),
        tab_per_section: Some(s.tab_per_section.clone()),
        section_selection: Some(s.section_selection.clone()),
        expanded: Some(s.expanded),
        seen_v3_intro: Some(s.seen_v3_intro),
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
    fn section_next_wraps() {
        assert_eq!(Section::Sessions.next(), Section::Days);
        assert_eq!(Section::Provider.next(), Section::Repos);
    }

    #[test]
    fn section_prev_wraps() {
        assert_eq!(Section::Days.prev(), Section::Sessions);
        assert_eq!(Section::Repos.prev(), Section::Provider);
    }

    #[test]
    fn repos_section_has_two_tabs() {
        assert_eq!(Section::Repos.tabs(), &["Costs", "Provider"]);
        assert!(Section::Days.tabs().is_empty());
    }

    #[test]
    fn rows_drill_to_sessions_excludes_provider_and_sessions() {
        assert!(Section::Repos.rows_drill_to_sessions());
        assert!(Section::Days.rows_drill_to_sessions());
        assert!(Section::Models.rows_drill_to_sessions());
        // Sessions rows drill *to events*, not to a sessions list.
        assert!(!Section::Sessions.rows_drill_to_sessions());
        assert!(!Section::Provider.rows_drill_to_sessions());
    }

    #[test]
    fn sort_cycle_pairs_desc_then_asc() {
        let mut sort = Sort::CostDesc;
        let mut seen = Vec::new();
        for _ in 0..6 {
            seen.push(sort.as_str());
            sort = sort.next();
        }

        assert_eq!(
            seen,
            vec!["cost↓", "cost↑", "recent↓", "recent↑", "alpha↓", "alpha↑"]
        );
        assert_eq!(sort, Sort::CostDesc);
    }

    #[test]
    fn next_section_action_changes_section_and_marks_dirty() {
        let mut s = AppState::default();
        let out = s.apply(Action::NextSection);
        assert_eq!(s.current_section, Section::Models);
        assert!(out.dirty);
        assert!(out.needs_refresh);
    }

    #[test]
    fn cycle_tab_advances_repos_tab() {
        let mut s = AppState {
            current_section: Section::Repos,
            sidebar_index: Section::ALL
                .iter()
                .position(|s| *s == Section::Repos)
                .unwrap_or(0),
            ..AppState::default()
        };
        assert_eq!(s.active_tab_index(), 0);
        let out = s.apply(Action::CycleTab);
        assert_eq!(s.active_tab_index(), 1);
        assert!(out.dirty);
        // wraps
        s.apply(Action::CycleTab);
        assert_eq!(s.active_tab_index(), 0);
    }

    #[test]
    fn cycle_tab_noop_for_single_lens_section() {
        let mut s = AppState {
            current_section: Section::Days,
            ..AppState::default()
        };
        let out = s.apply(Action::CycleTab);
        assert!(!out.dirty);
        assert_eq!(s.active_tab_index(), 0);
    }

    #[test]
    fn drill_set_and_pop() {
        let mut s = AppState {
            current_section: Section::Repos,
            ..AppState::default()
        };
        s.push_drill(Drill {
            kind: DrillKind::Sessions {
                from_section: Section::Repos,
            },
            key: "tokctl".into(),
            label: "tokctl".into(),
            cursor: 0,
        });
        assert_eq!(s.drill_depth(), 1);
        let out = s.apply(Action::PopDrill);
        assert_eq!(s.drill_depth(), 0);
        assert!(out.dirty);
    }

    #[test]
    fn jump_to_provider_switches_section() {
        let mut s = AppState::default();
        let out = s.apply(Action::JumpToSection(Section::Provider));
        assert_eq!(s.current_section, Section::Provider);
        assert!(out.dirty);
    }

    #[test]
    fn pop_cascades_drill_then_focus() {
        let mut s = AppState {
            current_section: Section::Repos,
            ..AppState::default()
        };
        s.push_drill(Drill {
            kind: DrillKind::Sessions {
                from_section: Section::Repos,
            },
            key: "x".into(),
            label: "x".into(),
            cursor: 0,
        });
        s.apply(Action::Pop);
        assert_eq!(s.drill_depth(), 0);
        assert_eq!(s.focus, Focus::Main);
        s.apply(Action::Pop);
        assert_eq!(s.focus, Focus::Sidebar);
    }

    #[test]
    fn move_in_sidebar_changes_section() {
        let mut s = AppState {
            focus: Focus::Sidebar,
            ..AppState::default()
        };
        s.apply(Action::MoveDown);
        assert_eq!(s.current_section, Section::Models);
        s.apply(Action::MoveUp);
        assert_eq!(s.current_section, Section::Days);
    }

    #[test]
    fn sidebar_move_only_refreshes_left() {
        let mut s = AppState {
            focus: Focus::Sidebar,
            ..AppState::default()
        };
        let out = s.apply(Action::MoveDown);
        assert_eq!(s.current_section, Section::Models);
        assert!(out.needs_refresh, "section change triggers a refresh");
        assert!(out.refresh.left, "left rebinds to the new section's data");
        // Trend / sparkline / sessions / events don't depend on section.
        assert!(!out.refresh.sessions);
        assert!(!out.refresh.trend);
        assert!(!out.refresh.sparkline);
        assert!(!out.refresh.events);
    }

    #[test]
    fn sidebar_bottom_jumps_and_refreshes_left_only() {
        let mut s = AppState {
            focus: Focus::Sidebar,
            ..AppState::default()
        };
        let out = s.apply(Action::Bottom);
        assert_eq!(s.current_section, Section::Sessions);
        assert!(out.needs_refresh);
        assert!(out.refresh.left);
        assert!(!out.refresh.trend);
        assert!(!out.refresh.sparkline);
    }

    #[test]
    fn roundtrip_persists_fields() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("ui_state.json");
        let mut s = AppState {
            current_section: Section::Provider,
            time_window: TimeWindow::Today,
            source_filter: SourceFilter::Claude,
            trend_granularity: TrendGranularity::Weekly,
            seen_v3_intro: true,
            ..AppState::default()
        };
        s.tab_per_section.insert(Section::Repos, 1);
        s.section_selection
            .insert(Section::Repos, "tokctl".to_owned());
        save(&p, &s).unwrap();
        let loaded = load(&p);
        // current_section is intentionally not restored — TUI always boots on Days.
        assert_eq!(loaded.current_section, Section::Days);
        assert_eq!(loaded.time_window, TimeWindow::Today);
        assert_eq!(loaded.source_filter, SourceFilter::Claude);
        assert_eq!(loaded.trend_granularity, TrendGranularity::Weekly);
        assert_eq!(loaded.tab_per_section.get(&Section::Repos), Some(&1u8));
        assert_eq!(
            loaded
                .section_selection
                .get(&Section::Repos)
                .map(String::as_str),
            Some("tokctl")
        );
        assert!(loaded.seen_v3_intro);
    }

    #[test]
    fn load_accepts_legacy_trend_section_name() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("ui_state.json");
        std::fs::write(
            &p,
            format!(
                r#"{{"version":{},"current_section":"trend","trend_granularity":"daily"}}"#,
                STATE_VERSION
            ),
        )
        .unwrap();
        let loaded = load(&p);
        // The legacy "trend" alias must not break deserialization of sibling fields,
        // even though current_section itself is no longer restored on load.
        assert_eq!(loaded.current_section, Section::Days);
        assert_eq!(loaded.trend_granularity, TrendGranularity::Daily);
    }

    #[test]
    fn v2_file_resets_to_defaults() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("ui_state.json");
        // Simulate an old v2 file shape.
        std::fs::write(
            &p,
            br#"{"version":2,"left_axis":"model","time_window":"today"}"#,
        )
        .unwrap();
        let loaded = load(&p);
        let default = AppState::default();
        assert_eq!(loaded.current_section, default.current_section);
        assert_eq!(loaded.time_window, default.time_window);
    }

    #[test]
    fn unknown_version_is_ignored() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("ui_state.json");
        std::fs::write(&p, br#"{"version":999,"current_section":"trend"}"#).unwrap();
        let loaded = load(&p);
        assert_eq!(loaded.current_section, AppState::default().current_section);
    }

    #[test]
    fn missing_file_is_default() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("nope.json");
        let loaded = load(&p);
        assert_eq!(loaded.time_window, TimeWindow::Month);
    }

    #[test]
    fn detail_toggle_and_pop_preserve_section() {
        let mut s = AppState::default();
        s.section_index.insert(Section::Repos, 3);
        s.apply(Action::ToggleDetail);
        assert!(s.detail_open);
        s.apply(Action::Pop);
        assert!(!s.detail_open);
        assert_eq!(s.section_index.get(&Section::Repos), Some(&3));
    }

    #[test]
    fn drill_is_not_persisted() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("ui_state.json");
        let mut s = AppState {
            current_section: Section::Repos,
            ..AppState::default()
        };
        s.push_drill(Drill {
            kind: DrillKind::Sessions {
                from_section: Section::Repos,
            },
            key: "tokctl".into(),
            label: "tokctl".into(),
            cursor: 0,
        });
        save(&p, &s).unwrap();
        let loaded = load(&p);
        assert_eq!(loaded.drill_depth(), 0);
    }

    // ---- new tests for the drill stack ---------------------------------

    #[test]
    fn next_drill_kind_root_repos_is_sessions() {
        let s = AppState {
            current_section: Section::Repos,
            ..AppState::default()
        };
        assert!(matches!(
            s.next_drill_kind_hint(),
            Some(DrillKind::Sessions {
                from_section: Section::Repos
            })
        ));
    }

    #[test]
    fn next_drill_kind_root_sessions_is_events() {
        let s = AppState {
            current_section: Section::Sessions,
            ..AppState::default()
        };
        assert!(matches!(
            s.next_drill_kind_hint(),
            Some(DrillKind::Events { .. })
        ));
    }

    #[test]
    fn next_drill_kind_root_provider_is_none() {
        let s = AppState {
            current_section: Section::Provider,
            ..AppState::default()
        };
        assert!(s.next_drill_kind_hint().is_none());
    }

    #[test]
    fn next_drill_kind_inside_sessions_drill_is_events() {
        let mut s = AppState {
            current_section: Section::Repos,
            ..AppState::default()
        };
        s.push_drill(Drill {
            kind: DrillKind::Sessions {
                from_section: Section::Repos,
            },
            key: "tokctl".into(),
            label: "tokctl".into(),
            cursor: 0,
        });
        assert!(matches!(
            s.next_drill_kind_hint(),
            Some(DrillKind::Events { .. })
        ));
    }

    #[test]
    fn next_drill_kind_inside_events_is_terminal() {
        let mut s = AppState {
            current_section: Section::Sessions,
            ..AppState::default()
        };
        s.push_drill(Drill {
            kind: DrillKind::Events {
                source: crate::types::Source::Claude,
            },
            key: "abc".into(),
            label: "abc".into(),
            cursor: 0,
        });
        assert!(s.next_drill_kind_hint().is_none());
    }

    #[test]
    fn esc_pops_one_level_at_a_time() {
        let mut s = AppState {
            current_section: Section::Repos,
            ..AppState::default()
        };
        s.push_drill(Drill {
            kind: DrillKind::Sessions {
                from_section: Section::Repos,
            },
            key: "tokctl".into(),
            label: "tokctl".into(),
            cursor: 0,
        });
        s.push_drill(Drill {
            kind: DrillKind::Events {
                source: crate::types::Source::Claude,
            },
            key: "abc".into(),
            label: "abc".into(),
            cursor: 0,
        });
        assert_eq!(s.drill_depth(), 2);
        s.apply(Action::PopDrill);
        assert_eq!(s.drill_depth(), 1);
        s.apply(Action::PopDrill);
        assert_eq!(s.drill_depth(), 0);
    }

    #[test]
    fn drill_action_signals_correct_refresh() {
        let mut s = AppState {
            current_section: Section::Repos,
            ..AppState::default()
        };
        let out = s.apply(Action::Drill);
        assert!(out.refresh.sessions);
        assert!(!out.refresh.events);

        let mut s = AppState {
            current_section: Section::Sessions,
            ..AppState::default()
        };
        let out = s.apply(Action::Drill);
        assert!(out.refresh.events);
        assert!(!out.refresh.sessions);

        let mut s = AppState {
            current_section: Section::Provider,
            ..AppState::default()
        };
        let out = s.apply(Action::Drill);
        assert!(!out.refresh.events);
        assert!(!out.refresh.sessions);
    }

    #[test]
    fn cursor_lives_inside_each_drill() {
        let mut s = AppState {
            current_section: Section::Repos,
            ..AppState::default()
        };
        s.push_drill(Drill {
            kind: DrillKind::Sessions {
                from_section: Section::Repos,
            },
            key: "tokctl".into(),
            label: "tokctl".into(),
            cursor: 0,
        });
        s.set_current_index(3);
        assert_eq!(s.current_index(), 3);
        s.push_drill(Drill {
            kind: DrillKind::Events {
                source: crate::types::Source::Claude,
            },
            key: "abc".into(),
            label: "abc".into(),
            cursor: 0,
        });
        // The new deepest drill starts at 0, even though the parent had cursor 3.
        assert_eq!(s.current_index(), 0);
        s.set_current_index(7);
        assert_eq!(s.current_index(), 7);
        // Pop, parent cursor restored.
        s.apply(Action::PopDrill);
        assert_eq!(s.current_index(), 3);
    }

    #[test]
    fn source_filter_inside_events_drill_skips_event_refetch_with_flash() {
        let mut s = AppState {
            current_section: Section::Sessions,
            ..AppState::default()
        };
        s.push_drill(Drill {
            kind: DrillKind::Events {
                source: crate::types::Source::Claude,
            },
            key: "abc".into(),
            label: "abc".into(),
            cursor: 0,
        });
        let out = s.apply(Action::SetSource(SourceFilter::Codex));
        // Filter persisted; the events slice is NOT re-fetched (the drilled
        // session is fixed to one source). Parent slices that genuinely
        // depend on source still refresh so popping back is correct.
        assert_eq!(s.source_filter, SourceFilter::Codex);
        assert!(!out.refresh.events);
        assert!(!out.refresh.sessions);
        assert!(out.refresh.left);
        assert_eq!(
            s.flash.as_deref(),
            Some("source filter ignored inside session")
        );
        // Subsequent change does NOT re-flash.
        s.flash = None;
        let _ = s.apply(Action::SetSource(SourceFilter::Cursor));
        assert!(s.flash.is_none());
    }

    // ---- new matrix tests for refresh_mask_for ------------------------------

    #[test]
    fn toggle_expand_does_no_refresh() {
        let mut s = AppState::default();
        let out = s.apply(Action::ToggleExpand);
        assert!(s.expanded);
        assert!(!out.refresh.any());
        assert!(!out.needs_refresh);
        // Still dirty so the redraw happens.
        assert!(out.dirty);
    }

    #[test]
    fn cycle_sort_at_root_only_refreshes_left() {
        let mut s = AppState {
            current_section: Section::Repos,
            ..AppState::default()
        };
        let out = s.apply(Action::CycleSort);
        assert!(out.refresh.left);
        assert!(!out.refresh.sessions);
        assert!(!out.refresh.events);
        assert!(!out.refresh.trend);
    }

    #[test]
    fn cycle_sort_in_sessions_drill_refreshes_sessions_too() {
        let mut s = AppState {
            current_section: Section::Repos,
            ..AppState::default()
        };
        s.push_drill(Drill {
            kind: DrillKind::Sessions {
                from_section: Section::Repos,
            },
            key: "tokctl".into(),
            label: "tokctl".into(),
            cursor: 0,
        });
        let out = s.apply(Action::CycleSort);
        assert!(out.refresh.left);
        assert!(out.refresh.sessions);
        assert!(!out.refresh.events);
    }

    #[test]
    fn cycle_sort_in_events_drill_refreshes_events_too() {
        let mut s = AppState {
            current_section: Section::Sessions,
            ..AppState::default()
        };
        s.push_drill(Drill {
            kind: DrillKind::Events {
                source: crate::types::Source::Claude,
            },
            key: "abc".into(),
            label: "abc".into(),
            cursor: 0,
        });
        let out = s.apply(Action::CycleSort);
        assert!(out.refresh.left);
        assert!(out.refresh.events);
        assert!(!out.refresh.sessions);
    }

    #[test]
    fn set_window_refreshes_left_trend_sparkline_only() {
        let mut s = AppState::default();
        let out = s.apply(Action::SetWindow(TimeWindow::Today));
        assert!(out.refresh.left);
        assert!(out.refresh.trend);
        assert!(out.refresh.sparkline);
        assert!(!out.refresh.sessions);
        assert!(!out.refresh.events);
    }

    #[test]
    fn set_granularity_refreshes_left_and_trend() {
        let mut s = AppState::default();
        let out = s.apply(Action::SetTrendGranularity(TrendGranularity::Daily));
        assert!(out.refresh.left);
        assert!(out.refresh.trend);
        assert!(!out.refresh.sparkline);
        assert!(!out.refresh.sessions);
    }

    #[test]
    fn cursor_move_at_root_refreshes_nothing() {
        let mut s = AppState {
            focus: Focus::Main,
            ..AppState::default()
        };
        let out = s.apply(Action::MoveDown);
        assert!(!out.needs_refresh, "no SQL on plain cursor moves");
        assert!(out.dirty, "still need to redraw to move the highlight");
    }

    #[test]
    fn refresh_action_invalidates_everything() {
        let mut s = AppState::default();
        let out = s.apply(Action::Refresh);
        assert!(out.refresh.left);
        assert!(out.refresh.sessions);
        assert!(out.refresh.events);
        assert!(out.refresh.trend);
        assert!(out.refresh.sparkline);
    }

    #[test]
    fn pop_drill_refreshes_nothing() {
        let mut s = AppState {
            current_section: Section::Repos,
            ..AppState::default()
        };
        s.push_drill(Drill {
            kind: DrillKind::Sessions {
                from_section: Section::Repos,
            },
            key: "tokctl".into(),
            label: "tokctl".into(),
            cursor: 0,
        });
        let out = s.apply(Action::PopDrill);
        assert!(!out.needs_refresh);
        assert!(out.dirty);
    }
}
