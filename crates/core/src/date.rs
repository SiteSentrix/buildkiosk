//! Civil dates and UTC timestamps.
//!
//! [`Date`] is a calendar day with no time or zone (due dates, daily log days, schedule dates);
//! [`Timestamp`] is a moment in UTC with millisecond precision (when something happened). The
//! conversions use Howard Hinnant's public-domain civil-calendar algorithms.

use std::fmt;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A day of the week.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Weekday {
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
    Sunday,
}

impl Weekday {
    pub const ALL: [Weekday; 7] =
        [Weekday::Monday, Weekday::Tuesday, Weekday::Wednesday, Weekday::Thursday, Weekday::Friday, Weekday::Saturday, Weekday::Sunday];

    /// 0 for Monday to 6 for Sunday.
    pub fn index(self) -> usize {
        match self {
            Weekday::Monday => 0,
            Weekday::Tuesday => 1,
            Weekday::Wednesday => 2,
            Weekday::Thursday => 3,
            Weekday::Friday => 4,
            Weekday::Saturday => 5,
            Weekday::Sunday => 6,
        }
    }

    pub fn is_weekend(self) -> bool {
        matches!(self, Weekday::Saturday | Weekday::Sunday)
    }

    pub fn short(self) -> &'static str {
        match self {
            Weekday::Monday => "Mon",
            Weekday::Tuesday => "Tue",
            Weekday::Wednesday => "Wed",
            Weekday::Thursday => "Thu",
            Weekday::Friday => "Fri",
            Weekday::Saturday => "Sat",
            Weekday::Sunday => "Sun",
        }
    }
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// A calendar date (proleptic Gregorian), stored as days since 1970-01-01.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    days: i32,
}

impl Date {
    /// Dates outside these years are rejected as input mistakes.
    pub const MIN_YEAR: i32 = 1900;
    pub const MAX_YEAR: i32 = 2399;

    pub fn from_ymd(year: i32, month: u32, day: u32) -> Option<Date> {
        if !(Date::MIN_YEAR..=Date::MAX_YEAR).contains(&year) || !(1..=12).contains(&month) || day == 0 || day > days_in_month(year, month) {
            return None;
        }
        let days = days_from_civil(i64::from(year), month, day);
        i32::try_from(days).ok().map(|days| Date { days })
    }

    pub fn from_days(days: i32) -> Option<Date> {
        let d = Date { days };
        let (y, _, _) = d.ymd();
        (Date::MIN_YEAR..=Date::MAX_YEAR).contains(&y).then_some(d)
    }

    pub fn days_since_epoch(self) -> i32 {
        self.days
    }

    /// (year, month 1–12, day 1–31).
    pub fn ymd(self) -> (i32, u32, u32) {
        let (y, m, d) = civil_from_days(i64::from(self.days));
        (i32::try_from(y).unwrap_or(i32::MAX), m, d)
    }

    /// Parses exactly `YYYY-MM-DD`.
    pub fn parse(s: &str) -> Option<Date> {
        let s = s.trim();
        let b = s.as_bytes();
        if b.len() != 10 || b.get(4) != Some(&b'-') || b.get(7) != Some(&b'-') {
            return None;
        }
        let y = s.get(0..4)?;
        let m = s.get(5..7)?;
        let d = s.get(8..10)?;
        if ![y, m, d].iter().all(|p| p.bytes().all(|c| c.is_ascii_digit())) {
            return None;
        }
        Date::from_ymd(y.parse().ok()?, m.parse().ok()?, d.parse().ok()?)
    }

    pub fn weekday(self) -> Weekday {
        // 1970-01-01 was a Thursday.
        let idx = (i64::from(self.days) + 3).rem_euclid(7);
        Weekday::ALL.get(usize::try_from(idx).unwrap_or(0)).copied().unwrap_or(Weekday::Monday)
    }

    pub fn add_days(self, n: i64) -> Option<Date> {
        let days = i64::from(self.days).checked_add(n)?;
        Date::from_days(i32::try_from(days).ok()?)
    }

    /// The date `n` working days (Monday to Friday) after this one. Starting on a weekend counts
    /// from the next Monday. `n` is capped at 50,000.
    pub fn add_business_days(self, n: u32) -> Option<Date> {
        if n > 50_000 {
            return None;
        }
        // Whole weeks first, then the remainder day by day.
        let mut date = self;
        while date.weekday().is_weekend() {
            date = date.add_days(1)?;
        }
        date = date.add_days(i64::from(n / 5) * 7)?;
        let mut left = n % 5;
        while left > 0 {
            date = date.add_days(1)?;
            if !date.weekday().is_weekend() {
                left -= 1;
            }
        }
        Some(date)
    }

    /// Number of days from `self` to `other` (negative if `other` is earlier).
    pub fn days_until(self, other: Date) -> i64 {
        i64::from(other.days) - i64::from(self.days)
    }

    /// "Oct 12, 2026".
    pub fn pretty(self) -> String {
        let (y, m, d) = self.ymd();
        let month = MONTHS.get(usize::try_from(m).unwrap_or(1).saturating_sub(1)).copied().unwrap_or("?");
        format!("{month} {d}, {y}")
    }

    /// The first day of this date's month.
    pub fn month_start(self) -> Date {
        let (y, m, _) = self.ymd();
        Date::from_ymd(y, m, 1).unwrap_or(self)
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (y, m, d) = self.ymd();
        write!(f, "{y:04}-{m:02}-{d:02}")
    }
}

impl fmt::Debug for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self}")
    }
}

impl Serialize for Date {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Date {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl Visitor<'_> for V {
            type Value = Date;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a date as YYYY-MM-DD")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Date, E> {
                Date::parse(v).ok_or_else(|| E::custom(format!("`{v}` is not a date (YYYY-MM-DD, years 1900–2399)")))
            }
        }
        d.deserialize_str(V)
    }
}

/// A moment in time: milliseconds since 1970-01-01T00:00:00Z.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(i64);

const MS_PER_DAY: i64 = 86_400_000;

impl Timestamp {
    pub const fn from_millis(ms: i64) -> Self {
        Timestamp(ms)
    }

    pub const fn millis(self) -> i64 {
        self.0
    }

    /// The calendar day at a fixed UTC offset (minutes east of UTC).
    pub fn date_at(self, offset_minutes: i32) -> Date {
        let local = self.0.saturating_add(i64::from(offset_minutes) * 60_000);
        let days = local.div_euclid(MS_PER_DAY);
        i32::try_from(days).ok().and_then(Date::from_days).unwrap_or(Date { days: 0 })
    }

    /// Midnight UTC at the start of `date`.
    pub fn at_midnight(date: Date) -> Timestamp {
        Timestamp(i64::from(date.days) * MS_PER_DAY)
    }

    /// RFC 3339 in UTC: `2026-10-12T09:04:05Z`, with `.mmm` when there are milliseconds.
    pub fn rfc3339(self) -> String {
        let days = self.0.div_euclid(MS_PER_DAY);
        let ms_of_day = self.0.rem_euclid(MS_PER_DAY);
        let (y, m, d) = civil_from_days(days);
        let (h, min, s, ms) = (ms_of_day / 3_600_000, ms_of_day / 60_000 % 60, ms_of_day / 1000 % 60, ms_of_day % 1000);
        if ms == 0 {
            format!("{y:04}-{m:02}-{d:02}T{h:02}:{min:02}:{s:02}Z")
        } else {
            format!("{y:04}-{m:02}-{d:02}T{h:02}:{min:02}:{s:02}.{ms:03}Z")
        }
    }

    /// Parses RFC 3339 (`2026-10-12T09:04:05Z`, `…05.123Z`, `…05+02:00`); a lowercase `t`/`z` or a
    /// space between date and time is accepted too.
    pub fn parse_rfc3339(s: &str) -> Option<Timestamp> {
        let s = s.trim();
        let date = Date::parse(s.get(0..10)?)?;
        let sep = s.as_bytes().get(10)?;
        if !matches!(sep, b'T' | b't' | b' ') {
            return None;
        }
        let rest = s.get(11..)?;
        let two = |t: &str, at: usize| -> Option<i64> {
            let p = t.get(at..at + 2)?;
            if p.bytes().all(|c| c.is_ascii_digit()) { p.parse().ok() } else { None }
        };
        let (h, mi, sec) = (two(rest, 0)?, two(rest, 3)?, two(rest, 6)?);
        if rest.as_bytes().get(2) != Some(&b':') || rest.as_bytes().get(5) != Some(&b':') || h > 23 || mi > 59 || sec > 60 {
            return None;
        }
        let mut tail = rest.get(8..)?;
        let mut ms = 0i64;
        if let Some(frac) = tail.strip_prefix('.') {
            let digits: String = frac.chars().take_while(char::is_ascii_digit).collect();
            if digits.is_empty() || digits.len() > 9 {
                return None;
            }
            let mut padded = digits.clone();
            padded.push_str("000");
            ms = padded.get(0..3)?.parse().ok()?;
            tail = frac.get(digits.len()..)?;
        }
        let offset_min = match tail {
            "Z" | "z" => 0,
            _ => {
                let sign = match tail.as_bytes().first()? {
                    b'+' => 1,
                    b'-' => -1,
                    _ => return None,
                };
                if tail.len() != 6 || tail.as_bytes().get(3) != Some(&b':') {
                    return None;
                }
                let (oh, om) = (two(tail, 1)?, two(tail, 4)?);
                if oh > 23 || om > 59 {
                    return None;
                }
                sign * (oh * 60 + om)
            }
        };
        let day_ms = i64::from(date.days).checked_mul(MS_PER_DAY)?;
        let local = day_ms.checked_add(h * 3_600_000 + mi * 60_000 + sec.min(59) * 1000 + ms)?;
        local.checked_sub(offset_min * 60_000).map(Timestamp)
    }

    /// "Oct 12, 2026 9:04 AM" at a fixed UTC offset.
    pub fn pretty_at(self, offset_minutes: i32) -> String {
        let local = self.0.saturating_add(i64::from(offset_minutes) * 60_000);
        let date = self.date_at(offset_minutes);
        let ms_of_day = local.rem_euclid(MS_PER_DAY);
        let (h24, min) = (ms_of_day / 3_600_000, ms_of_day / 60_000 % 60);
        let (h12, ampm) = match h24 {
            0 => (12, "AM"),
            1..=11 => (h24, "AM"),
            12 => (12, "PM"),
            _ => (h24 - 12, "PM"),
        };
        format!("{} {h12}:{min:02} {ampm}", date.pretty())
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.rfc3339())
    }
}

impl Serialize for Timestamp {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.rfc3339())
    }
}

impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl Visitor<'_> for V {
            type Value = Timestamp;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an RFC 3339 timestamp such as 2026-10-12T09:04:05Z")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Timestamp, E> {
                Timestamp::parse_rfc3339(v).ok_or_else(|| E::custom(format!("`{v}` is not an RFC 3339 timestamp")))
            }
        }
        d.deserialize_str(V)
    }
}

fn is_leap(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(y) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Days since 1970-01-01 of a proleptic Gregorian date.
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = i64::from((m + 9) % 12);
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The proleptic Gregorian date of a day count since 1970-01-01.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z.saturating_add(719_468);
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = u32::try_from(doy - (153 * mp + 2) / 5 + 1).unwrap_or(1);
    let m = u32::try_from(if mp < 10 { mp + 3 } else { mp - 9 }).unwrap_or(1);
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn d(s: &str) -> Date {
        Date::parse(s).unwrap()
    }

    #[test]
    fn parse_and_display() {
        assert_eq!(d("2026-10-12").to_string(), "2026-10-12");
        assert_eq!(d("1970-01-01").days_since_epoch(), 0);
        assert_eq!(d("2024-02-29").ymd(), (2024, 2, 29));
        for bad in ["2023-02-29", "2026-13-01", "2026-00-10", "2026-1-01", "26-10-12", "1899-12-31", "2400-01-01", "2026-10-12x", ""] {
            assert_eq!(Date::parse(bad), None, "{bad:?}");
        }
        assert_eq!(d("2026-10-12").pretty(), "Oct 12, 2026");
    }

    #[test]
    fn weekdays_and_business_days() {
        assert_eq!(d("1970-01-01").weekday(), Weekday::Thursday);
        assert_eq!(d("2026-10-12").weekday(), Weekday::Monday);
        // Monday + 5 working days = next Monday; Friday + 1 = Monday; Saturday + 1 = Tuesday.
        assert_eq!(d("2026-10-12").add_business_days(5), Some(d("2026-10-19")));
        assert_eq!(d("2026-10-16").add_business_days(1), Some(d("2026-10-19")));
        assert_eq!(d("2026-10-17").add_business_days(1), Some(d("2026-10-20")));
        assert_eq!(d("2026-10-17").add_business_days(0), Some(d("2026-10-19")));
        assert_eq!(d("2026-10-12").add_business_days(7), Some(d("2026-10-21")));
        assert_eq!(d("2026-10-12").days_until(d("2026-10-02")), -10);
        assert_eq!(d("2399-12-31").add_days(1), None);
    }

    #[test]
    fn timestamps() {
        let t = Timestamp::parse_rfc3339("2026-10-12T09:04:05Z").unwrap();
        assert_eq!(t.rfc3339(), "2026-10-12T09:04:05Z");
        assert_eq!(Timestamp::parse_rfc3339("2026-10-12T11:04:05.5+02:00"), Some(Timestamp::from_millis(t.millis() + 500)));
        assert_eq!(Timestamp::from_millis(t.millis() + 7).rfc3339(), "2026-10-12T09:04:05.007Z");
        assert_eq!(t.date_at(-600), d("2026-10-11"));
        assert_eq!(t.date_at(0), d("2026-10-12"));
        assert_eq!(t.pretty_at(-240), "Oct 12, 2026 5:04 AM");
        assert_eq!(Timestamp::from_millis(t.millis() + 6 * 3_600_000).pretty_at(0), "Oct 12, 2026 3:04 PM");
        for bad in ["2026-10-12", "2026-10-12T25:00:00Z", "2026-10-12T09:04:05", "2026-10-12T09:04:05+2:00", "2026-10-12T09:04:05.Z"] {
            assert_eq!(Timestamp::parse_rfc3339(bad), None, "{bad:?}");
        }
        assert_eq!(serde_json::to_string(&t).unwrap(), "\"2026-10-12T09:04:05Z\"");
        assert_eq!(serde_json::from_str::<Timestamp>("\"2026-10-12T09:04:05Z\"").unwrap(), t);
        assert_eq!(serde_json::from_str::<Date>("\"2026-10-12\"").unwrap(), d("2026-10-12"));
    }

    proptest! {
        #[test]
        fn civil_round_trip(days in -25_000i32..156_000) {
            if let Some(date) = Date::from_days(days) {
                let (y, m, dd) = date.ymd();
                prop_assert_eq!(Date::from_ymd(y, m, dd), Some(date));
                prop_assert_eq!(Date::parse(&date.to_string()), Some(date));
            }
        }

        #[test]
        fn timestamp_round_trip(ms in -2_000_000_000_000i64..10_000_000_000_000) {
            let t = Timestamp::from_millis(ms);
            prop_assert_eq!(Timestamp::parse_rfc3339(&t.rfc3339()), Some(t));
        }

        #[test]
        fn parsers_never_panic(s in "\\PC{0,40}") {
            let _ = Date::parse(&s);
            let _ = Timestamp::parse_rfc3339(&s);
        }

        #[test]
        fn business_days_land_on_weekdays(days in 0i32..100_000, n in 0u32..400) {
            if let Some(date) = Date::from_days(days).and_then(|x| x.add_business_days(n)) {
                prop_assert!(!date.weekday().is_weekend());
            }
        }
    }
}
