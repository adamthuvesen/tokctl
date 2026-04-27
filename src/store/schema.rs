pub const SCHEMA_VERSION: i64 = 5;

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
  cost_usd     REAL NOT NULL
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
"#;
