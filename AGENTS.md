# todoro

A vim-style terminal todo app in Rust with [ratatui](https://ratatui.rs). It runs as `todoro` and opens on today's list.

## Commands

```sh
cargo build                 # build
cargo test                  # run all tests; keep them passing
cargo clippy --all-targets  # lint, including tests; keep it warning-free
cargo install --path .      # install/update the `todoro` binary in ~/.cargo/bin
```

## Layout

- `src/main.rs`: terminal setup/teardown and the event loop
- `src/app.rs`: app state, modes (`Normal`, `Insert`, `ConfirmDelete`, `Notes`, `Help`) and key handling
- `src/input.rs`: `LineInput`, single-line typing with a movable cursor
- `src/notes.rs`: the notes editor, a vim key layer over `ratatui-textarea`
- `src/calendar.rs`: the calendar's state and keys (`Calendar`, `Zoom`), returning an `Action` for the app to carry out
- `src/search.rs`: fuzzy search over every day's items (and notes with `S`), using `nucleo-matcher`
- `src/options.rs`: `Settings` (saved to `settings.json`), the `TOGGLES` shown in the `o` popup, and the popup's keys
- `src/tags.rs`: finding `#tags` in items' text (`find_tags`), counting them (`all_tags`) and the `#` popup's keys
- `src/help.rs`: the `?` popup's keybinding table (`SECTIONS`) and search
- `src/ui.rs`: all rendering (list, notes screen, status bar, delete and help popups)
- `src/store.rs`: JSON persistence, keyed by `YYYY-MM-DD`
- `src/test_util.rs`: helpers shared by the tests
- `src/snapshots/`: saved screen snapshots for the UI tests

## Keybindings

Normal mode: `h`/`l` previous/next day, `t` today, `j`/`k` move down/up, `gg`/`G` first/last item, a count before `j`/`k`/`G`/`gg` (`4j`, `42G`; digits build `App::count`, and `0` only continues one), `J`/`K` move the item down/up (within the open or completed items, and only among items stored on the same day), `H`/`L` move the item to the day before/after the one on screen and follow it there, `a` add below the cursor (or at the end of the open items when on a completed one), `A` add several (`Insert { repeat: true }`: `Enter` adds and starts the next item below, `Esc` saves and stops, `Enter` on an empty line stops), `e` edit, `x` toggle done, `m` toggle pinned, `!` triage (priority cycles none → High → Medium → Low → none), `d` delete (opens a popup; `d` confirms, `c` cancels; the deleted item goes to the paste register), `yy` copy, `p`/`P` paste below/above (as open items, keeping notes, pin and priority), `V` select several, `Enter` open notes, `u`/`Ctrl+R` undo/redo, `c` calendar, `s`/`S` search items / items and notes, `#` tags (then `Enter` shows a tag's items in the search, filtered by `Search::tag`), `o` options, `?` help, `q` quit.

Selecting (`Mode::Visual`): `j`/`k`/`G` extend, `x` complete all (or reopen if all complete), `m` pin all (or unpin if all pinned), `!` next priority after the first item's, `d` delete (one confirmation), `y` copy, `H`/`L` move to the previous/next day and follow, `Esc`/`V` stop. Each action returns to `Normal`. Selections can include carried items, so act through `App::group_by_day`, which groups screen rows by the day they're stored on.

Options: `j`/`k` select, `Space`/`Enter` toggle (saved immediately), `Esc`/`q`/`o` close. To add an option, add a field to `Settings` (with a default of off) and an entry to `TOGGLES`; the popup, saving and loading follow.

Search: every typed character goes into the query; `↑`/`↓`/`Ctrl+N`/`Ctrl+P`/`Ctrl+J`/`Ctrl+K` select, `Enter` goes to the item (its day, with it selected), `Esc` closes.

Calendar: `h`/`j`/`k`/`l` follow the layout (month and year: `h`/`l` day, `j`/`k` week; week view: `j`/`k` day, `h`/`l` week), `H`/`L` month, `t` today, `w`/`m`/`y` week/month/year view, `a` add an item to the selected day (typed in a popup with `LineInput`; `Enter`/`Esc` save), `Enter` open that day's list, `u`/`Ctrl+R` undo/redo, `?` help, `Esc`/`q`/`c` back to the list.

Insert mode: type to insert at the cursor, `←`/`→`/`Home`/`End` move, `Backspace`/`Delete` remove, `Enter`/`Esc` save. Saving an empty new item discards it. Saving an edited item as empty opens the delete popup.

Notes screen, normal mode: `h`/`j`/`k`/`l`, `w`/`b`/`e`, `0`/`$`, `_`/`^` (first non-blank; `gg`/`G`/`:42` land there too), `gg`/`G` move, with a count (`4j`, `42G`; also `3x`, `2dd`); `:` opens a command line (`NotesEditor::command`, drawn in place of the status bar): `:42` goes to line 42, `:q`/`:wq`/`:x` close, `:w` does nothing; `i`/`a`/`I`/`A`/`o`/`O` enter insert mode; `x` deletes a character, `dd` a line; `u`/`Ctrl+R` undo/redo; `?` help; `Esc`/`q` back to the list. Notes insert mode: type freely, `Esc` back to normal mode. Anything not listed here is not implemented (only the `:` commands above).

Help popup (from the list or notes normal mode): every typed character goes into the search, `↑`/`↓`/`Ctrl+N`/`Ctrl+P`/`PageUp`/`PageDown` scroll, `Backspace` and `Ctrl+U` edit the search, `Esc` returns to the screen it was opened from. One-character searches match key names only (case-sensitive); longer ones match keys or descriptions (case-insensitive).

Keep new bindings vim-like. When you add or change one, update `SECTIONS` in `src/help.rs` (what `?` shows), the hints in `src/ui.rs` (bottom border and status bar), the list above and the tables in `README.md`.

## Conventions

- Each day's items are stored with the open ones first and the completed ones after; `Store` keeps this order (`open_count` gives the boundary). `x` moves an item to the boundary: the top of the completed items or the bottom of the open ones. Only open items are numbered in the UI; completed ones follow a "Completed" header row, so their list row is their index plus one.
- Items stay on their day by default. On startup, `Store::roll_over` moves pinned, open items from earlier days to the top of today, still pinned. Unpinned and completed items stay on their day.
- The event loop wakes every 30 seconds to call `App::set_today`, so midnight is noticed while todoro is open. It only acts in `Normal` mode (positions mustn't shift mid-edit), carries pinned items over and clears undo history.
- Today and future days also show the pinned, open items from earlier days (`Store::pinned_before`), first, without moving them. So an unfinished pinned item shows on its own day and every day after it. So screen positions aren't always store positions: `App::slots()` maps each row on screen to the `(day, index)` where its item is stored. Use it (or `App::items()`) for anything that reads or changes the selected item, never `store.items(app.day)[app.selected]`.
- On disk an item is a plain string, or an object with `text` and optional `notes`, `done`, `pinned` and `priority` (`"high"`, `"medium"` or `"low"`) (see `RawItem` in `store.rs`). Items with only text must stay plain strings so simple files stay readable by older versions.
- `Store` saves after every change by writing a temp file and renaming it. Don't defer or batch saves.
- "No colours" (`Settings::no_colour`) is applied after drawing, by `strip_colour` in `ui.rs`: background colours become reversed text and grey becomes dim. New UI needs no special handling, but anything highlighted only by colour must also differ in some other way (a symbol, bold, reversed) to stay readable without colour.
- The calendar shows only the items stored on each day, not pinned items carried forward, which would fill every later day.
- List undo is automatic: `App::handle_key` snapshots the store before each key in `Normal`, `Insert`, `ConfirmDelete`, `Visual` and `Calendar` mode and records it if the key changed anything. A notes visit is recorded as one change when it closes (`notes_before`). New list actions need no undo code, but must change the store only through `App::handle_key`.
- Single-line typing (the list's insert mode) goes through `LineInput` in `input.rs`. Its `cursor` is a byte offset that must stay on a char boundary.
- The UI must work down to about 30 columns. Any text with a fixed length needs narrower fallbacks: `fit_first` (shorter alternatives), `fit_hints` (status bar hints by priority, `? help` last to go) or `truncate` (ends with "…"). The `screens_at_30_columns` and `screens_at_40_columns` snapshots cover every screen, so check them after any UI change.
- Long text wraps rather than being cut off: list rows and the delete popup both use `wrap_ranges` in `ui.rs`, which keeps the text exactly as typed (so the cursor can be placed with `cursor_position`) and measures display width with `unicode-width`, not `chars().count()`. List rows can be several lines tall, so screen positions come from summing row heights.
- The notes editor must keep vim's behavior where it differs from `ratatui-textarea`'s defaults: `h`/`l`/`x` never cross line boundaries, the normal-mode cursor never sits past the last character, and each command is one undo step.
- The notes editor keeps its own undo history (`NotesEditor::undo`), not the text area's: `handle_key` snapshots the text before each normal-mode command and records it if the text changed, and a command that enters insert mode is recorded when insert mode ends. So commands can change the text however is simplest, including rebuilding it with `set_lines`.

## Tests

Every new feature or bug fix comes with tests in the same commit. Tests live in a `#[cfg(test)] mod tests` at the bottom of the file they cover:

- `app.rs`: key handling. Build an app with `test_util::app_with(&[...])` and send keys with `press` and `type_str`. Cover the behavior, edge cases (empty list, first/last item, multibyte text) and that keys meant for one mode do nothing in the others.
- `help.rs`: search matching and the popup's own keys.
- `notes.rs`: the notes editor's keys, driven with the `send` helper (`<esc>` and `<cr>` stand for Escape and Enter).
- `store.rs`: persistence, always against a temporary file.
- `calendar.rs`: the calendar's keys and date maths, driven with its own `send` helper.
- `search.rs`: matching and ordering (`find`) and the search's keys.
- `tags.rs`: what counts as a tag, counting, and the tag list's keys.
- Match positions from `nucleo-matcher` count graphemes, not chars or bytes; highlight with `unicode-segmentation`'s graphemes (see `highlighted` in `ui.rs`).
- `ui.rs`: screen snapshots with [insta](https://insta.rs). Add one for any new screen or popup.

Tests run on a fixed date (`test_util::today()`, 2026-10-05) and must never read the clock, the real data file or `TODORO_FILE`.

### Snapshots

A snapshot test draws the UI into memory and compares it with the saved copy in `src/snapshots/`. When a change to the UI is intended, the test fails and prints a diff. To accept it:

1. Read the diff and check that every changed line is meant to change.
2. Rerun with `INSTA_UPDATE=always cargo test` to overwrite the saved copies, or use `cargo insta review` if `cargo-insta` is installed.
3. Read the new `.snap` files before committing. A snapshot only records what the code drew, so a wrong one locks a bug in.

Never update snapshots just to make a failing test pass without reading the diff.

## Data and manual testing

Todos are stored in `~/.local/share/todoro/todos.json` by default, and settings in `~/.config/todoro/settings.json`. Set `TODORO_FILE` and `TODORO_SETTINGS` to use other files, and always set both when running the app to test it, so the user's real todos and settings are never touched. Unit tests leave `App::settings_path` as `None`, so they never save settings.

To drive the real TUI, use a separate tmux server (`tmux -L todoro-test ...`) and target sessions with an exact match (`-t '=name:'`). The user runs their own tmux, and a plain `-t name` can prefix-match their windows and send keystrokes into them.

## Releasing

Releases are built by [dist](https://github.com/axodotdev/cargo-dist) in `.github/workflows/release.yml`, which runs when a version tag is pushed. It builds Linux and macOS (x86_64 and ARM) and Windows (x86_64), and publishes them to GitHub Releases with `curl | sh` and PowerShell installers.

To release a new version, with the user's go-ahead:

1. Draft the new `CHANGELOG.md` section with [git-cliff](https://git-cliff.org): `git cliff --unreleased --tag vX.Y.Z --prepend CHANGELOG.md` (it reads `cliff.toml`). Move the new section below the file's intro if it lands above it, and edit it into plain, user-facing wording: a commit for a change that a later commit replaced shouldn't be listed. The section's heading must be `## [X.Y.Z] - YYYY-MM-DD`; dist uses it as the release notes.
2. Set `version` in `Cargo.toml` (run `cargo build` so `Cargo.lock` updates too), and commit both as `chore: release vX.Y.Z`.
3. Check the notes with `dist manifest --tag vX.Y.Z --output-format=json` (`announcement_changelog`), then `git tag vX.Y.Z` and `git push origin main vX.Y.Z`. The tag must match the version.

`dist-workspace.toml` holds dist's settings. Don't edit `release.yml` by hand: change the settings and run `dist generate` to regenerate it. `ci.yml` (tests on all three systems) is hand-written.

## Commits

Use [Conventional Commits](https://www.conventionalcommits.org): `<type>[optional scope]: <description>`, with the description in lowercase imperative mood and no trailing period.

| Type | Use for |
|---|---|
| `feat` | A new user-facing feature or keybinding |
| `fix` | A bug fix |
| `refactor` | A code change that doesn't change behavior |
| `perf` | A performance improvement |
| `docs` | Documentation only, including this file |
| `test` | Adding or changing tests |
| `style` | Formatting only |
| `build` | Dependencies, `Cargo.toml`, the build setup |
| `ci` | CI configuration |
| `chore` | Anything else (tooling, `.gitignore`, housekeeping) |

Mark a breaking change with `!` after the type (`feat!: ...`) and use no other marker: no `BREAKING CHANGE:` footer. If it needs explaining, do that in plain sentences in the commit body. Breaking changes include changing an existing keybinding and changing the format of the todo data file.

Examples:

```
feat: add x to mark an item as done
fix(store): keep the data file intact when a save fails
feat!: move day navigation to H/L
```

Keep each commit to one logical change.
