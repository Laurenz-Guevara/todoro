//! Items' notes, kept as Markdown files in a folder beside the todos file so
//! they can be read, edited, synced and diffed with any tool. The todos file
//! only names each item's notes file.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::store::Item;

/// A notes file changed outside todoro while todoro had changes for it, so
/// todoro kept the outside version and saved its own beside it.
#[derive(Debug, PartialEq)]
pub struct Conflict {
    /// The notes file, which now has the outside version.
    pub file: String,
    /// Where todoro's version went instead.
    pub copy: String,
}

/// The notes folder, and what todoro last read or wrote in each file there,
/// to notice files changed by something else.
pub struct NotesFiles {
    pub dir: PathBuf,
    known: HashMap<String, String>,
}

impl NotesFiles {
    /// The notes folder for a todos file: `notes` beside a workspace's
    /// `todos.json`, or `<name>-notes` beside any other file, so a single
    /// file like `~/work.json` doesn't claim a generic `~/notes`.
    pub fn for_todos(todos: &Path) -> Self {
        let parent = todos.parent().unwrap_or(Path::new("."));
        let dir = match todos.file_name().and_then(|name| name.to_str()) {
            Some(crate::workspaces::TODOS_FILE) | None => parent.join("notes"),
            Some(_) => {
                let stem = todos.file_stem().and_then(|stem| stem.to_str()).unwrap_or("todos");
                parent.join(format!("{stem}-notes"))
            }
        };
        Self { dir, known: HashMap::new() }
    }

    /// The text in `file`, or `None` if it's gone. What's read is remembered
    /// as todoro's view of the file.
    pub fn read(&mut self, file: &str) -> io::Result<Option<String>> {
        match fs::read_to_string(self.dir.join(file)) {
            Ok(text) => {
                self.known.insert(file.to_string(), text.clone());
                Ok(Some(text))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                self.known.remove(file);
                Ok(None)
            }
            Err(e) => Err(e),
        }
    }

    /// Creates an empty notes file named after `text`, for another editor to
    /// open, and returns its name. Until it's given some text, the next sync
    /// deletes it again.
    pub fn create(&mut self, text: &str, taken: &HashSet<String>) -> io::Result<String> {
        let file = self.free_name(&slug(text), taken);
        self.write(&file, "")?;
        Ok(file)
    }

    /// Loads every item's notes from its file. An item whose file has gone
    /// has no notes any more.
    pub fn load(&mut self, days: &mut BTreeMap<String, Vec<Item>>) -> io::Result<()> {
        for item in days.values_mut().flatten() {
            if let Some(file) = item.notes_file.clone() {
                match self.read(&file)? {
                    Some(text) => item.notes = text,
                    None => {
                        item.notes.clear();
                        item.notes_file = None;
                    }
                }
            }
        }
        Ok(())
    }

    /// Makes the notes folder match the items: gives notes without a file a
    /// new one named after the item, writes files whose text changed, and
    /// deletes files no item has any more. A file changed outside todoro is
    /// never overwritten or deleted: its item takes the outside version, and
    /// todoro's goes in a copy, reported as a conflict.
    pub fn sync(&mut self, days: &mut BTreeMap<String, Vec<Item>>) -> io::Result<Vec<Conflict>> {
        let mut conflicts = Vec::new();
        // Each file belongs to one item; a second item naming it gets its own.
        let mut taken: HashSet<String> = HashSet::new();
        for item in days.values_mut().flatten() {
            if item.notes.is_empty() {
                item.notes_file = None;
            } else if let Some(file) = &item.notes_file
                && !taken.insert(file.clone())
            {
                item.notes_file = None;
            }
        }
        for item in days.values_mut().flatten() {
            if !item.notes.is_empty() && item.notes_file.is_none() {
                let file = self.free_name(&slug(&item.text), &taken);
                taken.insert(file.clone());
                item.notes_file = Some(file);
            }
        }

        for item in days.values_mut().flatten() {
            let Some(file) = item.notes_file.clone() else { continue };
            if self.known.get(&file) == Some(&item.notes) {
                continue;
            }
            match self.changed_outside(&file)? {
                Some(outside) => {
                    let copy = self.free_name(&format!("{} (conflict)", stem(&file)), &taken);
                    taken.insert(copy.clone());
                    self.write(&copy, &item.notes)?;
                    // The copy is the user's to merge or delete; todoro forgets
                    // it, so it's never tidied away as an unused file.
                    self.known.remove(&copy);
                    // The outside version stays, as this item's notes.
                    self.known.insert(file.clone(), outside.clone());
                    item.notes = outside;
                    conflicts.push(Conflict { file, copy });
                }
                None => self.write(&file, &item.notes)?,
            }
        }

        let gone: Vec<String> = self.known.keys().filter(|file| !taken.contains(*file)).cloned().collect();
        for file in gone {
            // Something else changed it since, so it isn't todoro's to delete.
            if self.changed_outside(&file)?.is_none() {
                match fs::remove_file(self.dir.join(&file)) {
                    Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                    _ => {}
                }
            }
            self.known.remove(&file);
        }
        Ok(conflicts)
    }

    /// The file's text if something other than todoro has changed it since
    /// todoro last read or wrote it (including creating it).
    fn changed_outside(&self, file: &str) -> io::Result<Option<String>> {
        let on_disk = match fs::read_to_string(self.dir.join(file)) {
            Ok(text) => Some(text),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(e),
        };
        Ok(match (on_disk, self.known.get(file)) {
            (Some(text), Some(known)) if &text != known => Some(text),
            (Some(text), None) => Some(text),
            _ => None,
        })
    }

    /// Writes a file whole, so a crash or a sync tool never sees half of it.
    fn write(&mut self, file: &str, text: &str) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        let path = self.dir.join(file);
        let tmp = self.dir.join(format!("{file}.tmp"));
        fs::write(&tmp, text)?;
        fs::rename(tmp, path)?;
        self.known.insert(file.to_string(), text.to_string());
        Ok(())
    }

    /// `<base>.md`, or `<base>-2.md` and so on, not used by another item and
    /// not a file already there that todoro doesn't know.
    fn free_name(&self, base: &str, taken: &HashSet<String>) -> String {
        (1..)
            .map(|n| if n == 1 { format!("{base}.md") } else { format!("{base}-{n}.md") })
            .find(|name| {
                !taken.contains(name) && (self.known.contains_key(name) || !self.dir.join(name).exists())
            })
            .expect("there's always a free name")
    }
}

fn stem(file: &str) -> &str {
    file.strip_suffix(".md").unwrap_or(file)
}

/// A file name from an item's text: lower case words joined by dashes, like
/// "Buy milk!" to "buy-milk". Never empty, never a name Windows reserves.
pub fn slug(text: &str) -> String {
    let mut slug = String::new();
    for c in text.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            slug.push(c);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let mut slug: String = slug.trim_end_matches('-').chars().take(60).collect();
    slug = slug.trim_end_matches('-').to_string();
    const RESERVED: [&str; 4] = ["con", "prn", "aux", "nul"];
    let reserved = RESERVED.contains(&slug.as_str()) || (slug.len() == 4 && (slug.starts_with("com") || slug.starts_with("lpt")));
    if slug.is_empty() || reserved { format!("note-{slug}").trim_end_matches('-').to_string() } else { slug }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day_with(items: Vec<Item>) -> BTreeMap<String, Vec<Item>> {
        BTreeMap::from([("2026-10-05".to_string(), items)])
    }

    fn with_notes(text: &str, notes: &str) -> Item {
        Item { text: text.into(), notes: notes.into(), ..Item::default() }
    }

    fn files(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .map(|entries| entries.map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect())
            .unwrap_or_default();
        names.sort();
        names
    }

    #[test]
    fn slugs_are_readable_file_names() {
        assert_eq!(slug("Buy milk"), "buy-milk");
        assert_eq!(slug("  Write the Q3 report!!  "), "write-the-q3-report");
        assert_eq!(slug("Café with Zoë"), "café-with-zoë");
        assert_eq!(slug("a/b\\c:d"), "a-b-c-d");
        assert_eq!(slug("!!!"), "note");
        assert_eq!(slug("CON"), "note-con");
        assert_eq!(slug(&"word ".repeat(30)).len(), 59);
    }

    #[test]
    fn a_workspace_keeps_notes_in_a_notes_folder_and_a_single_file_beside_it() {
        assert_eq!(NotesFiles::for_todos(Path::new("/w/Home/todos.json")).dir, PathBuf::from("/w/Home/notes"));
        assert_eq!(NotesFiles::for_todos(Path::new("/home/sam/work.json")).dir, PathBuf::from("/home/sam/work-notes"));
    }

    #[test]
    fn notes_are_written_to_files_named_after_their_items() {
        let dir = tempfile::tempdir().unwrap();
        let mut notes = NotesFiles::for_todos(&dir.path().join("todos.json"));
        let mut days = day_with(vec![
            with_notes("Buy milk", "oat"),
            Item { text: "No notes".into(), ..Item::default() },
            with_notes("Buy milk", "the other one"),
        ]);
        assert!(notes.sync(&mut days).unwrap().is_empty());
        let items = &days["2026-10-05"];
        assert_eq!(items[0].notes_file.as_deref(), Some("buy-milk.md"));
        assert_eq!(items[1].notes_file, None);
        assert_eq!(items[2].notes_file.as_deref(), Some("buy-milk-2.md"));
        assert_eq!(fs::read_to_string(notes.dir.join("buy-milk.md")).unwrap(), "oat");
        assert_eq!(files(&notes.dir), ["buy-milk-2.md", "buy-milk.md"]);
    }

    #[test]
    fn only_changed_notes_are_written_and_gone_ones_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let mut notes = NotesFiles::for_todos(&dir.path().join("todos.json"));
        let mut days = day_with(vec![with_notes("One", "first"), with_notes("Two", "second")]);
        notes.sync(&mut days).unwrap();
        // Change one item's notes and clear the other's.
        let items = days.get_mut("2026-10-05").unwrap();
        items[1].notes = "changed".into();
        items[0].notes.clear();
        notes.sync(&mut days).unwrap();
        assert_eq!(files(&notes.dir), ["two.md"]);
        assert_eq!(fs::read_to_string(notes.dir.join("two.md")).unwrap(), "changed");
        assert_eq!(days["2026-10-05"][0].notes_file, None);
        // Deleting the item deletes its file.
        days.get_mut("2026-10-05").unwrap().remove(1);
        notes.sync(&mut days).unwrap();
        assert!(files(&notes.dir).is_empty());
    }

    #[test]
    fn renaming_an_item_keeps_its_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut notes = NotesFiles::for_todos(&dir.path().join("todos.json"));
        let mut days = day_with(vec![with_notes("Buy milk", "oat")]);
        notes.sync(&mut days).unwrap();
        days.get_mut("2026-10-05").unwrap()[0].text = "Buy oat milk".into();
        notes.sync(&mut days).unwrap();
        assert_eq!(files(&notes.dir), ["buy-milk.md"]);
    }

    #[test]
    fn two_items_naming_the_same_file_get_one_each() {
        let dir = tempfile::tempdir().unwrap();
        let mut notes = NotesFiles::for_todos(&dir.path().join("todos.json"));
        let mut copy = with_notes("Buy milk", "oat");
        copy.notes_file = Some("buy-milk.md".into());
        let mut days = day_with(vec![copy.clone(), copy]);
        notes.sync(&mut days).unwrap();
        assert_eq!(files(&notes.dir), ["buy-milk-2.md", "buy-milk.md"]);
    }

    #[test]
    fn a_file_already_there_from_elsewhere_isnt_taken() {
        let dir = tempfile::tempdir().unwrap();
        let mut notes = NotesFiles::for_todos(&dir.path().join("todos.json"));
        fs::create_dir_all(&notes.dir).unwrap();
        fs::write(notes.dir.join("buy-milk.md"), "someone else's").unwrap();
        let mut days = day_with(vec![with_notes("Buy milk", "oat")]);
        notes.sync(&mut days).unwrap();
        assert_eq!(days["2026-10-05"][0].notes_file.as_deref(), Some("buy-milk-2.md"));
        assert_eq!(fs::read_to_string(notes.dir.join("buy-milk.md")).unwrap(), "someone else's");
    }

    #[test]
    fn a_file_changed_outside_is_kept_and_todoros_version_saved_beside_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut notes = NotesFiles::for_todos(&dir.path().join("todos.json"));
        let mut days = day_with(vec![with_notes("Buy milk", "oat")]);
        notes.sync(&mut days).unwrap();
        fs::write(notes.dir.join("buy-milk.md"), "edited in another app").unwrap();
        days.get_mut("2026-10-05").unwrap()[0].notes = "edited in todoro".into();
        let conflicts = notes.sync(&mut days).unwrap();
        assert_eq!(conflicts, [Conflict { file: "buy-milk.md".into(), copy: "buy-milk (conflict).md".into() }]);
        assert_eq!(fs::read_to_string(notes.dir.join("buy-milk.md")).unwrap(), "edited in another app");
        assert_eq!(fs::read_to_string(notes.dir.join("buy-milk (conflict).md")).unwrap(), "edited in todoro");
        // The item now has the outside version, and saving again changes nothing.
        assert_eq!(days["2026-10-05"][0].notes, "edited in another app");
        assert!(notes.sync(&mut days).unwrap().is_empty());
        assert_eq!(files(&notes.dir), ["buy-milk (conflict).md", "buy-milk.md"]);
    }

    #[test]
    fn a_file_changed_outside_is_not_deleted_with_its_item() {
        let dir = tempfile::tempdir().unwrap();
        let mut notes = NotesFiles::for_todos(&dir.path().join("todos.json"));
        let mut days = day_with(vec![with_notes("Buy milk", "oat")]);
        notes.sync(&mut days).unwrap();
        fs::write(notes.dir.join("buy-milk.md"), "edited in another app").unwrap();
        days.get_mut("2026-10-05").unwrap().clear();
        notes.sync(&mut days).unwrap();
        assert_eq!(fs::read_to_string(notes.dir.join("buy-milk.md")).unwrap(), "edited in another app");
    }

    #[test]
    fn load_reads_notes_and_forgets_files_that_are_gone() {
        let dir = tempfile::tempdir().unwrap();
        let mut notes = NotesFiles::for_todos(&dir.path().join("todos.json"));
        fs::create_dir_all(&notes.dir).unwrap();
        fs::write(notes.dir.join("buy-milk.md"), "# Oat\n- not dairy").unwrap();
        let linked = |file: &str| Item { text: "x".into(), notes_file: Some(file.into()), ..Item::default() };
        let mut days = day_with(vec![linked("buy-milk.md"), linked("gone.md")]);
        notes.load(&mut days).unwrap();
        let items = &days["2026-10-05"];
        assert_eq!(items[0].notes, "# Oat\n- not dairy");
        assert_eq!(items[1].notes, "");
        assert_eq!(items[1].notes_file, None);
    }
}
