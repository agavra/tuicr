use crate::app::*;
use crate::model::{ReviewSession, SessionDiffSource};
use crate::review_store::ReviewStore;
use crate::vcs::FileBackend;

struct TestReviewsDir {
    _dir: tempfile::TempDir,
}

impl TestReviewsDir {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        crate::persistence::storage::set_test_reviews_dir(Some(dir.path().to_path_buf()));
        Self { _dir: dir }
    }
}

impl Drop for TestReviewsDir {
    fn drop(&mut self) {
        crate::persistence::storage::set_test_reviews_dir(None);
    }
}

fn file_options(path: &str) -> AppStartupOptions<'_> {
    AppStartupOptions {
        revisions: None,
        working_tree: false,
        path_filter: None,
        file_path: Some(path),
        all_files: false,
        show_pr_checks: false,
        show_pr_comments: true,
        pr_comments_visibility: None,
        git_backend_preference: GitBackendPreference::Libgit2,
        diff_whitespace_mode: DiffWhitespaceMode::Normal,
        commit_selection: CommitSelectionStart::All,
        pr_target: None,
        repo_url_override: None,
    }
}

#[test]
fn should_give_sibling_file_reviews_distinct_session_slugs() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.md");
    let second = dir.path().join("second.md");
    std::fs::write(&first, "first").unwrap();
    std::fs::write(&second, "second").unwrap();

    let first_app = App::new(
        Theme::dark(),
        None,
        false,
        file_options(first.to_str().unwrap()),
    )
    .unwrap();
    let second_app = App::new(
        Theme::dark(),
        None,
        false,
        file_options(second.to_str().unwrap()),
    )
    .unwrap();

    assert_eq!(first_app.session.diff_source, SessionDiffSource::File);
    assert_eq!(second_app.session.diff_source, SessionDiffSource::File);
    assert_ne!(first_app.session_slug(), second_app.session_slug());
}

#[test]
fn should_leave_directory_file_review_identity_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("file.md"), "content").unwrap();

    let app = App::new(
        Theme::dark(),
        None,
        false,
        file_options(dir.path().to_str().unwrap()),
    )
    .unwrap();

    assert_eq!(app.session.diff_source, SessionDiffSource::WorkingTree);
    assert_eq!(app.session.base_commit, "file");
}

#[test]
fn should_leave_legacy_single_file_session_separate_and_accessible() {
    let _reviews = TestReviewsDir::new();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("review.md");
    std::fs::write(&path, "content").unwrap();
    let backend = FileBackend::new(path.to_str().unwrap()).unwrap();
    let diff_files = backend
        .get_working_tree_diff(&SyntaxHighlighter::default())
        .unwrap();
    let info = backend.info();
    let store = ReviewStore::new();
    let mut legacy = ReviewSession::new(
        info.root_path.clone(),
        "file".to_string(),
        None,
        SessionDiffSource::WorkingTree,
    );
    legacy.add_diff_file(&diff_files[0]);
    legacy
        .files
        .get_mut(diff_files[0].display_path())
        .unwrap()
        .reviewed = true;
    let legacy_ref = store.save_review(&legacy).unwrap();

    let mut app = App::new(
        Theme::dark(),
        None,
        false,
        file_options(path.to_str().unwrap()),
    )
    .unwrap();
    assert!(
        !app.session
            .files
            .get(diff_files[0].display_path())
            .unwrap()
            .reviewed
    );
    let current_path = app
        .ensure_ephemeral_session_file()
        .unwrap()
        .expect("new identity should create its own session");

    assert_ne!(legacy_ref.path(), current_path);
    assert!(legacy_ref.path().is_file());
    let sessions = store.list_sessions_for_repo(&info.root_path).unwrap();
    assert_eq!(sessions.len(), 2);
    assert!(
        sessions
            .iter()
            .any(|summary| summary.slug.contains("/worktree/file"))
    );
    assert!(
        sessions
            .iter()
            .any(|summary| summary.slug.contains("/file/"))
    );
    assert!(
        store
            .get_review(&legacy_ref)
            .unwrap()
            .files
            .get(diff_files[0].display_path())
            .unwrap()
            .reviewed
    );
}
