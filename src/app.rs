use std::cell::Cell;
use std::io;
use std::path::PathBuf;

use chrono::{Days, NaiveDate};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::calendar::{self, Calendar};
use crate::help::Help;
use crate::input::LineInput;
use crate::notes::{Action, NotesEditor};
use crate::options::{self, Options, Settings, TOGGLES};
use crate::search::{self, Search};
use crate::store::{Item, Snapshot, Store};

/// How many changes `u` can undo.
const UNDO_LIMIT: usize = 200;

/// Every item, plus the day and row on screen, from before (or after) a change.
struct State {
    snapshot: Snapshot,
    day: NaiveDate,
    selected: usize,
}

pub enum Mode {
    Normal,
    /// Typing an item at `index`. When `editing`, it replaces the existing item
    /// there; otherwise it is inserted as a new one.
    Insert { index: usize, input: LineInput, editing: bool },
    ConfirmDelete,
    /// The notes screen for the selected item.
    Notes(Box<NotesEditor>),
    /// The options popup.
    Options(Options),
    /// Fuzzy search over every day's items.
    Search(Box<Search>),
    /// The calendar, for planning ahead.
    Calendar(Box<Calendar>),
    /// The keybinding help popup, over the screen in `back`.
    Help { help: Help, back: Box<Mode> },
}

/// Where an item on the current screen is stored.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slot {
    pub day: NaiveDate,
    pub index: usize,
}

pub struct App {
    pub store: Store,
    pub today: NaiveDate,
    pub day: NaiveDate,
    pub selected: usize,
    /// The first list row on screen. The UI updates it while drawing so the
    /// view only scrolls when the cursor reaches its top or bottom edge.
    pub list_offset: Cell<usize>,
    pub mode: Mode,
    pub quit: bool,
    pub settings: Settings,
    /// Where to save settings when they change, if anywhere.
    pub settings_path: Option<PathBuf>,
    undo: Vec<State>,
    redo: Vec<State>,
    /// The state when the notes screen was opened. Everything typed there is
    /// one change for the list's undo.
    notes_before: Option<State>,
}

impl App {
    pub fn new(store: Store, today: NaiveDate) -> Self {
        Self {
            store,
            today,
            day: today,
            selected: 0,
            list_offset: Cell::new(0),
            mode: Mode::Normal,
            quit: false,
            settings: Settings::default(),
            settings_path: None,
            undo: Vec::new(),
            redo: Vec::new(),
            notes_before: None,
        }
    }

    /// Every item shown for the current day, in screen order. Today and future
    /// days also show, first, the pinned items from earlier days that will have
    /// moved to them by then. Usually startup has already moved them to today,
    /// but one can land on a past day mid-session, e.g. moved there with `<`.
    pub fn slots(&self) -> Vec<Slot> {
        let mut slots: Vec<Slot> = if self.day >= self.today {
            self.store.pinned_before(self.day).into_iter().map(|(day, index)| Slot { day, index }).collect()
        } else {
            Vec::new()
        };
        slots.extend((0..self.store.items(self.day).len()).map(|index| Slot { day: self.day, index }));
        slots
    }

    /// The items shown for the current day, in screen order.
    pub fn items(&self) -> Vec<&Item> {
        self.slots().into_iter().map(|slot| &self.store.items(slot.day)[slot.index]).collect()
    }

    /// How many shown items come from earlier days.
    pub fn carried(&self) -> usize {
        self.slots().iter().take_while(|slot| slot.day != self.day).count()
    }

    fn slot(&self, index: usize) -> Option<Slot> {
        self.slots().get(index).copied()
    }

    fn clamp_selection(&mut self) {
        self.selected = self.selected.min(self.slots().len().saturating_sub(1));
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> io::Result<()> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && key.code == KeyCode::Char('c') {
            self.quit = true;
            return Ok(());
        }
        // Undo works from the list and the calendar, but not while typing.
        let can_undo = match &self.mode {
            Mode::Normal => true,
            Mode::Calendar(calendar) => calendar.adding.is_none(),
            _ => false,
        };
        if can_undo {
            match key.code {
                KeyCode::Char('u') if !ctrl => return self.undo(),
                KeyCode::Char('r') if ctrl => return self.redo(),
                _ => {}
            }
        }
        // Any change made from the list or calendar is undoable. The notes
        // screen records its own change when it closes, and help changes nothing.
        let before = matches!(self.mode, Mode::Normal | Mode::Insert { .. } | Mode::ConfirmDelete | Mode::Calendar(_))
            .then(|| self.state());
        self.mode_key(key)?;
        if let Some(before) = before {
            self.record(before);
        }
        Ok(())
    }

    fn mode_key(&mut self, key: KeyEvent) -> io::Result<()> {
        match &mut self.mode {
            Mode::Normal => self.normal_key(key.code)?,
            Mode::Insert { index, input, editing } => {
                if input.handle_key(key.code) {
                    let (index, text, editing) = (*index, input.text.trim().to_string(), *editing);
                    self.mode = Mode::Normal;
                    match (editing, text.is_empty()) {
                        (true, true) => self.mode = Mode::ConfirmDelete,
                        (true, false) => {
                            if let Some(slot) = self.slot(index) {
                                self.store.set_text(slot.day, slot.index, text)?;
                            }
                        }
                        (false, true) => {}
                        (false, false) => {
                            // New items belong to the day on screen, after any carried ones.
                            let carried = self.carried();
                            let index = index.saturating_sub(carried).min(self.store.open_count(self.day));
                            self.store.insert(self.day, index, text)?;
                            self.selected = carried + index;
                        }
                    }
                }
            }
            Mode::ConfirmDelete => match key.code {
                KeyCode::Char('d') => {
                    if let Some(slot) = self.slot(self.selected) {
                        self.store.remove(slot.day, slot.index)?;
                    }
                    self.clamp_selection();
                    self.mode = Mode::Normal;
                }
                KeyCode::Char('c') | KeyCode::Esc => self.mode = Mode::Normal,
                _ => {}
            },
            Mode::Notes(editor) => {
                let action = editor.handle_key(key);
                // Save on every change, like the rest of the app.
                let notes = editor.notes();
                if let Some(slot) = self.slot(self.selected)
                    && notes != self.store.items(slot.day)[slot.index].notes
                {
                    self.store.set_notes(slot.day, slot.index, notes)?;
                }
                match action {
                    Action::Stay => {}
                    Action::Close => {
                        self.mode = Mode::Normal;
                        if let Some(before) = self.notes_before.take() {
                            self.record(before);
                        }
                    }
                    Action::Help => self.open_help(),
                }
            }
            Mode::Calendar(calendar) => match calendar.handle_key(key) {
                calendar::Action::Stay => {}
                calendar::Action::Close => self.mode = Mode::Normal,
                calendar::Action::Open(day) => {
                    self.mode = Mode::Normal;
                    self.show_day(day);
                }
                calendar::Action::Add(day, text) => {
                    let index = self.store.open_count(day);
                    self.store.insert(day, index, text)?;
                }
                calendar::Action::Help => self.open_help(),
            },
            Mode::Options(popup) => match popup.handle_key(key) {
                options::Action::Stay => {}
                options::Action::Close => self.mode = Mode::Normal,
                options::Action::Toggle(i) => {
                    let toggle = &TOGGLES[i];
                    let on = !(toggle.get)(&self.settings);
                    (toggle.set)(&mut self.settings, on);
                    if let Some(path) = &self.settings_path {
                        self.settings.save(path)?;
                    }
                }
            },
            Mode::Search(search) => {
                let hits = search.find(&self.store, self.today);
                match search.handle_key(key, &hits) {
                    search::Action::Stay => {}
                    search::Action::Close => self.mode = Mode::Normal,
                    search::Action::Open { day, index } => {
                        self.mode = Mode::Normal;
                        self.show_day(day);
                        self.selected = self.slots().iter().position(|slot| *slot == Slot { day, index }).unwrap_or(0);
                    }
                }
            }
            Mode::Help { help, back } => {
                if help.handle_key(key) {
                    self.mode = std::mem::replace(back.as_mut(), Mode::Normal);
                }
            }
        }
        Ok(())
    }

    fn normal_key(&mut self, code: KeyCode) -> io::Result<()> {
        let slot = self.slot(self.selected);
        let len = self.slots().len();
        let open = self.carried() + self.store.open_count(self.day);
        match code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('?') => self.open_help(),
            KeyCode::Char('o') => self.mode = Mode::Options(Options::default()),
            KeyCode::Char('s') => self.mode = Mode::Search(Box::new(Search::new(false))),
            KeyCode::Char('S') => self.mode = Mode::Search(Box::new(Search::new(true))),
            KeyCode::Char('c') => self.mode = Mode::Calendar(Box::new(Calendar::new(self.day, self.today))),
            KeyCode::Char('h') | KeyCode::Left => self.change_day(-1),
            KeyCode::Char('l') | KeyCode::Right => self.change_day(1),
            KeyCode::Char('j') | KeyCode::Down if self.selected + 1 < len => self.selected += 1,
            KeyCode::Char('k') | KeyCode::Up => self.selected = self.selected.saturating_sub(1),
            // Move the item down or up, within the open or completed items.
            KeyCode::Char('J') => self.move_selected(1)?,
            KeyCode::Char('K') => self.move_selected(-1)?,
            // Like h and l, but taking the selected item along to that day.
            KeyCode::Char('H') => self.move_to_day(-1)?,
            KeyCode::Char('L') => self.move_to_day(1)?,
            // Like vim's `a`, append after the cursor. From a completed item (or an
            // empty day), append to the end of the open items instead.
            KeyCode::Char('a') => {
                let index = if self.selected < open { self.selected + 1 } else { open };
                self.mode = Mode::Insert { index, input: LineInput::default(), editing: false };
            }
            KeyCode::Char('e') if len > 0 => {
                let input = LineInput::new(&self.items()[self.selected].text);
                self.mode = Mode::Insert { index: self.selected, input, editing: true };
            }
            KeyCode::Char('d') if len > 0 => self.mode = Mode::ConfirmDelete,
            // The cursor stays put, so you can tick off several items in a row.
            // A carried item is changed where it's stored, which can take it off
            // this day's screen, hence the clamp.
            KeyCode::Char('x') => {
                if let Some(slot) = slot {
                    self.store.toggle_done(slot.day, slot.index)?;
                    self.clamp_selection();
                }
            }
            // Triage: cycle the item's priority.
            KeyCode::Char('t') => {
                if let Some(slot) = slot {
                    self.store.cycle_priority(slot.day, slot.index)?;
                }
            }
            KeyCode::Char('p') => {
                if let Some(slot) = slot {
                    self.store.toggle_pinned(slot.day, slot.index)?;
                    self.clamp_selection();
                }
            }
            KeyCode::Enter if len > 0 => {
                self.notes_before = Some(self.state());
                let editor = NotesEditor::new(&self.items()[self.selected].notes);
                self.mode = Mode::Notes(Box::new(editor));
            }
            _ => {}
        }
        Ok(())
    }

    /// Swaps the selected item with its neighbour `delta` rows away, if both are
    /// stored on the same day, and keeps it selected.
    fn move_selected(&mut self, delta: isize) -> io::Result<()> {
        let Some(other) = self.selected.checked_add_signed(delta) else { return Ok(()) };
        if let (Some(a), Some(b)) = (self.slot(self.selected), self.slot(other))
            && a.day == b.day
            && self.store.swap(a.day, a.index, b.index)?
        {
            self.selected = other;
        }
        Ok(())
    }

    /// Moves the selected item to the day `delta` days from the one on screen
    /// and shows that day with the item selected.
    fn move_to_day(&mut self, delta: i64) -> io::Result<()> {
        let Some(slot) = self.slot(self.selected) else { return Ok(()) };
        let Some(to) = self.day.checked_add_signed(chrono::Duration::days(delta)) else { return Ok(()) };
        if let Some(index) = self.store.move_to(slot.day, slot.index, to)? {
            self.change_day(delta);
            self.selected = self.carried() + index;
        }
        Ok(())
    }

    fn state(&self) -> State {
        State { snapshot: self.store.snapshot(), day: self.day, selected: self.selected }
    }

    /// Adds `before` to the undo history if the items have changed since.
    fn record(&mut self, before: State) {
        if before.snapshot == self.store.snapshot() {
            return;
        }
        if self.undo.len() == UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.undo.push(before);
        self.redo.clear();
    }

    fn undo(&mut self) -> io::Result<()> {
        if let Some(state) = self.undo.pop() {
            self.redo.push(self.state());
            self.restore(state)?;
        }
        Ok(())
    }

    fn redo(&mut self) -> io::Result<()> {
        if let Some(state) = self.redo.pop() {
            self.undo.push(self.state());
            self.restore(state)?;
        }
        Ok(())
    }

    /// Puts the items back and returns to the day and row the change was made
    /// on, so you can see what was undone.
    fn restore(&mut self, state: State) -> io::Result<()> {
        self.store.restore(state.snapshot)?;
        if self.day != state.day {
            self.day = state.day;
            self.list_offset.set(0);
        }
        self.selected = state.selected;
        self.clamp_selection();
        Ok(())
    }

    fn open_help(&mut self) {
        let back = std::mem::replace(&mut self.mode, Mode::Normal);
        self.mode = Mode::Help { help: Help::default(), back: Box::new(back) };
    }

    fn change_day(&mut self, delta: i64) {
        let days = Days::new(delta.unsigned_abs());
        let next = if delta < 0 { self.day.checked_sub_days(days) } else { self.day.checked_add_days(days) };
        if let Some(day) = next {
            self.show_day(day);
        }
    }

    /// Shows `day` on the list, from the top.
    fn show_day(&mut self, day: NaiveDate) {
        self.day = day;
        self.selected = 0;
        self.list_offset.set(0);
    }
}

#[cfg(test)]
mod tests {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::*;
    use crate::test_util::{app_with, press, today, type_str};

    fn texts_on(app: &App, day: NaiveDate) -> Vec<&str> {
        app.store.items(day).iter().map(|item| item.text.as_str()).collect()
    }

    fn items(app: &App) -> Vec<&str> {
        app.items().into_iter().map(|item| item.text.as_str()).collect()
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
    fn question_mark_opens_help_and_esc_returns_to_the_list() {
        let (mut app, _dir) = app_with(&["one", "two"]);
        type_str(&mut app, "j?");
        assert!(matches!(app.mode, Mode::Help { .. }));
        press(&mut app, KeyCode::Esc);
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn keys_typed_in_help_search_instead_of_acting_on_the_list() {
        let (mut app, _dir) = app_with(&["one", "two"]);
        type_str(&mut app, "?xjdaq");
        let Mode::Help { help, .. } = &app.mode else { panic!("help should still be open") };
        assert_eq!(help.query, "xjdaq");
        assert_eq!(items(&app), ["one", "two"]);
        assert!(app.items().iter().all(|item| !item.done));
        assert_eq!(app.selected, 0);
        assert!(!app.quit);
    }

    #[test]
    fn help_from_notes_returns_to_the_notes_as_they_were() {
        let (mut app, _dir) = app_with(&["one"]);
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "idraft");
        press(&mut app, KeyCode::Esc);
        type_str(&mut app, "?undo");
        assert!(matches!(app.mode, Mode::Help { .. }));
        press(&mut app, KeyCode::Esc);
        let Mode::Notes(editor) = &app.mode else { panic!("should be back on the notes screen") };
        assert!(!editor.insert);
        assert_eq!(editor.notes(), "draft");
        // The help search didn't leak into the notes.
        assert_eq!(app.items()[0].notes, "draft");
    }

    #[test]
    fn question_mark_while_typing_is_text() {
        let (mut app, _dir) = app_with(&["one"]);
        type_str(&mut app, "e?");
        press(&mut app, KeyCode::Enter);
        assert_eq!(items(&app), ["one?"]);
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "iwhy?");
        let Mode::Notes(editor) = &app.mode else { panic!("should be on the notes screen") };
        assert_eq!(editor.notes(), "why?");
    }

    #[test]
    fn ctrl_c_quits_from_help() {
        let (mut app, _dir) = app_with(&[]);
        type_str(&mut app, "?");
        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)).unwrap();
        assert!(app.quit);
    }

    #[test]
    fn p_pins_and_unpins_the_selected_item_in_place() {
        let (mut app, _dir) = app_with(&["one", "two"]);
        type_str(&mut app, "jp");
        assert_eq!(items(&app), ["one", "two"]);
        assert!(!app.items()[0].pinned);
        assert!(app.items()[1].pinned);
        assert_eq!(app.selected, 1);
        type_str(&mut app, "p");
        assert!(!app.items()[1].pinned);
    }

    #[test]
    fn p_on_an_empty_day_does_nothing() {
        let (mut app, _dir) = app_with(&[]);
        type_str(&mut app, "p");
        assert!(app.items().is_empty());
    }

    #[test]
    fn p_while_typing_is_text() {
        let (mut app, _dir) = app_with(&["one"]);
        type_str(&mut app, "e p");
        press(&mut app, KeyCode::Enter);
        assert_eq!(items(&app), ["one p"]);
        assert!(!app.items()[0].pinned);
    }

    #[test]
    fn completing_a_pinned_item_keeps_it_pinned() {
        let (mut app, _dir) = app_with(&["one"]);
        type_str(&mut app, "px");
        assert!(app.items()[0].done);
        assert!(app.items()[0].pinned);
    }

    /// An app with `items` today and `future` on the day after tomorrow, with
    /// today's items that start with "pin" pinned.
    fn app_with_future(items: &[&str], future: &[&str]) -> (App, tempfile::TempDir) {
        let (mut app, dir) = app_with(items);
        for (i, text) in items.iter().enumerate() {
            if text.starts_with("pin") {
                app.store.toggle_pinned(today(), i).unwrap();
            }
        }
        let later = today() + chrono::Duration::days(2);
        for (i, text) in future.iter().enumerate() {
            app.store.insert(later, i, text.to_string()).unwrap();
        }
        (app, dir)
    }

    #[test]
    fn future_days_show_pinned_items_first() {
        let (mut app, _dir) = app_with_future(&["pin a", "plain", "pin b"], &["later"]);
        type_str(&mut app, "l");
        assert_eq!(items(&app), ["pin a", "pin b"]);
        assert_eq!(app.carried(), 2);
        type_str(&mut app, "l");
        assert_eq!(items(&app), ["pin a", "pin b", "later"]);
        // Today and past days are unchanged.
        type_str(&mut app, "hh");
        assert_eq!(items(&app), ["pin a", "plain", "pin b"]);
        assert_eq!(app.carried(), 0);
        type_str(&mut app, "h");
        assert!(app.items().is_empty());
    }

    #[test]
    fn completed_pinned_items_do_not_show_on_future_days() {
        let (mut app, _dir) = app_with_future(&["pin a", "pin b"], &[]);
        type_str(&mut app, "x");
        type_str(&mut app, "l");
        assert_eq!(items(&app), ["pin b"]);
    }

    #[test]
    fn x_on_a_carried_item_completes_it_where_it_is_stored() {
        let (mut app, _dir) = app_with_future(&["pin a", "pin b"], &["later"]);
        type_str(&mut app, "llx");
        assert_eq!(items(&app), ["pin b", "later"]);
        assert_eq!(app.selected, 0);
        assert!(app.store.items(today()).iter().any(|item| item.text == "pin a" && item.done));
    }

    #[test]
    fn x_on_the_last_carried_item_keeps_the_cursor_in_range() {
        let (mut app, _dir) = app_with_future(&["pin a"], &[]);
        type_str(&mut app, "lx");
        assert!(app.items().is_empty());
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn p_on_a_carried_item_unpins_it_back_to_its_day() {
        let (mut app, _dir) = app_with_future(&["pin a"], &["later"]);
        type_str(&mut app, "llp");
        assert_eq!(items(&app), ["later"]);
        assert!(!app.store.items(today())[0].pinned);
    }

    #[test]
    fn editing_and_notes_on_a_carried_item_change_the_original() {
        let (mut app, _dir) = app_with_future(&["pin a"], &["later"]);
        type_str(&mut app, "lle!");
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "inote");
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.store.items(today())[0].text, "pin a!");
        assert_eq!(app.store.items(today())[0].notes, "note");
        let later = today() + chrono::Duration::days(2);
        assert_eq!(app.store.items(later)[0].text, "later");
        assert_eq!(app.store.items(later)[0].notes, "");
    }

    #[test]
    fn d_on_a_carried_item_deletes_the_original() {
        let (mut app, _dir) = app_with_future(&["pin a", "plain"], &[]);
        type_str(&mut app, "ldd");
        assert!(app.items().is_empty());
        assert_eq!(app.store.items(today()).len(), 1);
        assert_eq!(app.store.items(today())[0].text, "plain");
    }

    #[test]
    fn a_on_a_future_day_adds_to_that_day_after_carried_items() {
        let (mut app, _dir) = app_with_future(&["pin a", "pin b"], &["later"]);
        type_str(&mut app, "ll");
        // From the first carried item, the new item still goes after all of them.
        type_str(&mut app, "anew");
        press(&mut app, KeyCode::Enter);
        assert_eq!(items(&app), ["pin a", "pin b", "new", "later"]);
        assert_eq!(app.selected, 2);
        let later = today() + chrono::Duration::days(2);
        assert_eq!(app.store.items(later).len(), 2);
        assert_eq!(app.store.items(today()).len(), 2);
    }

    #[test]
    fn capital_j_and_k_move_the_item_and_the_cursor_follows() {
        let (mut app, _dir) = app_with(&["one", "two", "three"]);
        type_str(&mut app, "J");
        assert_eq!(items(&app), ["two", "one", "three"]);
        assert_eq!(app.selected, 1);
        type_str(&mut app, "JJ");
        assert_eq!(items(&app), ["two", "three", "one"]);
        assert_eq!(app.selected, 2);
        type_str(&mut app, "KK");
        assert_eq!(items(&app), ["one", "two", "three"]);
        assert_eq!(app.selected, 0);
        type_str(&mut app, "K");
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn capital_j_and_k_never_cross_into_the_completed_items() {
        let (mut app, _dir) = app_with(&["one", "two", "three", "four"]);
        type_str(&mut app, "jjxx");
        // ["one", "two", "four" (done), "three" (done)], cursor on "four".
        type_str(&mut app, "K");
        assert_eq!(items(&app), ["one", "two", "four", "three"]);
        type_str(&mut app, "J");
        assert_eq!(items(&app), ["one", "two", "three", "four"]);
        assert_eq!(app.selected, 3);
        type_str(&mut app, "kk");
        type_str(&mut app, "J");
        assert_eq!(items(&app), ["one", "two", "three", "four"]);
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn carried_items_reorder_among_themselves_but_not_with_the_days_own() {
        let (mut app, _dir) = app_with_future(&["pin a", "pin b"], &["later"]);
        type_str(&mut app, "llJ");
        assert_eq!(items(&app), ["pin b", "pin a", "later"]);
        assert_eq!(app.store.items(today())[0].text, "pin b");
        type_str(&mut app, "J");
        assert_eq!(items(&app), ["pin b", "pin a", "later"]);
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn capital_j_while_typing_is_text() {
        let (mut app, _dir) = app_with(&["one"]);
        type_str(&mut app, "aJK");
        press(&mut app, KeyCode::Enter);
        assert_eq!(items(&app), ["one", "JK"]);
    }

    #[test]
    fn capital_l_moves_the_item_to_the_next_day_and_follows_it() {
        let (mut app, _dir) = app_with(&["one", "two"]);
        let tomorrow = today().succ_opt().unwrap();
        app.store.insert(tomorrow, 0, "t1".into()).unwrap();
        type_str(&mut app, "jL");
        assert_eq!(app.day, tomorrow);
        assert_eq!(items(&app), ["t1", "two"]);
        assert_eq!(app.selected, 1);
        assert_eq!(texts_on(&app, today()), ["one"]);
        // Again, and it keeps going.
        type_str(&mut app, "LL");
        assert_eq!(app.day, tomorrow + chrono::Duration::days(2));
        assert_eq!(items(&app), ["two"]);
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn capital_h_moves_the_item_to_the_previous_day() {
        let (mut app, _dir) = app_with(&["one"]);
        type_str(&mut app, "H");
        assert_eq!(app.day, today().pred_opt().unwrap());
        assert_eq!(items(&app), ["one"]);
        type_str(&mut app, "ll");
        assert!(app.items().is_empty());
    }

    #[test]
    fn moving_on_an_empty_day_does_nothing() {
        let (mut app, _dir) = app_with(&[]);
        type_str(&mut app, "LH");
        assert_eq!(app.day, today());
    }

    #[test]
    fn moving_a_completed_item_keeps_it_completed() {
        let (mut app, _dir) = app_with(&["one", "two"]);
        type_str(&mut app, "x");
        type_str(&mut app, "jL");
        // "one" was completed, so it lands among tomorrow's completed items.
        assert_eq!(items(&app), ["one"]);
        assert!(app.items()[0].done);
    }

    #[test]
    fn moving_lands_after_the_carried_items_on_a_future_day() {
        let (mut app, _dir) = app_with_future(&["pin a", "plain"], &[]);
        type_str(&mut app, "jL");
        assert_eq!(items(&app), ["pin a", "plain"]);
        assert_eq!(app.carried(), 1);
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn moving_a_carried_item_takes_it_off_the_earlier_days() {
        let (mut app, _dir) = app_with_future(&["pin a"], &[]);
        type_str(&mut app, "lL");
        let later = today() + chrono::Duration::days(2);
        assert_eq!(app.day, later);
        assert_eq!(items(&app), ["pin a"]);
        assert_eq!(app.carried(), 0);
        type_str(&mut app, "h");
        assert!(app.items().is_empty());
        assert!(app.store.items(today()).is_empty());
    }

    #[test]
    fn angle_brackets_no_longer_move_items() {
        let (mut app, _dir) = app_with(&["one"]);
        type_str(&mut app, "><");
        assert_eq!(app.day, today());
        assert_eq!(items(&app), ["one"]);
    }

    #[test]
    fn capital_h_and_l_while_typing_are_text() {
        let (mut app, _dir) = app_with(&["one"]);
        type_str(&mut app, "aHL");
        press(&mut app, KeyCode::Enter);
        assert_eq!(items(&app), ["one", "HL"]);
        assert_eq!(app.day, today());
    }

    fn ctrl(app: &mut App, c: char) {
        app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)).unwrap();
    }

    #[test]
    fn u_undoes_completing_and_ctrl_r_redoes_it() {
        let (mut app, _dir) = app_with(&["one", "two"]);
        type_str(&mut app, "x");
        assert_eq!(items(&app), ["two", "one"]);
        type_str(&mut app, "u");
        assert_eq!(items(&app), ["one", "two"]);
        assert!(app.items().iter().all(|item| !item.done));
        ctrl(&mut app, 'r');
        assert_eq!(items(&app), ["two", "one"]);
        assert!(app.items()[1].done);
    }

    #[test]
    fn u_brings_back_a_deleted_item_and_selects_it() {
        let (mut app, _dir) = app_with(&["one", "two", "three"]);
        type_str(&mut app, "jdd");
        assert_eq!(items(&app), ["one", "three"]);
        type_str(&mut app, "ju");
        assert_eq!(items(&app), ["one", "two", "three"]);
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn u_undoes_adding_editing_pinning_and_reordering() {
        let (mut app, _dir) = app_with(&["one"]);
        type_str(&mut app, "atwo");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "e!");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "pK");
        assert_eq!(items(&app), ["two!", "one"]);
        assert!(app.items()[0].pinned);
        type_str(&mut app, "u");
        assert_eq!(items(&app), ["one", "two!"]);
        type_str(&mut app, "u");
        assert!(!app.items()[1].pinned);
        type_str(&mut app, "u");
        assert_eq!(items(&app), ["one", "two"]);
        type_str(&mut app, "u");
        assert_eq!(items(&app), ["one"]);
        // Nothing left to undo.
        type_str(&mut app, "u");
        assert_eq!(items(&app), ["one"]);
    }

    #[test]
    fn u_after_moving_to_another_day_goes_back_with_the_item() {
        let (mut app, _dir) = app_with(&["one", "two"]);
        type_str(&mut app, "jLL");
        type_str(&mut app, "u");
        assert_eq!(app.day, today().succ_opt().unwrap());
        assert_eq!(items(&app), ["two"]);
        type_str(&mut app, "u");
        assert_eq!(app.day, today());
        assert_eq!(items(&app), ["one", "two"]);
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn u_undoes_a_whole_notes_visit_in_one_step() {
        let (mut app, _dir) = app_with(&["one"]);
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "ifirst line");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "second");
        press(&mut app, KeyCode::Esc);
        type_str(&mut app, "?");
        press(&mut app, KeyCode::Esc);
        type_str(&mut app, "q");
        assert_eq!(app.items()[0].notes, "first line\nsecond");
        type_str(&mut app, "u");
        assert_eq!(app.items()[0].notes, "");
        ctrl(&mut app, 'r');
        assert_eq!(app.items()[0].notes, "first line\nsecond");
    }

    #[test]
    fn opening_notes_without_changing_them_is_not_a_change() {
        let (mut app, _dir) = app_with(&["one"]);
        type_str(&mut app, "x");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "q");
        type_str(&mut app, "u");
        assert!(!app.items()[0].done);
    }

    #[test]
    fn moving_the_cursor_and_changing_day_are_not_changes() {
        let (mut app, _dir) = app_with(&["one", "two"]);
        type_str(&mut app, "x");
        type_str(&mut app, "jkllhhJ");
        type_str(&mut app, "u");
        assert!(app.items().iter().all(|item| !item.done));
    }

    #[test]
    fn a_new_change_clears_redo() {
        let (mut app, _dir) = app_with(&["one", "two"]);
        type_str(&mut app, "xu");
        type_str(&mut app, "p");
        ctrl(&mut app, 'r');
        assert!(app.items().iter().all(|item| !item.done));
        assert!(app.items()[0].pinned);
    }

    #[test]
    fn undo_is_saved_to_disk() {
        let (mut app, dir) = app_with(&["one"]);
        type_str(&mut app, "ddu");
        let reloaded = Store::open(dir.path().join("todos.json")).unwrap();
        assert_eq!(reloaded.items(today())[0].text, "one");
    }

    #[test]
    fn u_while_typing_is_text() {
        let (mut app, _dir) = app_with(&["one"]);
        type_str(&mut app, "au");
        press(&mut app, KeyCode::Enter);
        assert_eq!(items(&app), ["one", "u"]);
    }

    #[test]
    fn undo_history_is_limited() {
        let (mut app, _dir) = app_with(&["one"]);
        for _ in 0..UNDO_LIMIT + 10 {
            type_str(&mut app, "p");
        }
        assert_eq!(app.undo.len(), UNDO_LIMIT);
    }

    #[test]
    fn a_pinned_item_moved_to_yesterday_still_shows_today_and_after() {
        let (mut app, _dir) = app_with(&["pin a", "plain"]);
        app.store.toggle_pinned(today(), 0).unwrap();
        type_str(&mut app, "H");
        assert_eq!(app.day, today().pred_opt().unwrap());
        assert_eq!(items(&app), ["pin a"]);
        type_str(&mut app, "l");
        assert_eq!(items(&app), ["pin a", "plain"]);
        assert_eq!(app.carried(), 1);
        type_str(&mut app, "l");
        assert_eq!(items(&app), ["pin a"]);
    }

    #[test]
    fn pinning_an_item_on_a_past_day_shows_it_today() {
        let (mut app, _dir) = app_with(&["plain"]);
        let yesterday = today().pred_opt().unwrap();
        app.store.insert(yesterday, 0, "old".into()).unwrap();
        type_str(&mut app, "hpl");
        assert_eq!(items(&app), ["old", "plain"]);
        // Unpinning it from today sends it back to its own day only.
        type_str(&mut app, "p");
        assert_eq!(items(&app), ["plain"]);
        type_str(&mut app, "h");
        assert_eq!(items(&app), ["old"]);
    }

    fn calendar(app: &App) -> &Calendar {
        let Mode::Calendar(calendar) = &app.mode else { panic!("the calendar should be open") };
        calendar
    }

    #[test]
    fn c_opens_the_calendar_on_the_day_shown() {
        let (mut app, _dir) = app_with(&["one"]);
        type_str(&mut app, "lc");
        assert_eq!(calendar(&app).cursor, today().succ_opt().unwrap());
        assert_eq!(calendar(&app).today, today());
    }

    #[test]
    fn esc_closes_the_calendar_back_on_the_same_day() {
        let (mut app, _dir) = app_with(&["one", "two"]);
        type_str(&mut app, "jcjjl");
        press(&mut app, KeyCode::Esc);
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.day, today());
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn enter_in_the_calendar_opens_that_day() {
        let (mut app, _dir) = app_with(&["one", "two"]);
        type_str(&mut app, "jcjl");
        press(&mut app, KeyCode::Enter);
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.day, today() + chrono::Duration::days(8));
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn a_in_the_calendar_adds_to_the_selected_day() {
        let (mut app, dir) = app_with(&["one"]);
        let later = today() + chrono::Duration::days(14);
        app.store.insert(later, 0, "already there".into()).unwrap();
        app.store.insert(later, 1, "finished".into()).unwrap();
        app.store.toggle_done(later, 1).unwrap();
        type_str(&mut app, "cjjaDentist 3pm");
        press(&mut app, KeyCode::Enter);
        // Still in the calendar, and the list's day is unchanged.
        assert!(calendar(&app).adding.is_none());
        assert_eq!(app.day, today());
        assert_eq!(texts_on(&app, later), ["already there", "Dentist 3pm", "finished"]);
        let reloaded = Store::open(dir.path().join("todos.json")).unwrap();
        assert_eq!(reloaded.items(later)[1].text, "Dentist 3pm");
    }

    #[test]
    fn u_in_the_calendar_undoes_an_add_and_ctrl_r_redoes_it() {
        let (mut app, _dir) = app_with(&[]);
        let tomorrow = today().succ_opt().unwrap();
        type_str(&mut app, "claplan");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "u");
        assert!(app.store.items(tomorrow).is_empty());
        assert!(matches!(app.mode, Mode::Calendar(_)));
        ctrl(&mut app, 'r');
        assert_eq!(texts_on(&app, tomorrow), ["plan"]);
        // And from the list afterwards too.
        press(&mut app, KeyCode::Esc);
        type_str(&mut app, "u");
        assert!(app.store.items(tomorrow).is_empty());
    }

    #[test]
    fn u_while_adding_in_the_calendar_is_text() {
        let (mut app, _dir) = app_with(&[]);
        type_str(&mut app, "cau");
        assert_eq!(calendar(&app).adding.as_ref().unwrap().text, "u");
    }

    #[test]
    fn help_from_the_calendar_returns_to_it() {
        let (mut app, _dir) = app_with(&[]);
        type_str(&mut app, "cwl?");
        assert!(matches!(app.mode, Mode::Help { .. }));
        press(&mut app, KeyCode::Esc);
        assert_eq!(calendar(&app).zoom, calendar::Zoom::Week);
        assert_eq!(calendar(&app).cursor, today() + chrono::Duration::days(7));
    }

    #[test]
    fn t_cycles_the_selected_items_priority() {
        use crate::store::Priority;
        let (mut app, _dir) = app_with(&["one", "two"]);
        type_str(&mut app, "j");
        let mut seen = Vec::new();
        for _ in 0..4 {
            type_str(&mut app, "t");
            seen.push(app.items()[1].priority);
        }
        assert_eq!(seen, [Some(Priority::High), Some(Priority::Medium), Some(Priority::Low), None]);
        // Only the selected item, and it stays where it is.
        assert_eq!(app.items()[0].priority, None);
        assert_eq!(items(&app), ["one", "two"]);
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn t_on_an_empty_day_does_nothing() {
        let (mut app, _dir) = app_with(&[]);
        type_str(&mut app, "t");
        assert!(app.items().is_empty());
    }

    #[test]
    fn t_while_typing_is_text() {
        let (mut app, _dir) = app_with(&["one"]);
        type_str(&mut app, "at");
        press(&mut app, KeyCode::Enter);
        assert_eq!(items(&app), ["one", "t"]);
        assert!(app.items().iter().all(|item| item.priority.is_none()));
    }

    #[test]
    fn u_undoes_triage() {
        let (mut app, _dir) = app_with(&["one"]);
        type_str(&mut app, "ttu");
        assert_eq!(app.items()[0].priority, Some(crate::store::Priority::High));
    }

    #[test]
    fn s_finds_an_item_on_another_day_and_enter_goes_to_it() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        let later = today() + chrono::Duration::days(9);
        app.store.insert(later, 0, "Pay rent".into()).unwrap();
        app.store.insert(later, 1, "Dentist at 3pm".into()).unwrap();
        type_str(&mut app, "sdntst");
        assert!(matches!(app.mode, Mode::Search(_)));
        press(&mut app, KeyCode::Enter);
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.day, later);
        assert_eq!(app.selected, 1);
        assert_eq!(app.items()[app.selected].text, "Dentist at 3pm");
    }

    #[test]
    fn search_selects_the_right_row_after_carried_and_completed_items() {
        let (mut app, _dir) = app_with_future(&["pin a"], &["open", "finished"]);
        let later = today() + chrono::Duration::days(2);
        app.store.toggle_done(later, 1).unwrap();
        type_str(&mut app, "sfinished");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.day, later);
        // "pin a" is carried in first, then "open", then the completed one.
        assert_eq!(items(&app), ["pin a", "open", "finished"]);
        assert_eq!(app.selected, 2);
    }

    #[test]
    fn capital_s_also_searches_notes() {
        let (mut app, _dir) = app_with(&["one", "two"]);
        app.store.set_notes(today(), 1, "remember the passport".into()).unwrap();
        type_str(&mut app, "spassport");
        press(&mut app, KeyCode::Enter);
        // Plain search doesn't look in notes, so Enter does nothing.
        assert!(matches!(app.mode, Mode::Search(_)));
        press(&mut app, KeyCode::Esc);
        type_str(&mut app, "Spassport");
        press(&mut app, KeyCode::Enter);
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn esc_closes_the_search_where_you_were() {
        let (mut app, _dir) = app_with(&["one", "two"]);
        type_str(&mut app, "jlsone");
        press(&mut app, KeyCode::Esc);
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.day, today().succ_opt().unwrap());
    }

    #[test]
    fn keys_typed_in_search_do_not_act_on_the_list() {
        let (mut app, _dir) = app_with(&["one", "two"]);
        type_str(&mut app, "sxdpjtqu");
        assert!(matches!(app.mode, Mode::Search(_)));
        assert!(app.items().iter().all(|item| !item.done && !item.pinned && item.priority.is_none()));
        assert!(!app.quit);
    }

    #[test]
    fn o_opens_options_and_toggling_saves_the_settings() {
        let (mut app, dir) = app_with(&["one"]);
        let path = dir.path().join("settings.json");
        app.settings_path = Some(path.clone());
        type_str(&mut app, "o");
        assert!(matches!(app.mode, Mode::Options(_)));
        type_str(&mut app, " ");
        assert!(app.settings.semantic_icons);
        type_str(&mut app, "j");
        press(&mut app, KeyCode::Enter);
        assert!(app.settings.no_colour);
        type_str(&mut app, " ");
        assert!(!app.settings.no_colour);
        let saved = Settings::load(Some(&path), false);
        assert_eq!(saved, app.settings);
        press(&mut app, KeyCode::Esc);
        assert!(matches!(app.mode, Mode::Normal));
    }

    #[test]
    fn keys_in_options_do_not_act_on_the_list() {
        let (mut app, _dir) = app_with(&["one", "two"]);
        type_str(&mut app, "oxdpta");
        assert!(matches!(app.mode, Mode::Options(_)));
        assert!(app.items().iter().all(|item| !item.done && !item.pinned && item.priority.is_none()));
        assert_eq!(items(&app), ["one", "two"]);
    }

    #[test]
    fn toggling_options_is_not_an_undoable_change() {
        let (mut app, _dir) = app_with(&["one"]);
        type_str(&mut app, "xo ");
        press(&mut app, KeyCode::Esc);
        type_str(&mut app, "u");
        assert!(app.settings.semantic_icons);
        assert!(!app.items()[0].done);
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
