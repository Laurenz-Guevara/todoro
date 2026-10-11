//! Times of day for items, and the timeline that shows them: a day from
//! top to bottom, each item with a time as a block from its start to its
//! end, and a line at the current time. Items without a time are all day.
//! The plan view (`Plan`) gives the day's items their times.

use chrono::{NaiveDate, NaiveTime, TimeDelta, Timelike};

use crate::deadline;
use crate::input::LineInput;

/// When an item is planned: a day, and a start and end on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeSlot {
    pub date: NaiveDate,
    pub start: NaiveTime,
    pub end: NaiveTime,
}

impl TimeSlot {
    /// How it's written to disk: `2026-10-11 09:00-10:30`.
    pub fn to_stored(self) -> String {
        format!("{} {}-{}", self.date.format("%Y-%m-%d"), self.start.format("%H:%M"), self.end.format("%H:%M"))
    }

    /// Reads what `to_stored` wrote, or `None` if it isn't a time slot.
    pub fn from_stored(text: &str) -> Option<Self> {
        let (date, times) = text.trim().split_once(' ')?;
        let (start, end) = times.trim().split_once('-')?;
        let slot = Self {
            date: NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()?,
            start: NaiveTime::parse_from_str(start, "%H:%M").ok()?,
            end: NaiveTime::parse_from_str(end, "%H:%M").ok()?,
        };
        (slot.end > slot.start).then_some(slot)
    }

    /// How it's shown: `09:00–10:30`, or `9AM–10:30AM`.
    pub fn describe(self, twelve_hour: bool) -> String {
        format!("{}–{}", deadline::format_time(self.start, twelve_hour), deadline::format_time(self.end, twelve_hour))
    }
}

/// The latest time an end can be, as times stay within one day.
pub fn last_minute() -> NaiveTime {
    NaiveTime::from_hms_opt(23, 59, 0).expect("a valid time")
}

/// `time` without its seconds, as times are planned to the minute.
pub fn to_the_minute(time: NaiveTime) -> NaiveTime {
    NaiveTime::from_hms_opt(time.hour(), time.minute(), 0).expect("a valid time")
}

/// An hour after `start`, or the end of the day if that's sooner.
pub fn an_hour_after(start: NaiveTime) -> NaiveTime {
    let (end, wrapped) = start.overflowing_add_signed(TimeDelta::hours(1));
    if wrapped != 0 { last_minute() } else { end }
}

/// A block on the timeline: which of the given items it is, and the
/// column it's in, of how many, so overlapping blocks sit side by side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placed {
    pub item: usize,
    pub column: usize,
    pub columns: usize,
}

/// Lays out `slots` (an item's index and its time) in columns: each block
/// takes the first column free at its start, and every block in a group of
/// overlapping ones shares that group's number of columns.
pub fn place(slots: &[(usize, TimeSlot)]) -> Vec<Placed> {
    let mut order: Vec<&(usize, TimeSlot)> = slots.iter().collect();
    order.sort_by_key(|(item, slot)| (slot.start, slot.end, *item));
    let mut placed: Vec<Placed> = Vec::new();
    // Each column's end, for the group of overlapping blocks being laid out.
    let mut column_ends: Vec<NaiveTime> = Vec::new();
    let mut group_start = 0;
    for (item, slot) in order {
        if column_ends.iter().all(|end| *end <= slot.start) {
            // Nothing still running: the group so far is finished.
            let columns = column_ends.len().max(1);
            for done in &mut placed[group_start..] {
                done.columns = columns;
            }
            group_start = placed.len();
            column_ends.clear();
        }
        let column = match column_ends.iter().position(|end| *end <= slot.start) {
            Some(free) => {
                column_ends[free] = slot.end;
                free
            }
            None => {
                column_ends.push(slot.end);
                column_ends.len() - 1
            }
        };
        placed.push(Placed { item: *item, column, columns: 0 });
    }
    let columns = column_ends.len().max(1);
    for done in &mut placed[group_start..] {
        done.columns = columns;
    }
    placed
}

/// The hours a timeline shows, from the first to the last (the end of the
/// last hour): the working day, widened to take in every slot and `now`.
pub fn hours(slots: &[TimeSlot], now: Option<NaiveTime>) -> (u32, u32) {
    let mut first = 7;
    let mut last = 22;
    for slot in slots {
        first = first.min(slot.start.hour());
        last = last.max(slot.end.hour() + u32::from(slot.end.minute() > 0));
    }
    if let Some(now) = now {
        first = first.min(now.hour());
        last = last.max(now.hour() + 1);
    }
    (first, last.min(24))
}

/// What's being typed in the plan view.
#[derive(Debug)]
pub enum Typing {
    Start(LineInput),
    /// The end, once the start is typed.
    End(NaiveTime, LineInput),
}

/// The plan view, giving the day's items their times. Changes go straight
/// into the store, so the list and the timeline show them; saving keeps
/// them as one change to undo, and leaving without saving puts back how
/// things were when it opened.
#[derive(Debug, Default)]
pub struct Plan {
    pub typing: Option<Typing>,
    /// Why the time typed can't be used.
    pub error: Option<String>,
    /// Whether anything changed since it opened (or was reset).
    pub changed: bool,
    /// Asking whether to throw the changes away, after Esc.
    pub confirm_cancel: bool,
    /// Whether keys move through the timeline (`L`) rather than the list.
    pub on_timeline: bool,
    /// Where the last time set ended, for the next to start at, so items
    /// can be planned one after another.
    pub last_end: Option<NaiveTime>,
}

impl Plan {
    /// When a start left empty begins: as the last time set ends, or now.
    pub fn default_start(&self, now: NaiveTime) -> NaiveTime {
        self.last_end.unwrap_or_else(|| to_the_minute(now))
    }
}

/// What a typed time means: a time, nothing (to skip or take the default),
/// or a mistake.
pub fn read_time(text: &str) -> Result<Option<NaiveTime>, String> {
    if text.trim().is_empty() {
        return Ok(None);
    }
    deadline::parse_time(text).map(Some).ok_or_else(|| "Type a time like 9, 9:30, 13:00, 1pm or 11:30am".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::today;

    fn at(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    fn slot(start: (u32, u32), end: (u32, u32)) -> TimeSlot {
        TimeSlot { date: today(), start: at(start.0, start.1), end: at(end.0, end.1) }
    }

    #[test]
    fn slots_are_stored_as_text_and_read_back() {
        let s = slot((9, 0), (10, 30));
        assert_eq!(s.to_stored(), "2026-10-05 09:00-10:30");
        assert_eq!(TimeSlot::from_stored("2026-10-05 09:00-10:30"), Some(s));
        for broken in ["", "2026-10-05", "2026-10-05 09:00", "2026-10-05 10:00-09:00", "2026-10-05 09:00-09:00", "soon"] {
            assert_eq!(TimeSlot::from_stored(broken), None, "{broken}");
        }
    }

    #[test]
    fn slots_are_described_on_either_clock() {
        assert_eq!(slot((9, 0), (10, 30)).describe(false), "09:00–10:30");
        assert_eq!(slot((13, 0), (14, 0)).describe(true), "1PM–2PM");
    }

    #[test]
    fn an_hour_after_stops_at_the_end_of_the_day() {
        assert_eq!(an_hour_after(at(9, 15)), at(10, 15));
        assert_eq!(an_hour_after(at(23, 30)), at(23, 59));
    }

    #[test]
    fn overlapping_blocks_share_columns_and_the_rest_get_the_width() {
        let slots = [
            (0, slot((9, 0), (10, 0))),
            (1, slot((9, 30), (11, 0))),
            (2, slot((10, 0), (10, 30))),
            (3, slot((12, 0), (13, 0))),
        ];
        let placed = place(&slots);
        let find = |item| placed.iter().find(|p| p.item == item).copied().unwrap();
        assert_eq!(find(0), Placed { item: 0, column: 0, columns: 2 });
        assert_eq!(find(1), Placed { item: 1, column: 1, columns: 2 });
        // 10:00 starts as 9:00's block ends, so it takes that column.
        assert_eq!(find(2), Placed { item: 2, column: 0, columns: 2 });
        assert_eq!(find(3), Placed { item: 3, column: 0, columns: 1 });
    }

    #[test]
    fn the_hours_shown_take_in_every_slot_and_now() {
        assert_eq!(hours(&[], None), (7, 22));
        assert_eq!(hours(&[slot((6, 30), (8, 0))], None), (6, 22));
        assert_eq!(hours(&[slot((21, 0), (23, 15))], None), (7, 24));
        assert_eq!(hours(&[], Some(at(23, 10))), (7, 24));
        assert_eq!(hours(&[], Some(at(5, 0))), (5, 22));
    }

    #[test]
    fn typed_times_are_read_on_either_clock() {
        assert_eq!(read_time("9"), Ok(Some(at(9, 0))));
        assert_eq!(read_time("1:30pm"), Ok(Some(at(13, 30))));
        assert_eq!(read_time("  "), Ok(None));
        assert!(read_time("lunch").is_err());
    }
}
