mod checks;
mod render;

use crate::discovery::{discover_claude, discover_codex, discover_cursor, DiscoverOpts};
use crate::store::store_path;
use crate::types::Source;
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;

pub use render::{render_human, render_json};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckSeverity {
    Ok,
    Warn,
    Error,
}

impl CheckSeverity {
    pub fn as_str(self) -> &'static str {
        match self {
            CheckSeverity::Ok => "ok",
            CheckSeverity::Warn => "warn",
            CheckSeverity::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DoctorCheck {
    pub category: String,
    pub severity: CheckSeverity,
    pub message: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub details: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
}

impl DoctorCheck {
    pub(crate) fn new(category: &str, severity: CheckSeverity, message: impl Into<String>) -> Self {
        Self {
            category: category.to_owned(),
            severity,
            message: message.into(),
            details: Vec::new(),
            action: None,
        }
    }

    pub(crate) fn details(mut self, details: Vec<String>) -> Self {
        self.details = details;
        self
    }

    pub(crate) fn action(mut self, action: impl Into<String>) -> Self {
        self.action = Some(action.into());
        self
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DoctorSummary {
    pub cache_path: String,
    pub event_count: u64,
    pub file_count: u64,
    pub repo_count: u64,
    pub discovered_claude_files: usize,
    pub discovered_codex_files: usize,
    pub discovered_cursor_files: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct DoctorReport {
    pub status: CheckSeverity,
    pub checks: Vec<DoctorCheck>,
    pub summary: DoctorSummary,
}

pub fn run() -> DoctorReport {
    let cache_path = store_path();
    let mut checks = Vec::new();
    let claude = checks::resolve_source_roots(Source::Claude);
    let codex = checks::resolve_source_roots(Source::Codex);
    let cursor = checks::resolve_source_roots(Source::Cursor);

    checks::push_root_check(&mut checks, Source::Claude, &claude);
    checks::push_root_check(&mut checks, Source::Codex, &codex);
    checks::push_root_check(&mut checks, Source::Cursor, &cursor);

    let discover_opts = DiscoverOpts {
        safety_window_ms: 60 * 60 * 1000,
        now_ms: chrono::Utc::now().timestamp_millis(),
    };
    let manifest: HashMap<PathBuf, crate::store::FileManifestRow> = HashMap::new();
    let claude_discovery = discover_claude(
        &checks::existing_roots(&claude.roots),
        &manifest,
        discover_opts,
    );
    let codex_discovery = discover_codex(
        &checks::existing_roots(&codex.roots),
        &manifest,
        discover_opts,
    );
    let cursor_discovery = discover_cursor(
        &checks::existing_roots(&cursor.roots),
        &manifest,
        discover_opts,
    );
    checks.push(checks::discovery_check(
        Source::Claude,
        claude_discovery.files.len(),
        claude.roots.is_empty(),
    ));
    checks.push(checks::discovery_check(
        Source::Codex,
        codex_discovery.files.len(),
        codex.roots.is_empty(),
    ));
    checks.push(checks::discovery_check(
        Source::Cursor,
        cursor_discovery.files.len(),
        cursor.roots.is_empty(),
    ));

    let mut summary = DoctorSummary {
        cache_path: cache_path.display().to_string(),
        event_count: 0,
        file_count: 0,
        repo_count: 0,
        discovered_claude_files: claude_discovery.files.len(),
        discovered_codex_files: codex_discovery.files.len(),
        discovered_cursor_files: cursor_discovery.files.len(),
    };

    match checks::inspect_cache(&cache_path) {
        Ok(cache) => {
            summary.event_count = cache.event_count;
            summary.file_count = cache.file_count;
            summary.repo_count = cache.repo_count;
            checks.extend(cache.checks);
        }
        Err(check) => checks.push(check),
    }

    checks.push(checks::cursor_sync_check());

    let status = checks
        .iter()
        .map(|check| check.severity)
        .max()
        .unwrap_or(CheckSeverity::Ok);

    DoctorReport {
        status,
        checks,
        summary,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::schema::DDL;
    use rusqlite::Connection;

    #[test]
    fn severity_status_uses_highest_check() {
        let report = DoctorReport {
            status: [CheckSeverity::Ok, CheckSeverity::Warn]
                .into_iter()
                .max()
                .unwrap(),
            checks: Vec::new(),
            summary: DoctorSummary {
                cache_path: "/tmp/cache.db".into(),
                event_count: 0,
                file_count: 0,
                repo_count: 0,
                discovered_claude_files: 0,
                discovered_codex_files: 0,
                discovered_cursor_files: 0,
            },
        };
        assert_eq!(report.status, CheckSeverity::Warn);
    }

    #[test]
    fn missing_cache_is_warning_and_not_created() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.db");
        let err = checks::inspect_cache(&path).unwrap_err();
        assert_eq!(err.severity, CheckSeverity::Warn);
        assert!(!path.exists());
    }

    #[test]
    fn newer_schema_is_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             INSERT INTO meta (key, value) VALUES ('schema_version', '999');",
        )
        .unwrap();
        drop(conn);

        let inspected = checks::inspect_cache(&path).unwrap();
        let schema = inspected
            .checks
            .iter()
            .find(|check| check.message.contains("newer"))
            .unwrap();
        assert_eq!(schema.severity, CheckSeverity::Error);
    }

    #[test]
    fn unknown_priced_models_ignore_cursor() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(DDL).unwrap();
        conn.execute(
            "INSERT INTO events
             (file_path, source, ts, day, month, session_id, project_path, repo, model,
              input, output, cache_read, cache_write, cost_usd)
             VALUES
             ('/a', 'claude', 1, '2026-04-01', '2026-04', 's1', NULL, NULL, 'mystery', 1, 0, 0, 0, 0),
             ('/b', 'cursor', 1, '2026-04-01', '2026-04', 's2', NULL, NULL, 'composer-1', 1, 0, 0, 0, 1)",
            [],
        )
        .unwrap();
        let unknown = checks::unknown_priced_models(&conn).unwrap();
        assert_eq!(unknown, vec!["mystery"]);
    }

    #[test]
    fn doctor_json_contains_status() {
        let report = DoctorReport {
            status: CheckSeverity::Ok,
            checks: vec![DoctorCheck::new("cache", CheckSeverity::Ok, "fine")],
            summary: DoctorSummary {
                cache_path: "/tmp/cache.db".into(),
                event_count: 1,
                file_count: 1,
                repo_count: 0,
                discovered_claude_files: 0,
                discovered_codex_files: 0,
                discovered_cursor_files: 0,
            },
        };
        let json = render_json(&report);
        assert!(json.contains("\"status\": \"ok\""));
    }
}
