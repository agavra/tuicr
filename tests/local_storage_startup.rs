//! End-to-end startup wiring for repo-local session storage.
//!
//! Runs `App::new` in a real git repository with `local_storage` enabled —
//! the same path `main.rs` takes — and verifies the session lands in
//! `<repo>/.tuicr/reviews`, carries a comment added through the shared
//! `ReviewStore`, resumes from there on the next start, and is cleaned up
//! on exit. Lives in its own integration-test process because it changes
//! the process cwd, like `empty_revision_startup`.

use std::{fs, path::Path};

use git2::{Repository, Signature};
use tuicr::{
    app::{App, AppStartupOptions, CommitSelectionStart},
    theme::Theme,
    vcs::{DiffWhitespaceMode, git::GitBackendPreference},
};

fn commit_file(repo: &Repository, content: &str, message: &str) {
    fs::write(repo.workdir().unwrap().join("fixture.txt"), content).unwrap();

    let mut index = repo.index().unwrap();
    index.add_path(Path::new("fixture.txt")).unwrap();
    index.write().unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    let signature = Signature::now("tuicr test", "tuicr-test@example.invalid").unwrap();
    let parent = repo.head().ok().and_then(|head| head.peel_to_commit().ok());
    let parents = parent.iter().collect::<Vec<_>>();

    repo.commit(
        Some("HEAD"),
        &signature,
        &signature,
        message,
        &tree,
        &parents,
    )
    .unwrap();
}

fn startup_options() -> AppStartupOptions<'static> {
    AppStartupOptions {
        revisions: None,
        working_tree: true,
        path_filter: None,
        file_path: None,
        all_files: false,
        show_pr_checks: false,
        show_pr_comments: false,
        pr_comments_visibility: None,
        git_backend_preference: GitBackendPreference::Libgit2,
        diff_whitespace_mode: DiffWhitespaceMode::default(),
        commit_selection: CommitSelectionStart::All,
        pr_target: None,
        repo_url_override: None,
        local_storage: true,
    }
}

#[test]
fn local_storage_sessions_persist_resume_and_clean_up_in_the_repo_local_root() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::init(temp.path()).unwrap();
    commit_file(&repo, "first\n", "first commit");
    commit_file(&repo, "second\n", "second commit");
    fs::write(temp.path().join("fixture.txt"), "working tree change\n").unwrap();
    drop(repo);

    let original_dir = std::env::current_dir().unwrap();
    std::env::set_current_dir(temp.path()).unwrap();

    // First run: the ephemeral session file is created under
    // <repo>/.tuicr/reviews, exactly where main.rs's startup call puts it.
    let mut first = App::new(Theme::dark(), None, false, startup_options())
        .expect("working-tree review should open");
    let created = first
        .ensure_ephemeral_session_file()
        .expect("ephemeral session file");
    let repo_local = temp.path().join(".tuicr").join("reviews");
    let saved = created.expect("a fresh session creates its file");
    assert!(
        saved.starts_with(&repo_local),
        "session must save under <repo>/.tuicr/reviews, got {}",
        saved.display()
    );

    // The motivating workflow: the agent adds a comment from outside the
    // TUI (`tuicr review add`'s ReviewStore path) into the same repo-local
    // file the TUI owns.
    let store = tuicr::review_store::ReviewStore::with_reviews_dir(&repo_local);
    let session_ref = store
        .list_sessions_for_repo(temp.path())
        .expect("list repo-local sessions")
        .pop()
        .expect("the repo-local session is listed");
    store
        .add_comment(
            &session_ref.session_ref,
            tuicr::review_store::AddCommentRequest {
                target: tuicr::review_store::CommentTarget::Review,
                content: "persisted across restarts".to_string(),
                comment_type: tuicr::model::CommentType::Custom("note".to_string()),
                author: "agent".to_string(),
                commit_id: None,
            },
        )
        .expect("add comment via the store");

    // Second run with the same identity resumes it rather than starting
    // over, carrying the comment.
    let mut second =
        App::new(Theme::dark(), None, false, startup_options()).expect("reopen should succeed");
    second
        .ensure_ephemeral_session_file()
        .expect("ephemeral session file");
    assert_eq!(
        second.session.id, first.session.id,
        "resume must reattach to the repo-local session"
    );
    assert_eq!(
        second.session.review_comments.len(),
        1,
        "resume must carry the repo-local session's comment"
    );
    let sessions_dir = repo_local.join("sessions");
    assert_eq!(
        fs::read_dir(&sessions_dir).unwrap().count(),
        1,
        "resume must not fork a second session file"
    );

    // Exit cleanup: a commented session is pinned — the resumed run above
    // loaded it with its comment intact — so once the comments are cleared,
    // the app that created the file (`first`) removes it on quit. The
    // ephemeral set is per-process and consumed by the cleanup walk.
    store
        .clear_comments(
            &session_ref.session_ref,
            tuicr::model::ClearScope::CommentsOnly,
        )
        .expect("clear comments");
    assert_eq!(
        first.cleanup_empty_ephemeral_sessions().expect("cleanup"),
        1,
        "the now-empty ephemeral session is removed"
    );
    assert!(!saved.exists(), "the session file is gone");
    second
        .clear_active_session_marker()
        .expect("clear active marker");
    let active: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(repo_local.join("active_sessions.json")).unwrap())
            .unwrap();
    assert_eq!(
        active["sessions"].as_array().map(Vec::len),
        Some(0),
        "our pid's entry must be cleared"
    );

    std::env::set_current_dir(original_dir).unwrap();
}
