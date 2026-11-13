use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use helix_core::fuzzy::fuzzy_match;
use helix_core::unicode::width::UnicodeWidthStr;
use helix_core::Position;
use helix_stdx::path::{expand_tilde, home_dir};
use helix_view::document::Mode as DocumentMode;
use helix_view::graphics::{CursorKind, Rect};
use helix_view::input::{KeyCode, KeyEvent, KeyModifiers};
use helix_view::Editor;
use tui::buffer::Buffer as Surface;
use tui::widgets::{Block, BorderType, Widget};

use crate::compositor::{Component, Context, Event, EventResult};
use crate::ui::text_input::TextInput;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Browse,
    Search,
    Path,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerAction {
    None,
    Close,
    Open(PathBuf),
}

pub struct FolderPicker {
    home: PathBuf,
    current: PathBuf,
    entries: Vec<PathBuf>,
    filtered: Vec<usize>,
    selected: usize,
    preview: Vec<PathBuf>,
    preview_error: Option<String>,
    query: TextInput,
    path: TextInput,
    mode: Mode,
    error: Option<String>,
    scroll: usize,
}

impl FolderPicker {
    pub fn new(editor: &Editor) -> Self {
        let mut home =
            canonicalize_or(&home_dir().unwrap_or_else(|_| editor.workspace_root.clone()));
        let cwd = canonicalize_or(&std::env::current_dir().unwrap_or_else(|_| home.clone()));
        if home.as_os_str().is_empty() {
            home = cwd.clone();
        }
        let start = initial_directory(&home, &cwd);
        Self::new_at(&home, &start)
    }

    fn new_at(home: &Path, start: &Path) -> Self {
        let home = canonicalize_or(home);
        let start = canonicalize_or(start);
        let mut picker = Self {
            home,
            current: start.clone(),
            entries: Vec::new(),
            filtered: Vec::new(),
            selected: 0,
            preview: Vec::new(),
            preview_error: None,
            query: TextInput::new(""),
            path: TextInput::new(start.display().to_string()),
            mode: Mode::Browse,
            error: None,
            scroll: 0,
        };
        if let Err(err) = picker.set_directory(start, None) {
            picker.error = Some(format!("Cannot read {}: {err}", picker.current.display()));
        }
        picker
    }

    pub fn current(&self) -> &Path {
        &self.current
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn query(&self) -> &str {
        self.query.value()
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn preview_error(&self) -> Option<&str> {
        self.preview_error.as_deref()
    }

    pub fn visible_paths(&self) -> Vec<&Path> {
        self.filtered
            .iter()
            .filter_map(|&index| self.entries.get(index).map(PathBuf::as_path))
            .collect()
    }

    pub fn preview_paths(&self) -> Vec<&Path> {
        self.preview.iter().map(PathBuf::as_path).collect()
    }

    pub fn selected_path(&self) -> Option<&Path> {
        self.filtered
            .get(self.selected)
            .and_then(|&index| self.entries.get(index))
            .map(PathBuf::as_path)
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> PickerAction {
        match self.mode {
            Mode::Search => self.handle_search_key(key),
            Mode::Path => self.handle_path_key(key),
            Mode::Browse => self.handle_browse_key(key),
        }
    }

    fn handle_search_key(&mut self, key: KeyEvent) -> PickerAction {
        if is_clear_chord(key) {
            self.query.set_value("");
            self.apply_filter(None);
            return PickerAction::None;
        }

        match key.code {
            KeyCode::Enter => {
                self.mode = Mode::Browse;
            }
            KeyCode::Esc => {
                self.query.set_value("");
                self.apply_filter(None);
                self.mode = Mode::Browse;
            }
            KeyCode::Up => self.move_selection(-1),
            KeyCode::Down => self.move_selection(1),
            _ => {
                if self.query.handle_key(key) {
                    self.apply_filter(None);
                }
            }
        }
        PickerAction::None
    }

    fn handle_path_key(&mut self, key: KeyEvent) -> PickerAction {
        if is_clear_chord(key) {
            self.path.set_value("");
            return PickerAction::None;
        }

        match key.code {
            KeyCode::Enter => {
                self.commit_path();
            }
            KeyCode::Esc => {
                self.mode = Mode::Browse;
                self.error = None;
                self.path.set_value(self.current.display().to_string());
            }
            _ => {
                self.path.handle_key(key);
            }
        }
        PickerAction::None
    }

    fn handle_browse_key(&mut self, key: KeyEvent) -> PickerAction {
        let plain = !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER);

        match key.code {
            KeyCode::Esc => {
                if self.query.value().is_empty() {
                    return PickerAction::Close;
                }
                self.query.set_value("");
                self.apply_filter(None);
            }
            KeyCode::Enter => {
                if let Some(path) = self.selected_path() {
                    return PickerAction::Open(path.to_path_buf());
                }
            }
            KeyCode::Up => self.move_selection(-1),
            KeyCode::Down => self.move_selection(1),
            KeyCode::Right | KeyCode::Char(' ') | KeyCode::Char('o') if plain => {
                self.enter_selected();
            }
            KeyCode::Left | KeyCode::Backspace | KeyCode::Delete if plain => {
                self.go_parent();
            }
            KeyCode::Char('p') if plain => {
                self.mode = Mode::Path;
                self.error = None;
                self.path.set_value(self.current.display().to_string());
            }
            KeyCode::Char('/') if plain => {
                self.mode = Mode::Search;
            }
            _ => {}
        }
        PickerAction::None
    }

    fn enter_selected(&mut self) {
        let Some(path) = self.selected_path().map(Path::to_path_buf) else {
            return;
        };
        if let Err(err) = self.set_directory(path.clone(), None) {
            self.error = Some(format!("Cannot read {}: {err}", path.display()));
        }
    }

    fn go_parent(&mut self) {
        let Some(parent) = self.current.parent() else {
            return;
        };
        let parent = parent.to_path_buf();
        if self.current.starts_with(&self.home) && !parent.starts_with(&self.home) {
            return;
        }
        let old = self.current.clone();
        if let Err(err) = self.set_directory(parent.clone(), Some(&old)) {
            self.error = Some(format!("Cannot read {}: {err}", parent.display()));
        }
    }

    fn commit_path(&mut self) {
        let raw = self.path.value().trim().to_string();
        if raw.is_empty() {
            self.error = Some("Path cannot be empty".to_string());
            return;
        }

        let path = expand_tilde(Path::new(&raw)).to_path_buf();
        if !path.is_absolute() {
            self.error = Some(format!("Path must be absolute: {}", path.display()));
            return;
        }

        let path = match fs::canonicalize(&path) {
            Ok(path) => path,
            Err(err) => {
                self.error = Some(format!("Cannot access {}: {err}", path.display()));
                return;
            }
        };
        if !path.is_dir() {
            self.error = Some(format!("Not a directory: {}", path.display()));
            return;
        }

        if let Err(err) = self.set_directory(path.clone(), None) {
            self.error = Some(format!("Cannot read {}: {err}", path.display()));
            return;
        }
        self.mode = Mode::Browse;
        self.error = None;
    }

    fn set_directory(&mut self, path: PathBuf, select: Option<&Path>) -> io::Result<()> {
        let entries = read_directories(&path)?;
        let select = select.map(Path::to_path_buf);
        self.current = path;
        self.entries = entries;
        self.query.set_value("");
        self.filtered = (0..self.entries.len()).collect();
        self.selected = select
            .and_then(|path| self.entries.iter().position(|entry| entry == &path))
            .unwrap_or(0);
        self.scroll = 0;
        self.error = None;
        self.path.set_value(self.current.display().to_string());
        self.update_preview();
        Ok(())
    }

    fn move_selection(&mut self, delta: isize) {
        if self.filtered.is_empty() {
            self.selected = 0;
            self.update_preview();
            return;
        }
        let last = self.filtered.len() - 1;
        self.selected = if delta < 0 {
            self.selected.saturating_sub(delta.unsigned_abs())
        } else {
            (self.selected + delta as usize).min(last)
        };
        self.update_preview();
    }

    fn apply_filter(&mut self, preserve: Option<&Path>) {
        let selected = preserve
            .map(Path::to_path_buf)
            .or_else(|| self.selected_path().map(Path::to_path_buf));
        let query = self.query.value().to_string();
        self.filtered.clear();

        if query.is_empty() {
            self.filtered.extend(0..self.entries.len());
        } else {
            let names: Vec<String> = self
                .entries
                .iter()
                .filter_map(|path| path.file_name())
                .map(|name| name.to_string_lossy().into_owned())
                .collect();
            for (matched, _) in fuzzy_match(&query, names.iter(), true) {
                if let Some(index) = names.iter().position(|name| name == matched) {
                    if !self.filtered.contains(&index) {
                        self.filtered.push(index);
                    }
                }
            }
        }

        self.selected = selected
            .and_then(|path| {
                let entry = self.entries.iter().position(|entry| entry == &path)?;
                self.filtered.iter().position(|&index| index == entry)
            })
            .unwrap_or(0);
        self.scroll = 0;
        self.update_preview();
    }

    fn update_preview(&mut self) {
        self.preview.clear();
        self.preview_error = None;
        let Some(path) = self.selected_path().map(Path::to_path_buf) else {
            return;
        };
        match read_directories(&path) {
            Ok(entries) => self.preview = entries,
            Err(err) => {
                self.preview_error = Some(format!("Cannot read {}: {err}", path.display()));
            }
        }
    }
}

fn initial_directory(home: &Path, cwd: &Path) -> PathBuf {
    if cwd.starts_with(home) {
        cwd.to_path_buf()
    } else {
        home.to_path_buf()
    }
}

fn canonicalize_or(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn is_clear_chord(key: KeyEvent) -> bool {
    key.modifiers
        .intersects(KeyModifiers::SUPER | KeyModifiers::ALT)
        && matches!(key.code, KeyCode::Backspace | KeyCode::Delete)
}

fn read_directories(path: &Path) -> io::Result<Vec<PathBuf>> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let is_dir = if file_type.is_dir() {
            true
        } else if file_type.is_symlink() {
            fs::metadata(entry.path())
                .map(|metadata| metadata.is_dir())
                .unwrap_or(false)
        } else {
            false
        };
        if is_dir {
            entries.push(entry.path());
        }
    }
    entries.sort_by_key(|path| directory_name(path));
    Ok(entries)
}

fn directory_name(path: &Path) -> (String, String) {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    (name.to_lowercase(), name)
}

impl Component for FolderPicker {
    fn render(&mut self, area: Rect, surface: &mut Surface, cx: &mut Context) {
        if area.width < 12 || area.height < 6 {
            return;
        }

        let theme = &cx.editor.theme;
        let text = theme.get("ui.text");
        let selected_style = theme.get("ui.menu.selected");
        let dim = theme.get("ui.linenr");
        let focus = theme.get("ui.text.focus");
        let error_style = theme.get("error");
        let directory_style = theme.get("ui.text.directory");
        let background = theme.get("ui.background");
        let border_style = theme
            .try_get_exact("ui.window")
            .or_else(|| theme.try_get_exact("ui.linenr"))
            .unwrap_or_default();

        surface.clear_with(area, background);
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(border_style)
            .title(" Open folder ");
        let inner = block.inner(area);
        block.render(area, surface);
        if inner.width < 8 || inner.height < 4 {
            return;
        }

        let full_width = inner.width as usize;
        let search_active = self.mode == Mode::Search;
        let search_label = if search_active {
            "▸ Search: "
        } else {
            "  Search: "
        };
        let search_label_width = search_label.width() as u16;
        surface.set_stringn(
            inner.x,
            inner.y,
            search_label,
            full_width,
            if search_active { focus } else { dim },
        );
        let search_width = inner.width.saturating_sub(search_label_width);
        if search_width > 0 {
            self.query.render(
                Rect::new(inner.x + search_label_width, inner.y, search_width, 1),
                surface,
                theme,
                search_active,
            );
        }

        let divider_y = inner.y + 1;
        if divider_y < inner.y + inner.height {
            surface.set_stringn(inner.x, divider_y, &"─".repeat(full_width), full_width, dim);
        }

        let path_y = inner.y + inner.height - 1;
        let hints_y = path_y.saturating_sub(1);
        let error_y = self.error.as_ref().map(|_| hints_y.saturating_sub(1));
        let pane_y = divider_y.saturating_add(1);
        let pane_bottom = error_y.unwrap_or(hints_y);
        let pane_height = pane_bottom.saturating_sub(pane_y);

        if pane_height > 0 {
            let split = inner.width / 2;
            let left_area = Rect::new(inner.x, pane_y, split.saturating_sub(1), pane_height);
            let right_x = inner.x.saturating_add(split).saturating_add(1);
            let right_area = Rect::new(
                right_x,
                pane_y,
                inner.width.saturating_sub(split).saturating_sub(1),
                pane_height,
            );
            if split > 0 {
                for y in pane_y..pane_y + pane_height {
                    surface.set_string(inner.x + split, y, "│", dim);
                }
            }

            let visible: Vec<PathBuf> = self
                .filtered
                .iter()
                .filter_map(|&index| self.entries.get(index).cloned())
                .collect();
            let height = pane_height as usize;
            if self.selected < self.scroll {
                self.scroll = self.selected;
            } else if self.selected >= self.scroll + height {
                self.scroll = self.selected + 1 - height;
            }

            if visible.is_empty() {
                let message = if self.query.value().is_empty() {
                    "No subfolders"
                } else {
                    "No matches"
                };
                surface.set_stringn(
                    left_area.x,
                    left_area.y,
                    message,
                    left_area.width as usize,
                    dim,
                );
            } else {
                for (offset, path) in visible.iter().skip(self.scroll).take(height).enumerate() {
                    let index = self.scroll + offset;
                    let style = if index == self.selected {
                        selected_style
                    } else {
                        text
                    };
                    let name = display_name(path);
                    surface.set_stringn(
                        left_area.x,
                        left_area.y + offset as u16,
                        &name,
                        left_area.width as usize,
                        style,
                    );
                }
            }

            if let Some(error) = &self.preview_error {
                surface.set_stringn(
                    right_area.x,
                    right_area.y,
                    error,
                    right_area.width as usize,
                    error_style,
                );
            } else if self.preview.is_empty() {
                surface.set_stringn(
                    right_area.x,
                    right_area.y,
                    "No subfolders",
                    right_area.width as usize,
                    dim,
                );
            } else {
                for (offset, path) in self
                    .preview
                    .iter()
                    .take(right_area.height as usize)
                    .enumerate()
                {
                    surface.set_stringn(
                        right_area.x,
                        right_area.y + offset as u16,
                        &display_name(path),
                        right_area.width as usize,
                        directory_style,
                    );
                }
            }
        }

        if let (Some(error_y), Some(error)) = (error_y, self.error.as_ref()) {
            surface.set_stringn(inner.x, error_y, error, full_width, error_style);
        }

        let hints =
            "/ search · p path · Enter open · →/Space/o enter · ←/Del/Bksp parent · Esc close";
        surface.set_stringn(inner.x, hints_y, hints, full_width, dim);

        let path_active = self.mode == Mode::Path;
        let path_label = if path_active {
            "▸ Path: "
        } else {
            "  Path: "
        };
        let path_label_width = path_label.width() as u16;
        surface.set_stringn(
            inner.x,
            path_y,
            path_label,
            full_width,
            if path_active { focus } else { dim },
        );
        let path_width = inner.width.saturating_sub(path_label_width);
        if path_width > 0 {
            self.path.render(
                Rect::new(inner.x + path_label_width, path_y, path_width, 1),
                surface,
                theme,
                path_active,
            );
        }
    }

    fn handle_event(&mut self, event: &Event, cx: &mut Context) -> EventResult {
        let Event::Key(key) = event else {
            if let Event::Paste(contents) = event {
                match self.mode {
                    Mode::Search => {
                        self.query.insert_str(contents);
                        self.apply_filter(None);
                    }
                    Mode::Path => self.path.insert_str(contents),
                    Mode::Browse => return EventResult::Consumed(None),
                }
                cx.editor.needs_redraw = true;
            }
            return EventResult::Consumed(None);
        };

        match self.handle_key(*key) {
            PickerAction::None => {
                cx.editor.needs_redraw = true;
                EventResult::Consumed(None)
            }
            PickerAction::Close => EventResult::Consumed(Some(Box::new(
                |compositor: &mut crate::compositor::Compositor, _cx| {
                    compositor.pop();
                },
            ))),
            PickerAction::Open(path) => {
                let name = path.display().to_string();
                cx.editor.set_status(format!("Opening {name}…"));
                cx.editor.needs_redraw = true;
                crate::job::dispatch_blocking(move |editor, _compositor| {
                    crate::commands::switch_workspace(editor, &path);
                    editor.set_status(format!("Opened {name}"));
                });
                EventResult::Consumed(Some(Box::new(
                    |compositor: &mut crate::compositor::Compositor, _cx| {
                        compositor.pop();
                    },
                )))
            }
        }
    }

    fn cursor(&self, area: Rect, editor: &Editor) -> (Option<Position>, CursorKind) {
        let block = Block::bordered();
        let inner = block.inner(area);
        if inner.width == 0 || inner.height == 0 {
            return (None, CursorKind::Hidden);
        }

        let cursor = match self.mode {
            Mode::Search => {
                let label = "▸ Search: ";
                let label_width = label.width() as u16;
                let width = inner.width.saturating_sub(label_width);
                (width > 0).then(|| {
                    let column = self.query.cursor_column();
                    let offset = column.saturating_sub(width.saturating_sub(1) as usize);
                    Position::new(
                        inner.y as usize,
                        (inner.x + label_width + column.saturating_sub(offset) as u16) as usize,
                    )
                })
            }
            Mode::Path => {
                let label = "▸ Path: ";
                let label_width = label.width() as u16;
                let width = inner.width.saturating_sub(label_width);
                (width > 0).then(|| {
                    let column = self.path.cursor_column();
                    let offset = column.saturating_sub(width.saturating_sub(1) as usize);
                    Position::new(
                        (inner.y + inner.height - 1) as usize,
                        (inner.x + label_width + column.saturating_sub(offset) as u16) as usize,
                    )
                })
            }
            Mode::Browse => None,
        };

        match cursor {
            Some(position) => (
                Some(position),
                editor.config().cursor_shape.from_mode(DocumentMode::Insert),
            ),
            None => (None, CursorKind::Hidden),
        }
    }

    fn required_size(&mut self, _viewport: (u16, u16)) -> Option<(u16, u16)> {
        Some(_viewport)
    }

    fn id(&self) -> Option<&'static str> {
        Some("folder-picker")
    }
}

fn display_name(path: &Path) -> String {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    format!("{name}/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use helix_view::input::{KeyCode, KeyModifiers};
    use std::path::Path;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn key_with(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent { code, modifiers }
    }

    fn type_text(picker: &mut FolderPicker, text: &str) {
        for c in text.chars() {
            picker.handle_key(key(KeyCode::Char(c)));
        }
    }

    fn names(paths: Vec<&Path>) -> Vec<String> {
        paths
            .into_iter()
            .map(|path| {
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect()
    }

    fn home_with_dirs(names: &[&str]) -> tempfile::TempDir {
        let home = tempfile::tempdir().unwrap();
        for name in names {
            std::fs::create_dir(home.path().join(name)).unwrap();
        }
        home
    }

    #[test]
    fn starts_at_cwd_when_it_is_inside_home() {
        let home = Path::new("/home/user");
        let cwd = Path::new("/home/user/project");
        assert_eq!(initial_directory(home, cwd), cwd);
    }

    #[test]
    fn starts_at_home_when_cwd_is_outside_home() {
        let home = Path::new("/home/user");
        let cwd = Path::new("/tmp/project");
        assert_eq!(initial_directory(home, cwd), home);
    }

    #[test]
    fn lists_only_direct_child_directories_including_hidden_ones() {
        let home = home_with_dirs(&["alpha", ".hidden"]);
        std::fs::write(home.path().join("file.txt"), "file").unwrap();

        let picker = FolderPicker::new_at(home.path(), home.path());
        assert_eq!(names(picker.visible_paths()), vec![".hidden", "alpha"]);
    }

    #[cfg(unix)]
    #[test]
    fn lists_symlinked_directories() {
        let home = home_with_dirs(&["target"]);
        std::os::unix::fs::symlink(home.path().join("target"), home.path().join("link")).unwrap();

        let picker = FolderPicker::new_at(home.path(), home.path());
        assert_eq!(names(picker.visible_paths()), vec!["link", "target"]);
    }

    #[test]
    fn listings_are_lazy_and_preview_only_shows_direct_children() {
        let home = home_with_dirs(&["alpha", "beta"]);
        std::fs::create_dir_all(home.path().join("alpha/child")).unwrap();
        std::fs::create_dir_all(home.path().join("alpha/child/grandchild")).unwrap();

        let mut picker = FolderPicker::new_at(home.path(), home.path());
        assert_eq!(names(picker.visible_paths()), vec!["alpha", "beta"]);
        assert_eq!(names(picker.preview_paths()), vec!["child"]);

        assert_eq!(picker.handle_key(key(KeyCode::Right)), PickerAction::None);
        assert_eq!(
            picker.current(),
            std::fs::canonicalize(home.path().join("alpha")).unwrap()
        );
        assert_eq!(names(picker.visible_paths()), vec!["child"]);
        assert_eq!(names(picker.preview_paths()), vec!["grandchild"]);
    }

    #[test]
    fn parent_reselects_the_directory_that_was_left() {
        let home = home_with_dirs(&["alpha", "beta"]);

        let mut picker = FolderPicker::new_at(home.path(), home.path());
        assert_eq!(names(picker.visible_paths()), vec!["alpha", "beta"]);
        assert_eq!(picker.handle_key(key(KeyCode::Right)), PickerAction::None);
        assert_eq!(
            picker.current(),
            std::fs::canonicalize(home.path().join("alpha")).unwrap()
        );

        assert_eq!(picker.handle_key(key(KeyCode::Left)), PickerAction::None);
        assert_eq!(
            picker.current(),
            std::fs::canonicalize(home.path()).unwrap()
        );
        assert_eq!(
            picker.selected_path(),
            Some(
                std::fs::canonicalize(home.path().join("alpha"))
                    .unwrap()
                    .as_path()
            )
        );
    }

    #[test]
    fn parent_navigation_stops_at_home() {
        let home = home_with_dirs(&[]);
        let mut picker = FolderPicker::new_at(home.path(), home.path());

        assert_eq!(
            picker.handle_key(key(KeyCode::Backspace)),
            PickerAction::None
        );
        assert_eq!(
            picker.current(),
            std::fs::canonicalize(home.path()).unwrap()
        );
    }

    #[test]
    fn search_filters_current_items_and_enter_stabilizes_results() {
        let home = home_with_dirs(&["alpha", "beta"]);
        let mut picker = FolderPicker::new_at(home.path(), home.path());

        assert_eq!(
            picker.handle_key(key(KeyCode::Char('/'))),
            PickerAction::None
        );
        type_text(&mut picker, "alp");
        assert_eq!(picker.mode(), Mode::Search);
        assert_eq!(picker.query(), "alp");
        assert_eq!(names(picker.visible_paths()), vec!["alpha"]);

        assert_eq!(picker.handle_key(key(KeyCode::Enter)), PickerAction::None);
        assert_eq!(picker.mode(), Mode::Browse);
        assert_eq!(names(picker.visible_paths()), vec!["alpha"]);

        type_text(&mut picker, "b");
        assert_eq!(picker.query(), "alp");
        assert_eq!(names(picker.visible_paths()), vec!["alpha"]);

        assert_eq!(picker.handle_key(key(KeyCode::Esc)), PickerAction::None);
        assert_eq!(picker.query(), "");
        assert_eq!(names(picker.visible_paths()), vec!["alpha", "beta"]);
        assert_eq!(picker.handle_key(key(KeyCode::Esc)), PickerAction::Close);

        assert_eq!(
            picker.handle_key(key(KeyCode::Char('/'))),
            PickerAction::None
        );
        assert_eq!(picker.mode(), Mode::Search);
        assert_eq!(
            picker.handle_key(key_with(KeyCode::Backspace, KeyModifiers::SUPER)),
            PickerAction::None
        );
        assert_eq!(picker.query(), "");
        assert_eq!(picker.mode(), Mode::Search);
        assert_eq!(names(picker.visible_paths()), vec!["alpha", "beta"]);

        assert_eq!(picker.handle_key(key(KeyCode::Esc)), PickerAction::None);
        assert_eq!(picker.mode(), Mode::Browse);
        assert_eq!(picker.query(), "");
    }

    #[test]
    fn entering_a_directory_clears_the_search() {
        let home = home_with_dirs(&["alpha"]);
        let mut picker = FolderPicker::new_at(home.path(), home.path());
        picker.handle_key(key(KeyCode::Char('/')));
        type_text(&mut picker, "alp");
        picker.handle_key(key(KeyCode::Enter));

        assert_eq!(picker.handle_key(key(KeyCode::Right)), PickerAction::None);
        assert_eq!(picker.query(), "");
        assert_eq!(picker.mode(), Mode::Browse);
    }

    #[test]
    fn path_editing_navigates_to_existing_directories_and_reports_errors() {
        let home = home_with_dirs(&["alpha"]);
        let other = tempfile::tempdir().unwrap();
        std::fs::create_dir(other.path().join("outside")).unwrap();
        let file = home.path().join("file.txt");
        std::fs::write(&file, "file").unwrap();

        let mut picker = FolderPicker::new_at(home.path(), home.path());
        assert_eq!(
            picker.handle_key(key(KeyCode::Char('p'))),
            PickerAction::None
        );
        assert_eq!(picker.mode(), Mode::Path);

        picker.path.set_value(file.display().to_string());
        assert_eq!(picker.handle_key(key(KeyCode::Enter)), PickerAction::None);
        assert!(picker.error().is_some());
        assert_eq!(picker.mode(), Mode::Path);
        assert_eq!(
            picker.current(),
            std::fs::canonicalize(home.path()).unwrap()
        );

        picker
            .path
            .set_value(another_dir(home.path(), "alpha").display().to_string());
        assert_eq!(picker.handle_key(key(KeyCode::Enter)), PickerAction::None);
        assert_eq!(picker.mode(), Mode::Browse);
        assert_eq!(
            picker.current(),
            std::fs::canonicalize(home.path().join("alpha")).unwrap()
        );

        picker.handle_key(key(KeyCode::Char('p')));
        picker
            .path
            .set_value(other.path().join("outside").display().to_string());
        assert_eq!(picker.handle_key(key(KeyCode::Enter)), PickerAction::None);
        assert_eq!(
            picker.current(),
            std::fs::canonicalize(other.path().join("outside")).unwrap()
        );
    }

    #[test]
    fn path_clear_chord_clears_the_field_and_keeps_editing() {
        let home = home_with_dirs(&["alpha"]);
        let mut picker = FolderPicker::new_at(home.path(), home.path());

        picker.handle_key(key(KeyCode::Char('p')));
        type_text(&mut picker, "draft");
        assert_eq!(
            picker.handle_key(key_with(KeyCode::Backspace, KeyModifiers::ALT)),
            PickerAction::None
        );

        assert_eq!(picker.mode(), Mode::Path);
        assert_eq!(picker.path.value(), "");

        type_text(&mut picker, "x");
        assert_eq!(picker.path.value(), "x");
    }

    #[test]
    fn esc_cancels_path_editing_without_changing_directory() {
        let home = home_with_dirs(&["alpha"]);
        let mut picker = FolderPicker::new_at(home.path(), home.path());

        picker.handle_key(key(KeyCode::Char('p')));
        picker.path.set_value("/definitely/not/here");
        assert_eq!(picker.handle_key(key(KeyCode::Esc)), PickerAction::None);

        assert_eq!(picker.mode(), Mode::Browse);
        assert_eq!(
            picker.current(),
            std::fs::canonicalize(home.path()).unwrap()
        );
        assert!(picker.error().is_none());
    }

    #[test]
    fn enter_opens_the_selected_folder_but_not_while_editing_path() {
        let home = home_with_dirs(&["alpha"]);

        let mut picker = FolderPicker::new_at(home.path(), home.path());
        assert_eq!(
            picker.handle_key(key(KeyCode::Enter)),
            PickerAction::Open(std::fs::canonicalize(home.path().join("alpha")).unwrap())
        );

        let mut picker = FolderPicker::new_at(home.path(), home.path());
        picker.handle_key(key(KeyCode::Char('p')));
        assert_eq!(picker.handle_key(key(KeyCode::Enter)), PickerAction::None);
    }

    fn another_dir(root: &Path, name: &str) -> PathBuf {
        root.join(name)
    }
}
