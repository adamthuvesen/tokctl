pub fn fmt_num(n: u64) -> String {
    fmt_wide_num(u128::from(n))
}

pub fn fmt_wide_num(n: u128) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}

/// Compact token count for wide tables: `340` / `12.3K` / `4.20M` / `1.50B`.
pub fn fmt_compact(n: u64) -> String {
    const UNITS: [(u64, &str); 3] = [(1_000_000_000, "B"), (1_000_000, "M"), (1_000, "K")];
    for (size, suffix) in UNITS {
        if n >= size {
            let value = n as f64 / size as f64;
            return if value >= 100.0 {
                format!("{value:.0}{suffix}")
            } else if value >= 10.0 {
                format!("{value:.1}{suffix}")
            } else {
                format!("{value:.2}{suffix}")
            };
        }
    }
    n.to_string()
}

#[cfg(test)]
mod tests {
    use super::fmt_compact;

    #[test]
    fn compact_counts_keep_three_significant_digits() {
        assert_eq!(fmt_compact(0), "0");
        assert_eq!(fmt_compact(999), "999");
        assert_eq!(fmt_compact(12_345), "12.3K");
        assert_eq!(fmt_compact(4_200_000), "4.20M");
        assert_eq!(fmt_compact(1_500_000_000), "1.50B");
        assert_eq!(fmt_compact(123_456_789_012), "123B");
    }
}
