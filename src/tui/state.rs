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

    /// Whether `Enter` on a row in this section pushes a drill view.
    pub fn supports_drill(self) -> bool {
        matches!(self, Section::Repos)
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

/// Which area currently owns keyboard focus. `Sidebar` = section list,
/// `Main` = the active section's content (or its drill view).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Sidebar,
    Main,
}

/// One level of in-main drill. `key` is the stable identifier (repo key,
/// day bucket, etc.) for the row that was drilled into; `label` is the
/// human-readable breadcrumb chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drill {
    pub section: Section,
    pub key: String,
    pub label: String,
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
    pub drill: Option<Drill>,
    /// Active tab index per section (only meaningful when `Section::tabs()` is non-empty).
    pub tab_per_section: BTreeMap<Section, u8>,
    /// Last-selected row key per section (persisted; falls back to first row when stale).
    pub section_selection: BTreeMap<Section, String>,
    /// Live row-cursor index per section (not persisted).
    pub section_index: BTreeMap<Section, usize>,
    /// Live row-cursor index inside an active drill (not persisted).
    pub drill_index: usize,
    /// Sidebar row cursor (live, not persisted — derived from `current_section`).
    pub sidebar_index: usize,
    pub filter: FilterState,
    pub flash: Option<String>,
    /// Whether tables show expanded columns. Default false = compact.
    pub expanded: bool,
    /// One-shot intro flash on first launch under v3. Persisted so it shows once.
    pub seen_v3_intro: bool,
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
            drill: None,
            tab_per_section: BTreeMap::new(),
            section_selection: BTreeMap::new(),
            section_index: BTreeMap::new(),
            drill_index: 0,
            sidebar_index,
            filter: FilterState::default(),
            flash: None,
            expanded: false,
            seen_v3_intro: false,
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

    pub fn current_index(&self) -> usize {
        if self.drill.is_some() {
            return self.drill_index;
        }
        *self.section_index.get(&self.current_section).unwrap_or(&0)
    }

    pub fn set_current_index(&mut self, idx: usize) {
        if self.drill.is_some() {
            self.drill_index = idx;
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
            Action::DismissFlash => {
                // already cleared above
            }
            Action::FocusSidebar => {
                self.focus = Focus::Sidebar;
            }
            Action::FocusMain => {
                self.focus = Focus::Main;
            }
            Action::MoveUp => self.nudge_selection(-1),
            Action::MoveDown => self.nudge_selection(1),
            Action::PageUp => self.nudge_selection(-10),
            Action::PageDown => self.nudge_selection(10),
            Action::Top => self.set_selection(0),
            Action::Bottom => self.set_selection(usize::MAX),
            Action::NextSection => {
                self.switch_section(self.current_section.next(), &mut out);
            }
            Action::PrevSection => {
                self.switch_section(self.current_section.prev(), &mut out);
            }
            Action::JumpToSection(s) => {
                if self.current_section != s {
                    self.switch_section(s, &mut out);
                } else if self.drill.is_some() {
                    // already on the section but drilled — pop back
                    self.drill = None;
                    self.drill_index = 0;
                    out.dirty = true;
                }
            }
            Action::CycleTab => {
                let tabs = self.current_section.tabs();
                if tabs.len() >= 2 {
                    let cur = self.active_tab_index();
                    let next = ((cur as usize + 1) % tabs.len()) as u8;
                    self.tab_per_section.insert(self.current_section, next);
                    out.refresh.left = true;
                    out.refresh.trend = true;
                    out.needs_refresh = true;
                    out.dirty = true;
                }
            }
            Action::Drill => {
                if self.drill.is_none() && self.current_section.supports_drill() {
                    // The actual key/label is filled in by mod.rs after lookup;
                    // here we just mark intent. mod.rs reads the focused row,
                    // computes a Drill, and assigns it via `set_drill`.
                    // For the purposes of ApplyOutcome we still mark refresh:
                    // the drill view fetches scoped sessions.
                    out.refresh.sessions = true;
                    out.needs_refresh = true;
                    // Mark dirty=false: drill itself isn't persisted, but we
                    // want the next refresh cycle to run.
                }
            }
            Action::PopDrill => {
                if self.drill.is_some() {
                    self.drill = None;
                    self.drill_index = 0;
                    self.focus = Focus::Main;
                    out.dirty = true;
                } else {
                    self.focus = Focus::Sidebar;
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
                } else if self.drill.is_some() {
                    self.drill = None;
                    self.drill_index = 0;
                    self.focus = Focus::Main;
                    out.dirty = true;
                } else if self.focus == Focus::Main {
                    self.focus = Focus::Sidebar;
                }
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
                if !self.help_open {
                    self.detail_open = !self.detail_open;
                }
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
                    // Days section also buckets by granularity, so refresh
                    // the left-table data too.
                    out.refresh.left = true;
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
                self.filter.active = false;
            }
            Action::FilterCancel => {
                self.filter.active = false;
                self.filter.query.clear();
            }
            Action::Yank => {}
            Action::YankSummary => {}
            Action::ToggleExpand => {
                self.expanded = !self.expanded;
                out.refresh.left = true;
                out.refresh.sessions = true;
                out.needs_refresh = true;
                out.dirty = true;
            }
        }

        // Selection movement triggers a downstream refresh: in main (without
        // drill) it may rescope sessions; in the sidebar it changes the
        // active section, handled by the section-change check below.
        match action {
            Action::MoveUp
            | Action::MoveDown
            | Action::PageUp
            | Action::PageDown
            | Action::Top
            | Action::Bottom => {
                if self.focus != Focus::Sidebar {
                    out.refresh.sessions = true;
                    out.needs_refresh = true;
                }
                out.dirty = true;
            }
            _ => {}
        }

        // Catch-all: if any path changed the active section, force a full
        // refresh. This covers sidebar j/k/gg/G/page nav that updates
        // current_section directly via nudge_selection / set_selection.
        if self.current_section != section_before {
            out.refresh = RefreshMask::all();
            out.needs_refresh = true;
            out.dirty = true;
        }

        out
    }

    fn switch_section(&mut self, target: Section, out: &mut ApplyOutcome) {
        if self.current_section == target {
            return;
        }
        self.current_section = target;
        self.sidebar_index = Section::ALL.iter().position(|s| *s == target).unwrap_or(0);
        self.drill = None;
        self.drill_index = 0;
        self.focus = Focus::Main;
        out.refresh = RefreshMask::all();
        out.needs_refresh = true;
        out.dirty = true;
    }

    fn nudge_selection(&mut self, delta: isize) {
        if self.focus == Focus::Sidebar {
            let new = (self.sidebar_index as isize + delta).rem_euclid(Section::ALL.len() as isize)
                as usize;
            self.sidebar_index = new;
            self.current_section = Section::ALL[new];
            self.drill = None;
            self.drill_index = 0;
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
            self.drill = None;
            self.drill_index = 0;
        } else {
            self.set_current_index(idx);
        }
    }

    /// Push a drill view onto the main pane. Called by the event loop after
    /// the focused row's key/label is resolved from the data cache.
    pub fn set_drill(&mut self, drill: Drill) {
        self.drill = Some(drill);
        self.drill_index = 0;
        self.focus = Focus::Main;
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
    fn only_repos_supports_drill() {
        assert!(Section::Repos.supports_drill());
        assert!(!Section::Days.supports_drill());
        assert!(!Section::Provider.supports_drill());
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
        let mut s = AppState::default();
        s.set_drill(Drill {
            section: Section::Repos,
            key: "tokctl".into(),
            label: "tokctl".into(),
        });
        assert!(s.drill.is_some());
        let out = s.apply(Action::PopDrill);
        assert!(s.drill.is_none());
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
        let mut s = AppState::default();
        s.set_drill(Drill {
            section: Section::Repos,
            key: "x".into(),
            label: "x".into(),
        });
        s.apply(Action::Pop);
        assert!(s.drill.is_none());
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
    fn sidebar_move_triggers_full_refresh() {
        let mut s = AppState {
            focus: Focus::Sidebar,
            ..AppState::default()
        };
        let out = s.apply(Action::MoveDown);
        assert_eq!(s.current_section, Section::Models);
        assert!(out.needs_refresh, "section change must trigger refresh");
        assert!(out.refresh.left, "left/main pane data must refetch");
        assert!(out.refresh.sessions);
        assert!(out.refresh.trend);
    }

    #[test]
    fn sidebar_bottom_jumps_and_refreshes() {
        let mut s = AppState {
            focus: Focus::Sidebar,
            ..AppState::default()
        };
        let out = s.apply(Action::Bottom);
        assert_eq!(s.current_section, Section::Sessions);
        assert!(out.needs_refresh);
        assert!(out.refresh.left);
        assert!(out.refresh.trend);
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
        assert_eq!(loaded.current_section, Section::Provider);
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
        assert_eq!(loaded.current_section, Section::Provider);
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
        let mut s = AppState::default();
        s.set_drill(Drill {
            section: Section::Repos,
            key: "tokctl".into(),
            label: "tokctl".into(),
        });
        save(&p, &s).unwrap();
        let loaded = load(&p);
        assert!(loaded.drill.is_none());
    }
}
