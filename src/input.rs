//! A single line of text being typed, with a cursor that can move within it.

use ratatui::crossterm::event::KeyCode;

#[derive(Debug, Default)]
pub struct LineInput {
    pub text: String,
    /// Byte offset into `text`, always on a char boundary.
    pub cursor: usize,
}

impl LineInput {
    /// Starts with `text` and the cursor at its end.
    pub fn new(text: &str) -> Self {
        Self { text: text.to_string(), cursor: text.len() }
    }

    /// Handles a key and returns `true` when typing is finished (Enter or Esc).
    pub fn handle_key(&mut self, code: KeyCode) -> bool {
        let (text, cursor) = (&mut self.text, &mut self.cursor);
        match code {
            KeyCode::Esc | KeyCode::Enter => return true,
            KeyCode::Left => *cursor = prev_boundary(text, *cursor),
            KeyCode::Right => *cursor = next_boundary(text, *cursor),
            KeyCode::Home => *cursor = 0,
            KeyCode::End => *cursor = text.len(),
            KeyCode::Backspace if *cursor > 0 => {
                *cursor = prev_boundary(text, *cursor);
                text.remove(*cursor);
            }
            KeyCode::Delete if *cursor < text.len() => {
                text.remove(*cursor);
            }
            KeyCode::Char(c) => {
                text.insert(*cursor, c);
                *cursor += c.len_utf8();
            }
            _ => {}
        }
        false
    }
}

fn prev_boundary(text: &str, cursor: usize) -> usize {
    text[..cursor].char_indices().next_back().map_or(0, |(i, _)| i)
}

fn next_boundary(text: &str, cursor: usize) -> usize {
    text[cursor..].chars().next().map_or(cursor, |c| cursor + c.len_utf8())
}
