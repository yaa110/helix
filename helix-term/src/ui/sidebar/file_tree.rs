use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};

use helix_view::graphics::Rect;
use helix_view::sidebar::SidebarState;
use helix_view::Editor;
use tui::buffer::Buffer as Surface;

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
