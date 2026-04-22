use crate::discovery::{discover_claude, discover_codex, DiscoverOpts, DiscoveredFile};
use crate::ingest::file_range::{ingest_claude_range, ingest_codex_range, RangeResult};
use crate::ingest::plan::{plan_ingest, IngestPlan, PlanInput};
use crate::pricing::cost_of;
use crate::repo::Resolver;
use crate::store::writes::{
    delete_file_and_events, insert_events, load_file_manifest, upsert_file_manifest, upsert_repo,
    EventRow, FileManifestRow, RepoRow,
};
use crate::types::{IngestStats, Source, UsageEvent};
use anyhow::Result;
use chrono::{DateTime, Datelike, Local, Utc};
use rayon::prelude::*;
use rusqlite::Connection;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct RunIngestOptions<'a> {
    pub conn: &'a mut Connection,
    pub claude_roots: Vec<PathBuf>,
    pub codex_roots: Vec<PathBuf>,
    pub include_claude: bool,
    pub include_codex: bool,
    pub safety_window_ms: i64,
    pub now_ms: i64,
}

/// Derive a local-time YYYY-MM-DD from an event timestamp (UTC DateTime).
pub fn local_day(ts: &DateTime<Utc>) -> String {
    let local = ts.with_timezone(&Local);
    format!(
        "{:04}-{:02}-{:02}",
        local.year(),
        local.month(),
        local.day()
    )
}

/// Derive a local-time YYYY-MM from an event timestamp (UTC DateTime).
pub fn local_month(ts: &DateTime<Utc>) -> String {
    let local = ts.with_timezone(&Local);
    format!("{:04}-{:02}", local.year(), local.month())
}

fn system_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Convert a parsed UsageEvent to a row ready for SQLite insert. Costs the
/// event against the pricing table, accumulating any unknown model IDs into
/// the provided set (which is thread-local during parallel parsing).
fn event_to_row(ev: &UsageEvent, file_path: &str, unknown: &mut HashSet<String>) -> EventRow {
    let cost = cost_of(ev, Some(unknown));
    EventRow {
        file_path: file_path.to_owned(),
        source: ev.source,
        ts: ev.timestamp.timestamp_millis(),
        day: local_day(&ev.timestamp),
        month: local_month(&ev.timestamp),
        session_id: ev.session_id.clone(),
        project_path: ev.project_path.clone(),
        repo: None,
        model: ev.model.clone(),
        input: ev.input_tokens,
        output: ev.output_tokens,
        cache_read: ev.cache_read_tokens,
        cache_write: ev.cache_write_tokens,
        cost_usd: cost,
    }
}

/// Result of parsing one file off the critical path. Carries everything the
/// serial writer needs to produce per-file transactions without re-parsing.
struct ParsedFile {
    file: DiscoveredFile,
    rows: Vec<EventRow>,
    first_session: Option<String>,
    first_model: Option<String>,
    skipped_lines: usize,
    unknown_models: HashSet<String>,
}

/// Parse a full or partial file range and produce insertable rows.
/// Runs on a rayon worker — no SQLite access here.
fn parse_file(file: DiscoveredFile, from_offset: u64) -> Result<ParsedFile> {
    let range = match file.source {
        Source::Claude => {
            ingest_claude_range(&file.path, file.project.as_deref(), from_offset, file.size)?
        }
        Source::Codex => ingest_codex_range(&file.path, from_offset, file.size)?,
    };
    let RangeResult {
        events,
        skipped_lines,
        ..
    } = range;

    let path_str = file.path.to_string_lossy().into_owned();
    let mut unknown = HashSet::new();
    let rows: Vec<EventRow> = events
        .iter()
        .map(|ev| event_to_row(ev, &path_str, &mut unknown))
        .collect();
    let first_session = events.first().map(|e| e.session_id.clone());
    let first_model = events.first().map(|e| e.model.clone());

    Ok(ParsedFile {
        file,
        rows,
        first_session,
        first_model,
        skipped_lines,
        unknown_models: unknown,
    })
}

pub fn run_ingest(opts: RunIngestOptions<'_>) -> Result<IngestStats> {
    let RunIngestOptions {
        conn,
        claude_roots,
        codex_roots,
        include_claude,
        include_codex,
        safety_window_ms,
        now_ms,
    } = opts;

    let now_ms = if now_ms > 0 { now_ms } else { system_now_ms() };

    let manifest = load_file_manifest(conn)?;

    let discover_opts = DiscoverOpts {
        safety_window_ms,
        now_ms,
    };

    let mut all = crate::discovery::Discovery::default();
    if include_claude {
        let d = discover_claude(&claude_roots, &manifest, discover_opts);
        all.files.extend(d.files);
        for p in d.unchanged_paths {
            all.unchanged_paths.insert(p);
        }
    }
    if include_codex {
        let d = discover_codex(&codex_roots, &manifest, discover_opts);
        all.files.extend(d.files);
        for p in d.unchanged_paths {
            all.unchanged_paths.insert(p);
        }
    }

    let plan = plan_ingest(PlanInput {
        manifest: &manifest,
        discovery: &all,
        safety_window_ms,
        now_ms,
    });

    execute_plan(conn, plan, manifest)
}

/// Apply the repo resolver to a batch of event rows, returning the set of
/// repo keys first seen in this batch along with the earliest timestamp per
/// key. Mutates `rows` in place to stamp `repo`.
fn stamp_repos(
    rows: &mut [EventRow],
    resolver: &mut Resolver,
    first_seen: &mut HashMap<String, i64>,
    no_repo_count: &mut usize,
) {
    for r in rows.iter_mut() {
        let Some(pp) = r.project_path.as_deref() else {
            *no_repo_count += 1;
            continue;
        };
        let id = resolver.resolve(pp);
        match id.key {
            Some(key) => {
                first_seen
                    .entry(key.clone())
                    .and_modify(|t| {
                        if r.ts < *t {
                            *t = r.ts;
                        }
                    })
                    .or_insert(r.ts);
                r.repo = Some(key);
            }
            None => {
                *no_repo_count += 1;
            }
        }
    }
}

fn execute_plan(
    conn: &mut Connection,
    plan: IngestPlan,
    manifest: HashMap<PathBuf, FileManifestRow>,
) -> Result<IngestStats> {
    let mut stats = IngestStats {
        files_scanned: plan.to_skip.len()
            + plan.to_tail.len()
            + plan.to_full_parse.len()
            + plan.to_purge.len(),
        files_skipped: plan.to_skip.len(),
        ..Default::default()
    };

    // ---- Purge (serial, fast) ----
    for p in &plan.to_purge {
        stats.files_purged += 1;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        delete_file_and_events(&tx, p.to_string_lossy().as_ref())?;
        tx.commit()?;
    }

    // ---- Parse phase (parallel) ----
    // rayon's par_iter spreads file parsing across worker threads. Each returns
    // a ParsedFile containing rows + thread-local unknown-model set + skipped
    // line count. No SQLite access happens on these threads.
    let full_parsed: Vec<ParsedFile> = plan
        .to_full_parse
        .into_par_iter()
        .map(|file| parse_file(file, 0))
        .collect::<Result<Vec<_>>>()?;

    let tail_parsed: Vec<(ParsedFile, u64)> = plan
        .to_tail
        .into_par_iter()
        .map(|item| {
            let from = item.from_offset;
            parse_file(item.file, from).map(|p| (p, from))
        })
        .collect::<Result<Vec<_>>>()?;

    // ---- Fold thread-local stats into the run-wide IngestStats ----
    for p in &full_parsed {
        stats.skipped_lines += p.skipped_lines;
        stats
            .unknown_models
            .extend(p.unknown_models.iter().cloned());
    }
    for (p, _) in &tail_parsed {
        stats.skipped_lines += p.skipped_lines;
        stats
            .unknown_models
            .extend(p.unknown_models.iter().cloned());
    }

    // ---- Resolve repo identity for every row in the serial phase ----
    // Parallel parsing doesn't touch the filesystem beyond the JSONL reads,
    // so we resolve here where we also write to SQLite. One Resolver per
    // run memoizes across files.
    let mut resolver = Resolver::new();
    let mut first_seen: HashMap<String, i64> = HashMap::new();

    // ---- Write phase (serial, per-file transactions) ----
    for mut parsed in full_parsed {
        stats.files_full_parsed += 1;
        let path_str = parsed.file.path.to_string_lossy().into_owned();
        let n_events = parsed.rows.len() as u64;

        stamp_repos(
            &mut parsed.rows,
            &mut resolver,
            &mut first_seen,
            &mut stats.events_no_repo,
        );

        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if manifest.contains_key(&parsed.file.path) {
            delete_file_and_events(&tx, &path_str)?;
        }
        let inserted = insert_events(&tx, &parsed.rows)?;
        stats.events_inserted += inserted;
        upsert_file_manifest(
            &tx,
            &FileManifestRow {
                path: parsed.file.path.clone(),
                source: parsed.file.source,
                project: parsed.file.project.clone(),
                size: parsed.file.size,
                mtime_ns: parsed.file.mtime_ns,
                last_offset: parsed.file.size,
                n_events,
                session_id: parsed.first_session,
                model: parsed.first_model,
            },
        )?;
        tx.commit()?;
    }

    for (mut parsed, _from_offset) in tail_parsed {
        stats.files_tailed += 1;
        let n_events_added = parsed.rows.len() as u64;
        let prev_n = manifest
            .get(&parsed.file.path)
            .map(|r| r.n_events)
            .unwrap_or(0);

        stamp_repos(
            &mut parsed.rows,
            &mut resolver,
            &mut first_seen,
            &mut stats.events_no_repo,
        );

        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let inserted = insert_events(&tx, &parsed.rows)?;
        stats.events_inserted += inserted;
        upsert_file_manifest(
            &tx,
            &FileManifestRow {
                path: parsed.file.path.clone(),
                source: parsed.file.source,
                project: parsed.file.project.clone(),
                size: parsed.file.size,
                mtime_ns: parsed.file.mtime_ns,
                last_offset: parsed.file.size,
                n_events: prev_n + n_events_added,
                session_id: parsed.first_session,
                model: parsed.first_model,
            },
        )?;
        tx.commit()?;
    }

    // ---- Upsert repos in a single final transaction ----
    if !first_seen.is_empty() {
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        for (key, id) in resolver.resolved_repos() {
            let ts = first_seen.get(key).copied().unwrap_or(0);
            upsert_repo(
                &tx,
                &RepoRow {
                    key: key.to_owned(),
                    display_name: id.display_name.clone(),
                    origin_url: id.origin_url.clone(),
                    first_seen: ts,
                },
            )?;
            stats.repos_resolved += 1;
        }
        tx.commit()?;
    }

    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_day_month_formats() {
        let ts: DateTime<Utc> = "2026-04-18T09:00:05Z".parse().unwrap();
        assert_eq!(local_day(&ts).len(), 10);
        assert_eq!(local_month(&ts).len(), 7);
    }
}
