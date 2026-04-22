# cache-store Specification

## Purpose
TBD - created by archiving change rust-rewrite. Update Purpose after archive.
## Requirements
### Requirement: SQLite schema
The cache store SHALL create the following tables on first open. The schema is version-stamped in the `meta` table to allow future migrations.

```sql
CREATE TABLE IF NOT EXISTS meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS files (
  path        TEXT PRIMARY KEY,
  source      TEXT NOT NULL,
  project     TEXT,
  size        INTEGER NOT NULL,
  mtime_ns    INTEGER NOT NULL,
  last_offset INTEGER NOT NULL DEFAULT 0,
  n_events    INTEGER NOT NULL DEFAULT 0,
  session_id  TEXT,
  model       TEXT
);

CREATE TABLE IF NOT EXISTS events (
  id           INTEGER PRIMARY KEY,
  file_path    TEXT NOT NULL,
  source       TEXT NOT NULL,
  ts           INTEGER NOT NULL,
  day          TEXT NOT NULL,
  month        TEXT NOT NULL,
  session_id   TEXT NOT NULL,
  project_path TEXT,
  model        TEXT NOT NULL,
  input        INTEGER NOT NULL,
  output       INTEGER NOT NULL,
  cache_read   INTEGER NOT NULL,
  cache_write  INTEGER NOT NULL,
  cost_usd     REAL NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_events_day       ON events(day);
CREATE INDEX IF NOT EXISTS idx_events_month     ON events(month);
CREATE INDEX IF NOT EXISTS idx_events_source_ts ON events(source, ts);
CREATE INDEX IF NOT EXISTS idx_events_session   ON events(session_id);
CREATE INDEX IF NOT EXISTS idx_events_project   ON events(project_path);
CREATE INDEX IF NOT EXISTS idx_events_file      ON events(file_path);
```

#### Scenario: Schema created on first open
- **WHEN** the database file does not yet exist
- **THEN** all tables and indexes are created automatically on `open`

#### Scenario: Idempotent schema on subsequent opens
- **WHEN** the database already contains the schema
- **THEN** opening the store does not error and does not drop existing data

### Requirement: Schema version check
On open, the store SHALL read `meta.value WHERE key = 'schema_version'`. If the stored version is less than the current version (2), the store SHALL apply any pending migrations. If the stored version is greater than the current version, the store SHALL return an error.

#### Scenario: Fresh database gets version written
- **WHEN** a new database is created
- **THEN** `meta` contains a row `('schema_version', '2')`

#### Scenario: Future schema version causes error
- **WHEN** the database has `schema_version = 999`
- **THEN** the store returns an error indicating the database was created by a newer version

### Requirement: Cache file path resolution
The cache file path SHALL be resolved in the following priority order:
1. `TOKCTL_CACHE_DIR` environment variable (appended with `/cache.db`)
2. `XDG_DATA_HOME/tokctl/cache.db` (if `XDG_DATA_HOME` is set)
3. `~/.local/share/tokctl/cache.db` on Linux/macOS

The parent directory SHALL be created if it does not exist.

#### Scenario: Default path used when no env vars set
- **WHEN** neither `TOKCTL_CACHE_DIR` nor `XDG_DATA_HOME` is set
- **THEN** the cache is stored at `~/.local/share/tokctl/cache.db`

#### Scenario: Directory created if missing
- **WHEN** the resolved parent directory does not exist
- **THEN** it is created before opening the database

### Requirement: File manifest read
The store SHALL provide a function that loads the entire `files` table into a `HashMap<String, FileManifestRow>` keyed by `path`.

#### Scenario: Manifest loaded
- **WHEN** load_file_manifest is called
- **THEN** all rows from the `files` table are returned as a map

### Requirement: Event insert
The store SHALL provide a function that bulk-inserts a slice of `EventRow` values. Duplicate `id` values (e.g. on re-parse) MUST be handled via `INSERT OR IGNORE` to avoid constraint violations when message ID deduplication is applied upstream.

#### Scenario: Events inserted
- **WHEN** insert_events is called with N rows
- **THEN** N rows appear in the events table (assuming no duplicate IDs)

### Requirement: File manifest upsert
The store SHALL provide a function to upsert a `FileManifestRow` into the `files` table, updating all fields on conflict.

#### Scenario: Manifest row created
- **WHEN** upsert_file_manifest is called for a new path
- **THEN** a row is inserted with the provided values

#### Scenario: Manifest row updated
- **WHEN** upsert_file_manifest is called for an existing path
- **THEN** all fields are updated to the new values

### Requirement: Delete file and its events
The store SHALL provide a function that deletes a row from `files` and all matching rows from `events` where `file_path = path`, within a single transaction.

#### Scenario: File and events deleted together
- **WHEN** delete_file_and_events is called
- **THEN** both the manifest row and all associated events are removed atomically

### Requirement: Daily report query
The store SHALL execute the following aggregation and return `Vec<AggregateRow>` ordered by `day ASC`:

```sql
SELECT day AS key, SUM(input), SUM(output), SUM(cache_read), SUM(cache_write),
       SUM(input+output+cache_read+cache_write) AS total_tokens, SUM(cost_usd)
FROM events
WHERE [source filter] AND [time filter]
GROUP BY day
ORDER BY day ASC
```

#### Scenario: Daily report filtered by source
- **WHEN** `filter.source = Some("claude")`
- **THEN** only Claude events are included

### Requirement: Monthly report query
The store SHALL execute the same aggregation as the daily report but grouped by `month` and ordered `month ASC`.

#### Scenario: Monthly grouping
- **WHEN** monthly_report_from_db is called
- **THEN** rows are grouped by YYYY-MM

### Requirement: Session report query
The store SHALL execute:

```sql
SELECT session_id, source, MAX(project_path), MAX(ts) AS latest_ts,
       SUM(input), SUM(output), SUM(cache_read), SUM(cache_write),
       SUM(input+output+cache_read+cache_write), SUM(cost_usd)
FROM events
WHERE [source filter] AND [time filter]
GROUP BY source, session_id
ORDER BY latest_ts DESC
```

#### Scenario: Session report ordered by recency
- **WHEN** session_report_from_db is called
- **THEN** the most recently active session appears first

