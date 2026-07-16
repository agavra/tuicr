use crate::app::*;
use crate::model::{DiffFile, DiffHunk, DiffLine, FileStatus, LineOrigin};
use crate::vcs::traits::{VcsBackend, VcsInfo, VcsType};
use std::fs;
use std::path::PathBuf;

struct StubVcs(VcsInfo);
impl VcsBackend for StubVcs {
    fn info(&self) -> &VcsInfo {
        &self.0
    }
    fn get_working_tree_diff(
        &self,
        _hl: &crate::syntax::SyntaxHighlighter,
    ) -> crate::error::Result<Vec<DiffFile>> {
        Ok(Vec::new())
    }
    fn fetch_context_lines(
        &self,
        _path: &std::path::Path,
        _status: FileStatus,
        _ref_commit: Option<&str>,
        _start: u32,
        _end: u32,
    ) -> crate::error::Result<Vec<DiffLine>> {
        Ok(Vec::new())
    }
    fn file_line_count(
        &self,
        _path: &std::path::Path,
        _status: FileStatus,
        _ref_commit: Option<&str>,
    ) -> crate::error::Result<u32> {
        Ok(0)
    }
}

fn hunk(start: u32, count: u32) -> DiffHunk {
    let lines = (0..count)
        .map(|i| DiffLine {
            origin: LineOrigin::Context,
            content: format!("line {}", start + i),
            old_lineno: Some(start + i),
            new_lineno: Some(start + i),
            highlighted_spans: None,
        })
        .collect();
    DiffHunk {
        header: format!("@@ -{start},{count} +{start},{count} @@"),
        lines,
        old_start: start,
        old_count: count,
        new_start: start,
        new_count: count,
    }
}

fn file(path: &str, hunks: Vec<DiffHunk>) -> DiffFile {
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

fn app_with(files: Vec<DiffFile>) -> App {
    app_with_root(PathBuf::from("/tmp"), files)
}

fn app_with_root(root_path: PathBuf, files: Vec<DiffFile>) -> App {
    let vcs_info = VcsInfo {
        root_path,
        head_commit: "head".into(),
        branch_name: Some("main".into()),
        vcs_type: VcsType::Git,
    };
    let session = ReviewSession::new(
        vcs_info.root_path.clone(),
        vcs_info.head_commit.clone(),
        vcs_info.branch_name.clone(),
        SessionDiffSource::WorkingTree,
    );
    App::build(
        Box::new(StubVcs(vcs_info.clone())),
        vcs_info,
        crate::theme::Theme::dark(),
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
    .expect("build app")
}

#[test]
fn annotation_ranges_index_each_file() {
    let app = app_with(vec![
        file("a.rs", vec![hunk(1, 2)]),
        file("b.rs", vec![hunk(10, 3)]),
    ]);

    let first = app.file_annotation_range(0).unwrap();
    let second = app.file_annotation_range(1).unwrap();
    assert_eq!(first.start, 1);
    assert_eq!(first.end, second.start);
    assert_eq!(second.end, app.line_annotations.len());
}

#[test]
fn editor_target_uses_selected_file_list_row() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("main.rs");
    fs::write(&path, "fn main() {}\n").expect("write file");

    let mut app = app_with_root(
        dir.path().to_path_buf(),
        vec![file("main.rs", vec![hunk(1, 1)])],
    );
    app.focused_panel = FocusedPanel::FileList;
    app.queue_editor_for_focused_item();

    let target = app.take_pending_editor_target().expect("editor target");
    assert_eq!(target.path, path);
    assert_eq!(target.line, None);
}

#[test]
fn edit_command_uses_selected_file_list_row() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("main.rs");
    fs::write(&path, "fn main() {}\n").expect("write file");

    let mut app = app_with_root(
        dir.path().to_path_buf(),
        vec![file("main.rs", vec![hunk(1, 1)])],
    );
    app.focused_panel = FocusedPanel::FileList;
    app.enter_command_mode();
    app.command_buffer = "edit".to_string();

    crate::handler::handle_command_action(&mut app, crate::input::Action::SubmitInput);

    let target = app.take_pending_editor_target().expect("editor target");
    assert_eq!(target.path, path);
    assert_eq!(target.line, None);
    assert_eq!(app.input_mode, InputMode::Normal);
}

#[test]
fn editor_target_uses_diff_cursor_line() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("main.rs");
    fs::write(&path, "line 1\nline 2\nline 3\n").expect("write file");

    let mut app = app_with_root(
        dir.path().to_path_buf(),
        vec![file("main.rs", vec![hunk(1, 3)])],
    );
    app.focused_panel = FocusedPanel::Diff;
    app.diff_state.cursor_line = app
        .line_annotations
        .iter()
        .position(|annotation| {
            matches!(
                annotation,
                AnnotatedLine::DiffLine {
                    new_lineno: Some(2),
                    ..
                }
            )
        })
        .expect("diff line annotation");
    app.queue_editor_for_focused_item();

    let target = app.take_pending_editor_target().expect("editor target");
    assert_eq!(target.path, path);
    assert_eq!(target.line, Some(2));
}

#[test]
fn editor_target_warns_for_missing_local_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut app = app_with_root(
        dir.path().to_path_buf(),
        vec![file("missing.rs", vec![hunk(1, 1)])],
    );
    app.focused_panel = FocusedPanel::Diff;

    app.queue_editor_for_focused_item();

    assert!(app.take_pending_editor_target().is_none());
    assert!(
        app.message
            .as_ref()
            .expect("warning")
            .content
            .contains("file does not exist")
    );
}

struct FakeForgeBackend {
    local_checkout: Option<PathBuf>,
}

impl crate::forge::traits::ForgeBackend for FakeForgeBackend {
    fn list_pull_requests(
        &self,
        _query: crate::forge::traits::PullRequestListQuery,
    ) -> crate::error::Result<crate::forge::traits::PagedPullRequests> {
        unimplemented!()
    }
    fn get_pull_request(
        &self,
        _target: crate::forge::traits::PullRequestTarget,
    ) -> crate::error::Result<crate::forge::traits::PullRequestDetails> {
        unimplemented!()
    }
    fn get_pull_request_diff(
        &self,
        _pr: &crate::forge::traits::PullRequestDetails,
    ) -> crate::error::Result<String> {
        unimplemented!()
    }
    fn fetch_file_lines(
        &self,
        _request: crate::forge::traits::ForgeFileLinesRequest,
    ) -> crate::error::Result<Vec<DiffLine>> {
        unimplemented!()
    }
    fn list_review_threads(
        &self,
        _pr: &crate::forge::traits::PullRequestDetails,
    ) -> crate::error::Result<Vec<crate::forge::remote_comments::RemoteReviewThread>> {
        unimplemented!()
    }
    fn list_pull_request_commits(
        &self,
        _pr: &crate::forge::traits::PullRequestDetails,
    ) -> crate::error::Result<Vec<crate::forge::traits::PullRequestCommit>> {
        unimplemented!()
    }
    fn get_pull_request_commit_range_diff(
        &self,
        _pr: &crate::forge::traits::PullRequestDetails,
        _start_sha: &str,
        _end_sha: &str,
    ) -> crate::error::Result<String> {
        unimplemented!()
    }
    fn create_review(
        &self,
        _pr: &crate::forge::traits::PullRequestDetails,
        _request: crate::forge::traits::CreateReviewRequest<'_>,
    ) -> crate::error::Result<crate::forge::traits::GhCreateReviewResponse> {
        unimplemented!()
    }
    fn local_checkout_path(&self) -> Option<PathBuf> {
        self.local_checkout.clone()
    }
}

#[test]
fn editor_target_falls_back_to_forge_backend_checkout_in_pr_mode() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("main.rs");
    fs::write(&path, "fn main() {}\n").expect("write file");

    let mut app = app_with_root(
        PathBuf::from("forge:github.com/agavra/tuicr"),
        vec![file("main.rs", vec![hunk(1, 1)])],
    );
    app.forge_backend = Some(Box::new(FakeForgeBackend {
        local_checkout: Some(dir.path().to_path_buf()),
    }));
    app.focused_panel = FocusedPanel::FileList;

    app.queue_editor_for_focused_item();

    let target = app.take_pending_editor_target().expect("editor target");
    assert_eq!(target.path, path);
}

#[test]
fn editor_target_warns_when_pr_mode_has_no_matching_checkout() {
    let mut app = app_with_root(
        PathBuf::from("forge:github.com/agavra/tuicr"),
        vec![file("main.rs", vec![hunk(1, 1)])],
    );
    app.forge_backend = Some(Box::new(FakeForgeBackend {
        local_checkout: None,
    }));
    app.focused_panel = FocusedPanel::FileList;

    app.queue_editor_for_focused_item();

    assert!(app.take_pending_editor_target().is_none());
    assert!(
        app.message
            .as_ref()
            .expect("warning")
            .content
            .contains("no local checkout")
    );
}

#[test]
fn toggle_preserves_file_position() {
    let files = vec![
        file("a.rs", vec![hunk(1, 3)]),
        file("b.rs", vec![hunk(1, 3)]),
        file("c.rs", vec![hunk(1, 3)]),
    ];
    let mut app = app_with(files);
    app.diff_state.current_file_idx = 1;
    let expected_multi = app.calculate_file_scroll_offset(1);
    app.diff_state.scroll_offset = expected_multi;

    app.toggle_single_file_view();
    assert!(app.is_single_file_view);
    let expected_single = app.calculate_file_scroll_offset(1);
    assert_eq!(app.diff_state.scroll_offset, expected_single);
    assert_eq!(app.diff_state.cursor_line, expected_single);

    app.toggle_single_file_view();
    assert!(!app.is_single_file_view);
    let expected_back = app.calculate_file_scroll_offset(1);
    assert_eq!(app.diff_state.scroll_offset, expected_back);
    assert_eq!(app.diff_state.cursor_line, expected_back);
}

#[test]
fn cursor_down_requires_two_presses_to_walk_to_next_file() {
    let files = vec![
        file("a.rs", vec![hunk(1, 3)]),
        file("b.rs", vec![hunk(1, 3)]),
    ];
    let mut app = app_with(files);
    app.is_single_file_view = true;
    app.diff_state.current_file_idx = 0;
    app.diff_state.cursor_line = app.max_cursor_line();
    let max_a = app.max_cursor_line();

    // First press at file end arms primed_walk_next and stays on max.
    app.cursor_down(1);
    assert_eq!(app.diff_state.current_file_idx, 0);
    assert_eq!(app.diff_state.cursor_line, max_a);
    assert!(app.primed_walk_next);

    // Second press consumes the prime and walks.
    app.cursor_down(1);
    assert_eq!(app.diff_state.current_file_idx, 1);
    assert!(!app.primed_walk_next);
}

#[test]
fn cursor_up_requires_two_presses_to_walk_to_prev_file() {
    let files = vec![
        file("a.rs", vec![hunk(1, 3)]),
        file("b.rs", vec![hunk(1, 3)]),
    ];
    let mut app = app_with(files);
    app.is_single_file_view = true;
    app.diff_state.current_file_idx = 1;
    let file_top = app.calculate_file_scroll_offset(1);
    app.diff_state.cursor_line = file_top;

    // First press at file top arms primed_walk_prev and stays.
    app.cursor_up(1);
    assert_eq!(app.diff_state.current_file_idx, 1);
    assert_eq!(app.diff_state.cursor_line, file_top);
    assert!(app.primed_walk_prev);

    // Second press walks to the previous file.
    app.cursor_up(1);
    assert_eq!(app.diff_state.current_file_idx, 0);
    assert!(!app.primed_walk_prev);
}

#[test]
fn primed_walk_clears_on_non_overflow_cursor_move() {
    let files = vec![
        file("a.rs", vec![hunk(1, 5)]),
        file("b.rs", vec![hunk(1, 5)]),
    ];
    let mut app = app_with(files);
    app.is_single_file_view = true;
    app.diff_state.current_file_idx = 0;
    app.diff_state.cursor_line = app.max_cursor_line();
    app.cursor_down(1); // arms
    assert!(app.primed_walk_next);

    // A non-overflow move (cursor up within file) clears the next prime.
    app.cursor_up(1);
    assert!(!app.primed_walk_next);
    assert_eq!(app.diff_state.current_file_idx, 0);
}

#[test]
fn next_hunk_crosses_into_next_file_in_single_file_view() {
    let files = vec![
        file("a.rs", vec![hunk(1, 3), hunk(10, 3)]),
        file("b.rs", vec![hunk(1, 3), hunk(10, 3)]),
    ];
    let mut app = app_with(files);
    app.is_single_file_view = true;
    app.diff_state.current_file_idx = 0;
    app.rebuild_annotations();
    let positions = app.hunk_positions();
    let last_hunk = *positions.last().expect("a.rs has two hunks");
    app.diff_state.cursor_line = last_hunk;

    // From a.rs's last hunk, `]` should land on b.rs's first hunk.
    app.next_hunk();
    assert_eq!(app.diff_state.current_file_idx, 1);
    let new_first = *app.hunk_positions().first().expect("b.rs has hunks");
    assert_eq!(app.diff_state.cursor_line, new_first);
}

#[test]
fn prev_hunk_crosses_into_prev_file_in_single_file_view() {
    let files = vec![
        file("a.rs", vec![hunk(1, 3), hunk(10, 3)]),
        file("b.rs", vec![hunk(1, 3), hunk(10, 3)]),
    ];
    let mut app = app_with(files);
    app.is_single_file_view = true;
    app.diff_state.current_file_idx = 1;
    app.rebuild_annotations();
    let first_hunk = *app.hunk_positions().first().expect("b.rs has hunks");
    app.diff_state.cursor_line = first_hunk;

    // From b.rs's first hunk, `[` should land on a.rs's last hunk.
    app.prev_hunk();
    assert_eq!(app.diff_state.current_file_idx, 0);
    let new_last = *app.hunk_positions().last().expect("a.rs has hunks");
    assert_eq!(app.diff_state.cursor_line, new_last);
}

#[test]
fn held_key_does_not_walk_when_keyboard_enhancement_supported() {
    // Simulates kitty REPORT_EVENT_TYPES: held-j auto-repeats arm the
    // prime but the release flag never trips, so consecutive
    // cursor_down calls park on max forever.
    let files = vec![
        file("a.rs", vec![hunk(1, 3)]),
        file("b.rs", vec![hunk(1, 3)]),
    ];
    let mut app = app_with(files);
    app.supports_keyboard_enhancement = true;
    app.is_single_file_view = true;
    app.diff_state.current_file_idx = 0;
    app.diff_state.cursor_line = app.max_cursor_line();
    let max_a = app.max_cursor_line();

    // 10 consecutive presses (no release) stay parked on max.
    for _ in 0..10 {
        app.cursor_down(1);
    }
    assert_eq!(app.diff_state.current_file_idx, 0);
    assert_eq!(app.diff_state.cursor_line, max_a);
    assert!(app.primed_walk_next);
    assert!(!app.down_released_since_arm);
}

#[test]
fn release_then_press_walks_when_keyboard_enhancement_supported() {
    let files = vec![
        file("a.rs", vec![hunk(1, 3)]),
        file("b.rs", vec![hunk(1, 3)]),
    ];
    let mut app = app_with(files);
    app.supports_keyboard_enhancement = true;
    app.is_single_file_view = true;
    app.diff_state.current_file_idx = 0;
    app.diff_state.cursor_line = app.max_cursor_line();

    // First press: arm.
    app.cursor_down(1);
    assert!(app.primed_walk_next);
    assert_eq!(app.diff_state.current_file_idx, 0);

    // Simulate a Down-Release event from the main loop.
    app.down_released_since_arm = true;

    // Second press: walks.
    app.cursor_down(1);
    assert_eq!(app.diff_state.current_file_idx, 1);
    assert!(!app.primed_walk_next);
    assert!(!app.down_released_since_arm);
}

#[test]
fn effective_file_height_is_zero_for_non_current_in_single_file_view() {
    let files = vec![
        file("a.rs", vec![hunk(1, 3)]),
        file("b.rs", vec![hunk(1, 3)]),
    ];
    let mut app = app_with(files);
    app.is_single_file_view = true;
    app.diff_state.current_file_idx = 0;
    let other = &app.diff_files[1].clone();
    assert_eq!(app.effective_file_height(1, other), 0);
    let current = &app.diff_files[0].clone();
    assert!(app.effective_file_height(0, current) > 0);
}
