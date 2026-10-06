//! The first-run screen, where you choose the todoro folder that every
//! workspace, todo and note goes in, and name your first workspace.

use std::path::PathBuf;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::input::LineInput;
use crate::workspaces::{self, valid_name};

pub struct Setup {
    pub folder: LineInput,
    pub name: LineInput,
    /// Whether the name is being typed, rather than the folder.
    pub on_name: bool,
    /// Why the last attempt didn't work, to show under the fields.
    pub error: Option<String>,
    /// Todos from before the todoro folder existed, to move into it.
    pub moving: Option<PathBuf>,
}

/// What to do after the setup screen handles a key.
#[derive(Debug, PartialEq)]
pub enum Action {
    Stay,
    /// Use this folder, with a workspace of this name.
    Done { dir: PathBuf, name: String },
    Quit,
}

impl Setup {
    /// Starts with `folder` (or `~/todoro`) and "Personal" filled in.
    pub fn new(folder: Option<PathBuf>, moving: Option<PathBuf>) -> Self {
        let folder = folder.unwrap_or_else(workspaces::default_dir);
        Self {
            folder: LineInput::new(&workspaces::display_path(&folder)),
            name: LineInput::new("Personal"),
            on_name: false,
            error: None,
            moving,
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Action {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Action::Quit;
        }
        match key.code {
            KeyCode::Esc => return Action::Quit,
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Up | KeyCode::Down => self.on_name = !self.on_name,
            // Enter on the folder moves on to the name; on the name, finishes.
            KeyCode::Enter if !self.on_name => self.on_name = true,
            KeyCode::Enter => return self.finish(),
            code => {
                let input = if self.on_name { &mut self.name } else { &mut self.folder };
                input.handle_key(code);
                self.error = None;
            }
        }
        Action::Stay
    }

    /// Pasted text goes into whichever field is being typed.
    pub fn paste(&mut self, text: &str) {
        let input = if self.on_name { &mut self.name } else { &mut self.folder };
        input.paste(text);
        self.error = None;
    }

    fn finish(&mut self) -> Action {
        if self.folder.text.trim().is_empty() {
            self.on_name = false;
            self.error = Some("Choose a folder for your todos".into());
            return Action::Stay;
        }
        match valid_name(&self.name.text) {
            Ok(name) => Action::Done { dir: workspaces::expand_path(&self.folder.text), name },
            Err(error) => {
                self.error = Some(error.into());
                Action::Stay
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(setup: &mut Setup, code: KeyCode) -> Action {
        setup.handle_key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn type_str(setup: &mut Setup, text: &str) {
        for c in text.chars() {
            press(setup, KeyCode::Char(c));
        }
    }

    fn clear(setup: &mut Setup) {
        for _ in 0..100 {
            press(setup, KeyCode::Backspace);
        }
    }

    #[test]
    fn it_suggests_a_folder_and_a_name() {
        let setup = Setup::new(None, None);
        assert_eq!(setup.folder.text, workspaces::display_path(&workspaces::default_dir()));
        assert_eq!(setup.name.text, "Personal");
        assert!(!setup.on_name);
    }

    #[test]
    fn enter_moves_to_the_name_then_finishes_with_both() {
        let mut setup = Setup::new(None, None);
        clear(&mut setup);
        type_str(&mut setup, "~/notes/todoro");
        assert_eq!(press(&mut setup, KeyCode::Enter), Action::Stay);
        assert!(setup.on_name);
        clear(&mut setup);
        type_str(&mut setup, " Work ");
        let home = dirs::home_dir().unwrap();
        assert_eq!(
            press(&mut setup, KeyCode::Enter),
            Action::Done { dir: home.join("notes").join("todoro"), name: "Work".into() }
        );
    }

    #[test]
    fn tab_and_arrows_switch_fields() {
        let mut setup = Setup::new(None, None);
        press(&mut setup, KeyCode::Tab);
        assert!(setup.on_name);
        press(&mut setup, KeyCode::Up);
        assert!(!setup.on_name);
    }

    #[test]
    fn a_folder_and_a_usable_name_are_required() {
        let mut setup = Setup::new(None, None);
        clear(&mut setup);
        press(&mut setup, KeyCode::Tab);
        assert_eq!(press(&mut setup, KeyCode::Enter), Action::Stay);
        assert!(setup.error.is_some());
        // It goes back to the folder, the field that needs fixing.
        assert!(!setup.on_name);
        type_str(&mut setup, "~/todoro");
        press(&mut setup, KeyCode::Tab);
        clear(&mut setup);
        type_str(&mut setup, "a/b");
        assert_eq!(press(&mut setup, KeyCode::Enter), Action::Stay);
        assert!(setup.error.is_some());
        // Typing again clears the message.
        press(&mut setup, KeyCode::Backspace);
        assert!(setup.error.is_none());
    }

    #[test]
    fn esc_or_ctrl_c_quits_without_setting_anything_up() {
        assert_eq!(press(&mut Setup::new(None, None), KeyCode::Esc), Action::Quit);
        let mut setup = Setup::new(None, None);
        assert_eq!(setup.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)), Action::Quit);
    }

    #[test]
    fn pasting_goes_into_the_field_being_typed() {
        let mut setup = Setup::new(None, None);
        clear(&mut setup);
        setup.paste("/some/folder\n");
        assert_eq!(setup.folder.text, "/some/folder");
    }
}
