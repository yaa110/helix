use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};

use helix_view::editor::Action;
use helix_view::graphics::Rect;
use helix_view::input::KeyEvent;
use helix_view::keyboard::{KeyCode, KeyModifiers};
use helix_view::sidebar::{SidebarFocus, SidebarState};
use helix_view::Editor;
use tui::buffer::Buffer as Surface;

use crate::commands;
use crate::ui;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Dir,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub path: PathBuf,
    pub depth: usize,
    pub kind: EntryKind,
    pub expanded: bool,
}

#[derive(Debug, Clone)]
pub struct FileTree {
    pub root: PathBuf,
    pub rows: Vec<Row>,
    pub cursor: usize,
    pub scroll: usize,
}

impl FileTree {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            rows: Vec::new(),
            cursor: 0,
            scroll: 0,
        }
    }

    pub fn rebuild(&mut self, editor: &Editor, state: &SidebarState) {
        let root = self.root.clone();
        self.rebuild_with(&root, &state.expanded, |path| {
            ui::directory_entries(path, editor)
        });
    }

    pub fn rebuild_with(
        &mut self,
        root: &Path,
        expanded: &HashSet<PathBuf>,
        list: impl Fn(&Path) -> io::Result<Vec<(PathBuf, bool)>>,
    ) {
        fn push_dir(
            rows: &mut Vec<Row>,
            dir: &Path,
            depth: usize,
            expanded: &HashSet<PathBuf>,
            list: &impl Fn(&Path) -> io::Result<Vec<(PathBuf, bool)>>,
        ) {
            let Ok(entries) = list(dir) else {
                return;
            };
            for (path, is_dir) in entries {
                let is_expanded = is_dir && expanded.contains(&path);
                rows.push(Row {
                    path: path.clone(),
                    depth,
                    kind: if is_dir {
                        EntryKind::Dir
                    } else {
                        EntryKind::File
                    },
                    expanded: is_expanded,
                });
                if is_expanded {
                    push_dir(rows, &path, depth + 1, expanded, list);
                }
            }
        }

        let mut rows = Vec::new();
        push_dir(&mut rows, root, 0, expanded, &list);
        self.rows = rows;
        self.cursor = self.cursor.min(self.rows.len().saturating_sub(1));
        self.clamp_scroll();
    }

    pub fn refresh(&mut self, editor: &Editor, state: &SidebarState) {
        let previous = self.rows.get(self.cursor).cloned();
        self.rebuild(editor, state);

        if let Some(previous) = previous {
            if let Some(index) = self.rows.iter().position(|row| row.path == previous.path) {
                self.cursor = index;
            } else {
                self.cursor = self
                    .rows
                    .iter()
                    .rposition(|row| row.depth <= previous.depth)
                    .unwrap_or(0);
            }
        }
        self.clamp_scroll();
    }

    pub fn render(&mut self, area: Rect, surface: &mut Surface, editor: &Editor, focused: bool) {
        let theme = &editor.theme;
        let title = theme
            .try_get_exact("ui.sidebar.title")
            .or_else(|| theme.try_get_exact("ui.text"))
            .unwrap_or_default();
        let directory = theme
            .try_get_exact("ui.tree.directory")
            .or_else(|| theme.try_get_exact("ui.text.directory"))
            .unwrap_or_default();
        let menu = theme
            .try_get_exact("ui.tree.selected")
            .or_else(|| theme.try_get_exact("ui.selection"))
            .unwrap_or_default();

        if area.height == 0 {
            return;
        }

        let header = self
            .root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.root.display().to_string());
        surface.set_string(area.x, area.y, &header, title);

        let height = area.height.saturating_sub(1) as usize;
        if height == 0 {
            return;
        }
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        } else if self.cursor >= self.scroll + height {
            self.scroll = self.cursor + 1 - height;
        }

        for (offset, row) in self.rows.iter().skip(self.scroll).take(height).enumerate() {
            let index = self.scroll + offset;
            let marker = match row.kind {
                EntryKind::Dir if row.expanded => "▾",
                EntryKind::Dir => "▸",
                EntryKind::File => " ",
            };
            let name = row
                .path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| row.path.display().to_string());
            let text = format!("{}{} {}", "  ".repeat(row.depth), marker, name);
            let style = if focused && index == self.cursor {
                menu
            } else if row.kind == EntryKind::Dir {
                directory
            } else {
                title
            };
            surface.set_string(area.x, area.y + 1 + offset as u16, &text, style);
        }
    }

    fn selected_row(&self) -> Option<&Row> {
        self.rows.get(self.cursor)
    }

    fn select_path(&mut self, path: &Path) {
        if let Some(index) = self.rows.iter().position(|row| row.path == path) {
            self.cursor = index;
        }
    }

    fn rebuild_from_editor(&mut self, cx: &mut commands::Context) {
        let state = cx.editor.sidebar.clone();
        self.rebuild(cx.editor, &state);
    }

    fn open_selected(&mut self, cx: &mut commands::Context, action: Action) {
        let Some(row) = self.selected_row().cloned() else {
            return;
        };
        match row.kind {
            EntryKind::Dir => {
                self.toggle_expand(cx);
            }
            EntryKind::File => {
                if let Err(err) = cx.editor.open(&row.path, action) {
                    cx.editor.set_error(err.to_string());
                } else {
                    cx.editor.sidebar.focus = SidebarFocus::Editor;
                    cx.editor.sidebar.selected = Some(row.path);
                }
            }
        }
    }

    fn toggle_expand(&mut self, cx: &mut commands::Context) {
        let Some(row) = self.selected_row().cloned() else {
            return;
        };
        if row.kind != EntryKind::Dir {
            return;
        }
        let path = row.path;
        if cx.editor.sidebar.expanded.contains(&path) {
            cx.editor.sidebar.expanded.remove(&path);
        } else {
            cx.editor.sidebar.expanded.insert(path.clone());
        }
        self.rebuild_from_editor(cx);
        self.select_path(&path);
    }

    fn collapse_or_parent(&mut self, cx: &mut commands::Context) {
        let Some(row) = self.selected_row().cloned() else {
            return;
        };
        if row.kind == EntryKind::Dir && row.expanded {
            cx.editor.sidebar.expanded.remove(&row.path);
            self.rebuild_from_editor(cx);
            self.select_path(&row.path);
        } else if row.depth > 0 {
            let target_depth = row.depth - 1;
            self.cursor = self.rows[..self.cursor]
                .iter()
                .rposition(|candidate| candidate.depth == target_depth)
                .unwrap_or(0);
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent, cx: &mut commands::Context) -> bool {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                if !self.rows.is_empty() {
                    self.cursor = (self.cursor + 1).min(self.rows.len() - 1);
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.cursor = self.cursor.saturating_sub(1);
            }
            KeyCode::Char('l') | KeyCode::Right => {
                if let Some(row) = self.selected_row().cloned() {
                    if row.kind == EntryKind::Dir && !row.expanded {
                        self.toggle_expand(cx);
                    } else {
                        self.open_selected(cx, Action::Replace);
                    }
                }
            }
            KeyCode::Char('h') | KeyCode::Left => self.collapse_or_parent(cx),
            KeyCode::Enter | KeyCode::Char('o') => self.open_selected(cx, Action::Replace),
            KeyCode::Char('s') => self.open_selected(cx, Action::HorizontalSplit),
            KeyCode::Char('v') => self.open_selected(cx, Action::VerticalSplit),
            KeyCode::Char('g') => {
                self.cursor = 0;
            }
            KeyCode::Char('G') => {
                self.cursor = self.rows.len().saturating_sub(1);
            }
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.cursor = (self.cursor + 10).min(self.rows.len().saturating_sub(1));
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.cursor = self.cursor.saturating_sub(10);
            }
            KeyCode::Char('H') => {
                cx.editor.sidebar.expanded.clear();
                self.rebuild_from_editor(cx);
                self.cursor = 0;
            }
            KeyCode::Char('R') => {
                let state = cx.editor.sidebar.clone();
                self.refresh(cx.editor, &state);
            }
            KeyCode::Char('.') => {
                let current = {
                    let (view, doc) = helix_view::current_ref!(cx.editor);
                    let _ = view;
                    doc.path().map(ToOwned::to_owned)
                };
                if let Some(path) = current {
                    let mut ancestor = path.parent();
                    while let Some(parent) = ancestor {
                        if parent == self.root || !parent.starts_with(&self.root) {
                            break;
                        }
                        cx.editor.sidebar.expanded.insert(parent.to_path_buf());
                        ancestor = parent.parent();
                    }
                    self.rebuild_from_editor(cx);
                    self.select_path(&path);
                    cx.editor.sidebar.selected = Some(path);
                }
            }
            KeyCode::Char('[') => {
                let config = cx.editor.config().sidebar;
                cx.editor.sidebar.width =
                    cx.editor.sidebar.width.saturating_sub(2).max(config.min_width);
            }
            KeyCode::Char(']') => {
                let config = cx.editor.config().sidebar;
                cx.editor.sidebar.width =
                    (cx.editor.sidebar.width + 2).min(config.max_width);
            }
            KeyCode::Char('q') | KeyCode::Esc => {
                cx.editor.sidebar.focus = SidebarFocus::Editor;
            }
            _ => return false,
        }

        cx.editor.sidebar.selected =
            self.selected_row().map(|row| row.path.clone());
        cx.editor.needs_redraw = true;
        true
    }

    fn clamp_scroll(&mut self) {
        if self.rows.is_empty() {
            self.cursor = 0;
            self.scroll = 0;
        } else {
            self.cursor = self.cursor.min(self.rows.len() - 1);
            self.scroll = self.scroll.min(self.cursor);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list_dir(path: &Path) -> io::Result<Vec<(PathBuf, bool)>> {
        let mut entries: Vec<_> = std::fs::read_dir(path)?
            .map(|entry| {
                let entry = entry?;
                Ok((entry.path(), entry.path().is_dir()))
            })
            .collect::<io::Result<_>>()?;
        entries.sort_by(|(a, ad), (b, bd)| {
            (!ad, a.to_string_lossy().to_lowercase())
                .cmp(&(!bd, b.to_string_lossy().to_lowercase()))
        });
        Ok(entries)
    }

    #[test]
    fn lists_directories_first_and_respects_expansion() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "").unwrap();
        std::fs::write(dir.path().join("README.md"), "").unwrap();

        let mut tree = FileTree::new(dir.path().to_path_buf());
        let mut expanded = HashSet::new();
        tree.rebuild_with(dir.path(), &expanded, list_dir);
        assert_eq!(tree.rows[0].kind, EntryKind::Dir);
        assert_eq!(tree.rows[0].path.file_name().unwrap(), "src");
        assert_eq!(tree.rows[1].path.file_name().unwrap(), "README.md");

        expanded.insert(dir.path().join("src"));
        tree.rebuild_with(dir.path(), &expanded, list_dir);
        assert_eq!(tree.rows.len(), 3);
        assert_eq!(tree.rows[1].depth, 1);
        assert_eq!(tree.rows[1].path.file_name().unwrap(), "main.rs");
    }
}
