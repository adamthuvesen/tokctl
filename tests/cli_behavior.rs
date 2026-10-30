// These tests corrupt or observe caches from outside, like another process would.
#![allow(clippy::disallowed_types)]

use std::fs;
use std::process::Command;

struct IsolatedCommand {
    command: Command,
    _home: tempfile::TempDir,
}
impl std::ops::Deref for IsolatedCommand {
    type Target = Command;
    fn deref(&self) -> &Command {
        &self.command
    }
}
impl std::ops::DerefMut for IsolatedCommand {
    fn deref_mut(&mut self) -> &mut Command {
        &mut self.command
    }
}

fn tokctl() -> IsolatedCommand {
    let home = tempfile::tempdir().expect("isolated CLI environment");
    let path = home.path();
    let mut command = Command::new(env!("CARGO_BIN_EXE_tokctl"));
    command
        .env("HOME", path)
        .env("XDG_CONFIG_HOME", path.join("config"))
        .env("XDG_DATA_HOME", path.join("data"))
        .env("TOKCTL_CACHE_DIR", path.join("cache"))
        .env("TOKCTL_CLAUDE_DIR", path.join("claude"))
        .env("TOKCTL_CODEX_DIR", path.join("codex"))
        .env("TOKCTL_CURSOR_DIR", path.join("cursor"))
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CODEX_HOME");
    IsolatedCommand {
        command,
        _home: home,
    }
}

fn isolated_cache_dir() -> tempfile::TempDir {
    tempfile::tempdir().expect("temp cache dir")
}

fn write_codex_session(root: &std::path::Path, file_name: &str, cwd: &std::path::Path, id: &str) {
    let session_meta = serde_json::json!({
        "timestamp": "2026-04-20T12:49:04.848Z",
        "type": "session_meta",
        "payload": {
            "id": id,
            "cwd": cwd,
            "originator": "Codex Desktop"
        }
    });
    let turn_context = serde_json::json!({
        "timestamp": "2026-04-20T12:49:05.000Z",
        "type": "turn_context",
        "payload": { "model": "gpt-5.4" }
    });
    let token_count = serde_json::json!({
        "timestamp": "2026-04-20T12:49:10.000Z",
        "type": "event_msg",
        "payload": {
            "type": "token_count",
            "info": {
                "last_token_usage": {
                    "input_tokens": 200,
                    "cached_input_tokens": 50,
                    "output_tokens": 60,
                    "total_tokens": 260
                }
            }
        }
    });
    let body = format!("{session_meta}\n{turn_context}\n{token_count}\n");
    fs::write(root.join(file_name), body).expect("write codex session");
}

#[test]
fn report_json_stdout_is_machine_readable() {
    let cache_dir = isolated_cache_dir();
    let output = tokctl()
        .args([
            "daily",
            "--json",
            "--no-cache",
            "--claude-dir",
            "/path/that/does/not/exist",
            "--codex-dir",
            "/path/that/does/not/exist",
            "--cursor-dir",
            "/path/that/does/not/exist",
        ])
        .env("TOKCTL_CACHE_DIR", cache_dir.path())
        .output()
        .expect("run tokctl daily");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid json stdout");
    assert!(parsed.is_array());
}

#[test]
fn compare_json_stdout_is_machine_readable() {
    let cache_dir = isolated_cache_dir();
    let output = tokctl()
        .args([
            "compare",
            "--json",
            "--no-cache",
            "--claude-dir",
            "/path/that/does/not/exist",
            "--codex-dir",
            "/path/that/does/not/exist",
            "--cursor-dir",
            "/path/that/does/not/exist",
        ])
        .env("TOKCTL_CACHE_DIR", cache_dir.path())
        .output()
        .expect("run tokctl compare");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid json stdout");
    assert!(parsed.is_object());
}

#[test]
fn rebuild_and_no_cache_remain_mutually_exclusive() {
    let cache_dir = isolated_cache_dir();
    let output = tokctl()
        .args(["daily", "--rebuild", "--no-cache"])
        .env("TOKCTL_CACHE_DIR", cache_dir.path())
        .output()
        .expect("run tokctl daily");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("utf8 stderr");
    assert!(stderr.contains("--rebuild and --no-cache are mutually exclusive"));
}

#[test]
fn export_db_prints_cache_path_without_creating_database() {
    let cache_dir = isolated_cache_dir();
    let expected_path = cache_dir.path().join("cache.db");
    let output = tokctl()
        .arg("export-db")
        .env("TOKCTL_CACHE_DIR", cache_dir.path())
        .output()
        .expect("run tokctl export-db");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    assert_eq!(
        stdout.trim(),
        expected_path.to_str().expect("utf8 cache path")
    );
    assert!(!expected_path.exists());
}

#[test]
fn doctor_json_does_not_create_cache_or_config() {
    let home = tempfile::tempdir().expect("home");
    let cache_dir = isolated_cache_dir();
    let expected_cache_path = cache_dir.path().join("cache.db");
    let expected_config_dir = home.path().join(".config").join("tokctl");
    let output = tokctl()
        .args(["doctor", "--json"])
        .env("HOME", home.path())
        .env("TOKCTL_CACHE_DIR", cache_dir.path())
        .env_remove("XDG_DATA_HOME")
        .env_remove("TOKCTL_CLAUDE_DIR")
        .env_remove("TOKCTL_CODEX_DIR")
        .env_remove("TOKCTL_CURSOR_DIR")
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CODEX_HOME")
        .output()
        .expect("run tokctl doctor");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid json stdout");
    assert_eq!(
        parsed["summary"]["cache_path"],
        expected_cache_path.to_str().expect("utf8 cache path")
    );
    let check_counts = &parsed["check_counts"];
    let ok = check_counts["ok"].as_u64().expect("ok count");
    let warn = check_counts["warn"].as_u64().expect("warn count");
    let error = check_counts["error"].as_u64().expect("error count");
    let checks = parsed["checks"].as_array().expect("checks array");
    assert_eq!(ok + warn + error, checks.len() as u64);
    assert!(!expected_cache_path.exists());
    assert!(!expected_config_dir.exists());
}

#[test]
fn no_cache_repo_filter_rejects_ambiguous_display_names() {
    let cache_dir = isolated_cache_dir();
    let workspace = tempfile::tempdir().expect("workspace");
    let codex_root = workspace.path().join("codex");
    let left_repo = workspace.path().join("left").join("alpha");
    let right_repo = workspace.path().join("right").join("alpha");
    fs::create_dir_all(left_repo.join(".git")).expect("left repo");
    fs::create_dir_all(right_repo.join(".git")).expect("right repo");
    fs::create_dir_all(&codex_root).expect("codex root");
    write_codex_session(&codex_root, "left.jsonl", &left_repo, "left-session");
    write_codex_session(&codex_root, "right.jsonl", &right_repo, "right-session");

    let output = tokctl()
        .args([
            "daily",
            "--no-cache",
            "--repo",
            "alpha",
            "--claude-dir",
            "/path/that/does/not/exist",
            "--codex-dir",
            codex_root.to_str().expect("utf8 codex root"),
            "--cursor-dir",
            "/path/that/does/not/exist",
        ])
        .env("TOKCTL_CACHE_DIR", cache_dir.path())
        .output()
        .expect("run tokctl daily");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("utf8 stderr");
    assert!(stderr.contains("repo name 'alpha' is ambiguous"));
    assert!(stderr.contains(left_repo.to_str().expect("utf8 left repo")));
    assert!(stderr.contains(right_repo.to_str().expect("utf8 right repo")));
}

#[test]
fn concurrent_processes_replace_changed_snapshots_once() {
    let workspace = tempfile::tempdir().unwrap();
    let claude = workspace.path().join("claude/project");
    fs::create_dir_all(&claude).unwrap();
    let path = claude.join("session.jsonl");
    let event = |id: &str| {
        format!(
            r#"{{"type":"assistant","timestamp":"2026-04-22T12:00:00Z","sessionId":"same","message":{{"id":"{id}","model":"claude-sonnet-4-6","usage":{{"input_tokens":1}}}}}}"#
        )
    };
    fs::write(&path, format!("{}\n", event("a"))).unwrap();
    let make_command = || {
        let mut command = tokctl();
        command
            .args(["daily", "--json", "--source", "claude"])
            .env("TOKCTL_CACHE_DIR", workspace.path().join("cache"))
            .env("TOKCTL_CLAUDE_DIR", workspace.path().join("claude"))
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        command
    };
    let mut left = make_command();
    let mut right = make_command();
    let left = left.spawn().unwrap();
    let right = right.spawn().unwrap();
    assert!(left.wait_with_output().unwrap().status.success());
    assert!(right.wait_with_output().unwrap().status.success());
    fs::write(&path, format!("{}\n{}\n", event("a"), event("b"))).unwrap();
    let mut left = make_command();
    let mut right = make_command();
    let left = left.spawn().unwrap();
    let right = right.spawn().unwrap();
    assert!(left.wait_with_output().unwrap().status.success());
    assert!(right.wait_with_output().unwrap().status.success());
    let conn = rusqlite::Connection::open(workspace.path().join("cache/cache.db")).unwrap();
    let events: i64 = conn
        .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
        .unwrap();
    let files: i64 = conn
        .query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))
        .unwrap();
    assert_eq!((events, files), (2, 1));
}

#[test]
fn invalid_date_order_fails_before_config_or_sync_access() {
    let home = tempfile::tempdir().unwrap();
    fs::create_dir_all(home.path().join("tokctl")).unwrap();
    fs::write(
        home.path().join("tokctl/cursor-credentials.json"),
        "invalid credentials",
    )
    .unwrap();
    let output = tokctl()
        .args(["daily", "--since", "2026-04-23", "--until", "2026-04-22"])
        .env("XDG_CONFIG_HOME", home.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8(output.stderr)
        .unwrap()
        .contains("--until must be on or after --since"));
}

#[test]
fn noninteractive_ui_fails_before_creating_demo_or_cache() {
    let directory = tempfile::tempdir().unwrap();
    for demo in [false, true] {
        let cache = directory.path().join(if demo { "demo" } else { "normal" });
        let mut command = tokctl();
        command.arg("ui").env("TOKCTL_CACHE_DIR", &cache);
        if demo {
            command.args(["--demo", "--cache-dir"]).arg(&cache);
        }
        let output = command.output().unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("requires an interactive terminal")
        );
        assert!(!cache.exists());
    }
}

#[cfg(unix)]
#[test]
fn unavailable_symlink_root_keeps_previously_cached_rows_visible() {
    let workspace = tempfile::tempdir().unwrap();
    let root = workspace.path().join("codex");
    let alias = workspace.path().join("alias");
    fs::create_dir(&root).unwrap();
    std::os::unix::fs::symlink(&root, &alias).unwrap();
    write_codex_session(&root, "session.jsonl", workspace.path(), "saved-session");
    let report = || {
        tokctl()
            .args(["daily", "--json", "--source", "codex"])
            .env("TOKCTL_CODEX_DIR", &alias)
            .env("TOKCTL_CACHE_DIR", workspace.path().join("cache"))
            .output()
            .unwrap()
    };
    let initial = report();
    assert!(
        initial.status.success(),
        "{}",
        String::from_utf8_lossy(&initial.stderr)
    );
    let expected: serde_json::Value = serde_json::from_slice(&initial.stdout).unwrap();
    assert_eq!(expected.as_array().unwrap().len(), 1);
    fs::rename(&root, workspace.path().join("offline")).unwrap();
    let offline = report();
    assert!(
        offline.status.success(),
        "{}",
        String::from_utf8_lossy(&offline.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&offline.stdout).unwrap(),
        expected
    );
    assert!(String::from_utf8_lossy(&offline.stderr).contains("warning:"));
}

#[test]
fn retained_stale_provenance_warns_only_for_matching_report_scope() {
    let workspace = tempfile::tempdir().unwrap();
    let root = workspace.path().join("codex");
    fs::create_dir(&root).unwrap();
    write_codex_session(&root, "session.jsonl", workspace.path(), "saved-session");
    let report = |extra: &[&str]| {
        tokctl()
            .args(["daily", "--json", "--source", "codex"])
            .args(extra)
            .env("TOKCTL_CODEX_DIR", &root)
            .env("TOKCTL_CACHE_DIR", workspace.path().join("cache"))
            .output()
            .unwrap()
    };
    assert!(report(&[]).status.success());
    let connection = rusqlite::Connection::open(workspace.path().join("cache/cache.db")).unwrap();
    connection.execute_batch("UPDATE events SET cost_provenance='legacy'; UPDATE parser_provenance SET signature='stale';").unwrap();
    drop(connection);
    fs::rename(&root, workspace.path().join("offline")).unwrap();
    let offline = report(&[]);
    assert!(
        offline.status.success(),
        "{}",
        String::from_utf8_lossy(&offline.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&offline.stdout)
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let warnings = String::from_utf8_lossy(&offline.stderr);
    assert!(
        warnings.contains("retained 1 legacy cost row(s)"),
        "{warnings}"
    );
    assert!(
        warnings.contains("retained 1 file snapshot(s) from a stale parser"),
        "{warnings}"
    );
    let excluded = report(&["--since", "2026-04-21", "--until", "2026-04-21"]);
    assert!(excluded.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&excluded.stdout).unwrap(),
        serde_json::json!([])
    );
    assert!(!String::from_utf8_lossy(&excluded.stderr).contains("retained"));
}

#[test]
fn diagnostic_failure_prevents_partial_report_stdout() {
    let workspace = tempfile::tempdir().unwrap();
    let root = workspace.path().join("codex");
    fs::create_dir(&root).unwrap();
    write_codex_session(&root, "session.jsonl", workspace.path(), "saved-session");
    let report = |command: &str| {
        let mut command_line = tokctl();
        command_line
            .args([command, "--json", "--source", "codex"])
            .env("TOKCTL_CODEX_DIR", &root)
            .env("TOKCTL_CACHE_DIR", workspace.path().join("cache"));
        if command == "compare" {
            command_line.args(["2026-04-20..2026-04-20", "2026-04-19..2026-04-19"]);
        }
        command_line.output().unwrap()
    };
    assert!(report("daily").status.success());
    let connection = rusqlite::Connection::open(workspace.path().join("cache/cache.db")).unwrap();
    connection
        .execute("UPDATE ingest_diagnostics SET skipped_records=-1", [])
        .unwrap();
    drop(connection);
    for command in ["daily", "monthly", "session", "repo", "compare"] {
        let output = report(command);
        assert!(!output.status.success(), "{command}");
        assert!(
            output.stdout.is_empty(),
            "{command}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
}

#[test]
fn malformed_only_files_warn_in_selected_source_roots_without_events() {
    let workspace = tempfile::tempdir().unwrap();
    let root = workspace.path().join("codex");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("malformed.jsonl"), "{broken json\n").unwrap();
    for no_cache in [false, true] {
        let mut command = tokctl();
        command
            .args(["daily", "--json", "--source", "codex"])
            .env("TOKCTL_CODEX_DIR", &root)
            .env("TOKCTL_CACHE_DIR", workspace.path().join("cache"));
        if no_cache {
            command.arg("--no-cache");
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
            serde_json::json!([])
        );
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("skipped 1 malformed input line(s)")
        );
    }
}
