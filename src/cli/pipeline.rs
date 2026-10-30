//! Shared CLI orchestration: root resolution, optional Cursor sync, cache vs no-cache ingest.

use anyhow::Result;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::cursor_sync::{has_configured_account, sync_active_account};
use crate::ingest::run::{run_ingest, RunIngestOptions};
use crate::paths::{
    cursor_sync_cache_dir, default_claude_roots, default_codex_roots, default_cursor_roots,
    resolve_roots, ResolveInput,
};
use crate::store::{store_path, Store};
use crate::types::{IngestStats, Source};

/// Resolved filesystem roots for each source.
#[derive(Debug, Clone)]
pub struct RootDirs {
    pub claude: Vec<PathBuf>,
    pub codex: Vec<PathBuf>,
    pub cursor: Vec<PathBuf>,
}

#[derive(Debug, Clone, Copy)]
pub struct SourceScope {
    pub filter: Option<Source>,
}
impl SourceScope {
    pub fn includes(self, source: Source) -> bool {
        self.filter.is_none_or(|selected| selected == source)
    }
}

pub struct CachedRun {
    pub conn: Store,
    pub stats: IngestStats,
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
        claude: canonical_roots(claude),
        codex: canonical_roots(codex),
        cursor: canonical_roots(cursor),
    }
}

fn canonical_roots(resolved: crate::paths::ResolvedRoots) -> Vec<PathBuf> {
    let mut roots: Vec<_> = resolved
        .roots
        .into_iter()
        .map(|path| path.canonicalize().unwrap_or(path))
        .collect();
    roots.sort();
    roots.dedup();
    roots
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

pub fn maybe_sync_cursor(target_dir: Option<&Path>) -> Result<()> {
    if !has_configured_account()? {
        return Ok(());
    }
    if let Err(error) = sync_active_account(target_dir) {
        eprintln!("warning: Cursor sync failed: {error:#}");
    }
    Ok(())
}

pub fn sync_cursor_if_needed(scope: SourceScope, cursor_dir: Option<&str>) -> Result<()> {
    if scope.includes(Source::Cursor) {
        let target = cursor_sync_target_dir(cursor_dir);
        maybe_sync_cursor(Some(&target))?;
    }
    Ok(())
}

pub fn prepare_run(
    scope: SourceScope,
    roots: &RootDirs,
    rebuild: bool,
    no_cache: bool,
) -> Result<CachedRun> {
    let mut conn = if no_cache {
        Store::ephemeral()?
    } else {
        Store::open(&store_path())?
    };
    if rebuild {
        conn.rebuild()?;
    }
    let stats = run_ingest(
        &mut conn,
        RunIngestOptions {
            claude_roots: roots.claude.clone(),
            codex_roots: roots.codex.clone(),
            cursor_roots: roots.cursor.clone(),
            source: scope.filter,
        },
    )?;
    Ok(CachedRun { conn, stats })
}

pub fn query_scope(scope: SourceScope, roots: &RootDirs) -> crate::store::queries::QueryScope {
    let mut selected = Vec::new();
    if scope.includes(Source::Claude) {
        selected.extend(roots.claude.clone());
    }
    if scope.includes(Source::Codex) {
        selected.extend(roots.codex.clone());
    }
    if scope.includes(Source::Cursor) {
        selected.extend(roots.cursor.clone());
    }
    crate::store::queries::QueryScope {
        source: scope.filter,
        roots: Some(selected),
        ..Default::default()
    }
}

pub fn finish_cached_warnings(
    conn: &Store,
    stats: &IngestStats,
    scope: crate::store::queries::QueryScope,
) -> Result<(HashSet<String>, usize, usize)> {
    let mut unknown = stats.unknown_models.clone();
    for model in crate::store::queries::unknown_models(conn, &scope)? {
        unknown.insert(model);
    }
    Ok((
        unknown,
        crate::store::queries::skipped_records(conn, &scope)?,
        stats.file_errors,
    ))
}
