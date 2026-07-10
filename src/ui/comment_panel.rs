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

/// Emit spans covering `line_text[start..end)` using the per-line markdown
/// highlight `runs` (concatenation of run text equals `line_text`). Falls back
/// to a single unstyled span when highlighting is unavailable. Offsets are byte
/// positions; callers pass char-boundary offsets (segment/cursor boundaries).
fn highlighted_window_spans(
    runs: Option<&[(Style, String)]>,
    line_text: &str,
    start: usize,
    end: usize,
) -> Vec<Span<'static>> {
    if start >= end {
        return Vec::new();
    }
    let Some(runs) = runs else {
        return vec![Span::raw(line_text[start..end].to_string())];
    };
    let mut out = Vec::new();
    let mut run_start = 0usize;
    for (style, text) in runs {
        let run_end = run_start + text.len();
        let lo = start.max(run_start);
        let hi = end.min(run_end);
        if lo < hi {
            out.push(Span::styled(
                text[lo - run_start..hi - run_start].to_string(),
                *style,
            ));
        }
        run_start = run_end;
        if run_start >= end {
            break;
        }
    }
    out
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
    width: usize,
    vim_mode: Option<(&str, bool)>,
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

    let newline_hint = "Shift-Enter"; // requires extended-keys on in tmux; Alt-Enter also works

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

    // Top border with type label and hints. In vim mode the hints describe the
    // modal bindings and a `[MODE]` tag is shown after the type label.
    let hint = match vim_mode {
        Some(_) => "(i:insert  Alt-Enter:save  Esc:normal  :w save  :q discard)".to_string(),
        None => format!("(Tab/S-Tab:type Enter:save {newline_hint}:newline Esc:cancel)"),
    };
    let mut header_spans = vec![
        Span::styled(top_prefix, border_style),
        Span::styled(format!("{action} "), styles::dim_style(theme)),
    ];
    // `None` has an empty label — show no `[TYPE]` badge while composing.
    if !comment_type.label.is_empty() {
        header_spans.push(Span::styled(
            format!("[{}] ", comment_type.label),
            type_style,
        ));
    }
    if let Some((mode, warn)) = vim_mode {
        // The cancel-confirm hint is painted red to flag the destructive action.
        let mode_style = if warn {
            Style::default()
                .fg(theme.comment_issue)
                .add_modifier(Modifier::BOLD)
        } else {
            type_style
        };
        header_spans.push(Span::styled(format!("[{mode}] "), mode_style));
    }
    header_spans.push(Span::styled(line_info, styles::dim_style(theme)));
    header_spans.push(Span::styled(hint, styles::dim_style(theme)));
    result.push(Line::from(header_spans));

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
        // Markdown-highlight the in-progress text; colors come from the active
        // syntect theme (same engine/theme as diff code highlighting).
        let owned_lines: Vec<String> = buffer_lines.iter().map(|s| s.to_string()).collect();
        let highlighted = theme
            .syntax_highlighter()
            .highlight_markdown_lines(&owned_lines);
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
                let line_runs = highlighted.get(line_idx).and_then(|o| o.as_deref());
                let seg_end_in_line = seg_byte_start + seg.len();

                if cursor_in_seg {
                    let cursor_pos_in_seg = (cursor_pos - seg_start).min(seg.len());
                    let (before, after) = seg.split_at(cursor_pos_in_seg);
                    let cursor_in_line = seg_byte_start + cursor_pos_in_seg;

                    // Track cursor position for IME
                    cursor_line_offset = 1 + total_visual_lines;
                    cursor_column = BORDER_PREFIX_WIDTH as u16 + before.width() as u16;

                    // before-cursor text, highlighted
                    line_spans.extend(highlighted_window_spans(
                        line_runs,
                        text,
                        seg_byte_start,
                        cursor_in_line,
                    ));
                    // Cursor char gets the cursor style; the rest stays highlighted.
                    // No trailing cell at end-of-segment — the terminal cursor
                    // (set_cursor_position) handles that position.
                    if let Some(cursor_char) = after.chars().next() {
                        line_spans.push(Span::styled(cursor_char.to_string(), cursor_style));
                        line_spans.extend(highlighted_window_spans(
                            line_runs,
                            text,
                            cursor_in_line + cursor_char.len_utf8(),
                            seg_end_in_line,
                        ));
                    }
                } else {
                    line_spans.extend(highlighted_window_spans(
                        line_runs,
                        text,
                        seg_byte_start,
                        seg_end_in_line,
                    ));
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

/// Format a remote review summary (the body of a `PullRequestReview`) as a
/// box with a `[github @author <state>]` header. Renders at review scope —
/// no line anchor — so the top corner is `╭`, not the line-anchored `├`.
pub fn format_remote_review_summary_lines(
    theme: &Theme,
    summary: &crate::forge::remote_comments::RemoteReviewSummary,
) -> Vec<Line<'static>> {
    let badge_fg = theme.diff_hunk_header;
    let border_fg = theme.diff_hunk_header;
    let body_fg = theme.fg_secondary;

    let badge_style = Style::default().fg(badge_fg).add_modifier(Modifier::BOLD);
    let border_style = Style::default().fg(border_fg);
    let body_style = Style::default().fg(body_fg);

    let author = summary.author.as_deref().unwrap_or("unknown");
    let mut badge_text = format!("[github @{author}");
    if let Some(state_label) = summary.state.badge_label() {
        badge_text.push(' ');
        badge_text.push_str(state_label);
    }
    badge_text.push_str("] ");

    let mut result = Vec::new();
    result.push(Line::from(vec![
        Span::styled("    ╭── ".to_string(), border_style),
        Span::styled(badge_text, badge_style),
        Span::styled("─".repeat(28), border_style),
    ]));

    for line in summary.body.split('\n') {
        result.push(Line::from(vec![
            Span::styled("    │  ".to_string(), border_style),
            Span::styled(line.to_string(), body_style),
        ]));
    }

    result.push(Line::from(vec![Span::styled(
        "    ╰".to_string() + &"─".repeat(39),
        border_style,
    )]));

    result
}

/// Format a comment as multiple lines with a box border (themed version).
///
/// `author` advertises the comment's author in the top-row badge and tints
/// the box border. Callers pass `Some(name)` for non-self comments — the
/// resulting badge reads `[TYPE @name]`, mirroring the `[github @author]`
/// format used for remote PR threads. `None` keeps the existing neutral
/// `[TYPE]` badge and theme border.
/// Render `content` as markdown-highlighted, border-prefixed, pre-wrapped lines
/// (no cursor). Colors come from the active syntect theme. Used for displayed
/// comment bodies; the editor box does its own variant with cursor handling.
fn markdown_body_lines(
    theme: &Theme,
    content: &str,
    content_area: usize,
    border_style: Style,
) -> Vec<Line<'static>> {
    let lines: Vec<&str> = content.split('\n').collect();
    let owned: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
    // Highlight all lines together so multi-line constructs (e.g. fenced code)
    // carry state across lines.
    let highlighted = theme.syntax_highlighter().highlight_markdown_lines(&owned);

    let mut out = Vec::new();
    for (idx, text) in lines.iter().enumerate() {
        let runs = highlighted.get(idx).and_then(|o| o.as_deref());
        let mut seg_start = 0usize;
        for seg in wrap_segments(text, content_area) {
            let seg_end = seg_start + seg.len();
            let mut spans = vec![Span::styled(BORDER_PREFIX, border_style)];
            spans.extend(highlighted_window_spans(runs, text, seg_start, seg_end));
            out.push(Line::from(spans));
            seg_start = seg_end;
        }
    }
    out
}

pub fn format_comment_lines(
    theme: &Theme,
    comment_type: CommentTypePresentation,
    content: &str,
    line_range: Option<LineRange>,
    width: usize,
    author: Option<&str>,
) -> Vec<Line<'static>> {
    let type_style = styles::comment_type_style(theme, comment_type.color);
    let border_style = match author {
        Some(name) => Style::default()
            .fg(styles::author_color_for(name))
            .add_modifier(ratatui::style::Modifier::BOLD),
        None => styles::comment_border_style(theme, comment_type.color),
    };

    // `None` comments have an empty label: drop the `[TYPE]` badge, keeping the
    // author tag when present so per-author coloring still reads.
    let badge_text = match (author, comment_type.label.is_empty()) {
        (Some(name), true) => format!("[@{name}] "),
        (Some(name), false) => format!("[{} @{name}] ", comment_type.label),
        (None, true) => String::new(),
        (None, false) => format!("[{}] ", comment_type.label),
    };
    let badge_width = badge_text.width();

    let line_info = match line_range {
        Some(range) if range.is_single() => format!("L{} ", range.start),
        Some(range) => format!("L{}-L{} ", range.start, range.end),
        None => String::new(),
    };

    // "    │  " is the per-line content prefix; everything past that is content.
    // Subtract two extra: one so ratatui never wraps an exact-fit line, and
    // one so the terminal cursor at end-of-segment stays clear of the border.
    let content_area = width.saturating_sub(BORDER_PREFIX_WIDTH + 2);

    let mut result = Vec::new();

    let top_corner = if line_range.is_some() { '├' } else { '╭' };
    let top_prefix = format!("    {top_corner}── ");

    // Top border — fill dynamically so total line = width.
    // top_prefix = 8 cols, then the badge (width depends on author), then
    // optional line_info, then `─` fill out to `width`.
    let top_fill = width.saturating_sub(8 + badge_width + line_info.width());
    result.push(Line::from(vec![
        Span::styled(top_prefix, border_style),
        Span::styled(badge_text, type_style),
        Span::styled(line_info, styles::dim_style(theme)),
        Span::styled("─".repeat(top_fill), border_style),
    ]));

    // Content lines — markdown-highlighted, pre-wrapped at content_area.
    result.extend(markdown_body_lines(
        theme,
        content,
        content_area,
        border_style,
    ));

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
            80,
            None,
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
            80,
            None,
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
            80,
            None,
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
            80,
            None,
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
            80,
            None,
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
            80,
            None,
        );

        // then
        assert_eq!(cursor_info.line_offset, 1);
        // "a" = 1 display width, "좋" = 2 display width, total = 3
        assert_eq!(cursor_info.column, 7 + 3);
    }

    // -- markdown highlighting tests --

    /// Reconstruct the buffer text from the rendered content lines: drop the
    /// header (first) and footer (last) lines, skip each line's BORDER_PREFIX
    /// span, concatenate the rest, and join visual lines with '\n'. With a wide
    /// width (no wrapping) each logical line is one visual line, so this must
    /// equal the original buffer regardless of how content is split into spans.
    fn reconstruct(lines: &[Line<'static>]) -> String {
        lines[1..lines.len() - 1]
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .skip(1) // BORDER_PREFIX
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn render_md(buffer: &str, cursor: usize) -> Vec<Line<'static>> {
        let theme = test_theme();
        format_comment_input_lines(
            &theme,
            CommentTypePresentation {
                label: "NOTE".to_string(),
                color: Color::Blue,
            },
            buffer,
            cursor,
            None,
            false,
            80,
            None,
        )
        .0
    }

    #[test]
    fn markdown_render_preserves_all_text() {
        let buffer = "# Title\n**bold** and `code`\n- item";
        // cursor in the middle of the bold span
        let lines = render_md(buffer, 11);
        assert_eq!(reconstruct(&lines), buffer);
    }

    #[test]
    fn markdown_render_preserves_multibyte_text() {
        let buffer = "# 世界\n**bold** 좋아";
        for cursor in [0, buffer.find('世').unwrap(), buffer.len()] {
            let lines = render_md(buffer, cursor);
            assert_eq!(reconstruct(&lines), buffer, "cursor={cursor}");
        }
    }

    #[test]
    fn displayed_comment_is_markdown_highlighted_and_preserves_text() {
        let theme = test_theme();
        let content = "# Heading\n**bold** and `code`\n- item";
        let lines = format_comment_lines(
            &theme,
            CommentTypePresentation {
                label: "NOTE".to_string(),
                color: Color::Blue,
            },
            content,
            None,
            80,
            None,
        );
        // Header + footer wrap the body; reconstruct must round-trip the text.
        assert_eq!(reconstruct(&lines), content);
        // The inline-code line should be split into multiple styled runs.
        let code_line = &lines[2]; // header, heading, **this**
        assert!(
            code_line.spans.len() - 1 > 1,
            "expected displayed markdown to be highlighted into multiple spans"
        );
    }

    #[test]
    fn markdown_highlighting_splits_line_into_runs() {
        // An inline-code line should yield multiple styled content spans (proof
        // the markdown grammar resolved and coloring is applied), not one raw
        // span. Cursor at end so no cursor cell splits the line artificially.
        let buffer = "plain `code` plain";
        let lines = render_md(buffer, buffer.len());
        let content_spans = lines[1].spans.len() - 1; // minus BORDER_PREFIX
        assert!(
            content_spans > 1,
            "expected markdown highlighting to split the line, got {content_spans} span(s)"
        );
    }
}
