use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};

const ROOT: &str = env!("CARGO_MANIFEST_DIR");
const SYNTHETIC_HOMES: &[&str] = &["dev", "me", "foo", "user", "demo", "test", "example"];

fn files_under(dirs: &[&str]) -> Vec<(PathBuf, String)> {
    let root = Path::new(ROOT);
    dirs.iter()
        .flat_map(|dir| walkdir::WalkDir::new(root.join(dir)))
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter_map(|entry| {
            let text = fs::read_to_string(entry.path()).ok()?;
            Some((entry.path().strip_prefix(root).unwrap().to_path_buf(), text))
        })
        .collect()
}

#[test]
fn fixtures_contain_no_real_identities() {
    let home = Regex::new(r"/(?:Users|home)/([A-Za-z0-9._-]+)").unwrap();
    let email = Regex::new(r"([A-Za-z0-9._%+-]+)@([A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+)").unwrap();
    let uuid = Regex::new(r"(?i)\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b")
        .unwrap();
    let synthetic_domain =
        Regex::new(r"(?i)(^|\.)(example(\.(com|org|net))?|test|invalid)$").unwrap();
    let token = Regex::new(r"eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}|user_[0-9A-Z]{26}").unwrap();
    let local_user = std::env::var("USER").unwrap_or_default();
    let mut found = Vec::new();
    for (path, text) in files_under(&["src", "tests", "benches", "examples"]) {
        for (number, line) in text.lines().enumerate() {
            let at = format!("{}:{}", path.display(), number + 1);
            for m in home.captures_iter(line) {
                if !SYNTHETIC_HOMES.contains(&&m[1]) {
                    found.push(format!(
                        "{at}: home path {:?}; use /Users/dev or a tempdir",
                        &m[0]
                    ));
                }
            }
            for m in email.captures_iter(line) {
                if &m[1] != "git" && !synthetic_domain.is_match(&m[2]) {
                    found.push(format!(
                        "{at}: email {:?}; use a @example.com address",
                        &m[0]
                    ));
                }
            }
            for m in token.find_iter(line) {
                found.push(format!(
                    "{at}: {:?} looks like a real Cursor credential or account ID",
                    m.as_str()
                ));
            }
            for m in uuid.find_iter(line) {
                if !m.as_str().starts_with("00000000-0000-0000-0000-") {
                    found.push(format!(
                        "{at}: UUID {:?} looks like a real session or account ID; use 00000000-0000-0000-0000-<counter>",
                        m.as_str()
                    ));
                }
            }
            if local_user.len() >= 3
                && !SYNTHETIC_HOMES.contains(&local_user.as_str())
                && line.contains(&local_user)
            {
                found.push(format!(
                    "{at}: contains the local user name; use a synthetic name"
                ));
            }
        }
    }
    assert!(
        found.is_empty(),
        "fixtures must be synthetic (AGENTS.md):\n{}",
        found.join("\n")
    );
}

#[test]
fn doctor_stays_offline() {
    let banned = [
        "reqwest",
        "BlockingCursorApi",
        "validate_",
        "sync_active_account",
    ];
    let mut found = Vec::new();
    for (path, text) in files_under(&["src/doctor", "src/store/inspection.rs"]) {
        for token in banned.iter().filter(|token| text.contains(**token)) {
            found.push(format!("{}: uses {token}", path.display()));
        }
    }
    assert!(
        found.is_empty(),
        "doctor must stay offline; move network work to cursor_sync callers:\n{}",
        found.join("\n")
    );
}

#[test]
fn fast_lane_runs_in_pre_commit_and_ci() {
    for (file, line) in [
        (".pre-commit-config.yaml", "entry: just check"),
        (".github/workflows/ci.yml", "run: just check"),
    ] {
        let text = fs::read_to_string(Path::new(ROOT).join(file)).unwrap();
        assert!(
            text.lines().any(|l| l.trim() == line),
            "{file} must run the fast lane with `{line}`"
        );
    }
}

#[test]
fn connection_lint_allows_stay_in_store() {
    let allowed = [
        "src/store/mod.rs",
        "tests/cli_behavior.rs",
        "tests/ingest_safety.rs",
    ];
    let stray: Vec<_> = files_under(&["src", "tests", "benches", "examples"])
        .into_iter()
        .filter(|(path, text)| {
            text.contains(&format!("clippy::{}", "disallowed_types"))
                && !allowed.contains(&path.to_str().unwrap())
        })
        .map(|(path, _)| path)
        .collect();
    assert!(
        stray.is_empty(),
        "only src/store/ may allow rusqlite::Connection; use crate::store::Store in {stray:?}"
    );
}

#[test]
fn enforced_rules_table_cites_real_enforcers() {
    let agents = fs::read_to_string(Path::new(ROOT).join("AGENTS.md")).unwrap();
    let section = agents
        .split("\n## Enforced Rules\n")
        .nth(1)
        .expect("AGENTS.md needs an `## Enforced Rules` table");
    let section = section.split("\n## ").next().unwrap();
    let cited = Regex::new(r"`([^`]+)`").unwrap();
    let rows: Vec<_> = section
        .lines()
        .filter(|l| l.starts_with('|'))
        .skip(2)
        .collect();
    assert!(
        !rows.is_empty(),
        "AGENTS.md Enforced Rules table has no rows"
    );
    let mut problems = Vec::new();
    for row in rows {
        let enforced_by = row.split('|').nth(2).unwrap_or_default();
        let names: Vec<_> = cited
            .captures_iter(enforced_by)
            .map(|c| c[1].to_string())
            .collect();
        if names.is_empty() {
            problems.push(format!("row cites nothing: {row}"));
        }
        for citation in names {
            let defined = citation.split_once("::").is_some_and(|(file, name)| {
                fs::read_to_string(Path::new(ROOT).join(file)).is_ok_and(|text| {
                    text.contains(&format!("fn {name}("))
                        || text.contains(&format!("def {name}("))
                        || text.lines().any(|line| {
                            let rest = line.trim_start().strip_prefix(name).unwrap_or("#");
                            rest.trim_start().starts_with(['=', ':'])
                        })
                })
            });
            if !defined {
                problems.push(format!(
                    "`{citation}` is not `path::name` defined in that file"
                ));
            }
        }
    }
    assert!(
        problems.is_empty(),
        "fix the AGENTS.md Enforced Rules rows:\n{}",
        problems.join("\n")
    );
}
