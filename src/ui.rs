use ratatui::layout::{Constraint, Flex, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::app::{App, Mode};

pub fn draw(frame: &mut Frame, app: &App) {
    let [main, status] = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(frame.area());

    draw_list(frame, app, main);
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
    let mut rows: Vec<(&str, Style)> = app.items().iter().map(|text| (text.as_str(), Style::new())).collect();
    let mut state = ListState::default();
    if let Mode::Insert { index, text, editing, .. } = &app.mode {
        let row = (text.as_str(), Style::new().fg(Color::Yellow));
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
        .map(|(i, (text, style))| ListItem::new(Line::from(vec![number(i + 1), text.to_string().into()])).style(style))
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

fn draw_status(frame: &mut Frame, app: &App, area: Rect) {
    let (mode, color, hints) = match app.mode {
        Mode::Normal => ("NORMAL", Color::Blue, "a add  e edit  d delete  q quit"),
        Mode::Insert { editing: false, .. } => ("INSERT", Color::Green, "←/→ move  enter/esc save  (empty discards)"),
        Mode::Insert { editing: true, .. } => ("INSERT", Color::Green, "←/→ move  enter/esc save  (empty asks to delete)"),
        Mode::ConfirmDelete => ("DELETE", Color::Red, "d confirm  c cancel"),
    };
    let line = Line::from(vec![
        format!(" {mode} ").bold().fg(Color::Black).bg(color),
        "  ".into(),
        hints.dim(),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_confirm(frame: &mut Frame, app: &App) {
    let text = app.items().get(app.selected).map(String::as_str).unwrap_or_default();
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

