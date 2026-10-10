use std::cell::Cell;
use std::io;
use std::path::PathBuf;

use chrono::{Days, NaiveDate, NaiveTime};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::calendar::{self, Calendar};
use crate::changelog::{self, ChangelogView};
use crate::help::Help;
use crate::input::LineInput;
use crate::notes::{Action, NotesEditor, Register};
use crate::options::{self, Options, Settings, TOGGLES};
use crate::search::{self, Search};
use crate::tags::{self, TagPicker};
use crate::viewer::{self, Viewer};
use crate::store::{Item, Priority, Snapshot, Store};
use crate::workspaces::{self, Picker, Workspaces};

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
    /// With `repeat` (from `A`), Enter adds the item and starts another below
    /// it, until Esc or Enter on an empty line.
    Insert { index: usize, input: LineInput, editing: bool, repeat: bool },
    /// Asking whether to delete the items at these screen rows.
    ConfirmDelete { rows: Vec<usize> },
    /// Selecting several items, from the row `anchor` to the selected row.
    Visual { anchor: usize },
    /// The notes screen for the selected item.
    Notes(Box<NotesEditor>),
    /// The selected item's notes as formatted Markdown, to read.
    View(Viewer),
    /// The options popup.
    Options(Options),
    /// The `W` popup, to switch, create or delete workspaces.
    Workspaces(Picker),
    /// Release notes: what's new after an update, or all of them.
    Changelog(ChangelogView),
    /// The `#` list of every tag.
    Tags(TagPicker),
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
    /// The time of day, to show deadlines passed today. The event loop sets
    /// it before drawing; tests can set any.
    pub now: NaiveTime,
    pub day: NaiveDate,
    pub selected: usize,
    /// The first list row on screen. The UI updates it while drawing so the
    /// view only scrolls when the cursor reaches its top or bottom edge.
    pub list_offset: Cell<usize>,
    pub mode: Mode,
    pub quit: bool,
    /// Set by resetting todoro: the event loop stops, and todoro starts
    /// again from the first-run screen.
    pub restart: bool,
    pub settings: Settings,
    /// Where to save settings when they change, if anywhere.
    pub settings_path: Option<PathBuf>,
    /// The todoro folder and the open workspace's name, unless todos come
    /// from a single file (`TODORO_FILE`, or tests).
    pub workspaces: Option<Workspaces>,
    pub workspace: Option<String>,
    undo: Vec<State>,
    redo: Vec<State>,
    /// The state when the notes screen was opened. Everything typed there is
    /// one change for the list's undo.
    notes_before: Option<State>,
    /// First key of a two-key command (`gg`, `yy`) waiting for its second key.
    pending: Option<char>,
    /// A count typed before a command, like the 4 in `4j`.
    count: Option<usize>,
    /// The `:` command being typed on the list, if any.
    pub command: Option<LineInput>,
    /// Something to tell the user, shown in the status bar until the next key.
    pub message: Option<String>,
    /// The items last copied (`yy`) or deleted, for `p` and `P` to paste.
    pub register: Vec<Item>,
    /// Text last copied or deleted in any item's notes, kept for the next
    /// notes screen as vim keeps its register.
    notes_register: Option<Register>,
    /// Notes to open in the user's own editor (`Settings::editor`): the item,
    /// once `Enter` asks for it, then also the state before, while it's open.
    external: Option<(Slot, Option<State>)>,
    /// Whether editing was started from the viewer, to go back to it after.
    view_after_edit: bool,
}

impl App {
    pub fn new(store: Store, today: NaiveDate) -> Self {
        Self {
            store,
            today,
            now: NaiveTime::MIN,
            day: today,
            selected: 0,
            list_offset: Cell::new(0),
            mode: Mode::Normal,
            quit: false,
            restart: false,
            settings: Settings::default(),
            settings_path: None,
            workspaces: None,
            workspace: None,
            undo: Vec::new(),
            redo: Vec::new(),
            notes_before: None,
            pending: None,
            count: None,
            command: None,
            message: None,
            register: Vec::new(),
            notes_register: None,
            external: None,
            view_after_edit: false,
        }
    }

    /// Every item shown for the current day, in screen order. Today and future
    /// days also show, first, the pinned items from earlier days that will have
    /// moved to them by then. Usually startup has already moved them to today,
    /// but one can land on a past day mid-session, e.g. moved there with `<`.
    pub fn slots(&self) -> Vec<Slot> {
        let mut slots: Vec<Slot> = if self.day >= self.today {
            self.store.carried_to(self.day).into_iter().map(|(day, index)| Slot { day, index }).collect()
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
        self.message = None;
        let result = self.key(key);
        self.report_conflicts();
        result
    }

    /// Tells the user about notes files that changed outside todoro, which
    /// saving left alone. If those notes are open, they show the outside
    /// version, so the next key can't overwrite it.
    fn report_conflicts(&mut self) {
        let conflicts = self.store.take_conflicts();
        let Some(conflict) = conflicts.first() else { return };
        self.message = Some(format!(
            "{} changed outside todoro, so it was kept. Your version is in {}.",
            conflict.file, conflict.copy
        ));
        let slot = self.slots().get(self.selected).copied();
        if let Mode::Notes(editor) = &mut self.mode
            && let Some(slot) = slot
        {
            let register = editor.register.clone();
            **editor = NotesEditor::new(&self.store.items(slot.day)[slot.index].notes);
            editor.register = register;
        }
    }

    fn key(&mut self, key: KeyEvent) -> io::Result<()> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && key.code == KeyCode::Char('c') {
            self.quit = true;
            return Ok(());
        }
        // Undo works from the list and the calendar, but not while typing.
        let can_undo = match &self.mode {
            Mode::Normal => self.command.is_none(),
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
        let before = matches!(
            self.mode,
            Mode::Normal | Mode::Insert { .. } | Mode::ConfirmDelete { .. } | Mode::Visual { .. } | Mode::Calendar(_)
        )
            .then(|| self.state());
        self.mode_key(key)?;
        if let Some(before) = before {
            self.record(before);
        }
        Ok(())
    }

    fn mode_key(&mut self, key: KeyEvent) -> io::Result<()> {
        match &mut self.mode {
            Mode::Normal if self.command.is_some() => self.command_key(key.code),
            Mode::Normal => self.normal_key(key.code)?,
            Mode::Insert { index, input, editing, repeat } => {
                if input.handle_key(key.code) {
                    let (index, text, editing) = (*index, input.text.trim().to_string(), *editing);
                    let next = *repeat && key.code == KeyCode::Enter && !text.is_empty();
                    self.mode = Mode::Normal;
                    match (editing, text.is_empty()) {
                        (true, true) => self.mode = Mode::ConfirmDelete { rows: vec![index] },
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
                            self.add_item(self.day, index, text)?;
                            self.selected = carried + index;
                            if next {
                                let index = self.selected + 1;
                                self.mode = Mode::Insert { index, input: LineInput::default(), editing: false, repeat: true };
                            }
                        }
                    }
                }
            }
            Mode::ConfirmDelete { rows } => match key.code {
                KeyCode::Char('d') => {
                    let rows = std::mem::take(rows);
                    self.mode = Mode::Normal;
                    // Deleting keeps the items to paste, as in vim, so dd then p moves one.
                    self.register = rows.iter().map(|&row| self.items()[row].clone()).collect();
                    for (day, indices) in self.group_by_day(&rows) {
                        self.store.remove_many(day, &indices)?;
                    }
                    self.selected = rows[0];
                    self.clamp_selection();
                }
                KeyCode::Char('c') | KeyCode::Esc => self.mode = Mode::Normal,
                _ => {}
            },
            Mode::Visual { anchor } => {
                let anchor = *anchor;
                self.visual_key(key.code, anchor)?;
            }
            Mode::Notes(editor) => {
                let action = editor.handle_key(key);
                let register = editor.register.clone();
                self.save_open_notes()?;
                match action {
                    Action::Stay => {}
                    Action::Close => {
                        self.notes_register = register;
                        self.mode = Mode::Normal;
                        if let Some(before) = self.notes_before.take() {
                            self.record(before);
                        }
                        self.back_to_viewer();
                    }
                    Action::Help => self.open_help(),
                }
            }
            Mode::View(viewer) => match viewer.handle_key(key) {
                viewer::Action::Stay => {}
                viewer::Action::Close => self.mode = Mode::Normal,
                viewer::Action::Help => self.open_help(),
                viewer::Action::Edit => {
                    self.mode = Mode::Normal;
                    self.view_after_edit = true;
                    // Recorded for undo like editing from the list.
                    let before = self.state();
                    self.open_notes()?;
                    self.record(before);
                }
            },
            Mode::Calendar(calendar) => match calendar.handle_key(key) {
                calendar::Action::Stay => {}
                calendar::Action::Close => self.mode = Mode::Normal,
                calendar::Action::Open(day) => {
                    self.mode = Mode::Normal;
                    self.show_day(day);
                }
                calendar::Action::Add(day, text) => {
                    let index = self.store.open_count(day);
                    self.add_item(day, index, text)?;
                }
                calendar::Action::SetDeadline { day, index, deadline } => {
                    self.mode = Mode::Normal;
                    self.store.set_deadline(day, index, deadline)?;
                    // Stay on the item, wherever it shows now. A deadline
                    // before the day on screen means it no longer shows here.
                    if let Some(row) = self.slots().iter().position(|slot| slot.day == day && slot.index == index) {
                        self.selected = row;
                    }
                    self.clamp_selection();
                }
                calendar::Action::Help => self.open_help(),
            },
            Mode::Changelog(view) => {
                if view.handle_key(key) {
                    self.mode = Mode::Normal;
                }
            }
            Mode::Workspaces(picker) => {
                let (Some(folder), Some(current)) = (&self.workspaces, &self.workspace) else {
                    self.mode = Mode::Normal;
                    return Ok(());
                };
                let names = folder.list()?;
                match picker.handle_key(key.code, &names, current) {
                    workspaces::Action::Stay => {}
                    workspaces::Action::Close => self.mode = Mode::Normal,
                    workspaces::Action::Open(name) => self.switch_workspace(name)?,
                    workspaces::Action::Create(name) => match folder.create(&name) {
                        Ok(name) => self.switch_workspace(name)?,
                        Err(error) => picker.error = Some(error.to_string()),
                    },
                    workspaces::Action::Delete(name) => {
                        let result = folder.delete(&name);
                        let left = folder.list()?.len();
                        picker.deleting = None;
                        picker.selected = picker.selected.min(left.saturating_sub(1));
                        if let Err(error) = result {
                            picker.error = Some(format!("Couldn't delete {name}: {error}"));
                        }
                    }
                }
            }
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
                options::Action::Clear(clear) => {
                    let result = self.clear(clear);
                    if let Mode::Options(popup) = &mut self.mode {
                        popup.message = Some(match result {
                            Ok(()) => (clear.done(self.workspace.as_deref()), true),
                            Err(error) => (format!("Couldn't finish: {error}"), false),
                        });
                    }
                }
                options::Action::SetEditor(command) => {
                    self.settings.editor = command;
                    if let Some(path) = &self.settings_path {
                        self.settings.save(path)?;
                    }
                }
                options::Action::MoveFolder(to) => {
                    let (Some(folder), Some(current)) = (&self.workspaces, &self.workspace) else { return Ok(()) };
                    match folder.move_to(&to) {
                        Ok(moved) => {
                            // The open todos moved too, so reopen them where they are now.
                            self.store = Store::open(moved.todos_path(current))?;
                            self.settings.data_dir = Some(moved.dir.clone());
                            if let Some(path) = &self.settings_path {
                                self.settings.save(path)?;
                            }
                            let shown = workspaces::display_path(&moved.dir);
                            popup.message = Some((format!("Moved your workspaces to {shown}"), true));
                            popup.folder = Some(shown);
                            popup.editing = None;
                            self.workspaces = Some(moved);
                        }
                        Err(error) => popup.message = Some((format!("Couldn't move them: {error}"), false)),
                    }
                }
            },
            Mode::Tags(picker) => match picker.handle_key(key, &tags::all_tags(&self.store)) {
                tags::Action::Stay => {}
                tags::Action::Close => self.mode = Mode::Normal,
                tags::Action::Open(name) => self.mode = Mode::Search(Box::new(Search::for_tag(name))),
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
        // Digits build a count for the next command, as in vim: 4j moves down
        // four items and 42G goes to item 42. A 0 only counts after another digit.
        if let KeyCode::Char(c @ '0'..='9') = code
            && (c != '0' || self.count.is_some())
        {
            let digit = c as usize - '0' as usize;
            self.count = Some((self.count.unwrap_or(0) * 10 + digit).min(99_999));
            return Ok(());
        }
        let count = self.count.take();
        let times = count.unwrap_or(1);
        // Item N is the Nth row, which is its number for open items.
        let row = |n: usize| n.saturating_sub(1).min(len.saturating_sub(1));
        // Any other key cancels an unfinished two-key command, then does its
        // own thing, as in vim.
        match (self.pending.take(), code) {
            (Some('g'), KeyCode::Char('g')) => {
                self.selected = row(count.unwrap_or(1));
                return Ok(());
            }
            (Some('y'), KeyCode::Char('y')) => {
                if let Some(slot) = slot {
                    self.register = vec![self.store.items(slot.day)[slot.index].clone()];
                }
                return Ok(());
            }
            _ => {}
        }
        match code {
            KeyCode::Char('g' | 'y') => {
                self.pending = Some(code.as_char().expect("a char key"));
                // Keep the count for the second key, as in 7gg.
                self.count = count;
            }
            // Paste below or above the cursor, like a and its opposite.
            KeyCode::Char('p') => self.paste(if self.selected < open && len > 0 { self.selected + 1 } else { open })?,
            KeyCode::Char('P') => self.paste(self.selected.min(open))?,
            KeyCode::Char('G') => self.selected = count.map_or(len.saturating_sub(1), row),
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char(':') => self.command = Some(LineInput::default()),
            KeyCode::Char('?') => self.open_help(),
            KeyCode::Char('o') => {
                let folder = self.workspaces.as_ref().map(|folder| workspaces::display_path(&folder.dir));
                self.mode = Mode::Options(Options::new(self.settings.editor.as_deref(), folder, self.workspace.clone()));
            }
            KeyCode::Char('N') => self.mode = Mode::Changelog(ChangelogView::all()),
            KeyCode::Char('W') => {
                if let (Some(folder), Some(current)) = (&self.workspaces, &self.workspace) {
                    self.mode = Mode::Workspaces(Picker::new(&folder.list()?, current));
                }
            }
            KeyCode::Char('#') => self.mode = Mode::Tags(TagPicker::default()),
            KeyCode::Char('s') => self.mode = Mode::Search(Box::new(Search::new(false))),
            KeyCode::Char('S') => self.mode = Mode::Search(Box::new(Search::new(true))),
            KeyCode::Char('c') => self.mode = Mode::Calendar(Box::new(Calendar::new(self.day, self.today))),
            KeyCode::Char('h') | KeyCode::Left => self.change_day(-1),
            KeyCode::Char('l') | KeyCode::Right => self.change_day(1),
            KeyCode::Char('j') | KeyCode::Down => self.selected = (self.selected + times).min(len.saturating_sub(1)),
            KeyCode::Char('k') | KeyCode::Up => self.selected = self.selected.saturating_sub(times),
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
                self.mode = Mode::Insert { index, input: LineInput::default(), editing: false, repeat: false };
            }
            // Like a, but Enter keeps adding items until Esc or an empty line.
            KeyCode::Char('A') => {
                let index = if self.selected < open { self.selected + 1 } else { open };
                self.mode = Mode::Insert { index, input: LineInput::default(), editing: false, repeat: true };
            }
            KeyCode::Char('e') if len > 0 => {
                let input = LineInput::new(&self.items()[self.selected].text);
                self.mode = Mode::Insert { index: self.selected, input, editing: true, repeat: false };
            }
            KeyCode::Char('d') if len > 0 => self.mode = Mode::ConfirmDelete { rows: vec![self.selected] },
            KeyCode::Char('V') if len > 0 => self.mode = Mode::Visual { anchor: self.selected },
            // The cursor stays put, so you can tick off several items in a row.
            // A carried item is changed where it's stored, which can take it off
            // this day's screen, hence the clamp.
            KeyCode::Char('x') => {
                if let Some(slot) = slot {
                    self.store.toggle_done(slot.day, slot.index)?;
                    self.clamp_selection();
                }
            }
            // Like the calendar's t: back to today, where you were if already there.
            KeyCode::Char('t') if self.day != self.today => self.show_day(self.today),
            // Triage: cycle the item's priority.
            KeyCode::Char('!') => {
                if let Some(slot) = slot {
                    self.store.cycle_priority(slot.day, slot.index)?;
                }
            }
            // Pin: "mark" the item to carry forward. One with a deadline
            // already moves on until then.
            KeyCode::Char('m') => {
                if let Some(slot) = slot {
                    if self.store.items(slot.day)[slot.index].deadline.is_some() {
                        self.message = Some("This has a deadline, so it moves on until then. @ then d removes it.".into());
                    } else {
                        self.store.toggle_pinned(slot.day, slot.index)?;
                        self.clamp_selection();
                    }
                }
            }
            KeyCode::Char('@') => self.open_deadline(),
            KeyCode::Enter if len > 0 => self.open_notes()?,
            KeyCode::Char('v') if len > 0 => {
                // Read the notes file again, in case something else changed it.
                if let Some(slot) = slot {
                    self.store.reload_notes(slot.day, slot.index)?;
                }
                self.mode = Mode::View(Viewer::default());
            }
            _ => {}
        }
        Ok(())
    }

    /// Opens the selected item's notes to edit: in the user's editor if one
    /// is set (which the event loop runs, since it owns the terminal),
    /// otherwise on the notes screen.
    fn open_notes(&mut self) -> io::Result<()> {
        let Some(slot) = self.slot(self.selected) else { return Ok(()) };
        if self.editor().is_some() {
            self.external = Some((slot, None));
            return Ok(());
        }
        // Read the notes file again, in case something else changed it.
        self.store.reload_notes(slot.day, slot.index)?;
        self.notes_before = Some(self.state());
        let mut editor = NotesEditor::new(&self.store.items(slot.day)[slot.index].notes);
        editor.register = self.notes_register.clone();
        self.mode = Mode::Notes(Box::new(editor));
        Ok(())
    }

    /// Deletes what the options asked for, for good. Undo can't bring it
    /// back, so its history goes too.
    fn clear(&mut self, clear: options::Clear) -> io::Result<()> {
        use options::Clear;
        self.undo.clear();
        self.redo.clear();
        self.register.clear();
        match clear {
            Clear::Items => self.store.delete_everything()?,
            Clear::Notes => self.store.delete_all_notes()?,
            Clear::AllNotes | Clear::AllItems => {
                let (Some(folder), Some(current)) = (&self.workspaces, &self.workspace) else { return Ok(()) };
                for name in folder.list()? {
                    let mut other;
                    let store = if &name == current {
                        &mut self.store
                    } else {
                        other = Store::open(folder.todos_path(&name))?;
                        &mut other
                    };
                    if clear == Clear::AllNotes {
                        store.delete_all_notes()?;
                    } else {
                        store.delete_everything()?;
                    }
                }
            }
            Clear::Reset => {
                if let Some(folder) = &self.workspaces {
                    folder.delete_all()?;
                }
                if let Some(path) = &self.settings_path {
                    match std::fs::remove_file(path) {
                        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                        _ => {}
                    }
                }
                self.restart = true;
            }
        }
        self.selected = 0;
        self.clamp_selection();
        Ok(())
    }

    /// Opens the year calendar to choose the selected item's deadline.
    fn open_deadline(&mut self) {
        let Some(slot) = self.slot(self.selected) else { return };
        let item = &self.store.items(slot.day)[slot.index];
        let calendar = Calendar::for_deadline(
            (slot.day, slot.index),
            &item.text,
            item.deadline,
            self.day,
            self.today,
            self.settings.twelve_hour,
        );
        self.mode = Mode::Calendar(Box::new(calendar));
    }

    /// Adds a new item, pinned if the options say new items start pinned.
    fn add_item(&mut self, day: NaiveDate, index: usize, text: String) -> io::Result<()> {
        let item = Item { text, pinned: self.settings.pin_new_items, ..Item::default() };
        self.store.insert_items(day, index, vec![item])?;
        Ok(())
    }

    /// After editing notes started from the viewer, shows them there again.
    fn back_to_viewer(&mut self) {
        if std::mem::take(&mut self.view_after_edit) && matches!(self.mode, Mode::Normal) {
            self.mode = Mode::View(Viewer::default());
        }
    }

    /// Keys while typing a `:` command on the list. `:42` goes to item 42 and
    /// `:q` (or `:wq`, `:x`) quits, as in vim; everything is already saved.
    fn command_key(&mut self, code: KeyCode) {
        let Some(input) = &mut self.command else { return };
        match code {
            KeyCode::Esc => self.command = None,
            // Backspace past the : leaves the command line, as in vim.
            KeyCode::Backspace if input.text.is_empty() => self.command = None,
            KeyCode::Enter => {
                let command = input.text.trim().to_string();
                self.command = None;
                match command.as_str() {
                    "q" | "q!" | "wq" | "wq!" | "x" | "x!" => self.quit = true,
                    "deadline" => self.open_deadline(),
                    line => {
                        if let Ok(n) = line.parse::<usize>() {
                            let len = self.slots().len();
                            self.selected = n.saturating_sub(1).min(len.saturating_sub(1));
                        }
                    }
                }
            }
            code => {
                input.handle_key(code);
            }
        }
    }

    /// The screen rows selected in visual mode, top to bottom.
    pub fn visual_rows(&self, anchor: usize) -> std::ops::RangeInclusive<usize> {
        anchor.min(self.selected)..=anchor.max(self.selected)
    }

    /// The store positions of the items at screen `rows`, grouped by the day
    /// they're stored on (carried items live on earlier days).
    fn group_by_day(&self, rows: &[usize]) -> Vec<(NaiveDate, Vec<usize>)> {
        let slots = self.slots();
        let mut groups: Vec<(NaiveDate, Vec<usize>)> = Vec::new();
        for slot in rows.iter().filter_map(|&row| slots.get(row)) {
            match groups.iter_mut().find(|(day, _)| *day == slot.day) {
                Some((_, indices)) => indices.push(slot.index),
                None => groups.push((slot.day, vec![slot.index])),
            }
        }
        groups
    }

    /// Keys while selecting several items. Each action applies to the whole
    /// selection and returns to the list, as in vim's visual mode.
    fn visual_key(&mut self, code: KeyCode, anchor: usize) -> io::Result<()> {
        let rows: Vec<usize> = self.visual_rows(anchor).collect();
        let len = self.slots().len();
        let first = rows[0];
        let items: Vec<Item> = rows.iter().map(|&row| self.items()[row].clone()).collect();
        let groups = self.group_by_day(&rows);
        match code {
            KeyCode::Esc | KeyCode::Char('V') => {}
            KeyCode::Char('j') | KeyCode::Down => {
                self.selected = (self.selected + 1).min(len - 1);
                return Ok(());
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
                return Ok(());
            }
            KeyCode::Char('G') => {
                self.selected = len - 1;
                return Ok(());
            }
            // Complete them all, or reopen them all if they're all complete.
            KeyCode::Char('x') => {
                let done = items.iter().any(|item| !item.done);
                for (day, indices) in groups {
                    self.store.set_done_many(day, &indices, done)?;
                }
            }
            // Pin them all, or unpin them all if they're all pinned. Ones
            // with a deadline move on until then, so they're left alone.
            KeyCode::Char('m') => {
                let pinned = items.iter().any(|item| !item.pinned && item.deadline.is_none());
                for (day, indices) in groups {
                    self.store.update_many(day, &indices, |item| {
                        if item.deadline.is_none() {
                            item.pinned = pinned;
                        }
                    })?;
                }
            }
            // The next priority after the first item's, for them all.
            KeyCode::Char('!') => {
                let priority = Priority::cycle(items[0].priority);
                for (day, indices) in groups {
                    self.store.update_many(day, &indices, |item| item.priority = priority)?;
                }
            }
            KeyCode::Char('y') => self.register = items,
            KeyCode::Char('d') => {
                self.mode = Mode::ConfirmDelete { rows };
                return Ok(());
            }
            KeyCode::Char('H' | 'L') => {
                let delta = if code == KeyCode::Char('L') { 1 } else { -1 };
                let Some(to) = self.day.checked_add_signed(chrono::Duration::days(delta)) else { return Ok(()) };
                let mut landed = None;
                for (day, indices) in groups {
                    let index = self.store.move_many(day, &indices, to)?;
                    landed = landed.or(index);
                }
                self.mode = Mode::Normal;
                self.change_day(delta);
                self.selected = self.carried() + landed.unwrap_or(0);
                self.clamp_selection();
                return Ok(());
            }
            _ => return Ok(()),
        }
        self.mode = Mode::Normal;
        self.selected = first;
        self.clamp_selection();
        Ok(())
    }

    /// Pastes the register at screen position `index` on the day on screen,
    /// after any carried items, and selects the first pasted item.
    fn paste(&mut self, index: usize) -> io::Result<()> {
        if self.register.is_empty() {
            return Ok(());
        }
        let carried = self.carried();
        let index = self.store.insert_items(self.day, index.saturating_sub(carried), self.register.clone())?;
        self.selected = carried + index;
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

    /// Opens another workspace: its todos, on today, with pinned items
    /// carried over, and a fresh undo history, so `u` never reaches into the
    /// workspace left. It's remembered for next time.
    pub fn switch_workspace(&mut self, name: String) -> io::Result<()> {
        let Some(folder) = &self.workspaces else { return Ok(()) };
        let mut store = Store::open(folder.todos_path(&name))?;
        store.roll_over(self.today)?;
        self.store = store;
        self.mode = Mode::Normal;
        self.show_day(self.today);
        self.undo.clear();
        self.redo.clear();
        self.notes_before = None;
        self.settings.workspace = Some(name.clone());
        self.workspace = Some(name);
        if let Some(path) = &self.settings_path {
            self.settings.save(path)?;
        }
        Ok(())
    }

    /// The command to open notes with instead of the notes screen, if any.
    fn editor(&self) -> Option<&str> {
        self.settings.editor.as_deref().map(str::trim).filter(|command| !command.is_empty())
    }

    /// If `Enter` asked to open notes in the user's editor: the command and
    /// the notes file to open with it. An item without notes gets an empty
    /// file. Call `finish_external_edit` once the editor has closed.
    pub fn start_external_edit(&mut self) -> io::Result<Option<(String, PathBuf)>> {
        // Only a request not started yet; one that's open waits for its finish.
        let Some((slot, None)) = self.external else { return Ok(None) };
        let Some(command) = self.editor().map(String::from) else {
            self.external = None;
            return Ok(None);
        };
        // Read the file first, so undo goes back to what was in it.
        self.store.reload_notes(slot.day, slot.index)?;
        let before = self.state();
        let Some(path) = self.store.notes_path(slot.day, slot.index)? else { return Ok(None) };
        self.external = Some((slot, Some(before)));
        Ok(Some((command, path)))
    }

    /// Takes in what the user's editor saved, as one change to undo, and
    /// says if the editor couldn't run.
    pub fn finish_external_edit(&mut self, result: io::Result<std::process::ExitStatus>) -> io::Result<()> {
        let Some((slot, Some(before))) = self.external.take() else { return Ok(()) };
        let command = self.editor().unwrap_or_default().to_string();
        self.store.notes_edited(slot.day, slot.index)?;
        self.record(before);
        self.back_to_viewer();
        self.message = match result {
            Ok(status) if status.success() => None,
            // What sh (127) and cmd (9009) give for a command they can't find.
            Ok(status) if matches!(status.code(), Some(127 | 9009)) => {
                Some(format!("Couldn't find {command}. Change the editor in the options (o)."))
            }
            Ok(_) => Some(format!("{command} closed with an error")),
            Err(error) => Some(format!("Couldn't open {command}: {error}. Change the editor in the options (o).")),
        };
        self.report_conflicts();
        Ok(())
    }

    /// Handles text pasted into the terminal, which arrives in one piece
    /// (bracketed paste) rather than as keys, so it's never run as commands.
    /// Notes keep its lines; single-line inputs get it on one line; anywhere
    /// else it's ignored.
    pub fn handle_paste(&mut self, text: &str) -> io::Result<()> {
        self.message = None;
        let result = self.paste_text(text);
        self.report_conflicts();
        result
    }

    fn paste_text(&mut self, text: &str) -> io::Result<()> {
        match &mut self.mode {
            Mode::Notes(editor) => {
                editor.paste_text(text);
                self.save_open_notes()?;
            }
            Mode::Insert { input, .. } => input.paste(text),
            Mode::Search(search) => search.paste(text),
            Mode::Calendar(calendar) => {
                let time = calendar.picking.as_mut().and_then(|picking| picking.time.as_mut());
                if let Some(input) = calendar.adding.as_mut().or(time) {
                    input.paste(text);
                }
            }
            Mode::Normal => {
                if let Some(input) = &mut self.command {
                    input.paste(text);
                }
            }
            Mode::Options(popup) => {
                let search = popup.search.as_mut().filter(|_| popup.searching);
                if let Some(input) = popup.editing.as_mut().or(search) {
                    input.paste(text);
                }
            }
            Mode::Workspaces(picker) => {
                if let Some(input) = picker.adding.as_mut().or(picker.deleting.as_mut().map(|(_, input)| input)) {
                    input.paste(text);
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Saves the open notes screen's text to its item if it has changed, so
    /// notes save as you type like the rest of the app.
    fn save_open_notes(&mut self) -> io::Result<()> {
        let Mode::Notes(editor) = &self.mode else { return Ok(()) };
        let notes = editor.notes();
        if let Some(slot) = self.slot(self.selected)
            && notes != self.store.items(slot.day)[slot.index].notes
        {
            self.store.set_notes(slot.day, slot.index, notes)?;
        }
        Ok(())
    }

    /// On startup, shows what's new if todoro was updated since it last ran,
    /// and records this version as seen so it only shows once.
    pub fn show_whats_new(&mut self) -> io::Result<()> {
        let has_todos = self.store.all().next().is_some();
        if let Some(view) = changelog::on_start(self.settings.last_seen_version.as_deref(), has_todos) {
            self.mode = Mode::Changelog(view);
        }
        if self.settings.last_seen_version.as_deref() != Some(changelog::VERSION) {
            self.settings.last_seen_version = Some(changelog::VERSION.to_string());
            if let Some(path) = &self.settings_path {
                self.settings.save(path)?;
            }
        }
        Ok(())
    }

    /// When something on screen (a copy's flash) needs redrawing, if ever.
    pub fn redraw_at(&self) -> Option<std::time::Instant> {
        match &self.mode {
            Mode::Notes(editor) => editor.flash_ends(),
            Mode::Help { back, .. } => match back.as_ref() {
                Mode::Notes(editor) => editor.flash_ends(),
                _ => None,
            },
            _ => None,
        }
    }

    /// Ends anything timed (a copy's flash) whose time is up at `now`.
    pub fn tick(&mut self, now: std::time::Instant) {
        if let Mode::Notes(editor) = &mut self.mode {
            editor.expire_flash(now);
        }
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

    /// Moves "today" on when the date changes while todoro is open, carrying
    /// pinned items over as at startup. It waits until you're back on the
    /// list, so nothing moves while you're typing or in a popup. Undo history
    /// is cleared, so `u` can't undo the carry-over by accident.
    pub fn set_today(&mut self, today: NaiveDate) -> io::Result<()> {
        if today == self.today || !matches!(self.mode, Mode::Normal) {
            return Ok(());
        }
        self.store.roll_over(today)?;
        if self.day == self.today {
            self.show_day(today);
        }
        self.today = today;
        self.undo.clear();
        self.redo.clear();
        self.pending = None;
        Ok(())
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
    use crate::test_util::{app_with, app_with_workspaces, press, today, type_str};

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
        assert!(matches!(app.mode, Mode::ConfirmDelete { .. }));
        type_str(&mut app, "c");
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(items(&app), ["one"]);
    }

    #[test]
    fn other_keys_do_not_dismiss_the_delete_popup() {
        let (mut app, _dir) = app_with(&["one"]);
        type_str(&mut app, "djkx");
        assert!(matches!(app.mode, Mode::ConfirmDelete { .. }));
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
        assert!(matches!(app.mode, Mode::ConfirmDelete { .. }));
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
    fn m_pins_and_unpins_the_selected_item_in_place() {
        let (mut app, _dir) = app_with(&["one", "two"]);
        type_str(&mut app, "jm");
        assert_eq!(items(&app), ["one", "two"]);
        assert!(!app.items()[0].pinned);
        assert!(app.items()[1].pinned);
        assert_eq!(app.selected, 1);
        type_str(&mut app, "m");
        assert!(!app.items()[1].pinned);
    }

    #[test]
    fn m_on_an_empty_day_does_nothing() {
        let (mut app, _dir) = app_with(&[]);
        type_str(&mut app, "m");
        assert!(app.items().is_empty());
    }

    #[test]
    fn m_while_typing_is_text() {
        let (mut app, _dir) = app_with(&["one"]);
        type_str(&mut app, "e m");
        press(&mut app, KeyCode::Enter);
        assert_eq!(items(&app), ["one m"]);
        assert!(!app.items()[0].pinned);
    }

    #[test]
    fn completing_a_pinned_item_keeps_it_pinned() {
        let (mut app, _dir) = app_with(&["one"]);
        type_str(&mut app, "mx");
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
    fn m_on_a_carried_item_unpins_it_back_to_its_day() {
        let (mut app, _dir) = app_with_future(&["pin a"], &["later"]);
        type_str(&mut app, "llm");
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
        type_str(&mut app, "mK");
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
        type_str(&mut app, "m");
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
            type_str(&mut app, "m");
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
        type_str(&mut app, "hml");
        assert_eq!(items(&app), ["old", "plain"]);
        // Unpinning it from today sends it back to its own day only.
        type_str(&mut app, "m");
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
    fn exclamation_mark_cycles_the_selected_items_priority() {
        use crate::store::Priority;
        let (mut app, _dir) = app_with(&["one", "two"]);
        type_str(&mut app, "j");
        let mut seen = Vec::new();
        for _ in 0..4 {
            type_str(&mut app, "!");
            seen.push(app.items()[1].priority);
        }
        assert_eq!(seen, [Some(Priority::High), Some(Priority::Medium), Some(Priority::Low), None]);
        // Only the selected item, and it stays where it is.
        assert_eq!(app.items()[0].priority, None);
        assert_eq!(items(&app), ["one", "two"]);
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn exclamation_mark_on_an_empty_day_does_nothing() {
        let (mut app, _dir) = app_with(&[]);
        type_str(&mut app, "!");
        assert!(app.items().is_empty());
    }

    #[test]
    fn exclamation_mark_and_t_while_typing_are_text() {
        let (mut app, _dir) = app_with(&["one"]);
        type_str(&mut app, "lat!");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.day, today().succ_opt().unwrap());
        assert_eq!(items(&app), ["t!"]);
        assert!(app.items().iter().all(|item| item.priority.is_none()));
    }

    #[test]
    fn t_goes_back_to_today() {
        let (mut app, _dir) = app_with(&["one", "two"]);
        type_str(&mut app, "hhhhjt");
        assert_eq!(app.day, today());
        assert_eq!(app.selected, 0);
        type_str(&mut app, "lllllllllt");
        assert_eq!(app.day, today());
    }

    #[test]
    fn t_on_today_keeps_the_selection() {
        let (mut app, _dir) = app_with(&["one", "two"]);
        type_str(&mut app, "jt");
        assert_eq!(app.day, today());
        assert_eq!(app.selected, 1);
        assert!(app.items().iter().all(|item| item.priority.is_none()));
    }

    #[test]
    fn u_undoes_triage() {
        let (mut app, _dir) = app_with(&["one"]);
        type_str(&mut app, "!!u");
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
    fn gg_and_capital_g_jump_to_the_first_and_last_item() {
        let (mut app, _dir) = app_with(&["one", "two", "three", "four"]);
        type_str(&mut app, "x");
        type_str(&mut app, "G");
        // The last row, below the completed header.
        assert_eq!(app.selected, 3);
        assert_eq!(app.items()[3].text, "one");
        type_str(&mut app, "gg");
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn a_single_g_then_another_key_does_that_key() {
        let (mut app, _dir) = app_with(&["one", "two", "three"]);
        type_str(&mut app, "Ggk");
        assert_eq!(app.selected, 1);
        // The pending g was cancelled, so one more g doesn't jump.
        type_str(&mut app, "g");
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn g_and_capital_g_on_an_empty_day_do_nothing() {
        let (mut app, _dir) = app_with(&[]);
        type_str(&mut app, "Ggg");
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn at_midnight_the_list_moves_on_to_the_new_today() {
        let (mut app, _dir) = app_with(&["pinned", "plain"]);
        app.store.toggle_pinned(today(), 0).unwrap();
        let tomorrow = today().succ_opt().unwrap();
        type_str(&mut app, "j");
        app.set_today(tomorrow).unwrap();
        assert_eq!(app.today, tomorrow);
        assert_eq!(app.day, tomorrow);
        assert_eq!(app.selected, 0);
        // The pinned item was carried over; the other stayed behind.
        assert_eq!(texts_on(&app, tomorrow), ["pinned"]);
        assert_eq!(texts_on(&app, today()), ["plain"]);
    }

    #[test]
    fn at_midnight_another_day_on_screen_stays_put() {
        let (mut app, _dir) = app_with(&["one"]);
        type_str(&mut app, "lll");
        let shown = app.day;
        app.set_today(today().succ_opt().unwrap()).unwrap();
        assert_eq!(app.day, shown);
    }

    #[test]
    fn midnight_waits_until_you_are_back_on_the_list() {
        let (mut app, _dir) = app_with(&["pinned"]);
        app.store.toggle_pinned(today(), 0).unwrap();
        let tomorrow = today().succ_opt().unwrap();
        type_str(&mut app, "e!");
        app.set_today(tomorrow).unwrap();
        assert_eq!(app.today, today());
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.items()[0].text, "pinned!");
        app.set_today(tomorrow).unwrap();
        assert_eq!(app.today, tomorrow);
        assert_eq!(texts_on(&app, tomorrow), ["pinned!"]);
    }

    #[test]
    fn midnight_clears_undo_so_the_carry_over_stays() {
        let (mut app, _dir) = app_with(&["pinned"]);
        type_str(&mut app, "m");
        let tomorrow = today().succ_opt().unwrap();
        app.set_today(tomorrow).unwrap();
        type_str(&mut app, "u");
        assert_eq!(texts_on(&app, tomorrow), ["pinned"]);
        assert!(app.store.items(tomorrow)[0].pinned);
    }

    #[test]
    fn the_same_date_changes_nothing() {
        let (mut app, _dir) = app_with(&["one"]);
        type_str(&mut app, "xl");
        app.set_today(today()).unwrap();
        assert_eq!(app.day, today().succ_opt().unwrap());
        type_str(&mut app, "hu");
        assert!(!app.items()[0].done);
    }

    #[test]
    fn yy_then_p_pastes_a_copy_below() {
        let (mut app, _dir) = app_with(&["one", "two"]);
        app.store.set_notes(today(), 0, "notes".into()).unwrap();
        type_str(&mut app, "!myyjp");
        assert_eq!(items(&app), ["one", "two", "one"]);
        assert_eq!(app.selected, 2);
        let copy = &app.items()[2];
        assert_eq!((copy.notes.as_str(), copy.pinned, copy.priority), ("notes", true, Some(crate::store::Priority::High)));
        // The original is untouched.
        assert_eq!(app.items()[0].notes, "notes");
    }

    #[test]
    fn capital_p_pastes_above() {
        let (mut app, _dir) = app_with(&["one", "two"]);
        type_str(&mut app, "jyyP");
        assert_eq!(items(&app), ["one", "two", "two"]);
        assert_eq!(app.selected, 1);
        type_str(&mut app, "ggP");
        assert_eq!(items(&app), ["two", "one", "two", "two"]);
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn paste_works_on_another_day() {
        let (mut app, _dir) = app_with(&["one"]);
        type_str(&mut app, "yylp");
        assert_eq!(items(&app), ["one"]);
        assert_eq!(texts_on(&app, today()), ["one"]);
        // And again, for as many copies as you like.
        type_str(&mut app, "p");
        assert_eq!(items(&app), ["one", "one"]);
    }

    #[test]
    fn deleting_keeps_the_item_to_paste_so_dd_p_moves_it() {
        let (mut app, _dir) = app_with(&["one", "two"]);
        app.store.set_notes(today(), 0, "notes".into()).unwrap();
        type_str(&mut app, "ddlp");
        assert_eq!(texts_on(&app, today()), ["two"]);
        assert_eq!(items(&app), ["one"]);
        assert_eq!(app.items()[0].notes, "notes");
    }

    #[test]
    fn pasting_a_completed_item_adds_it_as_open() {
        let (mut app, _dir) = app_with(&["one", "two"]);
        type_str(&mut app, "xjyyp");
        // The copy goes to the open items, not after the completed one.
        assert_eq!(items(&app), ["two", "one", "one"]);
        assert!(!app.items()[1].done);
        assert!(app.items()[2].done);
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn paste_lands_after_carried_items_on_a_future_day() {
        let (mut app, _dir) = app_with_future(&["pin a", "plain"], &[]);
        type_str(&mut app, "jyylP");
        assert_eq!(items(&app), ["pin a", "plain"]);
        assert_eq!(app.carried(), 1);
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn nothing_to_paste_does_nothing() {
        let (mut app, _dir) = app_with(&["one"]);
        type_str(&mut app, "pP");
        assert_eq!(items(&app), ["one"]);
    }

    #[test]
    fn a_single_y_is_cancelled_by_the_next_key() {
        let (mut app, _dir) = app_with(&["one", "two"]);
        type_str(&mut app, "yjyp");
        // y j moved down; y p pasted nothing because nothing was copied.
        assert_eq!(items(&app), ["one", "two"]);
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn u_undoes_a_paste() {
        let (mut app, _dir) = app_with(&["one"]);
        type_str(&mut app, "yypu");
        assert_eq!(items(&app), ["one"]);
    }

    fn visual(app: &App) -> Vec<usize> {
        let Mode::Visual { anchor } = app.mode else { panic!("should be selecting") };
        app.visual_rows(anchor).collect()
    }

    #[test]
    fn capital_v_selects_and_j_and_k_extend_either_way() {
        let (mut app, _dir) = app_with(&["one", "two", "three", "four"]);
        type_str(&mut app, "jV");
        assert_eq!(visual(&app), [1]);
        type_str(&mut app, "jj");
        assert_eq!(visual(&app), [1, 2, 3]);
        type_str(&mut app, "kkk");
        assert_eq!(visual(&app), [0, 1]);
        type_str(&mut app, "kG");
        assert_eq!(visual(&app), [1, 2, 3]);
        press(&mut app, KeyCode::Esc);
        assert!(matches!(app.mode, Mode::Normal));
        type_str(&mut app, "VV");
        assert!(matches!(app.mode, Mode::Normal));
    }

    #[test]
    fn x_completes_the_selection_or_reopens_it_if_all_complete() {
        let (mut app, _dir) = app_with(&["one", "two", "three"]);
        type_str(&mut app, "jVjx");
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(items(&app), ["one", "two", "three"]);
        assert_eq!(app.items().iter().map(|item| item.done).collect::<Vec<_>>(), [false, true, true]);
        // A mix of open and complete: all complete.
        type_str(&mut app, "ggVjjx");
        assert!(app.items().iter().all(|item| item.done));
        // All complete: all reopened.
        type_str(&mut app, "ggVGx");
        assert!(app.items().iter().all(|item| !item.done));
    }

    #[test]
    fn m_pins_the_selection_or_unpins_it_if_all_pinned() {
        let (mut app, _dir) = app_with(&["one", "two", "three"]);
        type_str(&mut app, "mVjm");
        assert!(app.items()[..2].iter().all(|item| item.pinned));
        assert!(!app.items()[2].pinned);
        type_str(&mut app, "ggVjm");
        assert!(app.items().iter().all(|item| !item.pinned));
    }

    #[test]
    fn exclamation_mark_gives_the_selection_the_next_priority() {
        use crate::store::Priority;
        let (mut app, _dir) = app_with(&["one", "two", "three"]);
        type_str(&mut app, "!!jj!Vkk!");
        // The cursor ended on "one" (Medium), so all become Low.
        let priorities: Vec<_> = app.items().iter().map(|item| item.priority).collect();
        assert_eq!(priorities, [Some(Priority::Low); 3]);
    }

    #[test]
    fn d_asks_once_then_deletes_the_selection_and_keeps_it_to_paste() {
        let (mut app, _dir) = app_with(&["one", "two", "three", "four"]);
        type_str(&mut app, "jVjd");
        let Mode::ConfirmDelete { rows } = &app.mode else { panic!("should be confirming") };
        assert_eq!(rows, &[1, 2]);
        type_str(&mut app, "d");
        assert_eq!(items(&app), ["one", "four"]);
        assert_eq!(app.selected, 1);
        type_str(&mut app, "lp");
        assert_eq!(items(&app), ["two", "three"]);
    }

    #[test]
    fn cancelling_a_selected_delete_keeps_everything() {
        let (mut app, _dir) = app_with(&["one", "two"]);
        type_str(&mut app, "VGdc");
        assert_eq!(items(&app), ["one", "two"]);
    }

    #[test]
    fn y_copies_the_selection_for_pasting() {
        let (mut app, _dir) = app_with(&["one", "two", "three"]);
        type_str(&mut app, "Vjy");
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.selected, 0);
        type_str(&mut app, "Gp");
        assert_eq!(items(&app), ["one", "two", "three", "one", "two"]);
    }

    #[test]
    fn capital_l_moves_the_selection_and_follows_it() {
        let (mut app, _dir) = app_with(&["one", "two", "three"]);
        let tomorrow = today().succ_opt().unwrap();
        app.store.insert(tomorrow, 0, "t".into()).unwrap();
        type_str(&mut app, "jVjL");
        assert_eq!(app.day, tomorrow);
        assert_eq!(items(&app), ["t", "two", "three"]);
        assert_eq!(app.selected, 1);
        assert_eq!(texts_on(&app, today()), ["one"]);
        type_str(&mut app, "VjH");
        assert_eq!(app.day, today());
        assert_eq!(items(&app), ["one", "two", "three"]);
    }

    #[test]
    fn a_selection_can_include_carried_items() {
        let (mut app, _dir) = app_with_future(&["pin a", "plain"], &["later"]);
        type_str(&mut app, "llVjx");
        // The carried item is completed where it's stored, today.
        assert!(app.store.items(today()).iter().any(|item| item.text == "pin a" && item.done));
        assert_eq!(items(&app), ["later"]);
        assert!(app.items()[0].done);
    }

    #[test]
    fn u_undoes_a_whole_selection_change_in_one_step() {
        let (mut app, _dir) = app_with(&["one", "two", "three"]);
        type_str(&mut app, "VGxu");
        assert!(app.items().iter().all(|item| !item.done));
        // Undo put the cursor back where it was, on the last row.
        type_str(&mut app, "ggVGdd");
        assert!(app.items().is_empty());
        type_str(&mut app, "u");
        assert_eq!(items(&app), ["one", "two", "three"]);
    }

    #[test]
    fn other_keys_while_selecting_do_nothing() {
        let (mut app, _dir) = app_with(&["one", "two"]);
        type_str(&mut app, "Vaetqc");
        assert_eq!(visual(&app), [0]);
        assert_eq!(items(&app), ["one", "two"]);
    }

    #[test]
    fn capital_v_on_an_empty_day_does_nothing() {
        let (mut app, _dir) = app_with(&[]);
        type_str(&mut app, "V");
        assert!(matches!(app.mode, Mode::Normal));
    }

    #[test]
    fn hash_lists_tags_and_enter_shows_items_with_one() {
        let (mut app, _dir) = app_with(&["Call #work about the #budget", "Water the plants #home"]);
        let later = today() + chrono::Duration::days(5);
        app.store.insert(later, 0, "Send #Work report".into()).unwrap();
        type_str(&mut app, "#");
        assert!(matches!(app.mode, Mode::Tags(_)));
        // Most used first: work (2), then budget and home (1 each).
        press(&mut app, KeyCode::Enter);
        let Mode::Search(search) = &app.mode else { panic!("should be showing the tag's items") };
        assert_eq!(search.tag.as_deref(), Some("work"));
        assert_eq!(search.find(&app.store, today()).len(), 2);
        // Enter goes to the selected item, the closest to today.
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.day, today());
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn a_tags_items_can_be_narrowed_by_typing() {
        let (mut app, _dir) = app_with(&["Call #work", "Email #work"]);
        type_str(&mut app, "#");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "email");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn esc_or_hash_closes_the_tag_list() {
        let (mut app, _dir) = app_with(&["#a"]);
        type_str(&mut app, "#");
        press(&mut app, KeyCode::Esc);
        assert!(matches!(app.mode, Mode::Normal));
        type_str(&mut app, "##");
        assert!(matches!(app.mode, Mode::Normal));
    }

    #[test]
    fn hash_while_typing_is_text() {
        let (mut app, _dir) = app_with(&[]);
        type_str(&mut app, "aBuy milk #shop");
        press(&mut app, KeyCode::Enter);
        assert_eq!(items(&app), ["Buy milk #shop"]);
        assert!(matches!(app.mode, Mode::Normal));
    }

    #[test]
    fn capital_a_keeps_adding_items_until_esc() {
        let (mut app, _dir) = app_with(&["one", "four"]);
        type_str(&mut app, "Atwo");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "three");
        press(&mut app, KeyCode::Enter);
        // A fresh line is waiting below the last item added.
        assert!(matches!(app.mode, Mode::Insert { index: 3, editing: false, repeat: true, .. }));
        assert_eq!(items(&app), ["one", "two", "three", "four"]);
        press(&mut app, KeyCode::Esc);
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(items(&app), ["one", "two", "three", "four"]);
        assert_eq!(app.selected, 2);
    }

    #[test]
    fn esc_saves_what_was_typed_on_the_last_line() {
        let (mut app, _dir) = app_with(&[]);
        type_str(&mut app, "Aone");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "two");
        press(&mut app, KeyCode::Esc);
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(items(&app), ["one", "two"]);
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn enter_on_an_empty_line_also_stops_adding() {
        let (mut app, _dir) = app_with(&["first"]);
        type_str(&mut app, "Aone");
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter);
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(items(&app), ["first", "one"]);
    }

    #[test]
    fn capital_a_from_a_completed_item_adds_after_the_open_ones() {
        let (mut app, _dir) = app_with(&["one", "done"]);
        type_str(&mut app, "jxjAtwo");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "three");
        press(&mut app, KeyCode::Esc);
        assert_eq!(items(&app), ["one", "two", "three", "done"]);
    }

    #[test]
    fn each_item_added_with_capital_a_is_its_own_undo_step() {
        let (mut app, _dir) = app_with(&[]);
        type_str(&mut app, "Aone");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "two");
        press(&mut app, KeyCode::Esc);
        type_str(&mut app, "u");
        assert_eq!(items(&app), ["one"]);
        type_str(&mut app, "u");
        assert!(app.items().is_empty());
    }

    #[test]
    fn a_still_adds_just_one_item() {
        let (mut app, _dir) = app_with(&[]);
        type_str(&mut app, "aone");
        press(&mut app, KeyCode::Enter);
        assert!(matches!(app.mode, Mode::Normal));
    }

    fn numbered(n: usize) -> Vec<String> {
        (1..=n).map(|i| format!("item {i}")).collect()
    }

    #[test]
    fn a_count_moves_several_items_with_j_and_k() {
        let names = numbered(20);
        let (mut app, _dir) = app_with(&names.iter().map(String::as_str).collect::<Vec<_>>());
        type_str(&mut app, "4j");
        assert_eq!(app.selected, 4);
        type_str(&mut app, "12j");
        assert_eq!(app.selected, 16);
        type_str(&mut app, "3k");
        assert_eq!(app.selected, 13);
        // Counts stop at the ends of the list.
        type_str(&mut app, "99j");
        assert_eq!(app.selected, 19);
        type_str(&mut app, "99k");
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn a_count_before_capital_g_or_gg_goes_to_that_item() {
        let names = numbered(50);
        let (mut app, _dir) = app_with(&names.iter().map(String::as_str).collect::<Vec<_>>());
        type_str(&mut app, "42G");
        assert_eq!(app.items()[app.selected].text, "item 42");
        type_str(&mut app, "7gg");
        assert_eq!(app.items()[app.selected].text, "item 7");
        type_str(&mut app, "500G");
        assert_eq!(app.selected, 49);
        type_str(&mut app, "G");
        assert_eq!(app.selected, 49);
    }

    #[test]
    fn a_count_is_used_once_and_zero_alone_is_not_a_count() {
        let names = numbered(10);
        let (mut app, _dir) = app_with(&names.iter().map(String::as_str).collect::<Vec<_>>());
        type_str(&mut app, "3jj");
        assert_eq!(app.selected, 4);
        type_str(&mut app, "0j");
        assert_eq!(app.selected, 5);
        type_str(&mut app, "10k");
        assert_eq!(app.selected, 0);
        // A count before another key is dropped.
        type_str(&mut app, "5xj");
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn digits_while_typing_are_text() {
        let (mut app, _dir) = app_with(&[]);
        type_str(&mut app, "a42j");
        press(&mut app, KeyCode::Enter);
        assert_eq!(items(&app), ["42j"]);
    }

    #[test]
    fn colon_q_in_the_notes_goes_back_to_the_list_with_them_saved() {
        let (mut app, _dir) = app_with(&["one"]);
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "inote");
        press(&mut app, KeyCode::Esc);
        type_str(&mut app, ":wq");
        press(&mut app, KeyCode::Enter);
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.items()[0].notes, "note");
    }

    #[test]
    fn lines_deleted_in_one_items_notes_can_be_pasted_in_anothers() {
        let (mut app, _dir) = app_with(&["one", "two"]);
        app.store.set_notes(today(), 0, "keep\nmove me".into()).unwrap();
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "jddq");
        type_str(&mut app, "j");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "Pq");
        assert_eq!(app.items()[0].notes, "keep");
        assert_eq!(app.items()[1].notes, "move me");
    }

    #[test]
    fn colon_and_a_number_goes_to_that_item() {
        let names = numbered(30);
        let (mut app, _dir) = app_with(&names.iter().map(String::as_str).collect::<Vec<_>>());
        type_str(&mut app, ":2");
        assert_eq!(app.command.as_ref().unwrap().text, "2");
        press(&mut app, KeyCode::Enter);
        assert!(app.command.is_none());
        assert_eq!(app.items()[app.selected].text, "item 2");
        type_str(&mut app, ":25");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.items()[app.selected].text, "item 25");
        type_str(&mut app, ":999");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.selected, 29);
    }

    #[test]
    fn colon_q_quits() {
        for command in ["q", "wq", "x"] {
            let (mut app, _dir) = app_with(&["one"]);
            type_str(&mut app, &format!(":{command}"));
            press(&mut app, KeyCode::Enter);
            assert!(app.quit, "{command}");
        }
    }

    #[test]
    fn keys_typed_after_a_colon_are_part_of_the_command() {
        let (mut app, _dir) = app_with(&["one", "two"]);
        type_str(&mut app, ":xdjmu");
        assert_eq!(app.command.as_ref().unwrap().text, "xdjmu");
        assert_eq!(app.selected, 0);
        assert!(app.items().iter().all(|item| !item.done && !item.pinned));
        // An unknown command does nothing.
        press(&mut app, KeyCode::Enter);
        assert!(app.command.is_none());
        assert_eq!(items(&app), ["one", "two"]);
        assert!(!app.quit);
    }

    #[test]
    fn esc_or_backspacing_past_the_colon_cancels_the_command() {
        let (mut app, _dir) = app_with(&["one", "two"]);
        type_str(&mut app, ":2");
        press(&mut app, KeyCode::Esc);
        assert!(app.command.is_none());
        assert_eq!(app.selected, 0);
        type_str(&mut app, ":2");
        press(&mut app, KeyCode::Backspace);
        press(&mut app, KeyCode::Backspace);
        assert!(app.command.is_none());
        type_str(&mut app, "j");
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn whats_new_shows_after_an_update_once() {
        let (mut app, dir) = app_with(&["one"]);
        let path = dir.path().join("settings.json");
        app.settings_path = Some(path.clone());
        app.settings.last_seen_version = Some("0.1.0".into());
        app.show_whats_new().unwrap();
        let Mode::Changelog(view) = &app.mode else { panic!("should show what's new") };
        assert_eq!(view.since, Some((0, 1, 0)));
        // This version is now recorded, so next time there's nothing.
        assert_eq!(Settings::load(Some(&path), false).last_seen_version.as_deref(), Some(changelog::VERSION));
        press(&mut app, KeyCode::Esc);
        assert!(matches!(app.mode, Mode::Normal));
        app.show_whats_new().unwrap();
        assert!(matches!(app.mode, Mode::Normal));
    }

    #[test]
    fn a_new_user_sees_the_newest_releases_once() {
        let (mut app, _dir) = app_with(&[]);
        app.show_whats_new().unwrap();
        let Mode::Changelog(view) = &app.mode else { panic!("the release notes") };
        assert_eq!(view.limit, Some(changelog::FIRST_START_RELEASES));
        assert_eq!(app.settings.last_seen_version.as_deref(), Some(changelog::VERSION));
        // Not again next time.
        app.mode = Mode::Normal;
        app.show_whats_new().unwrap();
        assert!(matches!(app.mode, Mode::Normal));
    }

    #[test]
    fn someone_updating_from_before_this_existed_sees_the_latest_notes() {
        let (mut app, _dir) = app_with(&["one"]);
        app.show_whats_new().unwrap();
        let Mode::Changelog(view) = &app.mode else { panic!("should show what's new") };
        assert_eq!(view.releases().len(), 1);
    }

    #[test]
    fn capital_n_shows_every_release() {
        let (mut app, _dir) = app_with(&["one"]);
        type_str(&mut app, "N");
        let Mode::Changelog(view) = &app.mode else { panic!("should show the changelog") };
        assert!(view.since.is_none());
        type_str(&mut app, "jkN");
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(items(&app), ["one"]);
    }

    #[test]
    fn pasting_into_notes_keeps_the_lines_and_saves() {
        let (mut app, dir) = app_with(&["one"]);
        press(&mut app, KeyCode::Enter);
        app.handle_paste("first line\nsecond line\nthird line").unwrap();
        let reloaded = Store::open(dir.path().join("todos.json")).unwrap();
        assert_eq!(reloaded.items(today())[0].notes, "first line\nsecond line\nthird line");
        // Leaving the notes keeps it, and it's one step for the list's undo.
        type_str(&mut app, "q");
        type_str(&mut app, "u");
        assert_eq!(app.items()[0].notes, "");
    }

    #[test]
    fn pasting_on_the_list_does_nothing() {
        let (mut app, _dir) = app_with(&["one", "two"]);
        app.handle_paste("dd\nx\nq\nj").unwrap();
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(items(&app), ["one", "two"]);
        assert!(app.items().iter().all(|item| !item.done));
        assert!(!app.quit);
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn pasting_while_adding_an_item_keeps_it_on_one_line() {
        let (mut app, _dir) = app_with(&[]);
        type_str(&mut app, "a");
        app.handle_paste("first line\nsecond line").unwrap();
        press(&mut app, KeyCode::Enter);
        assert_eq!(items(&app), ["first line second line"]);
    }

    #[test]
    fn pasting_into_search_and_the_command_line() {
        let (mut app, _dir) = app_with(&["alpha", "beta", "gamma"]);
        type_str(&mut app, "s");
        app.handle_paste("gam\n").unwrap();
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.selected, 2);
        type_str(&mut app, ":");
        app.handle_paste("2").unwrap();
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn capital_w_switches_workspace_and_keeps_them_separate() {
        let (mut app, _dir) = crate::test_util::app_with_workspaces(&["Home", "Work"]);
        assert_eq!(items(&app), ["Home item"]);
        type_str(&mut app, "W");
        let Mode::Workspaces(picker) = &app.mode else { panic!("the workspaces popup") };
        assert_eq!(picker.selected, 0);
        type_str(&mut app, "j");
        press(&mut app, KeyCode::Enter);
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.workspace.as_deref(), Some("Work"));
        assert_eq!(items(&app), ["Work item"]);
        // Changes stay in their own workspace.
        type_str(&mut app, "anew");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "W");
        type_str(&mut app, "k");
        press(&mut app, KeyCode::Enter);
        assert_eq!(items(&app), ["Home item"]);
        assert_eq!(app.settings.workspace.as_deref(), Some("Home"));
    }

    #[test]
    fn undo_never_reaches_into_another_workspace() {
        let (mut app, _dir) = crate::test_util::app_with_workspaces(&["Home", "Work"]);
        type_str(&mut app, "x");
        type_str(&mut app, "Wj");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "u");
        assert_eq!(items(&app), ["Work item"]);
        assert!(!app.items()[0].done);
    }

    #[test]
    fn a_in_the_workspaces_popup_creates_one_and_opens_it() {
        let (mut app, dir) = crate::test_util::app_with_workspaces(&["Home"]);
        type_str(&mut app, "Wa");
        type_str(&mut app, "Side project");
        press(&mut app, KeyCode::Enter);
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.workspace.as_deref(), Some("Side project"));
        assert!(app.items().is_empty());
        assert!(dir.path().join("todoro").join("Side project").join("todos.json").exists());
        // A name that's taken stays in the popup with a message.
        type_str(&mut app, "Wahome");
        press(&mut app, KeyCode::Enter);
        let Mode::Workspaces(picker) = &app.mode else { panic!("still in the popup") };
        assert!(picker.error.as_ref().unwrap().contains("already"));
    }

    #[test]
    fn d_deletes_another_workspace_after_typing_its_name() {
        let (mut app, dir) = crate::test_util::app_with_workspaces(&["Home", "Work"]);
        type_str(&mut app, "Wjd");
        type_str(&mut app, "Work");
        press(&mut app, KeyCode::Enter);
        let Mode::Workspaces(picker) = &app.mode else { panic!("still in the popup") };
        assert!(picker.deleting.is_none());
        assert_eq!(picker.selected, 0);
        assert!(!dir.path().join("todoro").join("Work").exists());
        assert_eq!(app.workspaces.as_ref().unwrap().list().unwrap(), ["Home"]);
        assert_eq!(items(&app), ["Home item"]);
    }

    #[test]
    fn capital_w_does_nothing_with_a_single_todos_file() {
        let (mut app, _dir) = app_with(&["one"]);
        type_str(&mut app, "W");
        assert!(matches!(app.mode, Mode::Normal));
    }

    #[test]
    fn moving_the_folder_from_the_options_moves_every_workspace() {
        let (mut app, dir) = crate::test_util::app_with_workspaces(&["Home", "Work"]);
        let to = dir.path().join("moved here");
        type_str(&mut app, "o");
        let Mode::Options(options) = &app.mode else { panic!("the options") };
        assert!(options.folder.is_some());
        type_str(&mut app, &"j".repeat(crate::options::FOLDER_ROW));
        press(&mut app, KeyCode::Enter);
        for _ in 0..200 {
            press(&mut app, KeyCode::Backspace);
        }
        app.handle_paste(&to.display().to_string()).unwrap();
        press(&mut app, KeyCode::Enter);
        let Mode::Options(options) = &app.mode else { panic!("still in the options") };
        assert!(options.message.as_ref().unwrap().1, "{:?}", options.message);
        assert_eq!(app.workspaces.as_ref().unwrap().list().unwrap(), ["Home", "Work"]);
        assert_eq!(app.settings.data_dir.as_deref(), Some(to.as_path()));
        assert!(!dir.path().join("todoro").exists());
        // The open todos were reopened from their new place, so changes save there.
        press(&mut app, KeyCode::Esc);
        type_str(&mut app, "aafter the move");
        press(&mut app, KeyCode::Enter);
        let saved = Store::open(to.join("Home").join("todos.json")).unwrap();
        assert_eq!(saved.items(today()).len(), 2);
    }

    #[test]
    fn a_clash_when_moving_the_folder_moves_nothing() {
        let (mut app, dir) = crate::test_util::app_with_workspaces(&["Home", "Work"]);
        let to = dir.path().join("taken");
        std::fs::create_dir_all(to.join("Work")).unwrap();
        type_str(&mut app, "o");
        type_str(&mut app, &"j".repeat(crate::options::FOLDER_ROW));
        press(&mut app, KeyCode::Enter);
        for _ in 0..200 {
            press(&mut app, KeyCode::Backspace);
        }
        app.handle_paste(&to.display().to_string()).unwrap();
        press(&mut app, KeyCode::Enter);
        let Mode::Options(options) = &app.mode else { panic!("still in the options") };
        let (message, ok) = options.message.as_ref().unwrap();
        assert!(!ok && message.contains("Work"), "{message}");
        assert_eq!(app.workspaces.as_ref().unwrap().dir, dir.path().join("todoro"));
        assert!(dir.path().join("todoro").join("Home").exists());
    }

    fn notes_file(dir: &tempfile::TempDir, file: &str) -> std::path::PathBuf {
        dir.path().join("notes").join(file)
    }

    #[test]
    fn notes_are_saved_to_a_markdown_file_as_you_type() {
        let (mut app, dir) = app_with(&["Buy milk"]);
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "i# Oat");
        assert_eq!(std::fs::read_to_string(notes_file(&dir, "buy-milk.md")).unwrap(), "# Oat");
        press(&mut app, KeyCode::Esc);
        type_str(&mut app, "q");
        let json = std::fs::read_to_string(dir.path().join("todos.json")).unwrap();
        assert!(json.contains("\"notes_file\": \"buy-milk.md\"") && !json.contains("# Oat"), "{json}");
    }

    #[test]
    fn opening_notes_shows_changes_made_in_another_app() {
        let (mut app, dir) = app_with(&["Buy milk"]);
        app.store.set_notes(today(), 0, "oat".into()).unwrap();
        std::fs::write(notes_file(&dir, "buy-milk.md"), "oat, two litres").unwrap();
        press(&mut app, KeyCode::Enter);
        let Mode::Notes(editor) = &app.mode else { panic!("the notes") };
        assert_eq!(editor.notes(), "oat, two litres");
    }

    #[test]
    fn a_clash_while_editing_keeps_both_versions_and_says_so() {
        let (mut app, dir) = app_with(&["Buy milk"]);
        app.store.set_notes(today(), 0, "oat".into()).unwrap();
        press(&mut app, KeyCode::Enter);
        std::fs::write(notes_file(&dir, "buy-milk.md"), "changed in another app").unwrap();
        // The first key typed is saved, which finds the clash.
        type_str(&mut app, "A,");
        // todoro's version went beside it, and the outside one stayed.
        assert_eq!(std::fs::read_to_string(notes_file(&dir, "buy-milk.md")).unwrap(), "changed in another app");
        assert_eq!(std::fs::read_to_string(notes_file(&dir, "buy-milk (conflict).md")).unwrap(), "oat,");
        assert!(app.message.as_ref().unwrap().contains("buy-milk (conflict).md"));
        // The open notes now show the outside version, so typing on can't overwrite it.
        let Mode::Notes(editor) = &app.mode else { panic!("still in the notes") };
        assert_eq!(editor.notes(), "changed in another app");
        // The message goes with the next key.
        press(&mut app, KeyCode::Esc);
        assert!(app.message.is_none());
    }

    #[test]
    fn copying_an_item_copies_its_notes_into_a_new_file() {
        let (mut app, dir) = app_with(&["Buy milk"]);
        app.store.set_notes(today(), 0, "oat".into()).unwrap();
        type_str(&mut app, "yyp");
        assert_eq!(app.items()[1].notes, "oat");
        assert_eq!(app.items()[1].notes_file.as_deref(), Some("buy-milk-2.md"));
        assert_eq!(std::fs::read_to_string(notes_file(&dir, "buy-milk-2.md")).unwrap(), "oat");
    }

    #[test]
    fn deleting_an_item_deletes_its_notes_file_and_undo_brings_it_back() {
        let (mut app, dir) = app_with(&["Buy milk"]);
        app.store.set_notes(today(), 0, "oat".into()).unwrap();
        type_str(&mut app, "dd");
        assert!(!notes_file(&dir, "buy-milk.md").exists());
        type_str(&mut app, "u");
        assert_eq!(std::fs::read_to_string(notes_file(&dir, "buy-milk.md")).unwrap(), "oat");
    }

    #[test]
    fn each_workspace_has_its_own_notes_folder() {
        let (mut app, dir) = crate::test_util::app_with_workspaces(&["Home", "Work"]);
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "ihome notes");
        press(&mut app, KeyCode::Esc);
        type_str(&mut app, "q");
        let path = dir.path().join("todoro").join("Home").join("notes").join("home-item.md");
        assert_eq!(std::fs::read_to_string(path).unwrap(), "home notes");
        assert!(!dir.path().join("todoro").join("Work").join("notes").exists());
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

    fn ok() -> io::Result<std::process::ExitStatus> {
        Ok(std::process::ExitStatus::default())
    }

    #[test]
    fn with_an_editor_set_enter_opens_the_notes_file_in_it() {
        let (mut app, _dir) = app_with(&["Buy milk", "Call Sam"]);
        app.settings.editor = Some("nvim".into());
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Enter);
        assert!(matches!(app.mode, Mode::Normal), "not the notes screen");
        let (command, path) = app.start_external_edit().unwrap().unwrap();
        assert_eq!(command, "nvim");
        assert_eq!(path, app.store.notes_dir().join("call-sam.md"));
        // Asked for once only.
        assert!(app.start_external_edit().unwrap().is_none());
        std::fs::write(&path, "about the trip\n").unwrap();
        app.finish_external_edit(ok()).unwrap();
        assert_eq!(app.items()[1].notes, "about the trip\n");
        assert!(app.message.is_none());
        // The whole visit is one undo step.
        press(&mut app, KeyCode::Char('u'));
        assert_eq!(app.items()[1].notes, "");
        assert!(!path.exists());
        ctrl(&mut app, 'r');
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "about the trip\n");
    }

    #[test]
    fn closing_the_editor_without_changes_records_nothing() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        app.settings.editor = Some("nvim".into());
        press(&mut app, KeyCode::Enter);
        let (_, path) = app.start_external_edit().unwrap().unwrap();
        app.finish_external_edit(ok()).unwrap();
        assert!(!path.exists(), "the empty file is tidied away");
        assert_eq!(app.items()[0].notes_file, None);
        press(&mut app, KeyCode::Char('u'));
        assert_eq!(app.items()[0].text, "Buy milk", "nothing to undo");
    }

    #[test]
    fn an_editor_that_wont_start_says_so() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        app.settings.editor = Some("nope".into());
        press(&mut app, KeyCode::Enter);
        app.start_external_edit().unwrap().unwrap();
        app.finish_external_edit(Err(io::Error::new(io::ErrorKind::NotFound, "not found"))).unwrap();
        let message = app.message.as_ref().unwrap();
        assert!(message.contains("Couldn't open nope") && message.contains("options"), "{message}");
    }

    #[cfg(unix)]
    #[test]
    fn an_editor_the_shell_cant_find_says_so() {
        use std::os::unix::process::ExitStatusExt;
        let (mut app, _dir) = app_with(&["Buy milk"]);
        app.settings.editor = Some("nope".into());
        for (code, says) in [(127, "Couldn't find nope"), (1, "nope closed with an error")] {
            press(&mut app, KeyCode::Enter);
            app.start_external_edit().unwrap().unwrap();
            app.finish_external_edit(Ok(std::process::ExitStatus::from_raw(code << 8))).unwrap();
            let message = app.message.as_ref().unwrap();
            assert!(message.contains(says), "{message}");
        }
    }

    #[test]
    fn without_an_editor_enter_opens_the_notes_screen() {
        for editor in [None, Some("  ".to_string())] {
            let (mut app, _dir) = app_with(&["Buy milk"]);
            app.settings.editor = editor;
            press(&mut app, KeyCode::Enter);
            assert!(matches!(app.mode, Mode::Notes(_)));
            assert!(app.start_external_edit().unwrap().is_none());
        }
        // Nor on an empty list.
        let (mut app, _dir) = app_with(&[]);
        app.settings.editor = Some("nvim".into());
        press(&mut app, KeyCode::Enter);
        assert!(app.start_external_edit().unwrap().is_none());
    }

    #[test]
    fn the_editor_is_set_from_the_options() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        type_str(&mut app, "o");
        type_str(&mut app, &"j".repeat(crate::options::EDITOR_ROW));
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "hx");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.settings.editor.as_deref(), Some("hx"));
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.start_external_edit().unwrap().unwrap().0, "hx");
    }

    #[test]
    fn v_views_the_notes_and_esc_goes_back() {
        let (mut app, _dir) = app_with(&["Buy milk", "Call Sam"]);
        app.store.set_notes(today(), 1, "# Trip".into()).unwrap();
        type_str(&mut app, "jv");
        assert!(matches!(app.mode, Mode::View(_)));
        // Keys that change things on the list do nothing here.
        type_str(&mut app, "xdp");
        assert!(matches!(app.mode, Mode::View(_)));
        assert_eq!(items(&app), ["Buy milk", "Call Sam"]);
        assert!(!app.items()[0].done);
        for close in [KeyCode::Esc, KeyCode::Char('q'), KeyCode::Char('v')] {
            press(&mut app, close);
            assert!(matches!(app.mode, Mode::Normal));
            assert_eq!(app.selected, 1);
            type_str(&mut app, "v");
        }
    }

    #[test]
    fn v_on_an_empty_list_does_nothing() {
        let (mut app, _dir) = app_with(&[]);
        type_str(&mut app, "v");
        assert!(matches!(app.mode, Mode::Normal));
    }

    #[test]
    fn v_rereads_the_notes_file() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        app.store.set_notes(today(), 0, "oat".into()).unwrap();
        std::fs::write(app.store.notes_dir().join("buy-milk.md"), "soy").unwrap();
        type_str(&mut app, "v");
        assert_eq!(app.items()[0].notes, "soy");
    }

    #[test]
    fn editing_from_the_viewer_comes_back_to_it() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        type_str(&mut app, "vi");
        let Mode::Notes(editor) = &app.mode else { panic!("the notes screen") };
        assert!(!editor.insert, "in normal mode, as from the list");
        type_str(&mut app, "ioat");
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Esc);
        assert!(matches!(app.mode, Mode::View(_)));
        assert_eq!(app.items()[0].notes, "oat");
        // Then back to the list, and the edit is one undo step.
        press(&mut app, KeyCode::Esc);
        type_str(&mut app, "u");
        assert_eq!(app.items()[0].notes, "");
        // Notes opened from the list still go back to the list.
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Esc);
        assert!(matches!(app.mode, Mode::Normal));
    }

    #[test]
    fn editing_from_the_viewer_in_your_own_editor_comes_back_to_it() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        app.settings.editor = Some("nvim".into());
        type_str(&mut app, "ve");
        let (_, path) = app.start_external_edit().unwrap().unwrap();
        std::fs::write(&path, "# Shopping").unwrap();
        app.finish_external_edit(ok()).unwrap();
        assert!(matches!(app.mode, Mode::View(_)));
        assert_eq!(app.items()[0].notes, "# Shopping");
        // From the list, it stays on the list.
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Enter);
        app.start_external_edit().unwrap().unwrap();
        app.finish_external_edit(ok()).unwrap();
        assert!(matches!(app.mode, Mode::Normal));
    }

    #[test]
    fn help_from_the_viewer_goes_back_to_it() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        type_str(&mut app, "v?");
        assert!(matches!(app.mode, Mode::Help { .. }));
        press(&mut app, KeyCode::Esc);
        assert!(matches!(app.mode, Mode::View(_)));
    }

    #[test]
    fn new_items_are_unpinned_by_default() {
        let (mut app, _dir) = app_with(&[]);
        type_str(&mut app, "aone");
        press(&mut app, KeyCode::Enter);
        assert!(!app.items()[0].pinned);
    }

    #[test]
    fn with_pin_new_items_on_new_items_start_pinned() {
        let (mut app, _dir) = app_with(&["old"]);
        app.settings.pin_new_items = true;
        // a, and each of several with A.
        type_str(&mut app, "anew");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "Atwo");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "three");
        press(&mut app, KeyCode::Esc);
        assert_eq!(items(&app), ["old", "new", "two", "three"]);
        let pinned: Vec<bool> = app.items().iter().map(|item| item.pinned).collect();
        assert_eq!(pinned, [false, true, true, true]);
        // Editing an item leaves its pin alone.
        type_str(&mut app, "gge!");
        press(&mut app, KeyCode::Enter);
        assert!(!app.items()[0].pinned);
        assert_eq!(items(&app)[0], "old!");
        // m still unpins one.
        type_str(&mut app, "jm");
        assert!(!app.items()[1].pinned);
        // Undo takes back the unpinning, then the edit.
        type_str(&mut app, "uu");
        assert!(app.items()[1].pinned);
        assert_eq!(items(&app), ["old", "new", "two", "three"]);
    }

    #[test]
    fn with_pin_new_items_on_items_added_in_the_calendar_start_pinned() {
        let (mut app, _dir) = app_with(&[]);
        app.settings.pin_new_items = true;
        type_str(&mut app, "clatrip");
        press(&mut app, KeyCode::Enter);
        let tomorrow = today().succ_opt().unwrap();
        assert_eq!(app.store.items(tomorrow)[0].text, "trip");
        assert!(app.store.items(tomorrow)[0].pinned);
    }

    #[test]
    fn pasting_keeps_each_items_own_pin_with_pin_new_items_on() {
        let (mut app, _dir) = app_with(&["plain"]);
        app.settings.pin_new_items = true;
        type_str(&mut app, "yyp");
        assert_eq!(items(&app), ["plain", "plain"]);
        assert!(!app.items()[1].pinned);
    }

    fn deadline(days: u64, time: Option<(u32, u32)>) -> crate::deadline::Deadline {
        let time = time.map(|(h, m)| NaiveTime::from_hms_opt(h, m, 0).unwrap());
        crate::deadline::Deadline { date: today() + Days::new(days), time }
    }

    #[test]
    fn at_sets_a_deadline_by_day_then_time() {
        let (mut app, _dir) = app_with(&["Buy milk", "Report"]);
        type_str(&mut app, "j@");
        let Mode::Calendar(calendar) = &app.mode else { panic!("the calendar") };
        assert!(calendar.picking.is_some());
        type_str(&mut app, "ll");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "1730");
        press(&mut app, KeyCode::Enter);
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.items()[1].deadline, Some(deadline(2, Some((17, 30)))));
        assert_eq!(app.selected, 1);
        // One undo step takes it off.
        type_str(&mut app, "u");
        assert_eq!(app.items()[1].deadline, None);
    }

    #[test]
    fn enter_twice_sets_just_the_day() {
        let (mut app, _dir) = app_with(&["Report"]);
        type_str(&mut app, "@j");
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.items()[0].deadline, Some(deadline(7, None)));
    }

    #[test]
    fn colon_deadline_does_the_same_as_at() {
        let (mut app, _dir) = app_with(&["Report"]);
        type_str(&mut app, ":deadline");
        press(&mut app, KeyCode::Enter);
        let Mode::Calendar(calendar) = &app.mode else { panic!("the calendar") };
        assert!(calendar.picking.is_some());
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "9am");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.items()[0].deadline, Some(deadline(0, Some((9, 0)))));
    }

    #[test]
    fn at_on_an_empty_list_does_nothing() {
        let (mut app, _dir) = app_with(&[]);
        type_str(&mut app, "@");
        assert!(matches!(app.mode, Mode::Normal));
        type_str(&mut app, ":deadline");
        press(&mut app, KeyCode::Enter);
        assert!(matches!(app.mode, Mode::Normal));
    }

    #[test]
    fn esc_cancels_choosing_a_deadline() {
        let (mut app, _dir) = app_with(&["Report"]);
        type_str(&mut app, "@l");
        press(&mut app, KeyCode::Esc);
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.items()[0].deadline, None);
        // Nothing changed, so nothing to undo.
        type_str(&mut app, "u");
        assert_eq!(items(&app), ["Report"]);
    }

    #[test]
    fn backing_out_of_changing_a_deadline_keeps_the_old_one() {
        let (mut app, _dir) = app_with(&["Report"]);
        let old = deadline(4, Some((17, 30)));
        app.store.set_deadline(today(), 0, Some(old)).unwrap();
        // A new day and a new time typed, then Esc back to the days and Esc again.
        type_str(&mut app, "@ll");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "9am");
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Esc);
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.items()[0].deadline, Some(old));
        // Opening it again starts from the old deadline, with its time.
        type_str(&mut app, "@");
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.items()[0].deadline, Some(old));
        // Cancelling with q or c works the same.
        for key in ["q", "c"] {
            type_str(&mut app, &format!("@jj{key}"));
            assert_eq!(app.items()[0].deadline, Some(old), "{key}");
        }
    }

    #[test]
    fn a_deadline_replaces_the_pin_and_m_says_so() {
        let (mut app, _dir) = app_with(&["Report"]);
        type_str(&mut app, "m@");
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter);
        assert!(!app.items()[0].pinned);
        type_str(&mut app, "m");
        assert!(!app.items()[0].pinned);
        assert!(app.message.as_ref().unwrap().contains("deadline"));
        // @ then d removes it, and m pins again.
        type_str(&mut app, "@d");
        assert_eq!(app.items()[0].deadline, None);
        type_str(&mut app, "m");
        assert!(app.items()[0].pinned);
    }

    #[test]
    fn pinning_several_leaves_ones_with_a_deadline_alone() {
        let (mut app, _dir) = app_with(&["one", "two"]);
        app.store.set_deadline(today(), 0, Some(deadline(1, None))).unwrap();
        type_str(&mut app, "Vjm");
        assert!(!app.items()[0].pinned);
        assert!(app.items()[1].pinned);
        assert!(app.items()[0].deadline.is_some());
    }

    #[test]
    fn items_with_a_deadline_show_on_later_days_until_it() {
        let (mut app, _dir) = app_with(&["Report", "Other"]);
        app.store.set_deadline(today(), 0, Some(deadline(2, None))).unwrap();
        type_str(&mut app, "l");
        assert_eq!(items(&app), ["Report"]);
        type_str(&mut app, "l");
        assert_eq!(items(&app), ["Report"], "on its deadline day");
        type_str(&mut app, "l");
        assert!(items(&app).is_empty(), "gone after it");
    }

    #[test]
    fn moving_a_deadline_earlier_than_the_day_on_screen_leaves_it() {
        let (mut app, _dir) = app_with(&["Report"]);
        app.store.set_deadline(today(), 0, Some(deadline(3, None))).unwrap();
        type_str(&mut app, "ll");
        assert_eq!(items(&app), ["Report"]);
        // Now due tomorrow, so it no longer shows two days on.
        type_str(&mut app, "@hh");
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter);
        assert!(items(&app).is_empty());
        assert_eq!(app.store.items(today())[0].deadline, Some(deadline(1, None)));
    }

    #[test]
    fn pasting_a_time_goes_into_the_time() {
        let (mut app, _dir) = app_with(&["Report"]);
        type_str(&mut app, "@");
        press(&mut app, KeyCode::Enter);
        app.handle_paste("14:15").unwrap();
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.items()[0].deadline, Some(deadline(0, Some((14, 15)))));
    }

    /// Chooses a deletion in the options and confirms it with `word`.
    fn clear_from_options(app: &mut App, clear: options::Clear, word: &str) {
        type_str(app, "o");
        let Mode::Options(popup) = &mut app.mode else { panic!("the options") };
        popup.selected = popup.first_clear_row() + popup.clears().iter().position(|&c| c == clear).unwrap();
        press(app, KeyCode::Enter);
        type_str(app, word);
        press(app, KeyCode::Enter);
    }

    /// Gives each workspace's item some notes, so there are files to delete.
    fn with_notes(app: &mut App, names: &[&str]) {
        let folder = app.workspaces.clone().unwrap();
        for name in names {
            if Some(name.to_string()) == app.workspace {
                app.store.set_notes(today(), 0, format!("{name} notes")).unwrap();
            } else {
                let mut store = Store::open(folder.todos_path(name)).unwrap();
                store.set_notes(today(), 0, format!("{name} notes")).unwrap();
            }
        }
    }

    fn notes_files(app: &App, name: &str) -> Vec<String> {
        let dir = app.workspaces.as_ref().unwrap().dir.join(name).join("notes");
        let mut files: Vec<String> = std::fs::read_dir(dir)
            .map(|entries| entries.map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect())
            .unwrap_or_default();
        files.sort();
        files
    }

    fn texts_in(app: &App, name: &str) -> Vec<String> {
        let store = Store::open(app.workspaces.as_ref().unwrap().todos_path(name)).unwrap();
        store.items(today()).iter().map(|item| item.text.clone()).collect()
    }

    #[test]
    fn deleting_the_workspaces_notes_keeps_its_items_and_other_workspaces() {
        let (mut app, _dir) = app_with_workspaces(&["Home", "Work"]);
        with_notes(&mut app, &["Home", "Work"]);
        let notes = app.workspaces.as_ref().unwrap().dir.join("Home").join("notes");
        // A conflict copy goes too, but not other files there.
        std::fs::write(notes.join("home-item (conflict).md"), "old").unwrap();
        std::fs::create_dir_all(notes.join(".obsidian")).unwrap();
        clear_from_options(&mut app, options::Clear::Notes, "Home");
        assert_eq!(items(&app), ["Home item"]);
        assert_eq!(app.items()[0].notes, "");
        assert_eq!(notes_files(&app, "Home"), [".obsidian"]);
        assert_eq!(notes_files(&app, "Work"), ["work-item.md"]);
        let Mode::Options(popup) = &app.mode else { panic!("still in the options") };
        assert_eq!(popup.message, Some(("Deleted every note in Home".into(), true)));
    }

    #[test]
    fn deleting_the_workspaces_items_and_notes_keeps_the_workspace() {
        let (mut app, _dir) = app_with_workspaces(&["Home", "Work"]);
        with_notes(&mut app, &["Home", "Work"]);
        clear_from_options(&mut app, options::Clear::Items, "Home");
        assert!(items(&app).is_empty());
        assert!(texts_in(&app, "Home").is_empty());
        assert!(notes_files(&app, "Home").is_empty());
        assert_eq!(texts_in(&app, "Work"), ["Work item"]);
        assert_eq!(app.workspaces.as_ref().unwrap().list().unwrap(), ["Home", "Work"]);
        // There's no undoing it.
        press(&mut app, KeyCode::Esc);
        type_str(&mut app, "u");
        assert!(items(&app).is_empty());
    }

    #[test]
    fn the_wrong_word_deletes_nothing() {
        let (mut app, _dir) = app_with_workspaces(&["Home"]);
        clear_from_options(&mut app, options::Clear::Items, "home");
        assert_eq!(items(&app), ["Home item"]);
        let Mode::Options(popup) = &app.mode else { panic!("still in the options") };
        assert!(popup.editing.is_some(), "still waiting for the word");
    }

    #[test]
    fn deleting_every_workspaces_notes_keeps_every_item() {
        let (mut app, _dir) = app_with_workspaces(&["Home", "Work"]);
        with_notes(&mut app, &["Home", "Work"]);
        clear_from_options(&mut app, options::Clear::AllNotes, "delete");
        assert!(notes_files(&app, "Home").is_empty());
        assert!(notes_files(&app, "Work").is_empty());
        assert_eq!(app.items()[0].notes, "");
        assert_eq!(texts_in(&app, "Home"), ["Home item"]);
        assert_eq!(texts_in(&app, "Work"), ["Work item"]);
    }

    #[test]
    fn deleting_every_workspaces_items_keeps_the_workspaces() {
        let (mut app, _dir) = app_with_workspaces(&["Home", "Work"]);
        with_notes(&mut app, &["Home", "Work"]);
        clear_from_options(&mut app, options::Clear::AllItems, "delete");
        assert!(items(&app).is_empty());
        assert!(texts_in(&app, "Work").is_empty());
        assert!(notes_files(&app, "Work").is_empty());
        assert_eq!(app.workspaces.as_ref().unwrap().list().unwrap(), ["Home", "Work"]);
    }

    #[test]
    fn resetting_deletes_every_workspace_and_the_settings_then_restarts() {
        let (mut app, dir) = app_with_workspaces(&["Home", "Work"]);
        let settings = dir.path().join("settings.json");
        app.settings_path = Some(settings.clone());
        app.settings.twelve_hour = true;
        app.settings.save(&settings).unwrap();
        let folder = app.workspaces.as_ref().unwrap().dir.clone();
        clear_from_options(&mut app, options::Clear::Reset, "reset");
        assert!(app.restart);
        assert!(!folder.exists(), "the empty todoro folder goes");
        assert!(!settings.exists());
    }

    #[test]
    fn resetting_leaves_other_files_in_the_todoro_folder() {
        let (mut app, _dir) = app_with_workspaces(&["Home"]);
        let folder = app.workspaces.as_ref().unwrap().dir.clone();
        std::fs::write(folder.join("readme.txt"), "mine").unwrap();
        std::fs::create_dir_all(folder.join("photos")).unwrap();
        clear_from_options(&mut app, options::Clear::Reset, "reset");
        assert!(app.restart);
        assert!(!folder.join("Home").exists());
        assert!(folder.join("readme.txt").exists());
        assert!(folder.join("photos").exists(), "not a workspace: it has no todos file");
    }

    #[test]
    fn with_a_single_file_only_its_deletions_are_offered() {
        let (mut app, _dir) = app_with(&["one"]);
        app.store.set_notes(today(), 0, "n".into()).unwrap();
        clear_from_options(&mut app, options::Clear::Notes, "delete");
        assert_eq!(items(&app), ["one"]);
        assert_eq!(app.items()[0].notes, "");
        press(&mut app, KeyCode::Esc);
        clear_from_options(&mut app, options::Clear::Items, "delete");
        assert!(items(&app).is_empty());
    }
}
