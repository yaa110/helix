use helix_view::graphics::Rect;
use helix_view::input::KeyCode;
use helix_view::sidebar::{SidebarFocus, SidebarMode};
use helix_view::Editor;
use tui::buffer::Buffer as Surface;

use crate::commands;
use crate::compositor::{Context, Event, EventResult};

/// The persistent left pane. Tasks 12 and 19 add the file tree and search
/// panes; this component owns the shared layout, header, and key routing.
pub struct Sidebar {
    pending_space: bool,
}

impl Sidebar {
    pub fn new() -> Self {
        Self {
            pending_space: false,
        }
    }

    pub fn ensure_init(&mut self, _editor: &mut Editor) {}

    pub fn render(&mut self, area: Rect, surface: &mut Surface, cx: &mut Context) {
        let theme = &cx.editor.theme;
        let background = theme
            .try_get_exact("ui.sidebar")
            .or_else(|| theme.try_get_exact("ui.background"))
            .unwrap_or_default();
        let border = theme
            .try_get_exact("ui.sidebar.border")
            .or_else(|| theme.try_get_exact("ui.linenr"))
            .unwrap_or_default();
        let title = theme
            .try_get_exact("ui.sidebar.title")
            .or_else(|| theme.try_get_exact("ui.text"))
            .unwrap_or_default();

        surface.clear_with(area, background);
        let title_text = match cx.editor.sidebar.mode {
            SidebarMode::Files => "FILES",
            SidebarMode::Search => "SEARCH",
            SidebarMode::Hidden => return,
        };
        surface.set_string(area.x + 1, area.y, title_text, title);

        if area.width > 0 {
            for y in area.y..area.y.saturating_add(area.height) {
                surface.set_string(area.x + area.width - 1, y, "│", border);
            }
        }
    }

    pub fn handle_event(
        &mut self,
        event: &Event,
        cx: &mut commands::Context,
    ) -> EventResult {
        let Event::Key(key) = event else {
            return EventResult::Consumed(None);
        };

        if self.pending_space {
            self.pending_space = false;
            match key.code {
                KeyCode::Char('b') => {
                    cx.editor.sidebar.mode = SidebarMode::Hidden;
                    cx.editor.sidebar.focus = SidebarFocus::Editor;
                }
                KeyCode::Char('/') => {
                    cx.editor.sidebar.mode = SidebarMode::Search;
                    cx.editor.sidebar.focus = SidebarFocus::Sidebar;
                }
                _ => {}
            }
            cx.editor.needs_redraw = true;
            return EventResult::Consumed(None);
        }

        match key.code {
            KeyCode::Char(' ') => {
                self.pending_space = true;
                EventResult::Consumed(None)
            }
            KeyCode::Esc | KeyCode::Char('q') => {
                cx.editor.sidebar.focus = SidebarFocus::Editor;
                cx.editor.needs_redraw = true;
                EventResult::Consumed(None)
            }
            _ => {
                // The placeholder consumes navigation keys so they never leak
                // into the editor; Task 12 handles them for real.
                cx.editor.needs_redraw = true;
                EventResult::Consumed(None)
            }
        }
    }
}

impl Default for Sidebar {
    fn default() -> Self {
        Self::new()
    }
}
