use ratatui::{
    Frame,
    layout::{Constraint, Flex, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::App;
use crate::model::LineRange;
use crate::theme::Theme;
use crate::ui::styles;

/// Content prefix used on every comment-body line. 4 pad chars + `│` + 2 spaces.
/// The 4-char left pad lets the bar painter draw `│` up through diff lines
/// without colliding with the col-6 `▌` add/del prefix.
const BORDER_PREFIX: &str = "    │  ";
const BORDER_PREFIX_WIDTH: usize = 7;

/// Split `text` into segments whose display width each fits within `content_area`.
/// Returns a single-element vec when the text already fits. The returned slices
/// borrow from `text`, so the caller must keep `text` alive while iterating.
pub(crate) fn wrap_segments(text: &str, content_area: usize) -> Vec<&str> {
    if content_area == 0 || text.width() <= content_area {
        return vec![text];
    }
    let mut segments = Vec::new();
    let mut remaining = text;
    while !remaining.is_empty() {
        let mut take_bytes = 0usize;
        let mut taken_width = 0usize;
        for c in remaining.chars() {
            let cw = UnicodeWidthChar::width(c).unwrap_or(0);
            if taken_width + cw > content_area {
                break;
            }
            taken_width += cw;
            take_bytes += c.len_utf8();
        }
        // Single character wider than content_area — emit it anyway so we don't loop forever
        if take_bytes == 0 {
            take_bytes = remaining.chars().next().map_or(0, |c| c.len_utf8());
        }
        let (seg, rest) = remaining.split_at(take_bytes);
        segments.push(seg);
        remaining = rest;
    }
    segments
}

/// Push spans for a text segment containing the cursor. The cursor is rendered
/// as a styled span on the character at the split point, or a trailing space
/// when the cursor is at end-of-segment.
fn push_cursor_spans(
    spans: &mut Vec<Span<'static>>,
    before: &str,
    after: &str,
    cursor_style: Style,
) {
    spans.push(Span::raw(before.to_string()));
    // Cursor at end-of-segment: no trailing space so the line stays within
    // bounds. The terminal cursor (set_cursor_position) handles the visible
    // position without needing an extra styled space.
    let mut chars = after.chars();
    if let Some(cursor_char) = chars.next() {
        spans.push(Span::styled(cursor_char.to_string(), cursor_style));
        spans.push(Span::raw(chars.as_str().to_string()));
    }
}

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

#[derive(Debug, Clone)]
pub struct CommentTypePresentation {
    pub label: String,
    pub color: Color,
}

/// Format a comment input as multiple lines with a box border for inline editing.
/// This mimics the normal comment display but shows it's being edited.
///
/// Returns a tuple of (lines, cursor_info) where cursor_info contains the position
/// of the cursor within the formatted output for IME positioning.
#[allow(clippy::too_many_arguments)]
pub fn format_comment_input_lines(
    theme: &Theme,
    comment_type: CommentTypePresentation,
    buffer: &str,
    cursor_pos: usize,
    line_range: Option<LineRange>,
    is_editing: bool,
    supports_keyboard_enhancement: bool,
    width: usize,
) -> (Vec<Line<'static>>, CommentCursorInfo) {
    let type_style = styles::comment_type_style(theme, comment_type.color);
    let border_style = styles::comment_border_style(theme, comment_type.color);
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

    // "    │  " is the per-line content prefix; everything past that is content.
    // Subtract two extra: one so ratatui never wraps an exact-fit line, and
    // one so the terminal cursor at end-of-segment stays clear of the border.
    let content_area = width.saturating_sub(BORDER_PREFIX_WIDTH + 2);

    let mut result = Vec::new();
    let mut cursor_line_offset: usize = 1;
    let mut cursor_column: u16 = BORDER_PREFIX_WIDTH as u16;

    // Top-left corner becomes `├` when a line range is present — the bar
    // painter then draws `│` going up through the range and a `╭` at the
    // topmost covered line, so the tee reads as the bar joining the box.
    let top_corner = if line_range.is_some() { '├' } else { '╭' };
    let top_prefix = format!("    {top_corner}── ");

    // Top border with type label and hints
    result.push(Line::from(vec![
        Span::styled(top_prefix, border_style),
        Span::styled(format!("{} ", action), styles::dim_style(theme)),
        Span::styled(format!("[{}] ", comment_type.label), type_style),
        Span::styled(line_info, styles::dim_style(theme)),
        Span::styled(
            format!(
                "(Tab/S-Tab:type Enter:save {}:newline Esc:cancel)",
                newline_hint
            ),
            styles::dim_style(theme),
        ),
    ]));

    // Content lines with cursor
    if buffer.is_empty() {
        // Show placeholder with cursor at start
        result.push(Line::from(vec![
            Span::styled(BORDER_PREFIX, border_style),
            Span::styled(" ", cursor_style),
            Span::styled("Type your comment...", styles::dim_style(theme)),
        ]));
        // cursor_line_offset is already 1 (first content line)
        // cursor_column is already BORDER_PREFIX_WIDTH (cursor at start of content)
    } else {
        let buffer_lines: Vec<&str> = buffer.split('\n').collect();
        let mut byte_offset = 0;
        // Tracks how many visual lines have been pushed so far (not counting the header).
        let mut total_visual_lines: usize = 0;

        for (line_idx, text) in buffer_lines.iter().enumerate() {
            let line_start = byte_offset;
            let line_end = byte_offset + text.len();
            let is_last_logical = line_idx + 1 == buffer_lines.len();

            // Check if cursor is on this line
            let cursor_on_this_line = cursor_pos >= line_start
                && (cursor_pos <= line_end || (is_last_logical && cursor_pos == buffer.len()));

            // Pre-wrap this logical line into segments so ratatui never wraps it.
            // Short lines come back as a single-element vec.
            let segments = wrap_segments(text, content_area);
            let mut seg_byte_start = 0usize;

            for (seg_idx, seg) in segments.iter().enumerate() {
                let seg_start = line_start + seg_byte_start;
                let seg_end = seg_start + seg.len();
                let is_last_seg = seg_idx + 1 == segments.len();
                let cursor_in_seg = cursor_on_this_line
                    && cursor_pos >= seg_start
                    && (cursor_pos < seg_end || is_last_seg);

                let mut line_spans = vec![Span::styled(BORDER_PREFIX, border_style)];

                if cursor_in_seg {
                    let cursor_pos_in_seg = (cursor_pos - seg_start).min(seg.len());
                    let (before, after) = seg.split_at(cursor_pos_in_seg);

                    // Track cursor position for IME
                    cursor_line_offset = 1 + total_visual_lines;
                    cursor_column = BORDER_PREFIX_WIDTH as u16 + before.width() as u16;
                    push_cursor_spans(&mut line_spans, before, after, cursor_style);
                } else {
                    line_spans.push(Span::raw(seg.to_string()));
                }

                result.push(Line::from(line_spans));
                total_visual_lines += 1;
                seg_byte_start += seg.len();
            }

            // Account for newline character (except for last line)
            byte_offset = line_end + 1;
        }
    }

    // Bottom border — "    ╰" = 5 chars, fill to width
    result.push(Line::from(vec![Span::styled(
        "    ╰".to_string() + &"─".repeat(width.saturating_sub(5)),
        border_style,
    )]));

    let cursor_info = CommentCursorInfo {
        line_offset: cursor_line_offset,
        column: cursor_column,
    };

    (result, cursor_info)
}

/// Format an entire remote (read-only) forge review thread as one fused
/// box so it reads as a single discussion unit. Root comment opens the
/// box; replies appear as `├─ ↳ @author ──` separator headers within the
/// same box; the bottom rule appears once at the end.
///
/// Visually distinct from local drafts: the `[github @author]` badge on
/// the root header, and a muted palette throughout for resolved/outdated
/// threads.
pub fn format_remote_thread_lines(
    theme: &Theme,
    thread: &crate::forge::remote_comments::RemoteReviewThread,
    muted: bool,
) -> Vec<Line<'static>> {
    let (badge_fg, border_fg, body_fg) = if muted {
        (theme.fg_dim, theme.fg_dim, theme.fg_dim)
    } else {
        (
            theme.diff_hunk_header,
            theme.diff_hunk_header,
            theme.fg_secondary,
        )
    };

    let badge_style = Style::default().fg(badge_fg).add_modifier(Modifier::BOLD);
    let reply_badge_style = Style::default().fg(badge_fg);
    let border_style = Style::default().fg(border_fg);
    let body_style = Style::default().fg(body_fg);

    let line_info = match thread.line.map(LineRange::single) {
        Some(range) if range.is_single() => format!("L{} ", range.start),
        Some(range) => format!("L{}-L{} ", range.start, range.end),
        None => String::new(),
    };

    // Remote review threads always anchor on a specific line/range, so the
    // top corner is a tee — the bar painter draws the rest going up.
    let mut result = Vec::new();
    let mut iter = thread.comments.iter().peekable();
    let mut is_first = true;
    while let Some(comment) = iter.next() {
        let author = comment.author.as_deref().unwrap_or("unknown");
        if is_first {
            let mut badge_text = format!("[github @{author}");
            if thread.is_resolved {
                badge_text.push_str(" resolved");
            } else if thread.is_outdated {
                badge_text.push_str(" outdated");
            }
            badge_text.push_str("] ");
            result.push(Line::from(vec![
                Span::styled("    ├── ".to_string(), border_style),
                Span::styled(badge_text, badge_style),
                Span::styled(line_info.clone(), styles::dim_style(theme)),
                Span::styled("─".repeat(20), border_style),
            ]));
        } else {
            result.push(Line::from(vec![
                Span::styled("    ├── ".to_string(), border_style),
                Span::styled(format!("↳ @{author} "), reply_badge_style),
                Span::styled("─".repeat(28), border_style),
            ]));
        }

        for line in comment.body.split('\n') {
            result.push(Line::from(vec![
                Span::styled("    │  ".to_string(), border_style),
                Span::styled(line.to_string(), body_style),
            ]));
        }

        is_first = false;
        let _ = iter.peek();
    }

    result.push(Line::from(vec![Span::styled(
        "    ╰".to_string() + &"─".repeat(39),
        border_style,
    )]));

    result
}

/// Format a comment as multiple lines with a box border (themed version)
pub fn format_comment_lines(
    theme: &Theme,
    comment_type: CommentTypePresentation,
    content: &str,
    line_range: Option<LineRange>,
    width: usize,
) -> Vec<Line<'static>> {
    let type_style = styles::comment_type_style(theme, comment_type.color);
    let border_style = styles::comment_border_style(theme, comment_type.color);

    let line_info = match line_range {
        Some(range) if range.is_single() => format!("L{} ", range.start),
        Some(range) => format!("L{}-L{} ", range.start, range.end),
        None => String::new(),
    };

    // "    │  " is the per-line content prefix; everything past that is content.
    // Subtract two extra: one so ratatui never wraps an exact-fit line, and
    // one so the terminal cursor at end-of-segment stays clear of the border.
    let content_area = width.saturating_sub(BORDER_PREFIX_WIDTH + 2);

    let content_lines: Vec<&str> = content.split('\n').collect();

    let mut result = Vec::new();

    let top_corner = if line_range.is_some() { '├' } else { '╭' };
    let top_prefix = format!("    {top_corner}── ");

    // Top border — fill dynamically so total line = width.
    // top_prefix = 8, "[LABEL] " = label.len()+3, line_info = variable
    let top_fill = width.saturating_sub(8 + comment_type.label.len() + 3 + line_info.width());
    result.push(Line::from(vec![
        Span::styled(top_prefix, border_style),
        Span::styled(format!("[{}] ", comment_type.label), type_style),
        Span::styled(line_info, styles::dim_style(theme)),
        Span::styled("─".repeat(top_fill), border_style),
    ]));

    // Content lines — pre-wrap at content_area so ratatui never wraps them
    for line in &content_lines {
        for seg in wrap_segments(line, content_area) {
            result.push(Line::from(vec![
                Span::styled(BORDER_PREFIX, border_style),
                Span::raw(seg.to_string()),
            ]));
        }
    }

    // Bottom border — "    ╰" = 5 chars, fill to width
    result.push(Line::from(vec![Span::styled(
        "    ╰".to_string() + &"─".repeat(width.saturating_sub(5)),
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
        .style(styles::popup_style(theme))
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

    let paragraph = Paragraph::new(lines)
        .style(styles::popup_style(theme))
        .alignment(ratatui::layout::Alignment::Center);
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
    use ratatui::style::Color;

    fn test_theme() -> Theme {
        Theme::default()
    }

    // -- wrap_segments tests --

    #[test]
    fn wrap_segments_returns_single_segment_when_text_fits() {
        // given
        let text = "hello";

        // when
        let segments = wrap_segments(text, 80);

        // then
        assert_eq!(segments, vec!["hello"]);
    }

    #[test]
    fn wrap_segments_returns_single_segment_for_empty_text() {
        // given
        let text = "";

        // when
        let segments = wrap_segments(text, 80);

        // then
        assert_eq!(segments, vec![""]);
    }

    #[test]
    fn wrap_segments_returns_text_unchanged_when_content_area_is_zero() {
        // given
        let text = "anything";

        // when
        let segments = wrap_segments(text, 0);

        // then
        assert_eq!(segments, vec!["anything"]);
    }

    #[test]
    fn wrap_segments_splits_long_ascii_at_content_area() {
        // given
        let text = "hello world";

        // when - content_area=5 means each segment is at most 5 display cols
        let segments = wrap_segments(text, 5);

        // then
        assert_eq!(segments, vec!["hello", " worl", "d"]);
    }

    #[test]
    fn wrap_segments_respects_cjk_display_width() {
        // given - each CJK char is 2 display cols, total = 8 cols, 12 bytes
        let text = "中文测试";

        // when - content_area=4 fits exactly 2 CJK chars per segment
        let segments = wrap_segments(text, 4);

        // then
        assert_eq!(segments, vec!["中文", "测试"]);
    }

    #[test]
    fn wrap_segments_handles_mixed_ascii_and_cjk() {
        // given - 'a'(1) + '中'(2) + 'b'(1) + '文'(2) = 6 display cols
        let text = "a中b文";

        // when - content_area=3 fits "a中" (1+2=3), then "b文" (1+2=3)
        let segments = wrap_segments(text, 3);

        // then
        assert_eq!(segments, vec!["a中", "b文"]);
    }

    #[test]
    fn wrap_segments_emits_oversized_char_to_avoid_infinite_loop() {
        // given - '中' is 2 cols wide but content_area only allows 1
        let text = "中a";

        // when
        let segments = wrap_segments(text, 1);

        // then - '中' is emitted even though it exceeds content_area
        assert_eq!(segments, vec!["中", "a"]);
    }

    #[test]
    fn wrap_segments_handles_exact_width_boundary() {
        // given - text exactly fills content_area
        let text = "12345";

        // when
        let segments = wrap_segments(text, 5);

        // then - single segment, no spurious empty trailing segment
        assert_eq!(segments, vec!["12345"]);
    }

    // -- format_comment_input_lines tests --

    #[test]
    fn should_return_cursor_at_start_for_empty_buffer() {
        // given
        let theme = test_theme();

        // when
        let (lines, cursor_info) = format_comment_input_lines(
            &theme,
            CommentTypePresentation {
                label: "NOTE".to_string(),
                color: Color::Blue,
            },
            "",
            0,
            None,
            false,
            false,
            80,
        );

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
            CommentTypePresentation {
                label: "NOTE".to_string(),
                color: Color::Blue,
            },
            buffer,
            cursor_pos,
            None,
            false,
            false,
            80,
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
            CommentTypePresentation {
                label: "NOTE".to_string(),
                color: Color::Blue,
            },
            buffer,
            cursor_pos,
            None,
            false,
            false,
            80,
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
            CommentTypePresentation {
                label: "NOTE".to_string(),
                color: Color::Blue,
            },
            buffer,
            cursor_pos,
            None,
            false,
            false,
            80,
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
            CommentTypePresentation {
                label: "NOTE".to_string(),
                color: Color::Blue,
            },
            buffer,
            cursor_pos,
            None,
            false,
            false,
            80,
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
            CommentTypePresentation {
                label: "NOTE".to_string(),
                color: Color::Blue,
            },
            buffer,
            cursor_pos,
            None,
            false,
            false,
            80,
        );

        // then
        assert_eq!(cursor_info.line_offset, 1);
        // "a" = 1 display width, "좋" = 2 display width, total = 3
        assert_eq!(cursor_info.column, 7 + 3);
    }
}
