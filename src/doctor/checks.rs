use crate::paths::{
    cursor_credentials_path, default_claude_roots, default_codex_roots, default_cursor_roots,
    resolve_roots, tokscale_cursor_credentials_path, ResolveInput,
};
use crate::pricing;
use crate::store::schema::SCHEMA_VERSION;
use crate::types::Source;
use anyhow::Result;
use rusqlite::{Connection, OpenFlags};
use std::path::{Path, PathBuf};

use super::{CheckSeverity, DoctorCheck};

pub(crate) struct RootSet {
    pub(crate) roots: Vec<PathBuf>,
    pub(crate) user_supplied: bool,
}

pub(crate) fn resolve_source_roots(source: Source) -> RootSet {
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

pub(crate) fn existing_roots(roots: &[PathBuf]) -> Vec<PathBuf> {
    roots.iter().filter(|path| path.is_dir()).cloned().collect()
}

pub(crate) fn push_root_check(checks: &mut Vec<DoctorCheck>, source: Source, roots: &RootSet) {
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

pub(crate) fn discovery_check(source: Source, files: usize, no_roots: bool) -> DoctorCheck {
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
pub(crate) struct CacheInspection {
    pub(crate) event_count: u64,
    pub(crate) file_count: u64,
    pub(crate) repo_count: u64,
    pub(crate) checks: Vec<DoctorCheck>,
}

pub(crate) fn inspect_cache(path: &Path) -> std::result::Result<CacheInspection, DoctorCheck> {
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

pub(crate) fn unknown_priced_models(
    conn: &Connection,
) -> std::result::Result<Vec<String>, DoctorCheck> {
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

pub(crate) fn cursor_sync_check() -> DoctorCheck {
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
