use ratatui::{
    Frame,
    layout::{Constraint, Flex, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::app::App;
use crate::model::{CommentType, LineRange};
use crate::theme::Theme;
use crate::ui::styles;

/// Format a comment input as multiple lines with a box border for inline editing
/// This mimics the normal comment display but shows it's being edited
pub fn format_comment_input_lines(
    theme: &Theme,
    comment_type: CommentType,
    buffer: &str,
    cursor_pos: usize,
    line_range: Option<LineRange>,
    is_editing: bool,
    supports_keyboard_enhancement: bool,
) -> Vec<Line<'static>> {
    let type_style = styles::comment_type_style(theme, comment_type);
    let border_style = styles::comment_border_style(theme, comment_type);
    let cursor_style = Style::default()
        .fg(theme.cursor_color)
        .add_modifier(Modifier::UNDERLINED);

    let action = if is_editing { "Edit" } else { "Add" };
    let line_info = match line_range {
        Some(range) if range.is_single() => format!("L{} ", range.start),
        Some(range) => format!("L{}-L{} ", range.start, range.end),
        None => String::new(),
    };

    let newline_hint = if supports_keyboard_enhancement {
        "Shift-Enter"
    } else {
        "Ctrl-J"
    };

    let mut result = Vec::new();

    // Top border with type label and hints
    result.push(Line::from(vec![
        Span::styled("     ╭─ ", border_style),
        Span::styled(format!("{} ", action), styles::dim_style(theme)),
        Span::styled(format!("[{}] ", comment_type.as_str()), type_style),
        Span::styled(line_info, styles::dim_style(theme)),
        Span::styled(
            format!("(Tab:type Enter:save {}:newline Esc:cancel)", newline_hint),
            styles::dim_style(theme),
        ),
    ]));

    // Content lines with cursor
    if buffer.is_empty() {
        // Show placeholder with cursor at start
        result.push(Line::from(vec![
            Span::styled("     │ ", border_style),
            Span::styled(" ", cursor_style),
            Span::styled("Type your comment...", styles::dim_style(theme)),
        ]));
    } else {
        // Split buffer into lines and render with cursor
        let buffer_lines: Vec<&str> = buffer.split('\n').collect();
        let mut char_offset = 0;

        for (line_idx, text) in buffer_lines.iter().enumerate() {
            let line_start = char_offset;
            let line_end = char_offset + text.len();

            // Check if cursor is on this line
            let cursor_on_this_line = cursor_pos >= line_start
                && (cursor_pos <= line_end
                    || (line_idx == buffer_lines.len() - 1 && cursor_pos == buffer.len()));

            let mut line_spans = vec![Span::styled("     │ ", border_style)];

            if cursor_on_this_line {
                let cursor_pos_in_line = cursor_pos - line_start;
                let cursor_pos_in_line = cursor_pos_in_line.min(text.len());
                let (before_cursor, after_cursor) = text.split_at(cursor_pos_in_line);
                if after_cursor.is_empty() {
                    line_spans.push(Span::raw(before_cursor.to_string()));
                    line_spans.push(Span::styled(" ", cursor_style));
                } else {
                    let mut chars = after_cursor.chars();
                    let cursor_char = chars.next().unwrap();
                    let remaining = chars.as_str();
                    line_spans.push(Span::raw(before_cursor.to_string()));
                    line_spans.push(Span::styled(cursor_char.to_string(), cursor_style));
                    line_spans.push(Span::raw(remaining.to_string()));
                }
            } else {
                line_spans.push(Span::raw(text.to_string()));
            }

            result.push(Line::from(line_spans));

            // Account for newline character (except for last line)
            char_offset = line_end + 1;
        }
    }

    // Bottom border
    result.push(Line::from(vec![Span::styled(
        "     ╰".to_string() + &"─".repeat(38),
        border_style,
    )]));

    result
}

/// Format a comment as multiple lines with a box border (themed version)
pub fn format_comment_lines(
    theme: &Theme,
    comment_type: CommentType,
    content: &str,
    line_range: Option<LineRange>,
) -> Vec<Line<'static>> {
    let type_style = styles::comment_type_style(theme, comment_type);
    let border_style = styles::comment_border_style(theme, comment_type);

    let line_info = match line_range {
        Some(range) if range.is_single() => format!("L{} ", range.start),
        Some(range) => format!("L{}-L{} ", range.start, range.end),
        None => String::new(),
    };
    let content_lines: Vec<&str> = content.split('\n').collect();

    let mut result = Vec::new();

    // Top border with type label
    result.push(Line::from(vec![
        Span::styled("     ╭─ ", border_style),
        Span::styled(format!("[{}] ", comment_type.as_str()), type_style),
        Span::styled(line_info, styles::dim_style(theme)),
        Span::styled("─".repeat(30), border_style),
    ]));

    // Content lines
    for line in &content_lines {
        result.push(Line::from(vec![
            Span::styled("     │ ", border_style),
            Span::raw(line.to_string()),
        ]));
    }

    // Bottom border
    result.push(Line::from(vec![Span::styled(
        "     ╰".to_string() + &"─".repeat(38),
        border_style,
    )]));

    result
}

pub fn render_confirm_dialog(frame: &mut Frame, app: &App, message: &str) {
    let theme = &app.theme;
    let area = centered_rect(50, 20, frame.area());

    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" Confirm ")
        .borders(Borders::ALL)
        .border_style(styles::border_style(theme, true));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines = vec![
        Line::from(""),
        Line::from(Span::raw(message)),
        Line::from(""),
        Line::from(vec![
            Span::styled("  [Y]", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw("es    "),
            Span::styled("[N]", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw("o"),
        ]),
    ];

    let paragraph = Paragraph::new(lines).alignment(ratatui::layout::Alignment::Center);
    frame.render_widget(paragraph, inner);
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::vertical([Constraint::Percentage(percent_y)]).flex(Flex::Center);
    let horizontal = Layout::horizontal([Constraint::Percentage(percent_x)]).flex(Flex::Center);
    let [area] = vertical.areas(area);
    let [area] = horizontal.areas(area);
    area
}
