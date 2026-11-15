use crate::discovery::{DiscoveredFile, Discovery};
use crate::store::FileManifestRow;
use crate::types::Source;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

#[derive(Debug, Default)]
pub struct IngestPlan {
    pub to_skip: Vec<PathBuf>,
    pub to_full_parse: Vec<DiscoveredFile>,
    pub to_purge: Vec<PathBuf>,
    pub to_mark_missing: Vec<PathBuf>,
}

pub struct PlanInput<'a> {
    pub manifest: &'a HashMap<PathBuf, FileManifestRow>,
    pub discovery: &'a Discovery,
}

pub fn plan_ingest(input: PlanInput<'_>) -> IngestPlan {
    let mut plan = IngestPlan::default();
    let mut touched: HashSet<PathBuf> = HashSet::new();
    for file in &input.discovery.files {
        if !touched.insert(file.path.clone()) {
            continue;
        }
        match input.manifest.get(&file.path) {
            Some(row)
                if row.missing_since.is_none()
                    && row.size == file.size
                    && row.mtime_ns == file.mtime_ns
                    && row.file_identity.is_some()
                    && row.file_identity == file.file_identity
                    && row.ctime_ns.is_some()
                    && row.ctime_ns == file.ctime_ns =>
            {
                plan.to_skip.push(file.path.clone())
            }
            _ => plan.to_full_parse.push(file.clone()),
        }
    }

    // Only complete scans establish that an undiscovered file was deleted.
    for (p, row) in input.manifest {
        if !touched.contains(p)
            && input
                .discovery
                .complete_roots
                .iter()
                .any(|(source, root)| row.source == *source && p.starts_with(root))
        {
            if row.source == Source::Cursor {
                plan.to_purge.push(p.clone());
            } else if row.missing_since.is_none() {
                plan.to_mark_missing.push(p.clone());
            }
        }
    }

    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn mk_file(path: &str, size: u64, mtime: i64) -> DiscoveredFile {
        mk_file_with_source(path, Source::Claude, size, mtime)
    }

    fn mk_file_with_source(path: &str, source: Source, size: u64, mtime: i64) -> DiscoveredFile {
        DiscoveredFile {
            path: PathBuf::from(path),
            source,
            project: None,
            size,
            mtime_ns: mtime,
            file_identity: Some("test-file".into()),
            ctime_ns: Some(1),
        }
    }

    fn mk_row(path: &str, size: u64, mtime: i64, last_offset: u64) -> FileManifestRow {
        FileManifestRow {
            path: PathBuf::from(path),
            source: Source::Claude,
            project: None,
            size,
            mtime_ns: mtime,
            file_identity: Some("test-file".into()),
            ctime_ns: Some(1),
            last_offset,
            n_events: 0,
            session_id: None,
            model: None,
            missing_since: None,
        }
    }

    fn far_past_ns() -> i64 {
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
            ..Default::default()
        };
        let manifest = HashMap::new();
        let plan = plan_ingest(PlanInput {
            manifest: &manifest,
            discovery: &discovery,
        });
        assert_eq!(plan.to_full_parse.len(), 1);
        assert!(plan.to_skip.is_empty());
    }

    #[test]
    fn unchanged_file_goes_to_skip() {
        let mt = far_past_ns();
        let files = vec![mk_file("/x.jsonl", 500, mt)];
        let discovery = Discovery {
            files,
            ..Default::default()
        };
        let mut manifest = HashMap::new();
        manifest.insert(PathBuf::from("/x.jsonl"), mk_row("/x.jsonl", 500, mt, 500));
        let plan = plan_ingest(PlanInput {
            manifest: &manifest,
            discovery: &discovery,
        });
        assert_eq!(plan.to_skip.len(), 1);
        assert!(plan.to_full_parse.is_empty());
    }

    #[test]
    fn grown_file_goes_to_full_parse() {
        let mt = far_past_ns();
        let files = vec![mk_file("/x.jsonl", 800, mt + 1)];
        let discovery = Discovery {
            files,
            ..Default::default()
        };
        let mut manifest = HashMap::new();
        manifest.insert(PathBuf::from("/x.jsonl"), mk_row("/x.jsonl", 500, mt, 500));
        let plan = plan_ingest(PlanInput {
            manifest: &manifest,
            discovery: &discovery,
        });
        assert_eq!(plan.to_full_parse.len(), 1);
    }

    #[test]
    fn shrunk_file_goes_to_full_parse() {
        let mt = far_past_ns();
        let files = vec![mk_file("/x.jsonl", 100, mt + 1)];
        let discovery = Discovery {
            files,
            ..Default::default()
        };
        let mut manifest = HashMap::new();
        manifest.insert(PathBuf::from("/x.jsonl"), mk_row("/x.jsonl", 500, mt, 500));
        let plan = plan_ingest(PlanInput {
            manifest: &manifest,
            discovery: &discovery,
        });
        assert_eq!(plan.to_full_parse.len(), 1);
    }

    fn complete_root_discovery(source: Source) -> Discovery {
        Discovery {
            complete_roots: vec![(source, PathBuf::from("/"))],
            ..Default::default()
        }
    }

    #[test]
    fn disappeared_log_is_marked_missing_once_and_keeps_its_events() {
        let discovery = complete_root_discovery(Source::Claude);
        let mut manifest = HashMap::new();
        manifest.insert(
            PathBuf::from("/gone.jsonl"),
            mk_row("/gone.jsonl", 100, 1, 100),
        );
        let plan = plan_ingest(PlanInput {
            manifest: &manifest,
            discovery: &discovery,
        });
        assert_eq!(plan.to_mark_missing, vec![PathBuf::from("/gone.jsonl")]);
        assert!(plan.to_purge.is_empty());

        manifest
            .get_mut(&PathBuf::from("/gone.jsonl"))
            .unwrap()
            .missing_since = Some(5);
        let plan = plan_ingest(PlanInput {
            manifest: &manifest,
            discovery: &discovery,
        });
        assert!(plan.to_mark_missing.is_empty() && plan.to_purge.is_empty());
    }

    #[test]
    fn disappeared_cursor_export_goes_to_purge() {
        let discovery = complete_root_discovery(Source::Cursor);
        let mut manifest = HashMap::new();
        let mut row = mk_row("/usage.csv", 100, 1, 100);
        row.source = Source::Cursor;
        manifest.insert(PathBuf::from("/usage.csv"), row);
        let plan = plan_ingest(PlanInput {
            manifest: &manifest,
            discovery: &discovery,
        });
        assert_eq!(plan.to_purge, vec![PathBuf::from("/usage.csv")]);
        assert!(plan.to_mark_missing.is_empty());
    }

    #[test]
    fn reappearing_missing_file_is_reparsed_even_when_stats_match() {
        let mt = far_past_ns();
        let discovery = Discovery {
            files: vec![mk_file("/x.jsonl", 500, mt)],
            ..Default::default()
        };
        let mut row = mk_row("/x.jsonl", 500, mt, 500);
        row.missing_since = Some(5);
        let mut manifest = HashMap::new();
        manifest.insert(PathBuf::from("/x.jsonl"), row);
        let plan = plan_ingest(PlanInput {
            manifest: &manifest,
            discovery: &discovery,
        });
        assert_eq!(plan.to_full_parse.len(), 1);
        assert!(plan.to_skip.is_empty());
    }

    #[test]
    fn recent_file_goes_to_full_parse_even_if_grown() {
        let now_ms = base_now_ms();
        let recent_ns = (now_ms - 1_000).saturating_mul(1_000_000);
        let files = vec![mk_file("/x.jsonl", 800, recent_ns)];
        let discovery = Discovery {
            files,
            ..Default::default()
        };
        let mut manifest = HashMap::new();
        manifest.insert(
            PathBuf::from("/x.jsonl"),
            mk_row("/x.jsonl", 500, recent_ns - 1000, 500),
        );
        let plan = plan_ingest(PlanInput {
            manifest: &manifest,
            discovery: &discovery,
        });
        assert_eq!(plan.to_full_parse.len(), 1);
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
                file_identity: Some("test-file".into()),
                ctime_ns: Some(1),
            }],
            ..Default::default()
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
                file_identity: Some("test-file".into()),
                ctime_ns: Some(1),
                last_offset: 500,
                n_events: 0,
                session_id: None,
                model: None,
                missing_since: None,
            },
        );
        let plan = plan_ingest(PlanInput {
            manifest: &manifest,
            discovery: &discovery,
        });
        assert_eq!(plan.to_full_parse.len(), 1);
    }

    #[test]
    fn changed_codex_file_goes_to_full_parse_not_tail() {
        let mt = far_past_ns();
        let discovery = Discovery {
            files: vec![mk_file_with_source(
                "/codex/sess.jsonl",
                Source::Codex,
                800,
                mt + 1,
            )],
            ..Default::default()
        };
        let mut manifest = HashMap::new();
        manifest.insert(
            PathBuf::from("/codex/sess.jsonl"),
            FileManifestRow {
                path: PathBuf::from("/codex/sess.jsonl"),
                source: Source::Codex,
                project: None,
                size: 500,
                mtime_ns: mt,
                file_identity: Some("test-file".into()),
                ctime_ns: Some(1),
                last_offset: 500,
                n_events: 0,
                session_id: None,
                model: None,
                missing_since: None,
            },
        );
        let plan = plan_ingest(PlanInput {
            manifest: &manifest,
            discovery: &discovery,
        });
        assert_eq!(plan.to_full_parse.len(), 1);
    }
}
