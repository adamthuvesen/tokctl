use crate::types::UsageEvent;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy)]
pub struct PriceEntry {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

// USD per 1M tokens. Last verified 2026-11-12 for Claude 5 and GPT-6 models, 2026-04 for the rest.
const PRICES: &[(&str, PriceEntry)] = &[
    // Anthropic
    (
        "claude-fable-5-2",
        PriceEntry {
            input: 8.0,
            output: 40.0,
            cache_read: 0.2,
            cache_write: 10.0,
        },
    ),
    (
        "claude-fable-5-1",
        PriceEntry {
            input: 10.0,
            output: 50.0,
            cache_read: 0.25,
            cache_write: 12.5,
        },
    ),
    (
        "claude-opus-5-5",
        PriceEntry {
            input: 4.0,
            output: 20.0,
            cache_read: 0.2,
            cache_write: 5.0,
        },
    ),
    (
        "claude-opus-5",
        PriceEntry {
            input: 5.0,
            output: 25.0,
            cache_read: 0.5,
            cache_write: 6.25,
        },
    ),
    (
        "claude-sonnet-5-5",
        PriceEntry {
            input: 2.0,
            output: 10.0,
            cache_read: 0.1,
            cache_write: 2.5,
        },
    ),
    (
        "claude-sonnet-5",
        PriceEntry {
            input: 2.0,
            output: 10.0,
            cache_read: 0.2,
            cache_write: 2.5,
        },
    ),
    // Rate for prompts up to LONG_PROMPT_TOKENS; longer prompts use LONG_PROMPT_PRICES.
    (
        "claude-haiku-5-5",
        PriceEntry {
            input: 0.1,
            output: 0.5,
            cache_read: 0.01,
            cache_write: 0.125,
        },
    ),
    (
        "claude-fable-5",
        PriceEntry {
            input: 10.0,
            output: 50.0,
            cache_read: 1.0,
            cache_write: 12.5,
        },
    ),
    (
        "claude-opus-4-8",
        PriceEntry {
            input: 5.0,
            output: 25.0,
            cache_read: 0.5,
            cache_write: 6.25,
        },
    ),
    (
        "claude-opus-4-7",
        PriceEntry {
            input: 5.0,
            output: 25.0,
            cache_read: 0.5,
            cache_write: 6.25,
        },
    ),
    (
        "claude-opus-4-6",
        PriceEntry {
            input: 5.0,
            output: 25.0,
            cache_read: 0.5,
            cache_write: 6.25,
        },
    ),
    (
        "claude-opus-4-5",
        PriceEntry {
            input: 5.0,
            output: 25.0,
            cache_read: 0.5,
            cache_write: 6.25,
        },
    ),
    (
        "claude-opus-4-1",
        PriceEntry {
            input: 15.0,
            output: 75.0,
            cache_read: 1.5,
            cache_write: 18.75,
        },
    ),
    (
        "claude-opus-4",
        PriceEntry {
            input: 15.0,
            output: 75.0,
            cache_read: 1.5,
            cache_write: 18.75,
        },
    ),
    (
        "claude-sonnet-4-6",
        PriceEntry {
            input: 3.0,
            output: 15.0,
            cache_read: 0.3,
            cache_write: 3.75,
        },
    ),
    (
        "claude-sonnet-4-5",
        PriceEntry {
            input: 3.0,
            output: 15.0,
            cache_read: 0.3,
            cache_write: 3.75,
        },
    ),
    (
        "claude-sonnet-4",
        PriceEntry {
            input: 3.0,
            output: 15.0,
            cache_read: 0.3,
            cache_write: 3.75,
        },
    ),
    (
        "claude-3-7-sonnet",
        PriceEntry {
            input: 3.0,
            output: 15.0,
            cache_read: 0.3,
            cache_write: 3.75,
        },
    ),
    (
        "claude-haiku-4-5",
        PriceEntry {
            input: 1.0,
            output: 5.0,
            cache_read: 0.1,
            cache_write: 1.25,
        },
    ),
    (
        "claude-3-5-haiku",
        PriceEntry {
            input: 0.8,
            output: 4.0,
            cache_read: 0.08,
            cache_write: 1.0,
        },
    ),
    (
        "claude-3-opus",
        PriceEntry {
            input: 15.0,
            output: 75.0,
            cache_read: 1.5,
            cache_write: 18.75,
        },
    ),
    (
        "claude-3-haiku",
        PriceEntry {
            input: 0.25,
            output: 1.25,
            cache_read: 0.03,
            cache_write: 0.3,
        },
    ),
    // OpenAI (Codex)
    // Codex's built-in approval reviewer. It is not billed.
    (
        "codex-auto-review",
        PriceEntry {
            input: 0.0,
            output: 0.0,
            cache_read: 0.0,
            cache_write: 0.0,
        },
    ),
    // Short-context rates, for up to 272K input tokens. A longer prompt costs more, which this table does not model.
    // GPT-6: cache_write uses OpenAI's 30-minute cache-write tier.
    (
        "gpt-6-astra",
        PriceEntry {
            input: 10.0,
            output: 50.0,
            cache_read: 1.0,
            cache_write: 12.5,
        },
    ),
    (
        "gpt-6.1-sol",
        PriceEntry {
            input: 2.0,
            output: 10.0,
            cache_read: 0.1,
            cache_write: 2.5,
        },
    ),
    (
        "gpt-6-sol",
        PriceEntry {
            input: 2.0,
            output: 10.0,
            cache_read: 0.2,
            cache_write: 2.5,
        },
    ),
    (
        "gpt-6.1-terra",
        PriceEntry {
            input: 1.0,
            output: 5.0,
            cache_read: 0.05,
            cache_write: 1.25,
        },
    ),
    (
        "gpt-6-luna",
        PriceEntry {
            input: 0.1,
            output: 0.5,
            cache_read: 0.01,
            cache_write: 0.125,
        },
    ),
    (
        "gpt-5",
        PriceEntry {
            input: 1.25,
            output: 10.0,
            cache_read: 0.125,
            cache_write: 0.0,
        },
    ),
    (
        "gpt-5-codex",
        PriceEntry {
            input: 1.25,
            output: 10.0,
            cache_read: 0.125,
            cache_write: 0.0,
        },
    ),
    (
        "gpt-5-codex-mini",
        PriceEntry {
            input: 0.25,
            output: 2.0,
            cache_read: 0.025,
            cache_write: 0.0,
        },
    ),
    (
        "gpt-5.1",
        PriceEntry {
            input: 1.75,
            output: 14.0,
            cache_read: 0.175,
            cache_write: 0.0,
        },
    ),
    (
        "gpt-5.1-codex-max",
        PriceEntry {
            input: 1.75,
            output: 14.0,
            cache_read: 0.175,
            cache_write: 0.0,
        },
    ),
    (
        "gpt-5.1-codex-mini",
        PriceEntry {
            input: 0.25,
            output: 2.0,
            cache_read: 0.025,
            cache_write: 0.0,
        },
    ),
    (
        "gpt-5.2",
        PriceEntry {
            input: 1.75,
            output: 14.0,
            cache_read: 0.175,
            cache_write: 0.0,
        },
    ),
    (
        "gpt-5.2-codex",
        PriceEntry {
            input: 1.75,
            output: 14.0,
            cache_read: 0.175,
            cache_write: 0.0,
        },
    ),
    (
        "gpt-5.3",
        PriceEntry {
            input: 1.75,
            output: 14.0,
            cache_read: 0.175,
            cache_write: 0.0,
        },
    ),
    (
        "gpt-5.3-codex",
        PriceEntry {
            input: 1.75,
            output: 14.0,
            cache_read: 0.175,
            cache_write: 0.0,
        },
    ),
    (
        "gpt-5.3-chat-latest",
        PriceEntry {
            input: 1.75,
            output: 14.0,
            cache_read: 0.175,
            cache_write: 0.0,
        },
    ),
    (
        "gpt-5.4",
        PriceEntry {
            input: 2.5,
            output: 15.0,
            cache_read: 0.25,
            cache_write: 0.0,
        },
    ),
    (
        "gpt-5.4-codex",
        PriceEntry {
            input: 2.5,
            output: 15.0,
            cache_read: 0.25,
            cache_write: 0.0,
        },
    ),
    (
        "gpt-5.4-mini",
        PriceEntry {
            input: 0.75,
            output: 4.5,
            cache_read: 0.075,
            cache_write: 0.0,
        },
    ),
    (
        "gpt-5.4-nano",
        PriceEntry {
            input: 0.2,
            output: 1.25,
            cache_read: 0.02,
            cache_write: 0.0,
        },
    ),
    (
        "gpt-5.4-pro",
        PriceEntry {
            input: 30.0,
            output: 180.0,
            cache_read: 0.0,
            cache_write: 0.0,
        },
    ),
    (
        "gpt-5.5",
        PriceEntry {
            input: 5.0,
            output: 30.0,
            cache_read: 0.5,
            cache_write: 0.0,
        },
    ),
    (
        "gpt-5.5-pro",
        PriceEntry {
            input: 30.0,
            output: 180.0,
            cache_read: 0.0,
            cache_write: 0.0,
        },
    ),
    (
        "o4-mini",
        PriceEntry {
            input: 1.1,
            output: 4.4,
            cache_read: 0.275,
            cache_write: 0.0,
        },
    ),
];

/// A prompt (input + cache read + cache write tokens of one event) longer than this
/// bills the whole event at the model's `LONG_PROMPT_PRICES` rate.
const LONG_PROMPT_TOKENS: u64 = 100_000;

const LONG_PROMPT_PRICES: &[(&str, PriceEntry)] = &[(
    "claude-haiku-5-5",
    PriceEntry {
        input: 0.5,
        output: 2.5,
        cache_read: 0.05,
        cache_write: 0.625,
    },
)];

/// Claude model IDs sometimes ship with a trailing `-YYYYMMDD` date suffix or a
/// `[1m]` context suffix, in either order. Anthropic charges no long-context premium; strip both.
pub fn normalize_model_id(model: &str) -> &str {
    let mut model = model;
    loop {
        let stripped = strip_context_suffix(strip_date_suffix(model));
        if stripped.len() == model.len() {
            return model;
        }
        model = stripped;
    }
}

fn strip_context_suffix(model: &str) -> &str {
    match model.strip_suffix(']').and_then(|m| m.rfind('[')) {
        Some(open) => &model[..open],
        None => model,
    }
}

fn strip_date_suffix(model: &str) -> &str {
    let bytes = model.as_bytes();
    if bytes.len() < 9 {
        return model;
    }
    let tail = &bytes[bytes.len() - 9..];
    if tail[0] == b'-' && tail[1..].iter().all(|b| b.is_ascii_digit()) {
        &model[..model.len() - 9]
    } else {
        model
    }
}

fn price_table() -> &'static HashMap<&'static str, &'static PriceEntry> {
    static TABLE: OnceLock<HashMap<&'static str, &'static PriceEntry>> = OnceLock::new();
    TABLE.get_or_init(|| PRICES.iter().map(|(k, v)| (*k, v)).collect())
}

fn lookup(model: &str) -> Option<&'static PriceEntry> {
    let key = normalize_model_id(model);
    price_table().get(key).copied()
}

fn rate_for(model: &str, event: &UsageEvent) -> Option<&'static PriceEntry> {
    let standard = lookup(model)?;
    let prompt_tokens = event.input_tokens + event.cache_read_tokens + event.cache_write_tokens;
    if prompt_tokens > LONG_PROMPT_TOKENS {
        let key = normalize_model_id(model);
        if let Some((_, long)) = LONG_PROMPT_PRICES.iter().find(|(k, _)| *k == key) {
            return Some(long);
        }
    }
    Some(standard)
}

pub fn has_price(model: &str) -> bool {
    lookup(model).is_some()
}

pub fn cost_of(event: &UsageEvent, unknown: Option<&mut HashSet<String>>) -> f64 {
    if let Some(cost) = event.explicit_cost_usd {
        return cost;
    }
    if event.source == crate::types::Source::Cursor {
        return 0.0;
    }
    let Some(p) = rate_for(&event.model, event) else {
        if let Some(set) = unknown {
            set.insert(event.model.clone());
        }
        return 0.0;
    };
    const M: f64 = 1_000_000.0;
    (event.input_tokens as f64 * p.input) / M
        + (event.output_tokens as f64 * p.output) / M
        + (event.cache_read_tokens as f64 * p.cache_read) / M
        + (event.cache_write_tokens as f64 * p.cache_write) / M
}

/// Fingerprint the price catalog and the rules that interpret it.
pub fn signature() -> String {
    static SIGNATURE: OnceLock<String> = OnceLock::new();
    SIGNATURE
        .get_or_init(|| format!("{:x}", Sha256::digest(include_bytes!("pricing.rs"))))
        .clone()
}

pub fn provenance(event: &UsageEvent) -> String {
    if event.explicit_cost_usd.is_some() {
        "source".to_owned()
    } else {
        signature()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Source;
    use chrono::Utc;

    fn event(model: &str, input: u64, output: u64, cr: u64, cw: u64) -> UsageEvent {
        UsageEvent {
            source: Source::Claude,
            timestamp: Utc::now(),
            session_id: "s".into(),
            project_path: None,
            model: model.into(),
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: cr,
            cache_write_tokens: cw,
            explicit_cost_usd: None,
        }
    }

    #[test]
    fn known_model_cost() {
        let e = event("claude-sonnet-4-6", 1_000_000, 0, 0, 0);
        assert!((cost_of(&e, None) - 3.0).abs() < 1e-9);
    }

    #[test]
    fn updated_opus_price_is_used() {
        let e = event(
            "claude-opus-4-7",
            1_000_000,
            1_000_000,
            1_000_000,
            1_000_000,
        );
        assert!((cost_of(&e, None) - 36.75).abs() < 1e-9);
    }

    #[test]
    fn fable_price_is_used() {
        // $10 input + $50 output + $1 cache read + $12.50 cache write per MTok
        let e = event("claude-fable-5", 1_000_000, 1_000_000, 1_000_000, 1_000_000);
        assert!((cost_of(&e, None) - 73.5).abs() < 1e-9);
    }

    #[test]
    fn fable_5_2_price_is_used() {
        // $8 input + $40 output + $0.20 cache read + $10 cache write per MTok
        let e = event(
            "claude-fable-5-2",
            1_000_000,
            1_000_000,
            1_000_000,
            1_000_000,
        );
        assert!((cost_of(&e, None) - 58.2).abs() < 1e-9);
    }

    #[test]
    fn haiku_long_prompt_uses_higher_rate_for_whole_event() {
        // Prompt is 60k input + 40k cache read = 100k: not over the threshold.
        let at_limit = event("claude-haiku-5-5", 60_000, 1_000_000, 40_000, 0);
        let standard = 60_000.0 * 0.1 / 1e6 + 1.0 * 0.5 + 40_000.0 * 0.01 / 1e6;
        assert!((cost_of(&at_limit, None) - standard).abs() < 1e-9);
        // One more token crosses it: every field uses the long-prompt rate.
        let over = event("claude-haiku-5-5", 60_001, 1_000_000, 40_000, 0);
        let long = 60_001.0 * 0.5 / 1e6 + 2.5 + 40_000.0 * 0.05 / 1e6;
        assert!((cost_of(&over, None) - long).abs() < 1e-9);
        // Cache writes count toward prompt length.
        let cache_write = event("claude-haiku-5-5", 0, 0, 0, 100_001);
        assert!((cost_of(&cache_write, None) - 100_001.0 * 0.625 / 1e6).abs() < 1e-9);
    }

    #[test]
    fn cache_tokens_contribute() {
        let e = event("claude-sonnet-4-6", 0, 0, 1_000_000, 0);
        assert!((cost_of(&e, None) - 0.3).abs() < 1e-9);
    }

    #[test]
    fn unknown_model_zero_and_tracked() {
        let e = event("gpt-99-ultra", 1000, 0, 0, 0);
        let mut unknown = HashSet::new();
        assert_eq!(cost_of(&e, Some(&mut unknown)), 0.0);
        assert!(unknown.contains("gpt-99-ultra"));
    }

    #[test]
    fn date_suffix_stripped() {
        let e = event("claude-sonnet-4-6-20250101", 1_000_000, 0, 0, 0);
        assert!((cost_of(&e, None) - 3.0).abs() < 1e-9);
    }

    #[test]
    fn context_suffix_stripped_in_either_order_with_date() {
        assert_eq!(normalize_model_id("claude-opus-5-5[1m]"), "claude-opus-5-5");
        assert_eq!(
            normalize_model_id("claude-opus-5-5-20261001[1m]"),
            "claude-opus-5-5"
        );
        assert_eq!(
            normalize_model_id("claude-opus-5-5[1m]-20261001"),
            "claude-opus-5-5"
        );
        assert!(has_price("claude-fable-5[1m]"));
    }

    #[test]
    fn auto_review_is_priced_at_zero() {
        assert!(has_price("codex-auto-review"));
        let mut unknown = HashSet::new();
        let e = event("codex-auto-review", 1_000_000, 1_000_000, 0, 0);
        assert_eq!(cost_of(&e, Some(&mut unknown)), 0.0);
        assert!(unknown.is_empty());
    }

    #[test]
    fn normalize_preserves_non_date_suffixes() {
        assert_eq!(normalize_model_id("gpt-5.4-codex"), "gpt-5.4-codex");
        assert_eq!(normalize_model_id("claude-sonnet-4-6"), "claude-sonnet-4-6");
    }

    #[test]
    fn has_price_works() {
        assert!(has_price("claude-opus-4-7"));
        assert!(has_price("claude-3-5-haiku-20241022"));
        assert!(has_price("claude-sonnet-4-6-20250101"));
        assert!(has_price("claude-opus-5-5"));
        assert!(has_price("gpt-6.1-sol"));
        assert!(has_price("gpt-5.5"));
        assert!(has_price("gpt-5.5-pro"));
        assert!(!has_price("gpt-99-ultra"));
    }

    #[test]
    fn explicit_cost_wins() {
        let mut e = event("auto", 10, 20, 30, 40);
        e.explicit_cost_usd = Some(0.19);
        let mut unknown = HashSet::new();
        assert_eq!(cost_of(&e, Some(&mut unknown)), 0.19);
        assert!(unknown.is_empty());
    }

    #[test]
    fn cursor_unknown_models_do_not_warn() {
        let mut e = event("composer-1", 10, 20, 30, 40);
        e.source = Source::Cursor;
        let mut unknown = HashSet::new();
        assert_eq!(cost_of(&e, Some(&mut unknown)), 0.0);
        assert!(unknown.is_empty());
    }
}
