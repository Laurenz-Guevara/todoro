//! `todoro --list`: today's list as plain text, for the terminal or scripts.

use chrono::{NaiveDate, NaiveTime};

use crate::outline;
use crate::store::{Item, Store};
use crate::ui::{DEADLINE_MARKER, NOTES_MARKER, PINNED_MARKER, REMINDER_MARKER, SUB_DONE, SUB_OPEN, priority_icon};

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

    let mut roots = store.carried_to(today, today);
    roots.extend(store.reminded_on(today, today, now));
    let items = store.items(today);
    let mut at = 0;
    while at < items.len() {
        roots.push((today, at));
        at = outline::block_end(items, at);
    }
    let (open, done): (Vec<_>, Vec<_>) = roots.into_iter().partition(|&(day, index)| !store.items(day)[index].done);

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
