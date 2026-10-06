//! Workspaces: separate sets of todos, each a folder inside the todoro folder
//! the user chose, holding its own `todos.json`. todoro never backs them up;
//! keeping everything in one plain folder makes that easy to do yourself.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The file each workspace's todos are kept in.
pub const TODOS_FILE: &str = "todos.json";

/// The todoro folder, holding one folder per workspace.
#[derive(Clone, Debug, PartialEq)]
pub struct Workspaces {
    pub dir: PathBuf,
}

impl Workspaces {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Every workspace, sorted by name ignoring case. Hidden folders (like a
    /// `.git` folder) aren't workspaces.
    pub fn list(&self) -> io::Result<Vec<String>> {
        let mut names = Vec::new();
        let entries = match fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(names),
            Err(e) => return Err(e),
        };
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            if entry.file_type()?.is_dir() && !name.starts_with('.') {
                names.push(name);
            }
        }
        names.sort_by_key(|name| name.to_lowercase());
        Ok(names)
    }

    /// Where a workspace's todos are kept.
    pub fn todos_path(&self, name: &str) -> PathBuf {
        self.dir.join(name).join(TODOS_FILE)
    }

    /// Makes a new, empty workspace, returning its name as it was saved.
    pub fn create(&self, name: &str) -> io::Result<String> {
        let name = valid_name(name).map_err(io::Error::other)?;
        if self.list()?.iter().any(|existing| existing.to_lowercase() == name.to_lowercase()) {
            return Err(io::Error::other(format!("There's already a workspace called {name}")));
        }
        fs::create_dir_all(self.dir.join(&name))?;
        // An empty list, so the folder already shows what it's for.
        fs::write(self.todos_path(&name), "{}")?;
        Ok(name)
    }

}

/// Which workspace to open in `dir`: `wanted` if it's there, otherwise the
/// first. `None` if the folder has none (it's new, or was emptied), when
/// the setup screen is needed.
pub fn pick(dir: &Path, wanted: Option<&str>) -> io::Result<Option<String>> {
    let names = Workspaces::new(dir.to_path_buf()).list()?;
    let found = wanted.and_then(|wanted| names.iter().find(|name| name.as_str() == wanted));
    Ok(found.or(names.first()).cloned())
}

/// Finishes setup: makes the todoro folder and the workspace `name` in it,
/// unless a workspace of that name is already there (a restored backup),
/// and moves todos from before the todoro folder existed (`legacy`) into it
/// if it's empty. Returns the workspace's name as saved.
pub fn set_up(dir: &Path, name: &str, legacy: Option<&Path>) -> io::Result<String> {
    let workspaces = Workspaces::new(dir.to_path_buf());
    fs::create_dir_all(dir)?;
    let existing = workspaces.list()?.into_iter().find(|existing| existing.to_lowercase() == name.trim().to_lowercase());
    let name = match existing {
        Some(name) => name,
        None => workspaces.create(name)?,
    };
    let todos = workspaces.todos_path(&name);
    let empty = fs::read_to_string(&todos).map_or(true, |text| text.trim() == "{}");
    if let Some(legacy) = legacy.filter(|legacy| legacy.exists())
        && empty
    {
        move_file(legacy, &todos)?;
    }
    Ok(name)
}

/// A workspace name, trimmed, if it can be a folder name on every system.
pub fn valid_name(name: &str) -> Result<String, &'static str> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Give the workspace a name");
    }
    if name.starts_with('.') {
        return Err("A workspace name can't start with a dot");
    }
    if name.chars().any(|c| matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control()) {
        return Err("A workspace name can't have / \\ : * ? \" < > or |");
    }
    Ok(name.to_string())
}

/// Turns what was typed into a folder path: `~` means the home folder, and a
/// relative path is taken from the current folder.
pub fn expand_path(typed: &str) -> PathBuf {
    let typed = typed.trim();
    let path = match typed.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with(['/', '\\']) => {
            let home = dirs::home_dir().unwrap_or_default();
            home.join(rest.trim_start_matches(['/', '\\']))
        }
        _ => PathBuf::from(typed),
    };
    if path.is_absolute() { path } else { std::env::current_dir().unwrap_or_default().join(path) }
}

/// Shows a path with the home folder as `~`, as it was most likely typed.
pub fn display_path(path: &Path) -> String {
    match dirs::home_dir().and_then(|home| path.strip_prefix(&home).ok().map(Path::to_path_buf)) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Some(rest) => format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display()),
        None => path.display().to_string(),
    }
}

/// The folder suggested on first run: `todoro` in the home folder.
pub fn default_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join("todoro")
}

/// Where todos were kept before the todoro folder existed.
pub fn legacy_file() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join("todoro").join(TODOS_FILE))
}

/// Moves a file, copying it when a plain rename can't (another drive).
pub fn move_file(from: &Path, to: &Path) -> io::Result<()> {
    if let Some(dir) = to.parent() {
        fs::create_dir_all(dir)?;
    }
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }
    fs::copy(from, to)?;
    fs::remove_file(from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder() -> (tempfile::TempDir, Workspaces) {
        let dir = tempfile::tempdir().unwrap();
        let workspaces = Workspaces::new(dir.path().join("todoro"));
        (dir, workspaces)
    }

    #[test]
    fn a_new_folder_has_no_workspaces() {
        let (_dir, workspaces) = folder();
        assert!(workspaces.list().unwrap().is_empty());
    }

    #[test]
    fn create_makes_a_folder_with_an_empty_list() {
        let (_dir, workspaces) = folder();
        assert_eq!(workspaces.create("  Work ").unwrap(), "Work");
        assert_eq!(fs::read_to_string(workspaces.todos_path("Work")).unwrap(), "{}");
        workspaces.create("personal").unwrap();
        workspaces.create("Side project").unwrap();
        assert_eq!(workspaces.list().unwrap(), ["personal", "Side project", "Work"]);
    }

    #[test]
    fn names_must_be_new_and_usable_as_folders() {
        let (_dir, workspaces) = folder();
        workspaces.create("Work").unwrap();
        assert!(workspaces.create("work").is_err(), "same name, other case");
        for bad in ["", "   ", ".hidden", "a/b", "a\\b", "a:b", "what?"] {
            assert!(workspaces.create(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn hidden_folders_and_files_are_not_workspaces() {
        let (_dir, workspaces) = folder();
        workspaces.create("Work").unwrap();
        fs::create_dir_all(workspaces.dir.join(".git")).unwrap();
        fs::write(workspaces.dir.join("README.txt"), "notes").unwrap();
        assert_eq!(workspaces.list().unwrap(), ["Work"]);
    }

    #[test]
    fn typed_paths_expand_the_home_folder() {
        let home = dirs::home_dir().unwrap();
        assert_eq!(expand_path("~/todoro"), home.join("todoro"));
        assert_eq!(expand_path(" ~ "), home);
        assert!(expand_path("relative/folder").is_absolute());
        assert_eq!(display_path(&home.join("todoro")), format!("~{}todoro", std::path::MAIN_SEPARATOR));
    }

    #[test]
    fn pick_opens_the_wanted_workspace_or_the_first() {
        let (_dir, workspaces) = folder();
        assert_eq!(pick(&workspaces.dir, Some("Work")).unwrap(), None);
        workspaces.create("Work").unwrap();
        workspaces.create("Home").unwrap();
        assert_eq!(pick(&workspaces.dir, Some("Work")).unwrap().as_deref(), Some("Work"));
        assert_eq!(pick(&workspaces.dir, Some("Gone")).unwrap().as_deref(), Some("Home"));
        assert_eq!(pick(&workspaces.dir, None).unwrap().as_deref(), Some("Home"));
    }

    #[test]
    fn set_up_makes_the_folder_and_moves_old_todos_in() {
        let (dir, workspaces) = folder();
        let legacy = dir.path().join("old").join("todos.json");
        fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        fs::write(&legacy, r#"{"2026-10-05":["kept"]}"#).unwrap();
        assert_eq!(set_up(&workspaces.dir, " Personal ", Some(&legacy)).unwrap(), "Personal");
        assert_eq!(fs::read_to_string(workspaces.todos_path("Personal")).unwrap(), r#"{"2026-10-05":["kept"]}"#);
        assert!(!legacy.exists());
    }

    #[test]
    fn set_up_uses_a_workspace_already_there_without_overwriting_it() {
        let (dir, workspaces) = folder();
        workspaces.create("Personal").unwrap();
        fs::write(workspaces.todos_path("Personal"), r#"{"2026-10-05":["restored"]}"#).unwrap();
        let legacy = dir.path().join("todos.json");
        fs::write(&legacy, r#"{"2026-10-05":["old"]}"#).unwrap();
        assert_eq!(set_up(&workspaces.dir, "personal", Some(&legacy)).unwrap(), "Personal");
        assert_eq!(fs::read_to_string(workspaces.todos_path("Personal")).unwrap(), r#"{"2026-10-05":["restored"]}"#);
        // The old file is left alone rather than lost.
        assert!(legacy.exists());
    }

    #[test]
    fn set_up_without_old_todos_starts_empty() {
        let (_dir, workspaces) = folder();
        set_up(&workspaces.dir, "Work", None).unwrap();
        assert_eq!(workspaces.list().unwrap(), ["Work"]);
    }

    #[test]
    fn move_file_creates_the_target_folder() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("old.json");
        fs::write(&from, "{}").unwrap();
        let to = dir.path().join("new").join("deeper").join("todos.json");
        move_file(&from, &to).unwrap();
        assert!(!from.exists());
        assert_eq!(fs::read_to_string(to).unwrap(), "{}");
    }
}
