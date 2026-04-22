use crate::discovery::ManifestLike;
use crate::types::Source;
use anyhow::Result;
use rusqlite::{params, Connection, Transaction};
use std::collections::HashMap;
use std::path::PathBuf;
use std::str::FromStr;

#[derive(Debug, Clone)]
pub struct FileManifestRow {
    pub path: PathBuf,
    pub source: Source,
    pub project: Option<String>,
    pub size: u64,
    pub mtime_ns: i64,
    pub last_offset: u64,
    pub n_events: u64,
    pub session_id: Option<String>,
    pub model: Option<String>,
}

impl ManifestLike for FileManifestRow {
    fn mtime_ns(&self) -> i64 {
        self.mtime_ns
    }
}

#[derive(Debug, Clone)]
pub struct EventRow {
    pub file_path: String,
    pub source: Source,
    pub ts: i64,
    pub day: String,
    pub month: String,
    pub session_id: String,
    pub project_path: Option<String>,
    pub repo: Option<String>,
    pub model: String,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub cost_usd: f64,
}

#[derive(Debug, Clone)]
pub struct RepoRow {
    pub key: String,
    pub display_name: String,
    pub origin_url: Option<String>,
    pub first_seen: i64,
}

pub fn load_file_manifest(conn: &Connection) -> Result<HashMap<PathBuf, FileManifestRow>> {
    let mut stmt = conn.prepare(
        "SELECT path, source, project, size, mtime_ns, last_offset, n_events, session_id, model FROM files",
    )?;
    let rows = stmt.query_map([], |row| {
        let path: String = row.get(0)?;
        let source_str: String = row.get(1)?;
        let source = Source::from_str(&source_str).unwrap_or(Source::Claude);
        Ok(FileManifestRow {
            path: PathBuf::from(&path),
            source,
            project: row.get(2)?,
            size: row.get::<_, i64>(3)? as u64,
            mtime_ns: row.get(4)?,
            last_offset: row.get::<_, i64>(5)? as u64,
            n_events: row.get::<_, i64>(6)? as u64,
            session_id: row.get(7)?,
            model: row.get(8)?,
        })
    })?;
    let mut map = HashMap::new();
    for r in rows {
        let row = r?;
        map.insert(row.path.clone(), row);
    }
    Ok(map)
}

pub fn upsert_file_manifest(tx: &Transaction<'_>, row: &FileManifestRow) -> Result<()> {
    tx.execute(
        r#"INSERT INTO files
             (path, source, project, size, mtime_ns, last_offset, n_events, session_id, model)
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
           ON CONFLICT(path) DO UPDATE SET
             source      = excluded.source,
             project     = excluded.project,
             size        = excluded.size,
             mtime_ns    = excluded.mtime_ns,
             last_offset = excluded.last_offset,
             n_events    = excluded.n_events,
             session_id  = COALESCE(excluded.session_id, files.session_id),
             model       = COALESCE(excluded.model, files.model)"#,
        params![
            row.path.to_string_lossy().as_ref(),
            row.source.as_str(),
            row.project,
            row.size as i64,
            row.mtime_ns,
            row.last_offset as i64,
            row.n_events as i64,
            row.session_id,
            row.model,
        ],
    )?;
    Ok(())
}

/// Upsert a repo row. `first_seen` is preserved on conflict.
pub fn upsert_repo(tx: &Transaction<'_>, row: &RepoRow) -> Result<()> {
    tx.execute(
        r#"INSERT INTO repos (key, display_name, origin_url, first_seen)
           VALUES (?1, ?2, ?3, ?4)
           ON CONFLICT(key) DO UPDATE SET
             display_name = excluded.display_name,
             origin_url   = excluded.origin_url"#,
        params![row.key, row.display_name, row.origin_url, row.first_seen],
    )?;
    Ok(())
}

pub fn delete_file_and_events(tx: &Transaction<'_>, file_path: &str) -> Result<()> {
    tx.execute(
        "DELETE FROM events WHERE file_path = ?1",
        params![file_path],
    )?;
    tx.execute("DELETE FROM files WHERE path = ?1", params![file_path])?;
    Ok(())
}

pub fn insert_events(tx: &Transaction<'_>, rows: &[EventRow]) -> Result<usize> {
    if rows.is_empty() {
        return Ok(0);
    }
    let mut stmt = tx.prepare(
        r#"INSERT INTO events
             (file_path, source, ts, day, month, session_id, project_path, repo, model,
              input, output, cache_read, cache_write, cost_usd)
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)"#,
    )?;
    let mut n = 0usize;
    for r in rows {
        stmt.execute(params![
            r.file_path,
            r.source.as_str(),
            r.ts,
            r.day,
            r.month,
            r.session_id,
            r.project_path,
            r.repo,
            r.model,
            r.input as i64,
            r.output as i64,
            r.cache_read as i64,
            r.cache_write as i64,
            r.cost_usd,
        ])?;
        n += 1;
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::schema::DDL;

    fn fresh_conn() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(DDL).unwrap();
        c
    }

    fn sample_event(file: &str, repo: Option<&str>) -> EventRow {
        EventRow {
            file_path: file.into(),
            source: Source::Claude,
            ts: 1,
            day: "2026-04-22".into(),
            month: "2026-04".into(),
            session_id: "s".into(),
            project_path: None,
            repo: repo.map(str::to_owned),
            model: "claude-sonnet-4-6".into(),
            input: 1,
            output: 2,
            cache_read: 3,
            cache_write: 4,
            cost_usd: 0.5,
        }
    }

    #[test]
    fn upsert_then_load_roundtrips() {
        let mut conn = fresh_conn();
        let row = FileManifestRow {
            path: PathBuf::from("/a.jsonl"),
            source: Source::Claude,
            project: Some("proj".into()),
            size: 100,
            mtime_ns: 12345,
            last_offset: 100,
            n_events: 5,
            session_id: Some("sess".into()),
            model: Some("claude-sonnet-4-6".into()),
        };
        let tx = conn.transaction().unwrap();
        upsert_file_manifest(&tx, &row).unwrap();
        tx.commit().unwrap();
        let manifest = load_file_manifest(&conn).unwrap();
        let got = &manifest[&PathBuf::from("/a.jsonl")];
        assert_eq!(got.size, 100);
        assert_eq!(got.n_events, 5);
        assert_eq!(got.session_id.as_deref(), Some("sess"));
    }

    #[test]
    fn delete_removes_events_and_file() {
        let mut conn = fresh_conn();
        let tx = conn.transaction().unwrap();
        upsert_file_manifest(
            &tx,
            &FileManifestRow {
                path: PathBuf::from("/a.jsonl"),
                source: Source::Claude,
                project: None,
                size: 10,
                mtime_ns: 1,
                last_offset: 10,
                n_events: 1,
                session_id: None,
                model: None,
            },
        )
        .unwrap();
        insert_events(&tx, &[sample_event("/a.jsonl", None)]).unwrap();
        tx.commit().unwrap();

        let tx = conn.transaction().unwrap();
        delete_file_and_events(&tx, "/a.jsonl").unwrap();
        tx.commit().unwrap();

        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
        let manifest = load_file_manifest(&conn).unwrap();
        assert!(manifest.is_empty());
    }

    #[test]
    fn upsert_repo_preserves_first_seen() {
        let mut conn = fresh_conn();
        let tx = conn.transaction().unwrap();
        upsert_repo(
            &tx,
            &RepoRow {
                key: "/a".into(),
                display_name: "a".into(),
                origin_url: None,
                first_seen: 1000,
            },
        )
        .unwrap();
        // Second upsert with a later first_seen should NOT overwrite.
        upsert_repo(
            &tx,
            &RepoRow {
                key: "/a".into(),
                display_name: "A".into(),
                origin_url: Some("git@example.com:a.git".into()),
                first_seen: 9999,
            },
        )
        .unwrap();
        tx.commit().unwrap();

        let (name, origin, first): (String, Option<String>, i64) = conn
            .query_row(
                "SELECT display_name, origin_url, first_seen FROM repos WHERE key = '/a'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(name, "A");
        assert_eq!(origin.as_deref(), Some("git@example.com:a.git"));
        assert_eq!(first, 1000);
    }

    #[test]
    fn events_round_trip_repo_column() {
        let mut conn = fresh_conn();
        let tx = conn.transaction().unwrap();
        insert_events(&tx, &[sample_event("/a.jsonl", Some("/repo/a"))]).unwrap();
        tx.commit().unwrap();
        let repo: Option<String> = conn
            .query_row("SELECT repo FROM events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(repo.as_deref(), Some("/repo/a"));
    }
}
