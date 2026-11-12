use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};

use helix_core::unicode::width::{UnicodeWidthChar, UnicodeWidthStr};
use helix_view::editor::Action;
use helix_view::graphics::{Modifier, Rect};
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
    pub overflow_hint: Option<String>,
    clipboard: Option<FileClipboard>,
    labels: Option<Vec<(String, usize)>>,
    label_prefix: Option<char>,
    pending_g: bool,
    pending_z: bool,
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
            overflow_hint: None,
            clipboard: None,
            labels: None,
            label_prefix: None,
            pending_g: false,
            pending_z: false,
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
        if !self.filter.is_empty() {
            let needle = self.filter.to_lowercase();
            let matches: HashSet<PathBuf> = rows
                .iter()
                .filter(|row| row.path.to_string_lossy().to_lowercase().contains(&needle))
                .map(|row| row.path.clone())
                .collect();
            rows.retain(|row| {
                matches.contains(&row.path)
                    || (row.kind == EntryKind::Dir
                        && matches.iter().any(|path| path.starts_with(&row.path)))
            });
        }
        self.rows = rows;
        self.cursor = self.cursor.min(self.rows.len().saturating_sub(1));
        self.labels = None;
        self.label_prefix = None;
        self.pending_g = false;
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
        let indicator = theme
            .try_get_exact(if focused {
                "ui.tree.selected.indicator"
            } else {
                "ui.tree.selected.indicator.unfocused"
            })
            .unwrap_or(menu);

        if area.height == 0 {
            return;
        }

        let mut header = self
            .root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.root.display().to_string());
        if !self.filter.is_empty() {
            header.push_str(&format!(" [{}]", self.filter));
        }
        surface.set_stringn(area.x, area.y, &header, area.width as usize, title);

        let height = area.height.saturating_sub(1) as usize;
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
            let text = match label {
                Some(label) => format!("{label} {}{} {}", "  ".repeat(row.depth), marker, name),
                None => format!("{}{} {}", "  ".repeat(row.depth), marker, name),
            };
            // Keep the selected row visible even when the editor owns focus,
            // so it is clear which file is currently open.
            let style = if index == self.cursor {
                menu
            } else if row.kind == EntryKind::Dir {
                directory
            } else {
                title
            };

            let selected_row = index == self.cursor;
            let indicator_width = u16::from(selected_row);
            let content_x = area.x + indicator_width;
            let content_width = (area.width as usize).saturating_sub(indicator_width as usize);
            let full_width = text.width();
            if selected_row {
                let max_scroll = full_width.saturating_sub(content_width);
                self.horizontal_scroll = self.horizontal_scroll.min(max_scroll);
                if full_width > content_width {
                    self.overflow_hint = Some(row.path.display().to_string());
                }
                // A focus-independent accent bar keeps the selected row
                // discoverable; its style is muted while the tree is inactive.
                surface.set_string(area.x, area.y + 1 + offset as u16, "▍", indicator);
            }
            let visible = skip_columns(&text, self.horizontal_scroll);
            // `set_stringn` is essential here: writing an overlong row with
            // `set_string` spills the remaining cells into the editor pane.
            surface.set_stringn(
                content_x,
                area.y + 1 + offset as u16,
                visible,
                content_width,
                style,
            );
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

    fn prompt_filter(&mut self, cx: &mut commands::Context) {
        let current = self.filter.clone();
        ui::prompt_with_input(
            cx,
            "Filter files:".into(),
            current,
            None,
            |_editor, _input| Vec::new(),
            move |cx, input, event| {
                if event == crate::ui::PromptEvent::Validate {
                    cx.editor.sidebar.tree_filter = input.trim().to_string();
                    cx.editor.needs_redraw = true;
                }
            },
        );
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

    pub fn handle_space_op(&mut self, key: KeyEvent, cx: &mut commands::Context) -> bool {
        match key.code {
            KeyCode::Char('c') => self.remember_for_paste(cx, false),
            KeyCode::Char('m') => self.remember_for_paste(cx, true),
            KeyCode::Char('p') => self.paste_clipboard(cx),
            KeyCode::Char('d') => self.delete(cx),
            _ => return false,
        }
        cx.editor.needs_redraw = true;
        true
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
        if let Some(parent) = path.parent() {
            editor.sidebar.refresh_dirs.insert(parent.to_path_buf());
        }
        editor.sidebar.refresh_epoch = editor.sidebar.refresh_epoch.wrapping_add(1);
        editor.sidebar.selected = Some(path.to_path_buf());
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
                        Self::mark_changed(editor, &target);
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
            cx.editor.sidebar.selected = self.selected_row().map(|row| row.path.clone());
            cx.editor.needs_redraw = true;
            return handled;
        }
        if self.pending_g {
            self.pending_g = false;
            match key.code {
                KeyCode::Char('w') => {
                    self.start_jump_labels(cx);
                    cx.editor.needs_redraw = true;
                    return true;
                }
                KeyCode::Char('g') => {
                    self.cursor = 0;
                    cx.editor.needs_redraw = true;
                    return true;
                }
                _ => {}
            }
        }

        if self.pending_z {
            self.pending_z = false;
            match key.code {
                KeyCode::Char('h') | KeyCode::Left => {
                    self.horizontal_scroll = self.horizontal_scroll.saturating_sub(4);
                    cx.editor.needs_redraw = true;
                    return true;
                }
                KeyCode::Char('l') | KeyCode::Right => {
                    self.horizontal_scroll = self.horizontal_scroll.saturating_add(4);
                    cx.editor.needs_redraw = true;
                    return true;
                }
                _ => {}
            }
        }

        if key.code == KeyCode::Esc && !self.filter.is_empty() {
            self.filter.clear();
            cx.editor.sidebar.tree_filter.clear();
            self.rebuild_from_editor(cx.editor);
            cx.editor.needs_redraw = true;
            return true;
        }

        match key.code {
            KeyCode::Char('/') => self.prompt_filter(cx),
            KeyCode::Char('z') => self.pending_z = true,
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
            KeyCode::Char('d') if !key.modifiers.contains(KeyModifiers::CONTROL) => self.delete(cx),
            KeyCode::Char('y') => self.copy_path(cx, false),
            KeyCode::Char('Y') => self.copy_path(cx, true),
            KeyCode::Char('g') => {
                self.pending_g = true;
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
            KeyCode::Tab => {
                if self
                    .selected_row()
                    .is_some_and(|row| row.kind == EntryKind::File)
                {
                    self.open_selected(cx, Action::Replace);
                } else {
                    cx.editor.sidebar.focus = SidebarFocus::Editor;
                }
            }
            KeyCode::Char('q') | KeyCode::Esc => {
                cx.editor.sidebar.focus = SidebarFocus::Editor;
            }
            _ => return false,
        }

        cx.editor.sidebar.selected = self.selected_row().map(|row| row.path.clone());
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
        tree.filter = "needle".to_string();
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
