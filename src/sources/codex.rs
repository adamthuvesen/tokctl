use crate::types::{Source, UsageEvent};
use chrono::{DateTime, Utc};
use serde::Deserialize;

/// Per-file parser state. `session_meta` and `turn_context` rows populate this
/// context; `token_count` rows read from it to emit events.
#[derive(Debug, Default)]
pub struct CodexCtx {
    pub session_id: Option<String>,
    pub project_path: Option<String>,
    pub current_model: Option<String>,
    previous_total: Option<TokenUsage>,
}

#[derive(Debug)]
pub enum CodexParsed {
    /// Context-populating rows update the CodexCtx and emit no event.
    ContextUpdated,
    /// A token_count row that successfully produced an event.
    Event(UsageEvent),
    /// Context estimates, repeated cumulative counters, and unrelated records.
    Skipped,
}

// Typed row shape. Codex uses `{type, payload, timestamp}` at the outer
// level. We flatten branches with `#[serde(tag = "type", content = "payload")]`-
// style matching by keeping the outer row generic and branching on type manually,
// since `payload`'s shape differs per variant.
#[derive(Debug, Deserialize)]
struct CodexRow<'a> {
    #[serde(rename = "type", default)]
    kind: &'a str,
    #[serde(default)]
    timestamp: Option<&'a str>,
    #[serde(default, borrow)]
    payload: Option<&'a serde_json::value::RawValue>,
}

#[derive(Debug, Deserialize, Default)]
struct SessionMetaPayload<'a> {
    #[serde(default)]
    id: Option<&'a str>,
    #[serde(default)]
    cwd: Option<&'a str>,
}

#[derive(Debug, Deserialize, Default)]
struct TurnContextPayload<'a> {
    #[serde(default)]
    model: Option<&'a str>,
}

#[derive(Debug, Deserialize, Default)]
struct EventMsgPayload<'a> {
    #[serde(rename = "type", default)]
    kind: &'a str,
    #[serde(default)]
    info: Option<TokenCountInfo>,
}

#[derive(Debug, Deserialize, Default)]
struct TokenCountInfo {
    #[serde(default)]
    last_token_usage: Option<TokenUsage>,
    #[serde(default)]
    total_token_usage: Option<TokenUsage>,
}

#[derive(Debug, Clone, Copy, Deserialize, Default, PartialEq, Eq)]
struct TokenUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default, rename = "reasoning_output_tokens")]
    _reasoning_output_tokens: u64,
    #[serde(default)]
    cache_write_input_tokens: u64,
    #[serde(default)]
    cached_input_tokens: u64,
}

impl TokenUsage {
    fn checked_delta(self, previous: Self) -> Option<Self> {
        Some(Self {
            input_tokens: self.input_tokens.checked_sub(previous.input_tokens)?,
            output_tokens: self.output_tokens.checked_sub(previous.output_tokens)?,
            cached_input_tokens: self
                .cached_input_tokens
                .checked_sub(previous.cached_input_tokens)?,
            cache_write_input_tokens: self
                .cache_write_input_tokens
                .checked_sub(previous.cache_write_input_tokens)?,
            _reasoning_output_tokens: 0,
        })
    }

    fn non_cached_input(self) -> Option<u64> {
        self.input_tokens
            .checked_sub(self.cached_input_tokens)?
            .checked_sub(self.cache_write_input_tokens)
    }
}

/// Parse a Codex JSONL line, updating `ctx` for stateful rows and returning
/// a token_count event when present and emittable.
pub fn parse_codex_line(line: &str, ctx: &mut CodexCtx) -> Option<CodexParsed> {
    let row: CodexRow = serde_json::from_str(line).ok()?;
    if !matches!(row.kind, "session_meta" | "turn_context" | "event_msg") {
        return Some(CodexParsed::Skipped);
    }
    let payload_raw = row.payload?;

    match row.kind {
        "session_meta" => {
            let payload: SessionMetaPayload = serde_json::from_str(payload_raw.get()).ok()?;
            if let Some(id) = payload.id {
                if id.is_empty() {
                    return None;
                }
                if ctx.session_id.as_deref() != Some(id) {
                    *ctx = CodexCtx {
                        session_id: Some(id.to_owned()),
                        ..Default::default()
                    };
                }
            }
            if let Some(cwd) = payload.cwd {
                ctx.project_path = Some(cwd.to_owned());
            }
            Some(CodexParsed::ContextUpdated)
        }
        "turn_context" => {
            let payload: TurnContextPayload = serde_json::from_str(payload_raw.get()).ok()?;
            if let Some(m) = payload.model {
                ctx.current_model = Some(m.to_owned());
            }
            Some(CodexParsed::ContextUpdated)
        }
        "event_msg" => {
            let payload: EventMsgPayload = serde_json::from_str(payload_raw.get()).ok()?;
            if payload.kind != "token_count" {
                return Some(CodexParsed::Skipped);
            }
            let Some(info) = payload.info else {
                return Some(CodexParsed::Skipped);
            };
            let session_id = ctx.session_id.clone()?;
            let timestamp = row.timestamp?.parse::<DateTime<Utc>>().ok()?;
            let mut next_total = None;
            let usage = if let Some(total) = info.total_token_usage {
                total.non_cached_input()?;
                total
                    .input_tokens
                    .checked_add(total.output_tokens)
                    .filter(|value| *value <= i64::MAX as u64)?;
                let usage = match ctx.previous_total {
                    Some(previous) => {
                        let delta = total.checked_delta(previous);
                        // A reset establishes a new baseline but cannot establish usage for this row.
                        if delta.is_none() {
                            ctx.previous_total = Some(total);
                        }
                        delta?
                    }
                    // Preserve first-request usage without charging inherited fork history.
                    None => info.last_token_usage.unwrap_or_default(),
                };
                usage.non_cached_input()?;
                next_total = Some(total);
                usage
            } else {
                // Once counters exist, a missing counter cannot safely advance the accumulator.
                if ctx.previous_total.is_some() {
                    return None;
                }
                let Some(last) = info.last_token_usage else {
                    return Some(CodexParsed::Skipped);
                };
                last
            };
            let non_cached_input = usage.non_cached_input()?;
            if usage.input_tokens == 0 && usage.output_tokens == 0 {
                if let Some(total) = next_total {
                    ctx.previous_total = Some(total);
                }
                return Some(CodexParsed::Skipped);
            }
            let model = ctx
                .current_model
                .clone()
                .unwrap_or_else(|| "unknown".to_owned());

            let event = UsageEvent {
                source: Source::Codex,
                timestamp,
                session_id,
                project_path: ctx.project_path.clone(),
                model,
                input_tokens: non_cached_input,
                output_tokens: usage.output_tokens,
                cache_read_tokens: usage.cached_input_tokens,
                cache_write_tokens: usage.cache_write_input_tokens,
                explicit_cost_usd: None,
            };
            event.validate().ok()?;
            if let Some(total) = next_total {
                ctx.previous_total = Some(total);
            }
            Some(CodexParsed::Event(event))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SESS_META: &str = r#"{"timestamp":"2026-04-20T12:49:04.848Z","type":"session_meta","payload":{"id":"sess-x","cwd":"/Users/dev/repo","originator":"Codex Desktop"}}"#;
    const TURN_CTX: &str = r#"{"timestamp":"2026-04-20T12:49:05.000Z","type":"turn_context","payload":{"model":"gpt-5.4","cwd":"/Users/dev/repo"}}"#;
    const TOKEN_COUNT: &str = r#"{"timestamp":"2026-04-20T12:49:10.000Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":200,"cached_input_tokens":50,"output_tokens":60,"reasoning_output_tokens":10,"total_tokens":320}}}}"#;

    fn counter(last: (u64, u64, u64), total: (u64, u64, u64)) -> String {
        serde_json::json!({"timestamp":"2026-04-20T12:49:10Z","type":"event_msg","payload":{
            "type":"token_count","info":{
                "last_token_usage":{"input_tokens":last.0,"cached_input_tokens":last.1,"output_tokens":last.2},
                "total_token_usage":{"input_tokens":total.0,"cached_input_tokens":total.1,"output_tokens":total.2}
            }
        }}).to_string()
    }

    fn context() -> CodexCtx {
        let mut ctx = CodexCtx::default();
        parse_codex_line(SESS_META, &mut ctx);
        parse_codex_line(TURN_CTX, &mut ctx);
        ctx
    }

    fn emitted(line: &str, ctx: &mut CodexCtx) -> UsageEvent {
        match parse_codex_line(line, ctx) {
            Some(CodexParsed::Event(event)) => event,
            other => panic!("expected usage, got {other:?}"),
        }
    }

    #[test]
    fn unchanged_counters_ignore_repeat_notifications_and_context_estimates() {
        let mut ctx = context();
        let line = counter((100, 20, 10), (100, 20, 10));
        assert_eq!(emitted(&line, &mut ctx).total_tokens(), Some(110));
        assert!(matches!(
            parse_codex_line(&line, &mut ctx),
            Some(CodexParsed::Skipped)
        ));
        let estimate = counter((999, 0, 0), (100, 20, 10));
        assert!(matches!(
            parse_codex_line(&estimate, &mut ctx),
            Some(CodexParsed::Skipped)
        ));
        let next = counter((100, 20, 10), (200, 40, 20));
        assert_eq!(emitted(&next, &mut ctx).total_tokens(), Some(110));
    }

    #[test]
    fn cumulative_deltas_recover_usage_between_notifications() {
        let mut ctx = context();
        emitted(&counter((100, 20, 10), (100, 20, 10)), &mut ctx);
        let event = emitted(&counter((50, 10, 5), (250, 50, 25)), &mut ctx);
        assert_eq!(
            (
                event.input_tokens,
                event.cache_read_tokens,
                event.output_tokens
            ),
            (120, 30, 15)
        );
    }

    #[test]
    fn first_snapshot_counts_latest_request_without_inherited_history() {
        let mut ctx = context();
        let event = emitted(&counter((100, 20, 10), (900, 200, 90)), &mut ctx);
        assert_eq!(event.total_tokens(), Some(110));
        let event = emitted(&counter((100, 20, 10), (1000, 220, 100)), &mut ctx);
        assert_eq!(event.total_tokens(), Some(110));
    }

    #[test]
    fn counter_reset_is_malformed_and_establishes_new_baseline() {
        let mut ctx = context();
        emitted(&counter((100, 20, 10), (100, 20, 10)), &mut ctx);
        assert!(parse_codex_line(&counter((30, 0, 5), (30, 0, 5)), &mut ctx).is_none());
        let event = emitted(&counter((20, 0, 2), (50, 0, 7)), &mut ctx);
        assert_eq!(event.total_tokens(), Some(22));
    }

    #[test]
    fn invalid_cached_subset_is_rejected_without_advancing_counter() {
        let mut ctx = context();
        assert!(parse_codex_line(&counter((1, 100, 2), (1, 100, 2)), &mut ctx).is_none());
        assert_eq!(
            emitted(&counter((100, 20, 10), (100, 20, 10)), &mut ctx).total_tokens(),
            Some(110)
        );
    }

    #[test]
    fn invalid_initial_total_does_not_establish_a_baseline() {
        let mut ctx = context();
        assert!(
            parse_codex_line(&counter((100, 0, 0), ((1 << 63) + 200, 0, 0)), &mut ctx).is_none()
        );
        assert_eq!(
            emitted(&counter((100, 0, 0), (100, 0, 0)), &mut ctx).input_tokens,
            100
        );
    }

    #[test]
    fn oversized_record_does_not_poison_the_next_valid_counter() {
        let mut ctx = context();
        emitted(&counter((100, 0, 0), (100, 0, 0)), &mut ctx);
        assert!(parse_codex_line(
            &counter(((1 << 63) + 100, 0, 0), ((1 << 63) + 200, 0, 0)),
            &mut ctx
        )
        .is_none());
        assert_eq!(
            emitted(&counter((100, 0, 0), (200, 0, 0)), &mut ctx).input_tokens,
            100
        );
        assert_eq!(
            emitted(&counter((100, 0, 0), (300, 0, 0)), &mut ctx).input_tokens,
            100
        );
        let mut ctx = context();
        emitted(&counter((100, 20, 0), (100, 20, 0)), &mut ctx);
        assert!(parse_codex_line(
            &counter(((1 << 63) + 100, 0, 0), ((1 << 63) + 200, 0, 0)),
            &mut ctx
        )
        .is_none());
        assert_eq!(
            emitted(&counter((100, 20, 0), (200, 40, 0)), &mut ctx).input_tokens,
            80
        );
    }

    #[test]
    fn new_session_resets_usage_context() {
        let mut ctx = context();
        emitted(&counter((100, 20, 10), (100, 20, 10)), &mut ctx);
        parse_codex_line(&SESS_META.replace("sess-x", "other"), &mut ctx);
        assert_eq!(
            emitted(&counter((20, 0, 2), (20, 0, 2)), &mut ctx).total_tokens(),
            Some(22)
        );
    }

    #[test]
    fn cache_write_is_a_disjoint_input_category() {
        let mut ctx = context();
        let line = r#"{"timestamp":"2026-04-20T12:49:10Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":100,"cached_input_tokens":20,"cache_write_input_tokens":30,"output_tokens":10}}}}"#;
        let event = emitted(line, &mut ctx);
        assert_eq!(
            (
                event.input_tokens,
                event.cache_read_tokens,
                event.cache_write_tokens,
                event.output_tokens
            ),
            (50, 20, 30, 10)
        );
        assert_eq!(event.total_tokens(), Some(110));
    }

    #[test]
    fn full_sequence_produces_event() {
        let mut ctx = CodexCtx::default();
        assert!(matches!(
            parse_codex_line(SESS_META, &mut ctx),
            Some(CodexParsed::ContextUpdated)
        ));
        assert_eq!(ctx.session_id.as_deref(), Some("sess-x"));
        assert_eq!(ctx.project_path.as_deref(), Some("/Users/dev/repo"));

        assert!(matches!(
            parse_codex_line(TURN_CTX, &mut ctx),
            Some(CodexParsed::ContextUpdated)
        ));
        assert_eq!(ctx.current_model.as_deref(), Some("gpt-5.4"));

        let Some(CodexParsed::Event(ev)) = parse_codex_line(TOKEN_COUNT, &mut ctx) else {
            panic!("expected event");
        };
        assert_eq!(ev.source, Source::Codex);
        assert_eq!(ev.session_id, "sess-x");
        assert_eq!(ev.model, "gpt-5.4");
        assert_eq!(ev.input_tokens, 150); // 200 - 50 cached = non-cached input only
        assert_eq!(ev.output_tokens, 60);
        assert_eq!(ev.cache_read_tokens, 50);
        assert_eq!(ev.cache_write_tokens, 0);
    }

    #[test]
    fn token_count_without_session_is_malformed() {
        let mut ctx = CodexCtx::default();
        let res = parse_codex_line(TOKEN_COUNT, &mut ctx);
        assert!(res.is_none());
    }

    #[test]
    fn zero_token_line_is_skipped() {
        let mut ctx = CodexCtx {
            session_id: Some("s".into()),
            current_model: Some("gpt-5.4".into()),
            ..Default::default()
        };
        let zero = r#"{"timestamp":"2026-04-20T12:49:10.000Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":0,"output_tokens":0,"cached_input_tokens":0,"reasoning_output_tokens":0}}}}"#;
        assert!(matches!(
            parse_codex_line(zero, &mut ctx),
            Some(CodexParsed::Skipped)
        ));
    }

    #[test]
    fn malformed_json_returns_none() {
        let mut ctx = CodexCtx::default();
        assert!(parse_codex_line(r#"not json but token_count hint"#, &mut ctx).is_none());
    }

    #[test]
    fn unknown_fields_ignored() {
        let line = r#"{"timestamp":"2026-04-20T12:49:10.000Z","type":"event_msg","newField":7,"payload":{"type":"token_count","futuristicField":true,"info":{"last_token_usage":{"input_tokens":50,"cached_input_tokens":0,"output_tokens":25,"reasoning_output_tokens":0,"total_tokens":75,"new_t":9}}}}"#;
        let mut ctx = CodexCtx {
            session_id: Some("s".into()),
            current_model: Some("gpt-5.4".into()),
            ..Default::default()
        };
        let Some(CodexParsed::Event(ev)) = parse_codex_line(line, &mut ctx) else {
            panic!("expected event");
        };
        assert_eq!(ev.input_tokens, 50);
    }

    #[test]
    fn reasoning_tokens_are_not_added_to_output_twice() {
        let line = r#"{"timestamp":"2026-04-22T10:59:50.068Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":22443,"cached_input_tokens":4480,"output_tokens":650,"reasoning_output_tokens":516,"total_tokens":23093}}}}"#;
        let mut ctx = CodexCtx {
            session_id: Some("s".into()),
            current_model: Some("gpt-5.4".into()),
            ..Default::default()
        };
        let Some(CodexParsed::Event(ev)) = parse_codex_line(line, &mut ctx) else {
            panic!("expected event");
        };
        assert_eq!(ev.input_tokens, 17_963);
        assert_eq!(ev.output_tokens, 650);
        assert_eq!(ev.cache_read_tokens, 4_480);
    }
}
