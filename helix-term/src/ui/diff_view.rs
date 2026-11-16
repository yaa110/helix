use std::{ops::Range, path::PathBuf, time::Instant};

use helix_core::graphemes::{grapheme_width, tab_width_at};
use helix_vcs::{DiffBase, DiffTarget, Hunk};
use helix_view::{
    graphics::{Rect, Style},
    theme::Theme,
    Editor,
};
use tui::{
    buffer::Buffer as Surface,
    text::Span,
    widgets::{Block, BorderType, Widget},
};
use unicode_segmentation::UnicodeSegmentation;

use crate::{
    compositor::{Component, Context, Event, EventResult},
    ctrl, key,
};

const CONTEXT_LINES: usize = 3;
const HORIZONTAL_SCROLL_STEP: usize = 8;
const MIN_AREA_WIDTH: u16 = 24;
const MIN_AREA_HEIGHT: u16 = 6;
const HEADER_HEIGHT: u16 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub number: usize,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffRow {
    Context {
        left: DiffLine,
        right: DiffLine,
    },
    Change {
        left: Option<DiffLine>,
        right: Option<DiffLine>,
    },
    HunkHeader {
        old: Range<usize>,
        new: Range<usize>,
    },
    Gap {
        skipped: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SideKind {
    Context,
    Removed,
    Added,
}

pub struct DiffView {
    path: PathBuf,
    display_path: String,
    old_text: String,
    new_text: String,
    target: DiffTarget,
    label: String,
    trust_full: bool,
    tab_width: usize,
    rows: Vec<DiffRow>,
    additions: usize,
    deletions: usize,
    max_line: usize,
    scroll: usize,
    horizontal_scroll: usize,
    scroll_accel: super::scroll::ScrollAccelerator,
    last_content_area: Rect,
}

impl DiffView {
    pub fn new(
        path: PathBuf,
        display_path: String,
        new_text: String,
        target: DiffTarget,
        base: DiffBase,
        trust_full: bool,
        tab_width: usize,
    ) -> Self {
        let mut view = Self {
            path,
            display_path,
            old_text: String::from_utf8_lossy(&base.contents).into_owned(),
            new_text,
            target,
            label: base.label,
            trust_full,
            tab_width: tab_width.max(1),
            rows: Vec::new(),
            additions: 0,
            deletions: 0,
            max_line: 1,
            scroll: 0,
            horizontal_scroll: 0,
            scroll_accel: Default::default(),
            last_content_area: Rect::default(),
        };
        view.refresh();
        view
    }

    fn refresh(&mut self) {
        self.rows = build_rows(&self.old_text, &self.new_text, CONTEXT_LINES);

        let mut additions = 0;
        let mut deletions = 0;
        let mut max_line = 1;
        for row in &self.rows {
            match row {
                DiffRow::Context { left, right } => {
                    max_line = max_line.max(left.number).max(right.number);
                }
                DiffRow::Change { left, right } => {
                    if let Some(left) = left {
                        deletions += 1;
                        max_line = max_line.max(left.number);
                    }
                    if let Some(right) = right {
                        additions += 1;
                        max_line = max_line.max(right.number);
                    }
                }
                DiffRow::HunkHeader { .. } | DiffRow::Gap { .. } => {}
            }
        }

        self.additions = additions;
        self.deletions = deletions;
        self.max_line = max_line;
        self.clamp_scroll();
    }

    fn mode_label(&self) -> String {
        match self.target {
            DiffTarget::Head => "uncommitted (vs HEAD)".to_string(),
            DiffTarget::DefaultBranch => format!("branch (vs {})", self.label),
        }
    }

    fn toggle(&mut self, editor: &mut Editor) {
        let target = match self.target {
            DiffTarget::Head => DiffTarget::DefaultBranch,
            DiffTarget::DefaultBranch => DiffTarget::Head,
        };

        match editor
            .diff_providers
            .get_diff_base_for(&self.path, target, self.trust_full)
        {
            Ok(base) => {
                self.old_text = String::from_utf8_lossy(&base.contents).into_owned();
                self.label = base.label;
                self.target = target;
                self.horizontal_scroll = 0;
                self.refresh();
                editor.set_status(format!("diff: {}", self.mode_label()));
            }
            Err(err) => editor.set_error(format!("diff: {err}")),
        }
    }

    fn page_height(&self) -> usize {
        self.last_content_area.height as usize
    }

    fn clamp_scroll(&mut self) {
        self.scroll = self
            .scroll
            .min(self.rows.len().saturating_sub(self.page_height()));
    }

    fn scroll_by(&mut self, delta: isize) {
        self.scroll =
            super::scroll::scroll_offset(self.scroll, delta, self.rows.len(), self.page_height());
    }

    fn scroll_to(&mut self, offset: usize) {
        self.scroll = offset.min(self.rows.len().saturating_sub(self.page_height()));
    }

    fn scroll_half_page(&mut self, direction: isize) {
        let step = (self.page_height() / 2).max(1) as isize * direction;
        self.scroll_by(step);
    }

    fn scroll_page(&mut self, direction: isize) {
        let step = self.page_height().max(1) as isize * direction;
        self.scroll_by(step);
    }

    fn scroll_horizontal(&mut self, direction: isize) {
        if direction < 0 {
            self.horizontal_scroll = self
                .horizontal_scroll
                .saturating_sub(HORIZONTAL_SCROLL_STEP);
        } else {
            self.horizontal_scroll = self
                .horizontal_scroll
                .saturating_add(HORIZONTAL_SCROLL_STEP);
        }
    }

    fn render_header(&self, area: Rect, surface: &mut Surface, theme: &Theme) {
        let text_style = theme.get("ui.text");
        let dim_style = theme.get("ui.text.inactive");
        let plus_style = theme.get("diff.plus");
        let minus_style = theme.get("diff.minus");

        let mode = self.mode_label();
        let mut x = area.x;
        write_clipped(surface, area, &mut x, &mode, text_style);

        x = x.saturating_add(1);
        write_clipped(
            surface,
            area,
            &mut x,
            &format!("+{}", self.additions),
            plus_style,
        );
        write_clipped(surface, area, &mut x, " ", text_style);
        write_clipped(
            surface,
            area,
            &mut x,
            &format!("-{}", self.deletions),
            minus_style,
        );

        let hints = "t toggle  q close";
        let hint_width = Span::raw(hints).width() as u16;
        if hint_width < area.width {
            let hint_x = area.right().saturating_sub(hint_width);
            if hint_x > x {
                surface.set_stringn(hint_x, area.y, hints, hint_width as usize, dim_style);
            }
        }
    }

    fn render_rows(&self, area: Rect, surface: &mut Surface, theme: &Theme) {
        let left_width = area.width / 2;
        let right_width = area.width.saturating_sub(left_width + 1);
        let divider_x = area.x + left_width;
        let divider_style = theme.get("ui.window");
        let dim_style = theme.get("ui.text.inactive");
        let pane_width = left_width.min(right_width);
        let number_width = (self.max_line.to_string().len().max(1) as u16)
            .min(pane_width.saturating_sub(4).max(1));

        for (offset, row) in self
            .rows
            .iter()
            .skip(self.scroll)
            .take(area.height as usize)
            .enumerate()
        {
            let y = area.y + offset as u16;

            match row {
                DiffRow::HunkHeader { old, new } => {
                    let text = format!(
                        "@@ {} {} @@",
                        format_range('-', old),
                        format_range('+', new)
                    );
                    surface.set_stringn(
                        area.x + 1,
                        y,
                        &text,
                        area.width.saturating_sub(2) as usize,
                        dim_style,
                    );
                }
                DiffRow::Gap { skipped } => {
                    let text = format!("... {skipped} unchanged lines ...");
                    let text_width = Span::raw(&text).width() as u16;
                    let x = area.x + area.width.saturating_sub(text_width) / 2;
                    surface.set_stringn(x, y, &text, area.width as usize, dim_style);
                }
                DiffRow::Context { left, right } => {
                    self.render_line(
                        surface,
                        area.x,
                        y,
                        left_width,
                        left,
                        SideKind::Context,
                        theme,
                        number_width,
                    );
                    self.render_line(
                        surface,
                        divider_x + 1,
                        y,
                        right_width,
                        right,
                        SideKind::Context,
                        theme,
                        number_width,
                    );
                    set_symbol(surface, divider_x, y, "│", divider_style);
                }
                DiffRow::Change { left, right } => {
                    if let Some(line) = left {
                        self.render_line(
                            surface,
                            area.x,
                            y,
                            left_width,
                            line,
                            SideKind::Removed,
                            theme,
                            number_width,
                        );
                    }
                    if let Some(line) = right {
                        self.render_line(
                            surface,
                            divider_x + 1,
                            y,
                            right_width,
                            line,
                            SideKind::Added,
                            theme,
                            number_width,
                        );
                    }
                    set_symbol(surface, divider_x, y, "│", divider_style);
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_line(
        &self,
        surface: &mut Surface,
        x: u16,
        y: u16,
        width: u16,
        line: &DiffLine,
        kind: SideKind,
        theme: &Theme,
        number_width: u16,
    ) {
        let (gutter_scope, text_scope, marker) = match kind {
            SideKind::Context => ("ui.linenr", "ui.text", " "),
            SideKind::Removed => ("diff.minus.gutter", "diff.minus", "-"),
            SideKind::Added => ("diff.plus.gutter", "diff.plus", "+"),
        };

        let gutter_style = theme.get(gutter_scope);
        let text_style = line_style(theme, text_scope);

        // Tint the whole pane row, not just the glyphs, so changed lines read
        // like GitHub's diff view.
        surface.set_style(Rect::new(x, y, width, 1), text_style);

        let number = format!("{:>width$}", line.number, width = number_width as usize);
        let (prefix_x, _) = surface.set_stringn(x, y, &number, number_width as usize, gutter_style);
        surface.set_stringn(prefix_x, y, " ", 1, gutter_style);
        surface.set_stringn(prefix_x + 1, y, marker, 1, gutter_style);
        surface.set_stringn(prefix_x + 2, y, " ", 1, gutter_style);

        let text_x = prefix_x + 3;
        let text_width = width.saturating_sub(number_width + 3);
        render_text(
            surface,
            text_x,
            y,
            text_width,
            &line.text,
            self.horizontal_scroll,
            self.tab_width,
            text_style,
        );
    }
}

impl Component for DiffView {
    fn handle_event(&mut self, event: &Event, cx: &mut Context) -> EventResult {
        match event {
            Event::Mouse(mouse) => {
                let max_lines = cx.editor.config().scroll_lines.unsigned_abs();
                let Some(delta) = super::scroll::wheel_delta(mouse.kind, max_lines) else {
                    return EventResult::Consumed(None);
                };
                let step = self
                    .scroll_accel
                    .step(delta.signum(), max_lines, Instant::now());
                if super::scroll::contains(self.last_content_area, mouse.column, mouse.row) {
                    self.scroll_by(step);
                }
                EventResult::Consumed(None)
            }
            Event::Key(event) => {
                let close: Option<crate::compositor::Callback> = Some(Box::new(
                    |compositor: &mut crate::compositor::Compositor, _: &mut Context| {
                        compositor.pop();
                    },
                ));

                match *event {
                    key!(Esc) | key!('q') => EventResult::Consumed(close),
                    key!('t') => {
                        self.toggle(cx.editor);
                        EventResult::Consumed(None)
                    }
                    key!('j') | key!(Down) => {
                        self.scroll_by(1);
                        EventResult::Consumed(None)
                    }
                    key!('k') | key!(Up) => {
                        self.scroll_by(-1);
                        EventResult::Consumed(None)
                    }
                    key!('h') | key!(Left) => {
                        self.scroll_horizontal(-1);
                        EventResult::Consumed(None)
                    }
                    key!('l') | key!(Right) => {
                        self.scroll_horizontal(1);
                        EventResult::Consumed(None)
                    }
                    key!('0') => {
                        self.horizontal_scroll = 0;
                        EventResult::Consumed(None)
                    }
                    key!('g') | key!(Home) => {
                        self.scroll_to(0);
                        EventResult::Consumed(None)
                    }
                    key!('G') | key!(End) => {
                        self.scroll_to(usize::MAX);
                        EventResult::Consumed(None)
                    }
                    key!(PageDown) | key!('f') => {
                        self.scroll_page(1);
                        EventResult::Consumed(None)
                    }
                    key!(PageUp) | key!('b') => {
                        self.scroll_page(-1);
                        EventResult::Consumed(None)
                    }
                    ctrl!('d') => {
                        self.scroll_half_page(1);
                        EventResult::Consumed(None)
                    }
                    ctrl!('u') => {
                        self.scroll_half_page(-1);
                        EventResult::Consumed(None)
                    }
                    _ => EventResult::Consumed(None),
                }
            }
            // Do not let pasted text bubble down into the editor while the
            // diff view is open.
            Event::Paste(_) => EventResult::Consumed(None),
            _ => EventResult::Ignored(None),
        }
    }

    fn render(&mut self, area: Rect, surface: &mut Surface, cx: &mut Context) {
        if area.width < MIN_AREA_WIDTH || area.height < MIN_AREA_HEIGHT {
            return;
        }

        let theme = &cx.editor.theme;
        let background = theme.get("ui.background");
        let border_style = theme
            .try_get_exact("ui.window")
            .or_else(|| theme.try_get_exact("ui.linenr"))
            .unwrap_or_default();

        surface.clear_with(area, background);
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(border_style)
            .title(Span::raw(format!(" diff: {} ", self.display_path)));
        let inner = block.inner(area);
        block.render(area, surface);
        if inner.width < 10 || inner.height <= HEADER_HEIGHT {
            return;
        }

        let content_area = Rect::new(
            inner.x,
            inner.y + HEADER_HEIGHT,
            inner.width,
            inner.height - HEADER_HEIGHT,
        );
        self.last_content_area = content_area;
        self.clamp_scroll();

        self.render_header(inner, surface, theme);

        let separator_style = theme.get("ui.background.separator");
        for x in inner.left()..inner.right() {
            set_symbol(surface, x, inner.y + 1, "─", separator_style);
        }

        if self.rows.is_empty() {
            let message = "No changes";
            let message_width = Span::raw(message).width() as u16;
            let x = content_area.x + content_area.width.saturating_sub(message_width) / 2;
            let y = content_area.y + content_area.height / 2;
            surface.set_stringn(
                x,
                y,
                message,
                content_area.width as usize,
                theme.get("ui.text.inactive"),
            );
            return;
        }

        self.render_rows(content_area, surface, theme);
    }
}

fn write_clipped(surface: &mut Surface, area: Rect, x: &mut u16, text: &str, style: Style) {
    let width = area.right().saturating_sub(*x) as usize;
    if width == 0 {
        return;
    }
    let (next, _) = surface.set_stringn(*x, area.y, text, width, style);
    *x = next;
}

fn line_style(theme: &Theme, scope: &str) -> Style {
    let mut style = theme.get(scope);
    if let Some(background) = theme
        .try_get_exact(&format!("{scope}.line"))
        .and_then(|style| style.bg)
    {
        style = style.bg(background);
    }
    style
}

fn format_range(prefix: char, range: &Range<usize>) -> String {
    let len = range.end.saturating_sub(range.start);
    let start = if len == 0 {
        range.start
    } else {
        range.start + 1
    };
    format!("{prefix}{start},{len}")
}

fn set_symbol(surface: &mut Surface, x: u16, y: u16, symbol: &str, style: Style) {
    if let Some(cell) = surface.get_mut(x, y) {
        cell.set_symbol(symbol).set_style(style);
    }
}

#[allow(clippy::too_many_arguments)]
fn render_text(
    surface: &mut Surface,
    x: u16,
    y: u16,
    width: u16,
    text: &str,
    horizontal_scroll: usize,
    tab_width: usize,
    style: Style,
) {
    if width == 0 {
        return;
    }

    let max_x = x.saturating_add(width);
    let tab_width = tab_width.max(1) as u16;
    let mut visual_x = 0usize;
    let mut draw_x = x;

    for grapheme in text.graphemes(true) {
        let grapheme_width = if grapheme == "\t" {
            tab_width_at(visual_x, tab_width)
        } else {
            grapheme_width(grapheme)
        };
        if grapheme_width == 0 {
            continue;
        }

        let start = visual_x;
        visual_x += grapheme_width;

        if start < horizontal_scroll {
            continue;
        }
        if draw_x.saturating_add(grapheme_width as u16) > max_x {
            break;
        }

        if grapheme == "\t" {
            let spaces = " ".repeat(grapheme_width);
            surface.set_tab(draw_x, y, &spaces, style);
        } else {
            surface.set_grapheme(draw_x, y, grapheme, grapheme_width, style);
        }
        draw_x += grapheme_width as u16;
    }
}

fn split_lines(text: &str) -> Vec<&str> {
    text.split_inclusive('\n')
        .map(|line| {
            let line = line.strip_suffix('\n').unwrap_or(line);
            line.strip_suffix('\r').unwrap_or(line)
        })
        .collect()
}

fn line(lines: &[&str], index: usize) -> DiffLine {
    DiffLine {
        number: index + 1,
        text: lines[index].to_string(),
    }
}

pub(crate) fn build_rows(old: &str, new: &str, context: usize) -> Vec<DiffRow> {
    let old_lines = split_lines(old);
    let new_lines = split_lines(new);
    let hunks = helix_vcs::line_hunks(old, new);
    if hunks.is_empty() {
        return Vec::new();
    }

    struct Window {
        old: Range<usize>,
        new: Range<usize>,
        hunks: Vec<Hunk>,
    }

    let mut windows: Vec<Window> = Vec::new();
    for hunk in hunks {
        let old_start = hunk.before.start as usize;
        let old_end = hunk.before.end as usize;
        let new_start = hunk.after.start as usize;
        let new_end = hunk.after.end as usize;

        let old = old_start.saturating_sub(context)..(old_end + context).min(old_lines.len());
        let new = new_start.saturating_sub(context)..(new_end + context).min(new_lines.len());

        if let Some(last) = windows.last_mut() {
            if old.start <= last.old.end && new.start <= last.new.end {
                last.old.end = last.old.end.max(old.end);
                last.new.end = last.new.end.max(new.end);
                last.hunks.push(hunk);
                continue;
            }
        }

        windows.push(Window {
            old,
            new,
            hunks: vec![hunk],
        });
    }

    let mut rows = Vec::new();
    let mut previous_old = 0;
    let mut previous_new = 0;

    for window in windows {
        let skipped_old = window.old.start.saturating_sub(previous_old);
        let skipped_new = window.new.start.saturating_sub(previous_new);
        if skipped_old > 0 || skipped_new > 0 {
            rows.push(DiffRow::Gap {
                skipped: skipped_old.max(skipped_new),
            });
        }

        rows.push(DiffRow::HunkHeader {
            old: window.old.clone(),
            new: window.new.clone(),
        });

        let mut old_pos = window.old.start;
        let mut new_pos = window.new.start;

        for hunk in &window.hunks {
            let hunk_old_start = hunk.before.start as usize;
            let hunk_old_end = (hunk.before.end as usize).min(old_lines.len());
            let hunk_new_start = hunk.after.start as usize;
            let hunk_new_end = (hunk.after.end as usize).min(new_lines.len());

            while old_pos < hunk_old_start && new_pos < hunk_new_start {
                rows.push(DiffRow::Context {
                    left: line(&old_lines, old_pos),
                    right: line(&new_lines, new_pos),
                });
                old_pos += 1;
                new_pos += 1;
            }

            let row_count = (hunk_old_end - hunk_old_start).max(hunk_new_end - hunk_new_start);
            for offset in 0..row_count {
                let left_index = hunk_old_start + offset;
                let right_index = hunk_new_start + offset;
                rows.push(DiffRow::Change {
                    left: (left_index < hunk_old_end).then(|| line(&old_lines, left_index)),
                    right: (right_index < hunk_new_end).then(|| line(&new_lines, right_index)),
                });
            }

            old_pos = hunk_old_end;
            new_pos = hunk_new_end;
        }

        while old_pos < window.old.end && new_pos < window.new.end {
            rows.push(DiffRow::Context {
                left: line(&old_lines, old_pos),
                right: line(&new_lines, new_pos),
            });
            old_pos += 1;
            new_pos += 1;
        }

        previous_old = window.old.end;
        previous_new = window.new.end;
    }

    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aligns_modified_lines() {
        let rows = build_rows("a\nb\nc\nd\ne\nf\ng\nh\n", "a\nb\nX\nd\ne\nf\ng\nh\n", 3);

        assert!(matches!(rows[0], DiffRow::HunkHeader { .. }));
        assert!(matches!(
            &rows[1],
            DiffRow::Context { left, right }
                if left.number == 1 && right.number == 1
                    && left.text == "a" && right.text == "a"
        ));
        assert!(matches!(
            &rows[3],
            DiffRow::Change { left: Some(left), right: Some(right) }
                if left.number == 3 && right.number == 3
                    && left.text == "c" && right.text == "X"
        ));
        assert!(matches!(
            &rows[6],
            DiffRow::Context { left, right }
                if left.number == 6 && right.number == 6
                    && left.text == "f" && right.text == "f"
        ));
        assert_eq!(rows.len(), 7);
    }

    #[test]
    fn insertion_has_no_left_side() {
        let rows = build_rows("a\nb\n", "new\na\nb\n", 3);

        assert!(matches!(
            &rows[1],
            DiffRow::Change {
                left: None,
                right: Some(right)
            } if right.number == 1 && right.text == "new"
        ));
        assert!(matches!(
            &rows[2],
            DiffRow::Context { left, right }
                if left.number == 1 && right.number == 2
                    && left.text == "a" && right.text == "a"
        ));
    }

    #[test]
    fn separates_distant_hunks_with_a_gap() {
        let old = (1..=30)
            .map(|line| format!("line {line}\n"))
            .collect::<String>();
        let new = old
            .replace("line 5\n", "changed 5\n")
            .replace("line 25\n", "changed 25\n");
        let rows = build_rows(&old, &new, 3);

        assert!(rows
            .iter()
            .any(|row| matches!(row, DiffRow::Gap { skipped } if *skipped > 0)));
        assert_eq!(
            rows.iter()
                .filter(|row| matches!(row, DiffRow::HunkHeader { .. }))
                .count(),
            2
        );
    }

    #[test]
    fn formats_empty_ranges_like_git() {
        assert_eq!(format_range('-', &(0..0)), "-0,0");
        assert_eq!(format_range('+', &(0..3)), "+1,3");
        assert_eq!(format_range('-', &(4..9)), "-5,5");
    }

    #[test]
    fn splits_crlf_lines() {
        assert_eq!(split_lines("a\r\nb\r\n"), vec!["a", "b"]);
        assert_eq!(split_lines(""), Vec::<&str>::new());
    }
}
