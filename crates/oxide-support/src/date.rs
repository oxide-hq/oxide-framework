//! Date helpers on `chrono`, with Carbon-style relative times.
//!
//! `chrono` is the date library because sqlx and the ORM already use it, it is stable on the
//! minimum supported Rust version, and it serializes with serde. The functions take explicit
//! instants where it matters, so the test clock (a later epic) can drive them.

use chrono::{DateTime, NaiveDate, Utc};

/// The current instant (UTC).
pub fn now() -> DateTime<Utc> {
    Utc::now()
}

/// Today's date (UTC).
pub fn today() -> NaiveDate {
    Utc::now().date_naive()
}

/// How long ago (or from now) `when` is, like Carbon's `diffForHumans`: `5 minutes ago`,
/// `in 2 days`.
pub fn diff_for_humans(when: DateTime<Utc>) -> String {
    diff_for_humans_from(when, now())
}

/// Like [`diff_for_humans`] but relative to an explicit instant.
pub fn diff_for_humans_from(when: DateTime<Utc>, relative_to: DateTime<Utc>) -> String {
    let seconds = relative_to.signed_duration_since(when).num_seconds();
    if seconds == 0 {
        return "just now".to_owned();
    }
    let magnitude = seconds.unsigned_abs();
    let (count, unit) = match magnitude {
        0..=59 => (magnitude, "second"),
        60..=3_599 => (magnitude / 60, "minute"),
        3_600..=86_399 => (magnitude / 3_600, "hour"),
        86_400..=604_799 => (magnitude / 86_400, "day"),
        604_800..=2_591_999 => (magnitude / 604_800, "week"),
        2_592_000..=31_535_999 => ((magnitude / 2_592_000).min(11), "month"),
        _ => (magnitude / 31_536_000, "year"),
    };
    let plural = if count == 1 { "" } else { "s" };
    if seconds > 0 {
        format!("{count} {unit}{plural} ago")
    } else {
        format!("in {count} {unit}{plural}")
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone};

    use super::*;

    fn base() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 15, 12, 0, 0).unwrap()
    }

    #[test]
    fn past_and_future_phrases() {
        let t = base();
        let cases = [
            (Duration::seconds(0), "just now"),
            (Duration::seconds(-1), "1 second ago"),
            (Duration::seconds(-59), "59 seconds ago"),
            (Duration::minutes(-5), "5 minutes ago"),
            (Duration::hours(-1), "1 hour ago"),
            (Duration::days(-3), "3 days ago"),
            (Duration::weeks(-2), "2 weeks ago"),
            (Duration::days(-65), "2 months ago"),
            (Duration::days(-364), "11 months ago"),
            (Duration::days(-800), "2 years ago"),
            (Duration::minutes(90), "in 1 hour"),
            (Duration::days(1), "in 1 day"),
        ];
        for (offset, expected) in cases {
            assert_eq!(diff_for_humans_from(t + offset, t), expected, "{offset:?}");
        }
    }

    #[test]
    fn now_and_today_agree() {
        assert_eq!(today(), now().date_naive());
        assert_eq!(diff_for_humans(now()), "just now");
    }
}
