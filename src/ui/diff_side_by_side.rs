use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use unicode_width::UnicodeWidthStr;

use crate::app::{
    App, DiffSource, ExpandDirection, FocusedPanel, GAP_EXPAND_BATCH, GapId, InputMode,
};
use crate::model::{FileStatus, LineOrigin, LineRange, LineSide};
use crate::theme::Theme;
use crate::ui::comment_panel;
use crate::ui::diff_view::{
    apply_horizontal_scroll, comment_type_presentation, cursor_indicator, cursor_indicator_spaced,
    diff_stat_title, is_line_highlighted, paint_visual_selection_overlay,
    populate_row_to_annotation, render_expander_line, render_hidden_lines,
    scroll_comment_input_into_view,
};
use crate::ui::styles;
use crate::ui::text_utils::{truncate_or_pad, truncate_or_pad_spans};
use crate::vcs::git::calculate_gap;

/// Cursor info for the inline comment input box in side-by-side view:
/// (cursor_logical_line, cursor_column, box_start_line, box_end_line)
type SideBySideCursorInfo = (usize, u16, usize, usize, usize);

/// Context for rendering side-by-side diff lines
struct SideBySideContext<'a> {
    app: &'a App,
    theme: &'a Theme,
    content_width: usize,
    panel_width: usize,
    current_line_idx: usize,
    lineno_width: usize,
    // Comment input state for inline editing
    comment_input_mode: bool,
    comment_line: Option<(u32, LineSide)>,
    comment_type: crate::model::CommentType,
    comment_buffer: &'a str,
    comment_cursor: usize,
    comment_line_range: Option<LineRange>,
    editing_comment_id: Option<&'a str>,
    supports_keyboard_enhancement: bool,
    current_file_idx: usize,
    // RefCell so deeply-nested rendering helpers can push without each
    // intermediate function needing a `&mut Vec` parameter threaded through.
    comment_bars: std::cell::RefCell<Vec<crate::ui::diff_view::CommentBarAnchor>>,
}

pub(super) fn render_side_by_side_diff(frame: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focused_panel == FocusedPanel::Diff;

    let title = crate::ui::diff_view::diff_title(app, area.width);

    let block = Block::default()
        .title(title)
        .title_top(diff_stat_title(app).right_aligned())
        .borders(Borders::ALL)
        .style(styles::panel_style(&app.theme))
        .border_style(styles::border_style(&app.theme, focused));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Update viewport height for scroll calculations
    app.diff_state.viewport_height = inner.height as usize;
    app.diff_inner_area = Some(inner);

    // Reset comment input annotation offset (will be set if a comment input box is rendered)
    app.comment_input_annotation_offset = None;

    let lw = app.lineno_width();
    let available_width = inner.width.saturating_sub(crate::app::sbs_overhead(lw)) as usize;
    let content_width = available_width / 2;

    // Determine if we're in line comment mode (not file-level)
    let comment_input_mode = app.input_mode == InputMode::Comment
        && !app.comment_is_file_level
        && !app.comment_is_review_level;

    let ctx = SideBySideContext {
        app,
        theme: &app.theme,
        content_width,
        panel_width: inner.width as usize,
        current_line_idx: app.diff_state.cursor_line,
        lineno_width: lw,
        comment_input_mode,
        comment_line: app.comment_line,
        comment_type: app.comment_type.clone(),
        comment_buffer: &app.comment_buffer,
        comment_cursor: app.comment_cursor,
        comment_line_range: app.comment_line_range.map(|(r, _)| r),
        editing_comment_id: app.editing_comment_id.as_deref(),
        supports_keyboard_enhancement: app.supports_keyboard_enhancement,
        current_file_idx: app.diff_state.current_file_idx,
        comment_bars: std::cell::RefCell::new(Vec::new()),
    };

    // Build all diff lines for side-by-side view
    let mut lines: Vec<Line> = Vec::new();
    let mut line_idx: usize = 0;

    // Track cursor position for IME when in Comment mode
    let mut comment_cursor_logical_line: Option<usize> = None;
    let mut comment_cursor_column: u16 = 0;
    // Track the full extent of the comment input box so we can auto-scroll
    // the viewport to keep it visible while the user types.
    let mut comment_input_box_range: Option<(usize, usize)> = None;
    let mut annotation_offset: Option<(usize, usize, usize)> = None;

    let is_review_comment_mode =
        app.input_mode == InputMode::Comment && app.comment_is_review_level;

    let general_indicator = cursor_indicator_spaced(line_idx, ctx.current_line_idx);
    lines.push(Line::from(vec![
        Span::styled(
            general_indicator,
            styles::current_line_indicator_style(&app.theme),
        ),
        Span::styled(
            "═══ Review Comments ",
            styles::file_header_style(&app.theme),
        ),
        Span::styled("═".repeat(40), styles::file_header_style(&app.theme)),
    ]));
    line_idx += 1;

    for comment in &app.session.review_comments {
        let is_being_edited =
            app.editing_comment_id.as_ref() == Some(&comment.id) && is_review_comment_mode;

        if is_being_edited {
            let (input_lines, cursor_info) = comment_panel::format_comment_input_lines(
                &app.theme,
                comment_type_presentation(app, &app.comment_type),
                &app.comment_buffer,
                app.comment_cursor,
                None,
                true,
                app.supports_keyboard_enhancement,
                ctx.panel_width.saturating_sub(1),
            );
            comment_cursor_logical_line = Some(line_idx + cursor_info.line_offset);
            comment_cursor_column = 1 + cursor_info.column;
            comment_input_box_range =
                Some((line_idx, line_idx + input_lines.len().saturating_sub(1)));
            let annotations_replaced = App::comment_display_lines(comment, inner.width as usize);
            annotation_offset = Some((line_idx, input_lines.len(), annotations_replaced));

            for mut input_line in input_lines {
                let indicator = cursor_indicator(line_idx, ctx.current_line_idx);
                input_line.spans.insert(
                    0,
                    Span::styled(indicator, styles::current_line_indicator_style(&app.theme)),
                );
                lines.push(input_line);
                line_idx += 1;
            }
        } else {
            let comment_lines = comment_panel::format_comment_lines(
                &app.theme,
                comment_type_presentation(app, &comment.comment_type),
                &comment.content,
                None,
                ctx.panel_width.saturating_sub(1),
            );
            for mut comment_line in comment_lines {
                let indicator = cursor_indicator(line_idx, ctx.current_line_idx);
                comment_line.spans.insert(
                    0,
                    Span::styled(indicator, styles::current_line_indicator_style(&app.theme)),
                );
                lines.push(comment_line);
                line_idx += 1;
            }
        }
    }

    if is_review_comment_mode && app.editing_comment_id.is_none() {
        let (input_lines, cursor_info) = comment_panel::format_comment_input_lines(
            &app.theme,
            comment_type_presentation(app, &app.comment_type),
            &app.comment_buffer,
            app.comment_cursor,
            None,
            false,
            app.supports_keyboard_enhancement,
            ctx.panel_width.saturating_sub(1),
        );
        comment_cursor_logical_line = Some(line_idx + cursor_info.line_offset);
        comment_cursor_column = 1 + cursor_info.column;
        comment_input_box_range = Some((line_idx, line_idx + input_lines.len().saturating_sub(1)));
        annotation_offset = Some((line_idx, input_lines.len(), 0));

        for mut input_line in input_lines {
            let indicator = cursor_indicator(line_idx, ctx.current_line_idx);
            input_line.spans.insert(
                0,
                Span::styled(indicator, styles::current_line_indicator_style(&app.theme)),
            );
            lines.push(input_line);
            line_idx += 1;
        }
    }

    for (file_idx, file) in app.diff_files.iter().enumerate() {
        let path = file.display_path();
        let status = file.status.as_char();
        let is_reviewed = app.session.is_file_reviewed(path);

        // File header
        let indicator = cursor_indicator_spaced(line_idx, ctx.current_line_idx);

        let review_mark = if is_reviewed { "✓ " } else { "" };

        let header_text = if file.is_commit_message {
            format!("═══ {}Commit Message ", review_mark)
        } else {
            format!("═══ {}{} [{}] ", review_mark, path.display(), status)
        };
        lines.push(Line::from(vec![
            Span::styled(indicator, styles::current_line_indicator_style(&app.theme)),
            Span::styled(header_text, styles::file_header_style(&app.theme)),
            Span::styled("═".repeat(40), styles::file_header_style(&app.theme)),
        ]));
        line_idx += 1;

        // If file is reviewed, skip rendering the body
        if is_reviewed {
            continue;
        }

        // Check if we're editing/adding a file-level comment for this file
        let is_file_comment_mode = app.input_mode == InputMode::Comment
            && app.comment_is_file_level
            && file_idx == app.diff_state.current_file_idx;

        // Show file-level comments
        if let Some(review) = app.session.files.get(path) {
            for comment in &review.file_comments {
                // Skip rendering this comment if it's being edited
                let is_being_edited =
                    app.editing_comment_id.as_ref() == Some(&comment.id) && is_file_comment_mode;

                if is_being_edited {
                    // Render the inline input instead
                    let (input_lines, cursor_info) = comment_panel::format_comment_input_lines(
                        &app.theme,
                        comment_type_presentation(app, &app.comment_type),
                        &app.comment_buffer,
                        app.comment_cursor,
                        None,
                        true,
                        app.supports_keyboard_enhancement,
                        ctx.panel_width.saturating_sub(1),
                    );
                    comment_cursor_logical_line = Some(line_idx + cursor_info.line_offset);
                    comment_cursor_column = 1 + cursor_info.column;
                    comment_input_box_range =
                        Some((line_idx, line_idx + input_lines.len().saturating_sub(1)));
                    let annotations_replaced =
                        App::comment_display_lines(comment, inner.width as usize);
                    annotation_offset = Some((line_idx, input_lines.len(), annotations_replaced));

                    for mut input_line in input_lines {
                        let indicator = cursor_indicator(line_idx, ctx.current_line_idx);
                        input_line.spans.insert(
                            0,
                            Span::styled(
                                indicator,
                                styles::current_line_indicator_style(&app.theme),
                            ),
                        );
                        lines.push(input_line);
                        line_idx += 1;
                    }
                } else {
                    let comment_lines = comment_panel::format_comment_lines(
                        &app.theme,
                        comment_type_presentation(app, &comment.comment_type),
                        &comment.content,
                        None,
                        ctx.panel_width.saturating_sub(1),
                    );
                    for mut comment_line in comment_lines {
                        let indicator = cursor_indicator(line_idx, ctx.current_line_idx);
                        comment_line.spans.insert(
                            0,
                            Span::styled(
                                indicator,
                                styles::current_line_indicator_style(&app.theme),
                            ),
                        );
                        lines.push(comment_line);
                        line_idx += 1;
                    }
                }
            }
        }

        // Render inline input for new file-level comment
        if is_file_comment_mode && app.editing_comment_id.is_none() {
            let (input_lines, cursor_info) = comment_panel::format_comment_input_lines(
                &app.theme,
                comment_type_presentation(app, &app.comment_type),
                &app.comment_buffer,
                app.comment_cursor,
                None,
                false,
                app.supports_keyboard_enhancement,
                ctx.panel_width.saturating_sub(1),
            );
            comment_cursor_logical_line = Some(line_idx + cursor_info.line_offset);
            comment_cursor_column = 1 + cursor_info.column;
            comment_input_box_range =
                Some((line_idx, line_idx + input_lines.len().saturating_sub(1)));
            annotation_offset = Some((line_idx, input_lines.len(), 0));

            for mut input_line in input_lines {
                let indicator = cursor_indicator(line_idx, ctx.current_line_idx);
                input_line.spans.insert(
                    0,
                    Span::styled(indicator, styles::current_line_indicator_style(&app.theme)),
                );
                lines.push(input_line);
                line_idx += 1;
            }
        }

        if file.is_too_large {
            let indicator = cursor_indicator_spaced(line_idx, ctx.current_line_idx);
            lines.push(Line::from(vec![
                Span::styled(indicator, styles::current_line_indicator_style(&app.theme)),
                Span::styled("(file too large to display)", styles::dim_style(&app.theme)),
            ]));
            line_idx += 1;
        } else if file.is_binary {
            let indicator = cursor_indicator_spaced(line_idx, ctx.current_line_idx);
            lines.push(Line::from(vec![
                Span::styled(indicator, styles::current_line_indicator_style(&app.theme)),
                Span::styled("(binary file)", styles::dim_style(&app.theme)),
            ]));
            line_idx += 1;
        } else if file.hunks.is_empty() {
            let indicator = cursor_indicator_spaced(line_idx, ctx.current_line_idx);
            lines.push(Line::from(vec![
                Span::styled(indicator, styles::current_line_indicator_style(&app.theme)),
                Span::styled("(no changes)", styles::dim_style(&app.theme)),
            ]));
            line_idx += 1;
        } else {
            let line_comments = app
                .session
                .files
                .get(path)
                .map(|r| &r.line_comments)
                .cloned()
                .unwrap_or_default();

            for (hunk_idx, hunk) in file.hunks.iter().enumerate() {
                // Calculate and render gap before this hunk
                let prev_hunk = if hunk_idx > 0 {
                    file.hunks.get(hunk_idx - 1)
                } else {
                    None
                };
                let gap = calculate_gap(
                    prev_hunk.map(|h| (&h.new_start, &h.new_count)),
                    hunk.new_start,
                );

                let gap_id = GapId { file_idx, hunk_idx };

                if gap > 0 {
                    let top_lines = app.expanded_top.get(&gap_id);
                    let bot_lines = app.expanded_bottom.get(&gap_id);
                    let top_len = top_lines.map_or(0, |v| v.len());
                    let bot_len = bot_lines.map_or(0, |v| v.len());
                    let remaining = (gap as usize).saturating_sub(top_len + bot_len);
                    let is_top_of_file = hunk_idx == 0;

                    // Render top expanded lines
                    if let Some(top) = top_lines {
                        for expanded_line in top {
                            render_sbs_expanded_context_line(
                                &mut lines,
                                &mut line_idx,
                                ctx.current_line_idx,
                                expanded_line,
                                ctx.content_width,
                                &app.theme,
                                lw,
                            );
                        }
                    }

                    // Render expanders / hidden lines
                    if remaining > 0 {
                        if is_top_of_file {
                            if remaining > GAP_EXPAND_BATCH {
                                render_hidden_lines(
                                    &mut lines,
                                    &mut line_idx,
                                    ctx.current_line_idx,
                                    remaining,
                                    &app.theme,
                                );
                            }
                            render_expander_line(
                                &mut lines,
                                &mut line_idx,
                                ctx.current_line_idx,
                                ExpandDirection::Up,
                                remaining,
                                &app.theme,
                            );
                        } else if remaining >= GAP_EXPAND_BATCH {
                            render_expander_line(
                                &mut lines,
                                &mut line_idx,
                                ctx.current_line_idx,
                                ExpandDirection::Down,
                                remaining,
                                &app.theme,
                            );
                            render_hidden_lines(
                                &mut lines,
                                &mut line_idx,
                                ctx.current_line_idx,
                                remaining,
                                &app.theme,
                            );
                            render_expander_line(
                                &mut lines,
                                &mut line_idx,
                                ctx.current_line_idx,
                                ExpandDirection::Up,
                                remaining,
                                &app.theme,
                            );
                        } else {
                            render_expander_line(
                                &mut lines,
                                &mut line_idx,
                                ctx.current_line_idx,
                                ExpandDirection::Both,
                                remaining,
                                &app.theme,
                            );
                        }
                    }

                    // Render bottom expanded lines
                    if let Some(bot) = bot_lines {
                        for expanded_line in bot {
                            render_sbs_expanded_context_line(
                                &mut lines,
                                &mut line_idx,
                                ctx.current_line_idx,
                                expanded_line,
                                ctx.content_width,
                                &app.theme,
                                lw,
                            );
                        }
                    }
                }

                // Hunk header
                let indicator = cursor_indicator_spaced(line_idx, ctx.current_line_idx);
                lines.push(Line::from(vec![
                    Span::styled(indicator, styles::current_line_indicator_style(&app.theme)),
                    Span::styled(
                        hunk.header.to_string(),
                        styles::diff_hunk_header_style(&app.theme),
                    ),
                ]));
                line_idx += 1;

                // Process diff lines in side-by-side format
                let (new_line_idx, cursor_info) = render_hunk_lines_side_by_side(
                    &hunk.lines,
                    &line_comments,
                    &ctx,
                    file_idx,
                    line_idx,
                    &mut lines,
                );
                line_idx = new_line_idx;
                if let Some((line, col, box_start, box_end, annotations_replaced)) = cursor_info {
                    comment_cursor_logical_line = Some(line);
                    comment_cursor_column = col;
                    comment_input_box_range = Some((box_start, box_end));
                    let box_len = box_end - box_start + 1;
                    annotation_offset = Some((box_start, box_len, annotations_replaced));
                }
            }
        }

        // End-of-file gap (after all hunks, not for deleted files)
        if file.status != FileStatus::Deleted
            && matches!(
                app.diff_source,
                DiffSource::WorkingTree
                    | DiffSource::Unstaged
                    | DiffSource::StagedAndUnstaged
                    | DiffSource::StagedUnstagedAndCommits(_)
                    | DiffSource::CommitRange(_)
            )
            && let Some(last_hunk) = file.hunks.last()
        {
            let eof_start = last_hunk.new_start + last_hunk.new_count;
            if let Some(&total) = app.file_line_count_cache.get(&file_idx)
                && eof_start <= total
            {
                let gap = (total - eof_start + 1) as usize;
                let eof_gap_id = GapId {
                    file_idx,
                    hunk_idx: file.hunks.len(),
                };
                let top_lines = app.expanded_top.get(&eof_gap_id);
                let bot_lines = app.expanded_bottom.get(&eof_gap_id);
                let top_len = top_lines.map_or(0, |v| v.len());
                let bot_len = bot_lines.map_or(0, |v| v.len());
                let remaining = gap.saturating_sub(top_len + bot_len);

                // Render top expanded lines (↓ direction)
                if let Some(top) = top_lines {
                    for expanded_line in top {
                        render_sbs_expanded_context_line(
                            &mut lines,
                            &mut line_idx,
                            ctx.current_line_idx,
                            expanded_line,
                            ctx.content_width,
                            &app.theme,
                            lw,
                        );
                    }
                }

                // Expander / hidden lines
                if remaining > 0 {
                    render_expander_line(
                        &mut lines,
                        &mut line_idx,
                        ctx.current_line_idx,
                        ExpandDirection::Down,
                        remaining,
                        &app.theme,
                    );
                    if remaining > GAP_EXPAND_BATCH {
                        render_hidden_lines(
                            &mut lines,
                            &mut line_idx,
                            ctx.current_line_idx,
                            remaining,
                            &app.theme,
                        );
                    }
                }

                // Render bottom expanded lines
                if let Some(bot) = bot_lines {
                    for expanded_line in bot {
                        render_sbs_expanded_context_line(
                            &mut lines,
                            &mut line_idx,
                            ctx.current_line_idx,
                            expanded_line,
                            ctx.content_width,
                            &app.theme,
                            lw,
                        );
                    }
                }
            }
        }

        // Spacing between files
        let indicator = cursor_indicator(line_idx, ctx.current_line_idx);
        lines.push(Line::from(Span::styled(
            indicator,
            styles::current_line_indicator_style(&app.theme),
        )));
        line_idx += 1;
    }

    let comment_bars = {
        let mut bars = ctx.comment_bars.borrow_mut();
        std::mem::take(&mut *bars)
    };
    drop(ctx);
    app.comment_input_annotation_offset = annotation_offset;

    // Auto-scroll so the comment input box stays visible while the user types.
    scroll_comment_input_into_view(
        &mut app.diff_state.scroll_offset,
        comment_input_box_range,
        comment_cursor_logical_line,
        inner.height as usize,
        lines.len(),
    );

    let visible_lines_unscrolled: Vec<Line> = lines
        .into_iter()
        .skip(app.diff_state.scroll_offset)
        .take(inner.height as usize)
        .collect();

    // Calculate the width of each line for max_content_width and visible line count
    let line_widths: Vec<usize> = visible_lines_unscrolled
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.width())
                .sum::<usize>()
        })
        .collect();

    let max_content_width = line_widths.iter().copied().max().unwrap_or(0);

    app.sync_viewport_width(inner.width as usize);
    app.diff_state.max_content_width = max_content_width;

    let scroll_offset = app.diff_state.scroll_offset;
    let wrap = app.diff_state.wrap_lines;
    app.diff_state.visible_line_count = populate_row_to_annotation(
        &mut app.diff_row_to_annotation,
        &line_widths,
        inner.width as usize,
        inner.height as usize,
        wrap,
        scroll_offset,
    );

    let max_scroll_x = max_content_width.saturating_sub(inner.width as usize);
    if app.diff_state.scroll_x > max_scroll_x {
        app.diff_state.scroll_x = max_scroll_x;
    }
    if app.diff_state.wrap_lines {
        app.diff_state.scroll_x = 0;
    }

    let scroll_x = app.diff_state.scroll_x;
    let visible_lines_unscrolled_for_overlay = visible_lines_unscrolled.clone();
    let visible_lines: Vec<Line> = if app.diff_state.wrap_lines {
        visible_lines_unscrolled
    } else {
        visible_lines_unscrolled
            .into_iter()
            .map(|line| apply_horizontal_scroll(line, scroll_x))
            .collect()
    };

    let overlay_ctx = crate::ui::diff_view::DiffOverlayPaint {
        inner,
        visible_lines_unscrolled: &visible_lines_unscrolled_for_overlay,
        line_widths: &line_widths,
        wrap_lines: app.diff_state.wrap_lines,
        viewport_width: inner.width as usize,
        scroll_x,
        scroll_offset: app.diff_state.scroll_offset,
        theme: &app.theme,
        comment_bars: &comment_bars,
    };

    // Section-marker row tint (hunk headers + expand/hidden stubs).
    crate::ui::diff_view::paint_section_highlight(frame, &overlay_ctx);

    let mut diff = Paragraph::new(visible_lines).style(styles::panel_style(&app.theme));
    if app.diff_state.wrap_lines {
        diff = diff.wrap(Wrap { trim: false });
    }
    frame.render_widget(diff, inner);

    if app.cursor_line_highlight {
        let viewport_height = inner.height as usize;
        for offset in 0..viewport_height {
            if is_line_highlighted(app, offset) {
                let row_rect = Rect {
                    x: inner.x,
                    y: inner.y + offset as u16,
                    width: inner.width,
                    height: 1,
                };
                frame
                    .buffer_mut()
                    .set_style(row_rect, Style::default().bg(app.theme.cursor_line_bg));
            }
        }
    }

    // Painted last so the cell overlay wins over cursor-line bg on overlap.
    if let Some(sel) = app.visual_selection {
        paint_visual_selection_overlay(frame, inner, app, sel, &app.theme);
    }

    // File-section header rules extended to the full viewport width.
    crate::ui::diff_view::paint_file_header_fill(frame, &overlay_ctx);

    // Comment-box overlays painted last so the box + bar always win on their
    // single cells.
    crate::ui::diff_view::paint_comment_box_bar(frame, &overlay_ctx);
    crate::ui::diff_view::paint_comment_box_right_border(frame, &overlay_ctx);

    // Calculate screen position for comment cursor if in Comment mode
    if let Some(cursor_logical_line) = comment_cursor_logical_line {
        let scroll_offset = app.diff_state.scroll_offset;
        let visible_lines_count = app.diff_state.visible_line_count.max(1);

        // Check if the cursor line is visible (after scrolling)
        if cursor_logical_line >= scroll_offset
            && cursor_logical_line < scroll_offset + visible_lines_count
        {
            // Calculate screen row - need to account for wrapping
            let logical_offset = cursor_logical_line - scroll_offset;

            let mut visual_row: u16 = 0;
            let viewport_width = inner.width as usize;

            if app.diff_state.wrap_lines && viewport_width > 0 {
                for i in 0..logical_offset {
                    if i < line_widths.len() {
                        let width = line_widths[i];
                        let rows = if width == 0 {
                            1
                        } else {
                            width.div_ceil(viewport_width)
                        };
                        visual_row += rows as u16;
                    } else {
                        visual_row += 1;
                    }
                }
            } else {
                visual_row = logical_offset as u16;
            }

            let screen_col = inner.x + comment_cursor_column;
            let screen_row_abs = inner.y + visual_row;

            app.comment_cursor_screen_pos = Some((screen_col, screen_row_abs));
        }
    }
}

/// Render a single expanded context line in side-by-side mode
fn render_sbs_expanded_context_line(
    lines: &mut Vec<Line<'_>>,
    line_idx: &mut usize,
    current_line_idx: usize,
    expanded_line: &crate::model::DiffLine,
    content_width: usize,
    theme: &Theme,
    lw: usize,
) {
    let indicator = cursor_indicator(*line_idx, current_line_idx);
    let old_line_num = expanded_line
        .old_lineno
        .map(|n| format!("{n:>lw$} "))
        .unwrap_or_else(|| " ".repeat(lw + 1));
    let new_line_num = expanded_line
        .new_lineno
        .map(|n| format!("{n:>lw$} "))
        .unwrap_or_else(|| " ".repeat(lw + 1));
    let line_spans = vec![
        Span::styled(indicator, styles::current_line_indicator_style(theme)),
        Span::styled(old_line_num, styles::expanded_context_style(theme)),
        Span::styled(" ", styles::expanded_context_style(theme)),
        Span::styled(
            truncate_or_pad(&expanded_line.content, content_width),
            styles::expanded_context_style(theme),
        ),
        Span::styled(" │ ", styles::dim_style(theme)),
        Span::styled(new_line_num, styles::expanded_context_style(theme)),
        Span::styled(" ", styles::expanded_context_style(theme)),
        Span::styled(
            truncate_or_pad(&expanded_line.content, content_width),
            styles::expanded_context_style(theme),
        ),
    ];
    lines.push(Line::from(line_spans));
    *line_idx += 1;
}

/// Process and render all diff lines in a hunk for side-by-side view
/// Returns (new_line_idx, optional cursor info for inline comment input)
fn render_hunk_lines_side_by_side(
    hunk_lines: &[crate::model::DiffLine],
    line_comments: &std::collections::HashMap<u32, Vec<crate::model::Comment>>,
    ctx: &SideBySideContext,
    file_idx: usize,
    mut line_idx: usize,
    lines: &mut Vec<Line>,
) -> (usize, Option<SideBySideCursorInfo>) {
    let mut i = 0;
    let mut cursor_info_out: Option<SideBySideCursorInfo> = None;

    while i < hunk_lines.len() {
        let diff_line = &hunk_lines[i];

        match diff_line.origin {
            LineOrigin::Context => {
                let (new_line_idx, cursor_info) = render_context_line_side_by_side(
                    diff_line,
                    line_comments,
                    ctx,
                    file_idx,
                    line_idx,
                    lines,
                );
                line_idx = new_line_idx;
                if cursor_info.is_some() {
                    cursor_info_out = cursor_info;
                }
                i += 1;
            }
            LineOrigin::Deletion => {
                let (new_line_idx, lines_processed, cursor_info) =
                    render_deletion_addition_pair_side_by_side(
                        hunk_lines,
                        i,
                        line_comments,
                        ctx,
                        file_idx,
                        line_idx,
                        lines,
                    );
                line_idx = new_line_idx;
                if cursor_info.is_some() {
                    cursor_info_out = cursor_info;
                }
                i = lines_processed;
            }
            LineOrigin::Addition => {
                let (new_line_idx, cursor_info) = render_standalone_addition_side_by_side(
                    diff_line,
                    line_comments,
                    ctx,
                    file_idx,
                    line_idx,
                    lines,
                );
                line_idx = new_line_idx;
                if cursor_info.is_some() {
                    cursor_info_out = cursor_info;
                }
                i += 1;
            }
        }
    }
    (line_idx, cursor_info_out)
}

/// Render a context line (appears on both sides)
/// Returns (new_line_idx, optional cursor info for inline comment input)
fn render_context_line_side_by_side(
    diff_line: &crate::model::DiffLine,
    line_comments: &std::collections::HashMap<u32, Vec<crate::model::Comment>>,
    ctx: &SideBySideContext,
    file_idx: usize,
    mut line_idx: usize,
    lines: &mut Vec<Line>,
) -> (usize, Option<SideBySideCursorInfo>) {
    let w = ctx.lineno_width;
    let old_line_num = diff_line
        .old_lineno
        .map(|n| format!("{n:>w$}"))
        .unwrap_or_else(|| " ".repeat(w));
    let new_line_num = diff_line
        .new_lineno
        .map(|n| format!("{n:>w$}"))
        .unwrap_or_else(|| " ".repeat(w));

    let indicator = cursor_indicator(line_idx, ctx.current_line_idx);

    let mut spans = vec![
        Span::styled(indicator, styles::current_line_indicator_style(ctx.theme)),
        Span::styled(format!("{old_line_num} "), styles::dim_style(ctx.theme)),
        Span::styled(" ".to_string(), styles::diff_context_style(ctx.theme)),
    ];

    // Left side content - use syntax highlighting if available
    if let Some(ref highlighted) = diff_line.highlighted_spans {
        let content_spans = truncate_or_pad_spans(
            highlighted,
            ctx.content_width,
            styles::diff_context_style(ctx.theme),
        );
        spans.extend(content_spans);
    } else {
        let content = truncate_or_pad(&diff_line.content, ctx.content_width);
        spans.push(Span::styled(content, styles::diff_context_style(ctx.theme)));
    }

    // Separator
    spans.push(Span::styled(" │ ", styles::dim_style(ctx.theme)));
    spans.push(Span::styled(
        format!("{new_line_num} "),
        styles::dim_style(ctx.theme),
    ));
    spans.push(Span::styled(
        " ".to_string(),
        styles::diff_context_style(ctx.theme),
    ));

    // Right side content - use same highlighting
    if let Some(ref highlighted) = diff_line.highlighted_spans {
        let content_spans = truncate_or_pad_spans(
            highlighted,
            ctx.content_width,
            styles::diff_context_style(ctx.theme),
        );
        spans.extend(content_spans);
    } else {
        let content = truncate_or_pad(&diff_line.content, ctx.content_width);
        spans.push(Span::styled(content, styles::diff_context_style(ctx.theme)));
    }

    lines.push(Line::from(spans));
    line_idx += 1;

    // Add comments if any
    let mut cursor_info_out: Option<SideBySideCursorInfo> = None;
    if let Some(new_ln) = diff_line.new_lineno {
        let (new_line_idx, cursor_info) = add_comments_to_line(
            new_ln,
            line_comments,
            LineSide::New,
            ctx,
            file_idx,
            line_idx,
            lines,
        );
        line_idx = new_line_idx;
        cursor_info_out = cursor_info;
        if let Some(file) = ctx.app.diff_files.get(file_idx) {
            line_idx = add_remote_threads_to_line(
                new_ln,
                LineSide::New,
                ctx,
                file.display_path(),
                line_idx,
                lines,
            );
        }
    }

    (line_idx, cursor_info_out)
}

/// Render paired deletions and additions side-by-side
/// Returns (line_idx, skip_count, optional cursor info for inline comment input)
fn render_deletion_addition_pair_side_by_side(
    hunk_lines: &[crate::model::DiffLine],
    start_idx: usize,
    line_comments: &std::collections::HashMap<u32, Vec<crate::model::Comment>>,
    ctx: &SideBySideContext,
    file_idx: usize,
    mut line_idx: usize,
    lines: &mut Vec<Line>,
) -> (usize, usize, Option<SideBySideCursorInfo>) {
    // Find the range of consecutive deletions
    let mut del_end = start_idx + 1;
    while del_end < hunk_lines.len() && hunk_lines[del_end].origin == LineOrigin::Deletion {
        del_end += 1;
    }

    // Find the range of consecutive additions following the deletions
    let add_start = del_end;
    let mut add_end = add_start;
    while add_end < hunk_lines.len() && hunk_lines[add_end].origin == LineOrigin::Addition {
        add_end += 1;
    }

    let del_count = del_end - start_idx;
    let add_count = add_end - add_start;
    let max_lines = del_count.max(add_count);
    let mut cursor_info_out: Option<SideBySideCursorInfo> = None;

    // Render each pair of deletion/addition
    for offset in 0..max_lines {
        let indicator = cursor_indicator(line_idx, ctx.current_line_idx);

        let mut spans = vec![Span::styled(
            indicator,
            styles::current_line_indicator_style(ctx.theme),
        )];

        // Left side (deletion)
        if offset < del_count {
            let del_line = &hunk_lines[start_idx + offset];
            add_deletion_spans(
                ctx.theme,
                &mut spans,
                del_line,
                ctx.content_width,
                ctx.lineno_width,
            );
        } else {
            add_empty_column_spans(&mut spans, ctx.content_width, ctx.lineno_width);
        }

        spans.push(Span::styled(" │ ", styles::dim_style(ctx.theme)));

        // Right side (addition)
        if offset < add_count {
            let add_line = &hunk_lines[add_start + offset];
            add_addition_spans(
                ctx.theme,
                &mut spans,
                add_line,
                ctx.content_width,
                ctx.lineno_width,
            );
        } else {
            add_empty_column_spans(&mut spans, ctx.content_width, ctx.lineno_width);
        }

        lines.push(Line::from(spans));
        line_idx += 1;

        // Add comments for deletion
        if offset < del_count {
            let del_line = &hunk_lines[start_idx + offset];
            if let Some(old_ln) = del_line.old_lineno {
                let (new_line_idx, cursor_info) = add_comments_to_line(
                    old_ln,
                    line_comments,
                    LineSide::Old,
                    ctx,
                    file_idx,
                    line_idx,
                    lines,
                );
                line_idx = new_line_idx;
                if cursor_info.is_some() {
                    cursor_info_out = cursor_info;
                }
                if let Some(file) = ctx.app.diff_files.get(file_idx) {
                    line_idx = add_remote_threads_to_line(
                        old_ln,
                        LineSide::Old,
                        ctx,
                        file.display_path(),
                        line_idx,
                        lines,
                    );
                }
            }
        }

        // Add comments for addition
        if offset < add_count {
            let add_line = &hunk_lines[add_start + offset];
            if let Some(new_ln) = add_line.new_lineno {
                let (new_line_idx, cursor_info) = add_comments_to_line(
                    new_ln,
                    line_comments,
                    LineSide::New,
                    ctx,
                    file_idx,
                    line_idx,
                    lines,
                );
                line_idx = new_line_idx;
                if cursor_info.is_some() {
                    cursor_info_out = cursor_info;
                }
                if let Some(file) = ctx.app.diff_files.get(file_idx) {
                    line_idx = add_remote_threads_to_line(
                        new_ln,
                        LineSide::New,
                        ctx,
                        file.display_path(),
                        line_idx,
                        lines,
                    );
                }
            }
        }
    }

    (line_idx, add_end, cursor_info_out)
}

/// Render a standalone addition (no matching deletion)
/// Returns (new_line_idx, optional cursor info for inline comment input)
fn render_standalone_addition_side_by_side(
    diff_line: &crate::model::DiffLine,
    line_comments: &std::collections::HashMap<u32, Vec<crate::model::Comment>>,
    ctx: &SideBySideContext,
    file_idx: usize,
    mut line_idx: usize,
    lines: &mut Vec<Line>,
) -> (usize, Option<SideBySideCursorInfo>) {
    let indicator = cursor_indicator(line_idx, ctx.current_line_idx);

    let mut spans = vec![Span::styled(
        indicator,
        styles::current_line_indicator_style(ctx.theme),
    )];
    add_empty_column_spans(&mut spans, ctx.content_width, ctx.lineno_width);
    spans.push(Span::styled(" │ ", styles::dim_style(ctx.theme)));
    add_addition_spans(
        ctx.theme,
        &mut spans,
        diff_line,
        ctx.content_width,
        ctx.lineno_width,
    );

    lines.push(Line::from(spans));
    line_idx += 1;

    // Add comments if any
    let mut cursor_info_out: Option<SideBySideCursorInfo> = None;
    if let Some(new_ln) = diff_line.new_lineno {
        let (new_line_idx, cursor_info) = add_comments_to_line(
            new_ln,
            line_comments,
            LineSide::New,
            ctx,
            file_idx,
            line_idx,
            lines,
        );
        line_idx = new_line_idx;
        cursor_info_out = cursor_info;
        if let Some(file) = ctx.app.diff_files.get(file_idx) {
            line_idx = add_remote_threads_to_line(
                new_ln,
                LineSide::New,
                ctx,
                file.display_path(),
                line_idx,
                lines,
            );
        }
    }

    (line_idx, cursor_info_out)
}

/// Add deletion line spans to the spans vector
fn add_deletion_spans(
    theme: &Theme,
    spans: &mut Vec<Span>,
    diff_line: &crate::model::DiffLine,
    content_width: usize,
    lw: usize,
) {
    let line_num = diff_line
        .old_lineno
        .map(|n| format!("{n:>lw$}"))
        .unwrap_or_else(|| " ".repeat(lw));

    spans.push(Span::styled(
        format!("{line_num} "),
        styles::dim_style(theme),
    ));
    spans.push(Span::styled("▌".to_string(), styles::diff_del_style(theme)));

    // Use syntax highlighting if available
    if let Some(ref highlighted) = diff_line.highlighted_spans {
        let syntax_pad_style = Style::default().fg(theme.diff_del).bg(theme.syntax_del_bg);
        let content_spans = truncate_or_pad_spans(highlighted, content_width, syntax_pad_style);
        spans.extend(content_spans);
    } else {
        // Fall back to plain text
        let content = truncate_or_pad(&diff_line.content, content_width);
        spans.push(Span::styled(content, styles::diff_del_style(theme)));
    }
}

/// Add addition line spans to the spans vector
fn add_addition_spans(
    theme: &Theme,
    spans: &mut Vec<Span>,
    diff_line: &crate::model::DiffLine,
    content_width: usize,
    lw: usize,
) {
    let line_num = diff_line
        .new_lineno
        .map(|n| format!("{n:>lw$}"))
        .unwrap_or_else(|| " ".repeat(lw));

    spans.push(Span::styled(
        format!("{line_num} "),
        styles::dim_style(theme),
    ));
    spans.push(Span::styled("▌".to_string(), styles::diff_add_style(theme)));

    // Use syntax highlighting if available
    if let Some(ref highlighted) = diff_line.highlighted_spans {
        let syntax_pad_style = Style::default().fg(theme.diff_add).bg(theme.syntax_add_bg);
        let content_spans = truncate_or_pad_spans(highlighted, content_width, syntax_pad_style);
        spans.extend(content_spans);
    } else {
        // Fall back to plain text
        let content = truncate_or_pad(&diff_line.content, content_width);
        spans.push(Span::styled(content, styles::diff_add_style(theme)));
    }
}

/// Add empty column spans (for when one side has no content)
fn add_empty_column_spans(spans: &mut Vec<Span>, content_width: usize, lw: usize) {
    // line_num(lw) + space(1) + prefix(1) + content
    spans.push(Span::styled(
        " ".repeat(lw + 1 + 1 + content_width),
        Style::default(),
    ));
}

/// Add comments for a specific line.
/// Returns (new_line_idx, optional cursor info for inline comment input)
/// Render remote review threads anchored at this `(file, line, side)`
/// position into the side-by-side rendering. Mirrors the unified-view
/// helper but uses the side-by-side cursor indicator path.
fn add_remote_threads_to_line(
    line_num: u32,
    side: LineSide,
    ctx: &SideBySideContext,
    file_path: &std::path::Path,
    mut line_idx: usize,
    lines: &mut Vec<Line>,
) -> usize {
    use crate::forge::remote_comments::{PrCommentsVisibility, RemoteCommentSide};
    let visibility = ctx.app.session.remote_comments_visibility;
    if matches!(visibility, PrCommentsVisibility::Hide) {
        return line_idx;
    }
    let target_path = file_path.to_string_lossy();
    for thread in &ctx.app.forge_review_threads {
        let Some(muted) = visibility.render_decision(thread) else {
            continue;
        };
        if thread.path != *target_path {
            continue;
        }
        let Some(thread_line) = thread.line else {
            continue;
        };
        if thread_line != line_num {
            continue;
        }
        let matches_side = matches!(
            (thread.side, side),
            (RemoteCommentSide::Right, LineSide::New) | (RemoteCommentSide::Left, LineSide::Old)
        );
        if !matches_side {
            continue;
        }
        let thread_lines = comment_panel::format_remote_thread_lines(ctx.theme, thread, muted);
        let box_top_row = line_idx;
        for mut comment_line in thread_lines {
            let indicator = cursor_indicator(line_idx, ctx.current_line_idx);
            comment_line.spans.insert(
                0,
                Span::styled(indicator, styles::current_line_indicator_style(ctx.theme)),
            );
            lines.push(comment_line);
            line_idx += 1;
        }
        crate::ui::diff_view::push_comment_bar(
            &mut ctx.comment_bars.borrow_mut(),
            box_top_row,
            Some(crate::model::LineRange::single(thread_line)),
        );
    }
    line_idx
}

fn add_comments_to_line(
    line_num: u32,
    line_comments: &std::collections::HashMap<u32, Vec<crate::model::Comment>>,
    side: LineSide,
    ctx: &SideBySideContext,
    file_idx: usize,
    mut line_idx: usize,
    lines: &mut Vec<Line>,
) -> (usize, Option<SideBySideCursorInfo>) {
    // Check if we're adding/editing a comment on this line and side
    let is_line_comment_mode = ctx.comment_input_mode
        && file_idx == ctx.current_file_idx
        && ctx.comment_line == Some((line_num, side));
    let mut cursor_info_out: Option<SideBySideCursorInfo> = None;

    if let Some(comments) = line_comments.get(&line_num) {
        for comment in comments {
            let comment_side = comment.side.unwrap_or(LineSide::New);
            if (side == LineSide::Old && comment_side == LineSide::Old)
                || (side == LineSide::New && comment_side != LineSide::Old)
            {
                // Check if this comment is being edited
                let is_being_edited =
                    is_line_comment_mode && ctx.editing_comment_id == Some(comment.id.as_str());

                if is_being_edited {
                    // Render inline input instead
                    let line_range = ctx
                        .comment_line_range
                        .or_else(|| Some(LineRange::single(line_num)));
                    let (input_lines, cursor_info) = comment_panel::format_comment_input_lines(
                        ctx.theme,
                        comment_type_presentation(ctx.app, &ctx.comment_type),
                        ctx.comment_buffer,
                        ctx.comment_cursor,
                        line_range,
                        true,
                        ctx.supports_keyboard_enhancement,
                        ctx.panel_width.saturating_sub(1),
                    );
                    let box_top_row = line_idx;
                    let box_end = line_idx + input_lines.len().saturating_sub(1);
                    let annotations_replaced = App::comment_display_lines(comment, ctx.panel_width);
                    cursor_info_out = Some((
                        line_idx + cursor_info.line_offset,
                        1 + cursor_info.column,
                        line_idx,
                        box_end,
                        annotations_replaced,
                    ));

                    for mut input_line in input_lines {
                        let indicator = cursor_indicator(line_idx, ctx.current_line_idx);
                        input_line.spans.insert(
                            0,
                            Span::styled(
                                indicator,
                                styles::current_line_indicator_style(ctx.theme),
                            ),
                        );
                        lines.push(input_line);
                        line_idx += 1;
                    }
                    crate::ui::diff_view::push_comment_bar(
                        &mut ctx.comment_bars.borrow_mut(),
                        box_top_row,
                        line_range,
                    );
                } else {
                    let line_range = comment
                        .line_range
                        .or_else(|| Some(LineRange::single(line_num)));
                    let comment_lines = comment_panel::format_comment_lines(
                        ctx.theme,
                        comment_type_presentation(ctx.app, &comment.comment_type),
                        &comment.content,
                        line_range,
                        ctx.panel_width.saturating_sub(1),
                    );
                    let box_top_row = line_idx;
                    for mut comment_line in comment_lines {
                        let indicator = cursor_indicator(line_idx, ctx.current_line_idx);
                        comment_line.spans.insert(
                            0,
                            Span::styled(
                                indicator,
                                styles::current_line_indicator_style(ctx.theme),
                            ),
                        );
                        lines.push(comment_line);
                        line_idx += 1;
                    }
                    crate::ui::diff_view::push_comment_bar(
                        &mut ctx.comment_bars.borrow_mut(),
                        box_top_row,
                        line_range,
                    );
                }
            }
        }
    }

    // Render inline input for new line comment
    if is_line_comment_mode && ctx.editing_comment_id.is_none() {
        let line_range = ctx
            .comment_line_range
            .or_else(|| Some(LineRange::single(line_num)));
        let (input_lines, cursor_info) = comment_panel::format_comment_input_lines(
            ctx.theme,
            comment_type_presentation(ctx.app, &ctx.comment_type),
            ctx.comment_buffer,
            ctx.comment_cursor,
            line_range,
            false,
            ctx.supports_keyboard_enhancement,
            ctx.panel_width.saturating_sub(1),
        );
        let box_top_row = line_idx;
        let box_end = line_idx + input_lines.len().saturating_sub(1);
        cursor_info_out = Some((
            line_idx + cursor_info.line_offset,
            1 + cursor_info.column,
            line_idx,
            box_end,
            0,
        ));

        for mut input_line in input_lines {
            let indicator = cursor_indicator(line_idx, ctx.current_line_idx);
            input_line.spans.insert(
                0,
                Span::styled(indicator, styles::current_line_indicator_style(ctx.theme)),
            );
            lines.push(input_line);
            line_idx += 1;
        }
        crate::ui::diff_view::push_comment_bar(
            &mut ctx.comment_bars.borrow_mut(),
            box_top_row,
            line_range,
        );
    }

    (line_idx, cursor_info_out)
}

#[cfg(test)]
mod remote_comments_side_by_side_snapshot_tests {
    //! Render-snapshot tests for inline remote review threads in the
    //! side-by-side diff view. Confirms the badge appears at least once
    //! when a thread is active and is hidden under `:comments hide`.
    use crate::app::{App, DiffSource, DiffViewMode, InputMode, PullRequestDiffSource};
    use crate::error::Result as TuicrResult;
    use crate::error::TuicrError;
    use crate::forge::remote_comments::{
        PrCommentsVisibility, RemoteCommentSide, RemoteReviewComment, RemoteReviewThread,
    };
    use crate::forge::traits::{ForgeRepository, PrSessionKey};
    use crate::model::{
        DiffFile, DiffHunk, DiffLine, FileStatus, LineOrigin, ReviewSession, SessionDiffSource,
    };
    use crate::syntax::SyntaxHighlighter;
    use crate::theme::Theme;
    use crate::ui::render;
    use crate::vcs::traits::{VcsBackend, VcsChangeStatus, VcsInfo, VcsType};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use std::path::{Path, PathBuf};

    struct SnapshotVcs {
        info: VcsInfo,
    }

    impl VcsBackend for SnapshotVcs {
        fn info(&self) -> &VcsInfo {
            &self.info
        }
        fn get_working_tree_diff(
            &self,
            _highlighter: &SyntaxHighlighter,
        ) -> TuicrResult<Vec<DiffFile>> {
            Err(TuicrError::NoChanges)
        }
        fn fetch_context_lines(
            &self,
            _file_path: &Path,
            _file_status: FileStatus,
            _ref_commit: Option<&str>,
            _start_line: u32,
            _end_line: u32,
        ) -> TuicrResult<Vec<DiffLine>> {
            Ok(Vec::new())
        }
        fn get_change_status(&self) -> TuicrResult<VcsChangeStatus> {
            Ok(VcsChangeStatus {
                staged: false,
                unstaged: false,
            })
        }
        fn file_line_count(
            &self,
            _file_path: &Path,
            _file_status: FileStatus,
            _ref_commit: Option<&str>,
        ) -> TuicrResult<u32> {
            Ok(0)
        }
    }

    fn repo() -> ForgeRepository {
        ForgeRepository::github("github.com", "agavra", "tuicr")
    }

    fn sample_diff_file() -> DiffFile {
        let lines = vec![
            DiffLine {
                origin: LineOrigin::Context,
                content: "first".to_string(),
                old_lineno: Some(1),
                new_lineno: Some(1),
                highlighted_spans: None,
            },
            DiffLine {
                origin: LineOrigin::Addition,
                content: "second".to_string(),
                old_lineno: None,
                new_lineno: Some(2),
                highlighted_spans: None,
            },
        ];
        let hunk = DiffHunk {
            header: "@@ -1,1 +1,2 @@".to_string(),
            lines,
            old_start: 1,
            old_count: 1,
            new_start: 1,
            new_count: 2,
        };
        let hunks = vec![hunk];
        let content_hash = DiffFile::compute_content_hash(&hunks);
        DiffFile {
            old_path: Some(PathBuf::from("src/lib.rs")),
            new_path: Some(PathBuf::from("src/lib.rs")),
            status: FileStatus::Modified,
            hunks,
            is_binary: false,
            is_too_large: false,
            is_commit_message: false,
            content_hash,
        }
    }

    fn thread() -> RemoteReviewThread {
        RemoteReviewThread {
            id: "T".to_string(),
            path: "src/lib.rs".to_string(),
            line: Some(2),
            side: RemoteCommentSide::Right,
            is_resolved: false,
            is_outdated: false,
            comments: vec![RemoteReviewComment {
                id: "C".to_string(),
                author: Some("alice".to_string()),
                body: "sbs hello".to_string(),
                created_at: None,
                in_reply_to: None,
                url: "https://example.com".to_string(),
            }],
        }
    }

    fn make_pr_app() -> App {
        let pr = PullRequestDiffSource {
            key: PrSessionKey::new(repo(), 125, "headsha".to_string()),
            base_sha: "basesha".to_string(),
            title: "test pr".to_string(),
            url: "https://example.com".to_string(),
            head_ref_name: "feat".to_string(),
            base_ref_name: "main".to_string(),
            state: "OPEN".to_string(),
            closed: false,
            merged: false,
        };
        let vcs_info = VcsInfo {
            root_path: PathBuf::from("forge:github.com/agavra/tuicr"),
            head_commit: "headsha".to_string(),
            branch_name: Some("feat".to_string()),
            vcs_type: VcsType::File,
        };
        let mut session = ReviewSession::new(
            vcs_info.root_path.clone(),
            "headsha".to_string(),
            Some("feat".to_string()),
            SessionDiffSource::PullRequest,
        );
        session.pr_session_key = Some(pr.key.clone());
        let mut app = App::build(
            Box::new(SnapshotVcs {
                info: vcs_info.clone(),
            }),
            vcs_info,
            Theme::dark(),
            None,
            false,
            vec![sample_diff_file()],
            session,
            DiffSource::PullRequest(Box::new(pr)),
            InputMode::Normal,
            Vec::new(),
            None,
        )
        .expect("build app");
        app.diff_view_mode = DiffViewMode::SideBySide;
        app
    }

    fn draw(app: &mut App) -> Buffer {
        let backend = TestBackend::new(160, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render(frame, app))
            .expect("draw frame");
        terminal.backend().buffer().clone()
    }

    fn body_text(buffer: &Buffer) -> String {
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn should_render_remote_comment_inline_in_side_by_side_diff() {
        // given
        let mut app = make_pr_app();
        app.forge_review_threads = vec![thread()];
        app.rebuild_annotations();
        // when
        let buffer = draw(&mut app);
        // then
        let body = body_text(&buffer);
        assert!(
            body.contains("[github @alice]"),
            "expected badge in side-by-side render:\n{body}"
        );
    }

    #[test]
    fn should_hide_remote_comments_under_comments_hide_in_side_by_side() {
        // given
        let mut app = make_pr_app();
        app.forge_review_threads = vec![thread()];
        app.set_remote_comments_visibility(PrCommentsVisibility::Hide);
        // when
        let buffer = draw(&mut app);
        // then
        let body = body_text(&buffer);
        assert!(
            !body.contains("[github @alice"),
            "remote comment leaked under Hide:\n{body}"
        );
    }
}
