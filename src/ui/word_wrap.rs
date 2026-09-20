//! Gutter-aligned wrap expansion for `wrap_style = "gutter"`.
//!
//! Pre-expands long diff lines into visual rows before the `Paragraph`, so
//! continuation rows keep the line-number gutter (a `↪` marker in the lineno
//! column plus the diff origin prefix) instead of flowing under it the way
//! ratatui's `Wrap` does. Rows break at word boundaries when possible
//! (matching flow wrap's feel); a single token longer than the content
//! column hard-cuts at the display-width limit so code identifiers and
//! URLs still wrap instead of overflowing.
//!
//! The expansion also returns rows-per-logical-line, which becomes the
//! geometry source for scroll math, hit-testing, and overlay painting —
//! renderer and geometry come from the same pass and cannot diverge.

use ratatui::text::{Line, Span};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::app::{AnnotatedLine, App};
use crate::theme::Theme;
use crate::ui::styles;

/// Whitespace the word-boundary backtrack may break at. Excludes the
/// non-breaking family (NBSP, narrow NBSP, figure space), whose entire
/// purpose is to forbid a break.
fn is_breakable_space(c: char) -> bool {
    c.is_whitespace() && !matches!(c, '\u{a0}' | '\u{202f}' | '\u{2007}')
}

/// How one logical line participates in gutter-mode expansion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WrapPlan {
    /// Never split. Rendered as one row; the no-`Wrap` `Paragraph` clips
    /// overflow. This is the safe default for decoration lines (file
    /// headers, spacing) — the bug class that sank the first wrap attempt
    /// was headers splitting mid-box.
    Exempt,
    /// The first `gutter_width` columns are gutter (indicator + lineno +
    /// origin prefix); content wraps into rows of
    /// `viewport_width - gutter_width`. Must be at least 4: continuation
    /// rows always rebuild indicator(1) + `↪` column(1+) + prefix(2).
    /// Production passes `unified_gutter(lineno_width()) >= 8`.
    Gutter { gutter_width: usize },
}

/// Expansion result. Invariants: `row_heights.len()` and
/// `row_char_ranges.len()` equal the logical input length, and
/// `rows.len() == row_heights.iter().sum()`.
pub(super) struct GutterWrapped<'a> {
    pub rows: Vec<Line<'a>>,
    pub row_heights: Vec<usize>,
    /// Per logical line: the content char range each visual row holds.
    /// Empty for unexpanded lines (uniform packing model applies).
    /// Word-boundary rows hold fewer chars than the content column, so
    /// selection geometry must consult these instead of assuming
    /// `which_row * content_width`.
    pub row_char_ranges: Vec<Vec<(usize, usize)>>,
}

/// Derive per-line wrap plans for the unified renderer's visible window.
/// Lines are looked up through `annotation_idx_for_rendered_line` (comment
/// input box aware); anything unmapped defaults to `Exempt`.
pub(super) fn unified_wrap_plans(
    app: &App,
    scroll_offset: usize,
    window_len: usize,
    gutter_width: usize,
) -> Vec<WrapPlan> {
    (0..window_len)
        .map(|i| {
            let annotation =
                super::diff_view::annotation_idx_for_rendered_line(app, scroll_offset + i)
                    .and_then(|idx| app.line_annotations.get(idx));
            match annotation {
                Some(AnnotatedLine::DiffLine { .. } | AnnotatedLine::ExpandedContext { .. }) => {
                    WrapPlan::Gutter { gutter_width }
                }
                _ => WrapPlan::Exempt,
            }
        })
        .collect()
}

/// Expand the visible logical-line window into gutter-aligned visual rows.
/// `max_rows` bounds the output (the viewport height); once reached, the
/// remaining logical lines pass through unexpanded with height 1 — they are
/// below the fold and never painted.
pub(super) fn expand_gutter_wrap<'a>(
    logical_lines: Vec<Line<'a>>,
    plans: &[WrapPlan],
    viewport_width: usize,
    max_rows: usize,
    theme: &Theme,
) -> GutterWrapped<'a> {
    let mut rows = Vec::with_capacity(logical_lines.len());
    let mut row_heights = Vec::with_capacity(logical_lines.len());
    let mut row_char_ranges = Vec::with_capacity(logical_lines.len());

    for (idx, line) in logical_lines.into_iter().enumerate() {
        let plan = plans.get(idx).copied().unwrap_or(WrapPlan::Exempt);
        let over_fold = rows.len() >= max_rows;

        let gutter_width = match plan {
            WrapPlan::Gutter { gutter_width } if !over_fold => {
                debug_assert!(
                    gutter_width >= 4,
                    "continuation gutter needs >= 4 cells (got {gutter_width})"
                );
                gutter_width
            }
            _ => {
                rows.push(line);
                row_heights.push(1);
                row_char_ranges.push(Vec::new());
                continue;
            }
        };

        let content_width = viewport_width.saturating_sub(gutter_width);
        let total_width: usize = line.spans.iter().map(|s| s.content.width()).sum();
        if content_width == 0 || total_width <= viewport_width {
            rows.push(line);
            row_heights.push(1);
            row_char_ranges.push(Vec::new());
            continue;
        }

        let (gutter_spans, content_spans) = split_spans_at_width(&line.spans, gutter_width);
        let content_text: String = content_spans.iter().map(|s| s.content.as_ref()).collect();
        if content_text.width() <= content_width {
            rows.push(line);
            row_heights.push(1);
            row_char_ranges.push(Vec::new());
            continue;
        }

        let mut char_cursor = 0;
        let mut line_ranges = Vec::new();
        let byte_ranges = gutter_row_byte_ranges(&content_text, content_width);
        for (line_rows, &(row_start_byte, row_end_byte)) in byte_ranges.iter().enumerate() {
            let row_spans = slice_spans_by_bytes(&content_spans, row_start_byte, row_end_byte);
            rows.push(if line_rows == 0 {
                let mut spans = gutter_spans.clone();
                spans.extend(row_spans);
                Line::from(spans)
            } else {
                continuation_row(&gutter_spans, gutter_width, row_spans, theme)
            });
            let row_chars = content_text[row_start_byte..row_end_byte].chars().count();
            line_ranges.push((char_cursor, char_cursor + row_chars));
            char_cursor += row_chars;
        }

        row_heights.push(byte_ranges.len().max(1));
        row_char_ranges.push(line_ranges);
    }

    GutterWrapped {
        rows,
        row_heights,
        row_char_ranges,
    }
}

/// Byte ranges of the visual rows `content_text` packs into at
/// `content_width`. This is the single packing routine behind gutter wrap:
/// the expansion slices spans along these ranges and the row-height helper
/// counts them, so navigation geometry and rendered rows cannot disagree.
pub(super) fn gutter_row_byte_ranges(
    content_text: &str,
    content_width: usize,
) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut byte_offset = 0;
    while byte_offset < content_text.len() {
        // Pack by grapheme clusters measured with str-width — the same
        // sequence-aware measure ratatui's renderer uses. Per-char
        // widths under-count emoji presentation sequences (U+FE0F makes
        // "❤️" 2 cells while its chars sum to 1), which produced rows
        // wider than the viewport whose tails the no-Wrap Paragraph
        // clipped invisibly. Grapheme packing also keeps ZWJ families
        // and flag pairs whole instead of tearing them at cut points.
        let mut row_end_byte = byte_offset;
        let mut row_width = 0;
        for grapheme in content_text[byte_offset..].graphemes(true) {
            let grapheme_width = grapheme.width();
            if row_width + grapheme_width > content_width {
                break;
            }
            row_width += grapheme_width;
            row_end_byte += grapheme.len();
        }
        // Word-boundary preference: when the cut lands mid-token,
        // backtrack to the last breakable whitespace in the row so the
        // token moves to the next row whole instead of orphaning a few
        // letters. Non-breaking spaces (NBSP, narrow NBSP, figure
        // space) are not break candidates — that is their whole point.
        // A token longer than the whole row has no whitespace to back
        // up to and hard-cuts at the column limit.
        if row_end_byte < content_text.len() {
            let cuts_mid_token = content_text[row_end_byte..]
                .chars()
                .next()
                .is_some_and(|c| !is_breakable_space(c))
                && content_text[byte_offset..row_end_byte]
                    .chars()
                    .next_back()
                    .is_some_and(|c| !is_breakable_space(c));
            if cuts_mid_token
                && let Some(ws_pos) =
                    content_text[byte_offset..row_end_byte].rfind(is_breakable_space)
            {
                let ws_start = byte_offset + ws_pos;
                let ws_len = content_text[ws_start..]
                    .chars()
                    .next()
                    .map_or(1, char::len_utf8);
                row_end_byte = ws_start + ws_len;
            }
        }
        // Forced progress: a single grapheme wider than the content
        // column still consumes one row instead of looping forever.
        if row_end_byte == byte_offset {
            if let Some(grapheme) = content_text[byte_offset..].graphemes(true).next() {
                row_end_byte += grapheme.len();
            } else {
                break;
            }
        }

        ranges.push((byte_offset, row_end_byte));
        byte_offset = row_end_byte;
    }
    ranges
}

/// Continuation rows reconstruct the gutter: blank indicator slot, a `↪`
/// right-aligned in the line-number column, and the origin prefix (`▌ `/
/// `  ` with its add/del/context style) copied from the logical line so
/// coloring stays consistent across visual rows.
fn continuation_row<'a>(
    gutter_spans: &[Span<'a>],
    gutter_width: usize,
    row_spans: Vec<Span<'a>>,
    theme: &Theme,
) -> Line<'a> {
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
}

/// Split styled spans at a display-width boundary, preserving styles on
/// both sides. A span straddling the boundary is split mid-span.
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
            let cells_needed = split_width - consumed;
            let mut left_content = String::new();
            let mut right_content = String::new();
            let mut taken_width = 0;
            for grapheme in span.content.graphemes(true) {
                let gw = grapheme.width();
                if taken_width + gw <= cells_needed && right_content.is_empty() {
                    left_content.push_str(grapheme);
                    taken_width += gw;
                } else {
                    right_content.push_str(grapheme);
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

/// Slice styled spans by byte range over their concatenated content,
/// preserving per-span styles. Empty slices are dropped.
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

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::{Color, Style};

    // indicator(1) + lineno(4)+space(1) + prefix(2) — matches unified_gutter(4).
    const GUTTER: usize = 8;

    fn diff_line_fixture(content: &str) -> Line<'static> {
        Line::from(vec![
            Span::raw(" "),                                        // indicator
            Span::raw("   1 "),                                    // lineno + space
            Span::styled("▌ ", Style::default().fg(Color::Green)), // origin prefix
            Span::styled(content.to_string(), Style::default().fg(Color::White)),
        ])
    }

    /// Concatenated content of a produced row (spans after the 3 gutter spans).
    fn row_content(row: &Line) -> String {
        row.spans[3..].iter().map(|s| s.content.as_ref()).collect()
    }

    fn rows_width(line: &Line) -> usize {
        line.spans.iter().map(|s| s.content.width()).sum()
    }

    #[test]
    fn should_hold_invariants_across_viewport_widths() {
        // Mixed content: ASCII, CJK, and a one-cell line. For every viewport
        // width, rows == sum(row_heights) and no produced row exceeds the
        // viewport.
        let contents = ["q".repeat(75), "漢字テスト".repeat(9), "x".to_string()];
        for viewport_width in 9..=60 {
            let lines: Vec<Line> = contents
                .iter()
                .map(|content| diff_line_fixture(content))
                .collect();
            let plans = vec![
                WrapPlan::Gutter {
                    gutter_width: GUTTER,
                };
                lines.len()
            ];
            let result = expand_gutter_wrap(lines, &plans, viewport_width, 200, &Theme::dark());
            assert_eq!(result.row_heights.len(), contents.len());
            assert_eq!(
                result.rows.len(),
                result.row_heights.iter().sum::<usize>(),
                "rows/heights diverged at viewport width {viewport_width}"
            );
            for row in &result.rows {
                assert!(
                    rows_width(row) <= viewport_width.max(GUTTER + 2),
                    "row exceeds viewport at width {viewport_width}: {} cells",
                    rows_width(row)
                );
            }
        }
    }

    #[test]
    fn should_pass_through_exempt_lines_unsplit() {
        let long_header = Line::from(format!("═══ {} [M] ═══", "a/very/long/path".repeat(8)));
        let original_width = rows_width(&long_header);
        let result = expand_gutter_wrap(
            vec![long_header],
            &[WrapPlan::Exempt],
            20,
            50,
            &Theme::dark(),
        );
        assert_eq!(result.row_heights, vec![1]);
        assert_eq!(result.rows.len(), 1);
        assert_eq!(rows_width(&result.rows[0]), original_width);
    }

    #[test]
    fn should_expand_long_diff_line_and_match_row_heights_sum() {
        let line = diff_line_fixture(&"x".repeat(40));
        let result = expand_gutter_wrap(
            vec![line],
            &[WrapPlan::Gutter {
                gutter_width: GUTTER,
            }],
            20, // content_width = 12 → 40 cells = 4 rows (12+12+12+4)
            50,
            &Theme::dark(),
        );
        assert_eq!(result.row_heights, vec![4]);
        assert_eq!(
            result.rows.len(),
            result.row_heights.iter().sum::<usize>(),
            "rows must equal the sum of row_heights"
        );
        // No produced row exceeds the viewport width.
        for row in &result.rows {
            assert!(rows_width(row) <= 20, "row wider than viewport");
        }
    }

    #[test]
    fn should_place_wrap_marker_and_origin_prefix_on_continuation_rows() {
        let line = diff_line_fixture(&"y".repeat(30));
        let result = expand_gutter_wrap(
            vec![line],
            &[WrapPlan::Gutter {
                gutter_width: GUTTER,
            }],
            20,
            50,
            &Theme::dark(),
        );
        assert!(result.rows.len() >= 2);
        let continuation = &result.rows[1];
        // span 1 is the lineno column ending in the wrap marker, width lineno+1
        let marker_span = &continuation.spans[1];
        assert!(
            marker_span.content.ends_with('↪'),
            "expected ↪ at end of lineno column, got {:?}",
            marker_span.content
        );
        assert_eq!(marker_span.content.width(), GUTTER - 4 + 1);
        // span 2 carries the origin prefix with its original style
        let prefix_span = &continuation.spans[2];
        assert_eq!(prefix_span.content.as_ref(), "▌ ");
        assert_eq!(prefix_span.style.fg, Some(Color::Green));
    }

    #[test]
    fn should_keep_vs16_emoji_rows_within_viewport() {
        // Regression (fuzz campaign): "❤️" is U+2764 U+FE0F — per-char widths
        // sum to 1 but the rendered grapheme is 2 cells. Char-based packing
        // produced rows wider than the viewport whose tails the no-Wrap
        // Paragraph clipped invisibly.
        let line = diff_line_fixture(
            &"fix: works \u{2714}\u{fe0f} now and \u{2764}\u{fe0f} forever ok".repeat(2),
        );
        let result = expand_gutter_wrap(
            vec![line],
            &[WrapPlan::Gutter {
                gutter_width: GUTTER,
            }],
            24,
            50,
            &Theme::dark(),
        );
        for row in &result.rows {
            assert!(
                rows_width(row) <= 24,
                "VS16 row overflows viewport: {} cells",
                rows_width(row)
            );
        }
        // Lossless: every input character reappears across the rows.
        let rejoined: String = result.rows.iter().map(row_content).collect();
        assert_eq!(
            rejoined,
            "fix: works \u{2714}\u{fe0f} now and \u{2764}\u{fe0f} forever ok".repeat(2)
        );
    }

    #[test]
    fn should_not_tear_zwj_emoji_families_at_cut_points() {
        // A ZWJ family is one grapheme; tearing it at a row boundary mutates
        // which emoji the user sees. Grapheme packing moves it whole.
        let family = "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}\u{200d}\u{1f466}";
        let content = format!("{}{family}{}", "x".repeat(11), "y".repeat(8));
        let line = diff_line_fixture(&content);
        let result = expand_gutter_wrap(
            vec![line],
            &[WrapPlan::Gutter {
                gutter_width: GUTTER,
            }],
            20, // content_width 12: family (2 cells) would straddle the cut
            50,
            &Theme::dark(),
        );
        for row in &result.rows {
            let text = row_content(row);
            let has_zwj = text.contains('\u{200d}');
            if has_zwj {
                assert!(
                    text.contains(family),
                    "ZWJ family torn across rows: {text:?}"
                );
            }
        }
    }

    #[test]
    fn should_not_break_at_non_breaking_spaces() {
        // NBSP exists to forbid a break; the backtrack must skip it and
        // hard-cut instead of "honoring" it as a boundary.
        let content = format!("alpha\u{a0}beta\u{a0}gamma {}", "d".repeat(12));
        let line = diff_line_fixture(&content);
        let result = expand_gutter_wrap(
            vec![line],
            &[WrapPlan::Gutter {
                gutter_width: GUTTER,
            }],
            20, // content_width 12 cuts inside the NBSP-glued token
            50,
            &Theme::dark(),
        );
        // First row must hard-cut the NBSP-glued token (12 cells), not
        // backtrack to an NBSP.
        assert_eq!(row_content(&result.rows[0]).width(), 12);
        assert!(
            !row_content(&result.rows[0]).ends_with('\u{a0}'),
            "row breaks at a non-breaking space"
        );
    }

    #[test]
    fn should_break_at_word_boundaries() {
        let line = diff_line_fixture("alpha beta gamma delta epsilon");
        let result = expand_gutter_wrap(
            vec![line],
            &[WrapPlan::Gutter {
                gutter_width: GUTTER,
            }],
            20, // content_width = 12
            50,
            &Theme::dark(),
        );
        assert_eq!(result.row_heights, vec![3]);
        assert_eq!(row_content(&result.rows[0]), "alpha beta ");
        assert_eq!(row_content(&result.rows[1]), "gamma delta ");
        assert_eq!(row_content(&result.rows[2]), "epsilon");
    }

    #[test]
    fn should_not_orphan_word_tail_on_next_row() {
        // Regression: with hard cuts, "efghi" lost its tail letter to the
        // next row ("abcd efgh" / "i"). Word-boundary backtrack moves the
        // whole token instead.
        let line = diff_line_fixture("abcd efghi");
        let result = expand_gutter_wrap(
            vec![line],
            &[WrapPlan::Gutter {
                gutter_width: GUTTER,
            }],
            GUTTER + 9, // content_width = 9: one cell short of the full text
            50,
            &Theme::dark(),
        );
        assert_eq!(result.row_heights, vec![2]);
        assert_eq!(row_content(&result.rows[0]), "abcd ");
        assert_eq!(row_content(&result.rows[1]), "efghi");
    }

    #[test]
    fn should_hard_cut_token_longer_than_content_column() {
        let line = diff_line_fixture("https://example.com/very/long/path/segment");
        let result = expand_gutter_wrap(
            vec![line],
            &[WrapPlan::Gutter {
                gutter_width: GUTTER,
            }],
            20, // content_width = 12, no whitespace anywhere
            50,
            &Theme::dark(),
        );
        assert!(result.row_heights[0] >= 3);
        assert_eq!(row_content(&result.rows[0]).len(), 12);
        let rejoined: String = result.rows.iter().map(|row| row_content(row)).collect();
        assert_eq!(rejoined, "https://example.com/very/long/path/segment");
    }

    #[test]
    fn should_keep_position_on_short_diff_line() {
        let line = diff_line_fixture("short");
        let result = expand_gutter_wrap(
            vec![line],
            &[WrapPlan::Gutter {
                gutter_width: GUTTER,
            }],
            80,
            50,
            &Theme::dark(),
        );
        assert_eq!(result.row_heights, vec![1]);
        assert_eq!(result.rows.len(), 1);
    }

    #[test]
    fn should_not_split_wide_chars_across_rows() {
        // CJK chars are 2 cells; content_width 12 fits 6 per row.
        let line = diff_line_fixture(&"漢".repeat(10));
        let result = expand_gutter_wrap(
            vec![line],
            &[WrapPlan::Gutter {
                gutter_width: GUTTER,
            }],
            20,
            50,
            &Theme::dark(),
        );
        assert_eq!(result.row_heights, vec![2]); // 6 + 4 chars
        for row in &result.rows {
            assert!(rows_width(row) <= 20);
            // every row's content is valid UTF-8 by construction; widths even
            let content: String = row.spans[3..].iter().map(|s| s.content.as_ref()).collect();
            assert_eq!(content.width() % 2, 0, "wide char split across rows");
        }
    }

    #[test]
    fn should_make_progress_when_char_wider_than_content_width() {
        // gutter 8 + viewport 9 → content_width 1; CJK char needs 2 cells.
        let line = diff_line_fixture("漢字");
        let result = expand_gutter_wrap(
            vec![line],
            &[WrapPlan::Gutter { gutter_width: 8 }],
            9,
            50,
            &Theme::dark(),
        );
        // Forced progress: one char per row, no infinite loop, no panic.
        assert_eq!(result.row_heights, vec![2]);
    }

    #[test]
    fn should_pass_through_when_content_width_is_zero() {
        let line = diff_line_fixture(&"z".repeat(30));
        let result = expand_gutter_wrap(
            vec![line],
            &[WrapPlan::Gutter { gutter_width: 30 }],
            20, // gutter wider than viewport → content_width 0
            50,
            &Theme::dark(),
        );
        assert_eq!(result.row_heights, vec![1]);
        assert_eq!(result.rows.len(), 1);
    }

    #[test]
    fn should_stop_expanding_below_the_fold() {
        let lines: Vec<Line> = (0..5).map(|_| diff_line_fixture(&"w".repeat(40))).collect();
        let plans = vec![
            WrapPlan::Gutter {
                gutter_width: GUTTER,
            };
            5
        ];
        // max_rows 4: the first line alone produces 4 rows; the rest pass through.
        let result = expand_gutter_wrap(lines, &plans, 20, 4, &Theme::dark());
        assert_eq!(result.row_heights, vec![4, 1, 1, 1, 1]);
        assert_eq!(result.rows.len(), 8);
    }

    #[test]
    fn should_split_spans_at_width_preserving_styles() {
        let spans = vec![
            Span::styled("abc", Style::default().fg(Color::Red)),
            Span::styled("defg", Style::default().fg(Color::Blue)),
        ];
        let (left, right) = split_spans_at_width(&spans, 5);
        let left_text: String = left.iter().map(|s| s.content.as_ref()).collect();
        let right_text: String = right.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(left_text, "abcde");
        assert_eq!(right_text, "fg");
        assert_eq!(left[0].style.fg, Some(Color::Red));
        assert_eq!(left[1].style.fg, Some(Color::Blue));
        assert_eq!(right[0].style.fg, Some(Color::Blue));
    }

    #[test]
    fn should_slice_spans_by_bytes_preserving_styles() {
        let spans = vec![
            Span::styled("hello", Style::default().fg(Color::Red)),
            Span::styled("world", Style::default().fg(Color::Blue)),
        ];
        let sliced = slice_spans_by_bytes(&spans, 3, 8);
        let text: String = sliced.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "lowor");
        assert_eq!(sliced[0].style.fg, Some(Color::Red));
        assert_eq!(sliced[1].style.fg, Some(Color::Blue));
    }
}
