use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::app::{App, FocusedPanel, InputMode};
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

            let is_reviewed = app
                .session
                .files
                .get(path)
                .map(|r| r.reviewed)
                .unwrap_or(false);

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
    let focused = app.focused_panel == FocusedPanel::Diff;

    let block = Block::default()
        .title(" Diff ")
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

        // File header
        let is_current = line_idx == current_line_idx;
        let indicator = if is_current { "▶ " } else { "  " };
        lines.push(Line::from(vec![
            Span::styled(indicator, styles::current_line_indicator_style()),
            Span::styled(
                format!("═══ {} [{}] ", path.display(), status),
                styles::file_header_style(),
            ),
            Span::styled("═".repeat(40), styles::file_header_style()),
        ]));
        line_idx += 1;

        // Show file-level comments right after the header
        if let Some(review) = app.session.files.get(path) {
            for comment in &review.file_comments {
                let comment_lines = comment_panel::format_comment_lines(
                    comment.comment_type,
                    &comment.content,
                    None,
                );
                for mut comment_line in comment_lines {
                    let is_current = line_idx == current_line_idx;
                    let indicator = if is_current { "▶" } else { " " };
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
            let is_current = line_idx == current_line_idx;
            let indicator = if is_current { "▶ " } else { "  " };
            lines.push(Line::from(vec![
                Span::styled(indicator, styles::current_line_indicator_style()),
                Span::styled("(binary file)", styles::dim_style()),
            ]));
            line_idx += 1;
        } else if file.hunks.is_empty() {
            let is_current = line_idx == current_line_idx;
            let indicator = if is_current { "▶ " } else { "  " };
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
                let is_current = line_idx == current_line_idx;
                let indicator = if is_current { "▶ " } else { "  " };
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

                    let is_current = line_idx == current_line_idx;
                    let indicator = if is_current { "▶" } else { " " };
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
                }
            }
        }

        // Spacing between files
        let is_current = line_idx == current_line_idx;
        let indicator = if is_current { "▶" } else { " " };
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
