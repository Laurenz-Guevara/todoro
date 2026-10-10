//! Fuzzy search across every day: finding items by their text (`Space
//! Space`, like finding files), or searching inside their notes (`s`, like
//! grep).

use std::cell::Cell;

use chrono::NaiveDate;
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::input::LineInput;
use crate::store::Store;
use crate::tags::has_tag;

/// What a search looks through.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// The items' text.
    Items,
    /// The items' notes, line by line.
    Notes,
}

pub struct Search {
    pub input: LineInput,
    pub kind: Kind,
    /// Only items with this tag (lower case, without the `#`), if any.
    pub tag: Option<String>,
    /// The selected result.
    pub selected: usize,
    /// The first result on screen, kept by the UI between frames.
    pub offset: Cell<usize>,
    /// Whether keys go into the query. `Esc` stops typing, to move through
    /// the results with `j`/`k` as in vim, and `i` starts again.
    pub typing: bool,
    /// The first line of the selected item's notes shown in the preview.
    pub preview_scroll: usize,
    /// How many preview lines fit and how many there are, recorded when
    /// drawn, so scrolling stops at the end. Zero with no preview.
    pub preview_height: Cell<usize>,
    pub preview_total: Cell<usize>,
    /// Whether `g` was pressed, waiting for a second `g`.
    pending_g: bool,
}

/// An item that matches the search.
#[derive(Debug, PartialEq)]
pub struct Hit {
    pub day: NaiveDate,
    pub index: usize,
    pub score: u32,
    /// Which characters of the item's text matched, counted in graphemes.
    pub text_matches: Vec<usize>,
    /// The line of the notes that matched best, and which of its characters,
    /// when searching notes.
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
    pub fn new(kind: Kind) -> Self {
        Self {
            input: LineInput::default(),
            kind,
            tag: None,
            selected: 0,
            offset: Cell::new(0),
            typing: true,
            preview_scroll: 0,
            preview_height: Cell::new(0),
            preview_total: Cell::new(0),
            pending_g: false,
        }
    }

    /// Every item with the tag `name`, narrowed down by typing.
    pub fn for_tag(name: String) -> Self {
        Self { tag: Some(name), ..Self::new(Kind::Items) }
    }

    /// Handles a key, given the results there are now. While typing, every
    /// character goes into the query; arrows (or Ctrl+N/P, Ctrl+J/K) move the
    /// selection, and `Esc` stops typing, to move with `j`/`k`.
    pub fn handle_key(&mut self, key: KeyEvent, hits: &[Hit]) -> Action {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let pending_g = std::mem::take(&mut self.pending_g);
        let half_page = (self.preview_height.get() / 2).max(1) as isize;
        // Keys that work the same whether typing or not.
        match key.code {
            KeyCode::Enter => {
                return match hits.get(self.selected) {
                    Some(hit) => Action::Open { day: hit.day, index: hit.index },
                    None => Action::Stay,
                };
            }
            KeyCode::Up => return self.select_by(-1, hits.len()),
            KeyCode::Down => return self.select_by(1, hits.len()),
            KeyCode::Char('p' | 'k') if ctrl => return self.select_by(-1, hits.len()),
            KeyCode::Char('n' | 'j') if ctrl => return self.select_by(1, hits.len()),
            KeyCode::Char('d') if ctrl => return self.scroll_preview(half_page),
            KeyCode::Char('u') if ctrl => return self.scroll_preview(-half_page),
            KeyCode::Char(_) if ctrl => return Action::Stay,
            _ => {}
        }
        if !self.typing {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => return Action::Close,
                KeyCode::Char('j') => return self.select_by(1, hits.len()),
                KeyCode::Char('k') => return self.select_by(-1, hits.len()),
                KeyCode::Char('g') if pending_g => return self.select_by(isize::MIN / 2, hits.len()),
                KeyCode::Char('g') => self.pending_g = true,
                KeyCode::Char('G') => return self.select_by(isize::MAX / 2, hits.len()),
                KeyCode::Char('i' | 'a' | '/') => self.typing = true,
                _ => {}
            }
            return Action::Stay;
        }
        match key.code {
            // With results to move through, stop typing; with none, there's
            // nothing to do here.
            KeyCode::Esc if hits.is_empty() => return Action::Close,
            KeyCode::Esc => self.typing = false,
            code => {
                let before = self.input.text.clone();
                self.input.handle_key(code);
                if self.input.text != before {
                    self.selected = 0;
                    self.offset.set(0);
                    self.preview_scroll = 0;
                }
            }
        }
        Action::Stay
    }

    /// Adds pasted text to the query, on one line, typing again if not.
    pub fn paste(&mut self, text: &str) {
        self.typing = true;
        self.input.paste(text);
        self.selected = 0;
        self.offset.set(0);
        self.preview_scroll = 0;
    }

    pub fn find(&self, store: &Store, today: NaiveDate) -> Vec<Hit> {
        find(store, &self.input.text, self.kind, self.tag.as_deref(), today)
    }

    /// Moves the selection, showing the top of the newly selected item's notes.
    fn select_by(&mut self, delta: isize, len: usize) -> Action {
        let selected = self.selected.saturating_add_signed(delta).min(len.saturating_sub(1));
        if selected != self.selected {
            self.selected = selected;
            self.preview_scroll = 0;
        }
        Action::Stay
    }

    fn scroll_preview(&mut self, delta: isize) -> Action {
        let max = self.preview_total.get().saturating_sub(self.preview_height.get());
        self.preview_scroll = self.preview_scroll.saturating_add_signed(delta).min(max);
        Action::Stay
    }
}

/// Every item matching `query`, best first. Equally good matches are ordered
/// by how close their day is to `today`. Finding items matches their text,
/// and with nothing typed lists every item (with a `tag`, every item with
/// it), closest first. Searching notes matches each line of them, keeping
/// each item's best line, and with nothing typed finds nothing.
pub fn find(store: &Store, query: &str, kind: Kind, tag: Option<&str>, today: NaiveDate) -> Vec<Hit> {
    let empty = query.trim().is_empty();
    if empty && kind == Kind::Notes {
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
            let hit = |score, text_matches, note| Hit { day, index, score, text_matches, note };
            match kind {
                Kind::Items if empty => hits.push(hit(0, Vec::new(), None)),
                Kind::Items => {
                    if let Some((score, indices)) = matches(&item.text) {
                        hits.push(hit(score, indices, None));
                    }
                }
                Kind::Notes => {
                    let best = item
                        .notes
                        .lines()
                        .filter_map(|line| matches(line).map(|(score, indices)| (score, line, indices)))
                        .max_by_key(|(score, ..)| *score);
                    if let Some((score, line, indices)) = best {
                        hits.push(hit(score, Vec::new(), Some((line.to_string(), indices))));
                    }
                }
            }
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
        let hits = find(&store, "", Kind::Items, Some("car"), today());
        assert_eq!(texts(&store, &hits), ["Book #Car service", "Renew #car insurance"]);
        // Typing narrows them down.
        assert_eq!(texts(&store, &find(&store, "renew", Kind::Items, Some("car"), today())), ["Renew #car insurance"]);
    }

    #[test]
    fn finding_items_with_nothing_typed_lists_every_item_closest_first() {
        let (store, _dir) = store();
        let all = texts(&store, &find(&store, "", Kind::Items, None, today()));
        assert_eq!(all, ["Buy milk", "Write the quarterly report", "Café with Zoë", "Call mum", "Buy stamps", "Dentist at 3pm"]);
        assert_eq!(find(&store, "   ", Kind::Items, None, today()).len(), all.len());
    }

    #[test]
    fn searching_notes_with_nothing_typed_finds_nothing() {
        let (store, _dir) = store();
        assert!(find(&store, "", Kind::Notes, None, today()).is_empty());
        assert!(find(&store, "  ", Kind::Notes, None, today()).is_empty());
    }

    #[test]
    fn letters_match_in_order_with_gaps() {
        let (store, _dir) = store();
        let hits = find(&store, "bmlk", Kind::Items, None, today());
        assert_eq!(texts(&store, &hits), ["Buy milk"]);
        assert_eq!(hits[0].text_matches, [0, 4, 6, 7]);
    }

    #[test]
    fn it_searches_every_day_including_completed_items() {
        let (store, _dir) = store();
        assert_eq!(texts(&store, &find(&store, "dentist", Kind::Items, None, today())), ["Dentist at 3pm"]);
        assert_eq!(texts(&store, &find(&store, "mum", Kind::Items, None, today())), ["Call mum"]);
    }

    #[test]
    fn equal_matches_are_ordered_by_closeness_to_today() {
        let (store, _dir) = store();
        // Both "Buy ..." items score the same for "buy".
        assert_eq!(texts(&store, &find(&store, "buy", Kind::Items, None, today())), ["Buy milk", "Buy stamps"]);
        assert_eq!(texts(&store, &find(&store, "buy", Kind::Items, None, day(-3))), ["Buy stamps", "Buy milk"]);
    }

    #[test]
    fn case_is_ignored_unless_the_query_has_capitals() {
        let (store, _dir) = store();
        assert_eq!(texts(&store, &find(&store, "dentist", Kind::Items, None, today())), ["Dentist at 3pm"]);
        assert!(find(&store, "DENTIST", Kind::Items, None, today()).is_empty());
    }

    #[test]
    fn accents_match_plain_letters() {
        let (store, _dir) = store();
        let hits = find(&store, "cafe zoe", Kind::Items, None, today());
        assert_eq!(texts(&store, &hits), ["Café with Zoë"]);
        assert_eq!(hits[0].text_matches, [0, 1, 2, 3, 10, 11, 12]);
    }

    #[test]
    fn words_match_separately() {
        let (store, _dir) = store();
        assert_eq!(texts(&store, &find(&store, "3pm dent", Kind::Items, None, today())), ["Dentist at 3pm"]);
    }

    #[test]
    fn finding_items_looks_only_at_their_text() {
        let (store, _dir) = store();
        assert!(find(&store, "dashboard", Kind::Items, None, today()).is_empty());
        assert!(find(&store, "report", Kind::Items, None, today())[0].note.is_none());
    }

    #[test]
    fn searching_notes_looks_only_inside_them() {
        let (store, _dir) = store();
        let hits = find(&store, "dashboard", Kind::Notes, None, today());
        assert_eq!(texts(&store, &hits), ["Write the quarterly report"]);
        let (line, indices) = hits[0].note.as_ref().unwrap();
        assert_eq!(line, "Charts from the dashboard");
        assert_eq!(indices, &(16..25).collect::<Vec<_>>());
        assert!(hits[0].text_matches.is_empty());
        // The item's own text doesn't count.
        assert!(find(&store, "quarterly", Kind::Notes, None, today()).is_empty());
        assert!(find(&store, "milk", Kind::Notes, None, today()).is_empty());
    }

    #[test]
    fn searching_notes_keeps_each_items_best_line() {
        let (store, _dir) = store();
        let hits = find(&store, "the", Kind::Notes, None, today());
        assert_eq!(hits.len(), 1, "one result per item");
        let (line, _) = hits[0].note.as_ref().unwrap();
        assert!(line == "Ask Sam for the Q3 numbers" || line == "Charts from the dashboard");
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn typing_builds_the_query_and_resets_the_selection() {
        let (store, _dir) = store();
        let mut search = Search::new(Kind::Items);
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
        let mut search = Search::new(Kind::Items);
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
        let mut search = Search::new(Kind::Items);
        search.input = LineInput::new("dentist");
        let hits = search.find(&store, today());
        assert_eq!(search.handle_key(key(KeyCode::Enter), &hits), Action::Open { day: day(10), index: 0 });
        assert_eq!(search.handle_key(key(KeyCode::Enter), &[]), Action::Stay);
        // Esc stops typing, then closes.
        assert_eq!(search.handle_key(key(KeyCode::Esc), &hits), Action::Stay);
        assert!(!search.typing);
        assert_eq!(search.handle_key(key(KeyCode::Esc), &hits), Action::Close);
    }

    /// A search for "u" (several results), no longer typing.
    fn moving(store: &Store) -> (Search, Vec<Hit>) {
        let mut search = Search::new(Kind::Items);
        search.input = LineInput::new("u");
        let hits = search.find(store, today());
        assert!(hits.len() >= 4, "enough to move through");
        search.handle_key(key(KeyCode::Esc), &hits);
        (search, hits)
    }

    #[test]
    fn after_esc_j_and_k_move_through_the_results() {
        let (store, _dir) = store();
        let (mut search, hits) = moving(&store);
        // Letters move rather than being typed.
        search.handle_key(key(KeyCode::Char('j')), &hits);
        search.handle_key(key(KeyCode::Char('j')), &hits);
        assert_eq!(search.selected, 2);
        search.handle_key(key(KeyCode::Char('k')), &hits);
        assert_eq!(search.selected, 1);
        assert_eq!(search.input.text, "u");
        search.handle_key(key(KeyCode::Char('G')), &hits);
        assert_eq!(search.selected, hits.len() - 1);
        search.handle_key(key(KeyCode::Char('j')), &hits);
        assert_eq!(search.selected, hits.len() - 1, "stays on the last");
        search.handle_key(key(KeyCode::Char('g')), &hits);
        assert_eq!(search.selected, hits.len() - 1, "one g does nothing");
        search.handle_key(key(KeyCode::Char('g')), &hits);
        assert_eq!(search.selected, 0);
        // Enter goes to the selected one.
        search.handle_key(key(KeyCode::Char('j')), &hits);
        let hit = &hits[1];
        assert_eq!(search.handle_key(key(KeyCode::Enter), &hits), Action::Open { day: hit.day, index: hit.index });
    }

    #[test]
    fn i_a_or_slash_type_again_and_q_closes() {
        let (store, _dir) = store();
        for start in ['i', 'a', '/'] {
            let (mut search, hits) = moving(&store);
            search.handle_key(key(KeyCode::Char(start)), &hits);
            assert!(search.typing);
            search.handle_key(key(KeyCode::Char('m')), &hits);
            assert_eq!(search.input.text, "um");
        }
        let (mut search, hits) = moving(&store);
        assert_eq!(search.handle_key(key(KeyCode::Char('q')), &hits), Action::Close);
        // While typing, q is just typed.
        let mut search = Search::new(Kind::Items);
        assert_eq!(search.handle_key(key(KeyCode::Char('q')), &[]), Action::Stay);
        assert_eq!(search.input.text, "q");
    }

    #[test]
    fn esc_with_no_results_closes_straight_away() {
        let mut search = Search::new(Kind::Items);
        assert_eq!(search.handle_key(key(KeyCode::Esc), &[]), Action::Close);
    }

    #[test]
    fn pasting_types_again() {
        let (store, _dir) = store();
        let (mut search, _) = moving(&store);
        search.paste("lk");
        assert!(search.typing);
        assert_eq!(search.input.text, "ulk");
    }

    #[test]
    fn ctrl_d_and_u_scroll_the_preview_and_moving_resets_it() {
        let (store, _dir) = store();
        let (mut search, hits) = moving(&store);
        search.preview_height.set(10);
        search.preview_total.set(30);
        let ctrl = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);
        search.handle_key(ctrl('d'), &hits);
        assert_eq!(search.preview_scroll, 5);
        for _ in 0..10 {
            search.handle_key(ctrl('d'), &hits);
        }
        assert_eq!(search.preview_scroll, 20, "stops at the end");
        search.handle_key(ctrl('u'), &hits);
        assert_eq!(search.preview_scroll, 15);
        // A new selection shows the top of its notes.
        search.handle_key(key(KeyCode::Char('j')), &hits);
        assert_eq!(search.preview_scroll, 0);
        // Ctrl+D scrolls while typing too, rather than being typed.
        search.typing = true;
        search.handle_key(ctrl('d'), &hits);
        assert_eq!(search.preview_scroll, 5);
        assert_eq!(search.input.text, "u");
    }
}
