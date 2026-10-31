use anyhow::Result;
use chrono::{DateTime, Datelike, Duration, Local, NaiveTime, TimeZone, Utc};
use std::path::{Path, PathBuf};

use crate::pricing;
#[cfg(test)]
use crate::store::db::{is_demo_cache, open_store};
use crate::store::EventRow;
use crate::store::Store;
use crate::store::{FileManifestRow, RepoRow};
use crate::types::{Source, UsageEvent};

#[derive(Debug, Clone)]
struct DemoRepo {
    key: &'static str,
    name: &'static str,
    origin: &'static str,
}

#[derive(Debug, Clone, Copy)]
struct DemoSource {
    source: Source,
    model: &'static str,
    weight: u64,
}

const REPOS: &[DemoRepo] = &[
    DemoRepo {
        key: "/demo/workspaces/atlas-web",
        name: "atlas-web",
        origin: "https://github.com/example/atlas-web",
    },
    DemoRepo {
        key: "/demo/workspaces/agent-hub",
        name: "agent-hub",
        origin: "https://github.com/example/agent-hub",
    },
    DemoRepo {
        key: "/demo/workspaces/data-lab",
        name: "data-lab",
        origin: "https://github.com/example/data-lab",
    },
    DemoRepo {
        key: "/demo/workspaces/docs-site",
        name: "docs-site",
        origin: "https://github.com/example/docs-site",
    },
    DemoRepo {
        key: "/demo/workspaces/mobile-kit",
        name: "mobile-kit",
        origin: "https://github.com/example/mobile-kit",
    },
    DemoRepo {
        key: "/demo/workspaces/search-core",
        name: "search-core",
        origin: "https://github.com/example/search-core",
    },
    DemoRepo {
        key: "/demo/workspaces/billing-api",
        name: "billing-api",
        origin: "https://github.com/example/billing-api",
    },
    DemoRepo {
        key: "/demo/workspaces/infra-ops",
        name: "infra-ops",
        origin: "https://github.com/example/infra-ops",
    },
    DemoRepo {
        key: "/demo/workspaces/design-system",
        name: "design-system",
        origin: "https://github.com/example/design-system",
    },
    DemoRepo {
        key: "/demo/workspaces/analytics-warehouse",
        name: "analytics-warehouse",
        origin: "https://github.com/example/analytics-warehouse",
    },
];

const SOURCES: &[DemoSource] = &[
    DemoSource {
        source: Source::Claude,
        model: "claude-sonnet-4-6",
        weight: 5,
    },
    DemoSource {
        source: Source::Codex,
        model: "gpt-5.4-codex",
        weight: 4,
    },
    DemoSource {
        source: Source::Cursor,
        model: "composer-2.5",
        weight: 3,
    },
];

#[derive(Debug, Clone)]
pub struct DemoSeedResult {
    pub cache_dir: PathBuf,
    pub cache_path: PathBuf,
    pub events: usize,
    pub repos: usize,
}

pub fn seed_demo_cache(cache_dir: &Path, overwrite: bool) -> Result<DemoSeedResult> {
    seed_demo_cache_at(cache_dir, overwrite, Utc::now())
}

fn seed_demo_cache_at(
    cache_dir: &Path,
    overwrite: bool,
    now: DateTime<Utc>,
) -> Result<DemoSeedResult> {
    let cache_path = cache_dir.join("cache.db");
    let events = build_demo_events(now);
    let now_ns = now.timestamp_nanos_opt().unwrap_or(0);
    let repos: Vec<RepoRow> = REPOS
        .iter()
        .map(|repo| RepoRow {
            key: repo.key.to_owned(),
            display_name: repo.name.to_owned(),
            origin_url: Some(repo.origin.to_owned()),
            first_seen: events
                .iter()
                .filter(|event| event.repo.as_deref() == Some(repo.key))
                .map(|event| event.ts)
                .min()
                .unwrap_or_else(|| now.timestamp_millis()),
        })
        .collect();
    let files: Vec<FileManifestRow> = [Source::Claude, Source::Codex, Source::Cursor]
        .into_iter()
        .map(|source| FileManifestRow {
            path: PathBuf::from(format!("/demo/logs/{}.jsonl", source.as_str())),
            source,
            project: Some("demo".to_owned()),
            size: 384_000,
            mtime_ns: now_ns,
            last_offset: 384_000,
            file_identity: None,
            ctime_ns: None,
            n_events: events.iter().filter(|event| event.source == source).count() as u64,
            session_id: None,
            model: None,
        })
        .collect();
    let event_count = Store::seed_demo(&cache_path, overwrite, &events, &repos, &files)?;

    Ok(DemoSeedResult {
        cache_dir: cache_dir.to_path_buf(),
        cache_path,
        events: event_count,
        repos: REPOS.len(),
    })
}

fn build_demo_events(now: DateTime<Utc>) -> Vec<EventRow> {
    let today = now.with_timezone(&Local).date_naive();
    let mut rows = Vec::new();
    let mut session_counter = 1000_u64;

    for day_back in (0..35).rev() {
        let day = today - Duration::days(day_back);
        if day.weekday().number_from_monday() >= 6 && day_back % 3 != 0 {
            continue;
        }

        let day_intensity = 3 + ((35 - day_back) % 6) as u64;
        let sessions_today = 7 + ((day_back + 1) % 6) as u64;
        for session_idx in 0..sessions_today {
            let repo_idx = ((day_back as usize * 3) + (session_idx as usize * 2)) % REPOS.len();
            let source = weighted_source(day_back as u64 + session_idx * 2);
            let hour = 7 + ((session_idx * 2 + day_intensity) % 13) as u32;
            let minute = (session_idx * 13 % 60) as u32;
            let ts = local_ts(day, hour, minute);
            session_counter += 1;
            let session_id = format!(
                "{}-demo-{}",
                source.source.as_str(),
                base36(session_counter)
            );

            let turns = 4 + ((day_back as u64 + session_idx) % 5);
            for turn in 0..turns {
                let mut event_ts = ts + Duration::minutes((turn * 17) as i64);
                if event_ts > now {
                    let minutes_back = ((session_idx + 1) * 19 + (turn + 1) * 7) as i64;
                    event_ts = now - Duration::minutes(minutes_back);
                }
                rows.push(demo_event(
                    &session_id,
                    &REPOS[repo_idx],
                    source,
                    event_ts,
                    day_intensity,
                    turn,
                ));
            }
        }
    }

    rows
}

fn weighted_source(seed: u64) -> DemoSource {
    let total: u64 = SOURCES.iter().map(|source| source.weight).sum();
    let mut slot = seed % total;
    for source in SOURCES {
        if slot < source.weight {
            return *source;
        }
        slot -= source.weight;
    }
    SOURCES[0]
}

fn demo_event(
    session_id: &str,
    repo: &DemoRepo,
    source: DemoSource,
    ts: DateTime<Utc>,
    intensity: u64,
    turn: u64,
) -> EventRow {
    let input = 52_000 + intensity * 12_500 + turn * 8_750;
    let output = 7_500 + intensity * 2_200 + turn * 1_450;
    let cache_read = match source.source {
        Source::Cursor => 0,
        _ => 48_000 + intensity * 18_500 + turn * 6_250,
    };
    let cache_write = match source.source {
        Source::Claude => 7_500 + intensity * 1_800,
        _ => 0,
    };
    let cost = match source.source {
        Source::Cursor => {
            // Cursor CSVs carry explicit costs, so mirror that in the demo.
            ((input + output) as f64 / 1_000_000.0) * 22.0
        }
        _ => pricing::cost_of(
            &UsageEvent {
                source: source.source,
                timestamp: ts,
                session_id: session_id.to_owned(),
                project_path: Some(repo.key.to_owned()),
                model: source.model.to_owned(),
                input_tokens: input,
                output_tokens: output,
                cache_read_tokens: cache_read,
                cache_write_tokens: cache_write,
                explicit_cost_usd: None,
            },
            None,
        ),
    };

    let local = ts.with_timezone(&Local);
    EventRow {
        file_path: format!("/demo/logs/{}/{}.jsonl", source.source.as_str(), session_id),
        source: source.source,
        ts: ts.timestamp_millis(),
        day: local.format("%Y-%m-%d").to_string(),
        month: local.format("%Y-%m").to_string(),
        session_id: session_id.to_owned(),
        project_path: Some(repo.key.to_owned()),
        repo: Some(repo.key.to_owned()),
        model: source.model.to_owned(),
        input,
        output,
        cache_read,
        cache_write,
        cost_usd: cost,
        explicit_cost_usd: (source.source == Source::Cursor).then_some(cost),
        cost_provenance: if source.source == Source::Cursor {
            "source".to_owned()
        } else {
            pricing::signature()
        },
        message_id: None,
    }
}

fn local_ts(day: chrono::NaiveDate, hour: u32, minute: u32) -> DateTime<Utc> {
    let naive = day.and_time(NaiveTime::from_hms_opt(hour, minute, 0).unwrap());
    Local
        .from_local_datetime(&naive)
        .single()
        .unwrap_or_else(|| Local.from_local_datetime(&naive).earliest().unwrap())
        .with_timezone(&Utc)
}

fn base36(mut n: u64) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut out = Vec::new();
    loop {
        out.push(DIGITS[(n % 36) as usize] as char);
        n /= 36;
        if n == 0 {
            break;
        }
    }
    out.iter().rev().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_demo_cache_writes_public_synthetic_rows() {
        let dir = tempfile::tempdir().unwrap();

        let result = seed_demo_cache(dir.path(), false).unwrap();

        assert_eq!(result.repos, 10);
        assert!(result.events > 1_500);
        let conn = open_store(&result.cache_path).unwrap();
        let repos: i64 = conn
            .query_row("SELECT COUNT(*) FROM repos", [], |row| row.get(0))
            .unwrap();
        let events: i64 = conn
            .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
            .unwrap();
        let private_paths: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE project_path LIKE '/Users/%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let future_events: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE ts > ?1",
                [Utc::now().timestamp_millis()],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(repos, 10);
        assert_eq!(events as usize, result.events);
        assert_eq!(private_paths, 0);
        assert_eq!(future_events, 0);
    }

    #[test]
    fn seed_demo_cache_refuses_to_overwrite_without_flag() {
        let dir = tempfile::tempdir().unwrap();
        seed_demo_cache(dir.path(), false).unwrap();

        let err = seed_demo_cache(dir.path(), false).unwrap_err();

        assert!(err.to_string().contains("--overwrite"));
    }
    #[test]
    fn overwrite_refuses_normal_cache_without_changing_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.db");
        let conn = open_store(&path).unwrap();
        conn.execute(
            "INSERT INTO meta (key, value) VALUES ('private', 'preserved')",
            [],
        )
        .unwrap();
        drop(conn);
        let err = seed_demo_cache(dir.path(), true).unwrap_err();
        assert!(err.to_string().contains("unmarked cache"));
        let conn = open_store(&path).unwrap();
        let value: String = conn
            .query_row("SELECT value FROM meta WHERE key = 'private'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(value, "preserved");
        assert!(!is_demo_cache(&path).unwrap());
    }

    #[test]
    fn failed_reseed_rolls_back_clear_and_seed() {
        let dir = tempfile::tempdir().unwrap();
        let first = seed_demo_cache(dir.path(), false).unwrap();
        let conn = open_store(&first.cache_path).unwrap();
        conn.execute_batch("CREATE TRIGGER fail_demo_insert BEFORE INSERT ON events BEGIN SELECT RAISE(FAIL, 'injected seed failure'); END;").unwrap();
        drop(conn);
        assert!(seed_demo_cache(dir.path(), true).is_err());
        let conn = open_store(&first.cache_path).unwrap();
        let events: usize = conn
            .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
            .unwrap();
        let repos: usize = conn
            .query_row("SELECT COUNT(*) FROM repos", [], |row| row.get(0))
            .unwrap();
        assert_eq!(events, first.events);
        assert_eq!(repos, first.repos);
        assert!(is_demo_cache(&first.cache_path).unwrap());
    }

    #[test]
    fn injected_time_makes_seed_deterministic() {
        let now = DateTime::parse_from_rfc3339("2026-04-24T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let dir = tempfile::tempdir().unwrap();
        let first = seed_demo_cache_at(dir.path(), false, now).unwrap();
        let first_rows = build_demo_events(now);
        let second = seed_demo_cache_at(dir.path(), true, now).unwrap();
        let second_rows = build_demo_events(now);
        assert_eq!(first.events, second.events);
        assert_eq!(first_rows.len(), second_rows.len());
        assert_eq!(
            first_rows.iter().map(|row| row.ts).collect::<Vec<_>>(),
            second_rows.iter().map(|row| row.ts).collect::<Vec<_>>()
        );
        assert!(second_rows
            .iter()
            .all(|row| row.ts <= now.timestamp_millis()));
    }
}
