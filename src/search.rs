//! Fuzzy search across every day's items, and optionally their notes.

use std::cell::Cell;

use chrono::NaiveDate;
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::input::LineInput;
use crate::store::Store;
use crate::tags::has_tag;

pub struct Search {
    pub input: LineInput,
    /// Whether notes are searched too (`S`), not just the items' text (`s`).
    pub notes: bool,
    /// Only items with this tag (lower case, without the `#`), if any.
    pub tag: Option<String>,
    /// The selected result.
    pub selected: usize,
    /// The first result on screen, kept by the UI between frames.
    pub offset: Cell<usize>,
}

/// An item that matches the search.
#[derive(Debug, PartialEq)]
pub struct Hit {
    pub day: NaiveDate,
    pub index: usize,
    pub score: u32,
    /// Which characters of the item's text matched, counted in graphemes.
    pub text_matches: Vec<usize>,
    /// The line of the notes that matched, and which of its characters, when
    /// the notes matched better than the text (or the text didn't match).
    pub note: Option<(String, Vec<usize>)>,
}

/// What the app should do after the search handles a key.
#[derive(Debug, PartialEq)]
pub enum Action {
    Stay,
    Close,
    /// Go to this item on the list.
    Open { day: NaiveDate, index: usize },
}

impl Search {
    pub fn new(notes: bool) -> Self {
        Self { input: LineInput::default(), notes, tag: None, selected: 0, offset: Cell::new(0) }
    }

    /// Every item with the tag `name`, narrowed down by typing.
    pub fn for_tag(name: String) -> Self {
        Self { tag: Some(name), ..Self::new(false) }
    }

    /// Handles a key, given how many results there are now. Every typed
    /// character goes into the query; arrows (or Ctrl+N/P, Ctrl+J/K) move the
    /// selection.
    pub fn handle_key(&mut self, key: KeyEvent, hits: &[Hit]) -> Action {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => return Action::Close,
            KeyCode::Enter => {
                return match hits.get(self.selected) {
                    Some(hit) => Action::Open { day: hit.day, index: hit.index },
                    None => Action::Stay,
                };
            }
            KeyCode::Up => self.select_by(-1, hits.len()),
            KeyCode::Down => self.select_by(1, hits.len()),
            KeyCode::Char('p' | 'k') if ctrl => self.select_by(-1, hits.len()),
            KeyCode::Char('n' | 'j') if ctrl => self.select_by(1, hits.len()),
            KeyCode::Char(_) if ctrl => {}
            code => {
                let before = self.input.text.clone();
                self.input.handle_key(code);
                if self.input.text != before {
                    self.selected = 0;
                    self.offset.set(0);
                }
            }
        }
        Action::Stay
    }

    /// Adds pasted text to the query, on one line.
    pub fn paste(&mut self, text: &str) {
        self.input.paste(text);
        self.selected = 0;
        self.offset.set(0);
    }

    pub fn find(&self, store: &Store, today: NaiveDate) -> Vec<Hit> {
        find(store, &self.input.text, self.notes, self.tag.as_deref(), today)
    }

    fn select_by(&mut self, delta: isize, len: usize) {
        self.selected = self.selected.saturating_add_signed(delta).min(len.saturating_sub(1));
    }
}

/// Every item matching `query`, best first. Equally good matches are ordered
/// by how close their day is to `today`. With a `tag`, only items with it
/// count, and an empty query matches all of them; otherwise an empty query
/// matches nothing.
pub fn find(store: &Store, query: &str, notes: bool, tag: Option<&str>, today: NaiveDate) -> Vec<Hit> {
    let empty = query.trim().is_empty();
    if empty && tag.is_none() {
        return Vec::new();
    }
    let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
    let mut matcher = Matcher::new(Config::DEFAULT);
    let mut matches = |text: &str| {
        let mut buf = Vec::new();
        let mut indices = Vec::new();
        let score = pattern.indices(Utf32Str::new(text, &mut buf), &mut matcher, &mut indices)?;
        indices.sort_unstable();
        indices.dedup();
        Some((score, indices.into_iter().map(|i| i as usize).collect::<Vec<_>>()))
    };

    let mut hits = Vec::new();
    for (day, items) in store.all() {
        for (index, item) in items.iter().enumerate() {
            if tag.is_some_and(|tag| !has_tag(&item.text, tag)) {
                continue;
            }
            if empty {
                hits.push(Hit { day, index, score: 0, text_matches: Vec::new(), note: None });
                continue;
            }
            let text = matches(&item.text);
            let note = if notes {
                item.notes
                    .lines()
                    .filter_map(|line| matches(line).map(|(score, indices)| (score, line, indices)))
                    .max_by_key(|(score, ..)| *score)
            } else {
                None
            };
            let text_score = text.as_ref().map(|(score, _)| *score);
            let note = note.filter(|(score, ..)| text_score.is_none_or(|text| *score > text));
            let score = match (&text, &note) {
                (_, Some((score, ..))) => *score,
                (Some((score, _)), None) => *score,
                (None, None) => continue,
            };
            hits.push(Hit {
                day,
                index,
                score,
                text_matches: text.map(|(_, indices)| indices).unwrap_or_default(),
                note: note.map(|(_, line, indices)| (line.to_string(), indices)),
            });
        }
    }
    hits.sort_by_key(|hit| (std::cmp::Reverse(hit.score), (hit.day - today).num_days().abs(), hit.day, hit.index));
    hits
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::today;

    fn day(offset: i64) -> NaiveDate {
        today() + chrono::Duration::days(offset)
    }

    /// A store with a few items over several days.
    fn store() -> (Store, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("todos.json")).unwrap();
        store.insert(today(), 0, "Buy milk".into()).unwrap();
        store.insert(today(), 1, "Write the quarterly report".into()).unwrap();
        store.set_notes(today(), 1, "Ask Sam for the Q3 numbers\nCharts from the dashboard".into()).unwrap();
        store.insert(day(-3), 0, "Buy stamps".into()).unwrap();
        store.insert(day(10), 0, "Dentist at 3pm".into()).unwrap();
        store.insert(day(2), 0, "Call mum".into()).unwrap();
        store.toggle_done(day(2), 0).unwrap();
        store.insert(day(1), 0, "Café with Zoë".into()).unwrap();
        (store, dir)
    }

    fn texts(store: &Store, hits: &[Hit]) -> Vec<String> {
        hits.iter().map(|hit| store.items(hit.day)[hit.index].text.clone()).collect()
    }

    #[test]
    fn a_tag_alone_finds_every_item_with_it_closest_first() {
        let (mut store, _dir) = store();
        store.insert(day(-3), 1, "Renew #car insurance".into()).unwrap();
        store.insert(day(1), 1, "Book #Car service".into()).unwrap();
        store.insert(day(1), 2, "#carpool rota".into()).unwrap();
        let hits = find(&store, "", false, Some("car"), today());
        assert_eq!(texts(&store, &hits), ["Book #Car service", "Renew #car insurance"]);
        // Typing narrows them down.
        assert_eq!(texts(&store, &find(&store, "renew", false, Some("car"), today())), ["Renew #car insurance"]);
    }

    #[test]
    fn an_empty_query_finds_nothing() {
        let (store, _dir) = store();
        assert!(find(&store, "", false, None, today()).is_empty());
        assert!(find(&store, "   ", false, None, today()).is_empty());
    }

    #[test]
    fn letters_match_in_order_with_gaps() {
        let (store, _dir) = store();
        let hits = find(&store, "bmlk", false, None, today());
        assert_eq!(texts(&store, &hits), ["Buy milk"]);
        assert_eq!(hits[0].text_matches, [0, 4, 6, 7]);
    }

    #[test]
    fn it_searches_every_day_including_completed_items() {
        let (store, _dir) = store();
        assert_eq!(texts(&store, &find(&store, "dentist", false, None, today())), ["Dentist at 3pm"]);
        assert_eq!(texts(&store, &find(&store, "mum", false, None, today())), ["Call mum"]);
    }

    #[test]
    fn equal_matches_are_ordered_by_closeness_to_today() {
        let (store, _dir) = store();
        // Both "Buy ..." items score the same for "buy".
        assert_eq!(texts(&store, &find(&store, "buy", false, None, today())), ["Buy milk", "Buy stamps"]);
        assert_eq!(texts(&store, &find(&store, "buy", false, None, day(-3))), ["Buy stamps", "Buy milk"]);
    }

    #[test]
    fn case_is_ignored_unless_the_query_has_capitals() {
        let (store, _dir) = store();
        assert_eq!(texts(&store, &find(&store, "dentist", false, None, today())), ["Dentist at 3pm"]);
        assert!(find(&store, "DENTIST", false, None, today()).is_empty());
    }

    #[test]
    fn accents_match_plain_letters() {
        let (store, _dir) = store();
        let hits = find(&store, "cafe zoe", false, None, today());
        assert_eq!(texts(&store, &hits), ["Café with Zoë"]);
        assert_eq!(hits[0].text_matches, [0, 1, 2, 3, 10, 11, 12]);
    }

    #[test]
    fn words_match_separately() {
        let (store, _dir) = store();
        assert_eq!(texts(&store, &find(&store, "3pm dent", false, None, today())), ["Dentist at 3pm"]);
    }

    #[test]
    fn notes_are_only_searched_when_asked() {
        let (store, _dir) = store();
        assert!(find(&store, "dashboard", false, None, today()).is_empty());
        let hits = find(&store, "dashboard", true, None, today());
        assert_eq!(texts(&store, &hits), ["Write the quarterly report"]);
        let (line, indices) = hits[0].note.as_ref().unwrap();
        assert_eq!(line, "Charts from the dashboard");
        assert_eq!(indices, &(16..25).collect::<Vec<_>>());
        assert!(hits[0].text_matches.is_empty());
    }

    #[test]
    fn a_better_text_match_hides_the_note() {
        let (store, _dir) = store();
        let hits = find(&store, "report", true, None, today());
        assert_eq!(texts(&store, &hits), ["Write the quarterly report"]);
        assert!(hits[0].note.is_none());
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn typing_builds_the_query_and_resets_the_selection() {
        let (store, _dir) = store();
        let mut search = Search::new(false);
        for c in "bu".chars() {
            search.handle_key(key(KeyCode::Char(c)), &[]);
        }
        let hits = search.find(&store, today());
        search.handle_key(key(KeyCode::Down), &hits);
        assert_eq!(search.selected, 1);
        search.handle_key(key(KeyCode::Char('y')), &hits);
        assert_eq!(search.input.text, "buy");
        assert_eq!(search.selected, 0);
        // Command keys are just text here.
        for c in "jkq".chars() {
            assert_eq!(search.handle_key(key(KeyCode::Char(c)), &hits), Action::Stay);
        }
        assert_eq!(search.input.text, "buyjkq");
    }

    #[test]
    fn the_selection_stays_within_the_results() {
        let (store, _dir) = store();
        let mut search = Search::new(false);
        search.handle_key(key(KeyCode::Char('u')), &[]);
        let hits = search.find(&store, today());
        assert!(hits.len() >= 2);
        search.handle_key(key(KeyCode::Up), &hits);
        assert_eq!(search.selected, 0);
        for _ in 0..20 {
            search.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL), &hits);
        }
        assert_eq!(search.selected, hits.len() - 1);
        search.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL), &hits);
        assert_eq!(search.selected, hits.len() - 2);
    }

    #[test]
    fn enter_opens_the_selected_result_and_esc_closes() {
        let (store, _dir) = store();
        let mut search = Search::new(false);
        search.input = LineInput::new("dentist");
        let hits = search.find(&store, today());
        assert_eq!(search.handle_key(key(KeyCode::Enter), &hits), Action::Open { day: day(10), index: 0 });
        assert_eq!(search.handle_key(key(KeyCode::Enter), &[]), Action::Stay);
        assert_eq!(search.handle_key(key(KeyCode::Esc), &hits), Action::Close);
    }
}
