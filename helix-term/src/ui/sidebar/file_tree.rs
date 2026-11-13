use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use helix_core::fuzzy::fuzzy_match;
use helix_core::unicode::width::{UnicodeWidthChar, UnicodeWidthStr};
use helix_view::editor::Action;
use helix_view::graphics::{Modifier, Rect};
use helix_view::info::Info;
use helix_view::input::KeyEvent;
use helix_view::keyboard::{KeyCode, KeyModifiers};
use helix_view::sidebar::{SidebarFocus, SidebarState};
use helix_view::Editor;
use tui::buffer::Buffer as Surface;

use crate::commands;
use crate::ui::{self, menu::Item, Select};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GitFileStatus {
    Untracked,
    Modified,
    Conflict,
    Renamed,
    Deleted,
}

#[derive(Debug, Clone)]
struct FileClipboard {
    paths: Vec<PathBuf>,
    cut: bool,
}

pub struct FileTree {
    pub root: PathBuf,
    pub rows: Vec<Row>,
    pub cursor: usize,
    pub scroll: usize,
    pub filter: String,
    pub command_mode: bool,
    pub overflow_hint: Option<String>,
    pub dirty_only: bool,
    git_status: HashMap<PathBuf, GitFileStatus>,
    git_status_root: Option<PathBuf>,
    git_status_epoch: Option<u64>,
    clipboard: Option<FileClipboard>,
    labels: Option<Vec<(String, usize)>>,
    label_prefix: Option<char>,
    horizontal_scroll: usize,
}

impl FileTree {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            rows: Vec::new(),
            cursor: 0,
            scroll: 0,
            filter: String::new(),
            command_mode: false,
            overflow_hint: None,
            dirty_only: false,
            git_status: HashMap::new(),
            git_status_root: None,
            git_status_epoch: None,
            clipboard: None,
            labels: None,
            label_prefix: None,
            horizontal_scroll: 0,
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
            recurse_all: bool,
        ) {
            let Ok(entries) = list(dir) else {
                return;
            };
            for (path, is_dir) in entries {
                let is_expanded = is_dir && (recurse_all || expanded.contains(&path));
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
                    push_dir(rows, &path, depth + 1, expanded, list, recurse_all);
                }
            }
        }

        let mut rows = Vec::new();
        // While filtering, walk the whole tree so matches inside collapsed
        // directories are still discoverable.
        push_dir(&mut rows, root, 0, expanded, &list, !self.filter.is_empty());
        if self.dirty_only {
            rows.retain(|row| self.git_status.contains_key(&row.path));
        }
        if self.filter.is_empty() {
            self.rows = rows;
        } else {
            // Fuzzy matches keep their tree structure: show matching rows and
            // their ancestor directories, rather than a flat result list.
            let by_name: HashMap<String, PathBuf> = rows
                .iter()
                .map(|row| {
                    let name = row
                        .path
                        .strip_prefix(&self.root)
                        .unwrap_or(&row.path)
                        .to_string_lossy()
                        .into_owned();
                    (name, row.path.clone())
                })
                .collect();
            let names: Vec<&str> = by_name.keys().map(String::as_str).collect();
            let matches: HashSet<PathBuf> = fuzzy_match(&self.filter, names, true)
                .into_iter()
                .filter_map(|(name, _)| by_name.get(name).cloned())
                .collect();
            rows.retain(|row| {
                matches.contains(&row.path)
                    || (row.kind == EntryKind::Dir
                        && matches.iter().any(|path| path.starts_with(&row.path)))
            });
            self.rows = rows;
        }
        self.cursor = self.cursor.min(self.rows.len().saturating_sub(1));
        self.labels = None;
        self.label_prefix = None;
        self.clamp_scroll();
    }

    pub fn git_status_needs_refresh(&self, editor: &Editor) -> bool {
        self.git_status_root.as_ref() != Some(&self.root)
            || self.git_status_epoch != Some(editor.sidebar.refresh_epoch)
    }

    pub fn refresh_git_status(&mut self, editor: &Editor) {
        let root = self.root.clone();
        self.git_status_root = Some(root.clone());
        self.git_status_epoch = Some(editor.sidebar.refresh_epoch);

        if !editor.config().sidebar.git_status {
            self.git_status.clear();
            return;
        }

        let workspace = helix_loader::find_workspace_in(&root).0;
        let trusted = editor
            .workspace_trust
            .query(&workspace, helix_loader::workspace_trust::TrustQuery::Git)
            .is_trusted();
        if !trusted {
            self.git_status.clear();
            return;
        }

        let output = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["status", "--porcelain=v1", "--untracked-files=all"])
            .output();
        let Ok(output) = output else {
            self.git_status.clear();
            return;
        };
        if !output.status.success() {
            self.git_status.clear();
            return;
        }

        let mut statuses = HashMap::new();
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            if line.len() < 4 {
                continue;
            }
            let code = &line[..2];
            let rest = &line[3..];
            let path = if code.starts_with('R') || code.starts_with('C') {
                rest.rsplit(" -> ").next().unwrap_or(rest)
            } else {
                rest
            };
            let path = path.trim_matches('"');
            let path = root.join(path);
            let status = if code.contains('U') || matches!(code, "AA" | "DD") {
                GitFileStatus::Conflict
            } else if code == "??" || code.starts_with('A') {
                GitFileStatus::Untracked
            } else if code.starts_with('R') {
                GitFileStatus::Renamed
            } else if code.contains('D') {
                GitFileStatus::Deleted
            } else {
                GitFileStatus::Modified
            };
            statuses.insert(path.clone(), status);

            let mut parent = path.parent();
            while let Some(dir) = parent {
                if dir == root || !dir.starts_with(&root) {
                    break;
                }
                statuses.entry(dir.to_path_buf()).or_insert(status);
                parent = dir.parent();
            }
        }
        self.git_status = statuses;
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
        let title = if focused {
            theme
                .try_get_exact("ui.sidebar.title.focused")
                .or_else(|| theme.try_get_exact("ui.sidebar.title"))
                .or_else(|| theme.try_get_exact("ui.text"))
                .unwrap_or_default()
                .add_modifier(Modifier::BOLD)
        } else {
            theme
                .try_get_exact("ui.sidebar.title")
                .or_else(|| theme.try_get_exact("ui.text"))
                .unwrap_or_default()
        };
        let directory = theme
            .try_get_exact("ui.tree.directory")
            .or_else(|| theme.try_get_exact("ui.text.directory"))
            .unwrap_or_default();
        let text_style = theme.try_get_exact("ui.text").unwrap_or_default();
        let header_bg = theme
            .try_get_exact("ui.tree.header")
            .or_else(|| theme.try_get_exact("ui.statusline"))
            .unwrap_or_default();
        let selected = theme
            .try_get_exact(if focused {
                "ui.tree.selected.focused"
            } else {
                "ui.tree.selected.unfocused"
            })
            .or_else(|| theme.try_get_exact("ui.tree.selected"))
            .or_else(|| theme.try_get_exact("ui.selection"))
            .unwrap_or_default();
        let menu = if focused {
            selected
        } else {
            selected.add_modifier(Modifier::DIM)
        };

        if area.height == 0 {
            return;
        }

        let root_name = self
            .root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.root.display().to_string());
        let folder_name = self
            .selected_row()
            .map(|row| {
                let path = if row.kind == EntryKind::Dir {
                    row.path.as_path()
                } else {
                    row.path.parent().unwrap_or(&row.path)
                };
                path.strip_prefix(&self.root)
                    .ok()
                    .filter(|path| !path.as_os_str().is_empty())
                    .map(|path| path.to_string_lossy().into_owned())
                    .unwrap_or_else(|| root_name.clone())
            })
            .unwrap_or_else(|| root_name.clone());
        let header = if self.command_mode {
            format!("{root_name} | {folder_name}  [commands]")
        } else {
            format!("{root_name} | {folder_name}")
        };
        surface.clear_with(Rect::new(area.x, area.y, area.width, 1), header_bg);
        surface.set_stringn(
            area.x,
            area.y,
            &header,
            area.width as usize,
            title.patch(header_bg),
        );

        // Dedicated fuzzy-search input beneath the project name.
        if area.height > 1 {
            let search_area = Rect::new(area.x, area.y + 1, area.width, 1);
            surface.clear_with(search_area, theme.get("ui.menu"));
            let label = " Find: ";
            let label_width = label.width() as u16;
            surface.set_stringn(
                area.x,
                search_area.y,
                label,
                area.width as usize,
                text_style,
            );
            let query_width = (area.width as usize).saturating_sub(label_width as usize);
            if query_width > 0 {
                if self.filter.is_empty() {
                    surface.set_stringn(
                        area.x + label_width,
                        search_area.y,
                        "type to filter",
                        query_width,
                        theme.get("ui.text.inactive"),
                    );
                } else {
                    surface.set_stringn(
                        area.x + label_width,
                        search_area.y,
                        &self.filter,
                        query_width,
                        text_style,
                    );
                }
                if focused && !self.command_mode {
                    let cursor_x = label_width as usize + self.filter.width();
                    if cursor_x < area.width as usize {
                        surface.set_style(
                            Rect::new(area.x + cursor_x as u16, search_area.y, 1, 1),
                            theme.get("ui.cursor"),
                        );
                    }
                }
            }
        }

        let height = area.height.saturating_sub(2) as usize;
        if height == 0 {
            return;
        }
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        } else if self.cursor >= self.scroll + height {
            self.scroll = self.cursor + 1 - height;
        }

        self.overflow_hint = None;
        for (offset, row) in self.rows.iter().skip(self.scroll).take(height).enumerate() {
            let index = self.scroll + offset;
            let marker = match row.kind {
                EntryKind::Dir if row.expanded => "▾",
                EntryKind::Dir => "▸",
                EntryKind::File => " ",
            };
            let indent = "│ ".repeat(row.depth);
            let name = row
                .path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| row.path.display().to_string());
            let label = self.labels.as_ref().and_then(|labels| {
                labels
                    .iter()
                    .find(|(_, label_index)| *label_index == index)
                    .map(|(label, _)| {
                        if self
                            .label_prefix
                            .is_some_and(|prefix| !label.starts_with(prefix))
                        {
                            "  "
                        } else {
                            label.as_str()
                        }
                    })
            });
            let label_prefix = label.map(|label| format!("{label} "));
            let text = match &label_prefix {
                Some(label) => format!("{label}{indent}{marker} {name}"),
                None => format!("{indent}{marker} {name}"),
            };
            // Keep the selected row visible even when the editor owns focus,
            // so it is clear which file is currently open.
            let mut style = if index == self.cursor {
                menu.patch(text_style)
            } else if row.kind == EntryKind::Dir {
                directory
            } else {
                text_style
            };
            if let Some(status) = self.git_status.get(&row.path) {
                let dirty = match status {
                    GitFileStatus::Untracked => theme.get("diff.plus"),
                    GitFileStatus::Modified => theme.get("diff.delta"),
                    GitFileStatus::Conflict => theme.get("diff.delta.conflict"),
                    GitFileStatus::Renamed => theme.get("diff.delta.moved"),
                    GitFileStatus::Deleted => theme.get("diff.minus"),
                };
                style = style.patch(dirty);
            }

            let selected_row = index == self.cursor;
            let content_x = area.x;
            let content_width = area.width as usize;
            let full_width = text.width();
            if selected_row {
                let max_scroll = full_width.saturating_sub(content_width);
                self.horizontal_scroll = self.horizontal_scroll.min(max_scroll);
                if full_width > content_width {
                    self.overflow_hint = Some(row.path.display().to_string());
                }
            }
            let y = area.y + 2 + offset as u16;
            if self.horizontal_scroll == 0 && !indent.is_empty() {
                // Draw the hierarchy guides with a faded style while keeping
                // the row's selection background intact.
                let guide_style = style.patch(
                    theme
                        .try_get_exact("ui.tree.indent")
                        .or_else(|| theme.try_get_exact("ui.virtual.indent-guide"))
                        .or_else(|| theme.try_get_exact("ui.linenr"))
                        .unwrap_or_default(),
                );
                let mut x = content_x;
                if let Some(label) = &label_prefix {
                    x = surface.set_stringn(x, y, label, content_width, style).0;
                }
                x = surface
                    .set_stringn(
                        x,
                        y,
                        &indent,
                        content_width.saturating_sub((x - content_x) as usize),
                        guide_style,
                    )
                    .0;
                let rest = format!("{marker} {name}");
                surface.set_stringn(
                    x,
                    y,
                    &rest,
                    content_width.saturating_sub((x - content_x) as usize),
                    style,
                );
            } else {
                let visible = skip_columns(&text, self.horizontal_scroll);
                // `set_stringn` is essential here: writing an overlong row with
                // `set_string` spills the remaining cells into the editor pane.
                surface.set_stringn(content_x, y, visible, content_width, style);
            }
        }
    }

    pub fn selected_path(&self) -> Option<&Path> {
        self.selected_row().map(|row| row.path.as_path())
    }

    pub fn set_filter(&mut self, filter: String, editor: &mut Editor) {
        self.filter = filter.clone();
        editor.sidebar.tree_filter = filter;
        self.rebuild_from_editor(editor);
    }

    fn remember_for_paste(&mut self, cx: &mut commands::Context, cut: bool) {
        let Some(row) = self.selected_row().cloned() else {
            return;
        };
        self.clipboard = Some(FileClipboard {
            paths: vec![row.path.clone()],
            cut,
        });
        cx.editor.set_status(format!(
            "{} {}",
            if cut { "cut" } else { "copied" },
            row.path.display()
        ));
    }

    fn paste_clipboard(&mut self, cx: &mut commands::Context) {
        let Some(clipboard) = self.clipboard.clone() else {
            cx.editor.set_error("file clipboard is empty");
            return;
        };
        let target_dir = self.target_parent();
        let mut pasted = Vec::new();
        for source in &clipboard.paths {
            let Some(name) = source.file_name() else {
                continue;
            };
            let destination = target_dir.join(name);
            if destination == *source {
                continue;
            }
            if destination.exists() {
                cx.editor
                    .set_error(format!("{} already exists", destination.display()));
                continue;
            }
            let result = if clipboard.cut {
                move_recursively(source, &destination).map(|_| {
                    move_open_documents(cx.editor, source, &destination);
                })
            } else {
                copy_recursively(source, &destination)
            };
            match result {
                Ok(()) => {
                    Self::mark_changed(cx.editor, &destination);
                    pasted.push(destination);
                }
                Err(err) => cx
                    .editor
                    .set_error(format!("failed to paste {}: {err}", source.display())),
            }
        }
        if clipboard.cut {
            self.clipboard = None;
        }
        if let Some(last) = pasted.last() {
            cx.editor.sidebar.selected = Some(last.clone());
        }
        self.rebuild_from_editor(cx.editor);
    }

    /// Expand all ancestors of `path` below `root`, rebuild, and select it.
    pub fn reveal(
        &mut self,
        editor: &Editor,
        expanded: &mut HashSet<PathBuf>,
        path: &Path,
    ) -> bool {
        if !path.starts_with(&self.root) {
            return false;
        }
        let mut ancestor = path.parent();
        while let Some(parent) = ancestor {
            if parent == self.root || !parent.starts_with(&self.root) {
                break;
            }
            expanded.insert(parent.to_path_buf());
            ancestor = parent.parent();
        }
        let state = SidebarState {
            expanded: expanded.clone(),
            ..SidebarState::default()
        };
        self.rebuild(editor, &state);
        self.select_path(path);
        true
    }

    fn start_jump_labels(&mut self, cx: &mut commands::Context) {
        let alphabet = cx.editor.config().jump_label_alphabet.clone();
        if alphabet.len() < 2 {
            return;
        }
        let limit = alphabet.len() * alphabet.len();
        let mut labels = Vec::new();
        for (index, _row) in self.rows.iter().enumerate().take(limit) {
            let first = alphabet[index / alphabet.len()];
            let second = alphabet[index % alphabet.len()];
            labels.push((format!("{first}{second}"), index));
        }
        self.labels = Some(labels);
        self.label_prefix = None;
    }

    fn handle_label_key(&mut self, key: KeyEvent) -> bool {
        let some_labels = self
            .labels
            .as_ref()
            .is_some_and(|labels| !labels.is_empty());
        if !some_labels {
            self.labels = None;
            self.label_prefix = None;
            return true;
        }
        match key.code {
            KeyCode::Esc => {
                self.labels = None;
                self.label_prefix = None;
            }
            KeyCode::Char(c) => {
                if let Some(prefix) = self.label_prefix.take() {
                    let target = self.labels.as_ref().and_then(|labels| {
                        labels
                            .iter()
                            .find(|(label, _)| {
                                let mut chars = label.chars();
                                chars.next() == Some(prefix) && chars.next() == Some(c)
                            })
                            .map(|(_, index)| *index)
                    });
                    if let Some(index) = target {
                        self.cursor = index;
                    }
                    self.labels = None;
                } else if self
                    .labels
                    .as_ref()
                    .is_some_and(|labels| labels.iter().any(|(label, _)| label.starts_with(c)))
                {
                    self.label_prefix = Some(c);
                } else {
                    self.labels = None;
                }
            }
            _ => {
                self.labels = None;
                self.label_prefix = None;
            }
        }
        true
    }

    fn selected_row(&self) -> Option<&Row> {
        self.rows.get(self.cursor)
    }

    fn target_parent(&self) -> PathBuf {
        match self.selected_row() {
            Some(row) if row.kind == EntryKind::Dir => row.path.clone(),
            Some(row) => row
                .path
                .parent()
                .map(PathBuf::from)
                .unwrap_or_else(|| self.root.clone()),
            None => self.root.clone(),
        }
    }

    fn mark_changed(editor: &mut Editor, path: &Path) {
        Self::mark_changed_select(editor, path, Some(path.to_path_buf()));
    }

    fn mark_changed_select(editor: &mut Editor, path: &Path, selected: Option<PathBuf>) {
        if let Some(parent) = path.parent() {
            editor.sidebar.refresh_dirs.insert(parent.to_path_buf());
        }
        editor.sidebar.refresh_epoch = editor.sidebar.refresh_epoch.wrapping_add(1);
        editor.sidebar.selected = selected;
        editor.session_dirty = true;
        editor.needs_redraw = true;
    }

    fn create(&mut self, cx: &mut commands::Context, directory: bool) {
        let parent = self.target_parent();
        let label = if directory {
            "New directory:"
        } else {
            "New file:"
        };
        ui::prompt(
            cx,
            label.into(),
            None,
            |_editor, _input| Vec::new(),
            move |cx, input, event| {
                if event != crate::ui::PromptEvent::Validate {
                    return;
                }
                let name = input.trim();
                if name.is_empty()
                    || name == "."
                    || name == ".."
                    || name.contains('/')
                    || name.contains('\\')
                {
                    cx.editor.set_error("invalid name");
                    return;
                }
                let path = parent.join(name);
                let result = if directory {
                    std::fs::create_dir(&path)
                } else {
                    std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&path)
                        .map(|_| ())
                };
                match result {
                    Ok(()) => {
                        Self::mark_changed(cx.editor, &path);
                        cx.editor.set_status(format!(
                            "created {}",
                            path.file_name().unwrap_or_default().to_string_lossy()
                        ));
                        if !directory {
                            if let Err(err) = cx.editor.open(&path, Action::Replace) {
                                cx.editor.set_error(err.to_string());
                            }
                        }
                    }
                    Err(err) => cx.editor.set_error(format!("failed to create: {err}")),
                }
            },
        );
    }

    fn rename(&mut self, cx: &mut commands::Context) {
        let Some(row) = self.selected_row().cloned() else {
            return;
        };
        let old = row.path;
        let name = old
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let parent = old
            .parent()
            .map(PathBuf::from)
            .unwrap_or_else(|| self.root.clone());
        ui::prompt_with_input(
            cx,
            "Rename:".into(),
            name,
            None,
            |_editor, _input| Vec::new(),
            move |cx, input, event| {
                if event != crate::ui::PromptEvent::Validate {
                    return;
                }
                let name = input.trim();
                if name.is_empty()
                    || name == "."
                    || name == ".."
                    || name.contains('/')
                    || name.contains('\\')
                {
                    cx.editor.set_error("invalid name");
                    return;
                }
                let new_path = parent.join(name);
                if new_path.exists() {
                    cx.editor
                        .set_error(format!("{} already exists", new_path.display()));
                    return;
                }
                let modified_affected = cx.editor.documents().any(|doc| {
                    doc.path()
                        .is_some_and(|path| path.starts_with(&old) && doc.is_modified())
                });
                if modified_affected {
                    cx.editor
                        .set_error("refusing to rename a file with unsaved changes");
                    return;
                }
                let affected: Vec<_> = cx
                    .editor
                    .documents()
                    .filter_map(|doc| {
                        let path = doc.path()?;
                        let relative = path.strip_prefix(&old).ok()?;
                        Some((doc.id(), new_path.join(relative)))
                    })
                    .collect();
                match std::fs::rename(&old, &new_path) {
                    Ok(()) => {
                        for (doc_id, path) in affected {
                            cx.editor.set_doc_path(doc_id, &path);
                        }
                        Self::mark_changed(cx.editor, &new_path);
                        cx.editor.set_status(format!("renamed to {}", name));
                    }
                    Err(err) => cx.editor.set_error(format!("failed to rename: {err}")),
                }
            },
        );
    }

    pub fn delete(&mut self, cx: &mut commands::Context) {
        let Some(row) = self.selected_row().cloned() else {
            return;
        };
        let path = row.path;
        let modified_open = cx.editor.documents().any(|doc| {
            doc.path()
                .is_some_and(|doc_path| doc_path.starts_with(&path) && doc.is_modified())
        });
        if modified_open {
            cx.editor
                .set_error("refusing to delete a file with unsaved changes");
            return;
        }
        let target = path.clone();
        let open_docs: Vec<_> = cx
            .editor
            .documents()
            .filter_map(|doc| {
                doc.path()
                    .is_some_and(|doc_path| doc_path.starts_with(&path))
                    .then(|| doc.id())
            })
            .collect();
        let fallback = self
            .rows
            .iter()
            .skip(self.cursor + 1)
            .find(|row| row.kind == EntryKind::File)
            .or_else(|| {
                self.rows[..self.cursor]
                    .iter()
                    .rev()
                    .find(|row| row.kind == EntryKind::File)
            })
            .map(|row| row.path.clone());
        let has_open = !open_docs.is_empty();
        let select = Select::new(
            format!("Delete {}?", path.display()),
            [DeleteChoice::No, DeleteChoice::Yes],
            (),
            move |editor, choice, event| {
                if event != crate::ui::PromptEvent::Validate || *choice != DeleteChoice::Yes {
                    return;
                }
                let result = if target.is_dir() {
                    std::fs::remove_dir_all(&target)
                } else {
                    std::fs::remove_file(&target)
                };
                match result {
                    Ok(()) => {
                        for doc_id in &open_docs {
                            let _ = editor.close_document(*doc_id, false);
                        }
                        if has_open {
                            editor.sidebar.reveal_current = true;
                            Self::mark_changed_select(editor, &target, None);
                        } else {
                            Self::mark_changed_select(editor, &target, fallback.clone());
                        }
                        editor.set_status("deleted");
                    }
                    Err(err) => editor.set_error(format!("failed to delete: {err}")),
                }
            },
        );
        cx.push_layer(Box::new(select));
    }

    fn copy_path(&mut self, cx: &mut commands::Context, relative: bool) {
        let Some(row) = self.selected_row().cloned() else {
            return;
        };
        let text = if relative {
            helix_stdx::path::get_relative_path(row.path)
                .to_string_lossy()
                .into_owned()
        } else {
            row.path.to_string_lossy().into_owned()
        };
        match cx.editor.registers.write('+', vec![text.clone()]) {
            Ok(()) => cx.editor.set_status(format!("copied {text}")),
            Err(err) => cx.editor.set_error(format!("failed to copy path: {err}")),
        }
    }

    pub fn select_path(&mut self, path: &Path) {
        if let Some(index) = self.rows.iter().position(|row| row.path == path) {
            self.cursor = index;
        }
    }

    fn rebuild_from_editor(&mut self, editor: &mut Editor) {
        let state = editor.sidebar.clone();
        self.rebuild(editor, &state);
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
        self.rebuild_from_editor(cx.editor);
        self.select_path(&path);
    }

    fn collapse_or_parent(&mut self, cx: &mut commands::Context) {
        let Some(row) = self.selected_row().cloned() else {
            return;
        };
        if row.kind == EntryKind::Dir && row.expanded {
            cx.editor.sidebar.expanded.remove(&row.path);
            self.rebuild_from_editor(cx.editor);
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
        if self.labels.is_some() {
            let handled = self.handle_label_key(key);
            self.sync_selection(cx);
            return handled;
        }

        if self.command_mode {
            return self.handle_command_key(key, cx);
        }

        // Everything that is not a navigation key is a live fuzzy query. The
        // tree actions live behind `<space>` so ordinary typing is free.
        if key.code == KeyCode::Char(' ')
            && !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            self.enter_command_mode(cx);
            return true;
        }

        match key.code {
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.set_query(String::new(), cx);
            }
            KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let mut query = self.filter.clone();
                while query.ends_with(char::is_whitespace) {
                    query.pop();
                }
                while query.chars().last().is_some_and(|c| !c.is_whitespace()) {
                    query.pop();
                }
                self.set_query(query, cx);
            }
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                let mut query = self.filter.clone();
                query.push(c);
                self.set_query(query, cx);
            }
            KeyCode::Backspace => {
                let mut query = self.filter.clone();
                query.pop();
                self.set_query(query, cx);
            }
            KeyCode::Esc => {
                if !self.filter.is_empty() {
                    self.set_query(String::new(), cx);
                } else {
                    cx.editor.sidebar.focus = SidebarFocus::Editor;
                    cx.editor.needs_redraw = true;
                }
            }
            KeyCode::Down if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let lines = cx.editor.config().scroll_lines.unsigned_abs().max(1);
                if !self.rows.is_empty() {
                    self.cursor = (self.cursor + lines).min(self.rows.len() - 1);
                }
                self.sync_selection(cx);
            }
            KeyCode::Up if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let lines = cx.editor.config().scroll_lines.unsigned_abs().max(1);
                self.cursor = self.cursor.saturating_sub(lines);
                self.sync_selection(cx);
            }
            KeyCode::Down => {
                if !self.rows.is_empty() {
                    self.cursor = (self.cursor + 1).min(self.rows.len() - 1);
                }
                self.sync_selection(cx);
            }
            KeyCode::Up => {
                self.cursor = self.cursor.saturating_sub(1);
                self.sync_selection(cx);
            }
            KeyCode::Right => {
                if let Some(row) = self.selected_row().cloned() {
                    if row.kind == EntryKind::Dir && !row.expanded {
                        self.toggle_expand(cx);
                    } else {
                        self.open_selected(cx, Action::Replace);
                    }
                }
                self.sync_selection(cx);
            }
            KeyCode::Left => {
                self.collapse_or_parent(cx);
                self.sync_selection(cx);
            }
            KeyCode::Enter => {
                self.open_selected(cx, Action::Replace);
                self.sync_selection(cx);
            }
            KeyCode::Tab => {
                if self
                    .selected_row()
                    .is_some_and(|row| row.kind == EntryKind::File)
                {
                    self.open_selected(cx, Action::Replace);
                } else {
                    cx.editor.sidebar.focus = SidebarFocus::Editor;
                }
                self.sync_selection(cx);
            }
            _ => {
                // Let global Ctrl/Alt chords through.
                if key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                {
                    return false;
                }
            }
        }

        true
    }

    fn set_query(&mut self, query: String, cx: &mut commands::Context) {
        self.filter = query;
        cx.editor.sidebar.tree_filter = self.filter.clone();
        self.cursor = 0;
        self.rebuild_from_editor(cx.editor);
        self.sync_selection(cx);
    }

    fn sync_selection(&mut self, cx: &mut commands::Context) {
        cx.editor.sidebar.selected = self.selected_row().map(|row| row.path.clone());
        cx.editor.needs_redraw = true;
    }

    fn enter_command_mode(&mut self, cx: &mut commands::Context) {
        self.command_mode = true;
        let body = [
            ("", "── Navigate ──"),
            ("j, Down", "Move down"),
            ("k, Up", "Move up"),
            ("h, Left", "Collapse or go to parent"),
            ("l, Right", "Expand or open"),
            ("g / G", "First / last row"),
            ("J / K", "Move ten rows"),
            ("w", "Jump labels"),
            ("", "── Open ──"),
            ("Enter, o", "Open"),
            ("s / v", "Open in horizontal / vertical split"),
            ("", "── Files ──"),
            ("a / A", "Create file / directory"),
            ("r", "Rename"),
            ("d", "Delete"),
            ("c / m / p", "Copy / cut / paste"),
            ("y / Y", "Copy absolute / relative path"),
            ("x", "Close current buffer"),
            ("", "── View ──"),
            ("D", "Show dirty files only"),
            (".", "Reveal current buffer"),
            ("H / R", "Collapse all / refresh"),
            ("[ / ]", "Shrink / grow sidebar"),
            ("< / >", "Scroll tree left / right"),
            ("", "── Session ──"),
            ("/", "Open global search"),
            ("b", "Hide sidebar"),
            ("q / Esc", "Return to editor / close commands"),
        ];
        cx.editor.autoinfo = Some(Info::new("Tree commands (Space)", &body));
        cx.editor.needs_redraw = true;
    }

    fn leave_command_mode(&mut self, cx: &mut commands::Context) {
        self.command_mode = false;
        cx.editor.autoinfo = None;
        cx.editor.needs_redraw = true;
    }

    fn handle_command_key(&mut self, key: KeyEvent, cx: &mut commands::Context) -> bool {
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            self.leave_command_mode(cx);
            return false;
        }

        match key.code {
            KeyCode::Esc => {
                self.leave_command_mode(cx);
                return true;
            }
            KeyCode::Char('q') => {
                self.leave_command_mode(cx);
                cx.editor.sidebar.focus = SidebarFocus::Editor;
                return true;
            }
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
            KeyCode::Char('a') => self.create(cx, false),
            KeyCode::Char('A') => self.create(cx, true),
            KeyCode::Char('r') => self.rename(cx),
            KeyCode::Char('d') => self.delete(cx),
            KeyCode::Char('c') => self.remember_for_paste(cx, false),
            KeyCode::Char('m') => self.remember_for_paste(cx, true),
            KeyCode::Char('p') => self.paste_clipboard(cx),
            KeyCode::Char('y') => self.copy_path(cx, false),
            KeyCode::Char('Y') => self.copy_path(cx, true),
            KeyCode::Char('D') => {
                self.dirty_only = !self.dirty_only;
                if self.dirty_only {
                    self.refresh_git_status(cx.editor);
                }
                self.rebuild_from_editor(cx.editor);
                cx.editor.set_status(if self.dirty_only {
                    "Showing dirty files only"
                } else {
                    "Showing all files"
                });
            }
            KeyCode::Char('x') => {
                self.leave_command_mode(cx);
                crate::commands::close_buffer(cx);
                return true;
            }
            KeyCode::Char('/') => {
                self.leave_command_mode(cx);
                cx.editor.sidebar.search_modal = true;
                cx.editor.sidebar.focus = SidebarFocus::Sidebar;
                return true;
            }
            KeyCode::Char('b') => {
                self.leave_command_mode(cx);
                cx.editor.sidebar.mode = helix_view::sidebar::SidebarMode::Hidden;
                cx.editor.sidebar.focus = SidebarFocus::Editor;
                cx.editor.save_ui_state();
                return true;
            }
            KeyCode::Char('w') => {
                self.start_jump_labels(cx);
                self.leave_command_mode(cx);
                return true;
            }
            KeyCode::Char('g') => self.cursor = 0,
            KeyCode::Char('G') => {
                self.cursor = self.rows.len().saturating_sub(1);
            }
            KeyCode::Char('J') => {
                self.cursor = (self.cursor + 10).min(self.rows.len().saturating_sub(1));
            }
            KeyCode::Char('K') => {
                self.cursor = self.cursor.saturating_sub(10);
            }
            KeyCode::Char('H') => {
                cx.editor.sidebar.expanded.clear();
                self.rebuild_from_editor(cx.editor);
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
                    self.rebuild_from_editor(cx.editor);
                    self.select_path(&path);
                    cx.editor.sidebar.selected = Some(path);
                }
            }
            KeyCode::Char('[') => {
                let config = cx.editor.config().sidebar;
                cx.editor.sidebar.width = cx
                    .editor
                    .sidebar
                    .width
                    .saturating_sub(2)
                    .max(config.min_width);
            }
            KeyCode::Char(']') => {
                let config = cx.editor.config().sidebar;
                cx.editor.sidebar.width = (cx.editor.sidebar.width + 2).min(config.max_width);
            }
            KeyCode::Char('<') => {
                self.horizontal_scroll = self.horizontal_scroll.saturating_sub(4);
            }
            KeyCode::Char('>') => {
                self.horizontal_scroll = self.horizontal_scroll.saturating_add(4);
            }
            _ => {}
        }

        self.leave_command_mode(cx);
        self.sync_selection(cx);
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

/// Skip a number of terminal columns from the start of `text`, preserving
/// UTF-8 boundaries and wide characters.
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

fn copy_recursively(source: &Path, destination: &Path) -> std::io::Result<()> {
    if source.is_dir() {
        std::fs::create_dir_all(destination)?;
        for entry in std::fs::read_dir(source)? {
            let entry = entry?;
            copy_recursively(&entry.path(), &destination.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(source, destination).map(|_| ())
    }
}

/// Move a path, falling back to copy + delete when `rename` crosses a device
/// or the filesystem does not support it.
fn move_recursively(source: &Path, destination: &Path) -> std::io::Result<()> {
    match std::fs::rename(source, destination) {
        Ok(()) => Ok(()),
        Err(_) => {
            copy_recursively(source, destination)?;
            if source.is_dir() {
                std::fs::remove_dir_all(source)
            } else {
                std::fs::remove_file(source)
            }
        }
    }
}

fn move_open_documents(editor: &mut Editor, old: &Path, new: &Path) {
    let affected: Vec<_> = editor
        .documents()
        .filter_map(|doc| {
            let path = doc.path()?;
            let relative = path.strip_prefix(old).ok()?;
            Some((doc.id(), new.join(relative)))
        })
        .collect();
    for (doc_id, path) in affected {
        editor.set_doc_path(doc_id, &path);
    }

    let expanded: Vec<_> = editor
        .sidebar
        .expanded
        .iter()
        .filter_map(|path| {
            path.strip_prefix(old)
                .ok()
                .map(|relative| (path.clone(), new.join(relative)))
        })
        .collect();
    for (old_path, new_path) in expanded {
        editor.sidebar.expanded.remove(&old_path);
        editor.sidebar.expanded.insert(new_path);
    }
    if let Some(selected) = editor.sidebar.selected.clone() {
        if let Ok(relative) = selected.strip_prefix(old) {
            editor.sidebar.selected = Some(new.join(relative));
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeleteChoice {
    No,
    Yes,
}

impl Item for DeleteChoice {
    type Data = ();

    fn format(&self, _data: &Self::Data) -> tui::widgets::Row<'_> {
        match self {
            DeleteChoice::No => "No",
            DeleteChoice::Yes => "Yes",
        }
        .into()
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

#[cfg(test)]
mod tree_tests {
    use super::*;

    #[test]
    fn skip_columns_stays_on_utf8_boundaries() {
        assert_eq!(skip_columns("abcdef", 0), "abcdef");
        assert_eq!(skip_columns("abcdef", 4), "ef");
        assert_eq!(skip_columns("abcdef", 99), "");
        assert_eq!(skip_columns("a世b", 2), "b");
    }

    #[test]
    fn filter_walks_collapsed_directories() {
        let root = PathBuf::from("/repo");
        let mut tree = FileTree::new(root.clone());
        tree.filter = "ndl".to_string();
        tree.rebuild_with(&root, &HashSet::new(), |dir| {
            Ok(match dir {
                _ if dir == Path::new("/repo") => vec![
                    (PathBuf::from("/repo/src"), true),
                    (PathBuf::from("/repo/README.md"), false),
                ],
                _ if dir == Path::new("/repo/src") => {
                    vec![(PathBuf::from("/repo/src/needle.rs"), false)]
                }
                _ => Vec::new(),
            })
        });
        let paths: Vec<_> = tree.rows.iter().map(|row| row.path.as_path()).collect();
        assert!(paths.contains(&Path::new("/repo/src")));
        assert!(paths.contains(&Path::new("/repo/src/needle.rs")));
        assert!(!paths.contains(&Path::new("/repo/README.md")));
    }
}
