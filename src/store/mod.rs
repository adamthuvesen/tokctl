#[cfg(not(test))]
mod db;
#[cfg(test)]
pub(crate) mod db;
pub mod queries;
pub mod schema;
#[cfg(not(test))]
mod writes;
#[cfg(test)]
pub(crate) mod writes;
pub(crate) use writes::{EventRow, RepoRow};

pub use db::path as store_path;
pub use writes::FileManifestRow;
#[path = "../ingest/run.rs"]
pub mod ingest_run;
#[cfg(any(test, feature = "test-fixtures"))]
#[path = "../test_support.rs"]
pub mod test_support;

/// SQLite-backed usage store. Both persistent and temporary reports use this type.
///
/// ```compile_fail
/// let store = tokctl::store::Store::ephemeral().unwrap();
/// store.connection();
/// ```
///
/// ```compile_fail
/// use tokctl::store::writes;
/// ```
pub struct Store {
    connection: rusqlite::Connection,
    path: Option<std::path::PathBuf>,
    writable: bool,
}

impl Store {
    pub fn open(path: &std::path::Path) -> anyhow::Result<Self> {
        Ok(Self {
            connection: db::open_store(path)?,
            path: Some(path.to_path_buf()),
            writable: true,
        })
    }

    pub fn open_read_only(path: &std::path::Path) -> anyhow::Result<Self> {
        use anyhow::Context;
        let connection =
            rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .with_context(|| format!("opening read-only store {}", path.display()))?;
        let version: String = connection.query_row(
            "SELECT value FROM meta WHERE key='schema_version'",
            [],
            |row| row.get(0),
        )?;
        anyhow::ensure!(
            version == schema::SCHEMA_VERSION.to_string(),
            "cache needs migration; run tokctl daily before inspection"
        );
        let status = db::provenance_status(&connection)?;
        anyhow::ensure!(
            status.legacy_cost_rows == 0
                && status.stale_pricing_rows == 0
                && status.stale_parser_files == 0
                && !status.stale_calendar,
            "cache provenance is stale; run tokctl daily with the source roots before inspection"
        );
        Ok(Self {
            connection,
            path: Some(path.to_path_buf()),
            writable: false,
        })
    }

    pub fn rebuild(&mut self) -> anyhow::Result<()> {
        let _lock = self.path.as_deref().map(acquire_ingest_lock).transpose()?;
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute_batch("DELETE FROM events; DELETE FROM files; DELETE FROM repos; DELETE FROM ingest_diagnostics; DELETE FROM parser_provenance; DELETE FROM meta WHERE key='demo_marker';")?;
        tx.execute("INSERT INTO meta(key,value) VALUES('generation','1') ON CONFLICT(key) DO UPDATE SET value=CAST(value AS INTEGER)+1", [])?;
        tx.commit()?;
        Ok(())
    }

    pub fn ephemeral() -> anyhow::Result<Self> {
        let connection = rusqlite::Connection::open_in_memory()?;
        connection.execute_batch("PRAGMA temp_store = MEMORY;")?;
        connection.execute_batch(schema::DDL)?;
        db::reproject(&connection, &crate::calendar::Calendar::system()?)?;
        Ok(Self {
            connection,
            path: None,
            writable: true,
        })
    }

    pub fn read_snapshot<T>(
        &self,
        read: impl FnOnce(&Self) -> anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        if !self.connection.is_autocommit() {
            return read(self);
        }
        let _lock = if self.writable {
            self.path.as_deref().map(acquire_ingest_lock).transpose()?
        } else {
            None
        };
        if self.writable {
            db::reproject(&self.connection, &crate::calendar::Calendar::system()?)?;
        }
        let transaction = self.connection.unchecked_transaction()?;
        if !self.writable {
            let status = db::provenance_status(&self.connection)?;
            anyhow::ensure!(
                status.legacy_cost_rows == 0
                    && status.stale_pricing_rows == 0
                    && status.stale_parser_files == 0
                    && !status.stale_calendar,
                "cache provenance changed; refresh with tokctl daily before inspection"
            );
        }
        let result = read(self)?;
        transaction.commit()?;
        Ok(result)
    }

    pub fn path(&self) -> Option<&std::path::Path> {
        self.path.as_deref()
    }
    pub(in crate::store) fn connection(&self) -> &rusqlite::Connection {
        &self.connection
    }
    pub(in crate::store) fn connection_mut(&mut self) -> &mut rusqlite::Connection {
        &mut self.connection
    }
}

#[cfg(test)]
impl std::ops::Deref for Store {
    type Target = rusqlite::Connection;
    fn deref(&self) -> &Self::Target {
        &self.connection
    }
}
#[cfg(test)]
impl std::ops::DerefMut for Store {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.connection
    }
}

fn acquire_ingest_lock(path: &std::path::Path) -> anyhow::Result<std::fs::File> {
    use anyhow::Context;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let lock_path = path.with_extension("ingest.lock");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("opening ingest lock {}", lock_path.display()))?;
    file.lock()
        .with_context(|| format!("locking ingest at {}", lock_path.display()))?;
    Ok(file)
}

#[path = "../doctor/checks.rs"]
pub(crate) mod inspection;
