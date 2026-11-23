use std::path::PathBuf;

use crate::app::{App, DiffSource, InputMode, PullRequestDiffSource};
use crate::error::TuicrError;
use crate::forge::traits::{ForgeRepository, PrSessionKey};
use crate::model::{Comment, CommentType, DiffFile, ReviewSession, SessionDiffSource};
use crate::persistence::storage;
use crate::theme::Theme;
use crate::vcs::traits::VcsType;
use crate::vcs::{PrNoopVcs, VcsInfo};

struct TestReviewsDir {
    dir: tempfile::TempDir,
}

impl TestReviewsDir {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        storage::set_test_reviews_dir(Some(dir.path().to_path_buf()));
        Self { dir }
    }
}

impl Drop for TestReviewsDir {
    fn drop(&mut self) {
        storage::set_test_reviews_dir(None);
    }
}

fn ghe_key() -> PrSessionKey {
    PrSessionKey::new(
        ForgeRepository::github("ghe.internal", "owner", "repo"),
        42,
        "abc1234567890",
    )
}

fn pr_session(key: &PrSessionKey, note: &str) -> ReviewSession {
    let mut session = ReviewSession::new(
        PathBuf::from("forge:ghe.internal/owner/repo"),
        key.head_sha.clone(),
        Some("feature".to_string()),
        SessionDiffSource::PullRequest,
    );
    session.pr_session_key = Some(key.clone());
    session.review_comments.push(Comment::new(
        note.to_string(),
        CommentType::from_id("note"),
        None,
    ));
    session
}

fn build_pr_app(session: ReviewSession) -> App {
    let key = session.pr_session_key.clone().unwrap();
    let pr = PullRequestDiffSource {
        key: key.clone(),
        base_sha: "def0987654321".to_string(),
        title: "Add panel".to_string(),
        url: "https://ghe.internal/owner/repo/pull/42".to_string(),
        head_ref_name: "feature".to_string(),
        base_ref_name: "main".to_string(),
        state: "OPEN".to_string(),
        closed: false,
        merged: false,
    };
    let vcs_info = VcsInfo {
        root_path: PathBuf::from("forge:ghe.internal/owner/repo"),
        head_commit: key.head_sha.clone(),
        branch_name: Some("feature".to_string()),
        vcs_type: VcsType::File,
    };
    App::build(
        Box::new(PrNoopVcs::new(vcs_info.clone())),
        vcs_info,
        Theme::dark(),
        None,
        false,
        Vec::<DiffFile>::new(),
        session,
        DiffSource::PullRequest(Box::new(pr)),
        InputMode::Normal,
        Vec::new(),
        None,
        None,
    )
    .expect("build pr app")
}

// T14b: an App holding the legacy file as its cached path must not silently
// switch to a host-aware file that appeared meanwhile, and must write neither.
#[test]
fn should_refuse_save_when_new_file_appears_for_cached_legacy_path() {
    let reviews = TestReviewsDir::new();
    let key = ghe_key();
    let session = pr_session(&key, "legacy draft");

    // legacy file at the hostless hash, as an older tuicr left it
    let slug = crate::slug::Slug::from(&key);
    let crate::slug::Slug::Pr(slug) = slug else {
        unreachable!()
    };
    let legacy_hash = {
        let mut hasher = crate::hash::Fnv1aHasher::new();
        hasher.write(format!("pr|{}|{}", slug.hostless(), key.head_sha).as_bytes());
        format!("{:016x}", hasher.finish())
    };
    let sessions = reviews.dir.path().join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    let legacy = sessions.join(format!("{legacy_hash}.json"));
    std::fs::write(&legacy, serde_json::to_string_pretty(&session).unwrap()).unwrap();

    let mut app = build_pr_app(session.clone());
    assert_eq!(app.session_path.as_deref(), Some(legacy.as_path()));
    assert_eq!(
        app.session_slug().as_deref(),
        Some("gh:owner/repo/pr/42@ghe.internal")
    );

    // another TUI creates the host-aware file
    let mut other = pr_session(&key, "other tui draft");
    other.review_comments[0].content = "other tui draft".to_string();
    let new_path = {
        let mut hasher = crate::hash::Fnv1aHasher::new();
        hasher.write(format!("pr2|gh|ghe.internal|owner/repo|42|{}", key.head_sha).as_bytes());
        sessions.join(format!("{:016x}.json", hasher.finish()))
    };
    std::fs::write(&new_path, serde_json::to_string_pretty(&other).unwrap()).unwrap();
    let legacy_before = std::fs::read(&legacy).unwrap();
    let new_before = std::fs::read(&new_path).unwrap();

    app.session.review_comments.push(Comment::new(
        "app draft".to_string(),
        CommentType::from_id("note"),
        None,
    ));
    app.dirty = true;
    let err = app.save_current_session_merging_external().unwrap_err();

    assert!(matches!(err, TuicrError::SessionTargetChanged(_)), "{err}");
    assert_eq!(std::fs::read(&legacy).unwrap(), legacy_before);
    assert_eq!(std::fs::read(&new_path).unwrap(), new_before);

    // a fresh App resolves the new file
    let fresh = build_pr_app(session);
    assert_eq!(fresh.session_path.as_deref(), Some(new_path.as_path()));
}

// T19 at App level: a session saved under one host spelling is reattached
// when the PR is reopened under another, and the next autosave keeps it.
#[test]
fn should_reattach_session_saved_under_other_host_spelling() {
    let _reviews = TestReviewsDir::new();
    let saved_key = PrSessionKey::new(
        ForgeRepository::github("GHE.Internal", "owner", "repo"),
        42,
        "abc1234567890",
    );
    storage::save_session(&pr_session(&saved_key, "precious draft")).unwrap();

    let opened_key = ghe_key();
    let opened = crate::forge::pr_open::OpenedPullRequest {
        details: test_details(&opened_key),
        diff_files: Vec::new(),
        session: pr_session(&opened_key, "fresh"),
        key: opened_key.clone(),
        commits: Vec::new(),
        review_metadata: Default::default(),
        pr_info: crate::forge::traits::PullRequestInfo::from_details(test_details(&opened_key)),
    };
    let opened = App::opened_pr_with_persisted_session(opened).unwrap();
    let texts: Vec<_> = opened
        .session
        .review_comments
        .iter()
        .map(|c| c.content.clone())
        .collect();
    assert_eq!(texts, vec!["precious draft"]);

    let mut app = build_pr_app(opened.session);
    app.session.review_comments.push(Comment::new(
        "new".to_string(),
        CommentType::from_id("note"),
        None,
    ));
    app.dirty = true;
    let path = app.save_current_session_merging_external().unwrap();
    let on_disk = storage::load_session(&path).unwrap();
    let texts: Vec<_> = on_disk
        .review_comments
        .iter()
        .map(|c| c.content.clone())
        .collect();
    assert_eq!(texts, vec!["precious draft", "new"]);
}

fn test_details(key: &PrSessionKey) -> crate::forge::traits::PullRequestDetails {
    crate::forge::traits::PullRequestDetails {
        repository: key.repository.clone(),
        number: key.number,
        title: "Add panel".to_string(),
        url: "https://ghe.internal/owner/repo/pull/42".to_string(),
        state: "OPEN".to_string(),
        is_draft: false,
        author: None,
        head_ref_name: "feature".to_string(),
        base_ref_name: "main".to_string(),
        head_sha: key.head_sha.clone(),
        base_sha: "def0987654321".to_string(),
        body: String::new(),
        updated_at: None,
        closed: false,
        merged_at: None,
        diff_start_sha: None,
    }
}

// SessionTargetChanged recovery: a same-head reload re-resolves the cached
// path, so the next autosave lands in the new file and keeps both drafts.
#[test]
fn should_reresolve_session_path_on_same_head_reload() {
    let reviews = TestReviewsDir::new();
    let key = ghe_key();
    let session = pr_session(&key, "legacy draft");
    let sessions = reviews.dir.path().join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    let legacy_hash = {
        let crate::slug::Slug::Pr(slug) = crate::slug::Slug::from(&key) else {
            unreachable!()
        };
        let mut hasher = crate::hash::Fnv1aHasher::new();
        hasher.write(format!("pr|{}|{}", slug.hostless(), key.head_sha).as_bytes());
        format!("{:016x}", hasher.finish())
    };
    let legacy = sessions.join(format!("{legacy_hash}.json"));
    std::fs::write(&legacy, serde_json::to_string_pretty(&session).unwrap()).unwrap();
    let mut app = build_pr_app(session);
    assert_eq!(app.session_path.as_deref(), Some(legacy.as_path()));

    let other = pr_session(&key, "other tui draft");
    let new_path = {
        let mut hasher = crate::hash::Fnv1aHasher::new();
        hasher.write(format!("pr2|gh|ghe.internal|owner/repo|42|{}", key.head_sha).as_bytes());
        sessions.join(format!("{:016x}.json", hasher.finish()))
    };
    std::fs::write(&new_path, serde_json::to_string_pretty(&other).unwrap()).unwrap();

    app.session.review_comments.push(Comment::new(
        "app draft".to_string(),
        CommentType::from_id("note"),
        None,
    ));
    app.dirty = true;
    assert!(app.save_current_session_merging_external().is_err());

    app.apply_same_head_pr_reload(crate::forge::pr_open::OpenedPullRequest {
        details: test_details(&key),
        diff_files: Vec::new(),
        session: app.session.clone(),
        key: key.clone(),
        commits: Vec::new(),
        review_metadata: Default::default(),
        pr_info: crate::forge::traits::PullRequestInfo::from_details(test_details(&key)),
    });
    assert_eq!(app.session_path.as_deref(), Some(new_path.as_path()));
    assert!(app.dirty);
    let path = app.save_current_session_merging_external().unwrap();
    assert_eq!(path, new_path);
    let texts: Vec<_> = storage::load_session(&path)
        .unwrap()
        .review_comments
        .iter()
        .map(|c| c.content.clone())
        .collect();
    assert!(texts.contains(&"app draft".to_string()), "{texts:?}");
    assert!(texts.contains(&"other tui draft".to_string()), "{texts:?}");
}

// The registry entry must follow the re-resolved path before any save.
#[test]
fn should_keep_reloaded_pr_session_path_active_before_autosave() {
    let reviews = TestReviewsDir::new();
    let key = ghe_key();
    let session = pr_session(&key, "legacy draft");
    let sessions = reviews.dir.path().join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    let legacy_hash = {
        let crate::slug::Slug::Pr(slug) = crate::slug::Slug::from(&key) else {
            unreachable!()
        };
        let mut hasher = crate::hash::Fnv1aHasher::new();
        hasher.write(format!("pr|{}|{}", slug.hostless(), key.head_sha).as_bytes());
        format!("{:016x}", hasher.finish())
    };
    let legacy = sessions.join(format!("{legacy_hash}.json"));
    std::fs::write(&legacy, serde_json::to_string_pretty(&session).unwrap()).unwrap();
    let mut app = build_pr_app(session);
    app.ensure_ephemeral_session_file().unwrap();
    assert!(
        storage::active_session_paths_in_dir(reviews.dir.path())
            .unwrap()
            .contains(&storage::normalize_path_for_comparison(&legacy))
    );
    assert_eq!(app.session_path.as_deref(), Some(legacy.as_path()));

    let other = pr_session(&key, "other tui draft");
    let new_path = {
        let mut hasher = crate::hash::Fnv1aHasher::new();
        hasher.write(format!("pr2|gh|ghe.internal|owner/repo|42|{}", key.head_sha).as_bytes());
        sessions.join(format!("{:016x}.json", hasher.finish()))
    };
    let legacy_bytes = std::fs::read(&legacy).unwrap();
    std::fs::write(&legacy, "{temporarily corrupt").unwrap();
    assert_eq!(storage::save_session(&other).unwrap(), new_path);
    std::fs::write(&legacy, legacy_bytes).unwrap();

    app.session.review_comments.push(Comment::new(
        "app draft".to_string(),
        CommentType::from_id("note"),
        None,
    ));
    app.dirty = true;
    assert!(app.save_current_session_merging_external().is_err());

    app.apply_same_head_pr_reload(crate::forge::pr_open::OpenedPullRequest {
        details: test_details(&key),
        diff_files: Vec::new(),
        session: app.session.clone(),
        key: key.clone(),
        commits: Vec::new(),
        review_metadata: Default::default(),
        pr_info: crate::forge::traits::PullRequestInfo::from_details(test_details(&key)),
    });
    assert_eq!(app.session_path.as_deref(), Some(new_path.as_path()));
    assert!(app.dirty);
    // No save after :e: this must still be protected as the open review.
    let store = crate::review_store::ReviewStore::with_reviews_dir(reviews.dir.path());
    let deletion = store
        .delete_review(&crate::review_store::SessionRef::from_path(
            new_path.clone(),
        ))
        .unwrap();
    assert_eq!(deletion, storage::DeleteOutcome::Active);
    let path = app.save_current_session_merging_external().unwrap();
    assert_eq!(path, new_path);
    let texts: Vec<_> = storage::load_session(&path)
        .unwrap()
        .review_comments
        .iter()
        .map(|c| c.content.clone())
        .collect();
    assert!(texts.contains(&"app draft".to_string()), "{texts:?}");
    assert!(texts.contains(&"other tui draft".to_string()), "{texts:?}");
}
