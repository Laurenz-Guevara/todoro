//! The notes viewer (`Space v`): an item's notes as formatted Markdown, to
//! read and scroll, with a key to start editing. `Space V` shows an item's
//! whole block the same way: it and every sub-item, each with its notes, and
//! `Space e` the whole day: every item on the list with its block.

use std::cell::Cell;
use std::path::PathBuf;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::input::LineInput;

#[derive(Default)]
pub struct Viewer {
    /// First visible line.
    pub scroll: usize,
    /// How many lines fit and how many there are once wrapped, recorded when
    /// drawn, so scrolling stops at the end.
    pub height: Cell<usize>,
    pub total: Cell<usize>,
    /// Whether `g` was pressed, waiting for a second `g`.
    pending_g: bool,
    /// What it shows.
    pub scope: Scope,
    /// Saving the day as a text file (`w` on `Space e`), once started.
    pub saving: Option<Saving>,
}

/// The `w` prompt: where to save the day's text.
pub struct Saving {
    /// The path being typed. Nothing means `default`.
    pub input: LineInput,
    pub default: PathBuf,
    /// Set when the file is already there: the next `Enter` replaces it.
    pub replace: bool,
    /// Why it couldn't save, or that `Enter` again replaces the file.
    pub error: Option<String>,
}

impl Saving {
    pub fn new(default: PathBuf) -> Self {
        Saving { input: LineInput::default(), default, replace: false, error: None }
    }
}

/// What the viewer shows.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Scope {
    /// The selected item's notes (`Space v`).
    #[default]
    Notes,
    /// The selected item and all its sub-items, with their notes (`Space V`).
    Item,
    /// Every item on the day's list, each as `Item` shows it (`Space e`).
    Day,
}

/// What the app should do after the viewer handles a key.
#[derive(Debug, PartialEq)]
pub enum Action {
    Stay,
    Close,
    /// Edit the notes, coming back here after.
    Edit,
    Help,
    /// Ask where to save the day (`w`), for the app to fill in where.
    StartSaving,
    /// Save the day where typed (`Saving::input`).
    Save,
}

impl Viewer {
    /// A viewer for an item and all its sub-items.
    pub fn whole() -> Self {
        Viewer { scope: Scope::Item, ..Viewer::default() }
    }

    /// A viewer for every item on the day.
    pub fn day() -> Self {
        Viewer { scope: Scope::Day, ..Viewer::default() }
    }

    /// Whether it shows more than one item's notes, so it's read-only: it
    /// isn't clear whose notes to edit.
    pub fn read_only(&self) -> bool {
        self.scope != Scope::Notes
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Action {
        if let Some(saving) = &mut self.saving {
            match key.code {
                KeyCode::Enter => return Action::Save,
                KeyCode::Esc => self.saving = None,
                code => {
                    let before = saving.input.text.clone();
                    saving.input.handle_key(code);
                    // A different path needs its own check before replacing.
                    if saving.input.text != before {
                        saving.replace = false;
                        saving.error = None;
                    }
                }
            }
            return Action::Stay;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let page = self.height.get().max(1) as isize;
        let pending_g = std::mem::take(&mut self.pending_g);
        match key.code {
            KeyCode::Esc | KeyCode::Char('q' | 'v') => return Action::Close,
            KeyCode::Char('V') if self.scope == Scope::Item => return Action::Close,
            KeyCode::Char('i' | 'e') | KeyCode::Enter if !self.read_only() => return Action::Edit,
            KeyCode::Char('?') => return Action::Help,
            KeyCode::Char('w') if self.scope == Scope::Day => return Action::StartSaving,
            KeyCode::Char('d') if ctrl => self.scroll_by(page / 2),
            KeyCode::Char('u') if ctrl => self.scroll_by(-page / 2),
            KeyCode::Char('f') if ctrl => self.scroll_by(page),
            KeyCode::Char('b') if ctrl => self.scroll_by(-page),
            KeyCode::Char('j') | KeyCode::Down => self.scroll_by(1),
            KeyCode::Char('k') | KeyCode::Up => self.scroll_by(-1),
            KeyCode::PageDown | KeyCode::Char(' ') => self.scroll_by(page),
            KeyCode::PageUp => self.scroll_by(-page),
            KeyCode::Char('g') if pending_g => self.scroll = 0,
            KeyCode::Char('g') => self.pending_g = true,
            KeyCode::Char('G') | KeyCode::End => self.scroll_by(isize::MAX / 2),
            KeyCode::Home => self.scroll = 0,
            _ => {}
        }
        Action::Stay
    }

    fn scroll_by(&mut self, delta: isize) {
        let max = self.total.get().saturating_sub(self.height.get());
        self.scroll = self.scroll.saturating_add_signed(delta).min(max);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(viewer: &mut Viewer, code: KeyCode) -> Action {
        viewer.handle_key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn ctrl(viewer: &mut Viewer, c: char) -> Action {
        viewer.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
    }

    /// A viewer showing 10 of 50 lines.
    fn viewer() -> Viewer {
        let viewer = Viewer::default();
        viewer.height.set(10);
        viewer.total.set(50);
        viewer
    }

    #[test]
    fn keys_scroll_and_stop_at_the_ends() {
        let mut v = viewer();
        press(&mut v, KeyCode::Char('j'));
        press(&mut v, KeyCode::Down);
        assert_eq!(v.scroll, 2);
        press(&mut v, KeyCode::Char('k'));
        assert_eq!(v.scroll, 1);
        ctrl(&mut v, 'd');
        assert_eq!(v.scroll, 6);
        ctrl(&mut v, 'u');
        assert_eq!(v.scroll, 1);
        press(&mut v, KeyCode::Char(' '));
        assert_eq!(v.scroll, 11);
        ctrl(&mut v, 'b');
        assert_eq!(v.scroll, 1);
        press(&mut v, KeyCode::Char('G'));
        assert_eq!(v.scroll, 40, "the last page");
        press(&mut v, KeyCode::Char('j'));
        assert_eq!(v.scroll, 40);
        press(&mut v, KeyCode::Char('g'));
        assert_eq!(v.scroll, 40, "one g does nothing");
        press(&mut v, KeyCode::Char('g'));
        assert_eq!(v.scroll, 0);
        press(&mut v, KeyCode::Char('k'));
        assert_eq!(v.scroll, 0);
    }

    #[test]
    fn w_on_the_day_asks_where_to_save_and_the_prompt_takes_the_keys() {
        assert_eq!(press(&mut Viewer::default(), KeyCode::Char('w')), Action::Stay, "only the day view saves");
        assert_eq!(press(&mut Viewer::whole(), KeyCode::Char('w')), Action::Stay);
        let mut v = Viewer::day();
        assert_eq!(press(&mut v, KeyCode::Char('w')), Action::StartSaving);
        v.saving = Some(Saving::new(PathBuf::from("/home/sam/todoro/export/2026-10-05.txt")));
        // Keys that close or scroll the viewer are typed instead.
        for c in "~/qj".chars() {
            assert_eq!(press(&mut v, KeyCode::Char(c)), Action::Stay);
        }
        assert_eq!(v.saving.as_ref().unwrap().input.text, "~/qj");
        assert_eq!(v.scroll, 0);
        assert_eq!(press(&mut v, KeyCode::Enter), Action::Save);
        // Typing after being asked to replace a file starts over.
        let saving = v.saving.as_mut().unwrap();
        saving.replace = true;
        saving.error = Some("exists".into());
        press(&mut v, KeyCode::Backspace);
        assert!(!v.saving.as_ref().unwrap().replace);
        assert_eq!(v.saving.as_ref().unwrap().error, None);
        // Esc stops saving, not viewing.
        assert_eq!(press(&mut v, KeyCode::Esc), Action::Stay);
        assert!(v.saving.is_none());
        assert_eq!(press(&mut v, KeyCode::Esc), Action::Close);
    }

    #[test]
    fn the_whole_view_is_read_only() {
        let mut v = Viewer::whole();
        for edit in [KeyCode::Char('i'), KeyCode::Char('e'), KeyCode::Enter] {
            assert_eq!(press(&mut v, edit), Action::Stay);
        }
        assert_eq!(press(&mut v, KeyCode::Char('V')), Action::Close);
        assert_eq!(press(&mut Viewer::default(), KeyCode::Char('V')), Action::Stay, "only the whole view closes with V");
    }

    #[test]
    fn the_day_view_is_read_only() {
        let mut v = Viewer::day();
        for edit in [KeyCode::Char('i'), KeyCode::Char('e'), KeyCode::Enter, KeyCode::Char('V')] {
            assert_eq!(press(&mut v, edit), Action::Stay);
        }
        for close in [KeyCode::Esc, KeyCode::Char('q'), KeyCode::Char('v')] {
            assert_eq!(press(&mut v, close), Action::Close);
        }
    }

    #[test]
    fn short_notes_do_not_scroll() {
        let mut v = Viewer::default();
        v.height.set(10);
        v.total.set(4);
        press(&mut v, KeyCode::Char('G'));
        press(&mut v, KeyCode::Char('j'));
        assert_eq!(v.scroll, 0);
    }

    #[test]
    fn keys_edit_close_and_open_help() {
        let mut v = viewer();
        for code in [KeyCode::Char('i'), KeyCode::Char('e'), KeyCode::Enter] {
            assert_eq!(press(&mut v, code), Action::Edit);
        }
        for code in [KeyCode::Esc, KeyCode::Char('q'), KeyCode::Char('v')] {
            assert_eq!(press(&mut v, code), Action::Close);
        }
        assert_eq!(press(&mut v, KeyCode::Char('?')), Action::Help);
        // Keys that edit text elsewhere do nothing here.
        for c in ['x', 'd', 'p', 'u'] {
            assert_eq!(press(&mut v, KeyCode::Char(c)), Action::Stay);
        }
    }
}
