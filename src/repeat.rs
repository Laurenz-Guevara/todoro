//! Repeating items (`Space r`): on days of the week, every so many weeks,
//! or every so many days or months, until an end day if one is set. The
//! rule is kept on the series' template (`store::TEMPLATES`), with its
//! notes; when a day it repeats on comes, todoro adds a fresh copy to that
//! day (`Store::roll_over`), with its time moved to the new day.

use chrono::{Datelike, Days, Months, NaiveDate, NaiveTime, Weekday};
use ratatui::crossterm::event::KeyCode;

use crate::input::LineInput;
use crate::timeline;

/// The most of any unit a repeat can be every, to keep date maths quick.
pub const MAX_EVERY: u32 = 999;

/// Which days of the week, Monday first, as bits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Weekdays(u8);

const WORKDAYS: u8 = 0b001_1111;
const ALL_DAYS: u8 = 0b111_1111;

/// The days of the week as stored and shown, Monday first.
const DAY_NAMES: [(&str, &str); 7] =
    [("mon", "Mon"), ("tue", "Tue"), ("wed", "Wed"), ("thu", "Thu"), ("fri", "Fri"), ("sat", "Sat"), ("sun", "Sun")];

impl Weekdays {
    pub fn of(day: Weekday) -> Self {
        Self(1 << day.num_days_from_monday())
    }

    pub fn has(self, day: Weekday) -> bool {
        self.0 & (1 << day.num_days_from_monday()) != 0
    }

    pub fn toggle(&mut self, day: Weekday) {
        self.0 ^= 1 << day.num_days_from_monday();
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    fn days(self) -> impl Iterator<Item = (usize, Weekday)> {
        (0..7).filter(move |n| self.0 & (1 << n) != 0).map(|n| (n, weekday(n)))
    }

    /// `Mon, Thu`, or `weekdays` and `every day` for those sets.
    fn describe(self) -> String {
        match self.0 {
            WORKDAYS => "weekdays".into(),
            ALL_DAYS => "every day".into(),
            _ => self.days().map(|(n, _)| DAY_NAMES[n].1).collect::<Vec<_>>().join(", "),
        }
    }
}

/// The day of the week `n` days after Monday.
pub fn weekday(n: usize) -> Weekday {
    Weekday::try_from(n as u8).expect("a day of the week")
}

/// What a repeat counts in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pattern {
    Days,
    /// On these days of the weeks it falls in.
    Weeks(Weekdays),
    /// On the day of the month it started on, or the month's last day if
    /// that's sooner.
    Months,
}

/// How an item repeats: every `every` days, weeks or months, counted from
/// `from`, the day it was set on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Repeat {
    pub every: u32,
    pub pattern: Pattern,
    pub from: NaiveDate,
    /// The last day it can fall on, if it ends.
    pub until: Option<NaiveDate>,
}

impl Repeat {
    /// How it's written to disk: `3d from 2026-10-05`,
    /// `1w mon,thu from 2026-10-05` or `1mo from 2026-10-05`, with
    /// ` until 2026-12-31` after if it ends.
    pub fn to_stored(self) -> String {
        let rule = match self.pattern {
            Pattern::Days => format!("{}d", self.every),
            Pattern::Weeks(days) => {
                let names: Vec<&str> = days.days().map(|(n, _)| DAY_NAMES[n].0).collect();
                format!("{}w {}", self.every, names.join(","))
            }
            Pattern::Months => format!("{}mo", self.every),
        };
        let until = self.until.map_or(String::new(), |until| format!(" until {}", until.format("%Y-%m-%d")));
        format!("{rule} from {}{until}", self.from.format("%Y-%m-%d"))
    }

    /// Reads what `to_stored` wrote, or `None` if it isn't a repeat.
    pub fn from_stored(text: &str) -> Option<Self> {
        let (rule, from) = text.trim().split_once(" from ")?;
        let (from, until) = match from.split_once(" until ") {
            Some((from, until)) => (from, Some(NaiveDate::parse_from_str(until.trim(), "%Y-%m-%d").ok()?)),
            None => (from, None),
        };
        let from = NaiveDate::parse_from_str(from.trim(), "%Y-%m-%d").ok()?;
        let every = |amount: &str| amount.parse::<u32>().ok().filter(|every| (1..=MAX_EVERY).contains(every));
        let (every, pattern) = if let Some((amount, names)) = rule.split_once(' ') {
            let mut days = Weekdays::default();
            for name in names.split(',') {
                let n = DAY_NAMES.iter().position(|(stored, _)| *stored == name.trim())?;
                days.toggle(weekday(n));
            }
            if days.is_empty() {
                return None;
            }
            (every(amount.strip_suffix('w')?)?, Pattern::Weeks(days))
        } else if let Some(amount) = rule.strip_suffix("mo") {
            (every(amount)?, Pattern::Months)
        } else {
            (every(rule.strip_suffix('d')?)?, Pattern::Days)
        };
        Some(Self { every, pattern, from, until })
    }

    /// Whether it falls on `date`.
    pub fn occurs_on(self, date: NaiveDate) -> bool {
        if date < self.from || self.until.is_some_and(|until| date > until) {
            return false;
        }
        let every = i64::from(self.every);
        match self.pattern {
            Pattern::Days => (date - self.from).num_days() % every == 0,
            Pattern::Weeks(days) => {
                let monday = |day: NaiveDate| day - chrono::TimeDelta::days(day.weekday().num_days_from_monday().into());
                let weeks = (monday(date) - monday(self.from)).num_days() / 7;
                days.has(date.weekday()) && weeks % every == 0
            }
            Pattern::Months => {
                let months = |day: NaiveDate| i64::from(day.year()) * 12 + i64::from(day.month0());
                let gone = months(date) - months(self.from);
                gone % every == 0 && date.day() == self.from.day().min(days_in_month(date))
            }
        }
    }

    /// The first day after `day` it falls on.
    pub fn next_after(self, day: NaiveDate) -> Option<NaiveDate> {
        // A whole cycle, with room for the longest month.
        let limit = u64::from(self.every) * 32 + 32;
        (1..=limit)
            .filter_map(|n| day.checked_add_days(Days::new(n)))
            .take_while(|&date| self.until.is_none_or(|until| date <= until))
            .find(|&date| self.occurs_on(date))
    }

    /// In words: `every Mon, Thu`, `every 2 weeks on Mon`, `every 3 days`,
    /// `every month on the 5th`.
    pub fn describe(self) -> String {
        let plural = |unit: &str| if self.every == 1 { format!("every {unit}") } else { format!("every {} {unit}s", self.every) };
        let rule = match self.pattern {
            Pattern::Days => plural("day"),
            Pattern::Weeks(days) if self.every == 1 => match days.0 {
                WORKDAYS => "every weekday".into(),
                ALL_DAYS => "every day".into(),
                _ => format!("every {}", days.describe()),
            },
            Pattern::Weeks(days) => format!("{} on {}", plural("week"), days.describe()),
            Pattern::Months => format!("{} on the {}", plural("month"), ordinal(self.from.day())),
        };
        match self.until {
            Some(until) => format!("{rule} until {}", until.format("%-d %b %Y")),
            None => rule,
        }
    }
}

/// Reads a typed end day: `2026-12-31`, `31 Dec`, `Dec 31`, with a year or
/// without (then the first such day on or after `from`). `None` for
/// nothing typed. It mustn't be before `from`.
pub fn read_date(text: &str, from: NaiveDate) -> Result<Option<NaiveDate>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let words: Vec<&str> = text.split([' ', ',']).filter(|word| !word.is_empty()).collect();
    let month = |word: &str| {
        let word = word.to_lowercase();
        let names = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
        (word.len() >= 3).then(|| names.iter().position(|name| word.starts_with(name))).flatten().map(|n| n as u32 + 1)
    };
    let date = NaiveDate::parse_from_str(text, "%Y-%m-%d").ok().or_else(|| {
        let (day, month, year) = match words.as_slice() {
            [a, b] | [a, b, _] if a.parse::<u32>().is_ok() => (a.parse().ok()?, month(b)?, words.get(2)),
            [a, b] | [a, b, _] => (b.parse().ok()?, month(a)?, words.get(2)),
            _ => return None,
        };
        match year {
            Some(year) => NaiveDate::from_ymd_opt(year.parse().ok()?, month, day),
            None => (from.year()..from.year() + 9)
                .filter_map(|year| NaiveDate::from_ymd_opt(year, month, day))
                .find(|&date| date >= from),
        }
    });
    match date {
        None => Err("Type a day like 2026-12-31 or 31 Dec, or nothing for no end".into()),
        Some(date) if date < from => Err(format!("It can't end before {}", from.format("%-d %b %Y"))),
        Some(date) => Ok(Some(date)),
    }
}

fn days_in_month(date: NaiveDate) -> u32 {
    let first = date.with_day(1).expect("every month has a first");
    let next = first.checked_add_months(Months::new(1)).unwrap_or(NaiveDate::MAX);
    (next - first).num_days() as u32
}

/// `1st`, `2nd`, `3rd`, `4th`, `11th`, `21st`.
pub fn ordinal(n: u32) -> String {
    let suffix = match (n % 10, n % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

/// What's being chosen in the popup, once `w`, `d` or `m` is pressed.
#[derive(Debug)]
pub enum Stage {
    /// Days of the week, every `every` weeks; `cursor` is the day (0 for
    /// Monday) `Space` toggles.
    Weeks { every: LineInput, days: Weekdays, cursor: usize },
    Days(LineInput),
    Months(LineInput),
}

impl Stage {
    pub fn every(&self) -> &LineInput {
        match self {
            Stage::Weeks { every, .. } | Stage::Days(every) | Stage::Months(every) => every,
        }
    }

    fn every_mut(&mut self) -> &mut LineInput {
        match self {
            Stage::Weeks { every, .. } | Stage::Days(every) | Stage::Months(every) => every,
        }
    }

    pub fn unit(&self) -> &'static str {
        match self {
            Stage::Weeks { .. } => "weeks",
            Stage::Days(_) => "days",
            Stage::Months(_) => "months",
        }
    }
}

/// The `Space r` popup: choosing how the selected item repeats.
pub struct Picker {
    /// Where the item is stored (`store::TEMPLATES` for a preview).
    pub day: NaiveDate,
    pub index: usize,
    /// The day a repeat counts from: the item's, or for one in a series,
    /// the day the series does.
    pub from: NaiveDate,
    /// The day after which copies come: the last a series added, or for a
    /// new one, the item's day or yesterday, whichever is later.
    pub after: NaiveDate,
    /// How it repeats now, which `x` stops.
    pub current: Option<Repeat>,
    pub stage: Option<Stage>,
    /// Whether the number is still the one it started with, so typing
    /// replaces it rather than adding to it.
    fresh: bool,
    /// Why what's chosen can't be saved.
    pub error: Option<String>,
    /// Asking whether to save what's chosen, after Enter.
    pub confirming: bool,
    /// The option highlighted before choosing (in `choices`), for `j`/`k`
    /// and Enter.
    pub selected: usize,
    /// Typing its time, after Enter and before asking.
    pub timing: Option<Timing>,
    /// Its start and end on each day it repeats on: what it has, until
    /// another is typed. The app sets the item's time to it on saving.
    pub times: Option<(NaiveTime, NaiveTime)>,
    /// The last day it repeats on, if it ends: what it has, until another
    /// is typed.
    pub until: Option<NaiveDate>,
}

/// Typing the repeat's time, as in the plan view: a start, then an end.
#[derive(Debug)]
pub enum Timing {
    Start(LineInput),
    /// The end, once the start is typed.
    End(NaiveTime, LineInput),
    /// The last day it repeats on, after its time; nothing typed for none.
    Until(LineInput),
}

/// What the app should do after the picker handles a key.
#[derive(Debug, PartialEq)]
pub enum Action {
    Stay,
    Close,
    /// Set (or with `None`, stop) the item's repeat.
    Set(Option<Repeat>),
}

impl Picker {
    pub fn new((day, index): (NaiveDate, usize), current: Option<Repeat>) -> Self {
        // Starting on what it repeats in now, if it does.
        let selected = match current.map(|repeat| repeat.pattern) {
            Some(Pattern::Days) => 1,
            Some(Pattern::Months) => 2,
            _ => 0,
        };
        let until = current.and_then(|repeat| repeat.until);
        Self { day, index, from: day, after: day, current, stage: None, fresh: false, error: None, confirming: false, selected, timing: None, times: None, until }
    }

    /// The options before choosing, by their keys: `x` (stop) only when it
    /// repeats.
    pub fn choices(&self) -> &'static [char] {
        if self.current.is_some() { &['w', 'd', 'm', 'x'] } else { &['w', 'd', 'm'] }
    }

    pub fn handle_key(&mut self, code: KeyCode) -> Action {
        if self.timing.is_some() {
            return self.time_key(code);
        }
        // Enter or y saves; n or Esc goes back to choosing, as it was.
        if self.confirming {
            match code {
                KeyCode::Enter | KeyCode::Char('y') => return Action::Set(self.chosen()),
                KeyCode::Esc | KeyCode::Char('n') => {
                    self.confirming = false;
                    self.timing = Some(self.until_input());
                }
                _ => {}
            }
            return Action::Stay;
        }
        let Some(stage) = &self.stage else { return self.choose(code) };
        match code {
            // Back to choosing what it counts in.
            KeyCode::Esc => {
                self.stage = None;
                self.error = None;
            }
            // Then its time, if it's to have one, before asking.
            KeyCode::Enter => match self.chosen() {
                Some(_) => self.timing = Some(Timing::Start(LineInput::default())),
                None => {
                    self.error = Some(match stage {
                        Stage::Weeks { days, .. } if days.is_empty() => {
                            "Choose a day: Space picks the one under the cursor".into()
                        }
                        stage => format!("Type every how many {}, from 1 to {MAX_EVERY}", stage.unit()),
                    });
                }
            },
            KeyCode::Char(c) if c.is_ascii_digit() => self.type_number(code),
            KeyCode::Backspace => self.type_number(code),
            code => {
                if let Some(Stage::Weeks { days, cursor, .. }) = &mut self.stage {
                    match code {
                        KeyCode::Char('h') | KeyCode::Left => *cursor = cursor.saturating_sub(1),
                        KeyCode::Char('l') | KeyCode::Right => *cursor = (*cursor + 1).min(6),
                        KeyCode::Char(' ' | 'x') => days.toggle(weekday(*cursor)),
                        _ => return Action::Stay,
                    }
                    self.error = None;
                }
            }
        }
        Action::Stay
    }

    /// Keys while typing its time. Nothing typed for the start keeps the
    /// time it has (or none) and goes on to asking; for the end, it's an
    /// hour after the start, as in the plan view. `Esc` steps back.
    fn time_key(&mut self, code: KeyCode) -> Action {
        let Some(timing) = &mut self.timing else { return Action::Stay };
        match (timing, code) {
            (Timing::Start(_), KeyCode::Esc) => self.timing = None,
            (Timing::End(..) | Timing::Until(_), KeyCode::Esc) => self.timing = Some(Timing::Start(LineInput::default())),
            (Timing::Start(input), KeyCode::Enter) => match timeline::read_time(&input.text) {
                Err(error) => {
                    self.error = Some(error);
                    return Action::Stay;
                }
                Ok(None) => self.timing = Some(self.until_input()),
                Ok(Some(start)) => self.timing = Some(Timing::End(start, LineInput::default())),
            },
            (Timing::Until(input), KeyCode::Enter) => match read_date(&input.text, self.from) {
                Err(error) => {
                    self.error = Some(error);
                    return Action::Stay;
                }
                Ok(until) => {
                    self.until = until;
                    self.timing = None;
                    self.confirming = true;
                }
            },
            (Timing::End(start, input), KeyCode::Enter) => {
                let start = *start;
                match timeline::read_time(&input.text) {
                    Err(error) => {
                        self.error = Some(error);
                        return Action::Stay;
                    }
                    Ok(end) => {
                        let end = end.unwrap_or_else(|| timeline::an_hour_after(start));
                        if end <= start {
                            self.error = Some("It has to end after it starts".into());
                            return Action::Stay;
                        }
                        self.times = Some((start, end));
                        self.timing = Some(self.until_input());
                    }
                }
            }
            (Timing::Start(input) | Timing::End(_, input) | Timing::Until(input), code) => {
                input.handle_key(code);
            }
        }
        self.error = None;
        Action::Stay
    }

    /// The repeat chosen so far, if it can be saved, to show as it's chosen.
    pub fn chosen(&self) -> Option<Repeat> {
        let stage = self.stage.as_ref()?;
        let every = stage.every().text.trim().parse::<u32>().ok().filter(|every| (1..=MAX_EVERY).contains(every))?;
        let pattern = match stage {
            Stage::Weeks { days, .. } if days.is_empty() => return None,
            Stage::Weeks { days, .. } => Pattern::Weeks(*days),
            Stage::Days(_) => Pattern::Days,
            Stage::Months(_) => Pattern::Months,
        };
        Some(Repeat { every, pattern, from: self.from, until: self.until })
    }

    /// The repeat to show while choosing: `chosen`, with the last day being
    /// typed as soon as it reads as one (or no end, once it's cleared).
    pub fn shown(&self) -> Option<Repeat> {
        let mut repeat = self.chosen()?;
        if let Some(Timing::Until(input)) = &self.timing
            && let Ok(until) = read_date(&input.text, self.from)
        {
            repeat.until = until;
        }
        Some(repeat)
    }

    /// Typing the end day, starting with the one it has.
    fn until_input(&self) -> Timing {
        Timing::Until(LineInput::new(&self.until.map_or(String::new(), |until| until.format("%Y-%m-%d").to_string())))
    }

    /// Edits the number, replacing it if it's the one it started with.
    fn type_number(&mut self, code: KeyCode) {
        let Some(stage) = &mut self.stage else { return };
        let input = stage.every_mut();
        if std::mem::take(&mut self.fresh) && matches!(code, KeyCode::Char(_)) {
            *input = LineInput::default();
        }
        input.handle_key(code);
        self.error = None;
    }

    /// Keys before choosing: `w` days of the week, `d` days, `m` months,
    /// `x` stop repeating.
    fn choose(&mut self, code: KeyCode) -> Action {
        let last = self.choices().len() - 1;
        let code = match code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.selected = (self.selected + 1).min(last);
                return Action::Stay;
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
                return Action::Stay;
            }
            // The highlighted option, as if its key were pressed.
            KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => KeyCode::Char(self.choices()[self.selected.min(last)]),
            code => code,
        };
        if let KeyCode::Char(key) = code
            && let Some(n) = self.choices().iter().position(|&choice| choice == key)
        {
            self.selected = n;
        }
        let current = self.current;
        let every = |same: bool| LineInput::new(&current.filter(|_| same).map_or(1, |repeat| repeat.every).to_string());
        let stage = match code {
            KeyCode::Char('w') => {
                let had = current.and_then(|repeat| match repeat.pattern {
                    Pattern::Weeks(days) => Some(days),
                    _ => None,
                });
                let today = self.from.weekday();
                let days = had.unwrap_or(Weekdays::of(today));
                let cursor = days.days().next().map_or(today.num_days_from_monday() as usize, |(n, _)| n);
                Stage::Weeks { every: every(had.is_some()), days, cursor }
            }
            KeyCode::Char('d') => Stage::Days(every(current.is_some_and(|repeat| repeat.pattern == Pattern::Days))),
            KeyCode::Char('m') => Stage::Months(every(current.is_some_and(|repeat| repeat.pattern == Pattern::Months))),
            KeyCode::Char('x') if self.current.is_some() => return Action::Set(None),
            KeyCode::Esc | KeyCode::Char('q') => return Action::Close,
            _ => return Action::Stay,
        };
        self.stage = Some(stage);
        self.fresh = true;
        Action::Stay
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::today;

    fn day(days: i64) -> NaiveDate {
        today() + chrono::TimeDelta::days(days)
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn weeks(every: u32, days: &[Weekday]) -> Repeat {
        let mut set = Weekdays::default();
        for &d in days {
            set.toggle(d);
        }
        Repeat { every, pattern: Pattern::Weeks(set), from: today(), until: None }
    }

    fn send(picker: &mut Picker, keys: &str) -> Action {
        let mut action = Action::Stay;
        let mut rest = keys;
        while let Some(c) = rest.chars().next() {
            let (code, len) = if rest.starts_with("<esc>") {
                (KeyCode::Esc, 5)
            } else if rest.starts_with("<cr>") {
                (KeyCode::Enter, 4)
            } else if rest.starts_with("<bs>") {
                (KeyCode::Backspace, 4)
            } else {
                (KeyCode::Char(c), c.len_utf8())
            };
            action = picker.handle_key(code);
            rest = &rest[len..];
        }
        action
    }

    #[test]
    fn repeats_are_stored_as_text_and_read_back() {
        // today() is a Monday.
        for (repeat, stored) in [
            (Repeat { every: 3, pattern: Pattern::Days, from: today(), until: None }, "3d from 2026-10-05"),
            (weeks(1, &[Weekday::Mon, Weekday::Thu]), "1w mon,thu from 2026-10-05"),
            (weeks(2, &[Weekday::Sun]), "2w sun from 2026-10-05"),
            (Repeat { every: 1, pattern: Pattern::Months, from: today(), until: None }, "1mo from 2026-10-05"),
        ] {
            assert_eq!(repeat.to_stored(), stored);
            assert_eq!(Repeat::from_stored(stored), Some(repeat), "{stored}");
        }
        for broken in ["", "weekly", "3d", "0d from 2026-10-05", "1w from 2026-10-05", "1w xyz from 2026-10-05", "1y from 2026-10-05", "2d from soon"] {
            assert_eq!(Repeat::from_stored(broken), None, "{broken}");
        }
    }

    #[test]
    fn days_repeat_every_so_many_days_from_the_start() {
        let every_three = Repeat { every: 3, pattern: Pattern::Days, from: today(), until: None };
        let on: Vec<i64> = (-3..10).filter(|&n| every_three.occurs_on(day(n))).collect();
        assert_eq!(on, [0, 3, 6, 9]);
    }

    #[test]
    fn weeks_repeat_on_their_days_in_every_so_many_weeks() {
        let weekly = weeks(1, &[Weekday::Mon, Weekday::Thu]);
        let on: Vec<i64> = (0..14).filter(|&n| weekly.occurs_on(day(n))).collect();
        assert_eq!(on, [0, 3, 7, 10]);
        // Every other week, counted from the week it started in.
        let fortnightly = Repeat { from: day(2), ..weeks(2, &[Weekday::Mon, Weekday::Thu]) };
        let on: Vec<i64> = (0..28).filter(|&n| fortnightly.occurs_on(day(n))).collect();
        assert_eq!(on, [3, 14, 17]);
    }

    #[test]
    fn months_keep_their_day_or_take_the_months_last() {
        let monthly = Repeat { every: 1, pattern: Pattern::Months, from: date(2026, 1, 31), until: None };
        assert!(monthly.occurs_on(date(2026, 2, 28)));
        assert!(!monthly.occurs_on(date(2026, 3, 28)));
        assert!(monthly.occurs_on(date(2026, 3, 31)));
        let quarterly = Repeat { every: 3, pattern: Pattern::Months, from: today(), until: None };
        assert_eq!(quarterly.next_after(today()), Some(date(2027, 1, 5)));
        assert!(!quarterly.occurs_on(date(2026, 11, 5)));
    }

    #[test]
    fn next_after_finds_the_next_day() {
        assert_eq!(weeks(1, &[Weekday::Mon, Weekday::Thu]).next_after(today()), Some(day(3)));
        assert_eq!(weeks(1, &[Weekday::Mon]).next_after(today()), Some(day(7)));
        assert_eq!(weeks(3, &[Weekday::Mon]).next_after(today()), Some(day(21)));
    }

    #[test]
    fn repeats_are_described_in_words() {
        assert_eq!(weeks(1, &[Weekday::Mon, Weekday::Thu]).describe(), "every Mon, Thu");
        assert_eq!(weeks(2, &[Weekday::Fri]).describe(), "every 2 weeks on Fri");
        let workdays = [Weekday::Mon, Weekday::Tue, Weekday::Wed, Weekday::Thu, Weekday::Fri];
        assert_eq!(weeks(1, &workdays).describe(), "every weekday");
        assert_eq!(weeks(2, &workdays).describe(), "every 2 weeks on weekdays");
        assert_eq!(Repeat { every: 1, pattern: Pattern::Days, from: today(), until: None }.describe(), "every day");
        assert_eq!(Repeat { every: 3, pattern: Pattern::Days, from: today(), until: None }.describe(), "every 3 days");
        assert_eq!(Repeat { every: 1, pattern: Pattern::Months, from: today(), until: None }.describe(), "every month on the 5th");
        assert_eq!(Repeat { every: 2, pattern: Pattern::Months, from: date(2026, 1, 22), until: None }.describe(), "every 2 months on the 22nd");
        assert_eq!(ordinal(11), "11th");
        assert_eq!(ordinal(31), "31st");
    }

    #[test]
    fn w_chooses_days_of_the_week_starting_from_the_items_own() {
        let mut picker = Picker::new((today(), 0), None);
        // Monday is chosen; move to Thursday and add it.
        assert_eq!(send(&mut picker, "wlll "), Action::Stay);
        assert_eq!(send(&mut picker, "<cr><cr><cr>"), Action::Stay);
        assert!(picker.confirming);
        assert_eq!(send(&mut picker, "<cr>"), Action::Set(Some(weeks(1, &[Weekday::Mon, Weekday::Thu]))));
    }

    #[test]
    fn typing_a_number_repeats_every_so_many_weeks() {
        let mut picker = Picker::new((today(), 0), None);
        assert_eq!(send(&mut picker, "w2<cr><cr><cr><cr>"), Action::Set(Some(weeks(2, &[Weekday::Mon]))));
        let mut picker = Picker::new((today(), 0), None);
        assert_eq!(send(&mut picker, "w12<bs><cr><cr><cr><cr>"), Action::Set(Some(weeks(1, &[Weekday::Mon]))));
    }

    #[test]
    fn weeks_need_a_day_and_a_number() {
        let mut picker = Picker::new((today(), 0), None);
        assert_eq!(send(&mut picker, "w <cr>"), Action::Stay);
        assert!(picker.error.as_deref().is_some_and(|error| error.contains("Choose a day")));
        assert_eq!(send(&mut picker, " <bs><cr>"), Action::Stay);
        assert!(picker.error.as_deref().is_some_and(|error| error.contains("weeks")));
    }

    #[test]
    fn d_and_m_repeat_every_so_many_days_or_months() {
        let mut picker = Picker::new((today(), 0), None);
        assert_eq!(send(&mut picker, "d3<cr><cr><cr><cr>"), Action::Set(Some(Repeat { every: 3, pattern: Pattern::Days, from: today(), until: None })));
        let mut picker = Picker::new((today(), 0), None);
        assert_eq!(send(&mut picker, "m<cr><cr><cr><cr>"), Action::Set(Some(Repeat { every: 1, pattern: Pattern::Months, from: today(), until: None })));
        let mut picker = Picker::new((today(), 0), None);
        assert_eq!(send(&mut picker, "d0<cr>"), Action::Stay);
        assert!(picker.error.is_some());
    }

    #[test]
    fn the_current_repeat_is_where_choosing_starts_and_x_stops_it() {
        let current = Repeat { from: day(-7), ..weeks(2, &[Weekday::Tue, Weekday::Fri]) };
        let mut picker = Picker::new((today(), 0), Some(current));
        // Counted again from the item's day.
        assert_eq!(send(&mut picker, "w<cr><cr><cr><cr>"), Action::Set(Some(weeks(2, &[Weekday::Tue, Weekday::Fri]))));
        let mut picker = Picker::new((today(), 0), Some(current));
        assert_eq!(send(&mut picker, "x"), Action::Set(None));
        // Without one, x does nothing.
        let mut picker = Picker::new((today(), 0), None);
        assert_eq!(send(&mut picker, "x"), Action::Stay);
    }

    #[test]
    fn enter_asks_first_and_n_or_esc_steps_back() {
        let mut picker = Picker::new((today(), 0), None);
        assert_eq!(send(&mut picker, "w<cr><cr><cr>"), Action::Stay);
        assert!(picker.confirming);
        // Other keys wait for an answer.
        assert_eq!(send(&mut picker, "lx"), Action::Stay);
        assert_eq!(send(&mut picker, "n"), Action::Stay);
        assert!(!picker.confirming);
        assert!(matches!(picker.timing, Some(Timing::Until(_))), "back to its last day");
        // Back to choosing, with what was chosen.
        assert_eq!(send(&mut picker, "<esc><esc>l <cr><cr><cr><esc><esc><esc>"), Action::Stay);
        assert!(!picker.confirming && picker.timing.is_none() && picker.stage.is_some());
        assert_eq!(send(&mut picker, "<cr><cr><cr>y"), Action::Set(Some(weeks(1, &[Weekday::Mon, Weekday::Tue]))));
    }

    #[test]
    fn a_last_day_can_be_typed_after_the_time() {
        let mut picker = Picker::new((today(), 0), None);
        send(&mut picker, "w<cr><cr>");
        assert!(matches!(picker.timing, Some(Timing::Until(_))));
        send(&mut picker, "soon<cr>");
        assert!(picker.error.as_deref().is_some_and(|error| error.contains("31 Dec")));
        send(&mut picker, &"<bs>".repeat(4));
        send(&mut picker, "1 Oct<cr>");
        assert!(picker.confirming);
        assert_eq!(picker.until, Some(date(2027, 10, 1)), "a day without a year is the next one");
        let until = Repeat { until: Some(date(2027, 10, 1)), ..weeks(1, &[Weekday::Mon]) };
        assert_eq!(send(&mut picker, "y"), Action::Set(Some(until)));
        // It starts with the one it has; clearing it means no end.
        let mut picker = Picker::new((today(), 0), Some(until));
        send(&mut picker, "w<cr><cr>");
        let Some(Timing::Until(input)) = &picker.timing else { panic!("typing the last day") };
        assert_eq!(input.text, "2027-10-01");
        send(&mut picker, &format!("{}<cr>", "<bs>".repeat(10)));
        assert_eq!(send(&mut picker, "y"), Action::Set(Some(weeks(1, &[Weekday::Mon]))));
    }

    #[test]
    fn the_last_day_shows_as_soon_as_it_reads_as_one() {
        let mut picker = Picker::new((today(), 0), None);
        send(&mut picker, "w<cr><cr>1 D");
        assert_eq!(picker.shown(), Some(weeks(1, &[Weekday::Mon])), "not a day yet");
        send(&mut picker, "ec");
        assert_eq!(picker.shown().and_then(|repeat| repeat.until), Some(date(2026, 12, 1)));
        assert_eq!(picker.until, None, "kept only on Enter");
        send(&mut picker, "<bs><bs><bs><bs><bs>");
        assert_eq!(picker.shown(), Some(weeks(1, &[Weekday::Mon])));
    }

    #[test]
    fn last_days_are_read_in_a_few_ways() {
        let from = today();
        for (text, expected) in [
            ("2026-12-31", date(2026, 12, 31)),
            ("31 Dec", date(2026, 12, 31)),
            ("Dec 31", date(2026, 12, 31)),
            ("december 31, 2027", date(2027, 12, 31)),
            ("5 oct", date(2026, 10, 5)),
            ("4 Oct", date(2027, 10, 4)),
        ] {
            assert_eq!(read_date(text, from), Ok(Some(expected)), "{text}");
        }
        assert_eq!(read_date("  ", from), Ok(None));
        assert!(read_date("2026-10-04", from).is_err());
        assert!(read_date("31 Feb", from).is_err());
        assert!(read_date("next week", from).is_err());
    }

    #[test]
    fn j_k_and_enter_choose_the_highlighted_option() {
        let mut picker = Picker::new((today(), 0), None);
        assert_eq!(picker.selected, 0);
        // Stops at the ends: there's no x without a repeat.
        assert_eq!(send(&mut picker, "jjjj"), Action::Stay);
        assert_eq!(picker.selected, 2);
        send(&mut picker, "k");
        picker.handle_key(KeyCode::Down);
        picker.handle_key(KeyCode::Up);
        assert_eq!(picker.selected, 1);
        send(&mut picker, "<cr>");
        assert!(matches!(picker.stage, Some(Stage::Days(_))));
        // Back again, still on it; l chooses too.
        send(&mut picker, "<esc>");
        assert_eq!(picker.selected, 1);
        send(&mut picker, "kl");
        assert!(matches!(picker.stage, Some(Stage::Weeks { .. })));
        // A key goes straight to its option, and is highlighted after.
        send(&mut picker, "<esc>m<esc>");
        assert_eq!(picker.selected, 2);
    }

    #[test]
    fn it_starts_on_what_it_repeats_in_and_can_reach_x() {
        let monthly = Repeat { every: 1, pattern: Pattern::Months, from: today(), until: None };
        let mut picker = Picker::new((today(), 0), Some(monthly));
        assert_eq!(picker.selected, 2);
        assert_eq!(send(&mut picker, "j<cr>"), Action::Set(None));
    }

    fn at(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    #[test]
    fn a_time_can_be_typed_after_choosing_like_planning() {
        let mut picker = Picker::new((today(), 0), None);
        send(&mut picker, "w<cr>");
        assert!(matches!(picker.timing, Some(Timing::Start(_))));
        send(&mut picker, "10:15<cr>10:45<cr><cr>");
        assert_eq!(picker.times, Some((at(10, 15), at(10, 45))));
        assert!(picker.confirming);
        assert_eq!(send(&mut picker, "y"), Action::Set(Some(weeks(1, &[Weekday::Mon]))));
        // Nothing typed for the end is an hour.
        let mut picker = Picker::new((today(), 0), None);
        send(&mut picker, "w<cr>9am<cr><cr>");
        assert_eq!(picker.times, Some((at(9, 0), at(10, 0))));
    }

    #[test]
    fn nothing_typed_keeps_the_time_it_had_and_mistakes_are_said() {
        let mut picker = Picker::new((today(), 0), None);
        picker.times = Some((at(10, 15), at(10, 45)));
        send(&mut picker, "w<cr><cr><cr>");
        assert!(picker.confirming);
        assert_eq!(picker.times, Some((at(10, 15), at(10, 45))));
        let mut picker = Picker::new((today(), 0), None);
        send(&mut picker, "w<cr>soon<cr>");
        assert!(picker.error.as_deref().is_some_and(|error| error.contains("Type a time")));
        send(&mut picker, "<esc><cr>10<cr>9<cr>");
        assert_eq!(picker.error.as_deref(), Some("It has to end after it starts"));
        // Esc steps back: to the start, then to choosing.
        send(&mut picker, "<esc>");
        assert!(matches!(picker.timing, Some(Timing::Start(_))));
        send(&mut picker, "<esc>");
        assert!(picker.timing.is_none() && picker.stage.is_some());
        assert_eq!(picker.times, None);
    }

    #[test]
    fn esc_steps_back_then_closes() {
        let mut picker = Picker::new((today(), 0), None);
        assert_eq!(send(&mut picker, "d<esc>"), Action::Stay);
        assert!(picker.stage.is_none());
        assert_eq!(send(&mut picker, "<esc>"), Action::Close);
    }
}
