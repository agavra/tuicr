//! `c` on a comment row replies in that thread (task 02_comment_reply).

use crate::app::*;
use crate::input::keybindings::Action;
use crate::model::{
    Comment, CommentType, DiffFile, DiffHunk, DiffLine, FileStatus, LineOrigin, LineRange, LineSide,
};
use crate::vcs::traits::{VcsBackend, VcsInfo, VcsType};
use std::path::PathBuf;

#[test]
fn should_reply_to_range_comment() {
    let mut app = app_with("a.rs");
    let c = Comment::new_with_range(
        "range comment".to_string(),
        CommentType::from_id("issue"),
        Some(LineSide::New),
        LineRange::new(2, 4),
    );
    add_line_comment(&mut app, "a.rs", 4, c);
    cursor_on(&mut app, |a| matches!(a, AnnotatedLine::LineComment { .. }));

    crate::handler::handle_diff_action(&mut app, Action::AddLineComment);

    assert_eq!(app.input_mode, InputMode::Comment);
    assert!(!app.comment_is_file_level);
    assert!(!app.comment_is_review_level);
    assert_eq!(app.comment_line, Some((4, LineSide::New)));
    assert_eq!(
        app.comment_line_range,
        Some((LineRange::new(2, 4), LineSide::New))
    );
    assert_eq!(app.comment_type, app.default_comment_type());
}

#[test]
fn should_reply_to_line_comment() {
    let mut app = app_with("a.rs");
    let c = Comment::new(
        "single comment".to_string(),
        CommentType::from_id("issue"),
        Some(LineSide::New),
    );
    add_line_comment(&mut app, "a.rs", 3, c);
    app.comment_line_range = Some((LineRange::new(1, 5), LineSide::Old));
    app.editing_comment_id = Some("stale".to_string());
    cursor_on(&mut app, |a| matches!(a, AnnotatedLine::LineComment { .. }));

    crate::handler::handle_diff_action(&mut app, Action::AddLineComment);

    assert_eq!(app.comment_line, Some((3, LineSide::New)));
    assert_eq!(app.comment_line_range, None);
    assert_eq!(app.editing_comment_id, None);
    assert_eq!(app.comment_type, app.default_comment_type());
}

#[test]
fn should_reply_to_file_comment() {
    let mut app = app_with("a.rs");
    let pb = PathBuf::from("a.rs");
    let review = app.session.get_file_mut(&pb).expect("file in session");
    review.add_file_comment(Comment::new(
        "file comment".to_string(),
        CommentType::from_id("note"),
        None,
    ));
    cursor_on(&mut app, |a| matches!(a, AnnotatedLine::FileComment { .. }));
    app.update_current_file_from_cursor();

    crate::handler::handle_diff_action(&mut app, Action::AddLineComment);

    assert_eq!(app.input_mode, InputMode::Comment);
    assert!(app.comment_is_file_level);
    assert_eq!(app.comment_line, None);
    assert_eq!(app.comment_line_range, None);
    assert_eq!(app.comment_type, app.default_comment_type());
    assert_eq!(app.current_file_path(), Some(&pb));
}

#[test]
fn should_reply_to_review_comment() {
    let mut app = app_with("a.rs");
    app.session.review_comments.push(Comment::new(
        "review comment".to_string(),
        CommentType::from_id("note"),
        None,
    ));
    cursor_on(&mut app, |a| {
        matches!(a, AnnotatedLine::ReviewComment { .. })
    });

    crate::handler::handle_diff_action(&mut app, Action::AddLineComment);

    assert_eq!(app.input_mode, InputMode::Comment);
    assert!(app.comment_is_review_level);
    assert!(!app.comment_is_file_level);
    assert_eq!(app.comment_line, None);
    assert_eq!(app.comment_line_range, None);
    assert_eq!(app.comment_type, app.default_comment_type());
}

#[test]
fn should_create_comment_on_a_diff_line() {
    let mut app = app_with("a.rs");
    cursor_on(&mut app, |a| matches!(a, AnnotatedLine::DiffLine { .. }));

    crate::handler::handle_diff_action(&mut app, Action::AddLineComment);

    assert_eq!(app.input_mode, InputMode::Comment);
    assert!(!app.comment_is_file_level);
}

#[test]
fn should_not_find_comment_on_bare_diff_line() {
    let mut app = app_with("a.rs");
    cursor_on(&mut app, |a| matches!(a, AnnotatedLine::DiffLine { .. }));

    assert!(app.find_comment_location_at_cursor().is_none());
    assert_eq!(app.input_mode, InputMode::Normal);
}

#[test]
fn should_not_find_comment_on_file_header() {
    let mut app = app_with("a.rs");
    cursor_on(&mut app, |a| matches!(a, AnnotatedLine::FileHeader { .. }));

    assert!(app.find_comment_location_at_cursor().is_none());
    assert_eq!(app.input_mode, InputMode::Normal);
}

#[test]
fn should_not_find_comment_on_remote_thread_line() {
    let mut app = app_with("a.rs");
    app.line_annotations
        .push(AnnotatedLine::RemoteThreadLine { thread_idx: 0 });
    app.diff_state.cursor_line = app.line_annotations.len() - 1;

    assert!(app.find_comment_location_at_cursor().is_none());
    assert_eq!(app.input_mode, InputMode::Normal);
}

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

fn line(content: &str, new: u32) -> DiffLine {
    DiffLine {
        origin: LineOrigin::Addition,
        content: content.to_string(),
        old_lineno: None,
        new_lineno: Some(new),
        highlighted_spans: None,
    }
}

fn file(path: &str) -> DiffFile {
    let contents = ["l1", "l2", "l3", "l4", "l5"];
    let lines = contents
        .iter()
        .enumerate()
        .map(|(idx, content)| line(content, idx as u32 + 1))
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

fn app_with(path: &str) -> App {
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
        vec![file(path)],
        session,
        DiffSource::WorkingTree,
        InputMode::Normal,
        Vec::new(),
        None,
        None,
    )
    .expect("build app")
}

fn add_line_comment(app: &mut App, path: &str, line_no: u32, comment: Comment) {
    let review = app
        .session
        .get_file_mut(&PathBuf::from(path))
        .expect("file in session");
    review
        .line_comments
        .entry(line_no)
        .or_default()
        .push(comment);
}

fn cursor_on(app: &mut App, matches: impl Fn(&AnnotatedLine) -> bool) {
    app.rebuild_annotations();
    let idx = app
        .line_annotations
        .iter()
        .position(|a| matches(a))
        .expect("expected the annotation under the cursor");
    app.diff_state.cursor_line = idx;
}
