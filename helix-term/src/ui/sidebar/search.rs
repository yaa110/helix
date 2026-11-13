use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};

use helix_view::graphics::Rect;
use helix_view::info::Info;
use helix_view::input::{KeyCode, KeyModifiers};
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

pub struct SearchPane {
    query: TextInput,
    replacement: TextInput,
    pub focus: SearchFocus,
    results: BTreeMap<PathBuf, FileResults>,
    rows: Vec<ResultRow>,
    cursor: usize,
    scroll: usize,
    receiver: Option<Receiver<SearchEvent>>,
    pending: bool,
    options_mode: bool,
    capped: bool,
    error: Option<String>,
}

impl SearchPane {
    pub fn new(inputs: &SearchInputs) -> Self {
        Self {
            query: TextInput::new(inputs.query.clone()),
            replacement: TextInput::new(inputs.replacement.clone()),
            focus: SearchFocus::Query,
            results: BTreeMap::new(),
            rows: Vec::new(),
            cursor: 0,
            scroll: 0,
            receiver: None,
            pending: false,
            options_mode: false,
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

    pub fn focus_query(&mut self, editor: &mut Editor) {
        self.focus = SearchFocus::Query;
        self.close_options(editor);
    }

    pub fn result_count(&self) -> usize {
        self.results.values().map(|file| file.matches.len()).sum()
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

        if area.width <= 2 || area.height == 0 {
            return;
        }

        let theme = &editor.theme;
        let text = theme.get("ui.text");
        let selected = theme.get("ui.menu.selected");
        let dim = theme.get("ui.linenr");
        let label = theme.get("ui.sidebar.title");
        let width = area.width.saturating_sub(2);
        let max_x = area.x + area.width - 1;
        let mut y = area.y;

        // Query input. Labels make it obvious that this is an editable field,
        // and TextInput clips/scolls long queries internally.
        let query_label = "Find: ";
        let query_label_width = query_label.len() as u16;
        surface.set_stringn(area.x + 1, y, query_label, width as usize, label);
        let query_width = width.saturating_sub(query_label_width);
        if query_width > 0 {
            self.query.render(
                Rect::new(area.x + 1 + query_label_width, y, query_width, 1),
                surface,
                theme,
                self.focus == SearchFocus::Query,
            );
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
            surface.set_stringn(
                area.x + 1,
                y,
                "Alt-c/w/r/p · Space options",
                width as usize,
                dim,
            );
            y += 1;
        }

        // Replacement input.
        let replace_label = "Replace: ";
        let replace_label_width = replace_label.len() as u16;
        if y < area.y + area.height {
            surface.set_stringn(area.x + 1, y, replace_label, width as usize, label);
            let replace_width = width.saturating_sub(replace_label_width);
            if replace_width > 0 {
                self.replacement.render(
                    Rect::new(area.x + 1 + replace_label_width, y, replace_width, 1),
                    surface,
                    theme,
                    self.focus == SearchFocus::Replace,
                );
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

        if editor.sidebar.search.query.is_empty() {
            surface.set_stringn(area.x + 1, y, "Type to search", width as usize, dim);
            return;
        }
        if self.pending || self.receiver.is_some() {
            surface.set_stringn(area.x + 1, y, "Searching…", width as usize, dim);
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
        let width = area.width.saturating_sub(4).min(120);
        let height = area.height.saturating_sub(4).min(34);
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

    fn render_preview(&self, area: Rect, surface: &mut Surface, editor: &Editor) {
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
            ("", "── Pane ──"),
            ("b", "Open file tree"),
            ("/", "Close search pane"),
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
        let Event::Key(key) = event else {
            return EventResult::Consumed(None);
        };

        if self.options_mode {
            match key.code {
                KeyCode::Char('c' | 'w' | 'r' | 'p') => {
                    if let KeyCode::Char(option) = key.code {
                        self.toggle_option(option, cx);
                    }
                }
                KeyCode::Char('/') => {
                    self.close_options(cx.editor);
                    cx.editor.sidebar.search_modal = false;
                    cx.editor.sidebar.focus = helix_view::sidebar::SidebarFocus::Editor;
                    cx.editor.needs_redraw = true;
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

        if key.modifiers.contains(KeyModifiers::ALT) {
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

        match self.focus {
            SearchFocus::Query | SearchFocus::Replace => match key.code {
                KeyCode::Tab => {
                    self.focus = if self.focus == SearchFocus::Query {
                        SearchFocus::Replace
                    } else {
                        SearchFocus::Results
                    };
                }
                KeyCode::Esc => {
                    self.focus = SearchFocus::Results;
                }
                KeyCode::Enter if self.focus == SearchFocus::Query => {
                    self.focus = SearchFocus::Results;
                }
                KeyCode::Enter if self.focus == SearchFocus::Replace => {
                    match self.replace_current_match(cx) {
                        Ok(count) => {
                            self.pending = true;
                            cx.editor
                                .set_status(format!("replaced {count} occurrence(s)"));
                        }
                        Err(err) => cx.editor.set_error(err.to_string()),
                    }
                }
                _ => {
                    let handled = if self.focus == SearchFocus::Query {
                        self.query.handle_key(*key)
                    } else {
                        self.replacement.handle_key(*key)
                    };
                    if handled {
                        let inputs = &mut cx.editor.sidebar.search;
                        inputs.query = self.query.value().to_string();
                        inputs.replacement = self.replacement.value().to_string();
                        self.pending = true;
                    } else if key.modifiers.contains(KeyModifiers::CONTROL)
                        || key.modifiers.contains(KeyModifiers::ALT)
                    {
                        return EventResult::Ignored(None);
                    }
                }
            },
            SearchFocus::Results => match key.code {
                KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    let result = if matches!(&self.rows.get(self.cursor), Some(ResultRow::File(_)))
                    {
                        self.selected_file()
                            .map(|path| self.replace_file(cx, &path))
                            .unwrap_or(Ok(0))
                    } else {
                        self.replace_current_match(cx)
                    };
                    match result {
                        Ok(count) => {
                            self.pending = true;
                            cx.editor
                                .set_status(format!("replaced {count} occurrence(s)"));
                        }
                        Err(err) => cx.editor.set_error(err.to_string()),
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
                KeyCode::Char('R') => {
                    self.pending = true;
                }
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
                KeyCode::Tab => {
                    self.focus = SearchFocus::Query;
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
