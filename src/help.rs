//! The `?` popup: every keybinding, searchable by key or by what it does.

use std::cell::Cell;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub struct Section {
    pub title: &'static str,
    /// `(keys, action)` pairs. Keys are separated by spaces or ` / `.
    pub bindings: &'static [(&'static str, &'static str)],
}

/// Every keybinding in the app. Keep it in sync with the key handling.
pub const SECTIONS: &[Section] = &[
    Section {
        title: "List",
        bindings: &[
            ("h / l", "Previous / next day"),
            ("j / k", "Move down / up"),
            ("J / K", "Move the item down / up the list"),
            ("> / <", "Move the item to the next / previous day"),
            ("a", "Add an item below the cursor"),
            ("e", "Edit the selected item"),
            ("x", "Mark the selected item done / not done"),
            ("p", "Pin / unpin: move to today until done"),
            ("d", "Delete the selected item"),
            ("Enter", "Open the selected item's notes"),
            ("u / Ctrl+R", "Undo / redo a change to the list"),
            ("?", "Show this help"),
            ("q", "Quit"),
        ],
    },
    Section {
        title: "Adding or editing an item",
        bindings: &[
            ("← / →", "Move the cursor"),
            ("Home / End", "Jump to the start / end of the line"),
            ("Backspace / Delete", "Delete before / under the cursor"),
            ("Enter / Esc", "Save"),
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
            ("0 / $", "Start / end of the line"),
            ("gg / G", "First / last line"),
            ("i / a", "Insert before / after the cursor"),
            ("I / A", "Insert at the start / end of the line"),
            ("o / O", "Open a new line below / above"),
            ("x", "Delete the character under the cursor"),
            ("dd", "Delete the line"),
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
        title: "Anywhere",
        bindings: &[("Ctrl+C", "Quit")],
    },
];

/// A section with only the bindings that match the search.
pub struct Matches {
    pub title: &'static str,
    pub bindings: Vec<(&'static str, &'static str)>,
}

#[derive(Default)]
pub struct Help {
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

    pub fn matches(&self) -> Vec<Matches> {
        SECTIONS
            .iter()
            .map(|section| Matches {
                title: section.title,
                bindings: section
                    .bindings
                    .iter()
                    .copied()
                    .filter(|&(keys, action)| matches(keys, action, &self.query))
                    .collect(),
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
/// `G` and `g` differ). Longer searches match keys or descriptions, ignoring case.
fn matches(keys: &str, action: &str, query: &str) -> bool {
    let query = query.trim();
    let mut chars = query.chars();
    match (chars.next(), chars.next()) {
        (None, _) => true,
        (Some(c), None) => keys.split([' ', '/']).any(|key| key.starts_with(c)),
        _ => {
            let query = query.to_lowercase();
            keys.to_lowercase().contains(&query) || action.to_lowercase().contains(&query)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn help(query: &str) -> Help {
        Help { query: query.into(), ..Help::default() }
    }

    /// The matching bindings as "section: keys".
    fn found(query: &str) -> Vec<String> {
        help(query)
            .matches()
            .iter()
            .flat_map(|section| section.bindings.iter().map(move |(keys, _)| format!("{}: {keys}", section.title)))
            .collect()
    }

    fn press(help: &mut Help, code: KeyCode) -> bool {
        help.handle_key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[test]
    fn empty_search_shows_everything() {
        let total: usize = SECTIONS.iter().map(|section| section.bindings.len()).sum();
        let shown: usize = help("").matches().iter().map(|section| section.bindings.len()).sum();
        assert_eq!(shown, total);
    }

    #[test]
    fn single_key_matches_key_names_only() {
        assert_eq!(found("x"), ["List: x", "Notes: x"]);
        assert_eq!(found("?"), ["List: ?", "Notes: ?"]);
        assert_eq!(found("$"), ["Notes: 0 / $"]);
    }

    #[test]
    fn single_key_search_is_case_sensitive() {
        assert_eq!(found("G"), ["Notes: gg / G"]);
        assert_eq!(found("O"), ["Notes: o / O"]);
        assert!(found("g").contains(&"Notes: gg / G".to_string()));
        assert!(!found("A").contains(&"List: a".to_string()));
    }

    #[test]
    fn longer_search_matches_descriptions_ignoring_case() {
        assert_eq!(found("UNDO"), ["List: u / Ctrl+R", "Notes: u / Ctrl+R"]);
        assert_eq!(found("next day"), ["List: h / l"]);
    }

    #[test]
    fn longer_search_matches_key_names_ignoring_case() {
        assert_eq!(found("ctrl"), ["List: u / Ctrl+R", "Notes: u / Ctrl+R", "Anywhere: Ctrl+C"]);
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
        for _ in 0..100 {
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
        h.height.set(10);
        h.total.set(h.line_count());
        press(&mut h, KeyCode::Down);
        assert_eq!(h.scroll, 0);
    }

    #[test]
    fn esc_closes() {
        assert!(press(&mut help("abc"), KeyCode::Esc));
        assert!(!press(&mut help(""), KeyCode::Enter));
    }
}
