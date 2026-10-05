# todoro

A vim-style terminal todo app in Rust with [ratatui](https://ratatui.rs). It runs as `todoro` and opens on today's list.

## Commands

```sh
cargo build                 # build
cargo clippy                # lint; keep it warning-free
cargo install --path .      # install/update the `todoro` binary in ~/.cargo/bin
```

## Layout

- `src/main.rs`: terminal setup/teardown and the event loop
- `src/app.rs`: app state, modes (`Normal`, `Insert`, `ConfirmDelete`) and key handling
- `src/ui.rs`: all rendering (list, status bar, delete popup)
- `src/store.rs`: JSON persistence, keyed by `YYYY-MM-DD`

## Keybindings

Normal mode: `h`/`l` previous/next day, `j`/`k` move down/up, `a` add below the cursor, `e` edit, `d` delete (opens a popup; `d` confirms, `c` cancels), `q` quit.

Insert mode: type to insert at the cursor, `←`/`→`/`Home`/`End` move, `Backspace`/`Delete` remove, `Enter`/`Esc` save. Saving an empty new item discards it. Saving an edited item as empty opens the delete popup.

Keep new bindings vim-like. When you add or change one, update the hints in `src/ui.rs` (bottom border and status bar) and the list above.

## Conventions

- Items are numbered from 1 in the UI and stored as plain strings.
- `Store` saves after every change by writing a temp file and renaming it. Don't defer or batch saves.
- `Insert { cursor }` is a byte offset that must stay on a char boundary. Use `prev_boundary`/`next_boundary` in `app.rs`.

## Data and testing

Todos are stored in `~/.local/share/todoro/todos.json` by default. Set `TODORO_FILE` to use another file, and always do this when testing so the user's real data is never touched.

To drive the TUI in tests, use a separate tmux server (`tmux -L todoro-test ...`) and target sessions with an exact match (`-t '=name:'`). The user runs their own tmux, and a plain `-t name` can prefix-match their windows and send keystrokes into them.

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
