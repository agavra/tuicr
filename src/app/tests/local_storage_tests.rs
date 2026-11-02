//! Repo-local review session storage (config `local_storage` /
//! `--local-storage`): the [`App::reviews_root`] routing and the
//! Sessions-tab listing merge. The full save/resume/cleanup cycle lives in
//! `tests/local_storage_startup.rs`.

use crate::app::*;
use crate::model::SessionDiffSource;
use crate::vcs::traits::{VcsBackend, VcsInfo, VcsType};
use std::path::PathBuf;

/// Redirects the platform-data-dir storage root to a temp dir for the
/// current thread, so "global untouched" assertions are meaningful.
struct TestReviewsDir {
    _dir: tempfile::TempDir,
}

impl TestReviewsDir {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("failed to create test reviews dir");
        crate::persistence::storage::set_test_reviews_dir(Some(dir.path().to_path_buf()));
        Self { _dir: dir }
    }
}

impl Drop for TestReviewsDir {
    fn drop(&mut self) {
        crate::persistence::storage::set_test_reviews_dir(None);
    }
}

/// Minimal backend: the persistence layer only reads `info()`.
struct NullVcs {
    info: VcsInfo,
}

impl VcsBackend for NullVcs {
    fn info(&self) -> &VcsInfo {
        &self.info
    }

    fn get_working_tree_diff(
        &self,
        _highlighter: &crate::syntax::SyntaxHighlighter,
    ) -> crate::error::Result<Vec<crate::model::DiffFile>> {
        Ok(Vec::new())
    }

    fn fetch_context_lines(
        &self,
        _file_path: &Path,
        _file_status: crate::model::FileStatus,
        _ref_commit: Option<&str>,
        _start_line: u32,
        _end_line: u32,
    ) -> crate::error::Result<Vec<crate::model::DiffLine>> {
        Ok(Vec::new())
    }

    fn file_line_count(
        &self,
        _file_path: &Path,
        _file_status: crate::model::FileStatus,
        _ref_commit: Option<&str>,
    ) -> crate::error::Result<u32> {
        Ok(0)
    }
}

fn git_vcs_info(root: PathBuf) -> VcsInfo {
    VcsInfo {
        root_path: root,
        head_commit: "abc1234".to_string(),
        branch_name: Some("main".to_string()),
        vcs_type: VcsType::Git,
    }
}

fn working_tree_session(vcs_info: &VcsInfo) -> ReviewSession {
    ReviewSession::new(
        vcs_info.root_path.clone(),
        vcs_info.head_commit.clone(),
        vcs_info.branch_name.clone(),
        SessionDiffSource::WorkingTree,
    )
}

#[test]
fn should_use_the_repo_local_root_for_local_reviews_when_enabled() {
    let _reviews = TestReviewsDir::new();
    let repo = tempfile::tempdir().unwrap();
    let vcs_info = git_vcs_info(repo.path().to_path_buf());
    let session = working_tree_session(&vcs_info);

    let root =
        App::session_reviews_root(true, &session, &vcs_info, &DiffSource::WorkingTree).unwrap();

    assert_eq!(
        root,
        crate::persistence::storage::repo_local_reviews_dir(repo.path()),
        "an opted-in local review must persist beside the checkout"
    );
}

#[test]
fn should_use_the_platform_dir_when_local_storage_is_disabled() {
    let _reviews = TestReviewsDir::new();
    let repo = tempfile::tempdir().unwrap();
    let vcs_info = git_vcs_info(repo.path().to_path_buf());
    let session = working_tree_session(&vcs_info);

    let root =
        App::session_reviews_root(false, &session, &vcs_info, &DiffSource::WorkingTree).unwrap();

    assert_eq!(
        root,
        crate::persistence::storage::get_reviews_dir().unwrap(),
        "the default must stay byte-identical to today's behavior"
    );
}

#[test]
fn should_keep_pr_sessions_in_the_platform_dir_even_with_local_storage() {
    let _reviews = TestReviewsDir::new();
    let repo = tempfile::tempdir().unwrap();
    let vcs_info = git_vcs_info(repo.path().to_path_buf());
    let key = crate::forge::traits::PrSessionKey::new(
        crate::forge::traits::ForgeRepository::github("github.com", "owner", "repo"),
        7,
        "head".to_string(),
    );

    // A PR session identity: pr_session_key set, PR diff source.
    let mut session = working_tree_session(&vcs_info);
    session.pr_session_key = Some(key.clone());
    session.diff_source = SessionDiffSource::PullRequest;
    let diff_source = DiffSource::PullRequest(Box::new(crate::app::PullRequestDiffSource {
        key,
        base_sha: "base".to_string(),
        title: "A PR".to_string(),
        url: "https://github.com/owner/repo/pull/7".to_string(),
        head_ref_name: "feature".to_string(),
        base_ref_name: "main".to_string(),
        state: "OPEN".to_string(),
        closed: false,
        merged: false,
    }));

    let root = App::session_reviews_root(true, &session, &vcs_info, &diff_source).unwrap();

    assert_eq!(
        root,
        crate::persistence::storage::get_reviews_dir().unwrap(),
        "PR sessions must never move into a checkout's .tuicr/"
    );

    // The session-identity guard alone (key set, non-PR source shape) must
    // also route globally: it covers PR state installed before diff_source.
    let diff_source = DiffSource::WorkingTree;
    let root = App::session_reviews_root(true, &session, &vcs_info, &diff_source).unwrap();
    assert_eq!(
        root,
        crate::persistence::storage::get_reviews_dir().unwrap()
    );
}

#[test]
fn should_keep_file_mode_in_the_platform_dir_even_with_local_storage() {
    let _reviews = TestReviewsDir::new();
    let dir = tempfile::tempdir().unwrap();
    let vcs_info = VcsInfo {
        root_path: dir.path().to_path_buf(),
        head_commit: "file".to_string(),
        branch_name: None,
        vcs_type: VcsType::File,
    };
    let session = working_tree_session(&vcs_info);

    let root =
        App::session_reviews_root(true, &session, &vcs_info, &DiffSource::WorkingTree).unwrap();

    assert_eq!(
        root,
        crate::persistence::storage::get_reviews_dir().unwrap(),
        "--file / --all-files must not scatter .tuicr/ next to arbitrary files"
    );
}

#[test]
fn should_list_repo_local_sessions_in_the_sessions_tab() {
    let _reviews = TestReviewsDir::new();
    let repo = tempfile::tempdir().unwrap();
    let vcs_info = git_vcs_info(repo.path().to_path_buf());

    // A resumable session (reviewed file) saved with local storage on.
    let repo_local = crate::persistence::storage::repo_local_reviews_dir(repo.path());
    let store = crate::review_store::ReviewStore::with_reviews_dir(&repo_local);
    let mut session = working_tree_session(&vcs_info);
    session.add_file(
        std::path::PathBuf::from("src/main.rs"),
        crate::model::FileStatus::Modified,
        0,
    );
    session
        .get_file_mut(&std::path::PathBuf::from("src/main.rs"))
        .unwrap()
        .reviewed = true;
    store.save_review(&session).unwrap();

    let mut app = App::build(
        Box::new(NullVcs {
            info: vcs_info.clone(),
        }),
        vcs_info,
        Theme::dark(),
        None,
        false,
        Vec::new(),
        working_tree_session(&git_vcs_info(repo.path().to_path_buf())),
        DiffSource::WorkingTree,
        InputMode::Normal,
        Vec::new(),
        None,
        None,
    )
    .expect("build app");

    // The flag is OFF here: the tab still surfaces repo-local sessions
    // because they may have been created with it on.
    app.load_sessions_tab();

    assert_eq!(app.sessions_tab.rows().len(), 1);
}
