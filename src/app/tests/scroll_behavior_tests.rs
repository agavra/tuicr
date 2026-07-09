use crate::app::*;
use crate::model::FileStatus;
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
