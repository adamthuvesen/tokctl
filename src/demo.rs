use anyhow::{bail, Context, Result};
use chrono::{DateTime, Datelike, Duration, Local, NaiveTime, TimeZone, Utc};
use rusqlite::Connection;
use std::path::{Path, PathBuf};

use crate::pricing;
use crate::store::db::open_store;
use crate::store::writes::{insert_events, upsert_file_manifest, upsert_repo, EventRow};
use crate::store::{writes::RepoRow, FileManifestRow};
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
        model: "cursor-usage",
        weight: 2,
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
    let cache_path = cache_dir.join("cache.db");
    if cache_path.exists() && !overwrite {
        bail!(
            "{} already exists; pass --overwrite to replace this demo cache",
            cache_path.display()
        );
    }

    let mut conn = open_store(&cache_path)?;
    clear_demo_cache(&conn)?;
    let events = build_demo_events();
    let now_ns = Utc::now().timestamp_nanos_opt().unwrap_or(0);

    let tx = conn.transaction()?;
    for repo in REPOS {
        upsert_repo(
            &tx,
            &RepoRow {
                key: repo.key.to_owned(),
                display_name: repo.name.to_owned(),
                origin_url: Some(repo.origin.to_owned()),
                first_seen: events
                    .iter()
                    .filter(|event| event.repo.as_deref() == Some(repo.key))
                    .map(|event| event.ts)
                    .min()
                    .unwrap_or_else(|| Utc::now().timestamp_millis()),
            },
        )?;
    }
    for source in [Source::Claude, Source::Codex, Source::Cursor] {
        upsert_file_manifest(
            &tx,
            &FileManifestRow {
                path: PathBuf::from(format!("/demo/logs/{}.jsonl", source.as_str())),
                source,
                project: Some("demo".to_owned()),
                size: 64_000,
                mtime_ns: now_ns,
                last_offset: 64_000,
                n_events: events.iter().filter(|event| event.source == source).count() as u64,
                session_id: None,
                model: None,
            },
        )?;
    }
    let event_count = insert_events(&tx, &events)?;
    tx.commit()?;

    Ok(DemoSeedResult {
        cache_dir: cache_dir.to_path_buf(),
        cache_path,
        events: event_count,
        repos: REPOS.len(),
    })
}

fn clear_demo_cache(conn: &Connection) -> Result<()> {
    conn.execute("DELETE FROM events", [])
        .context("clearing demo events")?;
    conn.execute("DELETE FROM files", [])
        .context("clearing demo file manifest")?;
    conn.execute("DELETE FROM repos", [])
        .context("clearing demo repo rows")?;
    Ok(())
}

fn build_demo_events() -> Vec<EventRow> {
    let today = Local::now().date_naive();
    let now = Utc::now();
    let mut rows = Vec::new();
    let mut session_counter = 1000_u64;

    for day_back in (0..35).rev() {
        let day = today - Duration::days(day_back);
        if day.weekday().number_from_monday() >= 6 && day_back % 3 != 0 {
            continue;
        }

        let day_intensity = 1 + ((35 - day_back) % 5) as u64;
        let sessions_today = 2 + ((day_back + 1) % 4) as u64;
        for session_idx in 0..sessions_today {
            let repo_idx = ((day_back as usize) + (session_idx as usize * 2)) % REPOS.len();
            let source = weighted_source(day_back as u64 + session_idx);
            let hour = 8 + ((session_idx * 3 + day_intensity) % 11) as u32;
            let minute = (session_idx * 13 % 60) as u32;
            let ts = local_ts(day, hour, minute);
            session_counter += 1;
            let session_id = format!(
                "{}-demo-{}",
                source.source.as_str(),
                base36(session_counter)
            );

            let turns = 2 + ((day_back as u64 + session_idx) % 4);
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
    let input = 18_000 + intensity * 4_400 + turn * 2_350;
    let output = 2_400 + intensity * 950 + turn * 875;
    let cache_read = match source.source {
        Source::Cursor => 0,
        _ => 8_000 + intensity * 3_200 + turn * 1_000,
    };
    let cache_write = match source.source {
        Source::Claude => 1_500 + intensity * 450,
        _ => 0,
    };
    let cost = match source.source {
        Source::Cursor => {
            // Cursor CSVs carry explicit costs, so mirror that in the demo.
            ((input + output) as f64 / 1_000_000.0) * 18.0
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

        assert_eq!(result.repos, 5);
        assert!(result.events > 300);
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

        assert_eq!(repos, 5);
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
}
