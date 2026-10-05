//! Helpers shared by the unit tests.

use chrono::NaiveDate;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tempfile::TempDir;

use crate::app::App;
use crate::store::Store;

/// The fixed "today" all tests run on.
pub fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, 5).unwrap()
}

/// An `App` on `today()` backed by a temporary file. Keep the `TempDir`
/// alive for as long as the app is used.
pub fn app_with(items: &[&str]) -> (App, TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(dir.path().join("todos.json")).unwrap();
    for (i, item) in items.iter().enumerate() {
        store.insert(today(), i, item.to_string()).unwrap();
    }
    (App::new(store, today()), dir)
}

pub fn press(app: &mut App, code: KeyCode) {
    app.handle_key(KeyEvent::new(code, KeyModifiers::NONE)).unwrap();
}

/// Presses each character of `keys` in turn.
pub fn type_str(app: &mut App, keys: &str) {
    for c in keys.chars() {
        press(app, KeyCode::Char(c));
    }
}
