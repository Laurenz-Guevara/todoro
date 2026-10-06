//! Settings the user can toggle from the `o` popup, saved between runs.

use std::fs;
use std::io;
use std::path::PathBuf;

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use serde::{Deserialize, Serialize};

use crate::input::LineInput;
use crate::workspaces;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Show an icon for each priority as well as the coloured number, for
    /// people who can't tell the colours apart.
    pub semantic_icons: bool,
    /// Draw everything without colour.
    pub no_colour: bool,
    /// The newest version whose release notes have been shown, so "what's
    /// new" appears once after each update. Not an option in the popup.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seen_version: Option<String>,
    /// The todoro folder, chosen on first run, holding every workspace.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_dir: Option<std::path::PathBuf>,
    /// The workspace open last, to open again next time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
}

/// One toggle in the options popup.
pub struct Toggle {
    pub section: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub get: fn(&Settings) -> bool,
    pub set: fn(&mut Settings, bool),
}

/// Every option, in the order shown. Options in the same section must be
/// next to each other.
pub const TOGGLES: &[Toggle] = &[
    Toggle {
        section: "Accessibility",
        label: "Semantic priority icons",
        description: "Show ∧ High, – Medium and ∨ Low beside triaged items, not just a colour.",
        get: |s| s.semantic_icons,
        set: |s, on| s.semantic_icons = on,
    },
    Toggle {
        section: "Accessibility",
        label: "No colours",
        description: "Draw everything in the terminal's own colours. Highlights use reversed text.",
        get: |s| s.no_colour,
        set: |s, on| s.no_colour = on,
    },
];

impl Settings {
    /// Where settings are saved: `$TODORO_SETTINGS`, or
    /// `<config dir>/todoro/settings.json`. `None` if there's no config dir.
    pub fn default_path() -> Option<PathBuf> {
        match std::env::var_os("TODORO_SETTINGS") {
            Some(path) => Some(PathBuf::from(path)),
            None => Some(dirs::config_dir()?.join("todoro").join("settings.json")),
        }
    }

    /// Loads from `path`. These are only display preferences, so a missing or
    /// unreadable file gives the defaults instead of an error. Before anything
    /// has been saved, the standard `NO_COLOR` variable turns colours off.
    pub fn load(path: Option<&PathBuf>, no_color_env: bool) -> Self {
        match path.and_then(|path| fs::read_to_string(path).ok()) {
            Some(json) => serde_json::from_str(&json).unwrap_or_default(),
            None => Settings { no_colour: no_color_env, ..Settings::default() },
        }
    }

    pub fn save(&self, path: &PathBuf) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_string_pretty(self).map_err(io::Error::other)?)?;
        fs::rename(tmp, path)
    }
}

/// The options popup.
#[derive(Default)]
pub struct Options {
    pub selected: usize,
    /// The todoro folder as shown, if there is one to change (not with
    /// `TODORO_FILE`). It's the row after the toggles.
    pub folder: Option<String>,
    /// A new folder being typed.
    pub editing: Option<LineInput>,
    /// The result of the last change: what happened, and whether it worked.
    pub message: Option<(String, bool)>,
}

/// What the app should do after the options popup handles a key.
#[derive(Debug, PartialEq)]
pub enum Action {
    Stay,
    Close,
    /// Flip this entry of `TOGGLES`.
    Toggle(usize),
    /// Move every workspace into this folder.
    MoveFolder(PathBuf),
}

impl Options {
    pub fn new(folder: Option<String>) -> Self {
        Self { folder, ..Self::default() }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Action {
        if let Some(input) = &mut self.editing {
            match key.code {
                KeyCode::Esc => self.editing = None,
                KeyCode::Enter if input.text.trim().is_empty() => {
                    self.message = Some(("Type the folder to move your workspaces to".into(), false));
                }
                KeyCode::Enter => return Action::MoveFolder(workspaces::expand_path(&input.text)),
                code => {
                    input.handle_key(code);
                    self.message = None;
                }
            }
            return Action::Stay;
        }
        let rows = TOGGLES.len() + usize::from(self.folder.is_some());
        let on_folder = self.selected == TOGGLES.len();
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.selected = (self.selected + 1).min(rows - 1),
            KeyCode::Char('k') | KeyCode::Up => self.selected = self.selected.saturating_sub(1),
            KeyCode::Char(' ') | KeyCode::Enter if on_folder => {
                self.editing = Some(LineInput::new(self.folder.as_deref().unwrap_or_default()));
                self.message = None;
            }
            KeyCode::Char(' ') | KeyCode::Enter => return Action::Toggle(self.selected),
            KeyCode::Esc | KeyCode::Char('q' | 'o') => return Action::Close,
            _ => {}
        }
        Action::Stay
    }
}

#[cfg(test)]
mod tests {
    use ratatui::crossterm::event::KeyModifiers;

    use super::*;

    fn press(options: &mut Options, code: KeyCode) -> Action {
        options.handle_key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[test]
    fn everything_is_off_by_default() {
        assert_eq!(Settings::default(), Settings { semantic_icons: false, no_colour: false, last_seen_version: None, ..Settings::default() });
        for toggle in TOGGLES {
            assert!(!(toggle.get)(&Settings::default()));
        }
    }

    #[test]
    fn toggles_set_their_own_setting() {
        let mut settings = Settings::default();
        (TOGGLES[0].set)(&mut settings, true);
        assert_eq!(settings, Settings { semantic_icons: true, no_colour: false, last_seen_version: None, ..Settings::default() });
        (TOGGLES[1].set)(&mut settings, true);
        (TOGGLES[0].set)(&mut settings, false);
        assert_eq!(settings, Settings { semantic_icons: false, no_colour: true, last_seen_version: None, ..Settings::default() });
    }

    #[test]
    fn saved_settings_load_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("settings.json");
        let settings = Settings { semantic_icons: true, no_colour: false, last_seen_version: None, ..Settings::default() };
        settings.save(&path).unwrap();
        assert_eq!(Settings::load(Some(&path), true), settings);
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn missing_or_broken_files_give_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        assert_eq!(Settings::load(Some(&path), false), Settings::default());
        assert_eq!(Settings::load(None, false), Settings::default());
        fs::write(&path, "not json").unwrap();
        assert_eq!(Settings::load(Some(&path), false), Settings::default());
        // Unknown or missing fields are fine, for older and newer versions.
        fs::write(&path, r#"{ "semantic_icons": true, "something_new": 1 }"#).unwrap();
        assert_eq!(Settings::load(Some(&path), false), Settings { semantic_icons: true, no_colour: false, last_seen_version: None, ..Settings::default() });
    }

    #[test]
    fn no_color_turns_colours_off_until_settings_are_saved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        assert!(Settings::load(Some(&path), true).no_colour);
        Settings::default().save(&path).unwrap();
        assert!(!Settings::load(Some(&path), true).no_colour);
    }

    #[test]
    fn the_folder_row_comes_after_the_toggles_and_opens_for_typing() {
        let mut options = Options::new(Some("~/todoro".into()));
        for _ in 0..5 {
            press(&mut options, KeyCode::Char('j'));
        }
        assert_eq!(options.selected, TOGGLES.len());
        press(&mut options, KeyCode::Enter);
        assert_eq!(options.editing.as_ref().unwrap().text, "~/todoro");
        // Keys are typed, not run.
        for c in "/xq".chars() {
            press(&mut options, KeyCode::Char(c));
        }
        let home = dirs::home_dir().unwrap();
        assert_eq!(press(&mut options, KeyCode::Enter), Action::MoveFolder(home.join("todoro").join("xq")));
        // Esc stops typing without closing.
        assert_eq!(press(&mut options, KeyCode::Esc), Action::Stay);
        assert!(options.editing.is_none());
    }

    #[test]
    fn without_a_folder_there_is_no_folder_row() {
        let mut options = Options::new(None);
        for _ in 0..5 {
            press(&mut options, KeyCode::Char('j'));
        }
        assert_eq!(options.selected, TOGGLES.len() - 1);
    }

    #[test]
    fn keys_move_toggle_and_close() {
        let mut options = Options::default();
        assert_eq!(press(&mut options, KeyCode::Char(' ')), Action::Toggle(0));
        press(&mut options, KeyCode::Char('j'));
        assert_eq!(press(&mut options, KeyCode::Enter), Action::Toggle(1));
        for _ in 0..5 {
            press(&mut options, KeyCode::Down);
        }
        assert_eq!(options.selected, TOGGLES.len() - 1);
        for _ in 0..5 {
            press(&mut options, KeyCode::Char('k'));
        }
        assert_eq!(options.selected, 0);
        for code in [KeyCode::Esc, KeyCode::Char('q'), KeyCode::Char('o')] {
            assert_eq!(press(&mut options, code), Action::Close);
        }
        assert_eq!(press(&mut options, KeyCode::Char('x')), Action::Stay);
    }
}
