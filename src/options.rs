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
    /// Show deadline times on the 12-hour clock (`11PM`), not the 24-hour
    /// one (`23:00`).
    pub twelve_hour: bool,
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
    Toggle {
        section: "Items",
        label: "12-hour clock",
        description: "Show deadline times like 11PM instead of 23:00. You can type either.",
        get: |s| s.twelve_hour,
        set: |s, on| s.twelve_hour = on,
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
    /// The open workspace's name (not with `TODORO_FILE`), for the rows that
    /// delete things, which come last.
    pub workspace: Option<String>,
    /// A new value being typed for the selected row: a command, a folder,
    /// or the word confirming a deletion.
    pub editing: Option<LineInput>,
    /// The result of the last change: what happened, and whether it worked.
    /// It shows under the selected row.
    pub message: Option<(String, bool)>,
}

/// The rows after the toggles.
pub const EDITOR_ROW: usize = TOGGLES.len();
pub const FOLDER_ROW: usize = TOGGLES.len() + 1;

/// What the last rows of the options delete, for good, in the order shown.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Clear {
    /// Every item and note in the open workspace (or file).
    Items,
    /// Every note in the open workspace (or file), keeping the items.
    Notes,
    /// Every note in every workspace.
    AllNotes,
    /// Every item and note in every workspace, keeping the workspaces.
    AllItems,
    /// Every workspace and the settings, to start again as if newly installed.
    Reset,
}

impl Clear {
    /// What it's called in the options, for the open `workspace` (`None`
    /// with a single todos file).
    pub fn label(self, workspace: Option<&str>) -> String {
        match (self, workspace) {
            (Clear::Items, Some(name)) => format!("Delete all items and notes in {name}"),
            (Clear::Items, None) => "Delete all items and notes".into(),
            (Clear::Notes, Some(name)) => format!("Delete all notes in {name}"),
            (Clear::Notes, None) => "Delete all notes".into(),
            (Clear::AllNotes, _) => "Delete all notes in every workspace".into(),
            (Clear::AllItems, _) => "Delete all items and notes in every workspace".into(),
            (Clear::Reset, _) => "Reset todoro".into(),
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Clear::Items => "Every day's items, and their notes files. The workspace stays, empty.",
            Clear::Notes => "Every item's notes, and the notes files. The items stay.",
            Clear::AllNotes => "Every item's notes in every workspace, and the notes files. The items stay.",
            Clear::AllItems => "Every workspace is emptied of items and notes. The workspaces stay.",
            Clear::Reset => {
                "Deletes every workspace and your settings, then starts again as if newly installed: \
                 choose a folder, then see what's new. Other files in the todoro folder stay."
            }
        }
    }

    /// What to say once it's done.
    pub fn done(self, workspace: Option<&str>) -> String {
        match (self, workspace) {
            (Clear::Items, Some(name)) => format!("Deleted every item and note in {name}"),
            (Clear::Items, None) => "Deleted every item and note".into(),
            (Clear::Notes, Some(name)) => format!("Deleted every note in {name}"),
            (Clear::Notes, None) => "Deleted every note".into(),
            (Clear::AllNotes, _) => "Deleted every note in every workspace".into(),
            (Clear::AllItems, _) => "Deleted every item and note in every workspace".into(),
            (Clear::Reset, _) => "Reset todoro".into(),
        }
    }

    /// What to type to confirm it: the workspace's name for the ones that
    /// only touch it, so it's clear which.
    pub fn confirm_word(self, workspace: Option<&str>) -> String {
        match (self, workspace) {
            (Clear::Items | Clear::Notes, Some(name)) => name.to_string(),
            (Clear::Reset, _) => "reset".into(),
            _ => "delete".into(),
        }
    }
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
    /// Open notes with this command, or todoro's own editor for `None`.
    SetEditor(Option<String>),
    /// Delete this, confirmed.
    Clear(Clear),
}

impl Options {
    pub fn new(editor: Option<&str>, folder: Option<String>, workspace: Option<String>) -> Self {
        Self { editor: editor.unwrap_or_default().to_string(), folder, workspace, ..Self::default() }
    }

    /// The deletions offered: all of them in a workspace, or just this
    /// file's with a single todos file.
    pub fn clears(&self) -> &'static [Clear] {
        if self.workspace.is_some() {
            &[Clear::Items, Clear::Notes, Clear::AllNotes, Clear::AllItems, Clear::Reset]
        } else {
            &[Clear::Items, Clear::Notes]
        }
    }

    /// The row of the first deletion, after the editor and the folder.
    pub fn first_clear_row(&self) -> usize {
        EDITOR_ROW + 1 + usize::from(self.folder.is_some())
    }

    /// The deletion on `row`, if it's one of those rows.
    pub fn clear_at(&self, row: usize) -> Option<Clear> {
        self.clears().get(row.checked_sub(self.first_clear_row())?).copied()
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Action {
        let clear = self.clear_at(self.selected);
        if let Some(input) = &mut self.editing {
            match key.code {
                KeyCode::Esc => self.editing = None,
                KeyCode::Enter if clear.is_some() => {
                    let clear = clear.expect("checked");
                    let word = clear.confirm_word(self.workspace.as_deref());
                    if input.text.trim() == word {
                        self.editing = None;
                        return Action::Clear(clear);
                    }
                    self.message = Some((format!("Type {word} exactly to confirm"), false));
                }
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
        let rows = self.first_clear_row() + self.clears().len();
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
            KeyCode::Char(' ') | KeyCode::Enter if clear.is_some() => {
                self.editing = Some(LineInput::default());
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
        let mut options = Options::new(None, Some("~/todoro".into()), None);
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
        let options = Options::new(None, None, None);
        // The deletions follow the editor straight away.
        assert_eq!(options.clear_at(EDITOR_ROW + 1), Some(Clear::Items));
    }

    #[test]
    fn the_editor_row_comes_after_the_toggles_and_takes_a_command() {
        let mut options = Options::new(None, Some("~/todoro".into()), None);
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
        let mut options = Options::new(Some("hx"), None, None);
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
        for _ in 0..20 {
            press(&mut options, KeyCode::Down);
        }
        assert_eq!(options.selected, options.first_clear_row() + options.clears().len() - 1, "the last row");
        for _ in 0..20 {
            press(&mut options, KeyCode::Char('k'));
        }
        assert_eq!(options.selected, 0);
        for code in [KeyCode::Esc, KeyCode::Char('q'), KeyCode::Char('o')] {
            assert_eq!(press(&mut options, code), Action::Close);
        }
        assert_eq!(press(&mut options, KeyCode::Char('x')), Action::Stay);
    }

    /// Options in a workspace called Work, with a todoro folder.
    fn in_work() -> Options {
        Options::new(None, Some("~/todoro".into()), Some("Work".into()))
    }

    #[test]
    fn the_deletions_come_last_in_order() {
        let options = in_work();
        assert_eq!(options.first_clear_row(), FOLDER_ROW + 1);
        let shown: Vec<Option<Clear>> = (FOLDER_ROW..FOLDER_ROW + 7).map(|row| options.clear_at(row)).collect();
        assert_eq!(
            shown,
            [None, Some(Clear::Items), Some(Clear::Notes), Some(Clear::AllNotes), Some(Clear::AllItems), Some(Clear::Reset), None]
        );
        assert_eq!(Clear::Items.label(Some("Work")), "Delete all items and notes in Work");
        assert_eq!(Clear::Notes.label(Some("Work")), "Delete all notes in Work");
        // j stops on the last one.
        let mut options = in_work();
        for _ in 0..50 {
            press(&mut options, KeyCode::Char('j'));
        }
        assert_eq!(options.clear_at(options.selected), Some(Clear::Reset));
    }

    #[test]
    fn with_a_single_file_only_its_own_deletions_are_offered() {
        let options = Options::new(None, None, None);
        assert_eq!(options.clears(), [Clear::Items, Clear::Notes]);
        assert_eq!(options.first_clear_row(), EDITOR_ROW + 1);
        assert_eq!(Clear::Items.label(None), "Delete all items and notes");
        assert_eq!(Clear::Items.confirm_word(None), "delete");
    }

    #[test]
    fn a_deletion_needs_its_word_typed_to_confirm() {
        for (clear, word) in [
            (Clear::Items, "Work"),
            (Clear::Notes, "Work"),
            (Clear::AllNotes, "delete"),
            (Clear::AllItems, "delete"),
            (Clear::Reset, "reset"),
        ] {
            let mut options = in_work();
            options.selected = options.first_clear_row() + options.clears().iter().position(|&c| c == clear).unwrap();
            // Enter asks; nothing is deleted yet.
            assert_eq!(press(&mut options, KeyCode::Enter), Action::Stay);
            assert!(options.editing.is_some());
            // The wrong word says so and waits.
            for c in "nope".chars() {
                press(&mut options, KeyCode::Char(c));
            }
            assert_eq!(press(&mut options, KeyCode::Enter), Action::Stay, "{clear:?}");
            assert!(!options.message.as_ref().unwrap().1);
            for _ in 0..4 {
                press(&mut options, KeyCode::Backspace);
            }
            for c in word.chars() {
                press(&mut options, KeyCode::Char(c));
            }
            assert_eq!(press(&mut options, KeyCode::Enter), Action::Clear(clear));
            assert!(options.editing.is_none());
        }
    }

    #[test]
    fn esc_backs_out_of_a_deletion() {
        let mut options = in_work();
        options.selected = options.first_clear_row();
        press(&mut options, KeyCode::Enter);
        for c in "Work".chars() {
            press(&mut options, KeyCode::Char(c));
        }
        assert_eq!(press(&mut options, KeyCode::Esc), Action::Stay);
        assert!(options.editing.is_none());
        // Keys like q and o are typed while confirming, not run.
        press(&mut options, KeyCode::Enter);
        assert_eq!(press(&mut options, KeyCode::Char('q')), Action::Stay);
        assert_eq!(options.editing.as_ref().unwrap().text, "q");
    }
}
