# Changelog

All notable changes to todoro. Each release's section is also shown on its GitHub release page.

## [0.7.0] - 2026-10-11

### Breaking

- Sub-items and planned times are saved in your todos file. An older version of todoro would drop them if it saved the file.
- `v` now selects several items, as `V` does (and as `v` does in the notes). Viewing an item's notes formatted has moved to `Space` `v`.

### Features

Sub-items:

- Any item can have sub-items, as deep as you like. `o` adds one under the selected item, and `O` adds several. `>` makes an item a sub-item of the one above it, and `<` moves it back out.
- A parent shows how many of its sub-items are done. `x` on it completes everything under it, and reopening a sub-item reopens what it's under.
- `za` folds an item's sub-items away and unfolds them, as do `zc` and `zo`, and `Shift+Enter` in terminals that tell it from `Enter`.
- Sub-items go where their item goes: moving, pinning, carrying over, copying and deleting an item take them along.

Planning your day:

- `Space` `t` shows the day as a timeline: each item with a time as a block from its start to its end, and a red line across at the time now. Items with no time are listed as all day.
- `Space` `p` plans the day: the list, with the timeline beside it. `Enter` types an item's start and then its end (just `Enter` for an hour), then goes on to the next item without a time, starting where the last one ended.
- In the plan, `L` moves into the timeline, where `j` and `k` go down a column of blocks and `h` and `l` across, and `H` back. `c` clears a time, `R` clears them all, `!` triages, `u` undoes, and the list's keys work too. `w` saves, and `q` asks whether to save first.
- A triaged item's block takes its priority's colour.

## [0.6.0] - 2026-10-11

### Breaking

- Less frequent commands have moved behind `Space`, which opens a menu of them that narrows down as you type, like which-key in Neovim: `Space` `s` `g` searches notes, `Space` `c` opens the calendar, `Space` `s` `t` lists tags (was `#`), `Space` `o` the options, `Space` `w` workspaces (was `W`) and `Space` `n` the patch notes (was `N`). `Space` `s` `k` searches the keybindings, as `?` does. The old keys do nothing now, to leave room for more.
- The searches have changed. `Space` `Space` now finds items by their text, and `Space` `s` `g` searches inside notes only. `S` is gone.
- Deadlines are saved in your todos file. An older version of todoro would drop them if it saved the file.

### Features

Deadlines:

- `@` (or `:deadline`) gives an item a deadline. Choose the day on the year calendar, press `Enter`, then type a time like `13:00`, `1300` or `1pm`, or press `Enter` again for the whole day. `d` removes a deadline, and `Esc` backs out without changing anything.
- An item with a deadline shows `◷` and when it's due in the pin's place, yellow on the day and red once it's passed. It moves on like a pinned item, up to its deadline day and not after.
- When something is overdue, the top-right corner of the list says so. `go` (or `:overdue`) lists what's overdue.
- A new 12-hour clock option shows times like `11PM` instead of `23:00`.

Reminders:

- `r` (or `:remind`) sets a reminder on an item: `h`, `d` or `m` and a number for hours, days or months before, or `c` to choose a day on the calendar. `x` removes it.
- Hours count back from the deadline, or from the end of the item's day. Days and months start at the beginning of the day, so you have all of it.
- Once a reminder is due, the item shows at the top of today's list with `⚑` until its own day. On other days, the top-right corner says how many are due.

Keys:

- Change any key on the list and in the leader menu in your own `keys.toml`. Open it from the options (`Space` `o`, under Keys): the first time, todoro writes it with every default listed, commented out, and when you close your editor the new keys work straight away. Keys you don't mention keep their defaults.
- `leader = ","` moves the leader menu off `Space`. `todoro --help` shows where `keys.toml` is.
- The help shows your keys, with each command's name to use in `keys.toml`.

Search:

- `Space` `s` `d` lists every deadline and `Space` `s` `r` every reminder, soonest first. Type to narrow them down.
- `Space` `s` `o` opens the options ready to search them.
- `Space` `Space` finds items by their text, on any day. With nothing typed, it lists every item, so you can browse them.
- `Space` `s` `g` searches inside every item's notes, and `Enter` opens the notes to edit on the line it found.
- `Esc` stops typing, so you can move through the results with `j` and `k`. `i` types again, and `Esc` or `q` closes.
- On a wide terminal, the selected item's notes show beside the results.

Notes:

- `v` shows an item's notes formatted: headings, lists and task lists, quotes, code, tables, links, bold and italic. `i` edits them and comes back.
- In the options, set your own editor for notes, like `nvim`. With one set, the notes search opens it on the line it found.

Items:

- Typing a `#tag` suggests the tags you've already used that start with it. `Tab` fills in the highlighted one, `↑` and `↓` choose another, and `Enter` keeps what you typed.
- `E` edits an item with its text cleared, to write it again. Leaving it empty keeps the item as it was.

Options:

- Pin new items: new items start pinned. Off by default.
- `/` searches the options.
- A Delete section removes every note, or every item and note, in this workspace or all of them. Each asks you to type a word to confirm.
- Reset todoro deletes every workspace and your settings, and starts again as if newly installed.
- A first start now shows the newest releases, with a link to the rest on GitHub.

### Fixes

- Lines in notes stay as you typed them when shown formatted, instead of joining into one paragraph.
- Resetting todoro also deletes empty workspace folders.
- On Windows, the release titles in the patch notes no longer carry a stray character from Windows line endings.

## [0.5.0] - 2026-10-06

### Breaking

- Notes are now plain Markdown files, one per item, in a `notes` folder inside your workspace, named after the item (like `buy-milk.md`). You can read, edit, sync or back them up with any tool, or open the folder in an app like Obsidian. Notes you already have move into files the first time you start this version, and older versions of todoro can't see them after that.

### Features

Notes:

- Use your own editor for notes. In the options (`o`), set the notes editor to a command like `nvim`, `hx` or `code --wait`, and `Enter` opens the item's notes file in it. todoro picks up what you saved when it closes, and `u` undoes the whole edit. Leave it empty to keep todoro's editor.
- If a notes file changes outside todoro while you have changes of your own, todoro never overwrites it. It keeps the other version and saves yours beside it as `<name> (conflict).md`, and tells you.
- `Ctrl+→` goes to the end of a word and `Ctrl+←` to its start, both in normal mode and while typing.
- `Home` and `End` go to the start and end of the line, and `Ctrl+Home` and `Ctrl+End` to the start and end of the notes, also while typing.

### Fixes

- Triaging the selected item shows its new colour straight away, instead of only once the cursor moves off it.

## [0.4.0] - 2026-10-06

### Breaking

- todoro now keeps everything in a folder you choose. The first time you start this version, it asks where (suggesting `~/todoro`) and what to call your first workspace, and moves your existing todos and notes into it. The old file in your system's data folder is no longer used.

### Features

Workspaces:

- Keep separate sets of todos and notes in workspaces, like Personal and Work. `W` lists them to switch, create or delete one; deleting asks you to type its name first. The workspace you're in shows in the corner of the list.
- Each workspace is a plain folder inside your todoro folder, so backing up is copying one folder. todoro leaves backups to you.
- Move the todoro folder from the options (`o`), under Data. Every workspace moves with it.
- `todoro --where` prints the folder, handy for backup scripts. `--version` and `--help` work too.

Notes:

- Paste text from elsewhere straight into notes, and it keeps its lines. Pasted text can no longer run as commands.
- `V` selects whole lines, `j` and `k` extend the selection, and `J` and `K` move the selected lines. `J` and `K` also move a `v` selection's lines.

Release notes:

- After an update, todoro shows what's new. `N` shows every release's notes at any time, with your place shown in the corner as you scroll.

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
