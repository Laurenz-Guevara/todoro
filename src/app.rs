use std::io;

use chrono::{Days, NaiveDate};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::notes::{Action, NotesEditor};
use crate::store::{Item, Store};

pub enum Mode {
    Normal,
    /// Typing an item at `index`. When `editing`, it replaces the existing item
    /// there; otherwise it is inserted as a new one. `cursor` is a byte offset
    /// into `text`, always on a char boundary.
    Insert { index: usize, text: String, cursor: usize, editing: bool },
    ConfirmDelete,
    /// The notes screen for the selected item.
    Notes(Box<NotesEditor>),
}

pub struct App {
    pub store: Store,
    pub today: NaiveDate,
    pub day: NaiveDate,
    pub selected: usize,
    pub mode: Mode,
    pub quit: bool,
}

impl App {
    pub fn new(store: Store, today: NaiveDate) -> Self {
        Self {
            store,
            today,
            day: today,
            selected: 0,
            mode: Mode::Normal,
            quit: false,
        }
    }

    pub fn items(&self) -> &[Item] {
        self.store.items(self.day)
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> io::Result<()> {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit = true;
            return Ok(());
        }
        match &mut self.mode {
            Mode::Normal => self.normal_key(key.code)?,
            Mode::Insert { index, text, cursor, editing } => match key.code {
                KeyCode::Esc | KeyCode::Enter => {
                    let (index, text, editing) = (*index, text.trim().to_string(), *editing);
                    self.mode = Mode::Normal;
                    match (editing, text.is_empty()) {
                        (true, true) => self.mode = Mode::ConfirmDelete,
                        (true, false) => self.store.set_text(self.day, index, text)?,
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
            Mode::Notes(editor) => {
                let action = editor.handle_key(key);
                // Save on every change, like the rest of the app.
                let notes = editor.notes();
                if notes != self.items()[self.selected].notes {
                    self.store.set_notes(self.day, self.selected, notes)?;
                }
                if action == Action::Close {
                    self.mode = Mode::Normal;
                }
            }
        }
        Ok(())
    }

    fn normal_key(&mut self, code: KeyCode) -> io::Result<()> {
        let len = self.items().len();
        let open = self.store.open_count(self.day);
        match code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('h') | KeyCode::Left => self.change_day(-1),
            KeyCode::Char('l') | KeyCode::Right => self.change_day(1),
            KeyCode::Char('j') | KeyCode::Down if self.selected + 1 < len => self.selected += 1,
            KeyCode::Char('k') | KeyCode::Up => self.selected = self.selected.saturating_sub(1),
            // Like vim's `a`, append after the cursor. From a completed item (or an
            // empty day), append to the end of the open items instead.
            KeyCode::Char('a') => {
                let index = if self.selected < open { self.selected + 1 } else { open };
                self.mode = Mode::Insert { index, text: String::new(), cursor: 0, editing: false };
            }
            KeyCode::Char('e') if len > 0 => {
                let text = self.items()[self.selected].text.clone();
                let cursor = text.len();
                self.mode = Mode::Insert { index: self.selected, text, cursor, editing: true };
            }
            KeyCode::Char('d') if len > 0 => self.mode = Mode::ConfirmDelete,
            // The cursor stays put, so you can tick off several items in a row.
            KeyCode::Char('x') if len > 0 => self.store.toggle_done(self.day, self.selected)?,
            KeyCode::Enter if len > 0 => {
                let editor = NotesEditor::new(&self.items()[self.selected].notes);
                self.mode = Mode::Notes(Box::new(editor));
            }
            _ => {}
        }
        Ok(())
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

#[cfg(test)]
mod tests {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::*;
    use crate::test_util::{app_with, press, today, type_str};

    fn items(app: &App) -> Vec<&str> {
        app.items().iter().map(|item| item.text.as_str()).collect()
    }

    #[test]
    fn starts_on_today_with_first_item_selected() {
        let (app, _dir) = app_with(&["one", "two"]);
        assert_eq!(app.day, today());
        assert_eq!(app.selected, 0);
        assert!(matches!(app.mode, Mode::Normal));
    }

    #[test]
    fn h_and_l_switch_days_and_reset_selection() {
        let (mut app, _dir) = app_with(&["one", "two"]);
        press(&mut app, KeyCode::Char('j'));
        type_str(&mut app, "l");
        assert_eq!(app.day, today().succ_opt().unwrap());
        assert_eq!(app.selected, 0);
        assert!(app.items().is_empty());
        type_str(&mut app, "hh");
        assert_eq!(app.day, today().pred_opt().unwrap());
    }

    #[test]
    fn j_and_k_stay_within_the_list() {
        let (mut app, _dir) = app_with(&["one", "two"]);
        type_str(&mut app, "k");
        assert_eq!(app.selected, 0);
        type_str(&mut app, "jjj");
        assert_eq!(app.selected, 1);
        type_str(&mut app, "k");
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn a_adds_below_the_cursor_and_selects_it() {
        let (mut app, _dir) = app_with(&["one", "two"]);
        type_str(&mut app, "a");
        type_str(&mut app, "new");
        press(&mut app, KeyCode::Enter);
        assert_eq!(items(&app), ["one", "new", "two"]);
        assert_eq!(app.selected, 1);
        assert!(matches!(app.mode, Mode::Normal));
    }

    #[test]
    fn a_on_an_empty_day_adds_the_first_item() {
        let (mut app, _dir) = app_with(&[]);
        type_str(&mut app, "afirst");
        press(&mut app, KeyCode::Esc);
        assert_eq!(items(&app), ["first"]);
    }

    #[test]
    fn insert_typing_hjkl_inserts_letters_instead_of_moving() {
        let (mut app, _dir) = app_with(&[]);
        type_str(&mut app, "ahjkld");
        press(&mut app, KeyCode::Enter);
        assert_eq!(items(&app), ["hjkld"]);
        assert_eq!(app.day, today());
    }

    #[test]
    fn saving_trims_whitespace_and_discards_empty_items() {
        let (mut app, _dir) = app_with(&[]);
        type_str(&mut app, "a  padded  ");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "a   ");
        press(&mut app, KeyCode::Enter);
        assert_eq!(items(&app), ["padded"]);
        assert!(matches!(app.mode, Mode::Normal));
    }

    #[test]
    fn d_then_c_cancels_the_delete() {
        let (mut app, _dir) = app_with(&["one"]);
        type_str(&mut app, "d");
        assert!(matches!(app.mode, Mode::ConfirmDelete));
        type_str(&mut app, "c");
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(items(&app), ["one"]);
    }

    #[test]
    fn other_keys_do_not_dismiss_the_delete_popup() {
        let (mut app, _dir) = app_with(&["one"]);
        type_str(&mut app, "djkx");
        assert!(matches!(app.mode, Mode::ConfirmDelete));
    }

    #[test]
    fn d_then_d_deletes_the_selected_item() {
        let (mut app, _dir) = app_with(&["one", "two", "three"]);
        type_str(&mut app, "jdd");
        assert_eq!(items(&app), ["one", "three"]);
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn deleting_the_last_item_moves_the_selection_up() {
        let (mut app, _dir) = app_with(&["one", "two"]);
        type_str(&mut app, "jdd");
        assert_eq!(items(&app), ["one"]);
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn d_on_an_empty_day_does_nothing() {
        let (mut app, _dir) = app_with(&[]);
        type_str(&mut app, "d");
        assert!(matches!(app.mode, Mode::Normal));
    }

    #[test]
    fn e_edits_the_selected_item_in_place() {
        let (mut app, _dir) = app_with(&["one", "two"]);
        type_str(&mut app, "je!");
        press(&mut app, KeyCode::Enter);
        assert_eq!(items(&app), ["one", "two!"]);
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn clearing_an_edited_item_asks_to_delete_it() {
        let (mut app, _dir) = app_with(&["ab"]);
        type_str(&mut app, "e");
        press(&mut app, KeyCode::Backspace);
        press(&mut app, KeyCode::Backspace);
        press(&mut app, KeyCode::Enter);
        assert!(matches!(app.mode, Mode::ConfirmDelete));
        // Cancelling keeps the original text.
        type_str(&mut app, "c");
        assert_eq!(items(&app), ["ab"]);
    }

    #[test]
    fn cursor_moves_and_edits_inside_the_text() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        type_str(&mut app, "e");
        for _ in 0..4 {
            press(&mut app, KeyCode::Left);
        }
        type_str(&mut app, "oat ");
        press(&mut app, KeyCode::Home);
        press(&mut app, KeyCode::Delete);
        press(&mut app, KeyCode::Delete);
        press(&mut app, KeyCode::Delete);
        type_str(&mut app, "Get");
        press(&mut app, KeyCode::End);
        type_str(&mut app, "!");
        press(&mut app, KeyCode::Enter);
        assert_eq!(items(&app), ["Get oat milk!"]);
    }

    #[test]
    fn cursor_stops_at_both_ends_of_the_text() {
        let (mut app, _dir) = app_with(&["ab"]);
        type_str(&mut app, "e");
        press(&mut app, KeyCode::Right);
        press(&mut app, KeyCode::Delete);
        press(&mut app, KeyCode::Home);
        press(&mut app, KeyCode::Left);
        press(&mut app, KeyCode::Backspace);
        type_str(&mut app, ">");
        press(&mut app, KeyCode::Enter);
        assert_eq!(items(&app), [">ab"]);
    }

    #[test]
    fn cursor_treats_multibyte_characters_as_one() {
        let (mut app, _dir) = app_with(&["naïve"]);
        type_str(&mut app, "e");
        press(&mut app, KeyCode::Left);
        press(&mut app, KeyCode::Left);
        press(&mut app, KeyCode::Backspace);
        type_str(&mut app, "é");
        press(&mut app, KeyCode::Enter);
        assert_eq!(items(&app), ["naéve"]);
    }

    #[test]
    fn enter_opens_the_notes_for_the_selected_item() {
        let (mut app, _dir) = app_with(&["one", "two"]);
        type_str(&mut app, "j");
        press(&mut app, KeyCode::Enter);
        assert!(matches!(app.mode, Mode::Notes(_)));
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn enter_on_an_empty_day_does_nothing() {
        let (mut app, _dir) = app_with(&[]);
        press(&mut app, KeyCode::Enter);
        assert!(matches!(app.mode, Mode::Normal));
    }

    #[test]
    fn notes_are_saved_as_you_type() {
        let (mut app, dir) = app_with(&["one", "two"]);
        type_str(&mut app, "j");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "idetails");
        // Still in insert mode, but already on disk.
        let reloaded = Store::open(dir.path().join("todos.json")).unwrap();
        assert_eq!(reloaded.items(today())[1].notes, "details");
        assert_eq!(reloaded.items(today())[0].notes, "");
    }

    #[test]
    fn leaving_notes_returns_to_the_list_with_the_same_selection() {
        let (mut app, _dir) = app_with(&["one", "two"]);
        type_str(&mut app, "j");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "inote");
        press(&mut app, KeyCode::Esc);
        assert!(matches!(app.mode, Mode::Notes(_)));
        press(&mut app, KeyCode::Esc);
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.selected, 1);
        assert_eq!(app.items()[1].notes, "note");
    }

    #[test]
    fn reopening_notes_shows_what_was_written() {
        let (mut app, _dir) = app_with(&["one"]);
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "ifirst");
        press(&mut app, KeyCode::Esc);
        type_str(&mut app, "q");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "A second");
        press(&mut app, KeyCode::Esc);
        type_str(&mut app, "q");
        assert_eq!(app.items()[0].notes, "first second");
        assert!(!app.quit);
    }

    #[test]
    fn editing_an_item_keeps_its_notes() {
        let (mut app, _dir) = app_with(&["one"]);
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "inote");
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Esc);
        type_str(&mut app, "e!");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.items()[0].text, "one!");
        assert_eq!(app.items()[0].notes, "note");
    }

    #[test]
    fn ctrl_c_in_notes_quits_with_the_notes_saved() {
        let (mut app, dir) = app_with(&["one"]);
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "iunsaved?");
        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)).unwrap();
        assert!(app.quit);
        let reloaded = Store::open(dir.path().join("todos.json")).unwrap();
        assert_eq!(reloaded.items(today())[0].notes, "unsaved?");
    }

    #[test]
    fn x_completes_the_item_and_keeps_the_cursor_in_place() {
        let (mut app, _dir) = app_with(&["one", "two", "three"]);
        type_str(&mut app, "x");
        assert_eq!(items(&app), ["two", "three", "one"]);
        assert!(app.items()[2].done);
        assert_eq!(app.selected, 0);
        // The cursor is now on "two", so x again ticks off the next item.
        type_str(&mut app, "x");
        assert_eq!(items(&app), ["three", "two", "one"]);
        assert_eq!(app.store.open_count(app.day), 1);
    }

    #[test]
    fn x_on_a_completed_item_reopens_it() {
        let (mut app, _dir) = app_with(&["one", "two"]);
        type_str(&mut app, "xj");
        assert_eq!(app.selected, 1);
        type_str(&mut app, "x");
        assert_eq!(items(&app), ["two", "one"]);
        assert!(app.items().iter().all(|item| !item.done));
    }

    #[test]
    fn x_on_an_empty_day_does_nothing() {
        let (mut app, _dir) = app_with(&[]);
        type_str(&mut app, "x");
        assert!(app.items().is_empty());
    }

    #[test]
    fn x_in_insert_mode_types_an_x() {
        let (mut app, _dir) = app_with(&["one"]);
        type_str(&mut app, "ax");
        press(&mut app, KeyCode::Enter);
        assert_eq!(items(&app), ["one", "x"]);
        assert!(app.items().iter().all(|item| !item.done));
    }

    #[test]
    fn a_on_a_completed_item_adds_to_the_end_of_the_open_items() {
        let (mut app, _dir) = app_with(&["one", "two", "three"]);
        type_str(&mut app, "jxj");
        // ["one", "three", "two" (done)], cursor on "two".
        assert_eq!(app.selected, 2);
        type_str(&mut app, "anew");
        press(&mut app, KeyCode::Enter);
        assert_eq!(items(&app), ["one", "three", "new", "two"]);
        assert_eq!(app.selected, 2);
        assert!(!app.items()[2].done);
    }

    #[test]
    fn a_when_everything_is_completed_adds_the_first_open_item() {
        let (mut app, _dir) = app_with(&["one"]);
        type_str(&mut app, "xanew");
        press(&mut app, KeyCode::Enter);
        assert_eq!(items(&app), ["new", "one"]);
    }

    #[test]
    fn completed_items_can_still_be_edited_and_deleted() {
        let (mut app, _dir) = app_with(&["one", "two"]);
        type_str(&mut app, "xj");
        type_str(&mut app, "e!");
        press(&mut app, KeyCode::Enter);
        assert_eq!(items(&app), ["two", "one!"]);
        assert!(app.items()[1].done);
        type_str(&mut app, "dd");
        assert_eq!(items(&app), ["two"]);
    }

    #[test]
    fn q_quits_from_normal_mode_only() {
        let (mut app, _dir) = app_with(&[]);
        type_str(&mut app, "aq");
        assert!(!app.quit);
        press(&mut app, KeyCode::Esc);
        type_str(&mut app, "q");
        assert!(app.quit);
    }

    #[test]
    fn ctrl_c_quits_from_any_mode() {
        let (mut app, _dir) = app_with(&[]);
        type_str(&mut app, "a");
        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)).unwrap();
        assert!(app.quit);
    }
}
