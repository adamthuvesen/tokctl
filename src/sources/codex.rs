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
}

#[derive(Debug)]
pub enum CodexParsed {
    /// Context-populating rows update the CodexCtx and emit no event.
    ContextUpdated,
    /// A token_count row that successfully produced an event.
    Event(UsageEvent),
    /// A token_count row that was well-formed JSON but lacked the data
    /// needed to emit an event (e.g. no prior session_meta). Not counted
    /// as malformed.
    Skipped,
}

/// Fast substring pre-filter. Cheap reject for lines that don't carry any of
/// the three signals we care about.
#[inline]
pub fn codex_line_has_signal(line: &str) -> bool {
    line.contains(r#""token_count""#)
        || line.contains(r#""session_meta""#)
        || line.contains(r#""turn_context""#)
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
}

#[derive(Debug, Deserialize, Default)]
struct TokenUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    reasoning_output_tokens: u64,
    #[serde(default)]
    cached_input_tokens: u64,
}

/// Parse a Codex JSONL line, updating `ctx` for stateful rows and returning
/// a token_count event when present and emittable.
pub fn parse_codex_line(line: &str, ctx: &mut CodexCtx) -> Option<CodexParsed> {
    if !codex_line_has_signal(line) {
        return None;
    }
    let row: CodexRow = serde_json::from_str(line).ok()?;
    let payload_raw = row.payload?;

    match row.kind {
        "session_meta" => {
            let payload: SessionMetaPayload = serde_json::from_str(payload_raw.get()).ok()?;
            if let Some(id) = payload.id {
                ctx.session_id = Some(id.to_owned());
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
                return None;
            }
            let Some(info) = payload.info else {
                return Some(CodexParsed::Skipped);
            };
            let Some(last) = info.last_token_usage else {
                return Some(CodexParsed::Skipped);
            };
            let total_output = last.output_tokens + last.reasoning_output_tokens;
            if last.input_tokens + total_output + last.cached_input_tokens == 0 {
                return Some(CodexParsed::Skipped);
            }
            let Some(session_id) = ctx.session_id.clone() else {
                return Some(CodexParsed::Skipped);
            };
            let Some(timestamp_str) = row.timestamp else {
                return Some(CodexParsed::Skipped);
            };
            let Ok(timestamp) = timestamp_str.parse::<DateTime<Utc>>() else {
                return Some(CodexParsed::Skipped);
            };
            let model = ctx
                .current_model
                .clone()
                .unwrap_or_else(|| "unknown".to_owned());

            Some(CodexParsed::Event(UsageEvent {
                source: Source::Codex,
                timestamp,
                session_id,
                project_path: ctx.project_path.clone(),
                model,
                input_tokens: last.input_tokens,
                output_tokens: total_output,
                cache_read_tokens: last.cached_input_tokens,
                cache_write_tokens: 0,
            }))
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

    #[test]
    fn signal_filter_rejects_unrelated_lines() {
        assert!(!codex_line_has_signal(r#"{"type":"other","foo":1}"#));
        assert!(codex_line_has_signal(SESS_META));
        assert!(codex_line_has_signal(TOKEN_COUNT));
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
        assert_eq!(ev.input_tokens, 200);
        assert_eq!(ev.output_tokens, 70);
        assert_eq!(ev.cache_read_tokens, 50);
        assert_eq!(ev.cache_write_tokens, 0);
    }

    #[test]
    fn token_count_without_session_is_skipped() {
        let mut ctx = CodexCtx::default();
        let res = parse_codex_line(TOKEN_COUNT, &mut ctx);
        assert!(matches!(res, Some(CodexParsed::Skipped)));
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
}
