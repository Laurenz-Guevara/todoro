//! Today's list from the command line: `todoro --list` prints it as plain
//! text, for the terminal or scripts, and `todoro --add` adds to it.

use std::io;

use chrono::{NaiveDate, NaiveTime};

use crate::outline;
use crate::store::{Item, Store};
use crate::ui::{DEADLINE_MARKER, NOTES_MARKER, PINNED_MARKER, REMINDER_MARKER, SUB_DONE, SUB_OPEN, priority_icon};

/// Where an item is stored: its day and its index there.
type Stored = (NaiveDate, usize);

/// Where today's top-level items are stored, in the order todoro shows
/// them: the open ones (numbered from 1 on screen) and the completed ones.
/// Pinned and overdue items from earlier days come first, then items whose
/// reminder is due, then the day's own.
fn roots(store: &Store, today: NaiveDate, now: NaiveTime) -> (Vec<Stored>, Vec<Stored>) {
    let mut roots = store.carried_to(today, today);
    roots.extend(store.reminded_on(today, today, now));
    let items = store.items(today);
    let mut at = 0;
    while at < items.len() {
        roots.push((today, at));
        at = outline::block_end(items, at);
    }
    roots.into_iter().partition(|&(day, index)| !store.items(day)[index].done)
}

/// `todoro -a TEXT`: adds an item to the end of today's open items, or with
/// `parent`, the number of an open item as `todoro -l` shows it, as that
/// item's last sub-item (showing it, if its sub-items were folded away).
/// New top-level items are pinned if `pin` (the "Pin new items" option), as
/// in todoro. Returns what to print, or why it couldn't.
pub fn add(store: &mut Store, today: NaiveDate, now: NaiveTime, text: &str, parent: Option<usize>, pin: bool) -> io::Result<Result<String, String>> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(Err("Give the item's text, like: todoro -a Buy milk".into()));
    }
    let Some(number) = parent else {
        let index = store.open_count(today);
        store.insert_at(today, index, vec![Item { text: text.into(), pinned: pin, ..Item::default() }], 0)?;
        let number = roots(store, today, now).0.len();
        return Ok(Ok(format!("Added {number}. {text}")));
    };
    let (open, _) = roots(store, today, now);
    let Some(&(day, index)) = number.checked_sub(1).and_then(|at| open.get(at)) else {
        let there = match open.len() {
            0 => "There are no open items on today's list.".to_string(),
            1 => "Only item 1 is open on today's list.".to_string(),
            n => format!("Today's open items are 1 to {n}."),
        };
        return Ok(Err(format!("There's no item {number} to add under. {there} todoro -l lists them.")));
    };
    let parent = store.items(day)[index].clone();
    if parent.folded {
        store.set_folded(day, index, false)?;
    }
    let end = outline::block_end(store.items(day), index);
    store.insert_at(day, end, vec![Item { text: text.into(), ..Item::default() }], parent.depth + 1)?;
    Ok(Ok(format!("Added under {number}. {}: {text}", parent.text)))
}

/// What todoro shows on `today` at `now`, as text: the date (and the
/// workspace, if there is one), then the open items numbered as on screen
/// (pinned and overdue items from earlier days first, then items whose
/// reminder is due), each with all of its sub-items, then the completed ones.
/// Reading only: nothing moves, as it would when todoro opens.
pub fn today(store: &Store, today: NaiveDate, now: NaiveTime, twelve_hour: bool, workspace: Option<&str>) -> String {
    let mut out = today.format("%A %-d %B %Y").to_string();
    if let Some(workspace) = workspace {
        out.push_str(&format!(" · {workspace}"));
    }
    out.push('\n');

    let (open, done) = roots(store, today, now);

    if open.is_empty() && done.is_empty() {
        out.push_str("\nNothing on today's list.\n");
        return out;
    }
    let line = |item: &Item, label: String, out: &mut String| {
        let mut text = label;
        if let Some(priority) = item.priority {
            text.push_str(&format!("{} ", priority_icon(priority)));
        }
        text.push_str(&item.text);
        if let Some(deadline) = item.deadline {
            text.push_str(&format!("{DEADLINE_MARKER}{}", deadline.describe(today, twelve_hour)));
        } else if item.pinned {
            text.push_str(PINNED_MARKER);
        }
        if item.reminder.is_some() {
            text.push_str(&format!(" {REMINDER_MARKER}"));
        }
        if !item.notes.trim().is_empty() {
            text.push_str(NOTES_MARKER);
        }
        out.push_str(&text);
        out.push('\n');
    };
    // An item, then every sub-item under it, folded or not, indented under
    // its text by depth.
    let block = |(day, index): (NaiveDate, usize), label: String, indent: usize, out: &mut String| {
        let items = store.items(day);
        line(&items[index], label, out);
        for sub in &items[index + 1..outline::block_end(items, index)] {
            let bullet = if sub.done { SUB_DONE } else { SUB_OPEN };
            let pad = " ".repeat(indent + 2 * (sub.depth - 1));
            line(sub, format!("{pad}{bullet} "), out);
        }
    };

    if !open.is_empty() {
        out.push('\n');
        // Numbers line up: "1. " to "9. ", then " 1. " to "10. ".
        let width = open.len().to_string().len();
        for (number, &root) in open.iter().enumerate() {
            block(root, format!("{:>width$}. ", number + 1), width + 2, &mut out);
        }
    }
    if !done.is_empty() {
        out.push_str("\nCompleted\n");
        for &root in &done {
            block(root, format!("{SUB_DONE} "), 2, &mut out);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use chrono::NaiveTime;

    use super::*;
    use crate::deadline::Deadline;
    use crate::test_util::today as day;

    fn noon() -> NaiveTime {
        NaiveTime::from_hms_opt(12, 0, 0).unwrap()
    }

    fn store(dir: &tempfile::TempDir) -> Store {
        Store::open(dir.path().join("todos.json")).unwrap()
    }

    #[test]
    fn an_empty_day_says_so() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(today(&store(&dir), day(), noon(), false, None), "Monday 5 October 2026\n\nNothing on today's list.\n");
    }

    #[test]
    fn open_items_are_numbered_and_completed_ones_follow() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(&dir);
        for (i, text) in ["Buy milk", "Call mom", "Write the report"].into_iter().enumerate() {
            store.insert(day(), i, text.into()).unwrap();
        }
        store.toggle_done(day(), 1).unwrap();
        assert_eq!(
            today(&store, day(), noon(), false, Some("work")),
            "Monday 5 October 2026 · work\n\n1. Buy milk\n2. Write the report\n\nCompleted\n✓ Call mom\n"
        );
    }

    #[test]
    fn sub_items_are_indented_under_their_item_folded_or_not() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(&dir);
        store.insert(day(), 0, "Plan the trip".into()).unwrap();
        store.insert_at(day(), 1, vec![Item { text: "Book flights".into(), ..Item::default() }], 1).unwrap();
        store.insert_at(day(), 2, vec![Item { text: "Compare prices".into(), ..Item::default() }], 2).unwrap();
        store.insert_at(day(), 3, vec![Item { text: "Pack".into(), ..Item::default() }], 1).unwrap();
        store.toggle_done(day(), 3).unwrap();
        store.set_folded(day(), 0, true).unwrap();
        assert_eq!(
            today(&store, day(), noon(), false, None),
            "Monday 5 October 2026\n\n1. Plan the trip\n   ↳ Book flights\n     ↳ Compare prices\n   ✓ Pack\n"
        );
    }

    #[test]
    fn markers_show_as_in_todoro() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(&dir);
        for (i, text) in ["Pay rent", "Water plants", "Read notes"].into_iter().enumerate() {
            store.insert(day(), i, text.into()).unwrap();
        }
        store.cycle_priority(day(), 0).unwrap();
        let five = NaiveTime::from_hms_opt(17, 0, 0);
        store.set_deadline(day(), 0, Some(Deadline { date: day(), time: five })).unwrap();
        store.toggle_pinned(day(), 1).unwrap();
        store.set_notes(day(), 2, "Chapter 3".into()).unwrap();
        assert_eq!(
            today(&store, day(), noon(), true, None),
            "Monday 5 October 2026\n\n1. ∧ Pay rent ◷ 5PM\n2. Water plants ⚲\n3. Read notes ≡\n"
        );
    }

    #[test]
    fn pinned_and_overdue_items_from_earlier_days_come_first_without_moving() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(&dir);
        let earlier = day().pred_opt().unwrap().pred_opt().unwrap();
        store.insert(earlier, 0, "Renew passport".into()).unwrap();
        store.insert(earlier, 1, "Left behind".into()).unwrap();
        store.toggle_pinned(earlier, 0).unwrap();
        store.insert(day(), 0, "Buy milk".into()).unwrap();
        let listed = today(&store, day(), noon(), false, None);
        assert_eq!(listed, "Monday 5 October 2026\n\n1. Renew passport ⚲\n2. Buy milk\n");
        // Still stored on its own day.
        assert_eq!(store.items(earlier)[0].text, "Renew passport");
    }

    fn texts(store: &Store, day: NaiveDate) -> Vec<(usize, String, bool)> {
        store.items(day).iter().map(|item| (item.depth, item.text.clone(), item.pinned)).collect()
    }

    #[test]
    fn adding_goes_after_the_open_items() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(&dir);
        for (i, text) in ["Buy milk", "Call mom"].into_iter().enumerate() {
            store.insert(day(), i, text.into()).unwrap();
        }
        store.toggle_done(day(), 1).unwrap();
        assert_eq!(add(&mut store, day(), noon(), "  Pay rent ", None, false).unwrap(), Ok("Added 2. Pay rent".into()));
        assert_eq!(texts(&store, day()), [(0, "Buy milk".into(), false), (0, "Pay rent".into(), false), (0, "Call mom".into(), false)]);
        // Saved, like any change.
        let reopened = Store::open(dir.path().join("todos.json")).unwrap();
        assert_eq!(texts(&reopened, day()), texts(&store, day()));
    }

    #[test]
    fn adding_pins_with_the_option_on() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(&dir);
        add(&mut store, day(), noon(), "Renew passport", None, true).unwrap().unwrap();
        assert_eq!(texts(&store, day()), [(0, "Renew passport".into(), true)]);
    }

    #[test]
    fn adding_a_sub_item_under_a_numbered_item() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(&dir);
        for (i, text) in ["Buy milk", "Plan the trip"].into_iter().enumerate() {
            store.insert(day(), i, text.into()).unwrap();
        }
        store.insert_at(day(), 2, vec![Item { text: "Book flights".into(), ..Item::default() }], 1).unwrap();
        store.set_folded(day(), 1, true).unwrap();
        let added = add(&mut store, day(), noon(), "Pack", Some(2), true).unwrap();
        assert_eq!(added, Ok("Added under 2. Plan the trip: Pack".into()));
        // The last sub-item, never pinned, and unfolded to show it.
        assert_eq!(texts(&store, day())[1..], [(0, "Plan the trip".into(), false), (1, "Book flights".into(), false), (1, "Pack".into(), false)]);
        assert!(!store.items(day())[1].folded);
    }

    #[test]
    fn numbers_count_items_carried_from_earlier_days() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(&dir);
        let earlier = day().pred_opt().unwrap();
        store.insert(earlier, 0, "Renew passport".into()).unwrap();
        store.toggle_pinned(earlier, 0).unwrap();
        store.insert(day(), 0, "Buy milk".into()).unwrap();
        // 1 is the pinned item from yesterday, where it's still stored.
        add(&mut store, day(), noon(), "Find the old one", Some(1), false).unwrap().unwrap();
        assert_eq!(texts(&store, earlier)[1], (1, "Find the old one".into(), false));
        assert_eq!(add(&mut store, day(), noon(), "Milk", None, false).unwrap(), Ok("Added 3. Milk".into()));
    }

    #[test]
    fn adding_nothing_or_under_an_item_that_isnt_there_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(&dir);
        let Err(message) = add(&mut store, day(), noon(), "  ", None, false).unwrap() else { panic!("should fail") };
        assert!(message.contains("todoro -a Buy milk"));
        let Err(message) = add(&mut store, day(), noon(), "Pack", Some(1), false).unwrap() else { panic!("should fail") };
        assert_eq!(message, "There's no item 1 to add under. There are no open items on today's list. todoro -l lists them.");
        store.insert(day(), 0, "Buy milk".into()).unwrap();
        store.insert(day(), 1, "Call mom".into()).unwrap();
        let Err(message) = add(&mut store, day(), noon(), "Pack", Some(3), false).unwrap() else { panic!("should fail") };
        assert!(message.contains("open items are 1 to 2"), "{message}");
        let Err(message) = add(&mut store, day(), noon(), "Pack", Some(0), false).unwrap() else { panic!("should fail") };
        assert!(message.contains("no item 0"));
        // Completed items aren't numbered, so nothing goes under them.
        store.toggle_done(day(), 1).unwrap();
        let Err(message) = add(&mut store, day(), noon(), "Pack", Some(2), false).unwrap() else { panic!("should fail") };
        assert!(message.contains("Only item 1 is open"), "{message}");
        assert_eq!(store.items(day()).len(), 2, "nothing added");
    }

    #[test]
    fn numbers_line_up_past_nine() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(&dir);
        for i in 0..10 {
            store.insert(day(), i, format!("Item {}", i + 1)).unwrap();
        }
        let listed = today(&store, day(), noon(), false, None);
        assert!(listed.contains("\n 9. Item 9\n10. Item 10\n"), "{listed}");
    }
}
