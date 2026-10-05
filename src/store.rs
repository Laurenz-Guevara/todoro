use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::PathBuf;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// One todo entry. `notes` is longer free text shown only on the notes screen.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(from = "RawItem", into = "RawItem")]
pub struct Item {
    pub text: String,
    pub notes: String,
}

/// How an item is written to disk. Items without notes stay plain strings, so
/// files from before notes existed load unchanged and remain readable by older
/// versions until notes are added.
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum RawItem {
    Text(String),
    Full { text: String, notes: String },
}

impl From<RawItem> for Item {
    fn from(raw: RawItem) -> Self {
        match raw {
            RawItem::Text(text) => Item { text, notes: String::new() },
            RawItem::Full { text, notes } => Item { text, notes },
        }
    }
}

impl From<Item> for RawItem {
    fn from(item: Item) -> Self {
        if item.notes.is_empty() {
            RawItem::Text(item.text)
        } else {
            RawItem::Full { text: item.text, notes: item.notes }
        }
    }
}

/// Todo items keyed by day, persisted as JSON.
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
        let days = match fs::read_to_string(&path) {
            Ok(s) => serde_json::from_str(&s).map_err(io::Error::other)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(e),
        };
        Ok(Self { path, days })
    }

    pub fn items(&self, day: NaiveDate) -> &[Item] {
        self.days.get(&key(day)).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn insert(&mut self, day: NaiveDate, index: usize, text: String) -> io::Result<()> {
        let items = self.days.entry(key(day)).or_default();
        items.insert(index.min(items.len()), Item { text, notes: String::new() });
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
