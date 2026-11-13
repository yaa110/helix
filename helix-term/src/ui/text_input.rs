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

    pub fn cursor_column(&self) -> usize {
        self.value[..self.cursor.min(self.value.len())].width()
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
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let sup = key.modifiers.contains(KeyModifiers::SUPER);

        match key.code {
            KeyCode::Char(c) if !control && !alt && !sup => {
                self.value.insert(self.cursor, c);
                self.cursor += c.len_utf8();
                true
            }
            KeyCode::Backspace | KeyCode::Delete if sup || alt => {
                self.value.clear();
                self.cursor = 0;
                true
            }
            KeyCode::Backspace if control => {
                self.delete_word_backward();
                true
            }
            KeyCode::Backspace => {
                self.delete_char_backward();
                true
            }
            KeyCode::Delete if control => {
                self.delete_word_forward();
                true
            }
            KeyCode::Delete => {
                self.delete_char_forward();
                true
            }
            KeyCode::Left if control || alt => {
                self.move_word_backward();
                true
            }
            KeyCode::Right if control || alt => {
                self.move_word_forward();
                true
            }
            KeyCode::Left => {
                self.move_char_backward();
                true
            }
            KeyCode::Right => {
                self.move_char_forward();
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
            KeyCode::Char('a') if control => {
                self.cursor = 0;
                true
            }
            KeyCode::Char('e') if control => {
                self.cursor = self.value.len();
                true
            }
            KeyCode::Char('b') if control || alt => {
                self.move_word_backward();
                true
            }
            KeyCode::Char('f') if control || alt => {
                self.move_word_forward();
                true
            }
            KeyCode::Char('d') if alt => {
                self.delete_word_forward();
                true
            }
            KeyCode::Char('w') if control => {
                self.delete_word_backward();
                true
            }
            KeyCode::Char('u') if control => {
                self.value.replace_range(..self.cursor, "");
                self.cursor = 0;
                true
            }
            KeyCode::Char('k') if control => {
                self.value.replace_range(self.cursor.., "");
                true
            }
            _ => false,
        }
    }

    fn move_char_backward(&mut self) {
        let rope = self.rope();
        let previous = prev_grapheme_boundary(rope.slice(..), self.char_cursor(&rope));
        self.cursor = rope.char_to_byte(previous);
    }

    fn move_char_forward(&mut self) {
        let rope = self.rope();
        let next = next_grapheme_boundary(rope.slice(..), self.char_cursor(&rope));
        self.cursor = rope.char_to_byte(next);
    }

    fn char_position(&self) -> usize {
        self.value
            .char_indices()
            .position(|(index, _)| index == self.cursor)
            .unwrap_or_else(|| self.value.chars().count())
    }

    fn move_word_backward(&mut self) {
        let chars: Vec<(usize, char)> = self.value.char_indices().collect();
        if chars.is_empty() {
            self.cursor = 0;
            return;
        }
        let mut position = self.char_position();
        if position == 0 {
            return;
        }
        while position > 0 && is_word_sep(chars[position - 1].1) {
            position -= 1;
        }
        while position > 0 && !is_word_sep(chars[position - 1].1) {
            position -= 1;
        }
        self.cursor = chars[position].0;
    }

    fn move_word_forward(&mut self) {
        let chars: Vec<(usize, char)> = self.value.char_indices().collect();
        if chars.is_empty() {
            self.cursor = 0;
            return;
        }
        let mut position = self.char_position();
        while position < chars.len() && !is_word_sep(chars[position].1) {
            position += 1;
        }
        while position < chars.len() && is_word_sep(chars[position].1) {
            position += 1;
        }
        self.cursor = chars
            .get(position)
            .map(|(index, _)| *index)
            .unwrap_or(self.value.len());
    }

    fn delete_char_backward(&mut self) {
        let before = self.cursor;
        self.move_char_backward();
        self.value.replace_range(self.cursor..before, "");
    }

    fn delete_char_forward(&mut self) {
        let before = self.cursor;
        self.move_char_forward();
        self.value.replace_range(before..self.cursor, "");
    }

    fn delete_word_backward(&mut self) {
        let before = self.cursor;
        self.move_word_backward();
        self.value.replace_range(self.cursor..before, "");
    }

    fn delete_word_forward(&mut self) {
        let before = self.cursor;
        self.move_word_forward();
        self.value.replace_range(before..self.cursor, "");
    }

    pub fn render(&self, area: Rect, surface: &mut Surface, theme: &Theme, _focused: bool) {
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
    }
}

fn is_word_sep(c: char) -> bool {
    c == std::path::MAIN_SEPARATOR || c.is_whitespace()
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

    #[test]
    fn alt_or_super_delete_clears_the_field() {
        for modifiers in [KeyModifiers::ALT, KeyModifiers::SUPER] {
            for code in [KeyCode::Backspace, KeyCode::Delete] {
                let mut input = TextInput::new("query");
                assert!(input.handle_key(KeyEvent { code, modifiers }));
                assert_eq!(input.value(), "");
                assert_eq!(input.cursor(), 0);
            }
        }
    }
}
