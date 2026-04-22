# ingest-run Specification

## Purpose
TBD - created by archiving change rust-rewrite. Update Purpose after archive.
## Requirements
### Requirement: Execute full-parse entries
For each file in `plan.toFullParse`, the ingest runner SHALL delete any existing events for that file from the database, parse the file from byte 0 to its full size, and insert the resulting events.

#### Scenario: Full parse replaces existing events
- **WHEN** a file is in `toFullParse` and the manifest already contains an entry for it
- **THEN** all prior events for that file path are deleted before new events are inserted

#### Scenario: Full parse of new file inserts events
- **WHEN** a file is in `toFullParse` with no prior manifest entry
- **THEN** all parsed events are inserted and a manifest row is created

### Requirement: Execute tail entries
For each file in `plan.toTail`, the ingest runner SHALL parse the byte range `[from_offset, current_size)` and insert only the new events. Existing events for the file are not touched.

#### Scenario: Tail inserts only new events
- **WHEN** a file is in `toTail` with `from_offset = 500`
- **THEN** the file is read starting at byte 500 and only those events are inserted

### Requirement: Execute purge entries
For each path in `plan.toPurge`, the ingest runner SHALL delete all events and the manifest row for that path.

#### Scenario: Purge removes events and manifest row
- **WHEN** a path is in `toPurge`
- **THEN** all events with `file_path = path` and the corresponding files row are deleted

### Requirement: Atomic writes per file
All database writes for a single file (event inserts, manifest upsert, event deletes) SHALL be committed in a single SQLite transaction. A failure mid-file MUST leave the database in the state it was in before processing that file started.

#### Scenario: Transaction rollback on error
- **WHEN** an error occurs while inserting events for a file
- **THEN** no partial inserts for that file are committed

### Requirement: Ingest statistics
The ingest runner SHALL return an `IngestStats` struct containing:
- `files_scanned`: total files examined
- `files_skipped`: files in `toSkip`
- `files_tailed`: files processed via tail
- `files_full_parsed`: files processed via full parse
- `files_purged`: files purged
- `events_inserted`: total events written to the database
- `skipped_lines`: total malformed JSONL lines across all files
- `unknown_models`: set of model IDs not found in the pricing table

#### Scenario: Stats accumulate across all files
- **WHEN** ingest processes multiple files
- **THEN** all counters reflect the combined totals

### Requirement: Unknown model collection
The ingest runner SHALL accumulate all model IDs not found in the pricing table into `IngestStats.unknown_models`. These are reported as warnings after the run.

#### Scenario: Unknown model collected
- **WHEN** a parsed event references a model ID not in the pricing table
- **THEN** the model ID is added to `unknown_models` and the event's `cost_usd` is set to 0

### Requirement: local day/month derivation
The ingest runner SHALL derive `day` (YYYY-MM-DD) and `month` (YYYY-MM) from the event timestamp using the **local timezone** at insert time. These values are stored in the `events` table and used directly for grouping in report queries.

#### Scenario: Day derived in local time
- **WHEN** an event has timestamp `2024-01-15T01:30:00Z` and the local timezone is UTC+5
- **THEN** `day` is stored as `2024-01-15` (the local date, not UTC)

