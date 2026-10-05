use std::ops::Range;

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
use crate::store::{Item, Priority};
use crate::help::{Help, SECTIONS};
use crate::notes::NotesEditor;
use crate::search::Search;

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
        Mode::ConfirmDelete => draw_confirm(frame, app),
        Mode::Help { help, .. } => draw_help(frame, help),
        Mode::Calendar(calendar) if calendar.adding.is_some() => draw_adding(frame, calendar),
        Mode::Search(search) => draw_search(frame, app, search),
        _ => {}
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
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Line::from(title).centered())
        .title_bottom(Line::from(hint).centered().dim());

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
    if let Mode::Insert { index, input, editing } = &app.mode {
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
    let prefix_width = width + 2;
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
        let (prefix, prefix_style, text_style) = if row.done {
            let crossed = Style::new().fg(Color::DarkGray).add_modifier(Modifier::CROSSED_OUT);
            (format!("{:>width$}  ", "✓"), Style::new().fg(Color::DarkGray), crossed)
        } else {
            let number_style = row.priority.map_or(Style::new().fg(Color::DarkGray), priority_style);
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
                let lead = if n == 0 {
                    Span::styled(prefix.clone(), prefix_style)
                } else {
                    Span::raw(" ".repeat(prefix_width))
                };
                Line::from(vec![lead, Span::styled(row.text[range.clone()].trim_end().to_string(), text_style)])
            })
            .collect();
        let last = lines.last_mut().expect("wrap_ranges returns at least one line");
        if row.pinned {
            last.push_span(PINNED_MARKER.fg(Color::Cyan));
        }
        if row.has_notes {
            last.push_span(NOTES_MARKER.dim());
        }
        let style = if row.typing { Style::new().fg(Color::Yellow) } else { Style::new() };
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
    frame.render_widget(&editor.textarea, block.inner(area));
}

fn draw_status(frame: &mut Frame, app: &App, area: Rect) {
    // Each hint has a priority. On a narrow screen the lowest go first, so
    // `? help` is the last to go.
    let (mode, color, hints): (_, _, &[(&str, u8)]) = match &app.mode {
        Mode::Normal => (
            "NORMAL",
            Color::Blue,
            &[("a add", 4), ("e edit", 2), ("x done", 3), ("d delete", 1), ("↵ notes", 0), ("? help", 5)],
        ),
        Mode::Insert { editing: false, .. } => {
            ("INSERT", Color::Green, &[("←/→ move", 1), ("enter/esc save", 2), ("(empty discards)", 0)])
        }
        Mode::Insert { editing: true, .. } => {
            ("INSERT", Color::Green, &[("←/→ move", 1), ("enter/esc save", 2), ("(empty asks to delete)", 0)])
        }
        Mode::ConfirmDelete => ("DELETE", Color::Red, &[("d confirm", 1), ("c cancel", 1)]),
        Mode::Notes(editor) if editor.insert => ("INSERT", Color::Green, &[("esc normal mode", 0)]),
        Mode::Notes(_) => {
            ("NORMAL", Color::Blue, &[("i/a/o insert", 2), ("x delete", 1), ("dd delete line", 0), ("? help", 3)])
        }
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

fn draw_confirm(frame: &mut Frame, app: &App) {
    let items = app.items();
    let Some(item) = items.get(app.selected) else { return };
    // Same prefix as on the list: completed items aren't numbered.
    let prefix = if item.done { "✓  ".to_string() } else { format!("{}. ", app.selected + 1) };

    // Wrap long text under itself, growing the popup up to the screen height.
    let screen = frame.area();
    let width = screen.width.saturating_sub(4).min(50);
    let text_width = (width as usize).saturating_sub(2 + prefix.width()).max(1);
    // Room for the borders, a blank line and the d/c hint, and a line of margin.
    let max_lines = (screen.height as usize).saturating_sub(6).max(1);
    let lines = wrap(&item.text, text_width, max_lines);

    let indent = " ".repeat(prefix.width());
    let mut body: Vec<Line> = lines
        .into_iter()
        .enumerate()
        .map(|(i, line)| Line::from(format!("{}{line}", if i == 0 { &prefix } else { &indent })))
        .collect();
    body.push(Line::default());
    body.push(
        Line::from(vec!["d".bold().fg(Color::Red), " delete   ".into(), "c".bold(), " cancel".into()]).centered(),
    );

    let area = centered(screen, width, body.len() as u16 + 2);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Color::Red))
        .title(" Delete item? ".bold());
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
    let title = if search.notes {
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
        let hints: &[&str] = if search.notes {
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

    if query.trim().is_empty() {
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
        type_str(&mut app, "pjpjpx");
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
        for _ in 0..200 {
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
        type_str(&mut app, "tjttjjttt");
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
    fn delete_popup() {
        let (mut app, _dir) = app_with(&["Buy milk", "Write report"]);
        type_str(&mut app, "jd");
        assert_snapshot!(render(&app).backend());
    }
}
