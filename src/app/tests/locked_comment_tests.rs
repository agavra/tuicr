//! The cursor asks whether the comment under it is locked before offering to
//! edit or delete it. The annotation carries an index straight into the
//! line's comment vec — the same index every other cursor lookup uses.

use crate::app::*;
use crate::model::comment::CommentLifecycleState;
use crate::model::{Comment, CommentType, DiffFile, FileStatus, LineSide};
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
        _file_status: crate::model::FileStatus,
        _ref_commit: Option<&str>,
        _start_line: u32,
        _end_line: u32,
    ) -> Result<Vec<DiffLine>> {
        Ok(Vec::new())
    }
    fn file_line_count(
        &self,
        _file_path: &Path,
        _file_status: crate::model::FileStatus,
        _ref_commit: Option<&str>,
    ) -> Result<u32> {
        Ok(0)
    }
    fn get_change_status(&self) -> Result<VcsChangeStatus> {
        Ok(VcsChangeStatus {
            staged: false,
            unstaged: false,
        })
    }
}

fn diff_file(path: &str) -> DiffFile {
    DiffFile {
        old_path: None,
        new_path: Some(PathBuf::from(path)),
        status: FileStatus::Modified,
        hunks: Vec::new(),
        is_binary: false,
        is_too_large: false,
        is_commit_message: false,
        content_hash: 0,
    }
}

fn comment(side: LineSide, state: CommentLifecycleState) -> Comment {
    let mut comment = Comment::new(
        "a remark".to_string(),
        CommentType::from_id("note"),
        Some(side),
    );
    comment.lifecycle_state = state;
    comment
}

fn locked_app() -> App {
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
    let mut app = App::build(
        Box::new(DummyVcs {
            info: vcs_info.clone(),
        }),
        vcs_info,
        Theme::dark(),
        None,
        false,
        vec![diff_file("src/main.rs")],
        session,
        DiffSource::WorkingTree,
        InputMode::Normal,
        Vec::new(),
        None,
        None,
    )
    .expect("failed to build test app");
    let path = PathBuf::from("src/main.rs");
    app.session.add_file(path.clone(), FileStatus::Modified, 0);
    app.session.get_file_mut(&path).unwrap().add_line_comment(
        42,
        comment(LineSide::New, CommentLifecycleState::LocalDraft),
    );
    app.line_annotations = vec![AnnotatedLine::LineComment {
        file_idx: 0,
        line: 42,
        comment_idx: 0,
        side: LineSide::New,
    }];
    app.diff_state.cursor_line = 0;
    app.diff_state.current_file_idx = 0;
    app
}

fn first_comment(app: &mut App) -> &mut Comment {
    let path = PathBuf::from("src/main.rs");
    &mut app
        .session
        .get_file_mut(&path)
        .unwrap()
        .line_comments
        .get_mut(&42)
        .unwrap()[0]
}

#[test]
fn should_refuse_to_edit_a_submitted_comment() {
    let mut app = locked_app();
    first_comment(&mut app).lifecycle_state = CommentLifecycleState::Submitted;

    assert!(!app.enter_edit_mode(true));
    assert_eq!(app.input_mode, InputMode::Normal);
}

#[test]
fn should_not_save_over_a_comment_submitted_mid_edit() {
    let mut app = locked_app();
    assert!(app.enter_edit_mode(true));
    first_comment(&mut app).lifecycle_state = CommentLifecycleState::Submitted;
    app.comment_buffer = "mutated after submit".to_string();

    app.save_comment();

    assert_eq!(first_comment(&mut app).content, "a remark");
}

#[test]
fn should_answer_about_the_comment_the_annotation_points_at() {
    // Line 42 holds a comment on each side. The annotation for the new-side
    // box carries index 1 — into the whole vec, the way the builder emits it
    // and every other lookup reads it. Re-counting per side would land on a
    // different comment and call a published one editable.
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
    let mut app = App::build(
        Box::new(DummyVcs {
            info: vcs_info.clone(),
        }),
        vcs_info,
        Theme::dark(),
        None,
        false,
        vec![diff_file("src/main.rs")],
        session,
        DiffSource::WorkingTree,
        InputMode::Normal,
        Vec::new(),
        None,
        None,
    )
    .expect("failed to build test app");

    let path = PathBuf::from("src/main.rs");
    app.session.add_file(path.clone(), FileStatus::Modified, 0);
    let review = app.session.get_file_mut(&path).unwrap();
    review.add_line_comment(42, comment(LineSide::Old, CommentLifecycleState::Submitted));
    review.add_line_comment(42, comment(LineSide::New, CommentLifecycleState::Submitted));

    app.line_annotations = vec![AnnotatedLine::LineComment {
        file_idx: 0,
        line: 42,
        comment_idx: 1,
        side: LineSide::New,
    }];
    app.diff_state.cursor_line = 0;
    app.diff_state.current_file_idx = 0;

    assert!(
        app.cursor_on_locked_comment(),
        "index 1 is the submitted new-side comment; a per-side recount would \
         miss it and offer to edit a comment that is already on the forge"
    );
}

fn two_file_app() -> App {
    let mut app = locked_app();
    app.diff_files.push(diff_file("src/other.rs"));
    app.session
        .add_file(PathBuf::from("src/other.rs"), FileStatus::Modified, 0);
    app.diff_state.current_file_idx = 0;
    app
}

fn other_review(app: &mut App) -> &mut crate::model::review::FileReview {
    app.session
        .get_file_mut(&PathBuf::from("src/other.rs"))
        .unwrap()
}

fn start_edit(app: &mut App, id: &str, text: &str) {
    app.editing_comment_id = Some(id.to_string());
    app.comment_buffer = text.to_string();
    app.input_mode = InputMode::Comment;
}

fn note(state: CommentLifecycleState) -> Comment {
    let mut c = Comment::new("orig".to_string(), CommentType::from_id("note"), None);
    c.lifecycle_state = state;
    c
}

#[test]
fn locked_edit_keeps_the_typed_buffer_and_mode() {
    let mut app = locked_app();
    assert!(app.enter_edit_mode(true));
    first_comment(&mut app).lifecycle_state = CommentLifecycleState::Submitted;
    app.comment_buffer = "typed text".to_string();

    app.save_comment();

    assert_eq!(app.comment_buffer, "typed text");
    assert_eq!(app.input_mode, InputMode::Comment);
    assert_eq!(first_comment(&mut app).content, "a remark");
}

#[test]
fn locked_review_comment_is_not_overwritten() {
    let mut app = locked_app();
    let c = note(CommentLifecycleState::Submitted);
    let id = c.id.clone();
    app.session.review_comments.push(c);
    start_edit(&mut app, &id, "changed");

    app.save_comment();

    assert_eq!(app.session.review_comments[0].content, "orig");
    assert_eq!(app.comment_buffer, "changed");
}

#[test]
fn locked_file_comment_is_not_overwritten() {
    let mut app = locked_app();
    let c = note(CommentLifecycleState::Submitted);
    let id = c.id.clone();
    let path = PathBuf::from("src/main.rs");
    app.session
        .get_file_mut(&path)
        .unwrap()
        .file_comments
        .push(c);
    start_edit(&mut app, &id, "changed");

    app.save_comment();

    let review = app.session.get_file_mut(&path).unwrap();
    assert_eq!(review.file_comments[0].content, "orig");
    assert_eq!(app.comment_buffer, "changed");
}

#[test]
fn edit_finds_review_file_and_line_comments_outside_the_current_file() {
    let mut app = two_file_app();
    // Cursor file is src/main.rs (0); every edited comment lives elsewhere.
    let file_c = note(CommentLifecycleState::LocalDraft);
    let file_id = file_c.id.clone();
    let line_c = comment(LineSide::New, CommentLifecycleState::LocalDraft);
    let line_id = line_c.id.clone();
    let review_c = note(CommentLifecycleState::LocalDraft);
    let review_id = review_c.id.clone();
    other_review(&mut app).file_comments.push(file_c);
    other_review(&mut app).add_line_comment(7, line_c);
    app.session.review_comments.push(review_c);

    for (id, text) in [(&file_id, "file edit"), (&line_id, "line edit")] {
        start_edit(&mut app, id, text);
        app.save_comment();
    }
    assert_eq!(other_review(&mut app).file_comments[0].content, "file edit");
    assert_eq!(
        other_review(&mut app).line_comments[&7][0].content,
        "line edit"
    );

    start_edit(&mut app, &review_id, "review edit");
    app.save_comment();
    assert_eq!(app.session.review_comments[0].content, "review edit");
    // The cursor file's own comment was untouched.
    assert_eq!(first_comment(&mut app).content, "a remark");
}

#[test]
fn visual_comment_moves_current_file_to_the_selected_file() {
    let mut app = two_file_app();
    app.line_annotations = (0..2)
        .map(|i| AnnotatedLine::DiffLine {
            file_idx: 1,
            hunk_idx: 0,
            line_idx: i,
            old_lineno: Some(i as u32 + 1),
            new_lineno: Some(i as u32 + 1),
        })
        .collect();
    let point = |annotation_idx| SelPoint {
        annotation_idx,
        char_offset: 0,
        side: LineSide::New,
    };
    app.visual_selection = Some(VisualSelection {
        anchor: point(0),
        head: point(1),
    });
    app.input_mode = InputMode::VisualSelect;
    app.diff_state.current_file_idx = 0; // paging moved it off the selection

    app.enter_comment_from_visual();
    assert_eq!(app.input_mode, InputMode::Comment);
    assert_eq!(app.diff_state.current_file_idx, 1);

    app.comment_buffer = "range".to_string();
    app.save_comment();
    assert!(other_review(&mut app).line_comments.contains_key(&2));
    assert!(
        !app.session
            .get_file_mut(&PathBuf::from("src/main.rs"))
            .unwrap()
            .line_comments
            .contains_key(&2)
    );
}

#[test]
fn visual_suggestion_across_files_is_rejected_without_leaving_visual_mode() {
    let mut app = two_file_app();
    app.line_annotations = (0..2)
        .map(|i| AnnotatedLine::DiffLine {
            file_idx: i,
            hunk_idx: 0,
            line_idx: 0,
            old_lineno: Some(1),
            new_lineno: Some(1),
        })
        .collect();
    let point = |annotation_idx| SelPoint {
        annotation_idx,
        char_offset: 0,
        side: LineSide::New,
    };
    app.visual_selection = Some(VisualSelection {
        anchor: point(0),
        head: point(1),
    });
    app.input_mode = InputMode::VisualSelect;

    crate::handler::handle_visual_action(&mut app, crate::input::Action::AddRangeSuggestion);

    assert_eq!(app.input_mode, InputMode::VisualSelect);
    assert!(app.visual_selection.is_some());
    assert!(app.comment_line_range.is_none());
}
