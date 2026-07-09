use crate::app::*;
use crate::model::{CommentType, LineSide};
use crate::vcs::traits::{CommitInfo, VcsType};

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
}

fn build_app_with_review_commits(commits: Vec<CommitInfo>) -> App {
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
        Vec::new(),
        session,
        DiffSource::WorkingTree,
        InputMode::Normal,
        Vec::new(),
        None,
        None,
    )
    .expect("failed to build test app");
    app.review_commits = commits;
    app
}

fn commit(id: &str) -> CommitInfo {
    CommitInfo {
        id: id.to_string(),
        short_id: id.to_string(),
        branch_name: None,
        summary: format!("commit {id}"),
        body: None,
        author: "tester".to_string(),
        time: Utc::now(),
    }
}

fn line_comment(content: &str, commit_id: Option<&str>) -> crate::model::Comment {
    let mut c =
        crate::model::Comment::new(content.to_string(), CommentType::Note, Some(LineSide::New));
    c.commit_id = commit_id.map(|s| s.to_string());
    c
}

#[test]
fn legacy_comment_with_no_commit_id_is_always_visible() {
    let mut app = build_app_with_review_commits(vec![commit("aaa"), commit("bbb")]);
    app.commit_selection_range = Some((0, 0)); // only "aaa" selected
    let comment = line_comment("old comment", None);
    assert!(
        app.comment_visible(&comment),
        "legacy comment with commit_id=None must be visible regardless of selection"
    );
}

#[test]
fn comment_scoped_to_selected_commit_is_visible() {
    let mut app = build_app_with_review_commits(vec![commit("aaa"), commit("bbb")]);
    app.commit_selection_range = Some((0, 0)); // only "aaa" selected
    let comment = line_comment("on aaa", Some("aaa"));
    assert!(
        app.comment_visible(&comment),
        "comment scoped to the selected commit must be visible"
    );
}

#[test]
fn comment_scoped_to_unselected_commit_is_hidden() {
    let mut app = build_app_with_review_commits(vec![commit("aaa"), commit("bbb")]);
    app.commit_selection_range = Some((0, 0)); // only "aaa" selected
    let comment = line_comment("on bbb", Some("bbb"));
    assert!(
        !app.comment_visible(&comment),
        "comment scoped to a commit outside the selection must be hidden"
    );
}

#[test]
fn full_range_shows_all_commit_scoped_comments() {
    let mut app = build_app_with_review_commits(vec![commit("aaa"), commit("bbb")]);
    app.commit_selection_range = Some((0, 1)); // full range
    assert!(
        app.comment_visible(&line_comment("on aaa", Some("aaa"))),
        "full range includes commit aaa"
    );
    assert!(
        app.comment_visible(&line_comment("on bbb", Some("bbb"))),
        "full range includes commit bbb"
    );
}

#[test]
fn no_selector_shows_all_comments() {
    let app = build_app_with_review_commits(vec![commit("aaa"), commit("bbb")]);
    // commit_selection_range is None by default
    assert!(
        app.comment_visible(&line_comment("on aaa", Some("aaa"))),
        "no selector => all comments visible"
    );
    assert!(
        app.comment_visible(&line_comment("on bbb", Some("bbb"))),
        "no selector => all comments visible"
    );
}

#[test]
fn commit_id_for_new_comment_is_none_without_selector() {
    let app = build_app_with_review_commits(vec![commit("aaa"), commit("bbb")]);
    assert_eq!(
        app.commit_id_for_new_comment(),
        None,
        "no selector => no commit_id stamp"
    );
}

#[test]
fn commit_id_for_new_comment_is_none_for_full_range() {
    let mut app = build_app_with_review_commits(vec![commit("aaa"), commit("bbb")]);
    app.commit_selection_range = Some((0, 1));
    assert_eq!(
        app.commit_id_for_new_comment(),
        None,
        "full range => no commit_id stamp (comment is against cumulative diff)"
    );
}

#[test]
fn commit_id_for_new_comment_is_none_for_multi_commit_subset() {
    let mut app = build_app_with_review_commits(vec![commit("aaa"), commit("bbb"), commit("ccc")]);
    app.commit_selection_range = Some((0, 1)); // 2 of 3 commits
    assert_eq!(
        app.commit_id_for_new_comment(),
        None,
        "multi-commit subset => no commit_id stamp"
    );
}

#[test]
fn commit_id_for_new_comment_is_sha_for_single_commit() {
    let mut app = build_app_with_review_commits(vec![commit("aaa"), commit("bbb")]);
    app.commit_selection_range = Some((1, 1)); // only "bbb"
    assert_eq!(
        app.commit_id_for_new_comment(),
        Some("bbb".to_string()),
        "single commit selection stamps that commit's SHA"
    );
}

#[test]
fn add_comment_to_session_stamps_commit_id_when_provided() {
    use crate::review_store::{AddCommentRequest, CommentTarget, add_comment_to_session};
    use std::path::PathBuf;

    let mut session = ReviewSession::new(
        PathBuf::from("/repo"),
        "head".to_string(),
        Some("main".to_string()),
        SessionDiffSource::WorkingTree,
    );
    session.add_file(
        PathBuf::from("src/lib.rs"),
        crate::model::FileStatus::Modified,
        0,
    );

    let comment = add_comment_to_session(
        &mut session,
        AddCommentRequest {
            target: CommentTarget::Line {
                path: PathBuf::from("src/lib.rs"),
                line: 10,
                side: LineSide::New,
            },
            content: "scoped note".to_string(),
            comment_type: CommentType::Note,
            author: "user".to_string(),
            commit_id: Some("abc123".to_string()),
        },
    )
    .unwrap();

    assert_eq!(
        comment.commit_id,
        Some("abc123".to_string()),
        "add_comment_to_session must stamp the provided commit_id"
    );
    let stored = &session
        .files
        .get(&PathBuf::from("src/lib.rs"))
        .unwrap()
        .line_comments
        .get(&10)
        .unwrap()[0];
    assert_eq!(stored.commit_id, Some("abc123".to_string()));
}

#[test]
fn add_comment_to_session_leaves_commit_id_none_when_not_provided() {
    use crate::review_store::{AddCommentRequest, CommentTarget, add_comment_to_session};
    use std::path::PathBuf;

    let mut session = ReviewSession::new(
        PathBuf::from("/repo"),
        "head".to_string(),
        Some("main".to_string()),
        SessionDiffSource::WorkingTree,
    );
    session.add_file(
        PathBuf::from("src/lib.rs"),
        crate::model::FileStatus::Modified,
        0,
    );

    let comment = add_comment_to_session(
        &mut session,
        AddCommentRequest {
            target: CommentTarget::Line {
                path: PathBuf::from("src/lib.rs"),
                line: 10,
                side: LineSide::New,
            },
            content: "unscoped note".to_string(),
            comment_type: CommentType::Note,
            author: "user".to_string(),
            commit_id: None,
        },
    )
    .unwrap();

    assert_eq!(
        comment.commit_id, None,
        "commit_id must stay None when not provided"
    );
}

#[test]
fn legacy_session_json_deserializes_comment_without_commit_id() {
    let json = r#"{
            "id": "test-id",
            "content": "old comment",
            "comment_type": "note",
            "created_at": "2024-01-01T00:00:00Z",
            "line_context": null,
            "side": null,
            "line_range": null,
            "author": "user",
            "lifecycle_state": "local_draft",
            "remote_review_id": null,
            "remote_comment_id": null
        }"#;
    let comment: crate::model::Comment = serde_json::from_str(json).unwrap();
    assert_eq!(
        comment.commit_id, None,
        "legacy JSON without commit_id must deserialize as None"
    );
}
