//! Today's list from the command line: `todoro --list` prints it as plain
//! text, for the terminal or scripts, `todoro --add` adds to it, `todoro
//! --delete` deletes from it, and `todoro --undo` puts back what that
//! deleted.

use std::io;

use chrono::{NaiveDate, NaiveTime};

use crate::export;
use crate::outline;
use crate::store::{Deleted, Item, Store};
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

/// The item at `address` on today's list: `[2]` for open item 2 as
/// `todoro -l` numbers it, `[2, 1]` for its first sub-item, and so on. Or
/// why there isn't one.
fn find(store: &Store, today: NaiveDate, now: NaiveTime, address: &[usize]) -> Result<Stored, String> {
    let (open, _) = roots(store, today, now);
    let Some((&number, subs)) = address.split_first() else { return Err("Give the item's number.".into()) };
    let Some(&(day, mut index)) = number.checked_sub(1).and_then(|at| open.get(at)) else {
        let there = match open.len() {
            0 => "There are no open items on today's list.".to_string(),
            1 => "Only item 1 is open on today's list.".to_string(),
            n => format!("Today's open items are 1 to {n}."),
        };
        return Err(format!("There's no item {number}. {there} todoro -l lists them."));
    };
    let items = store.items(day);
    for (depth, &sub) in subs.iter().enumerate() {
        let children = outline::children(items, index);
        let Some(&child) = sub.checked_sub(1).and_then(|at| children.get(at)) else {
            let parent = address_text(&address[..=depth]);
            return Err(match children.len() {
                0 => format!("Item {parent} has no sub-items."),
                1 => format!("Item {parent} has only one sub-item, {parent}.1."),
                n => format!("Item {parent}'s sub-items are {parent}.1 to {parent}.{n}."),
            });
        };
        index = child;
    }
    Ok((day, index))
}

/// An address as typed: `2.1.3`.
pub fn address_text(address: &[usize]) -> String {
    address.iter().map(usize::to_string).collect::<Vec<_>>().join(".")
}

/// How much goes with the item at `index`: "1 item with 2 sub-items, and 2
/// notes".
fn amount(items: &[Item]) -> String {
    let subs = match items.len() - 1 {
        0 => String::new(),
        1 => " with 1 sub-item".into(),
        n => format!(" with {n} sub-items"),
    };
    let notes = match items.iter().filter(|item| !item.notes.trim().is_empty()).count() {
        0 => "no notes".to_string(),
        1 => "1 note".into(),
        n => format!("{n} notes"),
    };
    format!("1 item{subs}, and {notes}")
}

/// `todoro -d`, before asking: everything the item at `address` takes with
/// it, its sub-items and every note in full, and a line saying how much
/// that is.
pub fn delete_preview(store: &Store, today: NaiveDate, now: NaiveTime, address: &[usize]) -> Result<String, String> {
    let (day, index) = find(store, today, now, address)?;
    let items = store.items(day);
    let mut out = String::from("This will be deleted:\n\n");
    let label = if address.len() == 1 { format!("{}. ", address[0]) } else { format!("{} ", address_text(address)) };
    export::block(items, index, &label, &mut out);
    out.push_str(&format!("\nThat's {}.\n", amount(&items[index..outline::block_end(items, index)])));
    Ok(out)
}

/// `todoro -d`, once confirmed: deletes the item at `address` with its
/// sub-items, keeping them in the file `todoro -u` puts them back from.
pub fn delete(store: &mut Store, today: NaiveDate, now: NaiveTime, address: &[usize]) -> io::Result<Result<String, String>> {
    let (day, index) = match find(store, today, now, address) {
        Ok(found) => found,
        Err(message) => return Ok(Err(message)),
    };
    let removed = store.delete_block(day, index)?;
    std::fs::write(store.deleted_path(), Deleted::new(day, index, &removed).to_json())?;
    Ok(Ok(format!("Deleted {}: {}. todoro -u puts it back.", removed[0].text, amount(&removed))))
}

/// `todoro -u`: puts back the item `todoro -d` deleted last, where it was,
/// with its sub-items and notes. Once only: that's the one kept.
pub fn undo(store: &mut Store) -> io::Result<Result<String, String>> {
    let path = store.deleted_path();
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(Err("Nothing to put back: todoro -u undoes the last todoro -d, and there's none.".into()));
        }
        Err(error) => return Err(error),
    };
    let deleted = Deleted::from_json(&text)?;
    let items = deleted.items();
    let (Some(day), Some(first)) = (deleted.day(), items.first()) else {
        return Ok(Err(format!("{} isn't readable, so there's nothing to put back.", path.display())));
    };
    let message = format!("Put back {}: {}.", first.text, amount(&items));
    store.put_back(day, deleted.index(), deleted.depth(), items)?;
    std::fs::remove_file(&path)?;
    Ok(Ok(message))
}

/// What todoro shows on `today` at `now`, as text: the date (and the
/// workspace, if there is one), then the open items numbered as on screen
/// (pinned and overdue items from earlier days first, then items whose
/// reminder is due), each with all of its sub-items, then the completed ones.
/// Reading only: nothing moves, as it would when todoro opens.
pub fn today(store: &Store, today: NaiveDate, now: NaiveTime, twelve_hour: bool, workspace: Option<&str>) -> String {
    text(store, today, now, twelve_hour, workspace, false)
}

/// Today's open items for `todoro -d` with no number: as `todoro -l` shows
/// them, but with every sub-item numbered among its siblings (`1. ↳`), so
/// its address can be read off the left edge, and without the completed
/// items, which `-d` can't delete.
pub fn numbered(store: &Store, today: NaiveDate, now: NaiveTime, twelve_hour: bool, workspace: Option<&str>) -> String {
    let mut out = text(store, today, now, twelve_hour, workspace, true);
    if out.ends_with(".\n") {
        // Nothing open: the list said so.
        return out;
    }
    out.push_str("\ntodoro -d 2 deletes item 2, and todoro -d 2.1 its first sub-item.\n");
    out
}

fn text(store: &Store, today: NaiveDate, now: NaiveTime, twelve_hour: bool, workspace: Option<&str>, numbered: bool) -> String {
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
    if numbered && open.is_empty() {
        out.push_str("\nNothing open on today's list, so nothing todoro -d can delete.\n");
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
    // its text by depth. With `numbered`, each sub-item starts with its
    // number among its siblings, as `todoro -d 2.1` counts them.
    let block = |(day, index): (NaiveDate, usize), label: String, indent: usize, numbered: bool, out: &mut String| {
        let items = store.items(day);
        let depth = items[index].depth;
        line(&items[index], label, out);
        // The number of the last sub-item seen at each depth below the item.
        let mut counts: Vec<usize> = Vec::new();
        for sub in &items[index + 1..outline::block_end(items, index)] {
            let level = sub.depth - depth;
            counts.truncate(level);
            counts.resize(level, 0);
            counts[level - 1] += 1;
            let bullet = if sub.done { SUB_DONE } else { SUB_OPEN };
            let pad = " ".repeat(indent + 2 * (level - 1));
            let number = if numbered { format!("{}. ", counts[level - 1]) } else { String::new() };
            line(sub, format!("{pad}{number}{bullet} "), out);
        }
    };

    if !open.is_empty() {
        out.push('\n');
        // Numbers line up: "1. " to "9. ", then " 1. " to "10. ".
        let width = open.len().to_string().len();
        for (number, &root) in open.iter().enumerate() {
            block(root, format!("{:>width$}. ", number + 1), width + 2, numbered, &mut out);
        }
    }
    if !done.is_empty() && !numbered {
        out.push_str("\nCompleted\n");
        for &root in &done {
            block(root, format!("{SUB_DONE} "), 2, false, &mut out);
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
    fn the_numbered_list_numbers_sub_items_and_leaves_out_completed_items() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(&dir);
        store.insert(day(), 0, "Plan the trip".into()).unwrap();
        store.insert_at(day(), 1, vec![Item { text: "Book flights".into(), ..Item::default() }], 1).unwrap();
        store.insert_at(day(), 2, vec![Item { text: "Compare prices".into(), ..Item::default() }], 2).unwrap();
        store.insert_at(day(), 3, vec![Item { text: "Pack".into(), ..Item::default() }], 1).unwrap();
        store.toggle_done(day(), 3).unwrap();
        store.insert(day(), 4, "Call mom".into()).unwrap();
        store.toggle_done(day(), 4).unwrap();
        assert_eq!(
            numbered(&store, day(), noon(), false, None),
            "Monday 5 October 2026\n\n1. Plan the trip\n   1. ↳ Book flights\n     1. ↳ Compare prices\n   2. ✓ Pack\n\n\
             todoro -d 2 deletes item 2, and todoro -d 2.1 its first sub-item.\n"
        );
        // The numbers are the addresses -d takes.
        assert_eq!(find(&store, day(), noon(), &[1, 2]), Ok((day(), 3)));
        assert_eq!(find(&store, day(), noon(), &[1, 1, 1]), Ok((day(), 2)));
    }

    #[test]
    fn the_numbered_list_with_nothing_open_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(&dir);
        assert_eq!(numbered(&store, day(), noon(), false, None), "Monday 5 October 2026\n\nNothing on today's list.\n");
        store.insert(day(), 0, "Call mom".into()).unwrap();
        store.toggle_done(day(), 0).unwrap();
        assert_eq!(
            numbered(&store, day(), noon(), false, None),
            "Monday 5 October 2026\n\nNothing open on today's list, so nothing todoro -d can delete.\n"
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

    /// Today: 1. Buy milk; 2. Plan the trip (notes), with Book flights
    /// (notes, and its own Compare prices) and Pack (done); 3. Call mom (done).
    fn trip(dir: &tempfile::TempDir) -> Store {
        let mut store = store(dir);
        for (i, text) in ["Buy milk", "Plan the trip", "Call mom"].into_iter().enumerate() {
            store.insert(day(), i, text.into()).unwrap();
        }
        store.set_notes(day(), 1, "Lisbon, mid-May".into()).unwrap();
        store.insert_at(day(), 2, vec![Item { text: "Book flights".into(), ..Item::default() }], 1).unwrap();
        store.set_notes(day(), 2, "Window seat".into()).unwrap();
        store.insert_at(day(), 3, vec![Item { text: "Compare prices".into(), ..Item::default() }], 2).unwrap();
        store.insert_at(day(), 4, vec![Item { text: "Pack".into(), ..Item::default() }], 1).unwrap();
        store.toggle_done(day(), 4).unwrap();
        store.toggle_done(day(), 5).unwrap();
        store
    }

    /// Every item on the day as (depth, text, done, notes).
    fn whole(store: &Store, day: NaiveDate) -> Vec<(usize, String, bool, String)> {
        store.items(day).iter().map(|item| (item.depth, item.text.clone(), item.done, item.notes.clone())).collect()
    }

    #[test]
    fn the_preview_shows_everything_that_goes() {
        let dir = tempfile::tempdir().unwrap();
        let store = trip(&dir);
        assert_eq!(
            delete_preview(&store, day(), noon(), &[2]).unwrap(),
            "This will be deleted:\n\n2. Plan the trip\n\nLisbon, mid-May\n\n- Book flights\n\n  Window seat\n\n  - Compare prices\n\n- [x] Pack\n\nThat's 1 item with 3 sub-items, and 2 notes.\n"
        );
        assert_eq!(
            delete_preview(&store, day(), noon(), &[2, 1, 1]).unwrap(),
            "This will be deleted:\n\n2.1.1 Compare prices\n\nThat's 1 item, and no notes.\n"
        );
    }

    #[test]
    fn deleting_then_undoing_puts_it_all_back_as_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = trip(&dir);
        let before = whole(&store, day());
        let message = delete(&mut store, day(), noon(), &[2]).unwrap().unwrap();
        assert_eq!(message, "Deleted Plan the trip: 1 item with 3 sub-items, and 2 notes. todoro -u puts it back.");
        assert_eq!(whole(&store, day()).iter().map(|item| item.1.as_str()).collect::<Vec<_>>(), ["Buy milk", "Call mom"]);
        // Its notes files go, as when deleting in todoro.
        assert_eq!(std::fs::read_dir(store.notes_dir()).unwrap().count(), 0);
        // Kept on disk, so a later todoro -u can put it back.
        let mut reopened = Store::open(dir.path().join("todos.json")).unwrap();
        assert_eq!(undo(&mut reopened).unwrap(), Ok("Put back Plan the trip: 1 item with 3 sub-items, and 2 notes.".into()));
        assert_eq!(whole(&reopened, day()), before);
        assert_eq!(std::fs::read_dir(reopened.notes_dir()).unwrap().count(), 2, "notes files again");
        assert!(!reopened.deleted_path().exists());
        // Only once.
        assert!(undo(&mut reopened).unwrap().unwrap_err().starts_with("Nothing to put back"));
    }

    #[test]
    fn a_sub_item_goes_back_under_its_item() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = trip(&dir);
        let before = whole(&store, day());
        delete(&mut store, day(), noon(), &[2, 1]).unwrap().unwrap();
        assert_eq!(whole(&store, day())[1..3].iter().map(|item| (item.0, item.1.as_str())).collect::<Vec<_>>(), [(0, "Plan the trip"), (1, "Pack")]);
        undo(&mut store).unwrap().unwrap();
        assert_eq!(whole(&store, day()), before);
    }

    #[test]
    fn undo_keeps_a_sub_item_valid_if_the_list_changed_meanwhile() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = trip(&dir);
        delete(&mut store, day(), noon(), &[2, 1, 1]).unwrap().unwrap();
        // Its parents go too, so it can't be two deep after Buy milk.
        delete(&mut store, day(), noon(), &[2]).unwrap().unwrap();
        std::fs::remove_file(store.deleted_path()).unwrap();
        // Kept as two deep, at index 1: after Buy milk, which has no sub-items.
        let kept = Deleted::new(day(), 1, &[Item { text: "Compare prices".into(), depth: 2, ..Item::default() }]);
        std::fs::write(store.deleted_path(), kept.to_json()).unwrap();
        undo(&mut store).unwrap().unwrap();
        assert_eq!(whole(&store, day()).iter().map(|item| (item.0, item.1.as_str())).collect::<Vec<_>>(), [(0, "Buy milk"), (1, "Compare prices"), (0, "Call mom")]);
    }

    #[test]
    fn carried_items_are_deleted_from_their_own_day_and_go_back_there() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(&dir);
        let earlier = day().pred_opt().unwrap();
        store.insert(earlier, 0, "Renew passport".into()).unwrap();
        store.toggle_pinned(earlier, 0).unwrap();
        delete(&mut store, day(), noon(), &[1]).unwrap().unwrap();
        assert!(store.items(earlier).is_empty());
        undo(&mut store).unwrap().unwrap();
        assert_eq!(store.items(earlier)[0].text, "Renew passport");
        assert!(store.items(earlier)[0].pinned);
    }

    #[test]
    fn addresses_that_arent_there_say_whats_there() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = trip(&dir);
        let err = |store: &Store, address: &[usize]| delete_preview(store, day(), noon(), address).unwrap_err();
        assert_eq!(err(&store, &[3]), "There's no item 3. Today's open items are 1 to 2. todoro -l lists them.");
        assert_eq!(err(&store, &[1, 1]), "Item 1 has no sub-items.");
        assert_eq!(err(&store, &[2, 5]), "Item 2's sub-items are 2.1 to 2.2.");
        assert_eq!(err(&store, &[2, 1, 2]), "Item 2.1 has only one sub-item, 2.1.1.");
        assert!(delete(&mut store, day(), noon(), &[0]).unwrap().is_err());
        assert!(!store.deleted_path().exists(), "nothing deleted, nothing kept");
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
