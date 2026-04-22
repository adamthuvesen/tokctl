use anyhow::{anyhow, Result};
use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};

/// Parse `--since` value as `YYYY-MM-DD` at local-midnight.
pub fn parse_since(value: Option<&str>) -> Result<Option<DateTime<Utc>>> {
    let Some(v) = value else { return Ok(None) };
    let date = NaiveDate::parse_from_str(v, "%Y-%m-%d")
        .map_err(|_| anyhow!("--since must be YYYY-MM-DD, got \"{}\"", v))?;
    let ndt = NaiveDateTime::new(date, NaiveTime::from_hms_opt(0, 0, 0).unwrap());
    let local = Local
        .from_local_datetime(&ndt)
        .single()
        .ok_or_else(|| anyhow!("--since not a valid local datetime: \"{}\"", v))?;
    Ok(Some(local.with_timezone(&Utc)))
}

/// Parse `--until` value as `YYYY-MM-DD` at local end-of-day (23:59:59.999).
pub fn parse_until(value: Option<&str>) -> Result<Option<DateTime<Utc>>> {
    let Some(v) = value else { return Ok(None) };
    let date = NaiveDate::parse_from_str(v, "%Y-%m-%d")
        .map_err(|_| anyhow!("--until must be YYYY-MM-DD, got \"{}\"", v))?;
    let ndt = NaiveDateTime::new(
        date,
        NaiveTime::from_hms_milli_opt(23, 59, 59, 999).unwrap(),
    );
    let local = Local
        .from_local_datetime(&ndt)
        .single()
        .ok_or_else(|| anyhow!("--until not a valid local datetime: \"{}\"", v))?;
    Ok(Some(local.with_timezone(&Utc)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_since_valid() {
        let d = parse_since(Some("2026-04-18")).unwrap();
        assert!(d.is_some());
    }

    #[test]
    fn parse_since_invalid_errors() {
        assert!(parse_since(Some("not-a-date")).is_err());
    }

    #[test]
    fn parse_since_none_is_none() {
        assert!(parse_since(None).unwrap().is_none());
    }
}
