//! Wrap-aware visual row-height helper for a single annotation.
//!
//! `annotation_row_height(app, idx)` returns the same number of terminal
//! rows the diff renderer will emit for `line_annotations[idx]` at the
//! current `viewport_width` and `wrap_lines`. It mirrors the renderer's
//! single wrap pass logical-line by logical-line so that jumps (zz, G,
//! paging) can sum heights without re-running the renderer.
//!
//! Every rendered-body format string used to reconstruct a row lives
//! once, in the renderer's shared text builders in `ui::diff_view`; this
//! module only calls them and concatenates the parts (cursor-indicator
//! spacing is the sole literal kept locally, since the renderer
//! constructs it as a styled span rather than as a shared string).
//! Comment-box annotations are pre-wrapped by `format_comment_lines`, so
//! each is exactly one row.

use ratatui::text::Span;

use crate::app::{AnnotatedLine, App, DiffViewMode, sbs_overhead};
use crate::ui::diff_view;
use crate::ui::text_utils::wrap_spans;

pub(crate) fn annotation_row_height(app: &App, idx: usize) -> usize {
    if !app.diff_state.wrap_lines {
        return 1;
    }
    let viewport_width = app.diff_state.viewport_width;
    if viewport_width == 0 {
        return 1;
    }
    // SBS parity: the renderer disables the entire wrap pass (including
    // non-content rows) when the per-side content width is 0, falling back
    // to height 1 for every row.
    if app.diff_view_mode == DiffViewMode::SideBySide && sbs_content_width(app, viewport_width) == 0
    {
        return 1;
    }
    let Some(annotation) = app.line_annotations.get(idx) else {
        return 1;
    };

    match annotation {
        // Pre-wrapped by comment_panel::wrap_segments to inner width - 1.
        AnnotatedLine::ReviewComment { .. }
        | AnnotatedLine::RemoteReviewSummaryLine { .. }
        | AnnotatedLine::FileComment { .. }
        | AnnotatedLine::LineComment { .. }
        | AnnotatedLine::RemoteThreadLine { .. } => 1,

        AnnotatedLine::SideBySideLine {
            file_idx,
            hunk_idx,
            del_line_idx,
            add_line_idx,
            ..
        } if app.diff_view_mode == DiffViewMode::SideBySide => {
            let content_width = sbs_content_width(app, viewport_width);
            let (left, right) =
                sbs_row_contents(app, *file_idx, *hunk_idx, *del_line_idx, *add_line_idx);
            wrap_side_max(&left, &right, content_width)
        }

        AnnotatedLine::ExpandedContext {
            gap_id,
            line_idx: li,
        } if app.diff_view_mode == DiffViewMode::SideBySide => {
            let content = expanded_line_content(app, gap_id, *li).unwrap_or_default();
            let content_width = sbs_content_width(app, viewport_width);
            wrap_side_max(&content, &content, content_width)
        }

        _ => {
            let text = full_row_text(app, annotation);
            wrap_len(&text, viewport_width)
        }
    }
}

fn wrap_len(text: &str, width: usize) -> usize {
    wrap_spans(&[Span::raw(text.to_string())], width).len()
}

fn wrap_side_max(left: &str, right: &str, content_width: usize) -> usize {
    let l = wrap_len(left, content_width);
    let r = wrap_len(right, content_width);
    l.max(r).max(1)
}

fn sbs_content_width(app: &App, viewport_width: usize) -> usize {
    let lw = app.lineno_width();
    let overhead = sbs_overhead(lw) as usize;
    viewport_width.saturating_sub(overhead) / 2
}

fn sbs_row_contents(
    app: &App,
    file_idx: usize,
    hunk_idx: usize,
    del_line_idx: Option<usize>,
    add_line_idx: Option<usize>,
) -> (String, String) {
    let hunk = app
        .diff_files
        .get(file_idx)
        .and_then(|f| f.hunks.get(hunk_idx));
    let get = |i: Option<usize>| -> String {
        i.and_then(|idx| hunk.and_then(|h| h.lines.get(idx)))
            .map(|dl| dl.content.clone())
            .unwrap_or_default()
    };
    (get(del_line_idx), get(add_line_idx))
}

fn expanded_line_content(app: &App, gap_id: &crate::app::GapId, idx: usize) -> Option<String> {
    let top = app.expanded_top.get(gap_id);
    let top_len = top.map_or(0, |v| v.len());
    if idx < top_len {
        top?.get(idx).map(|dl| dl.content.clone())
    } else {
        app.expanded_bottom
            .get(gap_id)?
            .get(idx - top_len)
            .map(|dl| dl.content.clone())
    }
}

/// Reconstruct the concatenated text of a rendered logical line by calling
/// the same text builders the renderer uses. Used for the "wrap at full
/// inner width" branch: unified mode, and SBS non-content rows.
fn full_row_text(app: &App, annotation: &AnnotatedLine) -> String {
    let lw = app.lineno_width();
    let indicator = " ";
    let indicator_spaced = "  ";

    match annotation {
        AnnotatedLine::ReviewCommentsHeader => {
            format!(
                "{indicator_spaced}{}{}",
                diff_view::REVIEW_COMMENTS_HEADER_PREFIX,
                diff_view::HEADER_RULE
            )
        }

        AnnotatedLine::FileHeader { file_idx } => match app.diff_files.get(*file_idx) {
            Some(file) => format!(
                "{indicator_spaced}{}{}",
                diff_view::file_header_prefix_text(app, file),
                diff_view::HEADER_RULE
            ),
            None => indicator_spaced.to_string(),
        },

        AnnotatedLine::HunkHeader { file_idx, hunk_idx } => {
            let text = app
                .diff_files
                .get(*file_idx)
                .and_then(|f| f.hunks.get(*hunk_idx))
                .map(|h| {
                    diff_view::hunk_header_text_and_style(
                        &app.theme,
                        h,
                        app.is_hunk_reviewed(*file_idx, *hunk_idx),
                    )
                    .0
                })
                .unwrap_or_default();
            format!("{indicator_spaced}{text}")
        }

        AnnotatedLine::Expander { gap_id, direction } => {
            // Mirror the renderer's `remaining` computation at the call site;
            // the annotation itself doesn't carry it.
            let remaining = match app.gap_size(gap_id) {
                Some(gap) => {
                    let top_len = app.expanded_top.get(gap_id).map_or(0, |v| v.len());
                    let bot_len = app.expanded_bottom.get(gap_id).map_or(0, |v| v.len());
                    (gap as usize).saturating_sub(top_len + bot_len)
                }
                None => 1,
            };
            format!(
                "{indicator_spaced}{}",
                diff_view::expander_body_text(*direction, remaining)
            )
        }

        AnnotatedLine::HiddenLines { count, .. } => {
            format!(
                "{indicator_spaced}{}",
                diff_view::hidden_lines_body_text(*count)
            )
        }

        AnnotatedLine::ExpandedContext { gap_id, line_idx } => {
            // Unified branch; SBS is handled by the outer match.
            let dl = expanded_diff_line(app, gap_id, *line_idx);
            let (lineno, content) = match dl {
                Some(dl) => (
                    diff_view::expanded_context_lineno_field(&dl, lw),
                    dl.content,
                ),
                None => (" ".repeat(lw + 1), String::new()),
            };
            format!("{indicator}{lineno}  {content}")
        }

        AnnotatedLine::DiffLine {
            file_idx,
            hunk_idx,
            line_idx,
            ..
        } => {
            let dl = app
                .diff_files
                .get(*file_idx)
                .and_then(|f| f.hunks.get(*hunk_idx))
                .and_then(|h| h.lines.get(*line_idx));
            match dl {
                Some(dl) => {
                    let lineno = diff_view::unified_line_number_field(dl, lw);
                    let prefix = diff_view::unified_line_origin_marker(dl);
                    format!("{indicator}{lineno}{prefix} {}", dl.content)
                }
                None => format!("{indicator}{}", " ".repeat(lw + 1)),
            }
        }

        // Defensive: SBS annotation seen while in Unified mode. The renderer
        // wouldn't emit this; concatenate both sides at full width as a best
        // effort so we still return a sensible row count.
        AnnotatedLine::SideBySideLine {
            file_idx,
            hunk_idx,
            del_line_idx,
            add_line_idx,
            ..
        } => {
            let (l, r) = sbs_row_contents(app, *file_idx, *hunk_idx, *del_line_idx, *add_line_idx);
            format!("{indicator}{l} {r}")
        }

        AnnotatedLine::BinaryOrEmpty { file_idx } => {
            let text = app
                .diff_files
                .get(*file_idx)
                .map(diff_view::binary_or_empty_label)
                .unwrap_or("");
            format!("{indicator_spaced}{text}")
        }

        AnnotatedLine::Spacing => {
            // The SBS renderer always emits a bare-indicator spacing row;
            // only unified single-file view emits the `↓ next-file` hint.
            if app.diff_view_mode == DiffViewMode::Unified
                && app.is_single_file_view
                && let Some(f) = app.diff_files.get(app.diff_state.current_file_idx + 1)
            {
                let path = f.display_path().display().to_string();
                format!(
                    "{indicator}{}",
                    diff_view::spacing_next_file_hint_text(&path)
                )
            } else {
                indicator.to_string()
            }
        }

        // Comment-ish rows are pre-wrapped and handled by the outer match.
        AnnotatedLine::ReviewComment { .. }
        | AnnotatedLine::RemoteReviewSummaryLine { .. }
        | AnnotatedLine::FileComment { .. }
        | AnnotatedLine::LineComment { .. }
        | AnnotatedLine::RemoteThreadLine { .. } => indicator.to_string(),
    }
}

fn expanded_diff_line(
    app: &App,
    gap_id: &crate::app::GapId,
    idx: usize,
) -> Option<crate::model::DiffLine> {
    let top = app.expanded_top.get(gap_id);
    let top_len = top.map_or(0, |v| v.len());
    if idx < top_len {
        top?.get(idx).cloned()
    } else {
        app.expanded_bottom.get(gap_id)?.get(idx - top_len).cloned()
    }
}

#[cfg(test)]
mod tests {
    //! Parity: for every fully visible logical line after render, the visual
    //! row count derived from `diff_row_to_annotation` must match
    //! `annotation_row_height`. The fixture also asserts coverage across all
    //! annotation variants so future edits can't silently drop branches.
    use super::*;
    use crate::app::{
        App, DiffSource, DiffViewMode, ExpandDirection, GapId, InputMode, PullRequestDiffSource,
    };
    use crate::error::Result as TuicrResult;
    use crate::error::TuicrError;
    use crate::forge::traits::{ForgeRepository, PrSessionKey};
    use crate::model::{
        Comment, CommentType, DiffFile, DiffHunk, DiffLine, FileStatus, LineOrigin, LineSide,
        ReviewSession, SessionDiffSource,
    };
    use crate::syntax::SyntaxHighlighter;
    use crate::theme::Theme;
    use crate::vcs::traits::{VcsBackend, VcsChangeStatus, VcsInfo, VcsType};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;
    use std::path::{Path, PathBuf};

    struct StubVcs {
        info: VcsInfo,
    }
    impl VcsBackend for StubVcs {
        fn info(&self) -> &VcsInfo {
            &self.info
        }
        fn get_working_tree_diff(
            &self,
            _highlighter: &SyntaxHighlighter,
        ) -> TuicrResult<Vec<DiffFile>> {
            Err(TuicrError::NoChanges)
        }
        fn fetch_context_lines(
            &self,
            _file_path: &Path,
            _file_status: FileStatus,
            _ref_commit: Option<&str>,
            start_line: u32,
            end_line: u32,
        ) -> TuicrResult<Vec<DiffLine>> {
            // Synthetic context so `expand_gap` can populate ExpandedContext
            // annotations in tests.
            Ok((start_line..=end_line)
                .map(|n| DiffLine {
                    origin: LineOrigin::Context,
                    content: format!("ctx line {n}"),
                    old_lineno: Some(n),
                    new_lineno: Some(n),
                    highlighted_spans: None,
                })
                .collect())
        }
        fn get_change_status(&self) -> TuicrResult<VcsChangeStatus> {
            Ok(VcsChangeStatus {
                staged: false,
                unstaged: false,
            })
        }
        fn file_line_count(
            &self,
            _file_path: &Path,
            _file_status: FileStatus,
            _ref_commit: Option<&str>,
        ) -> TuicrResult<u32> {
            // Non-zero so EOF gaps and top-of-file gaps get expander/hidden
            // annotations built.
            Ok(500)
        }
    }

    fn code_file() -> DiffFile {
        let long_left = "L".repeat(140);
        let long_right = "R".repeat(90);
        // Hunk 1 starts at line 30 (gap-of-29 before it → HiddenLines + Expander(Up)).
        // Hunk 2 starts at line 100 (gap of ~66 between hunks → Expander(Down) +
        // HiddenLines + Expander(Up)).
        let hunk1 = DiffHunk {
            header: "@@ -30,2 +30,3 @@ fn foo".to_string(),
            lines: vec![
                DiffLine {
                    origin: LineOrigin::Context,
                    content: "context short".to_string(),
                    old_lineno: Some(30),
                    new_lineno: Some(30),
                    highlighted_spans: None,
                },
                DiffLine {
                    origin: LineOrigin::Deletion,
                    content: long_left,
                    old_lineno: Some(31),
                    new_lineno: None,
                    highlighted_spans: None,
                },
                DiffLine {
                    origin: LineOrigin::Addition,
                    content: long_right,
                    old_lineno: None,
                    new_lineno: Some(31),
                    highlighted_spans: None,
                },
                DiffLine {
                    origin: LineOrigin::Addition,
                    content: "y".to_string(),
                    old_lineno: None,
                    new_lineno: Some(32),
                    highlighted_spans: None,
                },
            ],
            old_start: 30,
            old_count: 2,
            new_start: 30,
            new_count: 3,
        };
        let hunk2 = DiffHunk {
            header: "@@ -100,1 +100,1 @@ fn bar".to_string(),
            lines: vec![DiffLine {
                origin: LineOrigin::Context,
                content: "tail".to_string(),
                old_lineno: Some(100),
                new_lineno: Some(100),
                highlighted_spans: None,
            }],
            old_start: 100,
            old_count: 1,
            new_start: 100,
            new_count: 1,
        };
        let hunks = vec![hunk1, hunk2];
        let content_hash = DiffFile::compute_content_hash(&hunks);
        DiffFile {
            old_path: Some(PathBuf::from("src/lib.rs")),
            new_path: Some(PathBuf::from("src/lib.rs")),
            status: FileStatus::Modified,
            hunks,
            is_binary: false,
            is_too_large: false,
            is_commit_message: false,
            content_hash,
        }
    }

    fn binary_file() -> DiffFile {
        DiffFile {
            old_path: Some(PathBuf::from("assets/logo.png")),
            new_path: Some(PathBuf::from("assets/logo.png")),
            status: FileStatus::Modified,
            hunks: Vec::new(),
            is_binary: true,
            is_too_large: false,
            is_commit_message: false,
            content_hash: 0,
        }
    }

    fn make_app() -> App {
        let repo = ForgeRepository::github("github.com", "test", "tuicr");
        let pr = PullRequestDiffSource {
            key: PrSessionKey::new(repo, 1, "headsha".to_string()),
            base_sha: "base".to_string(),
            title: "t".to_string(),
            url: "u".to_string(),
            head_ref_name: "feat".to_string(),
            base_ref_name: "main".to_string(),
            state: "OPEN".to_string(),
            closed: false,
            merged: false,
        };
        let vcs_info = VcsInfo {
            root_path: PathBuf::from("forge:github.com/test/tuicr"),
            head_commit: "headsha".to_string(),
            branch_name: Some("feat".to_string()),
            vcs_type: VcsType::File,
        };
        let mut session = ReviewSession::new(
            vcs_info.root_path.clone(),
            "headsha".to_string(),
            Some("feat".to_string()),
            SessionDiffSource::PullRequest,
        );
        session.pr_session_key = Some(pr.key.clone());
        let mut app = App::build(
            Box::new(StubVcs {
                info: vcs_info.clone(),
            }),
            vcs_info,
            Theme::dark(),
            None,
            false,
            vec![code_file(), binary_file()],
            session,
            DiffSource::PullRequest(Box::new(pr)),
            InputMode::Normal,
            Vec::new(),
            None,
            None,
        )
        .expect("build app");
        let path = PathBuf::from("src/lib.rs");
        let review = app.session.get_file_mut(&path).expect("file registered");
        review.add_line_comment(
            31,
            Comment::new(
                "this line is important".to_string(),
                CommentType::from_id("note"),
                Some(LineSide::New),
            ),
        );
        review.add_file_comment(Comment::new(
            "overall the file looks good".to_string(),
            CommentType::from_id("praise"),
            None,
        ));
        // `sort_files_by_directory` reorders diff_files by parent directory,
        // so `assets/logo.png` lands before `src/lib.rs`; find the code file
        // by path rather than assuming its post-sort index.
        let code_idx = app
            .diff_files
            .iter()
            .position(|f| f.display_path().as_path() == Path::new("src/lib.rs"))
            .expect("code file present");
        // Partially expand the gap between hunk 1 and hunk 2 so we get both
        // an Expander row and ExpandedContext rows.
        app.expand_gap(
            GapId {
                file_idx: code_idx,
                hunk_idx: 1,
            },
            ExpandDirection::Down,
            Some(2),
        )
        .expect("expand gap");
        app
    }

    fn render_diff(app: &mut App, mode: DiffViewMode, w: u16, h: u16) {
        app.diff_view_mode = mode;
        app.rebuild_annotations();
        let backend = TestBackend::new(w, h);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| match mode {
                DiffViewMode::Unified => {
                    crate::ui::diff_unified::render_unified_diff(frame, app, Rect::new(0, 0, w, h));
                }
                DiffViewMode::SideBySide => {
                    crate::ui::diff_side_by_side::render_side_by_side_diff(
                        frame,
                        app,
                        Rect::new(0, 0, w, h),
                    );
                }
            })
            .expect("draw");
    }

    /// Group consecutive identical annotation indices in `diff_row_to_annotation`
    /// into (annotation_idx, visible_row_count) pairs. Skips the final group
    /// because the renderer may have truncated it at the viewport bottom.
    fn observed_heights(app: &App) -> Vec<(usize, usize)> {
        let map = &app.diff_row_to_annotation;
        let mut groups: Vec<(usize, usize)> = Vec::new();
        for &ann in map {
            match groups.last_mut() {
                Some(g) if g.0 == ann => g.1 += 1,
                _ => groups.push((ann, 1)),
            }
        }
        if !groups.is_empty() {
            groups.pop();
        }
        groups
    }

    fn assert_parity(app: &App) {
        let obs = observed_heights(app);
        assert!(
            !obs.is_empty(),
            "no fully visible annotations captured; increase viewport height"
        );
        for (ann_idx, observed) in obs {
            let computed = annotation_row_height(app, ann_idx);
            assert_eq!(
                observed,
                computed,
                "annotation {ann_idx} ({:?}): renderer produced {observed} rows, helper computed {computed}",
                app.line_annotations.get(ann_idx)
            );
        }
    }

    /// Bin an annotation into one of the coverage buckets. Returns `None` for
    /// variants we don't require the fixture to exercise.
    fn variant_bucket(ann: &AnnotatedLine) -> Option<&'static str> {
        match ann {
            AnnotatedLine::FileHeader { .. } => Some("FileHeader"),
            AnnotatedLine::HunkHeader { .. } => Some("HunkHeader"),
            AnnotatedLine::DiffLine { .. } => Some("DiffLine"),
            AnnotatedLine::SideBySideLine { .. } => Some("SideBySideLine"),
            AnnotatedLine::Expander { .. } => Some("Expander"),
            AnnotatedLine::HiddenLines { .. } => Some("HiddenLines"),
            AnnotatedLine::ExpandedContext { .. } => Some("ExpandedContext"),
            AnnotatedLine::LineComment { .. } => Some("LineComment"),
            AnnotatedLine::FileComment { .. } => Some("FileComment"),
            AnnotatedLine::Spacing => Some("Spacing"),
            AnnotatedLine::BinaryOrEmpty { .. } => Some("BinaryOrEmpty"),
            _ => None,
        }
    }

    fn assert_coverage(app: &App, mode: DiffViewMode) {
        use std::collections::BTreeSet;
        // Derive coverage from the parity-verified set (i.e. the same
        // groups `assert_parity` iterates over) so every counted variant
        // is guaranteed to have been height-checked. The final possibly-
        // truncated group that `observed_heights` drops is excluded here
        // for the same reason.
        let mut seen: BTreeSet<&'static str> = BTreeSet::new();
        for (ann_idx, _) in observed_heights(app) {
            if let Some(a) = app.line_annotations.get(ann_idx)
                && let Some(bucket) = variant_bucket(a)
            {
                seen.insert(bucket);
            }
        }
        let mut required: BTreeSet<&'static str> = [
            "FileHeader",
            "HunkHeader",
            "Expander",
            "HiddenLines",
            "ExpandedContext",
            "LineComment",
            "FileComment",
            "Spacing",
            "BinaryOrEmpty",
        ]
        .into_iter()
        .collect();
        required.insert(match mode {
            DiffViewMode::Unified => "DiffLine",
            DiffViewMode::SideBySide => "SideBySideLine",
        });
        let missing: Vec<_> = required.difference(&seen).copied().collect();
        assert!(
            missing.is_empty(),
            "parity fixture missing coverage for: {missing:?} (mode {mode:?}); saw {seen:?}"
        );
    }

    #[test]
    fn parity_unified_with_wrap() {
        let mut app = make_app();
        app.set_diff_wrap(true);
        // Tall viewport so every logical line, including BinaryOrEmpty on the
        // second file, is fully visible and captured by `observed_heights`.
        render_diff(&mut app, DiffViewMode::Unified, 40, 200);
        assert_coverage(&app, DiffViewMode::Unified);
        assert_parity(&app);
    }

    #[test]
    fn parity_side_by_side_with_wrap() {
        let mut app = make_app();
        app.set_diff_wrap(true);
        render_diff(&mut app, DiffViewMode::SideBySide, 60, 200);
        assert_coverage(&app, DiffViewMode::SideBySide);
        assert_parity(&app);
    }

    #[test]
    fn parity_unified_wrap_on_single_file_view_next_hint_wraps() {
        // Single-file view emits a `↓ <next-file-path>` hint on the
        // inter-file Spacing row. Pick a long path + narrow viewport so
        // the hint wraps, and assert parity still holds for that row.
        let mut app = make_app();
        app.set_diff_wrap(true);
        // Replace the second file with one whose display path is long
        // enough to wrap at the chosen viewport width.
        // Rename the file that follows the current one in sort order so
        // its display path is long enough to wrap at the chosen viewport
        // width, then focus the file just before it in single-file view
        // so the Spacing row after it renders the next-file hint.
        let long_name = format!("{}.txt", "a".repeat(120));
        let long_path = PathBuf::from("src").join(long_name);
        // In the parity fixture there are exactly two diff files; after
        // `sort_files_by_directory` the code file lands second, so focus
        // index 0 and rewrite index 1's path to be long.
        assert_eq!(app.diff_files.len(), 2);
        app.diff_files[1].old_path = Some(long_path.clone());
        app.diff_files[1].new_path = Some(long_path);
        app.diff_state.current_file_idx = 0;
        app.is_single_file_view = true;
        app.rebuild_annotations();
        render_diff(&mut app, DiffViewMode::Unified, 40, 400);

        // Locate the Spacing annotation and confirm it wrapped to > 1 row.
        // In single-file view Spacing is the very last logical line, which
        // `observed_heights` drops on principle, so count its visible rows
        // directly against the row-to-annotation map instead.
        let spacing_idx = app
            .line_annotations
            .iter()
            .position(|a| matches!(a, AnnotatedLine::Spacing))
            .expect("Spacing annotation present");
        let observed_spacing_rows = app
            .diff_row_to_annotation
            .iter()
            .filter(|&&idx| idx == spacing_idx)
            .count();
        let computed = annotation_row_height(&app, spacing_idx);
        assert!(
            computed > 1,
            "expected next-file hint to wrap, computed {computed} rows"
        );
        assert_eq!(
            observed_spacing_rows, computed,
            "Spacing parity: renderer produced {observed_spacing_rows} rows, helper computed {computed}"
        );
        assert_parity(&app);
    }

    #[test]
    fn parity_side_by_side_single_file_view_spacing_is_bare() {
        // SBS always emits a bare-indicator Spacing row, even in
        // single-file view. Use the same long next-file path as the
        // unified test to prove it does NOT wrap here.
        let mut app = make_app();
        app.set_diff_wrap(true);
        let long_name = format!("{}.txt", "a".repeat(120));
        let long_path = PathBuf::from("src").join(long_name);
        assert_eq!(app.diff_files.len(), 2);
        app.diff_files[1].old_path = Some(long_path.clone());
        app.diff_files[1].new_path = Some(long_path);
        app.diff_state.current_file_idx = 0;
        app.is_single_file_view = true;
        app.rebuild_annotations();
        render_diff(&mut app, DiffViewMode::SideBySide, 60, 400);

        let spacing_idx = app
            .line_annotations
            .iter()
            .position(|a| matches!(a, AnnotatedLine::Spacing))
            .expect("Spacing annotation present");
        let observed_spacing_rows = app
            .diff_row_to_annotation
            .iter()
            .filter(|&&idx| idx == spacing_idx)
            .count();
        assert_eq!(
            observed_spacing_rows, 1,
            "SBS Spacing row must stay 1 row even in single-file view"
        );
        assert_eq!(annotation_row_height(&app, spacing_idx), 1);
        assert_parity(&app);
    }

    #[test]
    fn parity_unified_wrap_off_all_ones() {
        let mut app = make_app();
        app.set_diff_wrap(false);
        render_diff(&mut app, DiffViewMode::Unified, 40, 200);
        for (_, h) in observed_heights(&app) {
            assert_eq!(h, 1);
        }
        for i in 0..app.line_annotations.len() {
            assert_eq!(annotation_row_height(&app, i), 1);
        }
    }

    #[test]
    fn parity_side_by_side_wrap_off_all_ones() {
        let mut app = make_app();
        app.set_diff_wrap(false);
        render_diff(&mut app, DiffViewMode::SideBySide, 60, 200);
        for (_, h) in observed_heights(&app) {
            assert_eq!(h, 1);
        }
        for i in 0..app.line_annotations.len() {
            assert_eq!(annotation_row_height(&app, i), 1);
        }
    }
}
