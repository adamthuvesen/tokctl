use anyhow::{Context, Result};
use chrono::{Datelike, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

use crate::calendar::Calendar;

/// A rolling spend window ending now. `D7` is the local today plus the 6
/// previous calendar days, starting at local midnight. The TUI and the CLI
/// share this one definition.
///
/// Serialized names are the original TUI names so existing `ui_state.json`
/// files keep loading.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum TimeWindow {
    #[serde(rename = "today")]
    D1,
    #[serde(rename = "week")]
    D7,
    #[serde(rename = "month")]
    D30,
    #[serde(rename = "year")]
    D365,
    #[serde(rename = "all")]
    All,
}

impl TimeWindow {
    pub const ALL: [TimeWindow; 5] = [Self::D1, Self::D7, Self::D30, Self::D365, Self::All];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::D1 => "1d",
            Self::D7 => "7d",
            Self::D30 => "30d",
            Self::D365 => "365d",
            Self::All => "all",
        }
    }

    /// Calendar days in the window; `None` for `All`.
    pub fn days(self) -> Option<i64> {
        match self {
            Self::D1 => Some(1),
            Self::D7 => Some(7),
            Self::D30 => Some(30),
            Self::D365 => Some(365),
            Self::All => None,
        }
    }

    /// Millisecond lower bound for this window against local `now`; `None` for `All`.
    pub fn since_ms(self, now: chrono::DateTime<Utc>) -> Result<Option<i64>> {
        self.since_ms_in(&Calendar::system()?, now)
    }

    /// The window of the same length that ends where this one starts, as
    /// `[start, end)` milliseconds; `None` for `All`.
    pub fn previous_bounds_ms(self, now: chrono::DateTime<Utc>) -> Result<Option<(i64, i64)>> {
        self.previous_bounds_ms_in(&Calendar::system()?, now)
    }

    fn start_ms(calendar: &Calendar, today: NaiveDate, days_back: i64) -> Result<i64> {
        let date = today
            .checked_sub_signed(Duration::days(days_back))
            .context("time window exceeds calendar range")?;
        Ok(calendar.day_start(date)?.timestamp_millis())
    }

    fn since_ms_in(self, calendar: &Calendar, now: chrono::DateTime<Utc>) -> Result<Option<i64>> {
        let Some(days) = self.days() else {
            return Ok(None);
        };
        Self::start_ms(calendar, calendar.local_date(now), days - 1).map(Some)
    }

    fn previous_bounds_ms_in(
        self,
        calendar: &Calendar,
        now: chrono::DateTime<Utc>,
    ) -> Result<Option<(i64, i64)>> {
        let Some(days) = self.days() else {
            return Ok(None);
        };
        let today = calendar.local_date(now);
        let start = Self::start_ms(calendar, today, 2 * days - 1)?;
        let end = Self::start_ms(calendar, today, days - 1)?;
        Ok(Some((start, end)))
    }
}

impl FromStr for TimeWindow {
    type Err = anyhow::Error;

    fn from_str(raw: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|window| window.as_str() == raw)
            .ok_or_else(|| {
                let valid: Vec<_> = Self::ALL.iter().map(|w| w.as_str()).collect();
                anyhow::anyhow!("invalid window '{raw}'; use one of: {}", valid.join(", "))
            })
    }
}

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

    fn ms(calendar: &Calendar, date: &str) -> i64 {
        calendar
            .day_start(NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap())
            .unwrap()
            .timestamp_millis()
    }

    #[test]
    fn parse_accepts_labels_and_lists_valid_values() {
        for window in TimeWindow::ALL {
            assert_eq!(window.as_str().parse::<TimeWindow>().unwrap(), window);
        }
        let error = "week".parse::<TimeWindow>().unwrap_err().to_string();
        assert_eq!(
            error,
            "invalid window 'week'; use one of: 1d, 7d, 30d, 365d, all"
        );
    }

    #[test]
    fn rolling_bounds_use_local_midnights() {
        let calendar = Calendar::named("Europe/Stockholm").unwrap();
        let now = Utc.with_ymd_and_hms(2026, 5, 20, 10, 0, 0).unwrap();
        assert_eq!(
            TimeWindow::D1.since_ms_in(&calendar, now).unwrap(),
            Some(ms(&calendar, "2026-05-20"))
        );
        assert_eq!(
            TimeWindow::D7.since_ms_in(&calendar, now).unwrap(),
            Some(ms(&calendar, "2026-05-14"))
        );
        assert_eq!(
            TimeWindow::D7
                .previous_bounds_ms_in(&calendar, now)
                .unwrap(),
            Some((ms(&calendar, "2026-05-07"), ms(&calendar, "2026-05-14")))
        );
        assert_eq!(TimeWindow::All.since_ms_in(&calendar, now).unwrap(), None);
        assert_eq!(
            TimeWindow::All
                .previous_bounds_ms_in(&calendar, now)
                .unwrap(),
            None
        );
    }

    #[test]
    fn previous_window_spans_a_dst_change() {
        // Stockholm springs forward on 2026-03-29: the previous 7d window is 167 hours.
        let calendar = Calendar::named("Europe/Stockholm").unwrap();
        let now = Utc.with_ymd_and_hms(2026, 4, 5, 10, 0, 0).unwrap();
        let (start, end) = TimeWindow::D7
            .previous_bounds_ms_in(&calendar, now)
            .unwrap()
            .unwrap();
        assert_eq!(start, ms(&calendar, "2026-03-23"));
        assert_eq!(end, ms(&calendar, "2026-03-30"));
        assert_eq!(end - start, (7 * 24 - 1) * 3_600_000);
    }

    #[test]
    fn old_ui_state_names_still_deserialize() {
        let window: TimeWindow = serde_json::from_str("\"week\"").unwrap();
        assert_eq!(window, TimeWindow::D7);
        assert_eq!(serde_json::to_string(&TimeWindow::D1).unwrap(), "\"today\"");
    }
}
