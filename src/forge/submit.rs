//! Cross-forge submission logic for tuicr-authored reviews.
//!
//! This module converts a set of local-draft `Comment`s plus the parsed PR
//! diff into a forge-agnostic `MappedComment` stream that downstream code
//! (currently the GitHub payload builder) consumes. The mapping rules and
//! body/footer formatting live here so future forge backends inherit them.
//!
//! PR 5 wires the local preflight, resolver, and final-confirmation modal
//! against these types. The actual `gh api` call is deferred to PR 6.

use std::path::PathBuf;

use crate::config::ForgeConfig;
use crate::model::comment::Comment;
use crate::model::{DiffFile, LineRange, LineSide};

/// Which forge review event a `:submit*` command corresponds to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmitEvent {
    /// `:submit` / `:submit comment` — publish a `COMMENT` review.
    Comment,
    /// `:submit approve` — publish an `APPROVE` review.
    Approve,
    /// `:submit request-changes` — publish a `REQUEST_CHANGES` review.
    RequestChanges,
    /// `:submit draft` — create a pending GitHub review (no `event` field).
    Draft,
}

impl SubmitEvent {
    /// GitHub `event` field value, if any. `Draft` returns `None` because the
    /// pending-review behavior is triggered by omitting `event`.
    pub fn github_event(self) -> Option<&'static str> {
        match self {
            SubmitEvent::Comment => Some("COMMENT"),
            SubmitEvent::Approve => Some("APPROVE"),
            SubmitEvent::RequestChanges => Some("REQUEST_CHANGES"),
            SubmitEvent::Draft => None,
        }
    }

    /// Short human-readable label for the confirmation modal.
    pub fn human_label(self) -> &'static str {
        match self {
            SubmitEvent::Comment => "Comment",
            SubmitEvent::Approve => "Approve",
            SubmitEvent::RequestChanges => "Request changes",
            SubmitEvent::Draft => "Draft (pending review)",
        }
    }
}

/// GitHub's per-comment `side` field. Maps 1:1 to `LineSide`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GhSide {
    Left,
    Right,
}

impl GhSide {
    pub fn as_str(self) -> &'static str {
        match self {
            GhSide::Left => "LEFT",
            GhSide::Right => "RIGHT",
        }
    }
}

impl From<LineSide> for GhSide {
    fn from(value: LineSide) -> Self {
        match value {
            LineSide::Old => GhSide::Left,
            LineSide::New => GhSide::Right,
        }
    }
}

/// A single inline review comment ready to be serialized into GitHub's
/// `comments` array. Bodies already include the `[TYPE]` prefix when the
/// active `ForgeConfig` enables it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineComment {
    pub path: PathBuf,
    pub line: u32,
    pub side: GhSide,
    /// Multi-line range start. `None` for single-line comments.
    pub start_line: Option<u32>,
    pub start_side: Option<GhSide>,
    pub body: String,
    /// Source `Comment.id` this inline was derived from. PR 6 uses this on
    /// success to locate the source `Comment` and flip its `lifecycle_state`.
    /// This field is INTERNAL — the GitHub payload builder does not include
    /// it in the JSON request body.
    pub comment_id: String,
}

/// Why the mapper could not produce an inline comment for a given local
/// `Comment`. Used by the resolver UI to explain the choice to the user and
/// to seed the "Unplaced comments" summary section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnmappableReason {
    /// Range spans both Old and Deletion sides — GitHub multi-line comments
    /// must stay on a single side.
    MixedSideRange,
    /// File-level comment, but the file has no first-valid line anchor on
    /// the New side (binary, too-large, or pure deletion with no Old line).
    FileLevelNoAnchor,
    /// The file is binary; no anchor can be derived.
    BinaryFile,
    /// The file exceeded the renderer's too-large threshold and was not
    /// diffed.
    TooLargeFile,
    /// The cursor line was outside any hunk's coverage. Should not happen
    /// for line comments authored through tuicr today, but we keep the
    /// variant so the resolver can surface a clear message if it ever does.
    LineNotInDiff,
}

impl UnmappableReason {
    pub fn human_label(&self) -> &'static str {
        match self {
            UnmappableReason::MixedSideRange => "range spans both diff sides",
            UnmappableReason::FileLevelNoAnchor => "no valid anchor line",
            UnmappableReason::BinaryFile => "binary file",
            UnmappableReason::TooLargeFile => "file too large",
            UnmappableReason::LineNotInDiff => "line not in current diff",
        }
    }
}

/// Outcome of mapping one local `Comment` against the displayed diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MappedComment {
    Inline(InlineComment),
    Unmappable {
        comment: Comment,
        file: PathBuf,
        reason: UnmappableReason,
    },
}

/// Compute the inline body for `comment` honoring the `[TYPE]` prefix toggle.
/// File-level bodies are prefixed `[TYPE] File-level:`.
fn build_inline_body(comment: &Comment, file_level: bool, config: &ForgeConfig) -> String {
    if !config.comment_type_prefix {
        return comment.content.clone();
    }
    let prefix = if file_level {
        format!("[{ty}] File-level: ", ty = comment.comment_type.as_str())
    } else {
        format!("[{ty}] ", ty = comment.comment_type.as_str())
    };
    format!("{prefix}{body}", body = comment.content)
}

/// Where a local comment is anchored. The caller knows this from how it
/// walked the session (`file_comments` vs `line_comments[key]`); supplying
/// it explicitly avoids inferring file-level-ness from missing fields on
/// `Comment` (which is wrong: `line_comments` entries don't carry their
/// line on the `Comment` — the HashMap key holds it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentAnchor {
    /// File-level comment — no line anchor. Falls back to first valid line.
    FileLevel,
    /// Single-line comment anchored at `line` on `side`.
    Line { line: u32, side: LineSide },
    /// Multi-line range. Range info comes from `comment.line_range`.
    Range,
}

/// Map a single local `Comment` to either an inline GitHub comment or an
/// `Unmappable` outcome. `file` must be the diff file that produced this
/// comment (lookup is the caller's responsibility — it owns the current
/// `diff_files` / `range_diff_files` slice).
pub fn map_comment(
    comment: &Comment,
    anchor: CommentAnchor,
    file: &DiffFile,
    config: &ForgeConfig,
) -> MappedComment {
    let path = file.display_path().clone();

    if file.is_binary {
        return MappedComment::Unmappable {
            comment: comment.clone(),
            file: path,
            reason: UnmappableReason::BinaryFile,
        };
    }
    if file.is_too_large {
        return MappedComment::Unmappable {
            comment: comment.clone(),
            file: path,
            reason: UnmappableReason::TooLargeFile,
        };
    }

    match anchor {
        CommentAnchor::FileLevel => match file.first_valid_line(LineSide::New) {
            Some(line) => MappedComment::Inline(InlineComment {
                path,
                line,
                side: GhSide::Right,
                start_line: None,
                start_side: None,
                body: build_inline_body(comment, true, config),
                comment_id: comment.id.clone(),
            }),
            None => MappedComment::Unmappable {
                comment: comment.clone(),
                file: path,
                reason: UnmappableReason::FileLevelNoAnchor,
            },
        },
        CommentAnchor::Range => match comment.line_range {
            Some(range) => map_range(comment, file, config, range),
            None => MappedComment::Unmappable {
                comment: comment.clone(),
                file: path,
                reason: UnmappableReason::MixedSideRange,
            },
        },
        CommentAnchor::Line { line, side } => {
            if !line_present_on_side(file, line, side) {
                return MappedComment::Unmappable {
                    comment: comment.clone(),
                    file: path,
                    reason: UnmappableReason::LineNotInDiff,
                };
            }
            MappedComment::Inline(InlineComment {
                path,
                line,
                side: side.into(),
                start_line: None,
                start_side: None,
                body: build_inline_body(comment, false, config),
                comment_id: comment.id.clone(),
            })
        }
    }
}

/// True when `line` appears on `side` somewhere in the file's hunks.
fn line_present_on_side(file: &DiffFile, line: u32, side: LineSide) -> bool {
    for hunk in &file.hunks {
        for dl in &hunk.lines {
            let candidate = match side {
                LineSide::Old => dl.old_lineno,
                LineSide::New => dl.new_lineno,
            };
            if candidate == Some(line) {
                return true;
            }
        }
    }
    false
}

/// Map a multi-line range comment, validating that the range sits on a
/// single diff side.
fn map_range(
    comment: &Comment,
    file: &DiffFile,
    config: &ForgeConfig,
    range: LineRange,
) -> MappedComment {
    let path = file.display_path().clone();
    let side = match comment.side {
        Some(s) => s,
        // No explicit side is treated as ambiguous for a range; surface it
        // through the resolver rather than guessing.
        None => {
            return MappedComment::Unmappable {
                comment: comment.clone(),
                file: path,
                reason: UnmappableReason::MixedSideRange,
            };
        }
    };

    // Verify both ends of the range exist on `side`. The hunks may not
    // contain every intermediate line (the user could have selected across
    // a gap), but the start and end must be anchorable.
    if !range_endpoints_present(file, range, side) {
        return MappedComment::Unmappable {
            comment: comment.clone(),
            file: path,
            reason: UnmappableReason::MixedSideRange,
        };
    }

    if range.is_single() {
        return MappedComment::Inline(InlineComment {
            path,
            line: range.start,
            side: side.into(),
            start_line: None,
            start_side: None,
            body: build_inline_body(comment, false, config),
            comment_id: comment.id.clone(),
        });
    }

    MappedComment::Inline(InlineComment {
        path,
        line: range.end,
        side: side.into(),
        start_line: Some(range.start),
        start_side: Some(side.into()),
        body: build_inline_body(comment, false, config),
        comment_id: comment.id.clone(),
    })
}

/// True when both the start and end of `range` appear on the requested side
/// somewhere in `file`'s hunks. Used to detect ranges that would straddle a
/// side boundary (e.g. user selected through deleted+added lines).
fn range_endpoints_present(file: &DiffFile, range: LineRange, side: LineSide) -> bool {
    let mut saw_start = false;
    let mut saw_end = false;
    for hunk in &file.hunks {
        for line in &hunk.lines {
            let lineno = match side {
                LineSide::New => match line.origin {
                    crate::model::LineOrigin::Context | crate::model::LineOrigin::Addition => {
                        line.new_lineno
                    }
                    crate::model::LineOrigin::Deletion => None,
                },
                LineSide::Old => match line.origin {
                    crate::model::LineOrigin::Deletion => line.old_lineno,
                    _ => None,
                },
            };
            if let Some(n) = lineno {
                if n == range.start {
                    saw_start = true;
                }
                if n == range.end {
                    saw_end = true;
                }
            }
        }
    }
    saw_start && saw_end
}

/// Output of preflight — drives the resolver and confirmation modal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreflightResult {
    pub event: SubmitEvent,
    pub mappable: Vec<InlineComment>,
    pub unmappable: Vec<UnmappableItem>,
    /// Originally-reviewed PR head SHA for the comments — `commit_id` in the
    /// GitHub payload. The caller captures this from `pr_session_key.head_sha`
    /// at preflight time so a subsequent reload does not steal the anchor.
    pub commit_id: String,
}

/// A bundled view of an unmappable comment together with the file path and
/// reason, for the resolver UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnmappableItem {
    pub comment: Comment,
    pub file: PathBuf,
    pub reason: UnmappableReason,
}

/// What the resolver decided to do with an unmappable comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ResolverAction {
    /// Render the comment into the review body under "Unplaced comments".
    /// Spec: default action for unmappable comments.
    #[default]
    MoveToSummary,
    /// Drop the comment from this submit entirely.
    Omit,
}

/// A single line in the "Unplaced comments" section of the review body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovedToSummaryItem {
    pub comment: Comment,
    pub file: PathBuf,
}

/// Builds the GitHub review body. Returns an empty string when there's
/// nothing to say (no summary items, no footer, no review-level comments).
///
/// `review_level` are tuicr's review-level comments (`session.review_comments`)
/// already-formatted for the body. They appear above the unplaced section.
pub fn build_review_body(
    review_level: &[Comment],
    moved_to_summary: &[MovedToSummaryItem],
    config: &ForgeConfig,
) -> String {
    let mut sections: Vec<String> = Vec::new();

    if !review_level.is_empty() {
        let mut block = String::new();
        for (i, c) in review_level.iter().enumerate() {
            if i > 0 {
                block.push_str("\n\n");
            }
            if config.comment_type_prefix {
                block.push_str(&format!("[{}] ", c.comment_type.as_str()));
            }
            block.push_str(&c.content);
        }
        sections.push(block);
    }

    if !moved_to_summary.is_empty() {
        let mut block = String::from("## Unplaced comments\n");
        for item in moved_to_summary {
            let prefix = if config.comment_type_prefix {
                format!("[{}] ", item.comment.comment_type.as_str())
            } else {
                String::new()
            };
            let path = item.file.display();
            block.push_str(&format!(
                "- {prefix}{path}: {body}\n",
                body = item.comment.content
            ));
        }
        // strip trailing newline so join below produces one blank line, not two
        if block.ends_with('\n') {
            block.pop();
        }
        sections.push(block);
    }

    sections.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::comment::{Comment, CommentType, LineContext, LineRange, LineSide};
    use crate::model::diff_types::{DiffHunk, DiffLine, FileStatus, LineOrigin};
    use std::path::PathBuf;

    fn line(origin: LineOrigin, new: Option<u32>, old: Option<u32>) -> DiffLine {
        DiffLine {
            origin,
            content: String::new(),
            old_lineno: old,
            new_lineno: new,
            highlighted_spans: None,
        }
    }

    fn hunk(lines: Vec<DiffLine>) -> DiffHunk {
        DiffHunk {
            header: "@@".to_string(),
            old_start: 1,
            old_count: 0,
            new_start: 1,
            new_count: 0,
            lines,
        }
    }

    fn file_with_hunks(hunks: Vec<DiffHunk>) -> DiffFile {
        DiffFile {
            old_path: Some(PathBuf::from("src/lib.rs")),
            new_path: Some(PathBuf::from("src/lib.rs")),
            status: FileStatus::Modified,
            hunks,
            is_binary: false,
            is_too_large: false,
            is_commit_message: false,
            content_hash: 0,
        }
    }

    fn typical_file() -> DiffFile {
        file_with_hunks(vec![hunk(vec![
            line(LineOrigin::Context, Some(10), Some(10)),
            line(LineOrigin::Deletion, None, Some(11)),
            line(LineOrigin::Addition, Some(11), None),
            line(LineOrigin::Context, Some(12), Some(12)),
        ])])
    }

    fn default_config() -> ForgeConfig {
        ForgeConfig::default()
    }

    fn comment_with_line(side: LineSide, new: Option<u32>, old: Option<u32>) -> Comment {
        let mut c = Comment::new("needs work".to_string(), CommentType::Issue, Some(side));
        c.line_context = Some(LineContext {
            new_line: new,
            old_line: old,
            content: String::new(),
        });
        c
    }

    fn comment_range(side: LineSide, range: LineRange) -> Comment {
        Comment::new_with_range("ranged".to_string(), CommentType::Note, Some(side), range)
    }

    fn comment_file_level() -> Comment {
        Comment::new("module is messy".to_string(), CommentType::Note, None)
    }

    /// Infer the `CommentAnchor` from a test fixture's `Comment` shape.
    /// Production callers (`App::start_submit`) build the anchor from the
    /// `line_comments` HashMap key, but tests construct the comment with
    /// `line_context` already populated, so we can reflect it.
    fn anchor_from(comment: &Comment) -> CommentAnchor {
        if comment.line_range.is_some() {
            return CommentAnchor::Range;
        }
        let side = comment.side.unwrap_or_default();
        let line = comment.line_context.as_ref().and_then(|ctx| match side {
            LineSide::New => ctx.new_line,
            LineSide::Old => ctx.old_line,
        });
        match line {
            Some(l) => CommentAnchor::Line { line: l, side },
            None => CommentAnchor::FileLevel,
        }
    }

    // first_valid_line on DiffFile

    #[test]
    fn should_return_first_addition_line_on_new_side() {
        let file = file_with_hunks(vec![hunk(vec![
            line(LineOrigin::Deletion, None, Some(11)),
            line(LineOrigin::Addition, Some(20), None),
            line(LineOrigin::Context, Some(21), Some(13)),
        ])]);
        assert_eq!(file.first_valid_line(LineSide::New), Some(20));
    }

    #[test]
    fn should_return_first_deletion_line_on_old_side() {
        let file = file_with_hunks(vec![hunk(vec![
            line(LineOrigin::Addition, Some(20), None),
            line(LineOrigin::Deletion, None, Some(11)),
            line(LineOrigin::Deletion, None, Some(12)),
        ])]);
        assert_eq!(file.first_valid_line(LineSide::Old), Some(11));
    }

    #[test]
    fn should_return_none_for_binary_file_first_valid_line() {
        let mut file = typical_file();
        file.is_binary = true;
        assert!(file.first_valid_line(LineSide::New).is_none());
    }

    #[test]
    fn should_return_none_for_too_large_file_first_valid_line() {
        let mut file = typical_file();
        file.is_too_large = true;
        assert!(file.first_valid_line(LineSide::New).is_none());
    }

    // Single-line mapping

    #[test]
    fn should_map_single_addition_line_to_right_side() {
        let comment = comment_with_line(LineSide::New, Some(11), None);
        let mapped = map_comment(
            &comment,
            anchor_from(&comment),
            &typical_file(),
            &default_config(),
        );
        match mapped {
            MappedComment::Inline(inline) => {
                assert_eq!(inline.line, 11);
                assert_eq!(inline.side, GhSide::Right);
                assert_eq!(inline.start_line, None);
                assert_eq!(inline.start_side, None);
                assert!(inline.body.starts_with("[ISSUE] "));
            }
            other => panic!("expected Inline, got {other:?}"),
        }
    }

    #[test]
    fn should_map_single_context_line_to_right_side() {
        let comment = comment_with_line(LineSide::New, Some(10), Some(10));
        let mapped = map_comment(
            &comment,
            anchor_from(&comment),
            &typical_file(),
            &default_config(),
        );
        assert!(matches!(
            mapped,
            MappedComment::Inline(InlineComment {
                side: GhSide::Right,
                line: 10,
                ..
            })
        ));
    }

    #[test]
    fn should_map_single_deletion_line_to_left_side() {
        let comment = comment_with_line(LineSide::Old, None, Some(11));
        let mapped = map_comment(
            &comment,
            anchor_from(&comment),
            &typical_file(),
            &default_config(),
        );
        match mapped {
            MappedComment::Inline(inline) => {
                assert_eq!(inline.line, 11);
                assert_eq!(inline.side, GhSide::Left);
            }
            other => panic!("expected Inline, got {other:?}"),
        }
    }

    // Range mapping

    #[test]
    fn should_map_new_side_range_to_right_start_and_end() {
        let file = file_with_hunks(vec![hunk(vec![
            line(LineOrigin::Addition, Some(10), None),
            line(LineOrigin::Addition, Some(11), None),
            line(LineOrigin::Addition, Some(12), None),
        ])]);
        let comment = comment_range(LineSide::New, LineRange::new(10, 12));
        let mapped = map_comment(&comment, anchor_from(&comment), &file, &default_config());
        match mapped {
            MappedComment::Inline(inline) => {
                assert_eq!(inline.line, 12);
                assert_eq!(inline.start_line, Some(10));
                assert_eq!(inline.side, GhSide::Right);
                assert_eq!(inline.start_side, Some(GhSide::Right));
            }
            other => panic!("expected Inline, got {other:?}"),
        }
    }

    #[test]
    fn should_map_old_side_range_to_left_start_and_end() {
        let file = file_with_hunks(vec![hunk(vec![
            line(LineOrigin::Deletion, None, Some(20)),
            line(LineOrigin::Deletion, None, Some(21)),
            line(LineOrigin::Deletion, None, Some(22)),
        ])]);
        let comment = comment_range(LineSide::Old, LineRange::new(20, 22));
        let mapped = map_comment(&comment, anchor_from(&comment), &file, &default_config());
        match mapped {
            MappedComment::Inline(inline) => {
                assert_eq!(inline.line, 22);
                assert_eq!(inline.start_line, Some(20));
                assert_eq!(inline.side, GhSide::Left);
                assert_eq!(inline.start_side, Some(GhSide::Left));
            }
            other => panic!("expected Inline, got {other:?}"),
        }
    }

    #[test]
    fn should_flatten_single_line_range_to_inline_without_start_fields() {
        let file = file_with_hunks(vec![hunk(vec![line(LineOrigin::Addition, Some(15), None)])]);
        let comment = comment_range(LineSide::New, LineRange::single(15));
        let mapped = map_comment(&comment, anchor_from(&comment), &file, &default_config());
        match mapped {
            MappedComment::Inline(inline) => {
                assert_eq!(inline.line, 15);
                assert_eq!(inline.start_line, None);
                assert_eq!(inline.start_side, None);
            }
            other => panic!("expected Inline, got {other:?}"),
        }
    }

    #[test]
    fn should_mark_mixed_side_range_as_unmappable() {
        // Range claims New side but file only has Old-side lines at 20-22.
        let file = file_with_hunks(vec![hunk(vec![
            line(LineOrigin::Deletion, None, Some(20)),
            line(LineOrigin::Deletion, None, Some(21)),
            line(LineOrigin::Deletion, None, Some(22)),
        ])]);
        let comment = comment_range(LineSide::New, LineRange::new(20, 22));
        let mapped = map_comment(&comment, anchor_from(&comment), &file, &default_config());
        match mapped {
            MappedComment::Unmappable { reason, .. } => {
                assert_eq!(reason, UnmappableReason::MixedSideRange);
            }
            other => panic!("expected Unmappable, got {other:?}"),
        }
    }

    // File-level mapping

    #[test]
    fn should_anchor_file_level_to_first_valid_new_line() {
        let comment = comment_file_level();
        let mapped = map_comment(
            &comment,
            anchor_from(&comment),
            &typical_file(),
            &default_config(),
        );
        match mapped {
            MappedComment::Inline(inline) => {
                assert_eq!(inline.line, 10);
                assert_eq!(inline.side, GhSide::Right);
                assert!(inline.body.starts_with("[NOTE] File-level: "));
            }
            other => panic!("expected Inline, got {other:?}"),
        }
    }

    #[test]
    fn should_mark_file_level_without_new_anchor_as_unmappable() {
        // Pure deletion file: nothing on the New side.
        let file = file_with_hunks(vec![hunk(vec![line(LineOrigin::Deletion, None, Some(5))])]);
        let comment = comment_file_level();
        let mapped = map_comment(&comment, anchor_from(&comment), &file, &default_config());
        match mapped {
            MappedComment::Unmappable { reason, .. } => {
                assert_eq!(reason, UnmappableReason::FileLevelNoAnchor);
            }
            other => panic!("expected Unmappable, got {other:?}"),
        }
    }

    #[test]
    fn should_mark_binary_file_comment_as_unmappable() {
        let mut file = typical_file();
        file.is_binary = true;
        let comment = comment_with_line(LineSide::New, Some(11), None);
        let mapped = map_comment(&comment, anchor_from(&comment), &file, &default_config());
        assert!(matches!(
            mapped,
            MappedComment::Unmappable {
                reason: UnmappableReason::BinaryFile,
                ..
            }
        ));
    }

    #[test]
    fn should_mark_too_large_file_comment_as_unmappable() {
        let mut file = typical_file();
        file.is_too_large = true;
        let comment = comment_file_level();
        let mapped = map_comment(&comment, anchor_from(&comment), &file, &default_config());
        assert!(matches!(
            mapped,
            MappedComment::Unmappable {
                reason: UnmappableReason::TooLargeFile,
                ..
            }
        ));
    }

    // Body prefix toggle

    #[test]
    fn should_omit_type_prefix_when_config_disables_it() {
        let comment = comment_with_line(LineSide::New, Some(11), None);
        let cfg = ForgeConfig {
            comment_type_prefix: false,
        };
        let mapped = map_comment(&comment, anchor_from(&comment), &typical_file(), &cfg);
        match mapped {
            MappedComment::Inline(inline) => {
                assert!(!inline.body.contains("[ISSUE]"));
                assert_eq!(inline.body, "needs work");
            }
            other => panic!("expected Inline, got {other:?}"),
        }
    }

    // build_review_body

    fn note(content: &str) -> Comment {
        Comment::new(content.to_string(), CommentType::Note, None)
    }

    #[test]
    fn should_return_empty_body_when_no_inputs() {
        let body = build_review_body(&[], &[], &default_config());
        assert_eq!(body, "");
    }

    #[test]
    fn should_render_review_level_comments_with_type_prefix() {
        let comments = vec![note("first"), note("second")];
        let body = build_review_body(&comments, &[], &default_config());
        assert_eq!(body, "[NOTE] first\n\n[NOTE] second");
    }

    #[test]
    fn should_render_unplaced_comments_section() {
        let item = MovedToSummaryItem {
            comment: Comment::new("kaboom".to_string(), CommentType::Issue, None),
            file: PathBuf::from("src/lib.rs"),
        };
        let body = build_review_body(&[], &[item], &default_config());
        assert!(body.contains("## Unplaced comments"));
        assert!(body.contains("- [ISSUE] src/lib.rs: kaboom"));
    }

    #[test]
    fn should_render_review_level_above_unplaced_section() {
        let review = vec![note("top")];
        let summary = vec![MovedToSummaryItem {
            comment: Comment::new("middle".to_string(), CommentType::Note, None),
            file: PathBuf::from("a.rs"),
        }];
        let body = build_review_body(&review, &summary, &default_config());
        let top = body.find("[NOTE] top").expect("review comment");
        let middle = body.find("## Unplaced comments").expect("unplaced section");
        assert!(top < middle, "section ordering: {body}");
    }

    #[test]
    fn should_omit_type_prefix_in_body_when_disabled() {
        let cfg = ForgeConfig {
            comment_type_prefix: false,
        };
        let comments = vec![note("just text")];
        let body = build_review_body(&comments, &[], &cfg);
        assert_eq!(body, "just text");
    }

    // SubmitEvent

    #[test]
    fn should_map_each_event_to_correct_github_field() {
        assert_eq!(SubmitEvent::Comment.github_event(), Some("COMMENT"));
        assert_eq!(SubmitEvent::Approve.github_event(), Some("APPROVE"));
        assert_eq!(
            SubmitEvent::RequestChanges.github_event(),
            Some("REQUEST_CHANGES")
        );
        assert_eq!(SubmitEvent::Draft.github_event(), None);
    }
}
