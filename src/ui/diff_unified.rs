use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
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
    apply_horizontal_scroll, comment_box_visible, comment_type_presentation, cursor_indicator,
    cursor_indicator_spaced, diff_stat_title, hunk_header_text_and_style,
    paint_cursor_line_highlight, paint_unified_diff_rows_with, paint_visual_selection_overlay,
    populate_row_to_annotation, push_comment_bar, render_expander_line, render_hidden_lines,
    scroll_comment_input_into_view, skip_comment_box, unified_line_bg_style,
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
    let search_style = styles::search_match_style(&app.theme);

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

    crate::ui::pr_info_panel::append_pr_info_section(
        app,
        &mut lines,
        &mut line_idx,
        current_line_idx,
    );

    // The `═══ Review Comments ═══` label is redundant in single-file
    // view (review-level comments are still rendered below; they just
    // don't need a banner that confuses horizontal scroll). It's also hidden
    // while the section has no content.
    if app.show_review_comments_header() {
        let general_indicator = cursor_indicator_spaced(line_idx, current_line_idx);
        lines.push(Line::from(vec![
            Span::styled(
                general_indicator,
                styles::current_line_indicator_style(&app.theme),
            ),
            Span::styled(
                crate::ui::diff_view::REVIEW_COMMENTS_HEADER_PREFIX,
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
        let summary_lines = comment_panel::format_remote_review_summary_lines(
            &app.theme,
            summary,
            app.forge_kind(),
        );
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
                comment_width,
                app.comment_vim_mode_label()
                    .as_ref()
                    .map(|(t, w)| (t.as_str(), *w)),
                app.supports_keyboard_enhancement,
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
            let rows = App::comment_display_lines(comment, inner.width as usize);
            if !comment_box_visible(line_idx, rows, (visible_start, visible_end)) {
                skip_comment_box(&mut lines, &mut line_idx, rows);
                continue;
            }
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

    // Render remote review-level threads (general MR notes, line: None).
    {
        use crate::forge::remote_comments::{PrCommentsVisibility, RemoteCommentSide};
        let _ = RemoteCommentSide::Right; // ensure import is used
        let visibility = app.session.remote_comments_visibility;
        if !matches!(visibility, PrCommentsVisibility::Hide) {
            for thread in &app.forge_review_threads {
                if thread.line.is_some() {
                    continue; // inline threads are rendered in-diff
                }
                let Some(muted) = visibility.render_decision(thread) else {
                    continue;
                };
                let thread_lines = comment_panel::format_remote_thread_lines(
                    &app.theme,
                    thread,
                    muted,
                    app.forge_kind(),
                );
                for mut comment_line in thread_lines {
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
    }

    if is_review_comment_mode && app.editing_comment_id.is_none() {
        let (input_lines, cursor_info) = comment_panel::format_comment_input_lines(
            &app.theme,
            comment_type_presentation(app, &app.comment_type),
            &app.comment_buffer,
            app.comment_cursor,
            None,
            false,
            comment_width,
            app.comment_vim_mode_label()
                .as_ref()
                .map(|(t, w)| (t.as_str(), *w)),
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

    crate::ui::pr_info_panel::append_issue_comments_section(
        app,
        &mut lines,
        &mut line_idx,
        current_line_idx,
        comment_width,
        (visible_start, visible_end),
    );

    for (file_idx, file) in app.diff_files.iter().enumerate() {
        // Single-file view hides every file except the one the cursor is
        // currently on. Navigation (`}`/`{`, file list) flips
        // `current_file_idx` and the next render shows the new file.
        if app.is_single_file_view && file_idx != app.diff_state.current_file_idx {
            continue;
        }
        // File-tree include/exclude filters hide files from the diff too.
        // Must stay in lockstep with `App::file_render_height`, which counts
        // these files as zero lines.
        if !app.file_passes_filter(file) {
            continue;
        }
        let path = file.display_path();
        let is_reviewed = app.session.is_file_reviewed(path);

        // The `═══ filename ═══` separator is redundant in single-file
        // view: the status bar and file list already name the file, and
        // the wide bar of `═` characters confuses horizontal scrolling.
        if !app.is_single_file_view {
            let indicator = cursor_indicator_spaced(line_idx, current_line_idx);
            let header_text = crate::ui::diff_view::file_header_prefix_text(app, file);
            lines.push(Line::from(vec![
                Span::styled(indicator, styles::current_line_indicator_style(&app.theme)),
                Span::styled(header_text, styles::file_header_style(&app.theme)),
                Span::styled(
                    crate::ui::diff_view::HEADER_RULE,
                    styles::file_header_style(&app.theme),
                ),
            ]));
            line_idx += 1;
        }

        // Reviewed files normally collapse in continuous view. A summary jump
        // may reveal one target body without changing its reviewed marker.
        if app.should_collapse_file(file_idx) {
            continue;
        }
        if is_reviewed && app.is_single_file_view {
            let indicator = cursor_indicator(line_idx, current_line_idx);
            lines.push(Line::from(vec![
                Span::styled(indicator, styles::current_line_indicator_style(&app.theme)),
                Span::styled(
                    crate::ui::diff_view::REVIEWED_BANNER_TEXT,
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
                if !app.comment_visible(comment) {
                    continue;
                }
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
                        comment_width,
                        app.comment_vim_mode_label()
                            .as_ref()
                            .map(|(t, w)| (t.as_str(), *w)),
                        app.supports_keyboard_enhancement,
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
                    let rows = App::comment_display_lines(comment, inner.width as usize);
                    if !comment_box_visible(line_idx, rows, (visible_start, visible_end)) {
                        skip_comment_box(&mut lines, &mut line_idx, rows);
                        continue;
                    }
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
                comment_width,
                app.comment_vim_mode_label()
                    .as_ref()
                    .map(|(t, w)| (t.as_str(), *w)),
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

        if file.is_too_large || file.is_binary || file.hunks.is_empty() {
            let indicator = cursor_indicator_spaced(line_idx, current_line_idx);
            lines.push(Line::from(vec![
                Span::styled(indicator, styles::current_line_indicator_style(&app.theme)),
                Span::styled(
                    crate::ui::diff_view::binary_or_empty_label(file),
                    styles::dim_style(&app.theme),
                ),
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

                if gap > 0 && app.should_render_gap_before_hunk(file_idx, hunk_idx) {
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
                            let line_search = app
                                .search_paint_at(line_idx)
                                .map(|needle| (needle, search_style));
                            render_expanded_context_line(
                                &mut lines,
                                &mut line_idx,
                                current_line_idx,
                                expanded_line,
                                &app.theme,
                                lw,
                                app.relative_line_numbers,
                                line_search,
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
                            let line_search = app
                                .search_paint_at(line_idx)
                                .map(|needle| (needle, search_style));
                            render_expanded_context_line(
                                &mut lines,
                                &mut line_idx,
                                current_line_idx,
                                expanded_line,
                                &app.theme,
                                lw,
                                app.relative_line_numbers,
                                line_search,
                            );
                        }
                    }
                }

                // Hunk header
                let is_hunk_reviewed = app.is_hunk_reviewed(file_idx, hunk_idx);
                let (hunk_header_text, hunk_header_style) =
                    hunk_header_text_and_style(&app.theme, hunk, is_hunk_reviewed);
                let indicator = cursor_indicator_spaced(line_idx, current_line_idx);
                lines.push(Line::from(vec![
                    Span::styled(indicator, styles::current_line_indicator_style(&app.theme)),
                    Span::styled(hunk_header_text, hunk_header_style),
                ]));
                line_idx += 1;
                if app.should_collapse_hunk(file_idx, hunk_idx) {
                    continue;
                }

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
                        let base_style = match diff_line.origin {
                            LineOrigin::Addition => styles::diff_add_style(&app.theme),
                            LineOrigin::Deletion => styles::diff_del_style(&app.theme),
                            LineOrigin::Context => styles::diff_context_style(&app.theme),
                        };
                        let style = base_style;
                        // A commit message is prose, not code: render it without
                        // line numbers, matching the side-by-side view.
                        let line_num_str = if file.is_commit_message {
                            " ".repeat(lw + 1)
                        } else if app.relative_line_numbers {
                            crate::ui::diff_view::relative_line_number_field(
                                diff_line.new_lineno.or(diff_line.old_lineno),
                                line_idx,
                                current_line_idx,
                                lw,
                            )
                        } else {
                            crate::ui::diff_view::unified_line_number_field(diff_line, lw)
                        };
                        let prefix = crate::ui::diff_view::unified_line_origin_marker(diff_line);

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
                        let content_start = line_spans.len();

                        if let Some(ref highlighted) = diff_line.highlighted_spans {
                            for (span_style, span_text) in highlighted {
                                line_spans.push(Span::styled(span_text.clone(), *span_style));
                            }
                        } else {
                            line_spans.push(Span::styled(diff_line.content.clone(), style));
                        }

                        // Mark add/del lines with their effective EOL style so we can paint full
                        // row backgrounds later (including wrapped visual rows).
                        let eol_marker = matches!(
                            diff_line.origin,
                            LineOrigin::Addition | LineOrigin::Deletion
                        )
                        .then(|| {
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
                            Span::styled(String::new(), eol_style)
                        });

                        if let Some(needle) = app.search_paint_at(line_idx) {
                            let content_spans = line_spans.split_off(content_start);
                            line_spans.extend(crate::ui::text_utils::apply_search_highlight_spans(
                                content_spans,
                                needle,
                                search_style,
                            ));
                        }
                        line_spans.extend(eol_marker);

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
                                if comment.side == Some(LineSide::Old)
                                    && app.comment_visible(comment)
                                {
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
                                                comment_width,
                                                app.comment_vim_mode_label()
                                                    .as_ref()
                                                    .map(|(t, w)| (t.as_str(), *w)),
                                                app.supports_keyboard_enhancement,
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
                                        let box_top_row = line_idx;
                                        let rows = App::comment_display_lines(
                                            comment,
                                            inner.width as usize,
                                        );
                                        // The bar is recorded either way: it is
                                        // painted above the box, so it can be on
                                        // screen while the box itself is not.
                                        if !comment_box_visible(
                                            line_idx,
                                            rows,
                                            (visible_start, visible_end),
                                        ) {
                                            skip_comment_box(&mut lines, &mut line_idx, rows);
                                        } else {
                                            let comment_lines = comment_panel::format_comment_lines(
                                                &app.theme,
                                                comment_type_presentation(
                                                    app,
                                                    &comment.comment_type,
                                                ),
                                                &comment.content,
                                                line_range,
                                                comment_width,
                                                (comment.author != app.username)
                                                    .then_some(comment.author.as_str()),
                                            );
                                            for mut comment_line in comment_lines {
                                                let is_current = line_idx == current_line_idx;
                                                let indicator =
                                                    if is_current { "▶" } else { " " };
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
                                    comment_width,
                                    app.comment_vim_mode_label()
                                        .as_ref()
                                        .map(|(t, w)| (t.as_str(), *w)),
                                    app.supports_keyboard_enhancement,
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
                                if comment.side != Some(LineSide::Old)
                                    && app.comment_visible(comment)
                                {
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
                                                comment_width,
                                                app.comment_vim_mode_label()
                                                    .as_ref()
                                                    .map(|(t, w)| (t.as_str(), *w)),
                                                app.supports_keyboard_enhancement,
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
                                        let box_top_row = line_idx;
                                        let rows = App::comment_display_lines(
                                            comment,
                                            inner.width as usize,
                                        );
                                        // The bar is recorded either way: it is
                                        // painted above the box, so it can be on
                                        // screen while the box itself is not.
                                        if !comment_box_visible(
                                            line_idx,
                                            rows,
                                            (visible_start, visible_end),
                                        ) {
                                            skip_comment_box(&mut lines, &mut line_idx, rows);
                                        } else {
                                            let comment_lines = comment_panel::format_comment_lines(
                                                &app.theme,
                                                comment_type_presentation(
                                                    app,
                                                    &comment.comment_type,
                                                ),
                                                &comment.content,
                                                line_range,
                                                comment_width,
                                                (comment.author != app.username)
                                                    .then_some(comment.author.as_str()),
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
                                    comment_width,
                                    app.comment_vim_mode_label()
                                        .as_ref()
                                        .map(|(t, w)| (t.as_str(), *w)),
                                    app.supports_keyboard_enhancement,
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
                    | DiffSource::PullRequest(_)
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
                        let line_search = app
                            .search_paint_at(line_idx)
                            .map(|needle| (needle, search_style));
                        render_expanded_context_line(
                            &mut lines,
                            &mut line_idx,
                            current_line_idx,
                            expanded_line,
                            &app.theme,
                            lw,
                            app.relative_line_numbers,
                            line_search,
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
                        let line_search = app
                            .search_paint_at(line_idx)
                            .map(|needle| (needle, search_style));
                        render_expanded_context_line(
                            &mut lines,
                            &mut line_idx,
                            current_line_idx,
                            expanded_line,
                            &app.theme,
                            lw,
                            app.relative_line_numbers,
                            line_search,
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
                    crate::ui::diff_view::spacing_next_file_hint_text(&next_path),
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
    let viewport_width = inner.width as usize;
    let visible_lines_unscrolled_for_bg = visible_lines_unscrolled.clone();
    let gutter_wrap = app.gutter_wrap_active();
    // In gutter mode the expansion itself produces both the rendered rows and
    // the row counts, so renderer and geometry come from the same pass.
    let gutter_expansion = (gutter_wrap && viewport_width > 0).then(|| {
        let gutter_width = crate::app::unified_gutter(app.lineno_width()) as usize;
        let plans = crate::ui::word_wrap::unified_wrap_plans(
            app,
            scroll_offset,
            visible_lines_unscrolled.len(),
            gutter_width,
        );
        crate::ui::word_wrap::expand_gutter_wrap(
            visible_lines_unscrolled.clone(),
            &plans,
            viewport_width,
            inner.height as usize,
            &app.theme,
        )
    });
    // Publish the expansion's per-row content char ranges for selection
    // geometry (word-boundary rows hold fewer chars than the content
    // column). Each render clears the map, so flow mode and stale
    // windows use the uniform packing model. Keys use the same raw
    // rendered-index space that `populate_row_to_annotation` stores
    // (scroll_offset + rendered line). Producer and consumers therefore
    // agree when the comment input box shifts indices.
    app.gutter_sel_ranges.clear();
    if let Some(expansion) = &gutter_expansion {
        let pairs: Vec<(usize, Vec<(usize, usize)>)> = expansion
            .row_char_ranges
            .iter()
            .enumerate()
            .filter(|(_, ranges)| !ranges.is_empty())
            .map(|(i, ranges)| (scroll_offset + i, ranges.clone()))
            .collect();
        app.gutter_sel_ranges.extend(pairs);
    }
    // Single pass: wrap each logical line once, producing both the visual
    // rows to render and the per-line height used by every row-mapping
    // consumer below, so the two can't disagree.
    let (row_heights, wrapped_lines): (Vec<usize>, Option<Vec<Line>>) = match gutter_expansion {
        Some(expansion) => (expansion.row_heights, Some(expansion.rows)),
        None if wrap && viewport_width > 0 => {
            let mut heights = Vec::with_capacity(visible_lines_unscrolled_for_bg.len());
            let mut out: Vec<Line> = Vec::new();
            for line in &visible_lines_unscrolled_for_bg {
                let rows = crate::ui::text_utils::wrap_spans(&line.spans, viewport_width);
                heights.push(rows.len());
                out.extend(rows.into_iter().map(Line::from));
            }
            (heights, Some(out))
        }
        None => (vec![1; visible_lines_unscrolled_for_bg.len()], None),
    };
    app.diff_state.visible_line_count = populate_row_to_annotation(
        &mut app.diff_row_to_annotation,
        &row_heights,
        viewport_width,
        inner.height as usize,
        wrap,
        scroll_offset,
    );
    // Stale-geometry catch-up: ensure_cursor_visible ran at keypress time
    // with the previous frame's visible_line_count. When fresh wrapping
    // shrinks the window and leaves the cursor below the fold, correct
    // the scroll for the next frame and for post-frame state. Only
    // cursor motion arms it, view scrolls never do. Every painter in
    // this frame uses the local `scroll_offset`, so the frame stays
    // consistent and the lag is at most one frame.
    if wrap
        && app.input_mode != crate::app::InputMode::Comment
        && std::mem::take(&mut app.diff_state.scroll_catchup_armed)
    {
        let visible_now = app.diff_state.effective_visible_lines();
        if app.diff_state.cursor_line >= scroll_offset + visible_now {
            app.diff_state.scroll_offset = app.diff_state.cursor_line + 1 - visible_now;
            // The corrected window can shrink again. Stay armed until a
            // frame passes with no correction, so convergence completes.
            // View scrolls disarm explicitly and stop the chain.
            app.diff_state.scroll_catchup_armed = true;
        }
    } else {
        app.diff_state.scroll_catchup_armed = false;
    }

    let max_scroll_x = max_content_width.saturating_sub(viewport_width);
    if app.diff_state.scroll_x > max_scroll_x {
        app.diff_state.scroll_x = max_scroll_x;
    }
    if app.diff_state.wrap_lines {
        app.diff_state.scroll_x = 0;
    }

    let scroll_x = app.diff_state.scroll_x;
    let visible_lines: Vec<Line> = match wrapped_lines {
        Some(out) => out,
        None => visible_lines_unscrolled
            .into_iter()
            .map(|line| apply_horizontal_scroll(line, scroll_x))
            .collect(),
    };

    // Paint per-visual-row add/del backgrounds across full row width.
    paint_unified_diff_rows_with(
        frame,
        inner,
        &visible_lines_unscrolled_for_bg,
        &row_heights,
        |_idx, line| unified_line_bg_style(line, &app.theme),
    );

    let overlay_ctx = crate::ui::diff_view::DiffOverlayPaint {
        inner,
        visible_lines_unscrolled: &visible_lines_unscrolled_for_bg,
        line_widths: &line_widths,
        row_heights: &row_heights,
        wrap_lines: app.diff_state.wrap_lines,
        viewport_width: inner.width as usize,
        scroll_x,
        // The frame's own offset, never the possibly caught-up live value:
        // overlays must match the row list this frame rendered.
        scroll_offset,
        theme: &app.theme,
        comment_bars: &comment_bars,
    };

    // Section-marker row tint (hunk headers + expand/hidden stubs). Painted
    // before the paragraph so cursor-line and selection overlays still win
    // on the active row.
    crate::ui::diff_view::paint_section_highlight(frame, &overlay_ctx);

    // Keep paragraph bg unset so pre-painted per-row diff backgrounds remain visible.
    let diff = Paragraph::new(visible_lines).style(Style::default().fg(app.theme.fg_primary));
    frame.render_widget(diff, inner);

    // Cursor-line bg has to land after the paragraph: spans on +/- lines carry
    // explicit diff_add_bg/diff_del_bg that would mask a pre-paint over the code.
    paint_cursor_line_highlight(
        frame,
        inner,
        &visible_lines_unscrolled_for_bg,
        &row_heights,
        app,
        scroll_offset,
    );

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
                // Sum the word-wrap-accurate heights of the lines before the
                // cursor so the terminal cursor lands on the right visual row.
                for i in 0..logical_offset {
                    visual_row += row_heights.get(i).copied().unwrap_or(1) as u16;
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
        let thread_lines =
            comment_panel::format_remote_thread_lines(&app.theme, thread, muted, app.forge_kind());
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
#[allow(clippy::too_many_arguments)]
fn render_expanded_context_line(
    lines: &mut Vec<Line<'_>>,
    line_idx: &mut usize,
    current_line_idx: usize,
    expanded_line: &crate::model::DiffLine,
    theme: &Theme,
    lw: usize,
    relative_line_numbers: bool,
    search: Option<(&str, Style)>,
) {
    let indicator = cursor_indicator(*line_idx, current_line_idx);
    let line_num = if relative_line_numbers {
        crate::ui::diff_view::relative_line_number_field(
            expanded_line.new_lineno,
            *line_idx,
            current_line_idx,
            lw,
        )
    } else {
        crate::ui::diff_view::expanded_context_lineno_field(expanded_line, lw)
    };
    let mut line_spans = vec![
        Span::styled(indicator, styles::current_line_indicator_style(theme)),
        Span::styled(line_num, styles::expanded_context_style(theme)),
        Span::styled("  ", styles::expanded_context_style(theme)),
    ];
    let content_start = line_spans.len();
    line_spans.push(Span::styled(
        expanded_line.content.clone(),
        styles::expanded_context_style(theme),
    ));
    if let Some((needle, hl)) = search {
        let content_spans = line_spans.split_off(content_start);
        line_spans.extend(crate::ui::text_utils::apply_search_highlight_spans(
            content_spans,
            needle,
            hl,
        ));
    }
    lines.push(Line::from(line_spans));
    *line_idx += 1;
}

#[cfg(test)]
mod remote_comments_snapshot_tests {
    //! Render-snapshot tests for inline remote review threads in the
    //! unified diff. We drive `ui::render` against `TestBackend` and check
    //! for the provider badge text on the expected row.
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
    use ratatui::layout::Rect;
    use std::path::{Path, PathBuf};

    pub(super) struct SnapshotVcs {
        pub(super) info: VcsInfo,
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

    pub(super) fn header_only_diff_file_at(path: &str) -> DiffFile {
        let hunks = Vec::new();
        let content_hash = DiffFile::compute_content_hash(&hunks);
        DiffFile {
            old_path: Some(PathBuf::from(path)),
            new_path: Some(PathBuf::from(path)),
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

    pub(super) fn make_revision_app(diff_files: Vec<DiffFile>) -> App {
        let vcs_info = VcsInfo {
            root_path: PathBuf::from("/tmp/tuicr"),
            head_commit: "headsha".to_string(),
            branch_name: None,
            vcs_type: VcsType::Git,
        };
        let session = ReviewSession::new(
            vcs_info.root_path.clone(),
            "headsha".to_string(),
            None,
            SessionDiffSource::CommitRange,
        );
        App::build(
            Box::new(SnapshotVcs {
                info: vcs_info.clone(),
            }),
            vcs_info,
            Theme::dark(),
            None,
            false,
            diff_files,
            session,
            DiffSource::CommitRange(vec!["HEAD".to_string()]),
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

    fn draw_unified_diff(app: &mut App) -> Buffer {
        let backend = TestBackend::new(100, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| super::render_unified_diff(frame, app, Rect::new(0, 0, 100, 12)))
            .expect("draw unified diff");
        terminal.backend().buffer().clone()
    }

    pub(super) fn body_text(buffer: &Buffer) -> String {
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn commit_message_file(message: &str) -> DiffFile {
        let lines: Vec<DiffLine> = message
            .lines()
            .enumerate()
            .map(|(i, line)| DiffLine {
                origin: LineOrigin::Context,
                content: line.to_string(),
                old_lineno: None,
                new_lineno: Some(i as u32 + 1),
                highlighted_spans: None,
            })
            .collect();
        let new_count = lines.len() as u32;
        let hunks = vec![DiffHunk {
            header: String::new(),
            lines,
            old_start: 0,
            old_count: 0,
            new_start: 1,
            new_count,
        }];
        let content_hash = DiffFile::compute_content_hash(&hunks);
        DiffFile {
            old_path: None,
            new_path: Some(PathBuf::from("Commit Message (abc1234)")),
            status: FileStatus::Added,
            hunks,
            is_binary: false,
            is_too_large: false,
            is_commit_message: true,
            content_hash,
        }
    }

    #[test]
    fn should_render_commit_message_without_line_numbers_in_unified() {
        let mut app = make_revision_app(vec![commit_message_file(
            "COMMITMSG summary\n\nsecond body line",
        )]);
        let buf = draw_unified_diff(&mut app);

        let mut checked = 0;
        for y in 0..buf.area.height {
            let cells: Vec<String> = (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect();
            let row: String = cells.concat();
            let Some(byte_col) = row
                .find("COMMITMSG")
                .or_else(|| row.find("second body line"))
            else {
                continue;
            };
            checked += 1;
            // No line-number gutter: nothing but whitespace precedes the text.
            let col = row[..byte_col].chars().count();
            let gutter: String = cells[..col].concat();
            assert!(
                gutter.chars().all(|c| c.is_whitespace() || c == '│'),
                "commit message row {y} should have no line number, got gutter {gutter:?}"
            );
        }
        assert_eq!(
            checked, 2,
            "expected both message body lines to render, got {checked}"
        );
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

    // Revision diffs with `wrap = true` render the file-header rule without a
    // cursor gutter. The right-edge fill overlay must measure that exact row:
    // treating it like guttered diff content truncated `README.md [M]` to
    // `README` in `tuicr -r HEAD`.
    #[test]
    fn should_render_full_file_header_for_revision_diff() {
        let mut app = make_revision_app(vec![header_only_diff_file_at("README.md")]);
        app.diff_state.wrap_lines = true;

        let body = body_text(&draw_unified_diff(&mut app));

        assert!(
            body.contains("═══ README.md [M] "),
            "expected full README.md file header in:\n{body}"
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

    #[test]
    fn should_render_review_level_remote_thread_in_review_comments_section() {
        // given — a review-level thread (line: None, path: "") as produced by
        // GitLab individual_note: true discussions
        let mut app = make_pr_app();
        app.forge_review_threads = vec![RemoteReviewThread {
            id: "rv1".to_string(),
            path: String::new(),
            line: None,
            side: RemoteCommentSide::Right,
            is_resolved: false,
            is_outdated: false,
            comments: vec![RemoteReviewComment {
                id: "rv1-root".to_string(),
                author: Some("carol".to_string()),
                body: "overall this looks fine".to_string(),
                created_at: None,
                in_reply_to: None,
                url: String::new(),
            }],
        }];
        app.rebuild_annotations();
        // when
        let buffer = draw(&mut app);
        let body = body_text(&buffer);
        // then — the badge and body appear in the rendered frame
        assert!(
            body.contains("carol"),
            "expected author in review comments:\n{body}"
        );
        assert!(
            body.contains("overall this looks fine"),
            "expected body in review comments:\n{body}"
        );
    }

    #[test]
    fn should_not_render_review_level_thread_when_comments_hidden() {
        let mut app = make_pr_app();
        app.forge_review_threads = vec![RemoteReviewThread {
            id: "rv1".to_string(),
            path: String::new(),
            line: None,
            side: RemoteCommentSide::Right,
            is_resolved: false,
            is_outdated: false,
            comments: vec![RemoteReviewComment {
                id: "rv1-root".to_string(),
                author: Some("carol".to_string()),
                body: "should be hidden".to_string(),
                created_at: None,
                in_reply_to: None,
                url: String::new(),
            }],
        }];
        app.set_remote_comments_visibility(PrCommentsVisibility::Hide);
        let buffer = draw(&mut app);
        let body = body_text(&buffer);
        assert!(
            !body.contains("should be hidden"),
            "review-level thread leaked under Hide:\n{body}"
        );
    }

    #[test]
    fn should_wrap_long_line_in_unified_view_when_wrap_enabled() {
        let long: String = "x".repeat(200);
        let hunk = DiffHunk {
            header: "@@ -0,0 +1,1 @@".to_string(),
            lines: vec![DiffLine {
                origin: LineOrigin::Addition,
                content: long.clone(),
                old_lineno: None,
                new_lineno: Some(1),
                highlighted_spans: None,
            }],
            old_start: 0,
            old_count: 0,
            new_start: 1,
            new_count: 1,
        };
        let hunks = vec![hunk];
        let content_hash = DiffFile::compute_content_hash(&hunks);
        let file = DiffFile {
            old_path: Some(PathBuf::from("src/lib.rs")),
            new_path: Some(PathBuf::from("src/lib.rs")),
            status: FileStatus::Modified,
            hunks,
            is_binary: false,
            is_too_large: false,
            is_commit_message: false,
            content_hash,
        };
        let mut app = make_revision_app(vec![file]);
        app.set_diff_wrap(true);
        app.rebuild_annotations();

        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| super::render_unified_diff(frame, &mut app, Rect::new(0, 0, 80, 20)))
            .expect("draw");
        let body = body_text(terminal.backend().buffer());

        let tail: String = long.chars().rev().take(20).collect::<String>();
        let tail: String = tail.chars().rev().collect();
        assert!(
            body.contains(&tail),
            "tail of wrapped long line should appear in body:\n{body}"
        );

        assert!(
            app.diff_state.visible_line_count > 0 && app.diff_state.visible_line_count < 20,
            "expected logical visible_line_count 1..20, got {}",
            app.diff_state.visible_line_count
        );
    }

    /// Comment boxes outside the viewport are replaced with blank placeholder
    /// rows instead of being formatted. The rows still have to be there, and in
    /// the right number, or every row below the comment would shift — so this
    /// also scrolls to the row `line_annotations` assigned the comment and
    /// expects the box to be there.
    #[test]
    fn should_cull_comment_boxes_outside_the_viewport() {
        use crate::app::AnnotatedLine;
        use crate::model::{Comment, CommentType};

        const NEEDLE: &str = "far-below-the-fold";

        let lines: Vec<DiffLine> = (1..=120)
            .map(|n| DiffLine {
                origin: LineOrigin::Addition,
                content: format!("line {n}"),
                old_lineno: None,
                new_lineno: Some(n),
                highlighted_spans: None,
            })
            .collect();
        let hunks = vec![DiffHunk {
            header: "@@ -0,0 +1,120 @@".to_string(),
            lines,
            old_start: 0,
            old_count: 0,
            new_start: 1,
            new_count: 120,
        }];
        let content_hash = DiffFile::compute_content_hash(&hunks);
        let path = PathBuf::from("src/lib.rs");
        let file = DiffFile {
            old_path: Some(path.clone()),
            new_path: Some(path.clone()),
            status: FileStatus::Modified,
            hunks,
            is_binary: false,
            is_too_large: false,
            is_commit_message: false,
            content_hash,
        };

        let mut app = make_revision_app(vec![file]);
        app.session
            .get_file_mut(&path)
            .expect("file registered in session")
            .add_line_comment(
                100,
                Comment::new(NEEDLE.to_string(), CommentType::from_id("note"), None),
            );
        app.rebuild_annotations();

        // Top of the file: the comment is ~100 rows below a 12-row viewport.
        let buffer = draw_unified_diff(&mut app);
        let body = body_text(&buffer);
        assert!(
            !body.contains(NEEDLE),
            "off-screen comment should not be visible:\n{body}"
        );
        assert!(
            body.contains("line 1"),
            "diff content should still render:\n{body}"
        );

        // Scroll to the row the annotation builder says the comment occupies.
        // If the culled box had emitted the wrong number of placeholder rows,
        // this index would point somewhere else and the body would not appear.
        let comment_row = app
            .line_annotations
            .iter()
            .position(|a| matches!(a, AnnotatedLine::LineComment { .. }))
            .expect("comment annotated in the document");
        app.diff_state.scroll_offset = comment_row;
        app.diff_state.cursor_line = comment_row;

        let buffer = draw_unified_diff(&mut app);
        let body = body_text(&buffer);
        assert!(
            body.contains(NEEDLE),
            "comment scrolled into view should render at its annotated row:\n{body}"
        );
    }

    #[test]
    fn should_reach_last_line_scrolling_down_through_wrapped_content() {
        // Many long lines that wrap to several visual rows each, so far fewer
        // logical lines fit per screen than the viewport height. This is
        // what makes `visible_line_count` (wrap-aware) diverge sharply from
        // `viewport_height`. A short, uniquely-named last line lets us detect
        // whether repeated `j` ever scrolls it into view.
        let long: String = "x".repeat(200);
        let mut lines: Vec<DiffLine> = (0..30)
            .map(|i| DiffLine {
                origin: LineOrigin::Addition,
                content: long.clone(),
                old_lineno: None,
                new_lineno: Some(i + 1),
                highlighted_spans: None,
            })
            .collect();
        lines.push(DiffLine {
            origin: LineOrigin::Addition,
            content: "LASTLINEMARKER".to_string(),
            old_lineno: None,
            new_lineno: Some(31),
            highlighted_spans: None,
        });
        let hunk = DiffHunk {
            header: "@@ -0,0 +1,31 @@".to_string(),
            lines,
            old_start: 0,
            old_count: 0,
            new_start: 1,
            new_count: 31,
        };
        let hunks = vec![hunk];
        let content_hash = DiffFile::compute_content_hash(&hunks);
        let file = DiffFile {
            old_path: Some(PathBuf::from("src/lib.rs")),
            new_path: Some(PathBuf::from("src/lib.rs")),
            status: FileStatus::Modified,
            hunks,
            is_binary: false,
            is_too_large: false,
            is_commit_message: false,
            content_hash,
        };
        let mut app = make_revision_app(vec![file]);
        app.set_diff_wrap(true);
        app.rebuild_annotations();

        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();

        // Drive `j` one keypress at a time, re-rendering between presses so
        // `visible_line_count` is refreshed the way it would be in the real
        // render loop. Far more presses than there are logical lines, so a
        // working implementation has ample opportunity to reach the end.
        let max_presses = app.total_lines() * 3;
        for _ in 0..max_presses {
            terminal
                .draw(|frame| super::render_unified_diff(frame, &mut app, Rect::new(0, 0, 80, 20)))
                .expect("draw");
            app.cursor_down(1);
        }
        terminal
            .draw(|frame| super::render_unified_diff(frame, &mut app, Rect::new(0, 0, 80, 20)))
            .expect("draw");
        let body = body_text(terminal.backend().buffer());

        assert_eq!(
            app.diff_state.cursor_line,
            app.max_cursor_line(),
            "cursor should saturate at the last navigable line"
        );
        assert!(
            body.contains("LASTLINEMARKER"),
            "scrolling down should eventually reveal the last line; view got stuck:\n{body}"
        );
    }

    #[test]
    fn should_extend_comment_bar_over_wrapped_rows_when_wrap_enabled() {
        use crate::model::{Comment, CommentType};

        let long: String = "x".repeat(200);
        let hunk = DiffHunk {
            header: "@@ -0,0 +1,1 @@".to_string(),
            lines: vec![DiffLine {
                origin: LineOrigin::Addition,
                content: long.clone(),
                old_lineno: None,
                new_lineno: Some(1),
                highlighted_spans: None,
            }],
            old_start: 0,
            old_count: 0,
            new_start: 1,
            new_count: 1,
        };
        let hunks = vec![hunk];
        let content_hash = DiffFile::compute_content_hash(&hunks);
        let path = PathBuf::from("src/lib.rs");
        let file = DiffFile {
            old_path: Some(path.clone()),
            new_path: Some(path.clone()),
            status: FileStatus::Modified,
            hunks,
            is_binary: false,
            is_too_large: false,
            is_commit_message: false,
            content_hash,
        };
        let mut app = make_revision_app(vec![file]);
        app.set_diff_wrap(true);

        let file_review = app
            .session
            .get_file_mut(&path)
            .expect("file registered in session");
        file_review.add_line_comment(
            1,
            Comment::new(
                "needs a rename".to_string(),
                CommentType::from_id("note"),
                None,
            ),
        );
        app.rebuild_annotations();

        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| super::render_unified_diff(frame, &mut app, Rect::new(0, 0, 80, 20)))
            .expect("draw");
        let buffer = terminal.backend().buffer();

        let mut cap: Option<(u16, u16)> = None;
        let mut cap_count = 0;
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                if buffer[(x, y)].symbol() == "╭" {
                    cap = Some((x, y));
                    cap_count += 1;
                }
            }
        }
        assert_eq!(
            cap_count, 1,
            "expected exactly one ╭ cap in the gutter, got {cap_count}"
        );
        let (bar_x, cap_y) = cap.unwrap();

        let mut box_top_y: Option<u16> = None;
        for y in (cap_y + 1)..buffer.area.height {
            if buffer[(bar_x, y)].symbol() == "├" {
                box_top_y = Some(y);
                break;
            }
        }
        let box_top_y = box_top_y.expect("expected ├ box top below the ╭ cap");
        assert!(
            box_top_y > cap_y + 1,
            "test needs at least one row between cap and box top; cap_y={cap_y} box_top_y={box_top_y}"
        );

        for y in (cap_y + 1)..box_top_y {
            let glyph = buffer[(bar_x, y)].symbol();
            assert_eq!(
                glyph, "│",
                "expected │ at ({bar_x},{y}) between cap ({cap_y}) and box top ({box_top_y}), got {glyph:?}"
            );
        }
    }
}

#[cfg(test)]
mod gutter_wrap_snapshot_tests {
    //! Render-snapshot tests for `wrap_style = "gutter"`: continuation rows
    //! keep the gutter (↪ in the lineno column + origin prefix), headers
    //! never split, and per-row backgrounds/highlights track the expansion.
    use super::remote_comments_snapshot_tests::{
        body_text, header_only_diff_file_at, make_revision_app,
    };
    use crate::app::{App, WrapStyle};
    use crate::model::{DiffFile, DiffHunk, DiffLine, FileStatus, LineOrigin};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use std::path::PathBuf;

    fn long_line_diff_file(content: &str) -> DiffFile {
        let lines = vec![
            DiffLine {
                origin: LineOrigin::Context,
                content: "short context".to_string(),
                old_lineno: Some(1),
                new_lineno: Some(1),
                highlighted_spans: None,
            },
            DiffLine {
                origin: LineOrigin::Addition,
                content: content.to_string(),
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

    fn gutter_app(diff_files: Vec<DiffFile>) -> App {
        let mut app = make_revision_app(diff_files);
        app.diff_state.wrap_style = WrapStyle::Gutter;
        app.diff_state.wrap_lines = true;
        app
    }

    /// A file of `n` additions, each long enough to wrap several times.
    fn many_wrapped_additions_file(n: usize) -> DiffFile {
        let lines: Vec<DiffLine> = (0..n)
            .map(|i| DiffLine {
                origin: LineOrigin::Addition,
                content: format!("line{i:02} {}", "wrapped content ".repeat(8)),
                old_lineno: None,
                new_lineno: Some(i as u32 + 1),
                highlighted_spans: None,
            })
            .collect();
        let hunk = DiffHunk {
            header: format!("@@ -0,0 +1,{n} @@"),
            lines,
            old_start: 0,
            old_count: 0,
            new_start: 1,
            new_count: n as u32,
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

    fn draw_at(app: &mut App, width: u16, height: u16) -> Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| super::render_unified_diff(frame, app, Rect::new(0, 0, width, height)))
            .expect("draw unified diff");
        terminal.backend().buffer().clone()
    }

    fn rows(buffer: &Buffer) -> Vec<String> {
        body_text(buffer).lines().map(str::to_string).collect()
    }

    #[test]
    fn should_render_continuation_gutter_for_wrapped_addition() {
        let mut app = gutter_app(vec![long_line_diff_file(&"a".repeat(120))]);
        let buffer = draw_at(&mut app, 40, 12);
        let body = body_text(&buffer);
        // Continuation row: ↪ in the lineno column, then the carried origin
        // prefix, then content at the content column — not at column 0.
        assert!(
            body.contains("↪▌ a"),
            "expected gutter-aligned continuation row:\n{body}"
        );
        let continuation = rows(&buffer)
            .into_iter()
            .find(|row| row.contains('↪'))
            .expect("a continuation row exists");
        // First cell inside the panel border is the blank indicator slot.
        assert_eq!(
            continuation.chars().nth(1),
            Some(' '),
            "continuation keeps the indicator slot blank: {continuation:?}"
        );
    }

    #[test]
    fn should_map_all_visual_rows_of_wrapped_line_to_same_annotation() {
        let mut app = gutter_app(vec![long_line_diff_file(&"b".repeat(120))]);
        let buffer = draw_at(&mut app, 40, 12);
        // The wrapped addition occupies as many consecutive identical
        // entries in the hit-test map as the buffer shows ↪ rows + 1.
        let marker_rows = rows(&buffer).iter().filter(|row| row.contains('↪')).count();
        assert!(marker_rows >= 2, "expected at least 2 continuation rows");
        let mut counts = std::collections::HashMap::new();
        for &idx in &app.diff_row_to_annotation {
            *counts.entry(idx).or_insert(0usize) += 1;
        }
        let max_duplicates = counts.values().copied().max().unwrap_or(0);
        assert_eq!(
            max_duplicates,
            marker_rows + 1,
            "hit-test rows must match rendered rows (marker rows {marker_rows} + first row)"
        );
    }

    #[test]
    fn should_never_split_file_header_in_gutter_mode() {
        // Narrow viewport, long path: the header would wrap under flow mode.
        let mut app = gutter_app(vec![header_only_diff_file_at(
            "src/some/deeply/nested/path/with_a_rather_long_name.rs",
        )]);
        let buffer = draw_at(&mut app, 30, 12);
        let body = body_text(&buffer);
        // The path appears on exactly one row (clipped at the panel edge),
        // never spilling onto a second row the way the wrapped header did.
        let header_rows = rows(&buffer)
            .iter()
            .filter(|row| row.contains("src/some"))
            .count();
        assert_eq!(
            header_rows, 1,
            "header must occupy exactly one row:\n{body}"
        );
        assert!(
            !body.contains("long_name"),
            "clipped header tail must not spill onto another row:\n{body}"
        );
        assert!(
            !body.contains('↪'),
            "decoration lines must never grow continuation rows:\n{body}"
        );
    }

    #[test]
    fn should_keep_short_file_header_intact_in_gutter_mode() {
        let mut app = gutter_app(vec![header_only_diff_file_at("README.md")]);
        let buffer = draw_at(&mut app, 60, 12);
        let body = body_text(&buffer);
        assert!(
            body.contains("═══ README.md [M] "),
            "expected full README.md file header in:\n{body}"
        );
    }

    #[test]
    fn should_keep_flow_continuations_at_column_zero_when_wrap_style_flow() {
        let mut app = make_revision_app(vec![long_line_diff_file(&"c".repeat(120))]);
        app.diff_state.wrap_lines = true; // default Flow style
        let buffer = draw_at(&mut app, 40, 12);
        let body = body_text(&buffer);
        assert!(
            !body.contains('↪'),
            "flow mode must not grow gutter markers:\n{body}"
        );
        // Flow continuations start at the panel's first inner column (right
        // after the border), under the gutter.
        assert!(
            rows(&buffer).iter().any(|row| row.starts_with("│c")),
            "flow continuation rows start at the inner left edge:\n{body}"
        );
    }

    #[test]
    fn should_paint_add_background_across_continuation_rows() {
        let mut app = gutter_app(vec![long_line_diff_file(&"d".repeat(120))]);
        let add_bg = app.theme.diff_add_bg;
        let buffer = draw_at(&mut app, 40, 12);
        let continuation_y = rows(&buffer)
            .iter()
            .position(|row| row.contains('↪'))
            .expect("a continuation row exists") as u16;
        // Last column inside the panel border carries the add background even
        // though the wrapped content ends earlier.
        assert_eq!(
            buffer[(buffer.area.width - 2, continuation_y)].style().bg,
            Some(add_bg),
            "continuation row right edge must carry the add background"
        );
    }

    #[test]
    fn should_highlight_all_visual_rows_of_cursor_line_in_gutter_mode() {
        let mut app = gutter_app(vec![long_line_diff_file(&"e".repeat(120))]);
        let cursor_bg = app.theme.cursor_line_bg;
        // Move the cursor onto the wrapped addition (last navigable line).
        app.diff_state.cursor_line = app.max_cursor_line();
        let buffer = draw_at(&mut app, 40, 12);
        let continuation_y = rows(&buffer)
            .iter()
            .position(|row| row.contains('↪'))
            .expect("a continuation row exists");
        // The wrapped line's first visual row sits directly above its first
        // continuation row; both must carry the cursor background on the
        // first inner column.
        for y in [continuation_y - 1, continuation_y] {
            assert_eq!(
                buffer[(1, y as u16)].style().bg,
                Some(cursor_bg),
                "row {y} of the cursor line must carry the cursor background"
            );
        }
    }

    #[test]
    fn should_render_cjk_diff_line_in_gutter_mode_without_panic() {
        let mut app = gutter_app(vec![long_line_diff_file(&"漢字テスト".repeat(12))]);
        let buffer = draw_at(&mut app, 40, 12);
        let body = body_text(&buffer);
        assert!(
            body.contains('↪'),
            "CJK content must wrap with gutter continuations:\n{body}"
        );
    }

    #[test]
    fn should_keep_catchup_frame_consistent_after_cursor_walk() {
        // A j-walk into heavy wrap arms the catch-up. The first frame
        // after the walk paints with the old offset. In that frame, each
        // row that carries the cursor background must map to the cursor
        // line: the catch-up must not shift overlays mid-render. The
        // second frame shows the corrected viewport.
        let mut app = gutter_app(vec![long_line_diff_file(&"g".repeat(200))]);
        let cursor_bg = app.theme.cursor_line_bg;
        draw_at(&mut app, 40, 10);
        let max = app.max_cursor_line();
        for _ in 0..max {
            app.cursor_down(1);
        }
        let buffer = draw_at(&mut app, 40, 10);
        // Inner rows start below the top border, so buffer row y maps to
        // hit-map index y - 1.
        for y in 1..buffer.area.height - 1 {
            if buffer[(1, y)].style().bg == Some(cursor_bg) {
                let ann = app.diff_row_to_annotation.get(y as usize - 1).copied();
                assert_eq!(
                    ann,
                    Some(app.diff_state.cursor_line),
                    "row {y} carries the cursor background but maps elsewhere"
                );
            }
        }
        draw_at(&mut app, 40, 10);
        assert!(app.is_cursor_visible(), "second frame shows the cursor");
    }

    #[test]
    fn should_keep_gutter_wrap_correct_while_comment_input_box_is_open() {
        // The wrap planner maps rendered rows to annotations through the
        // comment-input-box offset. With the box open on the context line
        // (above the wrapped addition), the addition must still expand with
        // gutter continuations and decoration rows must stay exempt.
        let mut app = gutter_app(vec![long_line_diff_file(&"f".repeat(120))]);
        app.enter_comment_mode(false, Some((1, crate::model::LineSide::New)));
        let buffer = draw_at(&mut app, 40, 16);
        let body = body_text(&buffer);
        assert!(
            body.contains("↪▌ f"),
            "wrapped addition keeps gutter continuations below the open box:\n{body}"
        );
        // Match the decoration form only: since v0.24 the panel border title
        // also carries the file name, which is not a rendered header row.
        let header_rows = rows(&buffer)
            .iter()
            .filter(|row| row.contains("═══ src/lib.rs"))
            .count();
        assert_eq!(
            header_rows, 1,
            "file header stays a single row with the box open:\n{body}"
        );
    }

    #[test]
    fn should_swap_render_modes_when_wrap_toggles_at_runtime() {
        let mut app = gutter_app(vec![long_line_diff_file(&"g".repeat(120))]);
        // Gutter wrap on: continuation markers present.
        let body_on = body_text(&draw_at(&mut app, 40, 12));
        assert!(
            body_on.contains('↪'),
            "expected markers while on:\n{body_on}"
        );

        // :set wrap! off — clipped single rows, no markers, no flow wrap.
        app.set_diff_wrap(false);
        let body_off = body_text(&draw_at(&mut app, 40, 12));
        assert!(
            !body_off.contains('↪'),
            "no markers with wrap off:\n{body_off}"
        );

        // Back on: gutter expansion resumes.
        app.set_diff_wrap(true);
        let body_back = body_text(&draw_at(&mut app, 40, 12));
        assert!(
            body_back.contains('↪'),
            "markers return after toggling wrap back on:\n{body_back}"
        );
    }

    #[test]
    fn should_show_last_line_after_jump_to_bottom_with_heavy_wrapping() {
        // Regression (test campaign, probe 1): jump_to_bottom positioned the
        // scroll using the raw viewport height in logical-line units, so
        // with wrapped lines the cursor and the last line landed far below
        // the fold.
        let mut app = gutter_app(vec![many_wrapped_additions_file(14)]);
        // First draw establishes the wrap-aware visible_line_count.
        let _ = draw_at(&mut app, 40, 12);
        app.jump_to_bottom();
        // Geometry is computed lazily during render, so the first frame
        // after a jump may still under-scroll; the render's catch-up clamp
        // corrects the scroll for the following frame — same cadence as
        // the event loop's next redraw.
        let _ = draw_at(&mut app, 40, 12);
        let buffer = draw_at(&mut app, 40, 12);
        let body = body_text(&buffer);
        assert!(
            body.contains("line13"),
            "last line must be on screen after G:\n{body}"
        );
        assert!(
            app.is_cursor_visible(),
            "cursor must be visible after G (scroll {}, cursor {}, visible {})",
            app.diff_state.scroll_offset,
            app.diff_state.cursor_line,
            app.diff_state.visible_line_count
        );
    }

    #[test]
    fn should_keep_cursor_on_screen_walking_j_across_wrapped_lines() {
        // Regression (test campaign, probe 2): the scroll-catch-up cap in
        // cursor_down assumed one visual row per logical line, accumulating
        // a scroll deficit on wrapped lines until the cursor walked off
        // screen.
        let mut app = gutter_app(vec![many_wrapped_additions_file(14)]);
        let _ = draw_at(&mut app, 40, 12);
        let max = app.max_cursor_line();
        for step in 0..max {
            app.cursor_down(1);
            // One frame of catch-up is allowed (lazy geometry; the render's
            // clamp corrects the scroll for the next frame). What must
            // never happen is the old unbounded deficit that walked the
            // cursor permanently off screen.
            let _ = draw_at(&mut app, 40, 12);
            let _ = draw_at(&mut app, 40, 12);
            assert!(
                app.is_cursor_visible(),
                "cursor off screen at step {step}: scroll {}, cursor {}, visible {}",
                app.diff_state.scroll_offset,
                app.diff_state.cursor_line,
                app.diff_state.visible_line_count
            );
        }
    }

    #[test]
    fn should_paint_visual_selection_on_every_row_of_word_wrapped_line() {
        // Regression (test campaign, probe 4b): word-boundary rows hold
        // fewer chars than the content column, so the uniform
        // `which_row * content_width` model exhausted the selection's char
        // range before the last visual row — the row rendered with no
        // selection background. The expansion's recorded per-row char
        // ranges fix the mapping.
        use crate::app::{SelPoint, VisualSelection};
        use crate::model::LineSide;

        // Four words that word-wrap into several short rows.
        let content = "alphaword betaword gammaword deltaword";
        let mut app = gutter_app(vec![long_line_diff_file(content)]);
        let buffer = draw_at(&mut app, 24, 12); // content_width 24-2(border)-8(gutter)
        let marker_rows: Vec<usize> = rows(&buffer)
            .iter()
            .enumerate()
            .filter(|(_, row)| row.contains('↪'))
            .map(|(y, _)| y)
            .collect();
        assert!(marker_rows.len() >= 2, "need a multi-row wrapped line");

        // Select the whole wrapped line.
        let ann_idx = app.diff_row_to_annotation[marker_rows[0]];
        let sel_bg = app.theme.bg_highlight; // visual_selection_style's bg
        app.visual_selection = Some(VisualSelection {
            anchor: SelPoint {
                annotation_idx: ann_idx,
                char_offset: 0,
                side: LineSide::New,
            },
            head: SelPoint {
                annotation_idx: ann_idx,
                char_offset: content.chars().count(),
                side: LineSide::New,
            },
        });
        let buffer = draw_at(&mut app, 24, 12);

        // Every visual row of the line (first row + each continuation row)
        // must carry the selection background at the content column.
        let content_x = 1 + 8; // border + gutter
        let first_row_y = marker_rows[0] - 1;
        for y in std::iter::once(first_row_y).chain(marker_rows.iter().copied()) {
            assert_eq!(
                buffer[(content_x as u16, y as u16)].style().bg,
                Some(sel_bg),
                "visual row {y} of the selected wrapped line lacks selection bg"
            );
        }
    }
}
