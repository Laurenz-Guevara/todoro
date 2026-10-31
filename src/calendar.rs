//! The calendar: a week, month or year at a glance, for planning ahead.

use std::cell::Cell;

use chrono::{Datelike, Days, Months, NaiveDate};
use ratatui::crossterm::event::{KeyCode, KeyEvent};

use crate::deadline::{self, Deadline};
use crate::input::LineInput;
use crate::reminder::Reminder;
use crate::snake::{self, Dir, Snake};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Zoom {
    Week,
    Month,
    Year,
}

pub struct Calendar {
    pub zoom: Zoom,
    /// The selected date.
    pub cursor: NaiveDate,
    pub today: NaiveDate,
    /// The item being typed for the selected date, if any.
    pub adding: Option<LineInput>,
    /// Choosing a deadline for an item (`@`), or a day to be reminded of
    /// it (`r` then `c`), rather than browsing.
    pub picking: Option<Picking>,
    /// How many months the year view last drew side by side, which `j` and
    /// `k` follow to the month above or below. Set while drawing.
    pub year_columns: Cell<u32>,
    /// A `:` command being typed. There's only one, kept secret: `snake`.
    pub command: Option<LineInput>,
    /// A game of Snake on the year view, which keeps every other key away
    /// until `Esc` ends it.
    pub snake: Option<Snake>,
}

/// Choosing a deadline or a reminder: the day with the calendar, then a time.
pub struct Picking {
    /// Whether it's for a reminder rather than a deadline.
    pub reminder: bool,
    /// Where the item is stored.
    pub day: NaiveDate,
    pub index: usize,
    /// Its text, for the title.
    pub item: String,
    /// Whether it has a deadline already, which `d` removes.
    pub had: bool,
    /// The time being typed once a day is chosen. It starts with the
    /// deadline's time, if it had one.
    pub time: Option<LineInput>,
    /// Shown as typed, to start the time with.
    pub start_time: String,
    /// Whether the time is still the one it started with, untouched, so
    /// typing replaces it rather than adding to it.
    pub fresh: bool,
    /// Why the time typed isn't one.
    pub error: Option<String>,
}

/// What the app should do after the calendar handles a key.
#[derive(Debug, PartialEq)]
pub enum Action {
    Stay,
    /// Back to the list, on the day it was showing.
    Close,
    /// Back to the list, on this day.
    Open(NaiveDate),
    /// Add an item to the end of this day's open items.
    Add(NaiveDate, String),
    /// Set (or with `None`, remove) the deadline of the item stored at
    /// `day`, `index`.
    SetDeadline { day: NaiveDate, index: usize, deadline: Option<Deadline> },
    /// Set the reminder of the item stored at `day`, `index`.
    SetReminder { day: NaiveDate, index: usize, reminder: Reminder },
    Help,
}

impl Calendar {
    pub fn new(cursor: NaiveDate, today: NaiveDate) -> Self {
        Self { zoom: Zoom::Month, cursor, today, adding: None, picking: None, year_columns: Cell::new(1), command: None, snake: None }
    }

    /// The year view, to choose a deadline for the item stored at `day`,
    /// `index`: starting on its deadline if it has one, otherwise on `cursor`.
    pub fn for_deadline(
        (day, index): (NaiveDate, usize),
        item: &str,
        current: Option<Deadline>,
        cursor: NaiveDate,
        today: NaiveDate,
        twelve_hour: bool,
    ) -> Self {
        let start_time = current.and_then(|deadline| deadline.time).map(|time| deadline::format_time(time, twelve_hour));
        let picking = Picking {
            reminder: false,
            day,
            index,
            item: item.to_string(),
            had: current.is_some(),
            time: None,
            start_time: start_time.unwrap_or_default(),
            fresh: false,
            error: None,
        };
        let cursor = current.map_or(cursor, |deadline| deadline.date);
        Self { zoom: Zoom::Year, cursor, today, adding: None, picking: Some(picking), year_columns: Cell::new(1), command: None, snake: None }
    }

    /// The year view, to choose a day to be reminded of the item stored at
    /// `day`, `index`: starting on the day of its reminder if it's on one,
    /// otherwise on `cursor`.
    pub fn for_reminder(
        (day, index): (NaiveDate, usize),
        item: &str,
        current: Option<Reminder>,
        cursor: NaiveDate,
        today: NaiveDate,
        twelve_hour: bool,
    ) -> Self {
        let on = match current {
            Some(Reminder::On(on)) => Some(on),
            _ => None,
        };
        let mut calendar = Self::for_deadline((day, index), item, on, cursor, today, twelve_hour);
        if let Some(picking) = &mut calendar.picking {
            picking.reminder = true;
            // Removing a reminder is in the reminder popup.
            picking.had = false;
        }
        calendar
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Action {
        if self.snake.is_some() {
            self.snake_key(key.code);
            return Action::Stay;
        }
        if let Some(input) = &mut self.command {
            match key.code {
                KeyCode::Enter => {
                    let command = input.text.trim().to_string();
                    self.command = None;
                    if command == "snake" {
                        self.start_snake(snake::random_seed());
                    }
                }
                KeyCode::Esc => self.command = None,
                KeyCode::Backspace if input.text.is_empty() => self.command = None,
                code => {
                    input.handle_key(code);
                }
            }
            return Action::Stay;
        }
        if let Some(picking) = &mut self.picking {
            if let Some(input) = &mut picking.time {
                match key.code {
                    // Back to choosing the day.
                    KeyCode::Esc => {
                        picking.time = None;
                        picking.error = None;
                    }
                    KeyCode::Enter if input.text.trim().is_empty() => {
                        return picking.chosen(Deadline { date: self.cursor, time: None });
                    }
                    KeyCode::Enter => match deadline::parse_time(&input.text) {
                        Some(time) => return picking.chosen(Deadline { date: self.cursor, time: Some(time) }),
                        None => picking.error = Some("Type a time like 13:00, 1300, 1pm or 11:30am".into()),
                    },
                    code => {
                        if picking.fresh && matches!(code, KeyCode::Char(_)) {
                            *input = LineInput::default();
                        }
                        input.handle_key(code);
                        picking.error = None;
                    }
                }
                picking.fresh = false;
                return Action::Stay;
            }
            match key.code {
                KeyCode::Enter => {
                    picking.time = Some(LineInput::new(&picking.start_time));
                    picking.fresh = !picking.start_time.is_empty();
                    return Action::Stay;
                }
                KeyCode::Char('d') if picking.had => {
                    return Action::SetDeadline { day: picking.day, index: picking.index, deadline: None };
                }
                // Adding items is for browsing, not choosing a deadline.
                KeyCode::Char('a' | 'd') => return Action::Stay,
                _ => {}
            }
        }
        if let Some(input) = &mut self.adding {
            if input.handle_key(key.code) {
                let text = input.text.trim().to_string();
                self.adding = None;
                if !text.is_empty() {
                    return Action::Add(self.cursor, text);
                }
            }
            return Action::Stay;
        }

        // h j k l follow the layout: the week view lists days top to bottom,
        // while the month and year views are grids of weeks.
        // The year view is a grid of months, so there each key stays in the
        // month until its edge, then crosses to the month beside, above or
        // below it.
        let (across, down) = if self.zoom == Zoom::Week { (7, 1) } else { (1, 7) };
        match key.code {
            KeyCode::Char('h') | KeyCode::Left if self.zoom == Zoom::Year => self.move_across_year(false),
            KeyCode::Char('l') | KeyCode::Right if self.zoom == Zoom::Year => self.move_across_year(true),
            KeyCode::Char('k') | KeyCode::Up if self.zoom == Zoom::Year => self.move_down_year(false),
            KeyCode::Char('j') | KeyCode::Down if self.zoom == Zoom::Year => self.move_down_year(true),
            KeyCode::Char('h') | KeyCode::Left => self.move_days(-across),
            KeyCode::Char('l') | KeyCode::Right => self.move_days(across),
            KeyCode::Char('k') | KeyCode::Up => self.move_days(-down),
            KeyCode::Char('j') | KeyCode::Down => self.move_days(down),
            // A whole row of months up or down the year view's grid.
            KeyCode::Char('K') if self.zoom == Zoom::Year => self.move_months(-(self.year_columns.get().max(1) as i32)),
            KeyCode::Char('J') if self.zoom == Zoom::Year => self.move_months(self.year_columns.get().max(1) as i32),
            KeyCode::Char('H') => self.move_months(-1),
            KeyCode::Char('L') => self.move_months(1),
            KeyCode::Char('t') => self.cursor = self.today,
            KeyCode::Char('w') => self.zoom = Zoom::Week,
            KeyCode::Char('m') => self.zoom = Zoom::Month,
            KeyCode::Char('y') => self.zoom = Zoom::Year,
            KeyCode::Char('a') => self.adding = Some(LineInput::default()),
            KeyCode::Char(':') if self.picking.is_none() => self.command = Some(LineInput::default()),
            KeyCode::Char('?') => return Action::Help,
            KeyCode::Enter => return Action::Open(self.cursor),
            KeyCode::Esc | KeyCode::Char('q' | 'c') => return Action::Close,
            _ => {}
        }
        Action::Stay
    }

    fn move_days(&mut self, days: i64) {
        let moved = if days < 0 {
            self.cursor.checked_sub_days(Days::new(days.unsigned_abs()))
        } else {
            self.cursor.checked_add_days(Days::new(days as u64))
        };
        self.cursor = moved.unwrap_or(self.cursor);
    }

    /// Starts a game of Snake on the year view, the snake's head on the
    /// selected day.
    fn start_snake(&mut self, seed: u64) {
        self.zoom = Zoom::Year;
        self.snake = Some(Snake::new(self.cursor, seed));
    }

    /// The game's keys: hjkl and the arrows turn, `Enter` plays again once
    /// it's over, `Esc` ends it. Nothing else does anything.
    fn snake_key(&mut self, code: KeyCode) {
        let Some(game) = &mut self.snake else { return };
        let dir = match code {
            KeyCode::Char('h') | KeyCode::Left => Dir::Left,
            KeyCode::Char('l') | KeyCode::Right => Dir::Right,
            KeyCode::Char('k') | KeyCode::Up => Dir::Up,
            KeyCode::Char('j') | KeyCode::Down => Dir::Down,
            KeyCode::Esc => {
                self.snake = None;
                return;
            }
            KeyCode::Enter if game.over() => {
                let seed = game.next_seed();
                self.start_snake(seed);
                return;
            }
            _ => return,
        };
        game.turn(dir);
    }

    /// Moves the snake, if a game is on and it's time at `now`.
    pub fn tick(&mut self, now: std::time::Instant) {
        if let Some(game) = &mut self.snake {
            game.tick(now, self.year_columns.get());
        }
    }

    /// Moves along the week's row in the year view, as `step_across` does.
    fn move_across_year(&mut self, forward: bool) {
        self.cursor = step_across(self.cursor, forward).unwrap_or(self.cursor);
    }

    /// Moves down or up the weekday's column in the year view, as
    /// `step_down` does.
    fn move_down_year(&mut self, forward: bool) {
        self.cursor = step_down(self.cursor, forward, self.year_columns.get()).unwrap_or(self.cursor);
    }

    /// Moves to the same day of another month, or its last day if shorter.
    fn move_months(&mut self, months: i32) {
        let moved = if months < 0 {
            self.cursor.checked_sub_months(Months::new(months.unsigned_abs()))
        } else {
            self.cursor.checked_add_months(Months::new(months as u32))
        };
        self.cursor = moved.unwrap_or(self.cursor);
    }
}

impl Picking {
    /// Sets the deadline, or the reminder, to the day and time chosen.
    fn chosen(&self, on: Deadline) -> Action {
        let (day, index) = (self.day, self.index);
        if self.reminder {
            Action::SetReminder { day, index, reminder: Reminder::On(on) }
        } else {
            Action::SetDeadline { day, index, deadline: Some(on) }
        }
    }
}

/// The next (or previous) day along `date`'s week row in its month, in the
/// year view. From the row's end it's on the same row of the next (or
/// previous) month, as near as that month has, on the side it came in from.
pub fn step_across(date: NaiveDate, forward: bool) -> Option<NaiveDate> {
    let step = if forward { date.succ_opt() } else { date.pred_opt() };
    if let Some(next) = step
        && next.month() == date.month()
        && week_start(next) == week_start(date)
    {
        return Some(next);
    }
    let first = date.with_day(1).expect("every month has a 1st");
    let row = month_weeks(date).iter().position(|&monday| monday == week_start(date)).unwrap_or(0);
    let other = if forward { first.checked_add_months(Months::new(1)) } else { first.checked_sub_months(Months::new(1)) }?;
    let weeks = month_weeks(other);
    let monday = weeks[row.min(weeks.len() - 1)];
    // Days of the row inside the month: coming from the left, the first;
    // from the right, the last.
    let mut days = (0..7).filter_map(|i| monday.checked_add_days(Days::new(i))).filter(|day| day.month() == other.month());
    if forward { days.next() } else { days.next_back() }
}

/// A week down (or up) `date`'s weekday column in its month, in the year
/// view. From the column's end it's in the month below (or above) on
/// screen, `columns` months on, on the same weekday nearest the side it came
/// in from. Every month has each weekday, so there's always one.
pub fn step_down(date: NaiveDate, forward: bool, columns: u32) -> Option<NaiveDate> {
    let step = if forward { date.checked_add_days(Days::new(7)) } else { date.checked_sub_days(Days::new(7)) };
    if let Some(next) = step
        && next.month() == date.month()
    {
        return Some(next);
    }
    let first = date.with_day(1).expect("every month has a 1st");
    let months = Months::new(columns.max(1));
    let other = if forward { first.checked_add_months(months) } else { first.checked_sub_months(months) }?;
    // The weekday's first week in the month, or from below, its last.
    let mut days = (0..35)
        .filter_map(|i| other.checked_add_days(Days::new(i)))
        .filter(|day| day.month() == other.month() && day.weekday() == date.weekday());
    if forward { days.next() } else { days.next_back() }
}

/// The Monday of `date`'s week.
pub fn week_start(date: NaiveDate) -> NaiveDate {
    date - Days::new(date.weekday().num_days_from_monday().into())
}

/// The Mondays of every week that has a day in `date`'s month.
pub fn month_weeks(date: NaiveDate) -> Vec<NaiveDate> {
    let first = date.with_day(1).expect("every month has a 1st");
    let last = first + Months::new(1) - Days::new(1);
    let mut weeks = vec![week_start(first)];
    while let Some(next) = weeks.last().and_then(|monday| monday.checked_add_days(Days::new(7)))
        && next <= last
    {
        weeks.push(next);
    }
    weeks
}

#[cfg(test)]
mod tests {
    use ratatui::crossterm::event::KeyModifiers;

    use super::*;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    /// A calendar on 2026-10-05 (a Monday), which is also today.
    fn calendar() -> Calendar {
        Calendar::new(date(2026, 10, 5), date(2026, 10, 5))
    }

    /// Sends each character of `keys`, with `<esc>` and `<cr>` for Escape and Enter.
    fn send(cal: &mut Calendar, keys: &str) -> Action {
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
            action = cal.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
            rest = &rest[len..];
        }
        action
    }

    #[test]
    fn typing_colon_snake_starts_a_game_on_the_year_view() {
        let mut cal = calendar();
        send(&mut cal, ":snake");
        assert!(cal.snake.is_none(), "not until Enter");
        assert_eq!(cal.command.as_ref().unwrap().text, "snake");
        send(&mut cal, "<cr>");
        assert!(cal.command.is_none());
        assert_eq!(cal.zoom, Zoom::Year);
        let game = cal.snake.as_ref().unwrap();
        assert_eq!(game.head(), date(2026, 10, 5));
        assert_eq!(game.state, snake::State::Ready);
    }

    #[test]
    fn other_commands_do_nothing() {
        let mut cal = calendar();
        // Each command's keys are typed, not run: "q" and "y" don't close
        // or change the view.
        assert_eq!(send(&mut cal, ":quiz<cr>"), Action::Stay);
        assert!(cal.snake.is_none());
        assert_eq!(cal.zoom, Zoom::Month);
        send(&mut cal, ":snake<esc>");
        assert!(cal.command.is_none() && cal.snake.is_none());
        // Backspace on nothing closes the line, as in vim.
        send(&mut cal, ":");
        cal.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert!(cal.command.is_none());
        assert_eq!(cal.cursor, date(2026, 10, 5));
    }

    #[test]
    fn theres_no_command_line_while_choosing_a_deadline() {
        let mut cal = picker(None);
        send(&mut cal, ":snake<cr>");
        assert!(cal.command.is_none() && cal.snake.is_none());
    }

    #[test]
    fn the_game_keeps_the_calendars_keys_away_until_esc() {
        let mut cal = calendar();
        send(&mut cal, ":snake<cr>");
        // None of these close, open a day, add, help, change the view or move the cursor.
        for keys in ["q", "c", "<cr>", "a", "?", "t", "m", "w", "H", "L", "J", "K", ":"] {
            assert_eq!(send(&mut cal, keys), Action::Stay, "{keys}");
        }
        assert!(cal.adding.is_none() && cal.command.is_none());
        assert_eq!(cal.zoom, Zoom::Year);
        assert_eq!(cal.cursor, date(2026, 10, 5));
        // hjkl turn the snake instead of moving the cursor.
        send(&mut cal, "j");
        assert_eq!(cal.snake.as_ref().unwrap().state, snake::State::Playing);
        assert_eq!(cal.cursor, date(2026, 10, 5));
        // Esc ends the game, back on the year view where it was.
        assert_eq!(send(&mut cal, "<esc>"), Action::Stay);
        assert!(cal.snake.is_none());
        assert_eq!(cal.zoom, Zoom::Year);
        assert_eq!(cal.cursor, date(2026, 10, 5));
        assert_eq!(send(&mut cal, "<esc>"), Action::Close);
    }

    #[test]
    fn the_snake_moves_on_the_calendars_tick_with_its_columns() {
        let start = std::time::Instant::now();
        let mut cal = calendar();
        cal.year_columns.set(4);
        send(&mut cal, ":snake<cr>j");
        cal.tick(start);
        cal.tick(start + std::time::Duration::from_secs(1));
        assert_eq!(cal.snake.as_ref().unwrap().head(), date(2026, 10, 12));
        cal.tick(start + std::time::Duration::from_secs(2));
        cal.tick(start + std::time::Duration::from_secs(3));
        cal.tick(start + std::time::Duration::from_secs(4));
        // 19, 26, then below October with four across is February 2027: a wall.
        assert_eq!(cal.snake.as_ref().unwrap().state, snake::State::Lost);
    }

    #[test]
    fn enter_plays_again_once_the_game_is_over() {
        let mut cal = calendar();
        cal.cursor = date(2026, 12, 31);
        send(&mut cal, ":snake<cr>l");
        cal.snake.as_mut().unwrap().step(3);
        assert_eq!(cal.snake.as_ref().unwrap().state, snake::State::Lost);
        send(&mut cal, "<cr>");
        let game = cal.snake.as_ref().unwrap();
        assert_eq!(game.state, snake::State::Ready);
        assert_eq!(game.score, 0);
        assert_eq!(game.head(), date(2026, 12, 31));
    }

    #[test]
    fn opens_on_the_month_view() {
        assert_eq!(calendar().zoom, Zoom::Month);
    }

    #[test]
    fn hjkl_move_by_day_and_week_in_the_month_and_year_views() {
        let mut cal = calendar();
        send(&mut cal, "l");
        assert_eq!(cal.cursor, date(2026, 10, 6));
        send(&mut cal, "j");
        assert_eq!(cal.cursor, date(2026, 10, 13));
        send(&mut cal, "y");
        send(&mut cal, "hk");
        assert_eq!(cal.cursor, date(2026, 10, 5));
    }

    #[test]
    fn h_and_l_in_the_year_view_stay_on_the_week_row_and_cross_months() {
        let mut cal = calendar();
        send(&mut cal, "y");
        // October 2026's second row runs Mon 5 to Sun 11.
        send(&mut cal, "l");
        assert_eq!(cal.cursor, date(2026, 10, 6));
        send(&mut cal, "hh");
        // From Monday 5, h goes to September's second row (7 to 13), at its right end.
        assert_eq!(cal.cursor, date(2026, 9, 13));
        send(&mut cal, "l");
        // And l from its right end comes back to October's second row, at its left.
        assert_eq!(cal.cursor, date(2026, 10, 5));
    }

    #[test]
    fn year_rows_that_start_or_end_mid_week_land_inside_the_month() {
        let mut cal = calendar();
        send(&mut cal, "y");
        // October 2026 starts on a Thursday, so its first row is 1 to 4.
        cal.cursor = date(2026, 10, 1);
        send(&mut cal, "h");
        // September's first row is Tue 1 to Sun 6.
        assert_eq!(cal.cursor, date(2026, 9, 6));
        send(&mut cal, "l");
        assert_eq!(cal.cursor, date(2026, 10, 1));
        // November's first row is just Sun 1, so l from Sun 4 lands there.
        cal.cursor = date(2026, 10, 4);
        send(&mut cal, "l");
        assert_eq!(cal.cursor, date(2026, 11, 1));
        // November has six rows and October five, so h from the sixth
        // (just Mon 30) goes to October's last row, at Sat 31.
        cal.cursor = date(2026, 11, 30);
        send(&mut cal, "h");
        assert_eq!(cal.cursor, date(2026, 10, 31));
        // Across the end of the year, and back.
        cal.cursor = date(2026, 12, 31);
        send(&mut cal, "l");
        assert_eq!(cal.cursor, date(2027, 1, 25));
        send(&mut cal, "h");
        assert_eq!(cal.cursor, date(2026, 12, 31));
    }

    #[test]
    fn j_and_k_in_the_year_view_stay_in_the_month_and_cross_to_the_one_below() {
        let mut cal = calendar();
        send(&mut cal, "y");
        cal.year_columns.set(3);
        // Monday 5 October: j goes down October's Mondays, 12, 19, 26.
        send(&mut cal, "jjj");
        assert_eq!(cal.cursor, date(2026, 10, 26));
        // Below October, three months on, is January 2027: its first Monday.
        send(&mut cal, "j");
        assert_eq!(cal.cursor, date(2027, 1, 4));
        // And k comes back up to October's last Monday.
        send(&mut cal, "k");
        assert_eq!(cal.cursor, date(2026, 10, 26));
        // Up from October's first Monday is July's last, 27.
        cal.cursor = date(2026, 10, 5);
        send(&mut cal, "k");
        assert_eq!(cal.cursor, date(2026, 7, 27));
    }

    #[test]
    fn j_and_k_in_the_year_view_follow_the_columns_drawn() {
        let mut cal = calendar();
        send(&mut cal, "y");
        // One month across: below October is November. Sat 31 to Sat 7.
        cal.cursor = date(2026, 10, 31);
        send(&mut cal, "j");
        assert_eq!(cal.cursor, date(2026, 11, 7));
        // Four across: above Thursday 1 October is June's last Thursday, 25.
        cal.year_columns.set(4);
        cal.cursor = date(2026, 10, 1);
        send(&mut cal, "k");
        assert_eq!(cal.cursor, date(2026, 6, 25));
    }

    #[test]
    fn capital_j_and_k_in_the_year_view_move_to_the_month_above_or_below() {
        let mut cal = calendar();
        send(&mut cal, "y");
        cal.year_columns.set(3);
        cal.cursor = date(2026, 10, 31);
        // Below October is January: the same day, 31.
        send(&mut cal, "J");
        assert_eq!(cal.cursor, date(2027, 1, 31));
        // Above January is October; above October, July.
        send(&mut cal, "KK");
        assert_eq!(cal.cursor, date(2026, 7, 31));
        // Above July is April, which has no 31st, so its last day.
        send(&mut cal, "K");
        assert_eq!(cal.cursor, date(2026, 4, 30));
        // With one month across, J is the next month, like L.
        cal.year_columns.set(1);
        send(&mut cal, "J");
        assert_eq!(cal.cursor, date(2026, 5, 30));
    }

    #[test]
    fn capital_j_and_k_do_nothing_in_the_month_and_week_views() {
        let mut cal = calendar();
        cal.year_columns.set(3);
        send(&mut cal, "JK");
        assert_eq!(cal.cursor, date(2026, 10, 5));
        send(&mut cal, "wJK");
        assert_eq!(cal.cursor, date(2026, 10, 5));
    }

    #[test]
    fn j_and_k_in_the_month_view_still_move_a_week_across_months() {
        let mut cal = calendar();
        cal.year_columns.set(3);
        cal.cursor = date(2026, 10, 26);
        send(&mut cal, "j");
        assert_eq!(cal.cursor, date(2026, 11, 2));
    }

    #[test]
    fn h_and_l_in_the_month_view_still_move_a_day_across_weeks() {
        let mut cal = calendar();
        send(&mut cal, "h");
        assert_eq!(cal.cursor, date(2026, 10, 4));
    }

    #[test]
    fn the_week_view_moves_by_day_down_the_list_and_by_week_across() {
        let mut cal = calendar();
        send(&mut cal, "w");
        send(&mut cal, "j");
        assert_eq!(cal.cursor, date(2026, 10, 6));
        send(&mut cal, "l");
        assert_eq!(cal.cursor, date(2026, 10, 13));
        send(&mut cal, "hk");
        assert_eq!(cal.cursor, date(2026, 10, 5));
    }

    #[test]
    fn capital_h_and_l_move_by_month_keeping_the_day_where_they_can() {
        let mut cal = Calendar::new(date(2027, 1, 31), date(2026, 10, 5));
        send(&mut cal, "L");
        assert_eq!(cal.cursor, date(2027, 2, 28));
        send(&mut cal, "H");
        assert_eq!(cal.cursor, date(2027, 1, 28));
        send(&mut cal, "HHHH");
        assert_eq!(cal.cursor, date(2026, 9, 28));
    }

    #[test]
    fn t_jumps_to_today() {
        let mut cal = Calendar::new(date(2027, 3, 14), date(2026, 10, 5));
        send(&mut cal, "t");
        assert_eq!(cal.cursor, date(2026, 10, 5));
    }

    #[test]
    fn w_m_and_y_switch_the_view() {
        let mut cal = calendar();
        send(&mut cal, "w");
        assert_eq!(cal.zoom, Zoom::Week);
        send(&mut cal, "y");
        assert_eq!(cal.zoom, Zoom::Year);
        send(&mut cal, "m");
        assert_eq!(cal.zoom, Zoom::Month);
    }

    #[test]
    fn a_adds_an_item_to_the_selected_day() {
        let mut cal = calendar();
        send(&mut cal, "ll");
        assert_eq!(send(&mut cal, "a"), Action::Stay);
        assert!(cal.adding.is_some());
        assert_eq!(send(&mut cal, "Dentist 3pm<cr>"), Action::Add(date(2026, 10, 7), "Dentist 3pm".into()));
        assert!(cal.adding.is_none());
    }

    #[test]
    fn esc_also_saves_like_the_list_does() {
        let mut cal = calendar();
        assert_eq!(send(&mut cal, "a  Call mom  <esc>"), Action::Add(date(2026, 10, 5), "Call mom".into()));
    }

    #[test]
    fn adding_nothing_adds_nothing() {
        let mut cal = calendar();
        assert_eq!(send(&mut cal, "a   <cr>"), Action::Stay);
        assert!(cal.adding.is_none());
    }

    #[test]
    fn keys_typed_while_adding_are_text() {
        let mut cal = calendar();
        send(&mut cal, "ahjklqcwy?");
        assert_eq!(cal.cursor, date(2026, 10, 5));
        assert_eq!(cal.zoom, Zoom::Month);
        assert_eq!(cal.adding.as_ref().unwrap().text, "hjklqcwy?");
    }

    #[test]
    fn enter_opens_the_selected_day() {
        let mut cal = calendar();
        assert_eq!(send(&mut cal, "j<cr>"), Action::Open(date(2026, 10, 12)));
    }

    #[test]
    fn esc_q_and_c_close_and_question_mark_asks_for_help() {
        assert_eq!(send(&mut calendar(), "<esc>"), Action::Close);
        assert_eq!(send(&mut calendar(), "q"), Action::Close);
        assert_eq!(send(&mut calendar(), "c"), Action::Close);
        assert_eq!(send(&mut calendar(), "?"), Action::Help);
    }

    #[test]
    fn weeks_start_on_monday() {
        assert_eq!(week_start(date(2026, 10, 5)), date(2026, 10, 5));
        assert_eq!(week_start(date(2026, 10, 11)), date(2026, 10, 5));
        assert_eq!(week_start(date(2026, 10, 1)), date(2026, 9, 28));
    }

    #[test]
    fn month_weeks_cover_the_whole_month() {
        // October 2026 starts on a Thursday and ends on a Saturday.
        assert_eq!(
            month_weeks(date(2026, 10, 20)),
            [date(2026, 9, 28), date(2026, 10, 5), date(2026, 10, 12), date(2026, 10, 19), date(2026, 10, 26)]
        );
        // February 2027 starts on a Monday and is exactly four weeks.
        assert_eq!(month_weeks(date(2027, 2, 1)).len(), 4);
        // August 2026 starts on a Saturday and needs six rows.
        assert_eq!(month_weeks(date(2026, 8, 1)).len(), 6);
    }

    fn time(h: u32, m: u32) -> chrono::NaiveTime {
        chrono::NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    /// Choosing a deadline for the item stored first on 2026-10-05.
    fn picker(current: Option<Deadline>) -> Calendar {
        Calendar::for_deadline((date(2026, 10, 5), 0), "Report", current, date(2026, 10, 5), date(2026, 10, 5), false)
    }

    fn set(deadline: Option<Deadline>) -> Action {
        Action::SetDeadline { day: date(2026, 10, 5), index: 0, deadline }
    }

    #[test]
    fn choosing_a_deadline_starts_in_the_year_view() {
        let cal = picker(None);
        assert_eq!(cal.zoom, Zoom::Year);
        assert_eq!(cal.cursor, date(2026, 10, 5));
        // On the deadline it has, if any.
        let cal = picker(Some(Deadline { date: date(2026, 12, 1), time: None }));
        assert_eq!(cal.cursor, date(2026, 12, 1));
    }

    #[test]
    fn enter_twice_sets_a_deadline_with_no_time() {
        let mut cal = picker(None);
        assert_eq!(send(&mut cal, "lll<cr>"), Action::Stay);
        assert!(cal.picking.as_ref().unwrap().time.is_some(), "asks for a time");
        assert_eq!(send(&mut cal, "<cr>"), set(Some(Deadline { date: date(2026, 10, 8), time: None })));
    }

    #[test]
    fn a_typed_time_goes_with_the_day() {
        for (typed, expected) in [("13:00", time(13, 0)), ("1300", time(13, 0)), ("12pm", time(12, 0)), ("11AM", time(11, 0))] {
            let mut cal = picker(None);
            send(&mut cal, "j<cr>");
            assert_eq!(
                send(&mut cal, &format!("{typed}<cr>")),
                set(Some(Deadline { date: date(2026, 10, 12), time: Some(expected) })),
                "{typed}"
            );
        }
    }

    #[test]
    fn a_time_that_isnt_one_says_so_and_waits() {
        let mut cal = picker(None);
        send(&mut cal, "<cr>");
        assert_eq!(send(&mut cal, "25:00<cr>"), Action::Stay);
        assert!(cal.picking.as_ref().unwrap().error.is_some());
        // Typing clears the message; a good time then saves.
        for _ in 0..5 {
            cal.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
            assert!(cal.picking.as_ref().unwrap().error.is_none());
        }
        assert_eq!(send(&mut cal, "9<cr>"), set(Some(Deadline { date: date(2026, 10, 5), time: Some(time(9, 0)) })));
    }

    #[test]
    fn esc_while_typing_a_time_goes_back_to_choosing_the_day() {
        let mut cal = picker(None);
        send(&mut cal, "<cr>13<esc>");
        assert!(cal.picking.as_ref().unwrap().time.is_none());
        assert_eq!(send(&mut cal, "l<cr><cr>"), set(Some(Deadline { date: date(2026, 10, 6), time: None })));
        // And Esc while choosing the day cancels.
        assert_eq!(send(&mut picker(None), "<esc>"), Action::Close);
    }

    #[test]
    fn changing_a_deadline_starts_with_its_time_and_d_removes_it() {
        let current = Deadline { date: date(2026, 10, 9), time: Some(time(17, 30)) };
        let mut cal = picker(Some(current));
        send(&mut cal, "<cr>");
        assert_eq!(cal.picking.as_ref().unwrap().time.as_ref().unwrap().text, "17:30");
        // Enter keeps it.
        assert_eq!(send(&mut cal, "<cr>"), set(Some(current)));
        assert_eq!(send(&mut picker(Some(current)), "d"), set(None));
        // Typing replaces it rather than adding to it.
        let mut cal = picker(Some(current));
        assert_eq!(send(&mut cal, "<cr>9<cr>"), set(Some(Deadline { time: Some(time(9, 0)), ..current })));
        // But it can be edited: Backspace takes off the last character.
        let mut cal = picker(Some(current));
        send(&mut cal, "<cr>");
        cal.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(send(&mut cal, "5<cr>"), set(Some(Deadline { time: Some(time(17, 35)), ..current })));
        // And emptied, for no time.
        let mut cal = picker(Some(current));
        send(&mut cal, "<cr>");
        for _ in 0..5 {
            cal.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        }
        assert_eq!(send(&mut cal, "<cr>"), set(Some(Deadline { time: None, ..current })));
    }

    #[test]
    fn choosing_a_deadline_doesnt_add_items_or_open_days() {
        let mut cal = picker(None);
        assert_eq!(send(&mut cal, "a"), Action::Stay);
        assert!(cal.adding.is_none());
        // d does nothing without a deadline to remove.
        assert_eq!(send(&mut cal, "d"), Action::Stay);
        // The usual moves and views still work.
        send(&mut cal, "Lm");
        assert_eq!(cal.cursor, date(2026, 11, 5));
        assert_eq!(cal.zoom, Zoom::Month);
    }

    #[test]
    fn times_show_on_the_chosen_clock_when_changing_one() {
        let current = Deadline { date: date(2026, 10, 9), time: Some(time(23, 0)) };
        let mut cal = Calendar::for_deadline((date(2026, 10, 5), 0), "Report", Some(current), date(2026, 10, 5), date(2026, 10, 5), true);
        send(&mut cal, "<cr>");
        assert_eq!(cal.picking.as_ref().unwrap().time.as_ref().unwrap().text, "11PM");
        assert_eq!(send(&mut cal, "<cr>"), set(Some(current)));
    }

    #[test]
    fn choosing_a_day_for_a_reminder_sets_the_reminder_not_the_deadline() {
        let mut cal = Calendar::for_reminder((date(2026, 10, 20), 1), "Gift", None, date(2026, 10, 5), date(2026, 10, 5), false);
        assert_eq!(cal.zoom, Zoom::Year);
        let on = Deadline { date: date(2026, 10, 7), time: Some(time(9, 0)) };
        assert_eq!(
            send(&mut cal, "ll<cr>9<cr>"),
            Action::SetReminder { day: date(2026, 10, 20), index: 1, reminder: Reminder::On(on) }
        );
        // It starts on the reminder's day, with its time, and d removes nothing.
        let mut cal = Calendar::for_reminder((date(2026, 10, 20), 1), "Gift", Some(Reminder::On(on)), date(2026, 10, 5), date(2026, 10, 5), false);
        assert_eq!(cal.cursor, date(2026, 10, 7));
        assert_eq!(send(&mut cal, "d"), Action::Stay);
        send(&mut cal, "<cr>");
        assert_eq!(cal.picking.as_ref().unwrap().time.as_ref().unwrap().text, "09:00");
    }
}
