//! Time helpers — convert UNIX timestamps to ISO-8601 UTC strings
//! without depending on the `chrono` or `time` crate (keeps `max_deps`
//! low). Accuracy is calendar-correct for 1970..2099 which covers every
//! realistic timestamp this tool will ever see.

const SECS_PER_MIN: i64 = 60;
const SECS_PER_HOUR: i64 = 3600;
const SECS_PER_DAY: i64 = 86_400;

const MONTH_DAYS: [u32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

/// `true` if `year` is a Gregorian leap year.
#[must_use]
const fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// Number of days in `month` (1-indexed) of `year`.
#[must_use]
fn days_in_month(year: i64, month: u32) -> u32 {
    let idx = (month - 1) as usize;
    let base = *MONTH_DAYS.get(idx).unwrap_or(&31);
    if month == 2 && is_leap(year) { 29 } else { base }
}

/// Format a UNIX timestamp (seconds, UTC) as ISO-8601
/// `YYYY-MM-DDTHH:MM:SSZ`.
#[must_use]
pub fn iso8601_utc(ts_unix: i64) -> String {
    if ts_unix < 0 {
        return "1970-01-01T00:00:00Z".to_string();
    }
    let days_total = ts_unix / SECS_PER_DAY;
    let mut remain = ts_unix % SECS_PER_DAY;
    let hour = remain / SECS_PER_HOUR;
    remain %= SECS_PER_HOUR;
    let minute = remain / SECS_PER_MIN;
    let second = remain % SECS_PER_MIN;

    let mut year: i64 = 1970;
    let mut day = days_total;
    loop {
        let ylen = if is_leap(year) { 366 } else { 365 };
        if day < ylen {
            break;
        }
        day -= ylen;
        year += 1;
        if year > 9999 {
            break;
        }
    }
    let mut month: u32 = 1;
    while month <= 12 {
        let mlen = i64::from(days_in_month(year, month));
        if day < mlen {
            break;
        }
        day -= mlen;
        month += 1;
    }
    let mday = day + 1;
    format!("{year:04}-{month:02}-{mday:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Parse a `--since` duration string ("24h", "30m", "7d", "3600s", "3600")
/// into a count of seconds.
///
/// Returns `None` on parse failure or on overflow.
#[must_use]
pub fn parse_since(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let last = s.chars().last()?;
    let (num_part, unit): (&str, &str) = if last.is_ascii_digit() {
        (s, "s")
    } else {
        let split = s.len().saturating_sub(1);
        let (a, b) = s.split_at(split);
        (a, b)
    };
    let n: i64 = num_part.parse().ok()?;
    let mult: i64 = match unit {
        "s" => 1,
        "m" => SECS_PER_MIN,
        "h" => SECS_PER_HOUR,
        "d" => SECS_PER_DAY,
        _ => return None,
    };
    n.checked_mul(mult)
}

#[cfg(test)]
mod tests {
    use super::iso8601_utc;
    use super::parse_since;

    #[test]
    fn epoch_renders() {
        assert_eq!(iso8601_utc(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn known_date_renders() {
        // 2026-05-22 23:48:04 UTC.
        let ts = 1_779_493_684;
        assert_eq!(iso8601_utc(ts), "2026-05-22T23:48:04Z");
    }

    #[test]
    fn leap_day_renders() {
        // 2024-02-29 12:00:00 UTC.
        let ts = 1_709_208_000;
        assert_eq!(iso8601_utc(ts), "2024-02-29T12:00:00Z");
    }

    #[test]
    fn since_parses_units() {
        assert_eq!(parse_since("24h"), Some(86_400));
        assert_eq!(parse_since("30m"), Some(1_800));
        assert_eq!(parse_since("7d"), Some(604_800));
        assert_eq!(parse_since("3600s"), Some(3_600));
        assert_eq!(parse_since("3600"), Some(3_600));
        assert_eq!(parse_since("nope"), None);
    }
}
