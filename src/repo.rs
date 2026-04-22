//! Repo identity resolution for events.
//!
//! Given a `project_path` observed during ingest, compute a stable repo key
//! (the absolute canonical path of the nearest `.git` ancestor), a human
//! display name (the basename of the repo root), and an optional remote
//! origin URL read best-effort from `<root>/.git/config`.
//!
//! The resolver is offline, dependency-free, and memoized per run.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Result of resolving a single `project_path`.
///
/// `key` is `None` when no `.git` ancestor was found — the caller is expected
/// to treat that as the `(no-repo)` bucket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoIdentity {
    pub key: Option<String>,
    pub display_name: String,
    pub origin_url: Option<String>,
}

impl RepoIdentity {
    /// Sentinel display name for events that couldn't be resolved to a repo.
    pub const NO_REPO_DISPLAY: &'static str = "(no-repo)";

    fn no_repo(fallback_display: String) -> Self {
        RepoIdentity {
            key: None,
            display_name: fallback_display,
            origin_url: None,
        }
    }
}

/// Memoizing wrapper around [`resolve_path`]. Constructed once per ingest run.
#[derive(Default)]
pub struct Resolver {
    cache: HashMap<String, RepoIdentity>,
}

impl Resolver {
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolve a `project_path`, memoizing the result. Subsequent calls with
    /// the same input return the cached identity without hitting the
    /// filesystem.
    pub fn resolve(&mut self, project_path: &str) -> RepoIdentity {
        if let Some(hit) = self.cache.get(project_path) {
            return hit.clone();
        }
        let id = resolve_path(project_path);
        self.cache.insert(project_path.to_owned(), id.clone());
        id
    }

    /// Iterate resolved identities keyed by their canonical repo key. Only
    /// entries with a resolved `key` are included. Useful for upserting into
    /// the `repos` table after a run.
    pub fn resolved_repos(&self) -> impl Iterator<Item = (&str, &RepoIdentity)> {
        self.cache
            .values()
            .filter_map(|id| id.key.as_deref().map(|k| (k, id)))
    }
}

/// Best-effort basename for a raw `project_path`. Handles both real absolute
/// paths (`/a/b/c` → `c`) and Claude's dash-encoded form (`-a-b-c` → `c`).
pub fn project_basename(s: &str) -> &str {
    if s.starts_with('/') {
        s.rsplit('/').find(|x| !x.is_empty()).unwrap_or(s)
    } else if s.starts_with('-') {
        s.rsplit('-').find(|x| !x.is_empty()).unwrap_or(s)
    } else {
        s
    }
}

/// Resolve without memoization. Prefer [`Resolver::resolve`] in hot paths.
pub fn resolve_path(project_path: &str) -> RepoIdentity {
    let candidates = decode_candidates(project_path);

    for cand in &candidates {
        if let Some(root) = nearest_git_root(cand) {
            return identity_for_root(&root);
        }
    }

    // Nothing matched. Use the original input as the opaque display name.
    let fallback = fallback_display(project_path);
    RepoIdentity::no_repo(fallback)
}

fn fallback_display(project_path: &str) -> String {
    // Prefer the basename of the path when it looks path-ish; otherwise keep
    // the raw value so the user can still tell buckets apart.
    let p = Path::new(project_path);
    p.file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.to_owned())
        .unwrap_or_else(|| project_path.to_owned())
}

/// Produce the ordered list of candidate paths to test against the filesystem.
///
/// A real absolute path is always the first candidate. Claude's dash-encoded
/// folder names additionally expand into a sequence of progressively
/// less-aggressively-decoded variants so that repos with literal hyphens in
/// their name still resolve.
fn decode_candidates(project_path: &str) -> Vec<PathBuf> {
    if project_path.is_empty() {
        return vec![PathBuf::from(project_path)];
    }

    // Absolute paths. Always try them as-is first. If the path doesn't
    // exist, generate progressive fallbacks by gluing trailing `/`-separated
    // segments back into hyphenated ones. This handles the case where
    // Claude's dash-encoded folder was greedily pre-decoded into an
    // absolute path upstream (e.g. `/Users/me/dev/some-repo/sub` got
    // decoded into `/Users/me/dev/some/repo/sub`).
    if let Some(stripped) = project_path.strip_prefix('/') {
        if Path::new(project_path).exists() {
            return vec![PathBuf::from(project_path)];
        }
        let parts: Vec<&str> = stripped.split('/').collect();
        let mut out = Vec::with_capacity(parts.len());
        for join_last_n in 0..parts.len() {
            out.push(decode_with_tail_glue(&parts, join_last_n));
        }
        return out;
    }

    // Claude dash-form looks like `-Users-me-dev-tokctl`.
    if let Some(stripped) = project_path.strip_prefix('-') {
        let parts: Vec<&str> = stripped.split('-').collect();
        let mut out = Vec::new();
        for join_last_n in 0..parts.len() {
            out.push(decode_with_tail_glue(&parts, join_last_n));
        }
        return out;
    }

    vec![PathBuf::from(project_path)]
}

/// Rebuild an absolute path from dash-split parts, treating the final
/// `join_last_n + 1` parts as a single hyphenated segment. `join_last_n = 0`
/// means no gluing (every `-` becomes `/`).
fn decode_with_tail_glue(parts: &[&str], join_last_n: usize) -> PathBuf {
    if parts.is_empty() {
        return PathBuf::from("/");
    }
    let split_at = parts.len().saturating_sub(join_last_n + 1);
    let (head, tail) = parts.split_at(split_at);
    let mut s = String::from("/");
    s.push_str(&head.join("/"));
    if !tail.is_empty() {
        if !head.is_empty() {
            s.push('/');
        }
        s.push_str(&tail.join("-"));
    }
    PathBuf::from(s)
}

/// Walk upward from `start` looking for the first ancestor that contains a
/// `.git` entry (directory, file, or worktree pointer). Returns the canonical
/// absolute path of that ancestor, or `None` if none was found or the path
/// does not exist.
///
/// Git worktrees (where `.git` is a file containing `gitdir: <path>`) are
/// resolved back to the *main* repo root so that, e.g., Codex's per-agent
/// worktrees under `~/.codex/worktrees/<hash>/<repo>/` roll up into the
/// primary checkout rather than appearing as N independent repos.
fn nearest_git_root(start: &Path) -> Option<PathBuf> {
    // Canonicalize up-front if possible; this resolves symlinks once.
    let canonical = fs::canonicalize(start)
        .ok()
        .unwrap_or_else(|| start.to_path_buf());
    if !canonical.exists() {
        return None;
    }

    let mut cur: &Path = &canonical;
    loop {
        let git = cur.join(".git");
        if git.exists() {
            if git.is_file() {
                if let Some(main) = main_repo_from_worktree_pointer(&git) {
                    return Some(main);
                }
            }
            return fs::canonicalize(cur)
                .ok()
                .or_else(|| Some(cur.to_path_buf()));
        }
        match cur.parent() {
            Some(parent) if parent != cur => cur = parent,
            _ => return None,
        }
    }
}

/// If `.git` is a worktree pointer (`gitdir: <path>/.git/worktrees/<name>`),
/// return the canonical path of the main working tree. Otherwise `None`.
fn main_repo_from_worktree_pointer(git_file: &Path) -> Option<PathBuf> {
    let contents = fs::read_to_string(git_file).ok()?;
    let gitdir = contents
        .lines()
        .find_map(|l| l.strip_prefix("gitdir:"))
        .map(|s| s.trim())?;
    // A linked worktree's gitdir looks like `<main>/.git/worktrees/<name>`.
    // Walk up three parents to recover `<main>`.
    let gitdir_path = Path::new(gitdir);
    let main = gitdir_path.parent()?.parent()?.parent()?;
    if main.join(".git").is_dir() {
        fs::canonicalize(main)
            .ok()
            .or_else(|| Some(main.to_path_buf()))
    } else {
        None
    }
}

fn identity_for_root(root: &Path) -> RepoIdentity {
    let key = root.to_string_lossy().into_owned();
    let display_name = root
        .file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.to_owned())
        .unwrap_or_else(|| key.clone());
    let origin_url = read_origin_url(root);
    RepoIdentity {
        key: Some(key),
        display_name,
        origin_url,
    }
}

/// Best-effort read of `<root>/.git/config`, extracting the `url =` line from
/// the `[remote "origin"]` section. Returns `None` if the file is missing,
/// unreadable, or malformed.
fn read_origin_url(root: &Path) -> Option<String> {
    let git_path = root.join(".git");
    let config_path = if git_path.is_file() {
        // Worktree / submodule: `.git` is a pointer file with `gitdir: <path>`.
        let contents = fs::read_to_string(&git_path).ok()?;
        let line = contents
            .lines()
            .find_map(|l| l.strip_prefix("gitdir:"))
            .map(|s| s.trim())?;
        let gitdir = if Path::new(line).is_absolute() {
            PathBuf::from(line)
        } else {
            root.join(line)
        };
        gitdir.join("config")
    } else {
        git_path.join("config")
    };
    let text = fs::read_to_string(&config_path).ok()?;
    parse_origin_url(&text)
}

fn parse_origin_url(config: &str) -> Option<String> {
    let mut in_origin = false;
    for raw in config.lines() {
        let line = raw.trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_origin = line == "[remote \"origin\"]" || line == "[remote 'origin']";
            continue;
        }
        if in_origin {
            if let Some(rest) = line.strip_prefix("url") {
                let rest = rest.trim_start();
                if let Some(val) = rest.strip_prefix('=') {
                    let url = val.trim();
                    if !url.is_empty() {
                        return Some(url.to_owned());
                    }
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn make_repo(dir: &Path) {
        fs::create_dir_all(dir.join(".git")).unwrap();
    }

    #[test]
    fn subdir_resolves_up_to_root() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("myrepo");
        let sub = root.join("src/deep");
        fs::create_dir_all(&sub).unwrap();
        make_repo(&root);

        let id = resolve_path(sub.to_str().unwrap());
        let canon = fs::canonicalize(&root).unwrap();
        assert_eq!(id.key.as_deref(), Some(canon.to_str().unwrap()));
        assert_eq!(id.display_name, "myrepo");
    }

    #[test]
    fn repo_root_resolves_to_itself() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("r");
        fs::create_dir_all(&root).unwrap();
        make_repo(&root);
        let id = resolve_path(root.to_str().unwrap());
        let canon = fs::canonicalize(&root).unwrap();
        assert_eq!(id.key.as_deref(), Some(canon.to_str().unwrap()));
    }

    #[test]
    fn no_git_returns_no_key() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("loose");
        fs::create_dir_all(&p).unwrap();
        let id = resolve_path(p.to_str().unwrap());
        assert!(id.key.is_none());
        assert_eq!(id.display_name, "loose");
    }

    #[test]
    fn claude_dash_form_decodes_and_resolves() {
        // Build a real repo, then feed its dash-encoded form to the resolver.
        let tmp = tempfile::tempdir().unwrap();
        let root_str = tmp.path().join("dev/tokctl").to_string_lossy().into_owned();
        let root = PathBuf::from(&root_str);
        fs::create_dir_all(&root).unwrap();
        make_repo(&root);

        let claude_form = root_str.replace('/', "-");
        let id = resolve_path(&claude_form);
        let canon = fs::canonicalize(&root).unwrap();
        assert_eq!(id.key.as_deref(), Some(canon.to_str().unwrap()));
    }

    #[test]
    fn claude_dash_form_with_literal_hyphen_falls_through() {
        // Repo path contains a literal hyphen: "some-repo".
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("some-repo");
        fs::create_dir_all(&root).unwrap();
        make_repo(&root);

        // Claude would encode `/tmp.../some-repo` by replacing every `/` with
        // `-`, producing `-tmp-...-some-repo`. The first decode candidate
        // splits the hyphen and fails; the progressive retry should recover.
        let base = tmp.path().to_string_lossy().into_owned();
        let claude_form = format!("{}-some-repo", base.replace('/', "-"));

        let id = resolve_path(&claude_form);
        let canon = fs::canonicalize(&root).unwrap();
        assert_eq!(id.key.as_deref(), Some(canon.to_str().unwrap()));
        assert_eq!(id.display_name, "some-repo");
    }

    #[test]
    fn unresolvable_claude_path_is_opaque_bucket() {
        let id = resolve_path("-definitely-not-a-real-path-xyzzy");
        assert!(id.key.is_none());
        // The display falls back to the basename of the raw input.
        assert!(!id.display_name.is_empty());
    }

    #[test]
    fn resolver_memoizes() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("memo");
        fs::create_dir_all(root.join("a")).unwrap();
        make_repo(&root);

        let mut r = Resolver::new();
        let path = root.join("a").to_string_lossy().into_owned();
        let a = r.resolve(&path);
        let b = r.resolve(&path);
        assert_eq!(a, b);
        assert_eq!(r.cache.len(), 1);
    }

    #[test]
    fn origin_url_parsed_when_present() {
        let config = r#"
[core]
    repositoryformatversion = 0
[remote "origin"]
    url = git@github.com:me/tokctl.git
    fetch = +refs/heads/*:refs/remotes/origin/*
[branch "main"]
    remote = origin
"#;
        assert_eq!(
            parse_origin_url(config).as_deref(),
            Some("git@github.com:me/tokctl.git")
        );
    }

    #[test]
    fn origin_url_missing_returns_none() {
        let config = "[core]\nbare = false\n";
        assert!(parse_origin_url(config).is_none());
    }

    #[test]
    fn absolute_path_with_split_hyphen_falls_back() {
        // Simulates discovery.rs greedily decoding
        // `-Users-me-dev-some-repo` into `/tmp…/Users/me/dev/some/repo`
        // while the real repo on disk lives at `.../dev/some-repo`.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("dev/some-repo");
        fs::create_dir_all(&root).unwrap();
        make_repo(&root);

        let bad = tmp.path().join("dev/some/repo"); // doesn't exist
        let id = resolve_path(bad.to_str().unwrap());
        let canon = fs::canonicalize(&root).unwrap();
        assert_eq!(id.key.as_deref(), Some(canon.to_str().unwrap()));
        assert_eq!(id.display_name, "some-repo");
    }

    #[test]
    fn worktree_pointer_resolves_to_main_repo() {
        // Simulate the Codex layout:
        //   <main>/.git/                                 (directory)
        //   <main>/.git/worktrees/wt1/                   (where the pointer aims)
        //   <wt>/.git                                    (file with `gitdir: …`)
        let tmp = tempfile::tempdir().unwrap();
        let main = tmp.path().join("dbt-transform");
        let wt_gitdir = main.join(".git/worktrees/wt1");
        fs::create_dir_all(&wt_gitdir).unwrap();
        // .git is a directory on the main — already created above as parent
        assert!(main.join(".git").is_dir());

        let wt = tmp.path().join("worktrees/abc/dbt-transform");
        fs::create_dir_all(&wt).unwrap();
        fs::write(
            wt.join(".git"),
            format!("gitdir: {}\n", wt_gitdir.display()),
        )
        .unwrap();

        let id = resolve_path(wt.to_str().unwrap());
        let canon_main = fs::canonicalize(&main).unwrap();
        assert_eq!(id.key.as_deref(), Some(canon_main.to_str().unwrap()));
        assert_eq!(id.display_name, "dbt-transform");
    }

    #[test]
    fn resolver_reads_origin_from_real_config() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("r");
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(
            root.join(".git/config"),
            "[remote \"origin\"]\n    url = https://example.com/r.git\n",
        )
        .unwrap();

        let id = resolve_path(root.to_str().unwrap());
        assert_eq!(id.origin_url.as_deref(), Some("https://example.com/r.git"));
    }
}
