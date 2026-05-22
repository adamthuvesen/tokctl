use super::refresh::{refresh_mask_for, Action, ApplyOutcome};
use super::types::*;

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
