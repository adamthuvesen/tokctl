use crate::discovery::{DiscoveredFile, Discovery};
use crate::store::writes::FileManifestRow;
use crate::types::Source;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

#[derive(Debug, Default)]
pub struct IngestPlan {
    pub to_skip: Vec<PathBuf>,
    pub to_tail: Vec<TailItem>,
    pub to_full_parse: Vec<DiscoveredFile>,
    pub to_purge: Vec<PathBuf>,
}

#[derive(Debug)]
pub struct TailItem {
    pub file: DiscoveredFile,
    pub from_offset: u64,
}

pub struct PlanInput<'a> {
    pub manifest: &'a HashMap<PathBuf, FileManifestRow>,
    pub discovery: &'a Discovery,
    pub safety_window_ms: i64,
    pub now_ms: i64,
}

pub fn plan_ingest(input: PlanInput<'_>) -> IngestPlan {
    let safety_threshold_ns = (input.now_ms - input.safety_window_ms).saturating_mul(1_000_000);

    let mut plan = IngestPlan::default();
    let mut touched: HashSet<PathBuf> = HashSet::new();

    for d in &input.discovery.files {
        touched.insert(d.path.clone());
        let Some(row) = input.manifest.get(&d.path) else {
            plan.to_full_parse.push(d.clone());
            continue;
        };

        // Shrunk/rotated file → full parse
        if d.size < row.last_offset {
            plan.to_full_parse.push(d.clone());
            continue;
        }

        // Recently modified file → full parse (don't risk partial tail)
        if d.mtime_ns >= safety_threshold_ns {
            // Even if it looks unchanged, re-parse inside safety window to be safe.
            // But if truly identical (same size + mtime) we can still skip.
            if d.size == row.size && d.mtime_ns == row.mtime_ns {
                plan.to_skip.push(d.path.clone());
            } else {
                plan.to_full_parse.push(d.clone());
            }
            continue;
        }

        // Unchanged
        if d.size == row.size && d.mtime_ns == row.mtime_ns {
            plan.to_skip.push(d.path.clone());
            continue;
        }

        if d.source == Source::Cursor {
            plan.to_full_parse.push(d.clone());
            continue;
        }

        // Grown, safe to tail
        if d.size > row.last_offset {
            plan.to_tail.push(TailItem {
                file: d.clone(),
                from_offset: row.last_offset,
            });
        } else {
            plan.to_skip.push(d.path.clone());
        }
    }

    // Files in unchanged directories → skip (manifest is trusted)
    for p in &input.discovery.unchanged_paths {
        if touched.contains(p) {
            continue;
        }
        if input.manifest.contains_key(p) {
            plan.to_skip.push(p.clone());
            touched.insert(p.clone());
        }
    }

    // Anything in manifest but not discovered → purge
    for p in input.manifest.keys() {
        if !touched.contains(p) {
            plan.to_purge.push(p.clone());
        }
    }

    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Source;
    use std::path::PathBuf;

    fn mk_file(path: &str, size: u64, mtime: i64) -> DiscoveredFile {
        DiscoveredFile {
            path: PathBuf::from(path),
            source: Source::Claude,
            project: None,
            size,
            mtime_ns: mtime,
        }
    }

    fn mk_row(path: &str, size: u64, mtime: i64, last_offset: u64) -> FileManifestRow {
        FileManifestRow {
            path: PathBuf::from(path),
            source: Source::Claude,
            project: None,
            size,
            mtime_ns: mtime,
            last_offset,
            n_events: 0,
            session_id: None,
            model: None,
        }
    }

    fn far_past_ns() -> i64 {
        // 2020-01-01 in nanoseconds — well outside any test's safety window.
        1_577_836_800_000_000_000
    }

    fn base_now_ms() -> i64 {
        // 2026-04-22 roughly
        1_777_000_000_000
    }

    #[test]
    fn new_file_goes_to_full_parse() {
        let files = vec![mk_file("/x.jsonl", 100, far_past_ns())];
        let discovery = Discovery {
            files,
            unchanged_paths: Default::default(),
        };
        let manifest = HashMap::new();
        let plan = plan_ingest(PlanInput {
            manifest: &manifest,
            discovery: &discovery,
            safety_window_ms: 3_600_000,
            now_ms: base_now_ms(),
        });
        assert_eq!(plan.to_full_parse.len(), 1);
        assert!(plan.to_tail.is_empty());
        assert!(plan.to_skip.is_empty());
    }

    #[test]
    fn unchanged_file_goes_to_skip() {
        let mt = far_past_ns();
        let files = vec![mk_file("/x.jsonl", 500, mt)];
        let discovery = Discovery {
            files,
            unchanged_paths: Default::default(),
        };
        let mut manifest = HashMap::new();
        manifest.insert(PathBuf::from("/x.jsonl"), mk_row("/x.jsonl", 500, mt, 500));
        let plan = plan_ingest(PlanInput {
            manifest: &manifest,
            discovery: &discovery,
            safety_window_ms: 3_600_000,
            now_ms: base_now_ms(),
        });
        assert_eq!(plan.to_skip.len(), 1);
        assert!(plan.to_full_parse.is_empty());
    }

    #[test]
    fn grown_file_goes_to_tail() {
        let mt = far_past_ns();
        let files = vec![mk_file("/x.jsonl", 800, mt + 1)];
        let discovery = Discovery {
            files,
            unchanged_paths: Default::default(),
        };
        let mut manifest = HashMap::new();
        manifest.insert(PathBuf::from("/x.jsonl"), mk_row("/x.jsonl", 500, mt, 500));
        let plan = plan_ingest(PlanInput {
            manifest: &manifest,
            discovery: &discovery,
            safety_window_ms: 3_600_000,
            now_ms: base_now_ms(),
        });
        assert_eq!(plan.to_tail.len(), 1);
        assert_eq!(plan.to_tail[0].from_offset, 500);
    }

    #[test]
    fn shrunk_file_goes_to_full_parse() {
        let mt = far_past_ns();
        let files = vec![mk_file("/x.jsonl", 100, mt + 1)];
        let discovery = Discovery {
            files,
            unchanged_paths: Default::default(),
        };
        let mut manifest = HashMap::new();
        manifest.insert(PathBuf::from("/x.jsonl"), mk_row("/x.jsonl", 500, mt, 500));
        let plan = plan_ingest(PlanInput {
            manifest: &manifest,
            discovery: &discovery,
            safety_window_ms: 3_600_000,
            now_ms: base_now_ms(),
        });
        assert_eq!(plan.to_full_parse.len(), 1);
    }

    #[test]
    fn disappeared_file_goes_to_purge() {
        let discovery = Discovery::default();
        let mut manifest = HashMap::new();
        manifest.insert(
            PathBuf::from("/gone.jsonl"),
            mk_row("/gone.jsonl", 100, 1, 100),
        );
        let plan = plan_ingest(PlanInput {
            manifest: &manifest,
            discovery: &discovery,
            safety_window_ms: 3_600_000,
            now_ms: base_now_ms(),
        });
        assert_eq!(plan.to_purge, vec![PathBuf::from("/gone.jsonl")]);
    }

    #[test]
    fn recent_file_goes_to_full_parse_even_if_grown() {
        // mtime within safety window
        let now_ms = base_now_ms();
        let recent_ns = (now_ms - 1_000).saturating_mul(1_000_000);
        let files = vec![mk_file("/x.jsonl", 800, recent_ns)];
        let discovery = Discovery {
            files,
            unchanged_paths: Default::default(),
        };
        let mut manifest = HashMap::new();
        manifest.insert(
            PathBuf::from("/x.jsonl"),
            mk_row("/x.jsonl", 500, recent_ns - 1000, 500),
        );
        let plan = plan_ingest(PlanInput {
            manifest: &manifest,
            discovery: &discovery,
            safety_window_ms: 3_600_000,
            now_ms,
        });
        assert_eq!(plan.to_full_parse.len(), 1);
        assert!(plan.to_tail.is_empty());
    }

    #[test]
    fn unchanged_directory_skips_file_without_stat() {
        let discovery = Discovery {
            files: vec![],
            unchanged_paths: {
                let mut s = HashSet::new();
                s.insert(PathBuf::from("/dir/x.jsonl"));
                s
            },
        };
        let mut manifest = HashMap::new();
        manifest.insert(
            PathBuf::from("/dir/x.jsonl"),
            mk_row("/dir/x.jsonl", 100, far_past_ns(), 100),
        );
        let plan = plan_ingest(PlanInput {
            manifest: &manifest,
            discovery: &discovery,
            safety_window_ms: 3_600_000,
            now_ms: base_now_ms(),
        });
        assert_eq!(plan.to_skip, vec![PathBuf::from("/dir/x.jsonl")]);
        assert!(plan.to_purge.is_empty());
    }

    #[test]
    fn changed_cursor_file_goes_to_full_parse_not_tail() {
        let mt = far_past_ns();
        let discovery = Discovery {
            files: vec![DiscoveredFile {
                path: PathBuf::from("/cursor/usage.csv"),
                source: Source::Cursor,
                project: None,
                size: 800,
                mtime_ns: mt + 1,
            }],
            unchanged_paths: Default::default(),
        };
        let mut manifest = HashMap::new();
        manifest.insert(
            PathBuf::from("/cursor/usage.csv"),
            FileManifestRow {
                path: PathBuf::from("/cursor/usage.csv"),
                source: Source::Cursor,
                project: None,
                size: 500,
                mtime_ns: mt,
                last_offset: 500,
                n_events: 0,
                session_id: None,
                model: None,
            },
        );
        let plan = plan_ingest(PlanInput {
            manifest: &manifest,
            discovery: &discovery,
            safety_window_ms: 3_600_000,
            now_ms: base_now_ms(),
        });
        assert_eq!(plan.to_full_parse.len(), 1);
        assert!(plan.to_tail.is_empty());
    }
}
