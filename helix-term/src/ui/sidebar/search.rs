use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};

use helix_view::graphics::Rect;
use helix_view::input::{KeyCode, KeyModifiers};
use helix_view::sidebar::SearchInputs;
use helix_view::Editor;
use tui::buffer::Buffer as Surface;

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

    pub fn focus_query(&mut self) {
        self.focus = SearchFocus::Query;
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
            .filter_map(|doc| doc.path().map(|path| (path.to_path_buf(), doc.text().to_string())))
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

    pub fn render(&mut self, area: Rect, surface: &mut Surface, editor: &Editor) {
        self.drain();
        self.rebuild_rows();

        let theme = &editor.theme;
        let text = theme.get("ui.text");
        let selected = theme.get("ui.menu.selected");
        let dim = theme.get("ui.linenr");
        let width = area.width.saturating_sub(2);
        let mut y = area.y;

        let query_area = Rect::new(area.x + 1, y, width, 1);
        self.query
            .render(query_area, surface, theme, self.focus == SearchFocus::Query);
        y += 1;

        if y < area.y + area.height {
            let mut toggles = String::new();
            toggles.push_str(if editor.sidebar.search.match_case { "[Aa] " } else { " Aa  " });
            toggles.push_str(if editor.sidebar.search.whole_word { "[ab] " } else { " ab  " });
            toggles.push_str(if editor.sidebar.search.regex { "[.*]" } else { " .* " });
            toggles.push_str(if editor.sidebar.search.preserve_case { " [AB]" } else { "  AB " });
            surface.set_string(area.x + 1, y, &toggles, dim);
            y += 1;
        }

        if y < area.y + area.height {
            let replace_area = Rect::new(area.x + 1, y, width, 1);
            self.replacement
                .render(replace_area, surface, theme, self.focus == SearchFocus::Replace);
            y += 1;
        }

        if y < area.y + area.height {
            surface.set_string(area.x, y, &"─".repeat(width as usize), dim);
            y += 1;
        }

        if let Some(error) = &self.error {
            if y < area.y + area.height {
                surface.set_string(area.x + 1, y, error, theme.get("error"));
            }
            return;
        }

        let height = (area.y + area.height).saturating_sub(y) as usize;
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
            surface.set_string(area.x, y + offset as u16, &line, style);
        }

        if self.capped && y + height as u16 > y {
            surface.set_string(area.x, y, "showing first 10000 matches", dim);
        }
    }

    fn drain(&mut self) {
        let Some(receiver) = &self.receiver else {
            return;
        };
        for _ in 0..500 {
            match receiver.try_recv() {
                Ok(SearchEvent::Match(matched)) => {
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
                    self.error = Some(err);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.receiver = None;
                    break;
                }
            }
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

    pub fn handle_event(
        &mut self,
        event: &Event,
        cx: &mut commands::Context,
    ) -> EventResult {
        let Event::Key(key) = event else {
            return EventResult::Consumed(None);
        };
        if key.modifiers.contains(KeyModifiers::ALT) {
            match key.code {
                KeyCode::Char('c') => {
                    cx.editor.sidebar.search.match_case = !cx.editor.sidebar.search.match_case;
                    self.pending = true;
                }
                KeyCode::Char('w') => {
                    cx.editor.sidebar.search.whole_word = !cx.editor.sidebar.search.whole_word;
                    self.pending = true;
                }
                KeyCode::Char('r') => {
                    cx.editor.sidebar.search.regex = !cx.editor.sidebar.search.regex;
                    self.pending = true;
                }
                KeyCode::Char('p') => {
                    cx.editor.sidebar.search.preserve_case =
                        !cx.editor.sidebar.search.preserve_case;
                }
                _ => return EventResult::Consumed(None),
            }
            cx.editor.needs_redraw = true;
            return EventResult::Consumed(None);
        }

        match self.focus {
            SearchFocus::Query | SearchFocus::Replace => {
                match key.code {
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
                        }
                    }
                }
            }
            SearchFocus::Results => match key.code {
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
                        if let Err(err) = open_match(cx, &matched) {
                            cx.editor.set_error(err.to_string());
                        }
                    }
                }
                KeyCode::Esc | KeyCode::Char('q') => {
                    cx.editor.sidebar.focus = helix_view::sidebar::SidebarFocus::Editor;
                }
                KeyCode::Tab => {
                    self.focus = SearchFocus::Query;
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

fn open_match(cx: &mut commands::Context, matched: &Match) -> anyhow::Result<()> {
    cx.editor
        .open(&matched.path, helix_view::editor::Action::Replace)?;
    let scrolloff = cx.editor.config().scrolloff;
    let (view, doc) = helix_view::current!(cx.editor);
    let start = doc.text().byte_to_char(matched.start);
    let end = doc.text().byte_to_char(matched.end);
    doc.set_selection(view.id, helix_core::Selection::single(start, end));
    view.ensure_cursor_in_view_center(doc, scrolloff);
    cx.editor.sidebar.focus = helix_view::sidebar::SidebarFocus::Editor;
    Ok(())
}
