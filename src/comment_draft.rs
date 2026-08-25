//! Composing a review comment in the user's `$EDITOR`.
//!
//! The comment box hands its draft to the editor as a temporary Markdown file
//! laid out like a `git commit` buffer: the comment body sits at the top, and
//! the diff being commented on follows below a scissors line as `#`-prefixed
//! context. Only the text above the scissors comes back.
//!
//! Everything here is pure text: the terminal handoff lives in `main`, and the
//! draft target is resolved by `App`.

use crate::model::{DiffHunk, DiffLine, LineOrigin, LineRange, LineSide};

/// Separates the comment body from the read-only context below it.
///
/// Matches `git commit --cleanup=scissors`, which the same editors and
/// filetype plugins already recognize.
pub const SCISSORS: &str = "# ------------------------ >8 ------------------------";

/// Diff rows kept either side of the commented lines.
///
/// A whole hunk can run to hundreds of lines, which would bury the comment
/// body; a window keeps the handoff readable while still showing what the
/// commented lines sit between.
const CONTEXT_RADIUS: usize = 20;

/// Column width for the line numbers in the context block.
const LINENO_WIDTH: usize = 5;

/// The diff a draft is attached to, rendered below the scissors line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DraftContext {
    /// Human-readable target, such as `src/main.rs:142-145 (new)`.
    pub target: String,
    /// Formatted diff rows for the commented lines and their neighbours.
    pub lines: Vec<String>,
}

impl DraftContext {
    /// Context for a comment with no diff rows to show, such as a file-level
    /// or review-level comment.
    pub fn targeting(target: impl Into<String>) -> Self {
        Self {
            target: target.into(),
            lines: Vec::new(),
        }
    }
}

/// Builds the editor buffer for `draft`.
///
/// The body is reproduced verbatim so a round-trip through the editor cannot
/// reword what the user already typed.
pub fn render_draft(draft: &str, context: &DraftContext) -> String {
    let mut out = String::new();
    if !draft.is_empty() {
        out.push_str(draft);
        if !draft.ends_with('\n') {
            out.push('\n');
        }
    }
    // A blank line above the scissors gives the cursor somewhere to land, and
    // keeps the body from butting against the notes.
    out.push('\n');
    out.push_str(SCISSORS);
    out.push('\n');
    push_note(&mut out, "Write the comment above the scissors line.");
    push_note(
        &mut out,
        "Everything below it is discarded, and an empty comment leaves the draft as it was.",
    );
    if !context.target.is_empty() {
        push_note(&mut out, &format!("Commenting on {}", context.target));
    }
    if !context.lines.is_empty() {
        push_note(&mut out, "");
        for line in &context.lines {
            push_note(&mut out, line);
        }
    }
    out
}

/// Extracts the comment body from an edited buffer.
///
/// Only the scissors line terminates the body: a `#` heading is ordinary
/// Markdown in a review comment, so `#`-prefixed lines above the scissors are
/// kept as typed.
pub fn parse_draft(text: &str) -> String {
    text.lines()
        .take_while(|line| line.trim_end() != SCISSORS)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

/// One-based line the editor should open on: the blank line under the body.
pub fn cursor_line(buffer: &str) -> Option<u32> {
    let scissors = buffer
        .lines()
        .position(|line| line.trim_end() == SCISSORS)?;
    u32::try_from(scissors.max(1)).ok()
}

/// Formats the diff rows around `range` in `hunk` for the context block.
///
/// Rows outside the window are replaced by a count, so a clipped hunk never
/// looks like the whole one.
pub fn context_lines(hunk: &DiffHunk, range: LineRange, side: LineSide) -> Vec<String> {
    let targeted: Vec<usize> = hunk
        .lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line_in_range(line, range, side))
        .map(|(idx, _)| idx)
        .collect();
    // Without a matching row there is nothing to centre the window on; the
    // target header still names the lines.
    let (Some(&first), Some(&last)) = (targeted.first(), targeted.last()) else {
        return Vec::new();
    };

    let start = first.saturating_sub(CONTEXT_RADIUS);
    let end = (last + CONTEXT_RADIUS + 1).min(hunk.lines.len());
    let mut rows = Vec::new();
    if start > 0 {
        rows.push(elision(start));
    }
    rows.extend(hunk.lines[start..end].iter().map(format_row));
    let trailing = hunk.lines.len() - end;
    if trailing > 0 {
        rows.push(elision(trailing));
    }
    rows
}

/// Whether `line` is one of the lines the comment targets.
fn line_in_range(line: &DiffLine, range: LineRange, side: LineSide) -> bool {
    let lineno = match side {
        LineSide::Old => line.old_lineno,
        LineSide::New => line.new_lineno,
    };
    lineno.is_some_and(|lineno| lineno >= range.start && lineno <= range.end)
}

/// Renders one diff row as `{origin}{lineno} {content}`.
fn format_row(line: &DiffLine) -> String {
    let origin = match line.origin {
        LineOrigin::Context => ' ',
        LineOrigin::Addition => '+',
        LineOrigin::Deletion => '-',
    };
    let lineno = match line.origin {
        LineOrigin::Deletion => line.old_lineno,
        _ => line.new_lineno.or(line.old_lineno),
    };
    let lineno = lineno.map_or_else(
        || " ".repeat(LINENO_WIDTH),
        |lineno| format!("{lineno:>LINENO_WIDTH$}"),
    );
    let content = line.content.trim_end_matches(['\n', '\r']);
    format!("{origin}{lineno} {content}")
}

fn elision(count: usize) -> String {
    let plural = if count == 1 { "" } else { "s" };
    format!("\u{2026} {count} more line{plural} in this hunk")
}

fn push_note(out: &mut String, text: &str) {
    if text.is_empty() {
        out.push_str("#\n");
    } else {
        out.push_str("# ");
        out.push_str(text);
        out.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(origin: LineOrigin, old: Option<u32>, new: Option<u32>, content: &str) -> DiffLine {
        DiffLine {
            origin,
            content: content.to_string(),
            old_lineno: old,
            new_lineno: new,
            highlighted_spans: None,
        }
    }

    fn hunk(lines: Vec<DiffLine>) -> DiffHunk {
        DiffHunk {
            header: "@@ -1,1 +1,1 @@".to_string(),
            lines,
            old_start: 1,
            old_count: 1,
            new_start: 1,
            new_count: 1,
        }
    }

    /// A hunk of `count` context lines numbered from 1 on both sides.
    fn context_hunk(count: u32) -> DiffHunk {
        hunk(
            (1..=count)
                .map(|n| line(LineOrigin::Context, Some(n), Some(n), &format!("line {n}")))
                .collect(),
        )
    }

    #[test]
    fn the_draft_body_is_reproduced_above_the_scissors() {
        let buffer = render_draft(
            "first\nsecond",
            &DraftContext::targeting("src/main.rs:1 (new)"),
        );
        assert!(buffer.starts_with("first\nsecond\n\n"), "{buffer}");
        assert_eq!(parse_draft(&buffer), "first\nsecond");
    }

    #[test]
    fn an_empty_draft_opens_on_an_empty_body() {
        let buffer = render_draft("", &DraftContext::targeting("src/main.rs (whole file)"));
        assert!(buffer.starts_with('\n'), "{buffer}");
        assert_eq!(parse_draft(&buffer), "");
    }

    #[test]
    fn the_target_and_context_are_commented_out() {
        let context = DraftContext {
            target: "src/main.rs:2 (new)".to_string(),
            lines: context_lines(&context_hunk(3), LineRange::single(2), LineSide::New),
        };
        let buffer = render_draft("look here", &context);
        let below: Vec<&str> = buffer
            .lines()
            .skip_while(|line| *line != SCISSORS)
            .collect();
        assert!(
            below.iter().all(|line| line.starts_with('#')),
            "context must stay commented: {below:?}"
        );
        assert!(
            below.contains(&"# Commenting on src/main.rs:2 (new)"),
            "{below:?}"
        );
        assert!(below.contains(&"#      2 line 2"), "{below:?}");
        assert_eq!(parse_draft(&buffer), "look here");
    }

    #[test]
    fn markdown_headings_survive_the_round_trip() {
        let body = "# Heading\n\nnot a template note";
        let buffer = render_draft(body, &DraftContext::targeting("src/main.rs:1 (new)"));
        assert_eq!(parse_draft(&buffer), body);
    }

    #[test]
    fn a_deleted_scissors_line_keeps_the_whole_buffer() {
        let text = "body\n\n# a note that is really part of the comment";
        assert_eq!(parse_draft(text), text);
    }

    #[test]
    fn surrounding_blank_lines_are_trimmed() {
        assert_eq!(parse_draft("\n\n  body  \n\n"), "body");
    }

    #[test]
    fn the_cursor_lands_on_the_blank_line_under_the_body() {
        let context = DraftContext::targeting("src/main.rs:1 (new)");
        assert_eq!(cursor_line(&render_draft("", &context)), Some(1));
        assert_eq!(cursor_line(&render_draft("one line", &context)), Some(2));
        assert_eq!(cursor_line(&render_draft("two\nlines", &context)), Some(3));
        assert_eq!(cursor_line("no scissors here"), None);
    }

    #[test]
    fn context_rows_carry_the_origin_and_line_number() {
        let hunk = hunk(vec![
            line(
                LineOrigin::Context,
                Some(10),
                Some(10),
                "    let cfg = load();",
            ),
            line(LineOrigin::Deletion, Some(11), None, "    let old = 1;"),
            line(LineOrigin::Addition, None, Some(11), "    let new = 2;"),
        ]);
        assert_eq!(
            context_lines(&hunk, LineRange::single(11), LineSide::New),
            vec![
                "    10     let cfg = load();".to_string(),
                "-   11     let old = 1;".to_string(),
                "+   11     let new = 2;".to_string(),
            ]
        );
    }

    #[test]
    fn a_long_hunk_is_windowed_around_the_commented_lines() {
        let rows = context_lines(&context_hunk(200), LineRange::new(100, 101), LineSide::New);
        assert_eq!(
            rows.first().map(String::as_str),
            Some("… 79 more lines in this hunk")
        );
        assert_eq!(
            rows.last().map(String::as_str),
            Some("… 79 more lines in this hunk")
        );
        assert!(rows.contains(&"   100 line 100".to_string()), "{rows:?}");
        assert!(rows.contains(&"   101 line 101".to_string()), "{rows:?}");
        // 20 rows either side of the two commented lines, plus both markers.
        assert_eq!(rows.len(), CONTEXT_RADIUS * 2 + 2 + 2);
    }

    #[test]
    fn a_short_hunk_is_shown_whole_without_markers() {
        let rows = context_lines(&context_hunk(3), LineRange::single(2), LineSide::New);
        assert_eq!(rows.len(), 3);
        assert!(
            !rows.iter().any(|row| row.contains("more line")),
            "{rows:?}"
        );
    }

    #[test]
    fn old_side_targets_match_deleted_lines() {
        let hunk = hunk(vec![
            line(LineOrigin::Deletion, Some(7), None, "gone"),
            line(LineOrigin::Addition, None, Some(7), "new"),
        ]);
        let rows = context_lines(&hunk, LineRange::single(7), LineSide::Old);
        assert_eq!(rows.len(), 2);
        assert!(rows[0].starts_with("-    7 gone"), "{rows:?}");
    }

    #[test]
    fn a_target_outside_the_hunk_renders_no_context() {
        let rows = context_lines(&context_hunk(3), LineRange::single(99), LineSide::New);
        assert!(rows.is_empty(), "{rows:?}");
    }
}
