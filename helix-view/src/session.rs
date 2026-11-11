use std::path::PathBuf;

use helix_core::RopeSlice;
use serde::{Deserialize, Serialize};

use crate::sidebar::{SearchInputs, SidebarConfig, SidebarMode};

pub const SESSION_VERSION: u32 = 1;

/// A character position stored as a line and a character column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "(usize, usize)", into = "(usize, usize)")]
pub struct Point {
    pub line: usize,
    pub col: usize,
}

impl From<(usize, usize)> for Point {
    fn from((line, col): (usize, usize)) -> Self {
        Self { line, col }
    }
}

impl From<Point> for (usize, usize) {
    fn from(point: Point) -> Self {
        (point.line, point.col)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionState {
    pub anchor: Point,
    pub head: Point,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScrollState {
    pub anchor_line: usize,
    pub vertical_offset: usize,
    pub horizontal_offset: usize,
}

impl Default for ScrollState {
    fn default() -> Self {
        Self {
            anchor_line: 0,
            vertical_offset: 0,
            horizontal_offset: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BufferState {
    pub path: PathBuf,
    pub selections: Vec<SelectionState>,
    pub primary: usize,
    pub scroll: ScrollState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SidebarSession {
    pub mode: SidebarMode,
    pub width: u16,
    pub expanded: Vec<PathBuf>,
    pub selected: Option<PathBuf>,
}

impl Default for SidebarSession {
    fn default() -> Self {
        Self {
            mode: SidebarMode::Hidden,
            width: SidebarConfig::default().width,
            expanded: Vec::new(),
            selected: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Session {
    pub version: u32,
    pub workspace: PathBuf,
    pub focused: Option<PathBuf>,
    pub buffers: Vec<BufferState>,
    pub sidebar: SidebarSession,
    pub search: SearchInputs,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            version: SESSION_VERSION,
            workspace: PathBuf::new(),
            focused: None,
            buffers: Vec::new(),
            sidebar: SidebarSession::default(),
            search: SearchInputs::default(),
        }
    }
}

/// Encode a character index as a line + character column.
pub fn encode_point(text: RopeSlice<'_>, char_idx: usize) -> Point {
    let char_idx = char_idx.min(text.len_chars());
    let line = text.char_to_line(char_idx);
    let line_start = text.line_to_char(line);
    Point {
        line,
        col: char_idx - line_start,
    }
}

/// Decode a line + character column, clamping to the current text. A column
/// that points at the line's newline is clamped to the end of the line's
/// contents so restoring a cursor does not jump to the next line.
pub fn decode_point(text: RopeSlice<'_>, point: Point) -> usize {
    if text.len_chars() == 0 {
        return 0;
    }
    if point.line >= text.len_lines() {
        return text.len_chars();
    }
    let line = point.line;
    let start = text.line_to_char(line);
    let mut end = if line + 1 < text.len_lines() {
        text.line_to_char(line + 1)
    } else {
        text.len_chars()
    };
    while end > start && matches!(text.char(end - 1), '\n' | '\r') {
        end -= 1;
    }
    start + point.col.min(end - start)
}

#[cfg(test)]
mod tests {
    use super::*;
    use helix_core::Rope;

    #[test]
    fn point_round_trip_handles_unicode_and_clamping() {
        let rope = Rope::from_str("alpha\nβeta\n🎉 end");
        let idx = rope.line_to_char(1) + 2; // after 'β'
        assert_eq!(
            decode_point(rope.slice(..), encode_point(rope.slice(..), idx)),
            idx
        );

        let clamped = decode_point(rope.slice(..), Point { line: 0, col: 999 });
        assert_eq!(clamped, rope.line_to_char(1) - 1); // end of first line contents

        let past_end = decode_point(rope.slice(..), Point { line: 999, col: 0 });
        assert_eq!(past_end, rope.len_chars());
    }

    #[test]
    fn session_json_round_trip() {
        let session = Session {
            version: 1,
            workspace: "/tmp/ws".into(),
            focused: Some("src/main.rs".into()),
            buffers: vec![BufferState {
                path: "src/main.rs".into(),
                selections: vec![SelectionState {
                    anchor: Point { line: 1, col: 2 },
                    head: Point { line: 3, col: 4 },
                }],
                primary: 0,
                scroll: ScrollState {
                    anchor_line: 1,
                    vertical_offset: 0,
                    horizontal_offset: 0,
                },
            }],
            sidebar: SidebarSession::default(),
            search: SearchInputs::default(),
        };
        let json = serde_json::to_string(&session).unwrap();
        assert_eq!(serde_json::from_str::<Session>(&json).unwrap(), session);
    }
}
