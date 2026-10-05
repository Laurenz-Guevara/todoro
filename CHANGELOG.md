# Changelog

All notable changes to todoro. Each release's section is also shown on its GitHub release page.

## [0.3.0] - 2026-10-06

### Features

On the list:

- A count before `j` or `k` moves that many items, like `4j`, and `42G` or `:42` goes to item 42. `:q` quits.
- The selected item's number is highlighted in bold yellow.

In notes:

- Line numbers, with the current line's number highlighted.
- Counts work with movement and editing, like `4j`, `3x` or `2dd`, and `42G` or `:42` goes to line 42. `:q` goes back to the list.
- `_` and `^` go to the first non-blank character of the line.
- `J` and `K` move the current line down or up.
- `dd` keeps deleted lines to paste, `yy` copies lines, and `p` or `P` pastes them below or above, even into another item's notes.
- `v` selects text to copy with `y` or cut with `d`, to paste inside a line.
- Copied lines flash briefly to show what was copied.
- Everything typed in one go in insert mode undoes in a single step, as in vim.

### Bug fixes

- Text selected in notes now has a dark background instead of a bright blue that made it hard to read.

## [0.2.0] - 2026-10-05

### Breaking

- Pinning items moves from `p` to `m`, because `p` now pastes.

### Features

- Add several items in a row with `A`: `Enter` adds each one and starts the next, and `Esc` stops.
- Copy an item with `yy` and paste it below or above the cursor with `p` or `P`, on any day. Deleting also keeps the item to paste, so `dd` then `p` moves it.
- Select several items with `V`, then complete, pin, triage, delete, copy or move them to another day all at once.
- Tag items by writing `#words` in them. Tags are coloured on the list, and `#` lists every tag so you can see all of a tag's items across every day.
- Jump to the first or last item with `gg` and `G`.
- If todoro is open at midnight, it moves on to the new day and carries pinned items over.

## [0.1.0] - 2026-10-05

The first release.

### Features

- A day-by-day todo list in the terminal with vim-style keys, opening on today: `h`/`l` change day, `t` returns to today, `j`/`k` move, and `J`/`K` reorder items.
- Add, edit and delete items (`a`, `e`, `d`), with a confirmation before deleting.
- Mark items done with `x`; completed items move under a Completed header.
- Pin items with `p` to carry them forward to today until they're done. Pinned items also show on future days.
- Move an item to the previous or next day with `H`/`L`.
- Triage items with `!`: High, Medium and Low colour the item's number red, yellow and green.
- Notes on any item (`Enter`), written in a small vim-style editor.
- A calendar (`c`) with week, month and year views, for adding items to future days without stepping through them.
- Fuzzy search across every day's items with `s`, and their notes too with `S`.
- Undo and redo for every change to the list (`u`, `Ctrl+R`).
- A searchable list of every key (`?`).
- Options (`o`), including two accessibility settings: semantic priority icons, and drawing without colour (also turned on by `NO_COLOR`).
- Long items wrap, and every screen fits narrow terminals down to about 30 columns.
- Todos are saved as JSON after every change; settings are saved separately.
- Installers and downloads for Linux, macOS and Windows.
