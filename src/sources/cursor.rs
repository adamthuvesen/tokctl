use crate::types::{Source, UsageEvent};
use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeZone, Utc};
use csv::StringRecord;
use sha2::{Digest, Sha256};
use std::path::Path;

pub fn parse_cursor_csv(path: &Path) -> (Vec<UsageEvent>, usize) {
    let mut reader = match csv::ReaderBuilder::new().flexible(true).from_path(path) {
        Ok(reader) => reader,
        Err(_) => return (Vec::new(), 1),
    };

    let header_fields = match reader.headers() {
        Ok(headers) => headers.clone(),
        Err(_) => return (Vec::new(), 1),
    };
    let Some(indexes) = CursorColumns::from_header(&header_fields) else {
        return (Vec::new(), 0);
    };

    let session_prefix = session_prefix(path);

    let mut events = Vec::new();
    let mut skipped = 0usize;
    for (row_idx, record) in reader.records().enumerate() {
        match record {
            Ok(fields) => {
                let is_empty = fields.iter().all(|field| field.trim().is_empty());
                if is_empty {
                    continue;
                }
                let Some(event) = parse_row(&fields, indexes, &session_prefix, row_idx + 1) else {
                    skipped += 1;
                    continue;
                };
                events.push(event);
            }
            Err(_) => skipped += 1,
        }
    }
    (events, skipped)
}

#[derive(Debug, Clone, Copy)]
struct CursorColumns {
    date: usize,
    model: usize,
    input_with_cache_write: usize,
    input_without_cache_write: usize,
    cache_read: usize,
    output_tokens: usize,
    cost: Option<usize>,
}

impl CursorColumns {
    fn from_header(fields: &StringRecord) -> Option<Self> {
        let normalized: Vec<String> = fields.iter().map(normalize_header).collect();
        let find = |name: &str| normalized.iter().position(|f| f == name);

        Some(Self {
            date: find("date")?,
            model: find("model")?,
            input_with_cache_write: find("input (w/ cache write)")?,
            input_without_cache_write: find("input (w/o cache write)")?,
            cache_read: find("cache read")?,
            output_tokens: find("output tokens")?,
            cost: find("cost"),
        })
    }
}

fn normalize_header(value: &str) -> String {
    value.trim().trim_matches('"').to_ascii_lowercase()
}

fn session_prefix(path: &Path) -> String {
    let stable_path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mut hasher = Sha256::new();
    hasher.update(stable_path.to_string_lossy().as_bytes());
    let hash = format!("{:x}", hasher.finalize());
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("cursor");
    format!("{stem}-{}", &hash[..12])
}

fn parse_row(
    fields: &StringRecord,
    columns: CursorColumns,
    session_prefix: &str,
    row_idx: usize,
) -> Option<UsageEvent> {
    let model = get_field(fields, columns.model)?;
    if model.is_empty() {
        return None;
    }
    let timestamp = parse_cursor_timestamp(get_field(fields, columns.date)?)?;
    let input_with_cache_write = parse_u64(get_field(fields, columns.input_with_cache_write)?);
    let input_without_cache_write =
        parse_u64(get_field(fields, columns.input_without_cache_write)?);
    let cache_read_tokens = parse_u64(get_field(fields, columns.cache_read)?);
    let output_tokens = parse_u64(get_field(fields, columns.output_tokens)?);
    let cache_write_tokens = input_with_cache_write.saturating_sub(input_without_cache_write);
    let explicit_cost_usd = columns
        .cost
        .and_then(|idx| get_field(fields, idx))
        .and_then(parse_cost);

    let event = UsageEvent {
        source: Source::Cursor,
        timestamp,
        session_id: format!("{session_prefix}-{row_idx}"),
        project_path: None,
        model: model.to_owned(),
        input_tokens: input_without_cache_write,
        output_tokens,
        cache_read_tokens,
        cache_write_tokens,
        explicit_cost_usd,
    };

    if event.input_tokens + event.output_tokens + event.cache_read_tokens + event.cache_write_tokens
        == 0
        && event.explicit_cost_usd.unwrap_or(0.0) == 0.0
    {
        return None;
    }

    Some(event)
}

fn get_field(fields: &StringRecord, idx: usize) -> Option<&str> {
    fields.get(idx).map(str::trim)
}

fn parse_u64(raw: &str) -> u64 {
    raw.trim()
        .trim_matches('"')
        .replace(',', "")
        .parse()
        .unwrap_or(0)
}

fn parse_cost(raw: &str) -> Option<f64> {
    let cleaned = raw.trim().trim_matches('"').replace(['$', ','], "");
    if cleaned.is_empty()
        || cleaned.eq_ignore_ascii_case("nan")
        || cleaned.eq_ignore_ascii_case("included")
        || cleaned == "-"
    {
        return Some(0.0);
    }
    cleaned.parse().ok()
}

fn parse_cursor_timestamp(raw: &str) -> Option<DateTime<Utc>> {
    let raw = raw.trim().trim_matches('"');
    if raw.is_empty() {
        return None;
    }

    if let Ok(dt) = DateTime::parse_from_rfc3339(raw) {
        return Some(dt.with_timezone(&Utc));
    }
    for fmt in [
        "%Y-%m-%dT%H:%M:%S%.3fZ",
        "%Y-%m-%dT%H:%M:%SZ",
        "%Y-%m-%dT%H:%M:%S%.3f",
        "%Y-%m-%dT%H:%M:%S",
    ] {
        if let Ok(dt) = NaiveDateTime::parse_from_str(raw, fmt) {
            return Some(Utc.from_utc_datetime(&dt));
        }
    }
    if let Ok(date) = NaiveDate::parse_from_str(raw, "%Y-%m-%d") {
        let dt = date.and_hms_opt(12, 0, 0)?;
        return Some(Utc.from_utc_datetime(&dt));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_cursor_format() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("usage.csv");
        std::fs::write(
            &path,
            "Date,Kind,Model,Max Mode,Input (w/ Cache Write),Input (w/o Cache Write),Cache Read,Output Tokens,Total Tokens,Cost\n\
             2025-11-13T13:35:04.658Z,On-Demand,gpt-5-codex,No,1000,700,4000,250,4950,$0.19\n",
        )
        .unwrap();

        let (events, skipped) = parse_cursor_csv(&path);
        assert_eq!(skipped, 0);
        assert_eq!(events.len(), 1);
        let event = &events[0];
        assert_eq!(event.source, Source::Cursor);
        assert_eq!(event.input_tokens, 700);
        assert_eq!(event.cache_write_tokens, 300);
        assert_eq!(event.cache_read_tokens, 4000);
        assert_eq!(event.explicit_cost_usd, Some(0.19));
    }

    #[test]
    fn synthetic_session_ids_include_file_identity() {
        let dir = tempfile::tempdir().unwrap();
        let a_dir = dir.path().join("a");
        let b_dir = dir.path().join("b");
        std::fs::create_dir_all(&a_dir).unwrap();
        std::fs::create_dir_all(&b_dir).unwrap();
        let a = a_dir.join("usage.csv");
        let b = b_dir.join("usage.csv");
        let csv = "Date,Model,Input (w/ Cache Write),Input (w/o Cache Write),Cache Read,Output Tokens,Cost\n\
                   2025-11-13,gpt-5-codex,10,5,0,1,$0.01\n";
        std::fs::write(&a, csv).unwrap();
        std::fs::write(&b, csv).unwrap();

        let (a_events, _) = parse_cursor_csv(&a);
        let (b_events, _) = parse_cursor_csv(&b);

        assert_ne!(a_events[0].session_id, b_events[0].session_id);
        assert!(a_events[0].session_id.starts_with("usage-"));
        assert!(b_events[0].session_id.starts_with("usage-"));
    }

    #[test]
    fn parses_quoted_csv_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("usage.csv");
        std::fs::write(
            &path,
            "Date,Model,Input (w/ Cache Write),Input (w/o Cache Write),Cache Read,Output Tokens,Cost\n\
             2025-11-13,\"gpt-5, codex\",10,5,0,1,\"$1,234.56\"\n",
        )
        .unwrap();

        let (events, skipped) = parse_cursor_csv(&path);

        assert_eq!(skipped, 0);
        assert_eq!(events[0].model, "gpt-5, codex");
        assert_eq!(events[0].explicit_cost_usd, Some(1234.56));
    }

    #[test]
    fn date_only_rows_land_at_noon_utc() {
        let timestamp = parse_cursor_timestamp("2025-02-05").unwrap();
        assert_eq!(timestamp.to_rfc3339(), "2025-02-05T12:00:00+00:00");
    }

    #[test]
    fn included_cost_is_zero() {
        assert_eq!(parse_cost("Included"), Some(0.0));
        assert_eq!(parse_cost("-"), Some(0.0));
    }
}
