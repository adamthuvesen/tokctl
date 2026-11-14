mod checks;
mod render;

use crate::discovery::{discover_claude, discover_codex, discover_cursor};
use crate::store::store_path;
use crate::types::Source;
use serde::Serialize;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct DoctorCheckCounts {
    pub ok: usize,
    pub warn: usize,
    pub error: usize,
}

impl DoctorCheckCounts {
    pub(crate) fn from_checks(checks: &[DoctorCheck]) -> Self {
        let mut counts = Self {
            ok: 0,
            warn: 0,
            error: 0,
        };
        for check in checks {
            match check.severity {
                CheckSeverity::Ok => counts.ok += 1,
                CheckSeverity::Warn => counts.warn += 1,
                CheckSeverity::Error => counts.error += 1,
            }
        }
        counts
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DoctorReport {
    pub status: CheckSeverity,
    pub check_counts: DoctorCheckCounts,
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

    let claude_discovery = discover_claude(&checks::existing_roots(&claude.roots));
    let codex_discovery = discover_codex(&checks::existing_roots(&codex.roots));
    let cursor_discovery = discover_cursor(&checks::existing_roots(&cursor.roots));
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

    let discovery_errors: Vec<String> = [&claude_discovery, &codex_discovery, &cursor_discovery]
        .into_iter()
        .flat_map(|discovery| &discovery.errors)
        .map(|error| format!("{}: {}", error.path.display(), error.reason))
        .collect();
    if !discovery_errors.is_empty() {
        checks.push(
            DoctorCheck::new(
                "discovery",
                CheckSeverity::Warn,
                "some source paths could not be read",
            )
            .details(discovery_errors)
            .action("check source path permissions, then run tokctl doctor again"),
        );
    }

    let mut summary = DoctorSummary {
        cache_path: cache_path.display().to_string(),
        event_count: 0,
        file_count: 0,
        repo_count: 0,
        discovered_claude_files: claude_discovery.files.len(),
        discovered_codex_files: codex_discovery.files.len(),
        discovered_cursor_files: cursor_discovery.files.len(),
    };

    match crate::store::inspection::inspect_cache(&cache_path) {
        Ok(facts) => {
            summary.event_count = facts.event_count;
            summary.file_count = facts.file_count;
            summary.repo_count = facts.repo_count;
            checks.extend(checks::cache_checks(facts));
        }
        Err(fault) => checks.push(checks::cache_fault_check(&cache_path, fault)),
    }

    checks.push(checks::cursor_sync_check());

    let status = checks
        .iter()
        .map(|check| check.severity)
        .max()
        .unwrap_or(CheckSeverity::Ok);
    let check_counts = DoctorCheckCounts::from_checks(&checks);

    DoctorReport {
        status,
        check_counts,
        checks,
        summary,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_status_uses_highest_check() {
        let report = DoctorReport {
            status: [CheckSeverity::Ok, CheckSeverity::Warn]
                .into_iter()
                .max()
                .unwrap(),
            check_counts: DoctorCheckCounts {
                ok: 0,
                warn: 0,
                error: 0,
            },
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
    fn check_counts_tally_each_severity() {
        let checks = vec![
            DoctorCheck::new("cache", CheckSeverity::Ok, "fine"),
            DoctorCheck::new("roots", CheckSeverity::Warn, "missing"),
            DoctorCheck::new("cache", CheckSeverity::Warn, "stale"),
            DoctorCheck::new("schema", CheckSeverity::Error, "newer"),
        ];

        assert_eq!(
            DoctorCheckCounts::from_checks(&checks),
            DoctorCheckCounts {
                ok: 1,
                warn: 2,
                error: 1,
            }
        );
    }

    #[test]
    fn doctor_rendering_contains_check_counts() {
        let report = DoctorReport {
            status: CheckSeverity::Warn,
            check_counts: DoctorCheckCounts {
                ok: 1,
                warn: 2,
                error: 0,
            },
            checks: vec![
                DoctorCheck::new("cache", CheckSeverity::Ok, "fine"),
                DoctorCheck::new("roots", CheckSeverity::Warn, "missing"),
                DoctorCheck::new("roots", CheckSeverity::Warn, "empty"),
            ],
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

        assert!(render_human(&report).contains("checks: 1 ok, 2 warn, 0 error"));
    }

    #[test]
    fn doctor_json_contains_status_and_check_counts() {
        let report = DoctorReport {
            status: CheckSeverity::Ok,
            check_counts: DoctorCheckCounts {
                ok: 1,
                warn: 0,
                error: 0,
            },
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
        assert!(json.contains("\"check_counts\": {"));
        assert!(json.contains("\"ok\": 1"));
    }
}
