//! Deadlines on items (`@`): a day, and optionally a time on it. An open
//! item with a deadline moves on to today like a pinned one, up to its
//! deadline day, and stays there once that's passed.

use chrono::{Datelike, NaiveDate, NaiveTime, Timelike};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Deadline {
    pub date: NaiveDate,
    pub time: Option<NaiveTime>,
}

impl Deadline {
    /// How it's written to disk: `2026-10-12`, or `2026-10-12 13:00`.
    pub fn to_stored(self) -> String {
        match self.time {
            Some(time) => format!("{} {}", self.date.format("%Y-%m-%d"), time.format("%H:%M")),
            None => self.date.format("%Y-%m-%d").to_string(),
        }
    }

    /// Reads what `to_stored` wrote, or `None` if it isn't a deadline.
    pub fn from_stored(text: &str) -> Option<Self> {
        let (date, time) = match text.trim().split_once(' ') {
            Some((date, time)) => (date, Some(NaiveTime::parse_from_str(time.trim(), "%H:%M").ok()?)),
            None => (text.trim(), None),
        };
        Some(Self { date: NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()?, time })
    }

    /// Whether it has passed, on `today` at `now`. A deadline with no time
    /// lasts all day.
    pub fn is_overdue(self, today: NaiveDate, now: NaiveTime) -> bool {
        self.date < today || (self.date == today && self.time.is_some_and(|time| time < now))
    }

    /// The day an open item stored on an earlier day moves on to, by `today`:
    /// today, or its deadline day once that's passed.
    pub fn roll_to(self, today: NaiveDate) -> NaiveDate {
        self.date.min(today)
    }

    /// How it's shown beside an item, counting days from `today`: just the
    /// time today (`19:25`), or the day too (`tomorrow 19:25`, `Fri`,
    /// `12 Oct 11PM`, `3 Jan 2027`).
    pub fn describe(self, today: NaiveDate, twelve_hour: bool) -> String {
        let days = (self.date - today).num_days();
        let day = match days {
            0 if self.time.is_some() => String::new(),
            0 => "today".into(),
            1 => "tomorrow".into(),
            -1 => "yesterday".into(),
            2..=6 => self.date.format("%a").to_string(),
            _ if self.date.year() == today.year() => self.date.format("%-d %b").to_string(),
            _ => self.date.format("%-d %b %Y").to_string(),
        };
        match self.time {
            Some(time) if day.is_empty() => format_time(time, twelve_hour),
            Some(time) => format!("{day} {}", format_time(time, twelve_hour)),
            None => day,
        }
    }
}

/// A time as shown: `19:25` on the 24-hour clock, or `7:25PM` (and `11PM`
/// on the hour) on the 12-hour one.
pub fn format_time(time: NaiveTime, twelve_hour: bool) -> String {
    if !twelve_hour {
        return time.format("%H:%M").to_string();
    }
    let (pm, hour) = time.hour12();
    let suffix = if pm { "PM" } else { "AM" };
    match time.minute() {
        0 => format!("{hour}{suffix}"),
        minute => format!("{hour}:{minute:02}{suffix}"),
    }
}

/// A typed time, on either clock: `13:00`, `1300`, `9`, `930`, `9.30`,
/// `12pm`, `11AM`, `11:30 pm`, `7p`. `None` if it isn't one.
pub fn parse_time(text: &str) -> Option<NaiveTime> {
    let text = text.trim().to_lowercase().replace(' ', "");
    let (body, pm) = if let Some(body) = text.strip_suffix("am").or_else(|| text.strip_suffix('a')) {
        (body, Some(false))
    } else if let Some(body) = text.strip_suffix("pm").or_else(|| text.strip_suffix('p')) {
        (body, Some(true))
    } else {
        (text.as_str(), None)
    };
    let (hour, minute) = match body.split_once([':', '.']) {
        Some((hour, minute)) if minute.len() == 2 => (hour, minute),
        Some(_) => return None,
        // Without a separator, the last two digits are the minutes: 930, 1300.
        None if body.len() > 2 => body.split_at(body.len() - 2),
        None => (body, "00"),
    };
    if hour.is_empty() || !(hour.len() <= 2 && hour.chars().chain(minute.chars()).all(|c| c.is_ascii_digit())) {
        return None;
    }
    let (hour, minute): (u32, u32) = (hour.parse().ok()?, minute.parse().ok()?);
    let hour = match pm {
        Some(_) if !(1..=12).contains(&hour) => return None,
        Some(pm) => hour % 12 + if pm { 12 } else { 0 },
        None => hour,
    };
    NaiveTime::from_hms_opt(hour, minute, 0).filter(|_| hour < 24 && minute < 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::today;

    fn time(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    fn on(days: i64, time: Option<NaiveTime>) -> Deadline {
        Deadline { date: today() + chrono::TimeDelta::days(days), time }
    }

    #[test]
    fn times_parse_on_either_clock() {
        for (typed, expected) in [
            ("13:00", time(13, 0)),
            ("1300", time(13, 0)),
            ("0930", time(9, 30)),
            ("930", time(9, 30)),
            ("9.30", time(9, 30)),
            ("9", time(9, 0)),
            ("00:00", time(0, 0)),
            ("23:59", time(23, 59)),
            ("12pm", time(12, 0)),
            ("12am", time(0, 0)),
            ("11AM", time(11, 0)),
            ("11 PM", time(23, 0)),
            ("11:30pm", time(23, 30)),
            ("7p", time(19, 0)),
            ("  1:05 am ", time(1, 5)),
        ] {
            assert_eq!(parse_time(typed), Some(expected), "{typed}");
        }
    }

    #[test]
    fn things_that_arent_times_dont_parse() {
        for typed in ["", "pm", "24:00", "12:60", "13pm", "0am", "9:5", "12345", "noon", "1:2:3", "-1", "9:30x", ":30"] {
            assert_eq!(parse_time(typed), None, "{typed}");
        }
    }

    #[test]
    fn times_show_on_either_clock() {
        assert_eq!(format_time(time(19, 25), false), "19:25");
        assert_eq!(format_time(time(9, 5), false), "09:05");
        assert_eq!(format_time(time(23, 0), true), "11PM");
        assert_eq!(format_time(time(19, 25), true), "7:25PM");
        assert_eq!(format_time(time(0, 0), true), "12AM");
        assert_eq!(format_time(time(12, 30), true), "12:30PM");
    }

    #[test]
    fn deadlines_are_described_from_today() {
        assert_eq!(on(0, Some(time(19, 25))).describe(today(), false), "19:25");
        assert_eq!(on(0, None).describe(today(), false), "today");
        assert_eq!(on(1, None).describe(today(), false), "tomorrow");
        assert_eq!(on(1, Some(time(23, 0))).describe(today(), true), "tomorrow 11PM");
        assert_eq!(on(-1, None).describe(today(), false), "yesterday");
        // Today is Monday 5 October 2026: the rest of the week by name.
        assert_eq!(on(4, Some(time(9, 0))).describe(today(), false), "Fri 09:00");
        assert_eq!(on(7, None).describe(today(), false), "12 Oct");
        assert_eq!(on(-5, None).describe(today(), false), "30 Sep");
        assert_eq!(on(100, None).describe(today(), false), "13 Jan 2027");
    }

    #[test]
    fn overdue_means_an_earlier_day_or_a_time_gone_today() {
        let noon = time(12, 0);
        assert!(on(-1, None).is_overdue(today(), noon));
        assert!(on(0, Some(time(9, 0))).is_overdue(today(), noon));
        assert!(!on(0, Some(time(13, 0))).is_overdue(today(), noon));
        assert!(!on(0, None).is_overdue(today(), noon), "a date alone lasts all day");
        assert!(!on(1, Some(time(0, 0))).is_overdue(today(), noon));
    }

    #[test]
    fn deadlines_are_stored_as_text_and_read_back() {
        let with_time = on(7, Some(time(13, 5)));
        assert_eq!(with_time.to_stored(), "2026-10-12 13:05");
        assert_eq!(Deadline::from_stored("2026-10-12 13:05"), Some(with_time));
        assert_eq!(on(7, None).to_stored(), "2026-10-12");
        assert_eq!(Deadline::from_stored("2026-10-12"), Some(on(7, None)));
        for broken in ["", "soon", "2026-13-01", "2026-10-12 25:00", "2026-10-12 1pm"] {
            assert_eq!(Deadline::from_stored(broken), None, "{broken}");
        }
    }

    #[test]
    fn items_roll_on_to_today_but_not_past_their_deadline() {
        assert_eq!(on(3, None).roll_to(today()), today());
        assert_eq!(on(-2, None).roll_to(today()), on(-2, None).date);
    }
}
