//! The release notes, built into the program from CHANGELOG.md, and the
//! popup that shows them: what's new after an update, or all of them with `N`.

use std::cell::Cell;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// CHANGELOG.md as it was when this version was built.
pub const CHANGELOG: &str = include_str!("../CHANGELOG.md");

/// This version, from Cargo.toml.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// One release's notes.
#[derive(Debug, PartialEq)]
pub struct Release<'a> {
    pub version: (u32, u32, u32),
    /// The heading, like "0.3.0 - 2026-10-06".
    pub title: String,
    /// The notes under the heading, as Markdown.
    pub notes: &'a str,
}

/// A version like "0.3.0" as numbers that compare in order.
pub fn parse_version(version: &str) -> Option<(u32, u32, u32)> {
    let mut parts = version.trim().trim_start_matches('v').split('.').map(|part| part.parse().ok());
    Some((parts.next()??, parts.next()??, parts.next()??))
}

/// Every release in a changelog, newest first. Sections are `## [X.Y.Z] - date`
/// headings; anything before the first one (the intro) is left out.
pub fn releases(changelog: &str) -> Vec<Release<'_>> {
    let mut releases = Vec::new();
    let mut rest = changelog;
    while let Some(start) = rest.find("\n## [") {
        rest = &rest[start + 1..];
        let heading_end = rest.find('\n').unwrap_or(rest.len());
        let heading = &rest[3..heading_end];
        let body = &rest[heading_end..];
        let end = body.find("\n## [").unwrap_or(body.len());
        if let Some(version) = heading.split(']').next().and_then(|v| parse_version(v.trim_start_matches('[')))
        {
            releases.push(Release { version, title: heading.replace(['[', ']'], ""), notes: body[..end].trim() });
        }
        rest = &body[end..];
    }
    releases
}

/// The popup listing release notes.
pub struct ChangelogView {
    /// Only releases newer than this, for "what's new"; all of them if `None`.
    pub since: Option<(u32, u32, u32)>,
    /// First visible line.
    pub scroll: usize,
    /// How many lines fit and how many there are once wrapped, recorded when
    /// drawn, so scrolling stops at the end.
    pub height: Cell<usize>,
    pub total: Cell<usize>,
}

impl ChangelogView {
    pub fn all() -> Self {
        Self { since: None, scroll: 0, height: Cell::new(0), total: Cell::new(0) }
    }

    /// What's new since `version`.
    pub fn since(version: (u32, u32, u32)) -> Self {
        Self { since: Some(version), ..Self::all() }
    }

    /// The releases to show.
    pub fn releases(&self) -> Vec<Release<'static>> {
        releases(CHANGELOG).into_iter().filter(|release| self.since.is_none_or(|since| release.version > since)).collect()
    }

    /// Handles a key and returns `true` when the popup should close.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        let page = self.height.get().max(1) as isize;
        match key.code {
            KeyCode::Esc | KeyCode::Char('q' | 'N') => return true,
            KeyCode::Char('j') | KeyCode::Down => self.scroll_by(1),
            KeyCode::Char('k') | KeyCode::Up => self.scroll_by(-1),
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => self.scroll_by(page / 2),
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => self.scroll_by(-page / 2),
            KeyCode::PageDown | KeyCode::Char(' ') => self.scroll_by(page),
            KeyCode::PageUp => self.scroll_by(-page),
            KeyCode::Char('g') => self.scroll = 0,
            KeyCode::Char('G') => self.scroll_by(isize::MAX / 2),
            _ => {}
        }
        false
    }

    fn scroll_by(&mut self, delta: isize) {
        let max = self.total.get().saturating_sub(self.height.get());
        self.scroll = self.scroll.saturating_add_signed(delta).min(max);
    }
}

/// What to show when todoro starts: what's new since the last version seen,
/// if that's older than this one. With no version recorded, someone with
/// todos already must have updated from before this was tracked, so they see
/// this release's notes; someone without any is new and sees nothing.
pub fn on_start(last_seen: Option<&str>, has_todos: bool) -> Option<ChangelogView> {
    let current = parse_version(VERSION)?;
    match last_seen.and_then(parse_version) {
        Some(seen) if seen < current => Some(ChangelogView::since(seen)),
        Some(_) => None,
        None if has_todos => releases(CHANGELOG).get(1).map(|previous| ChangelogView::since(previous.version)),
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# Changelog\n\nIntro text.\n\n## [0.2.0] - 2026-10-05\n\n### Features\n\n- Two\n\n## [0.1.0] - 2026-10-01\n\nThe first release.\n";

    #[test]
    fn versions_parse_and_compare() {
        assert_eq!(parse_version("0.3.0"), Some((0, 3, 0)));
        assert_eq!(parse_version("v1.12.3"), Some((1, 12, 3)));
        assert_eq!(parse_version("1.2"), None);
        assert_eq!(parse_version("one"), None);
        assert!(parse_version("0.10.0") > parse_version("0.9.9"));
    }

    #[test]
    fn releases_are_split_by_heading_without_the_intro() {
        let releases = releases(SAMPLE);
        assert_eq!(releases.len(), 2);
        assert_eq!(releases[0].version, (0, 2, 0));
        assert_eq!(releases[0].title, "0.2.0 - 2026-10-05");
        assert_eq!(releases[0].notes, "### Features\n\n- Two");
        assert_eq!(releases[1].notes, "The first release.");
    }

    #[test]
    fn the_built_in_changelog_has_this_version_first() {
        let releases = releases(CHANGELOG);
        assert_eq!(Some(releases[0].version), parse_version(VERSION), "add this version's notes to CHANGELOG.md");
        assert!(releases.windows(2).all(|pair| pair[0].version > pair[1].version), "newest first");
    }

    #[test]
    fn whats_new_shows_releases_since_the_last_one_seen() {
        let view = ChangelogView::since((0, 1, 0));
        let versions: Vec<_> = view.releases().iter().map(|release| release.version).collect();
        assert!(!versions.is_empty());
        assert!(versions.iter().all(|&version| version > (0, 1, 0)));
        assert!(ChangelogView::all().releases().len() > versions.len());
    }

    #[test]
    fn on_start_shows_whats_new_only_after_an_update() {
        let current = parse_version(VERSION).unwrap();
        // Up to date: nothing.
        assert!(on_start(Some(VERSION), true).is_none());
        // Updated from an older version: what's new since then.
        let view = on_start(Some("0.1.0"), true).unwrap();
        assert_eq!(view.since, Some((0, 1, 0)));
        // A newer version than this one (after a downgrade): nothing.
        assert!(on_start(Some("99.0.0"), true).is_none());
        // Nothing recorded and no todos: a new user, so nothing.
        assert!(on_start(None, false).is_none());
        // Nothing recorded but some todos: this release's notes.
        let view = on_start(None, true).unwrap();
        assert_eq!(view.releases()[0].version, current);
        assert_eq!(view.releases().len(), 1);
    }

    #[test]
    fn keys_scroll_and_close() {
        let mut view = ChangelogView::all();
        view.height.set(10);
        view.total.set(25);
        let mut press = |code| view.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
        assert!(!press(KeyCode::Char('j')));
        assert!(!press(KeyCode::Char('G')));
        assert!(!press(KeyCode::Char('k')));
        assert!(press(KeyCode::Esc));
        assert_eq!(view.scroll, 14);
        view.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
        assert_eq!(view.scroll, 0);
        for code in [KeyCode::Char('q'), KeyCode::Char('N')] {
            assert!(view.handle_key(KeyEvent::new(code, KeyModifiers::NONE)));
        }
    }
}
