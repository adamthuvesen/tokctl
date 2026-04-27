use crate::types::{Source, UsageEvent};
use chrono::{DateTime, Utc};
use serde::Deserialize;

pub struct ClaudeParsed {
    pub event: UsageEvent,
    pub message_id: Option<String>,
}

/// Fast substring pre-filter. Lines missing either marker cannot carry usage.
#[inline]
pub fn claude_line_has_signal(line: &str) -> bool {
    line.contains(r#""type":"assistant""#) && line.contains(r#""usage""#)
}

// Typed views onto a Claude JSONL line. `#[serde(default)]` on every optional
// field means missing keys decode to `None`/default; unknown keys in the JSON
// are ignored by serde. This keeps the same forward-compat behaviour as the
// old `Value`-walking parser while cutting ~40% of the allocation cost.
#[derive(Debug, Deserialize)]
struct ClaudeLine<'a> {
    #[serde(rename = "type", default)]
    kind: &'a str,
    #[serde(rename = "sessionId", default)]
    session_id: Option<&'a str>,
    #[serde(default)]
    timestamp: Option<&'a str>,
    #[serde(default, borrow)]
    message: Option<ClaudeMessage<'a>>,
}

#[derive(Debug, Deserialize)]
struct ClaudeMessage<'a> {
    #[serde(default)]
    id: Option<&'a str>,
    #[serde(default)]
    model: Option<&'a str>,
    #[serde(default)]
    usage: Option<ClaudeUsage>,
}

#[derive(Debug, Deserialize, Default)]
struct ClaudeUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    cache_read_input_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: u64,
}

/// Parse a Claude JSONL line, returning the UsageEvent and optional message ID.
pub fn parse_claude_line(line: &str, project_path: Option<&str>) -> Option<ClaudeParsed> {
    if !claude_line_has_signal(line) {
        return None;
    }
    let parsed: ClaudeLine = serde_json::from_str(line).ok()?;
    if parsed.kind != "assistant" {
        return None;
    }
    let message = parsed.message?;
    let usage = message.usage?;
    let total = usage.input_tokens
        + usage.output_tokens
        + usage.cache_read_input_tokens
        + usage.cache_creation_input_tokens;
    if total == 0 {
        return None;
    }
    let session_id = parsed.session_id?;
    if session_id.is_empty() {
        return None;
    }
    let timestamp_str = parsed.timestamp?;
    let timestamp: DateTime<Utc> = timestamp_str.parse().ok()?;

    Some(ClaudeParsed {
        event: UsageEvent {
            source: Source::Claude,
            timestamp,
            session_id: session_id.to_owned(),
            project_path: project_path.map(str::to_owned),
            model: message.model.unwrap_or("unknown").to_owned(),
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cache_read_tokens: usage.cache_read_input_tokens,
            cache_write_tokens: usage.cache_creation_input_tokens,
            explicit_cost_usd: None,
        },
        message_id: message.id.map(str::to_owned),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ASSISTANT_LINE: &str = r#"{"type":"assistant","timestamp":"2026-04-18T09:00:05.000Z","sessionId":"sess-a","message":{"id":"m1","model":"claude-sonnet-4-6","role":"assistant","usage":{"input_tokens":100,"output_tokens":40,"cache_read_input_tokens":500,"cache_creation_input_tokens":200}}}"#;
    const USER_LINE: &str = r#"{"type":"user","timestamp":"2026-04-18T09:00:00.000Z","sessionId":"sess-a","message":{"role":"user","content":"hello"}}"#;
    const ZERO_LINE: &str = r#"{"type":"assistant","timestamp":"2026-04-18T09:00:05.000Z","sessionId":"sess-a","message":{"id":"m9","model":"claude-sonnet-4-6","role":"assistant","usage":{"input_tokens":0,"output_tokens":0,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}"#;

    #[test]
    fn signal_filter_rejects_user_lines() {
        assert!(!claude_line_has_signal(USER_LINE));
        assert!(claude_line_has_signal(ASSISTANT_LINE));
    }

    #[test]
    fn parses_assistant_line() {
        let p = parse_claude_line(ASSISTANT_LINE, Some("my-proj")).expect("should parse");
        assert_eq!(p.event.source, Source::Claude);
        assert_eq!(p.event.session_id, "sess-a");
        assert_eq!(p.event.model, "claude-sonnet-4-6");
        assert_eq!(p.event.input_tokens, 100);
        assert_eq!(p.event.output_tokens, 40);
        assert_eq!(p.event.cache_read_tokens, 500);
        assert_eq!(p.event.cache_write_tokens, 200);
        assert_eq!(p.event.project_path.as_deref(), Some("my-proj"));
        assert_eq!(p.message_id.as_deref(), Some("m1"));
    }

    #[test]
    fn zero_token_line_returns_none() {
        assert!(parse_claude_line(ZERO_LINE, None).is_none());
    }

    #[test]
    fn user_line_returns_none() {
        assert!(parse_claude_line(USER_LINE, None).is_none());
    }

    #[test]
    fn malformed_json_returns_none() {
        assert!(parse_claude_line("not valid json with usage and assistant", None).is_none());
    }

    #[test]
    fn missing_session_returns_none() {
        let line = r#"{"type":"assistant","timestamp":"2026-04-18T09:00:05.000Z","message":{"model":"claude-sonnet-4-6","usage":{"input_tokens":1}}}"#;
        assert!(parse_claude_line(line, None).is_none());
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let line = r#"{"type":"assistant","timestamp":"2026-04-18T09:00:05.000Z","sessionId":"sess-a","brandNewFieldFromFuture":42,"message":{"id":"m1","model":"claude-sonnet-4-6","role":"assistant","somethingNew":"ok","usage":{"input_tokens":100,"output_tokens":40,"cache_read_input_tokens":0,"cache_creation_input_tokens":0,"new_token_type":99}}}"#;
        let p = parse_claude_line(line, None).expect("should parse");
        assert_eq!(p.event.input_tokens, 100);
    }
}
