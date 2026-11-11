use std::io::Write;
use std::path::{Path, PathBuf};

use helix_core::RopeSlice;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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

/// Persistent per-workspace session storage.
#[derive(Clone, Debug)]
pub struct SessionStore {
    dir: PathBuf,
}

impl Default for SessionStore {
    fn default() -> Self {
        Self {
            dir: helix_loader::data_dir().join("sessions"),
        }
    }
}

impl SessionStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn path_for(&self, workspace: &Path) -> PathBuf {
        let mut hasher = Sha256::new();
        hasher.update(workspace.as_os_str().to_string_lossy().as_bytes());
        let digest = hasher.finalize();
        let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
        self.dir.join(format!("{hex}.json"))
    }

    pub fn save(&self, session: &Session) -> anyhow::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let path = self.path_for(&session.workspace);
        let mut tmp = tempfile::NamedTempFile::new_in(&self.dir)?;
        serde_json::to_writer_pretty(&mut tmp, session)?;
        tmp.flush()?;
        tmp.as_file().sync_all()?;

        #[cfg(windows)]
        if path.exists() {
            // Windows cannot atomically replace an existing file with rename.
            std::fs::remove_file(&path)?;
        }
        tmp.persist(&path).map_err(|err| err.error)?;
        Ok(())
    }

    pub fn load(&self, workspace: &Path) -> Option<Session> {
        let path = self.path_for(workspace);
        let contents = match std::fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return None,
            Err(err) => {
                log::warn!("failed to read session {:?}: {err}", path);
                return None;
            }
        };

        let parsed = serde_json::from_str::<Session>(&contents)
            .ok()
            .filter(|session| session.version == SESSION_VERSION);
        if parsed.is_some() {
            return parsed;
        }

        let backup = path.with_extension("json.bak");
        if let Err(err) = std::fs::rename(&path, &backup) {
            log::warn!("failed to back up corrupt session {:?}: {err}", path);
        } else {
            log::warn!("ignored corrupt session {:?} (backed up to {:?})", path, backup);
        }
        None
    }

    pub fn clear(&self, workspace: &Path) -> anyhow::Result<()> {
        let path = self.path_for(workspace);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use helix_core::Rope;
    use std::path::Path;

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

    #[test]
    fn store_round_trips_and_backs_up_corrupt_files() {
        let tmp = tempfile::tempdir().unwrap();
        let store = SessionStore::new(tmp.path().to_path_buf());
        let workspace = tmp.path().join("ws");
        let session = Session {
            workspace: workspace.clone(),
            ..Session::default()
        };

        store.save(&session).unwrap();
        assert_eq!(store.load(&workspace), Some(session));

        std::fs::write(store.path_for(&workspace), b"{not json").unwrap();
        assert_eq!(store.load(&workspace), None);
        assert!(store.path_for(&workspace).with_extension("json.bak").exists());

        store
            .save(&Session {
                workspace: workspace.clone(),
                ..Session::default()
            })
            .unwrap();
        store.clear(&workspace).unwrap();
        assert!(!store.path_for(&workspace).exists());
    }

    #[test]
    fn session_filename_is_stable_for_the_same_workspace() {
        let store = SessionStore::new("/tmp/sessions".into());
        let a = store.path_for(Path::new("/tmp/project"));
        let b = store.path_for(Path::new("/tmp/project"));
        let c = store.path_for(Path::new("/tmp/other"));
        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}
