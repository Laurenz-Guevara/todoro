//! Saving the day's view (`Space e`, then `w`) as a text file to send to
//! someone: every item on the list with its sub-items and notes, as plain
//! UTF-8 text that any editor or email opens cleanly.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::NaiveDate;

use crate::app::App;
use crate::outline;
use crate::store::Item;
use crate::workspaces::expand_path;

/// The folder in the todoro folder that exports go in unless another is typed.
pub const EXPORT_DIR: &str = "export";

/// A rule between items, as the view draws one.
const RULE: &str = "----------------------------------------";

/// The day on screen as text: the date, then each top-level item as the
/// view shows it, numbered as on the list (`[x]` for a completed one), its
/// notes as written, and its sub-items under it (`-`, or `- [x]` once
/// completed), each with its notes. Folded sub-items are included, as in
/// the view.
pub fn day_text(app: &App) -> String {
    let mut out = app.day.format("%A %-d %B %Y").to_string();
    out.push('\n');
    let items = app.items();
    let mut number = 0;
    let mut any = false;
    for (row, slot) in app.slots().into_iter().enumerate() {
        if items[row].depth > 0 {
            continue;
        }
        out.push('\n');
        if any {
            out.push_str(RULE);
            out.push_str("\n\n");
        }
        any = true;
        let label = if items[row].done {
            "[x] ".to_string()
        } else {
            number += 1;
            format!("{number}. ")
        };
        block(app.store.items(slot.day), slot.index, &label, &mut out);
    }
    if !any {
        out.push_str("\nNothing on this day.\n");
    }
    out
}

/// An item and its whole block: its title after `label`, then its notes,
/// then each sub-item indented by depth, with its notes under its text.
fn block(items: &[Item], root: usize, label: &str, out: &mut String) {
    let depth = items[root].depth;
    for item in &items[root..outline::block_end(items, root)] {
        let (title, notes_indent) = if item.depth == depth {
            (format!("{label}{}", item.text), String::new())
        } else {
            let pad = "  ".repeat(item.depth - depth - 1);
            let bullet = if item.done { "- [x] " } else { "- " };
            out.push('\n');
            (format!("{pad}{bullet}{}", item.text), format!("{pad}  "))
        };
        out.push_str(&title);
        out.push('\n');
        let notes = item.notes.trim_end();
        if !notes.trim().is_empty() {
            out.push('\n');
            for line in notes.lines() {
                if line.trim().is_empty() {
                    out.push('\n');
                } else {
                    out.push_str(&format!("{notes_indent}{line}\n"));
                }
            }
        }
    }
}

/// The file name for a day's export: its date, and the workspace's name
/// when there's more than one, so they don't overwrite each other.
pub fn file_name(day: NaiveDate, workspace: Option<&str>) -> String {
    match workspace {
        Some(name) => format!("{} {name}.txt", day.format("%Y-%m-%d")),
        None => format!("{}.txt", day.format("%Y-%m-%d")),
    }
}

/// Where to save for what was typed: `default` for nothing; otherwise the
/// path typed (`~` for the home folder, relative to the current folder),
/// with `default`'s file name added if it's a folder or ends in a separator.
pub fn target(typed: &str, default: &Path) -> PathBuf {
    let typed = typed.trim();
    if typed.is_empty() {
        return default.to_path_buf();
    }
    let path = expand_path(typed);
    if typed.ends_with(['/', '\\']) || path.is_dir() {
        path.join(default.file_name().unwrap_or_default())
    } else {
        path
    }
}

/// Writes `text` to `path`, making its folder first if need be.
pub fn write(path: &Path, text: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{app_with, today};

    #[test]
    fn the_day_as_text() {
        let (mut app, _dir) = app_with(&["Buy milk", "Plan the trip", "Call mom"]);
        let day = today();
        app.store.set_notes(day, 1, "## Flights\nCheck **prices** first\n\n- Tuesday\n".into()).unwrap();
        app.store.insert_at(day, 2, vec![Item { text: "Book flights".into(), ..Item::default() }], 1).unwrap();
        app.store.set_notes(day, 2, "Window seat\nNot Ryanair".into()).unwrap();
        app.store.insert_at(day, 3, vec![Item { text: "Compare prices".into(), ..Item::default() }], 2).unwrap();
        app.store.insert_at(day, 4, vec![Item { text: "Pack".into(), ..Item::default() }], 1).unwrap();
        app.store.toggle_done(day, 4).unwrap();
        app.store.toggle_done(day, 5).unwrap();
        app.store.set_folded(day, 1, true).unwrap();
        assert_eq!(
            day_text(&app),
            "\
Monday 5 October 2026

1. Buy milk

----------------------------------------

2. Plan the trip

## Flights
Check **prices** first

- Tuesday

- Book flights

  Window seat
  Not Ryanair

  - Compare prices

- [x] Pack

----------------------------------------

[x] Call mom
"
        );
    }

    #[test]
    fn an_empty_day_says_so() {
        let (app, _dir) = app_with(&[]);
        assert_eq!(day_text(&app), "Monday 5 October 2026\n\nNothing on this day.\n");
    }

    #[test]
    fn file_names_are_the_date_and_the_workspace_when_there_are_several() {
        assert_eq!(file_name(today(), None), "2026-10-05.txt");
        assert_eq!(file_name(today(), Some("work")), "2026-10-05 work.txt");
    }

    #[test]
    fn where_to_save() {
        let dir = tempfile::tempdir().unwrap();
        let default = dir.path().join("export").join("2026-10-05.txt");
        assert_eq!(target("", &default), default);
        assert_eq!(target("   ", &default), default);
        // A file.
        let file = dir.path().join("for-sam.txt");
        assert_eq!(target(&file.display().to_string(), &default), file);
        // A folder that exists, or one ending in a separator, gets the date.
        assert_eq!(target(&dir.path().display().to_string(), &default), dir.path().join("2026-10-05.txt"));
        let new = format!("{}/shared/", dir.path().display());
        assert_eq!(target(&new, &default), dir.path().join("shared").join("2026-10-05.txt"));
    }

    #[test]
    fn writing_makes_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("export").join("2026-10-05.txt");
        write(&path, "Monday\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "Monday\n");
    }
}
