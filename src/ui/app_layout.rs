use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
};
use unicode_width::UnicodeWidthStr;

use crate::app::{App, DiffViewMode, FileTreeItem, FocusedPanel, GapId, InputMode};
use crate::git::calculate_gap;
use crate::model::{LineOrigin, LineSide};
use crate::ui::{comment_panel, help_popup, status_bar, styles};

pub fn render(frame: &mut Frame, app: &mut App) {
    // Special handling for commit selection mode
    if app.input_mode == InputMode::CommitSelect {
        render_commit_select(frame, app);
        return;
    }

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

    // Render comment input popup if in comment mode
    if app.input_mode == InputMode::Comment {
        comment_panel::render_comment_input(frame, app);
    }

    // Render confirm dialog if in confirm mode
    if app.input_mode == InputMode::Confirm {
        comment_panel::render_confirm_dialog(frame, "Copy review to clipboard?");
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
        .style(styles::header_style())
        .block(Block::default());
    frame.render_widget(header, chunks[0]);

    // Commit list
    let block = Block::default()
        .title(" Recent Commits ")
        .borders(Borders::ALL)
        .border_style(styles::border_style(true));

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
                styles::selected_style()
            } else if is_selected {
                Style::default().fg(styles::FG_SECONDARY)
            } else {
                Style::default()
            };

            let checkbox_style = if is_selected {
                styles::reviewed_style()
            } else {
                styles::pending_style()
            };

            let range_style = if is_selected {
                styles::reviewed_style()
            } else {
                Style::default().fg(styles::FG_SECONDARY)
            };

            // Format: > ┌ [x] abc1234  Commit message (author, date)
            let time_str = commit.time.format("%Y-%m-%d").to_string();
            Line::from(vec![
                Span::styled(format!("{} ", pointer), style),
                Span::styled(format!("{} ", range_marker), range_style),
                Span::styled(format!("{} ", checkbox), checkbox_style),
                Span::styled(format!("{} ", commit.short_id), styles::hash_style()),
                Span::styled(truncate_str(&commit.summary, 50), style),
                Span::styled(
                    format!(" ({}, {})", commit.author, time_str),
                    Style::default().fg(styles::FG_SECONDARY),
                ),
            ])
        })
        .collect();

    let list = Paragraph::new(items);
    frame.render_widget(list, inner);

    // Footer hints with selection count
    let selected_count = match range {
        Some((start, end)) => end - start + 1,
        None => 0,
    };
    let selection_info = if selected_count > 0 {
        format!(" ({} selected)", selected_count)
    } else {
        String::new()
    };
    let hints = format!(
        " j/k:navigate  Space:select range  Enter:confirm  q:quit{}",
        selection_info
    );
    let footer = Paragraph::new(hints)
        .style(styles::status_bar_style())
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

        render_file_list(frame, app, chunks[0]);
        render_diff_view(frame, app, chunks[1]);
    } else {
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
        .border_style(styles::border_style(focused));

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
    app.file_list_state.max_content_width = max_content_width;

    let max_scroll_x = max_content_width.saturating_sub(inner.width as usize);
    if app.file_list_state.scroll_x > max_scroll_x {
        app.file_list_state.scroll_x = max_scroll_x;
    }
    let scroll_x = app.file_list_state.scroll_x;

    if app.focused_panel == FocusedPanel::Diff {
        let current_file_idx = app.diff_state.current_file_idx;
        for (tree_idx, item) in visible_items.iter().enumerate() {
            if let FileTreeItem::File { file_idx, .. } = item
                && *file_idx == current_file_idx
            {
                if app.file_list_state.selected() != tree_idx {
                    app.file_list_state.select(tree_idx);
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
                        styles::selected_style().add_modifier(Modifier::UNDERLINED)
                    } else {
                        Style::default()
                    };

                    let line = Line::from(vec![
                        Span::styled(indent, Style::default()),
                        Span::styled(format!("{} ", icon), styles::dir_icon_style()),
                        Span::styled(format!("{}/", dir_name), style),
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
                        styles::selected_style().add_modifier(Modifier::UNDERLINED)
                    } else {
                        Style::default()
                    };

                    let line = Line::from(vec![
                        Span::styled(indent, Style::default()),
                        Span::styled(
                            format!("[{}]", review_mark),
                            if is_reviewed {
                                styles::reviewed_style()
                            } else {
                                styles::pending_style()
                            },
                        ),
                        Span::styled(format!(" {} ", status), styles::file_status_style(status)),
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
        .border_style(styles::border_style(focused));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Update viewport height for scroll calculations
    app.diff_state.viewport_height = inner.height as usize;

    // Build all diff lines for infinite scroll
    // Track line index to mark the current line (cursor position)
    let mut lines: Vec<Line> = Vec::new();
    let mut line_idx: usize = 0;
    let current_line_idx = app.diff_state.cursor_line;

    for (file_idx, file) in app.diff_files.iter().enumerate() {
        let path = file.display_path();
        let status = file.status.as_char();
        let is_reviewed = app.session.is_file_reviewed(path);

        // File header
        let indicator = cursor_indicator_spaced(line_idx, current_line_idx);

        // Add checkmark if reviewed (using same character as file list)
        let review_mark = if is_reviewed { "✓ " } else { "" };

        lines.push(Line::from(vec![
            Span::styled(indicator, styles::current_line_indicator_style()),
            Span::styled(
                format!("═══ {}{} [{}] ", review_mark, path.display(), status),
                styles::file_header_style(),
            ),
            Span::styled("═".repeat(40), styles::file_header_style()),
        ]));
        line_idx += 1;

        // If file is reviewed, skip rendering the body (fold it away)
        if is_reviewed {
            continue;
        }

        // Show file-level comments right after the header
        if let Some(review) = app.session.files.get(path) {
            for comment in &review.file_comments {
                let comment_lines = comment_panel::format_comment_lines(
                    comment.comment_type,
                    &comment.content,
                    None,
                );
                for mut comment_line in comment_lines {
                    let indicator = cursor_indicator(line_idx, current_line_idx);
                    comment_line.spans.insert(
                        0,
                        Span::styled(indicator, styles::current_line_indicator_style()),
                    );
                    lines.push(comment_line);
                    line_idx += 1;
                }
            }
        }

        if file.is_binary {
            let indicator = cursor_indicator_spaced(line_idx, current_line_idx);
            lines.push(Line::from(vec![
                Span::styled(indicator, styles::current_line_indicator_style()),
                Span::styled("(binary file)", styles::dim_style()),
            ]));
            line_idx += 1;
        } else if file.hunks.is_empty() {
            let indicator = cursor_indicator_spaced(line_idx, current_line_idx);
            lines.push(Line::from(vec![
                Span::styled(indicator, styles::current_line_indicator_style()),
                Span::styled("(no changes)", styles::dim_style()),
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
                                    .map(|n| format!("{:>4} ", n))
                                    .unwrap_or_else(|| "     ".to_string());

                                let line_spans = vec![
                                    Span::styled(indicator, styles::current_line_indicator_style()),
                                    Span::styled(line_num, styles::expanded_context_style()),
                                    Span::styled("  ", styles::expanded_context_style()),
                                    Span::styled(
                                        expanded_line.content.clone(),
                                        styles::expanded_context_style(),
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
                            Span::styled(indicator, styles::current_line_indicator_style()),
                            Span::styled(
                                format!("       ... expand ({} lines) ...", gap),
                                styles::dim_style(),
                            ),
                        ]));
                        line_idx += 1;
                    }
                }

                // Hunk header
                let indicator = cursor_indicator_spaced(line_idx, current_line_idx);
                lines.push(Line::from(vec![
                    Span::styled(indicator, styles::current_line_indicator_style()),
                    Span::styled(hunk.header.to_string(), styles::diff_hunk_header_style()),
                ]));
                line_idx += 1;

                // Diff lines
                for diff_line in &hunk.lines {
                    let (prefix, style) = match diff_line.origin {
                        LineOrigin::Addition => ("+", styles::diff_add_style()),
                        LineOrigin::Deletion => ("-", styles::diff_del_style()),
                        LineOrigin::Context => (" ", styles::diff_context_style()),
                    };

                    let line_num = match diff_line.origin {
                        LineOrigin::Addition => diff_line
                            .new_lineno
                            .map(|n| format!("{:>4} ", n))
                            .unwrap_or_else(|| "     ".to_string()),
                        LineOrigin::Deletion => diff_line
                            .old_lineno
                            .map(|n| format!("{:>4} ", n))
                            .unwrap_or_else(|| "     ".to_string()),
                        _ => diff_line
                            .new_lineno
                            .or(diff_line.old_lineno)
                            .map(|n| format!("{:>4} ", n))
                            .unwrap_or_else(|| "     ".to_string()),
                    };

                    let indicator = cursor_indicator(line_idx, current_line_idx);

                    // Build line spans - use syntax highlighting if available
                    let mut line_spans = vec![
                        Span::styled(indicator, styles::current_line_indicator_style()),
                        Span::styled(line_num, styles::dim_style()),
                        Span::styled(format!("{} ", prefix), style),
                    ];

                    // Add content spans
                    if let Some(ref highlighted) = diff_line.highlighted_spans {
                        // Use syntax-highlighted spans
                        for (span_style, span_text) in highlighted {
                            line_spans.push(Span::styled(span_text.clone(), *span_style));
                        }
                    } else {
                        // Fall back to default diff styling
                        line_spans.push(Span::styled(diff_line.content.clone(), style));
                    }

                    lines.push(Line::from(line_spans));
                    line_idx += 1;

                    // Show line comments for both old side (deleted lines) and new side (added/context)
                    // Old side comments (for deleted lines)
                    if let Some(old_ln) = diff_line.old_lineno
                        && let Some(comments) = line_comments.get(&old_ln)
                    {
                        for comment in comments {
                            if comment.side == Some(LineSide::Old) {
                                let comment_lines = comment_panel::format_comment_lines(
                                    comment.comment_type,
                                    &comment.content,
                                    Some(old_ln),
                                );
                                for mut comment_line in comment_lines {
                                    let is_current = line_idx == current_line_idx;
                                    let indicator = if is_current { "▶" } else { " " };
                                    comment_line.spans.insert(
                                        0,
                                        Span::styled(
                                            indicator,
                                            styles::current_line_indicator_style(),
                                        ),
                                    );
                                    lines.push(comment_line);
                                    line_idx += 1;
                                }
                            }
                        }
                    }
                    // New side comments (for added/context lines)
                    if let Some(new_ln) = diff_line.new_lineno
                        && let Some(comments) = line_comments.get(&new_ln)
                    {
                        for comment in comments {
                            if comment.side != Some(LineSide::Old) {
                                let comment_lines = comment_panel::format_comment_lines(
                                    comment.comment_type,
                                    &comment.content,
                                    Some(new_ln),
                                );
                                for mut comment_line in comment_lines {
                                    let indicator = cursor_indicator(line_idx, current_line_idx);
                                    comment_line.spans.insert(
                                        0,
                                        Span::styled(
                                            indicator,
                                            styles::current_line_indicator_style(),
                                        ),
                                    );
                                    lines.push(comment_line);
                                    line_idx += 1;
                                }
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
            styles::current_line_indicator_style(),
        )));
        line_idx += 1;
    }

    let visible_lines_unscrolled: Vec<Line> = lines
        .into_iter()
        .skip(app.diff_state.scroll_offset)
        .take(inner.height as usize)
        .collect();

    let max_content_width = visible_lines_unscrolled
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.width())
                .sum::<usize>()
        })
        .max()
        .unwrap_or(0);

    app.diff_state.viewport_width = inner.width as usize;
    app.diff_state.max_content_width = max_content_width;

    let max_scroll_x = max_content_width.saturating_sub(inner.width as usize);
    if app.diff_state.scroll_x > max_scroll_x {
        app.diff_state.scroll_x = max_scroll_x;
    }

    let scroll_x = app.diff_state.scroll_x;
    let visible_lines: Vec<Line> = visible_lines_unscrolled
        .into_iter()
        .map(|line| apply_horizontal_scroll(line, scroll_x))
        .collect();

    let diff = Paragraph::new(visible_lines);
    frame.render_widget(diff, inner);
}

/// Context for rendering side-by-side diff lines
struct SideBySideContext {
    content_width: usize,
    current_line_idx: usize,
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
        .border_style(styles::border_style(focused));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Update viewport height for scroll calculations
    app.diff_state.viewport_height = inner.height as usize;

    // Calculate column widths (split the area in half)
    // Layout: indicator(1) + linenum(4) + space(1) + prefix(1) + content + " │ "(3) + linenum(4) + space(1) + prefix(1) + content
    // Total overhead: 1 + 5 + 1 + 3 + 5 + 1 = 16
    let available_width = inner.width.saturating_sub(16) as usize;
    let content_width = available_width / 2;

    let ctx = SideBySideContext {
        content_width,
        current_line_idx: app.diff_state.cursor_line,
    };

    // Build all diff lines for side-by-side view
    let mut lines: Vec<Line> = Vec::new();
    let mut line_idx: usize = 0;

    for (file_idx, file) in app.diff_files.iter().enumerate() {
        let path = file.display_path();
        let status = file.status.as_char();
        let is_reviewed = app.session.is_file_reviewed(path);

        // File header
        let indicator = cursor_indicator_spaced(line_idx, ctx.current_line_idx);

        let review_mark = if is_reviewed { "✓ " } else { "" };

        lines.push(Line::from(vec![
            Span::styled(indicator, styles::current_line_indicator_style()),
            Span::styled(
                format!("═══ {}{} [{}] ", review_mark, path.display(), status),
                styles::file_header_style(),
            ),
            Span::styled("═".repeat(40), styles::file_header_style()),
        ]));
        line_idx += 1;

        // If file is reviewed, skip rendering the body
        if is_reviewed {
            continue;
        }

        // Show file-level comments
        if let Some(review) = app.session.files.get(path) {
            for comment in &review.file_comments {
                let comment_lines = comment_panel::format_comment_lines(
                    comment.comment_type,
                    &comment.content,
                    None,
                );
                for mut comment_line in comment_lines {
                    let indicator = cursor_indicator(line_idx, ctx.current_line_idx);
                    comment_line.spans.insert(
                        0,
                        Span::styled(indicator, styles::current_line_indicator_style()),
                    );
                    lines.push(comment_line);
                    line_idx += 1;
                }
            }
        }

        if file.is_binary {
            let indicator = cursor_indicator_spaced(line_idx, ctx.current_line_idx);
            lines.push(Line::from(vec![
                Span::styled(indicator, styles::current_line_indicator_style()),
                Span::styled("(binary file)", styles::dim_style()),
            ]));
            line_idx += 1;
        } else if file.hunks.is_empty() {
            let indicator = cursor_indicator_spaced(line_idx, ctx.current_line_idx);
            lines.push(Line::from(vec![
                Span::styled(indicator, styles::current_line_indicator_style()),
                Span::styled("(no changes)", styles::dim_style()),
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
                                    .map(|n| format!("{:>4} ", n))
                                    .unwrap_or_else(|| "     ".to_string());

                                // In side-by-side, show context on both sides
                                let line_spans = vec![
                                    Span::styled(indicator, styles::current_line_indicator_style()),
                                    Span::styled(
                                        line_num.clone(),
                                        styles::expanded_context_style(),
                                    ),
                                    Span::styled("  ", styles::expanded_context_style()),
                                    Span::styled(
                                        truncate_or_pad(&expanded_line.content, ctx.content_width),
                                        styles::expanded_context_style(),
                                    ),
                                    Span::styled(" │ ", styles::dim_style()),
                                    Span::styled(line_num, styles::expanded_context_style()),
                                    Span::styled("  ", styles::expanded_context_style()),
                                    Span::styled(
                                        truncate_or_pad(&expanded_line.content, ctx.content_width),
                                        styles::expanded_context_style(),
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
                            Span::styled(indicator, styles::current_line_indicator_style()),
                            Span::styled(
                                format!("       ... expand ({} lines) ...", gap),
                                styles::dim_style(),
                            ),
                        ]));
                        line_idx += 1;
                    }
                }

                // Hunk header
                let indicator = cursor_indicator_spaced(line_idx, ctx.current_line_idx);
                lines.push(Line::from(vec![
                    Span::styled(indicator, styles::current_line_indicator_style()),
                    Span::styled(hunk.header.to_string(), styles::diff_hunk_header_style()),
                ]));
                line_idx += 1;

                // Process diff lines in side-by-side format
                line_idx = render_hunk_lines_side_by_side(
                    &hunk.lines,
                    &line_comments,
                    &ctx,
                    line_idx,
                    &mut lines,
                );
            }
        }

        // Spacing between files
        let indicator = cursor_indicator(line_idx, ctx.current_line_idx);
        lines.push(Line::from(Span::styled(
            indicator,
            styles::current_line_indicator_style(),
        )));
        line_idx += 1;
    }

    let visible_lines_unscrolled: Vec<Line> = lines
        .into_iter()
        .skip(app.diff_state.scroll_offset)
        .take(inner.height as usize)
        .collect();

    let max_content_width = visible_lines_unscrolled
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.width())
                .sum::<usize>()
        })
        .max()
        .unwrap_or(0);

    app.diff_state.viewport_width = inner.width as usize;
    app.diff_state.max_content_width = max_content_width;

    let max_scroll_x = max_content_width.saturating_sub(inner.width as usize);
    if app.diff_state.scroll_x > max_scroll_x {
        app.diff_state.scroll_x = max_scroll_x;
    }

    let scroll_x = app.diff_state.scroll_x;
    let visible_lines: Vec<Line> = visible_lines_unscrolled
        .into_iter()
        .map(|line| apply_horizontal_scroll(line, scroll_x))
        .collect();

    let diff = Paragraph::new(visible_lines);
    frame.render_widget(diff, inner);
}

/// Process and render all diff lines in a hunk for side-by-side view
fn render_hunk_lines_side_by_side(
    hunk_lines: &[crate::model::DiffLine],
    line_comments: &std::collections::HashMap<u32, Vec<crate::model::Comment>>,
    ctx: &SideBySideContext,
    mut line_idx: usize,
    lines: &mut Vec<Line>,
) -> usize {
    let mut i = 0;
    while i < hunk_lines.len() {
        let diff_line = &hunk_lines[i];

        match diff_line.origin {
            LineOrigin::Context => {
                line_idx = render_context_line_side_by_side(
                    diff_line,
                    line_comments,
                    ctx,
                    line_idx,
                    lines,
                );
                i += 1;
            }
            LineOrigin::Deletion => {
                let (new_line_idx, lines_processed) = render_deletion_addition_pair_side_by_side(
                    hunk_lines,
                    i,
                    line_comments,
                    ctx,
                    line_idx,
                    lines,
                );
                line_idx = new_line_idx;
                i = lines_processed;
            }
            LineOrigin::Addition => {
                line_idx = render_standalone_addition_side_by_side(
                    diff_line,
                    line_comments,
                    ctx,
                    line_idx,
                    lines,
                );
                i += 1;
            }
        }
    }
    line_idx
}

/// Render a context line (appears on both sides)
fn render_context_line_side_by_side(
    diff_line: &crate::model::DiffLine,
    line_comments: &std::collections::HashMap<u32, Vec<crate::model::Comment>>,
    ctx: &SideBySideContext,
    mut line_idx: usize,
    lines: &mut Vec<Line>,
) -> usize {
    let line_num = diff_line
        .old_lineno
        .or(diff_line.new_lineno)
        .map(|n| format!("{:>4}", n))
        .unwrap_or_else(|| "    ".to_string());

    let indicator = cursor_indicator(line_idx, ctx.current_line_idx);

    let mut spans = vec![
        Span::styled(indicator, styles::current_line_indicator_style()),
        Span::styled(format!("{} ", line_num), styles::dim_style()),
        Span::styled(" ".to_string(), styles::diff_context_style()),
    ];

    // Left side content - use syntax highlighting if available
    if let Some(ref highlighted) = diff_line.highlighted_spans {
        let content_spans =
            truncate_or_pad_spans(highlighted, ctx.content_width, styles::diff_context_style());
        spans.extend(content_spans);
    } else {
        let content = truncate_or_pad(&diff_line.content, ctx.content_width);
        spans.push(Span::styled(content, styles::diff_context_style()));
    }

    // Separator
    spans.push(Span::styled(" │ ", styles::dim_style()));
    spans.push(Span::styled(format!("{} ", line_num), styles::dim_style()));
    spans.push(Span::styled(" ".to_string(), styles::diff_context_style()));

    // Right side content - use same highlighting
    if let Some(ref highlighted) = diff_line.highlighted_spans {
        let content_spans =
            truncate_or_pad_spans(highlighted, ctx.content_width, styles::diff_context_style());
        spans.extend(content_spans);
    } else {
        let content = truncate_or_pad(&diff_line.content, ctx.content_width);
        spans.push(Span::styled(content, styles::diff_context_style()));
    }

    lines.push(Line::from(spans));
    line_idx += 1;

    // Add comments if any
    if let Some(new_ln) = diff_line.new_lineno {
        line_idx = add_comments_to_line(new_ln, line_comments, LineSide::New, ctx, line_idx, lines);
    }

    line_idx
}

/// Render paired deletions and additions side-by-side
fn render_deletion_addition_pair_side_by_side(
    hunk_lines: &[crate::model::DiffLine],
    start_idx: usize,
    line_comments: &std::collections::HashMap<u32, Vec<crate::model::Comment>>,
    ctx: &SideBySideContext,
    mut line_idx: usize,
    lines: &mut Vec<Line>,
) -> (usize, usize) {
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

    // Render each pair of deletion/addition
    for offset in 0..max_lines {
        let indicator = cursor_indicator(line_idx, ctx.current_line_idx);

        let mut spans = vec![Span::styled(
            indicator,
            styles::current_line_indicator_style(),
        )];

        // Left side (deletion)
        if offset < del_count {
            let del_line = &hunk_lines[start_idx + offset];
            add_deletion_spans(&mut spans, del_line, ctx.content_width);
        } else {
            add_empty_column_spans(&mut spans, ctx.content_width);
        }

        spans.push(Span::styled(" │ ", styles::dim_style()));

        // Right side (addition)
        if offset < add_count {
            let add_line = &hunk_lines[add_start + offset];
            add_addition_spans(&mut spans, add_line, ctx.content_width);
        } else {
            add_empty_column_spans(&mut spans, ctx.content_width);
        }

        lines.push(Line::from(spans));
        line_idx += 1;

        // Add comments for deletion
        if offset < del_count {
            let del_line = &hunk_lines[start_idx + offset];
            if let Some(old_ln) = del_line.old_lineno {
                line_idx = add_comments_to_line(
                    old_ln,
                    line_comments,
                    LineSide::Old,
                    ctx,
                    line_idx,
                    lines,
                );
            }
        }

        // Add comments for addition
        if offset < add_count {
            let add_line = &hunk_lines[add_start + offset];
            if let Some(new_ln) = add_line.new_lineno {
                line_idx = add_comments_to_line(
                    new_ln,
                    line_comments,
                    LineSide::New,
                    ctx,
                    line_idx,
                    lines,
                );
            }
        }
    }

    (line_idx, add_end)
}

/// Render a standalone addition (no matching deletion)
fn render_standalone_addition_side_by_side(
    diff_line: &crate::model::DiffLine,
    line_comments: &std::collections::HashMap<u32, Vec<crate::model::Comment>>,
    ctx: &SideBySideContext,
    mut line_idx: usize,
    lines: &mut Vec<Line>,
) -> usize {
    let indicator = cursor_indicator(line_idx, ctx.current_line_idx);

    let mut spans = vec![Span::styled(
        indicator,
        styles::current_line_indicator_style(),
    )];
    add_empty_column_spans(&mut spans, ctx.content_width);
    spans.push(Span::styled(" │ ", styles::dim_style()));
    add_addition_spans(&mut spans, diff_line, ctx.content_width);

    lines.push(Line::from(spans));
    line_idx += 1;

    // Add comments if any
    if let Some(new_ln) = diff_line.new_lineno {
        line_idx = add_comments_to_line(new_ln, line_comments, LineSide::New, ctx, line_idx, lines);
    }

    line_idx
}

/// Add deletion line spans to the spans vector
fn add_deletion_spans(
    spans: &mut Vec<Span>,
    diff_line: &crate::model::DiffLine,
    content_width: usize,
) {
    let line_num = diff_line
        .old_lineno
        .map(|n| format!("{:>4}", n))
        .unwrap_or_else(|| "    ".to_string());

    spans.push(Span::styled(format!("{} ", line_num), styles::dim_style()));
    spans.push(Span::styled("-".to_string(), styles::diff_del_style()));

    // Use syntax highlighting if available
    if let Some(ref highlighted) = diff_line.highlighted_spans {
        let content_spans =
            truncate_or_pad_spans(highlighted, content_width, styles::diff_del_style());
        spans.extend(content_spans);
    } else {
        // Fall back to plain text
        let content = truncate_or_pad(&diff_line.content, content_width);
        spans.push(Span::styled(content, styles::diff_del_style()));
    }
}

/// Add addition line spans to the spans vector
fn add_addition_spans(
    spans: &mut Vec<Span>,
    diff_line: &crate::model::DiffLine,
    content_width: usize,
) {
    let line_num = diff_line
        .new_lineno
        .map(|n| format!("{:>4}", n))
        .unwrap_or_else(|| "    ".to_string());

    spans.push(Span::styled(format!("{} ", line_num), styles::dim_style()));
    spans.push(Span::styled("+".to_string(), styles::diff_add_style()));

    // Use syntax highlighting if available
    if let Some(ref highlighted) = diff_line.highlighted_spans {
        let content_spans =
            truncate_or_pad_spans(highlighted, content_width, styles::diff_add_style());
        spans.extend(content_spans);
    } else {
        // Fall back to plain text
        let content = truncate_or_pad(&diff_line.content, content_width);
        spans.push(Span::styled(content, styles::diff_add_style()));
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

/// Add comments for a specific line
fn add_comments_to_line(
    line_num: u32,
    line_comments: &std::collections::HashMap<u32, Vec<crate::model::Comment>>,
    side: LineSide,
    ctx: &SideBySideContext,
    mut line_idx: usize,
    lines: &mut Vec<Line>,
) -> usize {
    if let Some(comments) = line_comments.get(&line_num) {
        for comment in comments {
            let comment_side = comment.side.unwrap_or(LineSide::New);
            if (side == LineSide::Old && comment_side == LineSide::Old)
                || (side == LineSide::New && comment_side != LineSide::Old)
            {
                let comment_lines = comment_panel::format_comment_lines(
                    comment.comment_type,
                    &comment.content,
                    Some(line_num),
                );
                for mut comment_line in comment_lines {
                    let indicator = cursor_indicator(line_idx, ctx.current_line_idx);
                    comment_line.spans.insert(
                        0,
                        Span::styled(indicator, styles::current_line_indicator_style()),
                    );
                    lines.push(comment_line);
                    line_idx += 1;
                }
            }
        }
    }
    line_idx
}

/// Truncate or pad a string to a specific width
fn truncate_or_pad(s: &str, width: usize) -> String {
    let char_count = s.chars().count();
    if char_count > width {
        s.chars().take(width.saturating_sub(3)).collect::<String>() + "..."
    } else {
        format!("{:width$}", s, width = width)
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
