use std::time::{Duration, Instant};

use helix_view::graphics::Rect;
use helix_view::input::MouseEventKind;

pub(crate) const WHEEL_ACCELERATION_WINDOW: Duration = Duration::from_millis(120);

#[derive(Debug, Clone, Default)]
pub(crate) struct ScrollAccelerator {
    direction: isize,
    lines: usize,
    last: Option<Instant>,
}

impl ScrollAccelerator {
    pub fn step(&mut self, direction: isize, max_lines: usize, now: Instant) -> isize {
        let direction = direction.signum();
        if direction == 0 || max_lines == 0 {
            return 0;
        }

        let continuing = self.direction == direction
            && self
                .last
                .is_some_and(|last| now.duration_since(last) <= WHEEL_ACCELERATION_WINDOW);
        self.lines = if continuing {
            (self.lines + 1).min(max_lines)
        } else {
            1
        };
        self.direction = direction;
        self.last = Some(now);
        direction * self.lines as isize
    }
}

pub(crate) fn contains(area: Rect, column: u16, row: u16) -> bool {
    column >= area.x && column < area.right() && row >= area.y && row < area.bottom()
}

/// Convert a wheel event into a line delta.
pub(crate) fn wheel_delta(kind: MouseEventKind, lines: usize) -> Option<isize> {
    let lines = lines as isize;
    match kind {
        MouseEventKind::ScrollUp => Some(-lines),
        MouseEventKind::ScrollDown => Some(lines),
        _ => None,
    }
}

/// Move a viewport offset by `delta` lines, clamped to the scrollable range.
pub(crate) fn scroll_offset(current: usize, delta: isize, len: usize, height: usize) -> usize {
    if len == 0 || height == 0 {
        return current;
    }
    let max = len.saturating_sub(height);
    let next = if delta < 0 {
        current.saturating_sub(delta.unsigned_abs())
    } else {
        current.saturating_add(delta as usize)
    };
    next.min(max)
}

/// Keep the selection visible after a viewport-only scroll, matching the
/// editor's behavior when the cursor would leave the screen.
pub(crate) fn clamp_selection(selected: usize, offset: usize, len: usize, height: usize) -> usize {
    if len == 0 || height == 0 {
        return selected.min(len.saturating_sub(1));
    }
    let last_visible = (offset + height - 1).min(len - 1);
    selected.clamp(offset, last_visible)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wheel_acceleration_starts_at_one_line_and_ramps_to_the_limit() {
        let start = Instant::now();
        let mut accel = ScrollAccelerator::default();

        assert_eq!(accel.step(1, 3, start), 1);
        assert_eq!(accel.step(1, 3, start + Duration::from_millis(10)), 2);
        assert_eq!(accel.step(1, 3, start + Duration::from_millis(20)), 3);
        assert_eq!(accel.step(1, 3, start + Duration::from_millis(30)), 3);
    }

    #[test]
    fn wheel_acceleration_resets_after_a_pause_or_direction_change() {
        let start = Instant::now();
        let mut accel = ScrollAccelerator::default();

        assert_eq!(accel.step(1, 3, start), 1);
        assert_eq!(accel.step(1, 3, start + Duration::from_millis(10)), 2);
        assert_eq!(
            accel.step(
                1,
                3,
                start + Duration::from_millis(10) + WHEEL_ACCELERATION_WINDOW * 2
            ),
            1
        );
        assert_eq!(accel.step(-1, 3, start + Duration::from_millis(500)), -1);
        assert_eq!(accel.step(-1, 3, start + Duration::from_millis(510)), -2);
    }

    #[test]
    fn wheel_events_map_to_line_deltas() {
        assert_eq!(wheel_delta(MouseEventKind::ScrollUp, 0), Some(0));
        assert_eq!(wheel_delta(MouseEventKind::ScrollDown, 3), Some(3));
        assert_eq!(wheel_delta(MouseEventKind::Moved, 3), None);
    }

    #[test]
    fn scrolls_by_delta_and_clamps_to_bounds() {
        assert_eq!(scroll_offset(0, 3, 100, 10), 3);
        assert_eq!(scroll_offset(3, -3, 100, 10), 0);
        assert_eq!(scroll_offset(0, -3, 100, 10), 0);
        assert_eq!(scroll_offset(8, 10, 12, 10), 2);
        assert_eq!(scroll_offset(8, 10, 12, 0), 8);
    }

    #[test]
    fn selection_clamps_to_the_visible_range() {
        assert_eq!(clamp_selection(0, 3, 100, 10), 3);
        assert_eq!(clamp_selection(9, 2, 100, 5), 6);
        assert_eq!(clamp_selection(5, 2, 100, 5), 5);
        assert_eq!(clamp_selection(99, 2, 100, 5), 6);
        assert_eq!(clamp_selection(99, 2, 0, 5), 0);
    }
}
