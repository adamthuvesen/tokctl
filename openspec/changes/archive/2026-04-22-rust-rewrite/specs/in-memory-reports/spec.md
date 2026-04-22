## ADDED Requirements

### Requirement: Date-range filter
The in-memory reports module SHALL provide a `filter_by_date` function that takes a slice of `UsageEvent` values and optional `since` / `until` bounds (inclusive on both ends). Events outside the range SHALL be excluded.

#### Scenario: Events before `since` are excluded
- **WHEN** filter_by_date is called with since = 2024-01-10 and an event timestamped 2024-01-09
- **THEN** that event is not in the result

#### Scenario: Events after `until` are excluded
- **WHEN** filter_by_date is called with until = 2024-01-20 and an event timestamped 2024-01-21
- **THEN** that event is not in the result

#### Scenario: Events on boundary dates are included
- **WHEN** an event's timestamp equals `since` or `until`
- **THEN** it is included in the result

#### Scenario: No bounds returns all events
- **WHEN** both since and until are None
- **THEN** all events are returned

### Requirement: Daily aggregation
The `daily_in_memory` function SHALL aggregate events by local calendar day (YYYY-MM-DD), summing token counts and computing cost for each day. Results SHALL be sorted ascending by day key.

#### Scenario: Events grouped by local day
- **WHEN** two events share the same local calendar day
- **THEN** their tokens and costs are combined into a single AggregateRow

#### Scenario: Results ordered ascending
- **WHEN** events span multiple days
- **THEN** the returned vec is ordered from earliest to latest day

### Requirement: Monthly aggregation
The `monthly_in_memory` function SHALL aggregate events by local calendar month (YYYY-MM). Results SHALL be sorted ascending by month key.

#### Scenario: Events grouped by month
- **WHEN** two events fall in the same month
- **THEN** they are combined into one AggregateRow

### Requirement: Session aggregation
The `session_in_memory` function SHALL aggregate events by `(source, session_id)` pair. Each resulting `AggregateRow` SHALL carry:
- `key`: the session ID
- `source`: the event source
- `project_path`: the first non-null project path seen for the session
- `latest_timestamp`: the most recent event timestamp in the session

Results SHALL be sorted descending by `latest_timestamp` (most recent first).

#### Scenario: Session groups by source+session_id
- **WHEN** two events share the same session_id but different sources
- **THEN** they appear as separate rows

#### Scenario: latest_timestamp tracks maximum
- **WHEN** a session has events at t=100 and t=200
- **THEN** latest_timestamp = 200

#### Scenario: project_path takes first non-null value
- **WHEN** the first event has project_path = null and the second has project_path = "my-proj"
- **THEN** the row's project_path = "my-proj"

#### Scenario: Results ordered by recency descending
- **WHEN** sessions have different latest_timestamps
- **THEN** the session with the most recent activity appears first

### Requirement: Cost computed via pricing module
All cost calculations in the in-memory report functions SHALL delegate to `pricing::cost_of`. Unknown models SHALL accumulate in the provided mutable unknown-models set.

#### Scenario: Unknown model cost is zero
- **WHEN** an event references an unknown model
- **THEN** cost_usd for that event is 0.0 and the model ID is added to the unknown set

### Requirement: Output identical to cache-backed reports
For the same set of events, the in-memory report functions SHALL produce `AggregateRow` values identical to those returned by the equivalent SQL queries in the cache-store module.

#### Scenario: In-memory and cached results match
- **WHEN** the same events are processed by both paths
- **THEN** the resulting AggregateRow slices are identical (same order, same values)
