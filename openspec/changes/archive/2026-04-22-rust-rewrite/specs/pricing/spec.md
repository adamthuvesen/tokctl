## ADDED Requirements

### Requirement: Static pricing table
The pricing module SHALL contain a hardcoded table of USD per 1 million tokens for each known model. The table SHALL include separate rates for `input`, `output`, `cache_read`, and `cache_write` tokens. Models that do not support a given token type SHALL have that rate as `0.0`.

The initial table SHALL contain at minimum:

| Model | Input | Output | Cache Read | Cache Write |
|-------|-------|--------|------------|-------------|
| claude-opus-4-7 | 15.0 | 75.0 | 1.5 | 18.75 |
| claude-opus-4-6 | 15.0 | 75.0 | 1.5 | 18.75 |
| claude-opus-4-5 | 15.0 | 75.0 | 1.5 | 18.75 |
| claude-sonnet-4-6 | 3.0 | 15.0 | 0.3 | 3.75 |
| claude-sonnet-4-5 | 3.0 | 15.0 | 0.3 | 3.75 |
| claude-haiku-4-5 | 1.0 | 5.0 | 0.1 | 1.25 |
| gpt-5 | 1.25 | 10.0 | 0.125 | 0.0 |
| gpt-5-codex | 1.25 | 10.0 | 0.125 | 0.0 |
| gpt-5-codex-mini | 0.25 | 2.0 | 0.025 | 0.0 |
| gpt-5.1 | 1.75 | 14.0 | 0.175 | 0.0 |
| gpt-5.1-codex-max | 1.75 | 14.0 | 0.175 | 0.0 |
| gpt-5.1-codex-mini | 0.25 | 2.0 | 0.025 | 0.0 |
| gpt-5.2 | 1.75 | 14.0 | 0.175 | 0.0 |
| gpt-5.2-codex | 1.75 | 14.0 | 0.175 | 0.0 |
| gpt-5.3 | 1.75 | 14.0 | 0.175 | 0.0 |
| gpt-5.3-codex | 1.75 | 14.0 | 0.175 | 0.0 |
| gpt-5.4 | 2.5 | 15.0 | 0.25 | 0.0 |
| gpt-5.4-codex | 2.5 | 15.0 | 0.25 | 0.0 |
| gpt-5.4-mini | 0.75 | 4.5 | 0.075 | 0.0 |
| gpt-5.4-nano | 0.2 | 1.25 | 0.02 | 0.0 |
| o4-mini | 1.1 | 4.4 | 0.275 | 0.0 |

#### Scenario: Known model returns correct cost
- **WHEN** cost_of is called with a `claude-sonnet-4-6` event with 1,000,000 input tokens and 0 others
- **THEN** the result is `3.0` USD

#### Scenario: Cache tokens contribute to cost
- **WHEN** cost_of is called with cache_read_tokens = 1,000,000 for claude-sonnet-4-6
- **THEN** the result includes `0.3` USD for the cache read

### Requirement: Model ID normalisation
Before lookup, model IDs SHALL have any trailing `-YYYYMMDD` date suffix stripped. This handles Claude model IDs that include a release date.

#### Scenario: Date suffix stripped before lookup
- **WHEN** the event model is `claude-sonnet-4-6-20250101`
- **THEN** it is looked up as `claude-sonnet-4-6` and the correct price is returned

### Requirement: Unknown model returns zero cost
If a model ID is not found in the pricing table after normalisation, `cost_of` SHALL return `0.0` and add the original model ID to the provided unknown-models set.

#### Scenario: Unknown model costs zero
- **WHEN** cost_of is called with model `gpt-99-ultra` and a non-null unknown set
- **THEN** the result is `0.0` and `gpt-99-ultra` is in the unknown set

### Requirement: has_price predicate
The module SHALL expose a `has_price(model: &str) -> bool` function that returns `true` if and only if the normalised model ID is in the pricing table.

#### Scenario: Known model returns true
- **WHEN** has_price is called with `claude-opus-4-7`
- **THEN** it returns true

#### Scenario: Unknown model returns false
- **WHEN** has_price is called with `gpt-99-ultra`
- **THEN** it returns false
