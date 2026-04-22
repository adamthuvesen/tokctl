use chrono::{DateTime, Duration, Local, Utc};

/// Thousands-separated count, matching `render::fmt_num`.
pub fn fmt_num(n: u64) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}

pub fn fmt_cost(n: f64) -> String {
    format!("${:.2}", n)
}

/// Short-form for large token counts: `340` / `12K` / `4.2M`.
pub fn fmt_tokens_short(n: u64) -> String {
    if n < 1_000 {
        format!("{n}")
    } else if n < 1_000_000 {
        format!("{:.1}K", (n as f64) / 1_000.0)
    } else {
        format!("{:.1}M", (n as f64) / 1_000_000.0)
    }
}

/// `3m ago`, `2h ago`, `yesterday`, `3d ago`, `2026-04-12`.
pub fn relative_time(ts: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let delta = now.signed_duration_since(ts);
    if delta < Duration::seconds(0) {
        return ts.with_timezone(&Local).format("%Y-%m-%d").to_string();
    }
    if delta < Duration::minutes(1) {
        return "just now".into();
    }
    if delta < Duration::hours(1) {
        return format!("{}m ago", delta.num_minutes());
    }
    if delta < Duration::hours(24) {
        return format!("{}h ago", delta.num_hours());
    }
    // Compare local calendar days for "yesterday".
    let today = now.with_timezone(&Local).date_naive();
    let then = ts.with_timezone(&Local).date_naive();
    let day_delta = today.signed_duration_since(then).num_days();
    if day_delta == 1 {
        return "yesterday".into();
    }
    if day_delta < 7 {
        return format!("{}d ago", day_delta);
    }
    ts.with_timezone(&Local).format("%Y-%m-%d").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 4, 22, 16, 30, 0).unwrap()
    }

    #[test]
    fn minutes() {
        let n = now();
        assert_eq!(relative_time(n - Duration::minutes(3), n), "3m ago");
    }
    #[test]
    fn hours() {
        let n = now();
        assert_eq!(relative_time(n - Duration::hours(2), n), "2h ago");
    }
    #[test]
    fn days() {
        let n = now();
        let three_days = n - Duration::days(3);
        assert_eq!(relative_time(three_days, n), "3d ago");
    }
    #[test]
    fn long_ago_date() {
        let n = now();
        let long_ago = n - Duration::days(40);
        let s = relative_time(long_ago, n);
        assert!(s.starts_with("2026-") || s.starts_with("2026"));
    }
    #[test]
    fn short_tokens() {
        assert_eq!(fmt_tokens_short(340), "340");
        assert_eq!(fmt_tokens_short(12_000), "12.0K");
        assert_eq!(fmt_tokens_short(4_200_000), "4.2M");
    }
}
