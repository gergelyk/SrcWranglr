//! A minimap of the file shown: one thin bar per line (as long as the line,
//! indented like it), the part on screen lit, the line in focus marked.
//! Clicking or dragging on it scrolls the code.

use std::ops::Range;

use egui::{Color32, Pos2, Rect, Sense, Ui, Vec2};

use super::scheme::SourceColors;

pub const MINIMAP_WIDTH: f32 = 90.0;
/// A line is drawn at most this tall, so short files don't blow up.
const MAX_LINE_HEIGHT: f32 = 2.0;
/// Columns that fit the width; longer lines are cut.
const MAX_COLUMNS: usize = 120;
const PADDING: f32 = 4.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Code,
    Comment,
    Preproc,
}

/// What the minimap draws for one line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Line {
    indent: u16,
    len: u16,
    kind: Kind,
}

/// The lines of `contents` (at `ranges`) boiled down for drawing.
pub fn summarize(contents: &str, ranges: &[Range<usize>]) -> Vec<Line> {
    ranges
        .iter()
        .map(|range| {
            let text = contents.get(range.clone()).unwrap_or("").trim_end();
            let trimmed = text.trim_start();
            let indent = text.chars().take_while(|c| c.is_whitespace()).count();
            let kind = if trimmed.starts_with("//")
                || trimmed.starts_with("/*")
                || trimmed.starts_with('*')
            {
                Kind::Comment
            } else if trimmed.starts_with('#') {
                Kind::Preproc
            } else {
                Kind::Code
            };
            Line {
                indent: indent.min(MAX_COLUMNS) as u16,
                len: trimmed.chars().count().min(MAX_COLUMNS) as u16,
                kind,
            }
        })
        .collect()
}

/// How far the code is scrolled, from its scroll area.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Metrics {
    /// Scrolled down by.
    pub offset: f32,
    /// The height of all the lines.
    pub content: f32,
    /// The height of what is shown.
    pub view: f32,
}

/// The height of a line in the minimap, for `count` lines in `height`.
fn line_height(count: usize, height: f32) -> f32 {
    (height / count.max(1) as f32).min(MAX_LINE_HEIGHT)
}

/// The scroll offset that puts the line under `y` (from the minimap's top)
/// in the middle of the view.
fn offset_for(y: f32, line_height: f32, count: usize, metrics: Metrics) -> f32 {
    let pitch = metrics.content / count.max(1) as f32;
    let line = (y / line_height).max(0.0);
    let max = (metrics.content - metrics.view).max(0.0);
    (line * pitch - metrics.view / 2.0).clamp(0.0, max)
}

/// Draws the minimap in `rect` and returns the scroll offset to jump to
/// when it is clicked or dragged. `focus` is the 0-based line in focus.
pub fn show(
    ui: &mut Ui,
    rect: Rect,
    lines: &[Line],
    colors: &SourceColors,
    metrics: Metrics,
    focus: usize,
) -> Option<f32> {
    let response = ui.interact(
        rect,
        ui.make_persistent_id("code_minimap"),
        Sense::click_and_drag(),
    );
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, colors.background);
    painter.vline(
        rect.left(),
        rect.y_range(),
        egui::Stroke::new(1.0, colors.line_number.gamma_multiply(0.4)),
    );
    let height = line_height(lines.len(), rect.height());
    let column = (rect.width() - 2.0 * PADDING) / MAX_COLUMNS as f32;
    let color_of = |kind: Kind| match kind {
        Kind::Code => colors.text,
        Kind::Comment => colors.comment,
        Kind::Preproc => colors.preproc,
    };
    // A file taller than the minimap shows the longest line of each row
    // of pixels.
    let step = height.recip().max(1.0);
    let pixel = height.max(1.0);
    let mut index = 0.0f32;
    while (index as usize) < lines.len() {
        let first = index as usize;
        let last = ((index + step) as usize).clamp(first + 1, lines.len());
        let widest = lines[first..last]
            .iter()
            .max_by_key(|l| l.len)
            .copied()
            .unwrap_or(lines[first]);
        if widest.len > 0 {
            let y = rect.top() + first as f32 * height;
            let x = rect.left() + PADDING + f32::from(widest.indent) * column;
            let bar = Rect::from_min_size(
                Pos2::new(x, y),
                Vec2::new(f32::from(widest.len) * column, (pixel - 0.5).max(0.5)),
            );
            painter.rect_filled(bar, 0.0, color_of(widest.kind).gamma_multiply(0.7));
        }
        index += step;
    }

    let pitch = metrics.content / lines.len().max(1) as f32;
    if pitch > 0.0 {
        let top = rect.top() + metrics.offset / pitch * height;
        let bottom = top + metrics.view / pitch * height;
        let shown = Rect::from_min_max(
            Pos2::new(rect.left() + 1.0, top.max(rect.top())),
            Pos2::new(rect.right(), bottom.min(rect.bottom())),
        );
        painter.rect_filled(
            shown,
            0.0,
            Color32::from_white_alpha(18).gamma_multiply(1.0),
        );
        painter.rect_stroke(
            shown,
            0.0,
            egui::Stroke::new(1.0, colors.line_number.gamma_multiply(0.6)),
            egui::StrokeKind::Inside,
        );
    }
    let y = rect.top() + focus as f32 * height;
    let marker = Rect::from_min_size(
        Pos2::new(rect.left() + 1.0, y),
        Vec2::new(rect.width() - 1.0, height.max(2.0)),
    );
    painter.rect_filled(marker, 0.0, colors.active_name);

    let pos = response.interact_pointer_pos()?;
    if !(response.clicked() || response.dragged() || response.is_pointer_button_down_on()) {
        return None;
    }
    Some(offset_for(pos.y - rect.top(), height, lines.len(), metrics))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_summarized_by_indent_length_and_kind() {
        let text = "#include <a>\n  int x; // c\n// note\n\n";
        let ranges = super::super::line_ranges(text);
        let lines = summarize(text, &ranges);
        assert_eq!(lines[0].kind, Kind::Preproc);
        assert_eq!((lines[1].indent, lines[1].kind), (2, Kind::Code));
        assert_eq!(lines[2].kind, Kind::Comment);
        assert_eq!(lines[3].len, 0);
    }

    #[test]
    fn clicking_centers_the_view_on_the_line_and_stays_in_range() {
        let metrics = Metrics {
            offset: 0.0,
            content: 1000.0,
            view: 100.0,
        };
        // 100 lines of 10px, drawn 2px each.
        assert_eq!(offset_for(100.0, 2.0, 100, metrics), 450.0);
        assert_eq!(offset_for(0.0, 2.0, 100, metrics), 0.0);
        assert_eq!(offset_for(400.0, 2.0, 100, metrics), 900.0);
    }

    #[test]
    fn short_files_are_not_stretched() {
        assert_eq!(line_height(10, 1000.0), MAX_LINE_HEIGHT);
        assert_eq!(line_height(2000, 1000.0), 0.5);
    }
}
