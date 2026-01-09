use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::app::{App, DiffViewMode, FocusedPanel, InputMode};
use crate::model::{LineOrigin, LineSide};
use crate::ui::{comment_panel, help_popup, status_bar, styles};

pub fn render(frame: &mut Frame, app: &mut App) {
    let show_command_line = app.input_mode == InputMode::Command;

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(if show_command_line {
            vec![
                Constraint::Length(1), // Header
                Constraint::Min(0),    // Main content
                Constraint::Length(1), // Status bar
                Constraint::Length(1), // Command line
            ]
        } else {
            vec![
                Constraint::Length(1), // Header
                Constraint::Min(0),    // Main content
                Constraint::Length(1), // Status bar
            ]
        })
        .split(frame.area());

    status_bar::render_header(frame, app, chunks[0]);
    render_main_content(frame, app, chunks[1]);
    status_bar::render_status_bar(frame, app, chunks[2]);

    if show_command_line {
        status_bar::render_command_line(frame, app, chunks[3]);
    }

    // Render help popup on top if in help mode
    if app.input_mode == InputMode::Help {
        help_popup::render_help(frame);
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

fn render_main_content(frame: &mut Frame, app: &mut App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(20), // File list
            Constraint::Percentage(80), // Diff view
        ])
        .split(area);

    render_file_list(frame, app, chunks[0]);
    render_diff_view(frame, app, chunks[1]);
}

fn render_file_list(frame: &mut Frame, app: &App, area: Rect) {
    let focused = app.focused_panel == FocusedPanel::FileList;

    let block = Block::default()
        .title(" Files ")
        .borders(Borders::ALL)
        .border_style(styles::border_style(focused));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let items: Vec<Line> = app
        .diff_files
        .iter()
        .enumerate()
        .map(|(i, file)| {
            let path = file.display_path();
            let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
            let status = file.status.as_char();
            let is_reviewed = app.session.is_file_reviewed(path);
            let review_mark = if is_reviewed { "✓" } else { " " };
            let is_current = i == app.diff_state.current_file_idx;
            let pointer = if is_current { "▶" } else { " " };

            let style = if is_current {
                styles::selected_style()
            } else {
                Style::default()
            };

            Line::from(vec![
                Span::styled(pointer.to_string(), style),
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
            ])
        })
        .collect();

    let list = Paragraph::new(items);
    frame.render_widget(list, inner);
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

    for file in &app.diff_files {
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

            for hunk in &file.hunks {
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
                    lines.push(Line::from(vec![
                        Span::styled(indicator, styles::current_line_indicator_style()),
                        Span::styled(line_num, styles::dim_style()),
                        Span::styled(format!("{} {}", prefix, diff_line.content), style),
                    ]));
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

    // Apply scroll offset
    let scroll_x = app.diff_state.scroll_x;
    let visible_lines: Vec<Line> = lines
        .into_iter()
        .skip(app.diff_state.scroll_offset)
        .take(inner.height as usize)
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

    for file in &app.diff_files {
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

            for hunk in &file.hunks {
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

    // Apply scroll offset
    let scroll_x = app.diff_state.scroll_x;
    let visible_lines: Vec<Line> = lines
        .into_iter()
        .skip(app.diff_state.scroll_offset)
        .take(inner.height as usize)
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

    let content = truncate_or_pad(&diff_line.content, ctx.content_width);

    let indicator = cursor_indicator(line_idx, ctx.current_line_idx);

    lines.push(Line::from(vec![
        Span::styled(indicator, styles::current_line_indicator_style()),
        Span::styled(format!("{} ", line_num), styles::dim_style()),
        Span::styled(
            format!(" {}", content.clone()),
            styles::diff_context_style(),
        ),
        Span::styled(" │ ", styles::dim_style()),
        Span::styled(format!("{} ", line_num), styles::dim_style()),
        Span::styled(format!(" {}", content), styles::diff_context_style()),
    ]));
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
    let content = truncate_or_pad(&diff_line.content, content_width);
    spans.push(Span::styled(format!("{} ", line_num), styles::dim_style()));
    spans.push(Span::styled(
        format!("-{}", content),
        styles::diff_del_style(),
    ));
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
    let content = truncate_or_pad(&diff_line.content, content_width);
    spans.push(Span::styled(format!("{} ", line_num), styles::dim_style()));
    spans.push(Span::styled(
        format!("+{}", content),
        styles::diff_add_style(),
    ));
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
