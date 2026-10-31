use std::cell::Cell;
use std::io;
use std::path::PathBuf;

use chrono::{Days, NaiveDate, NaiveTime, Timelike};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::calendar::{self, Calendar};
use crate::changelog::{self, ChangelogView};
use crate::clipboard::{Clipboard, Copied};
use crate::export;
use crate::help::Help;
use crate::input::LineInput;
use crate::keymap::{self, Command, Key, Keymap};
use crate::leader::{self, Leader};
use crate::notes::{Action, NotesEditor, Register};
use crate::options::{self, Options, Settings, TOGGLES};
use crate::timeline::{self, Plan, TimeSlot, Typing};
use crate::outline;
use crate::reminder;
use crate::search::{self, Search};
use crate::tags::{self, TagPicker};
use crate::update;
use crate::viewer::{self, Viewer};
use crate::store::{Item, Priority, Snapshot, Store};
use crate::workspaces::{self, Picker, Workspaces};

/// How long a brief message (like `yy`'s "No notes to copy") shows for.
const BRIEF_MESSAGE: std::time::Duration = std::time::Duration::from_secs(1);

/// How many changes `u` can undo.
const UNDO_LIMIT: usize = 200;

/// Every item, plus the day and row on screen, from before (or after) a change.
pub struct State {
    snapshot: Snapshot,
    day: NaiveDate,
    selected: usize,
}

pub enum Mode {
    Normal,
    /// Typing an item at screen row `index`. When `editing`, it replaces the
    /// existing item there; otherwise it's a new one, which goes at `place`.
    /// With `repeat` (from `A`), Enter adds the item and starts another below
    /// it, until Esc or Enter on an empty line. With `replacing` (from `E`),
    /// the item's text starts cleared, and leaving it empty keeps it as it was.
    Insert { index: usize, input: LineInput, editing: bool, repeat: bool, replacing: bool, place: Option<Place> },
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
    /// `:update`: checking for a newer todoro, then its notes.
    Update(Box<update::Popup>),
    /// The `#` list of every tag.
    Tags(TagPicker),
    /// Fuzzy search over every day's items.
    Search(Box<Search>),
    /// The calendar, for planning ahead.
    Calendar(Box<Calendar>),
    /// The `r` popup, to choose when to be reminded of the selected item.
    Remind(reminder::Picker),
    /// The `Space` popup of less frequent commands, narrowing as keys are typed.
    Leader(Leader),
    /// The day as a timeline, items with a time as blocks (`Space t`).
    /// `scroll` is the hour at the top, once moved from where it starts.
    Timeline { scroll: Option<u32> },
    /// Giving the day's items their times (`Space p`), beside the timeline.
    /// `before` is how things were when it opened, for reset, cancel and undo.
    Plan { plan: Plan, before: Box<State> },
    /// The keybinding help popup, over the screen in `back`.
    Help { help: Help, back: Box<Mode> },
}

/// Notes asked to be opened in the user's own editor.
struct External {
    slot: Slot,
    /// The line to open them on, counting from 0, from searching notes.
    line: Option<usize>,
    /// The state before, once the editor is open, for undo.
    before: Option<State>,
}

/// Editors that open a file on a line given as `+N` before it.
const PLUS_LINE_EDITORS: &[&str] = &["vi", "vim", "nvim", "nano", "emacs", "emacsclient", "micro", "kak"];

/// `command` with `+N` added to open the file on `line` (counting from 0),
/// if it's an editor known to take that; otherwise just `command`.
fn with_line(command: &str, line: Option<usize>) -> String {
    let program = command.split_whitespace().next().unwrap_or_default();
    let name = std::path::Path::new(program).file_stem().and_then(|name| name.to_str()).unwrap_or_default();
    match line {
        Some(line) if PLUS_LINE_EDITORS.contains(&name) => format!("{command} +{}", line + 1),
        _ => command.to_string(),
    }
}

/// Starts typing a start time in the plan view.
fn plan_typing(mode: &mut Mode) {
    if let Mode::Plan { plan, .. } = mode {
        plan.typing = Some(Typing::Start(LineInput::default()));
        plan.error = None;
    }
}

/// Where a new item goes: the day it's stored on, its index there, and how
/// deep (0 for a top-level item, more for a sub-item).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    pub day: NaiveDate,
    pub index: usize,
    pub depth: usize,
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
    /// The hour at the top of the timeline as last drawn, to scroll from.
    pub timeline_top: Cell<u32>,
    /// Whether the plan view had room for the timeline when last drawn.
    pub timeline_shown: Cell<bool>,
    /// The plan view, set aside while an item is added, edited or deleted
    /// from it, to go back to once that's done. What's typed then is part
    /// of the plan, saved or put back with it.
    pub plan_stash: Option<(Plan, Box<State>)>,
    /// The plan view's own undo and redo, a step for each change made in
    /// it, while it's open. Saving still makes the whole plan one change.
    plan_undo: Vec<State>,
    plan_redo: Vec<State>,
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
    /// The start of a key sequence (`g` of `gg`), waiting for the rest.
    pending: Vec<Key>,
    /// A count typed before a command, like the 4 in `4j`.
    count: Option<usize>,
    /// The `:` command being typed on the list, if any.
    pub command: Option<LineInput>,
    /// Something to tell the user, shown in the status bar until the next key.
    pub message: Option<String>,
    /// Whether `message` goes by itself, `BRIEF_MESSAGE` after it shows,
    /// and once the first tick has seen it, when.
    message_brief: bool,
    /// When items were copied on the list (`yy`), and their rows, with
    /// their sub-items', while they flash to show it.
    pub list_flash: Option<(std::time::Instant, Vec<usize>)>,
    /// Where newer versions are asked for: GitHub, or something else in tests.
    pub fetcher: update::Fetcher,
    /// The day's check for a newer version, while it's running.
    update_check: Option<std::sync::mpsc::Receiver<Result<String, String>>>,
    /// `:update`'s check, while it's running.
    update_find: Option<std::sync::mpsc::Receiver<Result<update::Found, String>>>,
    /// The version to install, once chosen in `:update`'s popup, for the
    /// event loop in `main.rs`, which owns the terminal the installer needs.
    pub install_update: Option<String>,
    message_ends: Option<std::time::Instant>,
    /// Where `yy` in the viewer copies to.
    pub clipboard: Clipboard,
    /// The items last copied (`yy`) or deleted, for `p` and `P` to paste.
    pub register: Vec<Item>,
    /// Text last copied or deleted in any item's notes, kept for the next
    /// notes screen as vim keeps its register.
    notes_register: Option<Register>,
    /// Notes to open in the user's own editor (`Settings::editor`).
    external: Option<External>,
    /// Whether editing was started from the viewer, to go back to it after.
    view_after_edit: bool,
    /// Which tag suggestion is selected while typing a `#tag`.
    pub tag_choice: usize,
    /// The keys of the list and the leader menu, with the user's own.
    pub keymap: Keymap,
    /// The user's `keys.toml`, if there's somewhere for it.
    pub keys_path: Option<PathBuf>,
    /// Editing `keys.toml` in the user's editor: asked for, then open.
    keys_edit: KeysEdit,
}

#[derive(Clone, Copy, PartialEq)]
enum KeysEdit {
    No,
    Asked,
    Open,
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
            timeline_top: Cell::new(0),
            timeline_shown: Cell::new(true),
            plan_stash: None,
            plan_undo: Vec::new(),
            plan_redo: Vec::new(),
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
            pending: Vec::new(),
            count: None,
            command: None,
            message: None,
            message_brief: false,
            list_flash: None,
            fetcher: update::Fetcher::github(),
            update_check: None,
            update_find: None,
            install_update: None,
            message_ends: None,
            clipboard: Clipboard::System,
            register: Vec::new(),
            notes_register: None,
            external: None,
            view_after_edit: false,
            tag_choice: 0,
            keymap: Keymap::default(),
            keys_path: None,
            keys_edit: KeysEdit::No,
        }
    }

    /// Every item shown for the current day, in screen order. Today and future
    /// days also show, first, the pinned items from earlier days that will have
    /// moved to them by then. Usually startup has already moved them to today,
    /// but one can land on a past day mid-session, e.g. moved there with `<`.
    /// Then come items from later days whose reminder is due by then.
    pub fn slots(&self) -> Vec<Slot> {
        let roots: Vec<(NaiveDate, usize)> = if self.day >= self.today {
            let carried = self.store.carried_to(self.day, self.today).into_iter();
            let reminded = self.store.reminded_on(self.day, self.today, self.now).into_iter();
            carried.chain(reminded).collect()
        } else {
            Vec::new()
        };
        // Each top-level item, then what shows of its sub-items. Planning
        // and the timeline are about what's left to do, so they leave out
        // what's completed (and a completed item's sub-items are too).
        let hide_done = self.planning_view();
        let shown = |day: NaiveDate, start: usize| {
            let items = self.store.items(day);
            outline::visible_block(items, start)
                .into_iter()
                .filter(move |&index| !(hide_done && items[index].done))
                .map(move |index| Slot { day, index })
        };
        let mut slots: Vec<Slot> = Vec::new();
        for (day, index) in roots {
            slots.extend(shown(day, index));
        }
        let items = self.store.items(self.day);
        let mut at = 0;
        while at < items.len() {
            slots.extend(shown(self.day, at));
            at = outline::block_end(items, at);
        }
        slots
    }

    /// Whether the plan view or the timeline is on screen (with or without
    /// help over it, or an item being typed from the plan).
    pub fn planning_view(&self) -> bool {
        let screen = match &self.mode {
            Mode::Help { back, .. } => back.as_ref(),
            mode => mode,
        };
        matches!(screen, Mode::Plan { .. } | Mode::Timeline { .. }) || self.plan_stash.is_some()
    }

    /// How many rows come before the day's completed top-level items.
    pub fn open_rows(&self) -> usize {
        self.slots()
            .iter()
            .position(|slot| slot.day == self.day && self.store.items(slot.day)[slot.index].depth == 0 && self.store.items(slot.day)[slot.index].done)
            .unwrap_or(self.slots().len())
    }

    /// The row of open top-level item number `n` (the numbers on the list),
    /// or the last row if there are fewer.
    fn row_for_number(&self, n: usize) -> usize {
        let open = self.open_rows();
        let slots = self.slots();
        slots[..open]
            .iter()
            .enumerate()
            .filter(|(_, slot)| self.store.items(slot.day)[slot.index].depth == 0)
            .map(|(row, _)| row)
            .nth(n.saturating_sub(1))
            .unwrap_or(slots.len().saturating_sub(1))
    }

    /// The items shown for the current day, in screen order.
    pub fn items(&self) -> Vec<&Item> {
        self.slots().into_iter().map(|slot| &self.store.items(slot.day)[slot.index]).collect()
    }

    /// How many shown items come from other days: earlier ones, or later
    /// ones being reminded of.
    pub fn carried(&self) -> usize {
        self.slots().iter().take_while(|slot| slot.day != self.day).count()
    }

    fn slot(&self, index: usize) -> Option<Slot> {
        self.slots().get(index).copied()
    }

    fn clear_message(&mut self) {
        self.message = None;
        self.message_brief = false;
        self.message_ends = None;
    }

    /// Shows a message that goes by itself after `BRIEF_MESSAGE`, as well as
    /// with the next key.
    fn brief_message(&mut self, message: String) {
        self.message = Some(message);
        self.message_brief = true;
        self.message_ends = None;
    }

    fn clamp_selection(&mut self) {
        self.selected = self.selected.min(self.slots().len().saturating_sub(1));
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> io::Result<()> {
        self.clear_message();
        // Any key ends a copy's flash early.
        self.list_flash = None;
        let before = self.slot(self.selected);
        let result = self.key(key);
        self.report_conflicts(before);
        result
    }

    /// Tells the user about notes files that changed outside todoro, which
    /// saving left alone. If those notes are open, they show the outside
    /// version, so the next key can't overwrite it.
    ///
    /// If a save merged into a todos file changed outside todoro, `selected`
    /// (the item selected before, as the key saw it) is followed to where
    /// it went, so notes being typed keep going to it. Anything else open
    /// holds old positions, so it closes, back to the list. Undo history is
    /// cleared, as undoing would bring back the old file.
    fn report_conflicts(&mut self, selected: Option<Slot>) {
        if let Some(merge) = self.store.take_merge() {
            self.undo.clear();
            self.redo.clear();
            self.plan_undo.clear();
            self.plan_redo.clear();
            self.plan_stash = None;
            self.notes_before = None;
            self.pending.clear();
            if !matches!(self.mode, Mode::Normal | Mode::Notes(_)) {
                self.mode = Mode::Normal;
            }
            match selected.and_then(|slot| Some((slot.day, merge.index(slot.day, slot.index)?))) {
                Some((day, index)) => self.select_stored(day, index),
                None => self.clamp_selection(),
            }
            if merge.added {
                self.message = Some("Changed elsewhere meanwhile: both kept, yours at the end".into());
            }
        }
        if let Some(copy) = self.store.take_kept_outside() {
            let name = copy.file_name().unwrap_or_default().to_string_lossy();
            self.message = Some(format!("The todos file changed outside todoro to something that isn't todos. That version is in {name}."));
        }
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
        // In the plan view, u and Ctrl+R step through its own changes.
        if let Mode::Plan { plan, .. } = &self.mode
            && plan.typing.is_none()
            && !plan.confirm_cancel
        {
            match key.code {
                KeyCode::Char('u') if !ctrl => return self.plan_step(true),
                KeyCode::Char('r') if ctrl => return self.plan_step(false),
                _ => {}
            }
        }
        let plan_before = (matches!(self.mode, Mode::Plan { .. }) || self.plan_stash.is_some()).then(|| self.state());
        // The list's keys and the leader menu's run commands, changed by
        // keys.toml. Undo and redo aren't changes to record themselves.
        if let Some(resolved) = self.resolve(key) {
            let Some((command, count)) = resolved else { return Ok(()) };
            match command {
                Command::Undo => return self.undo(),
                Command::Redo => return self.redo(),
                _ => {}
            }
            let before = self.state();
            self.run(command, count)?;
            self.record(before);
            return Ok(());
        }
        // The calendar's undo, which keeps its own keys (not from the plan
        // view, which has its own).
        if let Mode::Calendar(calendar) = &self.mode
            && calendar.adding.is_none()
            && calendar.command.is_none()
            && calendar.snake.is_none()
            && self.plan_stash.is_none()
        {
            match key.code {
                KeyCode::Char('u') if !ctrl => return self.undo(),
                KeyCode::Char('r') if ctrl => return self.redo(),
                _ => {}
            }
        }
        // Any change made from the list or calendar is undoable. The notes
        // screen records its own change when it closes, and help changes
        // nothing. Edits made from the plan view are part of the plan, so the
        // plan records them when it's saved.
        let before = (self.plan_stash.is_none() && matches!(
            self.mode,
            Mode::Normal
                | Mode::Insert { .. }
                | Mode::ConfirmDelete { .. }
                | Mode::Visual { .. }
                | Mode::Calendar(_)
                | Mode::Remind(_)
        ))
            .then(|| self.state());
        self.mode_key(key)?;
        if let Some(before) = before {
            self.record(before);
        }
        self.back_to_plan();
        // A change in the plan view is a step to undo in it, while it's open.
        if let Some(before) = plan_before
            && before.snapshot != self.store.snapshot()
            && (matches!(self.mode, Mode::Plan { .. }) || self.plan_stash.is_some())
        {
            self.plan_undo.push(before);
            self.plan_redo.clear();
        }
        Ok(())
    }

    /// Undoes the plan view's last change (`back`), or redoes it, keeping
    /// whether the plan has changed since it opened up to date.
    fn plan_step(&mut self, back: bool) -> io::Result<()> {
        let (from, to) = if back { (&mut self.plan_undo, &mut self.plan_redo) } else { (&mut self.plan_redo, &mut self.plan_undo) };
        let Some(state) = from.pop() else {
            self.message = Some(if back { "Nothing in the plan to undo" } else { "Nothing in the plan to redo" }.into());
            return Ok(());
        };
        to.push(State { snapshot: self.store.snapshot(), day: self.day, selected: self.selected });
        self.store.restore(state.snapshot)?;
        self.selected = state.selected;
        self.clamp_selection();
        let now = self.store.snapshot();
        if let Mode::Plan { plan, before } = &mut self.mode {
            plan.changed = before.snapshot != now;
            if plan.on_timeline {
                self.select_block();
            }
        }
        Ok(())
    }

    /// Back to the plan view once an item added, edited or deleted from it
    /// is done, noting that the plan changed.
    fn back_to_plan(&mut self) {
        if !matches!(self.mode, Mode::Normal) {
            return;
        }
        let Some((mut plan, before)) = self.plan_stash.take() else { return };
        plan.changed |= before.snapshot != self.store.snapshot();
        let on_timeline = plan.on_timeline;
        self.mode = Mode::Plan { plan, before };
        if on_timeline {
            self.select_block();
        }
    }

    fn mode_key(&mut self, key: KeyEvent) -> io::Result<()> {
        if self.complete_tag(key) {
            return Ok(());
        }
        match &mut self.mode {
            Mode::Normal if self.command.is_some() => self.command_key(key.code),
            // Handled in `resolve`.
            Mode::Normal => {}
            Mode::Insert { index, input, editing, repeat, replacing, place } => {
                if input.handle_key(key.code) {
                    let (index, text, editing, replacing, place) = (*index, input.text.trim().to_string(), *editing, *replacing, *place);
                    let next = *repeat && key.code == KeyCode::Enter && !text.is_empty();
                    self.mode = Mode::Normal;
                    match (editing, text.is_empty()) {
                        // Nothing typed after E: leave it as it was.
                        (true, true) if replacing => {}
                        (true, true) => self.mode = Mode::ConfirmDelete { rows: vec![index] },
                        (true, false) => {
                            if let Some(slot) = self.slot(index) {
                                self.store.set_text(slot.day, slot.index, text)?;
                            }
                        }
                        (false, true) => {}
                        (false, false) => {
                            let place = place.unwrap_or(Place { day: self.day, index: self.store.open_count(self.day), depth: 0 });
                            let at = self.add_item(place, text)?;
                            self.select_stored(place.day, at);
                            if next {
                                // The next one goes just below, at the same level.
                                let place = Some(Place { index: at + 1, ..place });
                                let index = self.selected + 1;
                                self.mode = Mode::Insert { index, input: LineInput::default(), editing: false, repeat: true, replacing: false, place };
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
                    self.register.clear();
                    for (day, indices) in self.group_by_day(&rows) {
                        let removed = self.store.remove_many(day, &indices)?;
                        self.register.extend(removed);
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
                viewer::Action::StartSaving => {
                    let default = self.export_default();
                    if let Mode::View(viewer) = &mut self.mode {
                        viewer.saving = Some(viewer::Saving::new(default));
                    }
                }
                viewer::Action::Save => self.save_export(),
                viewer::Action::Copy => self.copy_view(),
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
                    self.add_item(Place { day, index, depth: 0 }, text)?;
                }
                calendar::Action::SetDeadline { day, index, deadline } => {
                    self.mode = Mode::Normal;
                    self.store.set_deadline(day, index, deadline)?;
                    // A deadline before the day on screen means it no
                    // longer shows here.
                    self.select_stored(day, index);
                }
                calendar::Action::SetReminder { day, index, reminder } => {
                    self.mode = Mode::Normal;
                    self.store.set_reminder(day, index, Some(reminder))?;
                    self.select_stored(day, index);
                }
                calendar::Action::Help => self.open_help(),
            },
            // Handled with the list's keys, in `resolve`.
            Mode::Leader(_) => {}
            Mode::Timeline { .. } => self.timeline_key(key.code),
            Mode::Plan { .. } => self.plan_key(key)?,
            Mode::Remind(picker) => {
                let (day, index) = (picker.day, picker.index);
                match picker.handle_key(key.code) {
                    reminder::Action::Stay => {}
                    reminder::Action::Close => self.mode = Mode::Normal,
                    reminder::Action::Set(reminder) => {
                        self.mode = Mode::Normal;
                        self.store.set_reminder(day, index, reminder)?;
                        self.select_stored(day, index);
                    }
                    reminder::Action::Calendar => {
                        let item = &self.store.items(day)[index];
                        let calendar = Calendar::for_reminder(
                            (day, index),
                            &item.text,
                            item.reminder,
                            self.day,
                            self.today,
                            self.settings.twelve_hour,
                        );
                        self.mode = Mode::Calendar(Box::new(calendar));
                    }
                }
            }
            Mode::Changelog(view) => {
                if view.handle_key(key) {
                    self.mode = Mode::Normal;
                }
            }
            Mode::Update(popup) => match popup.handle_key(key) {
                update::Action::Stay => {}
                update::Action::Close => {
                    self.mode = Mode::Normal;
                    self.update_find = None;
                }
                update::Action::Install(version) => {
                    self.mode = Mode::Normal;
                    self.install_update = Some(version);
                }
            },
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
                options::Action::EditKeys => match &self.keys_path {
                    Some(_) => self.keys_edit = KeysEdit::Asked,
                    None => popup.message = Some(("There's no settings folder here to keep keys.toml in".into(), false)),
                },
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
                let hits = search.find(&self.store, self.today, self.now);
                match search.handle_key(key, &hits) {
                    search::Action::Stay => {}
                    search::Action::Close => self.mode = Mode::Normal,
                    search::Action::Open { day, index, line } => {
                        self.mode = Mode::Normal;
                        self.reveal(day, index)?;
                        self.show_day(day);
                        self.selected = self.slots().iter().position(|slot| *slot == Slot { day, index }).unwrap_or(0);
                        // Found in its notes: open them to edit, on that line.
                        if let Some(line) = line {
                            self.open_notes()?;
                            if let Mode::Notes(editor) = &mut self.mode {
                                editor.start_on_line(line + 1);
                            }
                            if let Some(external) = &mut self.external {
                                external.line = Some(line);
                            }
                        }
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

    /// What a key on the list or in the leader menu runs, once a whole
    /// binding is typed: `None` if it isn't one of those (the key is for
    /// something else), `Some(None)` if it ran nothing yet (a count, or the
    /// start of a sequence), or the command with any count typed before it.
    fn resolve(&mut self, event: KeyEvent) -> Option<Option<(Command, Option<usize>)>> {
        match &mut self.mode {
            Mode::Normal if self.command.is_none() => Some(self.resolve_list(Key::from_event(event))),
            Mode::Leader(leader) => match leader.handle_key(event, &self.keymap.leader) {
                leader::Action::Stay => Some(None),
                leader::Action::Close => {
                    self.mode = Mode::Normal;
                    Some(None)
                }
                leader::Action::Run(command) => {
                    self.mode = Mode::Normal;
                    Some(Some((command, None)))
                }
            },
            _ => None,
        }
    }

    fn resolve_list(&mut self, key: Key) -> Option<(Command, Option<usize>)> {
        // Digits build a count for the next command, as in vim: 4j moves down
        // four items and 42G goes to item 42. A 0 only counts after another digit.
        if self.pending.is_empty()
            && !key.ctrl
            && let KeyCode::Char(c @ '0'..='9') = key.code
            && (c != '0' || self.count.is_some())
        {
            let digit = c as usize - '0' as usize;
            self.count = Some((self.count.unwrap_or(0) * 10 + digit).min(99_999));
            return None;
        }
        self.pending.push(key);
        match keymap::lookup(&self.keymap.list, &self.pending) {
            keymap::Match::Run(command) => {
                self.pending.clear();
                Some((command, self.count.take()))
            }
            // Wait for the rest, keeping the count, as in 7gg.
            keymap::Match::More => None,
            keymap::Match::Nothing => {
                // Any other key cancels an unfinished sequence, then does
                // its own thing, as in vim.
                let unfinished = self.pending.len() > 1;
                self.pending.clear();
                if unfinished {
                    return self.resolve_list(key);
                }
                self.count = None;
                None
            }
        }
    }

    /// Opens the selected item's notes to edit: in the user's editor if one
    /// is set (which the event loop runs, since it owns the terminal),
    /// otherwise on the notes screen.
    fn open_notes(&mut self) -> io::Result<()> {
        let Some(slot) = self.slot(self.selected) else { return Ok(()) };
        if self.editor().is_some() {
            self.external = Some(External { slot, line: None, before: None });
            return Ok(());
        }
        // Read the notes file again, in case something else changed it.
        self.store.reload_notes(slot.day, slot.index)?;
        self.notes_before = Some(self.state());
        let notes = &self.store.items(slot.day)[slot.index].notes;
        let mut editor = NotesEditor::new(notes);
        editor.register = self.notes_register.clone();
        // Ready to type, or in normal mode, as the options say for empty
        // notes and for notes with text.
        let typing = if notes.trim().is_empty() { !self.settings.empty_notes_normal } else { self.settings.notes_insert };
        if typing {
            editor.start_typing();
        }
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

    /// The item being typed, on the list or in the calendar, if any.
    fn typing(&self) -> Option<&LineInput> {
        match &self.mode {
            Mode::Insert { input, .. } => Some(input),
            Mode::Calendar(calendar) => calendar.adding.as_ref(),
            _ => None,
        }
    }

    fn typing_mut(&mut self) -> Option<&mut LineInput> {
        match &mut self.mode {
            Mode::Insert { input, .. } => Some(input),
            Mode::Calendar(calendar) => calendar.adding.as_mut(),
            _ => None,
        }
    }

    /// How wide the name of the `#tag` being typed is so far, to line the
    /// suggestions up under it.
    pub fn typing_tag_width(&self) -> usize {
        use unicode_width::UnicodeWidthStr;
        self.typing().and_then(|input| tags::typing_tag(&input.text, input.cursor)).map_or(0, |(_, typed)| typed.width())
    }

    /// Tags to complete the `#tag` being typed with, most used first.
    pub fn tag_suggestions(&self) -> Vec<(String, usize)> {
        let Some(input) = self.typing() else { return Vec::new() };
        match tags::typing_tag(&input.text, input.cursor) {
            Some((_, typed)) => tags::suggestions(&self.store, typed),
            None => Vec::new(),
        }
    }

    /// Keys for the tag suggestions while typing a `#tag`: `Tab` fills in
    /// the selected one, and `↑`/`↓` (or `Ctrl+N`/`Ctrl+P`) choose. Returns
    /// whether the key was used; any other key types as usual, and `Enter`
    /// keeps the tag as typed.
    fn complete_tag(&mut self, key: KeyEvent) -> bool {
        let suggestions = self.tag_suggestions();
        if suggestions.is_empty() {
            self.tag_choice = 0;
            return false;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let last = suggestions.len() - 1;
        match key.code {
            KeyCode::Up => self.tag_choice = self.tag_choice.saturating_sub(1),
            KeyCode::Char('p') if ctrl => self.tag_choice = self.tag_choice.saturating_sub(1),
            KeyCode::Down => self.tag_choice = (self.tag_choice + 1).min(last),
            KeyCode::Char('n') if ctrl => self.tag_choice = (self.tag_choice + 1).min(last),
            KeyCode::Tab => {
                let name = suggestions[self.tag_choice.min(last)].0.clone();
                self.tag_choice = 0;
                if let Some(input) = self.typing_mut()
                    && let Some((start, _)) = tags::typing_tag(&input.text, input.cursor)
                {
                    input.text.replace_range(start..input.cursor, &name);
                    input.cursor = start + name.len();
                }
            }
            _ => {
                // Typing changes what's suggested, so start from the top.
                self.tag_choice = 0;
                return false;
            }
        }
        true
    }

    /// Carries out a command from the list's keys or the leader menu, with
    /// the count typed before it, if any.
    fn run(&mut self, command: Command, count: Option<usize>) -> io::Result<()> {
        let slot = self.slot(self.selected);
        let len = self.slots().len();
        let times = count.unwrap_or(1);
        match command {
            Command::First => self.selected = count.map_or(0, |n| self.row_for_number(n)),
            Command::Last => self.selected = count.map_or(len.saturating_sub(1), |n| self.row_for_number(n)),
            Command::Overdue => self.mode = Mode::Search(Box::new(Search::overdue())),
            Command::Copy if len > 0 => self.copy_items(&[self.selected]),
            Command::Leader => self.mode = Mode::Leader(Leader::default()),
            // Paste below or above the cursor, like a and its opposite.
            Command::PasteBelow => self.paste(true)?,
            Command::PasteAbove => self.paste(false)?,
            Command::Quit => self.quit = true,
            Command::Colon => self.command = Some(LineInput::default()),
            Command::Help => self.open_help(),
            Command::PrevDay => self.change_day(-1),
            Command::NextDay => self.change_day(1),
            Command::Down => self.selected = (self.selected + times).min(len.saturating_sub(1)),
            Command::Up => self.selected = self.selected.saturating_sub(times),
            // Move the item down or up, within the open or completed items.
            Command::MoveDown => self.move_selected(1)?,
            Command::MoveUp => self.move_selected(-1)?,
            // Like the day keys, but taking the selected item along to that day.
            Command::MoveToPrevDay => self.move_to_day(-1)?,
            Command::MoveToNextDay => self.move_to_day(1)?,
            // Like vim's `a`, append after the cursor (and what's under it). From
            // a completed item (or an empty day), append to the end of the open
            // items instead.
            Command::Add | Command::AddSeveral | Command::AddSub | Command::AddSubs => {
                let (index, place) = self.new_item_place(matches!(command, Command::AddSub | Command::AddSubs))?;
                let repeat = matches!(command, Command::AddSeveral | Command::AddSubs);
                let input = LineInput::default();
                self.mode = Mode::Insert { index, input, editing: false, repeat, replacing: false, place: Some(place) };
            }
            Command::Indent => {
                if let Some(slot) = slot {
                    match self.store.indent(slot.day, slot.index)? {
                        Some(index) => self.select_stored(slot.day, index),
                        None => self.message = Some("There's nothing above it at its level to go under.".into()),
                    }
                }
            }
            Command::Outdent => {
                if let Some(slot) = slot
                    && let Some(index) = self.store.outdent(slot.day, slot.index)?
                {
                    self.select_stored(slot.day, index);
                }
            }
            Command::FoldToggle | Command::Fold | Command::Unfold => {
                if let Some(slot) = slot {
                    self.fold(slot, command)?;
                }
            }
            Command::Edit if len > 0 => {
                let input = LineInput::new(&self.items()[self.selected].text);
                self.mode = Mode::Insert { index: self.selected, input, editing: true, repeat: false, replacing: false, place: None };
            }
            // Edit with the text cleared, to write it again, as vim's cc.
            Command::EditCleared if len > 0 => {
                let input = LineInput::default();
                self.mode = Mode::Insert { index: self.selected, input, editing: true, repeat: false, replacing: true, place: None };
            }
            Command::Delete if len > 0 => self.mode = Mode::ConfirmDelete { rows: vec![self.selected] },
            Command::Select if len > 0 => self.mode = Mode::Visual { anchor: self.selected },
            // The cursor stays put, so you can tick off several items in a row.
            // A carried item is changed where it's stored, which can take it off
            // this day's screen, hence the clamp.
            Command::ToggleDone => {
                if let Some(slot) = slot {
                    self.store.toggle_done(slot.day, slot.index)?;
                    self.clamp_selection();
                }
            }
            // Like the calendar's t: back to today, where you were if already there.
            Command::Today if self.day != self.today => self.show_day(self.today),
            // Triage: cycle the item's priority.
            Command::Triage => {
                if let Some(slot) = slot {
                    self.store.cycle_priority(slot.day, slot.index)?;
                }
            }
            // Pin: "mark" the item to carry forward. One with a deadline
            // already moves on until then.
            Command::TogglePin => {
                if let Some(slot) = slot {
                    if self.store.items(slot.day)[slot.index].depth > 0 {
                        self.message = Some("Sub-items go where the item they're under goes: pin that instead.".into());
                    } else if self.store.items(slot.day)[slot.index].deadline.is_some() {
                        let how = self.keymap.key_for(Command::Deadline).map_or(String::new(), |key| format!(" {key} then d removes it."));
                        self.message = Some(format!("This has a deadline, so it moves on until then.{how}"));
                    } else {
                        self.store.toggle_pinned(slot.day, slot.index)?;
                        self.clamp_selection();
                    }
                }
            }
            Command::Deadline => self.open_deadline(),
            Command::Reminder => self.open_reminder(),
            Command::OpenNotes if len > 0 => self.open_notes()?,
            Command::ViewNotes if len > 0 => {
                // Read the notes file again, in case something else changed it.
                if let Some(slot) = slot {
                    self.store.reload_notes(slot.day, slot.index)?;
                }
                self.mode = Mode::View(Viewer::default());
            }
            Command::ViewAll if len > 0 => {
                if let Some(slot) = slot {
                    let end = outline::block_end(self.store.items(slot.day), slot.index);
                    for index in slot.index..end {
                        self.store.reload_notes(slot.day, index)?;
                    }
                }
                self.mode = Mode::View(Viewer::whole());
            }
            Command::ViewDay => {
                // Every item on the list and everything under it, so read
                // each notes file again in case something else changed it.
                for slot in self.slots() {
                    if self.store.items(slot.day)[slot.index].depth == 0 {
                        let end = outline::block_end(self.store.items(slot.day), slot.index);
                        for index in slot.index..end {
                            self.store.reload_notes(slot.day, index)?;
                        }
                    }
                }
                self.mode = Mode::View(Viewer::day());
            }
            // Handled before running, as they aren't changes to record.
            Command::Undo | Command::Redo => {}
            // Needing an item, with none.
            Command::Copy | Command::Edit | Command::EditCleared | Command::Delete | Command::Select | Command::OpenNotes | Command::ViewNotes | Command::ViewAll | Command::Today => {}
            // Like finding files in many vim setups.
            Command::FindItems => self.mode = Mode::Search(Box::new(Search::new(search::Kind::Items))),
            Command::SearchNotes => self.mode = Mode::Search(Box::new(Search::new(search::Kind::Notes))),
            Command::Deadlines => self.mode = Mode::Search(Box::new(Search::filtered(search::Filter::Deadlines))),
            Command::Reminders => self.mode = Mode::Search(Box::new(Search::filtered(search::Filter::Reminders))),
            Command::Calendar => self.mode = Mode::Calendar(Box::new(Calendar::new(self.day, self.today))),
            Command::Tags => self.mode = Mode::Tags(TagPicker::default()),
            // Searching the options opens them ready to type.
            Command::Options | Command::SearchOptions => {
                let folder = self.workspaces.as_ref().map(|folder| workspaces::display_path(&folder.dir));
                let mut options = Options::new(self.settings.editor.as_deref(), folder, self.workspace.clone());
                options.keys = self.keys_path.as_deref().map(workspaces::display_path);
                if command == Command::SearchOptions {
                    options.start_search();
                }
                self.mode = Mode::Options(options);
            }
            Command::Workspaces => match (&self.workspaces, &self.workspace) {
                (Some(folder), Some(current)) => self.mode = Mode::Workspaces(Picker::new(&folder.list()?, current)),
                _ => self.message = Some("Workspaces are off while TODORO_FILE is set.".into()),
            },
            Command::PatchNotes => self.mode = Mode::Changelog(ChangelogView::all()),
            Command::Timeline => self.mode = Mode::Timeline { scroll: None },
            Command::Plan => self.open_plan(),
        }
        Ok(())
    }

    /// Keys on the timeline: `j`/`k` scroll an hour, `h`/`l` the day before
    /// or after, `t` today, `p` plans the day, `Esc`/`q` back to the list.
    fn timeline_key(&mut self, code: KeyCode) {
        let Mode::Timeline { scroll } = &mut self.mode else { return };
        match code {
            KeyCode::Char('j') | KeyCode::Down => *scroll = Some(self.timeline_top.get() + 1),
            KeyCode::Char('k') | KeyCode::Up => *scroll = Some(self.timeline_top.get().saturating_sub(1)),
            KeyCode::Char('h') | KeyCode::Left => {
                *scroll = None;
                self.change_day(-1);
            }
            KeyCode::Char('l') | KeyCode::Right => {
                *scroll = None;
                self.change_day(1);
            }
            KeyCode::Char('t') => {
                *scroll = None;
                self.show_day(self.today);
            }
            KeyCode::Char('p') => self.open_plan(),
            KeyCode::Char('?') => self.open_help(),
            KeyCode::Esc | KeyCode::Char('q') => self.mode = Mode::Normal,
            _ => {}
        }
    }

    /// Opens the plan view for the day on screen, on the selected item (or
    /// the top-level item it's under).
    fn open_plan(&mut self) {
        if !self.slots().iter().any(|slot| self.plannable(*slot)) {
            self.message = Some("There's nothing open to plan on this day.".into());
            return;
        }
        self.select_root();
        let before = Box::new(self.state());
        self.plan_undo.clear();
        self.plan_redo.clear();
        self.mode = Mode::Plan { plan: Plan::default(), before };
    }

    /// Whether the item at `slot` can be given a time: an open, top-level one.
    fn plannable(&self, slot: Slot) -> bool {
        let item = &self.store.items(slot.day)[slot.index];
        item.depth == 0 && !item.done
    }

    /// Moves the selection from a sub-item to the top-level item it's under.
    fn select_root(&mut self) {
        if let Some(slot) = self.slot(self.selected) {
            let items = self.store.items(slot.day);
            if let Some(&root) = outline::ancestors(items, slot.index).last() {
                self.select_stored(slot.day, root);
            }
        }
    }

    /// The rows of the items that are blocks on the timeline, by when they
    /// start: what can be chosen there.
    pub fn timeline_blocks(&self) -> Vec<usize> {
        let mut timed: Vec<_> = self
            .slots()
            .iter()
            .enumerate()
            .filter_map(|(row, slot)| {
                let item = &self.store.items(slot.day)[slot.index];
                let time = item.time.filter(|time| item.depth == 0 && time.date == self.day)?;
                Some((time.start, time.end, row))
            })
            .collect();
        timed.sort();
        timed.into_iter().map(|(_, _, row)| row).collect()
    }

    /// The blocks on the timeline as it lays them out: each one's row, its
    /// time, and its column (overlapping blocks sit side by side).
    fn timeline_columns(&self) -> Vec<(usize, TimeSlot, usize)> {
        let slots = self.slots();
        let timed: Vec<(usize, TimeSlot)> = self
            .timeline_blocks()
            .into_iter()
            .filter_map(|row| Some((row, self.store.items(slots[row].day)[slots[row].index].time?)))
            .collect();
        timeline::place(&timed)
            .into_iter()
            .filter_map(|placed| timed.iter().find(|(row, _)| *row == placed.item).map(|(row, time)| (*row, *time, placed.column)))
            .collect()
    }

    /// The block to go to from the selected one on the timeline: down or up
    /// its column (`j`/`k`), or into the column beside it (`h`/`l`), to the
    /// block there that overlaps it most, or else starts nearest.
    fn timeline_step(&self, code: KeyCode) -> Option<usize> {
        let layout = self.timeline_columns();
        let &(_, time, column) = layout.iter().find(|(row, ..)| *row == self.selected)?;
        let in_column = |c: usize| layout.iter().filter(move |(_, _, col)| *col == c);
        match code {
            KeyCode::Char('j') | KeyCode::Down => {
                in_column(column).filter(|(_, t, _)| t.start > time.start).min_by_key(|(_, t, _)| t.start).map(|b| b.0)
            }
            KeyCode::Char('k') | KeyCode::Up => {
                in_column(column).filter(|(_, t, _)| t.start < time.start).max_by_key(|(_, t, _)| t.start).map(|b| b.0)
            }
            KeyCode::Char('h' | 'l') => {
                let to = if code == KeyCode::Char('l') { column + 1 } else { column.checked_sub(1)? };
                let minutes = |t: chrono::NaiveTime| i64::from(t.hour() * 60 + t.minute());
                let overlap = |t: &TimeSlot| minutes(t.end.min(time.end)) - minutes(t.start.max(time.start));
                let distance = |t: &TimeSlot| (minutes(t.start) - minutes(time.start)).abs();
                in_column(to).max_by_key(|(_, t, _)| (overlap(t).max(0), -distance(t))).map(|b| b.0)
            }
            _ => None,
        }
    }

    /// On the timeline, makes sure a block is selected: the selected item if
    /// it's one, otherwise the next block after where it was, or the last.
    /// With no blocks, back to the list. Returns whether it's on a block.
    fn select_block(&mut self) -> bool {
        let blocks = self.timeline_blocks();
        if blocks.contains(&self.selected) {
            return true;
        }
        match blocks.iter().find(|&&row| row > self.selected).or(blocks.last()) {
            Some(&row) => {
                self.selected = row;
                true
            }
            None => {
                if let Mode::Plan { plan, .. } = &mut self.mode {
                    plan.on_timeline = false;
                }
                false
            }
        }
    }

    /// The row of the next item to plan after the selected one, if any.
    /// Only those still without a time on the day: below the selected one,
    /// then from the top, so none is missed.
    fn next_plannable(&self) -> Option<usize> {
        let slots = self.slots();
        let untimed = |row: usize| {
            let slot = slots[row];
            self.plannable(slot) && !self.store.items(slot.day)[slot.index].time.is_some_and(|time| time.date == self.day)
        };
        (self.selected + 1..slots.len()).chain(0..self.selected).find(|&row| untimed(row))
    }

    /// Keys in the plan view. `Enter` types the selected item's start time
    /// (nothing typed is now), then its end (nothing typed is an hour later),
    /// and goes on to the next item, as `Tab` does without changing it; `c`
    /// clears its time and `x` (or `d`) deletes it; `w` saves, `R` resets,
    /// `Esc` cancels.
    fn plan_key(&mut self, event: KeyEvent) -> io::Result<()> {
        let code = event.code;
        let Mode::Plan { plan, .. } = &mut self.mode else { return Ok(()) };
        if plan.confirm_cancel {
            match code {
                KeyCode::Char('q') => self.close_plan(false)?,
                KeyCode::Char('w') => self.close_plan(true)?,
                KeyCode::Char('n') | KeyCode::Esc => plan.confirm_cancel = false,
                _ => {}
            }
            return Ok(());
        }
        if let Some(typing) = &mut plan.typing {
            let (Typing::Start(input) | Typing::End(_, input)) = typing;
            match code {
                // Back out of this item, keeping the rest.
                KeyCode::Esc => {
                    plan.typing = None;
                    plan.error = None;
                }
                // Leave this item as it is, for the next one.
                KeyCode::Tab => {
                    plan.typing = None;
                    plan.error = None;
                    self.next_in_plan();
                }
                KeyCode::Enter => {
                    let read = timeline::read_time(&input.text);
                    let typing = match (typing, read) {
                        (_, Err(error)) => {
                            plan.error = Some(error);
                            return Ok(());
                        }
                        // Nothing typed for the start: as the last one set ends,
                        // or now, to the minute.
                        (Typing::Start(_), Ok(start)) => {
                            let start = start.unwrap_or_else(|| plan.default_start(self.now));
                            Some(Typing::End(start, LineInput::default()))
                        }
                        (Typing::End(start, _), Ok(end)) => {
                            let start = *start;
                            let end = end.unwrap_or_else(|| timeline::an_hour_after(start));
                            if end <= start {
                                plan.error = Some("It has to end after it starts".into());
                                return Ok(());
                            }
                            plan.changed = true;
                            plan.last_end = Some(end);
                            if let Some(slot) = self.slot(self.selected) {
                                self.store.set_time(slot.day, slot.index, Some(TimeSlot { date: self.day, start, end }))?;
                            }
                            None
                        }
                    };
                    let Mode::Plan { plan, .. } = &mut self.mode else { return Ok(()) };
                    plan.error = None;
                    plan.typing = typing;
                    // From the list, on to the next item; on the timeline,
                    // stay on the block just moved.
                    if plan.typing.is_none() && !plan.on_timeline {
                        self.next_in_plan();
                    }
                }
                code => {
                    input.handle_key(code);
                    plan.error = None;
                }
            }
            return Ok(());
        }
        // Partway through a sequence (gg, yy, za) or a count (4j): the
        // list's keys finish it.
        if !self.pending.is_empty() || self.count.is_some() && !matches!(code, KeyCode::Char('0'..='9')) && !plan.on_timeline {
            return self.plan_list_key(event);
        }
        // On the timeline, moving follows the blocks: down and up a column,
        // and across to the next. Only H goes back to the list.
        if plan.on_timeline {
            let blocks = self.timeline_blocks();
            match code {
                KeyCode::Char('j' | 'k' | 'h' | 'l') | KeyCode::Down | KeyCode::Up => {
                    if let Some(row) = self.timeline_step(code) {
                        self.selected = row;
                    }
                    return Ok(());
                }
                KeyCode::Char('g') | KeyCode::Char('G') => {
                    let end = if code == KeyCode::Char('g') { blocks.first() } else { blocks.last() };
                    if let Some(&row) = end {
                        self.selected = row;
                    }
                    return Ok(());
                }
                _ => {}
            }
        }
        let Mode::Plan { plan, .. } = &mut self.mode else { return Ok(()) };
        match code {
            // Over to the timeline, on the selected item (or what it's under),
            // and back.
            KeyCode::Char('L') if self.timeline_shown.get() => {
                plan.on_timeline = true;
                self.select_root();
                if !self.select_block() {
                    self.message = Some("Nothing's on the timeline yet: give an item a time first.".into());
                }
            }
            KeyCode::Char('L') => self.message = Some("Widen the terminal to see the timeline beside the list.".into()),
            KeyCode::Char('H') => plan.on_timeline = false,
            KeyCode::Enter => {
                self.select_root();
                if let Some(slot) = self.slot(self.selected)
                    && self.plannable(slot)
                {
                    plan_typing(&mut self.mode);
                } else {
                    self.message = Some("Only open, top-level items get a time.".into());
                }
            }
            // Clear the item's time.
            KeyCode::Char('c') => {
                self.select_root();
                if let Some(slot) = self.slot(self.selected)
                    && self.store.items(slot.day)[slot.index].time.is_some()
                {
                    self.store.set_time(slot.day, slot.index, None)?;
                    if let Mode::Plan { plan, .. } = &mut self.mode {
                        plan.changed = true;
                        // It's all day now, so not a block to be on.
                        if plan.on_timeline {
                            self.select_block();
                        }
                    }
                }
            }
            // Adding, editing and deleting as on the list, typed in place,
            // then back to the plan.
            KeyCode::Char(c @ ('a' | 'A' | 'o' | 'O' | 'e' | 'E' | 'd' | 'x')) => {
                let command = match c {
                    'a' => Command::Add,
                    'A' => Command::AddSeveral,
                    'o' => Command::AddSub,
                    'O' => Command::AddSubs,
                    'e' => Command::Edit,
                    'E' => Command::EditCleared,
                    _ => Command::Delete,
                };
                if let Mode::Plan { plan, before } = std::mem::replace(&mut self.mode, Mode::Normal) {
                    self.plan_stash = Some((plan, before));
                }
                self.run(command, None)?;
                // Nothing to do it to, on an empty day: straight back.
                self.back_to_plan();
            }
            // Triage, as on the list, part of the plan like its times.
            KeyCode::Char('!') => {
                if let Some(slot) = self.slot(self.selected) {
                    self.store.cycle_priority(slot.day, slot.index)?;
                    if let Mode::Plan { plan, .. } = &mut self.mode {
                        plan.changed = true;
                    }
                }
            }
            KeyCode::Char('w') => self.close_plan(true)?,
            // Reset: every item on the day back to all day.
            KeyCode::Char('R') => {
                let timed: Vec<Slot> = self
                    .slots()
                    .into_iter()
                    .filter(|slot| self.store.items(slot.day)[slot.index].time.is_some_and(|time| time.date == self.day))
                    .collect();
                for slot in &timed {
                    self.store.set_time(slot.day, slot.index, None)?;
                }
                if let Mode::Plan { plan, .. } = &mut self.mode
                    && !timed.is_empty()
                {
                    plan.changed = true;
                    plan.on_timeline = false;
                    // Nothing to follow on from now.
                    plan.last_end = None;
                }
                self.message = Some(if timed.is_empty() { "Nothing has a time to clear" } else { "Every time cleared" }.into());
            }
            KeyCode::Esc | KeyCode::Char('q') if plan.changed => plan.confirm_cancel = true,
            KeyCode::Esc | KeyCode::Char('q') => self.close_plan(false)?,
            KeyCode::Char('?') => self.open_help(),
            // Everything else as on the list, from its keys.
            _ => self.plan_list_key(event)?,
        }
        Ok(())
    }

    /// The list's keys that work in the plan view too, as `keys.toml` has
    /// them, with counts: moving, reordering, copying and pasting,
    /// indenting, folding, pinning, deadlines, reminders and selecting.
    /// Those that open something come back to the plan once it's done.
    fn plan_list_key(&mut self, event: KeyEvent) -> io::Result<()> {
        let Some((command, count)) = self.resolve_list(Key::from_event(event)) else { return Ok(()) };
        match command {
            Command::Down | Command::Up | Command::First | Command::Last => self.run(command, count)?,
            Command::MoveDown
            | Command::MoveUp
            | Command::Copy
            | Command::PasteBelow
            | Command::PasteAbove
            | Command::Indent
            | Command::Outdent
            | Command::FoldToggle
            | Command::Fold
            | Command::Unfold
            | Command::TogglePin
            | Command::Deadline
            | Command::Reminder
            | Command::Select => {
                if let Mode::Plan { plan, before } = std::mem::replace(&mut self.mode, Mode::Normal) {
                    self.plan_stash = Some((plan, before));
                }
                self.run(command, count)?;
                self.back_to_plan();
            }
            _ => {}
        }
        Ok(())
    }

    /// After an item's time, goes on to the next item and starts on its
    /// time, or says it's done if that was the last.
    fn next_in_plan(&mut self) {
        match self.next_plannable() {
            Some(row) => {
                self.selected = row;
                plan_typing(&mut self.mode);
            }
            None => self.message = Some("Every item has a time now. w saves the plan.".into()),
        }
    }

    /// Leaves the plan view, keeping what was planned as one change to undo
    /// (`save`), or putting back how it was.
    fn close_plan(&mut self, save: bool) -> io::Result<()> {
        let Mode::Plan { before, .. } = std::mem::replace(&mut self.mode, Mode::Normal) else { return Ok(()) };
        self.plan_undo.clear();
        self.plan_redo.clear();
        if save {
            let changed = before.snapshot != self.store.snapshot();
            self.record(*before);
            self.message = Some(if changed { "Plan saved" } else { "Nothing changed" }.into());
        } else {
            let selected = self.selected;
            self.store.restore(before.snapshot)?;
            self.selected = selected;
            self.clamp_selection();
        }
        Ok(())
    }

    /// Folds or unfolds the item at `slot` (`za`, `zc`, `zo`). An item with
    /// nothing under it folds the item it's under instead, as vim closes
    /// the fold around the cursor.
    fn fold(&mut self, slot: Slot, command: Command) -> io::Result<()> {
        let items = self.store.items(slot.day);
        let has_children = outline::block_end(items, slot.index) > slot.index + 1;
        let index = match (has_children, outline::parent(items, slot.index)) {
            (true, _) => slot.index,
            (false, Some(parent)) if command != Command::Unfold => parent,
            _ => return Ok(()),
        };
        let folded = match command {
            Command::Fold => true,
            Command::Unfold => false,
            _ => !self.store.items(slot.day)[index].folded,
        };
        self.store.set_folded(slot.day, index, folded)?;
        self.select_stored(slot.day, index);
        Ok(())
    }

    /// Stays on the item stored at `day`, `index`, wherever it shows now, or
    /// on what it's folded under, if it still shows.
    fn select_stored(&mut self, day: NaiveDate, index: usize) {
        let slots = self.slots();
        let items = self.store.items(day);
        let wanted = std::iter::once(index).chain(if index < items.len() { outline::ancestors(items, index) } else { Vec::new() });
        for index in wanted {
            if let Some(row) = slots.iter().position(|slot| slot.day == day && slot.index == index) {
                self.selected = row;
                break;
            }
        }
        self.clamp_selection();
    }

    /// Opens the popup to choose when to be reminded of the selected item.
    fn open_reminder(&mut self) {
        let Some(slot) = self.slot(self.selected) else { return };
        let current = self.store.items(slot.day)[slot.index].reminder;
        self.mode = Mode::Remind(reminder::Picker::new((slot.day, slot.index), current));
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

    /// Adds a new item at `place`, pinned if it's a top-level one and the
    /// options say new items start pinned. Returns its index.
    fn add_item(&mut self, place: Place, text: String) -> io::Result<usize> {
        let pinned = self.settings.pin_new_items && place.depth == 0;
        let item = Item { text, pinned, ..Item::default() };
        self.store.insert_at(place.day, place.index, vec![item], place.depth)
    }

    /// Where a new item typed now goes, and the row it's typed on: below
    /// the selected item and what's under it, at its level (`a`), or as
    /// its last sub-item (`o`). Top-level items go among the day's own open
    /// ones, after any carried from other days.
    fn new_item_place(&mut self, sub: bool) -> io::Result<(usize, Place)> {
        let open = self.store.open_count(self.day);
        let Some(slot) = self.slot(self.selected) else {
            return Ok((0, Place { day: self.day, index: open, depth: 0 }));
        };
        let item = &self.store.items(slot.day)[slot.index];
        let (depth, folded, done) = (item.depth, item.folded, item.done);
        if sub && folded {
            self.store.set_folded(slot.day, slot.index, false)?;
        }
        let items = self.store.items(slot.day);
        let below = self.selected + outline::visible_block(items, slot.index).len();
        let after = outline::block_end(items, slot.index);
        Ok(match (sub, depth) {
            (true, _) => (below, Place { day: slot.day, index: after, depth: depth + 1 }),
            (false, 0) if slot.day != self.day => (self.carried(), Place { day: self.day, index: 0, depth: 0 }),
            (false, 0) if done => (self.open_rows(), Place { day: self.day, index: open, depth: 0 }),
            (false, _) => (below, Place { day: slot.day, index: after, depth }),
        })
    }

    /// Shows the item stored at `day`, `index` if a folded item hides it.
    fn reveal(&mut self, day: NaiveDate, index: usize) -> io::Result<()> {
        let items = self.store.items(day);
        if index >= items.len() || !outline::hidden(items, index) {
            return Ok(());
        }
        for up in outline::ancestors(items, index) {
            self.store.set_folded(day, up, false)?;
        }
        Ok(())
    }

    /// After editing notes started from the viewer, shows them there again.
    /// Where the day's text is saved unless another place is typed: the
    /// `export` folder in the todoro folder (or beside `TODORO_FILE`).
    fn export_default(&self) -> std::path::PathBuf {
        let dir = match &self.workspaces {
            Some(workspaces) => workspaces.dir.clone(),
            None => self.store.path().parent().map(std::path::Path::to_path_buf).unwrap_or_default(),
        };
        // The workspace's name, when there's more than one to tell apart.
        let several = self.workspaces.as_ref().is_some_and(|workspaces| workspaces.list().is_ok_and(|names| names.len() > 1));
        let workspace = self.workspace.as_deref().filter(|_| several);
        dir.join(export::EXPORT_DIR).join(export::file_name(self.day, workspace))
    }

    /// Copies what the viewer shows to the clipboard, as text: the day as
    /// it's saved, the item with its sub-items in the same way, or the
    /// item's notes as written.
    fn copy_view(&mut self) {
        let Mode::View(viewer) = &self.mode else { return };
        let (text, what) = match viewer.scope {
            viewer::Scope::Day => (export::day_text(self), "the day"),
            viewer::Scope::Item => (export::item_text(self), "the item and its sub-items"),
            viewer::Scope::Notes => {
                let notes = self.items().get(self.selected).map(|item| item.notes.clone()).unwrap_or_default();
                if notes.trim().is_empty() {
                    self.brief_message("No notes to copy".into());
                    return;
                }
                (notes, "the notes")
            }
        };
        // The text flashes to show it's copied, as copied lines do in the
        // notes.
        if self.copy_to_clipboard(&text, what)
            && let Mode::View(viewer) = &mut self.mode
        {
            viewer.flash = Some(std::time::Instant::now());
        }
    }

    /// Puts `text` on the system clipboard, saying so only if it went
    /// through the terminal, which may not allow it, or failed. Whether it
    /// was copied, for a flash to show it.
    fn copy_to_clipboard(&mut self, text: &str, what: &str) -> bool {
        match self.clipboard.copy(text) {
            Ok(copied) => {
                if copied == Copied::Terminal {
                    self.brief_message(format!("Asked the terminal to copy {what}"));
                }
                true
            }
            Err(error) => {
                self.brief_message(format!("Couldn't copy: {error}"));
                false
            }
        }
    }

    /// Saves the day's text where the `w` prompt says. A file that's already
    /// there is only replaced on a second `Enter`.
    fn save_export(&mut self) {
        let text = export::day_text(self);
        let Mode::View(viewer) = &mut self.mode else { return };
        let Some(saving) = &mut viewer.saving else { return };
        let path = export::target(&saving.input.text, &saving.default);
        let shown = workspaces::display_path(&path);
        if path.is_dir() {
            saving.error = Some(format!("{shown} is a folder"));
            return;
        }
        if path.exists() && !saving.replace {
            saving.replace = true;
            saving.error = Some("That file exists. ↵ again replaces it".into());
            return;
        }
        match export::write(&path, &text) {
            Ok(()) => {
                viewer.saving = None;
                self.message = Some(format!("Saved to {shown}"));
            }
            Err(error) => {
                saving.replace = false;
                saving.error = Some(format!("Couldn't save: {error}"));
            }
        }
    }

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
                    "remind" | "reminder" => self.open_reminder(),
                    "overdue" => self.mode = Mode::Search(Box::new(Search::overdue())),
                    "update" => {
                        self.mode = Mode::Update(Box::new(update::Popup::Checking));
                        self.update_find = Some(update::find(self.fetcher));
                    }
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
            KeyCode::Esc | KeyCode::Char('V' | 'v') => {}
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
            KeyCode::Char('y') => self.copy_items(&rows),
            KeyCode::Char('d') => {
                self.mode = Mode::ConfirmDelete { rows };
                return Ok(());
            }
            // A plan is for its day, so selecting from it doesn't move items off it.
            KeyCode::Char('H' | 'L') if self.plan_stash.is_some() => return Ok(()),
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
                self.select_stored(to, landed.unwrap_or(0));
                return Ok(());
            }
            _ => return Ok(()),
        }
        self.mode = Mode::Normal;
        self.selected = first;
        self.clamp_selection();
        Ok(())
    }

    /// Copies of the items at screen `rows` with their sub-items, each
    /// starting at the top level, as `yy` and `y` keep them to paste.
    /// Copies the items at screen `rows`, with their sub-items, to paste
    /// with `p`, and their text to the system clipboard (`export::titles`).
    /// Their rows flash to show it.
    fn copy_items(&mut self, rows: &[usize]) {
        self.register = self.copy_rows(rows);
        let text = export::titles(&self.register);
        let what = if self.register.len() == 1 { "the item" } else { "the items" };
        if self.copy_to_clipboard(&text, what) {
            self.list_flash = Some((std::time::Instant::now(), self.block_rows(rows)));
        }
    }

    /// The screen rows of the items at `rows` and of what shows of their
    /// sub-items.
    fn block_rows(&self, rows: &[usize]) -> Vec<usize> {
        let mut blocks = Vec::new();
        for (day, indices) in self.group_by_day(rows) {
            let items = self.store.items(day);
            blocks.extend(outline::outermost(items, &indices).into_iter().map(|start| (day, start..outline::block_end(items, start))));
        }
        let slots = self.slots();
        (0..slots.len()).filter(|&row| blocks.iter().any(|(day, block)| *day == slots[row].day && block.contains(&slots[row].index))).collect()
    }

    fn copy_rows(&self, rows: &[usize]) -> Vec<Item> {
        let mut copied = Vec::new();
        for (day, indices) in self.group_by_day(rows) {
            let items = self.store.items(day);
            for start in outline::outermost(items, &indices) {
                let mut block = items[start..outline::block_end(items, start)].to_vec();
                outline::rebase(&mut block, 0);
                copied.extend(block);
            }
        }
        copied
    }

    /// Pastes the register below the selected item and what's under it
    /// (`below`), or above it, at its level, and selects the first pasted
    /// item. Top-level items go among the day's own open ones, as new ones do.
    fn paste(&mut self, below: bool) -> io::Result<()> {
        if self.register.is_empty() {
            return Ok(());
        }
        let register = self.register.clone();
        let open = self.store.open_count(self.day);
        let place = match self.slot(self.selected) {
            None => Place { day: self.day, index: 0, depth: 0 },
            Some(slot) => {
                let items = self.store.items(slot.day);
                let item = &items[slot.index];
                let index = if below { outline::block_end(items, slot.index) } else { slot.index };
                match item.depth {
                    0 if slot.day != self.day => Place { day: self.day, index: 0, depth: 0 },
                    0 if item.done => Place { day: self.day, index: open, depth: 0 },
                    depth => Place { day: slot.day, index, depth },
                }
            }
        };
        let at = if place.depth == 0 {
            self.store.insert_items(place.day, place.index, register)?
        } else {
            self.store.insert_at(place.day, place.index, register, place.depth)?
        };
        self.select_stored(place.day, at);
        Ok(())
    }

    /// Moves the selected item (with what's under it) past the next item at
    /// its level, or above the previous one, and keeps it selected.
    fn move_selected(&mut self, delta: isize) -> io::Result<()> {
        let Some(slot) = self.slot(self.selected) else { return Ok(()) };
        if let Some(index) = self.store.move_sibling(slot.day, slot.index, delta > 0)? {
            self.select_stored(slot.day, index);
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
            self.select_stored(to, index);
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

    /// How to open the options, in brackets, like ` (Space o)`.
    fn options_hint(&self) -> String {
        self.keymap.key_for(Command::Options).map_or(String::new(), |key| format!(" ({key})"))
    }

    /// Reads the user's `keys.toml` again, and says what in it couldn't be
    /// used, if anything: in the options if they're open, otherwise in the
    /// status bar. Returns whether all of it could.
    pub fn load_keys(&mut self) -> bool {
        let (keymap, problems) = Keymap::load(self.keys_path.as_deref());
        self.keymap = keymap;
        self.pending.clear();
        let Some(first) = problems.first() else { return true };
        let more = match problems.len() {
            1 => String::new(),
            n => format!(" (and {} more)", n - 1),
        };
        let message = format!("keys.toml: {first}{more}");
        match &mut self.mode {
            Mode::Options(options) => options.message = Some((message, false)),
            _ => self.message = Some(message),
        }
        false
    }

    /// The editor for `keys.toml`: the one set for notes, otherwise
    /// `$VISUAL` or `$EDITOR`, otherwise one every system has.
    fn keys_editor(&self) -> String {
        let from_env = ["VISUAL", "EDITOR"]
            .iter()
            .filter_map(|name| std::env::var(name).ok())
            .map(|command| command.trim().to_string())
            .find(|command| !command.is_empty());
        let fallback = if cfg!(windows) { "notepad" } else { "vi" };
        self.editor().map(String::from).or(from_env).unwrap_or_else(|| fallback.into())
    }

    /// If `Enter` asked to open notes in the user's editor: the command and
    /// the notes file to open with it. An item without notes gets an empty
    /// file. Call `finish_external_edit` once the editor has closed.
    pub fn start_external_edit(&mut self) -> io::Result<Option<(String, PathBuf)>> {
        if self.keys_edit == KeysEdit::Asked {
            let Some(path) = self.keys_path.clone() else {
                self.keys_edit = KeysEdit::No;
                return Ok(None);
            };
            keymap::ensure_file(&path)?;
            self.keys_edit = KeysEdit::Open;
            return Ok(Some((self.keys_editor(), path)));
        }
        // Only a request not started yet; one that's open waits for its finish.
        let Some(External { slot, line, before: None }) = self.external else { return Ok(None) };
        let Some(command) = self.editor().map(String::from) else {
            self.external = None;
            return Ok(None);
        };
        // Read the file first, so undo goes back to what was in it.
        self.store.reload_notes(slot.day, slot.index)?;
        let before = self.state();
        let Some(path) = self.store.notes_path(slot.day, slot.index)? else { return Ok(None) };
        self.external = Some(External { slot, line, before: Some(before) });
        Ok(Some((with_line(&command, line), path)))
    }

    /// Takes in what the user's editor saved, as one change to undo, and
    /// says if the editor couldn't run.
    pub fn finish_external_edit(&mut self, result: io::Result<std::process::ExitStatus>) -> io::Result<()> {
        if self.keys_edit == KeysEdit::Open {
            self.keys_edit = KeysEdit::No;
            let command = self.keys_editor();
            let failed = match result {
                Ok(status) if status.success() => None,
                Ok(status) if matches!(status.code(), Some(127 | 9009)) => Some(format!("Couldn't find {command}")),
                Ok(_) => Some(format!("{command} closed with an error")),
                Err(error) => Some(format!("Couldn't open {command}: {error}")),
            };
            if self.load_keys()
                && let Mode::Options(options) = &mut self.mode
            {
                options.message = Some(match failed {
                    Some(failed) => (failed, false),
                    None => ("Keys loaded".into(), true),
                });
            }
            return Ok(());
        }
        let Some(External { slot, before: Some(before), .. }) = self.external.take() else { return Ok(()) };
        let command = self.editor().unwrap_or_default().to_string();
        self.store.notes_edited(slot.day, slot.index)?;
        self.record(before);
        self.back_to_viewer();
        self.message = match result {
            Ok(status) if status.success() => None,
            // What sh (127) and cmd (9009) give for a command they can't find.
            Ok(status) if matches!(status.code(), Some(127 | 9009)) => {
                Some(format!("Couldn't find {command}. Change the editor in the options{}.", self.options_hint()))
            }
            Ok(_) => Some(format!("{command} closed with an error")),
            Err(error) => Some(format!("Couldn't open {command}: {error}. Change the editor in the options{}.", self.options_hint())),
        };
        self.report_conflicts(Some(slot));
        Ok(())
    }

    /// Handles text pasted into the terminal, which arrives in one piece
    /// (bracketed paste) rather than as keys, so it's never run as commands.
    /// Notes keep its lines; single-line inputs get it on one line; anywhere
    /// else it's ignored.
    pub fn handle_paste(&mut self, text: &str) -> io::Result<()> {
        self.clear_message();
        let before = self.slot(self.selected);
        let result = self.paste_text(text);
        self.report_conflicts(before);
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
            Mode::View(viewer) => {
                if let Some(saving) = &mut viewer.saving {
                    saving.input.paste(text);
                }
            }
            Mode::Calendar(calendar) => {
                let time = calendar.picking.as_mut().and_then(|picking| picking.time.as_mut());
                if let Some(input) = calendar.adding.as_mut().or(calendar.command.as_mut()).or(time) {
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
        let timed = match &self.mode {
            Mode::Notes(editor) => editor.flash_ends(),
            Mode::View(viewer) => viewer.flash_ends(),
            Mode::Calendar(calendar) => calendar.snake.as_ref().and_then(|game| game.next_step()),
            Mode::Help { back, .. } => match back.as_ref() {
                Mode::Notes(editor) => editor.flash_ends(),
                _ => None,
            },
            _ => None,
        };
        let list_flash = self.list_flash.as_ref().map(|(since, _)| *since + crate::notes::FLASH);
        // Soon, while :update waits for GitHub's answer.
        let checking = self.update_find.as_ref().map(|_| std::time::Instant::now() + std::time::Duration::from_millis(100));
        [timed, list_flash, checking, self.message_ends].into_iter().flatten().min()
    }

    /// Ends anything timed (a copy's flash) whose time is up at `now`, and
    /// moves the calendar's snake when it's due.
    /// Starts the day's check for a newer version, unless the option is off
    /// or it's been done today. It only finds out: the status bar then says
    /// one is available, and `:update` installs it.
    pub fn start_update_check(&mut self) {
        let today = self.today.format("%Y-%m-%d").to_string();
        if !self.settings.no_update_check && self.settings.update_checked.as_deref() != Some(today.as_str()) {
            self.update_check = Some(update::check(self.fetcher));
        }
    }

    /// A newer version to mention in the status bar, if the option is on.
    pub fn update_available(&self) -> Option<&str> {
        let version = self.settings.update_available.as_deref()?;
        (!self.settings.no_update_check && update::is_newer(version)).then_some(version)
    }

    /// Takes in what the background checks for a newer version found, if
    /// they're done.
    fn receive_updates(&mut self) {
        if let Some(Ok(answer)) = self.update_check.as_ref().map(|check| check.try_recv()) {
            self.update_check = None;
            // Offline, it asks again next time todoro opens.
            if let Ok(version) = answer {
                self.settings.update_checked = Some(self.today.format("%Y-%m-%d").to_string());
                self.settings.update_available = update::is_newer(&version).then_some(version);
                self.save_settings_quietly();
            }
        }
        if let Some(Ok(answer)) = self.update_find.as_ref().map(|find| find.try_recv()) {
            self.update_find = None;
            if !matches!(&self.mode, Mode::Update(popup) if matches!(**popup, update::Popup::Checking)) {
                return;
            }
            self.mode = Mode::Normal;
            match answer {
                Ok(update::Found::UpToDate) => {
                    self.settings.update_available = None;
                    self.save_settings_quietly();
                    self.brief_message(format!("todoro {} is the latest version", changelog::VERSION));
                }
                Ok(update::Found::Newer { version, changelog }) => {
                    self.settings.update_available = Some(version.clone());
                    self.save_settings_quietly();
                    self.mode = Mode::Update(Box::new(update::Popup::newer(version, changelog, (self.fetcher.installed)())));
                }
                Err(error) => self.brief_message(format!("Couldn't check for updates: {error}")),
            }
        }
    }

    /// Saves the settings, if there's a file for them, ignoring a failure:
    /// for what todoro notes down itself, which it can find out again.
    fn save_settings_quietly(&self) {
        if let Some(path) = &self.settings_path {
            let _ = self.settings.save(path);
        }
    }

    pub fn tick(&mut self, now: std::time::Instant) {
        self.receive_updates();
        match &mut self.mode {
            Mode::Notes(editor) => editor.expire_flash(now),
            Mode::View(viewer) => viewer.expire_flash(now),
            Mode::Calendar(calendar) => calendar.tick(now),
            _ => {}
        }
        if self.list_flash.as_ref().is_some_and(|(since, _)| now >= *since + crate::notes::FLASH) {
            self.list_flash = None;
        }
        // A brief message's time starts once it's on screen.
        if self.message_brief {
            match self.message_ends {
                None => self.message_ends = Some(now + BRIEF_MESSAGE),
                Some(ends) if now >= ends => self.clear_message(),
                Some(_) => {}
            }
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
        self.mode = Mode::Help { help: Help::new(&self.keymap), back: Box::new(back) };
    }

    /// Moves "today" on when the date changes while todoro is open, carrying
    /// pinned items over as at startup. It waits until you're back on the
    /// list, so nothing moves while you're typing or in a popup. Undo history
    /// is cleared, so `u` can't undo the carry-over by accident.
    /// Rereads the todos file if something else changed it, such as
    /// `todoro -a` or `-d` in another terminal, so the list shows it and the
    /// next save doesn't overwrite it. Only on the list (`Normal` mode, with
    /// no keys pending), as positions mustn't shift mid-edit; otherwise the
    /// next save keeps the outside version beside it. The same item stays
    /// selected where it can, and undo history is cleared, as undoing would
    /// bring back the old file.
    pub fn reload_if_changed(&mut self) -> io::Result<()> {
        if !matches!(self.mode, Mode::Normal) || !self.pending.is_empty() || self.count.is_some() || !self.store.changed_on_disk() {
            return Ok(());
        }
        let selected = self.slot(self.selected).map(|slot| self.store.items(slot.day)[slot.index].text.clone());
        self.store.reload()?;
        self.undo.clear();
        self.redo.clear();
        let rows = self.items();
        let same = selected.and_then(|text| rows.iter().position(|item| item.text == text));
        if let Some(row) = same {
            self.selected = row;
        }
        self.clamp_selection();
        self.report_conflicts(None);
        Ok(())
    }

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
        self.pending.clear();
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
        type_str(&mut app, " c");
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
        type_str(&mut app, " c");
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
    fn notes_open_ready_to_type_when_empty_and_in_normal_mode_with_text() {
        let (mut app, _dir) = app_with(&["Buy milk", "Call mom"]);
        app.store.set_notes(today(), 1, "Ask about\nSunday".into()).unwrap();
        let typing = |app: &App| match &app.mode {
            Mode::Notes(editor) => editor.insert,
            _ => panic!("the notes screen"),
        };
        press(&mut app, KeyCode::Enter);
        assert!(typing(&app));
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Esc);
        type_str(&mut app, "j");
        press(&mut app, KeyCode::Enter);
        assert!(!typing(&app));
        press(&mut app, KeyCode::Esc);
        // Both the other way round, with the options.
        app.settings.empty_notes_normal = true;
        app.settings.notes_insert = true;
        press(&mut app, KeyCode::Enter);
        assert!(typing(&app));
        // Ready to type at the end.
        type_str(&mut app, " lunch");
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.items()[1].notes, "Ask about\nSunday lunch");
        type_str(&mut app, "k");
        press(&mut app, KeyCode::Enter);
        assert!(!typing(&app));
    }

    #[test]
    fn what_is_typed_on_opening_is_one_undo_step_in_the_notes() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        app.store.set_notes(today(), 0, "Oat".into()).unwrap();
        app.settings.notes_insert = true;
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, " and soy");
        press(&mut app, KeyCode::Esc);
        type_str(&mut app, "u");
        let Mode::Notes(editor) = &app.mode else { panic!("the notes screen") };
        assert_eq!(editor.notes(), "Oat");
    }

    #[test]
    fn notes_are_saved_as_you_type() {
        let (mut app, dir) = app_with(&["one", "two"]);
        type_str(&mut app, "j");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "details");
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
        type_str(&mut app, "note");
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
        type_str(&mut app, "first");
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
        type_str(&mut app, "note");
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
        type_str(&mut app, "unsaved?");
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
        type_str(&mut app, "draft");
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
        type_str(&mut app, "why?");
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
        type_str(&mut app, "note");
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
        type_str(&mut app, "first line");
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
        press(&mut app, KeyCode::Esc);
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
    fn space_c_opens_the_calendar_on_the_day_shown() {
        let (mut app, _dir) = app_with(&["one"]);
        type_str(&mut app, "l c");
        assert_eq!(calendar(&app).cursor, today().succ_opt().unwrap());
        assert_eq!(calendar(&app).today, today());
    }

    #[test]
    fn esc_closes_the_calendar_back_on_the_same_day() {
        let (mut app, _dir) = app_with(&["one", "two"]);
        type_str(&mut app, "j cjjl");
        press(&mut app, KeyCode::Esc);
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.day, today());
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn enter_in_the_calendar_opens_that_day() {
        let (mut app, _dir) = app_with(&["one", "two"]);
        type_str(&mut app, "j cjl");
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
        type_str(&mut app, " cjjaDentist 3pm");
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
        type_str(&mut app, " claplan");
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
    fn u_and_ctrl_r_do_nothing_during_the_calendars_game() {
        let (mut app, _dir) = app_with(&[]);
        let tomorrow = today().succ_opt().unwrap();
        type_str(&mut app, " claplan");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, ":u");
        assert_eq!(texts_on(&app, tomorrow), ["plan"], "u is typed into the command");
        press(&mut app, KeyCode::Esc);
        type_str(&mut app, ":snake");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "u");
        ctrl(&mut app, 'r');
        assert_eq!(texts_on(&app, tomorrow), ["plan"]);
        assert!(calendar(&app).snake.is_some());
    }

    #[test]
    fn the_app_wakes_and_ticks_for_the_calendars_snake() {
        let (mut app, _dir) = app_with(&[]);
        type_str(&mut app, " c:snake");
        press(&mut app, KeyCode::Enter);
        // Waiting for a key, nothing's timed.
        assert_eq!(app.redraw_at(), None);
        type_str(&mut app, "l");
        let start = std::time::Instant::now();
        app.tick(start);
        let at = app.redraw_at().expect("the snake's next move");
        app.tick(at);
        assert_eq!(calendar(&app).snake.as_ref().unwrap().head(), today().succ_opt().unwrap());
    }

    #[test]
    fn pasting_into_the_calendars_command_line() {
        let (mut app, _dir) = app_with(&[]);
        type_str(&mut app, " c:");
        app.handle_paste("snake").unwrap();
        press(&mut app, KeyCode::Enter);
        assert!(calendar(&app).snake.is_some());
    }

    #[test]
    fn u_while_adding_in_the_calendar_is_text() {
        let (mut app, _dir) = app_with(&[]);
        type_str(&mut app, " cau");
        assert_eq!(calendar(&app).adding.as_ref().unwrap().text, "u");
    }

    #[test]
    fn help_from_the_calendar_returns_to_it() {
        let (mut app, _dir) = app_with(&[]);
        type_str(&mut app, " cwl?");
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
    fn space_space_finds_an_item_on_another_day_and_enter_goes_to_it() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        let later = today() + chrono::Duration::days(9);
        app.store.insert(later, 0, "Pay rent".into()).unwrap();
        app.store.insert(later, 1, "Dentist at 3pm".into()).unwrap();
        type_str(&mut app, "  dntst");
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
        type_str(&mut app, "  finished");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.day, later);
        // "pin a" is carried in first, then "open", then the completed one.
        assert_eq!(items(&app), ["pin a", "open", "finished"]);
        assert_eq!(app.selected, 2);
    }

    #[test]
    fn space_s_searches_notes_and_space_space_finds_items() {
        let (mut app, _dir) = app_with(&["one", "two"]);
        app.store.set_notes(today(), 1, "remember the passport".into()).unwrap();
        // Finding items doesn't look in notes, so Enter does nothing.
        type_str(&mut app, "  passport");
        press(&mut app, KeyCode::Enter);
        assert!(matches!(app.mode, Mode::Search(_)));
        press(&mut app, KeyCode::Esc);
        // And searching notes doesn't look at the items' text.
        type_str(&mut app, " sgtwo");
        press(&mut app, KeyCode::Enter);
        assert!(matches!(app.mode, Mode::Search(_)));
        press(&mut app, KeyCode::Esc);
        type_str(&mut app, " sgpassport");
        press(&mut app, KeyCode::Enter);
        assert!(matches!(app.mode, Mode::Notes(_)), "editing its notes");
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn space_space_with_nothing_typed_lists_every_item() {
        let (mut app, _dir) = app_with(&["one", "two"]);
        type_str(&mut app, "  ");
        let Mode::Search(search) = &app.mode else { panic!("finding items") };
        assert_eq!(search.find(&app.store, app.today, app.now).len(), 2);
        // Esc to move, j, Enter goes to the second.
        press(&mut app, KeyCode::Esc);
        type_str(&mut app, "j");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn space_then_a_key_with_no_command_closes_and_does_nothing() {
        let (mut app, _dir) = app_with(&["one", "two"]);
        type_str(&mut app, " ");
        assert!(matches!(app.mode, Mode::Leader(_)));
        type_str(&mut app, "j");
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.selected, 0, "j isn't run as a list key");
        // Esc closes it too.
        type_str(&mut app, " ");
        press(&mut app, KeyCode::Esc);
        assert!(matches!(app.mode, Mode::Normal));
    }

    #[test]
    fn space_commands_open_their_screens() {
        let (mut app, _dir) = crate::test_util::app_with_workspaces(&["Home"]);
        for (keys, opened) in [
            ("  ", "search"),
            (" sg", "search"),
            (" c", "calendar"),
            (" st", "tags"),
            (" o", "options"),
            (" w", "workspaces"),
            (" n", "changelog"),
            (" sk", "help"),
        ] {
            app.mode = Mode::Normal;
            type_str(&mut app, keys);
            let mode = match &app.mode {
                Mode::Search(_) => "search",
                Mode::Calendar(_) => "calendar",
                Mode::Tags(_) => "tags",
                Mode::Options(_) => "options",
                Mode::Workspaces(_) => "workspaces",
                Mode::Changelog(_) => "changelog",
                Mode::Help { .. } => "help",
                _ => "something else",
            };
            assert_eq!(mode, opened, "{keys:?}");
        }
    }

    #[test]
    fn the_old_keys_for_space_commands_do_nothing() {
        let (mut app, _dir) = crate::test_util::app_with_workspaces(&["Home"]);
        // o adds a sub-item now.
        for key in ['s', 'c', '#', 'W', 'N'] {
            press(&mut app, KeyCode::Char(key));
            assert!(matches!(app.mode, Mode::Normal), "{key}");
        }
    }

    #[test]
    fn space_w_with_a_single_todos_file_says_why() {
        let (mut app, _dir) = app_with(&["one"]);
        type_str(&mut app, " w");
        assert!(matches!(app.mode, Mode::Normal));
        assert!(app.message.as_ref().unwrap().contains("TODORO_FILE"));
    }

    #[test]
    fn esc_closes_the_search_where_you_were() {
        let (mut app, _dir) = app_with(&["one", "two"]);
        type_str(&mut app, "jl  one");
        // The first Esc stops typing; the second closes.
        press(&mut app, KeyCode::Esc);
        assert!(matches!(app.mode, Mode::Search(_)));
        press(&mut app, KeyCode::Esc);
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.day, today().succ_opt().unwrap());
    }

    #[test]
    fn keys_typed_in_search_do_not_act_on_the_list() {
        let (mut app, _dir) = app_with(&["one", "two"]);
        type_str(&mut app, " sgxdpjtqu");
        assert!(matches!(app.mode, Mode::Search(_)));
        assert!(app.items().iter().all(|item| !item.done && !item.pinned && item.priority.is_none()));
        assert!(!app.quit);
    }

    #[test]
    fn space_o_opens_options_and_toggling_saves_the_settings() {
        let (mut app, dir) = app_with(&["one"]);
        let path = dir.path().join("settings.json");
        app.settings_path = Some(path.clone());
        type_str(&mut app, " o");
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
        type_str(&mut app, " oxdpta");
        assert!(matches!(app.mode, Mode::Options(_)));
        assert!(app.items().iter().all(|item| !item.done && !item.pinned && item.priority.is_none()));
        assert_eq!(items(&app), ["one", "two"]);
    }

    #[test]
    fn toggling_options_is_not_an_undoable_change() {
        let (mut app, _dir) = app_with(&["one"]);
        type_str(&mut app, "x o ");
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
    fn yy_on_the_list_copies_the_item_and_its_sub_items_to_the_clipboard() {
        let (mut app, _dir) = app_with(&["Buy milk", "Plan the trip"]);
        let day = today();
        app.store.insert_at(day, 2, vec![Item { text: "Book flights".into(), ..Item::default() }], 1).unwrap();
        app.store.insert_at(day, 3, vec![Item { text: "Compare prices".into(), ..Item::default() }], 2).unwrap();
        app.store.insert_at(day, 4, vec![Item { text: "Pack".into(), ..Item::default() }], 1).unwrap();
        app.store.toggle_done(day, 4).unwrap();
        type_str(&mut app, "yy");
        assert_eq!(app.clipboard.copied(), ["Buy milk"]);
        assert_eq!(app.message, None, "no message: the row flashes");
        type_str(&mut app, "jyy");
        assert_eq!(app.clipboard.copied()[1], "Plan the trip\n- Book flights\n  - Compare prices\n- [x] Pack");
        // p still pastes it, sub-items and all.
        type_str(&mut app, "Gp");
        assert_eq!(items(&app).len(), 9);
        // A sub-item on its own is just its text.
        type_str(&mut app, "ggjjyy");
        assert_eq!(app.clipboard.copied()[2], "Book flights\n- Compare prices");
    }

    #[test]
    fn copying_on_the_list_flashes_the_copied_rows_until_its_time_or_a_key() {
        let (mut app, _dir) = app_with(&["Buy milk", "Plan the trip", "Call mom"]);
        app.store.insert_at(today(), 2, vec![Item { text: "Book flights".into(), ..Item::default() }], 1).unwrap();
        type_str(&mut app, "jyy");
        let (since, rows) = app.list_flash.clone().unwrap();
        assert_eq!(rows, [1, 2]);
        let ends = since + crate::notes::FLASH;
        assert_eq!(app.redraw_at(), Some(ends));
        app.tick(ends);
        assert!(app.list_flash.is_none());
        assert_eq!(app.redraw_at(), None);
        type_str(&mut app, "yyj");
        assert!(app.list_flash.is_none());
    }

    #[test]
    fn selected_items_are_copied_to_the_clipboard_with_y() {
        let (mut app, _dir) = app_with(&["Buy milk", "Call mom", "Pay rent"]);
        type_str(&mut app, "Vjy");
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.clipboard.copied(), ["Buy milk\nCall mom"]);
        assert_eq!(app.list_flash.as_ref().map(|(_, rows)| rows.clone()), Some(vec![0, 1]));
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
    fn space_s_t_lists_tags_and_enter_shows_items_with_one() {
        let (mut app, _dir) = app_with(&["Call #work about the #budget", "Water the plants #home"]);
        let later = today() + chrono::Duration::days(5);
        app.store.insert(later, 0, "Send #Work report".into()).unwrap();
        type_str(&mut app, " st");
        assert!(matches!(app.mode, Mode::Tags(_)));
        // Most used first: work (2), then budget and home (1 each).
        press(&mut app, KeyCode::Enter);
        let Mode::Search(search) = &app.mode else { panic!("should be showing the tag's items") };
        assert_eq!(search.tag.as_deref(), Some("work"));
        assert_eq!(search.find(&app.store, today(), app.now).len(), 2);
        // Enter goes to the selected item, the closest to today.
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.day, today());
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn a_tags_items_can_be_narrowed_by_typing() {
        let (mut app, _dir) = app_with(&["Call #work", "Email #work"]);
        type_str(&mut app, " st");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "email");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn esc_or_hash_closes_the_tag_list() {
        let (mut app, _dir) = app_with(&["#a"]);
        type_str(&mut app, " st");
        press(&mut app, KeyCode::Esc);
        assert!(matches!(app.mode, Mode::Normal));
        type_str(&mut app, " st#");
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
        type_str(&mut app, "note");
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
        // Empty, so it opened ready to type.
        press(&mut app, KeyCode::Esc);
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
    fn space_n_shows_every_release() {
        let (mut app, _dir) = app_with(&["one"]);
        type_str(&mut app, " n");
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
        press(&mut app, KeyCode::Esc);
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
        type_str(&mut app, "  ");
        app.handle_paste("gam\n").unwrap();
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.selected, 2);
        type_str(&mut app, ":");
        app.handle_paste("2").unwrap();
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn space_w_switches_workspace_and_keeps_them_separate() {
        let (mut app, _dir) = crate::test_util::app_with_workspaces(&["Home", "Work"]);
        assert_eq!(items(&app), ["Home item"]);
        type_str(&mut app, " w");
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
        type_str(&mut app, " w");
        type_str(&mut app, "k");
        press(&mut app, KeyCode::Enter);
        assert_eq!(items(&app), ["Home item"]);
        assert_eq!(app.settings.workspace.as_deref(), Some("Home"));
    }

    #[test]
    fn undo_never_reaches_into_another_workspace() {
        let (mut app, _dir) = crate::test_util::app_with_workspaces(&["Home", "Work"]);
        type_str(&mut app, "x");
        type_str(&mut app, " wj");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "u");
        assert_eq!(items(&app), ["Work item"]);
        assert!(!app.items()[0].done);
    }

    #[test]
    fn a_in_the_workspaces_popup_creates_one_and_opens_it() {
        let (mut app, dir) = crate::test_util::app_with_workspaces(&["Home"]);
        type_str(&mut app, " wa");
        type_str(&mut app, "Side project");
        press(&mut app, KeyCode::Enter);
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.workspace.as_deref(), Some("Side project"));
        assert!(app.items().is_empty());
        assert!(dir.path().join("todoro").join("Side project").join("todos.json").exists());
        // A name that's taken stays in the popup with a message.
        type_str(&mut app, " wahome");
        press(&mut app, KeyCode::Enter);
        let Mode::Workspaces(picker) = &app.mode else { panic!("still in the popup") };
        assert!(picker.error.as_ref().unwrap().contains("already"));
    }

    #[test]
    fn d_deletes_another_workspace_after_typing_its_name() {
        let (mut app, dir) = crate::test_util::app_with_workspaces(&["Home", "Work"]);
        type_str(&mut app, " wjd");
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
    fn moving_the_folder_from_the_options_moves_every_workspace() {
        let (mut app, dir) = crate::test_util::app_with_workspaces(&["Home", "Work"]);
        let to = dir.path().join("moved here");
        type_str(&mut app, " o");
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
        type_str(&mut app, " o");
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
        type_str(&mut app, "# Oat");
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
        type_str(&mut app, "home notes");
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
        type_str(&mut app, " o");
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
        type_str(&mut app, "j v");
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
            type_str(&mut app, " v");
        }
    }

    #[test]
    fn space_shift_v_views_the_item_and_all_its_sub_items() {
        let (mut app, _dir) = app_with(&["Trip", "Buy milk"]);
        type_str(&mut app, "oFlights");
        press(&mut app, KeyCode::Esc);
        type_str(&mut app, "k V");
        let Mode::View(viewer) = &app.mode else { panic!("the viewer") };
        assert_eq!(viewer.scope, viewer::Scope::Item);
        // Read-only: these don't open the notes.
        type_str(&mut app, "ie");
        press(&mut app, KeyCode::Enter);
        assert!(matches!(app.mode, Mode::View(_)));
        for close in [KeyCode::Esc, KeyCode::Char('q'), KeyCode::Char('V')] {
            press(&mut app, close);
            assert!(matches!(app.mode, Mode::Normal));
            assert_eq!(app.selected, 0);
            type_str(&mut app, " V");
        }
        // Help comes back to the same view.
        type_str(&mut app, "?");
        press(&mut app, KeyCode::Esc);
        assert!(matches!(&app.mode, Mode::View(viewer) if viewer.scope == viewer::Scope::Item));
    }

    #[test]
    fn space_shift_v_rereads_every_notes_file_in_the_block() {
        let (mut app, _dir) = app_with(&["Trip"]);
        type_str(&mut app, "oFlights");
        press(&mut app, KeyCode::Esc);
        app.store.set_notes(today(), 1, "aisle".into()).unwrap();
        std::fs::write(app.store.notes_dir().join("flights.md"), "window").unwrap();
        type_str(&mut app, "k V");
        assert_eq!(app.store.items(today())[1].notes, "window");
    }

    #[test]
    fn space_shift_v_on_an_empty_list_does_nothing() {
        let (mut app, _dir) = app_with(&[]);
        type_str(&mut app, " V");
        assert!(matches!(app.mode, Mode::Normal));
    }

    #[test]
    fn space_e_views_the_whole_day_read_only() {
        let (mut app, _dir) = app_with(&["Trip", "Buy milk"]);
        type_str(&mut app, "j");
        type_str(&mut app, " e");
        let Mode::View(viewer) = &app.mode else { panic!("the viewer") };
        assert_eq!(viewer.scope, viewer::Scope::Day);
        // Read-only: these don't open any notes.
        type_str(&mut app, "ieV");
        press(&mut app, KeyCode::Enter);
        assert!(matches!(app.mode, Mode::View(_)));
        for close in [KeyCode::Esc, KeyCode::Char('q')] {
            press(&mut app, close);
            assert!(matches!(app.mode, Mode::Normal));
            assert_eq!(app.selected, 1, "the selection stays");
            type_str(&mut app, " e");
        }
        // Help comes back to the same view.
        type_str(&mut app, "?");
        press(&mut app, KeyCode::Esc);
        assert!(matches!(&app.mode, Mode::View(viewer) if viewer.scope == viewer::Scope::Day));
    }

    fn saving(app: &App) -> Option<&viewer::Saving> {
        match &app.mode {
            Mode::View(viewer) => viewer.saving.as_ref(),
            _ => None,
        }
    }

    #[test]
    fn w_on_the_day_saves_it_to_the_export_folder_by_default() {
        let (mut app, dir) = app_with(&["Buy milk", "Call mom"]);
        type_str(&mut app, " ew");
        let default = dir.path().join("export").join("2026-10-05.txt");
        assert_eq!(saving(&app).unwrap().default, default);
        press(&mut app, KeyCode::Enter);
        assert_eq!(std::fs::read_to_string(&default).unwrap(), export::day_text(&app));
        assert!(saving(&app).is_none());
        assert!(matches!(&app.mode, Mode::View(_)), "still viewing");
        assert_eq!(app.message.as_deref(), Some(format!("Saved to {}", workspaces::display_path(&default)).as_str()));
    }

    #[test]
    fn saving_over_a_file_asks_first() {
        let (mut app, dir) = app_with(&["Buy milk"]);
        let path = dir.path().join("export").join("2026-10-05.txt");
        export::write(&path, "older\n").unwrap();
        type_str(&mut app, " ew");
        press(&mut app, KeyCode::Enter);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "older\n");
        assert!(saving(&app).unwrap().error.as_ref().unwrap().contains("exists"));
        press(&mut app, KeyCode::Enter);
        assert!(std::fs::read_to_string(&path).unwrap().contains("1. Buy milk"));
        assert!(saving(&app).is_none());
    }

    #[test]
    fn a_typed_path_or_folder_is_saved_to() {
        let (mut app, dir) = app_with(&["Buy milk"]);
        let file = dir.path().join("for sam.txt");
        type_str(&mut app, " ew");
        type_str(&mut app, &file.display().to_string());
        press(&mut app, KeyCode::Enter);
        assert!(std::fs::read_to_string(&file).unwrap().contains("1. Buy milk"));
        // A folder gets the day's file name, and pasting works too.
        let folder = dir.path().join("shared");
        type_str(&mut app, "w");
        app.handle_paste(&format!("{}/", folder.display())).unwrap();
        press(&mut app, KeyCode::Enter);
        assert!(folder.join("2026-10-05.txt").exists());
    }

    #[test]
    fn a_save_that_fails_says_why_and_keeps_the_prompt() {
        let (mut app, dir) = app_with(&["Buy milk"]);
        // A file where its folder should be.
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, "").unwrap();
        type_str(&mut app, " ew");
        type_str(&mut app, &blocker.join("day.txt").display().to_string());
        press(&mut app, KeyCode::Enter);
        assert!(saving(&app).unwrap().error.as_ref().unwrap().starts_with("Couldn't save"));
        press(&mut app, KeyCode::Esc);
        assert!(saving(&app).is_none());
        assert!(matches!(&app.mode, Mode::View(_)));
    }

    #[test]
    fn exports_go_in_the_todoro_folder_named_for_the_workspace_when_there_are_several() {
        let (mut app, dir) = app_with_workspaces(&["home", "work"]);
        type_str(&mut app, " ew");
        assert_eq!(saving(&app).unwrap().default, dir.path().join("todoro").join("export").join("2026-10-05 home.txt"));
        let (mut app, dir) = app_with_workspaces(&["home"]);
        type_str(&mut app, " ew");
        assert_eq!(saving(&app).unwrap().default, dir.path().join("todoro").join("export").join("2026-10-05.txt"));
    }

    #[test]
    fn yy_copies_what_each_view_shows() {
        let (mut app, _dir) = app_with(&["Buy milk", "Plan the trip"]);
        app.store.set_notes(today(), 1, "Lisbon, **May**".into()).unwrap();
        let flashing = |app: &App| matches!(&app.mode, Mode::View(viewer) if viewer.flash.is_some());
        type_str(&mut app, " eyy");
        assert!(flashing(&app));
        assert_eq!(app.message, None);
        press(&mut app, KeyCode::Esc);
        type_str(&mut app, "j Vyy");
        assert!(flashing(&app));
        press(&mut app, KeyCode::Esc);
        type_str(&mut app, " vyy");
        assert!(flashing(&app));
        assert!(matches!(&app.mode, Mode::View(_)), "still viewing");
        let copied = app.clipboard.copied();
        assert_eq!(copied.len(), 3);
        assert_eq!(copied[0], export::day_text(&app));
        assert_eq!(copied[1], "2. Plan the trip\n\nLisbon, **May**\n");
        assert_eq!(copied[2], "Lisbon, **May**");
    }

    #[test]
    fn a_brief_message_goes_after_a_second() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        type_str(&mut app, " v");
        assert_eq!(app.redraw_at(), None);
        type_str(&mut app, "yy");
        assert_eq!(app.message.as_deref(), Some("No notes to copy"));
        let start = std::time::Instant::now();
        app.tick(start);
        assert_eq!(app.redraw_at(), Some(start + BRIEF_MESSAGE));
        app.tick(start + std::time::Duration::from_millis(900));
        assert_eq!(app.message.as_deref(), Some("No notes to copy"));
        app.tick(start + BRIEF_MESSAGE);
        assert_eq!(app.message, None);
        assert_eq!(app.redraw_at(), None);
        // Other messages stay until a key, as before.
        press(&mut app, KeyCode::Esc);
        app.message = Some("Saved".into());
        app.tick(start + std::time::Duration::from_secs(5));
        assert_eq!(app.message.as_deref(), Some("Saved"));
    }

    #[test]
    fn a_key_before_the_second_is_up_clears_it_as_before() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        type_str(&mut app, " vyy");
        let start = std::time::Instant::now();
        app.tick(start);
        type_str(&mut app, "j");
        assert_eq!(app.message, None);
        assert_eq!(app.redraw_at(), None);
    }

    #[test]
    fn copying_notes_that_arent_there_says_so() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        type_str(&mut app, " vyy");
        assert_eq!(app.message.as_deref(), Some("No notes to copy"));
        assert!(app.clipboard.copied().is_empty());
    }

    #[test]
    fn only_the_day_view_saves() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        type_str(&mut app, " Vw");
        assert!(saving(&app).is_none());
        press(&mut app, KeyCode::Esc);
        type_str(&mut app, " vw");
        assert!(saving(&app).is_none());
    }

    #[test]
    fn space_e_rereads_every_notes_file_on_the_day() {
        let (mut app, _dir) = app_with(&["Trip", "Buy milk"]);
        type_str(&mut app, "oFlights");
        press(&mut app, KeyCode::Esc);
        app.store.set_notes(today(), 1, "aisle".into()).unwrap();
        app.store.set_notes(today(), 2, "oat".into()).unwrap();
        std::fs::write(app.store.notes_dir().join("flights.md"), "window").unwrap();
        std::fs::write(app.store.notes_dir().join("buy-milk.md"), "soy").unwrap();
        type_str(&mut app, " e");
        assert_eq!(app.store.items(today())[1].notes, "window");
        assert_eq!(app.store.items(today())[2].notes, "soy");
    }

    #[test]
    fn space_e_opens_on_an_empty_day_too() {
        let (mut app, _dir) = app_with(&[]);
        type_str(&mut app, " e");
        assert!(matches!(app.mode, Mode::View(_)));
        press(&mut app, KeyCode::Esc);
        assert!(matches!(app.mode, Mode::Normal));
    }

    #[test]
    fn v_on_an_empty_list_does_nothing() {
        let (mut app, _dir) = app_with(&[]);
        type_str(&mut app, " v");
        assert!(matches!(app.mode, Mode::Normal));
    }

    #[test]
    fn v_rereads_the_notes_file() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        app.store.set_notes(today(), 0, "oat".into()).unwrap();
        std::fs::write(app.store.notes_dir().join("buy-milk.md"), "soy").unwrap();
        type_str(&mut app, " v");
        assert_eq!(app.items()[0].notes, "soy");
    }

    #[test]
    fn editing_from_the_viewer_comes_back_to_it() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        type_str(&mut app, " vi");
        let Mode::Notes(editor) = &app.mode else { panic!("the notes screen") };
        assert!(editor.insert, "empty, so ready to type, as from the list");
        type_str(&mut app, "oat");
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
        press(&mut app, KeyCode::Esc);
        assert!(matches!(app.mode, Mode::Normal));
    }

    #[test]
    fn editing_from_the_viewer_in_your_own_editor_comes_back_to_it() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        app.settings.editor = Some("nvim".into());
        type_str(&mut app, " ve");
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
        type_str(&mut app, " v?");
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
        type_str(&mut app, " clatrip");
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
    fn an_overdue_item_is_on_todays_list_but_not_the_days_ahead() {
        let (mut app, _dir) = app_with(&["Today's"]);
        let days_ago = today() - Days::new(3);
        app.store.insert(days_ago, 0, "Report".into()).unwrap();
        let due = crate::deadline::Deadline { date: days_ago + Days::new(1), time: None };
        app.store.set_deadline(days_ago, 0, Some(due)).unwrap();
        assert_eq!(items(&app), ["Report", "Today's"]);
        assert!(app.items()[0].is_overdue(today(), app.now));
        type_str(&mut app, "l");
        assert!(items(&app).is_empty(), "overdue, it doesn't follow you ahead");
        // And startup moves it to today, as it does pinned items.
        app.store.roll_over(today()).unwrap();
        assert_eq!(app.store.items(today())[0].text, "Report");
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
        type_str(app, " o");
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
    fn resetting_deletes_empty_workspace_folders_too() {
        let (mut app, _dir) = app_with_workspaces(&["Home"]);
        let folder = app.workspaces.as_ref().unwrap().dir.clone();
        // An empty folder shows as a workspace, though it has no todos file.
        std::fs::create_dir_all(folder.join("Work")).unwrap();
        assert_eq!(app.workspaces.as_ref().unwrap().list().unwrap(), ["Home", "Work"]);
        clear_from_options(&mut app, options::Clear::Reset, "reset");
        assert!(app.restart);
        assert!(!folder.exists(), "nothing was left, so the todoro folder goes too");
    }

    #[test]
    fn resetting_leaves_other_files_in_the_todoro_folder() {
        let (mut app, _dir) = app_with_workspaces(&["Home"]);
        let folder = app.workspaces.as_ref().unwrap().dir.clone();
        std::fs::write(folder.join("readme.txt"), "mine").unwrap();
        std::fs::create_dir_all(folder.join("photos")).unwrap();
        std::fs::write(folder.join("photos").join("cat.jpg"), "meow").unwrap();
        clear_from_options(&mut app, options::Clear::Reset, "reset");
        assert!(app.restart);
        assert!(!folder.join("Home").exists());
        assert!(folder.join("readme.txt").exists());
        assert!(folder.join("photos").join("cat.jpg").exists(), "not a workspace: it has no todos file");
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

    #[test]
    fn enter_in_a_notes_search_opens_them_on_the_line_found() {
        let (mut app, _dir) = app_with(&["Buy milk", "Trip"]);
        let later = today() + chrono::Duration::days(3);
        app.store.insert(later, 0, "Plan".into()).unwrap();
        app.store.set_notes(later, 0, "# Plan\n\nbook the train\nask Sam about dates".into()).unwrap();
        type_str(&mut app, " sgdates");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.day, later);
        let Mode::Notes(editor) = &app.mode else { panic!("editing the notes") };
        assert_eq!(editor.textarea.cursor(), (3, 0), "on the line that matched");
        assert!(!editor.insert, "in normal mode, ready to move or edit");
        // Edits save to that item, and closing goes back to its day's list.
        type_str(&mut app, "A!");
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Esc);
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.items()[0].notes, "# Plan\n\nbook the train\nask Sam about dates!");
        // As one undo step.
        type_str(&mut app, "u");
        assert_eq!(app.items()[0].notes, "# Plan\n\nbook the train\nask Sam about dates");
    }

    #[test]
    fn enter_in_a_notes_search_opens_them_in_your_own_editor() {
        let (mut app, _dir) = app_with(&["Trip"]);
        app.store.set_notes(today(), 0, "ask Sam".into()).unwrap();
        app.settings.editor = Some("nvim".into());
        type_str(&mut app, " sgsam");
        press(&mut app, KeyCode::Enter);
        let (command, path) = app.start_external_edit().unwrap().unwrap();
        assert_eq!(command, "nvim +1", "on the line found");
        assert!(path.ends_with("trip.md"));
    }

    #[test]
    fn enter_when_finding_items_just_goes_to_the_item() {
        let (mut app, _dir) = app_with(&["Buy milk", "Trip"]);
        app.store.set_notes(today(), 1, "ask Sam".into()).unwrap();
        type_str(&mut app, "  trip");
        press(&mut app, KeyCode::Enter);
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.selected, 1);
    }

    /// An app at noon on `today()` with an item overdue since yesterday, one
    /// overdue since 9:00 today, one due later today, and a done one.
    fn app_with_overdue() -> (App, tempfile::TempDir) {
        let (mut app, dir) = app_with(&["Due at nine", "Due tonight", "Done late"]);
        app.now = NaiveTime::from_hms_opt(12, 0, 0).unwrap();
        let yesterday = today().pred_opt().unwrap();
        app.store.insert(yesterday, 0, "Due yesterday".into()).unwrap();
        let at = |date: NaiveDate, h: Option<u32>| Some(crate::deadline::Deadline { date, time: h.map(|h| NaiveTime::from_hms_opt(h, 0, 0).unwrap()) });
        app.store.set_deadline(yesterday, 0, at(yesterday, None)).unwrap();
        app.store.set_deadline(today(), 0, at(today(), Some(9))).unwrap();
        app.store.set_deadline(today(), 1, at(today(), Some(20))).unwrap();
        app.store.set_deadline(today(), 2, at(yesterday, None)).unwrap();
        app.store.toggle_done(today(), 2).unwrap();
        (app, dir)
    }

    #[test]
    fn go_lists_whats_overdue() {
        let (mut app, _dir) = app_with_overdue();
        assert_eq!(app.store.overdue_count(app.today, app.now), 2);
        type_str(&mut app, "go");
        let Mode::Search(search) = &app.mode else { panic!("the overdue items") };
        assert_eq!(search.filter, Some(search::Filter::Overdue));
        let found: Vec<String> = search
            .find(&app.store, app.today, app.now)
            .iter()
            .map(|hit| app.store.items(hit.day)[hit.index].text.clone())
            .collect();
        assert_eq!(found, ["Due at nine", "Due yesterday"]);
        // Typing narrows them down, and Enter goes to one on its day.
        type_str(&mut app, "yest");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.day, today().pred_opt().unwrap());
        assert_eq!(items(&app), ["Due yesterday"]);
    }

    #[test]
    fn colon_overdue_does_the_same_as_go() {
        let (mut app, _dir) = app_with_overdue();
        type_str(&mut app, ":overdue");
        press(&mut app, KeyCode::Enter);
        let Mode::Search(search) = &app.mode else { panic!("the overdue items") };
        assert_eq!(search.filter, Some(search::Filter::Overdue));
    }

    #[test]
    fn g_then_another_key_still_works() {
        let (mut app, _dir) = app_with(&["one", "two", "three"]);
        type_str(&mut app, "Ggg");
        assert_eq!(app.selected, 0);
        assert!(matches!(app.mode, Mode::Normal));
    }

    #[test]
    fn editors_that_take_a_line_get_one() {
        assert_eq!(with_line("nvim", Some(3)), "nvim +4");
        assert_eq!(with_line("vim -p", Some(0)), "vim -p +1");
        assert_eq!(with_line("/usr/bin/nvim", Some(9)), "/usr/bin/nvim +10");
        assert_eq!(with_line("nano", Some(1)), "nano +2");
        // Others don't know +N, so they get the file alone.
        assert_eq!(with_line("code --wait", Some(3)), "code --wait");
        assert_eq!(with_line("hx", Some(3)), "hx");
        // And with no line, nothing is added.
        assert_eq!(with_line("nvim", None), "nvim");
    }

    #[test]
    fn a_notes_search_opens_your_editor_on_the_line_found() {
        let (mut app, _dir) = app_with(&["Trip"]);
        app.store.set_notes(today(), 0, "# Trip\nbook train\nask Sam".into()).unwrap();
        app.settings.editor = Some("nvim".into());
        type_str(&mut app, " sgsam");
        press(&mut app, KeyCode::Enter);
        let (command, _) = app.start_external_edit().unwrap().unwrap();
        assert_eq!(command, "nvim +3");
        app.finish_external_edit(ok()).unwrap();
        // Enter on the list opens it at the top, as before.
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.start_external_edit().unwrap().unwrap().0, "nvim");
    }

    #[test]
    fn e_edits_with_the_cursor_at_the_end() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        type_str(&mut app, "e");
        let Mode::Insert { input, replacing, .. } = &app.mode else { panic!("editing") };
        assert_eq!(input.text, "Buy milk");
        assert_eq!(input.cursor, "Buy milk".len());
        assert!(!replacing);
        type_str(&mut app, " and eggs");
        press(&mut app, KeyCode::Enter);
        assert_eq!(items(&app), ["Buy milk and eggs"]);
    }

    #[test]
    fn capital_e_edits_with_the_text_cleared() {
        let (mut app, _dir) = app_with(&["Buy milk", "Café ☕"]);
        type_str(&mut app, "jE");
        let Mode::Insert { input, index: 1, editing: true, replacing: true, .. } = &app.mode else { panic!("replacing") };
        assert_eq!(input.text, "");
        type_str(&mut app, "Tea ☕");
        press(&mut app, KeyCode::Enter);
        assert_eq!(items(&app), ["Buy milk", "Tea ☕"]);
        // One undo step brings the old text back.
        type_str(&mut app, "u");
        assert_eq!(items(&app), ["Buy milk", "Café ☕"]);
    }

    #[test]
    fn leaving_the_text_empty_after_capital_e_keeps_the_item() {
        for key in [KeyCode::Enter, KeyCode::Esc] {
            let (mut app, _dir) = app_with(&["Buy milk"]);
            type_str(&mut app, "Ex");
            press(&mut app, KeyCode::Backspace);
            press(&mut app, key);
            assert!(matches!(app.mode, Mode::Normal), "no delete popup");
            assert_eq!(items(&app), ["Buy milk"]);
        }
        // Whereas emptying it with e still asks to delete it.
        let (mut app, _dir) = app_with(&["Buy"]);
        type_str(&mut app, "e");
        for _ in 0..3 {
            press(&mut app, KeyCode::Backspace);
        }
        press(&mut app, KeyCode::Enter);
        assert!(matches!(app.mode, Mode::ConfirmDelete { .. }));
    }

    #[test]
    fn capital_e_keeps_the_items_notes_pin_and_deadline() {
        let (mut app, _dir) = app_with(&["Report"]);
        app.store.set_notes(today(), 0, "numbers".into()).unwrap();
        app.store.cycle_priority(today(), 0).unwrap();
        type_str(&mut app, "EQ3 report");
        press(&mut app, KeyCode::Enter);
        let item = &app.items()[0];
        assert_eq!(item.text, "Q3 report");
        assert_eq!(item.notes, "numbers");
        assert!(item.priority.is_some());
    }

    #[test]
    fn capital_e_on_an_empty_list_does_nothing() {
        let (mut app, _dir) = app_with(&[]);
        type_str(&mut app, "E");
        assert!(matches!(app.mode, Mode::Normal));
    }

    fn days_before(amount: u32) -> reminder::Reminder {
        reminder::Reminder::Before { amount, unit: reminder::Unit::Days }
    }

    #[test]
    fn r_sets_a_reminder_some_days_before() {
        let (mut app, _dir) = app_with(&["Buy milk", "Gift"]);
        type_str(&mut app, "jr");
        assert!(matches!(app.mode, Mode::Remind(_)));
        type_str(&mut app, "d3");
        press(&mut app, KeyCode::Enter);
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.items()[1].reminder, Some(days_before(3)));
        assert_eq!(app.selected, 1);
        // One undo step takes it off.
        type_str(&mut app, "u");
        assert_eq!(app.items()[1].reminder, None);
    }

    #[test]
    fn hours_and_months_set_reminders_too_and_x_removes_one() {
        let (mut app, _dir) = app_with(&["Gift"]);
        type_str(&mut app, "rh5");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.items()[0].reminder, Some(reminder::Reminder::Before { amount: 5, unit: reminder::Unit::Hours }));
        type_str(&mut app, "rm2");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.items()[0].reminder, Some(reminder::Reminder::Before { amount: 2, unit: reminder::Unit::Months }));
        type_str(&mut app, "rx");
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.items()[0].reminder, None);
    }

    #[test]
    fn colon_remind_does_the_same_as_r() {
        let (mut app, _dir) = app_with(&["Gift"]);
        type_str(&mut app, ":remind");
        press(&mut app, KeyCode::Enter);
        assert!(matches!(app.mode, Mode::Remind(_)));
    }

    #[test]
    fn r_does_nothing_on_an_empty_day() {
        let (mut app, _dir) = app_with(&[]);
        type_str(&mut app, "r");
        assert!(matches!(app.mode, Mode::Normal));
    }

    #[test]
    fn esc_backs_out_of_a_reminder_without_changing_it() {
        let (mut app, _dir) = app_with(&["Gift"]);
        app.store.set_reminder(today(), 0, Some(days_before(2))).unwrap();
        type_str(&mut app, "rd9");
        press(&mut app, KeyCode::Esc);
        assert!(matches!(app.mode, Mode::Remind(_)), "back to choosing");
        press(&mut app, KeyCode::Esc);
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.items()[0].reminder, Some(days_before(2)));
    }

    #[test]
    fn c_chooses_a_day_to_be_reminded_on_the_calendar() {
        let (mut app, _dir) = app_with(&["Gift"]);
        type_str(&mut app, "rc");
        let Mode::Calendar(calendar) = &app.mode else { panic!("the calendar") };
        assert!(calendar.picking.as_ref().is_some_and(|picking| picking.reminder));
        type_str(&mut app, "l");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "9am");
        press(&mut app, KeyCode::Enter);
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.items()[0].reminder, Some(reminder::Reminder::On(deadline(1, Some((9, 0))))));
        assert_eq!(app.items()[0].deadline, None, "the deadline is left alone");
    }

    #[test]
    fn an_item_shows_on_today_once_its_reminder_is_due() {
        let (mut app, _dir) = app_with(&["Today's"]);
        let later = today() + Days::new(3);
        app.store.insert(later, 0, "Gift".into()).unwrap();
        app.store.set_reminder(later, 0, Some(days_before(5))).unwrap();
        assert_eq!(items(&app), ["Gift", "Today's"], "reminded first, after anything carried");
        assert_eq!(app.carried(), 1);
        // Completing it here completes it on its day, and it goes.
        press(&mut app, KeyCode::Char('x'));
        assert!(app.store.items(later)[0].done);
        assert_eq!(items(&app), ["Today's"]);
    }

    #[test]
    fn setting_a_due_reminder_brings_the_item_to_today_and_follows_it() {
        let (mut app, _dir) = app_with(&["Today's"]);
        let later = today() + Days::new(2);
        app.store.insert(later, 0, "Gift".into()).unwrap();
        type_str(&mut app, "ll");
        assert_eq!(items(&app), ["Gift"]);
        type_str(&mut app, "rd2");
        press(&mut app, KeyCode::Enter);
        // Still on its own day; on today, it now shows first.
        assert_eq!(items(&app), ["Gift"]);
        type_str(&mut app, "t");
        assert_eq!(items(&app), ["Gift", "Today's"]);
    }

    #[test]
    fn space_s_d_and_space_s_r_list_deadlines_and_reminders() {
        let (mut app, _dir) = app_with(&["Report"]);
        type_str(&mut app, " sd");
        let Mode::Search(search) = &app.mode else { panic!("the deadlines") };
        assert_eq!(search.filter, Some(search::Filter::Deadlines));
        assert!(search.typing);
        app.mode = Mode::Normal;
        type_str(&mut app, " sr");
        let Mode::Search(search) = &app.mode else { panic!("the reminders") };
        assert_eq!(search.filter, Some(search::Filter::Reminders));
    }

    #[test]
    fn space_s_o_opens_the_options_ready_to_search() {
        let (mut app, _dir) = app_with(&["Report"]);
        type_str(&mut app, " sotwelve");
        let Mode::Options(options) = &app.mode else { panic!("the options") };
        assert!(options.searching);
        assert_eq!(options.search.as_ref().unwrap().text, "twelve");
        // Space o opens them without searching, so keys move and toggle.
        app.mode = Mode::Normal;
        type_str(&mut app, " o");
        let Mode::Options(options) = &app.mode else { panic!("the options") };
        assert!(!options.searching);
    }

    #[test]
    fn tab_completes_a_tag_being_typed() {
        let (mut app, _dir) = app_with(&["Read #later", "Ship #launch"]);
        type_str(&mut app, "aCall #l");
        assert_eq!(app.tag_suggestions().len(), 2);
        press(&mut app, KeyCode::Tab);
        let Mode::Insert { input, .. } = &app.mode else { panic!("still typing") };
        assert_eq!(input.text, "Call #later");
        assert_eq!(input.cursor, input.text.len());
        assert!(app.tag_suggestions().is_empty(), "typed in full");
        type_str(&mut app, " soon");
        press(&mut app, KeyCode::Enter);
        assert_eq!(items(&app)[1], "Call #later soon");
    }

    #[test]
    fn enter_keeps_a_tag_as_typed() {
        let (mut app, _dir) = app_with(&["Read #later"]);
        type_str(&mut app, "aCall #l");
        press(&mut app, KeyCode::Enter);
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(items(&app)[1], "Call #l");
    }

    #[test]
    fn arrows_choose_which_tag_tab_fills_in() {
        let (mut app, _dir) = app_with(&["#later", "#later", "#launch"]);
        type_str(&mut app, "a#la");
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Down);
        assert_eq!(app.tag_choice, 1, "stops at the last");
        app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL)).unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL)).unwrap();
        press(&mut app, KeyCode::Tab);
        let Mode::Insert { input, .. } = &app.mode else { panic!("still typing") };
        assert_eq!(input.text, "#launch");
    }

    #[test]
    fn tab_does_nothing_without_a_tag_to_complete() {
        let (mut app, _dir) = app_with(&["#later"]);
        type_str(&mut app, "aplain");
        press(&mut app, KeyCode::Tab);
        let Mode::Insert { input, .. } = &app.mode else { panic!("still typing") };
        assert_eq!(input.text, "plain");
        // On the list, Tab does nothing either.
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Tab);
        assert!(matches!(app.mode, Mode::Normal));
    }

    #[test]
    fn tags_complete_in_the_calendar_too() {
        let (mut app, _dir) = app_with(&["Read #later"]);
        type_str(&mut app, " ca#lat");
        press(&mut app, KeyCode::Tab);
        let Mode::Calendar(calendar) = &app.mode else { panic!("the calendar") };
        assert_eq!(calendar.adding.as_ref().unwrap().text, "#later");
    }

    #[test]
    fn space_s_k_shows_the_help_and_esc_comes_back_to_the_list() {
        let (mut app, _dir) = app_with(&["one"]);
        type_str(&mut app, " sk");
        let Mode::Help { back, .. } = &app.mode else { panic!("the help") };
        assert!(matches!(**back, Mode::Normal));
        press(&mut app, KeyCode::Esc);
        assert!(matches!(app.mode, Mode::Normal));
        // ? still does the same.
        type_str(&mut app, "?");
        assert!(matches!(app.mode, Mode::Help { .. }));
    }

    /// An app with these items and `keys` as its keys.toml.
    fn app_with_keys(items: &[&str], keys: &str) -> (App, tempfile::TempDir) {
        let (mut app, dir) = app_with(items);
        let path = dir.path().join("keys.toml");
        std::fs::write(&path, keys).unwrap();
        app.keys_path = Some(path);
        app.load_keys();
        (app, dir)
    }

    #[test]
    fn keys_from_keys_toml_run_their_commands() {
        let (mut app, _dir) = app_with_keys(&["one", "two"], "[list]\n\"c\" = \"calendar\"\n\"x\" = \"none\"\n\"ctrl+d\" = \"toggle-done\"\n");
        assert!(app.message.is_none());
        type_str(&mut app, "x");
        assert!(!app.items()[0].done, "x does nothing now");
        app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL)).unwrap();
        // "one" is done, so it moves below "two".
        assert_eq!(app.store.items(today())[1].text, "one");
        assert!(app.store.items(today())[1].done);
        // Undo still undoes a change made with a new key.
        type_str(&mut app, "u");
        assert!(!app.store.items(today()).iter().any(|item| item.done));
        type_str(&mut app, "c");
        assert!(matches!(app.mode, Mode::Calendar(_)));
    }

    #[test]
    fn the_leader_can_move_and_its_menu_change() {
        let (mut app, _dir) = app_with_keys(&["one"], "leader = \",\"\n[leader-menu]\n\"f\" = \"find-items\"\n\"x\" = \"toggle-done\"\n");
        type_str(&mut app, " ");
        assert!(matches!(app.mode, Mode::Normal), "space isn't the leader now");
        type_str(&mut app, ",");
        assert!(matches!(app.mode, Mode::Leader(_)));
        type_str(&mut app, "f");
        assert!(matches!(app.mode, Mode::Search(_)));
        // A list command from the leader menu is undoable like any change.
        app.mode = Mode::Normal;
        type_str(&mut app, ",x");
        assert!(app.items()[0].done);
        type_str(&mut app, "u");
        assert!(!app.items()[0].done);
    }

    #[test]
    fn counts_and_sequences_work_with_new_keys() {
        let (mut app, _dir) = app_with_keys(&["a", "b", "c", "d", "e"], "[list]\n\"n\" = \"down\"\n\"z z\" = \"last\"\n");
        type_str(&mut app, "3n");
        assert_eq!(app.selected, 3);
        type_str(&mut app, "gg");
        assert_eq!(app.selected, 0, "the defaults stay");
        type_str(&mut app, "zz");
        assert_eq!(app.selected, 4);
        // An unfinished sequence is cancelled by a key that doesn't finish
        // it, which then does its own thing.
        type_str(&mut app, "zk");
        assert_eq!(app.selected, 3);
    }

    #[test]
    fn problems_in_keys_toml_are_reported_and_the_rest_applies() {
        let (app, _dir) = app_with_keys(&["one"], "[list]\n\"x\" = \"explode\"\n\"c\" = \"calendar\"\n\"1\" = \"add\"\n");
        assert_eq!(app.message.as_deref(), Some("keys.toml: [list] \"x\": \"explode\" isn't a command (and 1 more)"));
        assert_eq!(app.keymap.key_for(Command::Calendar).as_deref(), Some("c"));
    }

    #[test]
    fn the_options_open_keys_toml_in_your_editor_and_reload_it() {
        let (mut app, dir) = app_with(&["one"]);
        let path = dir.path().join("config").join("keys.toml");
        app.keys_path = Some(path.clone());
        app.settings.editor = Some("nvim".into());
        type_str(&mut app, " o");
        let Mode::Options(options) = &mut app.mode else { panic!("the options") };
        assert_eq!(options.keys.as_deref(), Some(crate::workspaces::display_path(&path).as_str()));
        options.selected = crate::options::KEYS_ROW;
        press(&mut app, KeyCode::Enter);
        // The first time, the starter file is written, then opened.
        let (command, opened) = app.start_external_edit().unwrap().unwrap();
        assert_eq!((command.as_str(), opened.as_path()), ("nvim", path.as_path()));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), keymap::starter_file());
        // What the editor saved is loaded when it closes.
        std::fs::write(&path, "[list]\n\"c\" = \"calendar\"\n").unwrap();
        app.finish_external_edit(ok()).unwrap();
        let Mode::Options(options) = &app.mode else { panic!("still the options") };
        assert_eq!(options.message, Some(("Keys loaded".into(), true)));
        assert_eq!(app.keymap.key_for(Command::Calendar).as_deref(), Some("c"));
        // A mistake is shown there instead.
        app.keys_edit = KeysEdit::Asked;
        app.start_external_edit().unwrap();
        std::fs::write(&path, "[list]\n\"c\" = \"nope\"\n").unwrap();
        app.finish_external_edit(ok()).unwrap();
        let Mode::Options(options) = &app.mode else { panic!("still the options") };
        assert_eq!(options.message, Some(("keys.toml: [list] \"c\": \"nope\" isn't a command".into(), false)));
    }

    #[test]
    fn without_somewhere_for_keys_toml_the_options_say_so() {
        let (mut app, _dir) = app_with(&["one"]);
        type_str(&mut app, " o");
        let Mode::Options(options) = &mut app.mode else { panic!("the options") };
        options.selected = crate::options::KEYS_ROW;
        press(&mut app, KeyCode::Enter);
        assert!(app.start_external_edit().unwrap().is_none());
        let Mode::Options(options) = &app.mode else { panic!("still the options") };
        assert!(!options.message.as_ref().unwrap().1);
    }

    /// The rows on screen as an outline: two spaces a level, `x ` if done,
    /// `+` after a folded item.
    fn shown(app: &App) -> Vec<String> {
        app.items()
            .iter()
            .map(|item| {
                let fold = if item.folded { "+" } else { "" };
                format!("{}{}{}{fold}", "  ".repeat(item.depth), if item.done { "x " } else { "" }, item.text)
            })
            .collect()
    }

    /// An app with "trip" (flights, hotel) and "milk" today.
    fn app_with_trip() -> (App, tempfile::TempDir) {
        let (mut app, dir) = app_with(&["trip", "milk"]);
        type_str(&mut app, "oflights");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "ohotel");
        press(&mut app, KeyCode::Esc);
        (app, dir)
    }

    #[test]
    fn o_adds_a_sub_item_under_the_selected_item() {
        let (mut app, _dir) = app_with_trip();
        // o on flights made hotel a sub-item of flights.
        assert_eq!(shown(&app), ["trip", "  flights", "    hotel", "milk"]);
        assert_eq!(app.selected, 2, "on the new one");
        // One undo step each.
        type_str(&mut app, "u");
        assert_eq!(shown(&app), ["trip", "  flights", "milk"]);
    }

    #[test]
    fn a_adds_below_an_item_and_what_is_under_it_at_its_level() {
        let (mut app, _dir) = app_with_trip();
        type_str(&mut app, "<");
        assert_eq!(shown(&app), ["trip", "  flights", "  hotel", "milk"]);
        type_str(&mut app, "kapacking");
        press(&mut app, KeyCode::Enter);
        assert_eq!(shown(&app), ["trip", "  flights", "  packing", "  hotel", "milk"]);
        // On a top-level item with sub-items, after all of them.
        type_str(&mut app, "ggaafter");
        press(&mut app, KeyCode::Enter);
        assert_eq!(shown(&app), ["trip", "  flights", "  packing", "  hotel", "after", "milk"]);
    }

    #[test]
    fn capital_a_keeps_adding_at_the_same_level() {
        let (mut app, _dir) = app_with(&["trip"]);
        type_str(&mut app, "oflights");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "Ahotel");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "car");
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter);
        assert_eq!(shown(&app), ["trip", "  flights", "  hotel", "  car"]);
    }

    #[test]
    fn indent_and_outdent_with_the_angle_brackets() {
        let (mut app, _dir) = app_with(&["trip", "flights", "hotel"]);
        type_str(&mut app, "j>j>");
        assert_eq!(shown(&app), ["trip", "  flights", "  hotel"]);
        type_str(&mut app, ">");
        assert_eq!(shown(&app), ["trip", "  flights", "    hotel"]);
        assert_eq!(app.selected, 2, "it stays selected");
        // Nothing above at its level: says so.
        type_str(&mut app, "gg>");
        assert!(app.message.is_some());
        type_str(&mut app, "G<<");
        assert_eq!(shown(&app), ["trip", "  flights", "hotel"]);
        assert_eq!(app.selected, 2);
    }

    #[test]
    fn x_on_an_item_completes_what_is_under_it_and_reopening_one_reopens_it() {
        let (mut app, _dir) = app_with_trip();
        type_str(&mut app, "ggx");
        assert_eq!(shown(&app), ["milk", "x trip", "  x flights", "    x hotel"]);
        type_str(&mut app, "Gx");
        assert_eq!(shown(&app), ["milk", "trip", "  flights", "    hotel"]);
    }

    #[test]
    fn numbers_count_top_level_items_only() {
        let (mut app, _dir) = app_with_trip();
        type_str(&mut app, "2G");
        assert_eq!(items(&app)[app.selected], "milk");
        type_str(&mut app, ":1");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn folding_hides_sub_items_and_zo_zc_za_change_it() {
        let (mut app, _dir) = app_with_trip();
        type_str(&mut app, "ggza");
        assert_eq!(shown(&app), ["trip+", "milk"]);
        type_str(&mut app, "zo");
        assert_eq!(shown(&app), ["trip", "  flights", "    hotel", "milk"]);
        // On an item with nothing under it, its parent folds.
        type_str(&mut app, "jjzc");
        assert_eq!(shown(&app), ["trip", "  flights+", "milk"]);
        assert_eq!(app.selected, 1, "on what folded");
        // Adding a sub-item to a folded item unfolds it.
        type_str(&mut app, "opack");
        press(&mut app, KeyCode::Enter);
        assert_eq!(shown(&app), ["trip", "  flights", "    hotel", "    pack", "milk"]);
    }

    #[test]
    fn j_and_k_move_through_the_rows_and_capital_j_k_among_siblings() {
        let (mut app, _dir) = app_with(&["trip", "flights", "hotel", "milk"]);
        type_str(&mut app, "j>j>");
        assert_eq!(shown(&app), ["trip", "  flights", "  hotel", "milk"]);
        type_str(&mut app, "kJ");
        assert_eq!(shown(&app), ["trip", "  hotel", "  flights", "milk"]);
        assert_eq!(app.selected, 2);
        type_str(&mut app, "J");
        assert_eq!(shown(&app), ["trip", "  hotel", "  flights", "milk"], "not past the last under trip");
        type_str(&mut app, "ggJ");
        assert_eq!(shown(&app), ["milk", "trip", "  hotel", "  flights"]);
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn copying_and_deleting_take_the_sub_items_along() {
        let (mut app, _dir) = app_with_trip();
        type_str(&mut app, "ggyyGp");
        assert_eq!(shown(&app), ["trip", "  flights", "    hotel", "milk", "trip", "  flights", "    hotel"]);
        type_str(&mut app, "jd");
        press(&mut app, KeyCode::Char('d'));
        assert_eq!(shown(&app), ["trip", "  flights", "    hotel", "milk", "trip"]);
        // Pasting under a sub-item pastes at its level.
        type_str(&mut app, "ggjp");
        assert_eq!(shown(&app), ["trip", "  flights", "    hotel", "  flights", "    hotel", "milk", "trip"]);
    }

    #[test]
    fn pinning_a_sub_item_says_to_pin_its_parent() {
        let (mut app, _dir) = app_with_trip();
        // On hotel, under flights.
        type_str(&mut app, "m");
        assert!(app.message.as_ref().unwrap().contains("pin that instead"));
        assert!(!app.items()[2].pinned);
    }

    #[test]
    fn search_shows_a_sub_item_folded_away() {
        let (mut app, _dir) = app_with_trip();
        type_str(&mut app, "ggza");
        type_str(&mut app, "  hotel");
        press(&mut app, KeyCode::Enter);
        assert_eq!(shown(&app), ["trip", "  flights", "    hotel", "milk"]);
        assert_eq!(items(&app)[app.selected], "hotel");
    }

    #[test]
    fn sub_items_follow_a_carried_item_and_o_adds_to_its_day() {
        let (mut app, _dir) = app_with(&[]);
        let yesterday = today() - Days::new(1);
        app.store.insert(yesterday, 0, "trip".into()).unwrap();
        app.store.toggle_pinned(yesterday, 0).unwrap();
        type_str(&mut app, "oflights");
        press(&mut app, KeyCode::Enter);
        assert_eq!(shown(&app), ["trip", "  flights"]);
        assert_eq!(app.store.items(yesterday).len(), 2, "stored with its parent");
    }

    #[test]
    fn selecting_a_parent_and_its_sub_items_acts_on_each_once() {
        let (mut app, _dir) = app_with_trip();
        type_str(&mut app, "ggVjjy");
        assert_eq!(app.register.iter().map(|item| (item.text.as_str(), item.depth)).collect::<Vec<_>>(), [("trip", 0), ("flights", 1), ("hotel", 2)]);
        type_str(&mut app, "ggVjd");
        press(&mut app, KeyCode::Char('d'));
        assert_eq!(shown(&app), ["milk"]);
        assert_eq!(app.register.len(), 3, "the whole block, once");
        type_str(&mut app, "u");
        type_str(&mut app, "ggVjx");
        assert_eq!(shown(&app), ["milk", "x trip", "  x flights", "    x hotel"]);
    }

    #[test]
    fn capital_o_adds_several_sub_items_until_esc_or_an_empty_line() {
        let (mut app, _dir) = app_with(&["trip", "milk"]);
        type_str(&mut app, "Oflights");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "hotel");
        press(&mut app, KeyCode::Enter);
        assert!(matches!(app.mode, Mode::Insert { repeat: true, .. }), "ready for the next");
        press(&mut app, KeyCode::Enter);
        assert!(matches!(app.mode, Mode::Normal), "an empty line stops");
        assert_eq!(shown(&app), ["trip", "  flights", "  hotel", "milk"]);
        // On a sub-item, under that one; Esc saves and stops.
        type_str(&mut app, "Ocompare");
        press(&mut app, KeyCode::Esc);
        assert_eq!(shown(&app), ["trip", "  flights", "  hotel", "    compare", "milk"]);
        assert!(matches!(app.mode, Mode::Normal));
    }

    #[test]
    fn shift_enter_folds_and_unfolds_and_enter_still_opens_notes() {
        let (mut app, _dir) = app_with_trip();
        let shift_enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT);
        type_str(&mut app, "gg");
        app.handle_key(shift_enter).unwrap();
        assert_eq!(shown(&app), ["trip+", "milk"]);
        app.handle_key(shift_enter).unwrap();
        assert_eq!(shown(&app), ["trip", "  flights", "    hotel", "milk"]);
        press(&mut app, KeyCode::Enter);
        assert!(matches!(app.mode, Mode::Notes(_)));
        // Shift with a letter is just the capital.
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Esc);
        app.handle_key(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT)).unwrap();
        assert_eq!(items(&app)[app.selected], "milk");
    }

    fn time_of(app: &App, row: usize) -> Option<String> {
        app.items()[row].time.map(|time| time.describe(false))
    }

    fn plan_typing_state(app: &App) -> Option<&'static str> {
        match &app.mode {
            Mode::Plan { plan, .. } => match &plan.typing {
                Some(Typing::Start(_)) => Some("start"),
                Some(Typing::End(..)) => Some("end"),
                None => None,
            },
            _ => None,
        }
    }

    #[test]
    fn planning_types_a_start_then_an_end_and_goes_on_to_the_next_item() {
        let (mut app, _dir) = app_with(&["Standup", "Report", "Gym"]);
        type_str(&mut app, " p");
        assert!(matches!(app.mode, Mode::Plan { .. }));
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "9");
        press(&mut app, KeyCode::Enter);
        assert_eq!(plan_typing_state(&app), Some("end"));
        type_str(&mut app, "9:30");
        press(&mut app, KeyCode::Enter);
        assert_eq!(time_of(&app, 0).as_deref(), Some("09:00–09:30"));
        // On to the next item, ready for its start.
        assert_eq!(app.selected, 1);
        assert_eq!(plan_typing_state(&app), Some("start"));
        // Enter for the end gives it an hour, on either clock.
        type_str(&mut app, "1:15pm");
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter);
        assert_eq!(time_of(&app, 1).as_deref(), Some("13:15–14:15"));
        // Tab leaves it as it is; after the last, it stops.
        press(&mut app, KeyCode::Tab);
        assert_eq!(time_of(&app, 2), None);
        assert_eq!(plan_typing_state(&app), None);
        assert!(app.message.as_ref().unwrap().contains("w saves"));
        // The day planned is the one on screen.
        assert_eq!(app.items()[0].time.unwrap().date, today());
    }

    #[test]
    fn a_time_that_isnt_one_or_ends_too_soon_says_so() {
        let (mut app, _dir) = app_with(&["Standup"]);
        type_str(&mut app, " p");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "lunch");
        press(&mut app, KeyCode::Enter);
        let Mode::Plan { plan, .. } = &app.mode else { panic!("planning") };
        assert!(plan.error.is_some());
        assert_eq!(plan_typing_state(&app), Some("start"));
        // Typing again clears it.
        press(&mut app, KeyCode::Backspace);
        let Mode::Plan { plan, .. } = &app.mode else { panic!("planning") };
        assert!(plan.error.is_none());
        for _ in 0..4 {
            press(&mut app, KeyCode::Backspace);
        }
        type_str(&mut app, "10");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "9");
        press(&mut app, KeyCode::Enter);
        let Mode::Plan { plan, .. } = &app.mode else { panic!("planning") };
        assert!(plan.error.as_ref().unwrap().contains("end after it starts"));
        assert_eq!(time_of(&app, 0), None);
        // Esc backs out of this item.
        press(&mut app, KeyCode::Esc);
        assert_eq!(plan_typing_state(&app), None);
        assert!(matches!(app.mode, Mode::Plan { .. }));
    }

    #[test]
    fn saving_a_plan_is_one_change_to_undo() {
        let (mut app, _dir) = app_with(&["Standup", "Report"]);
        type_str(&mut app, " p");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "9");
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "10");
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "w");
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(time_of(&app, 0).as_deref(), Some("09:00–10:00"));
        assert_eq!(time_of(&app, 1).as_deref(), Some("10:00–11:00"));
        type_str(&mut app, "u");
        assert_eq!((time_of(&app, 0), time_of(&app, 1)), (None, None));
    }

    #[test]
    fn reset_clears_every_time_until_saved() {
        let (mut app, _dir) = planning_app();
        type_str(&mut app, "R");
        assert_eq!((time_of(&app, 0), time_of(&app, 1), time_of(&app, 2)), (None, None, None));
        assert!(matches!(app.mode, Mode::Plan { .. }), "still planning");
        // The next start is now again, not after a cleared time.
        let Mode::Plan { plan, .. } = &app.mode else { panic!("planning") };
        assert_eq!(plan.last_end, None);
        // Not kept unless saved: q, q puts them back.
        type_str(&mut app, "qq");
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(time_of(&app, 0).as_deref(), Some("09:00–09:30"));
        // Saved, it's one change to undo.
        type_str(&mut app, " pRw");
        assert_eq!(time_of(&app, 1), None);
        type_str(&mut app, "u");
        assert_eq!(time_of(&app, 1).as_deref(), Some("18:00–19:00"));
    }

    #[test]
    fn cancelling_with_changes_asks_first_and_puts_them_back() {
        let (mut app, _dir) = app_with(&["Standup"]);
        type_str(&mut app, " p");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "9");
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Esc);
        let Mode::Plan { plan, .. } = &app.mode else { panic!("asking") };
        assert!(plan.confirm_cancel);
        type_str(&mut app, "n");
        assert!(matches!(&app.mode, Mode::Plan { plan, .. } if !plan.confirm_cancel), "kept planning");
        // q twice: leave without saving.
        type_str(&mut app, "qq");
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(time_of(&app, 0), None);
        // With nothing changed, Esc just leaves.
        type_str(&mut app, " p");
        press(&mut app, KeyCode::Esc);
        assert!(matches!(app.mode, Mode::Normal));
    }

    #[test]
    fn only_open_top_level_items_get_times() {
        let (mut app, _dir) = app_with_trip();
        // On hotel, a sub-item: the plan starts on trip, what it's under.
        type_str(&mut app, " p");
        assert_eq!(app.selected, 0);
        let (mut app, _dir) = app_with(&["done"]);
        type_str(&mut app, "x p");
        assert!(matches!(app.mode, Mode::Normal));
        assert!(app.message.as_ref().unwrap().contains("nothing open to plan"));
    }

    #[test]
    fn the_timeline_view_moves_between_days_and_opens_the_plan() {
        let (mut app, _dir) = app_with(&["Standup"]);
        type_str(&mut app, " t");
        assert!(matches!(app.mode, Mode::Timeline { scroll: None }));
        type_str(&mut app, "l");
        assert_eq!(app.day, today() + Days::new(1));
        type_str(&mut app, "t");
        assert_eq!(app.day, today());
        type_str(&mut app, "j");
        assert!(matches!(app.mode, Mode::Timeline { scroll: Some(1) }));
        type_str(&mut app, "p");
        assert!(matches!(app.mode, Mode::Plan { .. }));
        press(&mut app, KeyCode::Esc);
        type_str(&mut app, " t");
        press(&mut app, KeyCode::Esc);
        assert!(matches!(app.mode, Mode::Normal));
    }

    #[test]
    fn enter_with_no_start_typed_starts_now() {
        let (mut app, _dir) = app_with(&["Standup"]);
        app.now = NaiveTime::from_hms_opt(14, 7, 42).unwrap();
        type_str(&mut app, " p");
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter);
        assert_eq!(time_of(&app, 0).as_deref(), Some("14:07–15:07"), "now, to the minute, for an hour");
    }

    /// An app planning today: Standup 9:00, Gym 18:00, Report 9:15, and
    /// Buy milk with no time, listed in that order.
    fn planning_app() -> (App, tempfile::TempDir) {
        let (mut app, dir) = app_with(&["Standup", "Gym", "Report", "Buy milk"]);
        let at = |h: u32, m: u32| NaiveTime::from_hms_opt(h, m, 0).unwrap();
        for (i, start, end) in [(0, at(9, 0), at(9, 30)), (1, at(18, 0), at(19, 0)), (2, at(9, 15), at(11, 0))] {
            app.store.set_time(today(), i, Some(TimeSlot { date: today(), start, end })).unwrap();
        }
        type_str(&mut app, " p");
        (app, dir)
    }

    #[test]
    fn capital_l_moves_into_the_timeline_where_j_and_k_go_from_block_to_block() {
        let (mut app, _dir) = planning_app();
        type_str(&mut app, "L");
        assert!(matches!(&app.mode, Mode::Plan { plan, .. } if plan.on_timeline));
        assert_eq!(app.timeline_blocks(), [0, 2, 1], "by start, without Buy milk");
        // From Standup, down its column to Gym; Report overlaps Standup, so
        // it's in the column beside it.
        type_str(&mut app, "j");
        assert_eq!(items(&app)[app.selected], "Gym");
        type_str(&mut app, "j");
        assert_eq!(items(&app)[app.selected], "Gym", "the last");
        type_str(&mut app, "kl");
        assert_eq!(items(&app)[app.selected], "Report");
        type_str(&mut app, "gk");
        assert_eq!(items(&app)[app.selected], "Standup", "the all-day Buy milk isn't one to choose");
        // Retiming a block stays on it, rather than going on down the list.
        type_str(&mut app, "G");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "20");
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter);
        assert_eq!(time_of(&app, 1).as_deref(), Some("20:00–21:00"));
        assert_eq!(items(&app)[app.selected], "Gym");
        assert_eq!(plan_typing_state(&app), None);
        // H goes back to the list, where j and k go down and up it.
        type_str(&mut app, "Hggj");
        assert_eq!(items(&app)[app.selected], "Gym");
        assert!(matches!(&app.mode, Mode::Plan { plan, .. } if !plan.on_timeline));
    }

    #[test]
    fn capital_l_without_room_for_the_timeline_says_so() {
        let (mut app, _dir) = planning_app();
        app.timeline_shown.set(false);
        type_str(&mut app, "L");
        assert!(matches!(&app.mode, Mode::Plan { plan, .. } if !plan.on_timeline));
        assert!(app.message.as_ref().unwrap().contains("Widen the terminal"));
    }

    #[test]
    fn capital_l_lands_on_a_block_and_clearing_one_moves_off_it() {
        let (mut app, _dir) = planning_app();
        // From Buy milk, all day, L goes to a block.
        type_str(&mut app, "GL");
        assert!(app.timeline_blocks().contains(&app.selected));
        // Clearing Gym's time makes it all day: on to another block.
        type_str(&mut app, "G");
        assert_eq!(items(&app)[app.selected], "Gym");
        type_str(&mut app, "c");
        assert_eq!(time_of(&app, 1), None);
        assert!(app.timeline_blocks().contains(&app.selected));
        // Clearing them all goes back to the list.
        type_str(&mut app, "cc");
        assert!(app.timeline_blocks().is_empty());
        assert!(matches!(&app.mode, Mode::Plan { plan, .. } if !plan.on_timeline));
        // And with no blocks, L says so.
        type_str(&mut app, "L");
        assert!(matches!(&app.mode, Mode::Plan { plan, .. } if !plan.on_timeline));
        assert!(app.message.as_ref().unwrap().contains("Nothing's on the timeline"));
    }

    #[test]
    fn on_the_timeline_j_and_k_go_down_a_column_and_h_and_l_across() {
        // 13:00–14:00 and 14:00–15:00 stack in the first column; 13:30–19:00
        // overlaps both, so it's in the second.
        let (mut app, _dir) = app_with(&["First", "Second", "Long", "Evening"]);
        let at = |h: u32, m: u32| NaiveTime::from_hms_opt(h, m, 0).unwrap();
        for (i, start, end) in [(0, at(13, 0), at(14, 0)), (1, at(14, 0), at(15, 0)), (2, at(13, 30), at(19, 0)), (3, at(20, 0), at(21, 0))] {
            app.store.set_time(today(), i, Some(TimeSlot { date: today(), start, end })).unwrap();
        }
        type_str(&mut app, " pL");
        assert_eq!(items(&app)[app.selected], "First");
        type_str(&mut app, "j");
        assert_eq!(items(&app)[app.selected], "Second", "down the column, past Long beside it");
        type_str(&mut app, "j");
        assert_eq!(items(&app)[app.selected], "Evening", "on down the first column");
        type_str(&mut app, "j");
        assert_eq!(items(&app)[app.selected], "Evening", "the last in it");
        type_str(&mut app, "kk");
        assert_eq!(items(&app)[app.selected], "First");
        type_str(&mut app, "l");
        assert_eq!(items(&app)[app.selected], "Long");
        type_str(&mut app, "l");
        assert_eq!(items(&app)[app.selected], "Long", "no column further right");
        type_str(&mut app, "jk");
        assert_eq!(items(&app)[app.selected], "Long", "alone in its column");
        // Left from Long: the block beside it that overlaps it most, an
        // hour of Second's against half an hour of First's.
        type_str(&mut app, "h");
        assert_eq!(items(&app)[app.selected], "Second");
        // Left from the first column stays there; H goes back to the list.
        type_str(&mut app, "h");
        assert!(matches!(&app.mode, Mode::Plan { plan, .. } if plan.on_timeline));
        assert_eq!(items(&app)[app.selected], "Second");
        type_str(&mut app, "H");
        assert!(matches!(&app.mode, Mode::Plan { plan, .. } if !plan.on_timeline));
    }

    #[test]
    fn q_with_changes_asks_and_w_there_saves() {
        let (mut app, _dir) = app_with(&["Standup"]);
        type_str(&mut app, " p");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "9");
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "q");
        assert!(matches!(&app.mode, Mode::Plan { plan, .. } if plan.confirm_cancel));
        type_str(&mut app, "w");
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(time_of(&app, 0).as_deref(), Some("09:00–10:00"));
    }

    #[test]
    fn the_next_items_start_is_where_the_last_one_ended() {
        let (mut app, _dir) = app_with(&["Standup", "Report", "Lunch"]);
        app.now = NaiveTime::from_hms_opt(8, 50, 0).unwrap();
        type_str(&mut app, " p");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "9");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "9:30");
        press(&mut app, KeyCode::Enter);
        // On to Report: Enter, Enter is 9:30 for an hour.
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter);
        assert_eq!(time_of(&app, 1).as_deref(), Some("09:30–10:30"));
        // And Lunch after it, ending when typed.
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "11");
        press(&mut app, KeyCode::Enter);
        assert_eq!(time_of(&app, 2).as_deref(), Some("10:30–11:00"));
    }

    #[test]
    fn bang_triages_in_the_plan_and_is_kept_only_if_saved() {
        let (mut app, _dir) = planning_app();
        type_str(&mut app, "!");
        assert_eq!(app.items()[0].priority, Some(Priority::High));
        type_str(&mut app, "L!");
        assert_eq!(app.items()[0].priority, Some(Priority::Medium), "on the timeline too");
        type_str(&mut app, "qq");
        assert_eq!(app.items()[0].priority, None, "left without saving");
        type_str(&mut app, " p!w");
        assert_eq!(app.items()[0].priority, Some(Priority::High));
    }

    #[test]
    fn after_a_time_it_goes_on_only_to_items_without_one() {
        // Standup 9:00, Gym 18:00 and Report 9:15 have times; Buy milk doesn't.
        let (mut app, _dir) = planning_app();
        app.store.insert(today(), 4, "Call mum".into()).unwrap();
        // Retime Standup: on to Buy milk, past Gym and Report.
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "8");
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter);
        assert_eq!(items(&app)[app.selected], "Buy milk");
        assert_eq!(plan_typing_state(&app), Some("start"));
        // Tab skips it, on to Call mum, the last without a time.
        press(&mut app, KeyCode::Tab);
        assert_eq!(items(&app)[app.selected], "Call mum");
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter);
        // Then round from the top to Buy milk, which still has none.
        assert_eq!(items(&app)[app.selected], "Buy milk");
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter);
        // Now everything has a time: it stops, rather than asking again.
        assert_eq!(plan_typing_state(&app), None);
        assert!(app.message.as_ref().unwrap().contains("Every item has a time"));
    }

    fn in_plan(app: &App) -> bool {
        matches!(app.mode, Mode::Plan { .. })
    }

    #[test]
    fn a_adds_an_item_in_the_plan_and_comes_back_to_it() {
        let (mut app, _dir) = planning_app();
        type_str(&mut app, "aDentist");
        assert!(matches!(app.mode, Mode::Insert { .. }));
        press(&mut app, KeyCode::Enter);
        assert!(in_plan(&app));
        assert_eq!(items(&app), ["Standup", "Dentist", "Gym", "Report", "Buy milk"]);
        assert_eq!(items(&app)[app.selected], "Dentist", "on the new one, ready to plan it");
        // A keeps adding until an empty line, then back to the plan.
        type_str(&mut app, "AOne");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "Two");
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter);
        assert!(in_plan(&app));
        assert_eq!(items(&app)[2..4], ["One", "Two"]);
    }

    #[test]
    fn e_and_capital_e_edit_and_d_deletes_in_the_plan() {
        let (mut app, _dir) = planning_app();
        type_str(&mut app, "e!");
        press(&mut app, KeyCode::Enter);
        assert!(in_plan(&app));
        assert_eq!(items(&app)[0], "Standup!");
        type_str(&mut app, "EDaily");
        press(&mut app, KeyCode::Enter);
        assert_eq!(items(&app)[0], "Daily");
        assert_eq!(time_of(&app, 0).as_deref(), Some("09:00–09:30"), "its time stays");
        // d asks, as on the list, then back to the plan; c keeps it.
        type_str(&mut app, "d");
        assert!(matches!(app.mode, Mode::ConfirmDelete { .. }));
        type_str(&mut app, "c");
        assert!(in_plan(&app));
        assert_eq!(items(&app).len(), 4);
        type_str(&mut app, "dd");
        assert!(in_plan(&app));
        assert_eq!(items(&app), ["Gym", "Report", "Buy milk"]);
    }

    #[test]
    fn edits_in_the_plan_are_kept_or_put_back_with_it() {
        let (mut app, _dir) = planning_app();
        type_str(&mut app, "aDentist");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "Gdd");
        // Leaving without saving puts back both.
        type_str(&mut app, "qq");
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(items(&app), ["Standup", "Gym", "Report", "Buy milk"]);
        // Saved, they're one change for u.
        type_str(&mut app, " paDentist");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "w");
        assert_eq!(items(&app).len(), 5);
        type_str(&mut app, "u");
        assert_eq!(items(&app), ["Standup", "Gym", "Report", "Buy milk"]);
    }

    #[test]
    fn o_and_capital_o_add_sub_items_in_the_plan() {
        let (mut app, _dir) = planning_app();
        type_str(&mut app, "oagenda");
        press(&mut app, KeyCode::Enter);
        assert!(in_plan(&app));
        assert_eq!(shown(&app)[..2], ["Standup", "  agenda"]);
        // O keeps adding under the same item until an empty line.
        type_str(&mut app, "ggOslides");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "notes");
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter);
        assert!(in_plan(&app));
        assert_eq!(shown(&app)[..4], ["Standup", "  agenda", "  slides", "  notes"]);
        // Sub-items don't get a time of their own: Enter on one plans its parent.
        type_str(&mut app, "k");
        assert_eq!(items(&app)[app.selected], "slides");
        press(&mut app, KeyCode::Enter);
        assert_eq!(items(&app)[app.selected], "Standup");
    }

    #[test]
    fn completed_items_dont_show_when_planning_or_on_the_timeline() {
        let (mut app, _dir) = app_with(&["Standup", "Done already", "Report"]);
        type_str(&mut app, "jx");
        type_str(&mut app, "ggoagenda");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "oold");
        press(&mut app, KeyCode::Esc);
        type_str(&mut app, "x");
        // On the list, the completed ones still show.
        assert_eq!(shown(&app), ["Standup", "  agenda", "    x old", "Report", "x Done already"]);
        type_str(&mut app, " p");
        assert_eq!(shown(&app), ["Standup", "  agenda", "Report"]);
        type_str(&mut app, "G");
        assert_eq!(items(&app)[app.selected], "Report", "the last is the last open one");
        press(&mut app, KeyCode::Esc);
        type_str(&mut app, " t");
        assert_eq!(shown(&app), ["Standup", "  agenda", "Report"]);
        press(&mut app, KeyCode::Esc);
        assert_eq!(shown(&app).len(), 5, "back on the list, all of them");
    }

    #[test]
    fn u_and_ctrl_r_undo_and_redo_in_the_plan() {
        let (mut app, _dir) = planning_app();
        // Delete Standup, then clear Gym's time.
        type_str(&mut app, "dd");
        assert_eq!(items(&app), ["Gym", "Report", "Buy milk"]);
        type_str(&mut app, "c");
        assert_eq!(time_of(&app, 0), None);
        type_str(&mut app, "u");
        assert_eq!(time_of(&app, 0).as_deref(), Some("18:00–19:00"));
        type_str(&mut app, "u");
        assert_eq!(items(&app), ["Standup", "Gym", "Report", "Buy milk"], "the delete undone");
        assert!(matches!(&app.mode, Mode::Plan { plan, .. } if !plan.changed), "back to how it opened");
        type_str(&mut app, "u");
        assert!(app.message.as_ref().unwrap().contains("Nothing in the plan to undo"));
        app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL)).unwrap();
        assert_eq!(items(&app), ["Gym", "Report", "Buy milk"], "redone");
        assert!(matches!(&app.mode, Mode::Plan { plan, .. } if plan.changed));
        // Saved, the whole plan is still one change on the list.
        type_str(&mut app, "w");
        type_str(&mut app, "u");
        assert_eq!(items(&app), ["Standup", "Gym", "Report", "Buy milk"]);
    }

    #[test]
    fn undo_in_the_plan_covers_adding_and_setting_times() {
        let (mut app, _dir) = planning_app();
        type_str(&mut app, "aDentist");
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "15");
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Esc);
        assert_eq!(time_of(&app, 1).as_deref(), Some("15:00–16:00"));
        type_str(&mut app, "u");
        assert_eq!(time_of(&app, 1), None, "the time first");
        type_str(&mut app, "u");
        assert!(!items(&app).contains(&"Dentist"), "then the add");
    }

    #[test]
    fn v_selects_on_the_list_and_space_v_views_the_notes() {
        let (mut app, _dir) = app_with(&["one", "two"]);
        type_str(&mut app, "v");
        assert!(matches!(app.mode, Mode::Visual { .. }));
        type_str(&mut app, "v");
        assert!(matches!(app.mode, Mode::Normal), "v again stops, as V does");
        type_str(&mut app, " v");
        assert!(matches!(app.mode, Mode::View(_)));
    }

    #[test]
    fn the_lists_keys_work_in_the_plan_too() {
        // Standup, Gym, Report and Buy milk.
        let (mut app, _dir) = planning_app();
        // J and K reorder, with counts and sequences as on the list.
        type_str(&mut app, "J");
        assert_eq!(items(&app), ["Gym", "Standup", "Report", "Buy milk"]);
        type_str(&mut app, "gg2j");
        assert_eq!(items(&app)[app.selected], "Report");
        // yy and p copy and paste.
        type_str(&mut app, "yyp");
        assert_eq!(items(&app)[..4], ["Gym", "Standup", "Report", "Report"]);
        // > and < indent and outdent; za folds.
        type_str(&mut app, ">");
        assert_eq!(shown(&app)[2..4], ["Report", "  Report"]);
        type_str(&mut app, "kza");
        assert_eq!(shown(&app)[2], "Report+");
        type_str(&mut app, "zoj<");
        assert_eq!(shown(&app)[3], "Report");
        assert!(in_plan(&app), "still planning after all of it");
        // m pins; a sub-item can't be.
        type_str(&mut app, "ggm");
        assert!(app.items()[0].pinned);
    }

    #[test]
    fn deadlines_reminders_and_selecting_from_the_plan_come_back_to_it() {
        let (mut app, _dir) = planning_app();
        type_str(&mut app, "@");
        assert!(matches!(app.mode, Mode::Calendar(_)));
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter);
        assert!(in_plan(&app));
        assert!(app.items()[0].deadline.is_some());
        type_str(&mut app, "rd2");
        press(&mut app, KeyCode::Enter);
        assert!(in_plan(&app));
        assert!(app.items()[0].reminder.is_some());
        // v selects; d deletes the selection, then it's back to the plan.
        type_str(&mut app, "vjd");
        press(&mut app, KeyCode::Char('d'));
        assert!(in_plan(&app));
        assert_eq!(items(&app), ["Report", "Buy milk"]);
        // Undo in the plan brings them back.
        type_str(&mut app, "u");
        assert_eq!(items(&app), ["Standup", "Gym", "Report", "Buy milk"]);
        // Selecting can't move items off the day being planned.
        type_str(&mut app, "vL");
        assert!(matches!(app.mode, Mode::Visual { .. }));
        assert_eq!(app.day, today());
    }

    #[test]
    fn x_deletes_in_the_plan_asking_first_and_c_clears_a_time() {
        let (mut app, _dir) = planning_app();
        type_str(&mut app, "c");
        assert_eq!(time_of(&app, 0), None);
        assert_eq!(items(&app).len(), 4, "c only clears the time");
        type_str(&mut app, "x");
        assert!(matches!(app.mode, Mode::ConfirmDelete { .. }), "asks first");
        type_str(&mut app, "d");
        assert!(in_plan(&app));
        assert_eq!(items(&app), ["Gym", "Report", "Buy milk"]);
    }

    /// Changes the app's todos file as another todoro would, `todoro -a`.
    fn add_outside(app: &App, text: &str) {
        let mut other = Store::open(app.store.path().to_path_buf()).unwrap();
        let end = other.open_count(today());
        other.insert(today(), end, text.into()).unwrap();
    }

    #[test]
    fn the_list_rereads_the_file_when_something_else_changes_it() {
        let (mut app, _dir) = app_with(&["Buy milk", "Call mom"]);
        type_str(&mut app, "xj");
        assert_eq!(app.items()[app.selected].text, "Buy milk");
        add_outside(&app, "Book flights");
        app.reload_if_changed().unwrap();
        assert_eq!(items(&app), ["Call mom", "Book flights", "Buy milk"]);
        // The same item is still selected, and undo can't bring back the old file.
        assert_eq!(app.items()[app.selected].text, "Buy milk");
        type_str(&mut app, "u");
        assert_eq!(items(&app), ["Call mom", "Book flights", "Buy milk"]);
        // Its next change keeps the outside one.
        type_str(&mut app, "gg");
        type_str(&mut app, "x");
        assert_eq!(Store::open(app.store.path().to_path_buf()).unwrap().items(today()).len(), 3);
        assert!(app.message.is_none());
    }

    #[test]
    fn nothing_is_reread_unless_the_file_changed() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        type_str(&mut app, "x");
        app.reload_if_changed().unwrap();
        // Undo still works, as nothing was reread.
        type_str(&mut app, "u");
        assert!(!app.store.items(today())[0].done);
    }

    #[test]
    fn mid_edit_the_file_isnt_reread_and_saving_keeps_both() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        type_str(&mut app, "a");
        add_outside(&app, "Book flights");
        app.reload_if_changed().unwrap();
        assert!(matches!(app.mode, Mode::Insert { .. }));
        type_str(&mut app, "Call mom");
        press(&mut app, KeyCode::Enter);
        assert_eq!(items(&app), ["Buy milk", "Book flights", "Call mom"]);
        assert_eq!(Store::open(app.store.path().to_path_buf()).unwrap().items(today()).len(), 3);
        assert!(app.message.as_ref().is_some_and(|message| message.contains("both kept")), "{:?}", app.message);
        // Undo can't bring back the old file.
        type_str(&mut app, "u");
        assert_eq!(items(&app), ["Buy milk", "Book flights", "Call mom"]);
    }

    #[test]
    fn notes_being_typed_follow_their_item_through_a_merge() {
        let (mut app, _dir) = app_with(&["Buy milk", "Call mom"]);
        type_str(&mut app, "j");
        press(&mut app, KeyCode::Enter);
        // Deleted elsewhere, so every index moves.
        let mut other = Store::open(app.store.path().to_path_buf()).unwrap();
        other.delete_block(today(), 0).unwrap();
        type_str(&mut app, "Ask");
        assert!(matches!(app.mode, Mode::Notes(_)));
        assert_eq!(items(&app), ["Call mom", "Buy milk", "Call mom"]);
        assert_eq!(app.selected, 2);
        type_str(&mut app, " about Sunday");
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Esc);
        let notes: Vec<&str> = app.store.items(today()).iter().map(|item| item.notes.as_str()).collect();
        assert_eq!(notes, ["", "", "Ask about Sunday"]);
    }

    #[test]
    fn a_merge_closes_what_holds_old_positions() {
        let (mut app, _dir) = app_with(&["Buy milk", "Call mom"]);
        type_str(&mut app, " p");
        assert!(matches!(app.mode, Mode::Plan { .. }));
        add_outside(&app, "Book flights");
        type_str(&mut app, "!");
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(items(&app), ["Buy milk", "Call mom", "Book flights", "Buy milk"]);
    }

    #[test]
    fn a_pending_key_waits_for_the_reread() {
        let (mut app, _dir) = app_with(&["Buy milk", "Call mom"]);
        type_str(&mut app, "g");
        add_outside(&app, "Book flights");
        app.reload_if_changed().unwrap();
        assert_eq!(items(&app), ["Buy milk", "Call mom"]);
        type_str(&mut app, "g");
        app.reload_if_changed().unwrap();
        assert_eq!(items(&app), ["Buy milk", "Call mom", "Book flights"]);
    }

    /// A newer todoro on "GitHub", with its notes, for the update tests.
    fn newer_on_github() -> update::Fetcher {
        update::Fetcher {
            latest: || Ok("99.0.0".into()),
            changelog: |_| Ok("# Changelog\n\n## [99.0.0] - 2027-01-01\n\n- Everything is better\n".into()),
            installed: || update::Install::Installer,
        }
    }

    /// Waits for the background checks to answer, as the event loop would.
    fn settle(app: &mut App) {
        for _ in 0..400 {
            app.tick(std::time::Instant::now());
            if app.update_find.is_none() && app.update_check.is_none() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("no answer");
    }

    fn colon(app: &mut App, command: &str) {
        type_str(app, &format!(":{command}"));
        press(app, KeyCode::Enter);
    }

    #[test]
    fn colon_update_shows_whats_new_and_installs_only_on_y() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        app.fetcher = newer_on_github();
        colon(&mut app, "update");
        assert!(matches!(&app.mode, Mode::Update(popup) if matches!(**popup, update::Popup::Checking)));
        settle(&mut app);
        let Mode::Update(popup) = &app.mode else { panic!("the update popup") };
        let update::Popup::Newer { version, notes, .. } = &**popup else { panic!("a newer version") };
        assert_eq!(version, "99.0.0");
        assert_eq!(notes.releases()[0].notes, "- Everything is better");
        assert_eq!(app.settings.update_available.as_deref(), Some("99.0.0"));
        // n stays on this version.
        type_str(&mut app, "n");
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.install_update, None);
        // y asks the event loop to install it.
        colon(&mut app, "update");
        settle(&mut app);
        type_str(&mut app, "y");
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.install_update.as_deref(), Some("99.0.0"));
    }

    #[test]
    fn colon_update_says_when_its_the_latest_or_github_cant_be_reached() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        app.fetcher = update::Fetcher { latest: || Ok(changelog::VERSION.into()), ..newer_on_github() };
        app.settings.update_available = Some("99.0.0".into());
        colon(&mut app, "update");
        settle(&mut app);
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.message, Some(format!("todoro {} is the latest version", changelog::VERSION)));
        assert_eq!(app.settings.update_available, None);
        app.fetcher = crate::test_util::offline();
        colon(&mut app, "update");
        settle(&mut app);
        assert_eq!(app.message.as_deref(), Some("Couldn't check for updates: no network in tests"));
    }

    #[test]
    fn esc_while_checking_ignores_the_answer() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        app.fetcher = newer_on_github();
        colon(&mut app, "update");
        press(&mut app, KeyCode::Esc);
        assert!(matches!(app.mode, Mode::Normal));
        settle(&mut app);
        assert!(matches!(app.mode, Mode::Normal));
    }

    #[test]
    fn installed_some_other_way_the_popup_says_how_to_update_and_y_does_nothing() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        app.fetcher = update::Fetcher { installed: || update::Install::Other, ..newer_on_github() };
        colon(&mut app, "update");
        settle(&mut app);
        type_str(&mut app, "y");
        assert!(matches!(app.mode, Mode::Update(_)));
        assert_eq!(app.install_update, None);
    }

    #[test]
    fn the_daily_check_only_notes_a_newer_version() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        app.fetcher = newer_on_github();
        app.start_update_check();
        settle(&mut app);
        assert_eq!(app.settings.update_checked.as_deref(), Some("2026-10-05"));
        assert_eq!(app.update_available(), Some("99.0.0"));
        assert!(matches!(app.mode, Mode::Normal), "nothing opens, and nothing is installed");
        assert_eq!(app.install_update, None);
        // Once a day.
        app.fetcher = update::Fetcher { latest: || panic!("asked twice in a day"), ..newer_on_github() };
        app.start_update_check();
        assert!(app.update_check.is_none());
        // Not at all with the option off, which hides what was found too.
        app.settings.update_checked = None;
        app.settings.no_update_check = true;
        app.start_update_check();
        assert!(app.update_check.is_none());
        assert_eq!(app.update_available(), None);
    }

    #[test]
    fn offline_the_daily_check_tries_again_next_time() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        app.start_update_check();
        settle(&mut app);
        assert_eq!(app.settings.update_checked, None);
        assert_eq!(app.update_available(), None);
    }
}
