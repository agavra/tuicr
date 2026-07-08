use ratatui::{style::Style, text::Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub(super) fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        let truncate_at = max_len.saturating_sub(3);
        let end = s
            .char_indices()
            .map(|(i, _)| i)
            .take_while(|&i| i <= truncate_at)
            .last()
            .unwrap_or(0);
        format!("{}...", &s[..end])
    }
}

/// Truncate or pad a string to a specific width
pub(super) fn truncate_or_pad(s: &str, width: usize) -> String {
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
pub(super) fn truncate_or_pad_spans(
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

pub(super) fn wrap_spans<'a>(spans: &[Span<'a>], width: usize) -> Vec<Vec<Span<'a>>> {
    if width == 0 {
        return vec![spans.to_vec()];
    }
    if spans.is_empty() || spans.iter().all(|s| s.content.is_empty()) {
        return vec![vec![]];
    }
    let total: usize = spans.iter().map(|s| s.content.width()).sum();
    if total <= width {
        return vec![spans.to_vec()];
    }

    #[derive(Clone)]
    struct Item {
        ch: char,
        style: Style,
        w: usize,
    }

    fn flush_unit(
        unit: &mut Vec<Item>,
        unit_w: &mut usize,
        current: &mut Vec<Item>,
        current_w: &mut usize,
        rows: &mut Vec<Vec<Item>>,
        width: usize,
    ) {
        if unit.is_empty() {
            return;
        }
        if *current_w + *unit_w <= width {
            current.append(unit);
            *current_w += *unit_w;
            *unit_w = 0;
            return;
        }
        if *unit_w <= width {
            rows.push(std::mem::take(current));
            *current_w = *unit_w;
            current.append(unit);
            *unit_w = 0;
            return;
        }
        while !unit.is_empty() {
            let remaining = width.saturating_sub(*current_w);
            let mut consumed = 0usize;
            let mut consumed_w = 0usize;
            for it in unit.iter() {
                if consumed_w + it.w > remaining {
                    break;
                }
                consumed_w += it.w;
                consumed += 1;
            }
            if consumed == 0 {
                if current.is_empty() {
                    let it = unit.remove(0);
                    let w = it.w;
                    current.push(it);
                    *current_w = w;
                    rows.push(std::mem::take(current));
                    *current_w = 0;
                } else {
                    rows.push(std::mem::take(current));
                    *current_w = 0;
                }
            } else {
                let drained: Vec<Item> = unit.drain(..consumed).collect();
                current.extend(drained);
                *current_w += consumed_w;
                if !unit.is_empty() {
                    rows.push(std::mem::take(current));
                    *current_w = 0;
                }
            }
        }
        *unit_w = 0;
    }

    let mut rows: Vec<Vec<Item>> = Vec::new();
    let mut current: Vec<Item> = Vec::new();
    let mut current_w: usize = 0;
    let mut word: Vec<Item> = Vec::new();
    let mut word_w: usize = 0;

    for span in spans {
        for ch in span.content.chars() {
            let w = UnicodeWidthChar::width(ch).unwrap_or(if ch == '\t' { 1 } else { 0 });
            let item = Item {
                ch,
                style: span.style,
                w,
            };
            if ch == ' ' || ch == '\t' {
                flush_unit(
                    &mut word,
                    &mut word_w,
                    &mut current,
                    &mut current_w,
                    &mut rows,
                    width,
                );
                let mut ws = vec![item];
                let mut ws_w = w;
                flush_unit(
                    &mut ws,
                    &mut ws_w,
                    &mut current,
                    &mut current_w,
                    &mut rows,
                    width,
                );
            } else {
                word.push(item);
                word_w += w;
            }
        }
    }
    flush_unit(
        &mut word,
        &mut word_w,
        &mut current,
        &mut current_w,
        &mut rows,
        width,
    );
    if !current.is_empty() || rows.is_empty() {
        rows.push(current);
    }

    rows.into_iter()
        .map(|row| {
            let mut out: Vec<Span<'a>> = Vec::new();
            let mut buf = String::new();
            let mut cur_style: Option<Style> = None;
            for it in row {
                match cur_style {
                    Some(s) if s == it.style => buf.push(it.ch),
                    _ => {
                        if let Some(s) = cur_style {
                            out.push(Span::styled(std::mem::take(&mut buf), s));
                        }
                        cur_style = Some(it.style);
                        buf.push(it.ch);
                    }
                }
            }
            if let Some(s) = cur_style {
                out.push(Span::styled(buf, s));
            }
            out
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    fn flatten<'a>(row: &[Span<'a>]) -> Vec<(Style, String)> {
        row.iter()
            .map(|s| (s.style, s.content.to_string()))
            .collect()
    }

    #[test]
    fn should_return_string_unchanged_when_within_max_len() {
        // given
        let s = "hello";
        // when
        let result = truncate_str(s, 10);
        // then
        assert_eq!(result, "hello");
    }

    #[test]
    fn should_truncate_ascii_string_with_ellipsis() {
        // given
        let s = "hello world this is long";
        // when
        let result = truncate_str(s, 10);
        // then
        assert_eq!(result, "hello w...");
    }

    #[test]
    fn should_truncate_without_panicking_on_multibyte_chars() {
        // given - the exact string from the bug report
        let s = "Resolve \"SD : Envoi en validation manuelle après 3 rejet de la fiche employé\"";
        // when
        let result = truncate_str(s, 47);
        // then - should not panic and should end with "..."
        assert!(result.ends_with("..."));
        assert!(result.len() <= 47);
    }

    #[test]
    fn should_handle_string_of_only_multibyte_chars() {
        // given
        let s = "ééééééééé";
        // when
        let result = truncate_str(s, 5);
        // then
        assert!(result.ends_with("..."));
        assert!(result.is_char_boundary(result.len()));
    }

    #[test]
    fn should_pad_highlighted_spans_to_exact_width() {
        // given - highlighted spans from the syntax highlighter (which strips
        // the trailing \n that syntect includes). Short content gets padded
        // by truncate_or_pad_spans; the result must have exactly `width`
        // characters so the side-by-side separator stays aligned.
        let highlighter = crate::syntax::SyntaxHighlighter::default();
        let lines = vec!["let x = 1;".to_string()];
        let highlighted = highlighter
            .highlight_file_lines(std::path::Path::new("test.rs"), &lines)
            .unwrap();
        let spans = highlighted[0].as_ref().unwrap();

        let width = 80;

        // when
        let result = truncate_or_pad_spans(spans, width, Style::default());

        // then - total char count must equal the target width so each
        // side-by-side column is the same size
        let total_chars: usize = result.iter().map(|s| s.content.chars().count()).sum();
        assert_eq!(
            total_chars, width,
            "padded spans should have exactly {width} chars, got {total_chars}"
        );
    }

    #[test]
    fn should_return_single_row_when_input_fits() {
        let spans = vec![Span::raw("hello")];
        let rows = wrap_spans(&spans, 10);
        assert_eq!(rows.len(), 1);
        assert_eq!(flatten(&rows[0]), vec![(Style::default(), "hello".into())]);
    }

    #[test]
    fn should_return_single_empty_row_for_empty_input() {
        let spans: Vec<Span> = vec![];
        let rows = wrap_spans(&spans, 10);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].is_empty());
    }

    #[test]
    fn should_return_single_empty_row_for_all_empty_span_contents() {
        let red = Style::default().fg(Color::Red);
        let spans = vec![Span::styled("", red), Span::raw("")];
        let rows = wrap_spans(&spans, 10);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].is_empty());
    }

    #[test]
    fn should_return_input_unchanged_when_width_is_zero() {
        let spans = vec![Span::raw("hello world")];
        let rows = wrap_spans(&spans, 0);
        assert_eq!(rows.len(), 1);
        assert_eq!(
            flatten(&rows[0]),
            vec![(Style::default(), "hello world".into())]
        );
    }

    #[test]
    fn should_wrap_at_whitespace_word_boundary() {
        let spans = vec![Span::raw("hello world foo")];
        let rows = wrap_spans(&spans, 8);
        assert_eq!(rows.len(), 3);
        assert_eq!(flatten(&rows[0]), vec![(Style::default(), "hello ".into())]);
        assert_eq!(flatten(&rows[1]), vec![(Style::default(), "world ".into())]);
        assert_eq!(flatten(&rows[2]), vec![(Style::default(), "foo".into())]);
    }

    #[test]
    fn should_hard_split_a_word_longer_than_width() {
        let spans = vec![Span::raw("aaaaaaaaaa")];
        let rows = wrap_spans(&spans, 4);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0][0].content, "aaaa");
        assert_eq!(rows[1][0].content, "aaaa");
        assert_eq!(rows[2][0].content, "aa");
    }

    #[test]
    fn should_preserve_styles_across_wrap_boundary() {
        let red = Style::default().fg(Color::Red);
        let blue = Style::default().fg(Color::Blue);
        let spans = vec![Span::styled("hello ", red), Span::styled("world", blue)];
        let rows = wrap_spans(&spans, 6);
        assert_eq!(rows.len(), 2);
        assert_eq!(flatten(&rows[0]), vec![(red, "hello ".into())]);
        assert_eq!(flatten(&rows[1]), vec![(blue, "world".into())]);
    }

    #[test]
    fn should_split_a_span_that_crosses_a_wrap_point() {
        let red = Style::default().fg(Color::Red);
        let spans = vec![Span::styled("hello world", red)];
        let rows = wrap_spans(&spans, 6);
        assert_eq!(rows.len(), 2);
        assert_eq!(flatten(&rows[0]), vec![(red, "hello ".into())]);
        assert_eq!(flatten(&rows[1]), vec![(red, "world".into())]);
    }

    #[test]
    fn should_merge_same_style_spans_into_one_when_wrapping() {
        let red = Style::default().fg(Color::Red);
        let spans = vec![Span::styled("he", red), Span::styled("llo", red)];
        let rows = wrap_spans(&spans, 3);
        assert_eq!(rows.len(), 2);
        assert_eq!(flatten(&rows[0]), vec![(red, "hel".into())]);
        assert_eq!(flatten(&rows[1]), vec![(red, "lo".into())]);
    }

    #[test]
    fn should_wrap_cjk_chars_by_display_width() {
        let spans = vec![Span::raw("中文測試")];
        let rows = wrap_spans(&spans, 5);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0][0].content, "中文");
        assert_eq!(rows[1][0].content, "測試");
    }

    #[test]
    fn should_emit_oversized_char_alone_when_width_is_one_and_char_is_wide() {
        let spans = vec![Span::raw("中")];
        let rows = wrap_spans(&spans, 1);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][0].content, "中");
    }

    #[test]
    fn should_treat_tab_as_whitespace_break() {
        let spans = vec![Span::raw("hello\tworld")];
        let rows = wrap_spans(&spans, 6);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0][0].content, "hello\t");
        assert_eq!(rows[1][0].content, "world");
    }

    #[test]
    fn should_preserve_leading_whitespace_on_continuation_rows() {
        let spans = vec![Span::raw("foo   bar baz")];
        let rows = wrap_spans(&spans, 7);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0][0].content, "foo   ");
        assert_eq!(rows[1][0].content, "bar baz");
    }

    #[test]
    fn should_handle_multibyte_utf8_char_boundaries_safely() {
        let spans = vec![Span::raw("ab中文cd")];
        let rows = wrap_spans(&spans, 4);
        assert_eq!(rows.len(), 2);
        for row in &rows {
            let w: usize = row.iter().map(|s| s.content.width()).sum();
            assert!(w <= 4, "row width {w} exceeds 4");
        }
        assert_eq!(rows[0][0].content, "ab中");
        assert_eq!(rows[1][0].content, "文cd");
    }
}
