use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};

use crate::app::{
    AnnotatedLine, App, DiffViewMode, ExpandDirection, GAP_EXPAND_BATCH, VisualSelection,
};
use crate::model::{Comment, DiffHunk, LineSide};
use crate::theme::Theme;
use crate::ui::comment_panel;
use crate::ui::diff_side_by_side::render_side_by_side_diff;
use crate::ui::diff_unified::render_unified_diff;
use crate::ui::styles;
use unicode_width::UnicodeWidthStr;

/// Static header rule used for file/section headers; avoids `"═".repeat(40)` per frame.
pub(super) const HEADER_RULE: &str = "════════════════════════════════════════";

/// Shared empty map so we can borrow `line_comments` without cloning per file per frame.
pub(super) static EMPTY_LINE_COMMENTS: std::sync::LazyLock<
    std::collections::HashMap<u32, Vec<Comment>>,
> = std::sync::LazyLock::new(std::collections::HashMap::new);

/// Compute the half-open `line_idx` range whose diff-line spans must be fully
/// built this frame. Outside this range the hot loops push `Line::default()`
/// placeholders so the bulk of per-line allocations are skipped.
///
/// In Comment mode the scroll offset may still be adjusted after building (to
/// keep the inline input box visible), so fall back to building everything.
pub(super) fn diff_visible_range(app: &App, inner: Rect) -> (usize, usize) {
    if app.input_mode == crate::app::InputMode::Comment {
        (0, usize::MAX)
    } else {
        let start = app.diff_state.scroll_offset;
        (start, start.saturating_add(inner.height as usize))
    }
}

pub(super) fn render_diff_view(frame: &mut Frame, app: &mut App, area: Rect) {
    match app.diff_view_mode {
        DiffViewMode::Unified => render_unified_diff(frame, app, area),
        DiffViewMode::SideBySide => render_side_by_side_diff(frame, app, area),
    }
}

/// Build the diff view's left title: ` <path> ` when a file is in view, or
/// ` Overview ` in overview mode. Long paths are prefix-truncated so the
/// most-informative tail (closest to the filename) survives.
pub(super) fn diff_title(app: &App, area_width: u16) -> String {
    if app.is_cursor_in_overview() || app.current_file_path().is_none() {
        return " Overview ".to_string();
    }
    let path = app
        .current_file_path()
        .map(|p| p.display().to_string())
        .unwrap_or_default();

    // Reserve room for the title's own spacing (` <path> `), the stats title
    // on the right, and the two corner glyphs. We deliberately under-reserve
    // — the precise stats width depends on the file's add/del counts, so the
    // worst case is a few-char overshoot that ratatui will truncate cleanly.
    let stats_reserve = 16; // ` +DDDD -DDDD ` plus padding
    let chrome_reserve = 4; // corners + leading/trailing space around title
    let max_path_width = (area_width as usize)
        .saturating_sub(stats_reserve + chrome_reserve)
        .max(8);

    format!(" {} ", truncate_path_smart(&path, max_path_width))
}

/// Truncate `path` to fit within `max_width` cells by dropping leading path
/// segments and prefixing with `…/`. Always keeps the basename intact; falls
/// back to suffix-truncating the basename only when the basename alone is
/// wider than `max_width`.
pub(super) fn truncate_path_smart(path: &str, max_width: usize) -> String {
    if path.chars().count() <= max_width {
        return path.to_string();
    }
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segments.is_empty() {
        return path.to_string();
    }

    for n in (1..segments.len()).rev() {
        let tail = segments[segments.len() - n..].join("/");
        let candidate = format!("\u{2026}/{tail}");
        if candidate.chars().count() <= max_width {
            return candidate;
        }
    }

    let basename = *segments.last().unwrap();
    let basename_w = basename.chars().count();
    if basename_w <= max_width {
        return basename.to_string();
    }
    // Basename alone is too wide: keep the leading chars + `…`.
    let kept = max_width.saturating_sub(1);
    let mut s: String = basename.chars().take(kept).collect();
    s.push('\u{2026}');
    s
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

pub(super) fn hunk_header_text_and_style(
    theme: &Theme,
    hunk: &DiffHunk,
    is_hunk_reviewed: bool,
) -> (String, Style) {
    if is_hunk_reviewed {
        (format!("✓ {}", hunk.header), styles::reviewed_style(theme))
    } else {
        (
            hunk.header.to_string(),
            styles::diff_hunk_header_style(theme),
        )
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
    row_heights: &[usize],
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
        for (i, &rows_for_line) in row_heights.iter().enumerate() {
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
        for i in 0..row_heights.len().min(viewport_height) {
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
    let prefix_span = line.spans.get(2)?;
    let default_bg = match prefix_span.style.fg {
        Some(fg) if fg == theme.diff_add => theme.diff_add_bg,
        Some(fg) if fg == theme.diff_del => theme.diff_del_bg,
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
    row_heights: &[usize],
    style_for: F,
) where
    F: Fn(usize, &Line) -> Option<Style>,
{
    let mut visual_row: usize = 0;

    for (idx, line) in visible_lines_unscrolled.iter().enumerate() {
        if visual_row >= inner.height as usize {
            break;
        }

        let rows_for_line = visual_rows_for_line(row_heights, idx);

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

#[derive(Clone, Copy)]
pub(super) enum CommentBoxRow {
    Top,
    Divider,
    Middle,
    Bottom,
}

/// Detect whether a logical diff line is part of an inline comment box and,
/// if so, which row of the box (top border, reply divider, content, or
/// bottom border). Inspected on unscrolled lines so we match the original
/// border-prefix span before any horizontal-scroll trimming.
///
/// Both `╭` (no line range) and `├` (line range present, bar joins from
/// above) appear at the prefix's corner slot; reply dividers in remote
/// threads also use `├`. We disambiguate Divider from Top by looking at the
/// next span's content — replies start with `↳`.
pub(super) fn comment_box_row(line: &Line) -> Option<CommentBoxRow> {
    let prefix = line.spans.get(1)?.content.as_ref();
    if prefix.starts_with("    ╭") {
        Some(CommentBoxRow::Top)
    } else if prefix.starts_with("    ├") {
        let next = line.spans.get(2).map(|s| s.content.as_ref()).unwrap_or("");
        if next.starts_with("↳") {
            Some(CommentBoxRow::Divider)
        } else {
            Some(CommentBoxRow::Top)
        }
    } else if prefix.starts_with("    │") {
        Some(CommentBoxRow::Middle)
    } else if prefix.starts_with("    ╰") {
        Some(CommentBoxRow::Bottom)
    } else {
        None
    }
}

pub(super) struct DiffOverlayPaint<'a> {
    pub inner: Rect,
    pub visible_lines_unscrolled: &'a [Line<'a>],
    pub line_widths: &'a [usize],
    /// Visual row count per logical line (word-wrap aware); see
    /// [`compute_row_heights`]. Kept alongside `line_widths`, which is still
    /// used for content-width math (right-border fill, horizontal scroll).
    pub row_heights: &'a [usize],
    pub wrap_lines: bool,
    pub viewport_width: usize,
    pub scroll_x: usize,
    pub scroll_offset: usize,
    pub theme: &'a Theme,
    pub comment_bars: &'a [CommentBarAnchor],
}

/// Records that an inline comment box at `box_top_row` (logical line index
/// in the full diff stream) covers a range `height` rows tall — the bar
/// painter extends `│` from `box_top_row - 1` up to `box_top_row - height`
/// and caps the topmost row with `╭`.
#[derive(Clone, Copy, Debug)]
pub(super) struct CommentBarAnchor {
    pub box_top_row: usize,
    pub height: usize,
}

/// Per-call-site helper: record a bar anchor if the comment has a line
/// range. No-op for file-level / review-level comments which don't anchor
/// to a covered line span.
pub(super) fn push_comment_bar(
    bars: &mut Vec<CommentBarAnchor>,
    box_top_row: usize,
    line_range: Option<crate::model::LineRange>,
) {
    if let Some(range) = line_range {
        bars.push(CommentBarAnchor {
            box_top_row,
            height: (range.end - range.start + 1) as usize,
        });
    }
}

/// Stamp the right-edge border glyph at the viewport's rightmost column for
/// each comment-box line so the box closes flush with the panel width. For
/// horizontal-rule rows (top/divider/bottom) the gap between the existing
/// dash content and the right corner is filled with `─` so the box reads as
/// one continuous frame. Runs after the paragraph and any cursor-line /
/// selection overlays so it always wins on the cells it writes.
pub(super) fn paint_comment_box_right_border(frame: &mut Frame, ctx: &DiffOverlayPaint) {
    if ctx.inner.width == 0 || ctx.viewport_width == 0 {
        return;
    }
    let bg = ctx.theme.panel_bg;
    let right_x = ctx.inner.x + ctx.inner.width - 1;
    let mut visual_row: usize = 0;
    for (idx, line) in ctx.visible_lines_unscrolled.iter().enumerate() {
        if visual_row >= ctx.inner.height as usize {
            break;
        }
        let rows = visual_rows_for_line(ctx.row_heights, idx);
        if let Some(pos) = comment_box_row(line) {
            let fg = line
                .spans
                .get(1)
                .and_then(|s| s.style.fg)
                .unwrap_or(ctx.theme.fg_primary);
            let line_width = ctx.line_widths.get(idx).copied().unwrap_or(0);

            for r in 0..rows {
                if visual_row + r >= ctx.inner.height as usize {
                    break;
                }
                let y = ctx.inner.y + (visual_row + r) as u16;
                let is_last_visual_row = r + 1 == rows;

                // For horizontal-rule rows the right border + dash fill only
                // belongs on the last visual row; the corner sits at the
                // logical line's true end. Middle rows close on every wrap
                // row since each is its own "content" line visually.
                let stamp_here = match pos {
                    CommentBoxRow::Top | CommentBoxRow::Divider | CommentBoxRow::Bottom => {
                        is_last_visual_row
                    }
                    CommentBoxRow::Middle => true,
                };
                if !stamp_here {
                    continue;
                }

                if matches!(
                    pos,
                    CommentBoxRow::Top | CommentBoxRow::Divider | CommentBoxRow::Bottom
                ) {
                    // Width of the visible content on this visual row.
                    let content_w = if ctx.wrap_lines {
                        let prior = r * ctx.viewport_width;
                        line_width.saturating_sub(prior).min(ctx.viewport_width)
                    } else {
                        line_width
                            .saturating_sub(ctx.scroll_x)
                            .min(ctx.viewport_width)
                    };
                    let fill_start = ctx.inner.x + content_w as u16;
                    let mut x = fill_start;
                    while x < right_x {
                        let cell = &mut frame.buffer_mut()[(x, y)];
                        cell.set_char('─');
                        cell.set_fg(fg);
                        cell.set_bg(bg);
                        x += 1;
                    }
                }

                let glyph = match pos {
                    CommentBoxRow::Top => '╮',
                    CommentBoxRow::Divider => '┤',
                    CommentBoxRow::Middle => '│',
                    CommentBoxRow::Bottom => '╯',
                };
                let cell = &mut frame.buffer_mut()[(right_x, y)];
                cell.set_char(glyph);
                cell.set_fg(fg);
                cell.set_bg(bg);
            }
        }
        visual_row += rows;
    }
}

/// Inline comment-box "bar" overlay: for each tracked comment box that has
/// a line range, draw a vertical bar at col 5 of the visible diff rows the
/// comment covers, capped with a `╭` at the topmost covered row. Painted
/// after the paragraph so the glyphs always win on their single cell.
pub(super) fn paint_comment_box_bar(frame: &mut Frame, ctx: &DiffOverlayPaint) {
    if ctx.inner.width == 0 || ctx.viewport_width == 0 || ctx.comment_bars.is_empty() {
        return;
    }
    if ctx.scroll_x > 4 {
        return;
    }
    let bar_screen_col = ctx.inner.x + 5 - ctx.scroll_x as u16;
    if bar_screen_col >= ctx.inner.x + ctx.inner.width {
        return;
    }
    // Bar style matches the box border (which is the file-header style),
    // so the bar reads as the same structural divider element.
    let style = styles::file_header_style(ctx.theme);
    let fg = style.fg.unwrap_or(ctx.theme.fg_primary);

    // Walk visible logical rows once, mapping each to its first visual row.
    let mut row_visual: Vec<(usize, u16)> = Vec::with_capacity(ctx.visible_lines_unscrolled.len());
    let mut visual_row: usize = 0;
    for (idx, _) in ctx.visible_lines_unscrolled.iter().enumerate() {
        if visual_row >= ctx.inner.height as usize {
            break;
        }
        let logical = ctx.scroll_offset + idx;
        row_visual.push((logical, ctx.inner.y + visual_row as u16));
        let rows = visual_rows_for_line(ctx.row_heights, idx);
        visual_row += rows;
    }

    for anchor in ctx.comment_bars {
        if anchor.height == 0 {
            continue;
        }
        let bar_top_logical = anchor.box_top_row.saturating_sub(anchor.height);
        // Bar covers logical rows [bar_top_logical, box_top_row - 1].
        for (logical, y) in &row_visual {
            if *logical >= anchor.box_top_row {
                break;
            }
            if *logical < bar_top_logical {
                continue;
            }
            let glyph = if *logical == bar_top_logical {
                '╭'
            } else {
                '│'
            };
            let cell = &mut frame.buffer_mut()[(bar_screen_col, *y)];
            cell.set_char(glyph);
            cell.set_fg(fg);
            if let Some(bg) = style.bg {
                cell.set_bg(bg);
            }
        }
    }
}

/// Hunk headers (`@@ … @@`), gap expanders (`... ↓ expand (N) ...`), and
/// hidden-line stubs (`... N lines hidden ...`) all read as structural
/// section markers in the diff stream — fill their row with a subtle bg
/// tint so they're easy to spot without using a loud accent colour. Painted
/// before the paragraph so cursor-line and selection overlays still win on
/// the active row.
pub(super) fn paint_section_highlight(frame: &mut Frame, ctx: &DiffOverlayPaint) {
    if ctx.inner.width == 0 || ctx.viewport_width == 0 {
        return;
    }
    let bg = ctx.theme.section_highlight_bg();
    let mut visual_row: usize = 0;
    for (idx, line) in ctx.visible_lines_unscrolled.iter().enumerate() {
        if visual_row >= ctx.inner.height as usize {
            break;
        }
        let rows = visual_rows_for_line(ctx.row_heights, idx);
        if is_section_highlight_line(line) {
            for r in 0..rows {
                if visual_row + r >= ctx.inner.height as usize {
                    break;
                }
                let row_rect = Rect {
                    x: ctx.inner.x,
                    y: ctx.inner.y + (visual_row + r) as u16,
                    width: ctx.inner.width,
                    height: 1,
                };
                frame
                    .buffer_mut()
                    .set_style(row_rect, Style::default().bg(bg));
            }
        }
        visual_row += rows;
    }
}

fn is_section_highlight_line(line: &Line) -> bool {
    let Some(content) = line.spans.get(1).map(|s| s.content.as_ref()) else {
        return false;
    };
    content.starts_with("@@") || content.starts_with("       ... ")
}

/// File-section header lines (the `═══ path/to/file [M] …══════` rows that
/// separate files in the diff stream) emit a fixed trailing run of `═`. Fill
/// any gap between that content and the right edge with `═` so the header
/// rule reads as one continuous bar across the viewport, regardless of how
/// wide the panel is.
pub(super) fn paint_file_header_fill(frame: &mut Frame, ctx: &DiffOverlayPaint) {
    if ctx.inner.width == 0 || ctx.viewport_width == 0 {
        return;
    }
    let panel_bg = ctx.theme.panel_bg;
    let right_x = ctx.inner.x + ctx.inner.width - 1;
    let mut visual_row: usize = 0;
    for (idx, line) in ctx.visible_lines_unscrolled.iter().enumerate() {
        if visual_row >= ctx.inner.height as usize {
            break;
        }
        let rows = visual_rows_for_line(ctx.row_heights, idx);
        if is_file_header_line(line) {
            let fg = line
                .spans
                .iter()
                .find(|s| s.content.starts_with('═'))
                .or_else(|| line.spans.get(1))
                .and_then(|s| s.style.fg)
                .unwrap_or(ctx.theme.fg_primary);
            let line_width = line
                .spans
                .iter()
                .map(|span| span.content.width())
                .sum::<usize>();
            // Only fill the trailing edge of the last visual row of the header
            // — wrapped intermediate rows of an unusually long header path are
            // already entirely covered by content.
            let last_row = rows.saturating_sub(1);
            if visual_row + last_row >= ctx.inner.height as usize {
                visual_row += rows;
                continue;
            }
            let y = ctx.inner.y + (visual_row + last_row) as u16;
            let content_w = if ctx.wrap_lines {
                let prior = last_row * ctx.viewport_width;
                line_width.saturating_sub(prior).min(ctx.viewport_width)
            } else {
                line_width
                    .saturating_sub(ctx.scroll_x)
                    .min(ctx.viewport_width)
            };
            let mut x = ctx.inner.x + content_w as u16;
            while x <= right_x {
                let cell = &mut frame.buffer_mut()[(x, y)];
                cell.set_char('═');
                cell.set_fg(fg);
                cell.set_bg(panel_bg);
                x += 1;
            }
        }
        visual_row += rows;
    }
}

/// A file-section header is a line whose first content span (after the
/// cursor indicator) begins with `═══ ` — covers both per-file headers and
/// the synthetic "Review Comments" section header.
fn is_file_header_line(line: &Line) -> bool {
    line.spans
        .get(1)
        .map(|s| s.content.starts_with("═══ "))
        .unwrap_or(false)
}

/// Number of visual (screen) rows each logical line occupies once rendered.
///
/// When wrapping is on this must match the `Paragraph`'s word wrap
/// (`Wrap { trim: false }`) exactly, so it reuses ratatui's own
/// [`Paragraph::line_count`]. A plain `div_ceil(width, viewport)` assumes
/// character packing and *under-counts* prose lines that word-wrap (a word
/// pushed to the next row leaves the first row short of the viewport width).
/// That under-count made every per-row background/overlay paint below such a
/// line drift upward relative to the rendered text.
///
/// Computed once per frame and shared by all row-mapping consumers.
pub(super) fn compute_row_heights(
    lines: &[Line],
    wrap_lines: bool,
    viewport_width: usize,
) -> Vec<usize> {
    if !wrap_lines || viewport_width == 0 {
        return vec![1; lines.len()];
    }
    let width = viewport_width as u16;
    lines
        .iter()
        .map(|line| {
            Paragraph::new(line.clone())
                .wrap(Wrap { trim: false })
                .line_count(width)
                .max(1)
        })
        .collect()
}

fn visual_rows_for_line(row_heights: &[usize], idx: usize) -> usize {
    row_heights.get(idx).copied().unwrap_or(1)
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

    /// Reference height straight from ratatui's renderer — the value
    /// `compute_row_heights` must reproduce for per-row paints to align.
    fn rendered_height(line: &Line, width: u16) -> usize {
        Paragraph::new(line.clone())
            .wrap(Wrap { trim: false })
            .line_count(width)
    }

    #[test]
    fn should_count_one_row_per_line_when_wrap_disabled() {
        let lines = vec![Line::from("a".repeat(200)), Line::from("short")];
        assert_eq!(compute_row_heights(&lines, false, 80), vec![1, 1]);
    }

    #[test]
    fn should_match_rendered_height_for_word_wrapped_prose() {
        // given: a prose line whose words can't be split, so word-wrap takes
        // more rows than character packing predicts.
        let line = Line::from(
            "PR review sessions are keyed by forge coordinates rather than a local checkout",
        );
        let width = 20usize;

        // then: our count matches what the Paragraph actually renders...
        let heights = compute_row_heights(std::slice::from_ref(&line), true, width);
        assert_eq!(heights[0], rendered_height(&line, width as u16));

        // ...and it exceeds the naive char-packing estimate that caused the
        // background/marker drift (regression guard).
        let total_width: usize = line.spans.iter().map(|s| s.content.width()).sum();
        assert!(
            heights[0] > total_width.div_ceil(width),
            "word wrap should take more rows than div_ceil for this prose line: \
             got {} vs div_ceil {}",
            heights[0],
            total_width.div_ceil(width),
        );
    }

    #[test]
    fn should_treat_empty_line_as_one_row() {
        let lines = vec![Line::from("")];
        assert_eq!(compute_row_heights(&lines, true, 80), vec![1]);
    }
}
