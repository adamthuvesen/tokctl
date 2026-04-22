# jsonl-parsing Specification

## Purpose
TBD - created by archiving change rust-rewrite. Update Purpose after archive.
## Requirements
### Requirement: Claude line fast pre-filter
Before parsing JSON, each line from a Claude session file SHALL be tested with a fast substring check. A line that does not contain both `"type":"assistant"` and `"usage"` MUST be skipped without calling the JSON parser.

#### Scenario: Line with both signals is parsed
- **WHEN** a line contains `"type":"assistant"` and `"usage"`
- **THEN** it is passed to the full JSON parser

#### Scenario: Line missing a signal is skipped cheaply
- **WHEN** a line contains `"type":"user"` or does not contain `"usage"`
- **THEN** it is discarded without JSON parsing and does not increment the skipped-lines counter

### Requirement: Claude UsageEvent extraction
The Claude parser SHALL extract a `UsageEvent` from each qualifying line using the following field mapping:

| JSON path | UsageEvent field |
|-----------|-----------------|
| `$.type` must equal `"assistant"` | (guard) |
| `$.sessionId` | `session_id` |
| `$.timestamp` (ISO-8601 string) | `timestamp` |
| `$.message.model` | `model` |
| `$.message.usage.input_tokens` | `input_tokens` |
| `$.message.usage.output_tokens` | `output_tokens` |
| `$.message.usage.cache_read_input_tokens` | `cache_read_tokens` |
| `$.message.usage.cache_creation_input_tokens` | `cache_write_tokens` |
| Context-supplied project path | `project_path` |

Lines where `session_id` or `timestamp` is absent, where `timestamp` is not a valid ISO-8601 datetime, or where all token counts sum to zero SHALL be skipped and counted as malformed.

#### Scenario: Valid Claude line produces UsageEvent
- **WHEN** a well-formed Claude assistant line is parsed
- **THEN** a `UsageEvent` is returned with all fields correctly populated

#### Scenario: Zero-token line is skipped
- **WHEN** all token counts are zero
- **THEN** no event is produced and the line is not counted as malformed

#### Scenario: Missing sessionId skips the line
- **WHEN** the line does not contain `sessionId`
- **THEN** no event is produced and the skipped-lines counter is incremented

#### Scenario: Invalid timestamp skips the line
- **WHEN** the `timestamp` field is not a valid ISO-8601 datetime string
- **THEN** no event is produced and the skipped-lines counter is incremented

### Requirement: Claude message ID deduplication support
The Claude parser SHALL return the `message.id` field alongside the `UsageEvent` when present. The caller uses this to detect duplicate events within a tail read.

#### Scenario: Message ID returned when present
- **WHEN** `$.message.id` is present in the JSON
- **THEN** the parser returns it alongside the event

#### Scenario: Null message ID when absent
- **WHEN** `$.message.id` is absent
- **THEN** the parser returns `None` for the message ID

### Requirement: Codex line fast pre-filter
Each line from a Codex session file SHALL be tested with a fast substring check before JSON parsing. A line MUST contain at least one of `"token_count"`, `"session_meta"`, or `"turn_context"` to be worth parsing. All three row types are needed: `session_meta` and `turn_context` populate stateful parser context; `token_count` emits the actual usage event.

#### Scenario: Line without any signal is discarded cheaply
- **WHEN** a line contains none of the three marker substrings
- **THEN** it is discarded without JSON parsing

### Requirement: Stateful Codex parse context
The Codex parser SHALL maintain per-file mutable context carrying the most recently seen `session_id`, `project_path`, and `current_model`. Context values persist across lines until overwritten by a later `session_meta` or `turn_context` row.

#### Scenario: session_meta updates context
- **WHEN** a `session_meta` row with `payload.id = "sess-x"` and `payload.cwd = "/path"` is parsed
- **THEN** context `session_id = "sess-x"` and `project_path = "/path"`; no UsageEvent is emitted

#### Scenario: turn_context updates model
- **WHEN** a `turn_context` row with `payload.model = "gpt-5.4"` is parsed
- **THEN** context `current_model = "gpt-5.4"`; no UsageEvent is emitted

### Requirement: Codex UsageEvent extraction
When a row has `type = "event_msg"` and `payload.type = "token_count"`, the parser SHALL extract a `UsageEvent` using the following field mapping:

| Source | UsageEvent field |
|--------|-----------------|
| `$.timestamp` (ISO-8601 string) | `timestamp` |
| context `session_id` | `session_id` |
| context `project_path` | `project_path` |
| context `current_model` (default `"unknown"`) | `model` |
| `$.payload.info.last_token_usage.input_tokens` | `input_tokens` |
| `$.payload.info.last_token_usage.output_tokens + reasoning_output_tokens` | `output_tokens` (sum) |
| `$.payload.info.last_token_usage.cached_input_tokens` | `cache_read_tokens` |
| `0` | `cache_write_tokens` (Codex does not report cache writes) |

Lines where context `session_id` is empty, `timestamp` is invalid, or token counts sum to zero SHALL be skipped silently (not counted as malformed).

#### Scenario: Valid token_count row produces UsageEvent
- **WHEN** a well-formed `event_msg`/`token_count` row is parsed after seeing `session_meta` and `turn_context`
- **THEN** a `UsageEvent` is returned with `source = "codex"`, `model` from context, and tokens from `last_token_usage`

#### Scenario: Reasoning tokens counted as output
- **WHEN** `last_token_usage` contains `output_tokens = 50` and `reasoning_output_tokens = 10`
- **THEN** the resulting `UsageEvent.output_tokens = 60`

#### Scenario: token_count without prior session_meta is skipped
- **WHEN** a `token_count` row appears before any `session_meta` row has set context
- **THEN** no event is produced

#### Scenario: Malformed JSON line increments counter
- **WHEN** a line contains invalid JSON
- **THEN** the JSON parser returns an error, no event is produced, and the skipped-lines counter is incremented

### Requirement: Source label on all events
Every `UsageEvent` produced by the Claude parser SHALL have `source = "claude"`. Every event produced by the Codex parser SHALL have `source = "codex"`.

#### Scenario: Source set correctly
- **WHEN** an event is produced by either parser
- **THEN** `source` matches the parser's assigned source

