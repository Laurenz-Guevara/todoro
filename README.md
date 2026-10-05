# todoro

A terminal todo app with vim-style keys. It opens on today's list, and you move between days with `h` and `l`. Mark items done with `x`, and pin the ones you want to follow you to the next day with `m`. Any item can have longer notes, which you open with `Enter`, and a calendar (`c`) lets you plan weeks or months ahead.

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

**macOS and Linux:**

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/Laurenz-Guevara/todoro/releases/latest/download/todoro-installer.sh | sh
```

**Windows** (PowerShell):

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/Laurenz-Guevara/todoro/releases/latest/download/todoro-installer.ps1 | iex"
```

Both put `todoro` in `~/.local/bin` and add it to your `PATH` if needed, so you may need to open a new terminal afterwards. Then run `todoro`.

You can also download the program yourself from the [latest release](https://github.com/Laurenz-Guevara/todoro/releases/latest): there are builds for Linux and macOS (Intel and ARM) and Windows. Unpack it and put `todoro` (or `todoro.exe`) somewhere on your `PATH`.

### From source

With Rust 1.88 or newer (install it with [rustup](https://rustup.rs)):

```sh
cargo install --git https://github.com/Laurenz-Guevara/todoro
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
| `t` | Go to today |
| `j` / `k` | Move down / up |
| `gg` / `G` | Jump to the first / last item |
| `4j` / `4k` | Move 4 items down / up (any number works) |
| `42G` | Go to item 42 |
| `J` / `K` | Move the selected item down / up the list |
| `H` / `L` | Move the selected item to the previous / next day, and go with it |
| `a` | Add an item below the cursor |
| `A` | Add several items: `Enter` adds each one and starts the next, `Esc` stops |
| `e` | Edit the selected item |
| `x` | Mark the selected item done, or not done again |
| `m` | Pin or unpin the selected item, so it moves to the next day until done |
| `!` | Triage the selected item: High, Medium, Low, then no priority again |
| `d` | Delete the selected item (asks to confirm) |
| `yy` | Copy the selected item |
| `p` / `P` | Paste the copied (or last deleted) item below / above the cursor |
| `V` | Select several items |
| `Enter` | Open the selected item's notes |
| `u` / `Ctrl+R` | Undo / redo a change to the list |
| `c` | Open the calendar |
| `s` / `S` | Fuzzy search every day's items / items and their notes |
| `#` | List your tags, to show every item with one |
| `o` | Options, including accessibility |
| `?` | Show all keybindings |
| `q` | Quit |

Undo covers everything you change from the list, including moving items between days and a whole visit to an item's notes, and takes you back to where the change was made.

Pinned items are marked with `⚲` and items that have notes with `≡`. A triaged item's number is coloured by its priority: red for High, yellow for Medium and green for Low. If colours are hard to tell apart, turn on semantic icons in the options (`o`) to also show `∧` High, `–` Medium and `∨` Low.

### Selecting several items

`V` starts selecting at the cursor, and `j` / `k` extend the selection. Then:

| Key | Action |
|---|---|
| `x` | Complete them all, or reopen them if they're all complete |
| `m` | Pin them all, or unpin them if they're all pinned |
| `!` | Give them all the next priority |
| `d` | Delete them (asks once) |
| `y` | Copy them, to paste with `p` |
| `H` / `L` | Move them to the previous / next day, and go with them |
| `Esc` / `V` | Stop selecting |

### Completed items

`x` moves an item under the Completed header, crossed out. The cursor stays where it was, so you can tick off several items in a row. `x` on a completed item moves it back to the bottom of the numbered list.

### Pinned items

Items stay on their day by default, so something like "Dentist at 3pm" doesn't follow you around. Pin an item with `m` if it should: when you start todoro, pinned items you didn't complete on earlier days move to the top of today's list, keeping their notes. They stay pinned, so they keep moving forward each day until you complete or unpin them.

Looking ahead with `l` shows pinned items on future days too, at the top of the list, as they'll be when that day comes. In general, an unfinished pinned item shows on its own day and every day after it, so if you move one to an earlier day with `H`, it still shows today. They're the same items, so completing, editing or unpinning one there changes it on the day it's on now.

If todoro is open at midnight, this happens then too, and the list moves on to the new day.

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

With `A`, `Enter` adds the item and starts a new one below it, so you can type a whole list in one go. `Esc` saves what you've typed and stops; `Enter` on an empty line stops too.

### Notes

`Enter` on an item opens its notes: free text that isn't shown on the main list. The notes screen has its own small vim-style editor, with line numbers, and saves as you type.

In normal mode:

| Key | Action |
|---|---|
| `h` `j` `k` `l` | Move |
| `w` / `b` / `e` | Next word / previous word / end of word |
| `0` / `$` | Start / end of the line |
| `_` / `^` | First non-blank character of the line |
| `gg` / `G` | First / last line |
| `4j` / `4k` | Move 4 lines down / up (a count works with most keys, like `3x` or `2dd`) |
| `:42` / `42G` | Go to line 42 |
| `:q` / `:wq` | Back to the list |
| `i` / `a` | Insert before / after the cursor |
| `I` / `A` | Insert at the start / end of the line |
| `o` / `O` | Open a new line below / above |
| `x` | Delete the character under the cursor |
| `dd` | Delete the line, keeping it to paste |
| `yy` | Copy the line |
| `p` / `P` | Paste below / above (also into another item's notes) |
| `v` | Select text by moving the cursor; then `y` copies it, `d` cuts it and `Esc` cancels |
| `J` / `K` | Move the line down / up |
| `u` / `Ctrl+R` | Undo / redo |
| `?` | Show all keybindings |
| `Esc` / `q` | Back to the list |

In insert mode, type as normal (`Enter` starts a new line) and press `Esc` to go back to normal mode.

### Tags

Write `#words` in an item to tag it, like "Call #work about the #budget". Tags are coloured on the list and ignore case, so `#Work` and `#work` are the same tag. `#` lists every tag with how many items have it; pick one with `j` / `k` and press `Enter` to see all its items across every day, where you can type to narrow them down and press `Enter` to go to one.

### Search

`s` opens a fuzzy search over the items on every day, and `S` searches their notes too. Type a few letters in order, like `dntst` for "Dentist at 3pm"; separate words match separately, capitals only match capitals, and plain letters match accented ones. The best matches come first, with the closest days first among equals, and the matched letters are highlighted. With `S`, the matching line of an item's notes shows under it.

`↑` / `↓` (or `Ctrl+N` / `Ctrl+P`) select a result, `Enter` goes to that item on its day, and `Esc` closes the search.

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

### Options

`o` opens the options. `j` / `k` select one, `Space` or `Enter` turns it on or off, and `Esc` closes them. They're saved and remembered next time.

**Accessibility**

- **Semantic priority icons:** show `∧` High, `–` Medium and `∨` Low beside triaged items, as well as the coloured number, for anyone who can't tell the colours apart.
- **No colours:** draw everything in your terminal's own colours. Highlights such as the selected item use reversed text instead. This starts on if you set the standard [`NO_COLOR`](https://no-color.org) environment variable, until you change it here.

### Help

`?` opens a popup listing every key, from the main list or the notes screen. Start typing to search: a single character such as `x` or `G` finds that key, and a word such as `undo` or `esc` finds keys by name or by what they do. `↑` / `↓` scroll and `Esc` closes it.

### Delete popup

Press `d` to delete the item, or `c` (or `Esc`) to cancel.

`Ctrl+C` quits from any mode.

## Data

Todos are saved after every change, as JSON grouped by date. Items with only text are plain strings, and others are objects like `{ "text": ..., "notes": ..., "done": true, "pinned": true, "priority": "high" }`:

| OS | Location |
|---|---|
| Linux | `~/.local/share/todoro/todos.json` |
| macOS | `~/Library/Application Support/todoro/todos.json` |
| Windows | `%APPDATA%\todoro\todos.json` |

Options are saved separately, in `settings.json` in your config folder (`~/.config/todoro/` on Linux, `~/Library/Application Support/todoro/` on macOS, `%APPDATA%\todoro\` on Windows), or wherever `TODORO_SETTINGS` points.

To use a different todo file, set `TODORO_FILE`:

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

## Licence

[MIT](LICENSE)
