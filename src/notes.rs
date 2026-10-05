//! The notes screen: a multi-line text area with a small set of vim keys.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Color, Modifier, Style};
use ratatui_textarea::{CursorMove, TextArea};

pub struct NotesEditor {
    pub textarea: TextArea<'static>,
    pub insert: bool,
    /// First key of a two-key command (`gg`, `dd`) waiting for its second key.
    pending: Option<char>,
    /// Undo history, kept here rather than in the text area so that every
    /// command (and a whole visit to insert mode) is exactly one step.
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// The text from before insert mode began, recorded when it ends.
    insert_before: Option<Snapshot>,
}

/// The text and cursor at one point, for undo.
#[derive(Clone, PartialEq)]
struct Snapshot {
    lines: Vec<String>,
    cursor: (usize, usize),
}

/// What the app should do after the editor handles a key.
#[derive(Debug, PartialEq)]
pub enum Action {
    Stay,
    Close,
    /// Open the keybinding help.
    Help,
}

impl NotesEditor {
    pub fn new(notes: &str) -> Self {
        let lines = if notes.is_empty() { vec![String::new()] } else { notes.lines().map(String::from).collect() };
        let mut editor = Self {
            textarea: TextArea::default(),
            insert: false,
            pending: None,
            undo: Vec::new(),
            redo: Vec::new(),
            insert_before: None,
        };
        editor.set_lines(lines, (0, 0));
        editor
    }

    /// Replaces the text and puts the cursor at `(row, col)`, clamped to it.
    fn set_lines(&mut self, lines: Vec<String>, (row, col): (usize, usize)) {
        let mut textarea = TextArea::new(lines);
        textarea.set_cursor_line_style(Style::new());
        textarea.set_placeholder_text("No notes yet. Press i to start writing.");
        textarea.set_placeholder_style(Style::new().fg(Color::DarkGray));
        let row = row.min(textarea.lines().len() - 1);
        let col = col.min(textarea.lines()[row].chars().count());
        textarea.move_cursor(CursorMove::Jump(row as u16, col as u16));
        self.textarea = textarea;
        self.update_cursor_style();
        if !self.insert {
            self.clamp();
        }
    }

    fn snapshot(&self) -> Snapshot {
        let cursor = self.textarea.cursor();
        Snapshot { lines: self.textarea.lines().to_vec(), cursor: (cursor.0, cursor.1) }
    }

    /// Adds `before` to the undo history if the text has changed since.
    fn record(&mut self, before: Snapshot) {
        if before.lines != self.textarea.lines() {
            self.undo.push(before);
            self.redo.clear();
        }
    }

    fn undo(&mut self) {
        if let Some(snapshot) = self.undo.pop() {
            self.redo.push(self.snapshot());
            self.set_lines(snapshot.lines, snapshot.cursor);
        }
    }

    fn redo(&mut self) {
        if let Some(snapshot) = self.redo.pop() {
            self.undo.push(self.snapshot());
            self.set_lines(snapshot.lines, snapshot.cursor);
        }
    }

    /// The notes as they should be saved: lines joined with `\n`, trailing
    /// whitespace removed, and empty if there is nothing but whitespace.
    pub fn notes(&self) -> String {
        self.textarea.lines().join("\n").trim_end().to_string()
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Action {
        if self.insert {
            if key.code == KeyCode::Esc {
                self.set_insert(false);
                // Like vim, leaving insert mode steps back onto the last typed character.
                self.back();
                if let Some(before) = self.insert_before.take() {
                    self.record(before);
                }
            } else {
                self.textarea.input(key);
            }
            return Action::Stay;
        }

        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('u') if !ctrl => {
                self.pending = None;
                self.undo();
                return Action::Stay;
            }
            KeyCode::Char('r') if ctrl => {
                self.pending = None;
                self.redo();
                return Action::Stay;
            }
            _ => {}
        }
        // Each command is one undo step. One that starts insert mode becomes a
        // step when insert mode ends, with everything typed in it.
        let before = self.snapshot();
        let action = self.normal_key(key, ctrl);
        if self.insert {
            self.insert_before = Some(before);
        } else {
            self.record(before);
        }
        action
    }

    fn normal_key(&mut self, key: KeyEvent, ctrl: bool) -> Action {
        let KeyCode::Char(c) = key.code else {
            self.pending = None;
            return match key.code {
                KeyCode::Esc => Action::Close,
                KeyCode::Left => self.motion(CursorMove::Back),
                KeyCode::Right => self.motion(CursorMove::Forward),
                KeyCode::Up => self.motion(CursorMove::Up),
                KeyCode::Down => self.motion(CursorMove::Down),
                _ => Action::Stay,
            };
        };

        match (self.pending.take(), c) {
            (Some('g'), 'g') => self.motion_to(CursorMove::Top, CursorMove::Head),
            (Some('d'), 'd') => {
                self.delete_line();
                Action::Stay
            }
            // Ignore other Ctrl combinations so Ctrl+U doesn't act like u.
            _ if ctrl => Action::Stay,
            (_, 'g' | 'd') => {
                self.pending = Some(c);
                Action::Stay
            }
            (_, 'q') => Action::Close,
            (_, '?') => Action::Help,
            (_, 'h') => self.motion(CursorMove::Back),
            (_, 'l') => self.motion(CursorMove::Forward),
            (_, 'j') => self.motion(CursorMove::Down),
            (_, 'k') => self.motion(CursorMove::Up),
            (_, 'w') => self.motion(CursorMove::WordForward),
            (_, 'b') => self.motion(CursorMove::WordBack),
            (_, 'e') => self.motion(CursorMove::WordEnd),
            (_, '0') => self.motion(CursorMove::Head),
            (_, '$') => self.motion(CursorMove::End),
            (_, 'G') => self.motion_to(CursorMove::Bottom, CursorMove::Head),
            (_, 'i') => self.enter_insert(None),
            (_, 'a') => {
                if self.col() < self.line_len() {
                    self.textarea.move_cursor(CursorMove::Forward);
                }
                self.enter_insert(None)
            }
            (_, 'I') => self.enter_insert(Some(CursorMove::Head)),
            (_, 'A') => self.enter_insert(Some(CursorMove::End)),
            (_, 'o') => {
                self.textarea.move_cursor(CursorMove::End);
                self.textarea.insert_newline();
                self.enter_insert(None)
            }
            (_, 'O') => {
                self.textarea.move_cursor(CursorMove::Head);
                self.textarea.insert_newline();
                self.textarea.move_cursor(CursorMove::Up);
                self.enter_insert(None)
            }
            (_, 'x') => {
                // Unlike delete_next_char, x never joins the next line onto this one.
                if self.col() < self.line_len() {
                    self.textarea.delete_next_char();
                    self.clamp();
                }
                Action::Stay
            }
            _ => Action::Stay,
        }
    }

    fn motion(&mut self, m: CursorMove) -> Action {
        // Back and Forward wrap across lines in the text area; vim's h and l don't.
        let blocked = match m {
            CursorMove::Back => self.col() == 0,
            CursorMove::Forward => self.col() + 1 >= self.line_len(),
            _ => false,
        };
        if !blocked {
            self.textarea.move_cursor(m);
        }
        self.clamp();
        Action::Stay
    }

    fn motion_to(&mut self, row: CursorMove, col: CursorMove) -> Action {
        self.textarea.move_cursor(row);
        self.textarea.move_cursor(col);
        Action::Stay
    }

    fn enter_insert(&mut self, m: Option<CursorMove>) -> Action {
        if let Some(m) = m {
            self.textarea.move_cursor(m);
        }
        self.set_insert(true);
        Action::Stay
    }

    /// Deletes the cursor's line as a single undo step.
    fn delete_line(&mut self) {
        let ta = &mut self.textarea;
        let (row, rows) = (ta.cursor().0, ta.lines().len());
        if rows == 1 {
            ta.move_cursor(CursorMove::Head);
            ta.start_selection();
            ta.move_cursor(CursorMove::End);
        } else if row + 1 < rows {
            // Select from the start of this line to the start of the next.
            ta.move_cursor(CursorMove::Head);
            ta.start_selection();
            ta.move_cursor(CursorMove::Down);
            ta.move_cursor(CursorMove::Head);
        } else {
            // Last line: select from the end of the previous line instead.
            ta.move_cursor(CursorMove::Up);
            ta.move_cursor(CursorMove::End);
            ta.start_selection();
            ta.move_cursor(CursorMove::Down);
            ta.move_cursor(CursorMove::End);
        }
        ta.cut();
        ta.move_cursor(CursorMove::Head);
    }

    fn set_insert(&mut self, insert: bool) {
        self.insert = insert;
        self.update_cursor_style();
    }

    fn update_cursor_style(&mut self) {
        let modifier = if self.insert { Modifier::UNDERLINED } else { Modifier::REVERSED };
        self.textarea.set_cursor_style(Style::new().add_modifier(modifier));
    }

    fn back(&mut self) {
        if self.col() > 0 {
            self.textarea.move_cursor(CursorMove::Back);
        }
    }

    /// In normal mode the cursor sits on a character, never past the end of the line.
    fn clamp(&mut self) {
        if self.line_len() > 0 && self.col() >= self.line_len() {
            self.textarea.move_cursor(CursorMove::End);
            self.textarea.move_cursor(CursorMove::Back);
        }
    }

    fn col(&self) -> usize {
        self.textarea.cursor().1
    }

    fn line_len(&self) -> usize {
        self.textarea.lines()[self.textarea.cursor().0].chars().count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor(text: &str) -> NotesEditor {
        NotesEditor::new(text)
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// Sends each character of `keys` as a key press, with `<esc>` for Escape
    /// and `<cr>` for Enter.
    fn send(ed: &mut NotesEditor, keys: &str) -> Action {
        let mut action = Action::Stay;
        let mut rest = keys;
        while let Some(c) = rest.chars().next() {
            let (code, len) = if rest.starts_with("<esc>") {
                (KeyCode::Esc, 5)
            } else if rest.starts_with("<cr>") {
                (KeyCode::Enter, 4)
            } else {
                (KeyCode::Char(c), c.len_utf8())
            };
            action = ed.handle_key(key(code));
            rest = &rest[len..];
        }
        action
    }

    fn cursor(ed: &NotesEditor) -> (usize, usize) {
        let c = ed.textarea.cursor();
        (c.0, c.1)
    }

    #[test]
    fn starts_in_normal_mode_with_the_saved_notes() {
        let ed = editor("one\ntwo");
        assert!(!ed.insert);
        assert_eq!(ed.textarea.lines(), ["one", "two"]);
        assert_eq!(ed.notes(), "one\ntwo");
        assert_eq!(editor("").notes(), "");
    }

    #[test]
    fn i_types_text_and_esc_steps_back_onto_it() {
        let mut ed = editor("");
        send(&mut ed, "ihello");
        assert!(ed.insert);
        send(&mut ed, "<esc>");
        assert!(!ed.insert);
        assert_eq!(ed.notes(), "hello");
        assert_eq!(cursor(&ed), (0, 4));
    }

    #[test]
    fn enter_in_insert_mode_starts_a_new_line() {
        let mut ed = editor("");
        send(&mut ed, "ione<cr>two<esc>");
        assert_eq!(ed.notes(), "one\ntwo");
    }

    #[test]
    fn insert_mode_types_command_keys_as_text() {
        let mut ed = editor("");
        let action = send(&mut ed, "iqhjkldd");
        assert_eq!(action, Action::Stay);
        assert_eq!(ed.notes(), "qhjkldd");
    }

    #[test]
    fn a_appends_after_the_cursor() {
        let mut ed = editor("ac");
        send(&mut ed, "ab<esc>");
        assert_eq!(ed.notes(), "abc");
    }

    #[test]
    fn capital_i_and_a_insert_at_line_start_and_end() {
        let mut ed = editor("mid");
        send(&mut ed, "l");
        send(&mut ed, "I<<esc>A><esc>");
        assert_eq!(ed.notes(), "<mid>");
    }

    #[test]
    fn o_and_capital_o_open_lines_below_and_above() {
        let mut ed = editor("middle");
        send(&mut ed, "obelow<esc>kOabove<esc>");
        assert_eq!(ed.notes(), "above\nmiddle\nbelow");
    }

    #[test]
    fn h_and_l_stay_on_the_line() {
        let mut ed = editor("ab\ncd");
        send(&mut ed, "h");
        assert_eq!(cursor(&ed), (0, 0));
        send(&mut ed, "lll");
        assert_eq!(cursor(&ed), (0, 1));
    }

    #[test]
    fn line_and_document_motions() {
        let mut ed = editor("first line\nsecond\nthird");
        send(&mut ed, "$");
        assert_eq!(cursor(&ed), (0, 9));
        send(&mut ed, "0");
        assert_eq!(cursor(&ed), (0, 0));
        send(&mut ed, "w");
        assert_eq!(cursor(&ed), (0, 6));
        send(&mut ed, "b");
        assert_eq!(cursor(&ed), (0, 0));
        send(&mut ed, "G");
        assert_eq!(cursor(&ed), (2, 0));
        send(&mut ed, "k");
        assert_eq!(cursor(&ed), (1, 0));
        send(&mut ed, "gg");
        assert_eq!(cursor(&ed), (0, 0));
    }

    #[test]
    fn moving_to_a_shorter_line_keeps_the_cursor_on_a_character() {
        let mut ed = editor("long line\nab");
        send(&mut ed, "$j");
        assert_eq!(cursor(&ed), (1, 1));
    }

    #[test]
    fn x_deletes_a_character_but_never_joins_lines() {
        let mut ed = editor("ab\n\ncd");
        send(&mut ed, "x");
        assert_eq!(ed.notes(), "b\n\ncd");
        send(&mut ed, "jx");
        assert_eq!(ed.textarea.lines(), ["b", "", "cd"]);
    }

    #[test]
    fn dd_deletes_a_middle_line() {
        let mut ed = editor("one\ntwo\nthree");
        send(&mut ed, "jdd");
        assert_eq!(ed.notes(), "one\nthree");
        assert_eq!(cursor(&ed), (1, 0));
    }

    #[test]
    fn dd_deletes_the_last_line() {
        let mut ed = editor("one\ntwo");
        send(&mut ed, "Gdd");
        assert_eq!(ed.textarea.lines(), ["one"]);
        assert_eq!(cursor(&ed), (0, 0));
    }

    #[test]
    fn dd_on_the_only_line_clears_it() {
        let mut ed = editor("only");
        send(&mut ed, "ldd");
        assert_eq!(ed.textarea.lines(), [""]);
    }

    #[test]
    fn u_undoes_dd_in_one_step_and_ctrl_r_redoes_it() {
        let mut ed = editor("one\ntwo\nthree");
        send(&mut ed, "jdd");
        send(&mut ed, "u");
        assert_eq!(ed.notes(), "one\ntwo\nthree");
        ed.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
        assert_eq!(ed.notes(), "one\nthree");
    }

    #[test]
    fn u_undoes_everything_typed_in_one_visit_to_insert_mode() {
        let mut ed = editor("start");
        send(&mut ed, "A more words<esc>");
        send(&mut ed, "oa new line<cr>and another<esc>");
        assert_eq!(ed.notes(), "start more words\na new line\nand another");
        send(&mut ed, "u");
        assert_eq!(ed.notes(), "start more words");
        send(&mut ed, "u");
        assert_eq!(ed.notes(), "start");
        // Nothing left to undo.
        send(&mut ed, "u");
        assert_eq!(ed.notes(), "start");
        ed.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
        ed.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
        assert_eq!(ed.notes(), "start more words\na new line\nand another");
    }

    #[test]
    fn undo_puts_the_cursor_back_where_the_change_started() {
        let mut ed = editor("one\ntwo\nthree");
        send(&mut ed, "jlx");
        assert_eq!(cursor(&ed), (1, 1));
        send(&mut ed, "ggu");
        assert_eq!(ed.notes(), "one\ntwo\nthree");
        assert_eq!(cursor(&ed), (1, 1));
    }

    #[test]
    fn moving_around_is_not_an_undo_step() {
        let mut ed = editor("one two");
        send(&mut ed, "x");
        send(&mut ed, "wbe$0i<esc>");
        send(&mut ed, "u");
        assert_eq!(ed.notes(), "one two");
    }

    #[test]
    fn a_new_change_clears_redo() {
        let mut ed = editor("abc");
        send(&mut ed, "xu");
        send(&mut ed, "$x");
        ed.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
        assert_eq!(ed.notes(), "ab");
    }

    #[test]
    fn an_unfinished_command_is_cancelled_by_the_next_key() {
        let mut ed = editor("one\ntwo");
        send(&mut ed, "dj");
        assert_eq!(ed.notes(), "one\ntwo");
        assert_eq!(cursor(&ed), (1, 0));
    }

    #[test]
    fn other_ctrl_keys_do_nothing() {
        let mut ed = editor("one\ntwo");
        send(&mut ed, "jdd");
        ed.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert_eq!(ed.notes(), "one");
    }

    #[test]
    fn question_mark_asks_for_help_from_normal_mode_only() {
        assert_eq!(send(&mut editor(""), "?"), Action::Help);
        assert_eq!(send(&mut editor(""), "i?"), Action::Stay);
    }

    #[test]
    fn esc_and_q_close_from_normal_mode() {
        assert_eq!(send(&mut editor(""), "q"), Action::Close);
        assert_eq!(send(&mut editor(""), "<esc>"), Action::Close);
        assert_eq!(send(&mut editor(""), "i<esc>"), Action::Stay);
    }

    #[test]
    fn notes_trims_trailing_whitespace_and_blank_lines() {
        let mut ed = editor("");
        send(&mut ed, "itext  <cr><cr>  <esc>");
        assert_eq!(ed.notes(), "text");
        let mut blank = editor("");
        send(&mut blank, "i <cr> <esc>");
        assert_eq!(blank.notes(), "");
    }

    #[test]
    fn multibyte_characters_count_as_one() {
        let mut ed = editor("naïve");
        send(&mut ed, "llx");
        assert_eq!(ed.notes(), "nave");
        send(&mut ed, "$");
        assert_eq!(cursor(&ed), (0, 3));
    }
}
