use helix_core::graphemes::{next_grapheme_boundary, prev_grapheme_boundary};
use helix_core::unicode::width::{UnicodeWidthChar, UnicodeWidthStr};
use helix_core::Rope;
use helix_view::graphics::Rect;
use helix_view::input::{KeyCode, KeyEvent, KeyModifiers};
use helix_view::Theme;
use tui::buffer::Buffer as Surface;

/// A single-line, grapheme-aware text input used by the search pane.
#[derive(Debug, Clone)]
pub struct TextInput {
    value: String,
    /// Byte index at a grapheme boundary.
    cursor: usize,
}

impl TextInput {
    pub fn new(value: impl Into<String>) -> Self {
        let value = value.into();
        let cursor = value.len();
        Self { value, cursor }
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn set_value(&mut self, value: impl Into<String>) {
        self.value = value.into();
        self.cursor = self.value.len();
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Insert pasted text at the cursor. The widget is single-line, so line
    /// breaks are flattened into spaces.
    pub fn insert_str(&mut self, text: &str) {
        let text = text.replace("\r\n", " ").replace(['\r', '\n'], " ");
        if text.is_empty() {
            return;
        }
        self.value.insert_str(self.cursor, &text);
        self.cursor += text.len();
    }

    fn rope(&self) -> Rope {
        Rope::from_str(&self.value)
    }

    fn char_cursor(&self, rope: &Rope) -> usize {
        rope.byte_to_char(self.cursor.min(self.value.len()))
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char(c) if !control && !key.modifiers.contains(KeyModifiers::ALT) => {
                self.value.insert(self.cursor, c);
                self.cursor += c.len_utf8();
                true
            }
            KeyCode::Backspace if control => {
                self.delete_word_backward();
                true
            }
            KeyCode::Backspace => {
                let rope = self.rope();
                let char_cursor = self.char_cursor(&rope);
                let previous = prev_grapheme_boundary(rope.slice(..), char_cursor);
                let start = rope.char_to_byte(previous);
                self.value.replace_range(start..self.cursor, "");
                self.cursor = start;
                true
            }
            KeyCode::Delete => {
                let rope = self.rope();
                let char_cursor = self.char_cursor(&rope);
                let next = next_grapheme_boundary(rope.slice(..), char_cursor);
                let end = rope.char_to_byte(next);
                if end > self.cursor {
                    self.value.replace_range(self.cursor..end, "");
                }
                true
            }
            KeyCode::Left => {
                let rope = self.rope();
                let previous = prev_grapheme_boundary(rope.slice(..), self.char_cursor(&rope));
                self.cursor = rope.char_to_byte(previous);
                true
            }
            KeyCode::Right => {
                let rope = self.rope();
                let next = next_grapheme_boundary(rope.slice(..), self.char_cursor(&rope));
                self.cursor = rope.char_to_byte(next);
                true
            }
            KeyCode::Home => {
                self.cursor = 0;
                true
            }
            KeyCode::End => {
                self.cursor = self.value.len();
                true
            }
            KeyCode::Char('w') if control => {
                self.delete_word_backward();
                true
            }
            KeyCode::Char('u') if control => {
                self.value.clear();
                self.cursor = 0;
                true
            }
            _ => false,
        }
    }

    fn delete_word_backward(&mut self) {
        let mut start = self.cursor;
        while start > 0 {
            let previous = self.value[..start]
                .char_indices()
                .next_back()
                .map(|(index, _)| index)
                .unwrap_or(0);
            if !self.value[previous..start].chars().all(char::is_whitespace) {
                break;
            }
            start = previous;
        }
        let mut end = start;
        while start > 0 {
            let previous = self.value[..start]
                .char_indices()
                .next_back()
                .map(|(index, _)| index)
                .unwrap_or(0);
            if self.value[previous..start].chars().all(char::is_whitespace) {
                break;
            }
            start = previous;
            end = end.max(start);
        }
        self.value.replace_range(start..self.cursor, "");
        self.cursor = start;
    }

    pub fn render(&self, area: Rect, surface: &mut Surface, theme: &Theme, focused: bool) {
        let style = theme.get("ui.text");
        if area.height == 0 || area.width == 0 {
            return;
        }

        let width = area.width as usize;
        let cursor_column = self.value[..self.cursor.min(self.value.len())].width();
        // Keep the cursor visible and clip long queries at the pane edge
        // instead of letting them spill into the editor.
        let offset = cursor_column.saturating_sub(width.saturating_sub(1));
        let visible = skip_columns(&self.value, offset);
        surface.set_stringn(area.x, area.y, visible, width, style);

        if focused {
            let cursor_x = cursor_column.saturating_sub(offset);
            if cursor_x < width {
                let cursor_style = theme.get("ui.cursor");
                surface.set_style(
                    Rect::new(area.x + cursor_x as u16, area.y, 1, 1),
                    cursor_style,
                );
            }
        }
    }
}

fn skip_columns(text: &str, columns: usize) -> &str {
    let mut skipped = 0;
    for (index, ch) in text.char_indices() {
        if skipped >= columns {
            return &text[index..];
        }
        skipped += ch.width().unwrap_or(0);
    }
    ""
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent {
            code: KeyCode::Char(c),
            modifiers: KeyModifiers::CONTROL,
        }
    }

    #[test]
    fn inserts_and_moves_by_grapheme() {
        let mut input = TextInput::new("");
        for c in "aβ🎉".chars() {
            assert!(input.handle_key(key(KeyCode::Char(c))));
        }
        input.handle_key(key(KeyCode::Home));
        input.handle_key(key(KeyCode::Right));
        input.handle_key(key(KeyCode::Char('x')));
        assert_eq!(input.value(), "axβ🎉");
        assert_eq!(input.cursor(), "ax".len());
    }

    #[test]
    fn delete_word_and_clear() {
        let mut input = TextInput::new("one two three");
        input.handle_key(ctrl('w'));
        assert_eq!(input.value(), "one two ");
        input.handle_key(ctrl('u'));
        assert_eq!(input.value(), "");
    }
}
