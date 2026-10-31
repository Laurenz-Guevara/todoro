//! Updating todoro from inside it. Once a day, if the option is on, todoro
//! asks GitHub for the latest version, only to show that one is available.
//! It never installs anything by itself: `:update` shows the new release's
//! notes, and only `y` there runs the official installer, after which
//! todoro restarts on the new version. A copy installed some other way (with
//! `cargo install`, or by hand) is left alone, with how to update it.
//!
//! GitHub is asked with `curl`, which macOS, Linux and Windows 10 and later
//! have, so todoro needs no networking of its own.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver};

use ratatui::crossterm::event::{KeyCode, KeyEvent};

use crate::changelog::{ChangelogView, VERSION, parse_version};

/// The GitHub repository releases come from.
const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");

/// The repository as `owner/name`.
fn owner_and_name() -> &'static str {
    REPOSITORY.trim_start_matches("https://github.com/").trim_end_matches('/')
}

/// Where todoro asks for the latest version and its notes, and how it finds
/// out how it was installed, swapped out in tests so they never touch the
/// network or the machine's files.
#[derive(Clone, Copy)]
pub struct Fetcher {
    /// The latest released version, like "1.0.0".
    pub latest: fn() -> Result<String, String>,
    /// The changelog as it was released in `version`.
    pub changelog: fn(&str) -> Result<String, String>,
    /// How this todoro was installed.
    pub installed: fn() -> Install,
}

impl Fetcher {
    /// GitHub, through `curl`.
    pub fn github() -> Self {
        Self { latest: latest_from_github, changelog: changelog_from_github, installed: Install::of_this }
    }
}

fn latest_from_github() -> Result<String, String> {
    tag_from_json(&curl(&format!("https://api.github.com/repos/{}/releases/latest", owner_and_name()))?)
}

fn changelog_from_github(version: &str) -> Result<String, String> {
    curl(&format!("https://raw.githubusercontent.com/{}/v{version}/CHANGELOG.md", owner_and_name()))
}

/// Fetches `url`, giving up after 15 seconds.
fn curl(url: &str) -> Result<String, String> {
    let output = Command::new("curl")
        .args(["--proto", "=https", "--tlsv1.2", "-fsSL", "--max-time", "15", "-H", "User-Agent: todoro", url])
        .output()
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => "curl isn't installed, so todoro can't ask GitHub".to_string(),
            _ => format!("couldn't run curl: {error}"),
        })?;
    if !output.status.success() {
        let said = String::from_utf8_lossy(&output.stderr);
        let reason = said.lines().next().unwrap_or("").trim().trim_start_matches("curl: ");
        return Err(if reason.is_empty() { "GitHub didn't answer".into() } else { format!("GitHub didn't answer: {reason}") });
    }
    String::from_utf8(output.stdout).map_err(|_| "GitHub's answer wasn't text".into())
}

/// The version in GitHub's answer about the latest release, from its tag.
pub fn tag_from_json(json: &str) -> Result<String, String> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(|_| "GitHub's answer wasn't understood".to_string())?;
    let tag = value["tag_name"].as_str().ok_or("GitHub didn't say which version is the latest")?;
    let version = tag.trim_start_matches('v');
    parse_version(version).ok_or(format!("the latest release, {tag}, isn't a version todoro knows"))?;
    Ok(version.to_string())
}

/// Whether `version` is newer than this todoro.
pub fn is_newer(version: &str) -> bool {
    match (parse_version(version), parse_version(VERSION)) {
        (Some(other), Some(this)) => other > this,
        _ => false,
    }
}

/// Asks for the latest version in the background, for the daily check.
pub fn check(fetcher: Fetcher) -> Receiver<Result<String, String>> {
    let (send, receive) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = send.send((fetcher.latest)());
    });
    receive
}

/// What `:update` found: nothing newer, or a newer version with its
/// changelog.
#[derive(Debug, PartialEq)]
pub enum Found {
    UpToDate,
    Newer { version: String, changelog: String },
}

/// Asks for the latest version, and its notes if it's newer, in the
/// background, for `:update`.
pub fn find(fetcher: Fetcher) -> Receiver<Result<Found, String>> {
    let (send, receive) = mpsc::channel();
    std::thread::spawn(move || {
        let found = (fetcher.latest)().and_then(|version| {
            if !is_newer(&version) {
                return Ok(Found::UpToDate);
            }
            let changelog = (fetcher.changelog)(&version)?;
            Ok(Found::Newer { version, changelog })
        });
        let _ = send.send(found);
    });
    receive
}

/// How this copy of todoro was installed, which decides how it updates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Install {
    /// By the official installer, which can update it.
    Installer,
    /// Some other way, like `cargo install`: todoro leaves it alone.
    Other,
}

impl Install {
    /// How the running todoro was installed: by the installer if it left its
    /// receipt saying it installed todoro where this one runs from.
    pub fn of_this() -> Self {
        let (Some(receipt), Ok(exe)) = (receipt_path(), std::env::current_exe()) else { return Install::Other };
        match (std::fs::read_to_string(receipt), exe.parent()) {
            (Ok(json), Some(dir)) if installed_here(&json, dir) => Install::Installer,
            _ => Install::Other,
        }
    }
}

/// Where the installer leaves its receipt, as its scripts decide.
fn receipt_path() -> Option<PathBuf> {
    let dir = match std::env::var_os("XDG_CONFIG_HOME").filter(|dir| !dir.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None if cfg!(windows) => PathBuf::from(std::env::var_os("LOCALAPPDATA")?),
        None => dirs::home_dir()?.join(".config"),
    };
    Some(dir.join("todoro").join("todoro-receipt.json"))
}

/// Whether the installer's receipt, `json`, says it installed todoro in
/// `dir` (or in `bin` there).
pub fn installed_here(json: &str, dir: &Path) -> bool {
    let Ok(receipt) = serde_json::from_str::<serde_json::Value>(json) else { return false };
    let Some(prefix) = receipt["install_prefix"].as_str() else { return false };
    let same = |a: &Path, b: &Path| a == b || a.canonicalize().ok().is_some_and(|a| b.canonicalize().ok() == Some(a));
    let prefix = Path::new(prefix);
    same(prefix, dir) || same(&prefix.join("bin"), dir)
}

/// What to tell someone whose todoro the installer didn't install.
pub const UPDATE_IT_YOURSELF: &str = concat!(
    "This todoro wasn't installed with its installer, so update it the way you installed it. With cargo, run: cargo install --git ",
    env!("CARGO_PKG_REPOSITORY"),
);

/// The `:update` popup.
pub enum Popup {
    /// Asking GitHub.
    Checking,
    /// A newer version, with its notes.
    Newer { version: String, notes: ChangelogView, install: Install },
}

/// What a key in the popup asks of the app.
#[derive(Debug, PartialEq)]
pub enum Action {
    Stay,
    Close,
    /// Install this version.
    Install(String),
}

impl Popup {
    /// The popup for what `:update` found newer.
    pub fn newer(version: String, changelog: String, install: Install) -> Self {
        let notes = ChangelogView::fetched(changelog, parse_version(VERSION).unwrap_or_default());
        Popup::Newer { version, notes, install }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Action {
        match self {
            Popup::Checking => match key.code {
                KeyCode::Esc | KeyCode::Char('q') => Action::Close,
                _ => Action::Stay,
            },
            Popup::Newer { version, notes, install } => match key.code {
                KeyCode::Char('y') | KeyCode::Enter if *install == Install::Installer => Action::Install(version.clone()),
                KeyCode::Char('n') => Action::Close,
                _ if notes.handle_key(key) => Action::Close,
                _ => Action::Stay,
            },
        }
    }
}

/// Runs the official installer for `version`, which replaces this todoro,
/// showing its output in the terminal. On Windows the running exe can't be
/// replaced, so it moves aside first (and comes back if the install fails);
/// `tidy_up` removes it on the next start.
pub fn install(version: &str) -> Result<(), String> {
    let base = format!("{REPOSITORY}/releases/download/v{version}");
    if cfg!(windows) {
        let exe = std::env::current_exe().map_err(|error| error.to_string())?;
        let aside = moved_aside(&exe);
        let _ = std::fs::remove_file(&aside);
        std::fs::rename(&exe, &aside).map_err(|error| format!("couldn't move todoro aside to replace it: {error}"))?;
        let result = run_windows_installer(&base);
        if result.is_err() && !exe.exists() {
            let _ = std::fs::rename(&aside, &exe);
        }
        result
    } else {
        let script = format!("curl --proto '=https' --tlsv1.2 -LsSf {base}/todoro-installer.sh | sh");
        let status = Command::new("sh").args(["-c", &script]).status().map_err(|error| format!("couldn't run the installer: {error}"))?;
        if status.success() { Ok(()) } else { Err("the installer didn't finish".into()) }
    }
}

/// Downloads the PowerShell installer and runs it, rather than piping it
/// into PowerShell, which some setups don't allow.
fn run_windows_installer(base: &str) -> Result<(), String> {
    let script = std::env::temp_dir().join("todoro-installer.ps1");
    let url = format!("{base}/todoro-installer.ps1");
    let script_path = script.to_string_lossy().to_string();
    let downloaded = Command::new("curl")
        .args(["--proto", "=https", "--tlsv1.2", "-fsSL", "-o", &script_path, &url])
        .status()
        .map_err(|error| format!("couldn't run curl: {error}"))?;
    if !downloaded.success() {
        return Err("couldn't download the installer".into());
    }
    let status = Command::new("powershell")
        .args(["-ExecutionPolicy", "Bypass", "-File", &script_path])
        .status()
        .map_err(|error| format!("couldn't run PowerShell: {error}"))?;
    let _ = std::fs::remove_file(&script);
    if status.success() { Ok(()) } else { Err("the installer didn't finish".into()) }
}

/// Where a running exe is moved to while it's replaced.
fn moved_aside(exe: &Path) -> PathBuf {
    exe.with_extension("old.exe")
}

/// Removes the exe a Windows update moved aside, now that it's not running.
pub fn tidy_up() {
    if cfg!(windows)
        && let Ok(exe) = std::env::current_exe()
    {
        let _ = std::fs::remove_file(moved_aside(&exe));
    }
}

/// Starts the updated todoro at `exe`, where this one was, in its place.
/// `exe` must be found before installing: on Linux, once the file is
/// replaced, `current_exe` names the old, deleted one. On Unix it takes over
/// this process; on Windows it runs while this one waits, then exits with it.
pub fn restart(exe: PathBuf) -> std::io::Error {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        Command::new(exe).exec()
    }
    #[cfg(not(unix))]
    {
        match Command::new(exe).status() {
            Ok(status) => std::process::exit(status.code().unwrap_or(0)),
            Err(error) => error,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::KeyModifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn the_latest_version_comes_from_the_release_tag() {
        assert_eq!(tag_from_json(r#"{"tag_name": "v1.2.3", "name": "1.2.3"}"#), Ok("1.2.3".into()));
        assert!(tag_from_json(r#"{"message": "Not Found"}"#).is_err());
        assert!(tag_from_json(r#"{"tag_name": "nightly"}"#).is_err());
        assert!(tag_from_json("<html>").is_err());
    }

    #[test]
    fn only_later_versions_are_newer() {
        let (major, minor, patch) = parse_version(VERSION).unwrap();
        assert!(is_newer(&format!("{major}.{minor}.{}", patch + 1)));
        assert!(is_newer(&format!("{}.0.0", major + 1)));
        assert!(!is_newer(VERSION));
        assert!(!is_newer("0.0.1"));
        assert!(!is_newer("not a version"));
    }

    #[test]
    fn the_installers_receipt_says_where_it_installed() {
        let dir = tempfile::tempdir().unwrap();
        let json = |prefix: &Path| format!(r#"{{"install_prefix": {:?}, "source": {{"app_name": "todoro"}}}}"#, prefix.display().to_string());
        assert!(installed_here(&json(dir.path()), dir.path()));
        // A prefix with a bin folder in it.
        let bin = dir.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        assert!(installed_here(&json(dir.path()), &bin));
        // Somewhere else, like ~/.cargo/bin.
        assert!(!installed_here(&json(&dir.path().join("elsewhere")), dir.path()));
        assert!(!installed_here("not json", dir.path()));
    }

    fn newer_release(_: &str) -> Result<String, String> {
        Ok("# Changelog\n\n## [99.0.0] - 2027-01-01\n\n- Everything\n\n## [0.0.1] - 2026-01-01\n\n- Old\n".into())
    }

    #[test]
    fn finding_an_update_fetches_its_notes_only_if_it_is_newer() {
        let newer = Fetcher { latest: || Ok("99.0.0".into()), changelog: newer_release, installed: || Install::Installer };
        let found = find(newer).recv().unwrap().unwrap();
        assert!(matches!(&found, Found::Newer { version, changelog } if version == "99.0.0" && changelog.contains("Everything")));
        let same = Fetcher { latest: || Ok(VERSION.into()), changelog: |_| panic!("no notes needed"), ..newer };
        assert_eq!(find(same).recv().unwrap(), Ok(Found::UpToDate));
        let offline = Fetcher { latest: || Err("GitHub didn't answer".into()), ..newer };
        assert_eq!(find(offline).recv().unwrap(), Err("GitHub didn't answer".into()));
        assert_eq!(check(offline).recv().unwrap(), Err("GitHub didn't answer".into()));
    }

    #[test]
    fn the_popup_shows_only_newer_notes_and_installs_only_on_y() {
        let mut popup = Popup::newer("99.0.0".into(), newer_release("99.0.0").unwrap(), Install::Installer);
        let Popup::Newer { notes, .. } = &popup else { panic!("newer") };
        let shown: Vec<_> = notes.releases().into_iter().map(|release| release.version).collect();
        assert_eq!(shown, [(99, 0, 0)]);
        assert_eq!(popup.handle_key(key(KeyCode::Char('j'))), Action::Stay);
        assert_eq!(popup.handle_key(key(KeyCode::Char('x'))), Action::Stay);
        assert_eq!(popup.handle_key(key(KeyCode::Char('y'))), Action::Install("99.0.0".into()));
        assert_eq!(popup.handle_key(key(KeyCode::Char('n'))), Action::Close);
        assert_eq!(popup.handle_key(key(KeyCode::Esc)), Action::Close);
        // Installed some other way, y does nothing.
        let mut other = Popup::newer("99.0.0".into(), newer_release("99.0.0").unwrap(), Install::Other);
        assert_eq!(other.handle_key(key(KeyCode::Char('y'))), Action::Stay);
        assert_eq!(Popup::Checking.handle_key(key(KeyCode::Esc)), Action::Close);
        assert_eq!(Popup::Checking.handle_key(key(KeyCode::Char('y'))), Action::Stay);
    }
}
