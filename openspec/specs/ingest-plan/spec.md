# ingest-plan Specification

## Purpose
TBD - created by archiving change rust-rewrite. Update Purpose after archive.
## Requirements
### Requirement: Ingest plan categories
Given a set of discovered files and a SQLite file manifest, the ingest planner SHALL categorise each file into exactly one of four lists:

- **toSkip**: file is unchanged; no re-reading needed
- **toTail**: file has grown; read only the new bytes from `last_offset` to current `size`
- **toFullParse**: file is new or has shrunk/rotated; read from byte 0
- **toPurge**: file is in the manifest but no longer exists on disk; its events must be deleted

#### Scenario: New file goes to toFullParse
- **WHEN** a discovered file has no matching entry in the manifest
- **THEN** it is added to `toFullParse`

#### Scenario: Unchanged file goes to toSkip
- **WHEN** a discovered file's `size` equals `manifest.last_offset` AND `mtime_ns` equals `manifest.mtime_ns`
- **THEN** it is added to `toSkip`

#### Scenario: Grown file goes to toTail
- **WHEN** a discovered file's `size` is greater than `manifest.last_offset` AND the file is outside the safety window
- **THEN** it is added to `toTail` with `from_offset = manifest.last_offset`

#### Scenario: Shrunk file goes to toFullParse
- **WHEN** a discovered file's `size` is less than `manifest.last_offset`
- **THEN** it is added to `toFullParse` (truncation/rotation detected)

#### Scenario: Disappeared file goes to toPurge
- **WHEN** a file appears in the manifest but is absent from the discovery results
- **THEN** it is added to `toPurge`

### Requirement: Safety window for recently-modified files
Files modified within the last `safety_window_ms` milliseconds (default: 1 hour) SHALL be excluded from the `toTail` and `toSkip` categories and placed in `toFullParse` instead. This prevents partial reads of files still being written.

The safety threshold is computed as: `now_ns - safety_window_ms * 1_000_000`.

#### Scenario: Recently modified file goes to toFullParse
- **WHEN** a file's `mtime_ns` is within the safety window
- **THEN** it is added to `toFullParse` regardless of offset state

#### Scenario: Old file outside safety window is tailed normally
- **WHEN** a file's `mtime_ns` is older than the safety window and the file has grown
- **THEN** it is categorised as `toTail`

### Requirement: Directory-level skip optimisation
If a file's parent directory is in the `unchangedPaths` set (provided by the discovery layer) AND the file appears in the manifest with matching size, it SHALL be added to `toSkip` without further checking.

#### Scenario: File in unchanged directory is skipped
- **WHEN** the file's parent directory is in `unchangedPaths` and the manifest matches
- **THEN** the file is in `toSkip`

### Requirement: Deterministic output
Given the same inputs, the ingest planner SHALL produce identical output every time. The plan MUST NOT depend on hash map iteration order or system time beyond the `now` parameter.

#### Scenario: Deterministic categorisation
- **WHEN** the planner is called twice with the same discovery set and manifest
- **THEN** both calls produce identical `IngestPlan` structs

