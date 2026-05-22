use nucleo_matcher::{
    pattern::{CaseMatching, Normalization, Pattern},
    Config as NucleoConfig, Matcher,
};

use crate::tui::data::{LeftRow, SessionRow};
use crate::tui::state::{AppState, Focus};

pub fn should_filter(state: &AppState) -> bool {
    !state.filter.query.is_empty() && state.focus == Focus::Main
}

pub fn apply_filter_left(rows: &[LeftRow], state: &AppState) -> (Vec<LeftRow>, Vec<u32>) {
    filter_rows(rows, state, |r| r.label.as_str())
}

pub fn apply_filter_sessions(rows: &[SessionRow], state: &AppState) -> (Vec<SessionRow>, Vec<u32>) {
    filter_rows(rows, state, |r| {
        r.project.as_deref().unwrap_or(r.session_id.as_str())
    })
}

fn filter_rows<T: Clone>(
    rows: &[T],
    state: &AppState,
    label: impl Fn(&T) -> &str,
) -> (Vec<T>, Vec<u32>) {
    if !should_filter(state) {
        return (rows.to_vec(), Vec::new());
    }
    let mut matcher = Matcher::new(NucleoConfig::DEFAULT);
    let pat = Pattern::parse(
        &state.filter.query,
        CaseMatching::Ignore,
        Normalization::Smart,
    );
    let mut scored: Vec<(T, u32)> = rows
        .iter()
        .filter_map(|r| {
            let mut buf = Vec::new();
            let haystack = nucleo_matcher::Utf32Str::new(label(r), &mut buf);
            pat.score(haystack, &mut matcher).map(|s| (r.clone(), s))
        })
        .collect();
    scored.sort_by_key(|x| std::cmp::Reverse(x.1));
    let (rows, scores): (Vec<_>, Vec<_>) = scored.into_iter().unzip();
    (rows, scores)
}
