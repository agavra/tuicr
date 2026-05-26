use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::theme::Theme;
use crate::ui::styles;

pub(super) fn expand_wrapped_lines<'a>(
    logical_lines: Vec<Line<'a>>,
    gutter_width: usize,
    viewport_width: usize,
    theme: &Theme,
) -> Vec<Line<'a>> {
    let content_width = viewport_width.saturating_sub(gutter_width);
    if content_width == 0 {
        return logical_lines;
    }

    let mut expanded = Vec::new();

    for line in logical_lines {
        let total_width: usize = line.spans.iter().map(|s| s.content.width()).sum();
        if total_width <= viewport_width {
            expanded.push(line);
            continue;
        }

        let (gutter_spans, content_spans) = split_spans_at_width(&line.spans, gutter_width);
        let content_text: String = content_spans.iter().map(|s| s.content.as_ref()).collect();
        let content_text_width = content_text.width();

        if content_text_width <= content_width {
            expanded.push(line);
            continue;
        }

        let mut byte_offset = 0;
        let mut row_index = 0;

        while byte_offset < content_text.len() {
            let row_width_limit = content_width;
            let mut row_end_byte = byte_offset;
            let mut row_width = 0;

            for character in content_text[byte_offset..].chars() {
                let character_width =
                    unicode_width::UnicodeWidthChar::width(character).unwrap_or(0);
                if row_width + character_width > row_width_limit {
                    break;
                }
                row_width += character_width;
                row_end_byte += character.len_utf8();
            }

            if row_end_byte == byte_offset && byte_offset < content_text.len() {
                let character = content_text[byte_offset..].chars().next().unwrap();
                row_end_byte += character.len_utf8();
            }

            let row_spans = slice_spans_by_bytes(&content_spans, byte_offset, row_end_byte);

            let row_line = if row_index == 0 {
                let mut spans = gutter_spans.clone();
                spans.extend(row_spans);
                Line::from(spans)
            } else {
                let indicator_span = Span::styled(" ", styles::current_line_indicator_style(theme));
                let linenum_width = gutter_width.saturating_sub(4);
                let linenum_span = Span::styled(
                    format!("{:>w$}↪", "", w = linenum_width),
                    styles::dim_style(theme),
                );
                let prefix_span = gutter_spans
                    .get(2)
                    .cloned()
                    .unwrap_or_else(|| Span::raw("  "));
                let mut spans = vec![indicator_span, linenum_span, prefix_span];
                spans.extend(row_spans);
                Line::from(spans)
            };

            expanded.push(row_line);
            byte_offset = row_end_byte;
            row_index += 1;
        }

        if row_index == 0 {
            expanded.push(line);
        }
    }

    expanded
}

fn split_spans_at_width<'a>(
    spans: &[Span<'a>],
    split_width: usize,
) -> (Vec<Span<'a>>, Vec<Span<'a>>) {
    let mut left = Vec::new();
    let mut right = Vec::new();
    let mut consumed = 0;

    for span in spans {
        let span_width = span.content.width();
        if consumed >= split_width {
            right.push(span.clone());
        } else if consumed + span_width <= split_width {
            left.push(span.clone());
            consumed += span_width;
        } else {
            let chars_needed = split_width - consumed;
            let mut left_content = String::new();
            let mut right_content = String::new();
            let mut char_width = 0;
            for character in span.content.chars() {
                let cw = unicode_width::UnicodeWidthChar::width(character).unwrap_or(0);
                if char_width + cw <= chars_needed {
                    left_content.push(character);
                    char_width += cw;
                } else {
                    right_content.push(character);
                }
            }
            if !left_content.is_empty() {
                left.push(Span::styled(left_content, span.style));
            }
            if !right_content.is_empty() {
                right.push(Span::styled(right_content, span.style));
            }
            consumed = split_width;
        }
    }

    (left, right)
}

fn slice_spans_by_bytes<'a>(spans: &[Span<'a>], start: usize, end: usize) -> Vec<Span<'a>> {
    let mut result = Vec::new();
    let mut position = 0;

    for span in spans {
        let span_start = position;
        let span_end = position + span.content.len();

        if span_end <= start || span_start >= end {
            position = span_end;
            continue;
        }

        let slice_start = start.saturating_sub(span_start);
        let slice_end = (end - span_start).min(span.content.len());

        if slice_start < slice_end && slice_start < span.content.len() {
            let content = &span.content[slice_start..slice_end];
            if !content.is_empty() {
                result.push(Span::styled(content.to_string(), span.style));
            }
        }

        position = span_end;
    }

    result
}
