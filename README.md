# todoro

A terminal todo app with vim-style keys. It opens on today's list, and you move between days with `h` and `l`.

```
╭───────────── Monday, October 5 2026 (today) ─────────────╮
│1. Buy milk                                               │
│2. Write report                                           │
│3. Call mom                                               │
│                                                          │
╰──── h ← prev day · k ↑ up · j ↓ down · next day → l ─────╯
 NORMAL   a add  e edit  d delete  q quit
```

## Install

You need Rust 1.88 or newer. Install it with [rustup](https://rustup.rs) if you don't have it.

```sh
cargo install --git https://github.com/Laurenz-Guevara/todoro
```

Or from a local clone:

```sh
git clone https://github.com/Laurenz-Guevara/todoro
cd todoro
cargo install --path .
```

This puts `todoro` in `~/.cargo/bin`, which rustup adds to your `PATH`.

## Usage

```sh
todoro
```

### Normal mode

| Key | Action |
|---|---|
| `h` / `l` | Previous / next day |
| `j` / `k` | Move down / up |
| `a` | Add an item below the cursor |
| `e` | Edit the selected item |
| `d` | Delete the selected item (asks to confirm) |
| `q` | Quit |

### Insert mode

Adding or editing an item puts you in insert mode.

| Key | Action |
|---|---|
| `←` / `→` | Move the cursor |
| `Home` / `End` | Jump to the start / end of the line |
| `Backspace` / `Delete` | Delete before / under the cursor |
| `Enter` / `Esc` | Save |

Saving a new item with no text discards it. Clearing all the text from an existing item and saving asks whether to delete it.

### Delete popup

Press `d` to delete the item, or `c` (or `Esc`) to cancel.

`Ctrl+C` quits from any mode.

## Data

Todos are saved after every change, as JSON grouped by date:

| OS | Location |
|---|---|
| Linux | `~/.local/share/todoro/todos.json` |
| macOS | `~/Library/Application Support/todoro/todos.json` |
| Windows | `%APPDATA%\todoro\todos.json` |

To use a different file, set `TODORO_FILE`:

```sh
TODORO_FILE=~/work-todos.json todoro
```

## Development

```sh
cargo run                    # run without installing
cargo test                   # run the tests
cargo clippy --all-targets   # lint
```

The UI tests compare the screen against saved snapshots in `src/snapshots/`. If you change the UI on purpose, check the diff the failing test prints, then accept the new snapshots with `INSTA_UPDATE=always cargo test`.

See [AGENTS.md](AGENTS.md) for the project layout, conventions and commit format. It's written for coding agents, but it applies to people too.
