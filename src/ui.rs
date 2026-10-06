use std::ops::Range;

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Flex, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, List, ListItem, ListState, Padding, Paragraph, Wrap};
use ratatui::Frame;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use chrono::{Datelike, Days, NaiveDate};

use crate::app::{App, Mode};
use crate::calendar::{self, Calendar, Zoom};
use crate::changelog::{self as changes, ChangelogView};
use crate::store::{Item, Priority};
use crate::help::{Help, SECTIONS};
use crate::notes::NotesEditor;
use crate::options::{Options, TOGGLES};
use crate::search::Search;
use crate::setup::Setup;
use crate::workspaces::Picker;
use crate::tags::{self, TagPicker};

pub fn draw(frame: &mut Frame, app: &App) {
    let [main, status] = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(frame.area());

    // The help popup is drawn over whichever screen it was opened from.
    let screen = match &app.mode {
        Mode::Help { back, .. } => back.as_ref(),
        mode => mode,
    };
    match screen {
        Mode::Notes(editor) => draw_notes(frame, app, editor, main),
        Mode::Calendar(calendar) => draw_calendar(frame, app, calendar, main),
        _ => draw_list(frame, app, main),
    }
    draw_status(frame, app, status);

    match &app.mode {
        Mode::ConfirmDelete { rows } => draw_confirm(frame, app, rows),
        Mode::Help { help, .. } => draw_help(frame, help),
        Mode::Calendar(calendar) if calendar.adding.is_some() => draw_adding(frame, calendar),
        Mode::Search(search) => draw_search(frame, app, search),
        Mode::Options(options) => draw_options(frame, app, options),
        Mode::Changelog(view) => draw_changelog(frame, view),
        Mode::Workspaces(picker) => draw_workspaces(frame, app, picker),
        Mode::Tags(picker) => draw_tags(frame, app, picker),
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
    // Use the longest date that fits, shortening it before dropping "(today)".
    let room = (area.width as usize).saturating_sub(4);
    let today = if app.day == app.today { " (today)" } else { "" };
    let dates = ["%A, %B %-d %Y", "%a, %b %-d %Y", "%a %-d %b"].map(|format| app.day.format(format).to_string());
    let (date, today) = [today, ""]
        .iter()
        .flat_map(|today| dates.iter().map(move |date| (date.clone(), *today)))
        .find(|(date, today)| date.width() + today.width() <= room)
        .unwrap_or_else(|| (truncate(&app.day.format("%-d/%-m").to_string(), room), ""));
    let title = vec![" ".into(), date.bold(), today.fg(Color::Green), " ".into()];

    let hint = fit_first(
        &[" h ← prev day · k ↑ up · j ↓ down · next day → l ", " h ← day · k ↑ · j ↓ · day → l ", " h/l day · j/k move "],
        (area.width as usize).saturating_sub(2),
    );
    let title = Line::from(title);
    let mut block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(title.clone().centered())
        .title_bottom(Line::from(hint).centered().dim());
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
    let mut rows: Vec<Row> = app
        .items()
        .into_iter()
        .map(|item| Row {
            text: &item.text,
            priority: item.priority,
            pinned: item.pinned,
            has_notes: !item.notes.is_empty(),
            done: item.done,
            typing: false,
        })
        .collect();
    let mut selected = (!rows.is_empty()).then_some(app.selected);
    if let Mode::Insert { index, input, editing, .. } = &app.mode {
        let text = &input.text;
        if *editing {
            rows[*index] = Row { text, typing: true, ..rows[*index] };
        } else {
            rows.insert(*index, Row { text, priority: None, pinned: false, has_notes: false, done: false, typing: true });
        }
        selected = Some(*index);
    }

    // Open items are numbered. Completed ones follow under a header row, so
    // their list row is one more than their item index.
    let open = rows.iter().take_while(|row| !row.done).count();
    let list_row = |index: usize| if index < open { index } else { index + 1 };
    let inner = block.inner(area);

    // Pad numbers so text lines up once there are 10 or more open items.
    let width = open.max(1).to_string().len();
    // With semantic icons on, triaged items get an icon after the number, and
    // the rest leave a gap so all the text lines up.
    let icons = app.settings.semantic_icons && rows.iter().any(|row| row.priority.is_some());
    let prefix_width = width + 2 + if icons { 2 } else { 0 };
    let mut items: Vec<ListItem> = Vec::with_capacity(rows.len() + 1);
    // How many screen lines each list row takes, to place the typing cursor.
    let mut heights: Vec<usize> = Vec::with_capacity(rows.len() + 1);
    // Where the typing cursor goes within its row: (line, column).
    let mut typing_cursor = (0, 0);
    for (i, row) in rows.iter().enumerate() {
        if i == open {
            let title = format!("── Completed ({}) ", rows.len() - open);
            let fill = (inner.width as usize).saturating_sub(title.chars().count());
            items.push(ListItem::new(format!("{title}{}", "─".repeat(fill)).dark_gray()));
            heights.push(1);
        }
        // A triaged item's number takes its priority's colour.
        // The selected item's number stands out as the current line's does in
        // the notes, even over a triaged item's priority colour.
        let current = selected == Some(i);
        let (prefix, prefix_style, text_style) = if row.done {
            let crossed = Style::new().fg(Color::DarkGray).add_modifier(Modifier::CROSSED_OUT);
            let tick = if current { CURSOR_LINE_NUMBER } else { Style::new().fg(Color::DarkGray) };
            (format!("{:>width$}  ", "✓"), tick, crossed)
        } else {
            let number_style = match row.priority {
                _ if current => CURSOR_LINE_NUMBER,
                Some(priority) => priority_style(priority),
                None => Style::new().fg(Color::DarkGray),
            };
            (format!("{:>width$}. ", i + 1), number_style, Style::new())
        };

        // Wrap long text under itself, leaving room for the markers at the end.
        let markers = [(row.pinned, PINNED_MARKER), (row.has_notes, NOTES_MARKER)];
        let markers_width: usize = markers.iter().filter(|(shown, _)| *shown).map(|(_, m)| m.width()).sum();
        let text_width = (inner.width as usize).saturating_sub(prefix_width + markers_width).max(1);
        let ranges = wrap_ranges(row.text, text_width);
        if let (true, Mode::Insert { input, .. }) = (row.typing, &app.mode) {
            typing_cursor = cursor_position(row.text, &ranges, input.cursor, text_width);
        }

        let mut lines: Vec<Line> = ranges
            .iter()
            .enumerate()
            .map(|(n, range)| {
                let mut spans = if n == 0 {
                    vec![Span::styled(prefix.clone(), prefix_style)]
                } else {
                    vec![Span::raw(" ".repeat(prefix_width))]
                };
                if n == 0 && icons {
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
        if row.pinned {
            last.push_span(PINNED_MARKER.fg(Color::Cyan));
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
        let empty =
            Paragraph::new("Nothing to do. Press a to add an item.").dim().centered().wrap(Wrap { trim: true });
        frame.render_widget(block, area);
        frame.render_widget(empty, inner);
        return;
    }

    let list = List::new(items)
        .block(block)
        .highlight_style(Style::new().bg(Color::Rgb(50, 50, 60)).add_modifier(Modifier::BOLD));
    frame.render_stateful_widget(list, area, &mut state);
    app.list_offset.set(state.offset());

    if let Mode::Insert { index, .. } = &app.mode {
        let (line, col) = typing_cursor;
        let row: usize = heights[state.offset()..list_row(*index)].iter().sum::<usize>() + line;
        frame.set_cursor_position(Position::new(inner.x + (prefix_width + col) as u16, inner.y + row as u16));
    }
}

/// One item row on the main list.
struct Row<'a> {
    text: &'a str,
    priority: Option<Priority>,
    pinned: bool,
    has_notes: bool,
    done: bool,
    /// The item being added or edited.
    typing: bool,
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

/// Shown after an item on the main list when it has notes.
const NOTES_MARKER: &str = " ≡";

/// Shown after a pinned item, which moves forward to today until completed.
const PINNED_MARKER: &str = " ⚲";

fn draw_notes(frame: &mut Frame, app: &App, editor: &NotesEditor, area: Rect) {
    let room = (area.width as usize).saturating_sub(4);
    let title = format!("{}. {}", app.selected + 1, app.items()[app.selected].text);
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

/// The current line's number in the notes, and the selected item's on the
/// list: bold, so it still stands out without colour.
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
        _ => None,
    };
    if let Some(input) = command {
        frame.render_widget(Line::from(format!(":{}", input.text)), area);
        let col = 1 + input.text[..input.cursor].width() as u16;
        frame.set_cursor_position(Position::new(area.x + col.min(area.width.saturating_sub(1)), area.y));
        return;
    }
    // Each hint has a priority. On a narrow screen the lowest go first, so
    // `? help` is the last to go.
    let (mode, color, hints): (_, _, &[(&str, u8)]) = match &app.mode {
        Mode::Normal => (
            "NORMAL",
            Color::Blue,
            &[("a add", 4), ("e edit", 2), ("x done", 3), ("d delete", 1), ("↵ notes", 0), ("? help", 5)],
        ),
        Mode::Insert { repeat: true, .. } => {
            ("INSERT", Color::Green, &[("←/→ move", 1), ("enter add & next", 3), ("esc done", 2)])
        }
        Mode::Insert { editing: false, .. } => {
            ("INSERT", Color::Green, &[("←/→ move", 1), ("enter/esc save", 2), ("(empty discards)", 0)])
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
        Mode::Options(_) => ("OPTIONS", Color::Blue, &[("j/k move", 1), ("space toggle", 2), ("esc close", 3)]),
        Mode::Search(_) => ("SEARCH", Color::Yellow, &[("↑/↓ select", 1), ("↵ go to item", 2), ("esc close", 3)]),
        Mode::Calendar(calendar) if calendar.adding.is_some() => {
            ("ADD", Color::Green, &[("enter/esc save", 1), ("(empty discards)", 0)])
        }
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
        let prefix = if item.done { "✓  ".to_string() } else { format!("{}. ", row + 1) };
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
        Zoom::Year => (vec![cursor.format("%Y").to_string()], &[" h/l day · j/k week · H/L month ", " H/L month "]),
    };
    let title = titles.iter().find(|title| title.width() <= room).cloned().unwrap_or_else(|| truncate(&titles[0], room));
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Color::Cyan))
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

/// The days of the selected week, top to bottom, each with its items.
fn draw_week(frame: &mut Frame, app: &App, calendar: &Calendar, area: Rect) {
    let start = calendar::week_start(calendar.cursor);
    let days: Vec<NaiveDate> = (0..7).map(|i| start + Days::new(i)).collect();
    let width = area.width as usize;
    let height = area.height as usize;
    // Show every item if they fit; otherwise give each day an equal share.
    let needed: usize = days.iter().map(|day| 1 + app.store.items(*day).len()).sum();
    let share = if needed <= height { usize::MAX } else { (height / 7).max(1) };

    let mut lines = Vec::new();
    for day in days {
        let mut header = vec![Span::raw(day.format("%a %-d %b").to_string())];
        if day == calendar.today {
            header.push(" (today)".fg(Color::Green));
        }
        let items = app.store.items(day);
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
    let items = app.store.items(day);
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
    let used = columns * MONTH_WIDTH + (columns - 1) * 2;
    let left = area.x + area.width.saturating_sub(used) / 2;
    let visible_rows = ((area.height + 1) / (MONTH_HEIGHT + 1)).max(1);
    let cursor_row = (calendar.cursor.month0() as u16) / columns;
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
        let first = NaiveDate::from_ymd_opt(calendar.cursor.year(), month, 1).expect("valid month");
        draw_year_month(frame, app, calendar, first, Rect::new(x, y, width, height));
    }
}

fn draw_year_month(frame: &mut Frame, app: &App, calendar: &Calendar, first: NaiveDate, rect: Rect) {
    let mut name = Line::from(first.format("%B").to_string()).centered();
    name = if first.month() == calendar.cursor.month() { name.cyan().bold() } else { name.bold() };
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

/// Open items as normal, completed ones crossed out.
fn item_style(item: &Item) -> Style {
    if item.done {
        Style::new().fg(Color::DarkGray).add_modifier(Modifier::CROSSED_OUT)
    } else {
        Style::new()
    }
}

/// The popup for typing an item to add to the calendar's selected day.
fn draw_adding(frame: &mut Frame, calendar: &Calendar) {
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
    frame.set_cursor_position(Position::new(inner.x + col as u16, inner.y + line as u16));
}

fn draw_search(frame: &mut Frame, app: &App, search: &Search) {
    let screen = frame.area();
    let area = centered(screen, screen.width.saturating_sub(4).min(80), screen.height.saturating_sub(2));
    let room = area.width.saturating_sub(2) as usize;
    let tag_title = search.tag.as_ref().map(|tag| truncate(&format!(" #{tag} "), room));
    let title = if let Some(title) = &tag_title {
        title.as_str()
    } else if search.notes {
        fit_first(&[" Search items and notes ", " Items and notes ", " Search "], room)
    } else {
        fit_first(&[" Search items ", " Search "], room)
    };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Color::Yellow))
        .title(title.bold())
        .title_bottom(
            Line::from(fit_first(&[" ↑/↓ select · ↵ go to item · esc close ", " ↵ go · esc close ", " esc close "], room))
                .centered()
                .dim(),
        )
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);

    let [prompt_area, rule, results] =
        Layout::vertical([Constraint::Length(1), Constraint::Length(1), Constraint::Fill(1)]).areas(inner);
    let prompt = "Search: ";
    let query = &search.input.text;
    let shown = if query.is_empty() {
        let hints: &[&str] = if search.tag.is_some() {
            &["type to narrow these down", "narrow down", ""]
        } else if search.notes {
            &["type to fuzzy find items and notes on any day", "items and notes, any day", "items and notes"]
        } else {
            &["type to fuzzy find items on any day", "items on any day", "any day"]
        };
        fit_first(hints, (prompt_area.width as usize).saturating_sub(prompt.width())).dark_gray()
    } else {
        query.as_str().into()
    };
    frame.render_widget(Line::from(vec![prompt.dim(), shown]), prompt_area);
    frame.render_widget("─".repeat(rule.width as usize).dark_gray(), rule);
    let typed = (prompt.width() + query[..search.input.cursor].width()) as u16;
    frame.set_cursor_position(Position::new(prompt_area.x + typed.min(prompt_area.width), prompt_area.y));

    if query.trim().is_empty() && search.tag.is_none() {
        return;
    }
    let hits = search.find(&app.store, app.today);
    if hits.is_empty() {
        frame.render_widget(Line::from(format!("No items match \"{query}\"")).dim(), results);
        return;
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
            let mut text = highlighted(&item.text, &hit.text_matches, text_width, item_style(item), match_style);
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

    // A heading per section, then each option with its description wrapped
    // underneath.
    let mut lines: Vec<Line> = Vec::new();
    let mut selected_lines = 0..0;
    let mut section = "";
    for (i, toggle) in TOGGLES.iter().enumerate() {
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

    // Where the folder is, and a way to move it. Where the cursor goes while
    // typing a new one: (line, column).
    let mut cursor = None;
    if let Some(folder) = &options.folder {
        let text_width = inner_width.saturating_sub(2).max(1);
        lines.push(Line::default());
        lines.push(Line::from("Data".bold()));
        let start = lines.len();
        let mut row = Line::from(vec!["Todoro folder: ".into(), truncate(folder, text_width.saturating_sub(15)).cyan()]);
        if options.selected == TOGGLES.len() {
            row = row.bg(SELECTED_BG);
        }
        lines.push(row);
        let about = "Where every workspace is kept. Enter to move them all to another folder.";
        lines.extend(wrap(about, text_width, usize::MAX).into_iter().map(|part| Line::from(format!("  {part}")).dim()));
        if let Some(input) = &options.editing {
            lines.push(Line::from("  Move to:"));
            let shown = truncate(&input.text, text_width.saturating_sub(2));
            cursor = Some((lines.len(), 4 + input.text[..input.cursor].width()));
            lines.push(Line::from(vec!["  › ".cyan().bold(), shown.into()]));
        }
        if let Some((message, ok)) = &options.message {
            let style = if *ok { Style::new().fg(Color::Green) } else { Style::new().fg(Color::Red) };
            for part in wrap(message, text_width, usize::MAX) {
                lines.push(Line::from(Span::styled(format!("  {part}"), style)));
            }
        }
        if options.selected == TOGGLES.len() {
            selected_lines = start..lines.len();
        }
    }

    let height = (lines.len() as u16 + 2).min(screen.height.saturating_sub(2));
    let area = centered(screen, width, height);
    let room = (width as usize).saturating_sub(2);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Color::Blue))
        .title(" Options ".bold())
        .title_bottom(
            Line::from(fit_first(&[" j/k move · space toggle · esc close ", " space toggle · esc ", " esc "], room))
                .centered()
                .dim(),
        )
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    // Scroll just enough to keep the selected option in view.
    let visible = inner.height as usize;
    let scroll = selected_lines.end.saturating_sub(visible).min(selected_lines.start);
    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(lines).block(block).scroll((scroll as u16, 0)), area);
    if let Some((line, col)) = cursor
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
    let title = if view.since.is_some() {
        fit_first(&[whats_new.as_str(), " What's new ", " New "], room)
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
    let keys_width = SECTIONS.iter().flat_map(|s| s.bindings).map(|(keys, _)| keys.width()).max().unwrap_or(0);
    let width = results.width as usize;
    let column = width.saturating_sub(keys_width + 4);
    let stacked = column < 16;
    let mut lines = Vec::new();
    for section in &matches {
        if !lines.is_empty() {
            lines.push(Line::default());
        }
        lines.push(Line::from(section.title.bold()));
        for (keys, action) in &section.bindings {
            if stacked {
                lines.push(Line::from(format!("  {keys}").yellow()));
                for part in wrap(action, width.saturating_sub(4).max(1), usize::MAX) {
                    lines.push(Line::from(format!("    {part}")));
                }
            } else {
                for (n, part) in wrap(action, column, usize::MAX).into_iter().enumerate() {
                    let keys = if n == 0 { keys } else { "" };
                    lines.push(Line::from(vec![format!("  {keys:<keys_width$}  ").yellow(), part.into()]));
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
    use crate::test_util::{app_with, press, type_str};

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
        for _ in 0..100 {
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
        app.store.set_notes(app.day, 1, "Ask for Q3 numbers".into()).unwrap();
        let mut out = String::new();
        let mut shot = |app: &App, name: &str| {
            out.push_str(&format!("{name}\n{}\n", render_sized(app, width, 12).backend()));
        };
        shot(&app, "list");
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
        type_str(&mut app, "l");
        shot(&app, "empty day");
        out
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
        type_str(&mut app, "c");
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
    fn search_results() {
        let (mut app, _dir) = search_app("sbu");
        press(&mut app, KeyCode::Down);
        let mut terminal = render_sized(&app, 70, 14);
        assert_snapshot!(terminal.backend());
        // The cursor follows the query: the 66-wide popup starts at x=2, then
        // border, padding, "Search: " and "bu".
        assert_eq!(terminal.get_cursor_position().unwrap(), Position::new(2 + 1 + 1 + 8 + 2, 2));
    }

    #[test]
    fn search_including_notes() {
        let (app, _dir) = search_app("Sdashboard");
        assert_snapshot!(render_sized(&app, 70, 14).backend());
    }

    #[test]
    fn search_empty_and_no_matches() {
        let (mut app, _dir) = search_app("S");
        let empty = render_sized(&app, 70, 8).backend().to_string();
        type_str(&mut app, "zzz");
        let none = render_sized(&app, 70, 8).backend().to_string();
        assert_snapshot!(format!("{empty}\n{none}"));
    }

    #[test]
    fn search_narrow() {
        let (app, _dir) = search_app("Sdashboard");
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
        type_str(&mut app, "oj");
        assert_snapshot!(render_sized(&app, 70, 16).backend());
    }

    #[test]
    fn options_popup_narrow() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        type_str(&mut app, "o");
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
        type_str(&mut app, "c");
        screens.push(render_sized(&app, 70, 24));
        press(&mut app, KeyCode::Esc);
        type_str(&mut app, "sfix");
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
        type_str(&mut app, "#j");
        assert_snapshot!(render_sized(&app, 60, 12).backend());
    }

    #[test]
    fn tag_list_when_there_are_none() {
        let (mut app, _dir) = app_with(&["Buy milk"]);
        type_str(&mut app, "#");
        assert_snapshot!(render_sized(&app, 40, 10).backend());
    }

    #[test]
    fn a_tags_items() {
        let (mut app, _dir) = app_with(&["Call #work about the #budget", "Water the plants #home"]);
        let later = app.day + chrono::Duration::days(5);
        app.store.insert(later, 0, "Send #Work report".into()).unwrap();
        type_str(&mut app, "#");
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
    fn the_selected_items_number_is_highlighted_even_if_triaged() {
        let (mut app, _dir) = app_with(&["one", "two", "three"]);
        app.store.cycle_priority(app.day, 2).unwrap();
        type_str(&mut app, "j");
        let number = |app: &App, y: u16| render(app).backend().buffer()[(1, y)].style();
        assert_eq!(number(&app, 2).fg, Some(Color::Yellow));
        assert!(number(&app, 2).add_modifier.contains(Modifier::BOLD));
        assert_eq!(number(&app, 1).fg, Some(Color::DarkGray));
        // A selected High item is highlighted too, and red again once left.
        assert_eq!(number(&app, 3).fg, Some(Color::Red));
        type_str(&mut app, "j");
        assert_eq!(number(&app, 3).fg, Some(Color::Yellow));
        assert_eq!(number(&app, 2).fg, Some(Color::DarkGray));
        // A selected completed item's tick is highlighted too.
        type_str(&mut app, "kxG");
        assert_eq!(render(&app).backend().buffer()[(1, 4)].symbol(), "✓");
        assert_eq!(number(&app, 4).fg, Some(Color::Yellow));
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
        type_str(&mut app, "Wj");
        assert_snapshot!(render_sized(&app, 60, 14).backend());
    }

    #[test]
    fn deleting_a_workspace() {
        let (mut app, _dir) = crate::test_util::app_with_workspaces(&["Home", "Work"]);
        type_str(&mut app, "WjdWor");
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
        type_str(&mut app, "o");
        let Mode::Options(options) = &mut app.mode else { panic!("the options") };
        // A fixed path, so the snapshot doesn't depend on where the test runs.
        options.folder = Some("~/todoro".into());
        type_str(&mut app, "jj");
        press(&mut app, KeyCode::Enter);
        assert_snapshot!(render_sized(&app, 70, 24).backend());
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
