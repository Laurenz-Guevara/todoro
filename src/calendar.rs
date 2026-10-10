//! The calendar: a week, month or year at a glance, for planning ahead.

use chrono::{Datelike, Days, Months, NaiveDate};
use ratatui::crossterm::event::{KeyCode, KeyEvent};

use crate::deadline::{self, Deadline};
use crate::input::LineInput;

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
    /// Choosing a deadline for an item (`@`), rather than browsing.
    pub picking: Option<Picking>,
}

/// Choosing a deadline: the day with the calendar, then a time.
pub struct Picking {
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
    Help,
}

impl Calendar {
    pub fn new(cursor: NaiveDate, today: NaiveDate) -> Self {
        Self { zoom: Zoom::Month, cursor, today, adding: None, picking: None }
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
        Self { zoom: Zoom::Year, cursor, today, adding: None, picking: Some(picking) }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Action {
        if let Some(picking) = &mut self.picking {
            if let Some(input) = &mut picking.time {
                match key.code {
                    // Back to choosing the day.
                    KeyCode::Esc => {
                        picking.time = None;
                        picking.error = None;
                    }
                    KeyCode::Enter if input.text.trim().is_empty() => {
                        let deadline = Deadline { date: self.cursor, time: None };
                        return Action::SetDeadline { day: picking.day, index: picking.index, deadline: Some(deadline) };
                    }
                    KeyCode::Enter => match deadline::parse_time(&input.text) {
                        Some(time) => {
                            let deadline = Deadline { date: self.cursor, time: Some(time) };
                            return Action::SetDeadline { day: picking.day, index: picking.index, deadline: Some(deadline) };
                        }
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
        let (across, down) = if self.zoom == Zoom::Week { (7, 1) } else { (1, 7) };
        match key.code {
            KeyCode::Char('h') | KeyCode::Left => self.move_days(-across),
            KeyCode::Char('l') | KeyCode::Right => self.move_days(across),
            KeyCode::Char('k') | KeyCode::Up => self.move_days(-down),
            KeyCode::Char('j') | KeyCode::Down => self.move_days(down),
            KeyCode::Char('H') => self.move_months(-1),
            KeyCode::Char('L') => self.move_months(1),
            KeyCode::Char('t') => self.cursor = self.today,
            KeyCode::Char('w') => self.zoom = Zoom::Week,
            KeyCode::Char('m') => self.zoom = Zoom::Month,
            KeyCode::Char('y') => self.zoom = Zoom::Year,
            KeyCode::Char('a') => self.adding = Some(LineInput::default()),
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
}
