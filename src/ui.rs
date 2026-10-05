use ratatui::layout::{Constraint, Flex, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, List, ListItem, ListState, Padding, Paragraph};
use ratatui::Frame;

use crate::app::{App, Mode};
use crate::notes::NotesEditor;

pub fn draw(frame: &mut Frame, app: &App) {
    let [main, status] = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(frame.area());

    match &app.mode {
        Mode::Notes(editor) => draw_notes(frame, app, editor, main),
        _ => draw_list(frame, app, main),
    }
    draw_status(frame, app, status);

    if let Mode::ConfirmDelete = app.mode {
        draw_confirm(frame, app);
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
    let mut rows: Vec<(&str, bool, Style)> =
        app.items().iter().map(|item| (item.text.as_str(), !item.notes.is_empty(), Style::new())).collect();
    let mut state = ListState::default();
    if let Mode::Insert { index, text, editing, .. } = &app.mode {
        let has_notes = *editing && !app.items()[*index].notes.is_empty();
        let row = (text.as_str(), has_notes, Style::new().fg(Color::Yellow));
        if *editing {
            rows[*index] = row;
        } else {
            rows.insert(*index, row);
        }
        state.select(Some(*index));
    } else if !rows.is_empty() {
        state.select(Some(app.selected));
    }

    // Pad numbers so text lines up once there are 10 or more items.
    let width = rows.len().to_string().len();
    let number = |n: usize| Span::styled(format!("{n:>width$}. "), Style::new().fg(Color::DarkGray));
    let rows: Vec<ListItem> = rows
        .into_iter()
        .enumerate()
        .map(|(i, (text, has_notes, style))| {
            let mut line = Line::from(vec![number(i + 1), text.to_string().into()]);
            if has_notes {
                line.push_span(NOTES_MARKER.dim());
            }
            ListItem::new(line).style(style)
        })
        .collect();

    let inner = block.inner(area);
    if rows.is_empty() {
        let empty = Paragraph::new("Nothing to do. Press a to add an item.").dim().centered();
        frame.render_widget(block, area);
        frame.render_widget(empty, inner);
        return;
    }

    let list = List::new(rows)
        .block(block)
        .highlight_style(Style::new().bg(Color::Rgb(50, 50, 60)).add_modifier(Modifier::BOLD));
    frame.render_stateful_widget(list, area, &mut state);

    if let Mode::Insert { index, text, cursor, .. } = &app.mode {
        let row = (*index - state.offset()) as u16;
        let col = (width + 2 + text[..*cursor].chars().count()) as u16;
        frame.set_cursor_position(Position::new(inner.x + col, inner.y + row));
    }
}

/// Shown after an item on the main list when it has notes.
const NOTES_MARKER: &str = " ≡";

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
        Mode::Normal => ("NORMAL", Color::Blue, "a add  e edit  d delete  enter notes  q quit"),
        Mode::Insert { editing: false, .. } => ("INSERT", Color::Green, "←/→ move  enter/esc save  (empty discards)"),
        Mode::Insert { editing: true, .. } => ("INSERT", Color::Green, "←/→ move  enter/esc save  (empty asks to delete)"),
        Mode::ConfirmDelete => ("DELETE", Color::Red, "d confirm  c cancel"),
        Mode::Notes(editor) if editor.insert => ("INSERT", Color::Green, "esc normal mode"),
        Mode::Notes(_) => ("NORMAL", Color::Blue, "i/a/o insert  x delete  dd delete line  u undo"),
    };
    let line = Line::from(vec![
        format!(" {mode} ").bold().fg(Color::Black).bg(color),
        "  ".into(),
        hints.dim(),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_confirm(frame: &mut Frame, app: &App) {
    let text = app.items().get(app.selected).map(|item| item.text.as_str()).unwrap_or_default();
    let area = centered(frame.area(), 50, 5);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Color::Red))
        .title(" Delete item? ".bold());
    let body = vec![
        Line::from(format!("{}. {text}", app.selected + 1)),
        Line::default(),
        Line::from(vec!["d".bold().fg(Color::Red), " delete   ".into(), "c".bold(), " cancel".into()]).centered(),
    ];
    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(body).block(block), area);
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
        let mut terminal = Terminal::new(TestBackend::new(60, 10)).unwrap();
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
    fn delete_popup() {
        let (mut app, _dir) = app_with(&["Buy milk", "Write report"]);
        type_str(&mut app, "jd");
        assert_snapshot!(render(&app).backend());
    }
}
