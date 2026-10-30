use crate::types::Source;
use std::collections::HashSet;
use std::fs::Metadata;
use std::path::PathBuf;
use std::time::UNIX_EPOCH;

#[derive(Debug, Clone)]
pub struct DiscoveredFile {
    pub path: PathBuf,
    pub source: Source,
    pub project: Option<String>,
    pub size: u64,
    pub mtime_ns: i64,
    pub file_identity: Option<String>,
    pub ctime_ns: Option<i64>,
}

#[derive(Debug, Default)]
pub struct Discovery {
    pub files: Vec<DiscoveredFile>,
    pub complete_roots: Vec<(Source, PathBuf)>,
    pub errors: Vec<DiscoveryError>,
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

pub fn file_identity(metadata: &Metadata) -> Option<String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some(format!("{}:{}", metadata.dev(), metadata.ino()))
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        None
    }
}

pub fn ctime_ns(metadata: &Metadata) -> Option<i64> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        metadata
            .ctime()
            .checked_mul(1_000_000_000)?
            .checked_add(metadata.ctime_nsec())
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        None
    }
}

impl DiscoveredFile {
    pub fn matches_metadata(&self, metadata: &Metadata) -> bool {
        self.size == metadata.len()
            && self.mtime_ns == mtime_ns(metadata)
            && self.file_identity == file_identity(metadata)
            && self.ctime_ns == ctime_ns(metadata)
    }
}

/// Decode Claude's project-slug folder naming. Claude Code encodes a cwd like
/// `/Users/foo/dev/repo` as `-Users-foo-dev-repo`. This decode is lossy for
/// paths containing literal dashes, but handles the common case.
fn decode_claude_slug(folder_name: &str) -> Option<String> {
    folder_name
        .strip_prefix('-')
        .map(|rest| format!("/{}", rest.replace('-', "/")))
}

#[derive(Debug)]
pub struct DiscoveryError {
    pub path: PathBuf,
    pub reason: String,
}

fn discover(roots: &[PathBuf], source: Source) -> Discovery {
    let mut out = Discovery::default();
    let mut seen = HashSet::new();
    for requested in roots {
        let root = match requested.canonicalize() {
            Ok(root) => root,
            Err(error) => {
                out.errors.push(DiscoveryError {
                    path: requested.clone(),
                    reason: error.to_string(),
                });
                continue;
            }
        };
        if !root.is_dir() {
            out.errors.push(DiscoveryError {
                path: root,
                reason: "root is not a directory".into(),
            });
            continue;
        }
        if !seen.insert(root.clone()) {
            continue;
        }
        let mut complete = true;
        for entry in walkdir::WalkDir::new(&root).follow_links(false) {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    complete = false;
                    out.errors.push(DiscoveryError {
                        path: error.path().unwrap_or(&root).to_path_buf(),
                        reason: error
                            .io_error()
                            .map(ToString::to_string)
                            .unwrap_or_else(|| "directory walk failed".into()),
                    });
                    continue;
                }
            };
            let path = entry.path();
            if !entry.file_type().is_file() {
                continue;
            }
            let wanted =
                path.extension()
                    .and_then(|s| s.to_str())
                    .is_some_and(|ext| match source {
                        Source::Cursor => ext.eq_ignore_ascii_case("csv"),
                        _ => ext == "jsonl",
                    });
            if !wanted {
                continue;
            }
            match entry.metadata() {
                Ok(metadata) => {
                    let project = if source == Source::Claude {
                        path.strip_prefix(&root)
                            .ok()
                            .and_then(|p| p.components().next())
                            .and_then(|c| c.as_os_str().to_str())
                            .and_then(decode_claude_slug)
                    } else {
                        None
                    };
                    out.files.push(DiscoveredFile {
                        path: path.to_path_buf(),
                        source,
                        project,
                        size: metadata.len(),
                        mtime_ns: mtime_ns(&metadata),
                        file_identity: file_identity(&metadata),
                        ctime_ns: ctime_ns(&metadata),
                    });
                }
                Err(error) => {
                    complete = false;
                    out.errors.push(DiscoveryError {
                        path: path.to_path_buf(),
                        reason: error.to_string(),
                    });
                }
            }
        }
        if complete {
            out.complete_roots.push((source, root));
        }
    }
    out.files.sort_by(|left, right| left.path.cmp(&right.path));
    out.files.dedup_by(|left, right| left.path == right.path);
    out
}

pub fn discover_claude(roots: &[PathBuf]) -> Discovery {
    discover(roots, Source::Claude)
}
pub fn discover_codex(roots: &[PathBuf]) -> Discovery {
    discover(roots, Source::Codex)
}
pub fn discover_cursor(roots: &[PathBuf]) -> Discovery {
    discover(roots, Source::Cursor)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[test]
    fn claude_decodes_project_and_walks_nested_directories() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("-Users-dev-repo/subagents");
        fs::create_dir_all(&project).unwrap();
        fs::write(project.join("a.jsonl"), "{}").unwrap();
        let result = discover_claude(&[dir.path().to_path_buf()]);
        assert_eq!(result.files.len(), 1);
        assert_eq!(result.files[0].project.as_deref(), Some("/Users/dev/repo"));
        assert_eq!(result.complete_roots.len(), 1);
    }
    #[test]
    fn missing_root_is_an_incomplete_scan() {
        let dir = tempfile::tempdir().unwrap();
        let result = discover_codex(&[dir.path().join("absent")]);
        assert!(result.complete_roots.is_empty());
        assert_eq!(result.errors.len(), 1);
    }
    #[test]
    fn cursor_csv_candidates_reach_the_parser_for_header_validation() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("malformed.csv"), "Date,Model").unwrap();
        fs::write(dir.path().join("other.txt"), "Date,Model").unwrap();
        assert_eq!(discover_cursor(&[dir.path().to_path_buf()]).files.len(), 1);
    }
}
