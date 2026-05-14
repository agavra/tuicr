use ratatui::{style::Style, text::Span};
use unicode_width::UnicodeWidthStr;

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
