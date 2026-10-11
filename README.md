# todoro

A terminal todo app with vim-style keys. It opens on today's list, and you move between days with `h` and `l`. Mark items done with `x`, and pin the ones you want to follow you to the next day with `m`. Any item can have longer notes, which you open with `Enter`, and a calendar (`Space` `c`) lets you plan weeks or months ahead.

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
todoro             # open todoro
todoro --where     # print the todoro folder, e.g. to back it up: cp -r "$(todoro --where)" /backup/
todoro --version
todoro --help
```

### Normal mode

These are the default keys. You can change any of them, and the leader menu's, in your own `keys.toml`: see [Changing keys](#changing-keys).

| Key | Action |
|---|---|
| `h` / `l` | Previous / next day |
| `t` | Go to today |
| `j` / `k` | Move down / up |
| `gg` / `G` | Jump to the first / last item |
| `4j` / `4k` | Move 4 items down / up (any number works) |
| `42G` / `:42` | Go to item 42 |
| `J` / `K` | Move the selected item down / up the list (a sub-item among the others under its parent) |
| `H` / `L` | Move the selected item to the previous / next day, and go with it |
| `a` | Add an item below the cursor |
| `A` | Add several items: `Enter` adds each one and starts the next, `Esc` stops |
| `o` | Add a sub-item under the selected item |
| `O` | Add several sub-items under it, like `A`: `Enter` adds each one and starts the next, `Esc` stops |
| `>` / `<` | Make the item a sub-item of the one above it / move it back out a level |
| `za` / `Shift+Enter` | Fold or unfold the item's sub-items |
| `zc` / `zo` | Fold / unfold them |
| `e` | Edit the selected item, with the cursor at the end |
| `E` | Edit the selected item with its text cleared, to write it again (leaving it empty keeps it as it was) |
| `x` | Mark the selected item done, or not done again |
| `m` | Pin or unpin the selected item, so it moves to the next day until done |
| `@` / `:deadline` | Set a deadline on the selected item |
| `go` / `:overdue` | List the items past their deadline |
| `r` / `:remind` | Set a reminder on the selected item |
| `!` | Triage the selected item: High, Medium, Low, then no priority again |
| `d` | Delete the selected item (asks to confirm) |
| `yy` | Copy the selected item |
| `p` / `P` | Paste the copied (or last deleted) item below / above the cursor |
| `V` | Select several items |
| `Enter` | Edit the selected item's notes, in your own editor if you've set one |
| `v` | View the selected item's notes, formatted |
| `u` / `Ctrl+R` | Undo / redo a change to the list |
| `Space` | Open the menu of the commands below |
| `Space` `Space` | Find items by their text, on any day |
| `Space` `s` `g` | Search inside every item's notes |
| `Space` `s` `d` | List every open item with a deadline, soonest first |
| `Space` `s` `r` | List every open item with a reminder, soonest first |
| `Space` `s` `t` | List your tags, to show every item with one |
| `Space` `s` `o` | Search the options |
| `Space` `s` `k` | Search the keybindings: the same help as `?` |
| `Space` `c` | Open the calendar |
| `Space` `o` | Options: accessibility, your own notes editor, the todoro folder |
| `Space` `w` | Workspaces: switch, create or delete |
| `Space` `n` | Patch notes: what changed in every release |
| `Space` `t` | The day as a timeline, with the time now |
| `Space` `p` | Plan the day: give items a start and end time |
| `?` | Show all keybindings |
| `q` / `:q` | Quit |

Keys you use all the time have one key each. Less frequent commands are behind `Space`: press it and a menu along the bottom shows what each next key does, like which-key in Neovim. Each key you type narrows it down to the commands that still match, and runs the one it finishes. A key that matches nothing closes the menu, as does `Esc`; `Backspace` steps back a key.

Undo covers everything you change from the list, including moving items between days and a whole visit to an item's notes, and takes you back to where the change was made.

Pinned items are marked with `⚲` and items that have notes with `≡`. A triaged item's number is coloured by its priority, including while it's selected: red for High, yellow for Medium and green for Low. If colours are hard to tell apart, turn on semantic icons in the options (`Space` `o`) to also show `∧` High, `–` Medium and `∨` Low.

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

### Sub-items

Any item can have sub-items, and those can have their own, as deep as you like:

```
1. Plan the trip 1/3
   ✓ Book flights
   ↳ Hotel ▸ 0/2
   ↳ Pack
2. Buy milk
```

`o` adds a sub-item under the selected item, and `O` adds several, one after another, as `A` does. Or add an item as usual and press `>` to make it a sub-item of the one above it; `<` moves it back out a level, after the rest of what was under its parent. `a` adds below the selected item and everything under it, at its level, and `A` keeps adding there.

A parent shows how many of its sub-items are done, like `1/3`. `x` on it completes it and everything under it, and `x` again reopens them all. Completing sub-items one by one leaves the parent open; reopening one reopens everything it's under.

`za` (or `Shift+Enter`) folds an item's sub-items away, and again unfolds them; `zc` folds and `zo` unfolds. `Shift+Enter` needs a terminal that tells it apart from `Enter`: kitty, Ghostty, WezTerm, foot, Alacritty, recent iTerm2, and Windows Terminal do; elsewhere it's the same as `Enter`, and `za` always works. In tmux, turn on `extended-keys` and tell tmux your terminal has them, like `set -ag terminal-features "xterm-256color:extkeys"` for Alacritty. A folded item shows `▸`. On a sub-item with nothing under it, `za` and `zc` fold the item it's under. Searching finds sub-items too, and going to one unfolds what hides it.

Sub-items go where the item they're under goes: moving, pinning, carrying over, copying or deleting an item takes them along, and `J` / `K` move a sub-item among the others under the same item. You can give a sub-item a priority, notes, a deadline or a reminder; only the top-level item decides which days they show on, and the calendar lists top-level items only.

### Completed items

`x` moves an item under the Completed header, crossed out. The cursor stays where it was, so you can tick off several items in a row. `x` on a completed item moves it back to the bottom of the numbered list.

### Pinned items

Items stay on their day by default, so something like "Dentist at 3pm" doesn't follow you around. Pin an item with `m` if it should: when you start todoro, pinned items you didn't complete on earlier days move to the top of today's list, keeping their notes. They stay pinned, so they keep moving forward each day until you complete or unpin them.

Looking ahead with `l` shows pinned items on future days too, at the top of the list, as they'll be when that day comes. In general, an unfinished pinned item shows on its own day and every day after it, so if you move one to an earlier day with `H`, it still shows today. They're the same items, so completing, editing or unpinning one there changes it on the day it's on now.

If todoro is open at midnight, this happens then too, and the list moves on to the new day.

Completed items always stay on the day you completed them, so you can go back with `h` to see what you did.

If most of what you add should follow you until it's done, turn on **Pin new items** in the options (`Space` `o`). New items then start pinned, and `m` unpins the ones that belong to their day.

### Deadlines

`@` (or `:deadline`) gives the selected item a deadline. It opens the year calendar: move to the day (`h`/`j`/`k`/`l`, `H`/`L` for months, `t` for today) and press `Enter`. Then type a time, like `13:00`, `1300`, `1pm` or `11:30am`, and press `Enter`; or press `Enter` straight away for the whole day. `Esc` goes back to choosing the day, and `d` removes a deadline.

An item with a deadline shows `◷` and when it's due where the pin would be, like `◷ 19:25` (today), `◷ tomorrow` or `◷ Fri 9AM`. It's yellow on the day it's due, and red once it's passed.

Whenever something is overdue, the top-right corner of the list says how many, like `⚠ 2 overdue · go`, on whichever day you're looking at. `go` (or `:overdue`) lists them, most recently due first, with their notes beside them on a wide terminal; `Enter` goes to one.

A deadline works like a pin that stops: the item moves on to today and shows on the days ahead, up to its deadline day, and not after. If you don't finish it in time, it stays on its deadline day, in red, rather than following you. Setting a deadline unpins an item, since the deadline decides how far it moves on; to pin it again, remove the deadline first.

### Reminders

`r` (or `:remind`) sets a reminder on the selected item, usually one on a later day. Press `h`, `d` or `m` and type how many hours, days or months before you want reminding, then press `Enter`. Or press `c` to choose a day on the calendar, then type a time, or press `Enter` for the start of the day. `x` removes a reminder, and `Esc` backs out.

Hours count back from the item's deadline, or from the end of its day if it has no deadline or no time, so `2` hours before an item on Friday reminds you at 22:00 on Friday. Days and months count back whole days and start at the beginning of the day, so you have all of it: `2` days before Friday reminds you from the start of Wednesday.

Once a reminder is due, the item also shows at the top of today's list until its own day, with `⚑` and the day it's on, like `⚑ Fri`. Before then, the item shows `⚑` and how long before it you'll be reminded, like `⚑ 2d`. On other days, the top-right corner says how many reminders are due today, like `⚑ 2 today · t`. Completing an item stops its reminder.

### Insert mode

Adding or editing an item puts you in insert mode.

| Key | Action |
|---|---|
| `←` / `→` | Move the cursor |
| `Home` / `End` | Jump to the start / end of the line |
| `Backspace` / `Delete` | Delete before / under the cursor |
| `Enter` / `Esc` | Save |
| `Tab` | Complete the `#tag` being typed |
| `↑` / `↓` | Choose among the tags suggested (also `Ctrl+N` / `Ctrl+P`) |

Saving a new item with no text discards it. Clearing all the text from an existing item and saving asks whether to delete it.

With `A`, `Enter` adds the item and starts a new one below it, so you can type a whole list in one go. `Esc` saves what you've typed and stops; `Enter` on an empty line stops too.

### Notes

`Enter` on an item opens its notes: free text that isn't shown on the main list. The notes screen has its own small vim-style editor, with line numbers, and saves as you type.

To use your own editor instead, open the options (`Space` `o`) and set the notes editor to its command, like `nvim`, `hx` or `code --wait`. `Enter` then opens the item's Markdown file in it, and todoro picks up what you saved when it closes. Leave the command empty to go back to todoro's editor.

`v` on an item shows its notes formatted, for reading: headings, lists and task lists (`- [ ]` and `- [x]`), quotes, code, tables, links and **bold** or *italic* text.

| Key | Action |
|---|---|
| `j` / `k` | Scroll down / up |
| `Ctrl+D` / `Ctrl+U` | Half a page down / up |
| `Space` / `Ctrl+B` | A page down / up |
| `gg` / `G` | Top / bottom |
| `i` / `e` / `Enter` | Edit the notes, then come back to the formatted view |
| `Esc` / `q` / `v` | Back to the list |

In normal mode:

| Key | Action |
|---|---|
| `h` `j` `k` `l` | Move |
| `w` / `b` / `e` | Next word / previous word / end of word |
| `Ctrl+→` / `Ctrl+←` | End / start of a word, in normal mode and while typing |
| `0` / `$` | Start / end of the line |
| `Home` / `End` | Start / end of the line, in normal mode and while typing |
| `Ctrl+Home` / `Ctrl+End` | Start / end of the notes, in normal mode and while typing |
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
| `yy` | Copy the line (it flashes briefly to show it's copied) |
| `p` / `P` | Paste below / above (also into another item's notes) |
| `v` | Select text by moving the cursor; then `y` copies it, `d` cuts it, `J` / `K` move its lines down / up, and `Esc` cancels |
| `V` | Select whole lines; `j` / `k` extend the selection, `J` / `K` move the lines, `y` copies them, `d` deletes them, and `Esc` deselects |
| `J` / `K` | Move the line down / up |
| `u` / `Ctrl+R` | Undo / redo |
| `?` | Show all keybindings |
| `Esc` / `q` | Back to the list |

In insert mode, type as normal (`Enter` starts a new line) and press `Esc` to go back to normal mode.

You can paste text from elsewhere straight into notes, in either mode: it goes in at the cursor with its lines kept, and `u` undoes the whole paste. Pasting into a single-line box, like adding an item or searching, puts it on one line.

### Tags

Write `#words` in an item to tag it, like "Call #work about the #budget". Tags are coloured on the list and ignore case, so `#Work` and `#work` are the same tag. `Space` `s` `t` lists every tag with how many items have it; pick one with `j` / `k` and press `Enter` to see all its items across every day, where you can type to narrow them down and press `Enter` to go to one.

While you type a tag in an item, the tags you've already used that start with what you've typed show under it, most used first: type `#l` and `#later` appears. `Tab` fills in the highlighted one, and `↑` / `↓` choose another. `Enter` saves the item as you typed it, so `#l` stays `#l`. Typing just `#` lists every tag. It works when adding items in the calendar too.

### Search

There are two fuzzy searches, like finding files and searching inside them in vim:

- `Space` `Space` finds items by their text, on every day. Before you type, it lists every item, closest day first, so you can browse them.
- `Space` `s` `g` searches inside every item's notes, line by line, and shows each item with its best matching line under it. The items' own text isn't searched. `Enter` opens the notes to edit, with the cursor on that line. With your own editor set, it opens the file there, on that line too for editors that take `+N` (vi, vim, nvim, nano, emacs, micro and kak).

`Space` `s` `d` and `Space` `s` `r` list every open item with a deadline or a reminder, soonest first, with when it's due or when you'll be reminded; type to narrow them down. `Space` `s` `o` opens the options with their search ready to type in.

Type a few letters in order, like `dntst` for "Dentist at 3pm"; separate words match separately, capitals only match capitals, and plain letters match accented ones. The best matches come first, with the closest days first among equals, and the matched letters are highlighted.

`↑` / `↓` (or `Ctrl+N` / `Ctrl+P`) select a result, and `Enter` goes to that item on its day. `Esc` stops typing, as in vim, so you can move through the results with `j` / `k` (`gg` / `G` to the first or last); `i` types again, and `Esc` or `q` closes the search.

On a wide terminal, the results are on the left and the selected item's notes on the right, formatted as with `v`, so you can read them as you move. `Ctrl+D` / `Ctrl+U` scroll them.

### Timeline and planning

Any top-level item can have a start and end time on a day. Items without one are all day.

`Space` `t` shows the day as a timeline: the items with no time listed as all day at the top, then the hours, with each item that has a time as a block from its start to its end, side by side where they overlap, and a red line right across at the time now. Each block has a bar down its left, and blocks alternate between two shades like a checkerboard, so you can see where one ends and the next begins. A triaged item's block takes its priority's colour instead, red, yellow or green, in two shades the same way. `j` / `k` scroll, `h` / `l` go to the day before or after, `t` back to today, `p` plans the day, and `Esc` goes back to the list.

Both leave out completed items: they're about what's left to do. `Space` `p` plans the day on screen: the list, as on the main screen, with the timeline beside it on a wide enough terminal, and the plan's keys at the bottom.

| Key | Action |
|---|---|
| `j` / `k` | Choose an item |
| `L` / `H` | Move into the timeline, and back to the list. There `j` / `k` go down and up a column of blocks, and `h` / `l` to the column on the left or right, to the block that overlaps most; `H` goes back to the list. The item chosen is highlighted on both. Retiming a block there stays on it, and clearing one's time moves on to another block |
| `Enter` | Type its start time, like `9`, `9:30`, `13:00` or `1pm`, then `Enter`, or just `Enter` to start when the last item you planned ends (now, for the first); then its end and `Enter`, or just `Enter` for an hour. Then it moves on to the next item that doesn't have a time yet (below, then from the top), ready for its start, and stops once every item has one |
| `Tab` | While typing a time: leave this item as it is, for the next one |
| `a` / `A` | Add an item, or several, as on the list |
| `o` / `O` | Add a sub-item, or several, under the selected item |
| `e` / `E` | Edit the item, or edit it with its text cleared, as on the list |
| `x` / `d` | Delete the item, asking first as on the list |
| `u` / `Ctrl+R` | Undo / redo a change made in the plan, one at a time |
| `J` / `K`, `yy` / `p` / `P`, `>` / `<`, `za` / `zc` / `zo`, `m`, `@`, `r`, `V`, counts | As on the list (selecting can't move items to another day here) |
| `c` | Clear the item's time, making it all day |
| `!` | Triage the item, as on the list: its block takes the priority's colour |
| `w` | Save the plan |
| `R` | Reset: clear every time on the day, so it's all day again (still only kept if you save) |
| `Esc` / `q` | Leave. If you changed anything, a popup asks whether to save first: `w` saves, `q` leaves without saving (so `q` `q` throws the changes away), `n` keeps planning |

The list shows an item's time on the day it's planned for, like `09:00–09:30`. A time belongs to that day, so a pinned item carried on to the next day is all day there until you plan it again. Saving a plan is one change for `u` to undo, including any items added, edited or deleted in it; leaving without saving puts those back too. Sub-items go inside their parent's block.

### Calendar

`Space` `c` opens a calendar on the day you're viewing, for planning ahead without stepping through days one at a time. It has three views:

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

`Space` `o` opens the options. `j` / `k` select one, `Space` or `Enter` turns it on or off, and `Esc` closes them. They're saved and remembered next time.

`/` searches them: type a word or two, like `clock` or `delete notes`, and only the options that match show, by name, description or section. `↑` / `↓` move among them as you type; `Enter` keeps the matches, so `j` / `k` and `Space` work on just those, and `Esc` clears the search (and then closes the options).

**Accessibility**

- **Semantic priority icons:** show `∧` High, `–` Medium and `∨` Low beside triaged items, as well as the coloured number, for anyone who can't tell the colours apart.
- **No colours:** draw everything in your terminal's own colours. Highlights such as the selected item use reversed text instead. This starts on if you set the standard [`NO_COLOR`](https://no-color.org) environment variable, until you change it here.

**Items**

- **Pin new items:** new items, from the list or the calendar, start pinned, so they move on to today until they're done. Off by default. Pasted items keep their own pin.
- **12-hour clock:** show deadline times like `11PM` instead of `23:00`. Off by default. You can type times either way.

**Notes**

- **Notes editor:** a command to open notes with instead of todoro's own editor, like `nvim`. `Enter` types it; an empty one goes back to todoro's editor. The command runs with the notes file after it, through `sh` (or `cmd` on Windows), so arguments and `$EDITOR` work. Editors that open a window and return straight away need their wait flag, like `code --wait`.

**Delete**

The last rows delete things for good. Undo can't bring them back, so each one asks you to type a word to confirm: the workspace's name for the first two, `delete` for the next two, and `reset` for the last.

- **Delete all items and notes in** (the open workspace): every day's items and their notes files. The workspace stays, empty.
- **Delete all notes in** (the open workspace): every item's notes and the notes files. The items stay.
- **Delete all notes in every workspace.**
- **Delete all items and notes in every workspace:** every workspace is emptied, but they all stay.
- **Reset todoro:** deletes every workspace and your settings, and starts again as if todoro were newly installed: you choose a folder, then see the notes for the last few releases. Only todoro's own folders go (those with a `todos.json`, and empty ones, which show as workspaces too); anything else you keep in the todoro folder stays, and so does the folder if it isn't empty.

Deleting notes removes the `.md` files in each workspace's `notes` folder, conflict copies included, and nothing else there (like an `.obsidian` folder).

### Workspaces

Workspaces keep separate sets of todos and notes, as if you had several todoros: say, Personal and Work. The one you're in shows in the top-left corner of the list.

`Space` `w` lists them. `j` / `k` pick one and `Enter` opens it; `a` creates a new one and opens it; `d` deletes one, after you type its name to confirm, since it deletes its todos and notes for good. You can't delete the workspace you're in or your only one. Each workspace has its own undo history, and todoro opens the one you used last.

### What's new

The first time you start todoro after updating, it shows what's new in the versions since the one you last used. The very first time (or after a reset), it shows the last five releases, with a link to the rest on GitHub. `Space` `n` shows the notes for every release at any time; `j` / `k` scroll (`Space` a page, `g` / `G` to the top or bottom), the corner shows where you are (`Top`, a percentage, `Bot`, or `All` when it all fits), and `Esc` closes them. The notes are built into todoro, so they always match the version you have.

### Help

`?` (or `Space` `s` `k`) opens a popup listing every key, from the main list or the notes screen, with the keys you've set in `keys.toml`. Start typing to search: a single character such as `x` or `G` finds that key, and a word such as `undo` or `esc` finds keys by name, by what they do, or by their command name in `keys.toml`. `↑` / `↓` scroll and `Esc` closes it.

### Delete popup

Press `d` to delete the item, or `c` (or `Esc`) to cancel.

`Ctrl+C` quits from any mode.

## Changing keys

Every key on the list and in the leader menu can be changed in a file of your own, `keys.toml`. It lists only what you change; every other key keeps its default. Without the file, todoro uses its defaults.

The easiest way to start: open the options (`Space` `o`), select **Keybindings** under **Keys** and press `Enter`. The first time, todoro writes the file with every default binding in it, commented out, and opens it in your editor (the notes editor from the options, otherwise `$VISUAL` or `$EDITOR`, otherwise `vi`, or Notepad on Windows). Uncomment a line, change its key, save and close the editor, and the new keys work straight away. If a line can't be used, todoro says which and why, and uses the rest.

You can also write the file yourself, keep it with your dotfiles, and copy it between computers. todoro never changes it. It lives beside the settings:

| System | `keys.toml` |
|---|---|
| Linux | `~/.config/todoro/keys.toml` |
| macOS | `~/Library/Application Support/todoro/keys.toml`, or `~/.config/todoro/keys.toml` if you keep it there |
| Windows | `C:\Users\<you>\AppData\Roaming\todoro\keys.toml` |

`todoro --help` shows where it is on your computer, and the `TODORO_KEYS` variable points todoro at another file.

```toml
# The key that opens the leader menu (Space by default). Keep it above [list].
leader = ","

[list]
"c" = "calendar"         # c opens the calendar
"ctrl+d" = "delete"      # Ctrl+D deletes, as well as d
"x" = "none"             # x does nothing

[leader-menu]
"f" = "find-items"       # leader f finds items
"s n" = "search-notes"   # leader s n searches notes
"s" = "+search"          # names the group shown after leader s
```

- **Keys** are a character (`x`, `G`, `@`), a name (`space`, `enter`, `tab`, `backspace`, `delete`, `esc`, `up`, `down`, `left`, `right`, `home`, `end`, `pageup`, `pagedown`, `f1` to `f12`), `ctrl+` and one of those, like `ctrl+r`, or `shift+` and a name, like `shift+enter` (for a letter, write the capital). Separate a sequence with spaces: `"g g"`, `"s n"`.
- **A new key adds to the old ones**: with `"ctrl+d" = "delete"`, `d` still deletes too. To move a command, also set its old key to `"none"`.
- **A key you set replaces any default it clashes with.** Binding `"g"` frees `gg` and `go`, since `g` would always run first. Two of your own keys that clash are reported, and the second is skipped.
- **Every command works in both places**, so you can put `toggle-done` in the leader menu or `calendar` on a key of its own.
- **Digits** are for counts (`4j`), and **`Ctrl+C`** always quits, so neither can be bound.
- Keys on other screens (the calendar, search, notes and popups) can't be changed yet.

On macOS, `Cmd` keys never reach programs in a terminal, and `Option` keys depend on the terminal's settings. On Windows, the console keeps a few `Ctrl` keys for itself.

The help (`?`) shows each key with its command's name in grey. These are all the commands:

| Command | What it does |
|---|---|
| `prev-day` | Previous day |
| `next-day` | Next day |
| `today` | Go to today |
| `down` | Move down |
| `up` | Move up |
| `first` | First item |
| `last` | Last item, or with a number before it, that item |
| `move-down` | Move the item down the list |
| `move-up` | Move the item up the list |
| `move-to-prev-day` | Move the item to the previous day |
| `move-to-next-day` | Move the item to the next day |
| `add` | Add an item below the cursor |
| `add-several` | Add several items, one after another |
| `add-sub` | Add a sub-item under the selected item |
| `add-several-subs` | Add several sub-items under it, one after another |
| `indent` | Make the item a sub-item of the one above it |
| `outdent` | Move a sub-item out a level |
| `fold-toggle` | Fold or unfold the item's sub-items |
| `fold` | Fold the item's sub-items away |
| `unfold` | Unfold the item's sub-items |
| `edit` | Edit the selected item, cursor at the end |
| `edit-cleared` | Edit it with the text cleared |
| `toggle-done` | Mark the selected item done / not done |
| `toggle-pin` | Pin / unpin: move to today until done |
| `deadline` | Set a deadline: a day, then a time or none |
| `reminder` | Set a reminder, some time before or on a day |
| `overdue` | List the items past their deadline |
| `triage` | Triage: High, Medium, Low, then none |
| `delete` | Delete the selected item |
| `copy` | Copy the selected item |
| `paste-below` | Paste below (also after deleting) |
| `paste-above` | Paste above |
| `select` | Select several items |
| `view-notes` | View the selected item's notes, formatted |
| `open-notes` | Edit the selected item's notes (in your own editor, if set in the options) |
| `undo` | Undo a change to the list |
| `redo` | Redo a change to the list |
| `command-line` | Type a command, like :42 to go to item 42 |
| `help` | Show this help |
| `quit` | Quit |
| `leader` | Menu of the commands below, narrowing as you type |
| `find-items` | Find items by their text, on any day |
| `search-notes` | Search inside every item's notes |
| `deadlines` | List every deadline, soonest first |
| `reminders` | List every reminder, soonest first |
| `tags` | List tags, to show the items with one |
| `search-options` | Search the options |
| `calendar` | Open the calendar |
| `options` | Options, including accessibility |
| `workspaces` | Workspaces: switch, create or delete |
| `patch-notes` | Patch notes: what changed in every release |
| `timeline` | The day as a timeline, with the time now |
| `plan` | Plan the day: give items a start and end time |

## Your todos folder

The first time you start todoro, it asks where to keep your todos (suggesting `~/todoro`). Everything goes in that folder: each workspace is a folder inside it, holding its own `todos.json` with its items, and a `notes` folder with each item's notes as a Markdown file.

```
~/todoro/
└── Personal/
    ├── todos.json
    └── notes/
        └── write-the-quarterly-report.md
```

Notes files are named after their item when you first write notes, and are plain Markdown, so you can read and edit them in any editor or open the workspace folder in Obsidian. todoro reads a note again each time you open it. If a note changes somewhere else while you're editing it in todoro, todoro never overwrites it: it keeps that version and saves yours beside it as `<name> (conflict).md`, and tells you.

todoro never backs anything up for you. Because it's one plain folder, that's easy to do yourself: copy it, put it in a folder you back up or sync, or commit it to git (ignore `*.json.tmp`, a file that briefly appears while saving). To restore, put the folder back; if you choose a folder that already has workspaces in it, todoro uses them.

If you used todoro before it had this folder, your existing todos move into your first workspace when you set it up.

To keep everything somewhere else later, open the options (`Space` `o`), select **Todoro folder** under **Data** and press `Enter`, then type the new folder. todoro moves every workspace there. If the new folder already has a workspace with the same name, it moves nothing and tells you.

Todos are saved after every change, as JSON grouped by date. Items with only text are plain strings, and others are objects like `{ "text": ..., "notes_file": "....md", "done": true, "pinned": true, "priority": "high" }`. An item's sub-items are in its `"items"`, written the same way, and `"folded": true` keeps them folded. A planned time is `"time": "2026-10-11 09:00-10:30"`.

If a folder is synced between machines, don't have todoro open on two of them at once: the last one to save wins.

Settings (options, the folder and workspace you use) are kept separately, in `settings.json` in your config folder (`~/.config/todoro/` on Linux, `~/Library/Application Support/todoro/` on macOS, `%APPDATA%\todoro\` on Windows), or wherever `TODORO_SETTINGS` points, since they belong to each machine.

To use a single file instead of the folder and workspaces (for scripts or testing), set `TODORO_FILE`. Its notes go in a folder beside it named after it, like `work-todos-notes`:

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
