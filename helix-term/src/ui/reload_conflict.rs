use helix_view::graphics::Rect;
use helix_view::input::KeyCode;
use helix_view::DocumentId;
use tui::buffer::Buffer as Surface;
use tui::widgets::{Block, Widget};

use crate::compositor::{Component, Context, Event, EventResult};

const OPTIONS: [&str; 3] = ["Keep", "Reload", "Overwrite"];

pub struct ReloadConflictPrompt {
    doc_id: DocumentId,
    choice: usize,
}

impl ReloadConflictPrompt {
    pub fn new(doc_id: DocumentId) -> Self {
        Self { doc_id, choice: 0 }
    }

    fn confirm(&self, context: &mut Context) -> EventResult {
        match self.choice {
            0 => context.editor.clear_reload_conflict(self.doc_id),
            1 => {
                if let Err(err) = context.editor.reload_document(self.doc_id) {
                    context.editor.set_error(format!("failed to reload: {err}"));
                } else {
                    context.editor.clear_reload_conflict(self.doc_id);
                }
            }
            _ => {
                if let Err(err) = crate::commands::typed::write_document(context, self.doc_id, true)
                {
                    context
                        .editor
                        .set_error(format!("failed to overwrite: {err}"));
                } else {
                    context.editor.clear_reload_conflict(self.doc_id);
                }
            }
        }

        EventResult::Consumed(Some(Box::new(|compositor, _| {
            compositor.pop();
        })))
    }
}

impl Component for ReloadConflictPrompt {
    fn handle_event(&mut self, event: &Event, context: &mut Context) -> EventResult {
        let Event::Key(key) = event else {
            return EventResult::Ignored(None);
        };

        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.choice = (self.choice + 1).min(OPTIONS.len() - 1);
                EventResult::Consumed(None)
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.choice = self.choice.saturating_sub(1);
                EventResult::Consumed(None)
            }
            KeyCode::Enter => self.confirm(context),
            KeyCode::Esc => {
                self.choice = 0;
                self.confirm(context)
            }
            _ => EventResult::Consumed(None),
        }
    }

    fn render(&mut self, area: Rect, surface: &mut Surface, context: &mut Context) {
        let width = 52.min(area.width.saturating_sub(2)).max(20);
        let height = 8.min(area.height.saturating_sub(2)).max(6);
        let popup = Rect {
            x: area.x + (area.width.saturating_sub(width)) / 2,
            y: area.y + (area.height.saturating_sub(height)) / 2,
            width,
            height,
        };

        let text = context.editor.theme.get("ui.text");
        let background = context.editor.theme.get("ui.popup");
        surface.clear_with(popup, background.patch(text));
        Block::bordered().render(popup, surface);

        let inner = popup.clip_left(2).clip_right(2).clip_top(1);
        surface.set_string(
            inner.x,
            inner.y,
            "File changed on disk",
            context.editor.theme.get("ui.text"),
        );
        surface.set_string(
            inner.x,
            inner.y + 1,
            "The buffer has unsaved changes.",
            context.editor.theme.get("ui.text"),
        );

        let selected = context.editor.theme.get("ui.menu.selected");
        for (index, option) in OPTIONS.iter().enumerate() {
            let y = inner.y + 3 + index as u16;
            let style = if index == self.choice { selected } else { text };
            surface.set_string(inner.x, y, &format!(" {option} "), style);
        }
    }
}
