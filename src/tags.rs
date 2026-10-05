//! `#tags` written in items' text, and the `#` popup that lists them.

use std::cell::Cell;
use std::ops::Range;

use ratatui::crossterm::event::{KeyCode, KeyEvent};

use crate::store::Store;

/// Every tag in `text`: where it is (including the `#`) and its name in lower
/// case. A tag is a `#` at the start of a word followed by letters, digits,
/// `-` or `_`, so "#work" is one but "issue#4" and a lone "#" aren't.
pub fn find_tags(text: &str) -> Vec<(Range<usize>, String)> {
    let mut tags = Vec::new();
    let mut previous: Option<char> = None;
    let mut chars = text.char_indices().peekable();
    while let Some((start, c)) = chars.next() {
        let at_word_start = previous.is_none_or(char::is_whitespace);
        previous = Some(c);
        if c != '#' || !at_word_start {
            continue;
        }
        let mut end = start + 1;
        while let Some(&(i, next)) = chars.peek() {
            if !(next.is_alphanumeric() || next == '-' || next == '_') {
                break;
            }
            end = i + next.len_utf8();
            previous = Some(next);
            chars.next();
        }
        if end > start + 1 {
            tags.push((start..end, text[start + 1..end].to_lowercase()));
        }
    }
    tags
}

/// Whether `text` has the tag `name` (given in lower case, without the `#`).
pub fn has_tag(text: &str, name: &str) -> bool {
    find_tags(text).iter().any(|(_, tag)| tag == name)
}

/// Every tag used on any day, with how many items have it, most used first.
pub fn all_tags(store: &Store) -> Vec<(String, usize)> {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for (_, items) in store.all() {
        for item in items {
            let mut names: Vec<String> = find_tags(&item.text).into_iter().map(|(_, name)| name).collect();
            names.dedup();
            for name in names {
                match counts.iter_mut().find(|(tag, _)| *tag == name) {
                    Some((_, count)) => *count += 1,
                    None => counts.push((name, 1)),
                }
            }
        }
    }
    counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    counts
}

/// The `#` popup listing every tag.
#[derive(Default)]
pub struct TagPicker {
    pub selected: usize,
    /// The first tag on screen, kept by the UI between frames.
    pub offset: Cell<usize>,
}

/// What the app should do after the tag list handles a key.
#[derive(Debug, PartialEq)]
pub enum Action {
    Stay,
    Close,
    /// Show the items with this tag.
    Open(String),
}

impl TagPicker {
    pub fn handle_key(&mut self, key: KeyEvent, tags: &[(String, usize)]) -> Action {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.selected = (self.selected + 1).min(tags.len().saturating_sub(1)),
            KeyCode::Char('k') | KeyCode::Up => self.selected = self.selected.saturating_sub(1),
            KeyCode::Char('g') => self.selected = 0,
            KeyCode::Char('G') => self.selected = tags.len().saturating_sub(1),
            KeyCode::Enter => {
                if let Some((name, _)) = tags.get(self.selected) {
                    return Action::Open(name.clone());
                }
            }
            KeyCode::Esc | KeyCode::Char('q' | '#') => return Action::Close,
            _ => {}
        }
        Action::Stay
    }
}

#[cfg(test)]
mod tests {
    use ratatui::crossterm::event::KeyModifiers;

    use super::*;
    use crate::test_util::today;

    fn names(text: &str) -> Vec<String> {
        find_tags(text).into_iter().map(|(_, name)| name).collect()
    }

    #[test]
    fn tags_start_a_word_with_a_hash() {
        assert_eq!(names("Call #work-team about the #budget"), ["work-team", "budget"]);
        assert_eq!(names("#home first"), ["home"]);
        assert_eq!(names("ends with #tag."), ["tag"]);
        assert_eq!(names("snake #a_b and #Q3"), ["a_b", "q3"]);
    }

    #[test]
    fn a_hash_inside_a_word_or_on_its_own_is_not_a_tag() {
        assert!(names("issue#4 and C# and # alone and ##").is_empty());
    }

    #[test]
    fn tags_are_lower_case_and_their_ranges_include_the_hash() {
        let text = "Café #Zoë-Day";
        let tags = find_tags(text);
        assert_eq!(tags[0].1, "zoë-day");
        assert_eq!(&text[tags[0].0.clone()], "#Zoë-Day");
    }

    #[test]
    fn has_tag_ignores_case() {
        assert!(has_tag("Plan the #Holiday", "holiday"));
        assert!(!has_tag("Plan the #holidays", "holiday"));
    }

    #[test]
    fn all_tags_counts_items_on_every_day() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("todos.json")).unwrap();
        store.insert(today(), 0, "#work report".into()).unwrap();
        store.insert(today(), 1, "#Work call #home".into()).unwrap();
        store.insert(today().succ_opt().unwrap(), 0, "#work #work twice".into()).unwrap();
        store.insert(today().succ_opt().unwrap(), 1, "#garden".into()).unwrap();
        assert_eq!(
            all_tags(&store),
            [("work".to_string(), 3), ("garden".to_string(), 1), ("home".to_string(), 1)]
        );
    }

    #[test]
    fn picker_keys_move_open_and_close() {
        let tags = vec![("a".to_string(), 2), ("b".to_string(), 1)];
        let mut picker = TagPicker::default();
        let mut press = |code| picker.handle_key(KeyEvent::new(code, KeyModifiers::NONE), &tags);
        press(KeyCode::Char('j'));
        press(KeyCode::Char('j'));
        assert_eq!(press(KeyCode::Enter), Action::Open("b".into()));
        press(KeyCode::Char('k'));
        assert_eq!(press(KeyCode::Enter), Action::Open("a".into()));
        assert_eq!(press(KeyCode::Char('#')), Action::Close);
        assert_eq!(press(KeyCode::Esc), Action::Close);
        assert_eq!(TagPicker::default().handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &[]), Action::Stay);
    }
}
