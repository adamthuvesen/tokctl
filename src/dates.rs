use anyhow::{anyhow, Result};
use chrono::{DateTime, NaiveDate, Utc};

/// Parse `--since` value as `YYYY-MM-DD` at local-midnight.
pub fn parse_since(value: Option<&str>) -> Result<Option<DateTime<Utc>>> {
    let Some(v) = value else { return Ok(None) };
    let date = NaiveDate::parse_from_str(v, "%Y-%m-%d")
        .map_err(|_| anyhow!("--since must be YYYY-MM-DD, got \"{}\"", v))?;
    Ok(Some(crate::calendar::Calendar::system()?.day_start(date)?))
}

/// Parse the inclusive date flag into an exclusive next-midnight bound.
pub fn parse_until(value: Option<&str>) -> Result<Option<DateTime<Utc>>> {
    let Some(v) = value else { return Ok(None) };
    let date = NaiveDate::parse_from_str(v, "%Y-%m-%d")
        .map_err(|_| anyhow!("--until must be YYYY-MM-DD, got \"{}\"", v))?;
    let next = date
        .succ_opt()
        .ok_or_else(|| anyhow!("--until date is out of range"))?;
    Ok(Some(crate::calendar::Calendar::system()?.day_start(next)?))
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
