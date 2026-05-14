use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use unicode_width::UnicodeWidthStr;

use crate::app::{App, ExpandDirection, FocusedPanel, GAP_EXPAND_BATCH, GapId, InputMode};
use crate::model::{LineOrigin, LineRange, LineSide};
use crate::theme::Theme;
use crate::ui::comment_panel;
use crate::ui::diff_view::{
    apply_horizontal_scroll, comment_type_presentation, cursor_indicator, cursor_indicator_spaced,
    diff_stat_title, is_line_highlighted, paint_unified_diff_rows_with,
    paint_visual_selection_overlay, populate_row_to_annotation, render_expander_line,
    render_hidden_lines, scroll_comment_input_into_view, unified_line_bg_style,
};
use crate::ui::styles;
use crate::vcs::git::calculate_gap;

pub(super) fn render_unified_diff(frame: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focused_panel == FocusedPanel::Diff;

    let title = if app.is_cursor_in_overview() || app.current_file_path().is_none() {
        " Diff (Unified) \u{2014} Overview ".to_string()
    } else {
        format!(
            " Diff (Unified) \u{2014} {} ",
            app.current_file_path().unwrap().display()
        )
    };

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

    // Build all diff lines for infinite scroll
    // Track line index to mark the current line (cursor position)
    let mut lines: Vec<Line> = Vec::new();
    let mut line_idx: usize = 0;
    let current_line_idx = app.diff_state.cursor_line;

    // Track cursor position for IME when in Comment mode
    // Store the logical line index and column where the cursor should be
    let mut comment_cursor_logical_line: Option<usize> = None;
    let mut comment_cursor_column: u16 = 0;
    // Track the full extent of the comment input box so we can auto-scroll
    // the viewport to keep it visible while the user types.
    let mut comment_input_box_range: Option<(usize, usize)> = None;

    let is_review_comment_mode =
        app.input_mode == InputMode::Comment && app.comment_is_review_level;

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
            );
            comment_cursor_logical_line = Some(line_idx + cursor_info.line_offset);
            comment_cursor_column = 1 + cursor_info.column;
            comment_input_box_range =
                Some((line_idx, line_idx + input_lines.len().saturating_sub(1)));
            let annotations_replaced = 2 + comment.content.split('\n').count();
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
        let path = file.display_path();
        let status = file.status.as_char();
        let is_reviewed = app.session.is_file_reviewed(path);

        // File header
        let indicator = cursor_indicator_spaced(line_idx, current_line_idx);

        // Add checkmark if reviewed (using same character as file list)
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

        // If file is reviewed, skip rendering the body (fold it away)
        if is_reviewed {
            continue;
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
                    );
                    // Track cursor position: logical line = current line_idx + cursor offset within input
                    comment_cursor_logical_line = Some(line_idx + cursor_info.line_offset);
                    // Column = indicator (1) + cursor_info.column
                    comment_cursor_column = 1 + cursor_info.column;
                    comment_input_box_range =
                        Some((line_idx, line_idx + input_lines.len().saturating_sub(1)));
                    let annotations_replaced = 2 + comment.content.split('\n').count();
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
                            render_expanded_context_line(
                                &mut lines,
                                &mut line_idx,
                                current_line_idx,
                                expanded_line,
                                &app.theme,
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
                            render_expanded_context_line(
                                &mut lines,
                                &mut line_idx,
                                current_line_idx,
                                expanded_line,
                                &app.theme,
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
                    let (prefix, base_style) = match diff_line.origin {
                        LineOrigin::Addition => ("+", styles::diff_add_style(&app.theme)),
                        LineOrigin::Deletion => ("-", styles::diff_del_style(&app.theme)),
                        LineOrigin::Context => (" ", styles::diff_context_style(&app.theme)),
                    };

                    let style = base_style;

                    let line_num_str = match diff_line.origin {
                        LineOrigin::Addition => diff_line
                            .new_lineno
                            .map(|n| format!("{n:>4} "))
                            .unwrap_or_else(|| "     ".to_string()),
                        LineOrigin::Deletion => diff_line
                            .old_lineno
                            .map(|n| format!("{n:>4} "))
                            .unwrap_or_else(|| "     ".to_string()),
                        _ => diff_line
                            .new_lineno
                            .or(diff_line.old_lineno)
                            .map(|n| format!("{n:>4} "))
                            .unwrap_or_else(|| "     ".to_string()),
                    };

                    let indicator = cursor_indicator(line_idx, current_line_idx);

                    let line_num_style = styles::dim_style(&app.theme);

                    let mut line_spans = vec![
                        Span::styled(indicator, styles::current_line_indicator_style(&app.theme)),
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
                                            );
                                        comment_cursor_logical_line =
                                            Some(line_idx + cursor_info.line_offset);
                                        comment_cursor_column = 1 + cursor_info.column;
                                        comment_input_box_range = Some((
                                            line_idx,
                                            line_idx + input_lines.len().saturating_sub(1),
                                        ));
                                        let annotations_replaced =
                                            2 + comment.content.split('\n').count();
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
                                    } else {
                                        let line_range = comment
                                            .line_range
                                            .or_else(|| Some(LineRange::single(old_ln)));
                                        let comment_lines = comment_panel::format_comment_lines(
                                            &app.theme,
                                            comment_type_presentation(app, &comment.comment_type),
                                            &comment.content,
                                            line_range,
                                        );
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
                                    }
                                }
                            }
                        }

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
                                );
                            comment_cursor_logical_line = Some(line_idx + cursor_info.line_offset);
                            comment_cursor_column = 1 + cursor_info.column;
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
                                            );
                                        comment_cursor_logical_line =
                                            Some(line_idx + cursor_info.line_offset);
                                        comment_cursor_column = 1 + cursor_info.column;
                                        comment_input_box_range = Some((
                                            line_idx,
                                            line_idx + input_lines.len().saturating_sub(1),
                                        ));
                                        let annotations_replaced =
                                            2 + comment.content.split('\n').count();
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
                                    } else {
                                        let line_range = comment
                                            .line_range
                                            .or_else(|| Some(LineRange::single(new_ln)));
                                        let comment_lines = comment_panel::format_comment_lines(
                                            &app.theme,
                                            comment_type_presentation(app, &comment.comment_type),
                                            &comment.content,
                                            line_range,
                                        );
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
                                    }
                                }
                            }
                        }

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
                                );
                            comment_cursor_logical_line = Some(line_idx + cursor_info.line_offset);
                            comment_cursor_column = 1 + cursor_info.column;
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
                        }
                    }
                }
            }
        }

        // Spacing between files
        let indicator = cursor_indicator(line_idx, current_line_idx);
        lines.push(Line::from(Span::styled(
            indicator,
            styles::current_line_indicator_style(&app.theme),
        )));
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

    app.diff_state.viewport_width = inner.width as usize;
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

/// Render a single expanded context line (shared by unified + side-by-side via unified path)
fn render_expanded_context_line(
    lines: &mut Vec<Line<'_>>,
    line_idx: &mut usize,
    current_line_idx: usize,
    expanded_line: &crate::model::DiffLine,
    theme: &Theme,
) {
    let indicator = cursor_indicator(*line_idx, current_line_idx);
    let line_num = expanded_line
        .new_lineno
        .map(|n| format!("{n:>4} "))
        .unwrap_or_else(|| "     ".to_string());
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
