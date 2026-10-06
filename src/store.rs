use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::PathBuf;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::notes_files::{Conflict, NotesFiles};

/// One todo entry. `notes` is longer free text shown only on the notes screen,
/// kept in its own Markdown file named by `notes_file`. A `pinned` item moves
/// forward to today until it is completed; others stay on their day.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(from = "RawItem", into = "RawItem")]
pub struct Item {
    pub text: String,
    pub notes: String,
    /// The notes file, in the notes folder, once the item has notes.
    pub notes_file: Option<String>,
    pub done: bool,
    pub pinned: bool,
    pub priority: Option<Priority>,
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
    },
}

impl From<RawItem> for Item {
    fn from(raw: RawItem) -> Self {
        match raw {
            RawItem::Text(text) => Item { text, ..Item::default() },
            RawItem::Full { text, notes, notes_file, done, pinned, priority } => {
                Item { text, notes, notes_file, done, pinned, priority }
            }
        }
    }
}

impl From<Item> for RawItem {
    fn from(item: Item) -> Self {
        if item.notes_file.is_none() && !item.done && !item.pinned && item.priority.is_none() {
            RawItem::Text(item.text)
        } else {
            let Item { text, notes, notes_file, done, pinned, priority } = item;
            RawItem::Full { text, notes, notes_file, done, pinned, priority }
        }
    }
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
        let mut days: BTreeMap<String, Vec<Item>> = match fs::read_to_string(&path) {
            Ok(s) => serde_json::from_str(&s).map_err(io::Error::other)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(e),
        };
        // A hand-edited file may mix open and completed items. Stable sort keeps
        // each group in its original order.
        for items in days.values_mut() {
            items.sort_by_key(|item| item.done);
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

    /// Notes files that saves left alone because they changed outside todoro.
    pub fn take_conflicts(&mut self) -> Vec<Conflict> {
        std::mem::take(&mut self.conflicts)
    }

    /// Moves every pinned, open item from days before `today` to the start of
    /// today's list, oldest day first. They stay pinned, so they keep moving
    /// forward until completed. Everything else stays on its day.
    pub fn roll_over(&mut self, today: NaiveDate) -> io::Result<()> {
        let today_key = key(today);
        let mut carried = Vec::new();
        self.days.retain(|day, items| {
            if *day < today_key {
                carried.extend(items.extract_if(.., |item| item.pinned && !item.done));
            }
            !items.is_empty()
        });
        if carried.is_empty() {
            return Ok(());
        }
        let items = self.days.entry(today_key).or_default();
        items.splice(0..0, carried);
        self.save()
    }

    /// Where the pinned, open items from days before `day` are, as `(day,
    /// index)`, oldest day first. These are what `roll_over` would move to `day`.
    pub fn pinned_before(&self, day: NaiveDate) -> Vec<(NaiveDate, usize)> {
        self.days
            .range(..key(day))
            .filter_map(|(k, items)| Some((NaiveDate::parse_from_str(k, "%Y-%m-%d").ok()?, items)))
            .flat_map(|(date, items)| {
                items.iter().enumerate().filter(|(_, item)| item.pinned && !item.done).map(move |(i, _)| (date, i))
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

    /// How many items on `day` are not completed. They come first in `items`.
    pub fn open_count(&self, day: NaiveDate) -> usize {
        self.items(day).iter().take_while(|item| !item.done).count()
    }

    /// Every day that has items, oldest first, with its items.
    pub fn all(&self) -> impl Iterator<Item = (NaiveDate, &[Item])> {
        self.days.iter().filter_map(|(k, items)| Some((NaiveDate::parse_from_str(k, "%Y-%m-%d").ok()?, items.as_slice())))
    }

    pub fn items(&self, day: NaiveDate) -> &[Item] {
        self.days.get(&key(day)).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn insert(&mut self, day: NaiveDate, index: usize, text: String) -> io::Result<()> {
        self.insert_items(day, index, vec![Item { text, ..Item::default() }]).map(|_| ())
    }

    /// Inserts `items` at `index` as open items, keeping their notes, pin and
    /// priority. Returns where the first one went.
    pub fn insert_items(&mut self, day: NaiveDate, index: usize, items: Vec<Item>) -> io::Result<usize> {
        // New items are open, so they always go among the open ones.
        let index = index.min(self.open_count(day));
        let day_items = self.days.entry(key(day)).or_default();
        // Copies get notes files of their own.
        day_items.splice(index..index, items.into_iter().map(|item| Item { done: false, notes_file: None, ..item }));
        self.save()?;
        Ok(index)
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

    /// Completes an open item or reopens a completed one. Either way it moves to
    /// the boundary between the two groups: the top of the completed items, or
    /// the bottom of the open ones.
    pub fn toggle_done(&mut self, day: NaiveDate, index: usize) -> io::Result<()> {
        let Some(items) = self.days.get_mut(&key(day)) else { return Ok(()) };
        if index >= items.len() {
            return Ok(());
        }
        let mut item = items.remove(index);
        item.done = !item.done;
        let boundary = items.iter().take_while(|item| !item.done).count();
        items.insert(boundary, item);
        self.save()
    }

    pub fn toggle_pinned(&mut self, day: NaiveDate, index: usize) -> io::Result<()> {
        if let Some(item) = self.item_mut(day, index) {
            item.pinned = !item.pinned;
        }
        self.save()
    }

    /// Moves an item to another day, where it goes at the boundary between the
    /// open and completed items, like `toggle_done`. Returns its new index.
    pub fn move_to(&mut self, day: NaiveDate, index: usize, to: NaiveDate) -> io::Result<Option<usize>> {
        let from = key(day);
        let Some(items) = self.days.get_mut(&from) else { return Ok(None) };
        if index >= items.len() {
            return Ok(None);
        }
        let item = items.remove(index);
        if items.is_empty() {
            self.days.remove(&from);
        }
        let items = self.days.entry(key(to)).or_default();
        let boundary = items.iter().take_while(|item| !item.done).count();
        items.insert(boundary, item);
        self.save()?;
        Ok(Some(boundary))
    }

    /// Completes or reopens the items at `indices` on `day`. Like `toggle_done`,
    /// they move to the boundary between open and completed items, keeping
    /// their order.
    pub fn set_done_many(&mut self, day: NaiveDate, indices: &[usize], done: bool) -> io::Result<()> {
        let Some(items) = self.days.get_mut(&key(day)) else { return Ok(()) };
        let mut changed = Vec::new();
        let mut rest = Vec::new();
        for (i, item) in std::mem::take(items).into_iter().enumerate() {
            if indices.contains(&i) && item.done != done {
                changed.push(Item { done, ..item });
            } else {
                rest.push(item);
            }
        }
        let boundary = rest.iter().take_while(|item| !item.done).count();
        rest.splice(boundary..boundary, changed);
        *items = rest;
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

    /// Removes the items at `indices` on `day` and returns them, in order.
    pub fn remove_many(&mut self, day: NaiveDate, indices: &[usize]) -> io::Result<Vec<Item>> {
        let k = key(day);
        let Some(items) = self.days.get_mut(&k) else { return Ok(Vec::new()) };
        let mut removed = Vec::new();
        let mut kept = Vec::new();
        for (i, item) in std::mem::take(items).into_iter().enumerate() {
            if indices.contains(&i) { removed.push(item) } else { kept.push(item) }
        }
        if kept.is_empty() {
            self.days.remove(&k);
        } else {
            *items = kept;
        }
        self.save()?;
        Ok(removed)
    }

    /// Moves the items at `indices` on `day` to the day `to`, at the boundary
    /// between its open and completed items like `move_to`, keeping their
    /// order. Returns where the first one went.
    pub fn move_many(&mut self, day: NaiveDate, indices: &[usize], to: NaiveDate) -> io::Result<Option<usize>> {
        let mut moved = self.remove_many(day, indices)?;
        if moved.is_empty() {
            return Ok(None);
        }
        // Open ones first, so the completed ones stay after them.
        moved.sort_by_key(|item| item.done);
        let items = self.days.entry(key(to)).or_default();
        let boundary = items.iter().take_while(|item| !item.done).count();
        items.splice(boundary..boundary, moved);
        self.save()?;
        Ok(Some(boundary))
    }

    /// Swaps two items on `day` if both are open or both completed, so the
    /// open items always stay above the completed ones. Returns whether it did.
    pub fn swap(&mut self, day: NaiveDate, a: usize, b: usize) -> io::Result<bool> {
        let Some(items) = self.days.get_mut(&key(day)) else { return Ok(false) };
        let same_group = matches!((items.get(a), items.get(b)), (Some(x), Some(y)) if x.done == y.done);
        if !same_group || a == b {
            return Ok(false);
        }
        items.swap(a, b);
        self.save()?;
        Ok(true)
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
        fs::write(&tmp, serde_json::to_string_pretty(&self.days).map_err(io::Error::other)?)?;
        fs::rename(tmp, &self.path)
    }
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
    fn pinned_before_lists_what_roll_over_would_move() {
        let (_dir, path) = temp_path();
        let mut store = Store::open(path).unwrap();
        insert_pinned(&mut store, day(-1), "older");
        store.insert(today(), 0, "not pinned".into()).unwrap();
        insert_pinned(&mut store, today(), "today");
        insert_pinned(&mut store, today(), "done");
        store.toggle_done(today(), 2).unwrap();
        insert_pinned(&mut store, day(2), "later");
        assert_eq!(store.pinned_before(day(2)), [(day(-1), 0), (today(), 1)]);
        assert_eq!(store.pinned_before(day(3)), [(day(-1), 0), (today(), 1), (day(2), 0)]);
        assert!(store.pinned_before(day(-1)).is_empty());
    }

    #[test]
    fn swap_reorders_within_the_open_or_completed_items() {
        let (_dir, path) = temp_path();
        let mut store = Store::open(path.clone()).unwrap();
        for (i, text) in ["a", "b", "c", "d"].into_iter().enumerate() {
            store.insert(today(), i, text.into()).unwrap();
        }
        store.toggle_done(today(), 3).unwrap();
        store.toggle_done(today(), 2).unwrap();
        // ["a", "b", "c" (done), "d" (done)]
        assert!(store.swap(today(), 0, 1).unwrap());
        assert!(store.swap(today(), 2, 3).unwrap());
        assert_eq!(texts(&store, today()), ["b", "a", "d", "c"]);
        // Never across the boundary, past the end, or on a day with no items.
        assert!(!store.swap(today(), 1, 2).unwrap());
        assert!(!store.swap(today(), 3, 4).unwrap());
        assert!(!store.swap(day(1), 0, 1).unwrap());
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
}
