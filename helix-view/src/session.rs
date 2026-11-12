use std::io::Write;
use std::path::{Path, PathBuf};

use helix_core::{Range, RopeSlice, Selection, SmallVec};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::editor::Action;
use crate::sidebar::{SearchInputs, SidebarConfig, SidebarMode};
use crate::view::ViewPosition;
use crate::{DocumentId, Editor};

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
            log::warn!(
                "ignored corrupt session {:?} (backed up to {:?})",
                path,
                backup
            );
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

fn encode_path(root: &Path, path: &Path) -> PathBuf {
    path.strip_prefix(root)
        .map(Path::to_path_buf)
        .unwrap_or_else(|_| path.to_path_buf())
}

fn decode_path(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

/// Snapshot the current editor state for session persistence.
pub fn collect(editor: &Editor) -> Session {
    let workspace = editor.workspace_root.clone();
    let (view_id, focused) = {
        let (view, doc) = crate::current_ref!(editor);
        (
            view.id,
            doc.path().map(|path| encode_path(&workspace, path)),
        )
    };

    let mut buffers = Vec::new();
    for doc in editor.documents() {
        let Some(path) = doc.path() else {
            continue;
        };
        let text = doc.text().slice(..);
        let (selections, primary, scroll) = match doc.selections().get(&view_id) {
            Some(selection) => {
                let selections = selection
                    .ranges()
                    .iter()
                    .map(|range| SelectionState {
                        anchor: encode_point(text, range.anchor),
                        head: encode_point(text, range.head),
                    })
                    .collect();
                let offset = doc.view_offset(view_id);
                let anchor_line = text.char_to_line(offset.anchor.min(text.len_chars()));
                (
                    selections,
                    selection.primary_index(),
                    ScrollState {
                        anchor_line,
                        vertical_offset: offset.vertical_offset,
                        horizontal_offset: offset.horizontal_offset,
                    },
                )
            }
            None => (
                vec![SelectionState {
                    anchor: Point { line: 0, col: 0 },
                    head: Point { line: 0, col: 0 },
                }],
                0,
                ScrollState::default(),
            ),
        };

        buffers.push(BufferState {
            path: encode_path(&workspace, path),
            selections,
            primary,
            scroll,
        });
    }

    Session {
        version: SESSION_VERSION,
        workspace,
        focused,
        buffers,
        sidebar: SidebarSession {
            mode: editor.sidebar.mode,
            width: editor.sidebar.width,
            expanded: editor
                .sidebar
                .expanded
                .iter()
                .map(|path| encode_path(&editor.workspace_root, path))
                .collect(),
            selected: editor
                .sidebar
                .selected
                .as_ref()
                .map(|path| encode_path(&editor.workspace_root, path)),
        },
        search: editor.sidebar.search.clone(),
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RestoreOutcome {
    pub restored: usize,
    pub skipped: Vec<PathBuf>,
}

/// Restore buffers, selections, scroll, and sidebar state from a session.
pub fn restore(editor: &mut Editor, session: &Session) -> RestoreOutcome {
    let mut outcome = RestoreOutcome::default();
    let mut restored: Vec<(DocumentId, &BufferState)> = Vec::new();

    // `restore` may run before Application creates the initial scratch view.
    let scratch = if editor.tree.views().next().is_none() {
        Some(editor.new_file(Action::VerticalSplit))
    } else {
        None
    };

    for buffer in &session.buffers {
        let path = decode_path(&editor.workspace_root, &buffer.path);
        if !path.exists() {
            outcome.skipped.push(path);
            continue;
        }
        match editor.open(&path, Action::Load) {
            Ok(doc_id) => {
                outcome.restored += 1;
                restored.push((doc_id, buffer));
            }
            Err(_) => outcome.skipped.push(path),
        }
    }

    if outcome.restored > 0 {
        if let Some(scratch) = scratch {
            let _ = editor.close_document(scratch, true);
        }
    }

    let view_id = crate::view!(editor).id;
    for (doc_id, buffer) in &restored {
        let doc = crate::doc_mut!(editor, doc_id);
        let (selection, offset) = {
            let text = doc.text().slice(..);
            let ranges: SmallVec<[Range; 1]> = buffer
                .selections
                .iter()
                .map(|selection| {
                    Range::new(
                        decode_point(text, selection.anchor),
                        decode_point(text, selection.head),
                    )
                })
                .collect();
            let primary = buffer.primary.min(ranges.len().saturating_sub(1));
            let anchor = text.line_to_char(
                buffer
                    .scroll
                    .anchor_line
                    .min(text.len_lines().saturating_sub(1)),
            );
            (
                Selection::new(ranges, primary),
                ViewPosition {
                    anchor,
                    vertical_offset: buffer.scroll.vertical_offset,
                    horizontal_offset: buffer.scroll.horizontal_offset,
                },
            )
        };
        doc.set_selection(view_id, selection);
        doc.set_view_offset(view_id, offset);
    }

    let focused_id = session
        .focused
        .as_ref()
        .map(|path| decode_path(&editor.workspace_root, path))
        .and_then(|path| editor.document_id_by_path(&path))
        .or_else(|| restored.first().map(|(id, _)| *id));
    if let Some(doc_id) = focused_id {
        // Do not use `Editor::switch(..., Action::Replace)` here: it dispatches
        // `DocumentFocusLost`, whose handlers can block on the async runtime
        // while `Application::new` is still constructing the editor. Point the
        // existing view at the restored document directly.
        let view = crate::view_mut!(editor);
        view.doc = doc_id;
        let doc = crate::doc_mut!(editor, &doc_id);
        doc.ensure_view_init(view.id);
        doc.mark_as_focused();
    }

    editor.sidebar.mode = session.sidebar.mode;
    editor.sidebar.width = session.sidebar.width;
    editor.sidebar.expanded = session
        .sidebar
        .expanded
        .iter()
        .map(|path| decode_path(&editor.workspace_root, path))
        .collect();
    editor.sidebar.selected = session
        .sidebar
        .selected
        .as_ref()
        .map(|path| decode_path(&editor.workspace_root, path));
    editor.sidebar.search = session.search.clone();

    // Do not call `ensure_cursor_in_view` here: it would re-scroll to satisfy
    // scrolloff and override the saved viewport. The saved selection and
    // viewport were captured together and are consistent.

    outcome
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
        assert!(store
            .path_for(&workspace)
            .with_extension("json.bak")
            .exists());

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
