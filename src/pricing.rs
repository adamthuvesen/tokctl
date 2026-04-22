use crate::types::UsageEvent;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy)]
pub struct PriceEntry {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

// USD per 1M tokens. Last verified 2026-04.
const PRICES: &[(&str, PriceEntry)] = &[
    // Anthropic
    (
        "claude-opus-4-7",
        PriceEntry {
            input: 15.0,
            output: 75.0,
            cache_read: 1.5,
            cache_write: 18.75,
        },
    ),
    (
        "claude-opus-4-6",
        PriceEntry {
            input: 15.0,
            output: 75.0,
            cache_read: 1.5,
            cache_write: 18.75,
        },
    ),
    (
        "claude-opus-4-5",
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
        "claude-haiku-4-5",
        PriceEntry {
            input: 1.0,
            output: 5.0,
            cache_read: 0.1,
            cache_write: 1.25,
        },
    ),
    // OpenAI (Codex)
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
        "o4-mini",
        PriceEntry {
            input: 1.1,
            output: 4.4,
            cache_read: 0.275,
            cache_write: 0.0,
        },
    ),
];

/// Claude model IDs sometimes ship with a trailing `-YYYYMMDD` date suffix; strip it.
pub fn normalize_model_id(model: &str) -> &str {
    let bytes = model.as_bytes();
    if bytes.len() < 9 {
        return model;
    }
    let tail = &bytes[bytes.len() - 9..];
    if tail[0] != b'-' {
        return model;
    }
    if tail[1..].iter().all(|b| b.is_ascii_digit()) {
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

pub fn has_price(model: &str) -> bool {
    lookup(model).is_some()
}

pub fn cost_of(event: &UsageEvent, unknown: Option<&mut HashSet<String>>) -> f64 {
    let Some(p) = lookup(&event.model) else {
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
        }
    }

    #[test]
    fn known_model_cost() {
        let e = event("claude-sonnet-4-6", 1_000_000, 0, 0, 0);
        assert!((cost_of(&e, None) - 3.0).abs() < 1e-9);
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
    fn normalize_preserves_non_date_suffixes() {
        assert_eq!(normalize_model_id("gpt-5.4-codex"), "gpt-5.4-codex");
        assert_eq!(normalize_model_id("claude-sonnet-4-6"), "claude-sonnet-4-6");
    }

    #[test]
    fn has_price_works() {
        assert!(has_price("claude-opus-4-7"));
        assert!(has_price("claude-sonnet-4-6-20250101"));
        assert!(!has_price("gpt-99-ultra"));
    }
}
