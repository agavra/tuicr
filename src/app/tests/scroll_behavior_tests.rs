use crate::app::*;
use crate::model::FileStatus;
use crate::ui::row_height::annotation_row_height;
use crate::vcs::traits::VcsType;

struct DummyVcs {
    info: VcsInfo,
}

impl VcsBackend for DummyVcs {
    fn info(&self) -> &VcsInfo {
        &self.info
    }

    fn get_working_tree_diff(&self, _highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
        Err(TuicrError::NoChanges)
    }

    fn fetch_context_lines(
        &self,
        _file_path: &Path,
        _file_status: FileStatus,
        _ref_commit: Option<&str>,
        _start_line: u32,
        _end_line: u32,
    ) -> Result<Vec<DiffLine>> {
        Ok(Vec::new())
    }

    fn file_line_count(
        &self,
        _file_path: &Path,
        _file_status: FileStatus,
        _ref_commit: Option<&str>,
    ) -> Result<u32> {
        Ok(0)
    }
}

/// Build a test App with a single file containing `n` context lines.
/// Total rendered lines = 1 (review header) + 1 (file header) + 1 (spacing)
///                       + 1 (hunk header) + n (diff lines) = n + 4.
/// The viewport is set to `viewport` lines.
fn build_scroll_app(n: usize, viewport: usize, scroll_offset_config: usize) -> App {
    let lines: Vec<DiffLine> = (1..=n)
        .map(|i| DiffLine {
            origin: crate::model::LineOrigin::Context,
            content: format!("line {i}"),
            old_lineno: Some(i as u32),
            new_lineno: Some(i as u32),
            highlighted_spans: None,
        })
        .collect();

    let hunk = DiffHunk {
        header: "@@ -1,N +1,N @@".to_string(),
        lines,
        old_start: 1,
        old_count: n as u32,
        new_start: 1,
        new_count: n as u32,
    };

    let file = DiffFile {
        old_path: None,
        new_path: Some(PathBuf::from("test.rs")),
        status: FileStatus::Modified,
        hunks: vec![hunk],
        is_binary: false,
        is_too_large: false,
        is_commit_message: false,
        content_hash: 0,
    };

    let vcs_info = VcsInfo {
        root_path: PathBuf::from("/tmp"),
        head_commit: "abc".to_string(),
        branch_name: Some("main".to_string()),
        vcs_type: VcsType::Git,
    };
    let session = ReviewSession::new(
        vcs_info.root_path.clone(),
        vcs_info.head_commit.clone(),
        vcs_info.branch_name.clone(),
        SessionDiffSource::WorkingTree,
    );

    let mut app = App::build(
        Box::new(DummyVcs {
            info: vcs_info.clone(),
        }),
        vcs_info,
        Theme::dark(),
        None,
        false,
        vec![file],
        session,
        DiffSource::WorkingTree,
        InputMode::Normal,
        Vec::new(),
        None,
        None,
    )
    .expect("failed to build test app");

    app.diff_state.viewport_height = viewport;
    app.diff_state.visible_line_count = viewport;
    app.scroll_offset = scroll_offset_config;
    app
}

#[test]
fn zz_on_last_line_centers_cursor() {
    // 40 diff lines + 4 overhead = 44 total. max_cursor = 42. Viewport = 20.
    let mut app = build_scroll_app(40, 20, 5);
    assert_eq!(app.total_lines(), 44);
    let last = app.max_cursor_line(); // 42

    app.diff_state.cursor_line = last;
    app.center_cursor();

    // scroll = cursor - viewport/2 = 42 - 10 = 32
    assert_eq!(app.diff_state.scroll_offset, 32);
    assert_eq!(app.diff_state.cursor_line, 42);
}

#[test]
fn after_zz_on_last_line_j_does_not_change_scroll() {
    let mut app = build_scroll_app(40, 20, 5);
    let last = app.max_cursor_line();

    app.diff_state.cursor_line = last;
    app.center_cursor();
    let scroll_after_zz = app.diff_state.scroll_offset;

    // Press j — cursor is already at max, and it's centered (not near bottom margin)
    app.cursor_down(1);

    assert_eq!(app.diff_state.cursor_line, last);
    assert_eq!(
        app.diff_state.scroll_offset, scroll_after_zz,
        "j after zz on last line should not change scroll"
    );
}

#[test]
fn after_zz_on_last_line_k_does_not_change_scroll() {
    let mut app = build_scroll_app(40, 20, 5);
    let last = app.max_cursor_line();

    app.diff_state.cursor_line = last;
    app.center_cursor();
    let scroll_after_zz = app.diff_state.scroll_offset;

    // Press k — cursor moves up 1, still in free zone
    app.cursor_up(1);

    assert_eq!(app.diff_state.cursor_line, last - 1);
    assert_eq!(
        app.diff_state.scroll_offset, scroll_after_zz,
        "k after zz on last line should not change scroll"
    );
}

#[test]
fn after_zz_no_oscillation_with_k_then_j() {
    let mut app = build_scroll_app(40, 20, 5);
    let last = app.max_cursor_line();

    app.diff_state.cursor_line = last;
    app.center_cursor();
    let scroll_after_zz = app.diff_state.scroll_offset;

    // k then j should return to the same state
    app.cursor_up(1);
    app.cursor_down(1);

    assert_eq!(app.diff_state.cursor_line, last);
    assert_eq!(
        app.diff_state.scroll_offset, scroll_after_zz,
        "k then j after zz should not cause oscillation"
    );
}

#[test]
fn j_scrolls_one_line_at_a_time() {
    // Viewport 20, total 44. Start at the middle and scroll down.
    let mut app = build_scroll_app(40, 20, 5);

    // Position cursor and scroll in steady state near the bottom margin
    app.diff_state.cursor_line = 20;
    app.diff_state.scroll_offset = 6;
    // steady state: cursor at bottom margin = scroll + visible - margin - 1

    // Scroll down multiple times and verify single-line increments
    for _ in 0..10 {
        let prev_scroll = app.diff_state.scroll_offset;
        let prev_cursor = app.diff_state.cursor_line;
        app.cursor_down(1);
        let scroll_delta = app.diff_state.scroll_offset - prev_scroll;
        let cursor_delta = app.diff_state.cursor_line - prev_cursor;
        assert_eq!(cursor_delta, 1, "cursor should advance by exactly 1");
        assert!(
            scroll_delta <= 1,
            "scroll should advance by at most 1, got {scroll_delta}"
        );
    }
}

#[test]
fn j_on_last_line_near_bottom_does_not_scroll() {
    let mut app = build_scroll_app(40, 20, 5);
    let last = app.max_cursor_line();

    // Put cursor at last line with it near the bottom of viewport
    app.diff_state.cursor_line = last;
    app.diff_state.scroll_offset = last.saturating_sub(19); // cursor at bottom of viewport

    let prev_scroll = app.diff_state.scroll_offset;
    app.cursor_down(1);

    assert_eq!(app.diff_state.cursor_line, last);
    assert_eq!(
        app.diff_state.scroll_offset, prev_scroll,
        "j on last line should never scroll the view"
    );
}

#[test]
fn j_on_last_line_centered_does_not_scroll() {
    let mut app = build_scroll_app(40, 20, 5);
    let last = app.max_cursor_line();

    // Center cursor on last line
    app.diff_state.cursor_line = last;
    app.center_cursor();
    let scroll_after_center = app.diff_state.scroll_offset;

    app.cursor_down(1);

    assert_eq!(
        app.diff_state.scroll_offset, scroll_after_center,
        "j on last line when centered should not scroll"
    );
}

#[test]
fn k_reclaims_empty_space_below() {
    let mut app = build_scroll_app(40, 20, 5);
    let last = app.max_cursor_line();

    // Put cursor at last line at top of view (maximum empty space below)
    app.diff_state.cursor_line = last;
    app.diff_state.scroll_offset = last; // only 1 line visible

    // Press k — should immediately reclaim space (reduce scroll)
    app.cursor_up(1);

    assert_eq!(app.diff_state.cursor_line, last - 1);
    assert!(
        app.diff_state.scroll_offset < last,
        "k should reclaim empty space below, scroll was {} expected less than {}",
        app.diff_state.scroll_offset,
        last
    );
}

#[test]
fn max_scroll_allows_last_line_at_top() {
    let app = build_scroll_app(40, 20, 5);
    let total = app.total_lines();

    assert_eq!(
        app.max_scroll_offset(),
        total - 1,
        "max scroll should allow last line at top of viewport"
    );
}

#[test]
fn smooth_scroll_to_end_no_jumps() {
    // Start at beginning, scroll all the way to the end with j presses
    let mut app = build_scroll_app(40, 20, 5);
    let last = app.max_cursor_line();

    app.diff_state.cursor_line = 0;
    app.diff_state.scroll_offset = 0;

    let mut max_scroll_delta = 0;
    for _ in 0..last {
        let prev_scroll = app.diff_state.scroll_offset;
        app.cursor_down(1);
        let delta = app.diff_state.scroll_offset.saturating_sub(prev_scroll);
        if delta > max_scroll_delta {
            max_scroll_delta = delta;
        }
    }

    assert_eq!(app.diff_state.cursor_line, last);
    assert!(
        max_scroll_delta <= 1,
        "scroll should never jump more than 1 line at a time, max was {max_scroll_delta}"
    );
}

#[test]
fn k_below_midpoint_only_moves_cursor() {
    // After G, cursor is near the bottom of viewport. Pressing k should
    // only move the cursor, not also scroll the view (which would cause
    // a visual 2-line jump).
    let mut app = build_scroll_app(40, 20, 5);
    let last = app.max_cursor_line();

    // Simulate G: cursor at last line, scroll positions it at bottom
    app.diff_state.cursor_line = last;
    app.diff_state.scroll_offset = last.saturating_sub(19);
    let scroll_before = app.diff_state.scroll_offset;

    // k should only move cursor, not scroll
    app.cursor_up(1);
    assert_eq!(app.diff_state.cursor_line, last - 1);
    assert_eq!(
        app.diff_state.scroll_offset, scroll_before,
        "k when cursor is below midpoint should not change scroll"
    );
}

#[test]
fn no_scroll_when_last_line_visible() {
    // When the last content line is visible, cursor should descend
    // to it without the view scrolling (no bottom margin near EOF).
    let mut app = build_scroll_app(40, 20, 5);
    let last = app.max_cursor_line(); // 42

    // Position so last line is visible at viewport bottom: scroll=23, shows lines 23-42
    app.diff_state.scroll_offset = last.saturating_sub(19); // 23
    app.diff_state.cursor_line = last - 5; // 37, viewport position 14

    // Descend toward the last line — scroll should not change
    for i in 0..5 {
        let scroll_before = app.diff_state.scroll_offset;
        app.cursor_down(1);
        assert_eq!(
            app.diff_state.scroll_offset, scroll_before,
            "scroll should not change on step {i} (cursor near EOF with last line visible)"
        );
    }
    assert_eq!(app.diff_state.cursor_line, last);
}

#[test]
fn cursor_cannot_go_past_last_content_line() {
    let mut app = build_scroll_app(40, 20, 5);
    let last = app.max_cursor_line();
    let total = app.total_lines();

    // max_cursor should be strictly less than total_lines - 1
    // (total-1 is the trailing Spacing line)
    assert_eq!(last, total - 2);

    // cursor_down from last line should not advance
    app.diff_state.cursor_line = last;
    app.cursor_down(1);
    assert_eq!(app.diff_state.cursor_line, last);
}

#[test]
fn effective_scroll_margin_prevents_oscillation() {
    // With viewport 21 (odd), margin should be at most 9 (= 21/2 - 1 = 9)
    // so that after centering at position 10 (= 21/2), there's free space
    let state = DiffState {
        visible_line_count: 21,
        viewport_height: 21,
        ..DiffState::default()
    };
    let margin = state.effective_scroll_margin(100);
    assert!(
        margin < 21 / 2,
        "margin ({margin}) must be strictly less than half viewport ({})",
        21 / 2
    );
}

#[test]
fn scroll_offset_zero_means_no_margin() {
    // When scroll_offset is 0, effective margin should be 0 (no margin at file start)
    let state = DiffState {
        visible_line_count: 20,
        viewport_height: 20,
        ..DiffState::default()
    };
    let margin = state.effective_scroll_margin(0);
    assert_eq!(margin, 0, "margin should be 0 when scroll_offset is 0");
}

#[test]
fn scroll_offset_for_rows_above_wrap_off_matches_saturating_sub() {
    let mut app = build_scroll_app(40, 20, 5);
    app.diff_state.wrap_lines = false;
    for anchor in [0usize, 1, 5, 20, 43] {
        for n in [0usize, 1, 3, 10, 100] {
            assert_eq!(
                app.scroll_offset_for_rows_above(anchor, n),
                anchor.saturating_sub(n),
                "anchor={anchor} n={n}"
            );
        }
    }
}

/// Build a scroll app whose diff lines are long enough that each wraps
/// to multiple visual rows at the given viewport_width.
fn build_wrapping_scroll_app(n: usize, viewport: usize, viewport_width: usize) -> App {
    let contents: Vec<String> = (0..n).map(|_| "L".repeat(viewport_width * 4)).collect();
    build_wrapping_scroll_app_with_contents(&contents, viewport, viewport_width)
}

/// Variant that lets callers set per-line diff content, so heterogeneous
/// heights can be arranged without duplicating the fixture.
fn build_wrapping_scroll_app_with_contents(
    contents: &[String],
    viewport: usize,
    viewport_width: usize,
) -> App {
    let mut app = build_scroll_app(contents.len(), viewport, 0);
    for (line, content) in app.diff_files[0].hunks[0]
        .lines
        .iter_mut()
        .zip(contents.iter())
    {
        line.content = content.clone();
    }
    app.diff_state.wrap_lines = true;
    app.rebuild_annotations();
    app.sync_viewport_width(viewport_width);
    app
}

#[test]
fn center_cursor_wrap_on_walks_maximally() {
    let mut app = build_wrapping_scroll_app(40, 20, 20);
    let cursor = 30usize;
    app.diff_state.cursor_line = cursor;
    app.center_cursor();

    let scroll = app.diff_state.scroll_offset;
    let budget = app.diff_state.viewport_height / 2;
    let sum: usize = (scroll..cursor)
        .map(|k| annotation_row_height(&app, k))
        .sum();
    assert!(
        sum <= budget,
        "sum of heights [{scroll}..{cursor}) = {sum} must be <= budget {budget}"
    );
    let max_scroll = app.max_scroll_offset();
    if scroll > 0 && scroll <= max_scroll {
        let next = sum + annotation_row_height(&app, scroll - 1);
        assert!(
            next > budget,
            "walk should be maximal: adding row {} (height {}) yields {next}, budget {budget}",
            scroll - 1,
            annotation_row_height(&app, scroll - 1)
        );
    }
}

#[test]
fn cursor_to_bottom_wrap_on_walks_maximally() {
    let mut app = build_wrapping_scroll_app(40, 20, 20);
    let cursor = 30usize;
    app.diff_state.cursor_line = cursor;
    let viewport = app.diff_state.viewport_height;
    let margin = app.diff_state.effective_scroll_margin(app.scroll_offset);
    let cursor_h = annotation_row_height(&app, cursor);
    let budget = viewport.saturating_sub(margin + cursor_h);

    app.cursor_to_bottom();

    let scroll = app.diff_state.scroll_offset;
    let sum: usize = (scroll..cursor)
        .map(|k| annotation_row_height(&app, k))
        .sum();
    let total = sum + cursor_h;
    assert!(
        total + margin <= viewport,
        "sum over [{scroll}..={cursor}] = {total} plus margin {margin} must be <= viewport {viewport}"
    );
    let max_scroll = app.max_scroll_offset();
    if scroll > 0 && scroll <= max_scroll {
        let next = sum + annotation_row_height(&app, scroll - 1);
        assert!(
            next > budget,
            "walk should be maximal: adding row {} (height {}) yields {next}, budget {budget}",
            scroll - 1,
            annotation_row_height(&app, scroll - 1)
        );
    }
}

#[test]
fn page_lines_wrap_off_equals_row_budget() {
    let mut app = build_scroll_app(40, 20, 0);
    app.diff_state.wrap_lines = false;
    app.diff_state.cursor_line = 20;
    assert_eq!(app.page_lines_down(5), 5);
    assert_eq!(app.page_lines_up(5), 5);
}

#[test]
fn page_lines_down_wrap_on_counts_visual_rows() {
    let mut app = build_wrapping_scroll_app(40, 20, 20);
    let cursor = 5usize;
    app.diff_state.cursor_line = cursor;
    let budget = 10usize;
    let count = app.page_lines_down(budget);
    assert!(count >= 1);
    let sum: usize = (cursor + 1..=cursor + count)
        .map(|k| annotation_row_height(&app, k))
        .sum();
    assert!(
        sum <= budget,
        "sum of heights {sum} must be <= budget {budget}"
    );
    let next_idx = cursor + count + 1;
    if next_idx < app.line_annotations.len() {
        let next_h = annotation_row_height(&app, next_idx);
        assert!(
            sum + next_h > budget,
            "walk should be maximal: adding next height {next_h} to {sum} would fit in {budget}"
        );
    }
}

#[test]
fn scroll_offset_for_rows_above_wrap_on_heterogeneous_heights() {
    // Alternating short (1-row) and long (3-row) diff lines at
    // viewport_width=20. Prefix width for a DiffLine at lw=1 is 5
    // (indicator + "n " + prefix + " "). Short content "x" -> 6 chars
    // total -> 1 row. Long content "L"*36 -> 41 chars total -> 3 rows.
    let contents: Vec<String> = (0..8)
        .map(|i| {
            if i % 2 == 0 {
                "x".to_string()
            } else {
                "L".repeat(36)
            }
        })
        .collect();
    let app = build_wrapping_scroll_app_with_contents(&contents, 20, 20);

    // Locate the first DiffLine annotation and pin the heights so the
    // test is self-documenting. Indices [first_diff..first_diff+8]
    // correspond to contents[0..8], alternating 1, 3, 1, 3, ...
    let first_diff = app
        .line_annotations
        .iter()
        .position(|a| matches!(a, AnnotatedLine::DiffLine { .. }))
        .expect("a diff line annotation must exist");
    for i in 0..8 {
        let expected = if i % 2 == 0 { 1 } else { 3 };
        let got = annotation_row_height(&app, first_diff + i);
        assert_eq!(
            got,
            expected,
            "height mismatch at content {i} (annotation {}): got {got}, want {expected}",
            first_diff + i
        );
    }

    // Anchor at content index 6 (short, height 1). Walk backwards:
    //   k = anchor-1 (idx 5, long,  h=3): acc=3 <=5, keep
    //   k = anchor-2 (idx 4, short, h=1): acc=4 <=5, keep
    //   k = anchor-3 (idx 3, long,  h=3): acc=7  >5, break
    // Expected offset = anchor-2.
    //
    // Off-by-one bug (reading height(k+1) or height(k-1) instead of k)
    // would misroute the walk at k=anchor-1, yielding anchor-3.
    let anchor = first_diff + 6;
    let budget = 5usize;
    assert_eq!(
        app.scroll_offset_for_rows_above(anchor, budget),
        anchor - 2,
        "walk must consume heights[anchor-1..anchor] correctly"
    );
}

#[test]
fn page_lines_return_at_least_one() {
    let mut app = build_scroll_app(40, 20, 0);
    app.diff_state.wrap_lines = false;
    app.diff_state.cursor_line = 10;
    assert_eq!(app.page_lines_down(0), 1);
    assert_eq!(app.page_lines_up(0), 1);

    let last = app.line_annotations.len().saturating_sub(1);
    app.diff_state.cursor_line = last;
    assert_eq!(app.page_lines_down(10), 1);
}

#[test]
fn jump_to_bottom_wrap_on_keeps_last_line_fully_visible() {
    let mut app = build_wrapping_scroll_app(40, 20, 20);
    app.jump_to_bottom();

    let scroll = app.diff_state.scroll_offset;
    let max_line = app.max_cursor_line();
    let viewport = app.diff_state.viewport_height;

    let total: usize = (scroll..=max_line)
        .map(|k| annotation_row_height(&app, k))
        .sum();
    assert!(
        total <= viewport,
        "rows over [{scroll}..={max_line}] = {total} must fit viewport {viewport}"
    );
    assert_eq!(app.diff_state.cursor_line, max_line);
    if scroll > 0 {
        let with_prev = total + annotation_row_height(&app, scroll - 1);
        assert!(
            with_prev > viewport,
            "walk should be maximal: adding row {} yields {with_prev}, viewport {viewport}",
            scroll - 1
        );
    }
}

#[test]
fn move_cursor_to_annotation_wrap_on_scrolls_target_fully_into_view() {
    let mut app = build_wrapping_scroll_app(40, 20, 20);
    app.diff_state.scroll_offset = 0;
    let target = app.max_cursor_line();

    app.move_cursor_to_annotation(target);

    let scroll = app.diff_state.scroll_offset;
    let viewport = app.diff_state.viewport_height;
    let total: usize = (scroll..=target)
        .map(|k| annotation_row_height(&app, k))
        .sum();
    assert!(
        total <= viewport,
        "target must be fully visible: rows over [{scroll}..={target}] = {total}, viewport {viewport}"
    );
    assert_eq!(app.diff_state.cursor_line, target);

    let scroll_before = app.diff_state.scroll_offset;
    app.move_cursor_to_annotation(target);
    assert_eq!(
        app.diff_state.scroll_offset, scroll_before,
        "already-visible target should not change scroll_offset"
    );
}
