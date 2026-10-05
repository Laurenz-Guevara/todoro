use std::ops::Range;

use ratatui::layout::{Constraint, Flex, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, List, ListItem, ListState, Padding, Paragraph};
use ratatui::Frame;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{App, Mode};
use crate::help::{Help, SECTIONS};
use crate::notes::NotesEditor;

pub fn draw(frame: &mut Frame, app: &App) {
    let [main, status] = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(frame.area());

    // The help popup is drawn over whichever screen it was opened from.
    let screen = match &app.mode {
        Mode::Help { back, .. } => back.as_ref(),
        mode => mode,
    };
    match screen {
        Mode::Notes(editor) => draw_notes(frame, app, editor, main),
        _ => draw_list(frame, app, main),
    }
    draw_status(frame, app, status);

    match &app.mode {
        Mode::ConfirmDelete => draw_confirm(frame, app),
        Mode::Help { help, .. } => draw_help(frame, help),
        _ => {}
    }
}

fn draw_list(frame: &mut Frame, app: &App, area: Rect) {
    let mut title = vec![" ".into(), app.day.format("%A, %B %-d %Y").to_string().bold()];
    if app.day == app.today {
        title.push(" (today)".fg(Color::Green));
    }
    title.push(" ".into());

    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Line::from(title).centered())
        .title_bottom(Line::from(" h ← prev day · k ↑ up · j ↓ down · next day → l ").centered().dim());

    // The rows to show, with the item being typed in place of (or inserted
    // among) the saved ones, so the numbering below it is already right.
    let mut rows: Vec<Row> = app
        .items()
        .into_iter()
        .map(|item| Row {
            text: &item.text,
            pinned: item.pinned,
            has_notes: !item.notes.is_empty(),
            done: item.done,
            typing: false,
        })
        .collect();
    let mut selected = (!rows.is_empty()).then_some(app.selected);
    if let Mode::Insert { index, text, editing, .. } = &app.mode {
        if *editing {
            rows[*index] = Row { text, typing: true, ..rows[*index] };
        } else {
            rows.insert(*index, Row { text, pinned: false, has_notes: false, done: false, typing: true });
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
        let (prefix, text_style) = if row.done {
            (format!("{:>width$}  ", "✓"), Style::new().fg(Color::DarkGray).add_modifier(Modifier::CROSSED_OUT))
        } else {
            (format!("{:>width$}. ", i + 1), Style::new())
        };

        // Wrap long text under itself, leaving room for the markers at the end.
        let markers = [(row.pinned, PINNED_MARKER), (row.has_notes, NOTES_MARKER)];
        let markers_width: usize = markers.iter().filter(|(shown, _)| *shown).map(|(_, m)| m.width()).sum();
        let text_width = (inner.width as usize).saturating_sub(prefix_width + markers_width).max(1);
        let ranges = wrap_ranges(row.text, text_width);
        if let (true, Mode::Insert { cursor, .. }) = (row.typing, &app.mode) {
            typing_cursor = cursor_position(row.text, &ranges, *cursor, text_width);
        }

        let mut lines: Vec<Line> = ranges
            .iter()
            .enumerate()
            .map(|(n, range)| {
                let lead = if n == 0 { prefix.clone() } else { " ".repeat(prefix_width) };
                Line::from(vec![
                    Span::styled(lead, Style::new().fg(Color::DarkGray)),
                    Span::styled(row.text[range.clone()].trim_end().to_string(), text_style),
                ])
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
    let mut state = ListState::default().with_selected(selected.map(list_row));

    if items.is_empty() {
        let empty = Paragraph::new("Nothing to do. Press a to add an item.").dim().centered();
        frame.render_widget(block, area);
        frame.render_widget(empty, inner);
        return;
    }

    let list = List::new(items)
        .block(block)
        .highlight_style(Style::new().bg(Color::Rgb(50, 50, 60)).add_modifier(Modifier::BOLD));
    frame.render_stateful_widget(list, area, &mut state);

    if let Mode::Insert { index, .. } = &app.mode {
        let (line, col) = typing_cursor;
        let row: usize = heights[state.offset()..list_row(*index)].iter().sum::<usize>() + line;
        frame.set_cursor_position(Position::new(inner.x + (prefix_width + col) as u16, inner.y + row as u16));
    }
}

/// One item row on the main list.
struct Row<'a> {
    text: &'a str,
    pinned: bool,
    has_notes: bool,
    done: bool,
    /// The item being added or edited.
    typing: bool,
}

/// Shown after an item on the main list when it has notes.
const NOTES_MARKER: &str = " ≡";

/// Shown after a pinned item, which moves forward to today until completed.
const PINNED_MARKER: &str = " ⚲";

fn draw_notes(frame: &mut Frame, app: &App, editor: &NotesEditor, area: Rect) {
    let title = format!(" {}. {} ", app.selected + 1, app.items()[app.selected].text);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Line::from(title.bold()).centered())
        .title_bottom(Line::from(" esc/q back to list ").centered().dim())
        .padding(Padding::horizontal(1));
    frame.render_widget(&block, area);
    frame.render_widget(&editor.textarea, block.inner(area));
}

fn draw_status(frame: &mut Frame, app: &App, area: Rect) {
    let (mode, color, hints) = match &app.mode {
        Mode::Normal => ("NORMAL", Color::Blue, "a add  e edit  x done  d delete  ↵ notes  ? help"),
        Mode::Insert { editing: false, .. } => ("INSERT", Color::Green, "←/→ move  enter/esc save  (empty discards)"),
        Mode::Insert { editing: true, .. } => ("INSERT", Color::Green, "←/→ move  enter/esc save  (empty asks to delete)"),
        Mode::ConfirmDelete => ("DELETE", Color::Red, "d confirm  c cancel"),
        Mode::Notes(editor) if editor.insert => ("INSERT", Color::Green, "esc normal mode"),
        Mode::Notes(_) => ("NORMAL", Color::Blue, "i/a/o insert  x delete  dd delete line  ? help"),
        Mode::Help { .. } => ("HELP", Color::Magenta, "type to search  ↑/↓ scroll  esc close"),
    };
    let line = Line::from(vec![
        format!(" {mode} ").bold().fg(Color::Black).bg(color),
        "  ".into(),
        hints.dim(),
    ]);
    frame.render_widget(Paragraph::new(line), area);
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

fn draw_help(frame: &mut Frame, help: &Help) {
    let screen = frame.area();
    let area = centered(screen, screen.width.saturating_sub(4).min(72), screen.height.saturating_sub(2));
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Color::Magenta))
        .title(" Keybindings ".bold())
        .title_bottom(Line::from(" ↑/↓ scroll · esc close ").centered().dim())
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);

    let [search, rule, results] =
        Layout::vertical([Constraint::Length(1), Constraint::Length(1), Constraint::Fill(1)]).areas(inner);
    let prompt = "Search: ";
    let query = if help.query.is_empty() {
        "type a key like x, or a word like undo".dark_gray()
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
    // doesn't shift as you search.
    let keys_width = SECTIONS.iter().flat_map(|s| s.bindings).map(|(keys, _)| keys.chars().count()).max().unwrap_or(0);
    let mut lines = Vec::new();
    for section in &matches {
        if !lines.is_empty() {
            lines.push(Line::default());
        }
        lines.push(Line::from(section.title.bold()));
        for (keys, action) in &section.bindings {
            lines.push(Line::from(vec![format!("  {keys:<keys_width$}  ").yellow(), (*action).into()]));
        }
    }

    let height = results.height as usize;
    help.height.set(height);
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

    #[test]
    fn delete_popup() {
        let (mut app, _dir) = app_with(&["Buy milk", "Write report"]);
        type_str(&mut app, "jd");
        assert_snapshot!(render(&app).backend());
    }
}
