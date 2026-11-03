//! Repo-local review session storage (config `local_storage` /
//! `--local-storage`): the [`App::reviews_root`] routing. The full
//! save/resume/cleanup cycle lives in `tests/local_storage_startup.rs`.

use crate::app::*;
use crate::model::SessionDiffSource;
use crate::vcs::traits::{VcsInfo, VcsType};
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
