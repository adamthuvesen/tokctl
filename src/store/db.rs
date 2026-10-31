use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
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
    let _lock = super::acquire_ingest_lock(db_path)?;
    open_store_locked(db_path)
}

fn open_store_locked(db_path: &Path) -> Result<Connection> {
    let mut conn =
        Connection::open(db_path).with_context(|| format!("opening {}", db_path.display()))?;
    let has_events: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'events')",
        [],
        |row| row.get(0),
    )?;
    match read_schema_version(&conn) {
        None if !has_events => init_schema(&conn)?,
        None => anyhow::bail!("cache schema version is missing; existing cache was preserved"),
        Some(version) if version > SCHEMA_VERSION => anyhow::bail!(
            "cache database was created by a newer version of tokctl (schema {}, expected {})",
            version,
            SCHEMA_VERSION
        ),
        Some(version) if version < 2 => {
            anyhow::bail!("unsupported legacy cache schema {version}; existing cache was preserved")
        }
        Some(2) => migrate_provenance(&mut conn).context("migrating legacy v2 cache")?,
        Some(version) if version < SCHEMA_VERSION => migrate_provenance(&mut conn)?,
        Some(_) => {
            conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")?;
            conn.execute_batch(DDL)?;
        }
    }
    reproject(&conn, &crate::calendar::Calendar::system()?)?;
    Ok(conn)
}

fn migrate_provenance(conn: &mut Connection) -> Result<()> {
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let columns = {
        let mut statement = tx.prepare("PRAGMA table_info(events)")?;
        let columns = statement
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        columns
    };
    if !columns.iter().any(|column| column == "repo") {
        migrate_v2_to_v3(&tx)?;
    }
    if !columns.iter().any(|column| column == "explicit_cost_usd") {
        tx.execute_batch("ALTER TABLE events ADD COLUMN explicit_cost_usd REAL;")?;
    }
    if !columns.iter().any(|column| column == "cost_provenance") {
        tx.execute_batch(
            "ALTER TABLE events ADD COLUMN cost_provenance TEXT NOT NULL DEFAULT 'legacy';",
        )?;
    }
    if !columns.iter().any(|column| column == "message_id") {
        tx.execute_batch("ALTER TABLE events ADD COLUMN message_id TEXT;")?;
    }
    let file_columns = {
        let mut statement = tx.prepare("PRAGMA table_info(files)")?;
        let columns = statement
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        columns
    };
    if !file_columns.iter().any(|column| column == "file_identity") {
        tx.execute_batch("ALTER TABLE files ADD COLUMN file_identity TEXT;")?;
    }
    if !file_columns.iter().any(|column| column == "ctime_ns") {
        tx.execute_batch("ALTER TABLE files ADD COLUMN ctime_ns INTEGER;")?;
    }
    tx.execute_batch(DDL)?;
    tx.execute(
        "INSERT OR REPLACE INTO meta(key,value) VALUES('schema_version',?1)",
        [SCHEMA_VERSION.to_string()],
    )?;
    tx.commit()?;
    Ok(())
}

pub fn parser_signature(source: crate::types::Source) -> String {
    use sha2::{Digest, Sha256};
    static SIGNATURES: std::sync::OnceLock<[String; 3]> = std::sync::OnceLock::new();
    let signatures = SIGNATURES.get_or_init(|| {
        [
            include_bytes!("../sources/claude.rs").as_slice(),
            include_bytes!("../sources/codex.rs").as_slice(),
            include_bytes!("../sources/cursor.rs").as_slice(),
        ]
        .map(|parser| {
            let mut hash = Sha256::new();
            hash.update(include_bytes!("../types.rs"));
            hash.update(include_bytes!("../sources/mod.rs"));
            hash.update(include_bytes!("../ingest/file_range.rs"));
            hash.update(include_bytes!("ingest_run.rs"));
            hash.update(include_bytes!("../discovery.rs"));
            hash.update(include_bytes!("../repo.rs"));
            hash.update(parser);
            format!("{:x}", hash.finalize())
        })
    });
    signatures[match source {
        crate::types::Source::Claude => 0,
        crate::types::Source::Codex => 1,
        crate::types::Source::Cursor => 2,
    }]
    .clone()
}

#[derive(Debug, Default)]
pub struct ProvenanceStatus {
    pub legacy_cost_rows: u64,
    pub stale_pricing_rows: u64,
    pub stale_parser_files: u64,
    pub stale_calendar: bool,
}

pub fn provenance_status(conn: &Connection) -> Result<ProvenanceStatus> {
    use rusqlite::OptionalExtension;
    let price = crate::pricing::signature();
    let legacy_cost_rows = conn.query_row(
        "SELECT COUNT(*) FROM events WHERE cost_provenance='legacy'",
        [],
        |row| row.get(0),
    )?;
    let stale_pricing_rows = conn.query_row(
        "SELECT COUNT(*) FROM events WHERE cost_provenance NOT IN ('legacy','source',?1)",
        [&price],
        |row| row.get(0),
    )?;
    let mut statement = conn.prepare(
        "SELECT f.source,p.signature FROM files f LEFT JOIN parser_provenance p ON p.path=f.path",
    )?;
    let signatures = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut stale_parser_files = 0;
    for (source, signature) in signatures {
        let source: crate::types::Source = source.parse().map_err(anyhow::Error::msg)?;
        if signature.as_deref() != Some(parser_signature(source).as_str()) {
            stale_parser_files += 1;
        }
    }
    let identity: Option<String> = conn
        .query_row(
            "SELECT value FROM meta WHERE key='calendar_signature'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let expected = crate::calendar::Calendar::system()?.identity();
    Ok(ProvenanceStatus {
        legacy_cost_rows,
        stale_pricing_rows,
        stale_parser_files,
        stale_calendar: identity.as_deref() != Some(expected.as_str()),
    })
}

pub(in crate::store) fn reproject(
    conn: &Connection,
    calendar: &crate::calendar::Calendar,
) -> Result<()> {
    use rusqlite::OptionalExtension;
    let price = crate::pricing::signature();
    let calendar_identity = calendar.identity();
    let metadata = |key: &str| -> Result<Option<String>> {
        Ok(conn
            .query_row("SELECT value FROM meta WHERE key=?1", [key], |row| {
                row.get(0)
            })
            .optional()?)
    };
    if metadata("pricing_signature")?.as_deref() == Some(&price)
        && metadata("calendar_signature")?.as_deref() == Some(&calendar_identity)
    {
        return Ok(());
    }
    let rows = {
        let mut statement = conn.prepare("SELECT id,source,ts,model,input,output,cache_read,cache_write,explicit_cost_usd,cost_provenance FROM events")?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, u64>(4)?,
                    row.get::<_, u64>(5)?,
                    row.get::<_, u64>(6)?,
                    row.get::<_, u64>(7)?,
                    row.get::<_, Option<f64>>(8)?,
                    row.get::<_, String>(9)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    let tx = rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)?;
    for (
        id,
        source,
        ts,
        model,
        input,
        output,
        cache_read,
        cache_write,
        explicit_cost_usd,
        provenance,
    ) in rows
    {
        let timestamp = chrono::DateTime::from_timestamp_millis(ts)
            .context("cached event timestamp is out of range")?;
        let (day, month) = calendar.local_day_month(timestamp);
        tx.execute(
            "UPDATE events SET day=?1,month=?2 WHERE id=?3",
            rusqlite::params![day, month, id],
        )?;
        if provenance != "legacy" {
            let event = crate::types::UsageEvent {
                source: source.parse().map_err(anyhow::Error::msg)?,
                timestamp,
                session_id: String::new(),
                project_path: None,
                model,
                input_tokens: input,
                output_tokens: output,
                cache_read_tokens: cache_read,
                cache_write_tokens: cache_write,
                explicit_cost_usd,
            };
            event.validate()?;
            tx.execute(
                "UPDATE events SET cost_usd=?1,cost_provenance=?2 WHERE id=?3",
                rusqlite::params![
                    crate::pricing::cost_of(&event, None),
                    crate::pricing::provenance(&event),
                    id
                ],
            )?;
        }
    }
    for (key, value) in [
        ("pricing_signature", price),
        ("calendar_signature", calendar_identity),
    ] {
        tx.execute(
            "INSERT OR REPLACE INTO meta(key,value) VALUES(?1,?2)",
            rusqlite::params![key, value],
        )?;
    }
    tx.execute("INSERT INTO meta(key,value) VALUES('generation','1') ON CONFLICT(key) DO UPDATE SET value=CAST(value AS INTEGER)+1", [])?;
    tx.commit()?;
    Ok(())
}

/// Migrate a v2 cache in place. All DDL and backfill writes run in a single
/// transaction; on failure the transaction rolls back and `schema_version`
/// stays at 2 so the user can retry (or run `--rebuild`).
fn migrate_v2_to_v3(tx: &rusqlite::Transaction<'_>) -> Result<()> {
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
        .collect::<rusqlite::Result<Vec<_>>>()?;
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
        rusqlite::params!["schema_version", "3"],
    )?;

    Ok(())
}

impl super::Store {
    pub(crate) fn seed_demo(
        path: &Path,
        overwrite: bool,
        events: &[super::writes::EventRow],
        repos: &[super::writes::RepoRow],
        files: &[super::writes::FileManifestRow],
    ) -> Result<usize> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let _lock = super::acquire_ingest_lock(path)?;
        if path.exists() && !overwrite {
            anyhow::bail!(
                "{} already exists; pass --overwrite to replace this demo cache",
                path.display()
            );
        }
        if path.exists() && !is_demo_cache(path)? {
            anyhow::bail!(
                "refusing to overwrite unmarked cache at {}; choose a new demo directory",
                path.display()
            );
        }
        let mut store = Self {
            connection: open_store_locked(path)?,
            path: Some(path.to_path_buf()),
            writable: true,
        };
        let tx = store.connection_mut().transaction()?;
        tx.execute("DELETE FROM events", [])?;
        tx.execute("DELETE FROM files", [])?;
        tx.execute("DELETE FROM repos", [])?;
        tx.execute("DELETE FROM parser_provenance", [])?;
        tx.execute("DELETE FROM ingest_diagnostics", [])?;
        tx.execute("INSERT OR REPLACE INTO meta(key,value) VALUES('demo_marker','tokctl synthetic demo v1')",[])?;
        for repo in repos {
            super::writes::upsert_repo(&tx, repo)?;
        }
        for file in files {
            super::writes::upsert_file_manifest(&tx, file)?;
            tx.execute(
                "INSERT INTO parser_provenance(path,signature) VALUES(?1,?2)",
                rusqlite::params![file.path.to_string_lossy(), parser_signature(file.source)],
            )?;
        }
        let count = super::writes::insert_events(&tx, events)?;
        tx.execute("INSERT INTO meta(key,value) VALUES('generation','1') ON CONFLICT(key) DO UPDATE SET value=CAST(value AS INTEGER)+1", [])?;
        tx.commit()?;
        Ok(count)
    }
}

pub(crate) fn is_demo_cache(path: &Path) -> Result<bool> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .context("inspecting existing demo cache")?;
    let has_meta: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'meta')",
        [],
        |row| row.get(0),
    )?;
    if !has_meta {
        return Ok(false);
    }
    let marker: Option<String> = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'demo_marker'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    Ok(marker.as_deref() == Some("tokctl synthetic demo v1"))
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
        assert_eq!(read_schema_version(&conn), Some(SCHEMA_VERSION));
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
    fn insert_cached_event(conn: &Connection, file: &str, explicit: Option<f64>, provenance: &str) {
        conn.execute("INSERT INTO events(file_path,source,ts,day,month,session_id,model,input,output,cache_read,cache_write,cost_usd,explicit_cost_usd,cost_provenance) VALUES(?1,'claude',1777073400000,'old-day','old-month','session','claude-sonnet-4-6',1000000,0,0,0,123,?2,?3)", rusqlite::params![file,explicit,provenance]).unwrap();
    }

    #[test]
    fn migration_preserves_legacy_cost_without_claiming_current_provenance() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.db");
        let conn = Connection::open(&path).unwrap();
        let legacy_ddl = DDL
            .replace("  explicit_cost_usd REAL,", "")
            .replace("  cost_provenance TEXT NOT NULL DEFAULT 'legacy'", "")
            .replace(
                "  cost_usd     REAL NOT NULL,",
                "  cost_usd     REAL NOT NULL",
            );
        conn.execute_batch(&legacy_ddl).unwrap();
        conn.execute(
            "INSERT INTO meta(key,value) VALUES('schema_version','6')",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO events(file_path,source,ts,day,month,session_id,model,input,output,cache_read,cache_write,cost_usd) VALUES('/unavailable','claude',1000,'1970-01-01','1970-01','s','claude-sonnet-4-6',1000000,0,0,0,123)", []).unwrap();
        drop(conn);
        let conn = open_store(&path).unwrap();
        let (cost, explicit, provenance): (f64, Option<f64>, String) = conn
            .query_row(
                "SELECT cost_usd,explicit_cost_usd,cost_provenance FROM events",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(cost, 123.0);
        assert_eq!(explicit, None);
        assert_eq!(provenance, "legacy");
        assert_eq!(provenance_status(&conn).unwrap().legacy_cost_rows, 1);
        assert_eq!(read_schema_version(&conn), Some(SCHEMA_VERSION));
    }

    #[test]
    fn repricing_preserves_source_cost_and_recomputes_estimates() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        insert_cached_event(&conn, "/source", Some(0.75), "source");
        insert_cached_event(&conn, "/estimate", None, "old-price-signature");
        insert_cached_event(&conn, "/legacy", None, "legacy");
        reproject(&conn, &crate::calendar::Calendar::named("UTC").unwrap()).unwrap();
        let mut statement = conn
            .prepare("SELECT cost_usd FROM events ORDER BY id")
            .unwrap();
        let costs = statement
            .query_map([], |row| row.get::<_, f64>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(costs, vec![0.75, 3.0, 123.0]);
        let provenance: String = conn
            .query_row(
                "SELECT cost_provenance FROM events WHERE file_path='/estimate'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(provenance, crate::pricing::signature());
    }

    #[test]
    fn calendar_change_rebuilds_buckets_and_increments_generation() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        insert_cached_event(&conn, "/source", Some(0.75), "source");
        let timestamp = chrono::DateTime::parse_from_rfc3339("2026-04-24T23:30:00Z")
            .unwrap()
            .timestamp_millis();
        conn.execute("UPDATE events SET ts=?1", [timestamp])
            .unwrap();
        reproject(&conn, &crate::calendar::Calendar::named("UTC").unwrap()).unwrap();
        let before: String = conn
            .query_row("SELECT day FROM events", [], |row| row.get(0))
            .unwrap();
        reproject(
            &conn,
            &crate::calendar::Calendar::named("Europe/Stockholm").unwrap(),
        )
        .unwrap();
        let after: String = conn
            .query_row("SELECT day FROM events", [], |row| row.get(0))
            .unwrap();
        let generation: String = conn
            .query_row("SELECT value FROM meta WHERE key='generation'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(before, "2026-04-24");
        assert_eq!(after, "2026-04-25");
        assert_eq!(generation, "2");
    }

    #[test]
    fn unsupported_old_cache_is_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT NOT NULL);INSERT INTO meta VALUES('schema_version','1');CREATE TABLE preserved(value TEXT);INSERT INTO preserved VALUES('keep');").unwrap();
        drop(conn);
        assert!(open_store(&path).is_err());
        let conn = Connection::open(&path).unwrap();
        let value: String = conn
            .query_row("SELECT value FROM preserved", [], |row| row.get(0))
            .unwrap();
        assert_eq!(value, "keep");
    }

    #[test]
    fn failed_projection_rolls_back_all_changes() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        insert_cached_event(&conn, "/valid", Some(0.75), "source");
        insert_cached_event(&conn, "/invalid", None, "old-price-signature");
        conn.execute(
            "UPDATE events SET ts=?1 WHERE file_path='/invalid'",
            [i64::MAX],
        )
        .unwrap();
        assert!(reproject(&conn, &crate::calendar::Calendar::named("UTC").unwrap()).is_err());
        let (day, cost): (String, f64) = conn
            .query_row(
                "SELECT day,cost_usd FROM events WHERE file_path='/valid'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(day, "old-day");
        assert_eq!(cost, 123.0);
        assert!(conn
            .query_row(
                "SELECT value FROM meta WHERE key='pricing_signature'",
                [],
                |row| row.get::<_, String>(0)
            )
            .is_err());
    }
    #[test]
    fn failed_schema_migration_preserves_version_and_columns() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.db");
        let conn = Connection::open(&path).unwrap();
        let legacy_ddl = DDL
            .replace("  explicit_cost_usd REAL,", "")
            .replace("  cost_provenance TEXT NOT NULL DEFAULT 'legacy'", "")
            .replace(
                "  cost_usd     REAL NOT NULL,",
                "  cost_usd     REAL NOT NULL",
            );
        conn.execute_batch(&legacy_ddl).unwrap();
        conn.execute(
            "INSERT INTO meta(key,value) VALUES('schema_version','6')",
            [],
        )
        .unwrap();
        conn.execute_batch("CREATE TRIGGER fail_schema_version BEFORE INSERT ON meta WHEN NEW.key='schema_version' BEGIN SELECT RAISE(FAIL,'injected migration failure'); END;").unwrap();
        drop(conn);
        assert!(open_store(&path).is_err());
        let conn = Connection::open(&path).unwrap();
        assert_eq!(read_schema_version(&conn), Some(6));
        let columns: Vec<String> = conn
            .prepare("PRAGMA table_info(events)")
            .unwrap()
            .query_map([], |row| row.get(1))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert!(!columns.iter().any(|column| column == "explicit_cost_usd"));
        assert!(!columns.iter().any(|column| column == "cost_provenance"));
    }
}
