use super::{CheckSeverity, DoctorCheck};
use crate::paths::{
    cursor_credentials_path, default_claude_roots, default_codex_roots, default_cursor_roots,
    resolve_roots, tokscale_cursor_credentials_path, ResolveInput,
};
use crate::store::inspection::{CacheFacts, CacheFault};
use crate::store::schema::SCHEMA_VERSION;
use crate::types::Source;
use std::path::{Path, PathBuf};

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

pub(crate) fn cache_fault_check(path: &Path, fault: CacheFault) -> DoctorCheck {
    match fault {
        CacheFault::Missing => DoctorCheck::new(
            "cache",
            CheckSeverity::Warn,
            format!("cache does not exist at {}", path.display()),
        )
        .action("run tokctl daily or another cached report to create the cache"),
        CacheFault::Open(err) => DoctorCheck::new(
            "cache",
            CheckSeverity::Error,
            format!("could not open cache at {}", path.display()),
        )
        .details(vec![err]),
        CacheFault::SchemaNewer(v) => DoctorCheck::new(
            "cache",
            CheckSeverity::Error,
            format!("cache schema is newer than this tokctl ({v}, current {SCHEMA_VERSION})"),
        )
        .action("use a newer tokctl binary, preserving this cache"),
        CacheFault::SchemaMissing => DoctorCheck::new(
            "cache",
            CheckSeverity::Error,
            "cache schema version is missing",
        )
        .action("preserve this cache and set TOKCTL_CACHE_DIR to a new directory before ingesting"),
        CacheFault::Table { table, error } => DoctorCheck::new(
            "cache",
            CheckSeverity::Error,
            format!("could not inspect {table} in {}", path.display()),
        )
        .details(vec![error])
        .action("preserve this cache and inspect its schema before repair"),
        CacheFault::ModelQuery(err) => DoctorCheck::new(
            "pricing",
            CheckSeverity::Error,
            "could not query cached models",
        )
        .details(vec![err]),
        CacheFault::ModelRow(err) => DoctorCheck::new(
            "pricing",
            CheckSeverity::Error,
            "could not read cached model row",
        )
        .details(vec![err]),
        CacheFault::Provenance(err) => DoctorCheck::new(
            "provenance",
            CheckSeverity::Error,
            "could not inspect cache provenance",
        )
        .details(vec![err]),
    }
}

pub(crate) fn cache_checks(facts: CacheFacts) -> Vec<DoctorCheck> {
    let mut checks = Vec::new();
    let version = facts.schema_version;
    if version == SCHEMA_VERSION {
        checks.push(DoctorCheck::new(
            "cache",
            CheckSeverity::Ok,
            format!("cache schema is current ({SCHEMA_VERSION})"),
        ));
    } else {
        checks.push(
            DoctorCheck::new(
                "cache",
                CheckSeverity::Warn,
                format!("cache schema is old ({version}, current {SCHEMA_VERSION})"),
            )
            .action("run a cached report or tokctl daily --rebuild to refresh the cache"),
        );
    }

    let (event_count, file_count, repo_count) =
        (facts.event_count, facts.file_count, facts.repo_count);
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

    if let Some(max_mtime) = facts.latest_file_mtime_ns {
        checks.push(DoctorCheck::new(
            "cache",
            CheckSeverity::Ok,
            format!(
                "latest indexed file mtime: {}",
                format_ns_timestamp(max_mtime)
            ),
        ));
    }

    let unknown = facts.unpriced_models;
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

    if let Some(provenance) = facts.provenance {
        let stale = provenance.legacy_cost_rows > 0
            || provenance.stale_pricing_rows > 0
            || provenance.stale_parser_files > 0
            || provenance.stale_calendar;
        checks.push(DoctorCheck::new("provenance", if stale { CheckSeverity::Warn } else { CheckSeverity::Ok },
            if stale { "cached derivations need refresh" } else { "cached derivation provenance is current" })
            .details(vec![format!("legacy cost rows: {}; stale price rows: {}; stale parser files: {}; stale calendar: {}",
                provenance.legacy_cost_rows, provenance.stale_pricing_rows, provenance.stale_parser_files, provenance.stale_calendar)])
            .action("run a cached report with the source files available to refresh stale derivations"));
    }
    checks
}

fn format_ns_timestamp(ns: i64) -> String {
    let secs = ns / 1_000_000_000;
    chrono::DateTime::from_timestamp(secs, 0)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| "unknown".to_owned())
}

pub(crate) fn cursor_sync_check() -> DoctorCheck {
    let primary = cursor_credentials_path();
    let legacy = tokscale_cursor_credentials_path();
    let mut details = vec![
        format!("tokctl credentials: {}", primary.display()),
        format!("legacy credentials: {}", legacy.display()),
    ];
    match crate::cursor_sync::load_credentials_store() {
        Ok(Some(store)) if !store.accounts.is_empty() => {
            details.push(
                "credentials parsed locally; doctor did not validate or sync over the network"
                    .into(),
            );
            DoctorCheck::new(
                "cursor-sync",
                CheckSeverity::Ok,
                "Cursor credentials are configured locally",
            )
            .details(details)
            .action("run tokctl cursor status or tokctl cursor sync to validate remotely")
        }
        Ok(_) => DoctorCheck::new(
            "cursor-sync",
            CheckSeverity::Warn,
            "no Cursor credentials found",
        )
        .details(details)
        .action("run tokctl cursor login to enable optional Cursor sync"),
        Err(error) => {
            details.push(error.to_string());
            DoctorCheck::new(
                "cursor-sync",
                CheckSeverity::Error,
                "Cursor credential configuration is invalid",
            )
            .details(details)
            .action("repair the local credential file, then run tokctl cursor login")
        }
    }
}
