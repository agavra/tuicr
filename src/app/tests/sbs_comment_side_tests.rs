//! Tests for choosing the comment side in side-by-side view: `<leader>h` /
//! `<leader>l` walk the panes and set `cursor_side`, and `get_line_at_cursor`
//! resolves that preference (clamped to the sides a line actually has). The
//! plain horizontal keys keep scrolling in every view.

use crate::app::*;
use crate::input::Action;
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

fn build_app() -> App {
    let vcs_info = VcsInfo {
        root_path: PathBuf::from("/tmp"),
        head_commit: "head".to_string(),
        branch_name: Some("main".to_string()),
        vcs_type: VcsType::Git,
    };
    let session = ReviewSession::new(
        vcs_info.root_path.clone(),
        vcs_info.head_commit.clone(),
        vcs_info.branch_name.clone(),
        SessionDiffSource::WorkingTree,
    );
    App::build(
        Box::new(DummyVcs {
            info: vcs_info.clone(),
        }),
        vcs_info,
        Theme::dark(),
        None,
        false,
        Vec::new(),
        session,
        DiffSource::WorkingTree,
        InputMode::Normal,
        Vec::new(),
        None,
        None,
    )
    .expect("failed to build test app")
}

fn sbs_line(old_lineno: Option<u32>, new_lineno: Option<u32>) -> AnnotatedLine {
    AnnotatedLine::SideBySideLine {
        file_idx: 0,
        hunk_idx: 0,
        del_line_idx: old_lineno.map(|_| 0),
        add_line_idx: new_lineno.map(|_| 0),
        old_lineno,
        new_lineno,
    }
}

#[test]
fn context_line_honors_cursor_side_in_side_by_side() {
    let mut app = build_app();
    app.diff_view_mode = DiffViewMode::SideBySide;
    app.line_annotations = vec![sbs_line(Some(10), Some(20))];
    app.diff_state.cursor_line = 0;

    app.cursor_side = LineSide::New;
    assert_eq!(app.get_line_at_cursor(), Some((20, LineSide::New)));

    app.cursor_side = LineSide::Old;
    assert_eq!(app.get_line_at_cursor(), Some((10, LineSide::Old)));
}

#[test]
fn addition_line_clamps_to_new_even_when_old_preferred() {
    let mut app = build_app();
    app.diff_view_mode = DiffViewMode::SideBySide;
    app.line_annotations = vec![sbs_line(None, Some(20))];
    app.diff_state.cursor_line = 0;

    app.cursor_side = LineSide::Old; // no old side on this line
    assert_eq!(app.get_line_at_cursor(), Some((20, LineSide::New)));
}

#[test]
fn deletion_line_clamps_to_old_even_when_new_preferred() {
    let mut app = build_app();
    app.diff_view_mode = DiffViewMode::SideBySide;
    app.line_annotations = vec![sbs_line(Some(10), None)];
    app.diff_state.cursor_line = 0;

    app.cursor_side = LineSide::New; // no new side on this line
    assert_eq!(app.get_line_at_cursor(), Some((10, LineSide::Old)));
}

#[test]
fn unified_view_ignores_cursor_side_and_prefers_new() {
    let mut app = build_app();
    app.diff_view_mode = DiffViewMode::Unified;
    app.line_annotations = vec![sbs_line(Some(10), Some(20))];
    app.diff_state.cursor_line = 0;

    app.cursor_side = LineSide::Old; // ignored in unified
    assert_eq!(app.effective_cursor_side(), LineSide::New);
    assert_eq!(app.get_line_at_cursor(), Some((20, LineSide::New)));
}

#[test]
fn horizontal_keys_scroll_in_side_by_side_and_leave_side_unchanged() {
    let mut app = build_app();
    app.diff_view_mode = DiffViewMode::SideBySide;
    app.cursor_side = LineSide::New;
    app.diff_state.wrap_lines = false;
    app.diff_state.max_content_width = 200;
    app.diff_state.viewport_width = 80;

    // Reviewers expect the horizontal keys to scroll in every view; the side is
    // switched with the `<leader>` panel walk instead.
    crate::handler::handle_diff_action(&mut app, Action::ScrollRight(4));
    assert_eq!(app.diff_state.scroll_x, 4);
    assert_eq!(app.cursor_side, LineSide::New);

    crate::handler::handle_diff_action(&mut app, Action::ScrollLeft(4));
    assert_eq!(app.diff_state.scroll_x, 0);
    assert_eq!(app.cursor_side, LineSide::New);
}

#[test]
fn leader_walk_steps_through_the_side_by_side_panes() {
    let mut app = build_app();
    app.diff_view_mode = DiffViewMode::SideBySide;
    app.show_file_list = true;
    app.focused_panel = FocusedPanel::Diff;
    app.cursor_side = LineSide::New;

    // new -> old inside the diff, then out to the file list.
    app.focus_pane_left();
    assert_eq!(app.focused_panel, FocusedPanel::Diff);
    assert_eq!(app.cursor_side, LineSide::Old);

    app.focus_pane_left();
    assert_eq!(app.focused_panel, FocusedPanel::FileList);
    assert_eq!(app.cursor_side, LineSide::Old);

    // ... and back: file list -> diff (side kept) -> new.
    app.focus_pane_right();
    assert_eq!(app.focused_panel, FocusedPanel::Diff);
    assert_eq!(app.cursor_side, LineSide::Old);

    app.focus_pane_right();
    assert_eq!(app.cursor_side, LineSide::New);

    // Already rightmost: stays put.
    app.focus_pane_right();
    assert_eq!(app.focused_panel, FocusedPanel::Diff);
    assert_eq!(app.cursor_side, LineSide::New);
}

#[test]
fn leader_walk_ignores_the_sides_in_unified_view() {
    let mut app = build_app();
    app.diff_view_mode = DiffViewMode::Unified;
    app.show_file_list = true;
    app.focused_panel = FocusedPanel::Diff;
    app.cursor_side = LineSide::New;

    // Unified has a single pane, so the walk leaves the diff immediately.
    app.focus_pane_left();
    assert_eq!(app.focused_panel, FocusedPanel::FileList);
    assert_eq!(app.cursor_side, LineSide::New);

    app.focus_pane_right();
    assert_eq!(app.focused_panel, FocusedPanel::Diff);
    assert_eq!(app.cursor_side, LineSide::New);
}

#[test]
fn horizontal_keys_scroll_in_unified_and_leave_side_unchanged() {
    let mut app = build_app();
    app.diff_view_mode = DiffViewMode::Unified;
    app.cursor_side = LineSide::New;
    app.diff_state.wrap_lines = false;
    app.diff_state.max_content_width = 100;
    app.diff_state.viewport_width = 10;

    crate::handler::handle_diff_action(&mut app, Action::ScrollRight(4));
    assert_eq!(app.diff_state.scroll_x, 4);
    assert_eq!(app.cursor_side, LineSide::New);
}
