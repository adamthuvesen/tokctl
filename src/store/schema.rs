pub const SCHEMA_VERSION: i64 = 9;

pub const DDL: &str = r#"
CREATE TABLE IF NOT EXISTS meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS files (
  path        TEXT PRIMARY KEY,
  source      TEXT NOT NULL,
  project     TEXT,
  size        INTEGER NOT NULL,
  mtime_ns    INTEGER NOT NULL,
  file_identity TEXT,
  ctime_ns    INTEGER,
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
  repo         TEXT,
  model        TEXT NOT NULL,
  input        INTEGER NOT NULL,
  output       INTEGER NOT NULL,
  cache_read   INTEGER NOT NULL,
  cache_write  INTEGER NOT NULL,
  cost_usd     REAL NOT NULL,
  explicit_cost_usd REAL,
  cost_provenance TEXT NOT NULL DEFAULT 'legacy',
  message_id   TEXT
);

CREATE TABLE IF NOT EXISTS parser_provenance (
  path TEXT PRIMARY KEY,
  signature TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS ingest_diagnostics (
  file_path TEXT PRIMARY KEY,
  skipped_records INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS source_roots (
  source TEXT NOT NULL,
  requested_path TEXT NOT NULL,
  canonical_path TEXT NOT NULL,
  PRIMARY KEY(source, requested_path)
);

CREATE TABLE IF NOT EXISTS repos (
  key          TEXT PRIMARY KEY,
  display_name TEXT NOT NULL,
  origin_url   TEXT,
  first_seen   INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_events_day       ON events(day);
CREATE INDEX IF NOT EXISTS idx_events_month     ON events(month);
CREATE INDEX IF NOT EXISTS idx_events_source_ts ON events(source, ts);
CREATE INDEX IF NOT EXISTS idx_events_session   ON events(session_id);
CREATE INDEX IF NOT EXISTS idx_events_project   ON events(project_path);
CREATE INDEX IF NOT EXISTS idx_events_repo      ON events(repo);
CREATE INDEX IF NOT EXISTS idx_events_file      ON events(file_path);
CREATE INDEX IF NOT EXISTS idx_events_message   ON events(message_id);

-- The same Claude message id in two session files (a resumed or forked session) counts once.
-- The copy in the lowest file path wins, so the result does not depend on ingest order.
CREATE VIEW IF NOT EXISTS counted_events AS
SELECT e.* FROM events e
WHERE e.message_id IS NULL
   OR NOT EXISTS (
     SELECT 1 FROM events other
     WHERE other.message_id = e.message_id AND other.source = 'claude'
       AND other.file_path < e.file_path
   );
"#;
