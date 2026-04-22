## ADDED Requirements

### Requirement: Discover Claude session files
The file-discovery module SHALL recursively scan one or more root directories for Claude JSONL session files. A valid Claude session file is a `.jsonl` file located two levels below the root: `<root>/<project-slug>/<session-id>.jsonl`. The `project-slug` directory name is treated as the `project` identifier for all events in that file.

#### Scenario: Files found at correct depth
- **WHEN** the root is `~/.claude/projects` and the directory contains `my-proj/abc123.jsonl`
- **THEN** one `DiscoveredFile` is returned with `path = "~/.claude/projects/my-proj/abc123.jsonl"`, `source = "claude"`, `project = "my-proj"`

#### Scenario: Files at wrong depth are ignored
- **WHEN** a `.jsonl` file exists directly under the root (depth 1)
- **THEN** it is not included in the results

#### Scenario: Non-JSONL files are ignored
- **WHEN** the scan encounters `.json`, `.txt`, or other non-`.jsonl` files
- **THEN** they are excluded from the results

### Requirement: Discover Codex session files
The file-discovery module SHALL recursively scan one or more root directories for Codex JSONL session files. A valid Codex session file is any `.jsonl` file found anywhere under the root. The `project` field is `null` for Codex files (project context is inferred from within the file).

#### Scenario: Codex files found at any depth
- **WHEN** the root is `~/.codex/sessions` and contains `2024/01/sess.jsonl`
- **THEN** one `DiscoveredFile` is returned with `source = "codex"` and `project = null`

### Requirement: Populated DiscoveredFile metadata
Every `DiscoveredFile` returned by discovery SHALL include the file's current byte `size` and `mtime_ns` (modification time in nanoseconds since the Unix epoch). These values are used by the ingest planner to detect changes.

#### Scenario: Metadata populated
- **WHEN** a file is discovered
- **THEN** `size` equals the file's current byte count and `mtime_ns` equals the nanosecond modification time

### Requirement: Non-existent roots are silently skipped
If a configured root directory does not exist or is not readable, discovery SHALL skip it without error.

#### Scenario: Missing root
- **WHEN** `--claude-dir /nonexistent` is passed
- **THEN** no files are returned for that root and no error is emitted

### Requirement: Directory mtime optimisation
Discovery SHALL record directory-level `mtime_ns` where supported, allowing the ingest planner to skip re-scanning directories that have not changed since the last run. This is an optimisation: when the directory mtime check is inconclusive, discovery MUST fall back to a full walk.

#### Scenario: Unchanged directory detected
- **WHEN** a directory's mtime matches the value stored in the manifest
- **THEN** the directory is added to the `unchangedPaths` set so the planner can skip its files
