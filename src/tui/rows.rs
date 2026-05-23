use chrono::{DateTime, Utc};

use crate::tui::state::{Section, SourceFilter, TimeWindow, TrendGranularity};
use crate::types::Source;

#[derive(Debug, Clone)]
pub struct LeftRow {
    /// Primary label shown in the pane.
    pub label: String,
    /// Internal key used to filter downstream panes. For repo/model/day it's
    /// the raw key; for session it's the session id.
    pub key: String,
    pub sessions: u64,
    pub total_tokens: u64,
    pub cost: f64,
    pub is_no_repo: bool,
    /// Most-recent timestamp for this row, when meaningful. Used by the
    /// recent sort for aggregate sections and by the Sessions "when" column.
    pub latest_ts: Option<DateTime<Utc>>,
    /// Source for this row, when the row maps 1:1 to a single source.
    pub source: Option<Source>,
}

#[derive(Debug, Clone)]
pub struct SessionRow {
    pub session_id: String,
    pub source: Source,
    pub latest_ts: DateTime<Utc>,
    pub project: Option<String>,
    pub cost: f64,
    pub total_tokens: u64,
}

/// One row per turn within a single session — the deepest drill level.
#[derive(Debug, Clone)]
pub struct EventRow {
    pub ts: DateTime<Utc>,
    pub model: String,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub cost: f64,
}

/// One time bucket with per-source (Claude / Codex / Cursor) costs.
#[derive(Debug, Clone)]
pub struct TrendRow {
    pub bucket: String,
    pub claude_cost: f64,
    pub codex_cost: f64,
    pub cursor_cost: f64,
    pub total_tokens: u64,
    pub total_cost: f64,
    pub is_current: bool,
}

#[derive(Debug, Clone)]
pub struct CacheStatus {
    pub cache_path: String,
    pub event_count: u64,
    pub freshness: String,
    pub last_query: DateTime<Utc>,
    pub mtime_ns: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshScope {
    Status,
    Left,
    Sessions,
    Events,
    Sparkline,
    Trend,
}

impl RefreshScope {
    pub fn as_str(self) -> &'static str {
        match self {
            RefreshScope::Status => "status",
            RefreshScope::Left => "rows",
            RefreshScope::Sessions => "sessions",
            RefreshScope::Events => "events",
            RefreshScope::Sparkline => "sparkline",
            RefreshScope::Trend => "trend",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshError {
    pub scope: RefreshScope,
    pub message: String,
}

impl RefreshError {
    pub fn new(scope: RefreshScope, err: impl std::fmt::Display) -> Self {
        Self {
            scope,
            message: err.to_string(),
        }
    }

    pub fn display_message(&self) -> String {
        format!("refresh failed: {}: {}", self.scope.as_str(), self.message)
    }
}

impl Default for CacheStatus {
    fn default() -> Self {
        Self {
            cache_path: crate::store::store_path().display().to_string(),
            event_count: 0,
            freshness: "unknown".into(),
            last_query: Utc::now(),
            mtime_ns: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct LeftMemoKey(
    pub Section,
    pub TimeWindow,
    pub SourceFilter,
    pub TrendGranularity,
);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct TrendMemoKey(pub TimeWindow, pub SourceFilter, pub TrendGranularity);
