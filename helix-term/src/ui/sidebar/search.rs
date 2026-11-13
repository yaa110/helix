use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};

use helix_core::unicode::width::UnicodeWidthStr;
use helix_core::Position;
use helix_view::document::Mode;
use helix_view::graphics::{CursorKind, Rect};
use helix_view::info::Info;
use helix_view::input::{KeyCode, KeyModifiers, MouseEvent};
use helix_view::sidebar::SearchInputs;
use helix_view::Editor;
use tui::buffer::Buffer as Surface;
use tui::widgets::{Block, BorderType, Widget};

use crate::commands;
use crate::compositor::{Event, EventResult};
use crate::search::{self, Match, SearchEvent, SearchOptions};
use crate::ui::text_input::TextInput;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchFocus {
    Query,
    Replace,
    Results,
}

#[derive(Debug, Default, Clone)]
pub struct FileResults {
    pub matches: Vec<Match>,
    pub collapsed: bool,
}

#[derive(Debug, Clone)]
enum ResultRow {
    File(PathBuf),
    Match(PathBuf, usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReplaceScope {
    Item,
    All,
}

#[derive(Debug, Clone)]
struct ReplacePreview {
    scope: ReplaceScope,
    lines: Vec<String>,
    scroll: usize,
}

pub struct SearchPane {
    query: TextInput,
    replacement: TextInput,
    pub focus: SearchFocus,
    results: BTreeMap<PathBuf, FileResults>,
    rows: Vec<ResultRow>,
    cursor: usize,
    scroll: usize,
    scroll_accel: super::super::scroll::ScrollAccelerator,
    results_area: Rect,
    preview_area: Rect,
    results_height: usize,
    preview_height: usize,
    receiver: Option<Receiver<SearchEvent>>,
    pending: bool,
    editing: bool,
    options_mode: bool,
    replace_preview: Option<ReplacePreview>,
    cursor_pos: Option<Position>,
    capped: bool,
    error: Option<String>,
}

impl SearchPane {
    pub fn new(inputs: &SearchInputs) -> Self {
        Self {
            query: TextInput::new(inputs.query.clone()),
            replacement: TextInput::new(inputs.replacement.clone()),
            focus: SearchFocus::Results,
            results: BTreeMap::new(),
            rows: Vec::new(),
            cursor: 0,
            scroll: 0,
            scroll_accel: Default::default(),
            results_area: Rect::new(0, 0, 0, 0),
            preview_area: Rect::new(0, 0, 0, 0),
            results_height: 0,
            preview_height: 0,
            receiver: None,
            pending: false,
            editing: false,
            options_mode: false,
            replace_preview: None,
            cursor_pos: None,
            capped: false,
            error: None,
        }
    }

    pub fn sync_from_inputs(&mut self, inputs: &SearchInputs) {
        if self.query.value() != inputs.query {
            self.query.set_value(inputs.query.clone());
        }
        if self.replacement.value() != inputs.replacement {
            self.replacement.set_value(inputs.replacement.clone());
        }
    }

    /// Called when the search window is opened. Results mode is the default:
    /// typing does nothing until `/` is pressed.
    pub fn focus_query(&mut self, editor: &mut Editor) {
        self.focus = SearchFocus::Results;
        self.editing = false;
        self.close_options(editor);
    }

    pub fn result_count(&self) -> usize {
        self.results.values().map(|file| file.matches.len()).sum()
    }

    pub fn options_open(&self) -> bool {
        self.options_mode
    }

    pub fn editing(&self) -> bool {
        self.editing
    }

    pub fn replace_preview_open(&self) -> bool {
        self.replace_preview.is_some()
    }

    pub fn cursor(&self, editor: &Editor) -> Option<(Position, CursorKind)> {
        self.cursor_pos
            .map(|pos| (pos, editor.config().cursor_shape.from_mode(Mode::Insert)))
    }

    pub fn handle_mouse_scroll(&mut self, event: &MouseEvent, editor: &mut Editor) -> bool {
        let max_lines = editor.config().scroll_lines.unsigned_abs();
        let Some(delta) = super::super::scroll::wheel_delta(event.kind, max_lines) else {
            return false;
        };
        let step = self
            .scroll_accel
            .step(delta.signum(), max_lines, std::time::Instant::now());
        if step == 0 {
            return true;
        }

        if self.replace_preview.is_some() {
            if super::super::scroll::contains(self.preview_area, event.column, event.row) {
                if let Some(preview) = self.replace_preview.as_mut() {
                    preview.scroll = super::super::scroll::scroll_offset(
                        preview.scroll,
                        step,
                        preview.lines.len(),
                        self.preview_height,
                    );
                }
                editor.needs_redraw = true;
            }
            return true;
        }

        if super::super::scroll::contains(self.results_area, event.column, event.row) {
            self.scroll = super::super::scroll::scroll_offset(
                self.scroll,
                step,
                self.rows.len(),
                self.results_height,
            );
            self.cursor = super::super::scroll::clamp_selection(
                self.cursor,
                self.scroll,
                self.rows.len(),
                self.results_height,
            );
            editor.needs_redraw = true;
        }
        true
    }

    fn begin_edit(&mut self, cx: &mut commands::Context, focus: SearchFocus) {
        self.close_options(cx.editor);
        self.editing = true;
        self.focus = focus;
        cx.editor.needs_redraw = true;
    }

    fn finish_edit(&mut self, cx: &mut commands::Context, run_search: bool) {
        self.editing = false;
        self.focus = SearchFocus::Results;
        if run_search {
            self.pending = true;
        }
        cx.editor.needs_redraw = true;
    }

    fn sync_inputs(&self, editor: &mut Editor) {
        let inputs = &mut editor.sidebar.search;
        inputs.query = self.query.value().to_string();
        inputs.replacement = self.replacement.value().to_string();
    }

    pub fn handle_idle_timeout(&mut self, editor: &Editor) {
        if !self.pending {
            return;
        }
        self.pending = false;
        let inputs = &editor.sidebar.search;
        if inputs.query.is_empty() {
            self.results.clear();
            self.rows.clear();
            self.receiver = None;
            self.error = None;
            self.capped = false;
            return;
        }
        let options = SearchOptions {
            match_case: inputs.match_case,
            whole_word: inputs.whole_word,
            regex: inputs.regex,
            preserve_case: inputs.preserve_case,
        };
        if let Err(err) = search::build_regex(&inputs.query, options) {
            self.error = Some(err.to_string());
            return;
        }

        self.error = None;
        self.capped = false;
        self.results.clear();
        self.rows.clear();
        self.cursor = 0;

        let open_docs: Vec<(PathBuf, String)> = editor
            .documents()
            .filter_map(|doc| {
                doc.path()
                    .map(|path| (path.to_path_buf(), doc.text().to_string()))
            })
            .collect();
        let (tx, rx) = std::sync::mpsc::channel();
        self.receiver = Some(rx);
        let query = inputs.query.clone();
        let root = editor.workspace_root.clone();
        let handle = tokio::runtime::Handle::current();
        std::thread::spawn(move || {
            let _guard = handle.enter();
            search::spawn_search(root, query, options, open_docs, tx, 10_000);
        });
    }

    pub fn render(&mut self, area: Rect, surface: &mut Surface, editor: &mut Editor) {
        self.drain();
        self.rebuild_rows();
        if self.results.is_empty() {
            editor.sidebar.search_files.clear();
        } else {
            editor.sidebar.search_files = self.results.keys().cloned().collect();
        }
        let match_count = self.result_count();
        let file_count = self.results.len();

        if area.width <= 2 || area.height == 0 {
            return;
        }

        let theme = &editor.theme;
        let text = theme.get("ui.text");
        let selected = theme.get("ui.menu.selected");
        let dim = theme.get("ui.linenr");
        let label = theme.get("ui.sidebar.title");
        let focus_style = theme.get("ui.text.focus");
        let width = area.width.saturating_sub(2);
        let max_x = area.x + area.width - 1;
        let mut y = area.y;
        self.cursor_pos = None;

        // Query input. Labels make it obvious which field is active, and
        // TextInput clips/scolls long queries internally.
        let query_active = self.editing && self.focus == SearchFocus::Query;
        let query_label = if query_active {
            "▸ Find: "
        } else {
            "  Find: "
        };
        let query_label_width = query_label.width() as u16;
        surface.set_stringn(
            area.x + 1,
            y,
            query_label,
            width as usize,
            if query_active { focus_style } else { label },
        );
        let query_width = width.saturating_sub(query_label_width);
        if query_width > 0 {
            self.query.render(
                Rect::new(area.x + 1 + query_label_width, y, query_width, 1),
                surface,
                theme,
                query_active,
            );
            if query_active {
                let cursor_column = self.query.cursor_column();
                let offset = cursor_column.saturating_sub(query_width.saturating_sub(1) as usize);
                let cursor_x =
                    area.x + 1 + query_label_width + cursor_column.saturating_sub(offset) as u16;
                self.cursor_pos = Some(Position::new(y as usize, cursor_x as usize));
            }
        }
        y += 1;

        // Search options, wrapped across as many rows as the pane needs.
        let options = [
            ("[Aa] case", editor.sidebar.search.match_case),
            ("[ab] word", editor.sidebar.search.whole_word),
            ("[.*] regex", editor.sidebar.search.regex),
            ("[AB] keep", editor.sidebar.search.preserve_case),
        ];
        let mut x = area.x + 1;
        let mut option_y = y;
        for (option, enabled) in options {
            let option_width = option.len() as u16;
            if x.saturating_add(option_width) > max_x {
                x = area.x + 1;
                option_y += 1;
            }
            if option_y >= area.y + area.height {
                break;
            }
            surface.set_stringn(
                x,
                option_y,
                option,
                option_width as usize,
                if enabled { selected } else { dim },
            );
            x = x.saturating_add(option_width + 1);
        }
        if option_y < area.y + area.height {
            y = option_y + 1;
        } else {
            y = area.y + area.height;
        }

        if y < area.y + area.height {
            let (focus_name, hints) = if self.editing {
                if self.focus == SearchFocus::Replace {
                    (
                        "Replace",
                        "Enter search · Tab Find · Esc results".to_string(),
                    )
                } else {
                    (
                        "Find",
                        "Enter search · Tab Replace · Esc results".to_string(),
                    )
                }
            } else if self.pending || self.receiver.is_some() {
                (
                    "Results",
                    format!("Searching… ({match_count} matches so far)"),
                )
            } else {
                (
                    "Results",
                    format!(
                        "{match_count} matches in {file_count} files · j/k move · Enter open · r preview replace · R preview all · / edit"
                    ),
                )
            };
            let prefix = format!("▸ {focus_name}  ");
            let prefix_width = prefix.width() as u16;
            surface.set_stringn(area.x + 1, y, &prefix, width as usize, focus_style);
            if prefix_width < width {
                surface.set_stringn(
                    area.x + 1 + prefix_width,
                    y,
                    &hints,
                    width.saturating_sub(prefix_width) as usize,
                    dim,
                );
            }
            y += 1;
        }

        // Replacement input.
        let replace_active = self.editing && self.focus == SearchFocus::Replace;
        let replace_label = if replace_active {
            "▸ Replace: "
        } else {
            "  Replace: "
        };
        let replace_label_width = replace_label.width() as u16;
        if y < area.y + area.height {
            surface.set_stringn(
                area.x + 1,
                y,
                replace_label,
                width as usize,
                if replace_active { focus_style } else { label },
            );
            let replace_width = width.saturating_sub(replace_label_width);
            if replace_width > 0 {
                self.replacement.render(
                    Rect::new(area.x + 1 + replace_label_width, y, replace_width, 1),
                    surface,
                    theme,
                    replace_active,
                );
                if replace_active {
                    let cursor_column = self.replacement.cursor_column();
                    let offset =
                        cursor_column.saturating_sub(replace_width.saturating_sub(1) as usize);
                    let cursor_x = area.x
                        + 1
                        + replace_label_width
                        + cursor_column.saturating_sub(offset) as u16;
                    self.cursor_pos = Some(Position::new(y as usize, cursor_x as usize));
                }
            }
            y += 1;
        }

        if y < area.y + area.height {
            surface.set_stringn(area.x, y, &"─".repeat(width as usize), width as usize, dim);
            y += 1;
        }

        if let Some(error) = &self.error {
            if y < area.y + area.height {
                surface.set_stringn(area.x + 1, y, error, width as usize, theme.get("error"));
            }
            return;
        }

        let height = (area.y + area.height).saturating_sub(y) as usize;
        if height == 0 {
            return;
        }
        self.results_area = Rect::new(area.x, y, area.width, height as u16);
        self.results_height = height;

        if editor.sidebar.search.query.is_empty() {
            let empty_hint = if self.editing {
                "Type to search · Enter to run · Tab for Replace"
            } else {
                "Press / to search · Space options"
            };
            surface.set_stringn(area.x + 1, y, empty_hint, width as usize, dim);
            return;
        }
        if self.pending || self.receiver.is_some() {
            let text = format!("Searching… ({match_count} matches so far)");
            surface.set_stringn(area.x + 1, y, &text, width as usize, dim);
            return;
        }
        if self.results.is_empty() {
            surface.set_stringn(area.x + 1, y, "No results found", width as usize, dim);
            return;
        }

        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        } else if self.cursor >= self.scroll + height {
            self.scroll = self.cursor + 1 - height;
        }

        for (offset, row) in self.rows.iter().skip(self.scroll).take(height).enumerate() {
            let index = self.scroll + offset;
            let style = if index == self.cursor { selected } else { text };
            let line = match row {
                ResultRow::File(path) => {
                    let count = self
                        .results
                        .get(path)
                        .map(|file| file.matches.len())
                        .unwrap_or(0);
                    format!(
                        "▾ {} ({count})",
                        path.file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_else(|| path.display().to_string())
                    )
                }
                ResultRow::Match(_, _) => {
                    let matched = self.selected_match_at(index);
                    match matched {
                        Some(matched) => {
                            format!("  {}: {}", matched.line + 1, matched.preview)
                        }
                        None => String::new(),
                    }
                }
            };
            surface.set_stringn(area.x, y + offset as u16, &line, area.width as usize, style);
        }

        if self.capped && area.height > 0 {
            surface.set_stringn(
                area.x,
                area.y + area.height - 1,
                "showing first 10000 matches",
                area.width as usize,
                dim,
            );
        }
    }

    pub fn render_modal(&mut self, area: Rect, surface: &mut Surface, editor: &mut Editor) {
        if area.width < 20 || area.height < 8 {
            return;
        }
        // Use nearly the whole screen so search results and replace previews
        // have room to breathe.
        let width = area.width.saturating_sub(4);
        let height = area.height.saturating_sub(4);
        let modal = Rect::new(
            area.x + (area.width - width) / 2,
            area.y + (area.height - height) / 2,
            width,
            height,
        );
        let background = editor.theme.get("ui.background");
        let border = editor
            .theme
            .try_get_exact("ui.window")
            .or_else(|| editor.theme.try_get_exact("ui.linenr"))
            .unwrap_or_default();
        let divider_style = editor.theme.get("ui.linenr");
        surface.clear_with(modal, background);
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(border);
        let inner = block.inner(modal);
        block.render(modal, surface);

        if inner.width < 20 || inner.height < 3 {
            return;
        }

        if self.replace_preview.is_some() {
            self.render_replace_preview(inner, surface, editor);
            return;
        }

        let preview = if inner.width >= 80 {
            Some(inner.width * 45 / 100)
        } else {
            None
        };
        let left_width = preview.unwrap_or(inner.width);
        let left = Rect::new(inner.x, inner.y, left_width, inner.height);
        let right = preview.map(|width| {
            Rect::new(
                inner.x + width + 1,
                inner.y,
                inner.width.saturating_sub(width + 1),
                inner.height,
            )
        });

        self.render(left, surface, editor);

        if let Some(width) = preview {
            for y in inner.y..inner.y + inner.height {
                surface.set_string(inner.x + width, y, "│", divider_style);
            }
            if let Some(right) = right {
                self.render_preview(right, surface, editor);
            }
        }
    }

    fn render_replace_preview(&mut self, area: Rect, surface: &mut Surface, editor: &Editor) {
        self.preview_area = area;
        self.preview_height = area.height as usize;
        let theme = &editor.theme;
        surface.clear_with(area, theme.get("ui.background"));
        let Some(preview) = self.replace_preview.as_mut() else {
            return;
        };

        let title = "Replace preview — Enter confirm · Esc cancel · j/k scroll";
        surface.set_stringn(
            area.x + 1,
            area.y,
            title,
            area.width.saturating_sub(1) as usize,
            theme.get("ui.text.focus"),
        );

        let visible = area.height.saturating_sub(2) as usize;
        if visible == 0 {
            return;
        }
        preview.scroll = preview
            .scroll
            .min(preview.lines.len().saturating_sub(visible));
        for (index, line) in preview
            .lines
            .iter()
            .skip(preview.scroll)
            .take(visible)
            .enumerate()
        {
            let style = if line.starts_with("+ ") {
                theme.get("diff.plus")
            } else if line.starts_with("- ") {
                theme.get("diff.minus")
            } else {
                theme.get("ui.linenr")
            };
            surface.set_stringn(
                area.x + 1,
                area.y + 1 + index as u16,
                line,
                area.width.saturating_sub(1) as usize,
                style,
            );
        }
    }

    fn render_preview(&mut self, area: Rect, surface: &mut Surface, editor: &Editor) {
        self.preview_area = area;
        if area.width == 0 || area.height == 0 {
            return;
        }
        let theme = &editor.theme;
        let Some(matched) = self.selected_match() else {
            surface.set_stringn(
                area.x,
                area.y,
                "Select a result to preview",
                area.width as usize,
                theme.get("ui.text.inactive"),
            );
            return;
        };

        let text = if let Some(doc_id) = editor.document_id_by_path(&matched.path) {
            editor
                .document(doc_id)
                .map(|doc| doc.text().to_string())
                .unwrap_or_default()
        } else {
            std::fs::read_to_string(&matched.path).unwrap_or_default()
        };
        let lines: Vec<&str> = text.lines().collect();
        if lines.is_empty() {
            return;
        }
        let current = matched.line.min(lines.len().saturating_sub(1));
        let start = current.saturating_sub(8);
        let end = (current + 9).min(lines.len());
        let header = format!("{}:{}", matched.path.display(), current + 1);
        surface.set_stringn(
            area.x,
            area.y,
            &header,
            area.width as usize,
            theme.get("ui.text.inactive"),
        );
        let height = area.height.saturating_sub(1) as usize;
        let number_width = (end + 1).to_string().len();
        let text_style = theme.get("ui.text");
        let selected_style = theme.get("ui.selection");
        for (offset, line) in lines[start..end].iter().take(height).enumerate() {
            let line_number = start + offset + 1;
            let is_current = start + offset == current;
            let text = format!(
                "{:>width$} {}",
                line_number,
                line.trim_end_matches('\r'),
                width = number_width
            );
            surface.set_stringn(
                area.x,
                area.y + 1 + offset as u16,
                &text,
                area.width as usize,
                if is_current {
                    selected_style
                } else {
                    text_style
                },
            );
        }
    }

    fn drain(&mut self) {
        let Some(receiver) = &self.receiver else {
            return;
        };
        let mut drained = 0usize;
        for _ in 0..500 {
            match receiver.try_recv() {
                Ok(SearchEvent::Match(matched)) => {
                    drained += 1;
                    self.results
                        .entry(matched.path.clone())
                        .or_default()
                        .matches
                        .push(matched);
                }
                Ok(SearchEvent::Done { capped }) => {
                    self.capped = capped;
                    self.receiver = None;
                    break;
                }
                Ok(SearchEvent::Error(err)) => {
                    drained += 1;
                    self.error = Some(err);
                }
                Ok(SearchEvent::Progress) => {
                    drained += 1;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.receiver = None;
                    break;
                }
            }
        }
        if drained == 500 && self.receiver.is_some() {
            helix_event::request_redraw();
        }
    }

    fn rebuild_rows(&mut self) {
        let mut rows = Vec::new();
        for (path, file) in &self.results {
            rows.push(ResultRow::File(path.clone()));
            if !file.collapsed {
                for index in 0..file.matches.len() {
                    rows.push(ResultRow::Match(path.clone(), index));
                }
            }
        }
        self.rows = rows;
        self.cursor = self.cursor.min(self.rows.len().saturating_sub(1));
    }

    fn selected_match_at(&self, index: usize) -> Option<&Match> {
        match self.rows.get(index)? {
            ResultRow::File(path) => self.results.get(path)?.matches.first(),
            ResultRow::Match(path, index) => self.results.get(path)?.matches.get(*index),
        }
    }

    fn selected_match(&self) -> Option<Match> {
        self.selected_match_at(self.cursor).cloned()
    }

    fn selected_file(&self) -> Option<PathBuf> {
        match self.rows.get(self.cursor)? {
            ResultRow::File(path) => Some(path.clone()),
            ResultRow::Match(path, _) => Some(path.clone()),
        }
    }

    fn move_cursor_by(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let max = self.rows.len().saturating_sub(1) as isize;
        self.cursor = (self.cursor as isize + delta).clamp(0, max) as usize;
    }

    pub fn open_match(
        &mut self,
        editor: &mut Editor,
        file: &Path,
        start: usize,
        end: usize,
    ) -> anyhow::Result<()> {
        let matched = Match {
            path: file.to_path_buf(),
            line: 0,
            col: 0,
            start,
            end,
            text: String::new(),
            preview: String::new(),
        };
        open_match_with_action(editor, &matched, helix_view::editor::Action::Replace)
    }

    pub fn select_all_in_file(&mut self, editor: &mut Editor) -> anyhow::Result<()> {
        let Some(path) = self.selected_file() else {
            return Ok(());
        };
        let matches = self
            .results
            .get(&path)
            .map(|file| file.matches.clone())
            .unwrap_or_default();
        editor.open(&path, helix_view::editor::Action::Replace)?;
        let scrolloff = editor.config().scrolloff;
        let (view, doc) = helix_view::current!(editor);
        let len = doc.text().len_bytes();
        let ranges: helix_core::SmallVec<[helix_core::Range; 1]> = matches
            .iter()
            .filter(|matched| matched.start <= matched.end && matched.end <= len)
            .map(|matched| {
                helix_core::Range::new(
                    doc.text().byte_to_char(matched.start),
                    doc.text().byte_to_char(matched.end),
                )
            })
            .collect();
        if ranges.is_empty() {
            return Ok(());
        }
        doc.set_selection(view.id, helix_core::Selection::new(ranges, 0));
        view.ensure_cursor_in_view_center(doc, scrolloff);
        editor.sidebar.focus = helix_view::sidebar::SidebarFocus::Editor;
        Ok(())
    }

    fn move_result(
        &mut self,
        forward: bool,
        action: helix_view::editor::Action,
        cx: &mut commands::Context,
    ) {
        if self.rows.is_empty() {
            return;
        }
        let mut index = self.cursor;
        loop {
            if forward {
                if index + 1 >= self.rows.len() {
                    index = 0;
                    break;
                }
                index += 1;
            } else {
                if index == 0 {
                    index = self.rows.len() - 1;
                    break;
                }
                index -= 1;
            }
            if matches!(&self.rows[index], ResultRow::Match(_, _)) {
                break;
            }
        }
        self.cursor = index;
        if let Some(matched) = self.selected_match() {
            if let Err(err) = open_match_with_action(cx.editor, &matched, action) {
                cx.editor.set_error(err.to_string());
            }
        }
    }

    fn replace_options(editor: &Editor) -> SearchOptions {
        let inputs = &editor.sidebar.search;
        SearchOptions {
            match_case: inputs.match_case,
            whole_word: inputs.whole_word,
            regex: inputs.regex,
            preserve_case: inputs.preserve_case,
        }
    }

    fn replace_current_match(&mut self, cx: &mut commands::Context) -> anyhow::Result<usize> {
        let Some(matched) = self.selected_match() else {
            return Ok(0);
        };
        let options = Self::replace_options(cx.editor);
        let inputs = cx.editor.sidebar.search.clone();
        let regex = search::build_regex(&inputs.query, options)?;
        cx.editor
            .open(&matched.path, helix_view::editor::Action::Replace)?;
        let view_id = helix_view::view!(cx.editor).id;
        let doc_id = helix_view::current_ref!(cx.editor).1.id();
        let doc = helix_view::doc_mut!(cx.editor, &doc_id);
        let text = doc.text().to_string();

        // The result list is a snapshot. Revalidate the exact range and, if it
        // no longer matches, fall back to the first current match. Never write
        // a replacement over text that no longer matches the query.
        let (start, end, caps) = if matched.start <= matched.end
            && matched.end <= text.len()
            && regex
                .find(&text[matched.start..matched.end])
                .is_some_and(|found| {
                    found.start() == 0 && found.end() == matched.end - matched.start
                }) {
            (
                matched.start,
                matched.end,
                regex.captures(&text[matched.start..matched.end]),
            )
        } else if let Some(caps) = regex.captures(&text) {
            let found = caps.get(0).unwrap();
            (found.start(), found.end(), Some(caps))
        } else {
            return Ok(0);
        };

        let slice = &text[start..end];
        let mut replacement =
            search::replacement_for(caps.as_ref(), slice, &inputs.replacement, options);
        if doc.line_ending == helix_core::LineEnding::Crlf {
            replacement = replacement.replace("\r\n", "\n").replace('\n', "\r\n");
        }
        let from = doc.text().byte_to_char(start);
        let to = doc.text().byte_to_char(end);
        let transaction = helix_core::Transaction::change(
            doc.text(),
            std::iter::once((from, to, Some(replacement.into()))),
        );
        doc.apply(&transaction, view_id);
        let view = helix_view::view_mut!(cx.editor, view_id);
        doc.append_changes_to_history(view);
        cx.editor.sidebar.focus = helix_view::sidebar::SidebarFocus::Editor;
        Ok(1)
    }

    fn replace_file(&mut self, cx: &mut commands::Context, path: &Path) -> anyhow::Result<usize> {
        let options = Self::replace_options(cx.editor);
        let inputs = cx.editor.sidebar.search.clone();
        let regex = search::build_regex(&inputs.query, options)?;
        if let Some(doc_id) = cx.editor.document_id_by_path(path) {
            search::replace_in_open_document(
                cx.editor,
                doc_id,
                &regex,
                &inputs.replacement,
                options,
            )
        } else {
            search::replace_in_file(path, &regex, &inputs.replacement, options)
        }
    }

    fn replace_selected_item(&mut self, cx: &mut commands::Context) -> anyhow::Result<usize> {
        if matches!(&self.rows.get(self.cursor), Some(ResultRow::File(_))) {
            self.selected_file()
                .map(|path| self.replace_file(cx, &path))
                .unwrap_or(Ok(0))
        } else {
            self.replace_current_match(cx)
        }
    }

    fn build_replace_preview(
        &mut self,
        cx: &mut commands::Context,
        scope: ReplaceScope,
    ) -> Option<ReplacePreview> {
        let options = Self::replace_options(cx.editor);
        let inputs = cx.editor.sidebar.search.clone();
        let regex = match search::build_regex(&inputs.query, options) {
            Ok(regex) => regex,
            Err(err) => {
                cx.editor.set_error(err.to_string());
                return None;
            }
        };

        let selected_file = match self.rows.get(self.cursor) {
            Some(ResultRow::File(path)) => Some(path.clone()),
            _ => None,
        };
        let selected_match = self.selected_match();
        let mut body = Vec::new();
        let mut matches = 0usize;
        let mut files = 0usize;

        'files: for (path, file) in &self.results {
            if scope == ReplaceScope::Item {
                let include_file = selected_file.as_ref() == Some(path);
                if !include_file
                    && selected_match.as_ref().is_none_or(|matched| {
                        &matched.path != path
                            || !file
                                .matches
                                .iter()
                                .any(|candidate| candidate.start == matched.start)
                    })
                {
                    continue;
                }
            }

            let mut included_any = false;
            for matched in &file.matches {
                if scope == ReplaceScope::Item
                    && selected_file.as_ref() != Some(path)
                    && selected_match.as_ref().is_none_or(|selected| {
                        selected.path != matched.path || selected.start != matched.start
                    })
                {
                    continue;
                }

                let replacement = regex
                    .captures(&matched.text)
                    .map(|caps| {
                        search::replacement_for(
                            Some(&caps),
                            &matched.text,
                            &inputs.replacement,
                            options,
                        )
                    })
                    .unwrap_or_else(|| {
                        search::replacement_for(None, &matched.text, &inputs.replacement, options)
                    });
                let relative = path.strip_prefix(&cx.editor.workspace_root).unwrap_or(path);
                let line = if matched.preview.is_empty() {
                    matched.text.as_str()
                } else {
                    matched.preview.as_str()
                };
                let (old_context, new_context) =
                    replace_context(line, matched.col, matched.text.len(), &replacement);
                body.push(format!(
                    "- {}:{}  {}",
                    relative.display(),
                    matched.line + 1,
                    old_context
                ));
                body.push(format!(
                    "+ {}:{}  {}",
                    relative.display(),
                    matched.line + 1,
                    new_context
                ));
                matches += 1;
                included_any = true;
                if body.len() >= 600 {
                    body.push("… preview truncated".to_string());
                    break 'files;
                }
            }
            if included_any {
                files += 1;
            }
        }

        if matches == 0 {
            cx.editor.set_status("No replacements to preview");
            return None;
        }

        let mut lines = vec![
            format!(
                "Replace {} preview",
                if scope == ReplaceScope::All {
                    "all"
                } else {
                    "selected item"
                }
            ),
            format!("{matches} match(es) in {files} file(s)"),
            String::new(),
        ];
        lines.extend(body);
        Some(ReplacePreview {
            scope,
            lines,
            scroll: 0,
        })
    }

    fn apply_replace_scope(
        &mut self,
        cx: &mut commands::Context,
        scope: ReplaceScope,
    ) -> anyhow::Result<()> {
        match scope {
            ReplaceScope::All => self.replace_all(cx),
            ReplaceScope::Item => {
                let count = self.replace_selected_item(cx)?;
                self.pending = true;
                cx.editor
                    .set_status(format!("replaced {count} occurrence(s)"));
                Ok(())
            }
        }
    }

    fn replace_all(&mut self, cx: &mut commands::Context) -> anyhow::Result<()> {
        let paths: Vec<PathBuf> = self.results.keys().cloned().collect();
        let mut replaced = 0;
        let mut files = 0;
        let mut skipped = 0;
        for path in paths {
            match self.replace_file(cx, &path) {
                Ok(0) => skipped += 1,
                Ok(count) => {
                    replaced += count;
                    files += 1;
                }
                Err(_) => skipped += 1,
            }
        }
        self.pending = true;
        cx.editor.set_status(format!(
            "Replaced {replaced} occurrences in {files} files ({skipped} skipped)"
        ));
        cx.editor.needs_redraw = true;
        Ok(())
    }

    fn open_options(&mut self, cx: &mut commands::Context) {
        self.options_mode = true;
        let body = [
            ("", "── Options ──"),
            ("c", "Match case"),
            ("w", "Whole word"),
            ("r", "Regex"),
            ("p", "Preserve case"),
            ("", "── Search ──"),
            ("/", "Edit Find (without clearing)"),
            ("Enter (editing)", "Run search"),
            ("Tab (editing)", "Switch Find ⇄ Replace"),
            ("", "── Results ──"),
            ("Enter", "Open selected item"),
            ("r", "Replace selected item"),
            ("R", "Replace all"),
            ("Alt-Enter", "Select all matches in file"),
            ("", "── Pane ──"),
            ("b", "Open file tree"),
            ("Space, Esc", "Close options"),
        ];
        cx.editor.autoinfo = Some(Info::new("Search options (Space)", &body));
    }

    fn close_options(&mut self, editor: &mut Editor) {
        if self.options_mode {
            self.options_mode = false;
            editor.autoinfo = None;
        }
    }

    fn toggle_option(&mut self, key: char, cx: &mut commands::Context) {
        let inputs = &mut cx.editor.sidebar.search;
        match key {
            'c' => inputs.match_case = !inputs.match_case,
            'w' => inputs.whole_word = !inputs.whole_word,
            'r' => inputs.regex = !inputs.regex,
            'p' => inputs.preserve_case = !inputs.preserve_case,
            _ => return,
        }
        if key != 'p' {
            self.pending = true;
        }
        cx.editor.needs_redraw = true;
    }

    pub fn handle_event(&mut self, event: &Event, cx: &mut commands::Context) -> EventResult {
        if let Event::Paste(contents) = event {
            self.close_options(cx.editor);
            match self.focus {
                SearchFocus::Replace if self.editing => self.replacement.insert_str(contents),
                _ => {
                    self.focus = SearchFocus::Query;
                    self.query.insert_str(contents);
                }
            }
            self.editing = true;
            self.sync_inputs(cx.editor);
            cx.editor.needs_redraw = true;
            return EventResult::Consumed(None);
        }

        let Event::Key(key) = event else {
            return EventResult::Consumed(None);
        };

        if self.replace_preview.is_some() {
            match key.code {
                KeyCode::Enter => {
                    let scope = self
                        .replace_preview
                        .as_ref()
                        .map(|preview| preview.scope)
                        .unwrap();
                    self.replace_preview = None;
                    if let Err(err) = self.apply_replace_scope(cx, scope) {
                        cx.editor.set_error(err.to_string());
                    }
                }
                KeyCode::Esc => {
                    self.replace_preview = None;
                }
                KeyCode::Char('j') | KeyCode::Down => {
                    if let Some(preview) = self.replace_preview.as_mut() {
                        preview.scroll = preview.scroll.saturating_add(1);
                    }
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    if let Some(preview) = self.replace_preview.as_mut() {
                        preview.scroll = preview.scroll.saturating_sub(1);
                    }
                }
                _ => {}
            }
            cx.editor.needs_redraw = true;
            return EventResult::Consumed(None);
        }

        // Cmd/Alt + Backspace/Delete clears the active search field. This must
        // run before the generic Alt chord handling below, which would
        // otherwise consume the key without forwarding it to the text input.
        if key
            .modifiers
            .intersects(KeyModifiers::SUPER | KeyModifiers::ALT)
            && matches!(key.code, KeyCode::Backspace | KeyCode::Delete)
        {
            self.close_options(cx.editor);
            match self.focus {
                SearchFocus::Replace if self.editing => self.replacement.set_value(""),
                _ => {
                    self.focus = SearchFocus::Query;
                    self.query.set_value("");
                }
            }
            self.editing = true;
            self.sync_inputs(cx.editor);
            cx.editor.needs_redraw = true;
            return EventResult::Consumed(None);
        }

        if self.options_mode {
            match key.code {
                KeyCode::Char('c' | 'w' | 'r' | 'p') => {
                    if let KeyCode::Char(option) = key.code {
                        self.toggle_option(option, cx);
                    }
                }
                KeyCode::Char('/') => {
                    self.begin_edit(cx, SearchFocus::Query);
                }
                KeyCode::Char('b') => {
                    self.close_options(cx.editor);
                    cx.editor.sidebar.search_modal = false;
                    cx.editor.sidebar.mode = helix_view::sidebar::SidebarMode::Files;
                    cx.editor.sidebar.focus = helix_view::sidebar::SidebarFocus::Sidebar;
                    cx.editor.needs_redraw = true;
                }
                KeyCode::Char(' ') | KeyCode::Esc | KeyCode::Tab => {
                    self.close_options(cx.editor);
                    cx.editor.needs_redraw = true;
                }
                _ => {}
            }
            return EventResult::Consumed(None);
        }

        if !self.editing
            && key.code == KeyCode::Char('/')
            && !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            self.begin_edit(cx, SearchFocus::Query);
            return EventResult::Consumed(None);
        }

        if key.code == KeyCode::Char(' ')
            && self.focus == SearchFocus::Results
            && !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            self.open_options(cx);
            cx.editor.needs_redraw = true;
            return EventResult::Consumed(None);
        }
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Up | KeyCode::Down)
        {
            let lines = cx.editor.config().scroll_lines.unsigned_abs().max(1) as isize;
            let delta = if key.code == KeyCode::Up {
                -lines
            } else {
                lines
            };
            self.move_cursor_by(delta);
            cx.editor.needs_redraw = true;
            return EventResult::Consumed(None);
        }

        let alt_moves_field = self.editing
            && matches!(
                key.code,
                KeyCode::Left | KeyCode::Right | KeyCode::Char('b' | 'f' | 'd')
            );
        if key.modifiers.contains(KeyModifiers::ALT) && !alt_moves_field {
            if key.code == KeyCode::Char(' ') {
                self.open_options(cx);
                cx.editor.needs_redraw = true;
                return EventResult::Consumed(None);
            }
            if key.code == KeyCode::Enter {
                let result = if key.modifiers.contains(KeyModifiers::CONTROL) {
                    self.replace_all(cx)
                } else {
                    self.select_all_in_file(cx.editor)
                };
                if let Err(err) = result {
                    cx.editor.set_error(err.to_string());
                }
                cx.editor.needs_redraw = true;
                return EventResult::Consumed(None);
            }
            match key.code {
                KeyCode::Char(option @ ('c' | 'w' | 'r' | 'p')) => {
                    self.toggle_option(option, cx);
                }
                _ => return EventResult::Consumed(None),
            }
            cx.editor.needs_redraw = true;
            return EventResult::Consumed(None);
        }

        if self.editing {
            match key.code {
                KeyCode::Tab => {
                    self.focus = if self.focus == SearchFocus::Query {
                        SearchFocus::Replace
                    } else {
                        SearchFocus::Query
                    };
                }
                KeyCode::Enter => {
                    self.finish_edit(cx, true);
                    return EventResult::Consumed(None);
                }
                KeyCode::Esc => {
                    self.finish_edit(cx, false);
                    return EventResult::Consumed(None);
                }
                _ => {
                    let handled = if self.focus == SearchFocus::Replace {
                        self.replacement.handle_key(*key)
                    } else {
                        self.query.handle_key(*key)
                    };
                    if handled {
                        self.sync_inputs(cx.editor);
                    } else if key.modifiers.contains(KeyModifiers::CONTROL)
                        || key.modifiers.contains(KeyModifiers::ALT)
                    {
                        return EventResult::Ignored(None);
                    }
                }
            }
            cx.editor.needs_redraw = true;
            return EventResult::Consumed(None);
        }

        match self.focus {
            // Results is the normal mode. Query/Replace are only focusable
            // while `editing` is true; fall back to results if needed.
            SearchFocus::Query | SearchFocus::Replace => {
                self.focus = SearchFocus::Results;
                return EventResult::Consumed(None);
            }
            SearchFocus::Results => match key.code {
                KeyCode::Char('r') => {
                    if let Some(preview) = self.build_replace_preview(cx, ReplaceScope::Item) {
                        self.replace_preview = Some(preview);
                    }
                }
                KeyCode::Char('R') => {
                    if let Some(preview) = self.build_replace_preview(cx, ReplaceScope::All) {
                        self.replace_preview = Some(preview);
                    }
                }
                KeyCode::Char('j') | KeyCode::Down => {
                    if !self.rows.is_empty() {
                        self.cursor = (self.cursor + 1).min(self.rows.len() - 1);
                    }
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.cursor = self.cursor.saturating_sub(1);
                }
                KeyCode::Char('h') | KeyCode::Left => self.collapse_selected(),
                KeyCode::Char('l') | KeyCode::Right => self.expand_selected(),
                KeyCode::Enter => {
                    if let Some(matched) = self.selected_match() {
                        if let Err(err) = open_match_with_action(
                            cx.editor,
                            &matched,
                            helix_view::editor::Action::Replace,
                        ) {
                            cx.editor.set_error(err.to_string());
                        }
                    }
                }
                KeyCode::Char('s') => {
                    if let Some(matched) = self.selected_match() {
                        if let Err(err) = open_match_with_action(
                            cx.editor,
                            &matched,
                            helix_view::editor::Action::HorizontalSplit,
                        ) {
                            cx.editor.set_error(err.to_string());
                        }
                    }
                }
                KeyCode::Char('v') => {
                    if let Some(matched) = self.selected_match() {
                        if let Err(err) = open_match_with_action(
                            cx.editor,
                            &matched,
                            helix_view::editor::Action::VerticalSplit,
                        ) {
                            cx.editor.set_error(err.to_string());
                        }
                    }
                }
                KeyCode::F(4) if key.modifiers.contains(KeyModifiers::SHIFT) => {
                    self.move_result(false, helix_view::editor::Action::Replace, cx)
                }
                KeyCode::Char('n') | KeyCode::F(4) => {
                    self.move_result(true, helix_view::editor::Action::Replace, cx)
                }
                KeyCode::Char('N') => {
                    self.move_result(false, helix_view::editor::Action::Replace, cx)
                }
                KeyCode::Esc | KeyCode::Char('q') => {
                    cx.editor.sidebar.focus = helix_view::sidebar::SidebarFocus::Editor;
                }
                _ if key.modifiers.contains(KeyModifiers::CONTROL)
                    || key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    return EventResult::Ignored(None);
                }
                _ => return EventResult::Consumed(None),
            },
        }

        cx.editor.needs_redraw = true;
        EventResult::Consumed(None)
    }

    fn collapse_selected(&mut self) {
        if let Some(ResultRow::File(path)) = self.rows.get(self.cursor).cloned() {
            if let Some(file) = self.results.get_mut(&path) {
                file.collapsed = true;
            }
            self.rebuild_rows();
        }
    }

    fn expand_selected(&mut self) {
        if let Some(ResultRow::File(path)) = self.rows.get(self.cursor).cloned() {
            if let Some(file) = self.results.get_mut(&path) {
                file.collapsed = false;
            }
            self.rebuild_rows();
        }
    }
}

/// Show the match inside its surrounding line. Very long lines are clipped
/// around the match so the changed text stays visible in the modal.
fn replace_context(
    line: &str,
    column: usize,
    match_len: usize,
    replacement: &str,
) -> (String, String) {
    if line.is_empty() {
        return (
            String::new(),
            replacement.replace('\r', "").replace('\n', "⏎"),
        );
    }

    let mut start = column.min(line.len());
    while start > 0 && !line.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = (start + match_len).min(line.len());
    while end < line.len() && !line.is_char_boundary(end) {
        end += 1;
    }

    let mut clip_start = start.saturating_sub(50);
    while clip_start > 0 && !line.is_char_boundary(clip_start) {
        clip_start -= 1;
    }
    let mut clip_end = (end + 50).min(line.len());
    while clip_end < line.len() && !line.is_char_boundary(clip_end) {
        clip_end += 1;
    }

    let mut old = String::new();
    if clip_start > 0 {
        old.push('…');
    }
    old.push_str(&line[clip_start..clip_end]);
    if clip_end < line.len() {
        old.push('…');
    }

    let replacement = replacement.replace('\r', "").replace('\n', "⏎");
    let mut new = String::new();
    if clip_start > 0 {
        new.push('…');
    }
    new.push_str(&line[clip_start..start]);
    new.push_str(&replacement);
    new.push_str(&line[end..clip_end]);
    if clip_end < line.len() {
        new.push('…');
    }

    (old, new)
}

fn open_match_with_action(
    editor: &mut Editor,
    matched: &Match,
    action: helix_view::editor::Action,
) -> anyhow::Result<()> {
    editor.open(&matched.path, action)?;
    let scrolloff = editor.config().scrolloff;
    let (view, doc) = helix_view::current!(editor);
    let len = doc.text().len_bytes();
    if matched.start > matched.end || matched.end > len {
        anyhow::bail!("search result is stale for {}", matched.path.display());
    }
    let start = doc.text().byte_to_char(matched.start);
    let end = doc.text().byte_to_char(matched.end);
    doc.set_selection(view.id, helix_core::Selection::single(start, end));
    view.ensure_cursor_in_view_center(doc, scrolloff);
    editor.sidebar.focus = helix_view::sidebar::SidebarFocus::Editor;
    editor.sidebar.search_modal = false;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::replace_context;

    #[test]
    fn replace_preview_keeps_surrounding_text() {
        let (old, new) = replace_context("before test after", 7, 4, "aaa");
        assert_eq!(old, "before test after");
        assert_eq!(new, "before aaa after");
    }

    #[test]
    fn replace_preview_clips_long_lines() {
        let line = format!("{}test{}", "a".repeat(80), "b".repeat(80));
        let (old, new) = replace_context(&line, 80, 4, "x");
        assert!(old.starts_with('…'));
        assert!(old.ends_with('…'));
        assert!(new.starts_with('…'));
        assert!(new.ends_with('…'));
        assert!(new.contains("x"));
    }
}
