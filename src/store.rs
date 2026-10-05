use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::PathBuf;

use chrono::NaiveDate;

/// Todo items keyed by day, persisted as JSON.
pub struct Store {
    path: PathBuf,
    days: BTreeMap<String, Vec<String>>,
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

    pub fn items(&self, day: NaiveDate) -> &[String] {
        self.days.get(&key(day)).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn insert(&mut self, day: NaiveDate, index: usize, text: String) -> io::Result<()> {
        let items = self.days.entry(key(day)).or_default();
        items.insert(index.min(items.len()), text);
        self.save()
    }

    pub fn set(&mut self, day: NaiveDate, index: usize, text: String) -> io::Result<()> {
        if let Some(item) = self.days.get_mut(&key(day)).and_then(|items| items.get_mut(index)) {
            *item = text;
        }
        self.save()
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
        store.set(today(), 0, "uno".into()).unwrap();
        store.remove(today(), 2).unwrap();

        let reloaded = Store::open(path).unwrap();
        assert_eq!(reloaded.items(today()), ["uno", "middle"]);
    }

    #[test]
    fn days_are_kept_separate() {
        let (_dir, path) = temp_path();
        let tomorrow = today().succ_opt().unwrap();
        let mut store = Store::open(path).unwrap();
        store.insert(today(), 0, "today".into()).unwrap();
        store.insert(tomorrow, 0, "tomorrow".into()).unwrap();
        assert_eq!(store.items(today()), ["today"]);
        assert_eq!(store.items(tomorrow), ["tomorrow"]);
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
        store.set(today(), 99, "nope".into()).unwrap();
        store.remove(today(), 99).unwrap();
        assert_eq!(store.items(today()), ["one", "two"]);
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
