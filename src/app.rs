use std::io;

use chrono::{Days, Local, NaiveDate};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::store::Store;

pub enum Mode {
    Normal,
    /// Typing an item at `index`. When `editing`, it replaces the existing item
    /// there; otherwise it is inserted as a new one. `cursor` is a byte offset
    /// into `text`, always on a char boundary.
    Insert { index: usize, text: String, cursor: usize, editing: bool },
    ConfirmDelete,
}

pub struct App {
    pub store: Store,
    pub day: NaiveDate,
    pub selected: usize,
    pub mode: Mode,
    pub quit: bool,
}

impl App {
    pub fn new(store: Store) -> Self {
        Self {
            store,
            day: Local::now().date_naive(),
            selected: 0,
            mode: Mode::Normal,
            quit: false,
        }
    }

    pub fn items(&self) -> &[String] {
        self.store.items(self.day)
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> io::Result<()> {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit = true;
            return Ok(());
        }
        match &mut self.mode {
            Mode::Normal => self.normal_key(key.code),
            Mode::Insert { index, text, cursor, editing } => match key.code {
                KeyCode::Esc | KeyCode::Enter => {
                    let (index, text, editing) = (*index, text.trim().to_string(), *editing);
                    self.mode = Mode::Normal;
                    match (editing, text.is_empty()) {
                        (true, true) => self.mode = Mode::ConfirmDelete,
                        (true, false) => self.store.set(self.day, index, text)?,
                        (false, true) => {}
                        (false, false) => {
                            self.store.insert(self.day, index, text)?;
                            self.selected = index;
                        }
                    }
                }
                KeyCode::Left => *cursor = prev_boundary(text, *cursor),
                KeyCode::Right => *cursor = next_boundary(text, *cursor),
                KeyCode::Home => *cursor = 0,
                KeyCode::End => *cursor = text.len(),
                KeyCode::Backspace if *cursor > 0 => {
                    *cursor = prev_boundary(text, *cursor);
                    text.remove(*cursor);
                }
                KeyCode::Delete if *cursor < text.len() => {
                    text.remove(*cursor);
                }
                KeyCode::Char(c) => {
                    text.insert(*cursor, c);
                    *cursor += c.len_utf8();
                }
                _ => {}
            },
            Mode::ConfirmDelete => match key.code {
                KeyCode::Char('d') => {
                    self.store.remove(self.day, self.selected)?;
                    self.selected = self.selected.min(self.items().len().saturating_sub(1));
                    self.mode = Mode::Normal;
                }
                KeyCode::Char('c') | KeyCode::Esc => self.mode = Mode::Normal,
                _ => {}
            },
        }
        Ok(())
    }

    fn normal_key(&mut self, code: KeyCode) {
        let len = self.items().len();
        match code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('h') | KeyCode::Left => self.change_day(-1),
            KeyCode::Char('l') | KeyCode::Right => self.change_day(1),
            KeyCode::Char('j') | KeyCode::Down if self.selected + 1 < len => self.selected += 1,
            KeyCode::Char('k') | KeyCode::Up => self.selected = self.selected.saturating_sub(1),
            // Like vim's `a`, append after the cursor (or start the list if empty).
            KeyCode::Char('a') => {
                let index = if len == 0 { 0 } else { self.selected + 1 };
                self.mode = Mode::Insert { index, text: String::new(), cursor: 0, editing: false };
            }
            KeyCode::Char('e') if len > 0 => {
                let text = self.items()[self.selected].clone();
                let cursor = text.len();
                self.mode = Mode::Insert { index: self.selected, text, cursor, editing: true };
            }
            KeyCode::Char('d') if len > 0 => self.mode = Mode::ConfirmDelete,
            _ => {}
        }
    }

    fn change_day(&mut self, delta: i64) {
        let days = Days::new(delta.unsigned_abs());
        let next = if delta < 0 { self.day.checked_sub_days(days) } else { self.day.checked_add_days(days) };
        if let Some(day) = next {
            self.day = day;
            self.selected = 0;
        }
    }
}

fn prev_boundary(text: &str, cursor: usize) -> usize {
    text[..cursor].char_indices().next_back().map_or(0, |(i, _)| i)
}

fn next_boundary(text: &str, cursor: usize) -> usize {
    text[cursor..].chars().next().map_or(cursor, |c| cursor + c.len_utf8())
}
