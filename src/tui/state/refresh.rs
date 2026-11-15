use super::types::Section;

#[derive(Debug, Clone, Copy, Default)]
pub struct ApplyOutcome {
    pub quit: bool,
    pub dirty: bool,
}

#[derive(Debug, Clone)]
pub enum Action {
    Quit,
    MoveDown,
    MoveUp,
    PageDown,
    PageUp,
    Top,
    Bottom,
    NextSection,
    PrevSection,
    JumpToSection(Section),
    CycleTab,
    Drill,
    PopDrill,
    FocusSidebar,
    FocusMain,
    Pop,
    CycleSort,
    ToggleHelp,
    ToggleDetail,
    SetWindow(super::types::TimeWindow),
    SetSource(super::types::SourceFilter),
    SetTrendGranularity(super::types::TrendGranularity),
    Refresh,
    FilterOpen,
    FilterChar(char),
    FilterBackspace,
    FilterCommit,
    FilterCancel,
    Yank,
    YankSummary,
    ToggleExpand,
    ToggleFamily,
    DismissFlash,
    None,
}
