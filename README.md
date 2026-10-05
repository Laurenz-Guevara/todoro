# todoro

A terminal todo app with vim-style keys. It opens on today's list, and you move between days with `h` and `l`. Mark items done with `x`, and pin the ones you want to follow you to the next day with `p`. Any item can have longer notes, which you open with `Enter`, and a calendar (`c`) lets you plan weeks or months ahead.

```
╭───────────── Monday, October 5 2026 (today) ─────────────╮
│1. Buy milk                                               │
│2. Book dentist                                           │
│── Completed (2) ─────────────────────────────────────────│
│✓  Call mom                                               │
│✓  Write report ≡                                         │
╰──── h ← prev day · k ↑ up · j ↓ down · next day → l ─────╯
 NORMAL   a add  e edit  x done  d delete  ↵ notes  ? help
```

Press `?` at any time for a searchable list of every key.

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
| `J` / `K` | Move the selected item down / up the list |
| `H` / `L` | Move the selected item to the previous / next day, and go with it |
| `a` | Add an item below the cursor |
| `e` | Edit the selected item |
| `x` | Mark the selected item done, or not done again |
| `p` | Pin or unpin the selected item, so it moves to the next day until done |
| `d` | Delete the selected item (asks to confirm) |
| `Enter` | Open the selected item's notes |
| `u` / `Ctrl+R` | Undo / redo a change to the list |
| `c` | Open the calendar |
| `?` | Show all keybindings |
| `q` | Quit |

Undo covers everything you change from the list, including moving items between days and a whole visit to an item's notes, and takes you back to where the change was made.

Pinned items are marked with `⚲` and items that have notes with `≡`.

### Completed items

`x` moves an item under the Completed header, crossed out. The cursor stays where it was, so you can tick off several items in a row. `x` on a completed item moves it back to the bottom of the numbered list.

### Pinned items

Items stay on their day by default, so something like "Dentist at 3pm" doesn't follow you around. Pin an item with `p` if it should: when you start todoro, pinned items you didn't complete on earlier days move to the top of today's list, keeping their notes. They stay pinned, so they keep moving forward each day until you complete or unpin them.

Looking ahead with `l` shows pinned items on future days too, at the top of the list, as they'll be when that day comes. In general, an unfinished pinned item shows on its own day and every day after it, so if you move one to an earlier day with `H`, it still shows today. They're the same items, so completing, editing or unpinning one there changes it on the day it's on now.

Completed items always stay on the day you completed them, so you can go back with `h` to see what you did.

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
| `?` | Show all keybindings |
| `Esc` / `q` | Back to the list |

In insert mode, type as normal (`Enter` starts a new line) and press `Esc` to go back to normal mode.

### Calendar

`c` opens a calendar on the day you're viewing, for planning ahead without stepping through days one at a time. It has three views:

- **Week** (`w`): each day of the week with its items.
- **Month** (`m`, the default): a grid with each day's items, or a `•` on days with open items when there's no room for them.
- **Year** (`y`): all twelve months, with days that have open items highlighted.

| Key | Action |
|---|---|
| `h` `j` `k` `l` | Move by day and week. In the month and year views `h`/`l` move a day and `j`/`k` a week; the week view lists days top to bottom, so there `j`/`k` move a day and `h`/`l` a week |
| `H` / `L` | Previous / next month |
| `t` | Jump to today |
| `w` / `m` / `y` | Week / month / year view |
| `a` | Add an item to the selected day, without leaving the calendar |
| `Enter` | Open the selected day's list |
| `u` / `Ctrl+R` | Undo / redo |
| `Esc` / `q` / `c` | Back to the list, on the day you were on |

The calendar shows the items that belong to each day. Pinned items aren't repeated on every later day there, so they don't fill the whole calendar.

### Help

`?` opens a popup listing every key, from the main list or the notes screen. Start typing to search: a single character such as `x` or `G` finds that key, and a word such as `undo` or `esc` finds keys by name or by what they do. `↑` / `↓` scroll and `Esc` closes it.

### Delete popup

Press `d` to delete the item, or `c` (or `Esc`) to cancel.

`Ctrl+C` quits from any mode.

## Data

Todos are saved after every change, as JSON grouped by date. Items with only text are plain strings, and others are objects like `{ "text": ..., "notes": ..., "done": true }`:

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
