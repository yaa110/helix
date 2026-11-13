use helix_core::Position;
use helix_view::{
    graphics::{CursorKind, Rect},
    Editor,
};
use tui::buffer::Buffer;

use crate::compositor::{Component, Context, Event, EventResult};

pub const FULL_OVERLAID_MAX_WIDTH: u16 = 200;

/// Contains a component placed in the center of the parent component
pub struct Overlay<T> {
    /// Child component
    pub content: T,
    /// Function to compute the size and position of the child component
    pub calc_child_size: Box<dyn Fn(Rect) -> Rect>,
}

/// Surrounds the component with a margin of 5% on each side, and an additional 2 rows at the bottom
pub fn overlaid<T>(content: T) -> Overlay<T> {
    Overlay {
        content,
        calc_child_size: Box::new(|rect: Rect| {
            let percentage = if rect.width < FULL_OVERLAID_MAX_WIDTH {
                100
            } else {
                90
            };
            clip_rect_relative(rect.clip_bottom(2), percentage, percentage)
        }),
    }
}

/// Centers a component using explicit percentages of the available area.
pub fn overlaid_with<T>(content: T, percent_horizontal: u8, percent_vertical: u8) -> Overlay<T> {
    Overlay {
        content,
        calc_child_size: Box::new(move |rect: Rect| {
            clip_rect_relative(rect.clip_bottom(2), percent_horizontal, percent_vertical)
        }),
    }
}

fn clip_rect_relative(rect: Rect, percent_horizontal: u8, percent_vertical: u8) -> Rect {
    fn mul_and_cast(size: u16, factor: u8) -> u16 {
        ((size as u32) * (factor as u32) / 100).try_into().unwrap()
    }

    let inner_w = mul_and_cast(rect.width, percent_horizontal);
    let inner_h = mul_and_cast(rect.height, percent_vertical);

    let offset_x = rect.width.saturating_sub(inner_w) / 2;
    let offset_y = rect.height.saturating_sub(inner_h) / 2;

    Rect {
        x: rect.x + offset_x,
        y: rect.y + offset_y,
        width: inner_w,
        height: inner_h,
    }
}

impl<T: Component + 'static> Component for Overlay<T> {
    fn render(&mut self, area: Rect, frame: &mut Buffer, ctx: &mut Context) {
        let dimensions = (self.calc_child_size)(area);
        self.content.render(dimensions, frame, ctx)
    }

    fn required_size(&mut self, (width, height): (u16, u16)) -> Option<(u16, u16)> {
        let area = Rect {
            x: 0,
            y: 0,
            width,
            height,
        };
        let dimensions = (self.calc_child_size)(area);
        let viewport = (dimensions.width, dimensions.height);
        let _ = self.content.required_size(viewport)?;
        Some((width, height))
    }

    fn handle_event(&mut self, event: &Event, ctx: &mut Context) -> EventResult {
        self.content.handle_event(event, ctx)
    }

    fn cursor(&self, area: Rect, ctx: &Editor) -> (Option<Position>, CursorKind) {
        let dimensions = (self.calc_child_size)(area);
        self.content.cursor(dimensions, ctx)
    }

    fn id(&self) -> Option<&'static str> {
        self.content.id()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_overlay_uses_requested_percentages() {
        let overlay = overlaid_with((), 30, 30);
        let child = (overlay.calc_child_size)(Rect::new(0, 0, 100, 40));
        assert_eq!(child.width, 30);
        assert_eq!(child.height, 11);
    }
}
