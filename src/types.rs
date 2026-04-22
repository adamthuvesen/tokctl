use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Claude,
    Codex,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Claude => "claude",
            Source::Codex => "codex",
        }
    }
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for Source {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "claude" => Ok(Source::Claude),
            "codex" => Ok(Source::Codex),
            other => Err(format!("unknown source: {other}")),
        }
    }
}

#[derive(Debug, Clone)]
pub struct UsageEvent {
    pub source: Source,
    pub timestamp: DateTime<Utc>,
    pub session_id: String,
    pub project_path: Option<String>,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
}

#[derive(Debug, Clone, Default)]
pub struct IngestStats {
    pub files_scanned: usize,
    pub files_skipped: usize,
    pub files_tailed: usize,
    pub files_full_parsed: usize,
    pub files_purged: usize,
    pub events_inserted: usize,
    pub skipped_lines: usize,
    pub unknown_models: HashSet<String>,
}

/// Label used on aggregate rows. `All` means "combined across sources".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceLabel {
    All,
    Source(Source),
}

impl SourceLabel {
    pub fn as_str(self) -> &'static str {
        match self {
            SourceLabel::All => "all",
            SourceLabel::Source(s) => s.as_str(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AggregateRow {
    pub key: String,
    pub source: SourceLabel,
    pub project_path: Option<String>,
    pub latest_timestamp: Option<DateTime<Utc>>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub total_tokens: u64,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportKind {
    Daily,
    Monthly,
    Session,
}
