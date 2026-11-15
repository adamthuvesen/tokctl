use super::Store;
use crate::pricing;
use crate::store::db::ProvenanceStatus;
use crate::store::schema::SCHEMA_VERSION;
use crate::types::Source;
use anyhow::Result;
use rusqlite::Connection;
use std::path::Path;

#[derive(Debug)]
pub(crate) struct CacheFacts {
    pub(crate) schema_version: i64,
    pub(crate) event_count: u64,
    pub(crate) file_count: u64,
    /// Events whose source file a complete scan no longer found. Only read when the schema is current.
    pub(crate) missing_source_event_count: u64,
    pub(crate) repo_count: u64,
    pub(crate) latest_file_mtime_ns: Option<i64>,
    pub(crate) unpriced_models: Vec<String>,
    /// Only read when the schema is current.
    pub(crate) provenance: Option<ProvenanceStatus>,
}

#[derive(Debug)]
pub(crate) enum CacheFault {
    Missing,
    Open(String),
    SchemaNewer(i64),
    SchemaMissing,
    Table { table: &'static str, error: String },
    ModelQuery(String),
    ModelRow(String),
    Provenance(String),
}

pub(crate) fn inspect_cache(path: &Path) -> std::result::Result<CacheFacts, CacheFault> {
    if !path.exists() {
        return Err(CacheFault::Missing);
    }

    let store =
        Store::open_for_inspection(path).map_err(|err| CacheFault::Open(err.to_string()))?;
    let conn = store.connection();

    let schema_version = match read_schema_version(conn) {
        Some(v) if v > SCHEMA_VERSION => return Err(CacheFault::SchemaNewer(v)),
        Some(v) => v,
        None => return Err(CacheFault::SchemaMissing),
    };

    let read_count = |table: &'static str| {
        count_table(conn, table).map_err(|error| CacheFault::Table {
            table,
            error: error.to_string(),
        })
    };
    let file_count = read_count("files")?;
    let event_count = read_count("events")?;
    let repo_count = read_count("repos")?;
    let unpriced_models = unknown_priced_models(conn)?;
    let missing_source_event_count = if schema_version == SCHEMA_VERSION {
        conn.query_row(
            "SELECT COUNT(*) FROM events WHERE file_path IN (SELECT path FROM files WHERE missing_since IS NOT NULL)",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| CacheFault::Table {
            table: "files",
            error: error.to_string(),
        })?
        .max(0) as u64
    } else {
        0
    };
    let provenance = if schema_version == SCHEMA_VERSION {
        Some(
            crate::store::db::provenance_status(conn)
                .map_err(|error| CacheFault::Provenance(error.to_string()))?,
        )
    } else {
        None
    };

    Ok(CacheFacts {
        schema_version,
        event_count,
        file_count,
        missing_source_event_count,
        repo_count,
        latest_file_mtime_ns: max_file_mtime_ns(conn),
        unpriced_models,
        provenance,
    })
}

fn read_schema_version(conn: &Connection) -> Option<i64> {
    conn.query_row(
        "SELECT value FROM meta WHERE key = 'schema_version'",
        [],
        |row| row.get::<_, String>(0),
    )
    .ok()
    .and_then(|value| value.parse::<i64>().ok())
}

fn count_table(conn: &Connection, table: &str) -> Result<u64> {
    let sql = format!("SELECT COUNT(*) FROM {table}");
    let count: i64 = conn.query_row(&sql, [], |row| row.get(0))?;
    Ok(count.max(0) as u64)
}

fn max_file_mtime_ns(conn: &Connection) -> Option<i64> {
    conn.query_row("SELECT MAX(mtime_ns) FROM files", [], |row| {
        row.get::<_, Option<i64>>(0)
    })
    .ok()
    .flatten()
}

pub(crate) fn unknown_priced_models(
    conn: &Connection,
) -> std::result::Result<Vec<String>, CacheFault> {
    let mut stmt = conn
        .prepare("SELECT DISTINCT model, source FROM events ORDER BY model")
        .map_err(|err| CacheFault::ModelQuery(err.to_string()))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|err| CacheFault::ModelQuery(err.to_string()))?;
    let mut unknown = Vec::new();
    for row in rows {
        let (model, source) = row.map_err(|err| CacheFault::ModelRow(err.to_string()))?;
        if source != Source::Cursor.as_str() && !pricing::has_price(&model) {
            unknown.push(model);
        }
    }
    unknown.sort();
    unknown.dedup();
    Ok(unknown)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::schema::DDL;

    #[test]
    fn missing_cache_is_warning_and_not_created() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.db");
        assert!(matches!(inspect_cache(&path), Err(CacheFault::Missing)));
        assert!(!path.exists());
    }

    #[test]
    fn newer_schema_is_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             INSERT INTO meta (key, value) VALUES ('schema_version', '999');",
        )
        .unwrap();
        drop(conn);

        assert!(matches!(
            inspect_cache(&path),
            Err(CacheFault::SchemaNewer(999))
        ));
    }

    #[test]
    fn missing_table_is_an_inspection_error_not_zero_events() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(DDL).unwrap();
        conn.execute(
            "INSERT INTO meta VALUES('schema_version', ?)",
            [crate::store::schema::SCHEMA_VERSION.to_string()],
        )
        .unwrap();
        conn.execute_batch("DROP TABLE events;").unwrap();
        drop(conn);
        match inspect_cache(&path) {
            Err(CacheFault::Table { table, error }) => {
                assert_eq!(table, "events");
                assert!(error.contains("no such table"));
            }
            other => panic!("expected a table fault, got {other:?}"),
        }
    }

    #[test]
    fn events_from_missing_logs_are_counted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(DDL).unwrap();
        conn.execute(
            "INSERT INTO meta VALUES('schema_version', ?)",
            [SCHEMA_VERSION.to_string()],
        )
        .unwrap();
        conn.execute_batch(
            "INSERT INTO files(path,source,size,mtime_ns,missing_since) VALUES('/gone','claude',1,1,5),('/here','claude',1,1,NULL);
             INSERT INTO events(file_path,source,ts,day,month,session_id,model,input,output,cache_read,cache_write,cost_usd)
             VALUES('/gone','claude',1,'2026-04-01','2026-04','s','m',1,0,0,0,0),('/gone','claude',2,'2026-04-01','2026-04','s','m',1,0,0,0,0),('/here','claude',3,'2026-04-01','2026-04','s','m',1,0,0,0,0);",
        )
        .unwrap();
        drop(conn);
        assert_eq!(inspect_cache(&path).unwrap().missing_source_event_count, 2);
    }

    #[test]
    fn unknown_priced_models_ignore_cursor() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(DDL).unwrap();
        conn.execute(
            "INSERT INTO events
             (file_path, source, ts, day, month, session_id, project_path, repo, model,
              input, output, cache_read, cache_write, cost_usd)
             VALUES
             ('/a', 'claude', 1, '2026-04-01', '2026-04', 's1', NULL, NULL, 'mystery', 1, 0, 0, 0, 0),
             ('/b', 'cursor', 1, '2026-04-01', '2026-04', 's2', NULL, NULL, 'composer-1', 1, 0, 0, 0, 1)",
            [],
        )
        .unwrap();
        let unknown = unknown_priced_models(&conn).unwrap();
        assert_eq!(unknown, vec!["mystery"]);
    }
}
