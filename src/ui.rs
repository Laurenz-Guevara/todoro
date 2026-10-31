use std::ops::Range;

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Flex, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, List, ListItem, ListState, Padding, Paragraph, Wrap};
use ratatui::Frame;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use chrono::{Datelike, Days, NaiveDate, Timelike};

use crate::app::{App, Mode};
use crate::calendar::{self, Calendar, Zoom};
use crate::changelog::{self as changes, ChangelogView};
use crate::store::{Item, Priority};
use crate::help::Help;
use crate::keymap::{self, Command};
use crate::leader::Leader;
use crate::outline;
use crate::timeline::{self, TimeSlot, Typing};
use crate::notes::NotesEditor;
use crate::options::{EDITOR_ABOUT, EDITOR_ROW, FOLDER_ABOUT, FOLDER_ROW, KEYS_ABOUT, KEYS_ROW, Options, TOGGLES};
use crate::search::{self, Hit, Search};
use crate::setup::Setup;
use crate::snake::{self, Snake};
use crate::workspaces::Picker;
use crate::tags::{self, TagPicker};
use crate::deadline::{self, Deadline};
use crate::reminder::{self, Reminder};
use crate::viewer::{Scope, Viewer};
use crate::markdown;

pub fn draw(frame: &mut Frame, app: &App) {
    let [main, status] = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(frame.area());

    // The help popup is drawn over whichever screen it was opened from.
    let screen = match &app.mode {
        Mode::Help { back, .. } => back.as_ref(),
        mode => mode,
    };
    match screen {
        Mode::Notes(editor) => draw_notes(frame, app, editor, main),
        Mode::View(viewer) => draw_view(frame, app, viewer, main),
        Mode::Calendar(calendar) => draw_calendar(frame, app, calendar, main),
        Mode::Timeline { scroll } => {
            let title = format!(" {} ", app.day.format("%A, %B %-d %Y"));
            draw_timeline(frame, app, main, &title, None, *scroll);
        }
        Mode::Plan { .. } => draw_plan(frame, app, main),
        // Adding, editing or deleting from the plan view keeps its layout.
        Mode::Insert { .. } | Mode::ConfirmDelete { .. } | Mode::Visual { .. } | Mode::Remind(_) if app.plan_stash.is_some() => {
            draw_plan(frame, app, main)
        }
        _ => draw_list(frame, app, main),
    }
    draw_status(frame, app, status);

    match &app.mode {
        Mode::ConfirmDelete { rows } => draw_confirm(frame, app, rows),
        Mode::Help { help, .. } => draw_help(frame, help),
        Mode::Calendar(calendar) if calendar.adding.is_some() => draw_adding(frame, app, calendar),
        Mode::Calendar(calendar) if calendar.picking.as_ref().is_some_and(|picking| picking.time.is_some()) => {
            draw_deadline_time(frame, app, calendar)
        }
        Mode::Search(search) => draw_search(frame, app, search),
        Mode::Options(options) => draw_options(frame, app, options),
        Mode::Changelog(view) => draw_changelog(frame, view),
        Mode::Workspaces(picker) => draw_workspaces(frame, app, picker),
        Mode::Tags(picker) => draw_tags(frame, app, picker),
        Mode::Remind(picker) => draw_remind(frame, app, picker),
        Mode::Leader(leader) => draw_leader(frame, app, leader, main),
        Mode::Plan { plan, .. } if plan.confirm_cancel => draw_discard_plan(frame),
        _ => {}
    }

    if app.settings.no_colour {
        strip_colour(frame.buffer_mut());
    }
}

/// Removes every colour from the finished screen. Anything that relied on a
/// background colour (the selected row, the mode label) is reversed instead,
/// and grey text is dimmed, so nothing that was highlighted is lost.
fn strip_colour(buffer: &mut Buffer) {
    for cell in buffer.content.iter_mut() {
        if cell.bg != Color::Reset {
            cell.modifier.insert(Modifier::REVERSED);
            cell.bg = Color::Reset;
        }
        if cell.fg == Color::DarkGray {
            cell.modifier.insert(Modifier::DIM);
        }
        cell.fg = Color::Reset;
        cell.underline_color = Color::Reset;
    }
}

fn draw_list(frame: &mut Frame, app: &App, area: Rect) {
    let title = day_title(app, (area.width as usize).saturating_sub(4));

    // The keys for moving around, as keys.toml has them.
    let key = |command| list_key(app, command);
    let (prev, up, down, next) = (key(Command::PrevDay), key(Command::Up), key(Command::Down), key(Command::NextDay));
    let hints = [
        format!(" {prev} ← prev day · {up} ↑ up · {down} ↓ down · next day → {next} "),
        format!(" {prev} ← day · {up} ↑ · {down} ↓ · day → {next} "),
        format!(" {prev}/{next} day · {down}/{up} move "),
    ];
    let hint = fit_first(
        &hints.iter().map(String::as_str).collect::<Vec<_>>(),
        (area.width as usize).saturating_sub(2),
    );
    let mut block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(title.clone().centered())
        .title_bottom(Line::from(hint).centered().dim());
    // How many items are overdue in the top-right corner, wherever you are:
    // they're on today's list, but this shows on any day.
    let overdue = app.store.overdue_count(app.today, app.now);
    if overdue > 0 {
        let beside = (area.width as usize).saturating_sub(title.width()) / 2;
        let go = app.keymap.key_for(Command::Overdue).map_or(String::new(), |key| format!(" · {key}"));
        let long = format!(" ⚠ {overdue} overdue{go} ");
        let short = format!(" ⚠ {overdue} ");
        let shown = [long, short].into_iter().find(|text| text.width() + 2 <= beside);
        if let Some(text) = shown {
            block = block.title(Line::from(text.red().bold()).right_aligned());
        }
    }
    // Reminders due today, in the same corner, from any other day. On
    // today, they're at the top of the list.
    let reminded = if app.day == app.today { 0 } else { app.store.reminded_on(app.today, app.today, app.now).len() };
    if reminded > 0 {
        let beside = (area.width as usize).saturating_sub(title.width()) / 2;
        let overdue_width = if overdue > 0 { format!(" ⚠ {overdue} ").width() } else { 0 };
        let today = app.keymap.key_for(Command::Today).map_or(String::new(), |key| format!(" · {key}"));
        let long = format!(" {REMINDER_MARKER} {reminded} today{today} ");
        let short = format!(" {REMINDER_MARKER} {reminded} ");
        let shown = [long, short].into_iter().find(|text| text.width() + overdue_width + 2 <= beside);
        if let Some(text) = shown {
            block = block.title(Line::from(text.yellow().bold()).right_aligned());
        }
    }
    // The workspace's name in the top-left corner, if it fits beside the date.
    if let Some(name) = &app.workspace {
        let name = format!(" {name} ");
        let beside = (area.width as usize).saturating_sub(title.width()) / 2;
        if name.width() + 2 <= beside {
            block = block.title(Line::from(name.cyan().bold()).left_aligned());
        }
    }

    // The rows to show, with the item being typed in place of (or inserted
    // among) the saved ones, so the numbering below it is already right.
    let now = app.today.and_time(app.now);
    let mut rows: Vec<Row> = app
        .slots()
        .into_iter()
        .map(|slot| {
            let items = app.store.items(slot.day);
            let item = &items[slot.index];
            let progress = outline::progress(items, slot.index);
            Row {
                text: &item.text,
                time: item.time.filter(|time| time.date == app.day),
                depth: item.depth,
                progress: (progress.1 > 0).then_some(progress),
                folded: item.folded,
                priority: item.priority,
                pinned: item.pinned,
                deadline: item.deadline,
                reminder: item.reminder.map(|reminder| RowReminder {
                    reminder,
                    due: item.reminder_due_at(slot.day).is_some_and(|at| at <= now),
                    from: (slot.day > app.day).then_some(slot.day),
                }),
                has_notes: !item.notes.is_empty(),
                done: item.done,
                typing: false,
            }
        })
        .collect();
    let mut selected = (!rows.is_empty()).then_some(app.selected);
    if let Mode::Insert { index, input, editing, place, .. } = &app.mode {
        let text = &input.text;
        if *editing {
            rows[*index] = Row { text, typing: true, ..rows[*index] };
        } else {
            let row = Row {
                text,
                time: None,
                depth: place.map_or(0, |place| place.depth),
                progress: None,
                folded: false,
                priority: None,
                pinned: false,
                deadline: None,
                reminder: None,
                has_notes: false,
                done: false,
                typing: true,
            };
            rows.insert(*index, row);
        }
        selected = Some(*index);
    }

    // Open top-level items are numbered, their sub-items indented under
    // them. Completed ones follow under a header row, so their list row is
    // one more than their index.
    let open = rows.iter().position(|row| row.depth == 0 && row.done).unwrap_or(rows.len());
    let list_row = |index: usize| if index < open { index } else { index + 1 };
    let inner = block.inner(area);
    let numbered = rows[..open].iter().filter(|row| row.depth == 0).count();
    let completed = rows[open..].iter().filter(|row| row.depth == 0).count();

    // Pad numbers so text lines up once there are 10 or more open items.
    let width = numbered.max(1).to_string().len();
    // With semantic icons on, triaged items get an icon after the number, and
    // the rest leave a gap so all the text lines up.
    let icons = app.settings.semantic_icons && rows.iter().any(|row| row.priority.is_some());
    let prefix_width = width + 2 + if icons { 2 } else { 0 };
    let mut items: Vec<ListItem> = Vec::with_capacity(rows.len() + 1);
    // How many screen lines each list row takes, to place the typing cursor.
    let mut heights: Vec<usize> = Vec::with_capacity(rows.len() + 1);
    // Where the typing cursor goes within its row: (line, column).
    let mut typing_cursor = (0, 0);
    let mut typing_lead = prefix_width;
    let mut number = 0;
    for (i, row) in rows.iter().enumerate() {
        if i == open {
            let title = format!("── Completed ({completed}) ");
            let fill = (inner.width as usize).saturating_sub(title.chars().count());
            items.push(ListItem::new(format!("{title}{}", "─".repeat(fill)).dark_gray()));
            heights.push(1);
        }
        // A triaged item's number takes its priority's colour.
        // A triaged item's number takes its priority's colour, selected or
        // not, so triaging shows the colour straight away; the row's highlight
        // shows which is selected.
        // A sub-item has a bullet in place of a number, two columns further
        // in for each level, so its text lines up under its parent's.
        let crossed = Style::new().fg(Color::DarkGray).add_modifier(Modifier::CROSSED_OUT);
        let number_style = row.priority.map_or(Style::new().fg(Color::DarkGray), priority_style);
        let lead_width = prefix_width + 2 * row.depth;
        let (prefix, prefix_style, text_style) = match (row.depth, row.done) {
            (0, true) => (format!("{:>width$}  ", "✓"), Style::new().fg(Color::DarkGray), crossed),
            (0, false) => {
                number += 1;
                (format!("{number:>width$}. "), number_style, Style::new())
            }
            (depth, done) => {
                let bullet = if done { SUB_DONE } else { SUB_OPEN };
                let style = if done { Style::new().fg(Color::DarkGray) } else { number_style };
                (format!("{}{bullet} ", " ".repeat(prefix_width + 2 * (depth - 1))), style, if done { crossed } else { Style::new() })
            }
        };
        // When it's planned for on this day.
        let time = row.time.map(|time| {
            let style = if row.done { Style::new().fg(Color::DarkGray) } else { Style::new().fg(Color::Blue) };
            Span::styled(format!(" {}", time.describe(app.settings.twelve_hour)), style)
        });
        // How many of its sub-items are done, and a ▸ when they're folded away.
        let progress = row.progress.map(|(done, total)| {
            let fold = if row.folded { format!(" {FOLDED_MARKER}") } else { String::new() };
            Span::styled(format!("{fold} {done}/{total}"), Style::new().fg(Color::DarkGray))
        });

        // A deadline takes the pin's place, with when it's due.
        let deadline = row.deadline.map(|deadline| {
            let text = format!("{DEADLINE_MARKER}{}", deadline.describe(app.today, app.settings.twelve_hour));
            let style = if row.done {
                Style::new().fg(Color::DarkGray)
            } else if deadline.is_overdue(app.today, app.now) {
                Style::new().fg(Color::Red).add_modifier(Modifier::BOLD)
            } else if deadline.date == app.today {
                Style::new().fg(Color::Yellow)
            } else {
                Style::new().fg(Color::Cyan)
            };
            Span::styled(text, style)
        });
        let pinned = row.pinned && deadline.is_none();
        // A reminder: how long before while it's to come, then bold once
        // it's due, with the day the item is on if it's from a later one.
        let reminder = row.reminder.map(|shown| {
            let when = match (shown.due, shown.from) {
                (false, _) => shown.reminder.describe(app.today, app.settings.twelve_hour),
                (true, Some(day)) if row.deadline.is_none() => {
                    Deadline { date: day, time: None }.describe(app.today, app.settings.twelve_hour)
                }
                (true, _) => String::new(),
            };
            let text = if when.is_empty() { format!(" {REMINDER_MARKER}") } else { format!(" {REMINDER_MARKER} {when}") };
            let style = if row.done {
                Style::new().fg(Color::DarkGray)
            } else if shown.due {
                Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(Color::Cyan)
            };
            Span::styled(text, style)
        });

        // Wrap long text under itself, leaving room for the markers at the end.
        let markers = [(pinned, PINNED_MARKER.width()), (row.has_notes, NOTES_MARKER.width())];
        let markers_width: usize = markers.iter().filter(|(shown, _)| *shown).map(|(_, width)| width).sum::<usize>()
            + deadline.as_ref().map_or(0, Span::width)
            + reminder.as_ref().map_or(0, Span::width)
            + progress.as_ref().map_or(0, Span::width)
            + time.as_ref().map_or(0, Span::width);
        let text_width = (inner.width as usize).saturating_sub(lead_width + markers_width).max(1);
        let ranges = wrap_ranges(row.text, text_width);
        if let (true, Mode::Insert { input, .. }) = (row.typing, &app.mode) {
            typing_cursor = cursor_position(row.text, &ranges, input.cursor, text_width);
            typing_lead = lead_width;
        }

        let mut lines: Vec<Line> = ranges
            .iter()
            .enumerate()
            .map(|(n, range)| {
                let mut spans = if n == 0 {
                    vec![Span::styled(prefix.clone(), prefix_style)]
                } else {
                    vec![Span::raw(" ".repeat(lead_width))]
                };
                if n == 0 && icons && row.depth == 0 {
                    spans.push(match row.priority {
                        Some(priority) if row.done => Span::styled(format!("{} ", priority_icon(priority)), Style::new().fg(Color::DarkGray)),
                        Some(priority) => Span::styled(format!("{} ", priority_icon(priority)), priority_style(priority)),
                        None => Span::raw("  "),
                    });
                }
                let line = range.start..range.start + row.text[range.clone()].trim_end().len();
                if row.done {
                    spans.push(Span::styled(row.text[line].to_string(), text_style));
                } else {
                    spans.extend(tagged(row.text, line, text_style));
                }
                Line::from(spans)
            })
            .collect();
        let last = lines.last_mut().expect("wrap_ranges returns at least one line");
        if let Some(progress) = progress {
            last.push_span(progress);
        }
        if let Some(time) = time {
            last.push_span(time);
        }
        if pinned {
            last.push_span(PINNED_MARKER.fg(Color::Cyan));
        }
        if let Some(deadline) = deadline {
            last.push_span(deadline);
        }
        if let Some(reminder) = reminder {
            last.push_span(reminder);
        }
        if row.has_notes {
            last.push_span(NOTES_MARKER.dim());
        }
        let selecting = match app.mode {
            Mode::Visual { anchor } => app.visual_rows(anchor).contains(&i),
            _ => false,
        };
        let style = if row.typing {
            Style::new().fg(Color::Yellow)
        } else if selecting {
            Style::new().bg(VISUAL_BG)
        } else {
            Style::new()
        };
        heights.push(lines.len());
        items.push(ListItem::new(lines).style(style));
    }
    let mut state = ListState::default().with_offset(app.list_offset.get()).with_selected(selected.map(list_row));

    if items.is_empty() {
        let text = match app.keymap.key_for(Command::Add) {
            Some(key) => format!("Nothing to do. Press {key} to add an item."),
            None => "Nothing to do.".into(),
        };
        let empty = Paragraph::new(text).dim().centered().wrap(Wrap { trim: true });
        frame.render_widget(block, area);
        frame.render_widget(empty, inner);
        return;
    }

    let list = List::new(items)
        .block(block)
        .highlight_style(if app.planning_view() {
            // Planning, the same clear highlight as the selected block beside it.
            Style::new().bg(BLOCK_SELECTED_BG).fg(Color::White).add_modifier(Modifier::BOLD)
        } else {
            Style::new().bg(Color::Rgb(50, 50, 60)).add_modifier(Modifier::BOLD)
        });
    frame.render_stateful_widget(list, area, &mut state);
    app.list_offset.set(state.offset());

    if let Mode::Insert { index, .. } = &app.mode {
        let (line, col) = typing_cursor;
        let row: usize = heights[state.offset()..list_row(*index)].iter().sum::<usize>() + line;
        let cursor = Position::new(inner.x + (typing_lead + col) as u16, inner.y + row as u16);
        frame.set_cursor_position(cursor);
        draw_tag_suggestions(frame, app, cursor);
    }
}

/// How many tag suggestions show at once.
const TAG_SUGGESTIONS: usize = 5;

/// The tags that could complete the `#tag` being typed, in a box under the
/// cursor (or above it, without room below), the selected one highlighted.
fn draw_tag_suggestions(frame: &mut Frame, app: &App, cursor: Position) {
    let suggestions = app.tag_suggestions();
    if suggestions.is_empty() {
        return;
    }
    let screen = frame.area();
    let selected = app.tag_choice.min(suggestions.len() - 1);
    // Scroll so the selected one is always shown.
    let first = (selected + 1).saturating_sub(TAG_SUGGESTIONS);
    let shown = &suggestions[first..suggestions.len().min(first + TAG_SUGGESTIONS)];
    let count_width = shown.iter().map(|(_, count)| count.to_string().len()).max().unwrap_or(1);
    let room = (screen.width as usize).saturating_sub(4 + 2 + count_width).max(2);
    let name_width = shown.iter().map(|(name, _)| name.width() + 1).max().unwrap_or(1).min(room);
    let lines: Vec<Line> = shown
        .iter()
        .enumerate()
        .map(|(i, (name, count))| {
            let name = truncate(&format!("#{name}"), name_width);
            let line = Line::from(vec![
                format!(" {name:<name_width$}").into(),
                format!("  {count:>count_width$} ").dim(),
            ]);
            if first + i == selected { line.bg(SELECTED_BG).bold() } else { line }
        })
        .collect();
    // Borders, a space each side, and two between the name and the count.
    let width = (name_width + count_width + 6) as u16;
    let height = lines.len() as u16 + 2;
    // Under the cursor, lined up with the #, or above it without room.
    let typed = app.typing_tag_width() as u16;
    // The # is just before what was typed; the border and a space come before it.
    let x = cursor.x.saturating_sub(typed + 3).min(screen.width.saturating_sub(width));
    let below = cursor.y + 1;
    let y = if below + height <= screen.height.saturating_sub(1) { below } else { cursor.y.saturating_sub(height) };
    let area = Rect::new(x, y, width.min(screen.width), height.min(screen.height));
    let block = Block::bordered().border_type(BorderType::Rounded).border_style(Style::new().fg(Color::Magenta));
    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

/// One item row on the main list.
struct Row<'a> {
    text: &'a str,
    priority: Option<Priority>,
    pinned: bool,
    deadline: Option<Deadline>,
    reminder: Option<RowReminder>,
    /// When it's planned for on the day on screen.
    time: Option<TimeSlot>,
    /// How deep a sub-item it is, 0 for a top-level item.
    depth: usize,
    /// How many of its direct sub-items are done, of how many, if it has any.
    progress: Option<(usize, usize)>,
    folded: bool,
    has_notes: bool,
    done: bool,
    /// The item being added or edited.
    typing: bool,
}

/// A row's reminder, as shown.
#[derive(Clone, Copy)]
struct RowReminder {
    reminder: Reminder,
    /// Whether it's due by now.
    due: bool,
    /// The later day the item is stored on, if it shows here for its reminder.
    from: Option<NaiveDate>,
}

/// The icon for a priority, shown when semantic icons are on.
fn priority_icon(priority: Priority) -> &'static str {
    match priority {
        Priority::High => "∧",
        Priority::Medium => "–",
        Priority::Low => "∨",
    }
}

/// The colour of a triaged item's number.
fn priority_style(priority: Priority) -> Style {
    Style::new().bold().fg(match priority {
        Priority::High => Color::Red,
        Priority::Medium => Color::Yellow,
        Priority::Low => Color::Green,
    })
}

/// The bullet of an open sub-item, and of a completed one.
const SUB_OPEN: &str = "↳";
const SUB_DONE: &str = "✓";

/// Shown before the progress of an item whose sub-items are folded away.
const FOLDED_MARKER: &str = "▸";

/// Shown after an item on the main list when it has notes.
const NOTES_MARKER: &str = " ≡";

/// Shown after a pinned item, which moves forward to today until completed.
const PINNED_MARKER: &str = " ⚲";

/// Shown after an item with a reminder, and beside how many are due today.
const REMINDER_MARKER: &str = "⚑";

/// Shown in the pin's place on an item with a deadline, before when it's due.
const DEADLINE_MARKER: &str = " ◷ ";

fn draw_notes(frame: &mut Frame, app: &App, editor: &NotesEditor, area: Rect) {
    let room = (area.width as usize).saturating_sub(4);
    let title = format!("{}{}", row_label(app, app.selected), app.items()[app.selected].text);
    let title = format!(" {} ", truncate(&title, room.saturating_sub(2)));
    let hint = fit_first(&[" esc/q back to list ", " esc back "], area.width.saturating_sub(2) as usize);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Line::from(title.bold()).centered())
        .title_bottom(Line::from(hint).centered().dim())
        .padding(Padding::horizontal(1));
    frame.render_widget(&block, area);
    let inner = block.inner(area);
    frame.render_widget(&editor.textarea, inner);
    highlight_cursor_line_number(frame.buffer_mut(), editor, inner);
    if let Some(lines) = editor.selected_lines() {
        paint_lines(frame.buffer_mut(), editor, inner, &lines, Style::new().bg(VISUAL_BG));
    }
    if let Some((lines, _)) = &editor.flash {
        paint_lines(frame.buffer_mut(), editor, inner, lines, Style::new().bg(SELECTED_BG).add_modifier(Modifier::BOLD));
    }
}

/// The day on screen as the list's title: the longest date that fits in
/// `room`, shortened before dropping "(today)".
fn day_title(app: &App, room: usize) -> Line<'static> {
    let today = if app.day == app.today { " (today)" } else { "" };
    let dates = ["%A, %B %-d %Y", "%a, %b %-d %Y", "%a %-d %b"].map(|format| app.day.format(format).to_string());
    let (date, today) = [today, ""]
        .iter()
        .flat_map(|today| dates.iter().map(move |date| (date.clone(), *today)))
        .find(|(date, today)| date.width() + today.width() <= room)
        .unwrap_or_else(|| (truncate(&app.day.format("%-d/%-m").to_string(), room), ""));
    Line::from(vec![" ".into(), date.bold(), today.fg(Color::Green), " ".into()])
}

/// The selected item's notes as formatted Markdown, scrolled as far as the
/// viewer says, with where you are in the corner as in vim.
fn draw_view(frame: &mut Frame, app: &App, viewer: &Viewer, area: Rect) {
    let room = (area.width as usize).saturating_sub(4);
    let title = match viewer.scope {
        Scope::Day => day_title(app, room),
        Scope::Notes | Scope::Item => {
            let title = format!("{}{}", row_label(app, app.selected), app.items()[app.selected].text);
            Line::from(format!(" {} ", truncate(&title, room.saturating_sub(2))).bold())
        }
    };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(title.centered())
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    let width = inner.width as usize;

    let lines = match viewer.scope {
        Scope::Day => day_lines(app, width),
        Scope::Item => {
            let slot = app.slots()[app.selected];
            whole_lines(app.store.items(slot.day), slot.index, "", width)
        }
        Scope::Notes => {
            let notes = &app.items()[app.selected].notes;
            if notes.trim().is_empty() {
                vec![Line::from("No notes yet. Press i to write some.").dark_gray()]
            } else {
                markdown::render(notes, width)
            }
        }
    };
    let height = inner.height as usize;
    viewer.height.set(height);
    viewer.total.set(lines.len());
    let max = lines.len().saturating_sub(height);
    let scroll = viewer.scroll.min(max);

    let position = format!(" {} ", scroll_position(scroll, max));
    let hint_room = (area.width as usize).saturating_sub(2 + 2 * (position.width() + 1));
    let hints: &[&str] = if viewer.read_only() {
        &[" j/k scroll · esc back ", " esc back "]
    } else {
        &[" j/k scroll · i edit · esc back ", " i edit · esc back ", " esc back "]
    };
    let hint = fit_first(hints, hint_room);
    let block = block
        .title_bottom(Line::from(hint).centered().dim())
        .title_bottom(Line::from(position).right_aligned().dim());
    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(lines).scroll((scroll as u16, 0)), inner);
}

/// Every item on the list, for `Space e`: each top-level item's block as
/// `Space V` shows it, numbered as on the list, with a rule between them.
/// Folded sub-items show too, as this is for reading everything.
fn day_lines(app: &App, width: usize) -> Vec<Line<'static>> {
    let items = app.items();
    let mut lines: Vec<Line<'static>> = Vec::new();
    for (row, slot) in app.slots().into_iter().enumerate() {
        if items[row].depth > 0 {
            continue;
        }
        if !lines.is_empty() {
            lines.push(Line::default());
            lines.push(Line::from("─".repeat(width)).dark_gray());
            lines.push(Line::default());
        }
        lines.extend(whole_lines(app.store.items(slot.day), slot.index, &row_label(app, row), width));
    }
    if lines.is_empty() {
        lines.push(Line::from("Nothing on this day.").dark_gray());
    }
    lines
}

/// The item at `root` and its whole block, for `Space V`: its title (after
/// `label`) and notes, then each sub-item's title, indented by depth under a bullet as on the list,
/// and its notes, with a blank line between each of them.
fn whole_lines(items: &[Item], root: usize, label: &str, width: usize) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let gap = |lines: &mut Vec<Line<'static>>| {
        if !lines.is_empty() {
            lines.push(Line::default());
        }
    };
    let depth = items[root].depth;
    for item in &items[root..outline::block_end(items, root)] {
        // Two columns a level, but always leaving most of the width for text.
        let indent = (2 * item.depth.saturating_sub(depth + 1)).min(width / 3);
        let pad = " ".repeat(indent);
        if item.depth == depth {
            for part in wrap(&format!("{label}{}", item.text), width.max(1), usize::MAX) {
                lines.push(Line::from(if item.done { part.dark_gray().bold() } else { part.bold() }));
            }
        } else {
            gap(&mut lines);
            let bullet = if item.done { SUB_DONE } else { SUB_OPEN };
            let room = width.saturating_sub(indent + bullet.width() + 1).max(1);
            for (n, part) in wrap(&item.text, room, usize::MAX).into_iter().enumerate() {
                let lead = if n == 0 { format!("{pad}{bullet} ") } else { " ".repeat(indent + bullet.width() + 1) };
                let text = if item.done { part.dark_gray().bold() } else { part.bold() };
                lines.push(Line::from(vec![lead.into(), text]));
            }
        }
        if !item.notes.trim().is_empty() {
            gap(&mut lines);
            // Notes sit under their title's text, past the bullet.
            let notes_indent = if item.depth > depth { indent + 2 } else { 0 };
            for mut line in markdown::render(&item.notes, width.saturating_sub(notes_indent).max(1)) {
                line.spans.insert(0, " ".repeat(notes_indent).into());
                lines.push(line);
            }
        }
    }
    lines
}

/// Paints whole lines of the notes in `style`: lines selected with `V`, or
/// just-copied lines flashing. The text area can't style lines like this, so
/// their screen rows are found from the gutter: a numbered row starts a line,
/// and any non-blank rows after it are that line wrapping.
fn paint_lines(
    buffer: &mut Buffer,
    editor: &NotesEditor,
    area: Rect,
    lines: &std::ops::RangeInclusive<usize>,
    style: Style,
) {
    let gutter = editor.textarea.lines().len().to_string().len() as u16 + 2;
    let text = (area.left() + gutter).min(area.right())..area.right();
    let mut line = None;
    for y in area.top()..area.bottom() {
        let number: String = (area.left()..text.start).map(|x| buffer[(x, y)].symbol().to_string()).collect();
        match number.trim().parse::<usize>() {
            Ok(n) => line = Some(n - 1),
            Err(_) if text.clone().all(|x| buffer[(x, y)].symbol() == " ") => line = None,
            Err(_) => {}
        }
        if line.is_some_and(|line| lines.contains(&line)) {
            for x in text.clone() {
                buffer[(x, y)].set_style(style);
            }
        }
    }
}

/// Colours the number of the line the cursor is on, as vim does. The text
/// area styles every line number alike, so this finds that line's number in
/// the drawn gutter (wrapped rows have none, so it appears once) and restyles it.
fn highlight_cursor_line_number(buffer: &mut Buffer, editor: &NotesEditor, area: Rect) {
    let number = (editor.textarea.cursor().0 + 1).to_string();
    let gutter = editor.textarea.lines().len().to_string().len() as u16 + 2;
    for y in area.top()..area.bottom() {
        let cells = area.left()..(area.left() + gutter).min(area.right());
        let text: String = cells.clone().map(|x| buffer[(x, y)].symbol().to_string()).collect();
        if text.trim() == number {
            for x in cells {
                if buffer[(x, y)].symbol() != " " {
                    buffer[(x, y)].set_style(CURSOR_LINE_NUMBER);
                }
            }
            return;
        }
    }
}

/// The current line's number in the notes: bold, so it still stands out
/// without colour.
const CURSOR_LINE_NUMBER: Style = Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD);

fn draw_status(frame: &mut Frame, app: &App, area: Rect) {
    // A message for the user takes the bottom line until the next key.
    if let Some(message) = &app.message {
        frame.render_widget(Line::from(truncate(message, area.width as usize).yellow()), area);
        return;
    }
    // A : command on the list or in the notes takes over the bottom line, as in vim.
    let command = match &app.mode {
        Mode::Notes(editor) => editor.command.as_ref(),
        Mode::Normal => app.command.as_ref(),
        Mode::Calendar(calendar) => calendar.command.as_ref(),
        _ => None,
    };
    if let Some(input) = command {
        frame.render_widget(Line::from(format!(":{}", input.text)), area);
        let col = 1 + input.text[..input.cursor].width() as u16;
        frame.set_cursor_position(Position::new(area.x + col.min(area.width.saturating_sub(1)), area.y));
        return;
    }
    // A time being typed in the plan view takes the bottom line too.
    if let Mode::Plan { plan, .. } = &app.mode
        && let Some(typing) = &plan.typing
    {
        let item = app.items().get(app.selected).map(|item| item.text.clone()).unwrap_or_default();
        let (labels, input) = match typing {
            Typing::Start(input) => {
                let start = deadline::format_time(plan.default_start(app.now), app.settings.twelve_hour);
                let default = if plan.last_end.is_some() { start } else { format!("now, {start}") };
                ([format!("Start for {} (↵ for {default}): ", truncate(&item, 20)), "Start: ".into()], input)
            }
            Typing::End(start, input) => {
                let hour = deadline::format_time(timeline::an_hour_after(*start), app.settings.twelve_hour);
                ([format!("End (↵ for {hour}): "), "End: ".into()], input)
            }
        };
        // The shorter label if the longer leaves no room for what's typed.
        let room = (area.width as usize).saturating_sub(input.text.width() + 6);
        let label = labels.iter().find(|label| label.width() <= room).unwrap_or(&labels[1]).clone();
        let mut line = Line::from(vec![label.clone().yellow().bold(), input.text.clone().into()]);
        if let Some(error) = &plan.error {
            line.push_span(format!("  {error}").red());
        }
        frame.render_widget(line, area);
        let col = (label.width() + input.text[..input.cursor].width()) as u16;
        frame.set_cursor_position(Position::new(area.x + col.min(area.width.saturating_sub(1)), area.y));
        return;
    }
    // The list's hints name its keys as keys.toml has them, leaving out a
    // command with none.
    let normal: Vec<(String, u8)> = [
        (Command::Add, "add", 4),
        (Command::AddSub, "add sub-item", 1),
        (Command::Edit, "edit", 2),
        (Command::ToggleDone, "done", 3),
        (Command::Delete, "delete", 1),
        (Command::OpenNotes, "notes", 0),
        (Command::Help, "help", 5),
    ]
    .into_iter()
    .filter_map(|(command, word, priority)| {
        let key = app.keymap.list.iter().find(|binding| binding.command == command)?;
        Some((format!("{} {word}", hint_key(&keymap::show_keys(&key.keys))), priority))
    })
    .collect();
    let normal: Vec<(&str, u8)> = normal.iter().map(|(hint, priority)| (hint.as_str(), *priority)).collect();
    // Each hint has a priority. On a narrow screen the lowest go first, so
    // `? help` is the last to go.
    let (mode, color, hints): (_, _, &[(&str, u8)]) = match &app.mode {
        Mode::Normal => ("NORMAL", Color::Blue, &normal),
        Mode::Insert { .. } if !app.tag_suggestions().is_empty() => {
            ("INSERT", Color::Green, &[("tab complete tag", 3), ("↑/↓ choose", 1), ("enter save as typed", 2)])
        }
        Mode::Insert { repeat: true, .. } => {
            ("INSERT", Color::Green, &[("←/→ move", 1), ("enter add & next", 3), ("esc done", 2)])
        }
        Mode::Insert { editing: false, .. } => {
            ("INSERT", Color::Green, &[("←/→ move", 1), ("enter/esc save", 2), ("(empty discards)", 0)])
        }
        Mode::Insert { replacing: true, .. } => {
            ("INSERT", Color::Green, &[("←/→ move", 1), ("enter/esc save", 2), ("(empty keeps it)", 0)])
        }
        Mode::Insert { editing: true, .. } => {
            ("INSERT", Color::Green, &[("←/→ move", 1), ("enter/esc save", 2), ("(empty asks to delete)", 0)])
        }
        Mode::ConfirmDelete { .. } => ("DELETE", Color::Red, &[("d confirm", 1), ("c cancel", 1)]),
        Mode::Visual { .. } => (
            "VISUAL",
            Color::Magenta,
            &[
                ("x done", 4),
                ("m pin", 2),
                ("! triage", 1),
                ("d delete", 3),
                ("y copy", 2),
                ("H/L move", 1),
                ("esc cancel", 5),
            ],
        ),
        Mode::Notes(editor) if editor.insert => ("INSERT", Color::Green, &[("esc normal mode", 0)]),
        Mode::Notes(editor) if editor.visual_lines => {
            ("V-LINE", Color::Magenta, &[("y copy", 2), ("d delete", 2), ("J/K move", 1), ("esc cancel", 3)])
        }
        Mode::Notes(editor) if editor.visual.is_some() => {
            ("VISUAL", Color::Magenta, &[("y copy", 2), ("d cut", 2), ("J/K move", 1), ("esc cancel", 3)])
        }
        Mode::Notes(_) => {
            ("NORMAL", Color::Blue, &[("i/a/o insert", 2), ("x delete", 1), ("dd delete line", 0), ("? help", 3)])
        }
        Mode::View(viewer) if viewer.read_only() => ("VIEW", Color::Cyan, &[("j/k scroll", 2), ("esc back", 1), ("? help", 4)]),
        Mode::View(_) => ("VIEW", Color::Cyan, &[("j/k scroll", 2), ("i edit", 3), ("esc back", 1), ("? help", 4)]),
        Mode::Tags(_) => ("TAGS", Color::Cyan, &[("j/k move", 1), ("↵ show items", 2), ("esc close", 3)]),
        Mode::Workspaces(picker) if picker.adding.is_some() || picker.deleting.is_some() => {
            ("WORKSPACES", Color::Cyan, &[("enter confirm", 2), ("esc back", 1)])
        }
        Mode::Workspaces(_) => (
            "WORKSPACES",
            Color::Cyan,
            &[("↵ open", 3), ("a new", 2), ("d delete", 1), ("esc close", 4)],
        ),
        Mode::Changelog(_) => ("NEWS", Color::Cyan, &[("j/k scroll", 1), ("esc close", 2)]),
        Mode::Options(options) if options.searching => {
            ("OPTIONS", Color::Blue, &[("type to search", 0), ("enter keep", 2), ("esc clear", 1)])
        }
        Mode::Options(_) => {
            ("OPTIONS", Color::Blue, &[("j/k move", 1), ("space toggle", 3), ("/ search", 2), ("esc close", 4)])
        }
        Mode::Search(search) if search.typing => {
            ("SEARCH", Color::Yellow, &[("↑/↓ select", 1), ("↵ go to item", 2), ("esc stop typing", 3)])
        }
        Mode::Search(_) => {
            ("SEARCH", Color::Yellow, &[("j/k select", 3), ("↵ go to item", 2), ("i type", 1), ("esc close", 4)])
        }
        Mode::Calendar(calendar) if calendar.snake.as_ref().is_some_and(|game| game.over()) => {
            ("SNAKE", Color::Green, &[("↵ play again", 2), ("esc quit", 1)])
        }
        Mode::Calendar(calendar) if calendar.snake.as_ref().is_some_and(|game| game.state == snake::State::Ready) => {
            ("SNAKE", Color::Green, &[("hjkl to start", 2), ("esc quit", 1)])
        }
        Mode::Calendar(calendar) if calendar.snake.is_some() => ("SNAKE", Color::Green, &[("hjkl turn", 2), ("esc quit", 1)]),
        Mode::Calendar(calendar) if calendar.adding.is_some() && !app.tag_suggestions().is_empty() => {
            ("ADD", Color::Green, &[("tab complete tag", 3), ("↑/↓ choose", 1), ("enter save as typed", 2)])
        }
        Mode::Calendar(calendar) if calendar.adding.is_some() => {
            ("ADD", Color::Green, &[("enter/esc save", 1), ("(empty discards)", 0)])
        }
        Mode::Timeline { .. } => {
            ("TIMELINE", Color::Cyan, &[("j/k scroll", 2), ("h/l day", 3), ("p plan", 4), ("esc back", 1), ("? help", 5)])
        }
        Mode::Plan { plan, .. } if plan.confirm_cancel => {
            ("PLAN", Color::Yellow, &[("w save", 3), ("q don't save", 2), ("n keep planning", 1)])
        }
        Mode::Plan { .. } => (
            "PLAN",
            Color::Yellow,
            &[("↵ set time", 5), ("a add", 1), ("! triage", 1), ("c clear", 2), ("w save", 4), ("R reset", 0), ("esc cancel", 3)],
        ),
        Mode::Leader(_) => ("LEADER", Color::Magenta, &[("type a key", 2), ("⌫ back", 0), ("esc cancel", 1)]),
        Mode::Remind(picker) if picker.amount.is_some() => {
            ("REMIND", Color::Yellow, &[("enter save", 2), ("esc back", 1)])
        }
        Mode::Remind(picker) if picker.current.is_some() => (
            "REMIND",
            Color::Yellow,
            &[("h/d/m before", 3), ("c on a day", 2), ("x remove", 1), ("esc cancel", 4)],
        ),
        Mode::Remind(_) => ("REMIND", Color::Yellow, &[("h/d/m before", 3), ("c on a day", 2), ("esc cancel", 4)]),
        Mode::Calendar(calendar) if calendar.picking.as_ref().is_some_and(|picking| picking.reminder && picking.time.is_some()) => {
            ("REMIND", Color::Yellow, &[("enter save", 2), ("(empty: all day)", 0), ("esc back", 1)])
        }
        Mode::Calendar(calendar) if calendar.picking.as_ref().is_some_and(|picking| picking.reminder) => (
            "REMIND",
            Color::Yellow,
            &[("↵ choose day", 3), ("y/m/w view", 0), ("esc cancel", 1), ("? help", 4)],
        ),
        Mode::Calendar(calendar) if calendar.picking.as_ref().is_some_and(|picking| picking.time.is_some()) => {
            ("DEADLINE", Color::Yellow, &[("enter save", 2), ("(empty: no time)", 0), ("esc back", 1)])
        }
        Mode::Calendar(calendar) if calendar.picking.as_ref().is_some_and(|picking| picking.had) => (
            "DEADLINE",
            Color::Yellow,
            &[("↵ choose day", 3), ("d remove", 2), ("y/m/w view", 0), ("esc cancel", 1), ("? help", 4)],
        ),
        Mode::Calendar(calendar) if calendar.picking.is_some() => (
            "DEADLINE",
            Color::Yellow,
            &[("↵ choose day", 3), ("y/m/w view", 0), ("esc cancel", 1), ("? help", 4)],
        ),
        Mode::Calendar(_) => (
            "CALENDAR",
            Color::Cyan,
            &[("a add", 4), ("↵ open day", 3), ("w/m/y view", 2), ("t today", 1), ("? help", 5)],
        ),
        Mode::Help { .. } => {
            ("HELP", Color::Magenta, &[("type to search", 0), ("↑/↓ scroll", 1), ("esc close", 2)])
        }
    };
    let label = format!(" {mode} ");
    let hints = fit_hints(hints, (area.width as usize).saturating_sub(label.width() + 2));
    let line = Line::from(vec![label.bold().fg(Color::Black).bg(color), "  ".into(), hints.dim()]);
    frame.render_widget(Paragraph::new(line), area);
}

/// Joins `hints` with two spaces, dropping the lowest-priority ones (the
/// rightmost of equals) until the line fits in `width`.
fn fit_hints(hints: &[(&str, u8)], width: usize) -> String {
    let mut shown = hints.to_vec();
    loop {
        let line = shown.iter().map(|(hint, _)| *hint).collect::<Vec<_>>().join("  ");
        if line.width() <= width || shown.is_empty() {
            return line;
        }
        let lowest = shown.iter().enumerate().rev().min_by_key(|(_, (_, priority))| *priority).map(|(i, _)| i);
        shown.remove(lowest.expect("shown is not empty"));
    }
}

/// A key as the hints show it: `↵` for Enter, to keep them short.
fn hint_key(key: &str) -> String {
    key.replace("Enter", "↵")
}

/// The first of the list's keys for `command`, as the hints show it, or
/// `–` if it has none.
fn list_key(app: &App, command: Command) -> String {
    match app.keymap.list.iter().find(|binding| binding.command == command) {
        Some(binding) => hint_key(&keymap::show_keys(&binding.keys)),
        None => "–".into(),
    }
}

/// The first of `options` that fits in `width`, or nothing.
fn fit_first<'a>(options: &[&'a str], width: usize) -> &'a str {
    options.iter().find(|option| option.width() <= width).copied().unwrap_or("")
}

/// `text` cut to `width` columns, ending with "…" if anything was cut.
fn truncate(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_string();
    }
    let mut cut = String::new();
    for c in text.chars() {
        if cut.width() + c.width().unwrap_or(0) + 1 > width {
            break;
        }
        cut.push(c);
    }
    if width > 0 {
        cut.push('…');
    }
    cut
}

fn draw_confirm(frame: &mut Frame, app: &App, rows: &[usize]) {
    let items = app.items();
    let screen = frame.area();
    let width = screen.width.saturating_sub(4).min(50);
    // Room for the borders, a blank line and the d/c hint, and a line of margin.
    let max_lines = (screen.height as usize).saturating_sub(6).max(1);

    // Each item as on the list (completed ones aren't numbered), its long text
    // wrapped under itself, until the popup reaches the screen height.
    let mut body: Vec<Line> = Vec::new();
    for (n, &row) in rows.iter().enumerate() {
        let Some(item) = items.get(row) else { continue };
        let prefix = row_label(app, row);
        let text_width = (width as usize).saturating_sub(2 + prefix.width()).max(1);
        let room = max_lines.saturating_sub(body.len());
        if room == 0 || (room == 1 && n + 1 < rows.len()) {
            body.push(Line::from(format!("…and {} more", rows.len() - n)).dim());
            break;
        }
        let indent = " ".repeat(prefix.width());
        for (i, line) in wrap(&item.text, text_width, room).into_iter().enumerate() {
            body.push(Line::from(format!("{}{line}", if i == 0 { &prefix } else { &indent })));
        }
        // What goes with it, so deleting a parent isn't a surprise.
        let under = app.slots().get(row).map_or(0, |slot| {
            let items = app.store.items(slot.day);
            outline::block_end(items, slot.index) - slot.index - 1
        });
        if under > 0 && body.len() < max_lines {
            let s = if under == 1 { "" } else { "s" };
            body.push(Line::from(format!("{indent}and its {under} sub-item{s}")).dim());
        }
    }
    body.push(Line::default());
    body.push(
        Line::from(vec!["d".bold().fg(Color::Red), " delete   ".into(), "c".bold(), " cancel".into()]).centered(),
    );

    let title = if rows.len() == 1 { " Delete item? ".to_string() } else { format!(" Delete {} items? ", rows.len()) };
    let area = centered(screen, width, body.len() as u16 + 2);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Color::Red))
        .title(title.bold());
    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(body).block(block), area);
}

/// How the row is labelled on the list, to name it elsewhere: its number,
/// `✓` if completed, or a bullet for a sub-item.
fn row_label(app: &App, row: usize) -> String {
    let items = app.items();
    let Some(item) = items.get(row) else { return String::new() };
    match (item.depth, item.done) {
        (0, true) => "✓  ".into(),
        (0, false) => format!("{}. ", items[..=row].iter().filter(|item| item.depth == 0).count()),
        (_, true) => format!("{SUB_DONE} "),
        (_, false) => format!("{SUB_OPEN} "),
    }
}

/// Splits `text` into lines of at most `width` columns, breaking after spaces
/// where possible and inside words that are longer than a line. The byte
/// ranges cover all of `text`, so the text being typed keeps its spaces and a
/// cursor offset can be placed on a line. Spaces at a break stay at the end of
/// the line before it and may run past `width`; trim them for display.
fn wrap_ranges(text: &str, width: usize) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut start = 0;
    let mut line_width = 0;
    // Just after the latest run of spaces on this line, and the width up to there.
    let mut last_break: Option<(usize, usize)> = None;
    for (i, c) in text.char_indices() {
        let w = c.width().unwrap_or(0);
        if c == ' ' {
            line_width += w;
            last_break = Some((i + 1, line_width));
            continue;
        }
        while line_width > 0 && line_width + w > width {
            match last_break.take() {
                Some((at, at_width)) if at > start => {
                    ranges.push(start..at);
                    start = at;
                    line_width -= at_width;
                }
                _ => {
                    ranges.push(start..i);
                    start = i;
                    line_width = 0;
                }
            }
        }
        line_width += w;
    }
    ranges.push(start..text.len());
    ranges
}

/// The (line, column) of byte offset `cursor` in text wrapped into `ranges`. At
/// a break the cursor goes to the start of the next line, like in an editor.
fn cursor_position(text: &str, ranges: &[Range<usize>], cursor: usize, width: usize) -> (usize, usize) {
    let line = ranges.iter().rposition(|range| range.start <= cursor).unwrap_or(0);
    let col = text[ranges[line].start..cursor].width();
    (line, col.min(width))
}

/// `text` wrapped to `width` columns for display. Past `max_lines`, the last
/// line ends with "…".
fn wrap(text: &str, width: usize, max_lines: usize) -> Vec<String> {
    let mut lines: Vec<String> =
        wrap_ranges(text, width).into_iter().map(|range| text[range].trim_end().to_string()).collect();
    if lines.len() > max_lines {
        lines.truncate(max_lines);
        let last = lines.last_mut().expect("max_lines is at least 1");
        while !last.is_empty() && last.width() + 1 > width {
            last.pop();
        }
        last.push('…');
    }
    lines
}

/// Background of the selected row or day.
const SELECTED_BG: Color = Color::Rgb(50, 50, 60);

/// The colour of `#tags` in items' text.
const TAG_COLOUR: Color = Color::Cyan;

/// The text of `text[line]` as spans, with any `#tags` in it coloured.
fn tagged(text: &str, line: Range<usize>, base: Style) -> Vec<Span<'static>> {
    let tags: Vec<Range<usize>> = tags::find_tags(text).into_iter().map(|(range, _)| range).collect();
    marked(text, line, &tags, base, base.fg(TAG_COLOUR))
}

/// The text of `text[line]` as spans, with the parts inside any of `marks`
/// (byte ranges of `text`, in order) in the `mark` style.
fn marked(text: &str, line: Range<usize>, marks: &[Range<usize>], base: Style, mark: Style) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut at = line.start;
    for range in marks {
        let (start, end) = (range.start.max(at), range.end.min(line.end));
        if start >= end {
            continue;
        }
        if start > at {
            spans.push(Span::styled(text[at..start].to_string(), base));
        }
        spans.push(Span::styled(text[start..end].to_string(), mark));
        at = end;
    }
    if at < line.end {
        spans.push(Span::styled(text[at..line.end].to_string(), base));
    }
    spans
}

fn draw_tags(frame: &mut Frame, app: &App, picker: &TagPicker) {
    let all = tags::all_tags(&app.store);
    let screen = frame.area();
    let width = screen.width.saturating_sub(4).min(44);
    let hint = "No tags yet. Write #words in an item to tag it.";
    let rows = if all.is_empty() { wrap(hint, (width as usize).saturating_sub(4).max(1), usize::MAX).len() } else { all.len() };
    let height = (rows as u16 + 2).min(screen.height.saturating_sub(2));
    let area = centered(screen, width, height);
    let room = (width as usize).saturating_sub(2);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(TAG_COLOUR))
        .title(" Tags ".bold())
        .title_bottom(Line::from(fit_first(&[" ↵ show items · esc close ", " esc close "], room)).centered().dim())
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    if all.is_empty() {
        frame.render_widget(Paragraph::new(hint).dim().wrap(Wrap { trim: true }), inner);
        return;
    }

    // Each tag with how many items have it, the counts lined up on the right.
    let width = inner.width as usize;
    let items: Vec<ListItem> = all
        .iter()
        .map(|(name, count)| {
            let count = count.to_string();
            let name = truncate(&format!("#{name}"), width.saturating_sub(count.len() + 1));
            let gap = " ".repeat(width.saturating_sub(name.width() + count.len()));
            ListItem::new(Line::from(vec![name.fg(TAG_COLOUR), gap.into(), count.dark_gray()]))
        })
        .collect();
    let mut state = ListState::default().with_offset(picker.offset.get()).with_selected(Some(picker.selected));
    frame.render_stateful_widget(List::new(items).highlight_style(Style::new().bg(SELECTED_BG)), inner, &mut state);
    picker.offset.set(state.offset());
}

/// Background of the rows selected with `V`.
/// Also used for text selected with `v` in the notes, so both look alike.
pub const VISUAL_BG: Color = Color::Rgb(70, 50, 90);

fn draw_calendar(frame: &mut Frame, app: &App, calendar: &Calendar, area: Rect) {
    let cursor = calendar.cursor;
    let room = (area.width as usize).saturating_sub(4);
    let (titles, hints): (Vec<String>, &[&str]) = match calendar.zoom {
        Zoom::Week => {
            let start = calendar::week_start(cursor);
            let end = start + Days::new(6);
            let long = if start.month() == end.month() {
                format!("{} – {}", start.format("%-d"), end.format("%-d %B %Y"))
            } else {
                format!("{} – {}", start.format("%-d %b"), end.format("%-d %b %Y"))
            };
            (
                vec![long, start.format("Week of %-d %b %Y").to_string(), start.format("w/c %-d %b").to_string()],
                &[" j/k day · h/l week · H/L month ", " j/k day · h/l week ", " j/k day "],
            )
        }
        Zoom::Month => (
            vec![cursor.format("%B %Y").to_string(), cursor.format("%b %Y").to_string()],
            &[" h/l day · j/k week · H/L month ", " H/L month "],
        ),
        Zoom::Year => (vec![cursor.format("%Y").to_string()], &[" hjkl move · H/L month ", " H/L month "]),
    };
    // Choosing a deadline says what for, and shows its own keys.
    let (titles, hints, colour): (Vec<String>, &[&str], Color) = match &calendar.picking {
        Some(picking) => {
            // The view's own title, like "2026" or "October 2026".
            let period = titles.last().cloned().unwrap_or_default();
            let (what, short) = if picking.reminder { ("Remind me of", "Remind") } else { ("Deadline for", "Deadline") };
            let titles = vec![
                format!("{what} {} · {}", picking.item, titles[0]),
                format!("{what} {}", truncate(&picking.item, room.saturating_sub(what.width() + 1).max(2))),
                format!("{short} · {period}"),
                short.into(),
                period,
            ];
            let hints: &[&str] = if picking.had {
                &[" ↵ choose day · d remove deadline · esc cancel ", " ↵ choose · d remove · esc ", " ↵ choose · esc "]
            } else {
                &[" ↵ choose day · esc cancel ", " ↵ choose · esc ", " ↵ choose "]
            };
            (titles, hints, Color::Yellow)
        }
        None => (titles, hints, Color::Cyan),
    };
    // A game of Snake shows its score as the title.
    let (titles, hints, colour): (Vec<String>, &[&str], Color) = match &calendar.snake {
        Some(game) => {
            let score = format!("{SNAKE_FOOD} {}", game.score);
            match game.state {
                snake::State::Ready => {
                    (vec![format!("Snake {} · {score}", game.year), score], &[" hjkl to start · esc quit ", " hjkl start ", " esc "], Color::Green)
                }
                snake::State::Playing => (vec![format!("Snake {} · {score}", game.year), score], &[" hjkl turn · esc quit ", " esc quit "], Color::Green),
                snake::State::Lost => (vec![format!("Game over · {score}"), score], &[" ↵ play again · esc quit ", " ↵ again · esc "], Color::Red),
                snake::State::Won => (
                    vec![format!("You filled {}! · {score}", game.year), format!("You won · {score}"), score],
                    &[" ↵ play again · esc quit ", " ↵ again · esc "],
                    Color::Green,
                ),
            }
        }
        None => (titles, hints, colour),
    };
    let title = titles.iter().find(|title| title.width() <= room).cloned().unwrap_or_else(|| truncate(&titles[0], room));
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(colour))
        .title(Line::from(format!(" {title} ").bold()).centered())
        .title_bottom(Line::from(fit_first(hints, area.width.saturating_sub(2) as usize)).centered().dim());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    match calendar.zoom {
        Zoom::Week => draw_week(frame, app, calendar, inner),
        Zoom::Month => draw_month(frame, app, calendar, inner),
        Zoom::Year => draw_year(frame, app, calendar, inner),
    }
}

/// A day's top-level items: the calendar leaves sub-items out, to keep
/// each day short.
fn top_level(app: &App, day: NaiveDate) -> Vec<&Item> {
    app.store.items(day).iter().filter(|item| item.depth == 0).collect()
}

/// The days of the selected week, top to bottom, each with its items.
fn draw_week(frame: &mut Frame, app: &App, calendar: &Calendar, area: Rect) {
    let start = calendar::week_start(calendar.cursor);
    let days: Vec<NaiveDate> = (0..7).map(|i| start + Days::new(i)).collect();
    let width = area.width as usize;
    let height = area.height as usize;
    // Show every item if they fit; otherwise give each day an equal share.
    let needed: usize = days.iter().map(|day| 1 + top_level(app, *day).len()).sum();
    let share = if needed <= height { usize::MAX } else { (height / 7).max(1) };

    let mut lines = Vec::new();
    for day in days {
        let mut header = vec![Span::raw(day.format("%a %-d %b").to_string())];
        if day == calendar.today {
            header.push(" (today)".fg(Color::Green));
        }
        let items = top_level(app, day);
        let room = share - 1;
        // With no room under the date, count the items beside it instead.
        if room == 0 && !items.is_empty() {
            header.push(format!("  +{}", items.len()).dim());
        }
        let mut header = Line::from(header).bold();
        if day == calendar.cursor {
            header = header.bg(SELECTED_BG);
        } else if day == calendar.today {
            header = header.fg(Color::Green);
        }
        lines.push(header);
        if room == 0 {
            continue;
        }

        let shown = if items.len() <= room { items.len() } else { room.saturating_sub(1) };
        let mut number = 0;
        for item in &items[..shown] {
            let prefix = if item.done {
                "   ✓ ".to_string()
            } else {
                number += 1;
                format!("  {number:>2}. ")
            };
            let text = truncate(&item.text, width.saturating_sub(prefix.width()));
            lines.push(Line::from(vec![prefix.dark_gray(), Span::styled(text, item_style(item))]));
        }
        if shown < items.len() {
            lines.push(Line::from(format!("     +{} more", items.len() - shown)).dim());
        }
    }
    frame.render_widget(Paragraph::new(lines), area);
}

/// A grid of the selected month's weeks, Monday first. Each day shows as many
/// of its items as fit, or a dot if there's only room for the date.
fn draw_month(frame: &mut Frame, app: &App, calendar: &Calendar, area: Rect) {
    let weeks = calendar::month_weeks(calendar.cursor);
    let column_x = |i: u16| area.x + i * area.width / 7;
    let cell_width = |i: u16| column_x(i + 1) - column_x(i);
    let narrowest = cell_width(0).min(cell_width(6)) as usize;

    let names = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    let name_len = narrowest.saturating_sub(1).clamp(1, 3);
    for (i, name) in names.iter().enumerate() {
        let rect = Rect::new(column_x(i as u16), area.y, cell_width(i as u16), 1);
        frame.render_widget(Line::from(&name[..name_len]).dim(), rect);
    }

    let rows = area.height.saturating_sub(1);
    let cell_height = (rows / weeks.len() as u16).max(1);
    for (w, monday) in weeks.iter().enumerate() {
        let y = area.y + 1 + w as u16 * cell_height;
        if y >= area.bottom() {
            break;
        }
        for i in 0..7u16 {
            let day = *monday + Days::new(i.into());
            let rect = Rect::new(column_x(i), y, cell_width(i), cell_height.min(area.bottom() - y));
            draw_month_day(frame, app, calendar, day, rect);
        }
    }
}

fn draw_month_day(frame: &mut Frame, app: &App, calendar: &Calendar, day: NaiveDate, rect: Rect) {
    let items = top_level(app, day);
    let in_month = day.month() == calendar.cursor.month();
    let mut number = Span::raw(format!("{:>2}", day.day()));
    if day == calendar.today {
        number = number.fg(Color::Green).bold();
    } else if !in_month {
        number = number.dark_gray();
    }
    let text_width = (rect.width as usize).saturating_sub(1);
    let mut lines = vec![Line::from(number)];
    if rect.height == 1 {
        if items.iter().any(|item| !item.done) && text_width >= 3 {
            lines[0].push_span("•".yellow());
        }
    } else {
        let room = rect.height as usize - 1;
        let shown = if items.len() <= room { items.len() } else { room.saturating_sub(1) };
        for item in &items[..shown] {
            lines.push(Line::from(Span::styled(truncate(&item.text, text_width), item_style(item))));
        }
        if shown < items.len() {
            lines.push(Line::from(truncate(&format!("+{} more", items.len() - shown), text_width)).dim());
        }
    }
    let mut paragraph = Paragraph::new(lines);
    if !in_month {
        paragraph = paragraph.dim();
    }
    if day == calendar.cursor {
        paragraph = paragraph.bg(SELECTED_BG);
    }
    frame.render_widget(paragraph, rect);
}

/// All twelve months of the selected year as small grids, as many side by side
/// as fit. Days with open items are highlighted. If they don't all fit, the
/// rows scroll to keep the selected month in view.
fn draw_year(frame: &mut Frame, app: &App, calendar: &Calendar, area: Rect) {
    const MONTH_WIDTH: u16 = 20;
    const MONTH_HEIGHT: u16 = 8;
    let columns = ((area.width + 2) / (MONTH_WIDTH + 2)).clamp(1, 4);
    calendar.year_columns.set(columns.into());
    let used = columns * MONTH_WIDTH + (columns - 1) * 2;
    let left = area.x + area.width.saturating_sub(used) / 2;
    let visible_rows = ((area.height + 1) / (MONTH_HEIGHT + 1)).max(1);
    // The game keeps the snake's head in view, as browsing does the cursor.
    let focus = calendar.snake.as_ref().map_or(calendar.cursor, |game| game.head());
    let cursor_row = (focus.month0() as u16) / columns;
    let first_row = (cursor_row + 1).saturating_sub(visible_rows);

    for month in 1..=12u32 {
        let index = month as u16 - 1;
        let (row, column) = (index / columns, index % columns);
        if row < first_row || row >= first_row + visible_rows {
            continue;
        }
        let y = area.y + (row - first_row) * (MONTH_HEIGHT + 1);
        let x = left + column * (MONTH_WIDTH + 2);
        let width = MONTH_WIDTH.min(area.right().saturating_sub(x));
        let height = MONTH_HEIGHT.min(area.bottom().saturating_sub(y));
        let first = NaiveDate::from_ymd_opt(focus.year(), month, 1).expect("valid month");
        draw_year_month(frame, app, calendar, first, Rect::new(x, y, width, height));
    }
}

fn draw_year_month(frame: &mut Frame, app: &App, calendar: &Calendar, first: NaiveDate, rect: Rect) {
    let focus = calendar.snake.as_ref().map_or(calendar.cursor, |game| game.head());
    let mut name = Line::from(first.format("%B").to_string()).centered();
    name = if first.month() == focus.month() { name.cyan().bold() } else { name.bold() };
    let mut lines = vec![name, Line::from("Mo Tu We Th Fr Sa Su").dim()];
    for monday in calendar::month_weeks(first) {
        let mut spans = Vec::new();
        for i in 0..7u64 {
            let day = monday + Days::new(i);
            if i > 0 {
                spans.push(Span::raw(" "));
            }
            if day.month() != first.month() {
                spans.push(Span::raw("  "));
                continue;
            }
            if let Some(game) = &calendar.snake {
                spans.push(snake_cell(game, day));
                continue;
            }
            let items = app.store.items(day);
            let mut style = Style::new();
            if items.iter().any(|item| !item.done) {
                style = style.fg(Color::Yellow).bold();
            } else if !items.is_empty() {
                style = style.fg(Color::DarkGray);
            }
            if day == calendar.today {
                style = style.fg(Color::Green).bold();
            }
            if day == calendar.cursor {
                style = style.bg(SELECTED_BG).add_modifier(Modifier::REVERSED);
            }
            spans.push(Span::styled(format!("{:>2}", day.day()), style));
        }
        lines.push(Line::from(spans));
    }
    frame.render_widget(Paragraph::new(lines), rect);
}

/// The food in the calendar's game of Snake, which is also its score's icon.
const SNAKE_FOOD: &str = "●";

/// The snake's head: a much brighter green than its body. An exact colour,
/// since a theme's "light green" can be barely brighter than its green.
const SNAKE_HEAD: Color = Color::Rgb(170, 255, 140);

/// A day in the game of Snake. The snake is the days under a green
/// background (reversed text without colour), its head bright and bold, and
/// red once the game is lost. The food replaces its day's number.
fn snake_cell(game: &Snake, day: NaiveDate) -> Span<'static> {
    let number = format!("{:>2}", day.day());
    match game.cell(day) {
        snake::Cell::Head => {
            let bg = if game.state == snake::State::Lost { Color::Red } else { SNAKE_HEAD };
            Span::styled(number, Style::new().fg(Color::Black).bg(bg).add_modifier(Modifier::BOLD))
        }
        snake::Cell::Body => Span::styled(number, Style::new().fg(Color::Black).bg(Color::Green)),
        snake::Cell::Food => Span::styled(format!("{SNAKE_FOOD:>2}"), Style::new().fg(Color::Red).add_modifier(Modifier::BOLD)),
        snake::Cell::Empty => Span::styled(number, Style::new().fg(Color::DarkGray)),
    }
}

/// Open items as normal, completed ones crossed out.
fn item_style(item: &Item) -> Style {
    if item.done {
        Style::new().fg(Color::DarkGray).add_modifier(Modifier::CROSSED_OUT)
    } else {
        Style::new()
    }
}

/// The popup for typing an item to add to the calendar's selected day.
fn draw_adding(frame: &mut Frame, app: &App, calendar: &Calendar) {
    let Some(input) = &calendar.adding else { return };
    let screen = frame.area();
    let width = screen.width.saturating_sub(4).min(50);
    let text_width = (width as usize).saturating_sub(2).max(1);
    let ranges = wrap_ranges(&input.text, text_width);
    let (line, col) = cursor_position(&input.text, &ranges, input.cursor, text_width);
    let lines: Vec<Line> = ranges.iter().map(|range| Line::from(input.text[range.clone()].trim_end().to_string())).collect();
    let area = centered(screen, width, lines.len() as u16 + 2);

    let titles = [
        calendar.cursor.format(" Add to %A %-d %B ").to_string(),
        calendar.cursor.format(" Add to %a %-d %b ").to_string(),
        calendar.cursor.format(" %a %-d %b ").to_string(),
    ];
    let room = (width as usize).saturating_sub(2);
    let title = titles.iter().find(|title| title.width() <= room).cloned().unwrap_or_default();
    let block = Block::bordered().border_type(BorderType::Rounded).border_style(Style::new().fg(Color::Green)).title(title.bold());
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(lines).block(block), area);
    let cursor = Position::new(inner.x + col as u16, inner.y + line as u16);
    frame.set_cursor_position(cursor);
    draw_tag_suggestions(frame, app, cursor);
}

/// The time for a deadline, typed once its day is chosen.
fn draw_deadline_time(frame: &mut Frame, app: &App, calendar: &Calendar) {
    let Some(picking) = &calendar.picking else { return };
    let Some(input) = &picking.time else { return };
    let screen = frame.area();
    let width = screen.width.saturating_sub(4).min(50);
    let room = (width as usize).saturating_sub(4);
    let example = if app.settings.twelve_hour { "like 1pm or 13:00" } else { "like 13:00 or 1pm" };
    let help = if picking.reminder {
        format!("A time, {example}. Leave it empty to be reminded from the start of the day.")
    } else {
        format!("A time, {example}. Leave it empty for the whole day.")
    };
    let mut lines = vec![Line::from(vec!["› ".yellow().bold(), truncate(&input.text, room.saturating_sub(2)).into()])];
    lines.extend(wrap(&help, room, usize::MAX).into_iter().map(|part| Line::from(part).dim()));
    if let Some(error) = &picking.error {
        lines.extend(wrap(error, room, usize::MAX).into_iter().map(|part| Line::from(part).red()));
    }
    let area = centered(screen, width, lines.len() as u16 + 2);

    let what = if picking.reminder { "Remind" } else { "Due" };
    let titles = [
        calendar.cursor.format(&format!(" {what} %A %-d %B %Y ")).to_string(),
        calendar.cursor.format(&format!(" {what} %a %-d %b %Y ")).to_string(),
        calendar.cursor.format(" %a %-d %b ").to_string(),
    ];
    let title = titles.iter().find(|title| title.width() <= room + 2).cloned().unwrap_or_default();
    let hint = fit_first(&[" enter save · esc pick another day ", " enter save · esc back ", " enter "], room + 2);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Color::Yellow))
        .title(title.bold())
        .title_bottom(Line::from(hint).centered().dim())
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(lines).block(block), area);
    let col = 2 + input.text[..input.cursor].width() as u16;
    frame.set_cursor_position(Position::new(inner.x + col.min(inner.width.saturating_sub(1)), inner.y));
}

/// The narrowest screen with room for the timeline beside the list.
const PLAN_WIDTH: u16 = 60;

/// The plan view: the list, as on the main screen, with the day's timeline
/// beside it when there's room.
fn draw_plan(frame: &mut Frame, app: &App, area: Rect) {
    app.timeline_shown.set(area.width >= PLAN_WIDTH);
    if area.width < PLAN_WIDTH {
        draw_list(frame, app, area);
        return;
    }
    let [list, side] = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(area);
    draw_list(frame, app, list);
    let selected = app.slots().get(app.selected).copied();
    let on_timeline = match (&app.mode, &app.plan_stash) {
        (Mode::Plan { plan, .. }, _) | (_, Some((plan, _))) => plan.on_timeline,
        _ => false,
    };
    // The side keys go to is bright; the other is dim.
    let title = if on_timeline { " Timeline · H list " } else { " Timeline · L to move here " };
    draw_timeline(frame, app, side, title, selected, None);
    if !on_timeline {
        dim_border(frame.buffer_mut(), side);
    } else {
        dim_border(frame.buffer_mut(), list);
    }
}

/// The popup asking whether to save a plan's changes before leaving, after
/// `q` or `Esc`, like the one asking before a delete. `q` again leaves
/// without saving.
fn draw_discard_plan(frame: &mut Frame) {
    let screen = frame.area();
    let width = screen.width.saturating_sub(4).min(50);
    let room = (width as usize).saturating_sub(2).max(1);
    let mut body: Vec<Line> = wrap("You've set times since opening the plan. Save them before you leave?", room, usize::MAX)
        .into_iter()
        .map(Line::from)
        .collect();
    body.push(Line::default());
    // The keys with what they do, spaced less, then said less, to fit.
    let choices: [(&[(&str, &str)], &str); 6] = [
        (&[("w", "save"), ("q", "don't save"), ("n", "keep planning")], "   "),
        (&[("w", "save"), ("q", "don't save"), ("n", "back")], "   "),
        (&[("w", "save"), ("q", "don't save"), ("n", "back")], "  "),
        (&[("w", "save"), ("q", "discard"), ("n", "back")], "  "),
        (&[("w", "save"), ("q", "discard"), ("n", "no")], "  "),
        (&[("w", ""), ("q", ""), ("n", "")], "   "),
    ];
    let width_of = |(keys, gap): &(&[(&str, &str)], &str)| {
        keys.iter().map(|(key, word)| key.width() + if word.is_empty() { 0 } else { 1 + word.width() }).sum::<usize>()
            + gap.width() * (keys.len() - 1)
    };
    let (keys, gap) = choices.iter().find(|choice| width_of(choice) <= room).unwrap_or(&choices[5]);
    let mut spans = Vec::new();
    for (i, (key, word)) in keys.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(*gap));
        }
        let key_style = match *key {
            "w" => Style::new().bold().fg(Color::Green),
            "q" => Style::new().bold().fg(Color::Red),
            _ => Style::new().bold(),
        };
        spans.push(Span::styled(*key, key_style));
        if !word.is_empty() {
            spans.push(Span::raw(format!(" {word}")));
        }
    }
    body.push(Line::from(spans).centered());
    let title = fit_first(&[" Save your changes to the plan? ", " Save your changes? ", " Save? "], room);
    let area = centered(screen, width, body.len() as u16 + 2);
    let block = Block::bordered().border_type(BorderType::Rounded).border_style(Style::new().fg(Color::Yellow)).title(title.bold());
    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(body).block(block), area);
}

/// Greys out the border around `area`, for the side of the plan view keys
/// don't go to.
fn dim_border(buffer: &mut Buffer, area: Rect) {
    for x in area.left()..area.right() {
        for y in [area.top(), area.bottom().saturating_sub(1)] {
            buffer[(x, y)].set_fg(Color::DarkGray).modifier.remove(Modifier::BOLD);
        }
    }
    for y in area.top()..area.bottom() {
        for x in [area.left(), area.right().saturating_sub(1)] {
            buffer[(x, y)].set_fg(Color::DarkGray);
        }
    }
}

/// Background of a block on the timeline, and of the one selected in the
/// plan view, which is also bold so it stands out without colour.
const BLOCK_BG: Color = Color::Rgb(45, 48, 60);
/// The other shade of the checkerboard, so neighbouring blocks differ.
const BLOCK_ALT_BG: Color = Color::Rgb(62, 66, 84);
/// The selected item in the plan view, on the list and the timeline: a
/// clear blue, unlike any block's shade, and bold too, so it shows without
/// colour.
const BLOCK_SELECTED_BG: Color = Color::Rgb(45, 95, 175);

/// A block's background and the colour of the bar down its left: grey for
/// every block, except a triaged item's, which takes its priority's. Each
/// has two shades for the checkerboard; `alt` is the lighter.
fn block_colours(priority: Option<Priority>, alt: bool) -> (Color, Color) {
    let (dark, light, bar) = match priority {
        Some(Priority::High) => (Color::Rgb(85, 35, 38), Color::Rgb(110, 50, 54), Color::Red),
        Some(Priority::Medium) => (Color::Rgb(80, 68, 28), Color::Rgb(104, 89, 40), Color::Yellow),
        Some(Priority::Low) => (Color::Rgb(35, 70, 42), Color::Rgb(48, 92, 57), Color::Green),
        None => (BLOCK_BG, BLOCK_ALT_BG, Color::Gray),
    };
    (if alt { light } else { dark }, bar)
}

/// Down the left of every block, so each one's edge is clear.
const BLOCK_EDGE: &str = "▌";

/// The day on screen from top to bottom: items without a time listed as all
/// day at the top, then the hours with each timed item as a block, and a
/// line at the time now if it's today. `scroll` is the hour at the top, or
/// `None` to start an hour above now (or the first block).
fn draw_timeline(frame: &mut Frame, app: &App, area: Rect, title: &str, selected: Option<crate::app::Slot>, scroll: Option<u32>) {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Color::Cyan))
        .title(Line::from(truncate(title, area.width.saturating_sub(2) as usize).bold()).centered());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width < 10 || inner.height < 3 {
        return;
    }
    let twelve = app.settings.twelve_hour;
    // The day's top-level items, as on the list.
    let entries: Vec<(crate::app::Slot, &Item)> = app
        .slots()
        .into_iter()
        .map(|slot| (slot, &app.store.items(slot.day)[slot.index]))
        .filter(|(_, item)| item.depth == 0)
        .collect();
    let timed: Vec<(usize, TimeSlot)> = entries
        .iter()
        .enumerate()
        .filter_map(|(i, (_, item))| item.time.filter(|time| time.date == app.day).map(|time| (i, time)))
        .collect();

    let mut y = inner.y;
    // All day: what has no time, on as many lines as it takes, up to two.
    // The selected one highlighted, as on the list.
    let all_day: Vec<(String, bool)> = entries
        .iter()
        .enumerate()
        .filter(|(i, _)| !timed.iter().any(|(t, _)| t == i))
        .map(|(_, (slot, item))| {
            let text = if item.done { format!("✓ {}", item.text) } else { item.text.clone() };
            (text, selected == Some(*slot))
        })
        .collect();
    if !all_day.is_empty() {
        let label = "All day  ";
        let width = inner.width as usize;
        let room = width.saturating_sub(label.width()).max(1);
        // Whole items to a line, on up to two lines under the label; any
        // left over are counted at the end.
        let mut lines: Vec<Vec<Span>> = vec![Vec::new()];
        let mut used = 0;
        for (n, (text, is_selected)) in all_day.iter().enumerate() {
            let style = if *is_selected { Style::new().bg(SELECTED_BG).add_modifier(Modifier::BOLD) } else { Style::new() };
            let gap = if used == 0 { 0 } else { 3 };
            let left = all_day.len() - n;
            // Keep room on the last line to say how many more there are.
            let more = if lines.len() == 2 && left > 1 { format!(" +{} more", left - 1).width() } else { 0 };
            if used > 0 && used + gap + text.width() + more > room {
                if lines.len() == 2 {
                    lines[1].push(format!(" +{left} more").dim());
                    break;
                }
                lines.push(Vec::new());
                used = 0;
            }
            let line = lines.last_mut().expect("a line");
            if used > 0 {
                line.push(" · ".dark_gray());
                used += 3;
            }
            let shown = truncate(text, room.saturating_sub(used));
            used += shown.width();
            line.push(Span::styled(shown, style));
        }
        for (n, spans) in lines.into_iter().enumerate() {
            let lead = if n == 0 { label.to_string() } else { " ".repeat(label.width()) };
            let line = Line::from([vec![lead.dim()], spans].concat());
            frame.render_widget(line, Rect::new(inner.x, y, inner.width, 1));
            y += 1;
        }
        frame.render_widget("─".repeat(inner.width as usize).dark_gray(), Rect::new(inner.x, y, inner.width, 1));
        y += 1;
    }
    if timed.is_empty() && all_day.is_empty() {
        frame.render_widget(Line::from("Nothing on this day.").dim(), Rect::new(inner.x, y, inner.width, 1));
        return;
    }
    let height = inner.bottom().saturating_sub(y) as u32;
    if height == 0 {
        return;
    }

    // The hours, with up to four rows for each when there's room.
    let now = (app.day == app.today).then_some(app.now);
    let slots: Vec<TimeSlot> = timed.iter().map(|(_, time)| *time).collect();
    let (mut first, mut last) = timeline::hours(&slots, now);
    // Whole rows an hour, up to four, so each hour and block is the same
    // height; with less than one each, it scrolls.
    let per_hour = (height / (last - first).max(1)).clamp(1, 4);
    // Room for more hours: show them, later ones first, then earlier.
    let fits = height / per_hour;
    while last - first < fits && (last < 24 || first > 0) {
        if last < 24 {
            last += 1;
        } else {
            first -= 1;
        }
    }
    let span = last - first;
    let total = span * per_hour;
    let minutes_in = |time: chrono::NaiveTime| (time.hour() * 60 + time.minute()).saturating_sub(first * 60);
    let minute_row = |time: chrono::NaiveTime| minutes_in(time) * per_hour / 60;
    let hour_row = |hour: u32| (hour - first) * per_hour;
    let max_top = (total.saturating_sub(height)).div_ceil(per_hour);
    let top_hour = match scroll {
        Some(hour) => hour.min(max_top),
        // An hour above the selected block, or now, or the first block.
        None => {
            let chosen = entries.iter().zip(0..).find(|((slot, _), _)| Some(*slot) == selected).map(|(_, i)| i);
            let chosen = chosen.and_then(|i| timed.iter().find(|(t, _)| *t == i)).map(|(_, time)| time.start);
            let focus = chosen.or(now).or(slots.iter().map(|slot| slot.start).min());
            focus.map_or(0, |time| time.hour().saturating_sub(first + 1)).min(max_top)
        }
    };
    app.timeline_top.set(top_hour);
    let offset = hour_row(first + top_hour);
    let label_width = 6u16;
    let grid_x = inner.x + label_width;
    let grid_width = inner.width.saturating_sub(label_width);
    let screen_row = |row: u32| (row >= offset && row < offset + height).then(|| y + (row - offset) as u16);

    // Each hour's label, with a faint line across when hours have room
    // for one, two rows or more each.
    for hour in first..last {
        let Some(row_y) = screen_row(hour_row(hour)) else { continue };
        let time = chrono::NaiveTime::from_hms_opt(hour, 0, 0).expect("an hour of the day");
        let label = format!("{:>5} ", deadline::format_time(time, twelve));
        frame.render_widget(Line::from(label).dark_gray(), Rect::new(inner.x, row_y, label_width, 1));
    }

    // The blocks, side by side where they overlap, in a checkerboard of two
    // shades: each block differs from the one above or below it in its
    // column, and from those beside it.
    let placed_blocks = timeline::place(&timed);
    let shade = |placed: &timeline::Placed| {
        let start = |p: &timeline::Placed| timed.iter().find(|(i, _)| *i == p.item).map(|(_, time)| (time.start, p.item));
        let above = placed_blocks.iter().filter(|other| other.column == placed.column && start(other) < start(placed)).count();
        (above + placed.column) % 2 == 1
    };
    for placed in placed_blocks.iter().copied() {
        let (slot, item) = entries[placed.item];
        let time = timed.iter().find(|(i, _)| *i == placed.item).map(|(_, time)| *time).expect("placed items have times");
        let start = minute_row(time.start);
        let end = (minutes_in(time.end) * per_hour).div_ceil(60).max(start + 1);
        let column_width = grid_width / placed.columns.max(1) as u16;
        let x = grid_x + column_width * placed.column as u16;
        let width = if placed.column + 1 == placed.columns { grid_width - column_width * placed.column as u16 } else { column_width };
        if width == 0 {
            continue;
        }
        let is_selected = selected == Some(slot);
        let (background, edge) = block_colours(item.priority, shade(&placed));
        let style = if item.done {
            Style::new().bg(background).fg(Color::DarkGray).add_modifier(Modifier::CROSSED_OUT)
        } else if is_selected {
            Style::new().bg(BLOCK_SELECTED_BG).fg(Color::White).add_modifier(Modifier::BOLD)
        } else {
            Style::new().bg(background).fg(Color::White)
        };
        let selected_edge = if item.priority.is_some() { edge } else { Color::White };
        let edge = Style::new().bg(style.bg.unwrap_or(BLOCK_BG)).fg(if item.done { Color::DarkGray } else if is_selected { selected_edge } else { edge });
        let when = time.describe(twelve);
        // The title on its first row, its time on the next if there's room,
        // and a bar down its left, so where each block starts is clear.
        let rows = end - start;
        for (n, row) in (start..end).enumerate() {
            let Some(row_y) = screen_row(row) else { continue };
            let text = match (n, rows) {
                (0, 1) => format!("{} {when}", item.text),
                (0, _) => item.text.clone(),
                (1, _) => when.clone(),
                _ => String::new(),
            };
            let room = width.saturating_sub(1) as usize;
            let shown = format!("{:<room$}", truncate(&text, room));
            let style = if n == 1 { style.remove_modifier(Modifier::BOLD).add_modifier(Modifier::DIM) } else { style };
            let line = Line::from(vec![Span::styled(BLOCK_EDGE, edge), Span::styled(shown, style)]);
            frame.render_widget(line, Rect::new(x, row_y, width, 1));
        }
    }

    // The time now, a red line right across, over the blocks too: where a
    // block's text is in the way, the text stays, in red, so the line runs on
    // through it and the item can still be read.
    if let Some(now) = now
        && let Some(row_y) = screen_row(minute_row(now))
    {
        let label = format!("{:>5} ", deadline::format_time(now, twelve));
        frame.render_widget(Line::from(label).red().bold(), Rect::new(inner.x, row_y, label_width, 1));
        let buffer = frame.buffer_mut();
        for x in grid_x..grid_x + grid_width {
            let cell = &mut buffer[(x, row_y)];
            if cell.symbol().trim().is_empty() {
                cell.set_symbol("─");
            }
            cell.set_fg(Color::Red);
            cell.modifier.insert(Modifier::BOLD);
        }
    }

}

/// The `Space` popup, along the bottom of the list in `area` as in
/// which-key: what each next key does, in as many columns as fit.
fn draw_leader(frame: &mut Frame, app: &App, leader: &Leader, area: Rect) {
    // Inside the list's border, so the popup doesn't cover it.
    let area = area.inner(ratatui::layout::Margin::new(1, 1));
    let entries = leader.entries(&app.keymap);
    let width = area.width.min(80);
    let inner_width = (width as usize).saturating_sub(4).max(1);
    // Each cell is "key  label", padded to the widest so columns line up.
    let key_width = entries.iter().map(|entry| entry.key.name().width()).max().unwrap_or(1);
    let label_width = entries.iter().map(|entry| entry.label.width()).max().unwrap_or(1);
    let gap = 3;
    let cell = (key_width + 2 + label_width).min(inner_width);
    let columns = ((inner_width + gap) / (cell + gap)).max(1);
    let rows = entries.len().div_ceil(columns).max(1);

    let mut lines: Vec<Line> = vec![Line::default(); rows];
    for (i, entry) in entries.iter().enumerate() {
        // Fill down each column, then across, so the order reads top to bottom.
        let line = &mut lines[i % rows];
        if i >= rows {
            line.push_span(" ".repeat(gap));
        }
        let key = format!("{:>key_width$}", entry.key.name());
        let label = truncate(&entry.label, cell.saturating_sub(key_width + 2));
        let label = format!("{label:<width$}", width = cell.saturating_sub(key_width + 2));
        line.push_span(key.yellow().bold());
        line.push_span("  ");
        line.push_span(if entry.group { label.magenta() } else { label.into() });
    }

    let height = (rows as u16 + 2).min(area.height);
    let popup = Rect::new(area.x + (area.width - width) / 2, area.bottom().saturating_sub(height), width, height);
    let typed: String = leader.typed.iter().map(|key| format!(" {}", key.name())).collect();
    let title = truncate(&format!(" {}{typed} ", app.keymap.leader_name()), (width as usize).saturating_sub(2));
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Color::Magenta))
        .title(title.bold())
        .padding(Padding::horizontal(1));
    frame.render_widget(Clear, popup);
    frame.render_widget(Paragraph::new(lines).block(block), popup);
}

/// The `r` popup: how long before the item to be reminded of it, or then how
/// many hours, days or months.
fn draw_remind(frame: &mut Frame, app: &App, picker: &reminder::Picker) {
    let Some(item) = app.store.items(picker.day).get(picker.index) else { return };
    let screen = frame.area();
    let width = screen.width.saturating_sub(4).min(50);
    let room = (width as usize).saturating_sub(4).max(1);
    let twelve_hour = app.settings.twelve_hour;
    let from = reminder::counted_from(picker.day, item.deadline, app.today, twelve_hour);

    let mut lines: Vec<Line> = Vec::new();
    let mut cursor = None;
    if let Some((unit, input)) = &picker.amount {
        cursor = Some(2 + input.text[..input.cursor].width());
        lines.push(Line::from(vec!["› ".yellow().bold(), truncate(&input.text, room.saturating_sub(2)).into()]));
        let help = format!("How many {} before {from}.", unit.name());
        lines.extend(wrap(&help, room, usize::MAX).into_iter().map(|part| Line::from(part).dim()));
        if let Some(error) = &picker.error {
            lines.extend(wrap(error, room, usize::MAX).into_iter().map(|part| Line::from(part).red()));
        }
    } else {
        // The keys first, so a short screen cuts off the explanation instead.
        lines.push(Line::from(truncate(&item.text, room)).bold());
        lines.push(Line::default());
        let mut choices = vec![("h", "hours before"), ("d", "days before"), ("m", "months before"), ("c", "on a day…")];
        if picker.current.is_some() {
            choices.push(("x", "remove the reminder"));
        }
        for (key, what) in choices {
            lines.push(Line::from(vec![format!("{key}  ").yellow().bold(), truncate(what, room.saturating_sub(3)).into()]));
        }
        lines.push(Line::default());
        if let Some(current) = picker.current {
            let now = format!("Reminds you {}.", current.explain(app.today, twelve_hour));
            lines.extend(wrap(&now, room, usize::MAX).into_iter().map(|part| Line::from(part).cyan()));
        }
        let counted = format!("Counted back from {from}.");
        lines.extend(wrap(&counted, room, usize::MAX).into_iter().map(|part| Line::from(part).dim()));
    }
    let height = (lines.len() as u16 + 2).min(screen.height.saturating_sub(2));
    let area = centered(screen, width, height);

    let title = match &picker.amount {
        Some((unit, _)) => {
            let long = match unit {
                reminder::Unit::Hours => " Hours before ",
                reminder::Unit::Days => " Days before ",
                reminder::Unit::Months => " Months before ",
            };
            fit_first(&[long, " Before "], room + 2).to_string()
        }
        None => fit_first(&[" Remind me ", " Remind "], room + 2).to_string(),
    };
    let hint = match &picker.amount {
        Some(_) => fit_first(&[" enter save · esc back ", " enter "], room + 2),
        None => fit_first(&[" esc cancel ", " esc "], room + 2),
    };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Color::Yellow))
        .title(title.bold())
        .title_bottom(Line::from(hint).centered().dim())
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(lines).block(block), area);
    if let Some(col) = cursor {
        frame.set_cursor_position(Position::new(inner.x + (col as u16).min(inner.width.saturating_sub(1)), inner.y));
    }
}

/// The narrowest search that has room for a preview of the selected item.
const SEARCH_PREVIEW_WIDTH: u16 = 90;

fn draw_search(frame: &mut Frame, app: &App, search: &Search) {
    let screen = frame.area();
    // Wide enough, it grows to show the selected item's notes beside the results.
    let preview = screen.width.saturating_sub(4) >= SEARCH_PREVIEW_WIDTH;
    let width = if preview { screen.width.saturating_sub(4).min(160) } else { screen.width.saturating_sub(4).min(80) };
    let area = centered(screen, width, screen.height.saturating_sub(2));
    let room = area.width.saturating_sub(2) as usize;
    let tag_title = search.tag.as_ref().map(|tag| truncate(&format!(" #{tag} "), room));
    let title = if let Some(title) = &tag_title {
        title.as_str()
    } else if let Some(filter) = search.filter {
        match filter {
            search::Filter::Overdue => fit_first(&[" Overdue ", " Late "], room),
            search::Filter::Deadlines => fit_first(&[" Deadlines ", " Due "], room),
            search::Filter::Reminders => fit_first(&[" Reminders ", " Remind "], room),
        }
    } else if search.kind == search::Kind::Notes {
        fit_first(&[" Search notes ", " Notes "], room)
    } else {
        fit_first(&[" Find items ", " Items "], room)
    };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Color::Yellow))
        .title(title.bold())
        .title_bottom(
            Line::from(if search.typing {
                fit_first(&[" ↑/↓ select · ↵ go to item · esc stop typing ", " ↵ go · esc stop typing ", " esc "], room)
            } else {
                fit_first(
                    &[" j/k select · ↵ go to item · i type · ctrl+d/u scroll notes · esc close ", " j/k · ↵ go · i type · esc close ", " ↵ go · esc close ", " esc "],
                    room,
                )
            })
            .centered()
            .dim(),
        )
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);

    let [prompt_area, rule, body] =
        Layout::vertical([Constraint::Length(1), Constraint::Length(1), Constraint::Fill(1)]).areas(inner);
    let (results, preview_area) = if preview {
        let [results, divider, preview_area] =
            Layout::horizontal([Constraint::Percentage(45), Constraint::Length(3), Constraint::Fill(1)]).areas(body);
        for y in divider.top()..divider.bottom() {
            frame.render_widget(Line::from("│").dark_gray().centered(), Rect { y, height: 1, ..divider });
        }
        (results, Some(preview_area))
    } else {
        (body, None)
    };
    let prompt = "Search: ";
    let query = &search.input.text;
    let shown = if query.is_empty() {
        let hints: &[&str] = if search.filter == Some(search::Filter::Overdue) {
            &["every item past its deadline; type to narrow down", "type to narrow down", ""]
        } else if search.filter == Some(search::Filter::Deadlines) {
            &["every deadline, soonest first; type to narrow down", "type to narrow down", ""]
        } else if search.filter == Some(search::Filter::Reminders) {
            &["every reminder, soonest first; type to narrow down", "type to narrow down", ""]
        } else if search.tag.is_some() {
            &["type to narrow these down", "narrow down", ""]
        } else if search.kind == search::Kind::Notes {
            &["type to search inside every item's notes", "inside every note", "notes"]
        } else {
            &["type to narrow down every item, on any day", "narrow down every item", "any item"]
        };
        fit_first(hints, (prompt_area.width as usize).saturating_sub(prompt.width())).dark_gray()
    } else {
        query.as_str().into()
    };
    frame.render_widget(Line::from(vec![prompt.dim(), shown]), prompt_area);
    frame.render_widget("─".repeat(rule.width as usize).dark_gray(), rule);
    if search.typing {
        let typed = (prompt.width() + query[..search.input.cursor].width()) as u16;
        frame.set_cursor_position(Position::new(prompt_area.x + typed.min(prompt_area.width), prompt_area.y));
    }

    if query.trim().is_empty() && search.kind == search::Kind::Notes {
        search.preview_height.set(0);
        return;
    }
    let hits = search.find(&app.store, app.today, app.now);
    if hits.is_empty() {
        let message = if search.filter == Some(search::Filter::Overdue) && query.trim().is_empty() {
            "Nothing is overdue".to_string()
        } else if search.filter == Some(search::Filter::Deadlines) && query.trim().is_empty() {
            "No open item has a deadline".to_string()
        } else if search.filter == Some(search::Filter::Reminders) && query.trim().is_empty() {
            "No open item has a reminder".to_string()
        } else if query.trim().is_empty() {
            "No items yet".to_string()
        } else if search.kind == search::Kind::Notes {
            format!("No notes match \"{query}\"")
        } else {
            format!("No items match \"{query}\"")
        };
        frame.render_widget(Line::from(message).dim(), results);
        search.preview_height.set(0);
        return;
    }
    if let (Some(area), Some(hit)) = (preview_area, hits.get(search.selected)) {
        draw_search_preview(frame, app, search, hit, area);
    }

    // The date goes beside each result, or above it when there isn't room.
    let width = results.width as usize;
    let date_width = 11;
    let stacked = width < date_width + 20;
    let text_width = if stacked { width } else { width - date_width };
    let indent = if stacked { 0 } else { date_width };
    let match_style = Style::new().fg(Color::Yellow).bold();
    let items: Vec<ListItem> = hits
        .iter()
        .map(|hit| {
            let item = &app.store.items(hit.day)[hit.index];
            let date = if hit.day == app.today { "Today".to_string() } else { hit.day.format("%a %-d %b").to_string() };
            let date_style = if hit.day == app.today { Style::new().fg(Color::Green) } else { Style::new().fg(Color::DarkGray) };
            // Deadlines and reminders say when, as the date is the day it's on.
            let when = match search.filter {
                // Coloured as on the list: red once passed, bold once due.
                Some(search::Filter::Deadlines) => item.deadline.map(|deadline| {
                    let text = format!("{DEADLINE_MARKER}{}", deadline.describe(app.today, app.settings.twelve_hour));
                    let style = if deadline.is_overdue(app.today, app.now) {
                        Style::new().fg(Color::Red).add_modifier(Modifier::BOLD)
                    } else if deadline.date == app.today {
                        Style::new().fg(Color::Yellow)
                    } else {
                        Style::new().fg(Color::Cyan)
                    };
                    Span::styled(text, style)
                }),
                Some(search::Filter::Reminders) => item.reminder_due_at(hit.day).map(|at| {
                    let time = Some(at.time()).filter(|time| *time != chrono::NaiveTime::MIN);
                    let when = Deadline { date: at.date(), time }.describe(app.today, app.settings.twelve_hour);
                    let style = if at <= app.today.and_time(app.now) {
                        Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                    } else {
                        Style::new().fg(Color::Cyan)
                    };
                    Span::styled(format!(" {REMINDER_MARKER} {when}"), style)
                }),
                _ => None,
            };
            let text_width = text_width.saturating_sub(when.as_ref().map_or(0, Span::width)).max(1);
            let mut text = highlighted(&item.text, &hit.text_matches, text_width, item_style(item), match_style);
            text.extend(when);
            let mut lines = Vec::new();
            if stacked {
                lines.push(Line::from(Span::styled(date, date_style)));
                lines.push(Line::from(text));
            } else {
                text.insert(0, Span::styled(format!("{date:<date_width$}"), date_style));
                lines.push(Line::from(text));
            }
            if let Some((note, matches)) = &hit.note {
                let mut spans = vec![Span::raw(" ".repeat(indent)), "≡ ".dim()];
                spans.extend(highlighted(note, matches, text_width.saturating_sub(2), Style::new().dim(), match_style));
                lines.push(Line::from(spans));
            }
            ListItem::new(lines)
        })
        .collect();
    let mut state = ListState::default().with_offset(search.offset.get()).with_selected(Some(search.selected));
    let list = List::new(items).highlight_style(Style::new().bg(SELECTED_BG));
    frame.render_stateful_widget(list, results, &mut state);
    search.offset.set(state.offset());
}

/// The selected result's item beside the results: its text, day and
/// deadline, then its notes formatted as in the viewer, scrolled with
/// Ctrl+D/U.
fn draw_search_preview(frame: &mut Frame, app: &App, search: &Search, hit: &Hit, area: Rect) {
    let item = &app.store.items(hit.day)[hit.index];
    let width = area.width as usize;
    let mut head: Vec<Line> = wrap(&item.text, width, 3).into_iter().map(|part| Line::from(part.bold())).collect();
    let mut about = vec![if hit.day == app.today { "Today".green() } else { hit.day.format("%a %-d %b %Y").to_string().dark_gray() }];
    if item.done {
        about.push(" · done".dark_gray());
    }
    if let Some(deadline) = item.deadline {
        about.push(format!(" · ◷ {}", deadline.describe(app.today, app.settings.twelve_hour)).cyan());
    } else if item.pinned {
        about.push(" · ⚲ pinned".cyan());
    }
    head.push(Line::from(about));
    head.push(Line::from("─".repeat(width)).dark_gray());
    let [head_area, notes_area] =
        Layout::vertical([Constraint::Length(head.len() as u16), Constraint::Fill(1)]).areas(area);
    frame.render_widget(Paragraph::new(head), head_area);

    let notes = if item.notes.trim().is_empty() {
        vec![Line::from("No notes").dark_gray()]
    } else {
        markdown::render(&item.notes, width)
    };
    let height = notes_area.height as usize;
    search.preview_height.set(height);
    search.preview_total.set(notes.len());
    let scroll = search.preview_scroll.min(notes.len().saturating_sub(height));
    frame.render_widget(Paragraph::new(notes).scroll((scroll as u16, 0)), notes_area);
}

/// `text` as spans no wider than `width`, with the graphemes at `matches` in
/// `matched` style. If the first match wouldn't be visible, the start is cut
/// (with "…") so it is.
fn highlighted(text: &str, matches: &[usize], width: usize, base: Style, matched: Style) -> Vec<Span<'static>> {
    let graphemes: Vec<&str> = text.graphemes(true).collect();
    let before_first: usize = matches.first().map_or(0, |&first| graphemes[..first.min(graphemes.len())].iter().map(|g| g.width()).sum());
    let mut start = 0;
    let mut spans = Vec::new();
    let mut used = 0;
    if before_first + 1 > width.saturating_sub(1) && width > 4 {
        // Keep a little context before the first match.
        let mut skipped = 0;
        while start < graphemes.len() && before_first - skipped > width / 3 {
            skipped += graphemes[start].width();
            start += 1;
        }
        spans.push(Span::styled("…", base));
        used = 1;
    }
    let mut run = String::new();
    let mut run_matched = false;
    for (i, grapheme) in graphemes.iter().enumerate().skip(start) {
        let w = grapheme.width();
        let rest: usize = graphemes[i..].iter().map(|g| g.width()).sum();
        if used + rest > width && used + w + 1 > width {
            if !run.is_empty() {
                spans.push(Span::styled(std::mem::take(&mut run), if run_matched { matched.patch(base) } else { base }));
            }
            spans.push(Span::styled("…", base));
            return spans;
        }
        let is_match = matches.binary_search(&i).is_ok();
        if is_match != run_matched && !run.is_empty() {
            spans.push(Span::styled(std::mem::take(&mut run), if run_matched { matched.patch(base) } else { base }));
        }
        run_matched = is_match;
        run.push_str(grapheme);
        used += w;
    }
    if !run.is_empty() {
        spans.push(Span::styled(run, if run_matched { matched.patch(base) } else { base }));
    }
    spans
}

fn draw_options(frame: &mut Frame, app: &App, options: &Options) {
    let screen = frame.area();
    let width = screen.width.saturating_sub(4).min(64);
    let inner_width = (width as usize).saturating_sub(4);

    // Where the cursor goes while typing: (line, column).
    let mut cursor = None;

    // The search, if there is one, stays at the top above the rows that
    // match it, which scroll beneath.
    let mut head: Vec<Line> = Vec::new();
    let mut search_cursor = None;
    if let Some(search) = &options.search {
        let shown = truncate(&search.text, inner_width.saturating_sub(10));
        if options.searching {
            search_cursor = Some(8 + search.text[..search.cursor].width());
        }
        head.push(Line::from(vec!["Search: ".dim(), shown.into()]));
        head.push(Line::from("─".repeat(inner_width)).dark_gray());
        if options.visible().is_empty() {
            head.push(Line::from(format!("No options match \"{}\"", search.text)).dim());
        }
    }
    let mut lines: Vec<Line> = Vec::new();

    // A heading per section, then each option with its description wrapped
    // underneath.
    let mut selected_lines = 0..0;
    let mut section = "";
    for (i, toggle) in TOGGLES.iter().enumerate() {
        if !options.matches(i) {
            continue;
        }
        if toggle.section != section {
            if !lines.is_empty() {
                lines.push(Line::default());
            }
            lines.push(Line::from(toggle.section.bold()));
            section = toggle.section;
        }
        let start = lines.len();
        let on = (toggle.get)(&app.settings);
        let check = if on { "[x] ".green().bold() } else { "[ ] ".into() };
        let text_width = inner_width.saturating_sub(4).max(1);
        for (n, part) in wrap(toggle.label, text_width, usize::MAX).into_iter().enumerate() {
            let lead = if n == 0 { check.clone() } else { "    ".into() };
            let mut row = Line::from(vec![lead, part.into()]);
            if i == options.selected {
                row = row.bg(SELECTED_BG);
            }
            lines.push(row);
        }
        for part in wrap(toggle.description, text_width, usize::MAX) {
            lines.push(Line::from(format!("    {part}")).dim());
        }
        if i == options.selected {
            selected_lines = start..lines.len();
        }
    }

    // The rows with a value to type: the notes editor, and the folder with a
    // way to move it.
    let editor = if options.editor.is_empty() { "Built-in" } else { options.editor.as_str() };
    let keys = options.keys.as_deref().unwrap_or("unavailable");
    let mut text_rows = vec![
        (EDITOR_ROW, "Notes", "Notes editor: ", editor, EDITOR_ABOUT, "Command:"),
        (KEYS_ROW, "Keys", "Keybindings: ", keys, KEYS_ABOUT, ""),
    ];
    if let Some(folder) = &options.folder {
        text_rows.push((FOLDER_ROW, "Data", "Todoro folder: ", folder.as_str(), FOLDER_ABOUT, "Move to:"));
    }
    for (index, section, label, value, about, prompt) in text_rows {
        if !options.matches(index) {
            continue;
        }
        let text_width = inner_width.saturating_sub(2).max(1);
        if !lines.is_empty() {
            lines.push(Line::default());
        }
        lines.push(Line::from(section.bold()));
        let start = lines.len();
        let room = text_width.saturating_sub(label.width());
        let mut row = Line::from(vec![label.into(), truncate(value, room).cyan()]);
        let selected = options.selected == index;
        if selected {
            row = row.bg(SELECTED_BG);
        }
        lines.push(row);
        lines.extend(wrap(about, text_width, usize::MAX).into_iter().map(|part| Line::from(format!("  {part}")).dim()));
        if selected && let Some(input) = &options.editing {
            lines.push(Line::from(format!("  {prompt}")));
            let shown = truncate(&input.text, text_width.saturating_sub(2));
            cursor = Some((lines.len(), 4 + input.text[..input.cursor].width()));
            lines.push(Line::from(vec!["  › ".cyan().bold(), shown.into()]));
        }
        if selected && let Some((message, ok)) = &options.message {
            let style = if *ok { Style::new().fg(Color::Green) } else { Style::new().fg(Color::Red) };
            for part in wrap(message, text_width, usize::MAX) {
                lines.push(Line::from(Span::styled(format!("  {part}"), style)));
            }
        }
        if selected {
            selected_lines = start..lines.len();
        }
    }

    // Last, the ways to delete things, each confirmed by typing a word.
    let text_width = inner_width.saturating_sub(2).max(1);
    let first = options.first_clear_row();
    if (first..first + options.clears().len()).any(|row| options.matches(row)) {
        if !lines.is_empty() {
            lines.push(Line::default());
        }
        lines.push(Line::from("Delete".bold().red()));
    }
    let workspace = options.workspace.as_deref();
    for (n, clear) in options.clears().iter().enumerate() {
        if !options.matches(first + n) {
            continue;
        }
        let selected = options.selected == first + n;
        let start = lines.len();
        for (i, part) in wrap(&clear.label(workspace), text_width, usize::MAX).into_iter().enumerate() {
            let mut row = Line::from(Span::styled(if i == 0 { part } else { format!("  {part}") }, Style::new().fg(Color::Red)));
            if selected {
                row = row.bg(SELECTED_BG);
            }
            lines.push(row);
        }
        lines.extend(wrap(clear.description(), text_width, usize::MAX).into_iter().map(|part| Line::from(format!("  {part}")).dim()));
        if selected && let Some(input) = &options.editing {
            let ask = format!("This can't be undone. Type {} to confirm:", clear.confirm_word(workspace));
            lines.extend(wrap(&ask, text_width, usize::MAX).into_iter().map(|part| Line::from(format!("  {part}"))));
            let shown = truncate(&input.text, text_width.saturating_sub(2));
            cursor = Some((lines.len(), 4 + input.text[..input.cursor].width()));
            lines.push(Line::from(vec!["  › ".red().bold(), shown.into()]));
        }
        if selected && let Some((message, ok)) = &options.message {
            let style = if *ok { Style::new().fg(Color::Green) } else { Style::new().fg(Color::Red) };
            for part in wrap(message, text_width, usize::MAX) {
                lines.push(Line::from(Span::styled(format!("  {part}"), style)));
            }
        }
        if selected {
            selected_lines = start..lines.len();
        }
    }

    let height = ((head.len() + lines.len()) as u16 + 2).min(screen.height.saturating_sub(2));
    let area = centered(screen, width, height);
    let room = (width as usize).saturating_sub(2);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Color::Blue))
        .title(" Options ".bold())
        .title_bottom(
            Line::from(if options.searching {
                fit_first(&[" type to search · enter keep · esc clear ", " enter keep · esc clear ", " esc "], room)
            } else {
                fit_first(&[" j/k move · space toggle · / search · esc close ", " space toggle · / search ", " / search · esc ", " esc "], room)
            })
                .centered()
                .dim(),
        )
        .padding(Padding::horizontal(1));
    let [top_area, inner] =
        Layout::vertical([Constraint::Length(head.len() as u16), Constraint::Fill(1)]).areas(block.inner(area));
    // Scroll just enough to keep the selected option in view.
    let visible = inner.height as usize;
    let scroll = selected_lines.end.saturating_sub(visible).min(selected_lines.start);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(head), top_area);
    frame.render_widget(Paragraph::new(lines).scroll((scroll as u16, 0)), inner);
    if let Some(col) = search_cursor {
        frame.set_cursor_position(Position::new(top_area.x + (col as u16).min(top_area.width), top_area.y));
    } else if let Some((line, col)) = cursor
        && line >= scroll
        && inner.y + ((line - scroll) as u16) < inner.bottom()
    {
        frame.set_cursor_position(Position::new(inner.x + (col as u16).min(inner.width), inner.y + (line - scroll) as u16));
    }
}

fn draw_changelog(frame: &mut Frame, view: &ChangelogView) {
    let screen = frame.area();
    let area = centered(screen, screen.width.saturating_sub(4).min(76), screen.height.saturating_sub(2));
    let room = area.width.saturating_sub(2) as usize;
    let whats_new = format!(" What's new in todoro {} ", changes::VERSION);
    let welcome = format!(" Welcome to todoro {} ", changes::VERSION);
    let title = if view.since.is_some() {
        fit_first(&[whats_new.as_str(), " What's new ", " New "], room)
    } else if view.limit.is_some() {
        fit_first(&[welcome.as_str(), " Welcome ", " Hi "], room)
    } else {
        fit_first(&[" Changelog ", " News "], room)
    };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Color::Cyan))
        .title(title.bold())
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);

    let width = inner.width as usize;
    let mut lines: Vec<Line> = Vec::new();
    for release in view.releases() {
        if !lines.is_empty() {
            lines.push(Line::default());
        }
        lines.push(Line::from(release.title.clone().bold().cyan()));
        markdown_lines(release.notes, width, &mut lines);
    }
    if view.limit.is_some() {
        lines.push(Line::default());
        let lead = if view.more() { "Older releases are on GitHub" } else { "Every release is also on GitHub" };
        let older = format!("{lead}: {}", changes::RELEASES_URL);
        lines.extend(wrap(&older, width, usize::MAX).into_iter().map(|part| Line::from(part).dim()));
    }

    let height = inner.height as usize;
    view.height.set(height);
    view.total.set(lines.len());
    let max = lines.len().saturating_sub(height);
    let scroll = view.scroll.min(max);

    // Where you are, as vim shows it, in the bottom-right corner, with the
    // hint centred in the room left beside it.
    let position = format!(" {} ", scroll_position(scroll, max));
    let hint_room = room.saturating_sub(2 * (position.width() + 1));
    let hint = fit_first(&[" j/k scroll · esc close ", " esc close "], hint_room);
    let block = block
        .title_bottom(Line::from(hint).centered().dim())
        .title_bottom(Line::from(position).right_aligned().dim());
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(lines).scroll((scroll as u16, 0)), inner);
}

/// How far through something scrollable you are, as vim shows it: "All" if
/// it all fits, "Top" and "Bot" at the ends, and a percentage between.
fn scroll_position(scroll: usize, max: usize) -> String {
    match (scroll, max) {
        (_, 0) => "All".to_string(),
        (0, _) => "Top".to_string(),
        (s, m) if s >= m => "Bot".to_string(),
        (s, m) => format!("{}%", s * 100 / m),
    }
}

/// The little Markdown the changelog uses, wrapped to `width`: `###`
/// headings, `- ` bullets with a hanging indent, paragraphs, and `code`.
fn markdown_lines(markdown: &str, width: usize, lines: &mut Vec<Line<'static>>) {
    let code = Style::new().fg(Color::Yellow);
    for source in markdown.lines() {
        let source = source.trim_end();
        if source.is_empty() {
            if lines.last().is_some_and(|line| line.width() > 0) {
                lines.push(Line::default());
            }
            continue;
        }
        if let Some(heading) = source.strip_prefix("### ") {
            lines.push(Line::from(heading.to_string().bold()));
            continue;
        }
        let (first, rest, text) = match source.strip_prefix("- ") {
            Some(text) => ("• ", "  ", text),
            None => ("", "", source),
        };
        let (plain, marks) = inline_code(text);
        for (n, range) in wrap_ranges(&plain, width.saturating_sub(first.width()).max(1)).into_iter().enumerate() {
            let line = range.start..range.start + plain[range.clone()].trim_end().len();
            let mut spans = vec![Span::raw(if n == 0 { first } else { rest })];
            spans.extend(marked(&plain, line, &marks, Style::new(), code));
            lines.push(Line::from(spans));
        }
    }
}

/// `text` without its backticks, and where the `code` parts are in the result.
fn inline_code(text: &str) -> (String, Vec<Range<usize>>) {
    let mut plain = String::new();
    let mut marks = Vec::new();
    let mut start = None;
    for c in text.chars() {
        if c != '`' {
            plain.push(c);
        } else if let Some(at) = start.take() {
            marks.push(at..plain.len());
        } else {
            start = Some(plain.len());
        }
    }
    (plain, marks)
}

/// The first-run screen: choose the todoro folder and name the first workspace.
pub fn draw_setup(frame: &mut Frame, setup: &Setup) {
    let screen = frame.area();
    let width = screen.width.saturating_sub(4).min(64);
    let text_width = (width as usize).saturating_sub(4).max(1);

    let mut lines: Vec<Line> = Vec::new();
    let intro = "Choose a folder for your todos. Everything goes in it: each workspace is a folder inside, \
                 with its todos and notes. todoro never backs it up for you, so put it somewhere you back up, \
                 or copy it yourself.";
    lines.extend(wrap(intro, text_width, usize::MAX).into_iter().map(Line::from));
    if setup.moving.is_some() {
        lines.push(Line::default());
        let moving = "Your existing todos will move into this first workspace.";
        lines.extend(wrap(moving, text_width, usize::MAX).into_iter().map(|line| Line::from(line).yellow()));
    }
    let mut fields = Vec::new();
    for (label, input, focused) in [
        ("Folder", &setup.folder, !setup.on_name),
        ("First workspace", &setup.name, setup.on_name),
    ] {
        lines.push(Line::default());
        lines.push(Line::from(label.bold()));
        // Scroll a long entry so the cursor stays in view.
        let field = text_width.saturating_sub(2).max(1);
        let before_cursor = input.text[..input.cursor].width();
        let skip = before_cursor.saturating_sub(field.saturating_sub(1));
        let shown: String = input.text.chars().scan(0, |at, c| {
            let start = *at;
            *at += c.width().unwrap_or(0);
            Some((start, c))
        }).filter(|(start, _)| *start >= skip).map(|(_, c)| c).collect();
        let marker = if focused { "› ".cyan().bold() } else { "  ".into() };
        let text = if focused { Span::from(truncate(&shown, field)) } else { Span::from(truncate(&shown, field)).dim() };
        fields.push((lines.len(), focused, before_cursor - skip));
        lines.push(Line::from(vec![marker, text]));
    }
    if let Some(error) = &setup.error {
        lines.push(Line::default());
        lines.extend(wrap(error, text_width, usize::MAX).into_iter().map(|line| Line::from(line).red()));
    }

    let height = (lines.len() as u16 + 2).min(screen.height);
    let area = centered(screen, width, height);
    let room = (width as usize).saturating_sub(2);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Color::Cyan))
        .title(fit_first(&[" Welcome to todoro ", " todoro "], room).bold())
        .title_bottom(
            Line::from(fit_first(&[" tab switch · enter continue · esc quit ", " enter continue ", " enter "], room))
                .centered()
                .dim(),
        )
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    frame.render_widget(Clear, screen);
    frame.render_widget(Paragraph::new(lines).block(block), area);
    if let Some(&(row, _, col)) = fields.iter().find(|(_, focused, _)| *focused) {
        let y = inner.y + row as u16;
        if y < inner.bottom() {
            frame.set_cursor_position(Position::new(inner.x + 2 + col as u16, y));
        }
    }
}

fn draw_workspaces(frame: &mut Frame, app: &App, picker: &Picker) {
    let names = app.workspaces.as_ref().and_then(|folder| folder.list().ok()).unwrap_or_default();
    let current = app.workspace.as_deref().unwrap_or_default();
    let screen = frame.area();
    let width = screen.width.saturating_sub(4).min(50);
    let text_width = (width as usize).saturating_sub(4).max(1);

    let mut lines: Vec<Line> = Vec::new();
    for (i, name) in names.iter().enumerate() {
        let marker = if name == current { "● ".cyan() } else { "  ".into() };
        let mut line = Line::from(vec![marker, truncate(name, text_width.saturating_sub(2)).into()]);
        if i == picker.selected && picker.adding.is_none() && picker.deleting.is_none() {
            line = line.bg(SELECTED_BG).bold();
        }
        lines.push(line);
    }
    // Where the cursor goes, if a name is being typed: (line, column).
    let mut cursor = None;
    let mut typing = |lines: &mut Vec<Line>, prompt: &str, input: &crate::input::LineInput| {
        lines.push(Line::default());
        lines.extend(wrap(prompt, text_width, usize::MAX).into_iter().map(Line::from));
        cursor = Some((lines.len(), 2 + input.text[..input.cursor].width()));
        lines.push(Line::from(vec!["› ".cyan().bold(), truncate(&input.text, text_width.saturating_sub(2)).into()]));
    };
    if let Some(input) = &picker.adding {
        typing(&mut lines, "Name the new workspace:", input);
    }
    if let Some((name, input)) = &picker.deleting {
        let prompt = format!("This deletes {name} and all its todos and notes for good. Type its name to confirm:");
        typing(&mut lines, &prompt, input);
    }
    if let Some(error) = &picker.error {
        lines.push(Line::default());
        lines.extend(wrap(error, text_width, usize::MAX).into_iter().map(|line| Line::from(line).red()));
    }

    let height = (lines.len() as u16 + 2).min(screen.height.saturating_sub(2));
    let area = centered(screen, width, height);
    let room = (width as usize).saturating_sub(2);
    let (title, border) = if picker.deleting.is_some() {
        (fit_first(&[" Delete workspace? ", " Delete? "], room), Color::Red)
    } else {
        (fit_first(&[" Workspaces ", " Spaces "], room), Color::Cyan)
    };
    let hint = if picker.adding.is_some() || picker.deleting.is_some() {
        fit_first(&[" enter confirm · esc back ", " esc back "], room)
    } else {
        fit_first(&[" ↵ open · a new · d delete · esc close ", " ↵ open · esc ", " esc "], room)
    };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(border))
        .title(title.bold())
        .title_bottom(Line::from(hint).centered().dim())
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    // Keep the selected workspace (or the name being typed) in view.
    let focus = cursor.map_or(picker.selected, |(line, _)| line);
    let scroll = (focus + 1).saturating_sub(inner.height as usize);
    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(lines).block(block).scroll((scroll as u16, 0)), area);
    if let Some((line, col)) = cursor {
        let y = inner.y + (line - scroll) as u16;
        if y < inner.bottom() {
            frame.set_cursor_position(Position::new(inner.x + (col as u16).min(inner.width), y));
        }
    }
}

fn draw_help(frame: &mut Frame, help: &Help) {
    let screen = frame.area();
    let area = centered(screen, screen.width.saturating_sub(4).min(72), screen.height.saturating_sub(2));
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Color::Magenta))
        .title(" Keybindings ".bold())
        .title_bottom(
            Line::from(fit_first(&[" ↑/↓ scroll · esc close ", " esc close "], area.width.saturating_sub(2) as usize))
                .centered()
                .dim(),
        )
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);

    let [search, rule, results] =
        Layout::vertical([Constraint::Length(1), Constraint::Length(1), Constraint::Fill(1)]).areas(inner);
    let prompt = "Search: ";
    let query = if help.query.is_empty() {
        let room = (search.width as usize).saturating_sub(prompt.width());
        fit_first(&["type a key like x, or a word like undo", "a key or a word", "key or word"], room).dark_gray()
    } else {
        help.query.as_str().into()
    };
    frame.render_widget(Line::from(vec![prompt.dim(), query]), search);
    frame.render_widget("─".repeat(rule.width as usize).dark_gray(), rule);
    let typed = (prompt.chars().count() + help.query.chars().count()) as u16;
    frame.set_cursor_position(Position::new(search.x + typed.min(search.width), search.y));

    let matches = help.matches();
    if matches.is_empty() {
        frame.render_widget(Line::from(format!("No keys match \"{}\"", help.query)).dim(), results);
        return;
    }

    // Line the descriptions up in one column, sized for every key so it
    // doesn't shift as you search, and wrap them. If that leaves too little
    // room, put each description under its key instead.
    let keys_width = help.sections.iter().flat_map(|s| &s.bindings).map(|row| row.keys.width()).max().unwrap_or(0);
    let width = results.width as usize;
    let column = width.saturating_sub(keys_width + 4);
    let stacked = column < 16;
    let mut lines: Vec<Line> = Vec::new();
    // Before searching, where to change the keys.
    if help.query.is_empty() {
        lines.extend(wrap(&help.tip, width.max(1), usize::MAX).into_iter().map(|part| Line::from(part).dim()));
    }
    for section in &matches {
        if !lines.is_empty() {
            lines.push(Line::default());
        }
        lines.push(Line::from(section.title.clone().bold()));
        for row in &section.bindings {
            let keys = row.keys.as_str();
            let (indent, room) = if stacked { (4, width.saturating_sub(4).max(1)) } else { (keys_width + 4, column) };
            if stacked {
                lines.push(Line::from(format!("  {keys}").yellow()));
            }
            for (n, part) in wrap(&row.action, room, usize::MAX).into_iter().enumerate() {
                let lead = if stacked || n > 0 { " ".repeat(indent) } else { format!("  {keys:<keys_width$}  ") };
                lines.push(Line::from(vec![lead.yellow(), part.into()]));
            }
            // The command's name for keys.toml, dimmed, after what it does,
            // or under it without room.
            if let Some(name) = row.name {
                let last = lines.last_mut().expect("wrap gives a row at least one line");
                if last.width() + 2 + name.width() <= width {
                    last.push_span(format!("  {name}").dark_gray());
                } else {
                    lines.push(Line::from(vec![" ".repeat(indent).into(), name.dark_gray()]));
                }
            }
        }
    }

    let height = results.height as usize;
    help.height.set(height);
    help.total.set(lines.len());
    let scroll = help.scroll.min(lines.len().saturating_sub(height));
    frame.render_widget(Paragraph::new(lines).scroll((scroll as u16, 0)), results);
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let [area] = Layout::horizontal([Constraint::Length(width)]).flex(Flex::Center).areas(area);
    let [area] = Layout::vertical([Constraint::Length(height)]).flex(Flex::Center).areas(area);
    area
}

#[cfg(test)]
mod tests {
    use insta::assert_snapshot;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::KeyCode;
    use ratatui::Terminal;

    use std::path::PathBuf;

    use super::*;
    use crate::test_util::{app_with, press, today, type_str};

    fn render(app: &App) -> Terminal<TestBackend> {
        render_sized(app, 60, 10)
    }

    fn render_sized(app: &App, width: u16, height: u16) -> Terminal<TestBackend> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| draw(frame, app)).unwrap();
        terminal
    }

    #[test]
    fn today_with_items() {
        let (mut app, _dir) = app_with(&["Buy milk", "Write report", "Call mom"]);
        type_str(&mut app, "j");
        assert_snapshot!(render(&app).backend());
    }

    #[test]
    fn space_menu() {
        let (mut app, _dir) = app_with(&["Buy milk", "Write report", "Call mom"]);
        type_str(&mut app, " ");
        assert_snapshot!(render_sized(&app, 70, 12).backend());
    }

    #[test]
    fn space_menu_narrowed_to_a_group() {
        let (mut app, _dir) = app_with(&["Buy milk", "Write report", "Call mom"]);
        type_str(&mut app, " s");
        assert_snapshot!(render_sized(&app, 70, 12).backend());
    }

    #[test]
    fn tag_suggestions_while_typing() {
        let (mut app, _dir) = app_with(&["Read the book #later", "Plan the #launch party", "Tidy up #later"]);
        type_str(&mut app, "GaCall the bank #la");
        press(&mut app, KeyCode::Down);
        let mut terminal = render_sized(&app, 60, 12);
        assert_snapshot!(terminal.backend());
        // The cursor stays in the item being typed.
        let cursor = terminal.get_cursor_position().unwrap();
        assert_eq!(cursor, Position::new(1 + 3 + 17, 4));
        // After ↓, the second suggestion is the highlighted one, lined up
        // under the # being typed.
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(18, 4)].symbol(), "#");
        assert_eq!(buffer[(18, 7)].symbol(), "#");
        assert_eq!(buffer[(18, 7)].bg, SELECTED_BG);
        assert_ne!(buffer[(18, 6)].bg, SELECTED_BG);
        assert_snapshot!("tag_suggestions_at_30_columns", render_sized(&app, 30, 12).backend());
    }

    /// A list with sub-items: "Plan the trip" with three, one done and
    /// one with its own, folded; and "Buy milk".
    fn sub_items_app() -> (App, tempfile::TempDir) {
        let (mut app, dir) = app_with(&["Plan the trip", "Buy milk"]);
        type_str(&mut app, "oBook flights");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "ACompare hotels near the old town and the station");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "Pack");
        press(&mut app, KeyCode::Esc);
        type_str(&mut app, "kocheck reviews");
        press(&mut app, KeyCode::Esc);
        type_str(&mut app, "kzc");
        type_str(&mut app, "ggjx");
        (app, dir)
    }

    #[test]
    fn sub_items_on_the_list() {
        let (app, _dir) = sub_items_app();
        assert_snapshot!(render_sized(&app, 50, 10).backend());
        assert_snapshot!("sub_items_at_30_columns", render_sized(&app, 30, 12).backend());
    }

    #[test]
    fn viewing_an_item_with_all_its_sub_items() {
        let (mut app, _dir) = sub_items_app();
        let notes = [
            ("Plan the trip", "Lisbon, **mid-May**."),
            ("Book flights", "Window seat."),
            ("check reviews", "- breakfast\n- quiet rooms"),
        ];
        for (text, note) in notes {
            let index = app.store.items(app.day).iter().position(|item| item.text == text).unwrap();
            app.store.set_notes(app.day, index, note.into()).unwrap();
        }
        type_str(&mut app, "gg V");
        assert_snapshot!(render_sized(&app, 50, 24).backend());
        assert_snapshot!("viewing_all_at_30_columns", render_sized(&app, 30, 24).backend());
    }

    #[test]
    fn viewing_the_whole_day() {
        let (mut app, _dir) = sub_items_app();
        type_str(&mut app, "GaCall the bank");
        press(&mut app, KeyCode::Esc);
        let notes = [("Plan the trip", "Lisbon, **mid-May**."), ("check reviews", "- breakfast"), ("Call the bank", "## Ask\nAbout the card.")];
        for (text, note) in notes {
            let index = app.store.items(app.day).iter().position(|item| item.text == text).unwrap();
            app.store.set_notes(app.day, index, note.into()).unwrap();
        }
        type_str(&mut app, " e");
        assert_snapshot!(render_sized(&app, 50, 30).backend());
    }

    #[test]
    fn viewing_an_empty_day() {
        let (mut app, _dir) = app_with(&[]);
        type_str(&mut app, " e");
        assert_snapshot!(render_sized(&app, 40, 8).backend());
    }

    #[test]
    fn deleting_an_item_says_its_sub_items_go_too() {
        let (mut app, _dir) = sub_items_app();
        type_str(&mut app, "ggd");
        assert_snapshot!(render_sized(&app, 50, 12).backend());
    }

    #[test]
    fn typing_a_sub_item() {
        let (mut app, _dir) = sub_items_app();
        type_str(&mut app, "Gowhole milk");
        let mut terminal = render_sized(&app, 50, 10);
        assert_snapshot!(terminal.backend());
        // The cursor is after the text, indented under its parent.
        let cursor = terminal.get_cursor_position().unwrap();
        let row: String = (0..50).map(|x| terminal.backend().buffer()[(x, cursor.y)].symbol().to_string()).collect();
        assert!(row.contains("↳ whole milk"), "{row}");
        assert_eq!(cursor.x, 1 + 3 + 2 + "whole milk".len() as u16);
    }

    fn items_on(app: &App) -> Vec<String> {
        app.items().iter().map(|item| item.text.clone()).collect()
    }

    /// Today at 10:30, with a standup and a report planned (overlapping),
    /// the gym in the evening, and two items with no time.
    fn planned_app() -> (App, tempfile::TempDir) {
        let (mut app, dir) = app_with(&["Standup", "Write the quarterly report", "Buy milk", "Call mum", "Gym"]);
        app.now = chrono::NaiveTime::from_hms_opt(10, 30, 0).unwrap();
        let at = |h: u32, m: u32| chrono::NaiveTime::from_hms_opt(h, m, 0).unwrap();
        for (i, (start, end)) in [(0, (at(9, 0), at(9, 30))), (1, (at(9, 15), at(11, 0))), (4, (at(18, 0), at(19, 0)))] {
            app.store.set_time(today(), i, Some(TimeSlot { date: today(), start, end })).unwrap();
        }
        (app, dir)
    }

    #[test]
    fn timeline_view() {
        let (mut app, _dir) = planned_app();
        type_str(&mut app, " t");
        assert_snapshot!(render_sized(&app, 70, 22).backend());
    }

    #[test]
    fn plan_view_typing_a_start() {
        let (mut app, _dir) = planned_app();
        type_str(&mut app, " pjjj");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "2pm");
        let mut terminal = render_sized(&app, 100, 22);
        assert_snapshot!(terminal.backend());
        // The cursor is after the time typed, on the bottom line.
        assert_eq!(terminal.get_cursor_position().unwrap(), Position::new(("Start for Call mum (↵ for now, 10:30): ".width() + 3) as u16, 21));
    }

    #[test]
    fn the_time_now_is_a_line_right_across_the_timeline_and_its_blocks() {
        let (mut app, _dir) = planned_app();
        type_str(&mut app, " p");
        let terminal = render_sized(&app, 100, 22);
        let buffer = terminal.backend().buffer();
        let row = (0..22).find(|&y| (0..100).map(|x| buffer[(x, y)].symbol()).collect::<String>().contains("10:30 ─")).unwrap();
        // The timeline is the right half; past its label, every cell is red,
        // including across the report's block, which keeps its text.
        let line: Vec<&ratatui::buffer::Cell> = (50 + 1 + 6..99).map(|x| &buffer[(x, row)]).collect();
        assert!(line.iter().all(|cell| cell.fg == Color::Red), "the whole row is red");
        let text: String = line.iter().map(|cell| cell.symbol()).collect();
        assert!(text.contains("09:15–11:00") && text.starts_with('─') && text.ends_with('─'), "{text}");
    }

    #[test]
    fn plan_view_moving_through_the_timeline() {
        let (mut app, _dir) = planned_app();
        type_str(&mut app, " pLl");
        let terminal = render_sized(&app, 100, 22);
        assert_snapshot!(terminal.backend());
        // From Standup, l goes to the report in the column beside it: its
        // block is the highlighted one, and Standup's isn't.
        let buffer = terminal.backend().buffer();
        let row = (0..22).find(|&y| (50..100).map(|x| buffer[(x, y)].symbol()).collect::<String>().contains("Standup")).unwrap();
        let bg_at = |text: char| (57..100).find(|&x| buffer[(x, row)].symbol() == text.to_string()).map(|x| buffer[(x, row)].bg);
        assert_eq!(bg_at('W'), Some(BLOCK_SELECTED_BG));
        assert_eq!(bg_at('S'), Some(BLOCK_BG));
        assert_eq!(items_on(&app)[app.selected], "Write the quarterly report");
        // The list side's border is dim, as keys go to the timeline.
        assert_eq!(buffer[(0, 0)].fg, Color::DarkGray);
    }

    #[test]
    fn too_many_all_day_items_for_two_lines_are_counted() {
        let (mut app, _dir) = app_with(&["milk", "eggs", "bread", "cheese", "apples", "pears", "rice", "tea"]);
        type_str(&mut app, " t");
        let screen = render_sized(&app, 30, 10).backend().to_string();
        let lines: Vec<&str> = screen.lines().collect();
        assert!(lines[1].contains("All day  milk · eggs"), "{screen}");
        assert!(lines[2].contains("more") && !lines[3].contains("tea"), "{screen}");
    }

    #[test]
    fn blocks_alternate_shades_and_take_their_prioritys_colour() {
        let (mut app, _dir) = app_with(&["First", "Second", "Urgent"]);
        let at = |h: u32| chrono::NaiveTime::from_hms_opt(h, 0, 0).unwrap();
        for (i, start, end) in [(0, at(13), at(14)), (1, at(14), at(15)), (2, at(16), at(17))] {
            app.store.set_time(today(), i, Some(TimeSlot { date: today(), start, end })).unwrap();
        }
        app.store.cycle_priority(today(), 2).unwrap();
        app.now = at(12);
        type_str(&mut app, " t");
        // Tall enough for two rows an hour.
        let terminal = render_sized(&app, 60, 40);
        let buffer = terminal.backend().buffer();
        let row_of = |text: &str| (0..40).find(|&y| (0..60).map(|x| buffer[(x, y)].symbol()).collect::<String>().contains(text)).unwrap();
        let (first, second, urgent) = (row_of("First"), row_of("Second"), row_of("Urgent"));
        assert!(second >= first + 2, "a row or more each, one after the other");
        // Each starts with the bar, and one after another they're the two
        // shades of the checkerboard, so it's clear where one ends.
        let edge = (0..60).find(|&x| buffer[(x, first)].symbol() == BLOCK_EDGE).unwrap();
        assert_eq!(buffer[(edge, second)].symbol(), BLOCK_EDGE);
        assert_eq!(buffer[(edge + 2, first)].bg, BLOCK_BG);
        assert_eq!(buffer[(edge + 2, second)].bg, BLOCK_ALT_BG);
        assert_eq!(buffer[(edge + 2, second - 1)].bg, BLOCK_BG, "the first runs to just above the second");
        // No lines across: the rows between hours are plain.
        assert!(!(0..60).any(|x| buffer[(x, first + 1)].symbol() == "┈"));
        // The High one is red, in the shade its place gives it.
        let (red, bar) = block_colours(Some(Priority::High), false);
        assert_eq!(buffer[(edge + 2, urgent)].bg, red);
        assert_eq!(buffer[(edge, urgent)].fg, bar);
    }

    #[test]
    fn priority_blocks_have_a_checkerboard_of_their_own() {
        let (mut app, _dir) = app_with(&["First", "Second", "Third"]);
        let at = |h: u32| chrono::NaiveTime::from_hms_opt(h, 0, 0).unwrap();
        for (i, start, end) in [(0, at(13), at(14)), (1, at(14), at(15)), (2, at(15), at(16))] {
            app.store.set_time(today(), i, Some(TimeSlot { date: today(), start, end })).unwrap();
            app.store.cycle_priority(today(), i).unwrap();
        }
        app.now = at(12);
        type_str(&mut app, " t");
        let terminal = render_sized(&app, 60, 40);
        let buffer = terminal.backend().buffer();
        let bg_of = |text: &str| {
            let y = (0..40).find(|&y| (0..60).map(|x| buffer[(x, y)].symbol()).collect::<String>().contains(text)).unwrap();
            let x = (0..60).find(|&x| buffer[(x, y)].symbol() == &text[..1]).unwrap();
            buffer[(x, y)].bg
        };
        let (dark, _) = block_colours(Some(Priority::High), false);
        let (light, _) = block_colours(Some(Priority::High), true);
        assert_ne!(dark, light);
        assert_eq!([bg_of("First"), bg_of("Second"), bg_of("Third")], [dark, light, dark]);
    }

    #[test]
    fn adding_an_item_in_the_plan_view_keeps_the_timeline_beside_it() {
        let (mut app, _dir) = planned_app();
        type_str(&mut app, " paDentist");
        assert_snapshot!(render_sized(&app, 100, 14).backend());
    }

    #[test]
    fn the_selected_item_stands_out_in_the_plan_view() {
        let (mut app, _dir) = planned_app();
        type_str(&mut app, " p");
        let terminal = render_sized(&app, 100, 22);
        let buffer = terminal.backend().buffer();
        // Standup, selected, on the list: the same blue as its block.
        let row = (0..22).find(|&y| (0..50).map(|x| buffer[(x, y)].symbol()).collect::<String>().contains("Standup")).unwrap();
        let x = (0..50).find(|&x| buffer[(x, row)].symbol() == "S").unwrap();
        assert_eq!(buffer[(x, row)].bg, BLOCK_SELECTED_BG);
        assert!(buffer[(x, row)].modifier.contains(Modifier::BOLD));
        let block_row = (0..22).find(|&y| (50..100).map(|x| buffer[(x, y)].symbol()).collect::<String>().contains("Standup")).unwrap();
        let bx = (50..100).find(|&x| buffer[(x, block_row)].symbol() == "S").unwrap();
        assert_eq!(buffer[(bx, block_row)].bg, BLOCK_SELECTED_BG);
        // The main list keeps its usual highlight.
        press(&mut app, KeyCode::Esc);
        let terminal = render_sized(&app, 100, 22);
        let buffer = terminal.backend().buffer();
        assert_ne!(buffer[(4, 1)].bg, BLOCK_SELECTED_BG);
    }

    #[test]
    fn plan_view_asking_before_throwing_changes_away() {
        let (mut app, _dir) = planned_app();
        type_str(&mut app, " pcq");
        assert_snapshot!(render_sized(&app, 100, 22).backend());
        assert_snapshot!("discarding_a_plan_at_30_columns", render_sized(&app, 30, 14).backend());
    }

    #[test]
    fn empty_day() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        type_str(&mut app, "l");
        assert_snapshot!(render(&app).backend());
    }

    #[test]
    fn numbers_are_padded_past_nine() {
        let items: Vec<String> = (1..=10).map(|i| format!("item {i}")).collect();
        let items: Vec<&str> = items.iter().map(String::as_str).collect();
        let (mut app, _dir) = app_with(&items);
        type_str(&mut app, "jjjjjjjjj");
        assert_snapshot!(render(&app).backend());
    }

    #[test]
    fn adding_an_item() {
        let (mut app, _dir) = app_with(&["Buy milk", "Call mom"]);
        type_str(&mut app, "aWrite report");
        let mut terminal = render(&app);
        assert_snapshot!(terminal.backend());
        // Border, "2. ", then the 12 typed characters.
        assert_eq!(terminal.get_cursor_position().unwrap(), Position::new(1 + 3 + 12, 2));
    }

    #[test]
    fn editing_with_the_cursor_mid_text() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        type_str(&mut app, "e");
        for _ in 0..4 {
            press(&mut app, KeyCode::Left);
        }
        let mut terminal = render(&app);
        assert_snapshot!(terminal.backend());
        // Border, "1. ", then "Buy ".
        assert_eq!(terminal.get_cursor_position().unwrap(), Position::new(1 + 3 + 4, 1));
    }

    #[test]
    fn items_with_notes_are_marked() {
        let (mut app, _dir) = app_with(&["Buy milk", "Write report"]);
        app.store.set_notes(app.day, 1, "Ask for Q3 numbers".into()).unwrap();
        assert_snapshot!(render(&app).backend());
    }

    #[test]
    fn notes_screen() {
        let (mut app, _dir) = app_with(&["Buy milk", "Write report"]);
        app.store.set_notes(app.day, 1, "Draft intro by Wed\n- ask for Q3 numbers\n- charts".into()).unwrap();
        type_str(&mut app, "j");
        press(&mut app, KeyCode::Enter);
        assert_snapshot!(render(&app).backend());
    }

    #[test]
    fn empty_notes_screen() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        press(&mut app, KeyCode::Enter);
        assert_snapshot!(render(&app).backend());
    }

    #[test]
    fn writing_notes_in_insert_mode() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "ioat milk");
        assert_snapshot!(render(&app).backend());
    }

    #[test]
    fn completed_items_below_a_header() {
        let (mut app, _dir) = app_with(&["Buy milk", "Write report", "Call mom", "Book dentist"]);
        app.store.set_notes(app.day, 1, "notes".into()).unwrap();
        type_str(&mut app, "jxx");
        assert_snapshot!(render(&app).backend());
    }

    #[test]
    fn all_items_completed() {
        let (mut app, _dir) = app_with(&["Buy milk", "Call mom"]);
        type_str(&mut app, "xx");
        assert_snapshot!(render(&app).backend());
    }

    #[test]
    fn editing_a_completed_item() {
        let (mut app, _dir) = app_with(&["Buy milk", "Call mom"]);
        type_str(&mut app, "xje");
        let mut terminal = render(&app);
        assert_snapshot!(terminal.backend());
        // Border, then row 1 (open item) + row 2 (header) puts the item on row 3.
        // The cursor follows "✓ " and "Buy milk".
        assert_eq!(terminal.get_cursor_position().unwrap(), Position::new(1 + 3 + 8, 3));
    }

    #[test]
    fn help_popup() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        type_str(&mut app, "?");
        let mut terminal = render_sized(&app, 80, 24);
        assert_snapshot!(terminal.backend());
        // The 72-wide popup starts at x=4, then border, padding and "Search: ".
        assert_eq!(terminal.get_cursor_position().unwrap(), Position::new(4 + 1 + 1 + 8, 2));
    }

    #[test]
    fn help_popup_scrolled_to_the_end() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        type_str(&mut app, "?");
        render_sized(&app, 80, 24);
        // More than there are lines, so it stops at the end.
        for _ in 0..1000 {
            press(&mut app, KeyCode::Down);
        }
        assert_snapshot!(render_sized(&app, 80, 24).backend());
    }

    #[test]
    fn help_popup_searching_for_a_key() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        type_str(&mut app, "?x");
        assert_snapshot!(render_sized(&app, 80, 16).backend());
    }

    #[test]
    fn help_popup_searching_for_a_word() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        type_str(&mut app, "?delete");
        assert_snapshot!(render_sized(&app, 80, 16).backend());
    }

    #[test]
    fn help_popup_with_no_matches() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        type_str(&mut app, "?zzz");
        assert_snapshot!(render_sized(&app, 80, 16).backend());
    }

    #[test]
    fn help_popup_over_the_notes_screen() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        app.store.set_notes(app.day, 0, "oat milk".into()).unwrap();
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "?undo");
        assert_snapshot!(render_sized(&app, 80, 16).backend());
    }

    #[test]
    fn pinned_items_are_marked() {
        let (mut app, _dir) = app_with(&["Buy milk", "Write report", "Call mom"]);
        app.store.set_notes(app.day, 1, "notes".into()).unwrap();
        type_str(&mut app, "mjmjmx");
        assert_snapshot!(render(&app).backend());
    }

    #[test]
    fn future_day_with_carried_pinned_items() {
        let (mut app, _dir) = app_with(&["Buy milk", "Write report"]);
        app.store.toggle_pinned(app.day, 1).unwrap();
        let tomorrow = app.day.succ_opt().unwrap();
        app.store.insert(tomorrow, 0, "Call mom".into()).unwrap();
        app.store.insert(tomorrow, 1, "Book dentist".into()).unwrap();
        app.store.toggle_done(tomorrow, 1).unwrap();
        type_str(&mut app, "l");
        assert_snapshot!(render(&app).backend());
    }

    #[test]
    fn wrap_keeps_short_text_on_one_line() {
        assert_eq!(wrap("Buy milk", 20, 5), ["Buy milk"]);
        assert_eq!(wrap("", 20, 5), [""]);
    }

    #[test]
    fn wrap_breaks_between_words() {
        assert_eq!(wrap("the quick brown fox jumps", 10, 5), ["the quick", "brown fox", "jumps"]);
        // A word that exactly fills the line stays on it.
        assert_eq!(wrap("abcde fghij", 5, 5), ["abcde", "fghij"]);
    }

    #[test]
    fn wrap_keeps_spaces_as_typed() {
        assert_eq!(wrap("a   b", 10, 5), ["a   b"]);
        // Spaces at a break are dropped from the end of the line they follow.
        assert_eq!(wrap("abc    def", 5, 5), ["abc", "def"]);
    }

    #[test]
    fn wrap_ranges_cover_the_whole_text() {
        for text in ["", "a", "Ring the council about the parking permit", "a  b   ", "日本語 のテキスト", "x".repeat(30).as_str()] {
            for width in 1..12 {
                let ranges = wrap_ranges(text, width);
                assert_eq!(ranges.first().unwrap().start, 0);
                assert_eq!(ranges.last().unwrap().end, text.len());
                assert!(ranges.windows(2).all(|pair| pair[0].end == pair[1].start), "{text:?} at {width}");
                // Apart from spaces left hanging at a break, every line fits.
                assert!(ranges.iter().all(|r| text[r.clone()].trim_end().width() <= width.max(2)), "{text:?} at {width}");
            }
        }
    }

    #[test]
    fn cursor_moves_to_the_next_line_at_a_break() {
        let text = "the quick brown";
        let ranges = wrap_ranges(text, 10);
        assert_eq!(ranges, [0..10, 10..15]);
        assert_eq!(cursor_position(text, &ranges, 0, 10), (0, 0));
        assert_eq!(cursor_position(text, &ranges, 9, 10), (0, 9));
        // Right after "quick ", the start of the next line.
        assert_eq!(cursor_position(text, &ranges, 10, 10), (1, 0));
        assert_eq!(cursor_position(text, &ranges, 15, 10), (1, 5));
    }

    #[test]
    fn cursor_after_hanging_spaces_stays_inside_the_line() {
        let text = "abcdefghi    ";
        let ranges = wrap_ranges(text, 10);
        assert_eq!(cursor_position(text, &ranges, text.len(), 10), (0, 10));
    }

    #[test]
    fn cursor_counts_wide_characters_as_two_columns() {
        let text = "日本語";
        let ranges = wrap_ranges(text, 20);
        assert_eq!(cursor_position(text, &ranges, text.len(), 20), (0, 6));
    }

    #[test]
    fn long_items_wrap_on_the_list() {
        let (mut app, _dir) = app_with(&[
            "Buy milk",
            "Ring the council about the parking permit renewal and ask whether the visitor passes carry over",
            "Call mom",
        ]);
        app.store.toggle_pinned(app.day, 1).unwrap();
        app.store.set_notes(app.day, 1, "notes".into()).unwrap();
        app.store.insert(app.day, 3, "Send the signed contract back to the letting agency".into()).unwrap();
        app.store.toggle_done(app.day, 3).unwrap();
        type_str(&mut app, "j");
        assert_snapshot!(render_sized(&app, 60, 12).backend());
    }

    #[test]
    fn typing_a_long_item_wraps_and_the_cursor_follows() {
        let (mut app, _dir) = app_with(&["Buy milk", "Call mom"]);
        type_str(&mut app, "aRing the council about the parking permit renewal and ask");
        let mut terminal = render(&app);
        assert_snapshot!(terminal.backend());
        // Second line of item 2: border row + item 1 + one line of item 2,
        // then border, the indent and "ask".
        assert_eq!(terminal.get_cursor_position().unwrap(), Position::new(1 + 3 + 3, 3));
    }

    #[test]
    fn typing_past_a_full_line_moves_the_cursor_to_the_next_line() {
        let (mut app, _dir) = app_with(&[]);
        // 55 columns fit after "1. ", so 11 "word "s fill the line exactly
        // and the next character starts a new one.
        type_str(&mut app, &format!("a{}x", "word ".repeat(11)));
        let mut terminal = render(&app);
        assert_eq!(terminal.get_cursor_position().unwrap(), Position::new(1 + 3 + 1, 2));
    }


    #[test]
    fn wrap_splits_words_longer_than_a_line() {
        assert_eq!(wrap("see https://example.com/a/long/path", 10, 5), ["see", "https://ex", "ample.com/", "a/long/pat", "h"]);
    }

    #[test]
    fn wrap_counts_wide_characters_as_two_columns() {
        assert_eq!(wrap("日本語のテキスト", 6, 5), ["日本語", "のテキ", "スト"]);
        assert!(wrap("日本語のテキスト", 5, 5).iter().all(|line| line.width() <= 5));
    }

    #[test]
    fn wrap_ends_with_an_ellipsis_when_out_of_lines() {
        assert_eq!(wrap("one two three four five six", 9, 2), ["one two", "three…"]);
        assert_eq!(wrap("abcdefghij", 5, 1), ["abcd…"]);
        assert!(wrap(&"word ".repeat(100), 12, 3).iter().all(|line| line.width() <= 12));
    }

    #[test]
    fn delete_popup_wraps_long_text() {
        let (mut app, _dir) = app_with(&[
            "Buy milk",
            "Ring the council about the parking permit renewal and ask whether the visitor passes carry over",
        ]);
        type_str(&mut app, "jd");
        assert_snapshot!(render_sized(&app, 60, 14).backend());
    }

    #[test]
    fn delete_popup_cuts_off_very_long_text() {
        let (mut app, _dir) = app_with(&[&"so many words ".repeat(30)]);
        type_str(&mut app, "d");
        assert_snapshot!(render(&app).backend());
    }

    #[test]
    fn delete_popup_for_a_completed_item_is_not_numbered() {
        let (mut app, _dir) = app_with(&["Buy milk", "Call mom"]);
        type_str(&mut app, "xjd");
        assert_snapshot!(render(&app).backend());
    }

    /// Presses each key and redraws after it, like the real event loop, so the
    /// list's scroll position carries from one frame to the next.
    fn press_and_draw(app: &mut App, keys: &str, width: u16, height: u16) -> Terminal<TestBackend> {
        let mut terminal = render_sized(app, width, height);
        for c in keys.chars() {
            press(app, KeyCode::Char(c));
            terminal = render_sized(app, width, height);
        }
        terminal
    }

    fn twenty_items() -> (App, tempfile::TempDir) {
        let items: Vec<String> = (1..=20).map(|i| format!("item {i}")).collect();
        app_with(&items.iter().map(String::as_str).collect::<Vec<_>>())
    }

    #[test]
    fn j_scrolls_down_once_the_cursor_reaches_the_bottom() {
        let (mut app, _dir) = twenty_items();
        let terminal = press_and_draw(&mut app, &"j".repeat(12), 40, 8);
        assert_snapshot!(terminal.backend());
    }

    #[test]
    fn k_moves_up_within_the_view_before_scrolling() {
        let (mut app, _dir) = twenty_items();
        // Down to item 13 (the view is 9-13), then up to item 10: the view stays put.
        let terminal = press_and_draw(&mut app, &format!("{}kkk", "j".repeat(12)), 40, 8);
        assert_snapshot!(terminal.backend());
        assert_eq!(app.list_offset.get(), 8);
    }

    #[test]
    fn k_scrolls_up_once_the_cursor_reaches_the_top() {
        let (mut app, _dir) = twenty_items();
        press_and_draw(&mut app, &format!("{}{}", "j".repeat(12), "k".repeat(6)), 40, 8);
        // Item 7 is the top row.
        assert_eq!(app.selected, 6);
        assert_eq!(app.list_offset.get(), 6);
    }

    #[test]
    fn changing_day_starts_the_list_at_the_top() {
        let (mut app, _dir) = twenty_items();
        press_and_draw(&mut app, &format!("{}lh", "j".repeat(15)), 40, 8);
        assert_eq!(app.selected, 0);
        assert_eq!(app.list_offset.get(), 0);
    }

    #[test]
    fn scrolling_keeps_a_wrapped_item_fully_in_view() {
        let long = "Ring the council about the parking permit renewal and ask whether the visitor passes";
        let (mut app, _dir) = app_with(&["one", "two", "three", "four", long]);
        let terminal = press_and_draw(&mut app, "jjjj", 40, 8);
        assert_snapshot!(terminal.backend());
    }

    #[test]
    fn fit_hints_drops_the_lowest_priority_first() {
        let hints = [("a add", 2), ("d delete", 0), ("x done", 1), ("? help", 3)];
        assert_eq!(fit_hints(&hints, 100), "a add  d delete  x done  ? help");
        assert_eq!(fit_hints(&hints, 25), "a add  x done  ? help");
        assert_eq!(fit_hints(&hints, 14), "a add  ? help");
        assert_eq!(fit_hints(&hints, 6), "? help");
        assert_eq!(fit_hints(&hints, 3), "");
        // Of equal priorities, the rightmost goes first.
        assert_eq!(fit_hints(&[("one", 0), ("two", 0)], 4), "one");
    }

    #[test]
    fn truncate_ends_cut_text_with_an_ellipsis() {
        assert_eq!(truncate("Write report", 20), "Write report");
        assert_eq!(truncate("Write report", 12), "Write report");
        assert_eq!(truncate("Write report", 8), "Write r…");
        assert_eq!(truncate("日本語", 4), "日…");
        assert_eq!(truncate("abc", 1), "…");
        assert_eq!(truncate("abc", 0), "");
    }

    #[test]
    fn fit_first_picks_the_first_option_that_fits() {
        assert_eq!(fit_first(&["long option", "short", "s"], 20), "long option");
        assert_eq!(fit_first(&["long option", "short", "s"], 5), "short");
        assert_eq!(fit_first(&["long option", "short"], 2), "");
    }

    /// The list, notes, help and insert screens of a small app at `width`.
    fn narrow_screens(width: u16) -> String {
        let (mut app, _dir) = app_with(&["Buy milk", "Write the quarterly report for the team"]);
        app.store.set_notes(app.day, 1, "## Q3\n- Ask for Q3 numbers\n- [ ] Book a room".into()).unwrap();
        let mut out = String::new();
        let mut shot = |app: &App, name: &str| {
            out.push_str(&format!("{name}\n{}\n", render_sized(app, width, 12).backend()));
        };
        shot(&app, "list");
        type_str(&mut app, " ");
        shot(&app, "space menu");
        press(&mut app, KeyCode::Esc);
        type_str(&mut app, "a");
        shot(&app, "adding");
        press(&mut app, KeyCode::Esc);
        type_str(&mut app, "j");
        press(&mut app, KeyCode::Enter);
        shot(&app, "notes");
        type_str(&mut app, "?");
        shot(&app, "help");
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Esc);
        type_str(&mut app, " v");
        shot(&app, "viewing notes");
        press(&mut app, KeyCode::Esc);
        type_str(&mut app, " V");
        shot(&app, "viewing all");
        press(&mut app, KeyCode::Esc);
        type_str(&mut app, " e");
        shot(&app, "viewing the day");
        press(&mut app, KeyCode::Esc);
        type_str(&mut app, "@");
        shot(&app, "choosing a deadline");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "17:30");
        shot(&app, "a deadline's time");
        press(&mut app, KeyCode::Enter);
        shot(&app, "with a deadline");
        type_str(&mut app, "r");
        shot(&app, "choosing a reminder");
        type_str(&mut app, "d2");
        shot(&app, "days before");
        press(&mut app, KeyCode::Enter);
        shot(&app, "with a reminder");
        type_str(&mut app, " t");
        shot(&app, "timeline");
        type_str(&mut app, "p");
        shot(&app, "planning");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "9");
        shot(&app, "typing a start");
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Esc);
        type_str(&mut app, "l");
        shot(&app, "empty day");
        type_str(&mut app, " c:snake");
        shot(&app, "typing a command in the calendar");
        press(&mut app, KeyCode::Enter);
        if let Mode::Calendar(calendar) = &mut app.mode {
            calendar.snake.as_mut().unwrap().set_food(NaiveDate::from_ymd_opt(2026, 10, 9).unwrap());
        }
        shot(&app, "snake");
        out
    }

    const RICH_NOTES: &str = "# Plan\n\nSee the **draft** and the `budget.xlsx` sheet, then *ask* [Sam](https://example.com).\n\n## To do\n\n- [x] Book a room\n- [ ] Send the agenda\n  - with the numbers\n1. First\n2. Second\n\n> Keep it short.\n\n```\nlet total = sum(q3);\n```\n\n| Who | When |\n|---|---|\n| Sam | Mon |\n\n---\n\nThe end.";

    #[test]
    fn viewing_notes() {
        let (mut app, _dir) = app_with(&["Plan the offsite"]);
        app.store.set_notes(app.day, 0, RICH_NOTES.into()).unwrap();
        type_str(&mut app, " v");
        assert_snapshot!(render_sized(&app, 60, 40).backend());
    }

    #[test]
    fn viewing_notes_scrolled() {
        let (mut app, _dir) = app_with(&["Plan the offsite"]);
        app.store.set_notes(app.day, 0, RICH_NOTES.into()).unwrap();
        type_str(&mut app, " v");
        render_sized(&app, 60, 16);
        type_str(&mut app, "G");
        assert_snapshot!(render_sized(&app, 60, 16).backend());
    }

    #[test]
    fn viewing_an_item_without_notes() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        type_str(&mut app, " v");
        assert_snapshot!(render_sized(&app, 50, 8).backend());
    }

    /// A list with deadlines: today at 12:00, one due earlier today, one
    /// later, one tomorrow, one later in the year, one done and one pinned.
    fn deadlines_app(twelve_hour: bool) -> (App, tempfile::TempDir) {
        use crate::deadline::Deadline;
        let (mut app, dir) = app_with(&["Overdue", "Due later today", "Tomorrow", "Next month", "Pinned", "Finished"]);
        app.now = chrono::NaiveTime::from_hms_opt(12, 0, 0).unwrap();
        app.settings.twelve_hour = twelve_hour;
        let at = |days: u64, time: Option<(u32, u32)>| Deadline {
            date: today() + chrono::Days::new(days),
            time: time.map(|(h, m)| chrono::NaiveTime::from_hms_opt(h, m, 0).unwrap()),
        };
        for (i, deadline) in [at(0, Some((9, 0))), at(0, Some((19, 25))), at(1, None), at(31, Some((23, 0)))].into_iter().enumerate() {
            app.store.set_deadline(today(), i, Some(deadline)).unwrap();
        }
        app.store.toggle_pinned(today(), 4).unwrap();
        app.store.set_deadline(today(), 5, Some(at(2, None))).unwrap();
        app.store.toggle_done(today(), 5).unwrap();
        (app, dir)
    }

    #[test]
    fn deadlines_on_the_list() {
        let (app, _dir) = deadlines_app(false);
        let terminal = render_sized(&app, 50, 12);
        assert_snapshot!(terminal.backend());
        // Overdue is red and bold, so it still stands out without colour.
        let buffer = terminal.backend().buffer();
        let overdue = (0..50).map(|x| &buffer[(x, 1)]).find(|cell| cell.symbol() == "◷").unwrap();
        assert_eq!(overdue.fg, Color::Red);
        assert!(overdue.modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn overdue_shows_in_the_corner_on_any_day() {
        let (mut app, _dir) = deadlines_app(false);
        let corner = |app: &App, width: u16| render_sized(app, width, 8).backend().to_string().lines().next().unwrap().to_string();
        assert!(corner(&app, 90).contains("⚠ 1 overdue · go"), "{}", corner(&app, 90));
        // Narrower, just the count; narrower still, nothing.
        assert!(corner(&app, 50).contains("⚠ 1 "), "{}", corner(&app, 50));
        assert!(!corner(&app, 30).contains('⚠'), "{}", corner(&app, 30));
        // On other days too.
        type_str(&mut app, "l");
        assert!(corner(&app, 90).contains("⚠ 1 overdue"));
        // And not once it's done.
        type_str(&mut app, "h");
        app.store.toggle_done(today(), 0).unwrap();
        assert!(!corner(&app, 90).contains('⚠'));
    }

    #[test]
    fn nothing_overdue_says_so() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        type_str(&mut app, "go");
        let screen = render_sized(&app, 60, 8).backend().to_string();
        assert!(screen.contains(" Overdue ") && screen.contains("Nothing is overdue"), "{screen}");
    }

    /// Today at 12:00: one item due later today with a reminder still to
    /// come, one with a reminder already due, and one from three days on
    /// that's here because its reminder is due.
    fn reminders_app() -> (App, tempfile::TempDir) {
        use crate::reminder::{Reminder, Unit};
        let (mut app, dir) = app_with(&["Call the bank", "Water plants"]);
        app.now = chrono::NaiveTime::from_hms_opt(12, 0, 0).unwrap();
        app.store.set_reminder(today(), 0, Some(Reminder::Before { amount: 2, unit: Unit::Hours })).unwrap();
        app.store.set_reminder(today(), 1, Some(Reminder::Before { amount: 1, unit: Unit::Days })).unwrap();
        let later = today() + chrono::Days::new(3);
        app.store.insert(later, 0, "Buy a birthday gift".into()).unwrap();
        app.store.set_reminder(later, 0, Some(Reminder::Before { amount: 5, unit: Unit::Days })).unwrap();
        (app, dir)
    }

    #[test]
    fn reminders_on_the_list() {
        let (app, _dir) = reminders_app();
        let terminal = render_sized(&app, 50, 8);
        assert_snapshot!(terminal.backend());
        // A due reminder is bold, so it still stands out without colour.
        let buffer = terminal.backend().buffer();
        let due = (0..50).map(|x| &buffer[(x, 1)]).find(|cell| cell.symbol() == "⚑").unwrap();
        assert!(due.modifier.contains(Modifier::BOLD));
        let to_come = (0..50).map(|x| &buffer[(x, 2)]).find(|cell| cell.symbol() == "⚑").unwrap();
        assert!(!to_come.modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn reminders_due_today_show_in_the_corner_on_other_days() {
        let (mut app, _dir) = reminders_app();
        let corner = |app: &App, width: u16| render_sized(app, width, 8).backend().to_string().lines().next().unwrap().to_string();
        // Not on today, where they're on the list.
        assert!(!corner(&app, 90).contains('⚑'), "{}", corner(&app, 90));
        type_str(&mut app, "h");
        assert!(corner(&app, 90).contains("⚑ 1 today · t"), "{}", corner(&app, 90));
        assert!(corner(&app, 50).contains("⚑ 1 "), "{}", corner(&app, 50));
        assert!(!corner(&app, 30).contains('⚑'), "{}", corner(&app, 30));
    }

    #[test]
    fn listing_deadlines() {
        let (mut app, _dir) = deadlines_app(false);
        type_str(&mut app, " sd");
        assert_snapshot!(render_sized(&app, 60, 12).backend());
    }

    #[test]
    fn listing_reminders() {
        let (mut app, _dir) = reminders_app();
        type_str(&mut app, " sr");
        assert_snapshot!(render_sized(&app, 60, 10).backend());
    }

    #[test]
    fn choosing_a_reminder() {
        let (mut app, _dir) = reminders_app();
        type_str(&mut app, "r");
        assert_snapshot!(render_sized(&app, 60, 16).backend());
    }

    #[test]
    fn typing_how_many_before() {
        let (mut app, _dir) = reminders_app();
        type_str(&mut app, "jjrdx");
        press(&mut app, KeyCode::Enter);
        let mut terminal = render_sized(&app, 60, 16);
        assert_snapshot!(terminal.backend());
        let cursor = terminal.get_cursor_position().unwrap();
        let row: String = (0..60).map(|x| terminal.backend().buffer()[(x, cursor.y)].symbol().to_string()).collect();
        assert!(row.contains("› x"), "{row}");
    }

    #[test]
    fn choosing_a_day_to_be_reminded() {
        let (mut app, _dir) = reminders_app();
        type_str(&mut app, "rc");
        let screen = render_sized(&app, 80, 24).backend().to_string();
        assert!(screen.contains("Remind me of Buy a birthday gift"), "{screen}");
        assert!(screen.contains("REMIND"), "{screen}");
        press(&mut app, KeyCode::Enter);
        let screen = render_sized(&app, 80, 24).backend().to_string();
        assert!(screen.contains("Remind Mon"), "{screen}");
    }

    #[test]
    fn deadlines_on_the_twelve_hour_clock() {
        let (app, _dir) = deadlines_app(true);
        assert_snapshot!(render_sized(&app, 50, 12).backend());
    }

    #[test]
    fn choosing_a_deadline() {
        let (mut app, _dir) = deadlines_app(false);
        type_str(&mut app, "@");
        assert_snapshot!(render_sized(&app, 80, 24).backend());
    }

    #[test]
    fn typing_a_deadlines_time() {
        let (mut app, _dir) = deadlines_app(false);
        type_str(&mut app, "jjj@j");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "25:00");
        press(&mut app, KeyCode::Enter);
        let mut terminal = render_sized(&app, 60, 16);
        assert_snapshot!(terminal.backend());
        let cursor = terminal.get_cursor_position().unwrap();
        let row: String = (0..60).map(|x| terminal.backend().buffer()[(x, cursor.y)].symbol().to_string()).collect();
        assert!(row.contains("› 25:00"), "{row}");
    }

    #[test]
    fn options_searching() {
        let (mut app, _dir) = crate::test_util::app_with_workspaces(&["Home"]);
        type_str(&mut app, " o/notes");
        let mut terminal = render_sized(&app, 70, 30);
        assert_snapshot!(terminal.backend());
        // The cursor is in the search.
        let cursor = terminal.get_cursor_position().unwrap();
        let row: String = (0..70).map(|x| terminal.backend().buffer()[(x, cursor.y)].symbol().to_string()).collect();
        assert!(row.contains("Search: notes"), "{row}");
    }

    #[test]
    fn options_searching_for_nothing() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        type_str(&mut app, " o/zzz");
        assert_snapshot!(render_sized(&app, 50, 12).backend());
    }

    #[test]
    fn options_confirming_a_deletion() {
        let (mut app, _dir) = crate::test_util::app_with_workspaces(&["Home"]);
        type_str(&mut app, " o");
        let Mode::Options(options) = &mut app.mode else { panic!("the options") };
        options.folder = Some("~/todoro".into());
        options.selected = options.first_clear_row();
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "Ho");
        let mut terminal = render_sized(&app, 70, 24);
        assert_snapshot!(terminal.backend());
        let cursor = terminal.get_cursor_position().unwrap();
        let row: String = (0..70).map(|x| terminal.backend().buffer()[(x, cursor.y)].symbol().to_string()).collect();
        assert!(row.contains("› Ho"), "{row}");
    }

    #[test]
    fn options_delete_section() {
        let (mut app, _dir) = crate::test_util::app_with_workspaces(&["Home"]);
        type_str(&mut app, " o");
        let Mode::Options(options) = &mut app.mode else { panic!("the options") };
        options.folder = Some("~/todoro".into());
        options.selected = options.first_clear_row() + options.clears().len() - 1;
        assert_snapshot!(render_sized(&app, 70, 40).backend());
    }

    #[test]
    fn welcome_shows_the_newest_releases_and_a_link() {
        let (mut app, _dir) = app_with(&[]);
        app.mode = Mode::Changelog(ChangelogView::newest(1));
        let screen = render_sized(&app, 70, 30).backend().to_string();
        assert!(screen.contains(&format!("Welcome to todoro {}", changes::VERSION)), "{screen}");
        // At the end, after the newest release, a link to the rest.
        type_str(&mut app, "G");
        let screen = render_sized(&app, 70, 30).backend().to_string();
        let flat: String = screen.lines().map(|line| line.trim_matches(|c| c == '"' || c == '│' || c == ' ')).collect();
        assert!(flat.contains("Older releases are on GitHub") || changes::releases(changes::CHANGELOG).len() == 1, "{screen}");
        assert!(flat.contains(changes::RELEASES_URL), "{screen}");
        // With every release shown, it still says where they are.
        app.mode = Mode::Changelog(ChangelogView::newest(99));
        render_sized(&app, 70, 30);
        type_str(&mut app, "G");
        let screen = render_sized(&app, 70, 30).backend().to_string();
        let flat: String = screen.lines().map(|line| line.trim_matches(|c| c == '"' || c == '│' || c == ' ')).collect();
        assert!(flat.contains("Every release is also on GitHub"), "{screen}");
        // But not when N shows them all.
        app.mode = Mode::Changelog(ChangelogView::all());
        render_sized(&app, 70, 30);
        type_str(&mut app, "G");
        let screen = render_sized(&app, 70, 30).backend().to_string();
        assert!(!screen.contains("on GitHub"), "{screen}");
    }

    #[test]
    fn screens_at_40_columns() {
        assert_snapshot!(narrow_screens(40));
    }

    #[test]
    fn screens_at_30_columns() {
        assert_snapshot!(narrow_screens(30));
    }

    #[test]
    fn help_scrolling_counts_wrapped_lines() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        type_str(&mut app, "?");
        render_sized(&app, 30, 12);
        // More presses than there are lines, so it must stop at the end.
        for _ in 0..1000 {
            press(&mut app, KeyCode::Down);
            render_sized(&app, 30, 12);
        }
        // The last binding is visible at the bottom once scrolled all the way.
        let screen = render_sized(&app, 30, 12).backend().to_string();
        assert!(screen.contains("Ctrl+C"), "{screen}");
    }

    /// An app with items spread over October 2026, the calendar open in `zoom`
    /// on today with `keys` pressed.
    fn calendar_app(keys: &str) -> (App, tempfile::TempDir) {
        let (mut app, dir) = app_with(&["Buy milk", "Write the quarterly report", "Call mom"]);
        app.store.toggle_done(app.day, 2).unwrap();
        let on = |d: u32| NaiveDate::from_ymd_opt(2026, 10, d).unwrap();
        for (d, text) in [(7, "Dentist 3pm"), (7, "Pick up parcel"), (9, "Team lunch"), (16, "Pay rent"), (31, "Halloween party")] {
            let index = app.store.open_count(on(d));
            app.store.insert(on(d), index, text.into()).unwrap();
        }
        app.store.insert(NaiveDate::from_ymd_opt(2026, 12, 25).unwrap(), 0, "Christmas".into()).unwrap();
        type_str(&mut app, " c");
        type_str(&mut app, keys);
        (app, dir)
    }

    #[test]
    fn calendar_month() {
        let (app, _dir) = calendar_app("l");
        assert_snapshot!(render_sized(&app, 80, 30).backend());
    }

    #[test]
    fn calendar_month_small() {
        let (app, _dir) = calendar_app("l");
        assert_snapshot!(render_sized(&app, 30, 12).backend());
    }

    #[test]
    fn calendar_week() {
        let (app, _dir) = calendar_app("wj");
        assert_snapshot!(render_sized(&app, 80, 24).backend());
    }

    #[test]
    fn calendar_week_small() {
        let (app, _dir) = calendar_app("w");
        assert_snapshot!(render_sized(&app, 30, 12).backend());
    }

    #[test]
    fn calendar_year() {
        let (app, _dir) = calendar_app("y");
        assert_snapshot!(render_sized(&app, 100, 30).backend());
    }

    #[test]
    fn calendar_year_small_scrolls_to_the_selected_month() {
        let (app, _dir) = calendar_app("yLL");
        assert_snapshot!(render_sized(&app, 30, 24).backend());
    }

    /// The calendar's game of Snake, its food placed on `food` so the
    /// snapshot doesn't depend on the random seed.
    fn snake_app(food: NaiveDate) -> (App, tempfile::TempDir) {
        let (mut app, dir) = calendar_app(":snake");
        press(&mut app, KeyCode::Enter);
        if let Mode::Calendar(calendar) = &mut app.mode {
            calendar.snake.as_mut().unwrap().set_food(food);
        }
        (app, dir)
    }

    fn snake_steps(app: &mut App, steps: usize) {
        if let Mode::Calendar(calendar) = &mut app.mode {
            for _ in 0..steps {
                calendar.snake.as_mut().unwrap().step(4);
            }
        }
    }

    #[test]
    fn calendar_snake() {
        let (mut app, _dir) = snake_app(NaiveDate::from_ymd_opt(2026, 10, 7).unwrap());
        assert_snapshot!("calendar_snake_ready", render_sized(&app, 100, 30).backend());
        // The snake starts on 3 to 5 October. Down to the 12th, then right
        // to the 14th, passing under the food on the 7th.
        type_str(&mut app, "j");
        snake_steps(&mut app, 1);
        type_str(&mut app, "l");
        snake_steps(&mut app, 2);
        let terminal = render_sized(&app, 100, 30);
        assert_snapshot!("calendar_snake_playing", terminal.backend());
        // The snake is drawn with colour: its head on the 14th, its body on
        // the 13th and 12th.
        let buffer = terminal.backend().buffer();
        let with_bg = |bg: Color| -> Vec<String> {
            let area = buffer.area;
            let mut found = Vec::new();
            // The calendar, not the status bar below it.
            for y in area.top()..area.bottom() - 1 {
                let row: String = (area.left()..area.right())
                    .map(|x| if buffer[(x, y)].bg == bg { buffer[(x, y)].symbol().to_string() } else { "|".into() })
                    .collect();
                found.extend(row.split('|').filter(|cell| !cell.trim().is_empty()).map(|cell| cell.trim().to_string()));
            }
            found
        };
        assert_eq!(with_bg(SNAKE_HEAD), ["14"]);
        assert_eq!(with_bg(Color::Green), ["12", "13"]);
    }

    #[test]
    fn calendar_snake_eats_and_scores() {
        let (mut app, _dir) = snake_app(NaiveDate::from_ymd_opt(2026, 10, 6).unwrap());
        type_str(&mut app, "l");
        snake_steps(&mut app, 1);
        if let Mode::Calendar(calendar) = &mut app.mode {
            let game = calendar.snake.as_mut().unwrap();
            assert_eq!(game.score, 1);
            game.set_food(NaiveDate::from_ymd_opt(2026, 11, 20).unwrap());
        }
        let screen = format!("{:?}", render_sized(&app, 100, 30).backend());
        assert!(screen.contains("Snake 2026 · ● 1"), "{screen}");
    }

    #[test]
    fn calendar_snake_game_over() {
        let (mut app, _dir) = snake_app(NaiveDate::from_ymd_opt(2026, 11, 20).unwrap());
        type_str(&mut app, "k");
        // Up from Monday 5 October, with four months across: June's Mondays,
        // then February's, then above February is 2025, a wall.
        snake_steps(&mut app, 20);
        let Mode::Calendar(calendar) = &app.mode else { panic!("the calendar should be open") };
        let game = calendar.snake.as_ref().unwrap();
        assert_eq!(game.state, snake::State::Lost);
        assert_eq!(game.head(), NaiveDate::from_ymd_opt(2026, 2, 2).unwrap());
        assert_snapshot!(render_sized(&app, 100, 30).backend());
    }

    #[test]
    fn year_view_j_follows_the_months_across_as_last_drawn() {
        let cursor = |app: &App| match &app.mode {
            Mode::Calendar(calendar) => calendar.cursor,
            _ => panic!("the calendar should be open"),
        };
        let (mut app, _dir) = calendar_app("y");
        // Monday 26 October is the last Monday in October.
        for _ in 0..3 {
            type_str(&mut app, "j");
        }
        // Wide enough for four months across: below October is February.
        render_sized(&app, 100, 30);
        type_str(&mut app, "j");
        assert_eq!(cursor(&app), NaiveDate::from_ymd_opt(2027, 2, 1).unwrap());
        type_str(&mut app, "k");
        // Resized to one month across: below October is November.
        render_sized(&app, 30, 24);
        type_str(&mut app, "j");
        assert_eq!(cursor(&app), NaiveDate::from_ymd_opt(2026, 11, 2).unwrap());
    }

    #[test]
    fn calendar_adding() {
        let (app, _dir) = calendar_app("jjaRenew the parking permit before it runs out at the end of the month");
        let mut terminal = render_sized(&app, 60, 20);
        assert_snapshot!(terminal.backend());
        // The cursor follows the text onto its second line.
        let position = terminal.get_cursor_position().unwrap();
        assert_eq!(position.y, 10);
    }

    #[test]
    fn triaged_items_keep_their_layout() {
        let (mut app, _dir) = app_with(&[
            "Fix the leaking tap",
            "Book flights for the holiday before the prices go up again",
            "Water the plants",
            "Sort the recycling",
        ]);
        app.store.toggle_pinned(app.day, 1).unwrap();
        type_str(&mut app, "!j!!jj!!!");
        let mut terminal = render(&app);
        assert_snapshot!(terminal.backend());
        // Only the numbers' colours change, so the text and cursor stay put.
        type_str(&mut app, "kkke");
        terminal = render(&app);
        assert_eq!(terminal.get_cursor_position().unwrap(), Position::new(1 + 3 + 19, 1));
    }

    /// An app with items on several days, some with notes, and `keys` pressed.
    fn search_app(keys: &str) -> (App, tempfile::TempDir) {
        let (mut app, dir) = app_with(&["Buy milk", "Write the quarterly report for the team", "Call mum"]);
        app.store.set_notes(app.day, 1, "Ask Sam for the Q3 numbers\nPull the charts from the dashboard".into()).unwrap();
        app.store.toggle_done(app.day, 2).unwrap();
        let on = |offset: i64| app.day + chrono::Duration::days(offset);
        let (yesterday, later) = (on(-1), on(9));
        app.store.insert(yesterday, 0, "Buy stamps for the birthday cards".into()).unwrap();
        app.store.insert(later, 0, "Dashboard review with the team".into()).unwrap();
        type_str(&mut app, keys);
        (app, dir)
    }

    #[test]
    fn search_with_a_preview_of_the_notes() {
        let (mut app, _dir) = search_app("");
        app.store
            .set_notes(app.day, 0, "## Shop\n- [ ] oat milk\n- [x] **bread**\n\n> the big bottle".into())
            .unwrap();
        type_str(&mut app, " sga");
        assert_snapshot!(render_sized(&app, 120, 20).backend());
    }

    #[test]
    fn search_moving_through_results_with_a_preview() {
        let (mut app, _dir) = search_app("");
        type_str(&mut app, " sgdash");
        press(&mut app, KeyCode::Esc);
        type_str(&mut app, "j");
        let terminal = render_sized(&app, 120, 20);
        assert_snapshot!(terminal.backend());
        // Not typing, so there's no cursor in the query.
        let Mode::Search(search) = &app.mode else { panic!("still searching") };
        assert!(!search.typing);
    }

    #[test]
    fn search_results() {
        let (mut app, _dir) = search_app("  bu");
        press(&mut app, KeyCode::Down);
        let mut terminal = render_sized(&app, 70, 14);
        assert_snapshot!(terminal.backend());
        // The cursor follows the query: the 66-wide popup starts at x=2, then
        // border, padding, "Search: " and "bu".
        assert_eq!(terminal.get_cursor_position().unwrap(), Position::new(2 + 1 + 1 + 8 + 2, 2));
    }

    #[test]
    fn searching_notes() {
        let (app, _dir) = search_app(" sgdashboard");
        assert_snapshot!(render_sized(&app, 70, 14).backend());
    }

    #[test]
    fn finding_items_lists_them_all_with_a_preview() {
        let (mut app, _dir) = search_app("");
        type_str(&mut app, "  ");
        assert_snapshot!(render_sized(&app, 120, 16).backend());
    }

    #[test]
    fn search_empty_and_no_matches() {
        let (mut app, _dir) = search_app(" sg");
        let empty = render_sized(&app, 70, 8).backend().to_string();
        type_str(&mut app, "zzz");
        let none = render_sized(&app, 70, 8).backend().to_string();
        assert_snapshot!(format!("{empty}\n{none}"));
    }

    #[test]
    fn search_narrow() {
        let (app, _dir) = search_app(" sgdashboard");
        assert_snapshot!(render_sized(&app, 30, 14).backend());
    }

    #[test]
    fn highlighted_keeps_the_first_match_in_view() {
        let base = Style::new();
        let matched = Style::new().bold();
        let text = |spans: Vec<Span>| spans.iter().map(|s| s.content.to_string()).collect::<String>();
        assert_eq!(text(highlighted("Buy milk", &[0, 4], 20, base, matched)), "Buy milk");
        assert_eq!(text(highlighted("Buy milk and bread", &[0], 10, base, matched)), "Buy milk …");
        // A match near the end scrolls the text so it shows.
        let long = "Pull the charts from the dashboard";
        let spans = highlighted(long, &(25..34).collect::<Vec<_>>(), 16, base, matched);
        let shown = text(spans.clone());
        assert!(shown.starts_with('…') && shown.contains("dashboard"), "{shown}");
        assert!(shown.width() <= 16, "{shown}");
        assert!(spans.iter().any(|s| s.content == "dashboard" && s.style == matched));
    }

    #[test]
    fn options_popup() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        app.settings.semantic_icons = true;
        type_str(&mut app, " oj");
        assert_snapshot!(render_sized(&app, 70, 16).backend());
    }

    #[test]
    fn options_keys_row() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        app.keys_path = Some(PathBuf::from("/home/sam/.config/todoro/keys.toml"));
        type_str(&mut app, " o");
        let Mode::Options(options) = &mut app.mode else { panic!("the options") };
        options.selected = crate::options::KEYS_ROW;
        options.message = Some(("Keys loaded".into(), true));
        assert_snapshot!(render_sized(&app, 70, 30).backend());
    }

    #[test]
    fn options_popup_narrow() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        type_str(&mut app, " o");
        assert_snapshot!(render_sized(&app, 30, 12).backend());
    }

    #[test]
    fn semantic_icons_show_beside_triaged_items() {
        let (mut app, _dir) = app_with(&["Fix the leaking tap", "Book flights", "Water the plants", "Sort the recycling", "Old"]);
        app.settings.semantic_icons = true;
        type_str(&mut app, "!j!!jj!!!j!x");
        assert_snapshot!(render(&app).backend());
        // Off again, the list looks as it does without triage icons.
        app.settings.semantic_icons = false;
        assert!(!render(&app).backend().to_string().contains('∧'));
    }

    #[test]
    fn no_colour_removes_every_colour_but_keeps_highlights() {
        let (mut app, _dir) = app_with(&["Fix the tap", "Book flights"]);
        app.store.set_notes(app.day, 0, "notes".into()).unwrap();
        type_str(&mut app, "!jm");
        app.settings.no_colour = true;
        let mut screens = vec![render(&app)];
        type_str(&mut app, " c");
        screens.push(render_sized(&app, 70, 24));
        press(&mut app, KeyCode::Esc);
        type_str(&mut app, " sgfix");
        screens.push(render_sized(&app, 70, 14));
        for terminal in &screens {
            let buffer = terminal.backend().buffer();
            assert!(buffer.content.iter().all(|cell| cell.fg == Color::Reset && cell.bg == Color::Reset));
        }
        // The list's selected row and the mode label are reversed instead.
        let list = screens[0].backend().buffer();
        assert!(list[(5, 2)].modifier.contains(Modifier::REVERSED), "selected row");
        assert!(!list[(5, 1)].modifier.contains(Modifier::REVERSED), "other row");
        assert!(list[(2, 9)].modifier.contains(Modifier::REVERSED), "mode label");
    }

    #[test]
    fn selecting_several_items() {
        let (mut app, _dir) = app_with(&["Buy milk", "Write report", "Call mom", "Book dentist"]);
        type_str(&mut app, "jVj");
        let terminal = render(&app);
        assert_snapshot!(terminal.backend());
        // The selected rows have the selection background; the others don't.
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(5, 2)].bg, VISUAL_BG);
        assert_eq!(buffer[(5, 3)].bg, SELECTED_BG);
        assert_eq!(buffer[(5, 1)].bg, Color::Reset);
        assert_eq!(buffer[(5, 4)].bg, Color::Reset);
    }

    #[test]
    fn delete_popup_for_several_items() {
        let (mut app, _dir) = app_with(&["Buy milk", "Write the quarterly report for the team", "Call mom", "Book dentist"]);
        type_str(&mut app, "VGd");
        assert_snapshot!(render_sized(&app, 60, 12).backend());
    }

    #[test]
    fn delete_popup_for_more_items_than_fit() {
        let items: Vec<String> = (1..=12).map(|i| format!("item {i}")).collect();
        let (mut app, _dir) = app_with(&items.iter().map(String::as_str).collect::<Vec<_>>());
        type_str(&mut app, "VGd");
        let screen = render_sized(&app, 60, 12).backend().to_string();
        assert!(screen.contains("Delete 12 items?"), "{screen}");
        assert!(screen.contains("…and 7 more"), "{screen}");
    }

    #[test]
    fn tags_are_coloured_on_the_list() {
        let (mut app, _dir) = app_with(&["Call #work about the #budget", "issue#4 is not a tag", "Done #work"]);
        type_str(&mut app, "jjx");
        let terminal = render(&app);
        let buffer = terminal.backend().buffer();
        let row = |y: u16| (1..58).map(|x| (buffer[(x, y)].symbol().to_string(), buffer[(x, y)].fg)).collect::<Vec<_>>();
        let coloured = |y: u16| row(y).into_iter().filter(|(_, fg)| *fg == TAG_COLOUR).map(|(s, _)| s).collect::<String>();
        assert_eq!(coloured(1), "#work#budget");
        assert_eq!(coloured(2), "");
        // Completed items stay grey.
        assert_eq!(coloured(4), "");
    }

    #[test]
    fn tag_list() {
        let (mut app, _dir) = app_with(&["Call #work about the #budget", "Water the plants #home", "Send #work report"]);
        type_str(&mut app, " stj");
        assert_snapshot!(render_sized(&app, 60, 12).backend());
    }

    #[test]
    fn tag_list_when_there_are_none() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        type_str(&mut app, " st");
        assert_snapshot!(render_sized(&app, 40, 10).backend());
    }

    #[test]
    fn a_tags_items() {
        let (mut app, _dir) = app_with(&["Call #work about the #budget", "Water the plants #home"]);
        let later = app.day + chrono::Duration::days(5);
        app.store.insert(later, 0, "Send #Work report".into()).unwrap();
        type_str(&mut app, " st");
        press(&mut app, KeyCode::Enter);
        assert_snapshot!(render_sized(&app, 60, 12).backend());
    }

    #[test]
    fn adding_several_items() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        type_str(&mut app, "AWrite report");
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "Call");
        let mut terminal = render(&app);
        assert_snapshot!(terminal.backend());
        // The cursor is on the new line, after "Call".
        assert_eq!(terminal.get_cursor_position().unwrap(), Position::new(1 + 3 + 4, 3));
    }

    #[test]
    fn notes_with_line_numbers_and_a_command() {
        let (mut app, _dir) = app_with(&["Write report"]);
        app.store.set_notes(app.day, 0, (1..=12).map(|i| format!("point {i}")).collect::<Vec<_>>().join("\n")).unwrap();
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, ":1");
        let mut terminal = render(&app);
        assert_snapshot!(terminal.backend());
        // The cursor is on the bottom line, after ":1".
        assert_eq!(terminal.get_cursor_position().unwrap(), Position::new(2, 9));
        type_str(&mut app, "1");
        press(&mut app, KeyCode::Enter);
        let Mode::Notes(editor) = &app.mode else { panic!("still in the notes") };
        assert_eq!(editor.textarea.cursor().0, 10);
    }

    #[test]
    fn selecting_text_in_the_notes() {
        let (mut app, _dir) = app_with(&["Write report"]);
        app.store.set_notes(app.day, 0, "Draft the intro".into()).unwrap();
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "wve");
        let terminal = render(&app);
        let buffer = terminal.backend().buffer();
        let text: String = (1..58).map(|x| buffer[(x, 1)].symbol().to_string()).collect();
        let start = text.find("the").unwrap() as u16 + 1;
        // "th" has the selection's dark background, the same as the list's
        // selection, and the text keeps its colour; the cursor covers the "e".
        assert_eq!(buffer[(start, 1)].bg, VISUAL_BG);
        assert_eq!(buffer[(start + 1, 1)].bg, VISUAL_BG);
        assert_eq!(buffer[(start, 1)].fg, Color::Reset);
        assert!(buffer[(start + 2, 1)].modifier.contains(Modifier::REVERSED));
        assert_eq!(buffer[(start - 2, 1)].bg, Color::Reset);
        assert!(terminal.backend().to_string().contains("VISUAL"));
    }

    #[test]
    fn the_cursor_lines_number_is_highlighted() {
        let (mut app, _dir) = app_with(&["Write report"]);
        let notes: Vec<String> = (1..=12).map(|i| format!("point {i}")).collect();
        app.store.set_notes(app.day, 0, notes.join("\n")).unwrap();
        press(&mut app, KeyCode::Enter);
        // Rows of the notes area whose gutter has the highlight.
        let highlighted = |app: &App| -> Vec<String> {
            let terminal = render(app);
            let buffer = terminal.backend().buffer().clone();
            (1..9)
                .filter(|&y| (2..5).any(|x| buffer[(x, y)].style().fg == Some(Color::Yellow)))
                .map(|y| (2..5).map(|x| buffer[(x, y)].symbol().to_string()).collect::<String>().trim().to_string())
                .collect()
        };
        assert_eq!(highlighted(&app), ["1"]);
        type_str(&mut app, "2j");
        assert_eq!(highlighted(&app), ["3"]);
        // Scrolled down, the right number is still found.
        type_str(&mut app, "G");
        assert_eq!(highlighted(&app), ["12"]);
    }

    #[test]
    fn typing_a_command_on_the_list() {
        let (mut app, _dir) = app_with(&["Buy milk", "Call mom"]);
        type_str(&mut app, ":2");
        let mut terminal = render(&app);
        let screen = terminal.backend().to_string();
        assert!(screen.contains("\":2 "), "{screen}");
        assert_eq!(terminal.get_cursor_position().unwrap(), Position::new(2, 9));
    }

    #[test]
    fn a_selected_items_number_shows_its_priority_colour() {
        let (mut app, _dir) = app_with(&["one", "two", "three"]);
        let number = |app: &App, y: u16| render(app).backend().buffer()[(1, y)].style();
        // Triaging the selected item shows its colour straight away.
        type_str(&mut app, "j!");
        assert_eq!(number(&app, 2).fg, Some(Color::Red));
        type_str(&mut app, "!");
        assert_eq!(number(&app, 2).fg, Some(Color::Yellow));
        // An untriaged item's number stays grey, selected or not.
        type_str(&mut app, "k");
        assert_eq!(number(&app, 1).fg, Some(Color::DarkGray));
        assert_eq!(number(&app, 2).fg, Some(Color::Yellow));
    }

    #[test]
    fn copied_lines_flash_like_a_selected_item() {
        let (mut app, _dir) = app_with(&["Write report"]);
        app.store.set_notes(app.day, 0, "one\ntwo\nthree\nfour".into()).unwrap();
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "j2yy");
        let flashing = |app: &App| -> Vec<u16> {
            let terminal = render(app);
            let buffer = terminal.backend().buffer().clone();
            (1..9).filter(|&y| buffer[(10, y)].bg == SELECTED_BG).collect()
        };
        // Lines 2 and 3, on screen rows 2 and 3; not the blank rows below.
        assert_eq!(flashing(&app), [2, 3]);
        let Mode::Notes(editor) = &app.mode else { panic!("in the notes") };
        let ends = editor.flash_ends().unwrap();
        assert_eq!(app.redraw_at(), Some(ends));
        app.tick(ends);
        assert!(flashing(&app).is_empty());
        assert_eq!(app.redraw_at(), None);
    }

    #[test]
    fn inline_code_drops_the_backticks_and_marks_the_code() {
        let (plain, marks) = inline_code("Press `j` or `4j` to move");
        assert_eq!(plain, "Press j or 4j to move");
        assert_eq!(marks.iter().map(|m| &plain[m.clone()]).collect::<Vec<_>>(), ["j", "4j"]);
    }

    #[test]
    fn markdown_has_headings_bullets_with_hanging_indents_and_code() {
        let mut lines = Vec::new();
        markdown_lines("### Features\n\n- Press `x` to mark an item done, then it moves below\n\nPlain text.", 30, &mut lines);
        let text: Vec<String> = lines.iter().map(|line| line.to_string()).collect();
        assert_eq!(
            text,
            ["Features", "", "• Press x to mark an item", "  done, then it moves below", "", "Plain text."]
        );
        let code = lines[2].spans.iter().find(|span| span.content == "x").expect("code span");
        assert_eq!(code.style.fg, Some(Color::Yellow));
    }

    #[test]
    fn whats_new_popup() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        app.mode = Mode::Changelog(ChangelogView::all());
        let screen = render_sized(&app, 80, 30).backend().to_string();
        let latest = &changes::releases(changes::CHANGELOG)[0];
        assert!(screen.contains("Changelog"), "{screen}");
        assert!(screen.contains(&latest.title), "{screen}");
        assert!(!screen.contains("```"), "{screen}");
        // What's new names this version.
        app.mode = Mode::Changelog(ChangelogView::since((0, 0, 1)));
        let screen = render_sized(&app, 80, 30).backend().to_string();
        assert!(screen.contains(&format!("What's new in todoro {}", changes::VERSION)), "{screen}");
    }

    #[test]
    fn whats_new_popup_scrolls_to_the_end_and_fits_narrow_screens() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        app.mode = Mode::Changelog(ChangelogView::all());
        render_sized(&app, 30, 12);
        for _ in 0..2000 {
            press(&mut app, KeyCode::Char('j'));
            render_sized(&app, 30, 12);
        }
        let screen = render_sized(&app, 30, 12).backend().to_string();
        // The oldest release's last note ends the last line.
        assert!(screen.contains("macOS and Windows."), "{screen}");
    }

    #[test]
    fn scroll_position_reads_like_vims() {
        assert_eq!(scroll_position(0, 0), "All");
        assert_eq!(scroll_position(0, 40), "Top");
        assert_eq!(scroll_position(10, 40), "25%");
        assert_eq!(scroll_position(39, 40), "97%");
        assert_eq!(scroll_position(40, 40), "Bot");
    }

    #[test]
    fn the_changelog_shows_where_you_are() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        app.mode = Mode::Changelog(ChangelogView::all());
        let bottom = |app: &App, width: u16| -> String {
            let terminal = render_sized(app, width, 16);
            let buffer = terminal.backend().buffer().clone();
            (0..width).map(|x| buffer[(x, 14)].symbol().to_string()).collect()
        };
        assert!(bottom(&app, 80).contains(" Top ╯"), "{}", bottom(&app, 80));
        for _ in 0..5 {
            press(&mut app, KeyCode::Char('j'));
        }
        let border = bottom(&app, 80);
        assert!(border.contains("% ╯") && border.contains("esc close"), "{border}");
        press(&mut app, KeyCode::Char('G'));
        assert!(bottom(&app, 80).contains(" Bot ╯"));
        // Narrow screens keep the position and drop the hint first. Lines
        // wrap more there, so go to the end again at this width.
        bottom(&app, 30);
        press(&mut app, KeyCode::Char('G'));
        let narrow = bottom(&app, 30);
        assert!(narrow.contains(" Bot ╯") && narrow.contains("esc close"), "{narrow}");
    }

    #[test]
    fn short_notes_say_all() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        // Only this release, on a tall screen, fits without scrolling.
        app.mode = Mode::Changelog(ChangelogView::since(changes::parse_version(changes::VERSION).unwrap()));
        let terminal = render_sized(&app, 80, 40);
        assert!(terminal.backend().to_string().contains(" All ╯"));
    }

    #[test]
    fn whole_lines_selected_with_capital_v_are_painted() {
        let (mut app, _dir) = app_with(&["Sample item"]);
        app.store.set_notes(app.day, 0, "one\ntwo\nthree\nfour".into()).unwrap();
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "jVj");
        let terminal = render(&app);
        let buffer = terminal.backend().buffer();
        let painted: Vec<u16> = (1..9).filter(|&y| buffer[(10, y)].bg == VISUAL_BG).collect();
        // Lines 2 and 3, across the whole width, not just up to the cursor.
        assert_eq!(painted, [2, 3]);
        assert_eq!(buffer[(50, 2)].bg, VISUAL_BG);
        assert!(terminal.backend().to_string().contains("V-LINE"));
    }

    fn render_setup(setup: &Setup, width: u16, height: u16) -> Terminal<TestBackend> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| draw_setup(frame, setup)).unwrap();
        terminal
    }

    #[test]
    fn setup_screen() {
        let mut setup = Setup::new(Some(PathBuf::from("/home/sam/todoro")), None);
        setup.handle_key(ratatui::crossterm::event::KeyEvent::new(KeyCode::Tab, ratatui::crossterm::event::KeyModifiers::NONE));
        let mut terminal = render_setup(&setup, 70, 20);
        assert_snapshot!(terminal.backend());
        // The cursor is at the end of the workspace name being typed.
        let cursor = terminal.get_cursor_position().unwrap();
        let row: String = (0..70).map(|x| terminal.backend().buffer()[(x, cursor.y)].symbol().to_string()).collect();
        assert!(row.contains("› Personal"), "{row}");
    }

    #[test]
    fn setup_screen_when_moving_old_todos_and_after_a_mistake() {
        let mut setup = Setup::new(Some(PathBuf::from("/home/sam/todoro")), Some(PathBuf::from("/old/todos.json")));
        setup.error = Some("A workspace name can't have / \\ : * ? \" < > or |".into());
        assert_snapshot!(render_setup(&setup, 40, 24).backend());
    }

    #[test]
    fn the_workspace_name_shows_when_it_fits() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        app.workspace = Some("Work".into());
        let wide = render_sized(&app, 70, 6).backend().to_string();
        assert!(wide.lines().next().unwrap().contains(" Work "), "{wide}");
        // Beside a long date on a narrow screen there's no room, so it's left out.
        let narrow = render_sized(&app, 30, 6).backend().to_string();
        assert!(!narrow.lines().next().unwrap().contains("Work"), "{narrow}");
    }

    #[test]
    fn workspaces_popup() {
        let (mut app, _dir) = crate::test_util::app_with_workspaces(&["Home", "Side project", "Work"]);
        type_str(&mut app, " wj");
        assert_snapshot!(render_sized(&app, 60, 14).backend());
    }

    #[test]
    fn deleting_a_workspace() {
        let (mut app, _dir) = crate::test_util::app_with_workspaces(&["Home", "Work"]);
        type_str(&mut app, " wjdWor");
        let mut terminal = render_sized(&app, 60, 16);
        assert_snapshot!(terminal.backend());
        // The cursor is after what's been typed.
        let cursor = terminal.get_cursor_position().unwrap();
        let row: String = (0..60).map(|x| terminal.backend().buffer()[(x, cursor.y)].symbol().to_string()).collect();
        assert!(row.contains("› Wor"), "{row}");
    }

    #[test]
    fn options_with_the_todoro_folder() {
        let (mut app, _dir) = crate::test_util::app_with_workspaces(&["Home"]);
        type_str(&mut app, " o");
        let Mode::Options(options) = &mut app.mode else { panic!("the options") };
        // A fixed path, so the snapshot doesn't depend on where the test runs.
        options.folder = Some("~/todoro".into());
        type_str(&mut app, &"j".repeat(crate::options::FOLDER_ROW));
        press(&mut app, KeyCode::Enter);
        assert_snapshot!(render_sized(&app, 70, 28).backend());
    }

    #[test]
    fn options_typing_a_notes_editor() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        type_str(&mut app, " o");
        type_str(&mut app, &"j".repeat(crate::options::EDITOR_ROW));
        press(&mut app, KeyCode::Enter);
        type_str(&mut app, "nvim");
        let mut terminal = render_sized(&app, 70, 22);
        assert_snapshot!(terminal.backend());
        let cursor = terminal.get_cursor_position().unwrap();
        let row: String = (0..70).map(|x| terminal.backend().buffer()[(x, cursor.y)].symbol().to_string()).collect();
        assert!(row.contains("› nvim"), "{row}");
    }

    #[test]
    fn options_with_a_notes_editor_set() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        app.settings.editor = Some("nvim".into());
        type_str(&mut app, " o");
        type_str(&mut app, &"j".repeat(crate::options::EDITOR_ROW));
        assert_snapshot!(render_sized(&app, 70, 18).backend());
    }

    #[test]
    fn a_message_takes_the_status_bar() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        app.message = Some("buy-milk.md changed outside todoro, so it was kept.".into());
        let screen = render(&app).backend().to_string();
        assert!(screen.lines().last().unwrap().contains("changed outside todoro"), "{screen}");
    }

    #[test]
    fn delete_popup() {
        let (mut app, _dir) = app_with(&["Buy milk", "Write report"]);
        type_str(&mut app, "jd");
        assert_snapshot!(render(&app).backend());
    }
}
