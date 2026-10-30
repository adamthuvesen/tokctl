use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Duration, Months, NaiveDate, TimeZone, Utc, Weekday};
use chrono_tz::{GapInfo, Tz};
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeriodGranularity {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

#[derive(Debug, Clone)]
pub struct Calendar {
    zone: Tz,
}

impl Calendar {
    pub fn named(name: &str) -> Result<Self> {
        let zone = name
            .parse()
            .map_err(|_| anyhow!("unsupported calendar timezone: {name}"))?;
        Ok(Self { zone })
    }

    pub fn system() -> Result<Self> {
        static SYSTEM: OnceLock<std::result::Result<Calendar, String>> = OnceLock::new();
        SYSTEM
            .get_or_init(|| {
                let name = match std::env::var("TZ") {
                    Ok(name) if !name.is_empty() => name.trim_start_matches(':').to_owned(),
                    _ => iana_time_zone::get_timezone().map_err(|error| error.to_string())?,
                };
                Self::named(&name).map_err(|error| error.to_string())
            })
            .clone()
            .map_err(anyhow::Error::msg)
    }

    pub fn identity(&self) -> String {
        use sha2::{Digest, Sha256};
        static RULES: OnceLock<String> = OnceLock::new();
        let rules =
            RULES.get_or_init(|| format!("{:x}", Sha256::digest(include_bytes!("calendar.rs"))));
        format!(
            "{}:{}:{rules}",
            self.zone.name(),
            chrono_tz::IANA_TZDB_VERSION
        )
    }

    pub fn local_day_month(&self, timestamp: DateTime<Utc>) -> (String, String) {
        let local = timestamp.with_timezone(&self.zone);
        (
            local.format("%Y-%m-%d").to_string(),
            local.format("%Y-%m").to_string(),
        )
    }

    pub fn local_date(&self, timestamp: DateTime<Utc>) -> NaiveDate {
        timestamp.with_timezone(&self.zone).date_naive()
    }

    pub fn day_start(&self, date: NaiveDate) -> Result<DateTime<Utc>> {
        let midnight = date.and_hms_opt(0, 0, 0).unwrap();
        self.zone
            .from_local_datetime(&midnight)
            .earliest()
            .or_else(|| GapInfo::new(&midnight, &self.zone).and_then(|gap| gap.end))
            .filter(|timestamp| timestamp.date_naive() == date)
            .map(|timestamp| timestamp.with_timezone(&Utc))
            .ok_or_else(|| {
                anyhow!(
                    "calendar date {date} does not exist in {}",
                    self.zone.name()
                )
            })
    }

    pub fn day_bounds(&self, date: NaiveDate) -> Result<(i64, i64)> {
        let next = date.succ_opt().context("date exceeds calendar range")?;
        Ok((
            self.day_start(date)?.timestamp_millis(),
            self.day_start(next)?.timestamp_millis(),
        ))
    }

    pub fn bucket_bounds(
        &self,
        bucket: &str,
        granularity: PeriodGranularity,
    ) -> Result<(i64, i64)> {
        let start = match granularity {
            PeriodGranularity::Daily => NaiveDate::parse_from_str(bucket, "%Y-%m-%d")?,
            PeriodGranularity::Weekly => {
                let (year, week) = bucket
                    .split_once("-W")
                    .ok_or_else(|| anyhow!("invalid week: {bucket}"))?;
                NaiveDate::from_isoywd_opt(year.parse()?, week.parse()?, Weekday::Mon)
                    .ok_or_else(|| anyhow!("invalid week: {bucket}"))?
            }
            PeriodGranularity::Monthly => {
                NaiveDate::parse_from_str(&format!("{bucket}-01"), "%Y-%m-%d")?
            }
            PeriodGranularity::Yearly => NaiveDate::from_ymd_opt(bucket.parse()?, 1, 1)
                .ok_or_else(|| anyhow!("invalid year: {bucket}"))?,
        };
        let end = match granularity {
            PeriodGranularity::Daily => start.checked_add_signed(Duration::days(1)),
            PeriodGranularity::Weekly => start.checked_add_signed(Duration::days(7)),
            PeriodGranularity::Monthly => start.checked_add_months(Months::new(1)),
            PeriodGranularity::Yearly => start.checked_add_months(Months::new(12)),
        }
        .ok_or_else(|| anyhow!("bucket exceeds calendar range: {bucket}"))?;
        Ok((
            self.day_start(start)?.timestamp_millis(),
            self.day_start(end)?.timestamp_millis(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dst_day_bounds_follow_zone_rules() {
        let calendar = Calendar::named("Europe/Stockholm").unwrap();
        for (date, hours) in [("2026-03-29", 23), ("2026-10-25", 25)] {
            let (start, end) = calendar
                .day_bounds(NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap())
                .unwrap();
            assert_eq!(end - start, hours * 3_600_000);
        }
    }

    #[test]
    fn skipped_midnight_starts_at_first_valid_instant() {
        let calendar = Calendar::named("America/Santiago").unwrap();
        let day = NaiveDate::from_ymd_opt(2026, 9, 6).unwrap();
        let (start, end) = calendar.day_bounds(day).unwrap();
        assert_eq!(end - start, 23 * 3_600_000);
        let (_, previous_end) = calendar.day_bounds(day.pred_opt().unwrap()).unwrap();
        assert_eq!(previous_end, start);
        assert_eq!(
            calendar
                .day_start(day)
                .unwrap()
                .with_timezone(&calendar.zone)
                .time(),
            chrono::NaiveTime::from_hms_opt(1, 0, 0).unwrap()
        );
    }

    #[test]
    fn skipped_whole_date_is_explicit() {
        let error = Calendar::named("Pacific/Apia")
            .unwrap()
            .day_start(NaiveDate::from_ymd_opt(2011, 12, 30).unwrap())
            .unwrap_err();
        assert!(error.to_string().contains("date 2011-12-30 does not exist"));
    }

    #[test]
    fn zone_identity_changes_projection_at_midnight() {
        let instant = DateTime::parse_from_rfc3339("2026-04-24T23:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            Calendar::named("UTC").unwrap().local_day_month(instant).0,
            "2026-04-24"
        );
        assert_eq!(
            Calendar::named("Europe/Stockholm")
                .unwrap()
                .local_day_month(instant)
                .0,
            "2026-04-25"
        );
        assert_ne!(
            Calendar::named("UTC").unwrap().identity(),
            Calendar::named("Europe/Stockholm").unwrap().identity()
        );
    }

    #[test]
    fn iso_week_bounds_cross_calendar_year() {
        let calendar = Calendar::named("UTC").unwrap();
        let (start, end) = calendar
            .bucket_bounds("2026-W01", PeriodGranularity::Weekly)
            .unwrap();
        assert_eq!(
            DateTime::from_timestamp_millis(start)
                .unwrap()
                .format("%Y-%m-%d")
                .to_string(),
            "2025-12-29"
        );
        assert_eq!(end - start, 7 * 86_400_000);
    }
}
