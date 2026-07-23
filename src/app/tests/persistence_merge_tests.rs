use crate::app::*;
use crate::forge::traits::{PrSessionKey, PullRequestCommit};

const SOME_HASH: u64 = 0xabc;

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

fn test_session() -> ReviewSession {
    let mut session = ReviewSession::new(
        PathBuf::from("/repo"),
        "abc1234".to_string(),
        Some("main".to_string()),
        SessionDiffSource::WorkingTree,
    );
    session.add_file(
        PathBuf::from("src/main.rs"),
        FileStatus::Modified,
        SOME_HASH,
    );
    session
}

fn comment(id: &str, content: &str) -> Comment {
    let mut comment = Comment::new(content.to_string(), CommentType::from_id("note"), None);
    comment.id = id.to_string();
    comment
}

fn push_file_comment(session: &mut ReviewSession, id: &str, content: &str) {
    session
        .get_file_mut(&PathBuf::from("src/main.rs"))
        .unwrap()
        .file_comments
        .push(comment(id, content));
}

fn file_comment_ids(session: &ReviewSession) -> Vec<String> {
    session
        .files
        .get(&PathBuf::from("src/main.rs"))
        .unwrap()
        .file_comments
        .iter()
        .map(|comment| comment.id.clone())
        .collect()
}

#[test]
fn queued_pr_selection_save_preserves_persisted_comments() {
    let _reviews_dir = TestReviewsDir::new();
    let key = PrSessionKey::new(
        ForgeRepository::github("github.com", "agavra", "tuicr"),
        475,
        "abc1234",
    );
    let mut identity = test_session();
    identity.diff_source = SessionDiffSource::PullRequest;
    identity.pr_session_key = Some(key.clone());

    let mut persisted = identity.clone();
    push_file_comment(&mut persisted, "external", "keep me");
    let session_path = crate::persistence::storage::save_session(&persisted).unwrap();

    let vcs_info = VcsInfo {
        root_path: identity.repo_path.clone(),
        head_commit: identity.base_commit.clone(),
        branch_name: identity.branch_name.clone(),
        vcs_type: VcsType::File,
    };
    let pr_source = PullRequestDiffSource {
        key,
        base_sha: "base".to_string(),
        title: "test pr".to_string(),
        url: "https://github.com/agavra/tuicr/pull/475".to_string(),
        head_ref_name: "feature".to_string(),
        base_ref_name: "main".to_string(),
        state: "OPEN".to_string(),
        closed: false,
        merged: false,
    };
    let mut app = App::build(
        Box::new(PrNoopVcs::new(vcs_info.clone())),
        vcs_info,
        Theme::dark(),
        None,
        false,
        Vec::new(),
        identity,
        DiffSource::PullRequest(Box::new(pr_source)),
        InputMode::Normal,
        Vec::new(),
        None,
        None,
    )
    .expect("failed to build test app");
    app.pr_commits = (0..3)
        .map(|idx| PullRequestCommit {
            oid: format!("commit-{idx}"),
            short_oid: format!("c{idx}"),
            summary: format!("commit {idx}"),
            author: "Test".to_string(),
            timestamp: None,
        })
        .collect();
    app.commit_selection_range = Some((1, 1));

    app.persist_pr_commit_selection_range();
    app.flush_pr_commit_selection_save();

    let saved = crate::persistence::storage::load_session(&session_path).unwrap();
    assert_eq!(saved.commit_selection_range, Some((1, 1)));
    assert_eq!(file_comment_ids(&saved), vec!["external"]);
}

#[test]
fn should_merge_external_comment_without_losing_local_comment() {
    let base = test_session();
    let mut current = base.clone();
    let mut latest = base.clone();

    push_file_comment(&mut current, "local", "from tui");
    push_file_comment(&mut latest, "external", "from cli");

    let changed = App::merge_external_session_changes(&mut current, &base, &latest);

    assert_eq!(changed, 1);
    assert_eq!(file_comment_ids(&current), vec!["local", "external"]);
}

#[test]
fn should_not_resurrect_locally_deleted_comment_when_disk_is_unchanged() {
    let mut base = test_session();
    push_file_comment(&mut base, "deleted", "old");
    let mut current = base.clone();
    current
        .get_file_mut(&PathBuf::from("src/main.rs"))
        .unwrap()
        .file_comments
        .clear();
    let latest = base.clone();

    let changed = App::merge_external_session_changes(&mut current, &base, &latest);

    assert_eq!(changed, 0);
    assert!(file_comment_ids(&current).is_empty());
}

#[test]
fn should_apply_external_edit_when_comment_is_unchanged_locally() {
    let mut base = test_session();
    push_file_comment(&mut base, "same", "old");
    let mut current = base.clone();
    let mut latest = base.clone();
    latest
        .get_file_mut(&PathBuf::from("src/main.rs"))
        .unwrap()
        .file_comments[0]
        .content = "new".to_string();

    let changed = App::merge_external_session_changes(&mut current, &base, &latest);

    assert_eq!(changed, 1);
    assert_eq!(
        current
            .files
            .get(&PathBuf::from("src/main.rs"))
            .unwrap()
            .file_comments[0]
            .content,
        "new"
    );
}
