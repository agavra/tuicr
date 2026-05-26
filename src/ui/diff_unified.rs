use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use unicode_width::UnicodeWidthStr;

use crate::app::{
    App, DiffSource, ExpandDirection, FocusedPanel, GAP_EXPAND_BATCH, GapId, InputMode,
};
use crate::forge::remote_comments::PrCommentsVisibility;
use crate::model::{FileStatus, LineOrigin, LineRange, LineSide};
use crate::theme::Theme;
use crate::ui::comment_panel;
use crate::ui::diff_view::{
    apply_horizontal_scroll, comment_type_presentation, cursor_indicator, cursor_indicator_spaced,
    diff_stat_title, is_line_highlighted, paint_unified_diff_rows_with,
    paint_visual_selection_overlay, populate_row_to_annotation, push_comment_bar,
    render_expander_line, render_hidden_lines, scroll_comment_input_into_view,
    unified_line_bg_style,
};
use crate::ui::styles;
use crate::vcs::git::calculate_gap;

pub(super) fn render_unified_diff(frame: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focused_panel == FocusedPanel::Diff;

    let title = crate::ui::diff_view::diff_title(app, area.width);

    let block = Block::default()
        .title(title)
        .title_top(diff_stat_title(app).right_aligned())
        .borders(Borders::ALL)
        .style(styles::panel_style(&app.theme))
        .border_style(styles::border_style(&app.theme, focused));

    let inner = block.inner(area);
    let comment_width = inner.width.saturating_sub(1) as usize;
    frame.render_widget(block, area);

    // Update viewport height for scroll calculations
    app.diff_state.viewport_height = inner.height as usize;
    app.diff_inner_area = Some(inner);

    // Reset comment input annotation offset (will be set if a comment input box is rendered)
    app.comment_input_annotation_offset = None;

    let lw = app.lineno_width();

    // Build all diff lines for infinite scroll
    // Track line index to mark the current line (cursor position)
    let mut lines: Vec<Line> = Vec::new();
    let mut line_idx: usize = 0;
    let current_line_idx = app.diff_state.cursor_line;

    // Only build the expensive per-diff-line spans for lines that are actually
    // visible. Everything else still pushes (cheap) so `lines.len()` keeps
    // matching `line_idx`, but the hot inner loops push `Line::default()` for
    // off-screen rows. In Comment mode the scroll offset may be adjusted after
    // building, so fall back to a full build there.
    let (visible_start, visible_end) = crate::ui::diff_view::diff_visible_range(app, inner);

    // Track cursor position for IME when in Comment mode
    // Store the logical line index and column where the cursor should be
    let mut comment_cursor_logical_line: Option<usize> = None;
    let mut comment_cursor_column: u16 = 0;
    // Track the full extent of the comment input box so we can auto-scroll
    // the viewport to keep it visible while the user types.
    let mut comment_input_box_range: Option<(usize, usize)> = None;
    // Records per-comment bar info — populated at each line-level comment
    // call site and consumed by the bar paint pass at the end of render.
    let mut comment_bars: Vec<crate::ui::diff_view::CommentBarAnchor> = Vec::new();

    let is_review_comment_mode =
        app.input_mode == InputMode::Comment && app.comment_is_review_level;

    // The `═══ Review Comments ═══` label is redundant in single-file
    // view (review-level comments are still rendered below; they just
    // don't need a banner that confuses horizontal scroll).
    if !app.is_single_file_view {
        let general_indicator = cursor_indicator_spaced(line_idx, current_line_idx);
        lines.push(Line::from(vec![
            Span::styled(
                general_indicator,
                styles::current_line_indicator_style(&app.theme),
            ),
            Span::styled(
                "═══ Review Comments ",
                styles::file_header_style(&app.theme),
            ),
            Span::styled(
                crate::ui::diff_view::HEADER_RULE,
                styles::file_header_style(&app.theme),
            ),
        ]));
        line_idx += 1;
    }

    for summary in &app.forge_review_summaries {
        let summary_lines = comment_panel::format_remote_review_summary_lines(&app.theme, summary);
        for mut summary_line in summary_lines {
            let indicator = cursor_indicator(line_idx, current_line_idx);
            summary_line.spans.insert(
                0,
                Span::styled(indicator, styles::current_line_indicator_style(&app.theme)),
            );
            lines.push(summary_line);
            line_idx += 1;
        }
    }

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
                comment_width,
            );
            comment_cursor_logical_line = Some(line_idx + cursor_info.line_offset);
            comment_cursor_column = 1 + cursor_info.column;
            comment_input_box_range =
                Some((line_idx, line_idx + input_lines.len().saturating_sub(1)));
            let annotations_replaced = App::comment_display_lines(comment, inner.width as usize);
            app.comment_input_annotation_offset =
                Some((line_idx, input_lines.len(), annotations_replaced));

            for mut input_line in input_lines {
                let indicator = cursor_indicator(line_idx, current_line_idx);
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
                comment_width,
                (comment.author != app.username).then_some(comment.author.as_str()),
            );
            for mut comment_line in comment_lines {
                let indicator = cursor_indicator(line_idx, current_line_idx);
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
            comment_width,
        );
        comment_cursor_logical_line = Some(line_idx + cursor_info.line_offset);
        comment_cursor_column = 1 + cursor_info.column;
        comment_input_box_range = Some((line_idx, line_idx + input_lines.len().saturating_sub(1)));
        app.comment_input_annotation_offset = Some((line_idx, input_lines.len(), 0));

        for mut input_line in input_lines {
            let indicator = cursor_indicator(line_idx, current_line_idx);
            input_line.spans.insert(
                0,
                Span::styled(indicator, styles::current_line_indicator_style(&app.theme)),
            );
            lines.push(input_line);
            line_idx += 1;
        }
    }

    for (file_idx, file) in app.diff_files.iter().enumerate() {
        // Single-file view hides every file except the one the cursor is
        // currently on. Navigation (`}`/`{`, file list) flips
        // `current_file_idx` and the next render shows the new file.
        if app.is_single_file_view && file_idx != app.diff_state.current_file_idx {
            continue;
        }
        let path = file.display_path();
        let status = file.status.as_char();
        let is_reviewed = app.session.is_file_reviewed(path);

        // File header box: ╔══╗ + ║ name ║ + ╚══╝. Drawn flush to both
        // panel edges — no indicator gutter, since the cursor never lands
        // on these decoration lines. Single-file view skips this entirely.
        if !app.is_single_file_view {
            let header_style = styles::file_header_style(&app.theme);

            // Top border
            lines.push(Line::from(vec![
                Span::styled("╔═", header_style),
                Span::styled(crate::ui::diff_view::HEADER_RULE, header_style),
            ]));
            line_idx += 1;

            // Filename row
            let review_mark = if is_reviewed { "✓ " } else { "" };
            let header_text = if file.is_commit_message {
                format!("║ {}Commit Message ", review_mark)
            } else if app.is_pristine_mode {
                format!("║ {}{} ", review_mark, path.display())
            } else {
                format!("║ {}{} [{}] ", review_mark, path.display(), status)
            };
            lines.push(Line::from(vec![Span::styled(header_text, header_style)]));
            line_idx += 1;

            // Bottom border
            lines.push(Line::from(vec![
                Span::styled("╚═", header_style),
                Span::styled(crate::ui::diff_view::HEADER_RULE, header_style),
            ]));
            line_idx += 1;
        }

        // If file is reviewed (and we're in multi-file view), skip
        // rendering the body. In single-file view the user explicitly
        // focused this file, so show its content under a dimmed banner.
        if is_reviewed && !app.is_single_file_view {
            continue;
        }
        if is_reviewed && app.is_single_file_view {
            let indicator = cursor_indicator(line_idx, current_line_idx);
            lines.push(Line::from(vec![
                Span::styled(indicator, styles::current_line_indicator_style(&app.theme)),
                Span::styled(
                    "  Marked reviewed -- r to re-open",
                    Style::default()
                        .fg(app.theme.fg_secondary)
                        .add_modifier(Modifier::DIM),
                ),
            ]));
            line_idx += 1;
        }

        // Check if we're editing/adding a file-level comment for this file
        let is_file_comment_mode = app.input_mode == InputMode::Comment
            && app.comment_is_file_level
            && file_idx == app.diff_state.current_file_idx;

        // Show file-level comments right after the header
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
                        comment_width,
                    );
                    // Track cursor position: logical line = current line_idx + cursor offset within input
                    comment_cursor_logical_line = Some(line_idx + cursor_info.line_offset);
                    // Column = indicator (1) + cursor_info.column
                    comment_cursor_column = 1 + cursor_info.column;
                    comment_input_box_range =
                        Some((line_idx, line_idx + input_lines.len().saturating_sub(1)));
                    let annotations_replaced =
                        App::comment_display_lines(comment, inner.width as usize);
                    app.comment_input_annotation_offset =
                        Some((line_idx, input_lines.len(), annotations_replaced));

                    for mut input_line in input_lines {
                        let indicator = cursor_indicator(line_idx, current_line_idx);
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
                        comment_width,
                        (comment.author != app.username).then_some(comment.author.as_str()),
                    );
                    for mut comment_line in comment_lines {
                        let indicator = cursor_indicator(line_idx, current_line_idx);
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
                comment_width,
            );
            // Track cursor position
            comment_cursor_logical_line = Some(line_idx + cursor_info.line_offset);
            comment_cursor_column = 1 + cursor_info.column;
            comment_input_box_range =
                Some((line_idx, line_idx + input_lines.len().saturating_sub(1)));
            app.comment_input_annotation_offset = Some((line_idx, input_lines.len(), 0));

            for mut input_line in input_lines {
                let indicator = cursor_indicator(line_idx, current_line_idx);
                input_line.spans.insert(
                    0,
                    Span::styled(indicator, styles::current_line_indicator_style(&app.theme)),
                );
                lines.push(input_line);
                line_idx += 1;
            }
        }

        if file.is_too_large {
            let indicator = cursor_indicator_spaced(line_idx, current_line_idx);
            lines.push(Line::from(vec![
                Span::styled(indicator, styles::current_line_indicator_style(&app.theme)),
                Span::styled("(file too large to display)", styles::dim_style(&app.theme)),
            ]));
            line_idx += 1;
        } else if file.is_binary {
            let indicator = cursor_indicator_spaced(line_idx, current_line_idx);
            lines.push(Line::from(vec![
                Span::styled(indicator, styles::current_line_indicator_style(&app.theme)),
                Span::styled("(binary file)", styles::dim_style(&app.theme)),
            ]));
            line_idx += 1;
        } else if file.hunks.is_empty() {
            let indicator = cursor_indicator_spaced(line_idx, current_line_idx);
            lines.push(Line::from(vec![
                Span::styled(indicator, styles::current_line_indicator_style(&app.theme)),
                Span::styled("(no changes)", styles::dim_style(&app.theme)),
            ]));
            line_idx += 1;
        } else {
            // Get line comments for this file
            let line_comments = app
                .session
                .files
                .get(path)
                .map(|r| &r.line_comments)
                .unwrap_or(&crate::ui::diff_view::EMPTY_LINE_COMMENTS);

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
                            if line_idx < visible_start || line_idx >= visible_end {
                                lines.push(Line::default());
                                line_idx += 1;
                                continue;
                            }
                            render_expanded_context_line(
                                &mut lines,
                                &mut line_idx,
                                current_line_idx,
                                expanded_line,
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
                                    current_line_idx,
                                    remaining,
                                    &app.theme,
                                );
                            }
                            render_expander_line(
                                &mut lines,
                                &mut line_idx,
                                current_line_idx,
                                ExpandDirection::Up,
                                remaining,
                                &app.theme,
                            );
                        } else if remaining >= GAP_EXPAND_BATCH {
                            render_expander_line(
                                &mut lines,
                                &mut line_idx,
                                current_line_idx,
                                ExpandDirection::Down,
                                remaining,
                                &app.theme,
                            );
                            render_hidden_lines(
                                &mut lines,
                                &mut line_idx,
                                current_line_idx,
                                remaining,
                                &app.theme,
                            );
                            render_expander_line(
                                &mut lines,
                                &mut line_idx,
                                current_line_idx,
                                ExpandDirection::Up,
                                remaining,
                                &app.theme,
                            );
                        } else {
                            render_expander_line(
                                &mut lines,
                                &mut line_idx,
                                current_line_idx,
                                ExpandDirection::Both,
                                remaining,
                                &app.theme,
                            );
                        }
                    }

                    // Render bottom expanded lines
                    if let Some(bot) = bot_lines {
                        for expanded_line in bot {
                            if line_idx < visible_start || line_idx >= visible_end {
                                lines.push(Line::default());
                                line_idx += 1;
                                continue;
                            }
                            render_expanded_context_line(
                                &mut lines,
                                &mut line_idx,
                                current_line_idx,
                                expanded_line,
                                &app.theme,
                                lw,
                            );
                        }
                    }
                }

                // Hunk header
                let indicator = cursor_indicator_spaced(line_idx, current_line_idx);
                lines.push(Line::from(vec![
                    Span::styled(indicator, styles::current_line_indicator_style(&app.theme)),
                    Span::styled(
                        hunk.header.to_string(),
                        styles::diff_hunk_header_style(&app.theme),
                    ),
                ]));
                line_idx += 1;

                // Diff lines
                for diff_line in &hunk.lines {
                    // Hot path: skip span/style allocation entirely for diff
                    // lines outside the viewport. Comment handling below still
                    // runs so `line_idx` stays exact and any comment box that
                    // crosses into the viewport is rendered.
                    if line_idx < visible_start || line_idx >= visible_end {
                        lines.push(Line::default());
                        line_idx += 1;
                    } else {
                        let (prefix, base_style) = match diff_line.origin {
                            LineOrigin::Addition => ("▌", styles::diff_add_style(&app.theme)),
                            LineOrigin::Deletion => ("▌", styles::diff_del_style(&app.theme)),
                            LineOrigin::Context => (" ", styles::diff_context_style(&app.theme)),
                        };

                        let style = base_style;

                        let blank = " ".repeat(lw + 1);
                        let line_num_str = match diff_line.origin {
                            LineOrigin::Addition => diff_line
                                .new_lineno
                                .map(|n| format!("{n:>lw$} "))
                                .unwrap_or_else(|| blank.clone()),
                            LineOrigin::Deletion => diff_line
                                .old_lineno
                                .map(|n| format!("{n:>lw$} "))
                                .unwrap_or_else(|| blank.clone()),
                            _ => diff_line
                                .new_lineno
                                .or(diff_line.old_lineno)
                                .map(|n| format!("{n:>lw$} "))
                                .unwrap_or_else(|| blank),
                        };

                        let indicator = cursor_indicator(line_idx, current_line_idx);

                        let line_num_style = styles::dim_style(&app.theme);

                        let mut line_spans = vec![
                            Span::styled(
                                indicator,
                                styles::current_line_indicator_style(&app.theme),
                            ),
                            Span::styled(line_num_str, line_num_style),
                            Span::styled(format!("{prefix} "), style),
                        ];

                        if let Some(ref highlighted) = diff_line.highlighted_spans {
                            for (span_style, span_text) in highlighted {
                                line_spans.push(Span::styled(span_text.clone(), *span_style));
                            }
                        } else {
                            line_spans.push(Span::styled(diff_line.content.clone(), style));
                        }

                        // Mark add/del lines with their effective EOL style so we can paint full
                        // row backgrounds later (including wrapped visual rows).
                        if matches!(
                            diff_line.origin,
                            LineOrigin::Addition | LineOrigin::Deletion
                        ) {
                            let eol_style = match diff_line.highlighted_spans.as_ref() {
                                // For syntax-highlighted lines (including empty highlighted lines),
                                // use syntax diff background so row fill matches code spans.
                                Some(_) => {
                                    let syntax_bg = match diff_line.origin {
                                        LineOrigin::Addition => app.theme.syntax_add_bg,
                                        LineOrigin::Deletion => app.theme.syntax_del_bg,
                                        LineOrigin::Context => app.theme.panel_bg,
                                    };
                                    let base = line_spans.last().map(|s| s.style).unwrap_or(style);
                                    base.bg(syntax_bg)
                                }
                                // Non-highlighted lines keep classic diff background.
                                None => line_spans.last().map(|s| s.style).unwrap_or(style),
                            };
                            // Zero-width marker span carrying the background style.
                            line_spans.push(Span::styled(String::new(), eol_style));
                        }

                        lines.push(Line::from(line_spans));
                        line_idx += 1;
                    }

                    // Show line comments for both old side (deleted lines) and new side (added/context)
                    // Old side comments (for deleted lines)
                    if let Some(old_ln) = diff_line.old_lineno {
                        // Check if we're adding/editing a comment on this line (old side)
                        let is_line_comment_mode = app.input_mode == InputMode::Comment
                            && !app.comment_is_file_level
                            && file_idx == app.diff_state.current_file_idx
                            && app.comment_line == Some((old_ln, LineSide::Old));

                        if let Some(comments) = line_comments.get(&old_ln) {
                            for comment in comments {
                                if comment.side == Some(LineSide::Old) {
                                    // Skip if this comment is being edited
                                    let is_being_edited = is_line_comment_mode
                                        && app.editing_comment_id.as_ref() == Some(&comment.id);

                                    if is_being_edited {
                                        let line_range = app
                                            .comment_line_range
                                            .map(|(r, _)| r)
                                            .or_else(|| Some(LineRange::single(old_ln)));
                                        let (input_lines, cursor_info) =
                                            comment_panel::format_comment_input_lines(
                                                &app.theme,
                                                comment_type_presentation(app, &app.comment_type),
                                                &app.comment_buffer,
                                                app.comment_cursor,
                                                line_range,
                                                true,
                                                app.supports_keyboard_enhancement,
                                                comment_width,
                                            );
                                        comment_cursor_logical_line =
                                            Some(line_idx + cursor_info.line_offset);
                                        comment_cursor_column = 1 + cursor_info.column;
                                        let box_top_row = line_idx;
                                        comment_input_box_range = Some((
                                            line_idx,
                                            line_idx + input_lines.len().saturating_sub(1),
                                        ));
                                        let annotations_replaced = App::comment_display_lines(
                                            comment,
                                            inner.width as usize,
                                        );
                                        app.comment_input_annotation_offset = Some((
                                            line_idx,
                                            input_lines.len(),
                                            annotations_replaced,
                                        ));

                                        for mut input_line in input_lines {
                                            let indicator =
                                                cursor_indicator(line_idx, current_line_idx);
                                            input_line.spans.insert(
                                                0,
                                                Span::styled(
                                                    indicator,
                                                    styles::current_line_indicator_style(
                                                        &app.theme,
                                                    ),
                                                ),
                                            );
                                            lines.push(input_line);
                                            line_idx += 1;
                                        }
                                        push_comment_bar(
                                            &mut comment_bars,
                                            box_top_row,
                                            line_range,
                                        );
                                    } else {
                                        let line_range = comment
                                            .line_range
                                            .or_else(|| Some(LineRange::single(old_ln)));
                                        let comment_lines = comment_panel::format_comment_lines(
                                            &app.theme,
                                            comment_type_presentation(app, &comment.comment_type),
                                            &comment.content,
                                            line_range,
                                            comment_width,
                                            (comment.author != app.username)
                                                .then_some(comment.author.as_str()),
                                        );
                                        let box_top_row = line_idx;
                                        for mut comment_line in comment_lines {
                                            let is_current = line_idx == current_line_idx;
                                            let indicator = if is_current { "▶" } else { " " };
                                            comment_line.spans.insert(
                                                0,
                                                Span::styled(
                                                    indicator,
                                                    styles::current_line_indicator_style(
                                                        &app.theme,
                                                    ),
                                                ),
                                            );
                                            lines.push(comment_line);
                                            line_idx += 1;
                                        }
                                        push_comment_bar(
                                            &mut comment_bars,
                                            box_top_row,
                                            line_range,
                                        );
                                    }
                                }
                            }
                        }

                        // Render remote review threads anchored at this old-side line.
                        render_remote_threads_for_anchor(
                            &mut lines,
                            &mut line_idx,
                            current_line_idx,
                            app,
                            path,
                            old_ln,
                            LineSide::Old,
                            &mut comment_bars,
                        );

                        // Render inline input for new line comment (old side)
                        if is_line_comment_mode && app.editing_comment_id.is_none() {
                            let line_range = app
                                .comment_line_range
                                .map(|(r, _)| r)
                                .or_else(|| Some(LineRange::single(old_ln)));
                            let (input_lines, cursor_info) =
                                comment_panel::format_comment_input_lines(
                                    &app.theme,
                                    comment_type_presentation(app, &app.comment_type),
                                    &app.comment_buffer,
                                    app.comment_cursor,
                                    line_range,
                                    false,
                                    app.supports_keyboard_enhancement,
                                    comment_width,
                                );
                            comment_cursor_logical_line = Some(line_idx + cursor_info.line_offset);
                            comment_cursor_column = 1 + cursor_info.column;
                            let box_top_row = line_idx;
                            comment_input_box_range =
                                Some((line_idx, line_idx + input_lines.len().saturating_sub(1)));
                            app.comment_input_annotation_offset =
                                Some((line_idx, input_lines.len(), 0));

                            for mut input_line in input_lines {
                                let indicator = cursor_indicator(line_idx, current_line_idx);
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
                            push_comment_bar(&mut comment_bars, box_top_row, line_range);
                        }
                    }

                    // New side comments (for added/context lines)
                    if let Some(new_ln) = diff_line.new_lineno {
                        // Check if we're adding/editing a comment on this line (new side)
                        let is_line_comment_mode = app.input_mode == InputMode::Comment
                            && !app.comment_is_file_level
                            && file_idx == app.diff_state.current_file_idx
                            && app.comment_line == Some((new_ln, LineSide::New));

                        if let Some(comments) = line_comments.get(&new_ln) {
                            for comment in comments {
                                if comment.side != Some(LineSide::Old) {
                                    // Skip if this comment is being edited
                                    let is_being_edited = is_line_comment_mode
                                        && app.editing_comment_id.as_ref() == Some(&comment.id);

                                    if is_being_edited {
                                        let line_range = app
                                            .comment_line_range
                                            .map(|(r, _)| r)
                                            .or_else(|| Some(LineRange::single(new_ln)));
                                        let (input_lines, cursor_info) =
                                            comment_panel::format_comment_input_lines(
                                                &app.theme,
                                                comment_type_presentation(app, &app.comment_type),
                                                &app.comment_buffer,
                                                app.comment_cursor,
                                                line_range,
                                                true,
                                                app.supports_keyboard_enhancement,
                                                comment_width,
                                            );
                                        comment_cursor_logical_line =
                                            Some(line_idx + cursor_info.line_offset);
                                        comment_cursor_column = 1 + cursor_info.column;
                                        let box_top_row = line_idx;
                                        comment_input_box_range = Some((
                                            line_idx,
                                            line_idx + input_lines.len().saturating_sub(1),
                                        ));
                                        let annotations_replaced = App::comment_display_lines(
                                            comment,
                                            inner.width as usize,
                                        );
                                        app.comment_input_annotation_offset = Some((
                                            line_idx,
                                            input_lines.len(),
                                            annotations_replaced,
                                        ));

                                        for mut input_line in input_lines {
                                            let indicator =
                                                cursor_indicator(line_idx, current_line_idx);
                                            input_line.spans.insert(
                                                0,
                                                Span::styled(
                                                    indicator,
                                                    styles::current_line_indicator_style(
                                                        &app.theme,
                                                    ),
                                                ),
                                            );
                                            lines.push(input_line);
                                            line_idx += 1;
                                        }
                                        push_comment_bar(
                                            &mut comment_bars,
                                            box_top_row,
                                            line_range,
                                        );
                                    } else {
                                        let line_range = comment
                                            .line_range
                                            .or_else(|| Some(LineRange::single(new_ln)));
                                        let comment_lines = comment_panel::format_comment_lines(
                                            &app.theme,
                                            comment_type_presentation(app, &comment.comment_type),
                                            &comment.content,
                                            line_range,
                                            comment_width,
                                            (comment.author != app.username)
                                                .then_some(comment.author.as_str()),
                                        );
                                        let box_top_row = line_idx;
                                        for mut comment_line in comment_lines {
                                            let indicator =
                                                cursor_indicator(line_idx, current_line_idx);
                                            comment_line.spans.insert(
                                                0,
                                                Span::styled(
                                                    indicator,
                                                    styles::current_line_indicator_style(
                                                        &app.theme,
                                                    ),
                                                ),
                                            );
                                            lines.push(comment_line);
                                            line_idx += 1;
                                        }
                                        push_comment_bar(
                                            &mut comment_bars,
                                            box_top_row,
                                            line_range,
                                        );
                                    }
                                }
                            }
                        }

                        // Render remote review threads anchored at this new-side line.
                        render_remote_threads_for_anchor(
                            &mut lines,
                            &mut line_idx,
                            current_line_idx,
                            app,
                            path,
                            new_ln,
                            LineSide::New,
                            &mut comment_bars,
                        );

                        // Render inline input for new line comment (new side)
                        if is_line_comment_mode && app.editing_comment_id.is_none() {
                            let line_range = app
                                .comment_line_range
                                .map(|(r, _)| r)
                                .or_else(|| Some(LineRange::single(new_ln)));
                            let (input_lines, cursor_info) =
                                comment_panel::format_comment_input_lines(
                                    &app.theme,
                                    comment_type_presentation(app, &app.comment_type),
                                    &app.comment_buffer,
                                    app.comment_cursor,
                                    line_range,
                                    false,
                                    app.supports_keyboard_enhancement,
                                    comment_width,
                                );
                            comment_cursor_logical_line = Some(line_idx + cursor_info.line_offset);
                            comment_cursor_column = 1 + cursor_info.column;
                            let box_top_row = line_idx;
                            comment_input_box_range =
                                Some((line_idx, line_idx + input_lines.len().saturating_sub(1)));
                            app.comment_input_annotation_offset =
                                Some((line_idx, input_lines.len(), 0));

                            for mut input_line in input_lines {
                                let indicator = cursor_indicator(line_idx, current_line_idx);
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
                            push_comment_bar(&mut comment_bars, box_top_row, line_range);
                        }
                    }
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
                        render_expanded_context_line(
                            &mut lines,
                            &mut line_idx,
                            current_line_idx,
                            expanded_line,
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
                        current_line_idx,
                        ExpandDirection::Down,
                        remaining,
                        &app.theme,
                    );
                    if remaining > GAP_EXPAND_BATCH {
                        render_hidden_lines(
                            &mut lines,
                            &mut line_idx,
                            current_line_idx,
                            remaining,
                            &app.theme,
                        );
                    }
                }

                // Render bottom expanded lines
                if let Some(bot) = bot_lines {
                    for expanded_line in bot {
                        render_expanded_context_line(
                            &mut lines,
                            &mut line_idx,
                            current_line_idx,
                            expanded_line,
                            &app.theme,
                            lw,
                        );
                    }
                }
            }
        }

        // Inter-file spacing. In single-file view, the row doubles as a
        // hint pointing at whichever file `j` would walk into next, so
        // the user always knows what's on the other side of the edge.
        // Falls back to a plain blank on the last file (or in multi-file
        // mode) where the indicator is already pulling its weight.
        let indicator = cursor_indicator(line_idx, current_line_idx);
        let next_hint_path = if app.is_single_file_view {
            app.diff_files
                .get(app.diff_state.current_file_idx + 1)
                .map(|f| f.display_path().display().to_string())
        } else {
            None
        };
        if let Some(next_path) = next_hint_path {
            lines.push(Line::from(vec![
                Span::styled(indicator, styles::current_line_indicator_style(&app.theme)),
                Span::styled(
                    format!("  \u{2193}  {next_path}"),
                    Style::default()
                        .fg(app.theme.fg_secondary)
                        .add_modifier(Modifier::DIM),
                ),
            ]));
        } else {
            lines.push(Line::from(Span::styled(
                indicator,
                styles::current_line_indicator_style(&app.theme),
            )));
        }
        line_idx += 1;
    }

    // Auto-scroll so the comment input box stays visible while the user types.
    // Without this, adding a comment near the bottom/top of the viewport would
    // place the input box off-screen and the user couldn't see what they type.
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
    let visible_lines_unscrolled_for_bg = visible_lines_unscrolled.clone();
    let visible_lines: Vec<Line> = if app.diff_state.wrap_lines {
        visible_lines_unscrolled
    } else {
        visible_lines_unscrolled
            .into_iter()
            .map(|line| apply_horizontal_scroll(line, scroll_x))
            .collect()
    };

    // Paint per-visual-row add/del backgrounds across full row width.
    paint_unified_diff_rows_with(
        frame,
        inner,
        &visible_lines_unscrolled_for_bg,
        &line_widths,
        app.diff_state.wrap_lines,
        inner.width as usize,
        |_idx, line| unified_line_bg_style(line, &app.theme),
    );

    let overlay_ctx = crate::ui::diff_view::DiffOverlayPaint {
        inner,
        visible_lines_unscrolled: &visible_lines_unscrolled_for_bg,
        line_widths: &line_widths,
        wrap_lines: app.diff_state.wrap_lines,
        viewport_width: inner.width as usize,
        scroll_x,
        scroll_offset: app.diff_state.scroll_offset,
        theme: &app.theme,
        comment_bars: &comment_bars,
    };

    // Section-marker row tint (hunk headers + expand/hidden stubs). Painted
    // before the paragraph so cursor-line and selection overlays still win
    // on the active row.
    crate::ui::diff_view::paint_section_highlight(frame, &overlay_ctx);

    // Keep paragraph bg unset so pre-painted per-row diff backgrounds remain visible.
    let mut diff = Paragraph::new(visible_lines).style(Style::default().fg(app.theme.fg_primary));
    if app.diff_state.wrap_lines {
        diff = diff.wrap(Wrap { trim: false });
    }
    frame.render_widget(diff, inner);

    // Cursor-line bg has to land after the paragraph: spans on +/- lines carry
    // explicit diff_add_bg/diff_del_bg that would mask a pre-paint over the code.
    if app.cursor_line_highlight {
        paint_unified_diff_rows_with(
            frame,
            inner,
            &visible_lines_unscrolled_for_bg,
            &line_widths,
            app.diff_state.wrap_lines,
            inner.width as usize,
            |idx, _line| {
                is_line_highlighted(app, idx).then(|| Style::default().bg(app.theme.cursor_line_bg))
            },
        );
    }

    if let Some(sel) = app.visual_selection {
        paint_visual_selection_overlay(frame, inner, app, sel, &app.theme);
    }

    // File-section header rules extended to the full viewport width.
    crate::ui::diff_view::paint_file_header_fill(frame, &overlay_ctx);

    // Comment-box overlays painted last so the box + bar always win on their
    // single cells regardless of cursor-line / selection underlays.
    crate::ui::diff_view::paint_comment_box_bar(frame, &overlay_ctx);
    crate::ui::diff_view::paint_comment_box_right_border(frame, &overlay_ctx);

    // Calculate screen position for comment cursor if in Comment mode
    if let Some(cursor_logical_line) = comment_cursor_logical_line {
        let scroll_offset = app.diff_state.scroll_offset;
        // Use visible_line_count which accounts for line wrapping
        let visible_lines_count = app.diff_state.visible_line_count.max(1);

        // Check if the cursor line is visible (after scrolling)
        if cursor_logical_line >= scroll_offset
            && cursor_logical_line < scroll_offset + visible_lines_count
        {
            // Calculate screen row - need to account for wrapping
            let logical_offset = cursor_logical_line - scroll_offset;

            // Calculate visual row by summing wrapped line heights
            let mut visual_row: u16 = 0;
            let viewport_width = inner.width as usize;

            if app.diff_state.wrap_lines && viewport_width > 0 {
                // Calculate how many visual rows the lines before cursor take
                // Note: line_widths is indexed from 0 and corresponds to visible lines
                // (i.e., line_widths[0] is the first visible line after scroll)
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

            // Account for diff area position (inner starts at diff block's inner area)
            let screen_col = inner.x + comment_cursor_column;
            let screen_row_abs = inner.y + visual_row;

            app.comment_cursor_screen_pos = Some((screen_col, screen_row_abs));
        }
    }
}

/// Render remote review threads anchored at `(path, line, side)` into the
/// growing line buffer. No-op when `:comments hide` is active or when no
/// threads anchor here. Resolved/outdated threads use muted styling per
/// the spec; visible-but-resolved threads only render under `:comments all`.
#[allow(clippy::too_many_arguments)]
fn render_remote_threads_for_anchor(
    lines: &mut Vec<ratatui::text::Line<'static>>,
    line_idx: &mut usize,
    current_line_idx: usize,
    app: &App,
    file_path: &std::path::Path,
    line: u32,
    side: LineSide,
    comment_bars: &mut Vec<crate::ui::diff_view::CommentBarAnchor>,
) {
    let visibility = app.session.remote_comments_visibility;
    if matches!(visibility, PrCommentsVisibility::Hide) {
        return;
    }
    if app.forge_review_threads.is_empty() {
        return;
    }
    let target_path = file_path.to_string_lossy();
    for thread in &app.forge_review_threads {
        let Some(muted) = visibility.render_decision(thread) else {
            continue;
        };
        if thread.path != *target_path {
            continue;
        }
        let Some(thread_line) = thread.line else {
            continue;
        };
        if thread_line != line {
            continue;
        }
        let matches_side = matches!(
            (thread.side, side),
            (
                crate::forge::remote_comments::RemoteCommentSide::Right,
                LineSide::New
            ) | (
                crate::forge::remote_comments::RemoteCommentSide::Left,
                LineSide::Old
            )
        );
        if !matches_side {
            continue;
        }

        // Render the entire thread as one fused box so it reads as a
        // single discussion unit.
        let thread_lines = comment_panel::format_remote_thread_lines(&app.theme, thread, muted);
        let box_top_row = *line_idx;
        for mut comment_line in thread_lines {
            let indicator = cursor_indicator(*line_idx, current_line_idx);
            comment_line.spans.insert(
                0,
                ratatui::text::Span::styled(
                    indicator,
                    styles::current_line_indicator_style(&app.theme),
                ),
            );
            lines.push(comment_line);
            *line_idx += 1;
        }
        push_comment_bar(
            comment_bars,
            box_top_row,
            Some(crate::model::LineRange::single(thread_line)),
        );
    }
}

/// Render a single expanded context line (shared by unified + side-by-side via unified path)
fn render_expanded_context_line(
    lines: &mut Vec<Line<'_>>,
    line_idx: &mut usize,
    current_line_idx: usize,
    expanded_line: &crate::model::DiffLine,
    theme: &Theme,
    lw: usize,
) {
    let indicator = cursor_indicator(*line_idx, current_line_idx);
    let line_num = expanded_line
        .new_lineno
        .map(|n| format!("{n:>lw$} "))
        .unwrap_or_else(|| " ".repeat(lw + 1));
    let line_spans = vec![
        Span::styled(indicator, styles::current_line_indicator_style(theme)),
        Span::styled(line_num, styles::expanded_context_style(theme)),
        Span::styled("  ", styles::expanded_context_style(theme)),
        Span::styled(
            expanded_line.content.clone(),
            styles::expanded_context_style(theme),
        ),
    ];
    lines.push(Line::from(line_spans));
    *line_idx += 1;
}

#[cfg(test)]
mod remote_comments_snapshot_tests {
    //! Render-snapshot tests for inline remote review threads in the
    //! unified diff. We drive `ui::render` against `TestBackend` and check
    //! for the `[github @author]` badge text on the expected row.
    use crate::app::{App, DiffSource, InputMode, PullRequestDiffSource};
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
        // Two-line file with one context line and one addition so we have
        // a stable `line=2` anchor for the test thread.
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

    fn thread(
        id: &str,
        author: &str,
        body: &str,
        line: u32,
        resolved: bool,
        outdated: bool,
    ) -> RemoteReviewThread {
        RemoteReviewThread {
            id: id.to_string(),
            path: "src/lib.rs".to_string(),
            line: Some(line),
            side: RemoteCommentSide::Right,
            is_resolved: resolved,
            is_outdated: outdated,
            comments: vec![RemoteReviewComment {
                id: format!("{id}-root"),
                author: Some(author.to_string()),
                body: body.to_string(),
                created_at: None,
                in_reply_to: None,
                url: "https://example.com/x".to_string(),
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
        App::build(
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
            None,
        )
        .expect("build app")
    }

    fn draw(app: &mut App) -> Buffer {
        let backend = TestBackend::new(140, 30);
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
    fn should_render_unresolved_remote_comment_inline_in_unified_diff() {
        // given a PR app with one unresolved remote thread anchored on
        // the addition line
        let mut app = make_pr_app();
        app.forge_review_threads = vec![thread("t1", "alice", "looks good?", 2, false, false)];
        app.rebuild_annotations();
        // when
        let buffer = draw(&mut app);
        // then — the badge appears somewhere in the rendered frame
        let body = body_text(&buffer);
        assert!(
            body.contains("[github @alice]"),
            "expected [github @alice] badge in:\n{body}"
        );
        assert!(
            body.contains("looks good?"),
            "expected remote comment body in:\n{body}"
        );
    }

    #[test]
    fn should_render_resolved_remote_comment_only_under_comments_all() {
        // given a PR app with one resolved remote thread
        let mut app = make_pr_app();
        app.forge_review_threads = vec![thread(
            "t1", "alice", "old note", 2, /* resolved */ true, false,
        )];
        // default Unresolved visibility — should not render
        app.rebuild_annotations();
        let before = body_text(&draw(&mut app));
        assert!(
            !before.contains("[github @alice"),
            "resolved thread leaked under Unresolved:\n{before}"
        );

        // when — flip to All
        assert!(app.set_remote_comments_visibility(PrCommentsVisibility::All));
        // then — the resolved badge appears with the "resolved" marker
        let after = body_text(&draw(&mut app));
        assert!(
            after.contains("[github @alice resolved]"),
            "expected resolved badge in:\n{after}"
        );
    }

    #[test]
    fn should_hide_all_remote_comments_when_comments_hide() {
        // given
        let mut app = make_pr_app();
        app.forge_review_threads = vec![thread("t1", "alice", "blocker", 2, false, false)];
        app.rebuild_annotations();
        // sanity: visible by default
        let before = body_text(&draw(&mut app));
        assert!(before.contains("[github @alice]"));

        // when
        assert!(app.set_remote_comments_visibility(PrCommentsVisibility::Hide));
        // then
        let after = body_text(&draw(&mut app));
        assert!(
            !after.contains("[github @alice"),
            "comment leaked under Hide:\n{after}"
        );
    }

    #[test]
    fn should_render_outdated_marker_for_outdated_thread_under_all() {
        // given
        let mut app = make_pr_app();
        app.forge_review_threads = vec![thread(
            "t1",
            "bob",
            "stale anchor",
            2,
            false,
            /* outdated */ true,
        )];
        // when — switch to all so the outdated thread is visible
        app.set_remote_comments_visibility(PrCommentsVisibility::All);
        let body = body_text(&draw(&mut app));
        // then
        assert!(
            body.contains("[github @bob outdated]"),
            "expected outdated badge in:\n{body}"
        );
    }
}
