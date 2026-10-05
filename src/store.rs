use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::PathBuf;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// One todo entry. `notes` is longer free text shown only on the notes screen.
/// A `pinned` item moves forward to today until it is completed; others stay
/// on their day.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(from = "RawItem", into = "RawItem")]
pub struct Item {
    pub text: String,
    pub notes: String,
    pub done: bool,
    pub pinned: bool,
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
        #[serde(default, skip_serializing_if = "String::is_empty")]
        notes: String,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        done: bool,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        pinned: bool,
    },
}

impl From<RawItem> for Item {
    fn from(raw: RawItem) -> Self {
        match raw {
            RawItem::Text(text) => Item { text, ..Item::default() },
            RawItem::Full { text, notes, done, pinned } => Item { text, notes, done, pinned },
        }
    }
}

impl From<Item> for RawItem {
    fn from(item: Item) -> Self {
        if item.notes.is_empty() && !item.done && !item.pinned {
            RawItem::Text(item.text)
        } else {
            RawItem::Full { text: item.text, notes: item.notes, done: item.done, pinned: item.pinned }
        }
    }
}

/// Todo items keyed by day, persisted as JSON. Each day's items are kept with
/// the open ones first and the completed ones after them.
pub struct Store {
    path: PathBuf,
    days: BTreeMap<String, Vec<Item>>,
}

impl Store {
    /// Loads from `$TODORO_FILE`, or `<data dir>/todoro/todos.json` by default.
    pub fn load() -> io::Result<Self> {
        let path = match std::env::var_os("TODORO_FILE") {
            Some(p) => PathBuf::from(p),
            None => dirs::data_dir()
                .ok_or_else(|| io::Error::other("could not determine data directory"))?
                .join("todoro")
                .join("todos.json"),
        };
        Self::open(path)
    }

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
        Ok(Self { path, days })
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

    /// How many items on `day` are not completed. They come first in `items`.
    pub fn open_count(&self, day: NaiveDate) -> usize {
        self.items(day).iter().take_while(|item| !item.done).count()
    }

    pub fn items(&self, day: NaiveDate) -> &[Item] {
        self.days.get(&key(day)).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn insert(&mut self, day: NaiveDate, index: usize, text: String) -> io::Result<()> {
        // New items are open, so they always go among the open ones.
        let index = index.min(self.open_count(day));
        let items = self.days.entry(key(day)).or_default();
        items.insert(index, Item { text, ..Item::default() });
        self.save()
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

    fn item_mut(&mut self, day: NaiveDate, index: usize) -> Option<&mut Item> {
        self.days.get_mut(&key(day)).and_then(|items| items.get_mut(index))
    }

    pub fn remove(&mut self, day: NaiveDate, index: usize) -> io::Result<()> {
        let k = key(day);
        if let Some(items) = self.days.get_mut(&k) {
            if index < items.len() {
                items.remove(index);
            }
            if items.is_empty() {
                self.days.remove(&k);
            }
        }
        self.save()
    }

    fn save(&self) -> io::Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
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
        store.remove(today(), 2).unwrap();

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
        let json: serde_json::Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "2026-10-05": ["plain", { "text": "detailed", "notes": "some notes" }] })
        );
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
                { "text": "both", "notes": "n", "done": true },
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
        store.remove(today(), 0).unwrap();
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
        store.remove(today(), 99).unwrap();
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
