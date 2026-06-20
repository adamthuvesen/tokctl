use std::fs;
use std::process::Command;

fn tokctl() -> Command {
    Command::new(env!("CARGO_BIN_EXE_tokctl"))
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
