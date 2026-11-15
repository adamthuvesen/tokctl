//! Persistent and ephemeral reports obey the same public fixture contract.
use serde_json::{json, Value};
use std::fs;
use std::process::Command;

struct Fixture {
    home: tempfile::TempDir,
}
impl Fixture {
    fn new() -> Self {
        let home = tempfile::tempdir().unwrap();
        let root = home.path();
        fs::create_dir_all(root.join("claude/project")).unwrap();
        fs::create_dir_all(root.join("codex")).unwrap();
        fs::create_dir_all(root.join("cursor")).unwrap();
        fs::write(root.join("claude/project/session.jsonl"), r#"{"type": "assistant", "timestamp":"2026-04-22T12:00:00Z","sessionId":"same","message":{"id":"a","model":"claude-sonnet-4-6","usage":{"input_tokens":100,"output_tokens":40,"cache_read_input_tokens":500,"cache_creation_input_tokens":200}}}
"#).unwrap();
        fs::write(root.join("codex/session.jsonl"), r#"{"type": "session_meta","payload":{"id":"same"}}
{"type": "turn_context","payload":{"model":"gpt-5.4"}}
{"type": "event_msg","timestamp":"2026-04-22T12:00:01Z","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":200,"cached_input_tokens":50,"output_tokens":60}}}}
"#).unwrap();
        Self { home }
    }
    fn report(&self, command: &str, no_cache: bool, extra: &[&str]) -> Value {
        let root = self.home.path();
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_tokctl"));
        cmd.arg(command)
            .arg("--json")
            .args(extra)
            .env("HOME", root)
            .env("XDG_CONFIG_HOME", root.join("config"))
            .env(
                "TOKCTL_CACHE_DIR",
                root.join(if no_cache { "unused-cache" } else { "cache" }),
            )
            .env("TOKCTL_CLAUDE_DIR", root.join("claude"))
            .env("TOKCTL_CODEX_DIR", root.join("codex"))
            .env("TOKCTL_CURSOR_DIR", root.join("cursor"))
            .env("TZ", "UTC")
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("CODEX_HOME");
        if no_cache {
            cmd.arg("--no-cache");
        }
        let output = cmd.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            output.stderr.is_empty(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
}

#[test]
fn daily_and_monthly_match_independent_expected_totals() {
    let fixture = Fixture::new();
    for no_cache in [true, false] {
        assert_eq!(
            fixture.report("daily", no_cache, &[]),
            json!([{"date":"2026-04-22","source":"all","input":250,"output":100,"cache_read":550,"cache_write":200,"totalTokens":1100,"costUsd":0.0031}])
        );
        assert_eq!(
            fixture.report("monthly", no_cache, &[]),
            json!([{"month":"2026-04","source":"all","input":250,"output":100,"cache_read":550,"cache_write":200,"totalTokens":1100,"costUsd":0.0031}])
        );
    }
    assert!(!fixture.home.path().join("unused-cache").exists());
}

#[test]
fn sessions_keep_source_qualified_identity_and_metadata() {
    let fixture = Fixture::new();
    let expected = json!([
        {"session_id":"same","source":"codex","project_path":null,"latest_timestamp":"2026-04-22T12:00:01+00:00","input":150,"output":60,"cache_read":50,"cache_write":0,"totalTokens":260,"costUsd":0.0013},
        {"session_id":"same","source":"claude","project_path":null,"latest_timestamp":"2026-04-22T12:00:00+00:00","input":100,"output":40,"cache_read":500,"cache_write":200,"totalTokens":840,"costUsd":0.0018}
    ]);
    assert_eq!(fixture.report("session", true, &[]), expected);
    assert_eq!(fixture.report("session", false, &[]), expected);
}

#[test]
fn source_and_inclusive_date_filters_match_both_stores() {
    let fixture = Fixture::new();
    for no_cache in [true, false] {
        assert_eq!(
            fixture.report(
                "daily",
                no_cache,
                &[
                    "--source",
                    "codex",
                    "--since",
                    "2026-04-22",
                    "--until",
                    "2026-04-22"
                ]
            ),
            json!([{"date":"2026-04-22","input":150,"output":60,"cache_read":50,"cache_write":0,"totalTokens":260,"costUsd":0.0013}])
        );
        assert_eq!(
            fixture.report("daily", no_cache, &["--until", "2026-04-21"]),
            json!([])
        );
    }
}

#[test]
fn comparison_uses_same_calendar_scope_and_source_session_counts() {
    let fixture = Fixture::new();
    let args = ["2026-04-22..2026-04-22", "2026-04-21..2026-04-21"];
    let ephemeral = fixture.report("compare", true, &args);
    let persistent = fixture.report("compare", false, &args);
    assert_eq!(ephemeral, persistent);
    assert_eq!(persistent["summary"]["current"]["total_tokens"], 1100);
    assert_eq!(persistent["summary"]["baseline"]["total_tokens"], 0);
    assert_eq!(
        persistent["breakdowns"]["session"]["positive"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}
