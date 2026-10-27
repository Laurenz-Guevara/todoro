use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::PathBuf;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::deadline::Deadline;
use crate::notes_files::{Conflict, NotesFiles};
use crate::outline;
use crate::reminder::Reminder;
use crate::timeline::TimeSlot;

/// One todo entry. `notes` is longer free text shown only on the notes screen,
/// kept in its own Markdown file named by `notes_file`. A `pinned` item moves
/// forward to today until it is completed, and one with a `deadline` until
/// its deadline day; others stay on their day. Sub-items follow the item
/// they're under, one `depth` deeper (see `outline`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Item {
    pub text: String,
    pub notes: String,
    /// The notes file, in the notes folder, once the item has notes.
    pub notes_file: Option<String>,
    pub done: bool,
    pub pinned: bool,
    pub priority: Option<Priority>,
    /// When it's due. Setting one unpins the item: the deadline decides how
    /// far it moves on.
    pub deadline: Option<Deadline>,
    /// When to be reminded of it, from an earlier day.
    pub reminder: Option<Reminder>,
    /// How many levels down it is: 0 for a top-level item, 1 for a
    /// sub-item of one, and so on.
    pub depth: usize,
    /// Whether its sub-items are hidden (`za`).
    pub folded: bool,
    /// When it's planned for, on a day (`Space p`), if it is.
    pub time: Option<TimeSlot>,
}

impl Item {
    /// Whether it's open and past its deadline, on `today` at `now`.
    pub fn is_overdue(&self, today: NaiveDate, now: chrono::NaiveTime) -> bool {
        !self.done && self.deadline.is_some_and(|deadline| deadline.is_overdue(today, now))
    }

    /// When its reminder is due, if it has one and is open, as stored on `day`.
    pub fn reminder_due_at(&self, day: NaiveDate) -> Option<chrono::NaiveDateTime> {
        let reminder = self.reminder.filter(|_| !self.done)?;
        Some(reminder.due_at(day, self.deadline))
    }

    /// The day this item, stored on an earlier day, moves on to by `today`:
    /// today if it's pinned or has a deadline, overdue or not, so what's
    /// late is always in front of you. `None` if it stays where it is
    /// (completed, or neither pinned nor due).
    pub fn roll_to(&self, today: NaiveDate) -> Option<NaiveDate> {
        (!self.done && (self.pinned || self.deadline.is_some())).then_some(today)
    }

    /// Whether this item, stored on an earlier day, shows on `day` too: if
    /// it's pinned, or has a deadline and `day` is no later than it, or no
    /// later than `today` (so an overdue item is on today's list, but not
    /// on the days ahead).
    pub fn shows_on(&self, day: NaiveDate, today: NaiveDate) -> bool {
        !self.done && (self.pinned || self.deadline.is_some_and(|deadline| day <= deadline.date || day <= today))
    }
}

/// How urgent an item is, set by triaging it with `t`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    High,
    Medium,
    Low,
}

impl Priority {
    /// The next step when triaging: none, High, Medium, Low, then none again.
    pub fn cycle(priority: Option<Priority>) -> Option<Priority> {
        match priority {
            None => Some(Priority::High),
            Some(Priority::High) => Some(Priority::Medium),
            Some(Priority::Medium) => Some(Priority::Low),
            Some(Priority::Low) => None,
        }
    }
}

/// How an item is written to disk. Items with only text stay plain strings, so
/// files from before notes and completion existed load unchanged and remain
/// readable by older versions until an item gets notes or is completed.
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum RawItem {
    Text(String),
    Full {
        text: String,
        /// Notes from before they had files; read, to move them into one,
        /// but never written.
        #[serde(default, skip_serializing)]
        notes: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        notes_file: Option<String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        done: bool,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        pinned: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        priority: Option<Priority>,
        /// `2026-10-12` or `2026-10-12 13:00`; anything else is ignored.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        deadline: Option<String>,
        /// `2d`, `3h`, `1mo`, or a day like a deadline; anything else is ignored.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reminder: Option<String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        folded: bool,
        /// `2026-10-11 09:00-10:30`; anything else is ignored.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        time: Option<String>,
        /// Its sub-items, written the same way.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        items: Vec<RawItem>,
    },
}

/// Reads items as written, nested, into one list in outline order, each
/// with its `depth`.
fn flatten(raw: Vec<RawItem>, depth: usize, into: &mut Vec<Item>) {
    for raw in raw {
        match raw {
            RawItem::Text(text) => into.push(Item { text, depth, ..Item::default() }),
            RawItem::Full { text, notes, notes_file, done, pinned, priority, deadline, reminder, folded, time, items } => {
                let deadline = deadline.as_deref().and_then(Deadline::from_stored);
                let reminder = reminder.as_deref().and_then(Reminder::from_stored);
                let time = time.as_deref().and_then(TimeSlot::from_stored);
                into.push(Item { text, notes, notes_file, done, pinned, priority, deadline, reminder, depth, folded, time });
                flatten(items, depth + 1, into);
            }
        }
    }
}

/// Writes a list in outline order nested, each item with its sub-items.
fn nest(items: &[Item]) -> Vec<RawItem> {
    let mut written = Vec::new();
    let mut i = 0;
    while i < items.len() {
        let end = outline::block_end(items, i);
        let item = items[i].clone();
        let sub = nest(&items[i + 1..end]);
        written.push(if item.notes_file.is_none()
            && !item.done
            && !item.pinned
            && item.priority.is_none()
            && item.deadline.is_none()
            && item.reminder.is_none()
            && !item.folded
            && item.time.is_none()
            && sub.is_empty()
        {
            RawItem::Text(item.text)
        } else {
            let Item { text, notes, notes_file, done, pinned, priority, deadline, reminder, folded, time, .. } = item;
            let deadline = deadline.map(Deadline::to_stored);
            let reminder = reminder.map(Reminder::to_stored);
            let time = time.map(TimeSlot::to_stored);
            RawItem::Full { text, notes, notes_file, done, pinned, priority, deadline, reminder, folded, time, items: sub }
        });
        i = end;
    }
    written
}

/// A copy of every item, to restore for undo.
#[derive(Clone, PartialEq)]
pub struct Snapshot(BTreeMap<String, Vec<Item>>);

/// Todo items keyed by day, persisted as JSON. Each day's items are kept with
/// the open ones first and the completed ones after them.
pub struct Store {
    path: PathBuf,
    days: BTreeMap<String, Vec<Item>>,
    notes: NotesFiles,
    /// Notes files changed outside todoro that a save left alone, to tell
    /// the user about.
    conflicts: Vec<Conflict>,
}

impl Store {
    /// Loads from `path`. A missing file is an empty store; it is created on first save.
    pub fn open(path: PathBuf) -> io::Result<Self> {
        let raw: BTreeMap<String, Vec<RawItem>> = match fs::read_to_string(&path) {
            Ok(s) => serde_json::from_str(&s).map_err(io::Error::other)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(e),
        };
        let mut days: BTreeMap<String, Vec<Item>> = raw
            .into_iter()
            .map(|(day, raw)| {
                let mut items = Vec::new();
                flatten(raw, 0, &mut items);
                (day, items)
            })
            .collect();
        // A hand-edited file may mix open and completed items, or complete an
        // item with open sub-items. This keeps each group in its order.
        for items in days.values_mut() {
            outline::normalize(items);
        }
        // Notes from before they had files move into them now.
        let inline = days.values().flatten().any(|item| !item.notes.is_empty() && item.notes_file.is_none());
        let mut notes = NotesFiles::for_todos(&path);
        notes.load(&mut days)?;
        let mut store = Self { path, days, notes, conflicts: Vec::new() };
        if inline {
            store.save()?;
        }
        Ok(store)
    }

    /// The folder notes files are kept in.
    #[cfg(test)]
    pub fn notes_dir(&self) -> &std::path::Path {
        &self.notes.dir
    }

    /// Reads an item's notes from its file again, in case something else
    /// changed it, as before opening them.
    pub fn reload_notes(&mut self, day: NaiveDate, index: usize) -> io::Result<()> {
        let Some(file) = self.items(day).get(index).and_then(|item| item.notes_file.clone()) else { return Ok(()) };
        let text = self.notes.read(&file)?;
        if let Some(item) = self.item_mut(day, index) {
            match text {
                Some(text) => item.notes = text,
                None => {
                    item.notes.clear();
                    item.notes_file = None;
                }
            }
        }
        Ok(())
    }

    /// The path of an item's notes file, to open in another editor, after
    /// reading it again. An item without notes gets an empty file, which is
    /// deleted again by `notes_edited` if it's still empty.
    pub fn notes_path(&mut self, day: NaiveDate, index: usize) -> io::Result<Option<PathBuf>> {
        self.reload_notes(day, index)?;
        let Some(item) = self.items(day).get(index) else { return Ok(None) };
        let file = match &item.notes_file {
            Some(file) => file.clone(),
            None => {
                let text = item.text.clone();
                let taken = self.days.values().flatten().filter_map(|item| item.notes_file.clone()).collect();
                let file = self.notes.create(&text, &taken)?;
                if let Some(item) = self.item_mut(day, index) {
                    item.notes_file = Some(file.clone());
                }
                file
            }
        };
        Ok(Some(self.notes.dir.join(file)))
    }

    /// Takes in what another editor saved in an item's notes file (from
    /// `notes_path`), and saves.
    pub fn notes_edited(&mut self, day: NaiveDate, index: usize) -> io::Result<()> {
        self.reload_notes(day, index)?;
        self.save()
    }

    /// How many open items are past their deadline, on `today` at `now`.
    pub fn overdue_count(&self, today: NaiveDate, now: chrono::NaiveTime) -> usize {
        self.days.values().flatten().filter(|item| item.is_overdue(today, now)).count()
    }

    /// Deletes every item's notes, and every notes file, for good.
    pub fn delete_all_notes(&mut self) -> io::Result<()> {
        for item in self.days.values_mut().flatten() {
            item.notes.clear();
            item.notes_file = None;
        }
        self.notes.delete_all()?;
        self.save()
    }

    /// Deletes every item and every notes file, for good.
    pub fn delete_everything(&mut self) -> io::Result<()> {
        self.days.clear();
        self.notes.delete_all()?;
        self.save()
    }

    /// Notes files that saves left alone because they changed outside todoro.
    pub fn take_conflicts(&mut self) -> Vec<Conflict> {
        std::mem::take(&mut self.conflicts)
    }

    /// Moves every pinned, open item from days before `today` to the start of
    /// today's list, oldest day first. They stay pinned, so they keep moving
    /// forward until completed. Everything else stays on its day.
    pub fn roll_over(&mut self, today: NaiveDate) -> io::Result<()> {
        let today_key = key(today);
        // Where each moving item goes: today. Oldest first, so they keep
        // their order.
        let mut moving: BTreeMap<NaiveDate, Vec<Item>> = BTreeMap::new();
        self.days.retain(|day, items| {
            if *day < today_key
                && let Ok(date) = NaiveDate::parse_from_str(day, "%Y-%m-%d")
            {
                // Top-level items move, with their sub-items.
                let moves: Vec<usize> = outline::roots(items)
                    .into_iter()
                    .filter(|&i| items[i].roll_to(today).is_some_and(|to| to > date))
                    .collect();
                let mut taken: Vec<(NaiveDate, Vec<Item>)> = Vec::new();
                for &start in moves.iter().rev() {
                    let to = items[start].roll_to(today).expect("only moving items are taken");
                    let end = outline::block_end(items, start);
                    taken.push((to, items.drain(start..end).collect()));
                }
                // Back in their order, after any from earlier days.
                for (to, block) in taken.into_iter().rev() {
                    moving.entry(to).or_default().extend(block);
                }
            }
            !items.is_empty()
        });
        if moving.is_empty() {
            return Ok(());
        }
        for (day, carried) in moving {
            self.days.entry(key(day)).or_default().splice(0..0, carried);
        }
        self.save()
    }

    /// Where the open items from days before `day` that show on it are, as
    /// `(day, index)`, oldest day first: pinned ones, ones due on `day` or
    /// later, and on `today` overdue ones (`Item::shows_on`).
    pub fn carried_to(&self, day: NaiveDate, today: NaiveDate) -> Vec<(NaiveDate, usize)> {
        self.days
            .range(..key(day))
            .filter_map(|(k, items)| Some((NaiveDate::parse_from_str(k, "%Y-%m-%d").ok()?, items)))
            .flat_map(|(date, items)| {
                items
                    .iter()
                    .enumerate()
                    .filter(|(_, item)| item.depth == 0 && item.shows_on(day, today))
                    .map(move |(i, _)| (date, i))
            })
            .collect()
    }

    /// Where the open items from days after `day` that show on it are, as
    /// `(day, index)`, soonest day first: ones whose reminder is due by then.
    /// Only on today (at `now`) and later days; a reminder is about what's
    /// coming, not what was.
    pub fn reminded_on(&self, day: NaiveDate, today: NaiveDate, now: chrono::NaiveTime) -> Vec<(NaiveDate, usize)> {
        if day < today {
            return Vec::new();
        }
        // By now today, or at any time on a later day.
        let by = if day == today { today.and_time(now) } else { day.and_hms_opt(23, 59, 59).expect("a valid time") };
        self.all()
            .filter(|(date, _)| *date > day)
            .flat_map(|(date, items)| {
                items
                    .iter()
                    .enumerate()
                    .filter(move |(_, item)| item.depth == 0 && item.reminder_due_at(date).is_some_and(|at| at <= by))
                    .map(move |(i, _)| (date, i))
            })
            .collect()
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot(self.days.clone())
    }

    /// Replaces every item with a snapshot's and saves.
    pub fn restore(&mut self, snapshot: Snapshot) -> io::Result<()> {
        self.days = snapshot.0;
        self.save()
    }

    /// Where the completed top-level items on `day` start: the open ones,
    /// with their sub-items, come first in `items`.
    pub fn open_count(&self, day: NaiveDate) -> usize {
        boundary(self.items(day))
    }

    /// Every day that has items, oldest first, with its items.
    pub fn all(&self) -> impl Iterator<Item = (NaiveDate, &[Item])> {
        self.days.iter().filter_map(|(k, items)| Some((NaiveDate::parse_from_str(k, "%Y-%m-%d").ok()?, items.as_slice())))
    }

    pub fn items(&self, day: NaiveDate) -> &[Item] {
        self.days.get(&key(day)).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Inserts a plain new item, for tests. The app adds items through
    /// `App::add_item`, which may pin them.
    #[cfg(test)]
    pub fn insert(&mut self, day: NaiveDate, index: usize, text: String) -> io::Result<()> {
        self.insert_items(day, index, vec![Item { text, ..Item::default() }]).map(|_| ())
    }

    /// Inserts `items` at `index` as open, top-level items (with any
    /// sub-items they bring), keeping their notes, pin and priority. Returns
    /// where the first one went.
    pub fn insert_items(&mut self, day: NaiveDate, index: usize, items: Vec<Item>) -> io::Result<usize> {
        // New items are open, so they always go among the open ones.
        let index = index.min(self.open_count(day));
        self.insert_at(day, index, items, 0)
    }

    /// Inserts `items` exactly at `index`, open, the first of them `depth`
    /// deep (a sub-item of what's above, at a depth over 0). Returns where
    /// the first one ended up, as an open sub-item reopens its parents.
    pub fn insert_at(&mut self, day: NaiveDate, index: usize, items: Vec<Item>, depth: usize) -> io::Result<usize> {
        let mut items: Vec<Item> = items.into_iter().map(|item| Item { done: false, notes_file: None, ..item }).collect();
        outline::rebase(&mut items, depth);
        let day_items = self.days.entry(key(day)).or_default();
        let index = index.min(day_items.len());
        // Copies get notes files of their own.
        day_items.splice(index..index, items);
        let moved = outline::normalize(day_items);
        self.save()?;
        Ok(moved.get(index).copied().unwrap_or(index))
    }

    pub fn set_text(&mut self, day: NaiveDate, index: usize, text: String) -> io::Result<()> {
        if let Some(item) = self.item_mut(day, index) {
            item.text = text;
        }
        self.save()
    }

    pub fn set_notes(&mut self, day: NaiveDate, index: usize, notes: String) -> io::Result<()> {
        if let Some(item) = self.item_mut(day, index) {
            item.notes = notes;
        }
        self.save()
    }

    /// Completes an open item or reopens a completed one, with its
    /// sub-items. A top-level item moves to the boundary between the two
    /// groups: the top of the completed items, or the bottom of the open
    /// ones. Reopening a sub-item reopens what it's under.
    pub fn toggle_done(&mut self, day: NaiveDate, index: usize) -> io::Result<()> {
        let done = !self.items(day).get(index).is_some_and(|item| item.done);
        self.set_done_many(day, &[index], done)
    }

    /// Sets or clears an item's deadline. Setting one unpins it, since the
    /// deadline decides how far it moves on.
    pub fn set_deadline(&mut self, day: NaiveDate, index: usize, deadline: Option<Deadline>) -> io::Result<()> {
        if let Some(item) = self.item_mut(day, index) {
            item.deadline = deadline;
            if deadline.is_some() {
                item.pinned = false;
            }
        }
        self.save()
    }

    /// Sets or clears an item's reminder.
    pub fn set_reminder(&mut self, day: NaiveDate, index: usize, reminder: Option<Reminder>) -> io::Result<()> {
        if let Some(item) = self.item_mut(day, index) {
            item.reminder = reminder;
        }
        self.save()
    }

    pub fn toggle_pinned(&mut self, day: NaiveDate, index: usize) -> io::Result<()> {
        if let Some(item) = self.item_mut(day, index) {
            item.pinned = !item.pinned;
        }
        self.save()
    }

    /// Moves an item, with its sub-items, to another day as a top-level
    /// item, where it goes at the boundary between the open and completed
    /// items, like `toggle_done`. Returns its new index.
    pub fn move_to(&mut self, day: NaiveDate, index: usize, to: NaiveDate) -> io::Result<Option<usize>> {
        self.move_many(day, &[index], to)
    }

    /// Completes or reopens the items at `indices` on `day`, with their
    /// sub-items. Like `toggle_done`, top-level ones move to the boundary
    /// between open and completed items, keeping their order.
    pub fn set_done_many(&mut self, day: NaiveDate, indices: &[usize], done: bool) -> io::Result<()> {
        let Some(items) = self.days.get_mut(&key(day)) else { return Ok(()) };
        for i in outline::outermost(items, indices) {
            let end = outline::block_end(items, i);
            for item in &mut items[i..end] {
                item.done = done;
            }
        }
        outline::normalize(items);
        self.save()
    }

    /// Applies `change` to each item at `indices` on `day`, saving once.
    pub fn update_many(&mut self, day: NaiveDate, indices: &[usize], mut change: impl FnMut(&mut Item)) -> io::Result<()> {
        if let Some(items) = self.days.get_mut(&key(day)) {
            for &i in indices {
                if let Some(item) = items.get_mut(i) {
                    change(item);
                }
            }
        }
        self.save()
    }

    /// Removes the items at `indices` on `day`, with their sub-items, and
    /// returns them in order, each block starting at the top level, as they
    /// would be pasted.
    pub fn remove_many(&mut self, day: NaiveDate, indices: &[usize]) -> io::Result<Vec<Item>> {
        let mut blocks = self.take_blocks(day, indices);
        for block in &mut blocks {
            outline::rebase(block, 0);
        }
        let removed = blocks.into_iter().flatten().collect();
        self.save()?;
        Ok(removed)
    }

    /// Takes the blocks of the items at `indices` out of `day`, in order.
    fn take_blocks(&mut self, day: NaiveDate, indices: &[usize]) -> Vec<Vec<Item>> {
        let k = key(day);
        let Some(items) = self.days.get_mut(&k) else { return Vec::new() };
        let mut blocks = Vec::new();
        for start in outline::outermost(items, indices).into_iter().rev() {
            let end = outline::block_end(items, start);
            blocks.push(items.drain(start..end).collect());
        }
        blocks.reverse();
        if items.is_empty() {
            self.days.remove(&k);
        } else {
            outline::normalize(items);
        }
        blocks
    }

    /// Moves the items at `indices` on `day`, with their sub-items, to the
    /// day `to` as top-level items, at the boundary between its open and
    /// completed items like `move_to`, keeping their order. Returns where
    /// the first one went.
    pub fn move_many(&mut self, day: NaiveDate, indices: &[usize], to: NaiveDate) -> io::Result<Option<usize>> {
        let mut blocks = self.take_blocks(day, indices);
        if blocks.is_empty() {
            return Ok(None);
        }
        // Open ones first, so the completed ones stay after them.
        blocks.sort_by_key(|block| block[0].done);
        for block in &mut blocks {
            outline::rebase(block, 0);
        }
        let items = self.days.entry(key(to)).or_default();
        let at = boundary(items);
        items.splice(at..at, blocks.into_iter().flatten());
        self.save()?;
        Ok(Some(at))
    }

    /// Moves an item (with its sub-items) past the next item at its level
    /// (`down`), or above the previous one, staying among the open or the
    /// completed ones. Returns its new index, if it moved.
    pub fn move_sibling(&mut self, day: NaiveDate, index: usize, down: bool) -> io::Result<Option<usize>> {
        let Some(items) = self.days.get_mut(&key(day)) else { return Ok(None) };
        if index >= items.len() {
            return Ok(None);
        }
        let other = if down { outline::next_sibling(items, index) } else { outline::prev_sibling(items, index) };
        let Some(other) = other.filter(|&other| items[index].depth > 0 || items[other].done == items[index].done) else {
            return Ok(None);
        };
        let (first, second) = if down { (index, other) } else { (other, index) };
        let end = outline::block_end(items, second);
        // The second block goes before the first.
        items[first..end].rotate_left(second - first);
        let moved = if down { index + (end - second) } else { other };
        self.save()?;
        Ok(Some(moved))
    }

    /// Makes an item (with its sub-items) a sub-item of the item above it at
    /// its level, as its last. Returns its new index, if it could.
    pub fn indent(&mut self, day: NaiveDate, index: usize) -> io::Result<Option<usize>> {
        let Some(items) = self.days.get_mut(&key(day)) else { return Ok(None) };
        if index >= items.len() || outline::prev_sibling(items, index).is_none() {
            return Ok(None);
        }
        let end = outline::block_end(items, index);
        for item in &mut items[index..end] {
            item.depth += 1;
        }
        // Under an item that's folded, it would vanish: show it.
        if let Some(parent) = outline::parent(items, index) {
            items[parent].folded = false;
        }
        let moved = outline::normalize(items);
        self.save()?;
        Ok(Some(moved[index]))
    }

    /// Moves a sub-item (with its own sub-items) out a level, to just after
    /// the item it was under. Returns its new index, if it moved.
    pub fn outdent(&mut self, day: NaiveDate, index: usize) -> io::Result<Option<usize>> {
        let Some(items) = self.days.get_mut(&key(day)) else { return Ok(None) };
        let Some(parent) = (index < items.len()).then(|| outline::parent(items, index)).flatten() else {
            return Ok(None);
        };
        let end = outline::block_end(items, index);
        let parent_end = outline::block_end(items, parent);
        for item in &mut items[index..end] {
            item.depth -= 1;
        }
        // Past the rest of what was under the parent, so they stay there.
        items[index..parent_end].rotate_left(end - index);
        let at = parent_end - (end - index);
        let moved = outline::normalize(items);
        self.save()?;
        Ok(Some(moved[at]))
    }

    /// Plans an item for a time on a day, or with `None`, for no time.
    pub fn set_time(&mut self, day: NaiveDate, index: usize, time: Option<TimeSlot>) -> io::Result<()> {
        if let Some(item) = self.item_mut(day, index) {
            item.time = time;
        }
        self.save()
    }

    /// Folds or unfolds an item's sub-items.
    pub fn set_folded(&mut self, day: NaiveDate, index: usize, folded: bool) -> io::Result<()> {
        if let Some(item) = self.item_mut(day, index) {
            item.folded = folded;
        }
        self.save()
    }

    /// Moves an item to its next priority: none, High, Medium, Low, none.
    pub fn cycle_priority(&mut self, day: NaiveDate, index: usize) -> io::Result<()> {
        if let Some(item) = self.item_mut(day, index) {
            item.priority = Priority::cycle(item.priority);
        }
        self.save()
    }

    fn item_mut(&mut self, day: NaiveDate, index: usize) -> Option<&mut Item> {
        self.days.get_mut(&key(day)).and_then(|items| items.get_mut(index))
    }

    fn save(&mut self) -> io::Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        // Notes first, since new ones get their file names here.
        let conflicts = self.notes.sync(&mut self.days)?;
        self.conflicts.extend(conflicts);
        // Write to a temp file and rename so a crash never leaves a half-written file.
        let tmp = self.path.with_extension("json.tmp");
        let nested: BTreeMap<&String, Vec<RawItem>> = self.days.iter().map(|(day, items)| (day, nest(items))).collect();
        fs::write(&tmp, serde_json::to_string_pretty(&nested).map_err(io::Error::other)?)?;
        fs::rename(tmp, &self.path)
    }
}

/// Where the completed top-level items start in a day's items.
fn boundary(items: &[Item]) -> usize {
    items.iter().position(|item| item.depth == 0 && item.done).unwrap_or(items.len())
}

fn key(day: NaiveDate) -> String {
    day.format("%Y-%m-%d").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::today;

    fn temp_path() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("todos.json");
        (dir, path)
    }

    fn texts(store: &Store, day: NaiveDate) -> Vec<&str> {
        store.items(day).iter().map(|item| item.text.as_str()).collect()
    }

    #[test]
    fn missing_file_is_an_empty_store() {
        let (_dir, path) = temp_path();
        let store = Store::open(path.clone()).unwrap();
        assert!(store.items(today()).is_empty());
        assert!(!path.exists());
    }

    #[test]
    fn changes_are_saved_and_reload() {
        let (_dir, path) = temp_path();
        let mut store = Store::open(path.clone()).unwrap();
        store.insert(today(), 0, "one".into()).unwrap();
        store.insert(today(), 1, "two".into()).unwrap();
        store.insert(today(), 1, "middle".into()).unwrap();
        store.set_text(today(), 0, "uno".into()).unwrap();
        store.set_notes(today(), 1, "line 1\nline 2".into()).unwrap();
        store.remove_many(today(), &[2]).unwrap();

        let reloaded = Store::open(path).unwrap();
        assert_eq!(texts(&reloaded, today()), ["uno", "middle"]);
        assert_eq!(reloaded.items(today())[0].notes, "");
        assert_eq!(reloaded.items(today())[1].notes, "line 1\nline 2");
    }

    #[test]
    fn days_are_kept_separate() {
        let (_dir, path) = temp_path();
        let tomorrow = today().succ_opt().unwrap();
        let mut store = Store::open(path).unwrap();
        store.insert(today(), 0, "today".into()).unwrap();
        store.insert(tomorrow, 0, "tomorrow".into()).unwrap();
        assert_eq!(texts(&store, today()), ["today"]);
        assert_eq!(texts(&store, tomorrow), ["tomorrow"]);
    }

    #[test]
    fn file_is_keyed_by_iso_date() {
        let (_dir, path) = temp_path();
        let mut store = Store::open(path.clone()).unwrap();
        store.insert(today(), 0, "one".into()).unwrap();
        let json: serde_json::Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(json, serde_json::json!({ "2026-10-05": ["one"] }));
    }

    #[test]
    fn items_with_notes_are_saved_as_objects_and_others_as_strings() {
        let (_dir, path) = temp_path();
        let mut store = Store::open(path.clone()).unwrap();
        store.insert(today(), 0, "plain".into()).unwrap();
        store.insert(today(), 1, "detailed".into()).unwrap();
        store.set_notes(today(), 1, "some notes".into()).unwrap();
        let json: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "2026-10-05": ["plain", { "text": "detailed", "notes_file": "detailed.md" }] })
        );
        // The notes themselves are a Markdown file beside it.
        let notes = fs::read_to_string(path.parent().unwrap().join("notes").join("detailed.md")).unwrap();
        assert_eq!(notes, "some notes");
    }

    #[test]
    fn clearing_notes_turns_an_item_back_into_a_string() {
        let (_dir, path) = temp_path();
        let mut store = Store::open(path.clone()).unwrap();
        store.insert(today(), 0, "one".into()).unwrap();
        store.set_notes(today(), 0, "notes".into()).unwrap();
        store.set_notes(today(), 0, String::new()).unwrap();
        let json: serde_json::Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(json, serde_json::json!({ "2026-10-05": ["one"] }));
    }

    #[test]
    fn files_from_before_notes_still_load() {
        let (_dir, path) = temp_path();
        fs::write(&path, r#"{ "2026-10-05": ["Buy milk", "Write report"] }"#).unwrap();
        let store = Store::open(path).unwrap();
        assert_eq!(texts(&store, today()), ["Buy milk", "Write report"]);
        assert!(store.items(today()).iter().all(|item| item.notes.is_empty()));
    }

    fn day(offset: i64) -> NaiveDate {
        today() + chrono::Duration::days(offset)
    }

    fn done_flags(store: &Store, day: NaiveDate) -> Vec<bool> {
        store.items(day).iter().map(|item| item.done).collect()
    }

    #[test]
    fn completing_moves_an_item_to_the_top_of_the_completed_ones() {
        let (_dir, path) = temp_path();
        let mut store = Store::open(path).unwrap();
        for (i, text) in ["a", "b", "c"].into_iter().enumerate() {
            store.insert(today(), i, text.into()).unwrap();
        }
        store.toggle_done(today(), 2).unwrap();
        store.toggle_done(today(), 0).unwrap();
        assert_eq!(texts(&store, today()), ["b", "a", "c"]);
        assert_eq!(done_flags(&store, today()), [false, true, true]);
        assert_eq!(store.open_count(today()), 1);
    }

    #[test]
    fn reopening_moves_an_item_to_the_bottom_of_the_open_ones() {
        let (_dir, path) = temp_path();
        let mut store = Store::open(path).unwrap();
        for (i, text) in ["a", "b", "c"].into_iter().enumerate() {
            store.insert(today(), i, text.into()).unwrap();
        }
        store.toggle_done(today(), 0).unwrap();
        store.toggle_done(today(), 0).unwrap();
        // Toggling index 0 twice completes a, then b.
        assert_eq!(texts(&store, today()), ["c", "b", "a"]);
        store.toggle_done(today(), 2).unwrap();
        assert_eq!(texts(&store, today()), ["c", "a", "b"]);
        assert_eq!(done_flags(&store, today()), [false, false, true]);
    }

    #[test]
    fn new_items_are_never_inserted_among_completed_ones() {
        let (_dir, path) = temp_path();
        let mut store = Store::open(path).unwrap();
        store.insert(today(), 0, "a".into()).unwrap();
        store.insert(today(), 1, "b".into()).unwrap();
        store.toggle_done(today(), 1).unwrap();
        store.insert(today(), 5, "new".into()).unwrap();
        assert_eq!(texts(&store, today()), ["a", "new", "b"]);
    }

    #[test]
    fn completed_items_are_saved_and_reload() {
        let (_dir, path) = temp_path();
        let mut store = Store::open(path.clone()).unwrap();
        store.insert(today(), 0, "plain".into()).unwrap();
        store.insert(today(), 1, "done".into()).unwrap();
        store.insert(today(), 2, "both".into()).unwrap();
        store.set_notes(today(), 2, "n".into()).unwrap();
        store.toggle_done(today(), 1).unwrap();
        store.toggle_done(today(), 1).unwrap();
        let json: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "2026-10-05": [
                "plain",
                { "text": "both", "notes_file": "both.md", "done": true },
                { "text": "done", "done": true },
            ] })
        );
        let reloaded = Store::open(path).unwrap();
        assert_eq!(done_flags(&reloaded, today()), [false, true, true]);
    }

    #[test]
    fn loading_puts_completed_items_after_open_ones() {
        let (_dir, path) = temp_path();
        fs::write(&path, r#"{ "2026-10-05": [{ "text": "d1", "done": true }, "o1", { "text": "d2", "done": true }, "o2"] }"#)
            .unwrap();
        let store = Store::open(path).unwrap();
        assert_eq!(texts(&store, today()), ["o1", "o2", "d1", "d2"]);
    }

    /// Adds an item to the end of `day`'s open items and pins it.
    fn insert_pinned(store: &mut Store, day: NaiveDate, text: &str) {
        let index = store.open_count(day);
        store.insert(day, index, text.into()).unwrap();
        store.toggle_pinned(day, index).unwrap();
    }

    #[test]
    fn roll_over_moves_pinned_open_items_from_past_days_to_today() {
        let (_dir, path) = temp_path();
        let mut store = Store::open(path.clone()).unwrap();
        insert_pinned(&mut store, day(-2), "oldest");
        insert_pinned(&mut store, day(-1), "yesterday");
        store.set_notes(day(-1), 0, "keep me".into()).unwrap();
        store.insert(day(-1), 1, "stays".into()).unwrap();
        insert_pinned(&mut store, day(-1), "finished");
        store.toggle_done(day(-1), 2).unwrap();
        store.insert(today(), 0, "planned".into()).unwrap();
        insert_pinned(&mut store, day(1), "tomorrow");

        store.roll_over(today()).unwrap();

        assert_eq!(texts(&store, today()), ["oldest", "yesterday", "planned"]);
        assert_eq!(store.items(today())[1].notes, "keep me");
        assert!(store.items(today())[..2].iter().all(|item| item.pinned));
        assert!(store.items(day(-2)).is_empty());
        assert_eq!(texts(&store, day(-1)), ["stays", "finished"]);
        assert_eq!(texts(&store, day(1)), ["tomorrow"]);

        // Saved, and the emptied day is gone from the file.
        let reloaded = Store::open(path.clone()).unwrap();
        assert_eq!(texts(&reloaded, today()), ["oldest", "yesterday", "planned"]);
        assert!(!fs::read_to_string(path).unwrap().contains("2026-10-03"));
    }

    #[test]
    fn pinned_items_keep_moving_forward_each_day() {
        let (_dir, path) = temp_path();
        let mut store = Store::open(path).unwrap();
        insert_pinned(&mut store, day(-1), "ongoing");
        store.roll_over(today()).unwrap();
        store.roll_over(day(1)).unwrap();
        assert!(store.items(today()).is_empty());
        assert_eq!(texts(&store, day(1)), ["ongoing"]);
    }

    #[test]
    fn roll_over_leaves_unpinned_items_on_their_day() {
        let (_dir, path) = temp_path();
        let mut store = Store::open(path.clone()).unwrap();
        store.insert(day(-1), 0, "dentist at 3pm".into()).unwrap();
        store.roll_over(today()).unwrap();
        assert_eq!(texts(&store, day(-1)), ["dentist at 3pm"]);
        assert!(store.items(today()).is_empty());
    }

    #[test]
    fn toggle_pinned_is_saved_and_reloads() {
        let (_dir, path) = temp_path();
        let mut store = Store::open(path.clone()).unwrap();
        store.insert(today(), 0, "a".into()).unwrap();
        store.insert(today(), 1, "b".into()).unwrap();
        store.toggle_pinned(today(), 0).unwrap();
        store.toggle_pinned(today(), 1).unwrap();
        store.toggle_pinned(today(), 1).unwrap();
        let json: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(json, serde_json::json!({ "2026-10-05": [{ "text": "a", "pinned": true }, "b"] }));
        let reloaded = Store::open(path).unwrap();
        assert!(reloaded.items(today())[0].pinned);
        assert!(!reloaded.items(today())[1].pinned);
    }

    #[test]
    fn roll_over_keeps_carried_items_above_completed_ones() {
        let (_dir, path) = temp_path();
        let mut store = Store::open(path).unwrap();
        insert_pinned(&mut store, day(-1), "carried");
        store.insert(today(), 0, "done today".into()).unwrap();
        store.toggle_done(today(), 0).unwrap();
        store.roll_over(today()).unwrap();
        assert_eq!(texts(&store, today()), ["carried", "done today"]);
        assert_eq!(store.open_count(today()), 1);
    }

    #[test]
    fn carried_to_lists_what_roll_over_would_move() {
        let (_dir, path) = temp_path();
        let mut store = Store::open(path).unwrap();
        insert_pinned(&mut store, day(-1), "older");
        store.insert(today(), 0, "not pinned".into()).unwrap();
        insert_pinned(&mut store, today(), "today");
        insert_pinned(&mut store, today(), "done");
        store.toggle_done(today(), 2).unwrap();
        insert_pinned(&mut store, day(2), "later");
        assert_eq!(store.carried_to(day(2), today()), [(day(-1), 0), (today(), 1)]);
        assert_eq!(store.carried_to(day(3), today()), [(day(-1), 0), (today(), 1), (day(2), 0)]);
        assert!(store.carried_to(day(-1), today()).is_empty());
    }

    #[test]
    fn move_sibling_reorders_within_the_open_or_completed_items() {
        let (_dir, path) = temp_path();
        let mut store = Store::open(path.clone()).unwrap();
        for (i, text) in ["a", "b", "c", "d"].into_iter().enumerate() {
            store.insert(today(), i, text.into()).unwrap();
        }
        store.toggle_done(today(), 3).unwrap();
        store.toggle_done(today(), 2).unwrap();
        // ["a", "b", "c" (done), "d" (done)]
        assert_eq!(store.move_sibling(today(), 0, true).unwrap(), Some(1));
        assert_eq!(store.move_sibling(today(), 3, false).unwrap(), Some(2));
        assert_eq!(texts(&store, today()), ["b", "a", "d", "c"]);
        // Never across the boundary, past the end, or on a day with no items.
        assert_eq!(store.move_sibling(today(), 1, true).unwrap(), None);
        assert_eq!(store.move_sibling(today(), 3, true).unwrap(), None);
        assert_eq!(store.move_sibling(today(), 0, false).unwrap(), None);
        assert_eq!(store.move_sibling(day(1), 0, true).unwrap(), None);
        assert_eq!(texts(&Store::open(path).unwrap(), today()), ["b", "a", "d", "c"]);
    }

    #[test]
    fn move_to_puts_the_item_at_the_boundary_of_the_other_day() {
        let (_dir, path) = temp_path();
        let mut store = Store::open(path.clone()).unwrap();
        store.insert(today(), 0, "open".into()).unwrap();
        store.insert(today(), 1, "done".into()).unwrap();
        store.set_notes(today(), 0, "notes".into()).unwrap();
        store.toggle_done(today(), 1).unwrap();
        store.insert(day(1), 0, "t1".into()).unwrap();
        store.insert(day(1), 1, "t2".into()).unwrap();
        store.toggle_done(day(1), 1).unwrap();
        // Tomorrow: ["t1", "t2" (done)].
        assert_eq!(store.move_to(today(), 0, day(1)).unwrap(), Some(1));
        assert_eq!(store.move_to(today(), 0, day(1)).unwrap(), Some(2));
        assert_eq!(texts(&store, day(1)), ["t1", "open", "done", "t2"]);
        assert_eq!(store.items(day(1))[1].notes, "notes");
        assert!(store.items(day(1))[2].done);
        // Today is now empty and gone from the file.
        assert!(store.items(today()).is_empty());
        assert!(!fs::read_to_string(&path).unwrap().contains("2026-10-05"));
        assert_eq!(store.move_to(today(), 0, day(1)).unwrap(), None);
    }

    #[test]
    fn restore_brings_back_a_snapshot_and_saves_it() {
        let (_dir, path) = temp_path();
        let mut store = Store::open(path.clone()).unwrap();
        store.insert(today(), 0, "one".into()).unwrap();
        let snapshot = store.snapshot();
        store.insert(today(), 1, "two".into()).unwrap();
        store.toggle_done(today(), 0).unwrap();
        assert!(store.snapshot() != snapshot);
        store.restore(snapshot.clone()).unwrap();
        assert!(store.snapshot() == snapshot);
        assert_eq!(texts(&Store::open(path).unwrap(), today()), ["one"]);
    }

    #[test]
    fn cycle_priority_goes_high_medium_low_then_none() {
        let (_dir, path) = temp_path();
        let mut store = Store::open(path).unwrap();
        store.insert(today(), 0, "a".into()).unwrap();
        let mut seen = Vec::new();
        for _ in 0..4 {
            store.cycle_priority(today(), 0).unwrap();
            seen.push(store.items(today())[0].priority);
        }
        assert_eq!(seen, [Some(Priority::High), Some(Priority::Medium), Some(Priority::Low), None]);
        store.cycle_priority(today(), 9).unwrap();
    }

    #[test]
    fn priority_is_saved_by_name_and_reloads() {
        let (_dir, path) = temp_path();
        let mut store = Store::open(path.clone()).unwrap();
        store.insert(today(), 0, "urgent".into()).unwrap();
        store.insert(today(), 1, "someday".into()).unwrap();
        store.insert(today(), 2, "plain".into()).unwrap();
        store.cycle_priority(today(), 0).unwrap();
        for _ in 0..3 {
            store.cycle_priority(today(), 1).unwrap();
        }
        let json: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "2026-10-05": [
                { "text": "urgent", "priority": "high" },
                { "text": "someday", "priority": "low" },
                "plain",
            ] })
        );
        let reloaded = Store::open(path).unwrap();
        let priorities: Vec<_> = reloaded.items(today()).iter().map(|item| item.priority).collect();
        assert_eq!(priorities, [Some(Priority::High), Some(Priority::Low), None]);
    }

    fn due(days: i64, time: Option<(u32, u32)>) -> Deadline {
        let time = time.map(|(h, m)| chrono::NaiveTime::from_hms_opt(h, m, 0).unwrap());
        Deadline { date: day(days), time }
    }

    #[test]
    fn deadlines_are_saved_and_loaded() {
        let (_dir, path) = temp_path();
        let mut store = Store::open(path.clone()).unwrap();
        store.insert(today(), 0, "Report".into()).unwrap();
        store.insert(today(), 1, "Call".into()).unwrap();
        store.set_deadline(today(), 0, Some(due(4, Some((17, 30))))).unwrap();
        store.set_deadline(today(), 1, Some(due(1, None))).unwrap();
        let json: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "2026-10-05": [
                { "text": "Report", "deadline": "2026-10-09 17:30" },
                { "text": "Call", "deadline": "2026-10-06" },
            ] })
        );
        let reopened = Store::open(path.clone()).unwrap();
        assert_eq!(reopened.items(today())[0].deadline, Some(due(4, Some((17, 30)))));
        // Removing it makes the item plain text again.
        store.set_deadline(today(), 1, None).unwrap();
        let json: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(json["2026-10-05"][1], "Call");
    }

    fn days_before(amount: u32) -> Reminder {
        Reminder::Before { amount, unit: crate::reminder::Unit::Days }
    }

    fn noon() -> chrono::NaiveTime {
        chrono::NaiveTime::from_hms_opt(12, 0, 0).unwrap()
    }

    #[test]
    fn reminders_are_saved_and_loaded() {
        let (_dir, path) = temp_path();
        let mut store = Store::open(path.clone()).unwrap();
        store.insert(day(9), 0, "Gift".into()).unwrap();
        store.insert(day(9), 1, "Cake".into()).unwrap();
        store.set_reminder(day(9), 0, Some(days_before(2))).unwrap();
        store.set_reminder(day(9), 1, Some(Reminder::On(due(3, Some((9, 0)))))).unwrap();
        let json: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "2026-10-14": [
                { "text": "Gift", "reminder": "2d" },
                { "text": "Cake", "reminder": "2026-10-08 09:00" },
            ] })
        );
        let reopened = Store::open(path.clone()).unwrap();
        assert_eq!(reopened.items(day(9))[0].reminder, Some(days_before(2)));
        assert_eq!(reopened.items(day(9))[1].reminder, Some(Reminder::On(due(3, Some((9, 0))))));
        // Removing it makes the item plain text again, and one that can't be
        // read is dropped.
        store.set_reminder(day(9), 1, None).unwrap();
        let json: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(json["2026-10-14"][1], "Cake");
        fs::write(&path, r#"{ "2026-10-14": [{ "text": "Gift", "reminder": "soon" }] }"#).unwrap();
        assert_eq!(Store::open(path).unwrap().items(day(9))[0].reminder, None);
    }

    #[test]
    fn items_show_on_days_from_their_reminder_until_their_own() {
        let (_dir, path) = temp_path();
        let mut store = Store::open(path).unwrap();
        store.insert(day(5), 0, "Gift".into()).unwrap();
        store.insert(day(5), 1, "Card".into()).unwrap();
        store.insert(day(9), 0, "Later".into()).unwrap();
        store.set_reminder(day(5), 0, Some(days_before(2))).unwrap();
        store.set_reminder(day(9), 0, Some(days_before(8))).unwrap();
        assert!(store.reminded_on(today(), today(), noon()).is_empty(), "not yet");
        assert_eq!(store.reminded_on(day(1), today(), noon()), [(day(9), 0)]);
        assert_eq!(store.reminded_on(day(3), today(), noon()), [(day(5), 0), (day(9), 0)], "soonest first");
        assert_eq!(store.reminded_on(day(4), today(), noon()), [(day(5), 0), (day(9), 0)]);
        assert_eq!(store.reminded_on(day(5), today(), noon()), [(day(9), 0)], "on its own day it's there anyway");
        // Not on days before today, and not once it's completed.
        assert!(store.reminded_on(day(3), day(4), noon()).is_empty());
        store.toggle_done(day(5), 0).unwrap();
        assert_eq!(store.reminded_on(day(3), today(), noon()), [(day(9), 0)]);
    }

    #[test]
    fn a_reminder_due_later_today_waits_for_its_time() {
        let (_dir, path) = temp_path();
        let mut store = Store::open(path).unwrap();
        store.insert(day(1), 0, "Call".into()).unwrap();
        // 2 hours before 1:00 tomorrow is 23:00 today.
        store.set_deadline(day(1), 0, Some(due(1, Some((1, 0))))).unwrap();
        store.set_reminder(day(1), 0, Some(Reminder::Before { amount: 2, unit: crate::reminder::Unit::Hours })).unwrap();
        assert!(store.reminded_on(today(), today(), noon()).is_empty());
        let late = chrono::NaiveTime::from_hms_opt(23, 0, 0).unwrap();
        assert_eq!(store.reminded_on(today(), today(), late), [(day(1), 0)]);
    }

    #[test]
    fn a_deadline_that_cant_be_read_is_dropped_not_an_error() {
        let (_dir, path) = temp_path();
        fs::write(&path, r#"{ "2026-10-05": [{ "text": "Report", "deadline": "someday" }] }"#).unwrap();
        let store = Store::open(path).unwrap();
        assert_eq!(store.items(today())[0].text, "Report");
        assert_eq!(store.items(today())[0].deadline, None);
    }

    #[test]
    fn setting_a_deadline_unpins_and_removing_it_leaves_it_unpinned() {
        let (_dir, path) = temp_path();
        let mut store = Store::open(path).unwrap();
        store.insert(today(), 0, "Report".into()).unwrap();
        store.toggle_pinned(today(), 0).unwrap();
        store.set_deadline(today(), 0, Some(due(2, None))).unwrap();
        assert!(!store.items(today())[0].pinned);
        store.set_deadline(today(), 0, None).unwrap();
        assert!(!store.items(today())[0].pinned);
    }

    #[test]
    fn items_with_a_deadline_show_on_later_days_up_to_it() {
        let (_dir, path) = temp_path();
        let mut store = Store::open(path).unwrap();
        store.insert(today(), 0, "Report".into()).unwrap();
        store.set_deadline(today(), 0, Some(due(2, Some((9, 0))))).unwrap();
        assert_eq!(store.carried_to(day(1), today()), [(today(), 0)]);
        assert_eq!(store.carried_to(day(2), today()), [(today(), 0)], "on its deadline day");
        assert!(store.carried_to(day(3), today()).is_empty(), "not after it");
        // Overdue by then, it shows on today, but still not on the days ahead.
        assert_eq!(store.carried_to(day(3), day(3)), [(today(), 0)]);
        assert!(store.carried_to(day(4), day(3)).is_empty());
        // Once done, it stays on its day.
        store.toggle_done(today(), 0).unwrap();
        assert!(store.carried_to(day(1), today()).is_empty());
    }

    #[test]
    fn roll_over_moves_items_with_a_deadline_to_today_overdue_or_not() {
        let (_dir, path) = temp_path();
        let mut store = Store::open(path).unwrap();
        // Due later: moves to today, like a pinned item.
        store.insert(day(-3), 0, "due later".into()).unwrap();
        store.set_deadline(day(-3), 0, Some(due(2, None))).unwrap();
        // Overdue since yesterday, or since before the day it's on: today too,
        // so what's late is in front of you.
        store.insert(day(-3), 1, "due yesterday".into()).unwrap();
        store.set_deadline(day(-3), 1, Some(due(-1, Some((17, 0))))).unwrap();
        store.insert(day(-2), 0, "due before".into()).unwrap();
        store.set_deadline(day(-2), 0, Some(due(-5, None))).unwrap();
        store.insert(day(-3), 2, "plain".into()).unwrap();
        store.insert(day(-3), 3, "done".into()).unwrap();
        store.set_deadline(day(-3), 3, Some(due(-4, None))).unwrap();
        store.toggle_done(day(-3), 3).unwrap();
        store.roll_over(today()).unwrap();
        assert_eq!(texts(&store, today()), ["due later", "due yesterday", "due before"], "oldest day first");
        assert!(store.items(day(-1)).is_empty());
        assert_eq!(texts(&store, day(-3)), ["plain", "done"], "the rest stay");
        // They keep moving until done.
        store.roll_over(day(1)).unwrap();
        assert_eq!(texts(&store, day(1)), ["due later", "due yesterday", "due before"]);
    }

    #[test]
    fn insert_items_adds_open_copies_among_the_open_items() {
        let (_dir, path) = temp_path();
        let mut store = Store::open(path).unwrap();
        store.insert(today(), 0, "a".into()).unwrap();
        store.insert(today(), 1, "b".into()).unwrap();
        store.toggle_done(today(), 1).unwrap();
        let copy = Item {
            text: "copy".into(),
            notes: "n".into(),
            notes_file: Some("original.md".into()),
            done: true,
            pinned: true,
            priority: Some(Priority::High),
            deadline: Some(Deadline { date: today(), time: None }),
            reminder: Some(Reminder::Before { amount: 2, unit: crate::reminder::Unit::Days }),
            depth: 0,
            folded: false,
            time: None,
        };
        assert_eq!(store.insert_items(today(), 9, vec![copy.clone(), copy.clone()]).unwrap(), 1);
        assert_eq!(texts(&store, today()), ["a", "copy", "copy", "b"]);
        // Open, with the same notes, pin and priority, but notes files of their own.
        let pasted = &store.items(today())[1..3];
        assert_eq!(pasted[0], Item { done: false, notes_file: Some("copy.md".into()), ..copy.clone() });
        assert_eq!(pasted[1].notes_file.as_deref(), Some("copy-2.md"));
    }

    /// A store with ["a", "b", "c", "d" (done), "e" (done)] today.
    fn five_items() -> (tempfile::TempDir, Store) {
        let (dir, path) = temp_path();
        let mut store = Store::open(path).unwrap();
        for (i, text) in ["a", "b", "c", "d", "e"].into_iter().enumerate() {
            store.insert(today(), i, text.into()).unwrap();
        }
        store.set_done_many(today(), &[3, 4], true).unwrap();
        (dir, store)
    }

    #[test]
    fn set_done_many_moves_items_to_the_boundary_in_order() {
        let (_dir, mut store) = five_items();
        assert_eq!(texts(&store, today()), ["a", "b", "c", "d", "e"]);
        assert_eq!(store.open_count(today()), 3);
        store.set_done_many(today(), &[0, 2], true).unwrap();
        assert_eq!(texts(&store, today()), ["b", "a", "c", "d", "e"]);
        assert_eq!(store.open_count(today()), 1);
        store.set_done_many(today(), &[1, 4], false).unwrap();
        assert_eq!(texts(&store, today()), ["b", "a", "e", "c", "d"]);
        assert_eq!(store.open_count(today()), 3);
        // Items already in the requested state stay where they are.
        store.set_done_many(today(), &[0, 3], true).unwrap();
        assert_eq!(texts(&store, today()), ["a", "e", "b", "c", "d"]);
    }

    #[test]
    fn update_many_changes_only_the_given_items() {
        let (_dir, mut store) = five_items();
        store.update_many(today(), &[1, 3, 99], |item| item.pinned = true).unwrap();
        let pinned: Vec<bool> = store.items(today()).iter().map(|item| item.pinned).collect();
        assert_eq!(pinned, [false, true, false, true, false]);
    }

    #[test]
    fn remove_many_returns_the_items_in_order() {
        let (_dir, mut store) = five_items();
        let removed = store.remove_many(today(), &[3, 0]).unwrap();
        assert_eq!(removed.iter().map(|item| item.text.as_str()).collect::<Vec<_>>(), ["a", "d"]);
        assert_eq!(texts(&store, today()), ["b", "c", "e"]);
        store.remove_many(today(), &[0, 1, 2]).unwrap();
        assert!(store.all().next().is_none());
    }

    #[test]
    fn move_many_keeps_open_items_above_completed_ones() {
        let (_dir, mut store) = five_items();
        store.insert(day(1), 0, "t".into()).unwrap();
        store.insert(day(1), 1, "t done".into()).unwrap();
        store.toggle_done(day(1), 1).unwrap();
        assert_eq!(store.move_many(today(), &[3, 1], day(1)).unwrap(), Some(1));
        assert_eq!(texts(&store, day(1)), ["t", "b", "d", "t done"]);
        assert_eq!(store.open_count(day(1)), 2);
        assert_eq!(texts(&store, today()), ["a", "c", "e"]);
        assert_eq!(store.move_many(today(), &[], day(1)).unwrap(), None);
    }

    #[test]
    fn notes_from_before_files_move_into_files_when_opened() {
        let (_dir, path) = temp_path();
        fs::write(&path, r#"{ "2026-10-05": [{ "text": "Write report", "notes": "Ask for numbers\n- charts" }, "plain"] }"#)
            .unwrap();
        let store = Store::open(path.clone()).unwrap();
        assert_eq!(store.items(today())[0].notes, "Ask for numbers\n- charts");
        let json: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(json, serde_json::json!({ "2026-10-05": [{ "text": "Write report", "notes_file": "write-report.md" }, "plain"] }));
        assert_eq!(fs::read_to_string(store.notes_dir().join("write-report.md")).unwrap(), "Ask for numbers\n- charts");
        // And reopening reads them back from the file.
        assert_eq!(Store::open(path).unwrap().items(today())[0].notes, "Ask for numbers\n- charts");
    }

    #[test]
    fn reload_notes_picks_up_changes_made_elsewhere() {
        let (_dir, path) = temp_path();
        let mut store = Store::open(path).unwrap();
        store.insert(today(), 0, "Buy milk".into()).unwrap();
        store.set_notes(today(), 0, "oat".into()).unwrap();
        fs::write(store.notes_dir().join("buy-milk.md"), "edited elsewhere").unwrap();
        store.reload_notes(today(), 0).unwrap();
        assert_eq!(store.items(today())[0].notes, "edited elsewhere");
        // A file deleted elsewhere means no notes.
        fs::remove_file(store.notes_dir().join("buy-milk.md")).unwrap();
        store.reload_notes(today(), 0).unwrap();
        assert_eq!(store.items(today())[0].notes, "");
        assert_eq!(store.items(today())[0].notes_file, None);
    }

    #[test]
    fn another_editor_edits_the_notes_file_in_place() {
        let (_dir, path) = temp_path();
        let mut store = Store::open(path.clone()).unwrap();
        store.insert(today(), 0, "Buy milk".into()).unwrap();
        store.set_notes(today(), 0, "oat".into()).unwrap();
        let file = store.notes_path(today(), 0).unwrap().unwrap();
        assert_eq!(file, store.notes_dir().join("buy-milk.md"));
        fs::write(&file, "oat\nsoy").unwrap();
        store.notes_edited(today(), 0).unwrap();
        assert_eq!(store.items(today())[0].notes, "oat\nsoy");
        // It's todoro's own version now, so nothing conflicts on later saves.
        store.insert(today(), 1, "more".into()).unwrap();
        assert!(store.take_conflicts().is_empty());
        assert_eq!(fs::read_to_string(&file).unwrap(), "oat\nsoy");
        assert_eq!(store.notes_path(today(), 5).unwrap(), None);
    }

    #[test]
    fn an_item_without_notes_gets_a_file_for_another_editor() {
        let (_dir, path) = temp_path();
        let mut store = Store::open(path.clone()).unwrap();
        store.insert(today(), 0, "Call Sam".into()).unwrap();
        let file = store.notes_path(today(), 0).unwrap().unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "");
        fs::write(&file, "about the trip").unwrap();
        store.notes_edited(today(), 0).unwrap();
        assert_eq!(store.items(today())[0].notes, "about the trip");
        let json: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(json, serde_json::json!({ "2026-10-05": [{ "text": "Call Sam", "notes_file": "call-sam.md" }] }));
    }

    #[test]
    fn a_notes_file_left_empty_by_another_editor_goes() {
        let (_dir, path) = temp_path();
        let mut store = Store::open(path.clone()).unwrap();
        store.insert(today(), 0, "Call Sam".into()).unwrap();
        let file = store.notes_path(today(), 0).unwrap().unwrap();
        store.notes_edited(today(), 0).unwrap();
        assert!(!file.exists());
        assert_eq!(store.items(today())[0].notes_file, None);
        // Notes emptied in the other editor go too.
        store.set_notes(today(), 0, "x".into()).unwrap();
        let file = store.notes_path(today(), 0).unwrap().unwrap();
        fs::write(&file, "").unwrap();
        store.notes_edited(today(), 0).unwrap();
        assert!(!file.exists());
        assert!(!fs::read_to_string(&path).unwrap().contains("notes_file"));
    }

    #[test]
    fn undoing_a_delete_brings_back_the_notes_file() {
        let (_dir, path) = temp_path();
        let mut store = Store::open(path).unwrap();
        store.insert(today(), 0, "Buy milk".into()).unwrap();
        store.set_notes(today(), 0, "oat".into()).unwrap();
        let before = store.snapshot();
        store.remove_many(today(), &[0]).unwrap();
        assert!(!store.notes_dir().join("buy-milk.md").exists());
        store.restore(before).unwrap();
        assert_eq!(fs::read_to_string(store.notes_dir().join("buy-milk.md")).unwrap(), "oat");
    }

    #[test]
    fn a_conflict_is_reported_once() {
        let (_dir, path) = temp_path();
        let mut store = Store::open(path).unwrap();
        store.insert(today(), 0, "Buy milk".into()).unwrap();
        store.set_notes(today(), 0, "oat".into()).unwrap();
        fs::write(store.notes_dir().join("buy-milk.md"), "edited elsewhere").unwrap();
        store.set_notes(today(), 0, "edited here".into()).unwrap();
        let conflicts = store.take_conflicts();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(store.items(today())[0].notes, "edited elsewhere");
        assert!(store.take_conflicts().is_empty());
    }

    #[test]
    fn roll_over_with_nothing_to_move_does_not_write_the_file() {
        let (_dir, path) = temp_path();
        let mut store = Store::open(path.clone()).unwrap();
        store.roll_over(today()).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn removing_the_last_item_drops_the_day() {
        let (_dir, path) = temp_path();
        let mut store = Store::open(path.clone()).unwrap();
        store.insert(today(), 0, "one".into()).unwrap();
        store.remove_many(today(), &[0]).unwrap();
        assert_eq!(fs::read_to_string(path).unwrap(), "{}");
    }

    #[test]
    fn out_of_range_index_is_ignored() {
        let (_dir, path) = temp_path();
        let mut store = Store::open(path).unwrap();
        store.insert(today(), 0, "one".into()).unwrap();
        store.insert(today(), 99, "two".into()).unwrap();
        store.set_text(today(), 99, "nope".into()).unwrap();
        store.set_notes(today(), 99, "nope".into()).unwrap();
        store.remove_many(today(), &[99]).unwrap();
        assert_eq!(texts(&store, today()), ["one", "two"]);
    }

    #[test]
    fn save_creates_missing_directories_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("todos.json");
        let mut store = Store::open(path.clone()).unwrap();
        store.insert(today(), 0, "one".into()).unwrap();
        assert!(path.exists());
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn corrupt_file_is_an_error_and_is_left_untouched() {
        let (_dir, path) = temp_path();
        fs::write(&path, "not json").unwrap();
        assert!(Store::open(path.clone()).is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "not json");
    }

    /// A store holding `json` as its file.
    fn store_with(json: &str) -> (tempfile::TempDir, PathBuf, Store) {
        let (dir, path) = temp_path();
        fs::write(&path, json).unwrap();
        let store = Store::open(path.clone()).unwrap();
        (dir, path, store)
    }

    /// The items on `day` as an outline: two spaces a level, `x ` if done.
    fn outline_of(store: &Store, day: NaiveDate) -> Vec<String> {
        store
            .items(day)
            .iter()
            .map(|item| format!("{}{}{}", "  ".repeat(item.depth), if item.done { "x " } else { "" }, item.text))
            .collect()
    }

    const TRIP: &str = r#"{ "2026-10-05": [
        { "text": "trip", "items": ["flights", { "text": "hotel", "items": ["compare", "book"] }] },
        "milk"
    ] }"#;

    #[test]
    fn sub_items_are_loaded_in_outline_order_and_saved_nested() {
        let (_dir, path, mut store) = store_with(TRIP);
        assert_eq!(outline_of(&store, today()), ["trip", "  flights", "  hotel", "    compare", "    book", "milk"]);
        store.set_folded(today(), 2, true).unwrap();
        let json: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "2026-10-05": [
                { "text": "trip", "items": ["flights", { "text": "hotel", "folded": true, "items": ["compare", "book"] }] },
                "milk",
            ] })
        );
        // And read back the same.
        let reopened = Store::open(path).unwrap();
        assert_eq!(outline_of(&reopened, today()), outline_of(&store, today()));
        assert!(reopened.items(today())[2].folded);
    }

    #[test]
    fn completing_an_item_completes_its_sub_items_and_moves_the_block() {
        let (_dir, _path, mut store) = store_with(TRIP);
        store.toggle_done(today(), 0).unwrap();
        assert_eq!(outline_of(&store, today()), ["milk", "x trip", "  x flights", "  x hotel", "    x compare", "    x book"]);
        assert_eq!(store.open_count(today()), 1);
        // Completing a sub-item leaves it in place, and its parent open.
        let (_dir, _path, mut store) = store_with(TRIP);
        store.toggle_done(today(), 2).unwrap();
        assert_eq!(outline_of(&store, today()), ["trip", "  flights", "  x hotel", "    x compare", "    x book", "milk"]);
    }

    #[test]
    fn reopening_a_sub_item_reopens_what_its_under() {
        let (_dir, _path, mut store) = store_with(TRIP);
        store.toggle_done(today(), 0).unwrap();
        // ["milk", "x trip", "  x flights", "  x hotel", "    x compare", "    x book"]
        store.toggle_done(today(), 4).unwrap();
        assert_eq!(outline_of(&store, today()), ["milk", "trip", "  x flights", "  hotel", "    compare", "    x book"]);
    }

    #[test]
    fn an_open_sub_item_added_under_a_completed_item_reopens_it() {
        let (_dir, _path, mut store) = store_with(TRIP);
        store.toggle_done(today(), 0).unwrap();
        let at = store.insert_at(today(), 6, vec![Item { text: "pack".into(), ..Item::default() }], 1).unwrap();
        assert_eq!(outline_of(&store, today()), ["milk", "trip", "  x flights", "  x hotel", "    x compare", "    x book", "  pack"]);
        assert_eq!(at, 6);
    }

    #[test]
    fn removing_and_moving_take_the_sub_items_along() {
        let (_dir, _path, mut store) = store_with(TRIP);
        let removed = store.remove_many(today(), &[2, 3]).unwrap();
        // The removed block starts at the top level, ready to paste.
        assert_eq!(removed.iter().map(|item| (item.text.as_str(), item.depth)).collect::<Vec<_>>(), [("hotel", 0), ("compare", 1), ("book", 1)]);
        assert_eq!(outline_of(&store, today()), ["trip", "  flights", "milk"]);
        let (_dir, _path, mut store) = store_with(TRIP);
        assert_eq!(store.move_to(today(), 2, day(1)).unwrap(), Some(0));
        assert_eq!(outline_of(&store, day(1)), ["hotel", "  compare", "  book"]);
        assert_eq!(outline_of(&store, today()), ["trip", "  flights", "milk"]);
    }

    #[test]
    fn sub_items_move_among_their_siblings_with_what_is_under_them() {
        let (_dir, _path, mut store) = store_with(TRIP);
        assert_eq!(store.move_sibling(today(), 1, true).unwrap(), Some(4));
        assert_eq!(outline_of(&store, today()), ["trip", "  hotel", "    compare", "    book", "  flights", "milk"]);
        assert_eq!(store.move_sibling(today(), 4, true).unwrap(), None, "the last under trip");
        assert_eq!(store.move_sibling(today(), 1, false).unwrap(), None, "the first under trip");
        assert_eq!(store.move_sibling(today(), 0, true).unwrap(), Some(1));
        assert_eq!(outline_of(&store, today()), ["milk", "trip", "  hotel", "    compare", "    book", "  flights"]);
    }

    #[test]
    fn indenting_and_outdenting() {
        let (_dir, _path, mut store) = store_with(TRIP);
        // milk goes under trip, as its last sub-item.
        assert_eq!(store.indent(today(), 5).unwrap(), Some(5));
        assert_eq!(outline_of(&store, today()), ["trip", "  flights", "  hotel", "    compare", "    book", "  milk"]);
        // The first item, or the first under its parent, has nothing to go under.
        assert_eq!(store.indent(today(), 0).unwrap(), None);
        assert_eq!(store.indent(today(), 1).unwrap(), None);
        // hotel goes under flights, taking its own sub-items.
        assert_eq!(store.indent(today(), 2).unwrap(), Some(2));
        assert_eq!(outline_of(&store, today()), ["trip", "  flights", "    hotel", "      compare", "      book", "  milk"]);
        // Out again goes after the rest of its parent's sub-items.
        assert_eq!(store.outdent(today(), 2).unwrap(), Some(2));
        assert_eq!(store.outdent(today(), 3).unwrap(), Some(4));
        assert_eq!(outline_of(&store, today()), ["trip", "  flights", "  hotel", "    book", "  compare", "  milk"]);
        assert_eq!(store.outdent(today(), 0).unwrap(), None, "already at the top");
    }

    #[test]
    fn indenting_under_a_folded_item_unfolds_it() {
        let (_dir, _path, mut store) = store_with(TRIP);
        store.set_folded(today(), 0, true).unwrap();
        store.indent(today(), 5).unwrap();
        assert!(!store.items(today())[0].folded);
    }

    #[test]
    fn pinned_items_carry_over_with_their_sub_items() {
        let (_dir, _path, mut store) = store_with(
            r#"{ "2026-10-03": [{ "text": "trip", "pinned": true, "items": ["flights", { "text": "pinned sub", "pinned": true }] }, "stays"] }"#,
        );
        assert_eq!(store.carried_to(today(), today()), [(day(-2), 0)], "only the top-level item");
        store.roll_over(today()).unwrap();
        assert_eq!(outline_of(&store, today()), ["trip", "  flights", "  pinned sub"]);
        assert_eq!(outline_of(&store, day(-2)), ["stays"]);
    }

    #[test]
    fn a_completed_parent_with_open_sub_items_in_the_file_is_open() {
        let (_dir, _path, store) = store_with(r#"{ "2026-10-05": [{ "text": "trip", "done": true, "items": ["flights"] }, "milk"] }"#);
        assert_eq!(outline_of(&store, today()), ["trip", "  flights", "milk"]);
    }

    #[test]
    fn times_are_saved_and_loaded() {
        let (_dir, path) = temp_path();
        let mut store = Store::open(path.clone()).unwrap();
        store.insert(today(), 0, "Standup".into()).unwrap();
        let slot = TimeSlot {
            date: today(),
            start: chrono::NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            end: chrono::NaiveTime::from_hms_opt(9, 30, 0).unwrap(),
        };
        store.set_time(today(), 0, Some(slot)).unwrap();
        let json: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(json, serde_json::json!({ "2026-10-05": [{ "text": "Standup", "time": "2026-10-05 09:00-09:30" }] }));
        assert_eq!(Store::open(path.clone()).unwrap().items(today())[0].time, Some(slot));
        // Without one it's plain text again.
        store.set_time(today(), 0, None).unwrap();
        let json: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(json["2026-10-05"][0], "Standup");
    }
}
