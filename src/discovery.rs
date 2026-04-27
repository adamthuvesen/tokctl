use crate::types::Source;
use std::collections::{HashMap, HashSet};
use std::fs::Metadata;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct DiscoveredFile {
    pub path: PathBuf,
    pub source: Source,
    pub project: Option<String>,
    pub size: u64,
    pub mtime_ns: i64,
}

#[derive(Debug, Default)]
pub struct Discovery {
    pub files: Vec<DiscoveredFile>,
    /// Paths whose containing directory looks unchanged; trust the manifest.
    pub unchanged_paths: HashSet<PathBuf>,
}

/// A manifest row has at least these fields from the planner's perspective.
/// The store module defines the full struct.
pub trait ManifestLike {
    fn mtime_ns(&self) -> i64;
}

/// Convert a SystemTime (typically from std::fs::Metadata::modified) to
/// nanoseconds since the Unix epoch. Negative values (pre-1970) clamp to 0.
pub fn mtime_ns(metadata: &Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

fn system_time_ns(t: SystemTime) -> i64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

/// Decode Claude's project-slug folder naming. Claude Code encodes a cwd like
/// `/Users/foo/dev/repo` as `-Users-foo-dev-repo`. This decode is lossy for
/// paths containing literal dashes, but handles the common case.
fn decode_claude_slug(folder_name: &str) -> Option<String> {
    folder_name
        .strip_prefix('-')
        .map(|rest| format!("/{}", rest.replace('-', "/")))
}

#[derive(Debug, Clone, Copy)]
pub struct DiscoverOpts {
    pub safety_window_ms: i64,
    pub now_ms: i64,
}

impl DiscoverOpts {
    pub fn safety_threshold_ns(&self) -> i64 {
        (self.now_ms - self.safety_window_ms).saturating_mul(1_000_000)
    }
}

/// Index a manifest map by the parent directory of each path.
pub fn index_manifest_by_parent<M: ManifestLike>(
    manifest: &HashMap<PathBuf, M>,
) -> HashMap<PathBuf, ParentEntry> {
    let mut by_parent: HashMap<PathBuf, ParentEntry> = HashMap::new();
    for (p, row) in manifest {
        let parent = p
            .parent()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(""));
        let entry = by_parent.entry(parent).or_default();
        entry.paths.push(p.clone());
        let m = row.mtime_ns();
        if m > entry.max_mtime_ns {
            entry.max_mtime_ns = m;
        }
    }
    by_parent
}

#[derive(Debug, Default)]
pub struct ParentEntry {
    pub paths: Vec<PathBuf>,
    pub max_mtime_ns: i64,
}

fn walk_with_short_circuit<M: ManifestLike>(
    dir: &Path,
    source: Source,
    project: Option<String>,
    manifest: &HashMap<PathBuf, M>,
    index: &HashMap<PathBuf, ParentEntry>,
    safety_threshold_ns: i64,
    out: &mut Discovery,
) {
    let Ok(md) = std::fs::metadata(dir) else {
        return;
    };
    if !md.is_dir() {
        return;
    }
    let dir_mtime_ns = md.modified().ok().map(system_time_ns).unwrap_or(0);

    let dir_pb = dir.to_path_buf();
    if let Some(entry) = index.get(&dir_pb) {
        if dir_mtime_ns <= entry.max_mtime_ns {
            // Dir hasn't advanced past any manifest entry. Short-circuit unless
            // any manifest file under here is "recent" (within safety window).
            let has_recent = entry.paths.iter().any(|p| {
                manifest
                    .get(p)
                    .map(|r| r.mtime_ns() >= safety_threshold_ns)
                    .unwrap_or(false)
            });
            if !has_recent {
                for p in &entry.paths {
                    out.unchanged_paths.insert(p.clone());
                }
                return;
            }
        }
    }

    // Full walk
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let path = e.path();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() {
            walk_with_short_circuit(
                &path,
                source,
                project.clone(),
                manifest,
                index,
                safety_threshold_ns,
                out,
            );
        } else if ft.is_file() && matches_discovered_file(&path, source) {
            let Ok(fmd) = e.metadata() else { continue };
            out.files.push(DiscoveredFile {
                path,
                source,
                project: project.clone(),
                size: fmd.len(),
                mtime_ns: mtime_ns(&fmd),
            });
        }
    }
}

fn matches_discovered_file(path: &Path, source: Source) -> bool {
    match source {
        Source::Claude | Source::Codex => path
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s == "jsonl"),
        Source::Cursor => is_cursor_usage_csv(path),
    }
}

fn is_cursor_usage_csv(path: &Path) -> bool {
    if !path
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("csv"))
    {
        return false;
    }
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    let mut reader = std::io::BufReader::new(file);
    let mut header = String::new();
    if reader.read_line(&mut header).is_err() {
        return false;
    }
    let lower = header.to_ascii_lowercase();
    lower.contains("date") && lower.contains("model")
}

/// Discover Claude session files at `<root>/<project-slug>/<session>.jsonl`.
pub fn discover_claude<M: ManifestLike>(
    roots: &[PathBuf],
    manifest: &HashMap<PathBuf, M>,
    opts: DiscoverOpts,
) -> Discovery {
    let index = index_manifest_by_parent(manifest);
    let threshold = opts.safety_threshold_ns();
    let mut out = Discovery::default();

    for root in roots {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for e in entries.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            if !ft.is_dir() {
                continue;
            }
            let project_dir = e.path();
            let folder_name = e.file_name().to_string_lossy().into_owned();
            let project = decode_claude_slug(&folder_name);
            walk_with_short_circuit(
                &project_dir,
                Source::Claude,
                project,
                manifest,
                &index,
                threshold,
                &mut out,
            );
        }
    }
    out
}

/// Discover Codex session files at any depth below `root`.
pub fn discover_codex<M: ManifestLike>(
    roots: &[PathBuf],
    manifest: &HashMap<PathBuf, M>,
    opts: DiscoverOpts,
) -> Discovery {
    let index = index_manifest_by_parent(manifest);
    let threshold = opts.safety_threshold_ns();
    let mut out = Discovery::default();
    for root in roots {
        walk_with_short_circuit(
            root,
            Source::Codex,
            None,
            manifest,
            &index,
            threshold,
            &mut out,
        );
    }
    out
}

pub fn discover_cursor<M: ManifestLike>(
    roots: &[PathBuf],
    manifest: &HashMap<PathBuf, M>,
    opts: DiscoverOpts,
) -> Discovery {
    let index = index_manifest_by_parent(manifest);
    let threshold = opts.safety_threshold_ns();
    let mut out = Discovery::default();
    for root in roots {
        walk_with_short_circuit(
            root,
            Source::Cursor,
            None,
            manifest,
            &index,
            threshold,
            &mut out,
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::fs;
    use tempfile::tempdir;

    struct FakeRow(i64);
    impl ManifestLike for FakeRow {
        fn mtime_ns(&self) -> i64 {
            self.0
        }
    }

    #[test]
    fn discover_claude_finds_files_at_correct_depth() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let proj = root.join("-Users-dev-repo");
        fs::create_dir_all(&proj).unwrap();
        fs::write(proj.join("abc.jsonl"), b"hello").unwrap();
        fs::write(proj.join("not-jsonl.txt"), b"x").unwrap();

        let manifest: HashMap<PathBuf, FakeRow> = HashMap::new();
        let d = discover_claude(
            &[root.to_path_buf()],
            &manifest,
            DiscoverOpts {
                safety_window_ms: 60_000,
                now_ms: 0,
            },
        );
        assert_eq!(d.files.len(), 1);
        assert_eq!(d.files[0].source, Source::Claude);
        assert_eq!(d.files[0].project.as_deref(), Some("/Users/dev/repo"));
    }

    #[test]
    fn missing_root_silently_skipped() {
        let manifest: HashMap<PathBuf, FakeRow> = HashMap::new();
        let d = discover_claude(
            &[PathBuf::from("/definitely/not/here")],
            &manifest,
            DiscoverOpts {
                safety_window_ms: 60_000,
                now_ms: 0,
            },
        );
        assert!(d.files.is_empty());
    }

    #[test]
    fn discover_codex_walks_any_depth() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let deep = root.join("2026").join("04");
        fs::create_dir_all(&deep).unwrap();
        fs::write(deep.join("sess.jsonl"), b"x").unwrap();

        let manifest: HashMap<PathBuf, FakeRow> = HashMap::new();
        let d = discover_codex(
            &[root.to_path_buf()],
            &manifest,
            DiscoverOpts {
                safety_window_ms: 60_000,
                now_ms: 0,
            },
        );
        assert_eq!(d.files.len(), 1);
        assert_eq!(d.files[0].source, Source::Codex);
        assert!(d.files[0].project.is_none());
    }

    #[test]
    fn discover_cursor_finds_usage_csv_and_ignores_other_csvs() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let deep = root.join("accounts").join("work");
        fs::create_dir_all(&deep).unwrap();
        fs::write(
            deep.join("usage.csv"),
            b"Date,Kind,Model,Max Mode,Input (w/ Cache Write),Input (w/o Cache Write),Cache Read,Output Tokens,Total Tokens,Cost\n",
        )
        .unwrap();
        fs::write(deep.join("other.csv"), b"name,value\nfoo,1\n").unwrap();

        let manifest: HashMap<PathBuf, FakeRow> = HashMap::new();
        let d = discover_cursor(
            &[root.to_path_buf()],
            &manifest,
            DiscoverOpts {
                safety_window_ms: 60_000,
                now_ms: 0,
            },
        );
        assert_eq!(d.files.len(), 1);
        assert_eq!(d.files[0].source, Source::Cursor);
        assert!(d.files[0].project.is_none());
    }
}
