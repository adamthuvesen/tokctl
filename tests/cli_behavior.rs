use std::process::Command;

fn tokctl() -> Command {
    Command::new(env!("CARGO_BIN_EXE_tokctl"))
}

fn isolated_cache_dir() -> tempfile::TempDir {
    tempfile::tempdir().expect("temp cache dir")
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
