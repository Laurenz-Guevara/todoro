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
