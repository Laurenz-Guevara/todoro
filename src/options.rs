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
    /// Start new items pinned, so they move on to today until they're done.
    pub pin_new_items: bool,
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
    /// A command to open notes files with, like `nvim`, instead of todoro's
    /// own notes editor.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editor: Option<String>,
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
    Toggle {
        section: "Items",
        label: "Pin new items",
        description: "New items start pinned, so they move on to today until they're done. m unpins one.",
        get: |s| s.pin_new_items,
        set: |s, on| s.pin_new_items = on,
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
    /// The notes editor's command, or empty for todoro's own. It's the row
    /// after the toggles (`EDITOR_ROW`).
    pub editor: String,
    /// The todoro folder as shown, if there is one to change (not with
    /// `TODORO_FILE`). It's the row after the editor (`FOLDER_ROW`).
    pub folder: Option<String>,
    /// A new value being typed for the selected row: a command or a folder.
    pub editing: Option<LineInput>,
    /// The result of the last change: what happened, and whether it worked.
    /// It shows under the selected row.
    pub message: Option<(String, bool)>,
}

/// The rows after the toggles.
pub const EDITOR_ROW: usize = TOGGLES.len();
pub const FOLDER_ROW: usize = TOGGLES.len() + 1;

/// What the app should do after the options popup handles a key.
#[derive(Debug, PartialEq)]
pub enum Action {
    Stay,
    Close,
    /// Flip this entry of `TOGGLES`.
    Toggle(usize),
    /// Move every workspace into this folder.
    MoveFolder(PathBuf),
    /// Open notes with this command, or todoro's own editor for `None`.
    SetEditor(Option<String>),
}

impl Options {
    pub fn new(editor: Option<&str>, folder: Option<String>) -> Self {
        Self { editor: editor.unwrap_or_default().to_string(), folder, ..Self::default() }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Action {
        if let Some(input) = &mut self.editing {
            match key.code {
                KeyCode::Esc => self.editing = None,
                KeyCode::Enter if self.selected == EDITOR_ROW => {
                    let command = input.text.trim();
                    let command = (!command.is_empty()).then(|| command.to_string());
                    self.editor = command.clone().unwrap_or_default();
                    self.editing = None;
                    self.message = Some(match &command {
                        Some(command) => (format!("Notes open in {command}"), true),
                        None => ("Notes open in todoro's editor".into(), true),
                    });
                    return Action::SetEditor(command);
                }
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
        let rows = EDITOR_ROW + 1 + usize::from(self.folder.is_some());
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.selected = (self.selected + 1).min(rows - 1);
                self.message = None;
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
                self.message = None;
            }
            KeyCode::Char(' ') | KeyCode::Enter if self.selected == EDITOR_ROW => {
                self.editing = Some(LineInput::new(&self.editor));
                self.message = None;
            }
            KeyCode::Char(' ') | KeyCode::Enter if self.selected == FOLDER_ROW => {
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
        let mut options = Options::new(None, Some("~/todoro".into()));
        for _ in 0..5 {
            press(&mut options, KeyCode::Char('j'));
        }
        assert_eq!(options.selected, FOLDER_ROW);
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
        let mut options = Options::new(None, None);
        for _ in 0..5 {
            press(&mut options, KeyCode::Char('j'));
        }
        assert_eq!(options.selected, EDITOR_ROW);
    }

    #[test]
    fn the_editor_row_comes_after_the_toggles_and_takes_a_command() {
        let mut options = Options::new(None, Some("~/todoro".into()));
        options.selected = EDITOR_ROW;
        press(&mut options, KeyCode::Enter);
        assert_eq!(options.editing.as_ref().unwrap().text, "");
        // Keys are typed, not run, and spaces around the command go.
        for c in " nvim -p ".chars() {
            press(&mut options, KeyCode::Char(c));
        }
        assert_eq!(press(&mut options, KeyCode::Enter), Action::SetEditor(Some("nvim -p".into())));
        assert!(options.editing.is_none());
        assert_eq!(options.editor, "nvim -p");
        assert!(options.message.as_ref().unwrap().0.contains("nvim -p"));
        // Typing again starts from the command; clearing it goes back to
        // todoro's own editor.
        press(&mut options, KeyCode::Char(' '));
        assert_eq!(options.editing.as_ref().unwrap().text, "nvim -p");
        for _ in 0..10 {
            press(&mut options, KeyCode::Backspace);
        }
        assert_eq!(press(&mut options, KeyCode::Enter), Action::SetEditor(None));
        assert_eq!(options.editor, "");
        // Moving away clears the message.
        press(&mut options, KeyCode::Char('j'));
        assert!(options.message.is_none());
    }

    #[test]
    fn esc_while_typing_a_command_keeps_the_old_one() {
        let mut options = Options::new(Some("hx"), None);
        options.selected = EDITOR_ROW;
        press(&mut options, KeyCode::Enter);
        press(&mut options, KeyCode::Char('x'));
        assert_eq!(press(&mut options, KeyCode::Esc), Action::Stay);
        assert_eq!(options.editor, "hx");
        assert!(options.editing.is_none());
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
        assert_eq!(options.selected, EDITOR_ROW);
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
