use std::path::PathBuf;

use helix_view::graphics::Rect;
use helix_view::input::KeyCode;
use helix_view::sidebar::{SidebarFocus, SidebarMode};
use helix_view::Editor;
use tui::buffer::Buffer as Surface;

use crate::commands;
use crate::compositor::{Context, Event, EventResult};

pub mod file_tree;
pub mod search;

use file_tree::FileTree;
use search::{SearchFocus, SearchPane};

/// The persistent left pane. Tasks 12 and 19 add the file tree and search
/// panes; this component owns the shared layout, header, and key routing.
pub struct Sidebar {
    tree: FileTree,
    search: SearchPane,
    last_refresh_epoch: u64,
    last_mode: SidebarMode,
}

impl Sidebar {
    pub fn new() -> Self {
        Self {
            tree: FileTree::new(PathBuf::new()),
            search: SearchPane::new(&helix_view::sidebar::SearchInputs::default()),
            last_refresh_epoch: 0,
            last_mode: SidebarMode::Hidden,
        }
    }

    pub fn row_paths(&self) -> Vec<PathBuf> {
        self.tree.rows.iter().map(|row| row.path.clone()).collect()
    }

    pub fn search_result_count(&self) -> usize {
        self.search.result_count()
    }

    pub fn handle_idle_timeout(&mut self, cx: &mut commands::Context) {
        if cx.editor.sidebar.mode == SidebarMode::Search || cx.editor.sidebar.search_modal {
            self.search.handle_idle_timeout(cx.editor);
            cx.editor.needs_redraw = true;
        }
    }

    pub fn handle_search_modal(
        &mut self,
        event: &Event,
        cx: &mut commands::Context,
    ) -> EventResult {
        self.search.handle_event(event, cx)
    }

    pub fn render_search_modal(&mut self, area: Rect, surface: &mut Surface, cx: &mut Context) {
        self.search.render_modal(area, surface, cx.editor);
    }

    pub fn ensure_init(&mut self, editor: &mut Editor) {
        if self.tree.root != editor.workspace_root {
            self.tree = FileTree::new(editor.workspace_root.clone());
            self.tree.rebuild(editor, &editor.sidebar);
            self.last_refresh_epoch = editor.sidebar.refresh_epoch;
        } else if self.last_refresh_epoch != editor.sidebar.refresh_epoch {
            let state = editor.sidebar.clone();
            self.tree.refresh(editor, &state);
            self.last_refresh_epoch = editor.sidebar.refresh_epoch;
        }

        if self.tree.filter != editor.sidebar.tree_filter {
            self.tree
                .set_filter(editor.sidebar.tree_filter.clone(), editor);
        }

        if let Some(selected) = editor.sidebar.selected.clone() {
            self.tree.select_path(&selected);
        }

        // Reveal the current buffer only when the focused buffer changed (new
        // file opened, next/previous buffer, session restore). Manual tree
        // navigation must survive losing focus, so this is event-driven rather
        // than running on every render.
        if editor.sidebar.reveal_current {
            editor.sidebar.reveal_current = false;
            let current = {
                let (_view, doc) = helix_view::current_ref!(editor);
                doc.path().map(ToOwned::to_owned)
            };
            if let Some(path) = current {
                let mut expanded = editor.sidebar.expanded.clone();
                if self.tree.reveal(editor, &mut expanded, &path) {
                    editor.sidebar.expanded = expanded;
                    editor.sidebar.selected = Some(path);
                }
            }
        }

        if editor.sidebar.search_focus_query {
            self.search.sync_from_inputs(&editor.sidebar.search);
            self.search.focus_query(editor);
            editor.sidebar.search_focus_query = false;
        }

        if self.last_mode != editor.sidebar.mode {
            if editor.sidebar.mode == SidebarMode::Search {
                self.search.sync_from_inputs(&editor.sidebar.search);
                self.search.focus_query(editor);
            }
            self.last_mode = editor.sidebar.mode;
        }
    }

    pub fn render(&mut self, area: Rect, surface: &mut Surface, cx: &mut Context) {
        let theme = &cx.editor.theme;
        let background = theme
            .try_get_exact("ui.sidebar")
            .or_else(|| theme.try_get_exact("ui.background"))
            .unwrap_or_default();
        surface.clear_with(area, background);
        let focused = cx.editor.sidebar.focus == SidebarFocus::Sidebar;
        let border = theme
            .try_get_exact("ui.sidebar.border")
            .or_else(|| theme.try_get_exact("ui.linenr"))
            .unwrap_or_default();
        match cx.editor.sidebar.mode {
            SidebarMode::Files => {
                self.tree.render(area, surface, cx.editor, focused);
                if focused && cx.editor.status_msg.is_none() {
                    if let Some(hint) = self.tree.overflow_hint.clone() {
                        // A terminal has no native tooltip; the statusline is
                        // the closest equivalent and remains visible while the
                        // selected tree row is horizontally clipped.
                        cx.editor.set_status(hint);
                    }
                }
            }
            SidebarMode::Search => {
                self.search.render(area, surface, cx.editor);
            }
            SidebarMode::Hidden => return,
        }

        if area.width > 0 {
            for y in area.y..area.y.saturating_add(area.height) {
                surface.set_string(area.x + area.width - 1, y, "│", border);
            }
        }
    }

    pub fn handle_event(&mut self, event: &Event, cx: &mut commands::Context) -> EventResult {
        self.ensure_init(cx.editor);
        let Event::Key(key) = event else {
            return EventResult::Consumed(None);
        };

        if cx.editor.sidebar.mode == SidebarMode::Search
            && self.search.focus != SearchFocus::Results
        {
            return self.search.handle_event(event, cx);
        }

        if cx.editor.sidebar.mode == SidebarMode::Search {
            return self.search.handle_event(event, cx);
        }

        if cx.editor.sidebar.mode == SidebarMode::Files && self.tree.handle_key(*key, cx) {
            return EventResult::Consumed(None);
        }

        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                cx.editor.sidebar.focus = SidebarFocus::Editor;
                cx.editor.needs_redraw = true;
                EventResult::Consumed(None)
            }
            _ => {
                // Let global Ctrl/Alt chords (file picker, window commands,
                // user bindings) reach the normal keymap. Plain keys stay
                // owned by the sidebar so they cannot edit the buffer.
                if key
                    .modifiers
                    .contains(helix_view::keyboard::KeyModifiers::CONTROL)
                    || key
                        .modifiers
                        .contains(helix_view::keyboard::KeyModifiers::ALT)
                {
                    EventResult::Ignored(None)
                } else {
                    cx.editor.needs_redraw = true;
                    EventResult::Consumed(None)
                }
            }
        }
    }
}

impl Default for Sidebar {
    fn default() -> Self {
        Self::new()
    }
}
