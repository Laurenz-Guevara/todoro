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
- `src/markdown.rs`: notes' Markdown as wrapped, styled lines for the viewer and the search's preview (`render`, with `pulldown-cmark`); a single line break stays a line break, as typed
- `src/viewer.rs`: the notes viewer's state and keys (`Viewer`), returning an `Action` for the app
- `src/calendar.rs`: the calendar's state and keys (`Calendar`, `Zoom`), returning an `Action` for the app to carry out; also choosing a deadline (`Calendar::picking`)
- `src/deadline.rs`: `Deadline` (a day and an optional time), how it's shown (`describe`, `format_time`), typed times (`parse_time`) and how far an item rolls on (`roll_to`)
- `src/search.rs`: fuzzy search over every day, using `nucleo-matcher`: finding items by their text (`Kind::Items`, `Space Space`; everything with nothing typed) or searching inside their notes (`Kind::Notes`, `s`; each item's best line, `Hit::note_line`, where `Enter` opens the notes to edit with `NotesEditor::start_on_line`)
- `src/options.rs`: `Settings` (saved to `settings.json`), the `TOGGLES` shown in the `o` popup, and the popup's keys
- `src/tags.rs`: finding `#tags` in items' text (`find_tags`), counting them (`all_tags`) and the `#` popup's keys
- `src/changelog.rs`: the release notes, built in from `CHANGELOG.md` with `include_str!`, and the popup showing them (`N`, and "what's new" on the first start after an update, tracked by `Settings::last_seen_version`)
- `src/cli.rs`: command-line flags (`--where`, `--version`, `--help`); `--where` prints only the folder path, for scripts
- `src/workspaces.rs`: the todoro folder the user chooses on first run and the workspaces in it (one folder each, with its own `todos.json`), plus first-run setup and moving pre-folder todos in
- `src/setup.rs`: the first-run screen's state and keys
- `src/help.rs`: the `?` popup's keybinding table (`SECTIONS`) and search
- `src/ui.rs`: all rendering (list, notes screen, status bar, delete and help popups)
- `src/store.rs`: JSON persistence, keyed by `YYYY-MM-DD`
- `src/notes_files.rs`: items' notes as Markdown files in a folder beside the todos file (`notes/` in a workspace), synced on every save
- `src/test_util.rs`: helpers shared by the tests
- `src/snapshots/`: saved screen snapshots for the UI tests

## Keybindings

Normal mode: `h`/`l` previous/next day, `t` today, `j`/`k` move down/up, `gg`/`G` first/last item, a count before `j`/`k`/`G`/`gg` (`4j`, `42G`; digits build `App::count`, and `0` only continues one), `J`/`K` move the item down/up (within the open or completed items, and only among items stored on the same day), `H`/`L` move the item to the day before/after the one on screen and follow it there, `a` add below the cursor (or at the end of the open items when on a completed one), `A` add several (`Insert { repeat: true }`: `Enter` adds and starts the next item below, `Esc` saves and stops, `Enter` on an empty line stops), `e` edit, `x` toggle done, `m` toggle pinned (not on an item with a deadline, which says so instead), `@` or `:deadline` set a deadline (`App::open_deadline`: the year calendar with `Calendar::for_deadline`; `Enter` chooses the day, then a time is typed, empty for none, replacing the old one when typing starts; `d` removes it; `Esc` steps back; the calendar returns `Action::SetDeadline`), `!` triage (priority cycles none → High → Medium → Low → none), `d` delete (opens a popup; `d` confirms, `c` cancels; the deleted item goes to the paste register), `yy` copy, `p`/`P` paste below/above (as open items, keeping notes, pin and priority), `V` select several, `v` view notes formatted (`Mode::View`), `Enter` open notes (in the user's editor if `Settings::editor` is set: `Enter` sets `App::external`, and the event loop in `main.rs` hands it the terminal between `App::start_external_edit` and `App::finish_external_edit`, which rereads the file and records one undo step), `u`/`Ctrl+R` undo/redo, `c` calendar, `Space Space` find items by text and `s` search notes (`Space` is a pending first key, like `g` and `y`), `#` tags (then `Enter` shows a tag's items in the search, filtered by `Search::tag`), `o` options, `W` workspaces (`Mode::Workspaces` with `workspaces::Picker`: `j`/`k`, `Enter` open, `a` create, `d` delete after typing the name, never the current or only one; switching goes through `App::switch_workspace`, which clears undo history), `:` command line (`App::command`, drawn in place of the status bar: `:42` goes to item 42, `:deadline` is `@`, `:q`/`:wq`/`:x` quit), `?` help, `q` quit.

Selecting (`Mode::Visual`): `j`/`k`/`G` extend, `x` complete all (or reopen if all complete), `m` pin all (or unpin if all pinned), `!` next priority after the first item's, `d` delete (one confirmation), `y` copy, `H`/`L` move to the previous/next day and follow, `Esc`/`V` stop. Each action returns to `Normal`. Selections can include carried items, so act through `App::group_by_day`, which groups screen rows by the day they're stored on.

Options: `j`/`k` select, `Space`/`Enter` toggle (saved immediately), `Esc`/`q`/`o` close. `/` searches (`Options::search`, typed while `Options::searching`): only rows whose section, name, description or value (`Options::row_text`) contain every word show (`Options::matches`, `visible`); `↑`/`↓`/`Ctrl+N`/`Ctrl+P` move among them while typing, `Enter` keeps the matches for `j`/`k`/`Space`, and `Esc` clears the search before it closes the popup. The search line stays above the rows, which scroll beneath it. After the toggles come rows with a typed value. The notes editor (`EDITOR_ROW`): `Enter` types a command, run with the file after it through `sh -c` (`cmd /C` on Windows), and an empty one means todoro's own editor. Then the todoro folder (`FOLDER_ROW`, `Options::folder`, only with workspaces): `Enter` types a new one and moves every workspace there with `Workspaces::move_to`, which moves nothing if a name clashes; the open store is then reopened from its new path. The last rows delete things (`Options::clears`, `options::Clear`, from `Options::first_clear_row`): all items and notes, or all notes, in the open workspace (or file); all notes, or all items and notes, in every workspace; and a reset. Each is confirmed by typing `Clear::confirm_word` (the workspace's name, `delete` or `reset`) and carried out by `App::clear`, which also clears undo history. Deleting notes removes the `.md` files in the notes folder (`NotesFiles::delete_all`), nothing else. A reset deletes every workspace folder with a todos file and the todoro folder if that empties it (`Workspaces::delete_all`), deletes the settings file and sets `App::restart`, so `main` starts again from the setup screen; with no `last_seen_version`, `changelog::on_start` then shows the newest `FIRST_START_RELEASES` releases with a link to the rest. To add an option, add a field to `Settings` (with a default of off) and an entry to `TOGGLES`; the popup, saving and loading follow.

Viewing notes (`Mode::View`): `j`/`k`, `Ctrl+D`/`Ctrl+U`, `Space`/`Ctrl+B`, `gg`/`G` scroll; `i`/`e`/`Enter` edit (as `Enter` on the list, setting `App::view_after_edit` so closing the notes or the user's editor comes back to the view); `?` help; `Esc`/`q`/`v` back to the list.

Search: while `Search::typing`, every typed character goes into the query; `↑`/`↓`/`Ctrl+N`/`Ctrl+P`/`Ctrl+J`/`Ctrl+K` select, `Enter` goes to the item (its day, with it selected), `Esc` stops typing (or closes with no results). Then, as in vim, `j`/`k`/`gg`/`G` select, `i`/`a`/`/` type again, `Esc`/`q` close. At least `SEARCH_PREVIEW_WIDTH` wide, the results sit beside a preview of the selected item's notes (`draw_search_preview`, with `markdown::render`), which `Ctrl+D`/`Ctrl+U` scroll (`Search::preview_scroll`).

Calendar: `h`/`j`/`k`/`l` follow the layout (month and year: `h`/`l` day, `j`/`k` week; week view: `j`/`k` day, `h`/`l` week), `H`/`L` month, `t` today, `w`/`m`/`y` week/month/year view, `a` add an item to the selected day (typed in a popup with `LineInput`; `Enter`/`Esc` save), `Enter` open that day's list, `u`/`Ctrl+R` undo/redo, `?` help, `Esc`/`q`/`c` back to the list.

Insert mode: type to insert at the cursor, `←`/`→`/`Home`/`End` move, `Backspace`/`Delete` remove, `Enter`/`Esc` save. Saving an empty new item discards it. Saving an edited item as empty opens the delete popup.

Notes screen, normal mode: `h`/`j`/`k`/`l`, `w`/`b`/`e` (`Ctrl+→` goes to a word's end like `e`, and while typing just after it; `Ctrl+←` to its start like `b`), `0`/`$` (and `Home`/`End`; `Ctrl+Home`/`Ctrl+End` go to the note's start/end, handled by todoro in insert mode too, since the text area treats them as the line's), `_`/`^` (first non-blank; `gg`/`G`/`:42` land there too), `gg`/`G` move, with a count (`4j`, `42G`; also `3x`, `2dd`); `:` opens a command line (`NotesEditor::command`, drawn in place of the status bar): `:42` goes to line 42, `:q`/`:wq`/`:x` close, `:w` does nothing; `i`/`a`/`I`/`A`/`o`/`O` enter insert mode; `x` deletes a character, `dd` a line (into the register), `yy` copies a line, `v` select text (motions extend it; `y` copy, `d`/`x` cut, `J`/`K` move its lines and keep it selected, `Esc`/`v` cancel; `V` selects whole lines (`NotesEditor::visual_lines`, painted by `paint_lines` in `ui.rs` since the text area only highlights up to the cursor; `y`/`d` copy/delete them as lines; `v`/`V` switch between the two); inclusive at both ends, and pastes within a line), `p`/`P` paste below/above (`NotesEditor::register`, kept by the app across notes screens; separate from the list's register of items); `J`/`K` move the line down/up (unlike vim's `J`, which joins lines, to match the list); `u`/`Ctrl+R` undo/redo; `?` help; `Esc`/`q` back to the list. Notes insert mode: type freely, `Esc` back to normal mode. Anything not listed here is not implemented (only the `:` commands above).

Help popup (from the list or notes normal mode): every typed character goes into the search, `↑`/`↓`/`Ctrl+N`/`Ctrl+P`/`PageUp`/`PageDown` scroll, `Backspace` and `Ctrl+U` edit the search, `Esc` returns to the screen it was opened from. One-character searches match key names only (case-sensitive); longer ones match keys or descriptions (case-insensitive).

Keep new bindings vim-like. When you add or change one, update `SECTIONS` in `src/help.rs` (what `?` shows), the hints in `src/ui.rs` (bottom border and status bar), the list above and the tables in `README.md`.

## Conventions

- Each day's items are stored with the open ones first and the completed ones after; `Store` keeps this order (`open_count` gives the boundary). `x` moves an item to the boundary: the top of the completed items or the bottom of the open ones. Only open items are numbered in the UI; completed ones follow a "Completed" header row, so their list row is their index plus one.
- Items stay on their day by default, unless the "Pin new items" option (`Settings::pin_new_items`) is on: then items added with `a`/`A` or in the calendar start pinned, through `App::add_item` (pasted items keep their own pin). On startup, `Store::roll_over` moves pinned, open items from earlier days to the top of today, still pinned. An open item with a deadline moves the same way, but only as far as its deadline day (`Item::roll_to`), and setting a deadline unpins it (`Store::set_deadline`). Unpinned and completed items stay on their day.
- Deadlines show in the pin's place (`◷` and `Deadline::describe`, on the 12-hour clock with `Settings::twelve_hour`): yellow on the day, red and bold once passed, judged by `App::now`, which the event loop sets before each draw. On disk they're `"deadline": "2026-10-12"` or `"2026-10-12 13:00"`.
- Bracketed paste is on, so text pasted into the terminal arrives as one `Event::Paste`, not as keys, and goes to `App::handle_paste`: notes insert it with its lines (`NotesEditor::paste_text`, one undo step), single-line inputs get it on one line (`LineInput::paste`), and anywhere else it's ignored so it can never run as commands. Old Windows consoles can't enable it; then a paste arrives as keys, as before.
- The event loop also wakes when `App::redraw_at` says something on screen is timed, such as the brief (0.1 s) flash of lines copied in the notes (`NotesEditor::flash`), and calls `App::tick` to end it. Timed things take the current `Instant` as an argument (`tick`, `expire_flash`), so tests can pass any time instead of waiting.
- The event loop wakes every 30 seconds to call `App::set_today`, so midnight is noticed while todoro is open. It only acts in `Normal` mode (positions mustn't shift mid-edit), carries pinned items over and clears undo history.
- Today and future days also show the pinned, open items from earlier days, and ones due on that day or later (`Store::carried_to`), first, without moving them. So an unfinished pinned item shows on its own day and every day after it. So screen positions aren't always store positions: `App::slots()` maps each row on screen to the `(day, index)` where its item is stored. Use it (or `App::items()`) for anything that reads or changes the selected item, never `store.items(app.day)[app.selected]`.
- On disk an item is a plain string, or an object with `text` and optional `notes_file`, `done`, `pinned` and `priority` (`"high"`, `"medium"` or `"low"`) (see `RawItem` in `store.rs`). Items with only text must stay plain strings. An inline `notes` field (from before notes files) is read and moved into a file on open, but never written.
- Notes are plain Markdown files, one per item with notes, named from the item's text when first written (`buy-milk.md`, `buy-milk-2.md`) and never renamed. In memory, `Item::notes` still holds the text, so nothing outside `Store` handles files; `NotesFiles::sync` writes, names and deletes files on every save. It never overwrites or deletes a file that changed outside todoro since todoro last read or wrote it: the outside version stays (and becomes the item's notes) and todoro's goes in `<name> (conflict).md`, reported through `Store::take_conflicts` and shown by `App::report_conflicts`. Opening an item's notes rereads its file (`Store::reload_notes`).
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

Todos live in the todoro folder the user chose (`Settings::data_dir`), one folder per workspace, each with a `todos.json`. Settings are in `~/.config/todoro/settings.json`. Unit tests leave `App::settings_path` as `None`, so they never save settings, and use temporary folders.

When running the app to test it, never touch the user's real todos or settings:
- Always set `TODORO_SETTINGS` to a scratch file. With `TODORO_FILE` also set to a scratch file, todoro uses just that file and skips the folder and workspaces.
- To test setup or workspaces, leave `TODORO_FILE` unset and give the scratch settings a `data_dir` in a scratch folder (or let setup create one there).
- Setup moves the user's pre-folder todos from `~/.local/share/todoro/todos.json` into the new workspace. On Linux, set `XDG_DATA_HOME` to a scratch folder when testing setup, or it will move the user's real file.

To drive the real TUI, use a separate tmux server (`tmux -L todoro-test ...`) and target sessions with an exact match (`-t '=name:'`). The user runs their own tmux, and a plain `-t name` can prefix-match their windows and send keystrokes into them.

## Releasing

Releases are built by [dist](https://github.com/axodotdev/cargo-dist) in `.github/workflows/release.yml`, which runs when a version tag is pushed. It builds Linux and macOS (x86_64 and ARM) and Windows (x86_64), and publishes them to GitHub Releases with `curl | sh` and PowerShell installers.

To release a new version, with the user's go-ahead:

1. Draft the new `CHANGELOG.md` section with [git-cliff](https://git-cliff.org): `git cliff --unreleased --tag vX.Y.Z --prepend CHANGELOG.md` (it reads `cliff.toml`). Move the new section below the file's intro if it lands above it, and edit it into plain, user-facing wording: a commit for a change that a later commit replaced shouldn't be listed. The section's heading must be `## [X.Y.Z] - YYYY-MM-DD`; dist uses it as the release notes.
2. Set `version` in `Cargo.toml` (run `cargo build` so `Cargo.lock` updates too), and commit both as `chore: release vX.Y.Z`.
3. Check the notes with `dist manifest --tag vX.Y.Z --output-format=json` (`announcement_changelog`), then `git tag vX.Y.Z` and `git push origin main vX.Y.Z`. The tag must match the version.

The changelog's sections are shown inside todoro (`N` and "what's new"), rendered from a small part of Markdown: `###` headings, `- ` bullets, paragraphs and `code`. Keep release notes to those. A test fails if the newest section isn't the version in `Cargo.toml`, so bump the version and add its notes together.

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
