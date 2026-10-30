// These tests corrupt or observe caches from outside, like another process would.
#![allow(clippy::disallowed_types)]

use std::fs;
use std::path::{Path, PathBuf};
use tokctl::ingest::run::{run_ingest, RunIngestOptions};
use tokctl::store::queries::{
    daily_report, event_rows, period_buckets, PeriodGranularity, QueryScope, RepoFilterSpec,
    SessionKey,
};
use tokctl::store::Store;
use tokctl::types::Source;

fn options(root: &Path, source: Source) -> RunIngestOptions {
    RunIngestOptions {
        claude_roots: if source == Source::Claude {
            vec![root.to_path_buf()]
        } else {
            vec![]
        },
        codex_roots: if source == Source::Codex {
            vec![root.to_path_buf()]
        } else {
            vec![]
        },
        cursor_roots: if source == Source::Cursor {
            vec![root.to_path_buf()]
        } else {
            vec![]
        },
        source: Some(source),
    }
}
fn claude(message: &str, tokens: u64) -> String {
    format!(
        r#"{{"type": "assistant", "timestamp":"2026-04-22T12:00:00Z","sessionId":"same","message":{{"id":"{message}","model":"claude-sonnet-4-6","usage":{{"input_tokens":{tokens}}}}}}}
"#
    )
}
fn root(workspace: &tempfile::TempDir, name: &str) -> PathBuf {
    let path = workspace.path().join(name);
    fs::create_dir_all(&path).unwrap();
    path.canonicalize().unwrap()
}

#[test]
fn absent_unselected_and_failed_roots_preserve_snapshots() {
    let workspace = tempfile::tempdir().unwrap();
    let left = root(&workspace, "left");
    let right = root(&workspace, "right");
    fs::write(left.join("a.jsonl"), claude("left", 3)).unwrap();
    fs::write(right.join("b.jsonl"), claude("right", 7)).unwrap();
    let mut store = Store::ephemeral().unwrap();
    run_ingest(&mut store, options(&left, Source::Claude)).unwrap();
    run_ingest(&mut store, options(&right, Source::Claude)).unwrap();
    assert_eq!(event_rows(&store, QueryScope::default()).unwrap().len(), 2);
    fs::rename(&left, workspace.path().join("unavailable")).unwrap();
    let stats = run_ingest(&mut store, options(&left, Source::Claude)).unwrap();
    assert_eq!(stats.file_errors, 1);
    assert_eq!(stats.files_purged, 0);
    fs::remove_file(right.join("b.jsonl")).unwrap();
    let stats = run_ingest(&mut store, options(&right, Source::Claude)).unwrap();
    assert_eq!(stats.files_purged, 1);
    assert_eq!(event_rows(&store, QueryScope::default()).unwrap().len(), 1);
}

#[test]
fn append_reparses_claude_with_whole_file_message_deduplication() {
    let workspace = tempfile::tempdir().unwrap();
    let root = root(&workspace, "claude");
    let path = root.join("a.jsonl");
    fs::write(&path, claude("same", 3)).unwrap();
    let mut store = Store::ephemeral().unwrap();
    run_ingest(&mut store, options(&root, Source::Claude)).unwrap();
    fs::write(
        &path,
        format!(
            "{}{}{}",
            claude("same", 3),
            claude("same", 3),
            claude("new", 7)
        ),
    )
    .unwrap();
    let stats = run_ingest(&mut store, options(&root, Source::Claude)).unwrap();
    assert_eq!(stats.files_full_parsed, 1);
    assert_eq!(
        daily_report(&store, QueryScope::default()).unwrap()[0].input_tokens,
        10
    );
}

#[test]
fn malformed_csv_header_preserves_the_previous_snapshot() {
    let workspace = tempfile::tempdir().unwrap();
    let root = root(&workspace, "cursor");
    let path = root.join("usage.csv");
    fs::write(&path,"Date,Model,Input (w/ Cache Write),Input (w/o Cache Write),Cache Read,Output Tokens,Cost\n2026-04-22,gpt-5.4,2,2,0,1,0.5\n").unwrap();
    let mut store = Store::ephemeral().unwrap();
    run_ingest(&mut store, options(&root, Source::Cursor)).unwrap();
    fs::write(&path, "Date,Model\n2026-04-22,gpt-5.4\n").unwrap();
    let stats = run_ingest(&mut store, options(&root, Source::Cursor)).unwrap();
    assert_eq!(stats.file_errors, 1);
    let events = event_rows(&store, QueryScope::default()).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].cost, 0.5);
}

#[test]
fn canonical_aliases_do_not_duplicate_files_and_roots_limit_reports() {
    let workspace = tempfile::tempdir().unwrap();
    let root = root(&workspace, "claude");
    fs::write(root.join("a.jsonl"), claude("a", 3)).unwrap();
    let mut opts = options(&root, Source::Claude);
    opts.claude_roots.push(root.join("."));
    let mut store = Store::ephemeral().unwrap();
    let stats = run_ingest(&mut store, opts).unwrap();
    assert_eq!(stats.events_inserted, 1);
    let scope = QueryScope {
        roots: Some(vec![root.clone()]),
        ..Default::default()
    };
    assert_eq!(daily_report(&store, scope).unwrap().len(), 1);
    let scope = QueryScope {
        roots: Some(vec![workspace.path().join("other")]),
        ..Default::default()
    };
    assert!(daily_report(&store, scope).unwrap().is_empty());
}

#[test]
fn unsafe_token_totals_are_diagnostics_instead_of_integer_wraparound() {
    let workspace = tempfile::tempdir().unwrap();
    let root = root(&workspace, "claude");
    fs::write(
        root.join("a.jsonl"),
        format!("{}{}", claude("bad", u64::MAX), claude("good", 7)),
    )
    .unwrap();
    let mut store = Store::ephemeral().unwrap();
    let stats = run_ingest(&mut store, options(&root, Source::Claude)).unwrap();
    assert_eq!(stats.skipped_lines, 1);
    assert_eq!(
        event_rows(&store, QueryScope::default()).unwrap()[0].input,
        7
    );
}

#[test]
fn exact_repo_and_literal_prefix_model_session_and_time_scopes_compose() {
    let mut store = Store::ephemeral().unwrap();
    // Persisted rows are intentionally seeded through SQLite only in this fixture.
    let workspace = tempfile::tempdir().unwrap();
    let path = workspace.path().join("cache.db");
    drop(store);
    store = Store::open(&path).unwrap();
    let conn = rusqlite::Connection::open(path).unwrap();
    conn.execute_batch("INSERT INTO events(file_path,source,ts,day,month,session_id,repo,model,input,output,cache_read,cache_write,cost_usd) VALUES ('/root/a','claude',1,'2025-12-29','2025-12','same','/repo_%','wanted',1,0,0,0,1),('/root/b','codex',1,'2025-12-29','2025-12','same','/repo_%','wanted',9,0,0,0,1),('/root/c','claude',2,'2025-12-29','2025-12','same','/repoZZ','other',9,0,0,0,1);").unwrap();
    let mut scope = QueryScope {
        repo: Some(RepoFilterSpec::KeyPrefix("/repo_%".into())),
        model: Some("wanted".into()),
        session: Some(SessionKey {
            source: Source::Claude,
            id: "same".into(),
        }),
        since_ms: Some(1),
        until_ms: Some(2),
        ..Default::default()
    };
    assert_eq!(event_rows(&store, scope.clone()).unwrap().len(), 1);
    assert_eq!(
        period_buckets(&store, scope.clone(), PeriodGranularity::Weekly).unwrap()[0].key,
        "2026-W01"
    );
    scope.repo = Some(RepoFilterSpec::KeyExact("/repo_%".into()));
    assert_eq!(
        daily_report(&store, scope.clone()).unwrap()[0].input_tokens,
        1
    );
    scope.intersect_time(Some(2), Some(3));
    assert!(event_rows(&store, scope).unwrap().is_empty());
}

#[test]
fn malformed_only_file_diagnostics_persist_and_obey_root_scope() {
    let workspace = tempfile::tempdir().unwrap();
    let root = root(&workspace, "claude");
    fs::write(root.join("bad.jsonl"), "{malformed}\n").unwrap();
    let path = workspace.path().join("cache.db");
    let mut store = Store::open(&path).unwrap();
    run_ingest(&mut store, options(&root, Source::Claude)).unwrap();
    drop(store);
    let mut store = Store::open(&path).unwrap();
    let stats = run_ingest(&mut store, options(&root, Source::Claude)).unwrap();
    assert_eq!(stats.files_skipped, 1);
    assert_eq!(
        tokctl::store::queries::skipped_records(&store, &QueryScope::default()).unwrap(),
        1
    );
    let scope = QueryScope {
        roots: Some(vec![workspace.path().join("other")]),
        ..Default::default()
    };
    assert_eq!(
        tokctl::store::queries::skipped_records(&store, &scope).unwrap(),
        0
    );
}

#[test]
fn migrated_legacy_costs_remain_stale_when_original_source_is_unavailable() {
    let workspace = tempfile::tempdir().unwrap();
    let path = workspace.path().join("cache.db");
    let absent = workspace.path().join("unavailable");
    let file = absent.join("session.jsonl");
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch("CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT NOT NULL); INSERT INTO meta VALUES('schema_version','6'); CREATE TABLE files(path TEXT PRIMARY KEY,source TEXT NOT NULL,project TEXT,size INTEGER NOT NULL,mtime_ns INTEGER NOT NULL,last_offset INTEGER NOT NULL,n_events INTEGER NOT NULL,session_id TEXT,model TEXT); CREATE TABLE events(id INTEGER PRIMARY KEY,file_path TEXT NOT NULL,source TEXT NOT NULL,ts INTEGER NOT NULL,day TEXT NOT NULL,month TEXT NOT NULL,session_id TEXT NOT NULL,project_path TEXT,repo TEXT,model TEXT NOT NULL,input INTEGER NOT NULL,output INTEGER NOT NULL,cache_read INTEGER NOT NULL,cache_write INTEGER NOT NULL,cost_usd REAL NOT NULL); CREATE TABLE repos(key TEXT PRIMARY KEY,display_name TEXT NOT NULL,origin_url TEXT,first_seen INTEGER NOT NULL);").unwrap();
    conn.execute(
        "INSERT INTO files VALUES(?1,'claude',NULL,1,1,1,1,'same','claude-sonnet-4-6')",
        [file.to_str().unwrap()],
    )
    .unwrap();
    conn.execute("INSERT INTO events VALUES(1,?1,'claude',1776859200000,'2026-04-22','2026-04','same',NULL,NULL,'claude-sonnet-4-6',1,0,0,0,100)",[file.to_str().unwrap()]).unwrap();
    drop(conn);
    let mut store = Store::open(&path).unwrap();
    run_ingest(&mut store, options(&absent, Source::Claude)).unwrap();
    assert_eq!(
        event_rows(&store, QueryScope::default()).unwrap()[0].cost,
        100.0
    );
    let conn = rusqlite::Connection::open(path).unwrap();
    let provenance: String = conn
        .query_row("SELECT cost_provenance FROM events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(provenance, "legacy");
    let signatures: i64 = conn
        .query_row("SELECT COUNT(*) FROM parser_provenance", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(signatures, 0);
}

#[test]
fn finite_source_costs_that_overflow_aggregation_return_errors() {
    let workspace = tempfile::tempdir().unwrap();
    let path = workspace.path().join("cache.db");
    let store = Store::open(&path).unwrap();
    let conn = rusqlite::Connection::open(path).unwrap();
    conn.execute_batch("INSERT INTO events(file_path,source,ts,day,month,session_id,model,input,output,cache_read,cache_write,cost_usd,explicit_cost_usd,cost_provenance) VALUES('/a','cursor',1776859200000,'2026-04-22','2026-04','same','unknown',1,0,0,0,1e308,1e308,'source'),('/b','cursor',1776859200000,'2026-04-22','2026-04','same','unknown',1,0,0,0,1e308,1e308,'source');").unwrap();
    assert!(daily_report(&store, QueryScope::default())
        .unwrap_err()
        .to_string()
        .contains("finite"));
    let windows = tokctl::compare::resolve_windows(
        Some("2026-04-22..2026-04-22"),
        Some("2026-04-21..2026-04-21"),
        chrono::Utc::now(),
    )
    .unwrap();
    assert!(tokctl::compare::compare_from_db(
        &store,
        windows,
        QueryScope::default(),
        &[tokctl::compare::CompareDimension::Source],
        5
    )
    .is_err());
}

#[test]
fn read_batches_pin_one_snapshot_and_readonly_queries_reject_changed_provenance() {
    let workspace = tempfile::tempdir().unwrap();
    let path = workspace.path().join("cache.db");
    let store = Store::open(&path).unwrap();
    let writer = rusqlite::Connection::open(&path).unwrap();
    writer.execute_batch("INSERT INTO events(file_path,source,ts,day,month,session_id,model,input,output,cache_read,cache_write,cost_usd,explicit_cost_usd,cost_provenance) VALUES('/a','cursor',1776859200000,'2026-04-22','2026-04','same','unknown',1,0,0,0,1,1,'source');").unwrap();
    store
        .read_snapshot(|store| {
            assert_eq!(
                daily_report(store, QueryScope::default())?[0].input_tokens,
                1
            );
            writer.execute("UPDATE events SET input=2", [])?;
            assert_eq!(
                daily_report(store, QueryScope::default())?[0].input_tokens,
                1
            );
            Ok(())
        })
        .unwrap();
    assert_eq!(
        daily_report(&store, QueryScope::default()).unwrap()[0].input_tokens,
        2
    );
    let readonly = Store::open_read_only(&path).unwrap();
    writer
        .execute(
            "UPDATE meta SET value='other-zone' WHERE key='calendar_signature'",
            [],
        )
        .unwrap();
    assert!(daily_report(&readonly, QueryScope::default()).is_err());
    assert_eq!(
        daily_report(&store, QueryScope::default()).unwrap()[0].input_tokens,
        2
    );
}

#[test]
fn normal_ingestion_and_rebuild_remove_demo_overwrite_authority() {
    let workspace = tempfile::tempdir().unwrap();
    let path = workspace.path().join("cache.db");
    let root = root(&workspace, "empty");
    let mut store = Store::open(&path).unwrap();
    let conn = rusqlite::Connection::open(path).unwrap();
    for rebuild in [false, true] {
        conn.execute(
            "INSERT INTO meta(key,value) VALUES('demo_marker','tokctl synthetic demo v1')",
            [],
        )
        .unwrap();
        if rebuild {
            store.rebuild().unwrap();
        } else {
            run_ingest(&mut store, options(&root, Source::Claude)).unwrap();
        }
        let markers: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM meta WHERE key='demo_marker'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(markers, 0);
    }
}

#[test]
fn complete_other_provider_root_cannot_purge_unavailable_source() {
    let workspace = tempfile::tempdir().unwrap();
    let shared = root(&workspace, "shared");
    let claude_root = shared.join("claude");
    fs::create_dir(&claude_root).unwrap();
    fs::write(claude_root.join("a.jsonl"), claude("a", 7)).unwrap();
    let mut store = Store::ephemeral().unwrap();
    run_ingest(&mut store, options(&claude_root, Source::Claude)).unwrap();
    fs::rename(&claude_root, workspace.path().join("offline")).unwrap();
    let mut opts = options(&claude_root, Source::Claude);
    opts.source = None;
    opts.cursor_roots = vec![shared];
    let stats = run_ingest(&mut store, opts).unwrap();
    assert_eq!(stats.files_purged, 0);
    assert_eq!(
        event_rows(&store, QueryScope::default()).unwrap()[0].input,
        7
    );
}

#[test]
fn conflicting_provider_claims_fail_before_any_ingest_mutation() {
    let workspace = tempfile::tempdir().unwrap();
    let root = root(&workspace, "shared");
    let file = root.join("a.jsonl");
    fs::write(&file, claude("a", 7)).unwrap();
    let db = workspace.path().join("cache.db");
    let mut store = Store::open(&db).unwrap();
    let observer = rusqlite::Connection::open(&db).unwrap();
    let mut opts = options(&root, Source::Claude);
    opts.source = None;
    opts.codex_roots = vec![root.clone()];
    assert!(run_ingest(&mut store, opts).is_err());
    let files: i64 = observer
        .query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))
        .unwrap();
    assert_eq!(files, 0);
    run_ingest(&mut store, options(&root, Source::Claude)).unwrap();
    observer
        .execute(
            "INSERT INTO meta(key,value) VALUES('demo_marker','synthetic')",
            [],
        )
        .unwrap();
    observer
        .execute(
            "UPDATE meta SET value='stale' WHERE key='pricing_signature'",
            [],
        )
        .unwrap();
    let before: String = observer.query_row("SELECT group_concat(key || '=' || value, '|') FROM (SELECT key,value FROM meta ORDER BY key)", [], |r| r.get(0)).unwrap();
    let error = run_ingest(&mut store, options(&root, Source::Codex))
        .unwrap_err()
        .to_string();
    assert!(error.contains(file.to_str().unwrap()), "{error}");
    assert!(
        error.contains("claude") && error.contains("codex"),
        "{error}"
    );
    assert!(error.contains("rebuild"), "{error}");
    let after: String = observer.query_row("SELECT group_concat(key || '=' || value, '|') FROM (SELECT key,value FROM meta ORDER BY key)", [], |r| r.get(0)).unwrap();
    assert_eq!(before, after);
    let input: i64 = observer
        .query_row("SELECT input FROM events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(input, 7);
}

#[test]
fn malformed_only_replacement_preserves_nonempty_valid_snapshot() {
    let workspace = tempfile::tempdir().unwrap();
    let root = root(&workspace, "claude");
    let file = root.join("a.jsonl");
    fs::write(&file, claude("a", 7)).unwrap();
    let mut store = Store::ephemeral().unwrap();
    run_ingest(&mut store, options(&root, Source::Claude)).unwrap();
    fs::write(&file, "{malformed}\n").unwrap();
    let stats = run_ingest(&mut store, options(&root, Source::Claude)).unwrap();
    assert_eq!(stats.file_errors, 1);
    assert_eq!(stats.files_full_parsed, 0);
    assert_eq!(
        event_rows(&store, QueryScope::default()).unwrap()[0].input,
        7
    );
    assert_eq!(
        run_ingest(&mut store, options(&root, Source::Claude))
            .unwrap()
            .file_errors,
        1
    );
}

#[test]
fn atomic_equal_size_equal_mtime_replacement_reparses() {
    let workspace = tempfile::tempdir().unwrap();
    let root = root(&workspace, "claude");
    let file = root.join("a.jsonl");
    fs::write(&file, claude("a", 3)).unwrap();
    let modified = fs::metadata(&file).unwrap().modified().unwrap();
    let mut store = Store::ephemeral().unwrap();
    run_ingest(&mut store, options(&root, Source::Claude)).unwrap();
    let replacement = root.join("replacement");
    fs::write(&replacement, claude("a", 7)).unwrap();
    fs::File::options()
        .write(true)
        .open(&replacement)
        .unwrap()
        .set_modified(modified)
        .unwrap();
    fs::rename(replacement, &file).unwrap();
    let stats = run_ingest(&mut store, options(&root, Source::Claude)).unwrap();
    assert_eq!(stats.files_full_parsed, 1);
    assert_eq!(
        event_rows(&store, QueryScope::default()).unwrap()[0].input,
        7
    );
}

#[test]
fn discovered_snapshot_cannot_publish_a_different_equal_mtime_file() {
    let workspace = tempfile::tempdir().unwrap();
    let root = root(&workspace, "claude");
    let file = root.join("a.jsonl");
    fs::write(&file, claude("a", 3)).unwrap();
    let modified = fs::metadata(&file).unwrap().modified().unwrap();
    let discovered = tokctl::discovery::discover_claude(std::slice::from_ref(&root))
        .files
        .remove(0);
    let replacement = root.join("replacement");
    fs::write(&replacement, claude("a", 7)).unwrap();
    fs::File::options()
        .write(true)
        .open(&replacement)
        .unwrap()
        .set_modified(modified)
        .unwrap();
    fs::rename(replacement, &file).unwrap();
    assert!(tokctl::ingest::file_range::parse_discovered_file(&discovered).is_err());
}

#[test]
fn schema_seven_snapshots_reparse_when_identity_is_unknown() {
    let workspace = tempfile::tempdir().unwrap();
    let root = root(&workspace, "claude");
    fs::write(root.join("a.jsonl"), claude("a", 3)).unwrap();
    let db = workspace.path().join("cache.db");
    let mut store = Store::open(&db).unwrap();
    run_ingest(&mut store, options(&root, Source::Claude)).unwrap();
    drop(store);
    let legacy = rusqlite::Connection::open(&db).unwrap();
    legacy.execute_batch("ALTER TABLE files DROP COLUMN file_identity; ALTER TABLE files DROP COLUMN ctime_ns; DROP TABLE source_roots; UPDATE meta SET value='7' WHERE key='schema_version';").unwrap();
    drop(legacy);
    let mut store = Store::open(&db).unwrap();
    assert_eq!(
        event_rows(&store, QueryScope::default()).unwrap()[0].input,
        3
    );
    let stats = run_ingest(&mut store, options(&root, Source::Claude)).unwrap();
    assert_eq!(stats.files_full_parsed, 1);
    assert_eq!(
        run_ingest(&mut store, options(&root, Source::Claude))
            .unwrap()
            .files_skipped,
        1
    );
}

#[cfg(unix)]
#[test]
fn persisted_symlink_roots_keep_offline_reports_in_the_original_scope() {
    let workspace = tempfile::tempdir().unwrap();
    let actual = root(&workspace, "actual");
    fs::write(actual.join("a.jsonl"), claude("a", 7)).unwrap();
    let alias = workspace.path().join("alias");
    std::os::unix::fs::symlink(&actual, &alias).unwrap();
    let db = workspace.path().join("cache.db");
    let mut store = Store::open(&db).unwrap();
    let resolved = store
        .resolve_source_roots(Source::Claude, std::slice::from_ref(&alias))
        .unwrap();
    assert_eq!(resolved, vec![actual.clone()]);
    run_ingest(&mut store, options(&resolved[0], Source::Claude)).unwrap();
    drop(store);
    fs::rename(&actual, workspace.path().join("offline")).unwrap();
    let mut store = Store::open(&db).unwrap();
    let resolved = store
        .resolve_source_roots(Source::Claude, std::slice::from_ref(&alias))
        .unwrap();
    assert_eq!(resolved, vec![actual]);
    let stats = run_ingest(&mut store, options(&resolved[0], Source::Claude)).unwrap();
    assert_eq!(stats.file_errors, 1);
    assert_eq!(stats.files_purged, 0);
    assert_eq!(
        event_rows(
            &store,
            QueryScope {
                roots: Some(resolved),
                ..Default::default()
            }
        )
        .unwrap()[0]
            .input,
        7
    );
    assert_eq!(
        store
            .resolve_source_roots(Source::Codex, std::slice::from_ref(&alias))
            .unwrap(),
        vec![alias]
    );
}
