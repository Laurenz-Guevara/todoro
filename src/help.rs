//! The `?` popup: every keybinding, searchable by key or by what it does.

use std::cell::Cell;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::keymap::{self, Command, Keymap};

pub struct Section {
    pub title: &'static str,
    /// `(keys, action)` pairs. Keys are separated by spaces or ` / `.
    pub bindings: &'static [(&'static str, &'static str)],
}

/// Every keybinding outside the list and the leader menu, whose sections
/// come from the keymap (`sections`). Keep it in sync with the key handling.
pub const SECTIONS: &[Section] = &[
    Section {
        title: "Adding or editing an item",
        bindings: &[
            ("← / →", "Move the cursor"),
            ("Home / End", "Jump to the start / end of the line"),
            ("Backspace / Delete", "Delete before / under the cursor"),
            ("Enter / Esc", "Save"),
            ("Tab", "Complete the #tag being typed"),
            ("↑ / ↓", "Choose among the tags suggested"),
        ],
    },
    Section {
        title: "Delete popup",
        bindings: &[("d", "Delete the item"), ("c / Esc", "Cancel")],
    },
    Section {
        title: "Notes",
        bindings: &[
            ("h j k l", "Move"),
            ("w / b / e", "Next word / previous word / end of word"),
            ("Ctrl+→ / Ctrl+←", "End / start of a word, also while typing"),
            ("0 / $", "Start / end of the line"),
            ("Home / End", "Start / end of the line, also while typing"),
            ("Ctrl+Home / End", "Start / end of the notes, also while typing"),
            ("_ / ^", "First non-blank character of the line"),
            ("gg / G", "First / last line"),
            ("4j / 4k", "Move four lines down / up (any number)"),
            (":42 / 42G", "Go to line 42"),
            (":q", "Back to the list (also :wq)"),
            ("i / a", "Insert before / after the cursor"),
            ("I / A", "Insert at the start / end of the line"),
            ("o / O", "Open a new line below / above"),
            ("x", "Delete the character under the cursor"),
            ("J / K", "Move the line down / up"),
            ("dd", "Delete the line, keeping it to paste"),
            ("yy", "Copy the line"),
            ("p / P", "Paste below / above"),
            ("v", "Select text: then y copies, d cuts, J / K move its lines"),
            ("V", "Select whole lines; j / k extend, J / K move them"),
            ("u / Ctrl+R", "Undo / redo"),
            ("?", "Show this help"),
            ("Esc / q", "Back to the list"),
        ],
    },
    Section {
        title: "Notes, insert mode",
        bindings: &[("Enter", "Start a new line"), ("Esc", "Back to normal mode")],
    },
    Section {
        title: "Viewing notes (v)",
        bindings: &[
            ("j / k", "Scroll down / up"),
            ("Ctrl+D / Ctrl+U", "Half a page down / up"),
            ("Space / Ctrl+B", "A page down / up"),
            ("gg / G", "Top / bottom"),
            ("i / e / Enter", "Edit the notes, then come back here"),
            ("Esc / q / v", "Back to the list"),
        ],
    },
    Section {
        title: "Viewing everything (Space V)",
        bindings: &[
            ("j / k, gg / G", "Scroll, as when viewing notes"),
            ("Esc / q / V", "Back to the list"),
        ],
    },
    Section {
        title: "Viewing the day (Space e)",
        bindings: &[
            ("j / k, gg / G", "Scroll, as when viewing notes"),
            ("Esc / q", "Back to the list"),
        ],
    },
    Section {
        title: "Selecting several (V)",
        bindings: &[
            ("j / k", "Extend the selection down / up"),
            ("x", "Complete them (or reopen if all complete)"),
            ("m", "Pin them (or unpin if all pinned)"),
            ("!", "Triage them all to the next priority"),
            ("d", "Delete them"),
            ("y", "Copy them"),
            ("H / L", "Move them to the previous / next day"),
            ("Esc / V", "Stop selecting"),
        ],
    },
    Section {
        title: "Options",
        bindings: &[
            ("j / k", "Select an option"),
            ("Space / Enter", "Turn it on or off"),
            ("/", "Search the options; Enter keeps the matches"),
            ("Esc / q / o", "Close (Esc clears a search first)"),
        ],
    },
    Section {
        title: "Workspaces",
        bindings: &[
            ("j / k", "Select a workspace"),
            ("Enter", "Open it"),
            ("a", "Create a workspace"),
            ("d", "Delete one (type its name to confirm)"),
            ("Esc / W", "Close"),
        ],
    },
    Section {
        title: "Tags",
        bindings: &[
            ("j / k", "Select a tag"),
            ("Enter", "Show every item with it"),
            ("Esc / #", "Close"),
        ],
    },
    Section {
        title: "Search",
        bindings: &[
            ("↑ / ↓", "Select a result"),
            ("Enter", "Go to the item (from s: edit its notes, on the line found)"),
            ("Esc", "Stop typing, to move with j / k (again to close)"),
            ("j / k, gg / G", "After Esc: select a result"),
            ("i / a / /", "After Esc: type again"),
            ("Ctrl+D / Ctrl+U", "Scroll the notes preview"),
            ("q", "After Esc: close the search"),
        ],
    },
    Section {
        title: "Setting a deadline (@)",
        bindings: &[
            ("h j k l / H L", "Move to the day, as in the calendar"),
            ("Enter", "Choose the day, then type a time (13:00, 1300, 1pm)"),
            ("Enter, Enter", "Just the day, with no time"),
            ("d", "Remove the deadline"),
            ("Esc", "Back to choosing the day, or cancel"),
        ],
    },
    Section {
        title: "Setting a reminder (r)",
        bindings: &[
            ("h / d / m", "Hours / days / months before, then type how many"),
            ("c", "On a day: choose it on the calendar, then a time or none"),
            ("x", "Remove the reminder"),
            ("Esc", "Back, or cancel"),
        ],
    },
    Section {
        title: "Timeline (Space t)",
        bindings: &[
            ("j / k", "Scroll an hour down / up"),
            ("h / l", "Previous / next day"),
            ("t", "Today"),
            ("p", "Plan the day"),
            ("Esc / q", "Back to the list"),
        ],
    },
    Section {
        title: "Planning the day (Space p)",
        bindings: &[
            ("j / k", "Choose an item, on the list or the timeline"),
            ("L / H", "Move into the timeline's blocks / back to the list"),
            ("j / k (timeline)", "Down / up the column of blocks"),
            ("h / l (timeline)", "The column to the left / right"),
            ("Enter", "Type its start time, then its end, then on to the next without one"),
            ("Enter (empty)", "Start as the last one ends (or now); end an hour later"),
            ("Tab", "While typing: leave this item, on to the next"),
            ("a / A", "Add an item / several, as on the list"),
            ("o / O", "Add a sub-item / several under the item"),
            ("e / E", "Edit / edit with the text cleared, as on the list"),
            ("u / Ctrl+R", "Undo / redo a change in the plan"),
            ("J / K, yy, p / P", "Move, copy and paste, as on the list"),
            ("> / <, za", "Indent / outdent, fold (zc, zo too)"),
            ("m / @ / r", "Pin, deadline, reminder"),
            ("v / V", "Select several (not to another day)"),
            ("c", "Clear the item's time"),
            ("x / d", "Delete the item, asking first"),
            ("!", "Triage: High, Medium, Low, then none"),
            ("w", "Save the plan"),
            ("R", "Reset: clear every time, all day again"),
            ("Esc / q", "Leave, asking to save if anything changed (w save, q don't, n back)"),
        ],
    },
    Section {
        title: "Calendar",
        bindings: &[
            ("h j k l", "Move by day and week, following the layout"),
            ("H / L", "Previous / next month"),
            ("t", "Jump to today"),
            ("w / m / y", "Week / month / year view"),
            ("a", "Add an item to the selected day"),
            ("Enter", "Open the selected day's list"),
            ("u / Ctrl+R", "Undo / redo"),
            ("?", "Show this help"),
            ("Esc / q / c", "Back to the list"),
        ],
    },
    Section {
        title: "Anywhere",
        bindings: &[("Ctrl+C", "Quit")],
    },
];

/// A section with only the bindings that match the search.
/// One binding as the help shows it: its keys, what it does, and for the
/// list's and leader menu's, the command's name in `keys.toml`.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub keys: String,
    pub action: String,
    pub name: Option<&'static str>,
}

impl Row {
    fn new(keys: impl Into<String>, action: impl Into<String>, name: Option<&'static str>) -> Self {
        Self { keys: keys.into(), action: action.into(), name }
    }
}

/// A section of the help, with its rows.
#[derive(Clone, Debug, PartialEq)]
pub struct Shown {
    pub title: String,
    pub bindings: Vec<Row>,
}

/// `:` commands that do the same as a command's keys.
fn colon_alias(command: Command) -> Option<&'static str> {
    match command {
        Command::Deadline => Some(":deadline"),
        Command::Reminder => Some(":remind"),
        Command::Overdue => Some(":overdue"),
        Command::Quit => Some(":q"),
        _ => None,
    }
}

/// Every section of the help, with the list's and the leader menu's keys
/// as `keymap` has them, so changes in keys.toml show.
pub fn sections(keymap: &Keymap) -> Vec<Shown> {
    let shown = |bindings: &[keymap::Binding], command: Command| -> Vec<String> {
        bindings.iter().filter(|binding| binding.command == command).map(|binding| keymap::show_keys(&binding.keys)).collect()
    };
    let mut list = Vec::new();
    for command in Command::all() {
        let mut keys = shown(&keymap.list, command);
        if keys.is_empty() {
            continue;
        }
        keys.extend(colon_alias(command).map(String::from));
        list.push(Row::new(keys.join(" / "), command.description(), Some(command.name())));
        // Counts, with the keys they go before.
        let first = |command| shown(&keymap.list, command).into_iter().next();
        if command == Command::Up
            && let (Some(down), Some(up)) = (first(Command::Down), first(Command::Up))
        {
            list.push(Row::new(format!("4{down} / 4{up}"), "Move four items down / up (any number)", None));
        }
        if command == Command::Last
            && let Some(last) = first(Command::Last)
        {
            list.push(Row::new(format!("42{last} / :42"), "Go to item 42", None));
        }
    }
    let leader = keymap.leader_name();
    let menu = Command::all()
        .flat_map(|command| {
            keymap
                .leader
                .iter()
                .filter(move |binding| binding.command == command)
                .map(move |binding| (binding, command))
                .collect::<Vec<_>>()
        })
        .map(|(binding, command)| {
            Row::new(format!("{leader} {}", keymap::show_spaced(&binding.keys)), command.description(), Some(command.name()))
        })
        .collect();
    let mut all = vec![
        Shown { title: "List".into(), bindings: list },
        Shown { title: format!("Leader menu ({leader})"), bindings: menu },
    ];
    all.extend(SECTIONS.iter().map(|section| Shown {
        title: section.title.into(),
        bindings: section.bindings.iter().map(|(keys, action)| Row::new(*keys, *action, None)).collect(),
    }));
    all.retain(|section| !section.bindings.is_empty());
    all
}

#[derive(Default)]
pub struct Help {
    /// Every section, with the keys as they were when it opened.
    pub sections: Vec<Shown>,
    /// Where to change the keys, shown above them.
    pub tip: String,
    pub query: String,
    /// First visible line of the results.
    pub scroll: usize,
    /// How many result lines fit on screen and how many there are once
    /// wrapped, recorded when the popup is drawn, so scrolling stops at the
    /// last page.
    pub height: Cell<usize>,
    pub total: Cell<usize>,
}

impl Help {
    pub fn new(keymap: &Keymap) -> Self {
        let options = keymap.key_for(Command::Options).map_or_else(|| "the options".into(), |key| format!("the options ({key})"));
        let tip = format!("Change the list's and the leader menu's keys in keys.toml: open it from {options}, under Keys.");
        Self { sections: sections(keymap), tip, ..Self::default() }
    }

    /// Handles a key and returns `true` when the popup should close.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => return true,
            KeyCode::Up => self.scroll_by(-1),
            KeyCode::Down => self.scroll_by(1),
            KeyCode::Char('p' | 'k') if ctrl => self.scroll_by(-1),
            KeyCode::Char('n' | 'j') if ctrl => self.scroll_by(1),
            KeyCode::PageUp => self.scroll_by(-(self.height.get().max(1) as isize)),
            KeyCode::PageDown => self.scroll_by(self.height.get().max(1) as isize),
            KeyCode::Char('u') if ctrl => self.set_query(String::new()),
            KeyCode::Backspace => {
                let mut query = self.query.clone();
                query.pop();
                self.set_query(query);
            }
            KeyCode::Char(c) if !ctrl => self.set_query(format!("{}{c}", self.query)),
            _ => {}
        }
        false
    }

    pub fn matches(&self) -> Vec<Shown> {
        self.sections
            .iter()
            .map(|section| Shown {
                title: section.title.clone(),
                bindings: section.bindings.iter().filter(|row| matches(row, &self.query)).cloned().collect(),
            })
            .filter(|section| !section.bindings.is_empty())
            .collect()
    }

    /// Number of result lines when nothing wraps: a title per section, then
    /// its bindings, with a blank line between sections.
    #[cfg(test)]
    pub fn line_count(&self) -> usize {
        let matches = self.matches();
        let lines: usize = matches.iter().map(|section| section.bindings.len() + 1).sum();
        lines + matches.len().saturating_sub(1)
    }

    fn set_query(&mut self, query: String) {
        self.query = query;
        self.scroll = 0;
    }

    fn scroll_by(&mut self, delta: isize) {
        let max = self.total.get().saturating_sub(self.height.get());
        self.scroll = self.scroll.saturating_add_signed(delta).min(max);
    }
}

/// A single character is almost always a key, and nearly every description
/// contains common letters, so it only matches key names (case-sensitively, so
/// `G` and `g` differ). Longer searches match keys, descriptions or command
/// names, ignoring case.
fn matches(row: &Row, query: &str) -> bool {
    let query = query.trim();
    let mut chars = query.chars();
    match (chars.next(), chars.next()) {
        (None, _) => true,
        (Some(c), None) => row.keys.split([' ', '/']).any(|key| key.starts_with(c)),
        _ => {
            let query = query.to_lowercase();
            [row.keys.as_str(), row.action.as_str(), row.name.unwrap_or_default()]
                .iter()
                .any(|text| text.to_lowercase().contains(&query))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn help(query: &str) -> Help {
        Help { query: query.into(), ..Help::new(&Keymap::default()) }
    }

    /// The matching bindings as "section: keys".
    fn found(query: &str) -> Vec<String> {
        help(query)
            .matches()
            .iter()
            .flat_map(|section| section.bindings.iter().map(|row| format!("{}: {}", section.title, row.keys)).collect::<Vec<_>>())
            .collect()
    }

    fn press(help: &mut Help, code: KeyCode) -> bool {
        help.handle_key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[test]
    fn empty_search_shows_everything() {
        let total: usize = sections(&Keymap::default()).iter().map(|section| section.bindings.len()).sum();
        let shown: usize = help("").matches().iter().map(|section| section.bindings.len()).sum();
        assert_eq!(shown, total);
    }

    #[test]
    fn single_key_matches_key_names_only() {
        assert_eq!(found("x"), ["List: x", "Notes: x", "Selecting several (V): x", "Setting a reminder (r): x", "Planning the day (Space p): x / d"]);
        assert_eq!(found("?"), ["List: ?", "Notes: ?", "Calendar: ?"]);
        assert_eq!(found("$"), ["Notes: 0 / $"]);
        assert!(found("v").contains(&"List: V / v".to_string()));
        assert!(found("v").contains(&"Viewing notes (v): Esc / q / v".to_string()));
    }

    #[test]
    fn single_key_search_is_case_sensitive() {
        assert_eq!(
            found("G"),
            [
                "List: G",
                "Notes: gg / G",
                "Viewing notes (v): gg / G",
                "Viewing everything (Space V): j / k, gg / G",
                "Viewing the day (Space e): j / k, gg / G",
                "Search: j / k, gg / G"
            ]
        );
        assert_eq!(found("O"), ["List: O", "Notes: o / O", "Planning the day (Space p): o / O"]);
        assert!(found("g").contains(&"Notes: gg / G".to_string()));
        assert!(!found("A").contains(&"List: a".to_string()));
    }

    #[test]
    fn longer_search_matches_descriptions_ignoring_case() {
        assert_eq!(found("UNDO"), ["List: u", "Notes: u / Ctrl+R", "Planning the day (Space p): u / Ctrl+R", "Calendar: u / Ctrl+R"]);
        assert_eq!(found("next day"), ["List: l / →", "List: L", "Selecting several (V): H / L", "Timeline (Space t): h / l"]);
    }

    #[test]
    fn longer_search_matches_key_names_ignoring_case() {
        assert_eq!(
            found("ctrl"),
            [
                "List: Ctrl+R",
                "Notes: Ctrl+→ / Ctrl+←",
                "Notes: Ctrl+Home / End",
                "Notes: u / Ctrl+R",
                "Viewing notes (v): Ctrl+D / Ctrl+U",
                "Viewing notes (v): Space / Ctrl+B",
                "Search: Ctrl+D / Ctrl+U",
                "Planning the day (Space p): u / Ctrl+R",
                "Calendar: u / Ctrl+R",
                "Anywhere: Ctrl+C",
            ]
        );
        assert!(found("esc").contains(&"Delete popup: c / Esc".to_string()));
        assert!(found("dd").contains(&"Notes: dd".to_string()));
    }

    #[test]
    fn no_match_is_empty() {
        assert!(help("zzz").matches().is_empty());
        assert_eq!(help("zzz").line_count(), 0);
    }

    #[test]
    fn typing_edits_the_search_and_resets_scrolling() {
        let mut h = help("");
        h.height.set(5);
        h.total.set(h.line_count());
        press(&mut h, KeyCode::Down);
        assert_eq!(h.scroll, 1);
        for c in "undx".chars() {
            press(&mut h, KeyCode::Char(c));
        }
        assert_eq!(h.scroll, 0);
        press(&mut h, KeyCode::Backspace);
        assert_eq!(h.query, "und");
        h.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert_eq!(h.query, "");
    }

    #[test]
    fn command_keys_are_typed_into_the_search() {
        let mut h = help("");
        for c in "jkq".chars() {
            assert!(!press(&mut h, KeyCode::Char(c)));
        }
        assert_eq!(h.query, "jkq");
        assert_eq!(h.scroll, 0);
    }

    #[test]
    fn scrolling_stops_at_both_ends() {
        let mut h = help("");
        h.height.set(10);
        h.total.set(h.line_count());
        let max = h.line_count() - 10;
        press(&mut h, KeyCode::Up);
        assert_eq!(h.scroll, 0);
        // More presses than there are lines, so it must stop at the end.
        for _ in 0..1000 {
            press(&mut h, KeyCode::Down);
        }
        assert_eq!(h.scroll, max);
        press(&mut h, KeyCode::PageUp);
        assert_eq!(h.scroll, max - 10);
        h.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
        assert_eq!(h.scroll, max - 11);
    }

    #[test]
    fn short_results_do_not_scroll() {
        let mut h = help("undo");
        h.height.set(h.line_count() + 1);
        h.total.set(h.line_count());
        press(&mut h, KeyCode::Down);
        assert_eq!(h.scroll, 0);
    }

    #[test]
    fn esc_closes() {
        assert!(press(&mut help("abc"), KeyCode::Esc));
        assert!(!press(&mut help(""), KeyCode::Enter));
    }

    #[test]
    fn the_list_and_leader_menu_show_the_keys_as_keys_toml_has_them() {
        let (keymap, problems) = Keymap::from_toml(
            r#"
            leader = ","
            [list]
            "c" = "calendar"
            "x" = "none"
            [leader-menu]
            "f" = "find-items"
            "#,
        );
        assert!(problems.is_empty(), "{problems:?}");
        let help = Help { query: "calendar".into(), ..Help::new(&keymap) };
        let found: Vec<String> =
            help.matches().iter().flat_map(|section| section.bindings.iter().map(|row| format!("{}: {}", section.title, row.keys)).collect::<Vec<_>>()).collect();
        assert!(found.contains(&"List: c".to_string()), "{found:?}");
        assert!(found.contains(&"Leader menu (,): , c".to_string()), "{found:?}");
        // A freed key is gone, and the leader's own key shows.
        let all = sections(&keymap);
        assert!(!all[0].bindings.iter().any(|row| row.name == Some("toggle-done")));
        assert!(all[0].bindings.iter().any(|row| row.keys == "," && row.name == Some("leader")));
        assert!(all[1].bindings.iter().any(|row| row.keys == ", f"));
    }

    #[test]
    fn command_names_are_shown_and_searchable() {
        let rows = &sections(&Keymap::default())[0].bindings;
        let done = rows.iter().find(|row| row.keys == "x").unwrap();
        assert_eq!(done.name, Some("toggle-done"));
        assert_eq!(found("toggle-done"), ["List: x"]);
        // : commands show beside their keys.
        assert!(rows.iter().any(|row| row.keys == "@ / :deadline"));
        assert!(rows.iter().any(|row| row.keys == "4j / 4k"));
        assert!(rows.iter().any(|row| row.keys == "42G / :42"));
    }
}
