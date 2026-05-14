use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
};

use crate::app::{
    AnnotatedLine, App, DiffViewMode, ExpandDirection, GAP_EXPAND_BATCH, VisualSelection,
};
use crate::model::LineSide;
use crate::theme::Theme;
use crate::ui::comment_panel;
use crate::ui::diff_side_by_side::render_side_by_side_diff;
use crate::ui::diff_unified::render_unified_diff;
use crate::ui::styles;

pub(super) fn render_diff_view(frame: &mut Frame, app: &mut App, area: Rect) {
    match app.diff_view_mode {
        DiffViewMode::Unified => render_unified_diff(frame, app, area),
        DiffViewMode::SideBySide => render_side_by_side_diff(frame, app, area),
    }
}

/// Build a right-aligned title showing diff stats for the current scope.
/// In overview: total stats across all files. In a file: that file's stats.
pub(super) fn diff_stat_title(app: &App) -> Line<'static> {
    let (additions, deletions) = if app.is_cursor_in_overview() || app.current_file_path().is_none()
    {
        let (_, a, d) = app.diff_stat();
        (a, d)
    } else {
        app.diff_files[app.diff_state.current_file_idx].stat()
    };

    let theme = &app.theme;
    Line::from(vec![
        Span::styled(
            format!(" +{additions}"),
            Style::default().fg(theme.diff_add),
        ),
        Span::raw(" "),
        Span::styled(
            format!("-{deletions} "),
            Style::default().fg(theme.diff_del),
        ),
    ])
}

pub(super) fn cursor_indicator(line_idx: usize, current_line_idx: usize) -> &'static str {
    if line_idx == current_line_idx {
        "▶"
    } else {
        " "
    }
}

/// Get cursor indicator with spacing (two characters for line prefixes)
pub(super) fn cursor_indicator_spaced(line_idx: usize, current_line_idx: usize) -> &'static str {
    if line_idx == current_line_idx {
        "▶ "
    } else {
        "  "
    }
}

/// Render an expander line with direction arrow
pub(super) fn render_expander_line(
    lines: &mut Vec<Line<'_>>,
    line_idx: &mut usize,
    current_line_idx: usize,
    direction: ExpandDirection,
    remaining: usize,
    theme: &Theme,
) {
    let arrow = match direction {
        ExpandDirection::Down => "↓",
        ExpandDirection::Up => "↑",
        ExpandDirection::Both => "↕",
    };
    let count = remaining.min(GAP_EXPAND_BATCH);
    let indicator = cursor_indicator_spaced(*line_idx, current_line_idx);
    lines.push(Line::from(vec![
        Span::styled(indicator, styles::current_line_indicator_style(theme)),
        Span::styled(
            format!("       ... {arrow} expand ({count} lines) ..."),
            styles::dim_style(theme),
        ),
    ]));
    *line_idx += 1;
}

/// Render a "N lines hidden" informational line
pub(super) fn render_hidden_lines(
    lines: &mut Vec<Line<'_>>,
    line_idx: &mut usize,
    current_line_idx: usize,
    count: usize,
    theme: &Theme,
) {
    let indicator = cursor_indicator_spaced(*line_idx, current_line_idx);
    lines.push(Line::from(vec![
        Span::styled(indicator, styles::current_line_indicator_style(theme)),
        Span::styled(
            format!("       ... {count} lines hidden ..."),
            styles::dim_style(theme),
        ),
    ]));
    *line_idx += 1;
}

pub(super) fn comment_type_presentation(
    app: &App,
    comment_type: &crate::model::CommentType,
) -> comment_panel::CommentTypePresentation {
    comment_panel::CommentTypePresentation {
        label: app.comment_type_label(comment_type),
        color: app.comment_type_color(comment_type),
    }
}

/// Adjust scroll_offset so the comment input box is visible in the viewport.
///
/// The input box is rendered inline in the diff view, so without this
/// adjustment it can end up below (or above) the visible area when a
/// comment is started near the viewport edge or when typing a multi-line
/// comment grows the box past the bottom. If the box is taller than the
/// viewport we fall back to keeping just the text cursor line visible.
pub(super) fn scroll_comment_input_into_view(
    scroll_offset: &mut usize,
    box_range: Option<(usize, usize)>,
    cursor_line: Option<usize>,
    viewport_height: usize,
    total_lines: usize,
) {
    let Some((box_start, box_end)) = box_range else {
        return;
    };
    if viewport_height == 0 {
        return;
    }

    let box_height = box_end.saturating_sub(box_start) + 1;

    if box_height <= viewport_height {
        if box_start < *scroll_offset {
            *scroll_offset = box_start;
        } else if box_end >= *scroll_offset + viewport_height {
            *scroll_offset = box_end + 1 - viewport_height;
        }
    } else if let Some(cursor) = cursor_line {
        // Box too tall for viewport: keep the text cursor line visible.
        if cursor < *scroll_offset {
            *scroll_offset = cursor;
        } else if cursor >= *scroll_offset + viewport_height {
            *scroll_offset = cursor + 1 - viewport_height;
        }
    }

    // Clamp so we never scroll past the last line.
    let max_scroll = total_lines.saturating_sub(viewport_height);
    if *scroll_offset > max_scroll {
        *scroll_offset = max_scroll;
    }
}

/// Populates `out` with the visual-row -> annotation-index map for the diff
/// viewport and returns how many logical lines fit. Reuses the buffer's
/// capacity to avoid per-frame allocations.
pub(super) fn populate_row_to_annotation(
    out: &mut Vec<usize>,
    line_widths: &[usize],
    viewport_width: usize,
    viewport_height: usize,
    wrap: bool,
    scroll_offset: usize,
) -> usize {
    out.clear();
    out.reserve(viewport_height);
    if wrap && viewport_width > 0 {
        let mut visual_rows_used = 0;
        let mut logical_lines_visible = 0;
        for (i, &width) in line_widths.iter().enumerate() {
            let rows_for_line = if width == 0 {
                1
            } else {
                width.div_ceil(viewport_width)
            };
            if visual_rows_used + rows_for_line > viewport_height {
                break;
            }
            for _ in 0..rows_for_line {
                out.push(scroll_offset + i);
            }
            visual_rows_used += rows_for_line;
            logical_lines_visible += 1;
        }
        logical_lines_visible.max(1)
    } else {
        for i in 0..line_widths.len().min(viewport_height) {
            out.push(scroll_offset + i);
        }
        viewport_height
    }
}

struct OverlayPaint {
    sel: VisualSelection,
    geom: crate::app::PaneGeom,
    inner_left: u16,
    inner_right: u16,
    style: Style,
}

pub(super) fn paint_visual_selection_overlay(
    frame: &mut Frame,
    inner: Rect,
    app: &App,
    sel: VisualSelection,
    theme: &Theme,
) {
    let (start, end) = sel.ordered();
    let paint = OverlayPaint {
        sel,
        geom: app.pane_geometry(inner, sel.anchor.side),
        inner_left: inner.x,
        inner_right: inner.x + inner.width.saturating_sub(1),
        style: styles::visual_selection_style(theme),
    };

    let mut current: Option<(usize, u16, u16)> = None;
    for rel in 0..app.diff_row_to_annotation.len() {
        let ann_idx = app.diff_row_to_annotation[rel];
        if ann_idx < start.annotation_idx {
            continue;
        }
        if ann_idx > end.annotation_idx {
            break;
        }
        let row = inner.y + rel as u16;
        match current {
            Some((cur, first, _)) if cur == ann_idx => {
                current = Some((cur, first, row));
            }
            _ => {
                if let Some(group) = current.take() {
                    paint_annotation_group(frame, app, group, &paint);
                }
                current = Some((ann_idx, row, row));
            }
        }
    }
    if let Some(group) = current.take() {
        paint_annotation_group(frame, app, group, &paint);
    }
}

fn paint_annotation_group(
    frame: &mut Frame,
    app: &App,
    group: (usize, u16, u16),
    paint: &OverlayPaint,
) {
    let (ann_idx, first_row, last_row) = group;
    if paint.geom.content_width == 0 {
        return;
    }

    let side = paint.sel.anchor.side;
    let group_height = (last_row - first_row) as usize + 1;
    let pane_last_col = paint
        .geom
        .content_x_end
        .saturating_sub(1)
        .min(paint.inner_right);

    let Some(content) = app.content_for_side(ann_idx, side) else {
        // Headers and other non-content rows aren't bound by the pane
        // gutter; tint the full inner width.
        for which_row in 0..group_height {
            let rect = Rect {
                x: paint.inner_left,
                y: first_row + which_row as u16,
                width: paint.inner_right - paint.inner_left + 1,
                height: 1,
            };
            frame.buffer_mut().set_style(rect, paint.style);
        }
        return;
    };

    let total_chars = content.chars().count();
    let (lo, hi) = paint.sel.char_range(ann_idx, total_chars);
    if hi <= lo {
        return;
    }

    for which_row in 0..group_height {
        let row_char_start = which_row * paint.geom.content_width;
        let row_char_end = row_char_start + paint.geom.content_width;
        let isect_lo = lo.max(row_char_start);
        let isect_hi = hi.min(row_char_end);
        if isect_hi <= isect_lo {
            continue;
        }
        let col_lo_off = (isect_lo - row_char_start) as u16;
        let col_hi_off = (isect_hi - row_char_start) as u16;
        let col_lo = (paint.geom.content_x_start + col_lo_off).min(pane_last_col);
        let col_hi_excl = paint.geom.content_x_start + col_hi_off;
        if col_hi_excl == 0 {
            continue;
        }
        let col_hi = col_hi_excl.saturating_sub(1).min(pane_last_col);
        if col_lo > col_hi {
            continue;
        }
        let rect = Rect {
            x: col_lo,
            y: first_row + which_row as u16,
            width: col_hi - col_lo + 1,
            height: 1,
        };
        frame.buffer_mut().set_style(rect, paint.style);
    }
}

pub(super) fn is_line_highlighted(app: &App, viewport_idx: usize) -> bool {
    if !app.cursor_line_highlight {
        return false;
    }

    let abs_idx = viewport_idx + app.diff_state.scroll_offset;

    // Cursor line
    if abs_idx == app.diff_state.cursor_line {
        return true;
    }

    // Carryover from V → c: keep the comment-input box lit. The visual
    // selection itself paints via the cell-precise overlay.
    let Some((range, sel_side)) = app.comment_line_range else {
        return false;
    };

    // Adjust the annotation index to account for the comment input box, which
    // may have a different line count than what line_annotations expects.
    let annotation_idx =
        if let Some((box_start, box_len, replaced)) = app.comment_input_annotation_offset {
            if abs_idx < box_start {
                abs_idx
            } else if abs_idx < box_start + box_len {
                // Inside the comment input box - only highlight the portion that
                // maps to annotation entries being replaced (edited comment lines)
                let offset_in_box = abs_idx - box_start;
                if offset_in_box < replaced {
                    box_start + offset_in_box
                } else {
                    return false;
                }
            } else {
                // After the box: shift by the difference between rendered and annotation counts
                // box_len > replaced: input box added extra lines → shift back
                // box_len < replaced: input box is shorter → shift forward
                abs_idx + replaced - box_len
            }
        } else {
            abs_idx
        };

    let Some(annotation) = app.line_annotations.get(annotation_idx) else {
        return false;
    };
    let (file_idx, lineno) = match annotation {
        AnnotatedLine::DiffLine {
            file_idx,
            old_lineno,
            new_lineno,
            ..
        }
        | AnnotatedLine::SideBySideLine {
            file_idx,
            old_lineno,
            new_lineno,
            ..
        } => {
            let ln = match sel_side {
                LineSide::New => *new_lineno,
                LineSide::Old => *old_lineno,
            };
            (*file_idx, ln)
        }
        _ => return false,
    };
    file_idx == app.diff_state.current_file_idx && lineno.is_some_and(|ln| range.contains(ln))
}

pub(super) fn unified_line_bg_style(line: &Line, theme: &Theme) -> Option<Style> {
    let prefix = line.spans.get(2)?.content.as_ref();
    let default_bg = match prefix {
        "+ " => theme.diff_add_bg,
        "- " => theme.diff_del_bg,
        _ => return None,
    };

    let bg = line
        .spans
        .last()
        .and_then(|span| span.style.bg)
        .unwrap_or(default_bg);

    Some(Style::default().bg(bg))
}

pub(super) fn paint_unified_diff_rows_with<F>(
    frame: &mut Frame,
    inner: Rect,
    visible_lines_unscrolled: &[Line],
    line_widths: &[usize],
    wrap_lines: bool,
    viewport_width: usize,
    style_for: F,
) where
    F: Fn(usize, &Line) -> Option<Style>,
{
    let mut visual_row: usize = 0;

    for (idx, line) in visible_lines_unscrolled.iter().enumerate() {
        if visual_row >= inner.height as usize {
            break;
        }

        let rows_for_line = if wrap_lines && viewport_width > 0 {
            let width = line_widths.get(idx).copied().unwrap_or(0);
            if width == 0 {
                1
            } else {
                width.div_ceil(viewport_width)
            }
        } else {
            1
        };

        if let Some(row_style) = style_for(idx, line) {
            for _ in 0..rows_for_line {
                if visual_row >= inner.height as usize {
                    break;
                }
                let row_rect = Rect {
                    x: inner.x,
                    y: inner.y + visual_row as u16,
                    width: inner.width,
                    height: 1,
                };
                frame.buffer_mut().set_style(row_rect, row_style);
                visual_row += 1;
            }
        } else {
            visual_row += rows_for_line;
        }
    }
}

/// Apply horizontal scroll to a line while preserving the first span (cursor indicator)
pub(super) fn apply_horizontal_scroll(line: Line, scroll_x: usize) -> Line {
    if scroll_x == 0 || line.spans.is_empty() {
        return line;
    }

    let mut spans: Vec<Span> = line.spans.into_iter().collect();

    // Preserve the first span (indicator)
    let indicator = spans.remove(0);

    // Skip scroll_x characters from the remaining spans
    let mut chars_to_skip = scroll_x;
    let mut new_spans = vec![indicator];

    for span in spans {
        let content = span.content.to_string();
        let char_count = content.chars().count();
        if chars_to_skip >= char_count {
            chars_to_skip -= char_count;
            // Skip this span entirely
        } else if chars_to_skip > 0 {
            // Partially skip this span
            let new_content: String = content.chars().skip(chars_to_skip).collect();
            chars_to_skip = 0;
            new_spans.push(Span::styled(new_content, span.style));
        } else {
            // Keep this span as-is
            new_spans.push(Span::styled(content, span.style));
        }
    }

    Line::from(new_spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_not_scroll_when_comment_box_already_visible() {
        // given: box at lines 5-7, viewport shows lines 0-9
        let mut scroll = 0;
        // when
        scroll_comment_input_into_view(&mut scroll, Some((5, 7)), Some(6), 10, 100);
        // then
        assert_eq!(scroll, 0);
    }

    #[test]
    fn should_scroll_down_when_comment_box_below_viewport() {
        // given: box at lines 20-22, viewport shows lines 0-9
        let mut scroll = 0;
        // when
        scroll_comment_input_into_view(&mut scroll, Some((20, 22)), Some(21), 10, 100);
        // then: scroll so box_end (22) is the last visible line => scroll = 22 - 10 + 1 = 13
        assert_eq!(scroll, 13);
    }

    #[test]
    fn should_scroll_up_when_comment_box_above_viewport() {
        // given: box at lines 5-7, viewport shows lines 20-29
        let mut scroll = 20;
        // when
        scroll_comment_input_into_view(&mut scroll, Some((5, 7)), Some(6), 10, 100);
        // then: scroll so box_start (5) is the first visible line
        assert_eq!(scroll, 5);
    }

    #[test]
    fn should_scroll_to_cursor_when_box_taller_than_viewport() {
        // given: box spans 20 lines, viewport only 10 lines
        let mut scroll = 0;
        // when
        scroll_comment_input_into_view(&mut scroll, Some((30, 49)), Some(45), 10, 100);
        // then: scroll so cursor (45) is the last visible line => scroll = 45 - 10 + 1 = 36
        assert_eq!(scroll, 36);
    }

    #[test]
    fn should_not_scroll_past_end_of_content() {
        // given: scroll already past max (e.g., content shrank)
        let mut scroll = 200;
        // when
        scroll_comment_input_into_view(&mut scroll, Some((95, 97)), Some(96), 10, 100);
        // then: clamped to max_scroll = 100 - 10 = 90
        assert_eq!(scroll, 90);
    }

    #[test]
    fn should_not_scroll_when_no_comment_box() {
        // given
        let mut scroll = 42;
        // when
        scroll_comment_input_into_view(&mut scroll, None, None, 10, 100);
        // then
        assert_eq!(scroll, 42);
    }

    #[test]
    fn should_handle_box_partially_below_viewport() {
        // given: viewport shows 0-9, box starts at 8 and ends at 10 (footer off-screen)
        let mut scroll = 0;
        // when
        scroll_comment_input_into_view(&mut scroll, Some((8, 10)), Some(9), 10, 100);
        // then: scroll so box_end (10) is visible => scroll = 10 - 10 + 1 = 1
        assert_eq!(scroll, 1);
    }
}
