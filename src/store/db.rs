use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::{Path, PathBuf};

use super::schema::{DDL, SCHEMA_VERSION};

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

    let conn =
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
        Some(v) if v < SCHEMA_VERSION => {
            // Migration path: for v1 → v2 we drop and rebuild; events are derivable.
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
        // Table should exist
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
    }
}
