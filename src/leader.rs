//! Commands behind `Space`, the leader key, as in many vim setups: `Space`
//! opens a popup of them, like which-key, and each key typed after it
//! narrows the popup to the commands that still match, until one does.
//! Frequent actions keep keys of their own; these are the occasional ones.

use ratatui::crossterm::event::KeyCode;

/// What a leader binding does, carried out by the app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    FindItems,
    SearchNotes,
    Deadlines,
    Reminders,
    SearchOptions,
    Calendar,
    Tags,
    Options,
    Workspaces,
    WhatsNew,
}

/// The keys typed after `Space` (with `' '` for `Space` itself), and what
/// they do.
pub struct Binding {
    pub keys: &'static str,
    pub label: &'static str,
    pub command: Command,
}

/// Every leader binding. No binding's keys may start another's, so each
/// runs as soon as it's typed.
pub const BINDINGS: &[Binding] = &[
    Binding { keys: " ", label: "find items", command: Command::FindItems },
    Binding { keys: "sg", label: "search notes", command: Command::SearchNotes },
    Binding { keys: "sd", label: "deadlines", command: Command::Deadlines },
    Binding { keys: "sr", label: "reminders", command: Command::Reminders },
    Binding { keys: "st", label: "tags", command: Command::Tags },
    Binding { keys: "so", label: "options", command: Command::SearchOptions },
    Binding { keys: "c", label: "calendar", command: Command::Calendar },
    Binding { keys: "o", label: "options", command: Command::Options },
    Binding { keys: "w", label: "workspaces", command: Command::Workspaces },
    Binding { keys: "n", label: "what's new", command: Command::WhatsNew },
];

/// Names for keys that start several bindings, shown as `+name` in the
/// popup before one is chosen, as which-key does.
pub const GROUPS: &[(&str, &str)] = &[("s", "search")];

/// The leader popup: the keys typed after `Space` so far.
#[derive(Default)]
pub struct Leader {
    pub typed: String,
}

/// What the app should do after the leader handles a key.
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
    pub key: char,
    pub label: String,
    pub group: bool,
}

impl Leader {
    pub fn handle_key(&mut self, code: KeyCode) -> Action {
        self.step(BINDINGS, code)
    }

    fn step(&mut self, bindings: &[Binding], code: KeyCode) -> Action {
        let key = match code {
            KeyCode::Esc => return Action::Close,
            // Back a key, or out of the popup.
            KeyCode::Backspace => {
                return if self.typed.pop().is_some() { Action::Stay } else { Action::Close };
            }
            KeyCode::Char(c) => c,
            _ => return Action::Stay,
        };
        let typed = format!("{}{key}", self.typed);
        if let Some(binding) = bindings.iter().find(|binding| binding.keys == typed) {
            return Action::Run(binding.command);
        }
        if bindings.iter().any(|binding| binding.keys.starts_with(&typed)) {
            self.typed = typed;
            return Action::Stay;
        }
        Action::Close
    }

    /// What can be typed next, in the order of `BINDINGS`.
    pub fn entries(&self) -> Vec<Entry> {
        entries(BINDINGS, GROUPS, &self.typed)
    }
}

/// The popup's rows after `typed`: one per next key, a binding if that key
/// finishes one, otherwise its group, named from `groups` if it has a name.
fn entries(bindings: &[Binding], groups: &[(&str, &str)], typed: &str) -> Vec<Entry> {
    let mut entries: Vec<Entry> = Vec::new();
    for binding in bindings {
        let Some(rest) = binding.keys.strip_prefix(typed) else { continue };
        let Some(key) = rest.chars().next() else { continue };
        if entries.iter().any(|entry| entry.key == key) {
            continue;
        }
        let group = rest.chars().count() > 1;
        let label = if group {
            let prefix = format!("{typed}{key}");
            let name = groups.iter().find(|(keys, _)| *keys == prefix).map_or("more", |(_, name)| name);
            format!("+{name}")
        } else {
            binding.label.to_string()
        };
        entries.push(Entry { key, label, group });
    }
    entries
}

/// How a key is shown in the popup: `Space` for `' '`.
pub fn key_name(key: char) -> String {
    if key == ' ' { "Space".into() } else { key.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A table with a group, `f`, to test narrowing down.
    const NESTED: &[Binding] = &[
        Binding { keys: "ff", label: "find items", command: Command::FindItems },
        Binding { keys: "fn", label: "find in notes", command: Command::SearchNotes },
        Binding { keys: "c", label: "calendar", command: Command::Calendar },
    ];

    fn send(leader: &mut Leader, bindings: &[Binding], keys: &str) -> Action {
        let mut action = Action::Stay;
        for c in keys.chars() {
            action = leader.step(bindings, KeyCode::Char(c));
        }
        action
    }

    #[test]
    fn a_key_that_finishes_a_binding_runs_it() {
        let mut leader = Leader::default();
        assert_eq!(leader.handle_key(KeyCode::Char('c')), Action::Run(Command::Calendar));
        let mut leader = Leader::default();
        assert_eq!(leader.handle_key(KeyCode::Char(' ')), Action::Run(Command::FindItems));
        let mut leader = Leader::default();
        assert_eq!(send(&mut leader, NESTED, "c"), Action::Run(Command::Calendar));
    }

    #[test]
    fn a_group_key_narrows_down_to_its_bindings() {
        let mut leader = Leader::default();
        assert_eq!(send(&mut leader, NESTED, "f"), Action::Stay);
        assert_eq!(leader.typed, "f");
        let shown = entries(NESTED, &[], &leader.typed);
        assert_eq!(shown.iter().map(|entry| entry.key).collect::<String>(), "fn");
        assert_eq!(send(&mut leader, NESTED, "n"), Action::Run(Command::SearchNotes));
    }

    #[test]
    fn a_key_that_matches_nothing_closes() {
        let mut leader = Leader::default();
        assert_eq!(leader.handle_key(KeyCode::Char('z')), Action::Close);
        let mut leader = Leader::default();
        assert_eq!(send(&mut leader, NESTED, "fz"), Action::Close);
    }

    #[test]
    fn esc_closes_and_backspace_steps_back() {
        let mut leader = Leader::default();
        assert_eq!(leader.handle_key(KeyCode::Esc), Action::Close);
        send(&mut leader, NESTED, "f");
        assert_eq!(leader.step(NESTED, KeyCode::Backspace), Action::Stay);
        assert_eq!(leader.typed, "");
        assert_eq!(leader.step(NESTED, KeyCode::Backspace), Action::Close);
    }

    #[test]
    fn groups_show_once_with_their_name() {
        let shown = entries(NESTED, &[("f", "find")], "");
        assert_eq!(
            shown,
            [
                Entry { key: 'f', label: "+find".into(), group: true },
                Entry { key: 'c', label: "calendar".into(), group: false },
            ]
        );
        assert_eq!(entries(NESTED, &[], "")[0].label, "+more");
    }

    #[test]
    fn no_binding_starts_another_so_each_can_run() {
        for a in BINDINGS {
            for b in BINDINGS {
                if !std::ptr::eq(a, b) {
                    assert!(!b.keys.starts_with(a.keys), "{:?} starts {:?}", a.keys, b.keys);
                }
            }
        }
        for (keys, _) in GROUPS {
            assert!(BINDINGS.iter().any(|binding| binding.keys.starts_with(keys) && binding.keys != *keys), "{keys}");
        }
    }

    #[test]
    fn space_s_is_the_search_group_and_g_searches_notes() {
        let mut leader = Leader::default();
        assert_eq!(leader.handle_key(KeyCode::Char('s')), Action::Stay);
        let shown = leader.entries();
        let keys: String = shown.iter().map(|entry| entry.key).collect();
        assert_eq!(keys, "gdrto");
        assert_eq!(shown[0], Entry { key: 'g', label: "search notes".into(), group: false });
        assert_eq!(leader.handle_key(KeyCode::Char('g')), Action::Run(Command::SearchNotes));
        let start = Leader::default().entries();
        assert!(start.contains(&Entry { key: 's', label: "+search".into(), group: true }));
    }

    #[test]
    fn every_binding_shows_at_the_start() {
        let shown = Leader::default().entries();
        // One row per first key: groups show once.
        let mut first: Vec<char> = BINDINGS.iter().filter_map(|binding| binding.keys.chars().next()).collect();
        first.dedup();
        assert_eq!(shown.len(), first.len());
        assert_eq!(key_name(shown[0].key), "Space");
    }
}
