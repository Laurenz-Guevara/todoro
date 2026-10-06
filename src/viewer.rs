//! The notes viewer (`v`): an item's notes as formatted Markdown, to read
//! and scroll, with a key to start editing.

use std::cell::Cell;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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
}

/// What the app should do after the viewer handles a key.
#[derive(Debug, PartialEq)]
pub enum Action {
    Stay,
    Close,
    /// Edit the notes, coming back here after.
    Edit,
    Help,
}

impl Viewer {
    pub fn handle_key(&mut self, key: KeyEvent) -> Action {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let page = self.height.get().max(1) as isize;
        let pending_g = std::mem::take(&mut self.pending_g);
        match key.code {
            KeyCode::Esc | KeyCode::Char('q' | 'v') => return Action::Close,
            KeyCode::Char('i' | 'e') | KeyCode::Enter => return Action::Edit,
            KeyCode::Char('?') => return Action::Help,
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
