//! Keeping todos safe through an update. Before the installer runs, todoro
//! counts what it can read (`Summary`), copies the todos, notes and settings
//! into a backup folder, and keeps the exe it's running from. Once the new
//! version is installed, it asks it to count the same data with
//! `todoro --check-data`, which only reads. If it can't, or finds less, the
//! old exe goes back and nothing has changed. `--force` keeps the new version
//! anyway, for someone who has made their own copies.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

use crate::options::Settings;
use crate::store::Store;
use crate::workspaces::Workspaces;

/// How many backups to keep, the newest.
const KEEP_BACKUPS: usize = 3;

/// What a version of todoro can read of the user's data.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    pub workspaces: usize,
    pub items: usize,
    /// Items with notes.
    pub notes: usize,
    /// Every character of every note.
    pub notes_chars: usize,
}

/// Where the user's todos are.
#[derive(Clone, Debug, PartialEq)]
pub enum Data {
    /// `TODORO_FILE`, alone.
    File(PathBuf),
    /// The todoro folder, with a workspace in each folder.
    Folder(PathBuf),
    /// Nowhere yet: todoro isn't set up.
    Nothing,
}

impl Data {
    pub fn of(settings: &Settings, todoro_file: Option<&Path>) -> Self {
        match (todoro_file, &settings.data_dir) {
            (Some(file), _) => Data::File(file.to_path_buf()),
            (None, Some(dir)) => Data::Folder(dir.clone()),
            (None, None) => Data::Nothing,
        }
    }

    /// Every todos file.
    fn todos_files(&self) -> io::Result<Vec<PathBuf>> {
        Ok(match self {
            Data::File(file) => vec![file.clone()],
            Data::Folder(dir) => {
                let workspaces = Workspaces::new(dir.clone());
                workspaces.list()?.iter().map(|name| workspaces.todos_path(name)).collect()
            }
            Data::Nothing => Vec::new(),
        })
    }

    /// Where backups go: a hidden folder in the todoro folder (which isn't
    /// listed as a workspace), or beside `TODORO_FILE`.
    pub fn backups_dir(&self) -> Option<PathBuf> {
        match self {
            Data::File(file) => Some(file.parent().unwrap_or(Path::new(".")).join(".todoro-backups")),
            Data::Folder(dir) => Some(dir.join(".backups")),
            Data::Nothing => None,
        }
    }
}

/// Counts everything this version reads in `data`, writing nothing.
pub fn summarize(data: &Data) -> io::Result<Summary> {
    let mut summary = Summary::default();
    for file in data.todos_files()? {
        let store = Store::open_read_only(file)?;
        summary.workspaces += 1;
        for item in store.all().flat_map(|(_, items)| items) {
            summary.items += 1;
            if !item.notes.is_empty() {
                summary.notes += 1;
                summary.notes_chars += item.notes.chars().count();
            }
        }
    }
    Ok(summary)
}

/// `todoro --check-data`: the summary as JSON, or why the data couldn't be
/// read.
pub fn check_data(data: &Data) -> Result<String, String> {
    let summary = summarize(data).map_err(|error| format!("couldn't read the todos: {error}"))?;
    serde_json::to_string(&summary).map_err(|error| error.to_string())
}

/// Asks the todoro at `exe` what it reads, with `--check-data`.
pub fn checked_by(exe: &Path) -> Result<Summary, String> {
    let output = Command::new(exe).arg("--check-data").output().map_err(|error| format!("couldn't run it: {error}"))?;
    if !output.status.success() {
        let said = String::from_utf8_lossy(&output.stderr);
        let said = said.lines().next().unwrap_or("").trim();
        return Err(if said.is_empty() { "it stopped with an error".into() } else { said.to_string() });
    }
    serde_json::from_slice(&output.stdout).map_err(|_| "it didn't say what it read".into())
}

/// What's missing from `after`, read by the new version, compared with
/// `before`, or `None` if it read it all.
pub fn missing(before: Summary, after: Summary) -> Option<String> {
    let counts = [
        (before.workspaces, after.workspaces, "workspaces"),
        (before.items, after.items, "items"),
        (before.notes, after.notes, "items with notes"),
    ];
    let mut short: Vec<String> = counts
        .iter()
        .filter(|(had, read, _)| read < had)
        .map(|(had, read, what)| format!("{read} of your {had} {what}"))
        .collect();
    if short.is_empty() && after.notes_chars < before.notes_chars {
        short.push("only part of your notes".into());
    }
    (!short.is_empty()).then(|| format!("it read {}", short.join(", ")))
}

/// Copies the todos, notes and the files in `extra` (settings, keys) into a
/// new folder for this update to `version`, named from `now` so they sort
/// oldest first, keeping the newest few. Returns where, or `None` with
/// nothing to back up.
pub fn backup(data: &Data, version: &str, extra: &[PathBuf], now: NaiveDateTime) -> io::Result<Option<PathBuf>> {
    let Some(dir) = data.backups_dir() else { return Ok(None) };
    let name = format!("{}-before-{version}", now.format("%Y-%m-%d-%H%M%S"));
    let mut to = dir.join(&name);
    let mut n = 2;
    while to.exists() {
        to = dir.join(format!("{name}-{n}"));
        n += 1;
    }
    fs::create_dir_all(&to)?;
    match data {
        Data::Folder(folder) => {
            for name in Workspaces::new(folder.clone()).list()? {
                copy_dir(&folder.join(&name), &to.join(&name))?;
            }
        }
        Data::File(file) => {
            let notes = crate::notes_files::NotesFiles::for_todos(file).dir;
            let deleted = file.with_file_name("last-deleted.json");
            for from in [file.clone(), deleted] {
                if from.exists() {
                    fs::copy(&from, to.join(from.file_name().unwrap_or_default()))?;
                }
            }
            if notes.exists() {
                copy_dir(&notes, &to.join(notes.file_name().unwrap_or_default()))?;
            }
        }
        Data::Nothing => {}
    }
    for file in extra.iter().filter(|file| file.exists()) {
        fs::copy(file, to.join(file.file_name().unwrap_or_default()))?;
    }
    prune(&dir)?;
    Ok(Some(to))
}

/// Copies the folder `from` and everything in it to `to`.
fn copy_dir(from: &Path, to: &Path) -> io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// Deletes all but the newest `KEEP_BACKUPS` backups in `dir`.
fn prune(dir: &Path) -> io::Result<()> {
    let mut backups: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().contains("-before-"))
        .map(|entry| entry.path())
        .collect();
    // Their names start with when they were made, so this is oldest first;
    // a second one in the same second, with -2 after the name, sorts after.
    backups.sort();
    let extra = backups.len().saturating_sub(KEEP_BACKUPS);
    for path in backups.into_iter().take(extra) {
        fs::remove_dir_all(path)?;
    }
    Ok(())
}

/// Where the exe that was running is kept until the new one has checked the
/// data: copied beside it on Unix, and on Windows where `update::install`
/// moves it aside, since a running exe can't be replaced there.
pub fn previous_exe(exe: &Path) -> PathBuf {
    if cfg!(windows) {
        exe.with_extension("old.exe")
    } else {
        let mut name = exe.file_name().unwrap_or_default().to_os_string();
        name.push(".previous");
        exe.with_file_name(name)
    }
}

/// How an update went, when the new version is installed.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// Installed, and it read everything.
    Updated { backup: Option<PathBuf> },
    /// Installed with `force`, though its check found this problem.
    Forced { problem: String, backup: Option<PathBuf> },
}

/// The ways an update can stop short.
#[derive(Debug, PartialEq)]
pub enum Failure {
    /// Before anything was installed.
    NotInstalled(String),
    /// The new version couldn't read everything, and the old one is back.
    RolledBack { problem: String, backup: Option<PathBuf> },
    /// The new version couldn't read everything, and the old one couldn't
    /// be put back.
    StuckOnNew { problem: String, backup: Option<PathBuf>, error: String },
}

/// What the update needs done, which tests replace.
pub struct Steps<'a> {
    /// Runs the installer, which replaces the exe.
    pub install: &'a mut dyn FnMut() -> Result<(), String>,
    /// What the newly installed todoro reads.
    pub check: &'a dyn Fn() -> Result<Summary, String>,
}

/// Updates to `version` safely: counts and backs up the data (with the
/// files in `extra`), keeps the exe at `exe`, installs, and has the new
/// version count the data. If it reads less, the old exe goes back, unless
/// `force`. With `force`, problems before installing don't stop it either.
pub fn update(
    data: &Data,
    version: &str,
    exe: &Path,
    extra: &[PathBuf],
    now: NaiveDateTime,
    force: bool,
    steps: Steps,
) -> Result<Outcome, Failure> {
    let stop = |problem: String| if force { Ok(()) } else { Err(Failure::NotInstalled(problem)) };
    let before = match summarize(data) {
        Ok(summary) => Some(summary),
        Err(error) => {
            stop(format!("this version couldn't read your todos to check them first: {error}"))?;
            None
        }
    };
    let backup = match backup(data, version, extra, now) {
        Ok(backup) => backup,
        Err(error) => {
            stop(format!("couldn't back up your todos first: {error}"))?;
            None
        }
    };
    let previous = previous_exe(exe);
    if cfg!(not(windows)) {
        let _ = fs::remove_file(&previous);
        if let Err(error) = fs::copy(exe, &previous) {
            stop(format!("couldn't keep a copy of this todoro to go back to: {error}"))?;
        }
    }
    if let Err(error) = (steps.install)() {
        if cfg!(not(windows)) {
            let _ = fs::remove_file(&previous);
        }
        return Err(Failure::NotInstalled(error));
    }
    let problem = match ((steps.check)(), before) {
        (Err(error), _) => Some(format!("it couldn't read your todos ({error})")),
        (Ok(after), Some(before)) => missing(before, after),
        (Ok(_), None) => None,
    };
    match problem {
        Some(problem) if !force => match put_back(&previous, exe) {
            Ok(()) => Err(Failure::RolledBack { problem, backup }),
            Err(error) => Err(Failure::StuckOnNew { problem, backup, error: error.to_string() }),
        },
        problem => {
            // On Windows it's the running exe, removed on the next start.
            if cfg!(not(windows)) {
                let _ = fs::remove_file(&previous);
            }
            Ok(match problem {
                Some(problem) => Outcome::Forced { problem, backup },
                None => Outcome::Updated { backup },
            })
        }
    }
}

/// Puts the kept exe back in place of the new one.
fn put_back(previous: &Path, exe: &Path) -> io::Result<()> {
    if !previous.exists() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "the copy of the old version is missing"));
    }
    fs::rename(previous, exe)
}

/// The command that updates anyway.
pub const FORCE: &str = "todoro --update --force (or todoro -U -f)";

impl Failure {
    /// What to tell the user.
    pub fn explain(&self, version: &str) -> String {
        let backup = |backup: &Option<PathBuf>| match backup {
            Some(path) => format!(" A copy of them from before the update is in {}.", path.display()),
            None => String::new(),
        };
        match self {
            Failure::NotInstalled(problem) => format!("Couldn't update: {problem}. Nothing was changed."),
            Failure::RolledBack { problem, backup: kept } => format!(
                "Didn't update: todoro {version} was installed, but {problem}, so the version you had is back. \
                 Your todos and notes weren't changed.{} To update anyway, run: {FORCE}",
                backup(kept),
            ),
            Failure::StuckOnNew { problem, backup: kept, error } => format!(
                "todoro {version} was installed, but {problem}, and the version you had couldn't be put back ({error}). \
                 Don't open todoro until you have reinstalled the version you had.{}",
                backup(kept),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(time: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(&format!("2026-10-05 {time}"), "%Y-%m-%d %H:%M:%S").unwrap()
    }

    const NOON: &str = "12:00:00";

    /// A todoro folder with two workspaces, one item with notes.
    fn folder() -> (tempfile::TempDir, Data) {
        let dir = tempfile::tempdir().unwrap();
        let workspaces = Workspaces::new(dir.path().join("todoro"));
        for name in ["home", "work"] {
            workspaces.create(name).unwrap();
        }
        let mut store = Store::open(workspaces.todos_path("home")).unwrap();
        let day = crate::test_util::today();
        store.insert(day, 0, "Water the plants".into()).unwrap();
        store.insert(day, 1, "Fix the bike".into()).unwrap();
        store.set_notes(day, 1, "Chain is loose".into()).unwrap();
        (dir, Data::Folder(workspaces.dir))
    }

    fn backed_up(dir: &tempfile::TempDir) -> Option<PathBuf> {
        Some(dir.path().join("todoro/.backups/2026-10-05-120000-before-9.0.0"))
    }

    #[test]
    fn the_summary_counts_every_workspace_without_writing() {
        let (dir, data) = folder();
        let todos = dir.path().join("todoro/home/todos.json");
        let before = fs::read_to_string(&todos).unwrap();
        let summary = summarize(&data).unwrap();
        assert_eq!(summary, Summary { workspaces: 2, items: 2, notes: 1, notes_chars: 14 });
        assert_eq!(fs::read_to_string(&todos).unwrap(), before);
        assert_eq!(check_data(&data).unwrap(), r#"{"workspaces":2,"items":2,"notes":1,"notes_chars":14}"#);
        assert_eq!(summarize(&Data::Nothing).unwrap(), Summary::default());
    }

    #[test]
    fn reading_inline_notes_to_check_them_leaves_the_file_as_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("todos.json");
        let old = r#"{"2026-10-05": [{"text": "Fix the bike", "notes": "Chain"}]}"#;
        fs::write(&file, old).unwrap();
        let summary = summarize(&Data::File(file.clone())).unwrap();
        assert_eq!((summary.items, summary.notes), (1, 1));
        assert_eq!(fs::read_to_string(&file).unwrap(), old);
        assert!(!dir.path().join("notes").exists());
    }

    #[test]
    fn reading_less_is_missing_something() {
        let all = Summary { workspaces: 2, items: 5, notes: 2, notes_chars: 40 };
        assert_eq!(missing(all, all), None);
        assert_eq!(missing(all, Summary { items: 9, notes_chars: 90, ..all }), None);
        assert_eq!(missing(all, Summary { items: 3, ..all }), Some("it read 3 of your 5 items".into()));
        assert_eq!(
            missing(all, Summary { notes: 0, notes_chars: 0, ..all }),
            Some("it read 0 of your 2 items with notes".into()),
        );
        assert_eq!(missing(all, Summary { notes_chars: 39, ..all }), Some("it read only part of your notes".into()));
    }

    #[test]
    fn a_backup_has_every_workspace_and_the_settings_and_only_the_newest_are_kept() {
        let (dir, data) = folder();
        let settings = dir.path().join("settings.json");
        fs::write(&settings, "{}").unwrap();
        let backup = backup(&data, "9.0.0", &[settings, dir.path().join("keys.toml")], at(NOON)).unwrap();
        assert_eq!(backup, backed_up(&dir));
        let backup = backup.unwrap();
        assert!(backup.join("home/todos.json").exists());
        assert!(backup.join("home/notes/fix-the-bike.md").exists());
        assert!(backup.join("work").is_dir());
        assert!(backup.join("settings.json").exists());
        // Missing files are skipped.
        assert!(!backup.join("keys.toml").exists());
        // The backups aren't a workspace.
        assert_eq!(summarize(&data).unwrap().workspaces, 2);
        for time in [NOON, "12:00:01", "09:00:00", "12:30:00"] {
            super::backup(&data, "9.0.0", &[], at(time)).unwrap();
        }
        let mut kept: Vec<_> = fs::read_dir(dir.path().join("todoro/.backups"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        kept.sort();
        assert_eq!(
            kept,
            ["2026-10-05-120000-before-9.0.0-2", "2026-10-05-120001-before-9.0.0", "2026-10-05-123000-before-9.0.0"],
        );
    }

    #[test]
    fn a_single_file_is_backed_up_beside_it_with_its_notes() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("mine.json");
        let mut store = Store::open(file.clone()).unwrap();
        store.insert(crate::test_util::today(), 0, "Fix the bike".into()).unwrap();
        store.set_notes(crate::test_util::today(), 0, "Chain".into()).unwrap();
        let backup = backup(&Data::File(file), "9.0.0", &[], at(NOON)).unwrap().unwrap();
        assert_eq!(backup, dir.path().join(".todoro-backups/2026-10-05-120000-before-9.0.0"));
        assert!(backup.join("mine.json").exists());
        assert!(backup.join("mine-notes/fix-the-bike.md").exists());
        assert_eq!(super::backup(&Data::Nothing, "9.0.0", &[], at(NOON)).unwrap(), None);
    }

    /// A stand-in for the todoro exe, saying which version it is. The
    /// update tests use them on Unix only: on Windows the installer moves
    /// the exe aside itself.
    #[cfg(not(windows))]
    fn exe(dir: &Path) -> PathBuf {
        let bin = dir.join("bin");
        fs::create_dir(&bin).unwrap();
        let exe = bin.join("todoro");
        fs::write(&exe, "old").unwrap();
        exe
    }

    /// Updates with an installer that writes "new" over `exe`, and a new
    /// version that reads `read`.
    #[cfg(not(windows))]
    fn run(data: &Data, exe: &Path, force: bool, read: Result<Summary, String>) -> Result<Outcome, Failure> {
        let installed = std::cell::Cell::new(false);
        let mut install = || {
            assert!(previous_exe(exe).exists(), "kept before installing");
            fs::write(exe, "new").unwrap();
            installed.set(true);
            Ok(())
        };
        let check = || read.clone();
        let outcome = update(data, "9.0.0", exe, &[], at(NOON), force, Steps { install: &mut install, check: &check });
        assert!(installed.get());
        outcome
    }

    #[cfg(not(windows))]
    #[test]
    fn an_update_that_reads_everything_stays() {
        let (dir, data) = folder();
        let exe = exe(dir.path());
        let all = summarize(&data).unwrap();
        assert_eq!(run(&data, &exe, false, Ok(all)), Ok(Outcome::Updated { backup: backed_up(&dir) }));
        assert_eq!(fs::read_to_string(&exe).unwrap(), "new");
        assert!(!previous_exe(&exe).exists());
    }

    #[cfg(not(windows))]
    #[test]
    fn an_update_that_reads_less_puts_the_old_version_back_unless_forced() {
        let (dir, data) = folder();
        let exe = exe(dir.path());
        let less = Summary { items: 1, ..summarize(&data).unwrap() };
        let problem = "it read 1 of your 2 items".to_string();
        let outcome = run(&data, &exe, false, Ok(less));
        assert_eq!(outcome, Err(Failure::RolledBack { problem: problem.clone(), backup: backed_up(&dir) }));
        assert_eq!(fs::read_to_string(&exe).unwrap(), "old");
        assert!(!previous_exe(&exe).exists());
        let told = outcome.unwrap_err().explain("9.0.0");
        assert!(told.contains("so the version you had is back"), "{told}");
        assert!(told.contains("2026-10-05-120000-before-9.0.0"), "{told}");
        assert!(told.contains("todoro --update --force (or todoro -U -f)"), "{told}");

        // Forced, the new one stays.
        let outcome = run(&data, &exe, true, Ok(less));
        assert!(matches!(&outcome, Ok(Outcome::Forced { problem: said, .. }) if *said == problem), "{outcome:?}");
        assert_eq!(fs::read_to_string(&exe).unwrap(), "new");
        assert!(!previous_exe(&exe).exists());
    }

    #[cfg(not(windows))]
    #[test]
    fn an_update_that_cant_read_at_all_puts_the_old_version_back() {
        let (dir, data) = folder();
        let exe = exe(dir.path());
        let outcome = run(&data, &exe, false, Err("unknown option --check-data".into()));
        assert!(
            matches!(&outcome, Err(Failure::RolledBack { problem, .. }) if problem.contains("unknown option --check-data")),
            "{outcome:?}",
        );
        assert_eq!(fs::read_to_string(&exe).unwrap(), "old");
    }

    #[cfg(not(windows))]
    #[test]
    fn unreadable_todos_stop_the_update_before_it_starts_unless_forced() {
        let (dir, data) = folder();
        fs::write(dir.path().join("todoro/work/todos.json"), "not json").unwrap();
        let exe = exe(dir.path());
        let mut install = || -> Result<(), String> { panic!("not installed") };
        let check = || Ok(Summary::default());
        let outcome = update(&data, "9.0.0", &exe, &[], at(NOON), false, Steps { install: &mut install, check: &check });
        assert!(matches!(&outcome, Err(Failure::NotInstalled(problem)) if problem.contains("couldn't read")), "{outcome:?}");
        assert!(!dir.path().join("todoro/.backups").exists());
        assert_eq!(run(&data, &exe, true, Ok(Summary::default())), Ok(Outcome::Updated { backup: backed_up(&dir) }));
    }

    #[cfg(not(windows))]
    #[test]
    fn a_failed_install_keeps_the_old_version_and_no_copy() {
        let (dir, data) = folder();
        let exe = exe(dir.path());
        let mut install = || Err("the installer didn't finish".to_string());
        let check = || -> Result<Summary, String> { panic!("nothing to check") };
        let outcome = update(&data, "9.0.0", &exe, &[], at(NOON), false, Steps { install: &mut install, check: &check });
        assert_eq!(outcome, Err(Failure::NotInstalled("the installer didn't finish".into())));
        assert_eq!(fs::read_to_string(&exe).unwrap(), "old");
        assert!(!previous_exe(&exe).exists());
    }
}
