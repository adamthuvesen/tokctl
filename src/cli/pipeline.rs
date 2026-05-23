//! Shared CLI orchestration: root resolution, optional Cursor sync, cache vs no-cache ingest.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::Connection;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::cursor_sync::{has_configured_account, sync_active_account};
use crate::discovery::{discover_claude, discover_codex, discover_cursor, DiscoverOpts};
use crate::ingest::file_range::{ingest_claude_range, ingest_codex_range, ingest_cursor_range};
use crate::ingest::run::{run_ingest, RunIngestOptions};
use crate::paths::{
    cursor_sync_cache_dir, default_claude_roots, default_codex_roots, default_cursor_roots,
    resolve_roots, ResolveInput,
};
use crate::pricing;
use crate::repo::RepoIdentity;
use crate::reports::in_memory::{filter_by_date, resolve_repos};
use crate::store::{open_store, store_path};
use crate::types::{IngestStats, Source, UsageEvent};

/// Resolved filesystem roots for each source.
#[derive(Debug, Clone)]
pub struct RootDirs {
    pub claude: Vec<PathBuf>,
    pub codex: Vec<PathBuf>,
    pub cursor: Vec<PathBuf>,
}

/// Which sources to include when discovering / ingesting.
#[derive(Debug, Clone, Copy)]
pub struct SourceScope {
    pub filter: Option<Source>,
    pub include_claude: bool,
    pub include_codex: bool,
    pub include_cursor: bool,
}

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)] // reserved for unified RunInput in future CLI refactors
pub enum CacheMode {
    Cached { rebuild: bool },
    NoCache,
}

/// After [`prepare_cached`].
pub struct CachedRun {
    pub conn: Connection,
    pub stats: IngestStats,
}

/// After [`prepare_no_cache`].
pub struct NoCacheRun {
    pub events: Vec<UsageEvent>,
    /// Full catalog for repo-name resolution (unfiltered by date).
    pub repo_catalog: Vec<(UsageEvent, RepoIdentity)>,
    /// Events annotated with repo identity, date-filtered when bounds are set.
    pub resolved: Vec<(UsageEvent, RepoIdentity)>,
    pub skipped_lines: usize,
    pub file_errors: usize,
}

pub fn resolve_root_dirs(
    claude_dir: Option<&str>,
    codex_dir: Option<&str>,
    cursor_dir: Option<&str>,
) -> RootDirs {
    let env = |k: &str| std::env::var(k).ok();
    let claude = resolve_roots(ResolveInput {
        flag: claude_dir,
        tokctl_env: env("TOKCTL_CLAUDE_DIR").as_deref(),
        tool_env: env("CLAUDE_CONFIG_DIR").as_deref(),
        tool_env_suffix: Some("projects"),
        defaults: default_claude_roots(),
    });
    let codex = resolve_roots(ResolveInput {
        flag: codex_dir,
        tokctl_env: env("TOKCTL_CODEX_DIR").as_deref(),
        tool_env: env("CODEX_HOME").as_deref(),
        tool_env_suffix: Some("sessions"),
        defaults: default_codex_roots(),
    });
    let cursor = resolve_roots(ResolveInput {
        flag: cursor_dir,
        tokctl_env: env("TOKCTL_CURSOR_DIR").as_deref(),
        tool_env: None,
        tool_env_suffix: None,
        defaults: default_cursor_roots(),
    });
    RootDirs {
        claude: existing_dirs(claude),
        codex: existing_dirs(codex),
        cursor: existing_dirs(cursor),
    }
}

fn existing_dirs(resolved: crate::paths::ResolvedRoots) -> Vec<PathBuf> {
    resolved.roots.into_iter().filter(|p| p.is_dir()).collect()
}

pub fn cursor_sync_target_dir(cursor_dir: Option<&str>) -> PathBuf {
    let env = |k: &str| std::env::var(k).ok();
    let resolved = resolve_roots(ResolveInput {
        flag: cursor_dir,
        tokctl_env: env("TOKCTL_CURSOR_DIR").as_deref(),
        tool_env: None,
        tool_env_suffix: None,
        defaults: default_cursor_roots(),
    });
    resolved
        .roots
        .into_iter()
        .next()
        .unwrap_or_else(cursor_sync_cache_dir)
}

pub fn maybe_sync_cursor(target_dir: Option<&Path>) {
    if !has_configured_account() {
        return;
    }
    let result = sync_active_account(target_dir);
    if !result.synced {
        if let Some(error) = result.error {
            eprintln!("warning: Cursor sync failed: {error}");
        }
    }
}

pub fn sync_cursor_if_needed(scope: SourceScope, cursor_dir: Option<&str>) {
    if scope.include_cursor {
        let target = cursor_sync_target_dir(cursor_dir);
        maybe_sync_cursor(Some(&target));
    }
}

pub fn prepare_cached(scope: SourceScope, roots: &RootDirs, rebuild: bool) -> Result<CachedRun> {
    let cache_path = store_path();
    if rebuild {
        std::fs::remove_file(&cache_path).ok();
    }
    let mut conn = open_store(&cache_path)
        .with_context(|| format!("opening cache at {}", cache_path.display()))?;
    let stats = run_ingest(RunIngestOptions {
        conn: &mut conn,
        claude_roots: roots.claude.clone(),
        codex_roots: roots.codex.clone(),
        cursor_roots: roots.cursor.clone(),
        include_claude: scope.include_claude,
        include_codex: scope.include_codex,
        include_cursor: scope.include_cursor,
        safety_window_ms: 60 * 60 * 1000,
        now_ms: 0,
    })?;
    Ok(CachedRun { conn, stats })
}

pub fn prepare_no_cache(scope: SourceScope, roots: &RootDirs) -> Result<NoCacheRun> {
    let (events, skipped_lines, file_errors) = gather_events(scope, roots)?;
    let repo_catalog = resolve_repos(&events);
    let resolved = repo_catalog.clone();
    Ok(NoCacheRun {
        events,
        repo_catalog,
        resolved,
        skipped_lines,
        file_errors,
    })
}

impl NoCacheRun {
    pub fn with_date_filter(
        mut self,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
    ) -> Self {
        if since.is_none() && until.is_none() {
            return self;
        }
        let filtered = filter_by_date(&self.events, since, until);
        self.resolved = resolve_repos(&filtered);
        self
    }
}

pub fn gather_events(
    scope: SourceScope,
    roots: &RootDirs,
) -> Result<(Vec<UsageEvent>, usize, usize)> {
    let mut events: Vec<UsageEvent> = Vec::new();
    let mut skipped_lines = 0usize;
    let mut file_errors = 0usize;
    let now_ms = chrono::Utc::now().timestamp_millis();
    let discover_opts = DiscoverOpts {
        safety_window_ms: 60 * 60 * 1000,
        now_ms,
    };
    let empty_manifest = std::collections::HashMap::<PathBuf, crate::store::FileManifestRow>::new();

    if scope.include_claude {
        let d = discover_claude(&roots.claude, &empty_manifest, discover_opts);
        for f in &d.files {
            match ingest_claude_range(&f.path, f.project.as_deref(), 0, f.size) {
                Ok(r) => {
                    skipped_lines += r.skipped_lines;
                    events.extend(r.events);
                }
                Err(_) => file_errors += 1,
            }
        }
    }
    if scope.include_codex {
        let d = discover_codex(&roots.codex, &empty_manifest, discover_opts);
        for f in &d.files {
            match ingest_codex_range(&f.path, 0, f.size) {
                Ok(r) => {
                    skipped_lines += r.skipped_lines;
                    events.extend(r.events);
                }
                Err(_) => file_errors += 1,
            }
        }
    }
    if scope.include_cursor {
        let d = discover_cursor(&roots.cursor, &empty_manifest, discover_opts);
        for f in &d.files {
            match ingest_cursor_range(&f.path) {
                Ok(r) => {
                    skipped_lines += r.skipped_lines;
                    events.extend(r.events);
                }
                Err(_) => file_errors += 1,
            }
        }
    }
    Ok((events, skipped_lines, file_errors))
}

pub fn collect_unknown_from_db(
    conn: &Connection,
    source: Option<Source>,
    unknown: &mut HashSet<String>,
) {
    let sql = match source {
        Some(_) => "SELECT DISTINCT model, source FROM events WHERE source = ?1",
        None => "SELECT DISTINCT model, source FROM events",
    };
    let mut stmt = match conn.prepare(sql) {
        Ok(s) => s,
        Err(_) => return,
    };
    let iter: Result<Vec<(String, String)>, _> = match source {
        Some(s) => stmt
            .query_map([s.as_str()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .and_then(|rs| rs.collect()),
        None => stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .and_then(|rs| rs.collect()),
    };
    if let Ok(models) = iter {
        for (m, src) in models {
            if src == Source::Cursor.as_str() {
                continue;
            }
            if !pricing::has_price(&m) {
                unknown.insert(m);
            }
        }
    }
}

pub fn finish_cached_warnings(
    conn: &Connection,
    stats: &IngestStats,
    source: Option<Source>,
) -> (HashSet<String>, usize, usize) {
    let mut unknown = stats.unknown_models.clone();
    collect_unknown_from_db(conn, source, &mut unknown);
    (unknown, stats.skipped_lines, stats.file_errors)
}
