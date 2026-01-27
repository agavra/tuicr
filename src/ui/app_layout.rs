use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
};
use unicode_width::UnicodeWidthStr;

use crate::app::{App, DiffViewMode, FileTreeItem, FocusedPanel, GapId, InputMode};
use crate::model::{LineOrigin, LineRange, LineSide};
use crate::theme::Theme;
use crate::ui::{comment_panel, help_popup, status_bar, styles};
use crate::vcs::git::calculate_gap;

pub fn render(frame: &mut Frame, app: &mut App) {
    // Special handling for commit selection mode
    if app.input_mode == InputMode::CommitSelect {
        render_commit_select(frame, app);
        return;
    }

    // Clear cursor position before rendering (will be set if in Comment mode)
    app.comment_cursor_screen_pos = None;

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![
            Constraint::Length(1), // Header
            Constraint::Min(0),    // Main content
            Constraint::Length(1), // Status bar (also shows command input in command mode)
        ])
        .split(frame.area());

    status_bar::render_header(frame, app, chunks[0]);
    render_main_content(frame, app, chunks[1]);
    status_bar::render_status_bar(frame, app, chunks[2]);

    // Render help popup on top if in help mode
    if app.input_mode == InputMode::Help {
        help_popup::render_help(frame, app);
    }

    // Comment input is now rendered inline in the diff view

    // Render confirm dialog if in confirm mode
    if app.input_mode == InputMode::Confirm {
        comment_panel::render_confirm_dialog(frame, app, "Copy review to clipboard?");
    }

    // Position terminal cursor for IME when in Comment mode
    // Always set a cursor position to prevent IME from showing at (0,0)
    if app.input_mode == InputMode::Comment {
        let (col, row) = app.comment_cursor_screen_pos.unwrap_or_else(|| {
            // Fallback: position cursor in the diff area or at a reasonable default
            // Use the diff area if available, otherwise use the main content area
            if let Some(diff_area) = app.diff_area {
                // Position at the start of the diff inner area (after border)
                (diff_area.x + 1, diff_area.y + 1)
            } else {
                // Last resort: position at the main content area
                (chunks[1].x + 1, chunks[1].y + 1)
            }
        });
        frame.set_cursor_position(ratatui::layout::Position { x: col, y: row });
    }
}

fn render_commit_select(frame: &mut Frame, app: &App) {
    let area = frame.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Header
            Constraint::Min(0),    // Commit list
            Constraint::Length(1), // Footer hints
        ])
        .split(area);

    // Header
    let header = Paragraph::new(" Select commits to review ")
        .style(styles::header_style(&app.theme))
        .block(Block::default());
    frame.render_widget(header, chunks[0]);

    // Commit list
    let block = Block::default()
        .title(" Recent Commits ")
        .borders(Borders::ALL)
        .border_style(styles::border_style(&app.theme, true));

    let inner = block.inner(chunks[1]);
    frame.render_widget(block, chunks[1]);

    // Get range info for visual indicators
    let range = app.commit_selection_range;

    let items: Vec<Line> = app
        .commit_list
        .iter()
        .enumerate()
        .map(|(i, commit)| {
            let is_selected = app.is_commit_selected(i);
            let is_cursor = i == app.commit_list_cursor;

            // Range boundary indicators
            let range_marker = match range {
                Some((start, end)) if i == start && i == end => "─",
                Some((start, _)) if i == start => "┌",
                Some((_, end)) if i == end => "└",
                Some((start, end)) if i > start && i < end => "│",
                _ => " ",
            };

            let checkbox = if is_selected { "[x]" } else { "[ ]" };
            let pointer = if is_cursor { ">" } else { " " };

            let style = if is_cursor {
                styles::selected_style(&app.theme)
            } else if is_selected {
                Style::default().fg(app.theme.fg_secondary)
            } else {
                Style::default()
            };

            let checkbox_style = if is_selected {
                styles::reviewed_style(&app.theme)
            } else {
                styles::pending_style(&app.theme)
            };

            let range_style = if is_selected {
                styles::reviewed_style(&app.theme)
            } else {
                Style::default().fg(app.theme.fg_secondary)
            };

            // Format: > ┌ [x] abc1234  Commit message (author, date)
            let time_str = commit.time.format("%Y-%m-%d").to_string();
            Line::from(vec![
                Span::styled(format!("{pointer} "), style),
                Span::styled(format!("{range_marker} "), range_style),
                Span::styled(format!("{checkbox} "), checkbox_style),
                Span::styled(
                    format!("{} ", commit.short_id),
                    styles::hash_style(&app.theme),
                ),
                Span::styled(truncate_str(&commit.summary, 50), style),
                Span::styled(
                    format!(" ({}, {})", commit.author, time_str),
                    Style::default().fg(app.theme.fg_secondary),
                ),
            ])
        })
        .collect();

    let list = Paragraph::new(items);
    frame.render_widget(list, inner);

    // Footer with mode, hints, and right-aligned message
    let theme = &app.theme;
    let mode_span = Span::styled(" SELECT ", styles::mode_style(theme));

    let selected_count = match app.commit_selection_range {
        Some((start, end)) => end - start + 1,
        None => 0,
    };
    let selection_info = if selected_count > 0 {
        format!(" ({selected_count} selected)")
    } else {
        String::new()
    };
    let hints = format!(" j/k:navigate  Space:select range  Enter:confirm  q:quit{selection_info}");
    let hints_span = Span::styled(hints, Style::default().fg(theme.fg_secondary));

    let left_spans = vec![mode_span, hints_span];

    let (message_span, message_width) = status_bar::build_message_span(app.message.as_ref(), theme);
    let spans = status_bar::build_right_aligned_spans(
        left_spans,
        message_span,
        message_width,
        chunks[2].width as usize,
    );

    let footer = Paragraph::new(Line::from(spans))
        .style(styles::status_bar_style(theme))
        .block(Block::default());
    frame.render_widget(footer, chunks[2]);
}

fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}

fn render_main_content(frame: &mut Frame, app: &mut App, area: Rect) {
    if app.show_file_list {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(20), // File list
                Constraint::Percentage(80), // Diff view
            ])
            .split(area);

        app.file_list_area = Some(chunks[0]);
        app.diff_area = Some(chunks[1]);

        render_file_list(frame, app, chunks[0]);
        render_diff_view(frame, app, chunks[1]);
    } else {
        app.file_list_area = None;
        app.diff_area = Some(area);

        render_diff_view(frame, app, area);
    }
}

fn render_file_list(frame: &mut Frame, app: &mut App, area: Rect) {
    use ratatui::style::Modifier;
    use std::path::Path;

    let focused = app.focused_panel == FocusedPanel::FileList;

    let block = Block::default()
        .title(" Files ")
        .borders(Borders::ALL)
        .border_style(styles::border_style(&app.theme, focused));

    let inner = block.inner(area);
    let visible_items = app.build_visible_items();

    let max_content_width = visible_items
        .iter()
        .map(|item| match item {
            FileTreeItem::Directory { path, depth, .. } => {
                let dir_name = Path::new(path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(path);
                depth * 2 + 2 + dir_name.width() + 1
            }
            FileTreeItem::File { file_idx, depth } => {
                let file = &app.diff_files[*file_idx];
                let filename = file
                    .display_path()
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("?");
                depth * 2 + 3 + 3 + filename.width()
            }
        })
        .max()
        .unwrap_or(0);

    app.file_list_state.viewport_width = inner.width as usize;
    app.file_list_state.viewport_height = inner.height as usize;
    app.file_list_state.max_content_width = max_content_width;

    let max_scroll_x = max_content_width.saturating_sub(inner.width as usize);
    if app.file_list_state.scroll_x > max_scroll_x {
        app.file_list_state.scroll_x = max_scroll_x;
    }
    let scroll_x = app.file_list_state.scroll_x;

    // When diff panel is focused, sync file list selection to current file
    // But preserve the current offset to not interfere with manual scrolling
    if app.focused_panel == FocusedPanel::Diff {
        let current_file_idx = app.diff_state.current_file_idx;
        for (tree_idx, item) in visible_items.iter().enumerate() {
            if let FileTreeItem::File { file_idx, .. } = item
                && *file_idx == current_file_idx
            {
                if app.file_list_state.selected() != tree_idx {
                    // Save current offset before changing selection
                    let current_offset = app.file_list_state.list_state.offset();
                    app.file_list_state.select(tree_idx);
                    // Restore offset to prevent auto-scrolling
                    *app.file_list_state.list_state.offset_mut() = current_offset;
                }
                break;
            }
        }
    }

    let selected_idx = app.file_list_state.selected();

    let items: Vec<ListItem> = visible_items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let is_selected = i == selected_idx;

            match item {
                FileTreeItem::Directory {
                    path,
                    depth,
                    expanded,
                } => {
                    let indent = "  ".repeat(*depth);
                    let icon = if *expanded { "▾" } else { "▸" };
                    let dir_name = Path::new(path)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(path);

                    let style = if is_selected {
                        styles::selected_style(&app.theme).add_modifier(Modifier::UNDERLINED)
                    } else {
                        Style::default()
                    };

                    let line = Line::from(vec![
                        Span::styled(indent, Style::default()),
                        Span::styled(format!("{icon} "), styles::dir_icon_style(&app.theme)),
                        Span::styled(format!("{dir_name}/"), style),
                    ]);

                    ListItem::new(apply_horizontal_scroll(line, scroll_x))
                }
                FileTreeItem::File { file_idx, depth } => {
                    let file = &app.diff_files[*file_idx];
                    let path = file.display_path();
                    let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
                    let status = file.status.as_char();
                    let is_reviewed = app.session.is_file_reviewed(path);
                    let review_mark = if is_reviewed { "✓" } else { " " };

                    let indent = "  ".repeat(*depth);

                    let style = if is_selected {
                        styles::selected_style(&app.theme).add_modifier(Modifier::UNDERLINED)
                    } else {
                        Style::default()
                    };

                    let line = Line::from(vec![
                        Span::styled(indent, Style::default()),
                        Span::styled(
                            format!("[{review_mark}]"),
                            if is_reviewed {
                                styles::reviewed_style(&app.theme)
                            } else {
                                styles::pending_style(&app.theme)
                            },
                        ),
                        Span::styled(
                            format!(" {status} "),
                            styles::file_status_style(&app.theme, status),
                        ),
                        Span::styled(filename.to_string(), style),
                    ]);

                    ListItem::new(apply_horizontal_scroll(line, scroll_x))
                }
            }
        })
        .collect();

    let list = List::new(items).block(block);

    frame.render_stateful_widget(list, area, &mut app.file_list_state.list_state);
}

fn render_diff_view(frame: &mut Frame, app: &mut App, area: Rect) {
    match app.diff_view_mode {
        DiffViewMode::Unified => render_unified_diff(frame, app, area),
        DiffViewMode::SideBySide => render_side_by_side_diff(frame, app, area),
    }
}

fn render_unified_diff(frame: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focused_panel == FocusedPanel::Diff;

    let block = Block::default()
        .title(" Diff (Unified) ")
        .borders(Borders::ALL)
        .border_style(styles::border_style(&app.theme, focused));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Update viewport height for scroll calculations
    app.diff_state.viewport_height = inner.height as usize;

    // Build all diff lines for infinite scroll
    // Track line index to mark the current line (cursor position)
    let mut lines: Vec<Line> = Vec::new();
    let mut line_idx: usize = 0;
    let current_line_idx = app.diff_state.cursor_line;

    // Track cursor position for IME when in Comment mode
    // Store the logical line index and column where the cursor should be
    let mut comment_cursor_logical_line: Option<usize> = None;
    let mut comment_cursor_column: u16 = 0;

    for (file_idx, file) in app.diff_files.iter().enumerate() {
        let path = file.display_path();
        let status = file.status.as_char();
        let is_reviewed = app.session.is_file_reviewed(path);

        // File header
        let indicator = cursor_indicator_spaced(line_idx, current_line_idx);

        // Add checkmark if reviewed (using same character as file list)
        let review_mark = if is_reviewed { "✓ " } else { "" };

        lines.push(Line::from(vec![
            Span::styled(indicator, styles::current_line_indicator_style(&app.theme)),
            Span::styled(
                format!("═══ {}{} [{}] ", review_mark, path.display(), status),
                styles::file_header_style(&app.theme),
            ),
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
                        app.comment_type,
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
                        comment.comment_type,
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
                app.comment_type,
                &app.comment_buffer,
                app.comment_cursor,
                None,
                false,
                app.supports_keyboard_enhancement,
            );
            // Track cursor position
            comment_cursor_logical_line = Some(line_idx + cursor_info.line_offset);
            comment_cursor_column = 1 + cursor_info.column;

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

        if file.is_binary {
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
                    if app.is_gap_expanded(&gap_id) {
                        // Render expanded context lines
                        if let Some(expanded_lines) = app.expanded_content.get(&gap_id) {
                            for expanded_line in expanded_lines {
                                let indicator = cursor_indicator(line_idx, current_line_idx);
                                let line_num = expanded_line
                                    .new_lineno
                                    .map(|n| format!("{n:>4} "))
                                    .unwrap_or_else(|| "     ".to_string());

                                let line_spans = vec![
                                    Span::styled(
                                        indicator,
                                        styles::current_line_indicator_style(&app.theme),
                                    ),
                                    Span::styled(
                                        line_num,
                                        styles::expanded_context_style(&app.theme),
                                    ),
                                    Span::styled("  ", styles::expanded_context_style(&app.theme)),
                                    Span::styled(
                                        expanded_line.content.clone(),
                                        styles::expanded_context_style(&app.theme),
                                    ),
                                ];
                                lines.push(Line::from(line_spans));
                                line_idx += 1;
                            }
                        }
                    } else {
                        // Render expander line
                        let indicator = cursor_indicator_spaced(line_idx, current_line_idx);
                        lines.push(Line::from(vec![
                            Span::styled(
                                indicator,
                                styles::current_line_indicator_style(&app.theme),
                            ),
                            Span::styled(
                                format!("       ... expand ({gap} lines) ..."),
                                styles::dim_style(&app.theme),
                            ),
                        ]));
                        line_idx += 1;
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

                    // Check if this line is in visual selection
                    let is_in_visual_selection = {
                        let line_num = match diff_line.origin {
                            LineOrigin::Addition | LineOrigin::Context => diff_line.new_lineno,
                            LineOrigin::Deletion => diff_line.old_lineno,
                        };
                        let side = match diff_line.origin {
                            LineOrigin::Addition | LineOrigin::Context => LineSide::New,
                            LineOrigin::Deletion => LineSide::Old,
                        };
                        line_num
                            .map(|ln| app.is_line_in_visual_selection(ln, side))
                            .unwrap_or(false)
                    };

                    // Apply visual selection highlighting if applicable
                    let style = if is_in_visual_selection {
                        base_style.patch(styles::visual_selection_style(&app.theme))
                    } else {
                        base_style
                    };

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

                    // Build line spans - use syntax highlighting if available
                    let line_num_style = if is_in_visual_selection {
                        styles::dim_style(&app.theme)
                            .patch(styles::visual_selection_style(&app.theme))
                    } else {
                        styles::dim_style(&app.theme)
                    };

                    let mut line_spans = vec![
                        Span::styled(indicator, styles::current_line_indicator_style(&app.theme)),
                        Span::styled(line_num_str, line_num_style),
                        Span::styled(format!("{prefix} "), style),
                    ];

                    // Add content spans
                    if let Some(ref highlighted) = diff_line.highlighted_spans {
                        // Use syntax-highlighted spans
                        for (span_style, span_text) in highlighted {
                            let final_style = if is_in_visual_selection {
                                span_style.patch(styles::visual_selection_style(&app.theme))
                            } else {
                                *span_style
                            };
                            line_spans.push(Span::styled(span_text.clone(), final_style));
                        }
                    } else {
                        // Fall back to default diff styling
                        line_spans.push(Span::styled(diff_line.content.clone(), style));
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
                                                app.comment_type,
                                                &app.comment_buffer,
                                                app.comment_cursor,
                                                line_range,
                                                true,
                                                app.supports_keyboard_enhancement,
                                            );
                                        comment_cursor_logical_line =
                                            Some(line_idx + cursor_info.line_offset);
                                        comment_cursor_column = 1 + cursor_info.column;

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
                                            comment.comment_type,
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
                                    app.comment_type,
                                    &app.comment_buffer,
                                    app.comment_cursor,
                                    line_range,
                                    false,
                                    app.supports_keyboard_enhancement,
                                );
                            comment_cursor_logical_line = Some(line_idx + cursor_info.line_offset);
                            comment_cursor_column = 1 + cursor_info.column;

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
                                                app.comment_type,
                                                &app.comment_buffer,
                                                app.comment_cursor,
                                                line_range,
                                                true,
                                                app.supports_keyboard_enhancement,
                                            );
                                        comment_cursor_logical_line =
                                            Some(line_idx + cursor_info.line_offset);
                                        comment_cursor_column = 1 + cursor_info.column;

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
                                            comment.comment_type,
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
                                    app.comment_type,
                                    &app.comment_buffer,
                                    app.comment_cursor,
                                    line_range,
                                    false,
                                    app.supports_keyboard_enhancement,
                                );
                            comment_cursor_logical_line = Some(line_idx + cursor_info.line_offset);
                            comment_cursor_column = 1 + cursor_info.column;

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

    // Calculate how many logical lines actually fit in the viewport when wrapped
    let viewport_width = inner.width as usize;
    let viewport_height = inner.height as usize;
    app.diff_state.visible_line_count = if app.diff_state.wrap_lines && viewport_width > 0 {
        let mut visual_rows_used = 0;
        let mut logical_lines_visible = 0;
        for &width in &line_widths {
            // Each line takes at least 1 row, plus extra rows if it wraps
            let rows_for_line = if width == 0 {
                1
            } else {
                width.div_ceil(viewport_width)
            };
            if visual_rows_used + rows_for_line > viewport_height {
                break;
            }
            visual_rows_used += rows_for_line;
            logical_lines_visible += 1;
        }
        logical_lines_visible.max(1)
    } else {
        viewport_height
    };

    let max_scroll_x = max_content_width.saturating_sub(inner.width as usize);
    if app.diff_state.scroll_x > max_scroll_x {
        app.diff_state.scroll_x = max_scroll_x;
    }
    if app.diff_state.wrap_lines {
        app.diff_state.scroll_x = 0;
    }

    let scroll_x = app.diff_state.scroll_x;
    let visible_lines: Vec<Line> = if app.diff_state.wrap_lines {
        visible_lines_unscrolled
    } else {
        visible_lines_unscrolled
            .into_iter()
            .map(|line| apply_horizontal_scroll(line, scroll_x))
            .collect()
    };

    let mut diff = Paragraph::new(visible_lines);
    if app.diff_state.wrap_lines {
        diff = diff.wrap(Wrap { trim: false });
    }
    frame.render_widget(diff, inner);

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

/// Context for rendering side-by-side diff lines
struct SideBySideContext<'a> {
    theme: &'a Theme,
    content_width: usize,
    current_line_idx: usize,
    // Comment input state for inline editing
    comment_input_mode: bool,
    comment_line: Option<(u32, LineSide)>,
    comment_type: crate::model::CommentType,
    comment_buffer: &'a str,
    comment_cursor: usize,
    comment_line_range: Option<LineRange>,
    editing_comment_id: Option<&'a str>,
    supports_keyboard_enhancement: bool,
}

/// Get cursor indicator (single character for inline content)
fn cursor_indicator(line_idx: usize, current_line_idx: usize) -> &'static str {
    if line_idx == current_line_idx {
        "▶"
    } else {
        " "
    }
}

/// Get cursor indicator with spacing (two characters for line prefixes)
fn cursor_indicator_spaced(line_idx: usize, current_line_idx: usize) -> &'static str {
    if line_idx == current_line_idx {
        "▶ "
    } else {
        "  "
    }
}

fn render_side_by_side_diff(frame: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focused_panel == FocusedPanel::Diff;

    let block = Block::default()
        .title(" Diff (Side-by-Side) ")
        .borders(Borders::ALL)
        .border_style(styles::border_style(&app.theme, focused));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Update viewport height for scroll calculations
    app.diff_state.viewport_height = inner.height as usize;

    // Calculate column widths (split the area in half)
    // Layout: indicator(1) + linenum(4) + space(1) + prefix(1) + content + " │ "(3) + linenum(4) + space(1) + prefix(1) + content
    // Total overhead: 1 + 5 + 1 + 3 + 5 + 1 = 16
    let available_width = inner.width.saturating_sub(16) as usize;
    let content_width = available_width / 2;

    // Determine if we're in line comment mode (not file-level)
    let comment_input_mode = app.input_mode == InputMode::Comment && !app.comment_is_file_level;

    let ctx = SideBySideContext {
        theme: &app.theme,
        content_width,
        current_line_idx: app.diff_state.cursor_line,
        comment_input_mode,
        comment_line: app.comment_line,
        comment_type: app.comment_type,
        comment_buffer: &app.comment_buffer,
        comment_cursor: app.comment_cursor,
        comment_line_range: app.comment_line_range.map(|(r, _)| r),
        editing_comment_id: app.editing_comment_id.as_deref(),
        supports_keyboard_enhancement: app.supports_keyboard_enhancement,
    };

    // Build all diff lines for side-by-side view
    let mut lines: Vec<Line> = Vec::new();
    let mut line_idx: usize = 0;

    // Track cursor position for IME when in Comment mode
    let mut comment_cursor_logical_line: Option<usize> = None;
    let mut comment_cursor_column: u16 = 0;

    for (file_idx, file) in app.diff_files.iter().enumerate() {
        let path = file.display_path();
        let status = file.status.as_char();
        let is_reviewed = app.session.is_file_reviewed(path);

        // File header
        let indicator = cursor_indicator_spaced(line_idx, ctx.current_line_idx);

        let review_mark = if is_reviewed { "✓ " } else { "" };

        lines.push(Line::from(vec![
            Span::styled(indicator, styles::current_line_indicator_style(&app.theme)),
            Span::styled(
                format!("═══ {}{} [{}] ", review_mark, path.display(), status),
                styles::file_header_style(&app.theme),
            ),
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
                        app.comment_type,
                        &app.comment_buffer,
                        app.comment_cursor,
                        None,
                        true,
                        app.supports_keyboard_enhancement,
                    );
                    comment_cursor_logical_line = Some(line_idx + cursor_info.line_offset);
                    comment_cursor_column = 1 + cursor_info.column;

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
                        comment.comment_type,
                        &comment.content,
                        None,
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
                app.comment_type,
                &app.comment_buffer,
                app.comment_cursor,
                None,
                false,
                app.supports_keyboard_enhancement,
            );
            comment_cursor_logical_line = Some(line_idx + cursor_info.line_offset);
            comment_cursor_column = 1 + cursor_info.column;

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

        if file.is_binary {
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
                    if app.is_gap_expanded(&gap_id) {
                        // Render expanded context lines
                        if let Some(expanded_lines) = app.expanded_content.get(&gap_id) {
                            for expanded_line in expanded_lines {
                                let indicator = cursor_indicator(line_idx, ctx.current_line_idx);
                                let line_num = expanded_line
                                    .new_lineno
                                    .map(|n| format!("{n:>4} "))
                                    .unwrap_or_else(|| "     ".to_string());

                                // In side-by-side, show context on both sides
                                let line_spans = vec![
                                    Span::styled(
                                        indicator,
                                        styles::current_line_indicator_style(&app.theme),
                                    ),
                                    Span::styled(
                                        line_num.clone(),
                                        styles::expanded_context_style(&app.theme),
                                    ),
                                    Span::styled("  ", styles::expanded_context_style(&app.theme)),
                                    Span::styled(
                                        truncate_or_pad(&expanded_line.content, ctx.content_width),
                                        styles::expanded_context_style(&app.theme),
                                    ),
                                    Span::styled(" │ ", styles::dim_style(&app.theme)),
                                    Span::styled(
                                        line_num,
                                        styles::expanded_context_style(&app.theme),
                                    ),
                                    Span::styled("  ", styles::expanded_context_style(&app.theme)),
                                    Span::styled(
                                        truncate_or_pad(&expanded_line.content, ctx.content_width),
                                        styles::expanded_context_style(&app.theme),
                                    ),
                                ];
                                lines.push(Line::from(line_spans));
                                line_idx += 1;
                            }
                        }
                    } else {
                        // Render expander line
                        let indicator = cursor_indicator_spaced(line_idx, ctx.current_line_idx);
                        lines.push(Line::from(vec![
                            Span::styled(
                                indicator,
                                styles::current_line_indicator_style(&app.theme),
                            ),
                            Span::styled(
                                format!("       ... expand ({gap} lines) ..."),
                                styles::dim_style(&app.theme),
                            ),
                        ]));
                        line_idx += 1;
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
                    line_idx,
                    &mut lines,
                );
                line_idx = new_line_idx;
                if cursor_info.is_some() {
                    comment_cursor_logical_line = cursor_info.map(|(line, _)| line);
                    comment_cursor_column = cursor_info.map(|(_, col)| col).unwrap_or(0);
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

    // Calculate how many logical lines actually fit in the viewport when wrapped
    let viewport_width = inner.width as usize;
    let viewport_height = inner.height as usize;
    app.diff_state.visible_line_count = if app.diff_state.wrap_lines && viewport_width > 0 {
        let mut visual_rows_used = 0;
        let mut logical_lines_visible = 0;
        for &width in &line_widths {
            // Each line takes at least 1 row, plus extra rows if it wraps
            let rows_for_line = if width == 0 {
                1
            } else {
                width.div_ceil(viewport_width)
            };
            if visual_rows_used + rows_for_line > viewport_height {
                break;
            }
            visual_rows_used += rows_for_line;
            logical_lines_visible += 1;
        }
        logical_lines_visible.max(1)
    } else {
        viewport_height
    };

    let max_scroll_x = max_content_width.saturating_sub(inner.width as usize);
    if app.diff_state.scroll_x > max_scroll_x {
        app.diff_state.scroll_x = max_scroll_x;
    }
    if app.diff_state.wrap_lines {
        app.diff_state.scroll_x = 0;
    }

    let scroll_x = app.diff_state.scroll_x;
    let visible_lines: Vec<Line> = if app.diff_state.wrap_lines {
        visible_lines_unscrolled
    } else {
        visible_lines_unscrolled
            .into_iter()
            .map(|line| apply_horizontal_scroll(line, scroll_x))
            .collect()
    };

    let mut diff = Paragraph::new(visible_lines);
    if app.diff_state.wrap_lines {
        diff = diff.wrap(Wrap { trim: false });
    }
    frame.render_widget(diff, inner);

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

/// Process and render all diff lines in a hunk for side-by-side view
/// Returns (new_line_idx, Option<(cursor_logical_line, cursor_column)>)
fn render_hunk_lines_side_by_side(
    hunk_lines: &[crate::model::DiffLine],
    line_comments: &std::collections::HashMap<u32, Vec<crate::model::Comment>>,
    ctx: &SideBySideContext,
    mut line_idx: usize,
    lines: &mut Vec<Line>,
) -> (usize, Option<(usize, u16)>) {
    let mut i = 0;
    let mut cursor_info_out: Option<(usize, u16)> = None;

    while i < hunk_lines.len() {
        let diff_line = &hunk_lines[i];

        match diff_line.origin {
            LineOrigin::Context => {
                let (new_line_idx, cursor_info) = render_context_line_side_by_side(
                    diff_line,
                    line_comments,
                    ctx,
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
/// Returns (new_line_idx, Option<(cursor_logical_line, cursor_column)>)
fn render_context_line_side_by_side(
    diff_line: &crate::model::DiffLine,
    line_comments: &std::collections::HashMap<u32, Vec<crate::model::Comment>>,
    ctx: &SideBySideContext,
    mut line_idx: usize,
    lines: &mut Vec<Line>,
) -> (usize, Option<(usize, u16)>) {
    let line_num = diff_line
        .old_lineno
        .or(diff_line.new_lineno)
        .map(|n| format!("{n:>4}"))
        .unwrap_or_else(|| "    ".to_string());

    let indicator = cursor_indicator(line_idx, ctx.current_line_idx);

    let mut spans = vec![
        Span::styled(indicator, styles::current_line_indicator_style(ctx.theme)),
        Span::styled(format!("{line_num} "), styles::dim_style(ctx.theme)),
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
        format!("{line_num} "),
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
    let mut cursor_info_out: Option<(usize, u16)> = None;
    if let Some(new_ln) = diff_line.new_lineno {
        let (new_line_idx, cursor_info) =
            add_comments_to_line(new_ln, line_comments, LineSide::New, ctx, line_idx, lines);
        line_idx = new_line_idx;
        cursor_info_out = cursor_info;
    }

    (line_idx, cursor_info_out)
}

/// Render paired deletions and additions side-by-side
/// Returns (line_idx, skip_count, Option<(cursor_logical_line, cursor_column)>)
fn render_deletion_addition_pair_side_by_side(
    hunk_lines: &[crate::model::DiffLine],
    start_idx: usize,
    line_comments: &std::collections::HashMap<u32, Vec<crate::model::Comment>>,
    ctx: &SideBySideContext,
    mut line_idx: usize,
    lines: &mut Vec<Line>,
) -> (usize, usize, Option<(usize, u16)>) {
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
    let mut cursor_info_out: Option<(usize, u16)> = None;

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
            add_deletion_spans(ctx.theme, &mut spans, del_line, ctx.content_width);
        } else {
            add_empty_column_spans(&mut spans, ctx.content_width);
        }

        spans.push(Span::styled(" │ ", styles::dim_style(ctx.theme)));

        // Right side (addition)
        if offset < add_count {
            let add_line = &hunk_lines[add_start + offset];
            add_addition_spans(ctx.theme, &mut spans, add_line, ctx.content_width);
        } else {
            add_empty_column_spans(&mut spans, ctx.content_width);
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
                    line_idx,
                    lines,
                );
                line_idx = new_line_idx;
                if cursor_info.is_some() {
                    cursor_info_out = cursor_info;
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
                    line_idx,
                    lines,
                );
                line_idx = new_line_idx;
                if cursor_info.is_some() {
                    cursor_info_out = cursor_info;
                }
            }
        }
    }

    (line_idx, add_end, cursor_info_out)
}

/// Render a standalone addition (no matching deletion)
/// Returns (new_line_idx, Option<(cursor_logical_line, cursor_column)>)
fn render_standalone_addition_side_by_side(
    diff_line: &crate::model::DiffLine,
    line_comments: &std::collections::HashMap<u32, Vec<crate::model::Comment>>,
    ctx: &SideBySideContext,
    mut line_idx: usize,
    lines: &mut Vec<Line>,
) -> (usize, Option<(usize, u16)>) {
    let indicator = cursor_indicator(line_idx, ctx.current_line_idx);

    let mut spans = vec![Span::styled(
        indicator,
        styles::current_line_indicator_style(ctx.theme),
    )];
    add_empty_column_spans(&mut spans, ctx.content_width);
    spans.push(Span::styled(" │ ", styles::dim_style(ctx.theme)));
    add_addition_spans(ctx.theme, &mut spans, diff_line, ctx.content_width);

    lines.push(Line::from(spans));
    line_idx += 1;

    // Add comments if any
    let mut cursor_info_out: Option<(usize, u16)> = None;
    if let Some(new_ln) = diff_line.new_lineno {
        let (new_line_idx, cursor_info) =
            add_comments_to_line(new_ln, line_comments, LineSide::New, ctx, line_idx, lines);
        line_idx = new_line_idx;
        cursor_info_out = cursor_info;
    }

    (line_idx, cursor_info_out)
}

/// Add deletion line spans to the spans vector
fn add_deletion_spans(
    theme: &Theme,
    spans: &mut Vec<Span>,
    diff_line: &crate::model::DiffLine,
    content_width: usize,
) {
    let line_num = diff_line
        .old_lineno
        .map(|n| format!("{n:>4}"))
        .unwrap_or_else(|| "    ".to_string());

    spans.push(Span::styled(
        format!("{line_num} "),
        styles::dim_style(theme),
    ));
    spans.push(Span::styled("-".to_string(), styles::diff_del_style(theme)));

    // Use syntax highlighting if available
    if let Some(ref highlighted) = diff_line.highlighted_spans {
        let content_spans =
            truncate_or_pad_spans(highlighted, content_width, styles::diff_del_style(theme));
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
) {
    let line_num = diff_line
        .new_lineno
        .map(|n| format!("{n:>4}"))
        .unwrap_or_else(|| "    ".to_string());

    spans.push(Span::styled(
        format!("{line_num} "),
        styles::dim_style(theme),
    ));
    spans.push(Span::styled("+".to_string(), styles::diff_add_style(theme)));

    // Use syntax highlighting if available
    if let Some(ref highlighted) = diff_line.highlighted_spans {
        let content_spans =
            truncate_or_pad_spans(highlighted, content_width, styles::diff_add_style(theme));
        spans.extend(content_spans);
    } else {
        // Fall back to plain text
        let content = truncate_or_pad(&diff_line.content, content_width);
        spans.push(Span::styled(content, styles::diff_add_style(theme)));
    }
}

/// Add empty column spans (for when one side has no content)
fn add_empty_column_spans(spans: &mut Vec<Span>, content_width: usize) {
    // line_num(4) + space(1) + prefix(1) + content
    spans.push(Span::styled(
        " ".repeat(5 + 1 + content_width),
        Style::default(),
    ));
}

/// Add comments for a specific line.
/// Returns (new_line_idx, Option<(cursor_logical_line, cursor_column)>)
fn add_comments_to_line(
    line_num: u32,
    line_comments: &std::collections::HashMap<u32, Vec<crate::model::Comment>>,
    side: LineSide,
    ctx: &SideBySideContext,
    mut line_idx: usize,
    lines: &mut Vec<Line>,
) -> (usize, Option<(usize, u16)>) {
    // Check if we're adding/editing a comment on this line and side
    let is_line_comment_mode = ctx.comment_input_mode && ctx.comment_line == Some((line_num, side));
    let mut cursor_info_out: Option<(usize, u16)> = None;

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
                        ctx.comment_type,
                        ctx.comment_buffer,
                        ctx.comment_cursor,
                        line_range,
                        true,
                        ctx.supports_keyboard_enhancement,
                    );
                    cursor_info_out =
                        Some((line_idx + cursor_info.line_offset, 1 + cursor_info.column));

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
                } else {
                    let line_range = comment
                        .line_range
                        .or_else(|| Some(LineRange::single(line_num)));
                    let comment_lines = comment_panel::format_comment_lines(
                        ctx.theme,
                        comment.comment_type,
                        &comment.content,
                        line_range,
                    );
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
            ctx.comment_type,
            ctx.comment_buffer,
            ctx.comment_cursor,
            line_range,
            false,
            ctx.supports_keyboard_enhancement,
        );
        cursor_info_out = Some((line_idx + cursor_info.line_offset, 1 + cursor_info.column));

        for mut input_line in input_lines {
            let indicator = cursor_indicator(line_idx, ctx.current_line_idx);
            input_line.spans.insert(
                0,
                Span::styled(indicator, styles::current_line_indicator_style(ctx.theme)),
            );
            lines.push(input_line);
            line_idx += 1;
        }
    }

    (line_idx, cursor_info_out)
}

/// Truncate or pad a string to a specific width
fn truncate_or_pad(s: &str, width: usize) -> String {
    let char_count = s.chars().count();
    if char_count > width {
        s.chars().take(width.saturating_sub(3)).collect::<String>() + "..."
    } else {
        format!("{s:width$}")
    }
}

/// Truncate or pad highlighted spans to a specific display width
/// Uses unicode width to properly handle wide characters (CJK, emoji, etc.)
/// Returns a vector of spans that fits exactly within the width
fn truncate_or_pad_spans(
    spans: &[(Style, String)],
    width: usize,
    base_style: Style,
) -> Vec<Span<'static>> {
    // Count total display width
    let total_width: usize = spans.iter().map(|(_, text)| text.width()).sum();

    if total_width > width {
        // Need to truncate
        let mut result = Vec::new();
        let mut remaining = width.saturating_sub(3); // Reserve space for "..."

        for (style, text) in spans {
            if remaining == 0 {
                break;
            }

            let text_width = text.width();
            if text_width <= remaining {
                result.push(Span::styled(text.clone(), *style));
                remaining -= text_width;
            } else {
                // Truncate this span character by character to fit remaining width
                let mut truncated = String::new();
                let mut current_width = 0;
                for c in text.chars() {
                    let char_width = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
                    if current_width + char_width > remaining {
                        break;
                    }
                    truncated.push(c);
                    current_width += char_width;
                }
                if !truncated.is_empty() {
                    result.push(Span::styled(truncated, *style));
                }
                remaining = 0;
            }
        }

        // Add ellipsis
        result.push(Span::styled("...".to_string(), base_style));
        result
    } else if total_width < width {
        // Need to pad
        let mut result: Vec<Span> = spans
            .iter()
            .map(|(style, text)| Span::styled(text.clone(), *style))
            .collect();

        // Add padding
        let padding = " ".repeat(width - total_width);
        result.push(Span::styled(padding, base_style));
        result
    } else {
        // Perfect fit
        spans
            .iter()
            .map(|(style, text)| Span::styled(text.clone(), *style))
            .collect()
    }
}

/// Apply horizontal scroll to a line while preserving the first span (cursor indicator)
fn apply_horizontal_scroll(line: Line, scroll_x: usize) -> Line {
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
