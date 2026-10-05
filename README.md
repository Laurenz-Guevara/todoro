# todoro

A terminal todo app with vim-style keys. It opens on today's list, and you move between days with `h` and `l`. Any item can have longer notes, which you open with `Enter`.

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
| `Enter` | Open the selected item's notes |
| `q` | Quit |

Items that have notes are marked with `≡`.

### Insert mode

Adding or editing an item puts you in insert mode.

| Key | Action |
|---|---|
| `←` / `→` | Move the cursor |
| `Home` / `End` | Jump to the start / end of the line |
| `Backspace` / `Delete` | Delete before / under the cursor |
| `Enter` / `Esc` | Save |

Saving a new item with no text discards it. Clearing all the text from an existing item and saving asks whether to delete it.

### Notes

`Enter` on an item opens its notes: free text that isn't shown on the main list. The notes screen has its own small vim-style editor and saves as you type.

In normal mode:

| Key | Action |
|---|---|
| `h` `j` `k` `l` | Move |
| `w` / `b` / `e` | Next word / previous word / end of word |
| `0` / `$` | Start / end of the line |
| `gg` / `G` | First / last line |
| `i` / `a` | Insert before / after the cursor |
| `I` / `A` | Insert at the start / end of the line |
| `o` / `O` | Open a new line below / above |
| `x` | Delete the character under the cursor |
| `dd` | Delete the line |
| `u` / `Ctrl+R` | Undo / redo |
| `Esc` / `q` | Back to the list |

In insert mode, type as normal (`Enter` starts a new line) and press `Esc` to go back to normal mode.

### Delete popup

Press `d` to delete the item, or `c` (or `Esc`) to cancel.

`Ctrl+C` quits from any mode.

## Data

Todos are saved after every change, as JSON grouped by date. Items without notes are plain strings, and items with notes are `{ "text": ..., "notes": ... }`:

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
