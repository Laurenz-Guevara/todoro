//! The notes screen: a multi-line text area with a small set of vim keys.

use std::ops::RangeInclusive;
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Color, Modifier, Style};
use ratatui_textarea::{CursorMove, TextArea};

use crate::input::LineInput;
use crate::ui::VISUAL_BG;

pub struct NotesEditor {
    pub textarea: TextArea<'static>,
    pub insert: bool,
    /// First key of a two-key command (`gg`, `dd`) waiting for its second key.
    pending: Option<char>,
    /// A count typed before a command, like the 4 in `4j`.
    count: Option<usize>,
    /// The `:` command being typed, if any.
    pub command: Option<LineInput>,
    /// Text deleted or copied, for `p` and `P` to paste. The app keeps it
    /// between notes screens, so lines can move from one item's notes to another's.
    pub register: Option<Register>,
    /// Where a `v` selection started, while selecting.
    pub visual: Option<(usize, usize)>,
    /// Lines just copied, briefly highlighted to show it, and since when.
    pub flash: Option<(RangeInclusive<usize>, Instant)>,
    /// Undo history, kept here rather than in the text area so that every
    /// command (and a whole visit to insert mode) is exactly one step.
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// The text from before insert mode began, recorded when it ends.
    insert_before: Option<Snapshot>,
}

/// How long copied lines stay highlighted.
pub const FLASH: Duration = Duration::from_millis(100);

/// Text to paste: whole lines (from `dd`, `yy`) or part of a line (from `v`).
#[derive(Clone, Debug, PartialEq)]
pub struct Register {
    pub text: String,
    pub linewise: bool,
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
            count: None,
            command: None,
            register: None,
            visual: None,
            flash: None,
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
        textarea.set_line_number_style(Style::new().fg(Color::DarkGray));
        // The text area's default selection is a bright blue that hides the
        // text; use the list's dark selection colour instead.
        textarea.set_selection_style(Style::new().bg(VISUAL_BG));
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
        // Any key ends a copy's flash early.
        self.flash = None;
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

        if let Some(input) = &mut self.command {
            match key.code {
                KeyCode::Esc => self.command = None,
                // Backspace past the : leaves the command line, as in vim.
                KeyCode::Backspace if input.text.is_empty() => self.command = None,
                KeyCode::Enter => {
                    let command = input.text.clone();
                    self.command = None;
                    return self.run_command(&command);
                }
                code => {
                    input.handle_key(code);
                }
            }
            return Action::Stay;
        }

        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('u') if !ctrl && self.visual.is_none() => {
                self.pending = None;
                self.undo();
                return Action::Stay;
            }
            KeyCode::Char('r') if ctrl && self.visual.is_none() => {
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
        let Some(anchor) = self.visual else { return self.command_key(key, ctrl) };
        // While selecting, motions extend the selection, y copies it and d or x
        // cuts it. Other commands do nothing until the selection ends.
        let action = match key.code {
            KeyCode::Esc | KeyCode::Char('v') => {
                self.end_visual();
                Action::Stay
            }
            KeyCode::Char('y') if !ctrl => self.yank_selection(anchor),
            KeyCode::Char('d' | 'x') if !ctrl => self.cut_selection(anchor),
            KeyCode::Char(c) if c.is_ascii_digit() || "hjklwbe$_^Gg".contains(c) => self.command_key(key, ctrl),
            KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down => self.command_key(key, ctrl),
            _ => Action::Stay,
        };
        if self.visual.is_some() {
            self.show_selection(anchor);
        }
        action
    }

    fn command_key(&mut self, key: KeyEvent, ctrl: bool) -> Action {
        let KeyCode::Char(c) = key.code else {
            self.pending = None;
            let times = self.count.take().unwrap_or(1);
            return match key.code {
                KeyCode::Esc => Action::Close,
                KeyCode::Left => self.repeat(times, CursorMove::Back),
                KeyCode::Right => self.repeat(times, CursorMove::Forward),
                KeyCode::Up => self.repeat(times, CursorMove::Up),
                KeyCode::Down => self.repeat(times, CursorMove::Down),
                _ => Action::Stay,
            };
        };
        // Ignore Ctrl combinations other than undo and redo, so Ctrl+U doesn't act like u.
        if ctrl {
            self.pending = None;
            self.count = None;
            return Action::Stay;
        }
        // Digits build a count for the next command, as in vim: 4j moves down
        // four lines and 42G goes to line 42. A 0 only counts after another digit.
        if c.is_ascii_digit() && (c != '0' || self.count.is_some()) {
            let digit = c as usize - '0' as usize;
            self.count = Some((self.count.unwrap_or(0) * 10 + digit).min(99_999));
            return Action::Stay;
        }
        let count = self.count.take();
        let times = count.unwrap_or(1);

        match (self.pending.take(), c) {
            (Some('g'), 'g') => self.go_to_line(count.unwrap_or(1)),
            (Some('d'), 'd') => self.delete_lines(times),
            (Some('y'), 'y') => self.yank_lines(times),
            (_, 'g' | 'd' | 'y') => {
                self.pending = Some(c);
                // Keep the count for the second key, as in 3dd.
                self.count = count;
                Action::Stay
            }
            (_, ':') => {
                self.command = Some(LineInput::default());
                Action::Stay
            }
            (_, 'v') => {
                let cursor = self.textarea.cursor();
                self.visual = Some((cursor.0, cursor.1));
                self.show_selection((cursor.0, cursor.1));
                Action::Stay
            }
            (_, 'q') => Action::Close,
            (_, '?') => Action::Help,
            (_, 'h') => self.repeat(times, CursorMove::Back),
            (_, 'l') => self.repeat(times, CursorMove::Forward),
            (_, 'j') => self.repeat(times, CursorMove::Down),
            (_, 'k') => self.repeat(times, CursorMove::Up),
            (_, 'w') => self.repeat(times, CursorMove::WordForward),
            (_, 'b') => self.repeat(times, CursorMove::WordBack),
            (_, 'e') => self.repeat(times, CursorMove::WordEnd),
            (_, '0') => self.motion(CursorMove::Head),
            (_, '_' | '^') => self.first_non_blank(),
            (_, 'p') => self.paste(times, true),
            (_, 'P') => self.paste(times, false),
            // Move the line down or up, like J and K on the list.
            (_, 'J') => self.move_line(times as isize),
            (_, 'K') => self.move_line(-(times as isize)),
            (_, '$') => self.motion(CursorMove::End),
            (_, 'G') => self.go_to_line(count.unwrap_or(usize::MAX)),
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
                for _ in 0..times {
                    if self.col() < self.line_len() {
                        self.textarea.delete_next_char();
                    }
                }
                self.clamp();
                Action::Stay
            }
            _ => Action::Stay,
        }
    }

    /// Runs a `:` command: a line number to go to it, `:q` (or `:wq`, `:x`)
    /// to go back to the list. Notes save as you type, so `:w` does nothing.
    fn run_command(&mut self, command: &str) -> Action {
        match command.trim() {
            "q" | "q!" | "wq" | "wq!" | "x" | "x!" => Action::Close,
            line => {
                if let Ok(line) = line.parse::<usize>() {
                    self.go_to_line(line);
                }
                Action::Stay
            }
        }
    }

    /// Moves to line `line`, counting from 1 (or the last line), on its first
    /// non-blank character as in vim.
    fn go_to_line(&mut self, line: usize) -> Action {
        let row = line.saturating_sub(1).min(self.textarea.lines().len() - 1);
        self.textarea.move_cursor(CursorMove::Jump(row as u16, 0));
        self.first_non_blank()
    }

    /// Highlights from `anchor` to the cursor, including the characters at
    /// both ends as vim does (the cursor itself covers the one under it).
    fn show_selection(&mut self, anchor: (usize, usize)) {
        let cursor = self.textarea.cursor();
        let cursor = (cursor.0, cursor.1);
        let start = if cursor < anchor { (anchor.0, anchor.1 + 1) } else { anchor };
        self.textarea.cancel_selection();
        self.textarea.move_cursor(CursorMove::Jump(start.0 as u16, start.1 as u16));
        self.textarea.start_selection();
        self.textarea.move_cursor(CursorMove::Jump(cursor.0 as u16, cursor.1 as u16));
    }

    fn end_visual(&mut self) {
        self.textarea.cancel_selection();
        self.visual = None;
    }

    /// The selection's first and last characters, in order.
    fn selection(&self, anchor: (usize, usize)) -> ((usize, usize), (usize, usize)) {
        let cursor = self.textarea.cursor();
        let cursor = (cursor.0, cursor.1);
        (anchor.min(cursor), anchor.max(cursor))
    }

    /// The selected text, from the first to the last character inclusive.
    fn selected_text(&self, anchor: (usize, usize)) -> String {
        let ((r1, c1), (r2, c2)) = self.selection(anchor);
        let lines = self.textarea.lines();
        (r1..=r2)
            .map(|row| {
                let chars = lines[row].chars();
                let from = if row == r1 { c1 } else { 0 };
                let to = if row == r2 { c2 + 1 } else { usize::MAX };
                chars.skip(from).take(to.saturating_sub(from)).collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Copies the selection to paste, and ends it with the cursor at its start.
    fn yank_selection(&mut self, anchor: (usize, usize)) -> Action {
        self.register = Some(Register { text: self.selected_text(anchor), linewise: false });
        let (start, end) = self.selection(anchor);
        self.flash = Some((start.0..=end.0, Instant::now()));
        self.end_visual();
        self.textarea.move_cursor(CursorMove::Jump(start.0 as u16, start.1 as u16));
        Action::Stay
    }

    /// Cuts the selection, keeping it to paste.
    fn cut_selection(&mut self, anchor: (usize, usize)) -> Action {
        self.register = Some(Register { text: self.selected_text(anchor), linewise: false });
        let ((r1, c1), (r2, c2)) = self.selection(anchor);
        let mut lines = self.textarea.lines().to_vec();
        let before: String = lines[r1].chars().take(c1).collect();
        let after: String = lines[r2].chars().skip(c2 + 1).collect();
        lines.splice(r1..=r2, [before + &after]);
        self.visual = None;
        self.set_lines(lines, (r1, c1));
        Action::Stay
    }

    /// Moves the cursor's line `by` lines down (or up), as far as it can go,
    /// keeping the cursor on it.
    fn move_line(&mut self, by: isize) -> Action {
        let mut lines = self.textarea.lines().to_vec();
        let (row, col) = (self.textarea.cursor().0, self.textarea.cursor().1);
        let to = row.saturating_add_signed(by).min(lines.len() - 1);
        if to != row {
            let line = lines.remove(row);
            lines.insert(to, line);
            self.set_lines(lines, (to, col));
        }
        Action::Stay
    }

    /// Moves to the first character on the line that isn't a space or tab.
    fn first_non_blank(&mut self) -> Action {
        let row = self.textarea.cursor().0;
        let col = self.textarea.lines()[row].chars().take_while(|c| c.is_whitespace()).count();
        self.textarea.move_cursor(CursorMove::Jump(row as u16, col as u16));
        self.clamp();
        Action::Stay
    }

    fn repeat(&mut self, times: usize, m: CursorMove) -> Action {
        for _ in 0..times {
            self.motion(m);
        }
        Action::Stay
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

    fn enter_insert(&mut self, m: Option<CursorMove>) -> Action {
        if let Some(m) = m {
            self.textarea.move_cursor(m);
        }
        self.set_insert(true);
        Action::Stay
    }

    /// Deletes `count` lines from the cursor's, keeping them to paste.
    fn delete_lines(&mut self, count: usize) -> Action {
        let mut lines = self.textarea.lines().to_vec();
        let row = self.textarea.cursor().0;
        let end = (row + count).min(lines.len());
        let removed: Vec<String> = lines.drain(row..end).collect();
        self.register = Some(Register { text: removed.join("\n"), linewise: true });
        if lines.is_empty() {
            lines.push(String::new());
        }
        self.set_lines(lines, (row, 0));
        self.first_non_blank()
    }

    /// Copies `count` lines from the cursor's, to paste.
    fn yank_lines(&mut self, count: usize) -> Action {
        let row = self.textarea.cursor().0;
        let lines = self.textarea.lines();
        let end = (row + count).min(lines.len());
        self.register = Some(Register { text: lines[row..end].join("\n"), linewise: true });
        self.flash = Some((row..=end - 1, Instant::now()));
        Action::Stay
    }

    /// Inserts pasted text at the cursor, keeping its lines, as one undo step.
    /// In normal mode it goes where `i` would type it. A paste while typing a
    /// `:` command goes into the command, on one line; while selecting, it's ignored.
    pub fn paste_text(&mut self, text: &str) {
        self.flash = None;
        if let Some(input) = &mut self.command {
            input.paste(text);
            return;
        }
        if self.visual.is_some() {
            return;
        }
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        if self.insert {
            // Recorded with the rest of the typing when insert mode ends.
            self.textarea.insert_str(&text);
            return;
        }
        let before = self.snapshot();
        self.pending = None;
        self.count = None;
        self.textarea.insert_str(&text);
        // Back onto the last character pasted, as after leaving insert mode.
        self.back();
        self.clamp();
        self.record(before);
    }

    /// When the current flash should end, if there is one.
    pub fn flash_ends(&self) -> Option<Instant> {
        self.flash.as_ref().map(|(_, since)| *since + FLASH)
    }

    /// Ends the flash once its time is up at `now`.
    pub fn expire_flash(&mut self, now: Instant) {
        if self.flash_ends().is_some_and(|end| now >= end) {
            self.flash = None;
        }
    }

    /// Pastes the register `times` times: whole lines below (`after`) or above
    /// the cursor's line, or text after or at the cursor.
    fn paste(&mut self, times: usize, after: bool) -> Action {
        let Some(register) = self.register.clone() else { return Action::Stay };
        let text = vec![register.text.as_str(); times].join(if register.linewise { "\n" } else { "" });
        let mut lines = self.textarea.lines().to_vec();
        let (row, col) = (self.textarea.cursor().0, self.textarea.cursor().1);
        if register.linewise {
            // Lines go below or above, with the cursor on the first of them.
            let at = if after { row + 1 } else { row };
            lines.splice(at..at, text.split('\n').map(String::from));
            self.set_lines(lines, (at, 0));
            return self.first_non_blank();
        }
        // Text goes after the cursor's character (or at it), with the cursor
        // ending on its last character.
        let col = if after && !lines[row].is_empty() { col + 1 } else { col };
        let at = lines[row].char_indices().nth(col).map_or(lines[row].len(), |(i, _)| i);
        let rest = lines[row].split_off(at);
        let parts: Vec<&str> = text.split('\n').collect();
        lines[row].push_str(parts[0]);
        let mut end = (row, col + parts[0].chars().count());
        for (n, part) in parts.iter().enumerate().skip(1) {
            lines.insert(row + n, part.to_string());
            end = (row + n, part.chars().count());
        }
        lines[end.0].push_str(&rest);
        self.set_lines(lines, (end.0, end.1.saturating_sub(1)));
        Action::Stay
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

    fn lines(n: usize) -> String {
        (1..=n).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn a_count_repeats_motions() {
        let mut ed = editor(&lines(20));
        send(&mut ed, "4j");
        assert_eq!(cursor(&ed), (4, 0));
        send(&mut ed, "12j3k");
        assert_eq!(cursor(&ed), (13, 0));
        send(&mut ed, "3l");
        assert_eq!(cursor(&ed), (13, 3));
        send(&mut ed, "99j");
        assert_eq!(cursor(&ed), (19, 3));
    }

    #[test]
    fn a_count_before_capital_g_or_gg_goes_to_that_line() {
        let mut ed = editor(&lines(50));
        send(&mut ed, "42G");
        assert_eq!(cursor(&ed), (41, 0));
        send(&mut ed, "7gg");
        assert_eq!(cursor(&ed), (6, 0));
        send(&mut ed, "500G");
        assert_eq!(cursor(&ed), (49, 0));
    }

    #[test]
    fn zero_alone_goes_to_the_line_start_but_counts_after_a_digit() {
        let mut ed = editor(&lines(20));
        send(&mut ed, "$0");
        assert_eq!(cursor(&ed), (0, 0));
        send(&mut ed, "10j");
        assert_eq!(cursor(&ed), (10, 0));
    }

    #[test]
    fn a_count_repeats_x_and_dd_as_one_undo_step() {
        let mut ed = editor("abcdef\ntwo\nthree\nfour");
        send(&mut ed, "3x");
        assert_eq!(ed.notes(), "def\ntwo\nthree\nfour");
        send(&mut ed, "j2dd");
        assert_eq!(ed.notes(), "def\nfour");
        send(&mut ed, "u");
        assert_eq!(ed.notes(), "def\ntwo\nthree\nfour");
    }

    #[test]
    fn colon_and_a_number_goes_to_that_line() {
        let mut ed = editor(&lines(50));
        send(&mut ed, ":42");
        assert_eq!(ed.command.as_ref().unwrap().text, "42");
        assert_eq!(send(&mut ed, "<cr>"), Action::Stay);
        assert!(ed.command.is_none());
        assert_eq!(cursor(&ed), (41, 0));
        send(&mut ed, ":999<cr>");
        assert_eq!(cursor(&ed), (49, 0));
    }

    #[test]
    fn colon_q_wq_and_x_close_and_w_does_nothing() {
        for command in [":q<cr>", ":wq<cr>", ":x<cr>", ":q!<cr>"] {
            assert_eq!(send(&mut editor("a"), command), Action::Close, "{command}");
        }
        let mut ed = editor("a");
        assert_eq!(send(&mut ed, ":w<cr>"), Action::Stay);
        assert_eq!(send(&mut ed, ":nonsense<cr>"), Action::Stay);
        assert_eq!(ed.notes(), "a");
    }

    #[test]
    fn esc_or_backspacing_past_the_colon_leaves_the_command_line() {
        let mut ed = editor(&lines(5));
        assert_eq!(send(&mut ed, ":3<esc>"), Action::Stay);
        assert!(ed.command.is_none());
        assert_eq!(cursor(&ed), (0, 0));
        send(&mut ed, ":3");
        ed.handle_key(key(KeyCode::Backspace));
        ed.handle_key(key(KeyCode::Backspace));
        assert!(ed.command.is_none());
    }

    #[test]
    fn keys_typed_in_the_command_line_are_text() {
        let mut ed = editor("abc");
        send(&mut ed, ":xddu");
        assert_eq!(ed.command.as_ref().unwrap().text, "xddu");
        assert_eq!(ed.notes(), "abc");
    }

    #[test]
    fn underscore_and_caret_go_to_the_first_non_blank_character() {
        let mut ed = editor("    indented line\n\t tabbed\n   ");
        send(&mut ed, "$_");
        assert_eq!(cursor(&ed), (0, 4));
        send(&mut ed, "0^");
        assert_eq!(cursor(&ed), (0, 4));
        send(&mut ed, "j$_");
        assert_eq!(cursor(&ed), (1, 2));
        // A line of only spaces: the last one, as in vim.
        send(&mut ed, "j_");
        assert_eq!(cursor(&ed), (2, 2));
    }

    #[test]
    fn going_to_a_line_lands_on_its_first_non_blank_character() {
        let mut ed = editor("first\n  second\n    third");
        send(&mut ed, "G");
        assert_eq!(cursor(&ed), (2, 4));
        send(&mut ed, ":2<cr>");
        assert_eq!(cursor(&ed), (1, 2));
        send(&mut ed, "gg");
        assert_eq!(cursor(&ed), (0, 0));
    }

    #[test]
    fn capital_j_and_k_move_the_line_and_the_cursor_follows() {
        let mut ed = editor("one\ntwo\nthree\nfour");
        send(&mut ed, "lJ");
        assert_eq!(ed.notes(), "two\none\nthree\nfour");
        assert_eq!(cursor(&ed), (1, 1));
        send(&mut ed, "2J");
        assert_eq!(ed.notes(), "two\nthree\nfour\none");
        assert_eq!(cursor(&ed), (3, 1));
        // It stops at the ends.
        send(&mut ed, "J");
        assert_eq!(ed.notes(), "two\nthree\nfour\none");
        send(&mut ed, "9K");
        assert_eq!(ed.notes(), "one\ntwo\nthree\nfour");
        assert_eq!(cursor(&ed), (0, 1));
    }

    #[test]
    fn moving_a_line_is_one_undo_step() {
        let mut ed = editor("one\ntwo\nthree");
        send(&mut ed, "2Ju");
        assert_eq!(ed.notes(), "one\ntwo\nthree");
        assert_eq!(cursor(&ed), (0, 0));
    }

    #[test]
    fn dd_then_p_moves_a_line_down() {
        let mut ed = editor("one\ntwo\nthree");
        send(&mut ed, "ddp");
        assert_eq!(ed.notes(), "two\none\nthree");
        assert_eq!(cursor(&ed), (1, 0));
    }

    #[test]
    fn capital_p_pastes_above() {
        let mut ed = editor("one\ntwo\nthree");
        send(&mut ed, "Gdd");
        send(&mut ed, "ggP");
        assert_eq!(ed.notes(), "three\none\ntwo");
        assert_eq!(cursor(&ed), (0, 0));
    }

    #[test]
    fn yy_copies_lines_and_counts_work_on_both() {
        let mut ed = editor("one\ntwo\nthree");
        send(&mut ed, "2yyGp");
        assert_eq!(ed.notes(), "one\ntwo\nthree\none\ntwo");
        assert_eq!(cursor(&ed), (3, 0));
        send(&mut ed, "gg3dd");
        assert_eq!(ed.notes(), "one\ntwo");
        send(&mut ed, "2P");
        assert_eq!(ed.notes(), "one\ntwo\nthree\none\ntwo\nthree\none\ntwo");
    }

    #[test]
    fn pasted_lines_keep_their_indent_and_the_cursor_lands_on_the_text() {
        let mut ed = editor("  - item\nnext");
        send(&mut ed, "yyjp");
        assert_eq!(ed.notes(), "  - item\nnext\n  - item");
        assert_eq!(cursor(&ed), (2, 2));
    }

    #[test]
    fn pasting_is_one_undo_step_and_nothing_to_paste_does_nothing() {
        let mut ed = editor("one\ntwo");
        send(&mut ed, "pP");
        assert_eq!(ed.notes(), "one\ntwo");
        send(&mut ed, "yy3pu");
        assert_eq!(ed.notes(), "one\ntwo");
    }

    #[test]
    fn deleting_every_line_leaves_one_empty_line() {
        let mut ed = editor("one\ntwo");
        send(&mut ed, "5dd");
        assert_eq!(ed.textarea.lines(), [""]);
        send(&mut ed, "p");
        assert_eq!(ed.textarea.lines(), ["", "one", "two"]);
    }

    fn register(ed: &NotesEditor) -> (&str, bool) {
        let register = ed.register.as_ref().expect("something copied");
        (register.text.as_str(), register.linewise)
    }

    #[test]
    fn v_selects_and_y_copies_including_both_ends() {
        let mut ed = editor("hello world");
        send(&mut ed, "wve");
        assert!(ed.visual.is_some());
        send(&mut ed, "y");
        assert!(ed.visual.is_none());
        assert_eq!(register(&ed), ("world", false));
        assert_eq!(cursor(&ed), (0, 6));
        assert_eq!(ed.notes(), "hello world");
    }

    #[test]
    fn selecting_backwards_includes_where_it_started() {
        let mut ed = editor("hello world");
        send(&mut ed, "$v4hy");
        assert_eq!(register(&ed), ("world", false));
        assert_eq!(cursor(&ed), (0, 6));
    }

    #[test]
    fn d_or_x_cuts_the_selection_across_lines() {
        let mut ed = editor("one two\nthree four");
        // j keeps the column, so the selection ends on the "e" of "three".
        send(&mut ed, "wvjd");
        assert_eq!(ed.notes(), "one  four");
        assert_eq!(register(&ed), ("two\nthree", false));
        assert_eq!(cursor(&ed), (0, 4));
        send(&mut ed, "u");
        assert_eq!(ed.notes(), "one two\nthree four");
        // Undo left the cursor where it was before the cut: on the "e".
        send(&mut ed, "vx");
        assert_eq!(register(&ed).0, "e");
    }

    #[test]
    fn copied_text_pastes_after_or_at_the_cursor() {
        let mut ed = editor("ab");
        send(&mut ed, "vy");
        // "a" copied; p puts it after the cursor's character.
        send(&mut ed, "p");
        assert_eq!(ed.notes(), "aab");
        assert_eq!(cursor(&ed), (0, 1));
        send(&mut ed, "$P");
        assert_eq!(ed.notes(), "aaab");
        assert_eq!(cursor(&ed), (0, 2));
        send(&mut ed, "02p");
        assert_eq!(ed.notes(), "aaaaab");
    }

    #[test]
    fn copied_text_spanning_lines_pastes_inside_a_line() {
        let mut ed = editor("ab\ncd\nXY");
        send(&mut ed, "lvjy");
        assert_eq!(register(&ed), ("b\ncd", false));
        send(&mut ed, "Gp");
        assert_eq!(ed.notes(), "ab\ncd\nXb\ncdY");
        assert_eq!(cursor(&ed), (3, 1));
    }

    #[test]
    fn esc_or_v_ends_the_selection_without_closing() {
        let mut ed = editor("abc");
        assert_eq!(send(&mut ed, "vl<esc>"), Action::Stay);
        assert!(ed.visual.is_none());
        send(&mut ed, "vv");
        assert!(ed.visual.is_none());
        assert_eq!(send(&mut ed, "<esc>"), Action::Close);
    }

    #[test]
    fn other_commands_do_nothing_while_selecting() {
        let mut ed = editor("abc\ndef");
        send(&mut ed, "viuJpo:q");
        assert!(ed.visual.is_some());
        assert!(!ed.insert);
        assert!(ed.command.is_none());
        assert_eq!(ed.notes(), "abc\ndef");
    }

    #[test]
    fn counts_and_gg_extend_the_selection() {
        let mut ed = editor("one\ntwo\nthree\nfour");
        send(&mut ed, "Gv2kd");
        assert_eq!(ed.notes(), "one\nour");
        // Undo puts the cursor back where the cut started, on line 2.
        send(&mut ed, "u$vggy");
        assert_eq!(register(&ed), ("one\ntwo", false));
    }

    fn flashed(ed: &NotesEditor) -> Option<RangeInclusive<usize>> {
        ed.flash.as_ref().map(|(lines, _)| lines.clone())
    }

    #[test]
    fn copying_lines_flashes_them() {
        let mut ed = editor("one\ntwo\nthree\nfour");
        send(&mut ed, "jyy");
        assert_eq!(flashed(&ed), Some(1..=1));
        send(&mut ed, "j");
        assert_eq!(flashed(&ed), None);
        send(&mut ed, "k3yy");
        assert_eq!(flashed(&ed), Some(1..=3));
        // Past the end, only the lines that exist.
        send(&mut ed, "G5yy");
        assert_eq!(flashed(&ed), Some(3..=3));
    }

    #[test]
    fn copying_a_selection_flashes_its_lines() {
        let mut ed = editor("one\ntwo\nthree");
        send(&mut ed, "lvjy");
        assert_eq!(flashed(&ed), Some(0..=1));
    }

    #[test]
    fn deleting_or_pasting_does_not_flash() {
        let mut ed = editor("one\ntwo");
        send(&mut ed, "yy");
        send(&mut ed, "p");
        assert_eq!(flashed(&ed), None);
        send(&mut ed, "dd");
        assert_eq!(flashed(&ed), None);
    }

    #[test]
    fn the_flash_ends_after_its_time() {
        let mut ed = editor("one");
        send(&mut ed, "yy");
        let (_, since) = ed.flash.clone().unwrap();
        assert_eq!(ed.flash_ends(), Some(since + FLASH));
        ed.expire_flash(since + FLASH / 2);
        assert!(ed.flash.is_some());
        ed.expire_flash(since + FLASH);
        assert!(ed.flash.is_none());
        assert_eq!(ed.flash_ends(), None);
    }

    #[test]
    fn pasting_in_normal_mode_keeps_the_lines_as_one_undo_step() {
        let mut ed = editor("start");
        send(&mut ed, "$");
        ed.paste_text("first line\nsecond line\nthird line");
        assert_eq!(ed.notes(), "starfirst line\nsecond line\nthird linet");
        // The cursor is on the last character pasted, in normal mode.
        assert!(!ed.insert);
        assert_eq!(cursor(&ed), (2, 9));
        send(&mut ed, "u");
        assert_eq!(ed.notes(), "start");
    }

    #[test]
    fn pasting_while_typing_continues_the_typing() {
        let mut ed = editor("");
        send(&mut ed, "ibefore ");
        ed.paste_text("one\ntwo");
        send(&mut ed, " after<esc>");
        assert_eq!(ed.notes(), "before one\ntwo after");
        // Everything typed and pasted in that visit undoes together.
        send(&mut ed, "u");
        assert_eq!(ed.notes(), "");
    }

    #[test]
    fn pasted_windows_line_endings_become_plain_lines() {
        let mut ed = editor("");
        ed.paste_text("one\r\ntwo\rthree");
        assert_eq!(ed.textarea.lines(), ["one", "two", "three"]);
    }

    #[test]
    fn pasted_text_is_never_run_as_commands() {
        let mut ed = editor("keep");
        ed.paste_text("dd\nx\n:q");
        assert_eq!(ed.notes(), "dd\nx\n:qkeep");
        assert!(ed.command.is_none());
    }

    #[test]
    fn pasting_into_a_command_keeps_it_on_one_line() {
        let mut ed = editor("one\ntwo\nthree");
        send(&mut ed, ":");
        ed.paste_text("3\n");
        assert_eq!(send(&mut ed, "<cr>"), Action::Stay);
        assert_eq!(cursor(&ed), (2, 0));
    }

    #[test]
    fn pasting_while_selecting_is_ignored() {
        let mut ed = editor("abc");
        send(&mut ed, "vl");
        ed.paste_text("zzz");
        assert_eq!(ed.notes(), "abc");
        assert!(ed.visual.is_some());
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
