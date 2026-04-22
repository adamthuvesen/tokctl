use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::{Path, PathBuf};

use super::schema::{DDL, SCHEMA_VERSION};
use crate::repo::Resolver;

/// Resolve the default cache file path.
///
/// Priority:
/// 1. `TOKCTL_CACHE_DIR` env var (appended with `/cache.db`)
/// 2. `XDG_DATA_HOME/tokctl/cache.db`
/// 3. `~/.local/share/tokctl/cache.db`
pub fn path() -> PathBuf {
    if let Some(dir) = std::env::var_os("TOKCTL_CACHE_DIR") {
        return PathBuf::from(dir).join("cache.db");
    }
    if let Some(base) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(base).join("tokctl").join("cache.db");
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    home.join(".local")
        .join("share")
        .join("tokctl")
        .join("cache.db")
}

fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")
        .context("applying pragmas")?;
    conn.execute_batch(DDL).context("applying DDL")?;
    conn.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES (?, ?)",
        rusqlite::params!["schema_version", SCHEMA_VERSION.to_string()],
    )
    .context("writing schema version")?;
    Ok(())
}

fn read_schema_version(conn: &Connection) -> Option<i64> {
    conn.query_row(
        "SELECT value FROM meta WHERE key = 'schema_version'",
        [],
        |row| row.get::<_, String>(0),
    )
    .ok()
    .and_then(|v| v.parse::<i64>().ok())
}

/// Open the SQLite store, creating the parent directory and schema as needed.
/// Returns an error if the stored schema version is greater than the current
/// version (indicating a newer binary wrote this database).
pub fn open_store(db_path: &Path) -> Result<Connection> {
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }

    let mut conn =
        Connection::open(db_path).with_context(|| format!("opening {}", db_path.display()))?;

    // Ensure meta table exists so we can read schema_version
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
    )?;

    let current = read_schema_version(&conn);
    match current {
        None => {
            // Fresh DB: write full schema
            init_schema(&conn)?;
        }
        Some(v) if v == SCHEMA_VERSION => {
            // Ensure pragmas + DDL are idempotently applied (indexes, etc.)
            conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")?;
            conn.execute_batch(DDL)?;
        }
        Some(2) => {
            // In-place migration from v2 → v3: add the `repo` column, create
            // the `repos` table, and backfill by resolving each distinct
            // `project_path` already in the cache. No JSONL re-parse needed.
            migrate_v2_to_v3(&mut conn).with_context(|| {
                "migrating cache from v2 to v3 failed; try running with --rebuild"
            })?;
        }
        Some(v) if v < SCHEMA_VERSION => {
            // Older (pre-v2) caches: drop and rebuild, the fallback we've
            // always taken for ancient schemas.
            drop(conn);
            std::fs::remove_file(db_path).ok();
            let conn = Connection::open(db_path)?;
            init_schema(&conn)?;
            return Ok(conn);
        }
        Some(v) => {
            anyhow::bail!(
                "cache database was created by a newer version of tokctl (schema {}, expected {})",
                v,
                SCHEMA_VERSION
            );
        }
    }

    Ok(conn)
}

/// Migrate a v2 cache in place. All DDL and backfill writes run in a single
/// transaction; on failure the transaction rolls back and `schema_version`
/// stays at 2 so the user can retry (or run `--rebuild`).
fn migrate_v2_to_v3(conn: &mut Connection) -> Result<()> {
    conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")?;

    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;

    // `ALTER TABLE ... ADD COLUMN` cannot run inside certain transaction
    // configurations on older SQLite builds, but rusqlite's bundled 3.x
    // handles it inside an immediate tx just fine.
    tx.execute_batch(
        r#"
        ALTER TABLE events ADD COLUMN repo TEXT;
        CREATE TABLE IF NOT EXISTS repos (
          key          TEXT PRIMARY KEY,
          display_name TEXT NOT NULL,
          origin_url   TEXT,
          first_seen   INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_events_repo ON events(repo);
        "#,
    )?;

    // Collect distinct (project_path, min_ts) pairs so we can stamp each
    // repo's `first_seen` with the timestamp of its earliest event.
    let mut stmt = tx.prepare(
        "SELECT project_path, MIN(ts) FROM events \
         WHERE project_path IS NOT NULL \
         GROUP BY project_path",
    )?;
    let rows: Vec<(String, i64)> = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?
        .filter_map(std::result::Result::ok)
        .collect();
    drop(stmt);

    let mut resolver = Resolver::new();
    // Track the earliest first_seen per repo across multiple project_paths.
    let mut first_seen: std::collections::HashMap<String, i64> = std::collections::HashMap::new();

    for (project_path, min_ts) in &rows {
        let id = resolver.resolve(project_path);
        if let Some(key) = id.key.as_deref() {
            tx.execute(
                "UPDATE events SET repo = ?1 WHERE project_path = ?2",
                rusqlite::params![key, project_path],
            )?;
            first_seen
                .entry(key.to_owned())
                .and_modify(|t| {
                    if *min_ts < *t {
                        *t = *min_ts;
                    }
                })
                .or_insert(*min_ts);
        }
    }

    for (key, id) in resolver.resolved_repos() {
        let ts = first_seen.get(key).copied().unwrap_or(0);
        tx.execute(
            "INSERT INTO repos (key, display_name, origin_url, first_seen) \
             VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(key) DO UPDATE SET \
               display_name = excluded.display_name, \
               origin_url   = excluded.origin_url",
            rusqlite::params![key, id.display_name, id.origin_url, ts],
        )?;
    }

    tx.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES (?, ?)",
        rusqlite::params!["schema_version", SCHEMA_VERSION.to_string()],
    )?;

    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_db_gets_schema_version() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        assert_eq!(read_schema_version(&conn), Some(SCHEMA_VERSION));
    }

    #[test]
    fn open_creates_schema() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("nested").join("cache.db");
        let conn = open_store(&db_path).unwrap();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM repos", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn migrates_v2_cache_to_v3() {
        // Build a v2 DB manually: old schema + sample events, then open.
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("cache.db");
        let repo_root = dir.path().join("r");
        std::fs::create_dir_all(repo_root.join(".git")).unwrap();
        let sub = repo_root.join("src");
        std::fs::create_dir_all(&sub).unwrap();
        let canon_root = std::fs::canonicalize(&repo_root).unwrap();
        let canon_sub = std::fs::canonicalize(&sub).unwrap();

        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch(
                r#"
                CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                CREATE TABLE events (
                  id INTEGER PRIMARY KEY,
                  file_path TEXT NOT NULL,
                  source TEXT NOT NULL,
                  ts INTEGER NOT NULL,
                  day TEXT NOT NULL,
                  month TEXT NOT NULL,
                  session_id TEXT NOT NULL,
                  project_path TEXT,
                  model TEXT NOT NULL,
                  input INTEGER NOT NULL,
                  output INTEGER NOT NULL,
                  cache_read INTEGER NOT NULL,
                  cache_write INTEGER NOT NULL,
                  cost_usd REAL NOT NULL
                );
                CREATE TABLE files (
                  path TEXT PRIMARY KEY, source TEXT NOT NULL, project TEXT,
                  size INTEGER NOT NULL, mtime_ns INTEGER NOT NULL,
                  last_offset INTEGER NOT NULL DEFAULT 0,
                  n_events INTEGER NOT NULL DEFAULT 0,
                  session_id TEXT, model TEXT
                );
                "#,
            )
            .unwrap();
            conn.execute(
                "INSERT INTO meta (key, value) VALUES ('schema_version', '2')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO events (file_path, source, ts, day, month, session_id, \
                 project_path, model, input, output, cache_read, cache_write, cost_usd) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    "/a.jsonl",
                    "claude",
                    1000i64,
                    "2026-04-20",
                    "2026-04",
                    "sess-a",
                    canon_sub.to_str().unwrap(),
                    "claude-sonnet-4-6",
                    1i64,
                    2i64,
                    3i64,
                    4i64,
                    0.1f64
                ],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO events (file_path, source, ts, day, month, session_id, \
                 project_path, model, input, output, cache_read, cache_write, cost_usd) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    "/b.jsonl",
                    "codex",
                    500i64,
                    "2026-04-20",
                    "2026-04",
                    "sess-b",
                    None::<String>,
                    "gpt-5.4",
                    5i64,
                    6i64,
                    0i64,
                    0i64,
                    0.2f64
                ],
            )
            .unwrap();
        }

        let conn = open_store(&db_path).unwrap();

        // schema_version bumped
        assert_eq!(read_schema_version(&conn), Some(3));
        // events.repo backfilled for the resolvable row
        let repo_for_a: Option<String> = conn
            .query_row(
                "SELECT repo FROM events WHERE file_path = '/a.jsonl'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(repo_for_a.as_deref(), Some(canon_root.to_str().unwrap()));
        // events with NULL project_path remain NULL
        let repo_for_b: Option<String> = conn
            .query_row(
                "SELECT repo FROM events WHERE file_path = '/b.jsonl'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(repo_for_b.is_none());
        // repos table has exactly one row
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM repos", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
        let (k, name, first): (String, String, i64) = conn
            .query_row("SELECT key, display_name, first_seen FROM repos", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .unwrap();
        assert_eq!(k, canon_root.to_str().unwrap());
        assert_eq!(name, "r");
        assert_eq!(first, 1000);
    }
}
