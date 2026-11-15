//! Shared CLI orchestration: root resolution, optional Cursor sync, cache vs no-cache ingest.

use anyhow::{Context, Result};
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
) -> Result<RootDirs> {
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
    let cursor = cursor_roots(cursor_dir, env("TOKCTL_CURSOR_DIR").as_deref());
    Ok(RootDirs {
        claude: requested_roots(claude)?,
        codex: requested_roots(codex)?,
        cursor: requested_roots(cursor)?,
    })
}

fn requested_roots(resolved: crate::paths::ResolvedRoots) -> Result<Vec<PathBuf>> {
    let mut roots: Vec<_> = resolved
        .roots
        .into_iter()
        .map(|path| std::path::absolute(path).context("resolving source root"))
        .collect::<Result<_>>()?;
    roots.sort();
    roots.dedup();
    Ok(roots)
}

fn cursor_roots(
    cursor_dir: Option<&str>,
    configured_dir: Option<&str>,
) -> crate::paths::ResolvedRoots {
    resolve_roots(ResolveInput {
        flag: cursor_dir,
        tokctl_env: configured_dir,
        tool_env: None,
        tool_env_suffix: None,
        defaults: default_cursor_roots(),
    })
}

pub fn cursor_sync_target_dir(cursor_dir: Option<&str>) -> PathBuf {
    cursor_roots(
        cursor_dir,
        std::env::var("TOKCTL_CURSOR_DIR").ok().as_deref(),
    )
    .roots
    .into_iter()
    .next()
    .unwrap_or_else(cursor_sync_cache_dir)
}

pub fn maybe_sync_cursor(target_dir: Option<&Path>) -> Result<Option<String>> {
    match has_configured_account() {
        Ok(true) => {}
        Ok(false) => return Ok(None),
        Err(error) => {
            return Ok(Some(format!(
                "Cursor unavailable: {error:#}. Reporting the other sources."
            )));
        }
    }
    Ok(sync_active_account(target_dir)
        .err()
        .map(|error| format!("Cursor sync failed. Using cached exports: {error:#}")))
}

pub fn sync_cursor_if_needed(
    scope: SourceScope,
    cursor_dir: Option<&str>,
) -> Result<Option<String>> {
    if scope.includes(Source::Cursor) {
        let target = cursor_sync_target_dir(cursor_dir);
        return maybe_sync_cursor(Some(&target));
    }
    Ok(None)
}

pub fn prepare_run(
    scope: SourceScope,
    roots: &mut RootDirs,
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
    if scope.includes(Source::Claude) {
        roots.claude = conn.resolve_source_roots(Source::Claude, &roots.claude)?;
    }
    if scope.includes(Source::Codex) {
        roots.codex = conn.resolve_source_roots(Source::Codex, &roots.codex)?;
    }
    if scope.includes(Source::Cursor) {
        roots.cursor = conn.resolve_source_roots(Source::Cursor, &roots.cursor)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_cursor_sync_target_is_the_selected_report_root() {
        let directory = tempfile::tempdir().unwrap();
        let configured = directory.path().join("selected-cursor");
        let configured = configured.to_str().unwrap();
        let roots = resolve_root_dirs(None, None, Some(configured)).unwrap();
        assert_eq!(cursor_sync_target_dir(Some(configured)), roots.cursor[0]);
        assert!(!roots.cursor[0].exists());
        let configured_roots = cursor_roots(None, Some(configured));
        assert_eq!(configured_roots.roots, roots.cursor);
    }

    #[cfg(unix)]
    #[test]
    fn requested_roots_preserve_symlink_identity_until_store_resolution() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("source");
        let alias = directory.path().join("alias");
        std::fs::create_dir(&target).unwrap();
        std::os::unix::fs::symlink(&target, &alias).unwrap();
        let roots = resolve_root_dirs(None, Some(alias.to_str().unwrap()), None).unwrap();
        assert_eq!(roots.codex, vec![alias]);
    }
}
