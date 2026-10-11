//! Reminders on items (`r`): hours, days or months before the item is due,
//! or on a day chosen on the calendar. Once a reminder is due, an open item
//! from a later day also shows on today's list, until its own day comes.

use chrono::{Days, Months, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta};
use ratatui::crossterm::event::KeyCode;

use crate::deadline::Deadline;
use crate::input::LineInput;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reminder {
    /// Some hours, days or months before the item is due.
    Before { amount: u32, unit: Unit },
    /// On a day chosen on the calendar, from a time on it or its start.
    On(Deadline),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Hours,
    Days,
    Months,
}

impl Unit {
    /// How it's written to disk and shown: `h`, `d` or `mo`.
    fn suffix(self) -> &'static str {
        match self {
            Unit::Hours => "h",
            Unit::Days => "d",
            Unit::Months => "mo",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Unit::Hours => "hours",
            Unit::Days => "days",
            Unit::Months => "months",
        }
    }
}

/// The most of any unit a reminder can be before, to keep date maths sane.
pub const MAX_AMOUNT: u32 = 999;

impl Reminder {
    /// How it's written to disk: `3h`, `2d` or `1mo` before, or a day as a
    /// deadline is (`2026-10-12`, `2026-10-12 09:00`).
    pub fn to_stored(self) -> String {
        match self {
            Reminder::Before { amount, unit } => format!("{amount}{}", unit.suffix()),
            Reminder::On(on) => on.to_stored(),
        }
    }

    /// Reads what `to_stored` wrote, or `None` if it isn't a reminder.
    pub fn from_stored(text: &str) -> Option<Self> {
        let text = text.trim();
        let before = |suffix: &str, unit| {
            let amount: u32 = text.strip_suffix(suffix)?.parse().ok()?;
            (1..=MAX_AMOUNT).contains(&amount).then_some(Reminder::Before { amount, unit })
        };
        before("mo", Unit::Months)
            .or_else(|| before("h", Unit::Hours))
            .or_else(|| before("d", Unit::Days))
            .or_else(|| Deadline::from_stored(text).map(Reminder::On))
    }

    /// When it's due, for an item stored on `day` with `deadline`. Hours
    /// count back from the deadline, or the end of its day if it has no time
    /// (or no deadline, then the end of `day`), so there's the evening left.
    /// Days and months count whole days back from that day, and start at its
    /// beginning, leaving all of it.
    pub fn due_at(self, day: NaiveDate, deadline: Option<Deadline>) -> NaiveDateTime {
        let due_day = deadline.map_or(day, |deadline| deadline.date);
        let start = |date: NaiveDate| date.and_time(NaiveTime::MIN);
        match self {
            Reminder::Before { amount, unit: Unit::Hours } => {
                let end = match deadline.and_then(|deadline| deadline.time) {
                    Some(time) => due_day.and_time(time),
                    None => start(due_day) + TimeDelta::days(1),
                };
                end - TimeDelta::hours(amount.into())
            }
            Reminder::Before { amount, unit: Unit::Days } => {
                start(due_day.checked_sub_days(Days::new(amount.into())).unwrap_or(NaiveDate::MIN))
            }
            Reminder::Before { amount, unit: Unit::Months } => {
                start(due_day.checked_sub_months(Months::new(amount)).unwrap_or(NaiveDate::MIN))
            }
            Reminder::On(on) => on.date.and_time(on.time.unwrap_or(NaiveTime::MIN)),
        }
    }

    /// How it's shown beside an item: `2d`, `3h`, `1mo`, or the day it's on
    /// like a deadline (`Fri`, `12 Oct 09:00`).
    pub fn describe(self, today: NaiveDate, twelve_hour: bool) -> String {
        match self {
            Reminder::Before { amount, unit } => format!("{amount}{}", unit.suffix()),
            Reminder::On(on) => on.describe(today, twelve_hour),
        }
    }

    /// In words, for the reminder popup: `2 days before`, `on Fri 09:00`.
    pub fn explain(self, today: NaiveDate, twelve_hour: bool) -> String {
        match self {
            Reminder::Before { amount: 1, unit } => format!("1 {} before", unit.name().trim_end_matches('s')),
            Reminder::Before { amount, unit } => format!("{amount} {} before", unit.name()),
            Reminder::On(on) => format!("on {}", on.describe(today, twelve_hour)),
        }
    }
}

/// The `r` popup: choosing how long before an item to be reminded.
pub struct Picker {
    /// Where the item is stored.
    pub day: NaiveDate,
    pub index: usize,
    /// Its reminder before, which `x` removes.
    pub current: Option<Reminder>,
    /// How many of a unit, once `h`, `d` or `m` is pressed.
    pub amount: Option<(Unit, LineInput)>,
    /// Whether the amount is still the one it started with, so typing
    /// replaces it rather than adding to it.
    pub fresh: bool,
    /// Why what was typed isn't an amount.
    pub error: Option<String>,
}

/// What the app should do after the picker handles a key.
#[derive(Debug, PartialEq)]
pub enum Action {
    Stay,
    Close,
    /// Set (or with `None`, remove) the item's reminder.
    Set(Option<Reminder>),
    /// Choose a day on the calendar.
    Calendar,
}

impl Picker {
    pub fn new((day, index): (NaiveDate, usize), current: Option<Reminder>) -> Self {
        Self { day, index, current, amount: None, fresh: false, error: None }
    }

    pub fn handle_key(&mut self, code: KeyCode) -> Action {
        if let Some((unit, input)) = &mut self.amount {
            let unit = *unit;
            match code {
                // Back to choosing the unit.
                KeyCode::Esc => {
                    self.amount = None;
                    self.error = None;
                }
                KeyCode::Enter => match input.text.trim().parse::<u32>() {
                    Ok(amount @ 1..=MAX_AMOUNT) => return Action::Set(Some(Reminder::Before { amount, unit })),
                    _ => self.error = Some(format!("Type how many {}, from 1 to {MAX_AMOUNT}", unit.name())),
                },
                code => {
                    if self.fresh && matches!(code, KeyCode::Char(_)) {
                        *input = LineInput::default();
                    }
                    input.handle_key(code);
                    self.error = None;
                }
            }
            self.fresh = false;
            return Action::Stay;
        }
        let unit = match code {
            KeyCode::Char('h') => Unit::Hours,
            KeyCode::Char('d') => Unit::Days,
            KeyCode::Char('m') => Unit::Months,
            KeyCode::Char('c') => return Action::Calendar,
            KeyCode::Char('x') if self.current.is_some() => return Action::Set(None),
            KeyCode::Esc | KeyCode::Char('q' | 'r') => return Action::Close,
            _ => return Action::Stay,
        };
        // Start from the amount it had, if it was the same unit.
        let start = match self.current {
            Some(Reminder::Before { amount, unit: had }) if had == unit => amount.to_string(),
            _ => String::new(),
        };
        self.fresh = !start.is_empty();
        self.amount = Some((unit, LineInput::new(&start)));
        Action::Stay
    }
}

/// What the reminder counts back from, in words, for the popup: the
/// deadline, or the end of the item's day.
pub fn counted_from(day: NaiveDate, deadline: Option<Deadline>, today: NaiveDate, twelve_hour: bool) -> String {
    match deadline {
        Some(deadline) => format!("its deadline, {}", deadline.describe(today, twelve_hour)),
        None => format!("its day, {}", Deadline { date: day, time: None }.describe(today, twelve_hour)),
    }
}

#[cfg(test)]
mod tests {
    use ratatui::crossterm::event::KeyCode;

    use super::*;
    use crate::test_util::today;

    fn at(days: i64, h: u32, m: u32) -> NaiveDateTime {
        (today() + TimeDelta::days(days)).and_hms_opt(h, m, 0).unwrap()
    }

    fn day(days: i64) -> NaiveDate {
        today() + TimeDelta::days(days)
    }

    fn before(amount: u32, unit: Unit) -> Reminder {
        Reminder::Before { amount, unit }
    }

    fn send(picker: &mut Picker, keys: &str) -> Action {
        let mut action = Action::Stay;
        let mut rest = keys;
        while let Some(c) = rest.chars().next() {
            let (code, len) = if rest.starts_with("<esc>") {
                (KeyCode::Esc, 5)
            } else if rest.starts_with("<cr>") {
                (KeyCode::Enter, 4)
            } else {
                (KeyCode::Char(c), c.len_utf8())
            };
            action = picker.handle_key(code);
            rest = &rest[len..];
        }
        action
    }

    #[test]
    fn reminders_are_stored_as_text_and_read_back() {
        for (reminder, stored) in [
            (before(3, Unit::Hours), "3h"),
            (before(2, Unit::Days), "2d"),
            (before(1, Unit::Months), "1mo"),
            (Reminder::On(Deadline { date: day(7), time: None }), "2026-10-12"),
            (Reminder::On(Deadline { date: day(7), time: NaiveTime::from_hms_opt(9, 0, 0) }), "2026-10-12 09:00"),
        ] {
            assert_eq!(reminder.to_stored(), stored);
            assert_eq!(Reminder::from_stored(stored), Some(reminder), "{stored}");
        }
        for broken in ["", "soon", "0d", "-1h", "2w", "1000d", "d", "2026-13-01"] {
            assert_eq!(Reminder::from_stored(broken), None, "{broken}");
        }
    }

    #[test]
    fn hours_count_back_from_the_end_of_the_day_without_a_deadline() {
        assert_eq!(before(2, Unit::Hours).due_at(day(3), None), at(3, 22, 0));
        assert_eq!(before(30, Unit::Hours).due_at(day(3), None), at(2, 18, 0));
    }

    #[test]
    fn hours_count_back_from_a_deadline_and_its_time() {
        let timed = Deadline { date: day(5), time: NaiveTime::from_hms_opt(15, 0, 0) };
        assert_eq!(before(2, Unit::Hours).due_at(day(1), Some(timed)), at(5, 13, 0));
        let untimed = Deadline { date: day(5), time: None };
        assert_eq!(before(2, Unit::Hours).due_at(day(1), Some(untimed)), at(5, 22, 0));
    }

    #[test]
    fn days_and_months_start_at_the_beginning_of_their_day() {
        assert_eq!(before(2, Unit::Days).due_at(day(10), None), at(8, 0, 0));
        let timed = Deadline { date: day(10), time: NaiveTime::from_hms_opt(15, 0, 0) };
        assert_eq!(before(2, Unit::Days).due_at(day(1), Some(timed)), at(8, 0, 0), "the whole day, not 15:00");
        // 5 November less a month.
        assert_eq!(before(1, Unit::Months).due_at(day(31), None), at(0, 0, 0));
    }

    #[test]
    fn a_day_from_the_calendar_starts_at_its_time_or_its_beginning() {
        assert_eq!(Reminder::On(Deadline { date: day(2), time: None }).due_at(day(9), None), at(2, 0, 0));
        let on = Deadline { date: day(2), time: NaiveTime::from_hms_opt(9, 30, 0) };
        assert_eq!(Reminder::On(on).due_at(day(9), None), at(2, 9, 30));
    }

    #[test]
    fn reminders_are_described_briefly_and_in_words() {
        assert_eq!(before(2, Unit::Days).describe(today(), false), "2d");
        assert_eq!(before(1, Unit::Months).describe(today(), false), "1mo");
        assert_eq!(before(1, Unit::Days).explain(today(), false), "1 day before");
        assert_eq!(before(3, Unit::Hours).explain(today(), false), "3 hours before");
        assert_eq!(Reminder::On(Deadline { date: day(4), time: None }).explain(today(), false), "on Fri");
    }

    #[test]
    fn a_unit_then_a_number_sets_the_reminder() {
        let mut picker = Picker::new((day(9), 0), None);
        assert_eq!(send(&mut picker, "d2<cr>"), Action::Set(Some(before(2, Unit::Days))));
        let mut picker = Picker::new((day(9), 0), None);
        assert_eq!(send(&mut picker, "h12<cr>"), Action::Set(Some(before(12, Unit::Hours))));
        let mut picker = Picker::new((day(9), 0), None);
        assert_eq!(send(&mut picker, "m1<cr>"), Action::Set(Some(before(1, Unit::Months))));
    }

    #[test]
    fn what_isnt_a_number_from_1_says_so() {
        let mut picker = Picker::new((day(9), 0), None);
        for typed in ["<cr>", "0<cr>", "two<cr>", "1000<cr>"] {
            send(&mut picker, "<esc>d");
            assert_eq!(send(&mut picker, typed), Action::Stay, "{typed}");
            assert!(picker.error.is_some(), "{typed}");
        }
        // Typing again clears the error.
        send(&mut picker, "3");
        assert!(picker.error.is_none());
    }

    #[test]
    fn esc_steps_back_then_closes() {
        let mut picker = Picker::new((day(9), 0), None);
        assert_eq!(send(&mut picker, "d4<esc>"), Action::Stay);
        assert!(picker.amount.is_none());
        assert_eq!(send(&mut picker, "<esc>"), Action::Close);
    }

    #[test]
    fn changing_a_reminder_starts_from_its_amount_and_typing_replaces_it() {
        let mut picker = Picker::new((day(9), 0), Some(before(5, Unit::Days)));
        send(&mut picker, "d");
        assert_eq!(picker.amount.as_ref().unwrap().1.text, "5");
        assert_eq!(send(&mut picker, "3<cr>"), Action::Set(Some(before(3, Unit::Days))));
        // Another unit starts empty.
        let mut picker = Picker::new((day(9), 0), Some(before(5, Unit::Days)));
        send(&mut picker, "h");
        assert_eq!(picker.amount.as_ref().unwrap().1.text, "");
        // Enter keeps it as it was.
        let mut picker = Picker::new((day(9), 0), Some(before(5, Unit::Days)));
        assert_eq!(send(&mut picker, "d<cr>"), Action::Set(Some(before(5, Unit::Days))));
    }

    #[test]
    fn x_removes_a_reminder_only_if_there_is_one() {
        let mut picker = Picker::new((day(9), 0), Some(before(5, Unit::Days)));
        assert_eq!(send(&mut picker, "x"), Action::Set(None));
        let mut picker = Picker::new((day(9), 0), None);
        assert_eq!(send(&mut picker, "x"), Action::Stay);
    }

    #[test]
    fn c_chooses_a_day_on_the_calendar() {
        let mut picker = Picker::new((day(9), 0), None);
        assert_eq!(send(&mut picker, "c"), Action::Calendar);
    }
}
