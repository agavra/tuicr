use ratatui::{
    Frame,
    layout::{Constraint, Flex, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};
use unicode_width::UnicodeWidthStr;

use crate::app::App;
use crate::model::{CommentType, LineRange};
use crate::theme::Theme;
use crate::ui::styles;

/// Information about where the cursor should be positioned within comment input
#[derive(Debug, Clone)]
pub struct CommentCursorInfo {
    /// Which line within the formatted output contains the cursor (0-indexed, relative to content start)
    /// This is the line index within the Vec<Line> returned by format_comment_input_lines,
    /// where 0 = header line, 1+ = content lines, last = footer line.
    /// The cursor is only on content lines (1 to n-2 inclusive for n total lines).
    pub line_offset: usize,
    /// Column offset (display width) from start of line where cursor should be
    pub column: u16,
}

/// Format a comment input as multiple lines with a box border for inline editing.
/// This mimics the normal comment display but shows it's being edited.
///
/// Returns a tuple of (lines, cursor_info) where cursor_info contains the position
/// of the cursor within the formatted output for IME positioning.
pub fn format_comment_input_lines(
    theme: &Theme,
    comment_type: CommentType,
    buffer: &str,
    cursor_pos: usize,
    line_range: Option<LineRange>,
    is_editing: bool,
    supports_keyboard_enhancement: bool,
) -> (Vec<Line<'static>>, CommentCursorInfo) {
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
    // Track cursor position: line offset within result, column (display width)
    // Default to first content line (index 1) with cursor at start of content (after border)
    let border_prefix = "     │ ";
    let border_width = border_prefix.width() as u16;
    let mut cursor_line_offset: usize = 1; // First content line (after header)
    let mut cursor_column: u16 = border_width; // After the border prefix

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
            Span::styled(border_prefix, border_style),
            Span::styled(" ", cursor_style),
            Span::styled("Type your comment...", styles::dim_style(theme)),
        ]));
        // cursor_line_offset is already 1 (first content line)
        // cursor_column is already border_width (cursor at start of content)
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

            let mut line_spans = vec![Span::styled(border_prefix, border_style)];

            if cursor_on_this_line {
                let cursor_pos_in_line = cursor_pos - line_start;
                let cursor_pos_in_line = cursor_pos_in_line.min(text.len());
                let (before_cursor, after_cursor) = text.split_at(cursor_pos_in_line);

                // Track cursor position for IME
                // line_offset: header (1) + current content line index
                cursor_line_offset = 1 + line_idx;
                // column: border width + display width of text before cursor
                cursor_column = border_width + before_cursor.width() as u16;

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

    let cursor_info = CommentCursorInfo {
        line_offset: cursor_line_offset,
        column: cursor_column,
    };

    (result, cursor_info)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;

    fn test_theme() -> Theme {
        Theme::default()
    }

    #[test]
    fn should_return_cursor_at_start_for_empty_buffer() {
        // given
        let theme = test_theme();

        // when
        let (lines, cursor_info) =
            format_comment_input_lines(&theme, CommentType::Note, "", 0, None, false, false);

        // then
        assert_eq!(lines.len(), 3); // header + content + footer
        assert_eq!(cursor_info.line_offset, 1); // cursor on first content line
        assert_eq!(cursor_info.column, 7); // "     │ " = 7 chars
    }

    #[test]
    fn should_return_cursor_position_for_ascii_text() {
        // given
        let theme = test_theme();
        let buffer = "hello";
        let cursor_pos = 3; // cursor after "hel"

        // when
        let (_, cursor_info) = format_comment_input_lines(
            &theme,
            CommentType::Note,
            buffer,
            cursor_pos,
            None,
            false,
            false,
        );

        // then
        assert_eq!(cursor_info.line_offset, 1); // first content line
        assert_eq!(cursor_info.column, 7 + 3); // border + "hel"
    }

    #[test]
    fn should_return_cursor_position_for_multibyte_text() {
        // given
        let theme = test_theme();
        let buffer = "안녕"; // 2 multibyte chars, 6 bytes, 4 display columns
        let cursor_pos = 3; // cursor after first multibyte char (after "안")

        // when
        let (_, cursor_info) = format_comment_input_lines(
            &theme,
            CommentType::Note,
            buffer,
            cursor_pos,
            None,
            false,
            false,
        );

        // then
        assert_eq!(cursor_info.line_offset, 1);
        // "안" has display width 2, so cursor column = border(7) + 2 = 9
        assert_eq!(cursor_info.column, 7 + 2);
    }

    #[test]
    fn should_return_cursor_position_at_end_of_text() {
        // given
        let theme = test_theme();
        let buffer = "test";
        let cursor_pos = 4; // cursor at end

        // when
        let (_, cursor_info) = format_comment_input_lines(
            &theme,
            CommentType::Note,
            buffer,
            cursor_pos,
            None,
            false,
            false,
        );

        // then
        assert_eq!(cursor_info.line_offset, 1);
        assert_eq!(cursor_info.column, 7 + 4); // border + "test"
    }

    #[test]
    fn should_return_cursor_position_on_second_line() {
        // given
        let theme = test_theme();
        let buffer = "line1\nline2";
        let cursor_pos = 8; // cursor after "li" in "line2"

        // when
        let (lines, cursor_info) = format_comment_input_lines(
            &theme,
            CommentType::Note,
            buffer,
            cursor_pos,
            None,
            false,
            false,
        );

        // then
        assert_eq!(lines.len(), 4); // header + 2 content lines + footer
        assert_eq!(cursor_info.line_offset, 2); // second content line (0=header, 1=line1, 2=line2)
        assert_eq!(cursor_info.column, 7 + 2); // border + "li"
    }

    #[test]
    fn should_return_cursor_position_for_mixed_content() {
        // given
        let theme = test_theme();
        let buffer = "a좋b"; // 1 + 3 + 1 = 5 bytes, 1 + 2 + 1 = 4 display columns
        let cursor_pos = 4; // cursor after "a좋" (1 + 3 bytes)

        // when
        let (_, cursor_info) = format_comment_input_lines(
            &theme,
            CommentType::Note,
            buffer,
            cursor_pos,
            None,
            false,
            false,
        );

        // then
        assert_eq!(cursor_info.line_offset, 1);
        // "a" = 1 display width, "좋" = 2 display width, total = 3
        assert_eq!(cursor_info.column, 7 + 3);
    }
}
