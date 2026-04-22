# render Specification

## Purpose
TBD - created by archiving change rust-rewrite. Update Purpose after archive.
## Requirements
### Requirement: AggregateRow type
The render module SHALL operate on `AggregateRow` values with the following fields:

| Field | Type | Notes |
|-------|------|-------|
| `key` | String | date, month, or session ID |
| `source` | Option\<String\> | "claude", "codex", or None for combined |
| `project_path` | Option\<String\> | session reports only |
| `latest_timestamp` | Option\<DateTime\> | session reports only |
| `input_tokens` | u64 | |
| `output_tokens` | u64 | |
| `cache_read_tokens` | u64 | |
| `cache_write_tokens` | u64 | |
| `total_tokens` | u64 | |
| `cost_usd` | f64 | |

#### Scenario: AggregateRow holds all fields
- **WHEN** an AggregateRow is constructed
- **THEN** all fields are accessible

### Requirement: Table rendering — daily and monthly
The render module SHALL format daily and monthly reports as a bordered table. The column set depends on the `show_source` flag:

Without source column: `[date/month, input, output, cache_read, cache_write, total, cost_usd]`
With source column: `[date/month, source, input, output, cache_read, cache_write, total, cost_usd]`

Integer token counts SHALL be formatted with locale-style thousands separators (e.g. `1,234,567`). Costs SHALL be formatted as `$N.NN`.

#### Scenario: Daily table columns without source
- **WHEN** render_table is called for daily report with show_source = false
- **THEN** the header row is `date | input | output | cache_read | cache_write | total | cost_usd`

#### Scenario: Token counts formatted with commas
- **WHEN** input_tokens = 1234567
- **THEN** the cell displays `1,234,567`

### Requirement: Table rendering — session
Session reports SHALL use the column set: `[session, source, project, last_activity, input, output, cache_read, cache_write, total, cost_usd]`.

The `session` column SHALL display only the first 8 characters of the session ID.
The `last_activity` column SHALL display the timestamp as `YYYY-MM-DD HH:MM:SS` in local time.

#### Scenario: Session ID truncated to 8 chars
- **WHEN** the session ID is `abcdef1234567890`
- **THEN** the table cell shows `abcdef12`

#### Scenario: Timestamp formatted without timezone
- **WHEN** the latest_timestamp is `2024-01-15T10:30:00Z`
- **THEN** the cell shows the equivalent local time as `2024-01-15 10:30:00` (or local equivalent)

### Requirement: Totals row
When the report contains at least one data row, the render module SHALL append a `TOTAL` row summing all numeric columns. The `TOTAL` row SHALL be visually distinct (e.g. bold or separator line).

#### Scenario: TOTAL row appended
- **WHEN** render_table is called with 3 rows
- **THEN** a TOTAL row appears at the bottom with the sum of each numeric column

#### Scenario: Empty report has no TOTAL row
- **WHEN** render_table is called with 0 rows
- **THEN** no TOTAL row is appended

### Requirement: JSON rendering
The render module SHALL provide a `render_json` function that serialises the rows as a JSON array. Each object's shape depends on report kind:

- Daily: `{ "date": "...", "source"?: "...", "input": N, "output": N, "cache_read": N, "cache_write": N, "totalTokens": N, "costUsd": F }`
- Monthly: same but key is `"month"`
- Session: key is `"session_id"`, plus `"source"`, `"project_path"`, `"latest_timestamp"` (ISO-8601 or null)

The output SHALL be pretty-printed with 2-space indentation.

#### Scenario: JSON array structure
- **WHEN** render_json is called with 2 rows
- **THEN** the output starts with `[` and contains exactly 2 objects

#### Scenario: Cost serialised to 4 decimal places
- **WHEN** cost_usd = 1.23456789
- **THEN** the JSON object has `"costUsd": 1.2346` (rounded to 4 decimal places)

### Requirement: Warning messages
The render module SHALL provide a `render_warnings` function that returns a `Vec<String>` of warning lines given an unknown-models set and a skipped-lines count.

- If `unknown_models` is non-empty: `"warning: no price for model(s): <sorted comma-separated list> (cost treated as 0)"`
- If `skipped_lines > 0`: `"warning: skipped N malformed JSONL line(s)"`

#### Scenario: No warnings when inputs are clean
- **WHEN** unknown_models is empty and skipped_lines = 0
- **THEN** render_warnings returns an empty vec

#### Scenario: Both warnings emitted
- **WHEN** unknown_models = {"model-x"} and skipped_lines = 3
- **THEN** two warning strings are returned

