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
- `src/app.rs`: app state, modes (`Normal`, `Insert`, `ConfirmDelete`, `Notes`) and key handling
- `src/notes.rs`: the notes editor, a vim key layer over `ratatui-textarea`
- `src/ui.rs`: all rendering (list, status bar, delete popup)
- `src/store.rs`: JSON persistence, keyed by `YYYY-MM-DD`
- `src/test_util.rs`: helpers shared by the tests
- `src/snapshots/`: saved screen snapshots for the UI tests

## Keybindings

Normal mode: `h`/`l` previous/next day, `j`/`k` move down/up, `a` add below the cursor, `e` edit, `d` delete (opens a popup; `d` confirms, `c` cancels), `Enter` open notes, `q` quit.

Insert mode: type to insert at the cursor, `←`/`→`/`Home`/`End` move, `Backspace`/`Delete` remove, `Enter`/`Esc` save. Saving an empty new item discards it. Saving an edited item as empty opens the delete popup.

Notes screen, normal mode: `h`/`j`/`k`/`l`, `w`/`b`/`e`, `0`/`$`, `gg`/`G` move; `i`/`a`/`I`/`A`/`o`/`O` enter insert mode; `x` deletes a character, `dd` a line; `u`/`Ctrl+R` undo/redo; `Esc`/`q` back to the list. Notes insert mode: type freely, `Esc` back to normal mode. Anything not listed here is not implemented (no visual mode, `:` commands or counts).

Keep new bindings vim-like. When you add or change one, update the hints in `src/ui.rs` (bottom border and status bar), the list above and the tables in `README.md`.

## Conventions

- Items are numbered from 1 in the UI. On disk an item is a plain string, or `{ "text", "notes" }` once it has notes (see `RawItem` in `store.rs`). Files without notes must stay readable by versions from before notes existed.
- `Store` saves after every change by writing a temp file and renaming it. Don't defer or batch saves.
- `Insert { cursor }` is a byte offset that must stay on a char boundary. Use `prev_boundary`/`next_boundary` in `app.rs`.
- The notes editor must keep vim's behavior where it differs from `ratatui-textarea`'s defaults: `h`/`l`/`x` never cross line boundaries, the normal-mode cursor never sits past the last character, and each command is one undo step.

## Tests

Every new feature or bug fix comes with tests in the same commit. Tests live in a `#[cfg(test)] mod tests` at the bottom of the file they cover:

- `app.rs`: key handling. Build an app with `test_util::app_with(&[...])` and send keys with `press` and `type_str`. Cover the behavior, edge cases (empty list, first/last item, multibyte text) and that keys meant for one mode do nothing in the others.
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
