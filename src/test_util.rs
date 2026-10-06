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

/// An app on `today()` in a todoro folder with these workspaces, each with
/// one item named after it, open in the first.
pub fn app_with_workspaces(names: &[&str]) -> (App, TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let folder = crate::workspaces::Workspaces::new(dir.path().join("todoro"));
    for name in names {
        folder.create(name).unwrap();
        let mut store = Store::open(folder.todos_path(name)).unwrap();
        store.insert(today(), 0, format!("{name} item")).unwrap();
    }
    let store = Store::open(folder.todos_path(names[0])).unwrap();
    let mut app = App::new(store, today());
    app.workspaces = Some(folder);
    app.workspace = Some(names[0].to_string());
    (app, dir)
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
