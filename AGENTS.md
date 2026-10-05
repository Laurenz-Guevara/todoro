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
- `src/notes.rs`: the notes editor, a vim key layer over `ratatui-textarea`
- `src/help.rs`: the `?` popup's keybinding table (`SECTIONS`) and search
- `src/ui.rs`: all rendering (list, notes screen, status bar, delete and help popups)
- `src/store.rs`: JSON persistence, keyed by `YYYY-MM-DD`
- `src/test_util.rs`: helpers shared by the tests
- `src/snapshots/`: saved screen snapshots for the UI tests

## Keybindings

Normal mode: `h`/`l` previous/next day, `j`/`k` move down/up, `J`/`K` move the item down/up (within the open or completed items, and only among items stored on the same day), `H`/`L` move the item to the day before/after the one on screen and follow it there, `a` add below the cursor (or at the end of the open items when on a completed one), `e` edit, `x` toggle done, `p` toggle pinned, `d` delete (opens a popup; `d` confirms, `c` cancels), `Enter` open notes, `u`/`Ctrl+R` undo/redo, `?` help, `q` quit.

Insert mode: type to insert at the cursor, `←`/`→`/`Home`/`End` move, `Backspace`/`Delete` remove, `Enter`/`Esc` save. Saving an empty new item discards it. Saving an edited item as empty opens the delete popup.

Notes screen, normal mode: `h`/`j`/`k`/`l`, `w`/`b`/`e`, `0`/`$`, `gg`/`G` move; `i`/`a`/`I`/`A`/`o`/`O` enter insert mode; `x` deletes a character, `dd` a line; `u`/`Ctrl+R` undo/redo; `?` help; `Esc`/`q` back to the list. Notes insert mode: type freely, `Esc` back to normal mode. Anything not listed here is not implemented (no visual mode, `:` commands or counts).

Help popup (from the list or notes normal mode): every typed character goes into the search, `↑`/`↓`/`Ctrl+N`/`Ctrl+P`/`PageUp`/`PageDown` scroll, `Backspace` and `Ctrl+U` edit the search, `Esc` returns to the screen it was opened from. One-character searches match key names only (case-sensitive); longer ones match keys or descriptions (case-insensitive).

Keep new bindings vim-like. When you add or change one, update `SECTIONS` in `src/help.rs` (what `?` shows), the hints in `src/ui.rs` (bottom border and status bar), the list above and the tables in `README.md`.

## Conventions

- Each day's items are stored with the open ones first and the completed ones after; `Store` keeps this order (`open_count` gives the boundary). `x` moves an item to the boundary: the top of the completed items or the bottom of the open ones. Only open items are numbered in the UI; completed ones follow a "Completed" header row, so their list row is their index plus one.
- Items stay on their day by default. On startup, `Store::roll_over` moves pinned, open items from earlier days to the top of today, still pinned. Unpinned and completed items stay on their day.
- Today and future days also show the pinned, open items from earlier days (`Store::pinned_before`), first, without moving them. So an unfinished pinned item shows on its own day and every day after it. So screen positions aren't always store positions: `App::slots()` maps each row on screen to the `(day, index)` where its item is stored. Use it (or `App::items()`) for anything that reads or changes the selected item, never `store.items(app.day)[app.selected]`.
- On disk an item is a plain string, or an object with `text` and optional `notes`, `done` and `pinned` (see `RawItem` in `store.rs`). Items with only text must stay plain strings so simple files stay readable by older versions.
- `Store` saves after every change by writing a temp file and renaming it. Don't defer or batch saves.
- List undo is automatic: `App::handle_key` snapshots the store before each key in `Normal`, `Insert` and `ConfirmDelete` mode and records it if the key changed anything. A notes visit is recorded as one change when it closes (`notes_before`). New list actions need no undo code, but must change the store only through `App::handle_key`.
- `Insert { cursor }` is a byte offset that must stay on a char boundary. Use `prev_boundary`/`next_boundary` in `app.rs`.
- The UI must work down to about 30 columns. Any text with a fixed length needs narrower fallbacks: `fit_first` (shorter alternatives), `fit_hints` (status bar hints by priority, `? help` last to go) or `truncate` (ends with "…"). The `screens_at_30_columns` and `screens_at_40_columns` snapshots cover every screen, so check them after any UI change.
- Long text wraps rather than being cut off: list rows and the delete popup both use `wrap_ranges` in `ui.rs`, which keeps the text exactly as typed (so the cursor can be placed with `cursor_position`) and measures display width with `unicode-width`, not `chars().count()`. List rows can be several lines tall, so screen positions come from summing row heights.
- The notes editor must keep vim's behavior where it differs from `ratatui-textarea`'s defaults: `h`/`l`/`x` never cross line boundaries, the normal-mode cursor never sits past the last character, and each command is one undo step.

## Tests

Every new feature or bug fix comes with tests in the same commit. Tests live in a `#[cfg(test)] mod tests` at the bottom of the file they cover:

- `app.rs`: key handling. Build an app with `test_util::app_with(&[...])` and send keys with `press` and `type_str`. Cover the behavior, edge cases (empty list, first/last item, multibyte text) and that keys meant for one mode do nothing in the others.
- `help.rs`: search matching and the popup's own keys.
- `notes.rs`: the notes editor's keys, driven with the `send` helper (`<esc>` and `<cr>` stand for Escape and Enter).
- `store.rs`: persistence, always against a temporary file.
- `ui.rs`: screen snapshots with [insta](https://insta.rs). Add one for any new screen or popup.

Tests run on a fixed date (`test_util::today()`, 2026-10-05) and must never read the clock, the real data file or `TODORO_FILE`.

### Snapshots

A snapshot test draws the UI into memory and compares it with the saved copy in `src/snapshots/`. When a change to the UI is intended, the test fails and prints a diff. To accept it:

1. Read the diff and check that every changed line is meant to change.
2. Rerun with `INSTA_UPDATE=always cargo test` to overwrite the saved copies, or use `cargo insta review` if `cargo-insta` is installed.
3. Read the new `.snap` files before committing. A snapshot only records what the code drew, so a wrong one locks a bug in.

Never update snapshots just to make a failing test pass without reading the diff.

## Data and manual testing

Todos are stored in `~/.local/share/todoro/todos.json` by default. Set `TODORO_FILE` to use another file, and always do this when running the app to test it so the user's real data is never touched.

To drive the real TUI, use a separate tmux server (`tmux -L todoro-test ...`) and target sessions with an exact match (`-t '=name:'`). The user runs their own tmux, and a plain `-t name` can prefix-match their windows and send keystrokes into them.

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
