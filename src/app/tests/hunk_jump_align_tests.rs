use crate::app::*;
use crate::model::{DiffHunk, DiffLine, FileStatus, LineOrigin};
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

fn make_hunk(new_start: u32, new_count: u32) -> DiffHunk {
    let lines = (0..new_count)
        .map(|i| DiffLine {
            origin: LineOrigin::Context,
            content: format!("hunk line {}", new_start + i),
            old_lineno: Some(new_start + i),
            new_lineno: Some(new_start + i),
            raw_content: None,
            highlighted_spans: None,
            stage: None,
        })
        .collect();
    DiffHunk {
        header: format!("@@ -{new_start},{new_count} +{new_start},{new_count} @@"),
        lines,
        old_start: new_start,
        old_count: new_count,
        new_start,
        new_count,
    }
}

fn make_file(path: &str, hunks: Vec<DiffHunk>) -> DiffFile {
    let content_hash = DiffFile::compute_content_hash(&hunks);
    DiffFile {
        old_path: None,
        new_path: Some(PathBuf::from(path)),
        status: FileStatus::Modified,
        hunks,
        is_binary: false,
        is_too_large: false,
        is_commit_message: false,
        content_hash,
    }
}

/// Viewport 20: enough room above both hunks for `top`/`center` to move.
fn build_app(files: Vec<DiffFile>, scroll_offset: usize) -> App {
    let vcs_info = VcsInfo {
        root_path: PathBuf::from("/tmp"),
        head_commit: "abc123".to_string(),
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
        files,
        session,
        DiffSource::WorkingTree,
        InputMode::Normal,
        Vec::new(),
        None,
        None,
    )
    .expect("failed to build test app");
    app.diff_state.viewport_height = 20;
    app.diff_state.visible_line_count = 20;
    app.scroll_offset = scroll_offset;
    app
}

/// Two distant hunks. The first is taller than the 20-line viewport, so the
/// second starts off screen, and the second is itself taller than the viewport.
fn two_hunk_app(align: HunkJumpAlign, scroll_offset: usize) -> App {
    let file = make_file("test.rs", vec![make_hunk(1, 100), make_hunk(300, 40)]);
    let mut app = build_app(vec![file], scroll_offset);
    app.hunk_jump_align = align;
    app
}

#[test]
fn visible_alignment_leaves_the_jumped_to_header_at_the_bottom_edge() {
    let mut app = two_hunk_app(HunkJumpAlign::Visible, 0);
    let headers = app.hunk_positions();
    app.diff_state.cursor_line = headers[0];
    app.next_hunk();

    // Historical behaviour: scroll only until the header is visible, i.e. the
    // last row of the viewport, with the hunk body below it.
    assert_eq!(app.diff_state.cursor_line, headers[1]);
    assert_eq!(app.diff_state.scroll_offset, headers[1] - 19);
}

#[test]
fn top_alignment_puts_the_jumped_to_header_at_the_top() {
    let mut app = two_hunk_app(HunkJumpAlign::Top, 0);
    let headers = app.hunk_positions();
    app.diff_state.cursor_line = headers[0];
    app.next_hunk();

    assert_eq!(app.diff_state.cursor_line, headers[1]);
    assert_eq!(app.diff_state.scroll_offset, headers[1]);
}

#[test]
fn top_alignment_respects_the_scroll_offset_margin() {
    let mut app = two_hunk_app(HunkJumpAlign::Top, 3);
    let headers = app.hunk_positions();
    app.diff_state.cursor_line = headers[0];
    app.next_hunk();

    assert_eq!(app.diff_state.scroll_offset, headers[1] - 3);
}

#[test]
fn top_alignment_clamps_at_the_start_of_the_diff() {
    let mut app = two_hunk_app(HunkJumpAlign::Top, 5);
    let headers = app.hunk_positions();
    app.diff_state.cursor_line = headers[1];
    app.prev_hunk();

    // Fewer rows above the first hunk than the margin: the viewport stays at
    // the top instead of scrolling past it.
    assert_eq!(app.diff_state.cursor_line, headers[0]);
    assert_eq!(app.diff_state.scroll_offset, 0);
}

#[test]
fn center_alignment_centers_the_jumped_to_header() {
    let mut app = two_hunk_app(HunkJumpAlign::Center, 0);
    let headers = app.hunk_positions();
    app.diff_state.cursor_line = headers[0];
    app.next_hunk();

    assert_eq!(app.diff_state.cursor_line, headers[1]);
    assert_eq!(app.diff_state.scroll_offset, headers[1] - 10);
}

#[test]
fn center_alignment_clamps_at_the_start_of_the_diff() {
    let mut app = two_hunk_app(HunkJumpAlign::Center, 0);
    let headers = app.hunk_positions();
    app.diff_state.cursor_line = headers[1];
    app.prev_hunk();

    assert_eq!(app.diff_state.cursor_line, headers[0]);
    assert_eq!(app.diff_state.scroll_offset, 0);
}

#[test]
fn file_jumps_follow_the_alignment_setting() {
    let files = || {
        vec![
            make_file("a.rs", vec![make_hunk(1, 30)]),
            make_file("b.rs", vec![make_hunk(1, 5)]),
            make_file("c.rs", vec![make_hunk(1, 5)]),
        ]
    };

    // `visible` keeps the new file's header at the top even with a scroll
    // margin: `jump_to_file` already put it there.
    let mut app = build_app(files(), 3);
    app.hunk_jump_align = HunkJumpAlign::Visible;
    app.next_file();
    let file_b_header = app.diff_state.cursor_line;
    assert_eq!(app.diff_state.scroll_offset, file_b_header);
    app.next_file();
    let file_c_header = app.diff_state.cursor_line;
    assert_eq!(app.diff_state.scroll_offset, file_c_header);
    app.prev_file();
    assert_eq!(app.diff_state.cursor_line, file_b_header);
    assert_eq!(app.diff_state.scroll_offset, file_b_header);

    let mut app = build_app(files(), 3);
    app.hunk_jump_align = HunkJumpAlign::Top;
    app.next_file();
    assert_eq!(app.diff_state.scroll_offset, app.diff_state.cursor_line - 3);

    let mut app = build_app(files(), 0);
    app.hunk_jump_align = HunkJumpAlign::Center;
    app.next_file();
    assert_eq!(
        app.diff_state.scroll_offset,
        app.diff_state.cursor_line - 10
    );

    // `{` walks back and aligns the same way.
    app.prev_file();
    let file_a_header = app.diff_state.cursor_line;
    assert_eq!(
        app.diff_state.scroll_offset,
        file_a_header.saturating_sub(10)
    );
}
