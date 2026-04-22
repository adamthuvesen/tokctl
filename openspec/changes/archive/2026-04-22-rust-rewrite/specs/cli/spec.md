## ADDED Requirements

### Requirement: Binary entry point
The `tokctl` binary SHALL be the sole entry point. It SHALL exit with code 0 on success and a non-zero code on error, printing the error message to stderr.

#### Scenario: Successful run prints output to stdout
- **WHEN** a valid subcommand is invoked with valid flags
- **THEN** the report is printed to stdout and the process exits 0

#### Scenario: Error exits non-zero and prints to stderr
- **WHEN** an unrecognised flag or invalid argument is supplied
- **THEN** the error message is printed to stderr and the process exits non-zero

### Requirement: Report subcommands
The CLI SHALL provide three subcommands: `daily`, `monthly`, and `session`. Each subcommand produces the corresponding aggregated report.

#### Scenario: daily subcommand
- **WHEN** `tokctl daily` is invoked
- **THEN** a report aggregated by local calendar day (YYYY-MM-DD) is printed

#### Scenario: monthly subcommand
- **WHEN** `tokctl monthly` is invoked
- **THEN** a report aggregated by local calendar month (YYYY-MM) is printed

#### Scenario: session subcommand
- **WHEN** `tokctl session` is invoked
- **THEN** a report aggregated by session ID is printed, ordered by most-recent activity descending

### Requirement: Global flags
The CLI SHALL support the following global flags applicable to all subcommands:

| Flag | Type | Description |
|------|------|-------------|
| `--source <claude\|codex\|all>` | string | Filter to a single source; default `all` |
| `--since <date>` | string | Inclusive lower bound (ISO-8601 or relative like `7d`) |
| `--until <date>` | string | Inclusive upper bound (same formats) |
| `--json` | bool | Emit JSON instead of a table |
| `--claude-dir <path>` | string | Override Claude session root directory (comma-separated) |
| `--codex-dir <path>` | string | Override Codex session root directory (comma-separated) |
| `--rebuild` | bool | Drop and rebuild the SQLite cache from scratch |
| `--no-cache` | bool | Run entirely in-memory without reading or writing SQLite |

#### Scenario: --source filters output
- **WHEN** `tokctl daily --source claude` is invoked
- **THEN** only rows sourced from Claude are included in the report

#### Scenario: --json emits valid JSON
- **WHEN** any subcommand is invoked with `--json`
- **THEN** the output is a valid JSON array and nothing else is written to stdout

#### Scenario: --no-cache skips SQLite
- **WHEN** `tokctl daily --no-cache` is invoked
- **THEN** no SQLite file is read or written; the report is produced from in-memory parsing

#### Scenario: --rebuild drops and recreates cache
- **WHEN** `tokctl daily --rebuild` is invoked
- **THEN** existing cached data is discarded, all session files are re-parsed, and the cache is repopulated

### Requirement: Environment variable overrides
The CLI SHALL read environment variables as fallbacks for directory flags:

| Env var | Corresponding flag |
|---------|--------------------|
| `TOKCTL_CLAUDE_DIR` | `--claude-dir` |
| `TOKCTL_CODEX_DIR` | `--codex-dir` |
| `CLAUDE_CONFIG_DIR` | Suffix `/projects` appended; used as Claude root if no other override |
| `CODEX_HOME` | Used as Codex root if no other override |

Flag values take precedence over environment variables.

#### Scenario: Environment variable used when flag absent
- **WHEN** `TOKCTL_CLAUDE_DIR=/tmp/claude` is set and `--claude-dir` is not passed
- **THEN** `/tmp/claude` is used as the Claude root directory

#### Scenario: Flag overrides environment variable
- **WHEN** both `TOKCTL_CLAUDE_DIR=/tmp/env` and `--claude-dir /tmp/flag` are set
- **THEN** `/tmp/flag` is used

### Requirement: Version flag
The CLI SHALL print the crate version when `--version` or `-V` is passed and exit 0.

#### Scenario: Version output
- **WHEN** `tokctl --version` is invoked
- **THEN** the output contains the version string from `Cargo.toml`

### Requirement: Warning output
The CLI SHALL print warnings to stderr after the report when unknown model IDs or malformed JSONL lines are encountered. Warnings SHALL NOT be included in `--json` stdout output.

#### Scenario: Unknown model warning
- **WHEN** a session file references a model not in the pricing table
- **THEN** a warning like `warning: no price for model(s): <id>` is printed to stderr

#### Scenario: Malformed line warning
- **WHEN** one or more JSONL lines fail to parse
- **THEN** a warning like `warning: skipped N malformed JSONL line(s)` is printed to stderr
