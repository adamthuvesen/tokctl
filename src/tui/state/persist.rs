use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::types::{
    AppState, Section, Sort, SourceFilter, TimeWindow, TrendGranularity, STATE_VERSION,
};

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
