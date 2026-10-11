//! The leader menu, as in many vim setups: the leader (`Space` unless
//! `keys.toml` says otherwise) opens a popup of commands, like which-key,
//! and each key typed after it narrows the popup to the commands that still
//! match, until one does. Frequent actions keep keys of their own; these are
//! the occasional ones. The bindings are in `Keymap::leader`.

use ratatui::crossterm::event::{KeyCode, KeyEvent};

use crate::keymap::{self, Binding, Command, Key, Keymap, Match};

/// The leader menu: the keys typed after the leader so far.
#[derive(Default)]
pub struct Leader {
    pub typed: Vec<Key>,
}

/// What the app should do after the leader menu handles a key.
#[derive(Debug, PartialEq)]
pub enum Action {
    /// Still narrowing down.
    Stay,
    /// Nothing matched, or it was cancelled.
    Close,
    Run(Command),
}

/// One row of the popup: the next key, and what it does or the group it
/// opens.
#[derive(Debug, PartialEq)]
pub struct Entry {
    pub key: Key,
    pub label: String,
    pub group: bool,
}

impl Leader {
    pub fn handle_key(&mut self, event: KeyEvent, bindings: &[Binding]) -> Action {
        match event.code {
            KeyCode::Esc => return Action::Close,
            // Back a key, or out of the popup.
            KeyCode::Backspace => {
                return if self.typed.pop().is_some() { Action::Stay } else { Action::Close };
            }
            _ => {}
        }
        self.typed.push(Key::from_event(event));
        match keymap::lookup(bindings, &self.typed) {
            Match::Run(command) => Action::Run(command),
            Match::More => Action::Stay,
            Match::Nothing => Action::Close,
        }
    }

    /// What can be typed next, in the order of the bindings.
    pub fn entries(&self, keymap: &Keymap) -> Vec<Entry> {
        let mut entries: Vec<Entry> = Vec::new();
        for binding in &keymap.leader {
            let Some(rest) = binding.keys.strip_prefix(self.typed.as_slice()) else { continue };
            let Some(&key) = rest.first() else { continue };
            if entries.iter().any(|entry| entry.key == key) {
                continue;
            }
            let group = rest.len() > 1;
            let label = if group {
                let mut prefix = self.typed.clone();
                prefix.push(key);
                format!("+{}", keymap.group_name(&prefix).unwrap_or("more"))
            } else {
                binding.command.label().to_string()
            };
            entries.push(Entry { key, label, group });
        }
        entries
    }
}

#[cfg(test)]
mod tests {
    use ratatui::crossterm::event::KeyModifiers;

    use super::*;

    /// A keymap whose leader menu has an unnamed group, `f`, to test
    /// narrowing down.
    fn nested() -> Keymap {
        let (keymap, problems) = Keymap::from_toml(
            r#"
            [leader-menu]
            "f f" = "find-items"
            "f n" = "search-notes"
            "#,
        );
        assert!(problems.is_empty(), "{problems:?}");
        keymap
    }

    fn send(leader: &mut Leader, keymap: &Keymap, keys: &str) -> Action {
        let mut action = Action::Stay;
        for c in keys.chars() {
            action = leader.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE), &keymap.leader);
        }
        action
    }

    fn press(leader: &mut Leader, keymap: &Keymap, code: KeyCode) -> Action {
        leader.handle_key(KeyEvent::new(code, KeyModifiers::NONE), &keymap.leader)
    }

    #[test]
    fn a_key_that_finishes_a_binding_runs_it() {
        let keymap = Keymap::default();
        assert_eq!(send(&mut Leader::default(), &keymap, "c"), Action::Run(Command::Calendar));
        assert_eq!(send(&mut Leader::default(), &keymap, " "), Action::Run(Command::FindItems));
        assert_eq!(send(&mut Leader::default(), &keymap, "sg"), Action::Run(Command::SearchNotes));
    }

    #[test]
    fn a_group_key_narrows_down_to_its_bindings() {
        let keymap = nested();
        let mut leader = Leader::default();
        assert_eq!(send(&mut leader, &keymap, "f"), Action::Stay);
        let keys: String = leader.entries(&keymap).iter().map(|entry| entry.key.name()).collect();
        assert_eq!(keys, "fn");
        assert_eq!(send(&mut leader, &keymap, "n"), Action::Run(Command::SearchNotes));
    }

    #[test]
    fn a_key_that_matches_nothing_closes() {
        let keymap = Keymap::default();
        assert_eq!(send(&mut Leader::default(), &keymap, "z"), Action::Close);
        assert_eq!(send(&mut Leader::default(), &keymap, "sz"), Action::Close);
        // Ctrl+S isn't s.
        let mut leader = Leader::default();
        let ctrl_s = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL);
        assert_eq!(leader.handle_key(ctrl_s, &keymap.leader), Action::Close);
    }

    #[test]
    fn esc_closes_and_backspace_steps_back() {
        let keymap = Keymap::default();
        let mut leader = Leader::default();
        assert_eq!(press(&mut leader, &keymap, KeyCode::Esc), Action::Close);
        send(&mut leader, &keymap, "s");
        assert_eq!(press(&mut leader, &keymap, KeyCode::Backspace), Action::Stay);
        assert!(leader.typed.is_empty());
        assert_eq!(press(&mut leader, &keymap, KeyCode::Backspace), Action::Close);
    }

    #[test]
    fn groups_show_once_with_their_name() {
        let keymap = Keymap::default();
        let shown = Leader::default().entries(&keymap);
        let search: Vec<&Entry> = shown.iter().filter(|entry| entry.key == Key::char('s')).collect();
        assert_eq!(search, [&Entry { key: Key::char('s'), label: "+search".into(), group: true }]);
        // A group without a name is "+more".
        let nested = nested();
        let shown = Leader::default().entries(&nested);
        let f = shown.iter().find(|entry| entry.key == Key::char('f')).unwrap();
        assert_eq!(f.label, "+more");
    }

    #[test]
    fn space_s_is_the_search_group() {
        let keymap = Keymap::default();
        let mut leader = Leader::default();
        assert_eq!(send(&mut leader, &keymap, "s"), Action::Stay);
        let shown = leader.entries(&keymap);
        let keys: String = shown.iter().map(|entry| entry.key.name()).collect();
        assert_eq!(keys, "gdrtok");
        assert_eq!(shown[0], Entry { key: Key::char('g'), label: "search notes".into(), group: false });
        assert_eq!(send(&mut leader, &keymap, "k"), Action::Run(Command::Help));
    }

    #[test]
    fn every_first_key_shows_at_the_start() {
        let keymap = Keymap::default();
        let shown = Leader::default().entries(&keymap);
        let mut first: Vec<Key> = keymap.leader.iter().map(|binding| binding.keys[0]).collect();
        first.dedup();
        assert_eq!(shown.len(), first.len());
        assert_eq!(shown[0].key.name(), "Space");
    }
}
