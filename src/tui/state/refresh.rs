use super::types::{AppState, DrillKind, Section};

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
    DismissFlash,
    None,
}

pub fn refresh_mask_for(action: &Action, section_before: Section, after: &AppState) -> RefreshMask {
    let drilled_kind = after.deepest_drill().map(|d| d.kind);

    if section_before != after.current_section {
        return RefreshMask {
            left: true,
            ..RefreshMask::default()
        };
    }

    match action {
        Action::Refresh => RefreshMask::all(),

        Action::SetWindow(_) => RefreshMask {
            left: true,
            sessions: matches!(drilled_kind, Some(DrillKind::Sessions { .. })),
            events: matches!(drilled_kind, Some(DrillKind::Events { .. })),
            trend: true,
            sparkline: true,
        },

        Action::SetSource(_) => RefreshMask {
            left: true,
            trend: true,
            sparkline: true,
            ..RefreshMask::default()
        },

        Action::SetTrendGranularity(_) => RefreshMask {
            left: true,
            trend: true,
            ..RefreshMask::default()
        },

        Action::CycleSort => {
            let at_sessions_root =
                after.current_section == Section::Sessions && !after.drill_active();
            RefreshMask {
                left: !at_sessions_root,
                sessions: at_sessions_root
                    || matches!(drilled_kind, Some(DrillKind::Sessions { .. })),
                events: matches!(drilled_kind, Some(DrillKind::Events { .. })),
                ..RefreshMask::default()
            }
        }

        Action::CycleTab => RefreshMask {
            left: true,
            trend: true,
            ..RefreshMask::default()
        },

        Action::Drill => match after.next_drill_kind_hint() {
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
