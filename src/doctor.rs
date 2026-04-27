use crate::discovery::{discover_claude, discover_codex, discover_cursor, DiscoverOpts};
use crate::paths::{
    cursor_credentials_path, default_claude_roots, default_codex_roots, default_cursor_roots,
    resolve_roots, tokscale_cursor_credentials_path, ResolveInput,
};
use crate::pricing;
use crate::store::schema::SCHEMA_VERSION;
use crate::store::{store_path, FileManifestRow};
use crate::types::Source;
use anyhow::Result;
use comfy_table::{presets::UTF8_FULL, Cell, ContentArrangement, Row, Table};
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

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
    fn new(category: &str, severity: CheckSeverity, message: impl Into<String>) -> Self {
        Self {
            category: category.to_owned(),
            severity,
            message: message.into(),
            details: Vec::new(),
            action: None,
        }
    }

    fn details(mut self, details: Vec<String>) -> Self {
        self.details = details;
        self
    }

    fn action(mut self, action: impl Into<String>) -> Self {
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

#[derive(Debug, Clone)]
struct RootSet {
    roots: Vec<PathBuf>,
    user_supplied: bool,
}

pub fn run() -> DoctorReport {
    let cache_path = store_path();
    let mut checks = Vec::new();
    let claude = resolve_source_roots(Source::Claude);
    let codex = resolve_source_roots(Source::Codex);
    let cursor = resolve_source_roots(Source::Cursor);

    push_root_check(&mut checks, Source::Claude, &claude);
    push_root_check(&mut checks, Source::Codex, &codex);
    push_root_check(&mut checks, Source::Cursor, &cursor);

    let discover_opts = DiscoverOpts {
        safety_window_ms: 60 * 60 * 1000,
        now_ms: chrono::Utc::now().timestamp_millis(),
    };
    let manifest: HashMap<PathBuf, FileManifestRow> = HashMap::new();
    let claude_discovery =
        discover_claude(&existing_roots(&claude.roots), &manifest, discover_opts);
    let codex_discovery = discover_codex(&existing_roots(&codex.roots), &manifest, discover_opts);
    let cursor_discovery =
        discover_cursor(&existing_roots(&cursor.roots), &manifest, discover_opts);
    checks.push(discovery_check(
        Source::Claude,
        claude_discovery.files.len(),
        claude.roots.is_empty(),
    ));
    checks.push(discovery_check(
        Source::Codex,
        codex_discovery.files.len(),
        codex.roots.is_empty(),
    ));
    checks.push(discovery_check(
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

    match inspect_cache(&cache_path) {
        Ok(cache) => {
            summary.event_count = cache.event_count;
            summary.file_count = cache.file_count;
            summary.repo_count = cache.repo_count;
            checks.extend(cache.checks);
        }
        Err(check) => checks.push(check),
    }

    checks.push(cursor_sync_check());

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

fn resolve_source_roots(source: Source) -> RootSet {
    let env = |k: &str| std::env::var(k).ok();
    let resolved = match source {
        Source::Claude => resolve_roots(ResolveInput {
            flag: None,
            tokctl_env: env("TOKCTL_CLAUDE_DIR").as_deref(),
            tool_env: env("CLAUDE_CONFIG_DIR").as_deref(),
            tool_env_suffix: Some("projects"),
            defaults: default_claude_roots(),
        }),
        Source::Codex => resolve_roots(ResolveInput {
            flag: None,
            tokctl_env: env("TOKCTL_CODEX_DIR").as_deref(),
            tool_env: env("CODEX_HOME").as_deref(),
            tool_env_suffix: Some("sessions"),
            defaults: default_codex_roots(),
        }),
        Source::Cursor => resolve_roots(ResolveInput {
            flag: None,
            tokctl_env: env("TOKCTL_CURSOR_DIR").as_deref(),
            tool_env: None,
            tool_env_suffix: None,
            defaults: default_cursor_roots(),
        }),
    };
    RootSet {
        roots: resolved.roots,
        user_supplied: resolved.user_supplied,
    }
}

fn existing_roots(roots: &[PathBuf]) -> Vec<PathBuf> {
    roots.iter().filter(|path| path.is_dir()).cloned().collect()
}

fn push_root_check(checks: &mut Vec<DoctorCheck>, source: Source, roots: &RootSet) {
    let missing: Vec<String> = roots
        .roots
        .iter()
        .filter(|path| !path.is_dir())
        .map(|path| path.display().to_string())
        .collect();
    let existing: Vec<String> = roots
        .roots
        .iter()
        .filter(|path| path.is_dir())
        .map(|path| path.display().to_string())
        .collect();

    let source_name = source.as_str();
    if existing.is_empty() {
        let origin = if roots.user_supplied {
            "configured"
        } else {
            "default"
        };
        checks.push(
            DoctorCheck::new(
                "roots",
                CheckSeverity::Warn,
                format!("no {source_name} {origin} roots exist"),
            )
            .details(missing)
            .action(format!(
                "set TOKCTL_{}_DIR or pass the source directory to a report command",
                source_name.to_ascii_uppercase()
            )),
        );
    } else {
        checks.push(
            DoctorCheck::new(
                "roots",
                CheckSeverity::Ok,
                format!("{} {source_name} root(s) available", existing.len()),
            )
            .details(existing),
        );
        if !missing.is_empty() {
            checks.push(
                DoctorCheck::new(
                    "roots",
                    CheckSeverity::Warn,
                    format!("{} {source_name} root(s) are missing", missing.len()),
                )
                .details(missing),
            );
        }
    }
}

fn discovery_check(source: Source, files: usize, no_roots: bool) -> DoctorCheck {
    let severity = if files > 0 || no_roots {
        CheckSeverity::Ok
    } else {
        CheckSeverity::Warn
    };
    let mut check = DoctorCheck::new(
        "discovery",
        severity,
        format!("discovered {files} {} input file(s)", source.as_str()),
    );
    if severity == CheckSeverity::Warn {
        check =
            check.action("run tokctl doctor after confirming the source root contains usage data");
    }
    check
}

#[derive(Debug)]
struct CacheInspection {
    event_count: u64,
    file_count: u64,
    repo_count: u64,
    checks: Vec<DoctorCheck>,
}

fn inspect_cache(path: &Path) -> std::result::Result<CacheInspection, DoctorCheck> {
    if !path.exists() {
        return Err(DoctorCheck::new(
            "cache",
            CheckSeverity::Warn,
            format!("cache does not exist at {}", path.display()),
        )
        .action("run tokctl daily or another cached report to create the cache"));
    }

    let conn =
        Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|err| {
            DoctorCheck::new(
                "cache",
                CheckSeverity::Error,
                format!("could not open cache at {}", path.display()),
            )
            .details(vec![err.to_string()])
        })?;

    let mut checks = Vec::new();
    let schema_version = read_schema_version(&conn);
    match schema_version {
        Some(SCHEMA_VERSION) => checks.push(DoctorCheck::new(
            "cache",
            CheckSeverity::Ok,
            format!("cache schema is current ({SCHEMA_VERSION})"),
        )),
        Some(v) if v < SCHEMA_VERSION => checks.push(
            DoctorCheck::new(
                "cache",
                CheckSeverity::Warn,
                format!("cache schema is old ({v}, current {SCHEMA_VERSION})"),
            )
            .action("run a cached report or tokctl daily --rebuild to refresh the cache"),
        ),
        Some(v) => checks.push(
            DoctorCheck::new(
                "cache",
                CheckSeverity::Error,
                format!("cache schema is newer than this tokctl ({v}, current {SCHEMA_VERSION})"),
            )
            .action("use a newer tokctl binary or rebuild the cache with this version"),
        ),
        None => checks.push(
            DoctorCheck::new(
                "cache",
                CheckSeverity::Error,
                "cache schema version is missing",
            )
            .action("run tokctl daily --rebuild"),
        ),
    }

    let file_count = count_table(&conn, "files").unwrap_or(0);
    let event_count = count_table(&conn, "events").unwrap_or(0);
    let repo_count = count_table(&conn, "repos").unwrap_or(0);
    checks.push(
        DoctorCheck::new(
            "cache",
            if event_count > 0 {
                CheckSeverity::Ok
            } else {
                CheckSeverity::Warn
            },
            format!(
                "cache contains {event_count} event(s), {file_count} file(s), {repo_count} repo(s)"
            ),
        )
        .action(if event_count == 0 {
            "run tokctl daily to ingest local usage"
        } else {
            "run tokctl daily --rebuild if these counts look stale"
        }),
    );

    if let Some(max_mtime) = max_file_mtime_ns(&conn) {
        checks.push(DoctorCheck::new(
            "cache",
            CheckSeverity::Ok,
            format!(
                "latest indexed file mtime: {}",
                format_ns_timestamp(max_mtime)
            ),
        ));
    }

    if !table_exists(&conn, "events") {
        checks.push(
            DoctorCheck::new(
                "pricing",
                CheckSeverity::Warn,
                "events table is unavailable",
            )
            .action("run tokctl daily --rebuild"),
        );
    } else {
        let unknown = unknown_priced_models(&conn)?;
        if unknown.is_empty() {
            checks.push(DoctorCheck::new(
                "pricing",
                CheckSeverity::Ok,
                "all cached Claude/Codex models have static prices",
            ));
        } else {
            checks.push(
                DoctorCheck::new(
                    "pricing",
                    CheckSeverity::Warn,
                    format!(
                        "{} cached Claude/Codex model(s) have no static price",
                        unknown.len()
                    ),
                )
                .details(unknown)
                .action("update src/pricing.rs, then run tokctl daily --rebuild"),
            );
        }
    }

    Ok(CacheInspection {
        event_count,
        file_count,
        repo_count,
        checks,
    })
}

fn read_schema_version(conn: &Connection) -> Option<i64> {
    conn.query_row(
        "SELECT value FROM meta WHERE key = 'schema_version'",
        [],
        |row| row.get::<_, String>(0),
    )
    .ok()
    .and_then(|value| value.parse::<i64>().ok())
}

fn count_table(conn: &Connection, table: &str) -> Result<u64> {
    let sql = format!("SELECT COUNT(*) FROM {table}");
    let count: i64 = conn.query_row(&sql, [], |row| row.get(0))?;
    Ok(count.max(0) as u64)
}

fn table_exists(conn: &Connection, table: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [table],
        |_| Ok(()),
    )
    .is_ok()
}

fn max_file_mtime_ns(conn: &Connection) -> Option<i64> {
    conn.query_row("SELECT MAX(mtime_ns) FROM files", [], |row| {
        row.get::<_, Option<i64>>(0)
    })
    .ok()
    .flatten()
}

fn format_ns_timestamp(ns: i64) -> String {
    let secs = ns / 1_000_000_000;
    chrono::DateTime::from_timestamp(secs, 0)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| "unknown".to_owned())
}

fn unknown_priced_models(conn: &Connection) -> std::result::Result<Vec<String>, DoctorCheck> {
    let mut stmt = conn
        .prepare("SELECT DISTINCT model, source FROM events ORDER BY model")
        .map_err(|err| {
            DoctorCheck::new(
                "pricing",
                CheckSeverity::Error,
                "could not query cached models",
            )
            .details(vec![err.to_string()])
        })?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|err| {
            DoctorCheck::new(
                "pricing",
                CheckSeverity::Error,
                "could not query cached models",
            )
            .details(vec![err.to_string()])
        })?;
    let mut unknown = Vec::new();
    for row in rows {
        let (model, source) = row.map_err(|err| {
            DoctorCheck::new(
                "pricing",
                CheckSeverity::Error,
                "could not read cached model row",
            )
            .details(vec![err.to_string()])
        })?;
        if source != Source::Cursor.as_str() && !pricing::has_price(&model) {
            unknown.push(model);
        }
    }
    unknown.sort();
    unknown.dedup();
    Ok(unknown)
}

fn cursor_sync_check() -> DoctorCheck {
    let primary = cursor_credentials_path();
    let legacy = tokscale_cursor_credentials_path();
    let mut details = vec![
        format!("tokctl credentials: {}", primary.display()),
        format!("legacy credentials: {}", legacy.display()),
    ];
    if primary.exists() || legacy.exists() {
        details
            .push("credentials present; doctor did not validate or sync over the network".into());
        DoctorCheck::new(
            "cursor-sync",
            CheckSeverity::Ok,
            "Cursor credentials are present locally",
        )
        .details(details)
        .action("run tokctl cursor status or tokctl cursor sync to validate remotely")
    } else {
        DoctorCheck::new(
            "cursor-sync",
            CheckSeverity::Warn,
            "no Cursor credentials found",
        )
        .details(details)
        .action("run tokctl cursor login to enable optional Cursor sync")
    }
}

pub fn render_human(report: &DoctorReport) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "tokctl doctor: {}\ncache: {}\n\n",
        report.status.as_str(),
        report.summary.cache_path
    ));

    let mut grouped: BTreeMap<&str, Vec<&DoctorCheck>> = BTreeMap::new();
    for check in &report.checks {
        grouped.entry(&check.category).or_default().push(check);
    }

    for (category, checks) in grouped {
        let mut table = Table::new();
        table
            .load_preset(UTF8_FULL)
            .set_content_arrangement(ContentArrangement::Dynamic);
        table.set_header(["status", category, "action"].iter().map(|s| Cell::new(*s)));
        for check in checks {
            let mut message = check.message.clone();
            if !check.details.is_empty() {
                message.push('\n');
                message.push_str(&check.details.join("\n"));
            }
            table.add_row(Row::from(vec![
                Cell::new(check.severity.as_str()),
                Cell::new(message),
                Cell::new(check.action.clone().unwrap_or_default()),
            ]));
        }
        out.push_str(&table.to_string());
        out.push('\n');
    }
    out
}

pub fn render_json(report: &DoctorReport) -> String {
    serde_json::to_string_pretty(report).unwrap_or_else(|_| "{}".into())
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
        let err = inspect_cache(&path).unwrap_err();
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

        let inspected = inspect_cache(&path).unwrap();
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
        let unknown = unknown_priced_models(&conn).unwrap();
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
