//! Repo-local review session storage (config `local_storage` /
//! `--local-storage`): the [`App::reviews_root`] routing and the
//! Sessions-tab listing merge. The full save/resume/cleanup cycle lives in
//! `tests/local_storage_startup.rs`.

use crate::app::*;
use crate::model::SessionDiffSource;
use crate::vcs::traits::{VcsBackend, VcsInfo, VcsType};
use std::path::{Path, PathBuf};

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

/// A `DiffSource::PullRequest` value for routing tests; only the variant
/// shape matters, never the payload.
fn pr_diff_source(key: crate::forge::traits::PrSessionKey) -> DiffSource {
    DiffSource::PullRequest(Box::new(crate::app::PullRequestDiffSource {
        key,
        base_sha: "base".to_string(),
        title: "A PR".to_string(),
        url: "https://github.com/owner/repo/pull/7".to_string(),
        head_ref_name: "feature".to_string(),
        base_ref_name: "main".to_string(),
        state: "OPEN".to_string(),
        closed: false,
        merged: false,
    }))
}

#[test]
fn should_route_each_session_kind_to_its_storage_root() {
    let _reviews = TestReviewsDir::new();
    let repo = tempfile::tempdir().unwrap();
    let git = git_vcs_info(repo.path().to_path_buf());
    let file = VcsInfo {
        root_path: repo.path().to_path_buf(),
        head_commit: "file".to_string(),
        branch_name: None,
        vcs_type: VcsType::File,
    };

    // A PR session identity: pr_session_key set, PR diff source.
    let mut pr_session = working_tree_session(&git);
    let key = crate::forge::traits::PrSessionKey::new(
        crate::forge::traits::ForgeRepository::github("github.com", "owner", "repo"),
        7,
        "head".to_string(),
    );
    pr_session.pr_session_key = Some(key.clone());
    pr_session.diff_source = SessionDiffSource::PullRequest;

    let repo_local = crate::persistence::storage::repo_local_reviews_dir(repo.path());
    let platform = crate::persistence::storage::get_reviews_dir().unwrap();

    let cases: &[(&str, bool, &VcsInfo, ReviewSession, DiffSource, &Path)] = &[
        (
            "an opted-in local review must persist beside the checkout",
            true,
            &git,
            working_tree_session(&git),
            DiffSource::WorkingTree,
            &repo_local,
        ),
        (
            "the default must stay byte-identical to today's behavior",
            false,
            &git,
            working_tree_session(&git),
            DiffSource::WorkingTree,
            &platform,
        ),
        (
            "PR sessions must never move into a checkout's .tuicr/",
            true,
            &git,
            pr_session.clone(),
            pr_diff_source(key.clone()),
            &platform,
        ),
        (
            "a PR identity before its diff source installs must route globally",
            true,
            &git,
            pr_session,
            DiffSource::WorkingTree,
            &platform,
        ),
        (
            "--file / --all-files must not scatter .tuicr/ next to arbitrary files",
            true,
            &file,
            working_tree_session(&file),
            DiffSource::WorkingTree,
            &platform,
        ),
    ];

    for &(name, local_storage, vcs_info, ref session, ref diff_source, expected) in cases {
        let root = App::session_reviews_root(local_storage, session, vcs_info, diff_source)
            .expect("routing must resolve");
        assert_eq!(root, expected, "{name}");
    }
}

/// An app with the flag OFF whose checkout nevertheless carries one
/// resumable repo-local session (a reviewed file), so the Sessions tab
/// sees a row from the run's inactive root.
fn app_with_repo_local_session(repo: &Path) -> (App, ReviewSession) {
    let vcs_info = git_vcs_info(repo.to_path_buf());
    let repo_local = crate::persistence::storage::repo_local_reviews_dir(repo);
    let store = crate::review_store::ReviewStore::with_reviews_dir(&repo_local);
    let mut session = working_tree_session(&vcs_info);
    session.add_file(
        PathBuf::from("src/main.rs"),
        crate::model::FileStatus::Modified,
        0,
    );
    session
        .get_file_mut(&PathBuf::from("src/main.rs"))
        .unwrap()
        .reviewed = true;
    store.save_review(&session).unwrap();

    let app = App::build(
        Box::new(NullVcs {
            info: vcs_info.clone(),
        }),
        vcs_info,
        Theme::dark(),
        None,
        false,
        Vec::new(),
        working_tree_session(&git_vcs_info(repo.to_path_buf())),
        DiffSource::WorkingTree,
        InputMode::Normal,
        Vec::new(),
        None,
        None,
    )
    .expect("build app");
    (app, session)
}

#[test]
fn should_list_repo_local_sessions_in_the_sessions_tab() {
    let _reviews = TestReviewsDir::new();
    let repo = tempfile::tempdir().unwrap();
    let (mut app, _session) = app_with_repo_local_session(repo.path());

    // The flag is OFF here: the tab still surfaces repo-local sessions
    // because they may have been created with it on.
    app.load_sessions_tab();

    assert_eq!(app.sessions_tab.rows().len(), 1);
}

#[test]
fn should_ask_to_restart_when_opening_a_session_from_the_inactive_root() {
    let _reviews = TestReviewsDir::new();
    let repo = tempfile::tempdir().unwrap();
    let (mut app, session) = app_with_repo_local_session(repo.path());

    app.load_sessions_tab();
    assert_eq!(app.sessions_tab.rows().len(), 1);
    app.sessions_tab_select().unwrap();

    assert!(
        app.message
            .as_ref()
            .is_some_and(|m| m.content.contains("--local-storage")),
        "selecting a cross-root row must explain the restart, got {:?}",
        app.message.as_ref().map(|m| &m.content)
    );
    assert_ne!(
        app.session.id, session.id,
        "the inactive root's session must not be adopted into this run"
    );
}
