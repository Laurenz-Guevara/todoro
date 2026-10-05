# Changelog

All notable changes to todoro. Each release's section is also shown on its GitHub release page.

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
