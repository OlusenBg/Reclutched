//! Calendar maths on plain dates, without a date library: what weekday is a day, how many days
//! a month has, the day before, and so on. Days are written as "2026-10-23", months as "2026-10".

pub const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

pub const WEEKDAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

/// A calendar day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Day {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

impl Day {
    pub fn new(year: i32, month: u32, day: u32) -> Option<Self> {
        let valid = (1..=12).contains(&month) && day >= 1 && day <= days_in_month(year, month);
        valid.then_some(Self { year, month, day })
    }

    /// Parses "2026-10-23".
    pub fn parse(key: &str) -> Option<Self> {
        let mut parts = key.split('-');
        let year = parts.next()?.parse().ok()?;
        let month = parts.next()?.parse().ok()?;
        let day = parts.next()?.parse().ok()?;
        if parts.next().is_some() || key.len() != 10 {
            return None;
        }
        Self::new(year, month, day)
    }

    /// "2026-10-23".
    pub fn key(self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }

    /// "2026-10".
    pub fn month_key(self) -> String {
        month_key(self.year, self.month)
    }

    /// Days since 1970-01-01 (negative before).
    pub fn to_days(self) -> i64 {
        // Howard Hinnant's days_from_civil.
        let y = i64::from(self.year) - i64::from(self.month <= 2);
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let m = i64::from(self.month);
        let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(self.day) - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }

    pub fn from_days(days: i64) -> Self {
        // Howard Hinnant's civil_from_days.
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
        Self {
            year: (y + i64::from(month <= 2)) as i32,
            month,
            day,
        }
    }

    pub fn plus_days(self, n: i64) -> Self {
        Self::from_days(self.to_days() + n)
    }

    /// 0 = Monday ... 6 = Sunday.
    pub fn weekday(self) -> u32 {
        // 1970-01-01 was a Thursday (3).
        (self.to_days() + 3).rem_euclid(7) as u32
    }

    /// The day of a unix time, seen with a fixed offset from UTC in seconds.
    pub fn from_unix(unix: i64, utc_offset: i64) -> Self {
        Self::from_days((unix + utc_offset).div_euclid(86_400))
    }
}

pub fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

pub fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// "2026-10".
pub fn month_key(year: i32, month: u32) -> String {
    format!("{year:04}-{month:02}")
}

/// Parses "2026-10".
pub fn parse_month(key: &str) -> Option<(i32, u32)> {
    let (year, month) = key.split_once('-')?;
    let (year, month): (i32, u32) = (year.parse().ok()?, month.parse().ok()?);
    ((1..=12).contains(&month) && key.len() == 7).then_some((year, month))
}

/// Whole hours and minutes as text: "3 h 12 min", "45 min", "2 h".
pub fn duration_text(seconds: i64) -> String {
    let minutes = (seconds.max(0) + 30) / 60;
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_round_trip_and_weekdays_are_right() {
        // 1970-01-01 was a Thursday, 2000-01-01 a Saturday, 2026-10-23 a Friday.
        assert_eq!(Day::new(1970, 1, 1).unwrap().to_days(), 0);
        assert_eq!(Day::new(1970, 1, 1).unwrap().weekday(), 3);
        assert_eq!(Day::new(2000, 1, 1).unwrap().weekday(), 5);
        assert_eq!(Day::new(2026, 10, 23).unwrap().weekday(), 4);
        for days in [-800_000, -1, 0, 1, 59, 11_016, 20_000, 400_000] {
            assert_eq!(Day::from_days(days).to_days(), days);
        }
    }

    #[test]
    fn month_lengths_and_leap_years() {
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2025, 2), 28);
        assert_eq!(days_in_month(1900, 2), 28);
        assert_eq!(days_in_month(2000, 2), 29);
        assert_eq!(days_in_month(2026, 4), 30);
        assert_eq!(days_in_month(2026, 12), 31);
        assert!(Day::new(2026, 2, 29).is_none());
        assert!(Day::new(2024, 2, 29).is_some());
    }

    #[test]
    fn parsing_and_stepping_over_month_and_year_ends() {
        let d = Day::parse("2026-03-01").unwrap();
        assert_eq!(d.plus_days(-1).key(), "2026-02-28");
        assert_eq!(
            Day::parse("2026-12-31").unwrap().plus_days(1).key(),
            "2027-01-01"
        );
        assert_eq!(
            Day::parse("2024-03-01").unwrap().plus_days(-1).key(),
            "2024-02-29"
        );
        assert_eq!(d.month_key(), "2026-03");
        for bad in [
            "",
            "2026",
            "2026-13-01",
            "2026-02-30",
            "2026-1-1",
            "x-y-z",
            "2026-10-23-1",
        ] {
            assert!(Day::parse(bad).is_none(), "{bad}");
        }
        assert_eq!(parse_month("2026-10"), Some((2026, 10)));
        assert_eq!(parse_month("2026-13"), None);
        assert_eq!(parse_month("2026"), None);
    }

    #[test]
    fn a_unix_time_lands_on_the_right_local_day() {
        // 2026-10-23 23:30 UTC is already the 24th at UTC+2, still the 23rd at UTC-5.
        let unix = Day::new(2026, 10, 23).unwrap().to_days() * 86_400 + 23 * 3_600 + 1_800;
        assert_eq!(Day::from_unix(unix, 0).key(), "2026-10-23");
        assert_eq!(Day::from_unix(unix, 2 * 3_600).key(), "2026-10-24");
        assert_eq!(Day::from_unix(unix, -5 * 3_600).key(), "2026-10-23");
    }

    #[test]
    fn durations_read_naturally() {
        assert_eq!(duration_text(0), "0 min");
        assert_eq!(duration_text(45 * 60), "45 min");
        assert_eq!(duration_text(2 * 3_600), "2 h");
        assert_eq!(duration_text(3 * 3_600 + 12 * 60), "3 h 12 min");
        assert_eq!(duration_text(-5), "0 min");
    }
}
