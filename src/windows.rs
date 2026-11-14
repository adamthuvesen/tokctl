use anyhow::{Context, Result};
use chrono::{Datelike, Duration, NaiveDate, Utc};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct CompareWindow {
    pub label: String,
    pub start: String,
    pub end: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompareWindows {
    pub current: CompareWindow,
    pub baseline: CompareWindow,
}

pub fn resolve_windows(
    current: Option<&str>,
    baseline: Option<&str>,
    now: chrono::DateTime<Utc>,
) -> Result<CompareWindows> {
    let local_today = crate::calendar::Calendar::system()?.local_date(now);
    match (current, baseline) {
        (None, None) => {
            let current = month_to_date_window(local_today, "this-month");
            let baseline = previous_month_comparable(&current, "last-month")?;
            Ok(CompareWindows { current, baseline })
        }
        (Some(cur), Some(base)) => Ok(CompareWindows {
            current: parse_window(cur, local_today)?,
            baseline: parse_window(base, local_today)?,
        }),
        (Some(cur), None) => {
            let current = parse_window(cur, local_today)?;
            let baseline = match cur {
                "today" => parse_window("yesterday", local_today)?,
                "this-week" => parse_window("last-week", local_today)?,
                "this-month" => parse_window("last-month", local_today)?,
                "this-year" => parse_window("last-year", local_today)?,
                _ => previous_period(&current)?,
            };
            Ok(CompareWindows { current, baseline })
        }
        (None, Some(_)) => anyhow::bail!("baseline window requires a current window"),
    }
}

fn parse_window(raw: &str, today: NaiveDate) -> Result<CompareWindow> {
    if let Some((start, end)) = raw.split_once("..") {
        let start = parse_day(start)?;
        let end = parse_day(end)?;
        if end < start {
            anyhow::bail!("comparison range end must be on or after start");
        }
        return Ok(window(raw, start, end));
    }

    match raw {
        "today" => Ok(window(raw, today, today)),
        "yesterday" => {
            let d = today - Duration::days(1);
            Ok(window(raw, d, d))
        }
        "this-week" => {
            let start = today - Duration::days(today.weekday().num_days_from_monday() as i64);
            Ok(window(raw, start, today))
        }
        "last-week" => {
            let this_start = today - Duration::days(today.weekday().num_days_from_monday() as i64);
            let start = this_start - Duration::days(7);
            Ok(window(raw, start, start + Duration::days(6)))
        }
        "this-month" => Ok(month_to_date_window(today, raw)),
        "last-month" => {
            let current = month_to_date_window(today, "this-month");
            previous_month_comparable(&current, raw)
        }
        "this-year" => Ok(window(
            raw,
            NaiveDate::from_ymd_opt(today.year(), 1, 1).unwrap(),
            today,
        )),
        "last-year" => {
            let start = NaiveDate::from_ymd_opt(today.year() - 1, 1, 1).unwrap();
            let day = today.ordinal().min(days_in_year(today.year() - 1));
            let end = NaiveDate::from_yo_opt(today.year() - 1, day).unwrap();
            Ok(window(raw, start, end))
        }
        _ => anyhow::bail!(
            "invalid comparison window '{raw}'; use a preset or YYYY-MM-DD..YYYY-MM-DD"
        ),
    }
}

pub(crate) fn parse_day(raw: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .with_context(|| format!("invalid date '{raw}', expected YYYY-MM-DD"))
}

fn month_to_date_window(today: NaiveDate, label: &str) -> CompareWindow {
    let start = NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap();
    window(label, start, today)
}

fn previous_month_comparable(current: &CompareWindow, label: &str) -> Result<CompareWindow> {
    let current_start = parse_day(&current.start)?;
    let current_end = parse_day(&current.end)?;
    let (year, month) = if current_start.month() == 1 {
        (current_start.year() - 1, 12)
    } else {
        (current_start.year(), current_start.month() - 1)
    };
    let start = NaiveDate::from_ymd_opt(year, month, 1).unwrap();
    let span_days = (current_end - current_start).num_days();
    let end_day = (1 + span_days as u32).min(days_in_month(year, month));
    let end = NaiveDate::from_ymd_opt(year, month, end_day).unwrap();
    Ok(window(label, start, end))
}

fn previous_period(current: &CompareWindow) -> Result<CompareWindow> {
    let start = parse_day(&current.start)?;
    let end = parse_day(&current.end)?;
    let span = (end - start).num_days() + 1;
    let baseline_end = start - Duration::days(1);
    let baseline_start = baseline_end - Duration::days(span - 1);
    Ok(window("previous", baseline_start, baseline_end))
}

fn days_in_month(year: i32, month: u32) -> u32 {
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    (NaiveDate::from_ymd_opt(next_year, next_month, 1).unwrap() - Duration::days(1)).day()
}

fn days_in_year(year: i32) -> u32 {
    NaiveDate::from_ymd_opt(year, 12, 31).unwrap().ordinal()
}

pub(crate) fn window(label: &str, start: NaiveDate, end: NaiveDate) -> CompareWindow {
    CompareWindow {
        label: label.to_owned(),
        start: start.format("%Y-%m-%d").to_string(),
        end: end.format("%Y-%m-%d").to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn default_windows_are_month_to_date_vs_previous_month() {
        let now = Utc.with_ymd_and_hms(2026, 4, 25, 12, 0, 0).unwrap();
        let windows = resolve_windows(None, None, now).unwrap();
        assert_eq!(windows.current.start, "2026-04-01");
        assert_eq!(windows.current.end, "2026-04-25");
        assert_eq!(windows.baseline.start, "2026-03-01");
        assert_eq!(windows.baseline.end, "2026-03-25");
    }

    #[test]
    fn explicit_range_parses() {
        let today = Utc.with_ymd_and_hms(2026, 4, 25, 12, 0, 0).unwrap();
        let windows = resolve_windows(
            Some("2026-04-01..2026-04-15"),
            Some("2026-03-01..2026-03-15"),
            today,
        )
        .unwrap();
        assert_eq!(windows.current.end, "2026-04-15");
    }
}
