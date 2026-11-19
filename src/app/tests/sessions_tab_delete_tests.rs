//! Deleting the saved review this TUI has open, from the Sessions tab.
//!
//! The active-session registry treated the TUI's own registration like another
//! process's, so `dd` on the review this very process had loaded reported
//! `DeleteOutcome::Active` and the only saved review could never be removed
//! (#865).

use crate::app::*;
use crate::model::{Comment, CommentType, FileStatus};
use crate::persistence::storage;
use crate::vcs::traits::{VcsChangeStatus, VcsType};

struct TestReviewsDir {
    _dir: tempfile::TempDir,
}

impl TestReviewsDir {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("failed to create test reviews dir");
        storage::set_test_reviews_dir(Some(dir.path().to_path_buf()));
        Self { _dir: dir }
    }
}

impl Drop for TestReviewsDir {
    fn drop(&mut self) {
        storage::set_test_reviews_dir(None);
    }
}

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

    fn get_change_status(&self) -> Result<VcsChangeStatus> {
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
    ) -> Result<u32> {
        Ok(0)
    }
}

/// App rooted at `root`, with a session that has not been saved yet.
fn build_app(root: PathBuf) -> App {
    let vcs_info = VcsInfo {
        root_path: root,
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

/// Save the current session with one comment, exactly as autosave does: the
/// file exists and this process is registered as its owner.
fn save_review_with_comment(app: &mut App) -> PathBuf {
    app.session.review_comments.push(Comment::new(
        "delete me".to_string(),
        CommentType::from_id("note"),
        None,
    ));
    app.dirty = true;
    app.save_current_session_merging_external()
        .expect("save session")
}

#[test]
fn should_delete_the_saved_review_this_tui_has_open() {
    let _reviews = TestReviewsDir::new();
    let repo = tempfile::tempdir().unwrap();
    let mut app = build_app(repo.path().to_path_buf());
    let path = save_review_with_comment(&mut app);
    app.load_sessions_tab();
    assert_eq!(app.sessions_tab.rows().len(), 1);

    app.sessions_tab_delete_selected();

    assert!(
        !path.exists(),
        "dd on the review this process has open must delete it"
    );
}

#[test]
fn should_drop_the_deleted_review_from_memory() {
    let _reviews = TestReviewsDir::new();
    let repo = tempfile::tempdir().unwrap();
    let mut app = build_app(repo.path().to_path_buf());
    let path = save_review_with_comment(&mut app);
    app.load_sessions_tab();

    app.sessions_tab_delete_selected();

    assert_eq!(
        app.session.comment_count(),
        0,
        "comments survived the delete"
    );
    assert!(
        app.session_path.is_none(),
        "the deleted file is still tracked"
    );
    assert!(!app.dirty);
    // A later save starts a new review: it must not carry the deleted
    // comment back to disk.
    app.save_current_session_merging_external()
        .expect("saving after the delete");
    let saved = std::fs::read_to_string(&path).expect("the fresh save must land at the same path");
    assert!(
        !saved.contains("delete me"),
        "the delete was undone: {saved}"
    );
}

#[test]
fn should_clean_up_an_empty_session_a_full_reload_recreates() {
    let _reviews = TestReviewsDir::new();
    let repo = tempfile::tempdir().unwrap();
    let mut app = build_app(repo.path().to_path_buf());
    let path = save_review_with_comment(&mut app);
    app.load_sessions_tab();

    app.sessions_tab_delete_selected();
    // A full reload persists the reconciled (now empty) review, recreating
    // the file; quit-time cleanup must remove it again.
    app.persist_diff_reconciliation(&[])
        .expect("the reload path saves the reconciled session");
    assert!(path.exists(), "the reload path must recreate the file");
    app.cleanup_empty_ephemeral_sessions()
        .expect("quit cleanup");

    assert!(!path.exists(), "the emptied session survived quit cleanup");
}

#[test]
fn should_drop_the_in_memory_review_when_its_file_is_already_gone() {
    let _reviews = TestReviewsDir::new();
    let repo = tempfile::tempdir().unwrap();
    let mut app = build_app(repo.path().to_path_buf());
    let path = save_review_with_comment(&mut app);
    app.load_sessions_tab();
    std::fs::remove_file(&path).expect("remove the session file first");

    app.sessions_tab_delete_selected();

    assert!(app.sessions_tab.is_empty());
    assert!(
        app.session_path.is_none(),
        "a missing file is still tracked"
    );
    assert_eq!(app.session.comment_count(), 0);
}

#[test]
fn should_still_refuse_a_review_another_process_has_open() {
    let _reviews = TestReviewsDir::new();
    let repo = tempfile::tempdir().unwrap();
    let mut app = build_app(repo.path().to_path_buf());
    let path = save_review_with_comment(&mut app);
    // A second live tuicr has the same session open; pid 1 is always running.
    let reviews_dir = storage::get_reviews_dir().unwrap();
    storage::mark_session_active_for_pid_in_dir(&app.session, &path, &reviews_dir, 1).unwrap();
    app.load_sessions_tab();

    app.sessions_tab_delete_selected();

    assert!(path.exists(), "another process's open review was deleted");
    assert_eq!(
        app.session.comment_count(),
        1,
        "the open review was dropped"
    );
}
