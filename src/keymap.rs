//! Keybindings for the list and the leader menu: todoro's defaults, with the
//! user's own from `keys.toml` applied over them. The file only lists what
//! changes; everything else keeps its default.
//!
//! ```toml
//! leader = "space"        # the key that opens the leader menu
//!
//! [list]
//! "c" = "calendar"        # a key of its own for the calendar
//! "s" = "none"            # s does nothing
//!
//! [leader-menu]
//! "f" = "find-items"      # leader f
//! "s n" = "search-notes"  # leader s n
//! "s" = "+search"         # names the group shown for leader s
//! ```

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Something a key can do, on the list or from the leader menu. Every
/// command works in both places.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    PrevDay,
    NextDay,
    Today,
    Down,
    Up,
    First,
    Last,
    MoveDown,
    MoveUp,
    MoveToPrevDay,
    MoveToNextDay,
    Add,
    AddSeveral,
    AddSub,
    AddSubs,
    Indent,
    Outdent,
    FoldToggle,
    Fold,
    Unfold,
    Edit,
    EditCleared,
    ToggleDone,
    TogglePin,
    Deadline,
    Reminder,
    Overdue,
    Triage,
    Delete,
    Copy,
    PasteBelow,
    PasteAbove,
    Select,
    ViewNotes,
    ViewAll,
    ViewDay,
    OpenNotes,
    Undo,
    Redo,
    Colon,
    Help,
    Quit,
    Leader,
    FindItems,
    SearchNotes,
    Deadlines,
    Reminders,
    Tags,
    SearchOptions,
    Calendar,
    Options,
    Workspaces,
    PatchNotes,
    Timeline,
    Plan,
}

/// Every command, in the order the help and the starter file list them:
/// its name in `keys.toml`, a short label for the leader menu, and what it
/// does, for the help.
const COMMANDS: &[(Command, &str, &str, &str)] = &[
    (Command::PrevDay, "prev-day", "previous day", "Previous day"),
    (Command::NextDay, "next-day", "next day", "Next day"),
    (Command::Today, "today", "today", "Go to today"),
    (Command::Down, "down", "down", "Move down"),
    (Command::Up, "up", "up", "Move up"),
    (Command::First, "first", "first item", "First item"),
    (Command::Last, "last", "last item", "Last item, or with a number before it, that item"),
    (Command::MoveDown, "move-down", "move item down", "Move the item down the list"),
    (Command::MoveUp, "move-up", "move item up", "Move the item up the list"),
    (Command::MoveToPrevDay, "move-to-prev-day", "move to previous day", "Move the item to the previous day"),
    (Command::MoveToNextDay, "move-to-next-day", "move to next day", "Move the item to the next day"),
    (Command::Add, "add", "add", "Add an item below the cursor"),
    (Command::AddSeveral, "add-several", "add several", "Add several items, one after another"),
    (Command::AddSub, "add-sub", "add sub-item", "Add a sub-item under the selected item"),
    (Command::AddSubs, "add-several-subs", "add sub-items", "Add several sub-items under it, one after another"),
    (Command::Indent, "indent", "indent", "Make the item a sub-item of the one above it"),
    (Command::Outdent, "outdent", "outdent", "Move a sub-item out a level"),
    (Command::FoldToggle, "fold-toggle", "fold / unfold", "Fold or unfold the item's sub-items"),
    (Command::Fold, "fold", "fold", "Fold the item's sub-items away"),
    (Command::Unfold, "unfold", "unfold", "Unfold the item's sub-items"),
    (Command::Edit, "edit", "edit", "Edit the selected item, cursor at the end"),
    (Command::EditCleared, "edit-cleared", "edit cleared", "Edit it with the text cleared"),
    (Command::ToggleDone, "toggle-done", "done", "Mark the selected item done / not done"),
    (Command::TogglePin, "toggle-pin", "pin", "Pin / unpin: move to today until done"),
    (Command::Deadline, "deadline", "deadline", "Set a deadline: a day, then a time or none"),
    (Command::Reminder, "reminder", "reminder", "Set a reminder, some time before or on a day"),
    (Command::Overdue, "overdue", "overdue", "List the items past their deadline"),
    (Command::Triage, "triage", "triage", "Triage: High, Medium, Low, then none"),
    (Command::Delete, "delete", "delete", "Delete the selected item"),
    (Command::Copy, "copy", "copy", "Copy the selected item, and its text to the clipboard"),
    (Command::PasteBelow, "paste-below", "paste below", "Paste below (also after deleting)"),
    (Command::PasteAbove, "paste-above", "paste above", "Paste above"),
    (Command::Select, "select", "select", "Select several items"),
    (Command::ViewNotes, "view-notes", "view notes", "View the selected item's notes, formatted"),
    (Command::ViewAll, "view-all", "view all", "View the selected item and all its sub-items, with their notes"),
    (Command::ViewDay, "view-day", "view the day", "View every item on the day, with its sub-items and notes"),
    (Command::OpenNotes, "open-notes", "notes", "Edit the selected item's notes (in your own editor, if set in the options)"),
    (Command::Undo, "undo", "undo", "Undo a change to the list"),
    (Command::Redo, "redo", "redo", "Redo a change to the list"),
    (Command::Colon, "command-line", "command line", "Type a command, like :42 to go to item 42"),
    (Command::Help, "help", "keybindings", "Show this help"),
    (Command::Quit, "quit", "quit", "Quit"),
    (Command::Leader, "leader", "leader menu", "Menu of the commands below, narrowing as you type"),
    (Command::FindItems, "find-items", "find items", "Find items by their text, on any day"),
    (Command::SearchNotes, "search-notes", "search notes", "Search inside every item's notes"),
    (Command::Deadlines, "deadlines", "deadlines", "List every deadline, soonest first"),
    (Command::Reminders, "reminders", "reminders", "List every reminder, soonest first"),
    (Command::Tags, "tags", "tags", "List tags, to show the items with one"),
    (Command::SearchOptions, "search-options", "options", "Search the options"),
    (Command::Calendar, "calendar", "calendar", "Open the calendar"),
    (Command::Options, "options", "options", "Options, including accessibility"),
    (Command::Workspaces, "workspaces", "workspaces", "Workspaces: switch, create or delete"),
    (Command::Timeline, "timeline", "timeline", "The day as a timeline, with the time now"),
    (Command::Plan, "plan", "plan the day", "Plan the day: give items a start and end time"),
    (Command::PatchNotes, "patch-notes", "patch notes", "Patch notes: what changed in every release"),
];

impl Command {
    fn entry(self) -> &'static (Command, &'static str, &'static str, &'static str) {
        COMMANDS.iter().find(|entry| entry.0 == self).expect("every command is in COMMANDS")
    }

    /// Its name in `keys.toml`, like `toggle-done`.
    pub fn name(self) -> &'static str {
        self.entry().1
    }

    /// A word or two for the leader menu.
    pub fn label(self) -> &'static str {
        self.entry().2
    }

    /// What it does, for the help.
    pub fn description(self) -> &'static str {
        self.entry().3
    }

    pub fn from_name(name: &str) -> Option<Self> {
        COMMANDS.iter().find(|entry| entry.1 == name).map(|entry| entry.0)
    }

    /// Every command, in the order they're listed.
    pub fn all() -> impl Iterator<Item = Command> {
        COMMANDS.iter().map(|entry| entry.0)
    }
}

/// One key press: a key, and whether `Ctrl` was held, and `Shift` for keys
/// that aren't characters (`Shift+Enter`). A character's `Shift` is in the
/// character itself (`G`, `@`). Terminals tell `Shift+Enter` from `Enter`
/// only with the keyboard protocol `main` asks for, where they have it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    pub code: KeyCode,
    pub ctrl: bool,
    pub shift: bool,
}

/// Keys with names, as written in `keys.toml` (ignoring case) and as shown.
const NAMED: &[(&str, KeyCode, &str)] = &[
    ("space", KeyCode::Char(' '), "Space"),
    ("enter", KeyCode::Enter, "Enter"),
    ("return", KeyCode::Enter, "Enter"),
    ("tab", KeyCode::Tab, "Tab"),
    ("backspace", KeyCode::Backspace, "Backspace"),
    ("delete", KeyCode::Delete, "Delete"),
    ("del", KeyCode::Delete, "Delete"),
    ("insert", KeyCode::Insert, "Insert"),
    ("esc", KeyCode::Esc, "Esc"),
    ("escape", KeyCode::Esc, "Esc"),
    ("up", KeyCode::Up, "↑"),
    ("down", KeyCode::Down, "↓"),
    ("left", KeyCode::Left, "←"),
    ("right", KeyCode::Right, "→"),
    ("home", KeyCode::Home, "Home"),
    ("end", KeyCode::End, "End"),
    ("pageup", KeyCode::PageUp, "PageUp"),
    ("pagedown", KeyCode::PageDown, "PageDown"),
];

impl Key {
    pub fn char(c: char) -> Self {
        Self { code: KeyCode::Char(c), ctrl: false, shift: false }
    }

    fn new(code: KeyCode, ctrl: bool, shift: bool) -> Self {
        Self { code, ctrl, shift: shift && !matches!(code, KeyCode::Char(_)) }
    }

    pub fn from_event(event: KeyEvent) -> Self {
        let shift = event.modifiers.contains(KeyModifiers::SHIFT);
        Self::new(event.code, event.modifiers.contains(KeyModifiers::CONTROL), shift)
    }

    /// How it's shown: `x`, `Space`, `Enter`, `Ctrl+R`, `←`.
    pub fn name(self) -> String {
        let base = match self.code {
            KeyCode::Char(c) if self.ctrl => c.to_uppercase().to_string(),
            KeyCode::F(n) => format!("F{n}"),
            code => NAMED
                .iter()
                .find(|(_, named, _)| *named == code)
                .map(|(_, _, shown)| shown.to_string())
                .unwrap_or_else(|| match code {
                    KeyCode::Char(c) => c.to_string(),
                    _ => "?".into(),
                }),
        };
        let shift = if self.shift { "Shift+" } else { "" };
        if self.ctrl { format!("Ctrl+{shift}{base}") } else { format!("{shift}{base}") }
    }

    /// How it's written in `keys.toml`: `x`, `space`, `ctrl+r`, `shift+enter`.
    fn written(self) -> String {
        let base = match self.code {
            KeyCode::Char(c) if c != ' ' => c.to_string(),
            KeyCode::F(n) => format!("f{n}"),
            code => NAMED.iter().find(|(_, named, _)| *named == code).map_or("?", |(name, _, _)| name).to_string(),
        };
        let shift = if self.shift { "shift+" } else { "" };
        if self.ctrl { format!("ctrl+{shift}{base}") } else { format!("{shift}{base}") }
    }

    fn is_plain_char(self) -> bool {
        matches!(self.code, KeyCode::Char(c) if c != ' ') && !self.ctrl
    }
}

/// Reads keys as written in `keys.toml`: separated by spaces, each a single
/// character (`x`, `G`, `@`), a name (`space`, `enter`, `up`, `f5`), or
/// `ctrl+` and either, or `shift+` and a name (`shift+enter`). A word that
/// isn't a name is one key per character, so `gg` is `g` then `g`.
pub fn parse_keys(text: &str) -> Result<Vec<Key>, String> {
    let mut keys = Vec::new();
    for word in text.split_whitespace() {
        let (mut ctrl, mut shift, mut rest) = (false, false, word);
        loop {
            let lower = rest.to_lowercase();
            if lower.starts_with("ctrl+") && rest.len() > 5 {
                (ctrl, rest) = (true, &rest[5..]);
            } else if lower.starts_with("shift+") && rest.len() > 6 {
                (shift, rest) = (true, &rest[6..]);
            } else {
                break;
            }
        }
        let lower = rest.to_lowercase();
        let named = NAMED.iter().find(|(name, _, _)| *name == lower).map(|(_, code, _)| *code);
        let function = lower.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()).filter(|n| (1..=12).contains(n));
        if let Some(code) = named.filter(|code| !(shift && *code == KeyCode::Char(' '))) {
            keys.push(Key::new(code, ctrl, shift));
        } else if let Some(n) = function {
            keys.push(Key::new(KeyCode::F(n), ctrl, shift));
        } else if shift {
            return Err(format!("\"{word}\" isn't a key: shift+ goes with a named key, like shift+enter; write a capital for a letter"));
        } else if ctrl {
            let mut chars = rest.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => keys.push(Key::new(KeyCode::Char(c.to_ascii_lowercase()), ctrl, false)),
                _ => return Err(format!("\"{word}\" isn't a key: ctrl+ takes one key, like ctrl+r")),
            }
        } else {
            keys.extend(rest.chars().map(Key::char));
        }
    }
    if keys.is_empty() {
        return Err("a binding needs at least one key".into());
    }
    Ok(keys)
}

/// Keys as shown: characters run together, like vim's `gg`, and other keys
/// apart (`Ctrl+R`, `Space s g`).
pub fn show_keys(keys: &[Key]) -> String {
    let mut shown = String::new();
    let mut previous_plain = false;
    for key in keys {
        let plain = key.is_plain_char();
        if !shown.is_empty() && !(plain && previous_plain) {
            shown.push(' ');
        }
        shown.push_str(&key.name());
        previous_plain = plain;
    }
    shown
}

/// Keys after the leader, each apart: `s g`.
pub fn show_spaced(keys: &[Key]) -> String {
    keys.iter().map(|key| key.name()).collect::<Vec<_>>().join(" ")
}

fn write_keys(keys: &[Key]) -> String {
    keys.iter().map(|key| key.written()).collect::<Vec<_>>().join(" ")
}

#[derive(Clone, Debug, PartialEq)]
pub struct Binding {
    pub keys: Vec<Key>,
    pub command: Command,
}

/// What the keys typed so far do.
#[derive(Debug, PartialEq)]
pub enum Match {
    Run(Command),
    /// They start a longer binding: wait for more.
    More,
    Nothing,
}

/// What `typed` does among `bindings`. No binding starts another, so an
/// exact match always runs.
pub fn lookup(bindings: &[Binding], typed: &[Key]) -> Match {
    if let Some(binding) = bindings.iter().find(|binding| binding.keys == typed) {
        return Match::Run(binding.command);
    }
    if bindings.iter().any(|binding| binding.keys.starts_with(typed)) {
        return Match::More;
    }
    Match::Nothing
}

/// The list's keys, the leader menu's (typed after the leader), and names
/// for the leader menu's groups.
#[derive(Clone, Debug, PartialEq)]
pub struct Keymap {
    pub list: Vec<Binding>,
    pub leader: Vec<Binding>,
    pub groups: Vec<(Vec<Key>, String)>,
}

const DEFAULT_LIST: &[(&str, Command)] = &[
    ("h", Command::PrevDay),
    ("left", Command::PrevDay),
    ("l", Command::NextDay),
    ("right", Command::NextDay),
    ("t", Command::Today),
    ("j", Command::Down),
    ("down", Command::Down),
    ("k", Command::Up),
    ("up", Command::Up),
    ("g g", Command::First),
    ("G", Command::Last),
    ("J", Command::MoveDown),
    ("K", Command::MoveUp),
    ("H", Command::MoveToPrevDay),
    ("L", Command::MoveToNextDay),
    ("a", Command::Add),
    ("A", Command::AddSeveral),
    ("o", Command::AddSub),
    ("O", Command::AddSubs),
    (">", Command::Indent),
    ("<", Command::Outdent),
    ("z a", Command::FoldToggle),
    ("shift+enter", Command::FoldToggle),
    ("z c", Command::Fold),
    ("z o", Command::Unfold),
    ("e", Command::Edit),
    ("E", Command::EditCleared),
    ("x", Command::ToggleDone),
    ("m", Command::TogglePin),
    ("@", Command::Deadline),
    ("r", Command::Reminder),
    ("g o", Command::Overdue),
    ("!", Command::Triage),
    ("d", Command::Delete),
    ("y y", Command::Copy),
    ("p", Command::PasteBelow),
    ("P", Command::PasteAbove),
    ("V", Command::Select),
    ("v", Command::Select),
    ("enter", Command::OpenNotes),
    ("u", Command::Undo),
    ("ctrl+r", Command::Redo),
    (":", Command::Colon),
    ("?", Command::Help),
    ("q", Command::Quit),
    ("space", Command::Leader),
];

const DEFAULT_LEADER: &[(&str, Command)] = &[
    ("space", Command::FindItems),
    ("s g", Command::SearchNotes),
    ("s d", Command::Deadlines),
    ("s r", Command::Reminders),
    ("s t", Command::Tags),
    ("s o", Command::SearchOptions),
    ("s k", Command::Help),
    ("c", Command::Calendar),
    ("o", Command::Options),
    ("w", Command::Workspaces),
    ("n", Command::PatchNotes),
    ("v", Command::ViewNotes),
    ("V", Command::ViewAll),
    ("e", Command::ViewDay),
    ("t", Command::Timeline),
    ("p", Command::Plan),
];

const DEFAULT_GROUPS: &[(&str, &str)] = &[("s", "search")];

fn bindings(table: &[(&str, Command)]) -> Vec<Binding> {
    table
        .iter()
        .map(|(keys, command)| Binding { keys: parse_keys(keys).expect("default keys parse"), command: *command })
        .collect()
}

impl Default for Keymap {
    fn default() -> Self {
        Self {
            list: bindings(DEFAULT_LIST),
            leader: bindings(DEFAULT_LEADER),
            groups: DEFAULT_GROUPS
                .iter()
                .map(|(keys, name)| (parse_keys(keys).expect("default keys parse"), name.to_string()))
                .collect(),
        }
    }
}

/// Whether one key sequence starts the other (or they're the same), so
/// both can't be bound: the shorter would always run first.
fn clash(a: &[Key], b: &[Key]) -> bool {
    a.starts_with(b) || b.starts_with(a)
}

/// Binds `keys` to `command` in `table`, or with `None` frees them and
/// whatever starts with them. A binding of the user's replaces any default
/// it clashes with, but not another of theirs (`set` lists those).
fn bind(table: &mut Vec<Binding>, set: &mut Vec<Vec<Key>>, keys: Vec<Key>, command: Option<Command>) -> Result<(), String> {
    if let Some(earlier) = set.iter().find(|earlier| clash(earlier, &keys)) {
        return Err(format!("clashes with \"{}\", set earlier", write_keys(earlier)));
    }
    match command {
        Some(command) => {
            table.retain(|binding| !clash(&binding.keys, &keys));
            table.push(Binding { keys: keys.clone(), command });
        }
        None => table.retain(|binding| !binding.keys.starts_with(&keys)),
    }
    set.push(keys);
    Ok(())
}

/// The key todoro always keeps: `Ctrl+C` quits from anywhere.
fn reserved(keys: &[Key]) -> Option<&'static str> {
    if keys.first() == Some(&Key { code: KeyCode::Char('c'), ctrl: true, shift: false }) {
        return Some("Ctrl+C always quits");
    }
    None
}

impl Keymap {
    /// The defaults with `text` (a `keys.toml`) applied, and what in it
    /// couldn't be used. A problem skips just its line.
    pub fn from_toml(text: &str) -> (Self, Vec<String>) {
        let mut keymap = Self::default();
        let mut problems = Vec::new();
        let table: toml::Table = match text.parse() {
            Ok(table) => table,
            Err(error) => {
                let message = error.message().to_string();
                let line = error.span().map(|span| text[..span.start].lines().count().max(1));
                problems.push(match line {
                    Some(line) => format!("line {line}: {message}"),
                    None => message,
                });
                return (keymap, problems);
            }
        };
        let mut list_set = Vec::new();
        // The leader first, so [list] can still change its key.
        if let Some(value) = table.get("leader") {
            match value.as_str().map(parse_keys) {
                Some(Ok(keys)) if reserved(&keys).is_some() => {
                    problems.push(format!("leader: {}", reserved(&keys).unwrap_or_default()));
                }
                Some(Ok(keys)) => {
                    keymap.list.retain(|binding| binding.command != Command::Leader);
                    if let Err(problem) = bind(&mut keymap.list, &mut list_set, keys, Some(Command::Leader)) {
                        problems.push(format!("leader: {problem}"));
                    }
                }
                Some(Err(problem)) => problems.push(format!("leader: {problem}")),
                None => problems.push("leader: write the key in quotes, like leader = \"space\"".into()),
            }
        }
        let mut leader_set = Vec::new();
        for (name, value) in &table {
            match (name.as_str(), value) {
                ("leader", _) => {}
                ("list", toml::Value::Table(entries)) => {
                    for (keys, value) in entries {
                        if let Err(problem) = keymap.apply_list(keys, value, &mut list_set) {
                            problems.push(format!("[list] \"{keys}\": {problem}"));
                        }
                    }
                }
                ("leader-menu", toml::Value::Table(entries)) => {
                    for (keys, value) in entries {
                        if let Err(problem) = keymap.apply_leader(keys, value, &mut leader_set) {
                            problems.push(format!("[leader-menu] \"{keys}\": {problem}"));
                        }
                    }
                }
                ("list" | "leader-menu", _) => problems.push(format!("{name} should be a section, [{name}]")),
                _ => problems.push(format!(
                    "\"{name}\" isn't something keys.toml has; it has leader, [list] and [leader-menu]"
                )),
            }
        }
        (keymap, problems)
    }

    fn apply_list(&mut self, keys: &str, value: &toml::Value, set: &mut Vec<Vec<Key>>) -> Result<(), String> {
        let keys = parse_keys(keys)?;
        if let Some(reason) = reserved(&keys) {
            return Err(reason.into());
        }
        if let Some(Key { code: KeyCode::Char(c @ '0'..='9'), ctrl: false, .. }) = keys.first() {
            return Err(format!("{c} is for counts, like 4j"));
        }
        let command = command_value(value)?;
        bind(&mut self.list, set, keys, command)
    }

    fn apply_leader(&mut self, keys: &str, value: &toml::Value, set: &mut Vec<Vec<Key>>) -> Result<(), String> {
        let keys = parse_keys(keys)?;
        if let Some(name) = value.as_str().and_then(|text| text.strip_prefix('+')) {
            self.groups.retain(|(group, _)| *group != keys);
            self.groups.push((keys, name.trim().to_string()));
            return Ok(());
        }
        let command = command_value(value)?;
        bind(&mut self.leader, set, keys, command)
    }

    /// Reads the user's `keys.toml` at `path`, if there is one. No file
    /// gives the defaults, with nothing to report.
    pub fn load(path: Option<&Path>) -> (Self, Vec<String>) {
        match path.map(fs::read_to_string) {
            Some(Ok(text)) => Self::from_toml(&text),
            Some(Err(error)) if error.kind() != io::ErrorKind::NotFound => {
                (Self::default(), vec![format!("couldn't read it: {error}")])
            }
            _ => (Self::default(), Vec::new()),
        }
    }

    /// How the leader is shown: its key, like `Space`.
    pub fn leader_name(&self) -> String {
        self.list
            .iter()
            .find(|binding| binding.command == Command::Leader)
            .map_or_else(|| "Leader".into(), |binding| show_keys(&binding.keys))
    }

    /// Every way to run `command`, as shown: on the list (`x`), then from
    /// the leader menu (`Space s g`).
    pub fn keys_for(&self, command: Command) -> Vec<String> {
        let list = self.list.iter().filter(|binding| binding.command == command).map(|binding| show_keys(&binding.keys));
        let leader = self
            .leader
            .iter()
            .filter(|binding| binding.command == command)
            .map(|binding| format!("{} {}", self.leader_name(), show_spaced(&binding.keys)));
        list.chain(leader).collect()
    }

    /// The first way to run `command`, as shown, if it has a key.
    pub fn key_for(&self, command: Command) -> Option<String> {
        self.keys_for(command).into_iter().next()
    }

    /// The name of the leader menu's group for `keys`, if it has one.
    pub fn group_name(&self, keys: &[Key]) -> Option<&str> {
        self.groups.iter().find(|(group, _)| group == keys).map(|(_, name)| name.as_str())
    }
}

/// A command's name, or `none` to free the keys.
fn command_value(value: &toml::Value) -> Result<Option<Command>, String> {
    let Some(name) = value.as_str() else {
        return Err("write the command in quotes, like \"toggle-done\"".into());
    };
    if name == "none" {
        return Ok(None);
    }
    Command::from_name(name).map(Some).ok_or_else(|| format!("\"{name}\" isn't a command"))
}

/// What a new `keys.toml` starts with: how it works, then every default
/// binding, commented out, to uncomment and change.
pub fn starter_file() -> String {
    let mut text = String::from(
        "# todoro's keybindings. Lines starting with # are ignored: uncomment one\n\
         # (remove the #) and change its key to rebind that command. Keys you\n\
         # don't mention keep their defaults.\n\
         #\n\
         # Keys: a character (x, G, @), a name (space, enter, tab, up, down, left,\n\
         # right, home, end, pageup, pagedown, backspace, delete, esc, f1-f12),\n\
         # ctrl+ and one of those (ctrl+r), or shift+ and a name (shift+enter). Separate a sequence with spaces\n\
         # (\"g g\", \"s n\"). \"none\" frees a key: \"x\" = \"none\".\n\
         # Every command works on the list and in the leader menu.\n\
         # Digits are for counts (4j), and Ctrl+C always quits.\n\
         \n\
         # The key that opens the leader menu. Keep it above [list].\n\
         # leader = \"space\"\n\
         \n\
         [list]\n",
    );
    let line = |keys: &str, value: &str, about: &str| format!("# {:<24}# {about}\n", format!("\"{keys}\" = \"{value}\""));
    for (keys, command) in DEFAULT_LIST {
        if *command != Command::Leader {
            text.push_str(&line(keys, command.name(), command.description()));
        }
    }
    text.push_str("\n# Keys typed after the leader. \"+name\" names a group.\n[leader-menu]\n");
    for (keys, name) in DEFAULT_GROUPS {
        text.push_str(&line(keys, &format!("+{name}"), "a group of keys"));
    }
    for (keys, command) in DEFAULT_LEADER {
        text.push_str(&line(keys, command.name(), command.description()));
    }
    text
}

/// Where the user's keys are: `$TODORO_KEYS`, or `keys.toml` beside the
/// settings. On macOS, `~/.config/todoro/keys.toml` too, where dotfiles
/// usually live, if it's there.
pub fn default_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("TODORO_KEYS") {
        return Some(PathBuf::from(path));
    }
    if cfg!(target_os = "macos")
        && let Some(dotfile) = dirs::home_dir().map(|home| home.join(".config").join("todoro").join("keys.toml"))
        && dotfile.exists()
    {
        return Some(dotfile);
    }
    Some(dirs::config_dir()?.join("todoro").join("keys.toml"))
}

/// Writes the starter file at `path` if there's nothing there yet.
pub fn ensure_file(path: &Path) -> io::Result<()> {
    if path.exists() {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, starter_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(text: &str) -> Vec<Key> {
        parse_keys(text).unwrap()
    }

    fn ctrl(c: char) -> Key {
        Key { code: KeyCode::Char(c), ctrl: true, shift: false }
    }

    #[test]
    fn keys_are_read_as_written() {
        assert_eq!(keys("x"), [Key::char('x')]);
        assert_eq!(keys("G"), [Key::char('G')]);
        assert_eq!(keys("gg"), [Key::char('g'), Key::char('g')]);
        assert_eq!(keys("g g"), [Key::char('g'), Key::char('g')]);
        assert_eq!(keys("space"), [Key::char(' ')]);
        assert_eq!(keys("Space s g"), [Key::char(' '), Key::char('s'), Key::char('g')]);
        assert_eq!(keys("ctrl+r"), [ctrl('r')]);
        assert_eq!(keys("Ctrl+R"), [ctrl('r')]);
        assert_eq!(keys("enter"), [Key::new(KeyCode::Enter, false, false)]);
        assert_eq!(keys("up"), [Key::new(KeyCode::Up, false, false)]);
        assert_eq!(keys("f5"), [Key::new(KeyCode::F(5), false, false)]);
        assert_eq!(keys("ctrl+up"), [Key::new(KeyCode::Up, true, false)]);
        assert_eq!(keys("shift+enter"), [Key::new(KeyCode::Enter, false, true)]);
        assert_eq!(keys("Shift+Tab"), [Key::new(KeyCode::Tab, false, true)]);
        assert_eq!(keys("ctrl+shift+up"), [Key::new(KeyCode::Up, true, true)]);
        assert_eq!(keys("@"), [Key::char('@')]);
        assert_eq!(keys("é"), [Key::char('é')]);
        for broken in ["", "   ", "ctrl+ab", "shift+x", "shift+space"] {
            assert!(parse_keys(broken).is_err(), "{broken:?}");
        }
    }

    #[test]
    fn keys_are_shown_as_in_the_help() {
        assert_eq!(show_keys(&keys("g g")), "gg");
        assert_eq!(show_keys(&keys("ctrl+r")), "Ctrl+R");
        assert_eq!(show_keys(&keys("space")), "Space");
        assert_eq!(show_keys(&keys("enter")), "Enter");
        assert_eq!(show_keys(&keys("left")), "←");
        assert_eq!(show_keys(&keys("shift+enter")), "Shift+Enter");
        assert_eq!(show_spaced(&keys("s g")), "s g");
        assert_eq!(show_spaced(&keys("space")), "Space");
    }

    #[test]
    fn written_keys_read_back_the_same() {
        for (text, _) in DEFAULT_LIST.iter().chain(DEFAULT_LEADER) {
            let parsed = keys(text);
            assert_eq!(keys(&write_keys(&parsed)), parsed, "{text}");
        }
    }

    #[test]
    fn every_command_has_one_entry_and_a_unique_name() {
        let names: Vec<&str> = Command::all().map(Command::name).collect();
        for (i, name) in names.iter().enumerate() {
            assert!(!names[..i].contains(name), "{name} twice");
            assert_eq!(Command::from_name(name).map(Command::name), Some(*name));
        }
        assert_eq!(Command::from_name("nope"), None);
    }

    #[test]
    fn the_readme_lists_every_command() {
        let readme = include_str!("../README.md");
        for command in Command::all() {
            let row = format!("| `{}` | {} |", command.name(), command.description());
            assert!(readme.contains(&row), "README.md's command table needs: {row}");
        }
    }

    #[test]
    fn no_default_binding_starts_another() {
        for table in [Keymap::default().list, Keymap::default().leader] {
            for (i, a) in table.iter().enumerate() {
                for b in &table[i + 1..] {
                    assert!(!clash(&a.keys, &b.keys), "{:?} and {:?}", a.keys, b.keys);
                }
            }
        }
    }

    #[test]
    fn lookup_runs_waits_or_finds_nothing() {
        let map = Keymap::default();
        assert_eq!(lookup(&map.list, &keys("x")), Match::Run(Command::ToggleDone));
        assert_eq!(lookup(&map.list, &keys("g")), Match::More);
        assert_eq!(lookup(&map.list, &keys("g g")), Match::Run(Command::First));
        assert_eq!(lookup(&map.list, &keys("g z")), Match::Nothing);
        assert_eq!(lookup(&map.list, &[ctrl('x')]), Match::Nothing, "ctrl+x isn't x");
        assert_eq!(lookup(&map.leader, &keys("s")), Match::More);
        assert_eq!(lookup(&map.leader, &keys("s g")), Match::Run(Command::SearchNotes));
    }

    #[test]
    fn no_file_and_an_empty_one_give_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let (map, problems) = Keymap::load(Some(&dir.path().join("keys.toml")));
        assert_eq!(map, Keymap::default());
        assert!(problems.is_empty());
        assert_eq!(Keymap::from_toml(""), (Keymap::default(), Vec::new()));
        assert_eq!(Keymap::load(None), (Keymap::default(), Vec::new()));
    }

    #[test]
    fn the_starter_file_changes_nothing_until_lines_are_uncommented() {
        let (map, problems) = Keymap::from_toml(&starter_file());
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(map, Keymap::default());
        // Uncommenting every line gives the defaults too.
        let uncommented: String = starter_file()
            .lines()
            .map(|line| match line.strip_prefix("# ") {
                Some(rest) if rest.starts_with('"') || rest.starts_with("leader") => {
                    rest.split(" #").next().unwrap_or(rest).to_string()
                }
                _ => line.to_string(),
            })
            .map(|line| line + "\n")
            .collect();
        let (map, problems) = Keymap::from_toml(&uncommented);
        assert!(problems.is_empty(), "{problems:?}\n{uncommented}");
        assert_eq!(map.list.len(), Keymap::default().list.len());
        assert_eq!(map.leader.len(), Keymap::default().leader.len());
    }

    #[test]
    fn list_keys_can_be_changed_added_and_freed() {
        let (map, problems) = Keymap::from_toml(
            r#"
            [list]
            "c" = "calendar"
            "x" = "none"
            "ctrl+d" = "delete"
            "#,
        );
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(lookup(&map.list, &keys("c")), Match::Run(Command::Calendar));
        assert_eq!(lookup(&map.list, &keys("x")), Match::Nothing);
        assert_eq!(lookup(&map.list, &keys("ctrl+d")), Match::Run(Command::Delete));
        // d still deletes too: a new key adds to the old ones.
        assert_eq!(lookup(&map.list, &keys("d")), Match::Run(Command::Delete));
        assert_eq!(map.keys_for(Command::Delete), ["d", "Ctrl+D"]);
    }

    #[test]
    fn a_users_key_replaces_defaults_it_clashes_with() {
        let (map, problems) = Keymap::from_toml("[list]\n\"g\" = \"first\"\n");
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(lookup(&map.list, &keys("g")), Match::Run(Command::First));
        assert!(map.keys_for(Command::Overdue).is_empty(), "go is gone with g taken");
        // Freeing g frees what starts with it too.
        let (map, _) = Keymap::from_toml("[list]\n\"g\" = \"none\"\n");
        assert_eq!(lookup(&map.list, &keys("g")), Match::Nothing);
        assert!(map.keys_for(Command::First).is_empty());
    }

    #[test]
    fn the_leader_and_its_menu_can_change() {
        let (map, problems) = Keymap::from_toml(
            r#"
            leader = ","

            [leader-menu]
            "f" = "find-items"
            "s n" = "search-notes"
            "s" = "+find"
            "c" = "none"
            "x" = "toggle-done"
            "#,
        );
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(lookup(&map.list, &keys(",")), Match::Run(Command::Leader));
        assert_eq!(lookup(&map.list, &keys("space")), Match::Nothing, "space isn't the leader now");
        assert_eq!(map.leader_name(), ",");
        assert_eq!(lookup(&map.leader, &keys("f")), Match::Run(Command::FindItems));
        assert_eq!(lookup(&map.leader, &keys("s n")), Match::Run(Command::SearchNotes));
        assert_eq!(lookup(&map.leader, &keys("c")), Match::Nothing);
        assert_eq!(lookup(&map.leader, &keys("x")), Match::Run(Command::ToggleDone), "any command works there");
        assert_eq!(map.group_name(&keys("s")), Some("find"));
        // The default s g stays too, as new keys add to old ones.
        assert_eq!(map.keys_for(Command::SearchNotes), [", s g", ", s n"]);
    }

    #[test]
    fn problems_say_what_and_where_and_skip_just_that_line() {
        let (map, problems) = Keymap::from_toml(
            r#"
            colours = "red"

            [list]
            "x" = "explode"
            "1" = "add"
            "ctrl+c" = "add"
            "z" = 3
            "w" = "add"
            "w e" = "edit"
            "#,
        );
        assert_eq!(
            problems,
            [
                "\"colours\" isn't something keys.toml has; it has leader, [list] and [leader-menu]",
                "[list] \"x\": \"explode\" isn't a command",
                "[list] \"1\": 1 is for counts, like 4j",
                "[list] \"ctrl+c\": Ctrl+C always quits",
                "[list] \"z\": write the command in quotes, like \"toggle-done\"",
                "[list] \"w e\": clashes with \"w\", set earlier",
            ]
        );
        // The rest still apply.
        assert_eq!(lookup(&map.list, &keys("w")), Match::Run(Command::Add));
        assert_eq!(lookup(&map.list, &keys("x")), Match::Run(Command::ToggleDone), "unchanged");
    }

    #[test]
    fn a_file_that_isnt_toml_says_which_line() {
        let (map, problems) = Keymap::from_toml("[list]\n\"x\" = \"add\"\n\"y\" = \n");
        assert_eq!(map, Keymap::default());
        assert_eq!(problems.len(), 1);
        assert!(problems[0].starts_with("line 3"), "{problems:?}");
    }

    #[test]
    fn ensure_file_writes_the_starter_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("todoro").join("keys.toml");
        ensure_file(&path).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), starter_file());
        fs::write(&path, "[list]\n").unwrap();
        ensure_file(&path).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "[list]\n", "left alone");
    }
}
