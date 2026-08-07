use crate::app::*;
use crate::model::{DiffFile, DiffHunk, DiffLine, FileStatus, LineOrigin};
use crate::vcs::traits::{VcsBackend, VcsInfo, VcsType};
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

fn line(origin: LineOrigin, content: &str, old: Option<u32>, new: Option<u32>) -> DiffLine {
    DiffLine {
        origin,
        content: content.to_string(),
        old_lineno: old,
        new_lineno: new,
        highlighted_spans: None,
    }
}

fn file(path: &str, contents: &[&str]) -> DiffFile {
    let lines = contents
        .iter()
        .enumerate()
        .map(|(idx, content)| line(LineOrigin::Addition, content, None, Some(idx as u32 + 1)))
        .collect::<Vec<_>>();
    let hunks = vec![DiffHunk {
        header: "@@ -0,0 +1 @@".to_string(),
        lines,
        old_start: 0,
        old_count: 0,
        new_start: 1,
        new_count: contents.len() as u32,
    }];
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
    let vcs_info = VcsInfo {
        root_path: PathBuf::from("/tmp"),
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

fn searchable_app() -> App {
    app_with(vec![file(
        "a.rs",
        &["alpha needle", "plain line", "second NEEDLE here", "tail"],
    )])
}

fn search(app: &mut App, pattern: &str) -> bool {
    app.search_buffer = pattern.to_string();
    app.search_in_diff_from_cursor()
}

fn message(app: &App) -> Option<String> {
    app.message.as_ref().map(|m| m.content.clone())
}

#[test]
fn should_collect_matches_and_move_to_first_match_at_or_after_cursor() {
    let mut app = searchable_app();

    assert!(search(&mut app, "needle"));

    assert_eq!(app.search_matches.len(), 2);
    assert!(app.search_highlight_visible);
    assert_eq!(app.diff_state.cursor_line, app.search_matches[0]);
    assert_eq!(app.search_match_position(), Some((1, 2)));
}

#[test]
fn should_match_case_insensitively() {
    let mut app = searchable_app();

    assert!(search(&mut app, "NeeDLE"));

    assert_eq!(app.search_matches.len(), 2);
}

#[test]
fn should_wrap_to_top_when_cycling_past_the_last_match() {
    let mut app = searchable_app();
    assert!(search(&mut app, "needle"));

    assert!(app.search_next_in_diff());
    assert_eq!(app.diff_state.cursor_line, app.search_matches[1]);
    assert_eq!(app.search_match_position(), Some((2, 2)));

    assert!(app.search_next_in_diff());
    assert_eq!(app.diff_state.cursor_line, app.search_matches[0]);
    assert_eq!(
        message(&app).as_deref(),
        Some("search hit BOTTOM, continuing at TOP")
    );
}

#[test]
fn should_wrap_to_bottom_when_cycling_before_the_first_match() {
    let mut app = searchable_app();
    assert!(search(&mut app, "needle"));

    assert!(app.search_prev_in_diff());

    assert_eq!(app.diff_state.cursor_line, app.search_matches[1]);
    assert_eq!(
        message(&app).as_deref(),
        Some("search hit TOP, continuing at BOTTOM")
    );
}

#[test]
fn should_report_no_matches_and_keep_cursor_still() {
    let mut app = searchable_app();
    let cursor_before = app.diff_state.cursor_line;

    assert!(!search(&mut app, "missing"));

    assert!(app.search_matches.is_empty());
    assert_eq!(app.diff_state.cursor_line, cursor_before);
    assert_eq!(app.search_match_position(), None);
    assert_eq!(message(&app).as_deref(), Some("No matches for \"missing\""));
}

#[test]
fn should_clear_highlight_but_keep_pattern_for_n() {
    let mut app = searchable_app();
    assert!(search(&mut app, "needle"));

    app.clear_search_highlight();

    assert!(!app.search_highlight_visible);
    assert_eq!(app.active_search_needle(), None);
    assert_eq!(app.search_match_position(), None);

    assert!(app.search_next_in_diff());
    assert!(app.search_highlight_visible);
    assert_eq!(app.active_search_needle(), Some("needle"));
}

#[test]
fn should_not_expose_a_needle_when_highlighting_is_disabled_in_config() {
    let mut app = searchable_app();
    app.search_highlight_enabled = false;

    assert!(search(&mut app, "needle"));

    assert_eq!(app.active_search_needle(), None);
    assert_eq!(app.search_match_position(), Some((1, 2)));
}

#[test]
fn should_recompute_matches_when_annotations_are_rebuilt() {
    let mut app = searchable_app();
    assert!(search(&mut app, "needle"));
    assert_eq!(app.search_matches.len(), 2);

    app.diff_files = vec![file("a.rs", &["only one needle left"])];
    app.rebuild_annotations();

    assert_eq!(app.search_matches.len(), 1);
}

#[test]
fn should_defer_recompute_to_n_while_highlight_is_cleared() {
    let mut app = searchable_app();
    assert!(search(&mut app, "needle"));
    app.clear_search_highlight();

    app.diff_files = vec![file("a.rs", &["only one needle left"])];
    app.rebuild_annotations();

    assert!(app.search_matches_stale);
    assert_eq!(app.search_matches.len(), 2);

    assert!(app.search_next_in_diff());
    assert!(!app.search_matches_stale);
    assert_eq!(app.search_matches.len(), 1);
}

#[test]
fn should_suppress_highlighting_while_typing_a_comment() {
    let mut app = searchable_app();
    assert!(search(&mut app, "needle"));
    assert!(app.active_search_needle().is_some());
    assert!(app.search_paint_at(app.diff_state.cursor_line).is_some());

    app.input_mode = InputMode::Comment;
    assert_eq!(app.active_search_needle(), None);
    assert_eq!(app.search_paint_at(app.diff_state.cursor_line), None);

    app.input_mode = InputMode::Normal;
    assert!(app.active_search_needle().is_some());
}

#[test]
fn should_not_leave_highlight_active_after_a_failed_search() {
    let mut app = searchable_app();
    assert!(search(&mut app, "needle"));

    assert!(!search(&mut app, "missing"));

    assert!(!app.search_highlight_visible);
    assert_eq!(app.active_search_needle(), None);
}

#[test]
fn should_clamp_counter_to_one_when_cursor_is_above_the_first_match() {
    let mut app = searchable_app();
    assert!(search(&mut app, "needle"));

    app.diff_state.cursor_line = 0;

    assert_eq!(app.search_match_position(), Some((1, 2)));
}
