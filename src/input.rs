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

    /// Inserts pasted text at the cursor. This is one line, so line breaks
    /// become spaces.
    pub fn paste(&mut self, text: &str) {
        let text = text.lines().map(str::trim_end).filter(|line| !line.is_empty()).collect::<Vec<_>>().join(" ");
        self.text.insert_str(self.cursor, &text);
        self.cursor += text.len();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pasting_inserts_at_the_cursor_on_one_line() {
        let mut input = LineInput::new("ab");
        input.handle_key(KeyCode::Left);
        input.paste("first line\r\nsecond line  \n\nthird");
        assert_eq!(input.text, "afirst line second line thirdb");
        assert_eq!(input.cursor, "afirst line second line third".len());
    }
}
