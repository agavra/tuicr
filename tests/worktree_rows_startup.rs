//! Startup paths for the "Staged changes" and "Unstaged changes" rows in a Git
//! repo: the commit selector `tuicr` opens with, and the inline pane `-r -w`
//! opens with. Both rows are always present, and the selector cursor starts
//! on the first row with something to review.

use std::{fs, path::Path};

use git2::{Repository, Signature};
use tuicr::{
    app::{
        App, AppStartupOptions, CommitSelectionStart, InputMode, STAGED_SELECTION_ID,
        UNSTAGED_SELECTION_ID,
    },
    theme::Theme,
    vcs::{DiffWhitespaceMode, git::GitBackendPreference},
};

fn commit_file(repo: &Repository, content: &str, message: &str) -> String {
    fs::write(repo.workdir().unwrap().join("fixture.txt"), content).unwrap();
    stage(repo);
    let mut index = repo.index().unwrap();
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
    .unwrap()
    .to_string()
}

fn stage(repo: &Repository) {
    let mut index = repo.index().unwrap();
    index.add_path(Path::new("fixture.txt")).unwrap();
    index.write().unwrap();
}

fn options(revisions: Option<&str>, working_tree: bool) -> AppStartupOptions<'_> {
    AppStartupOptions {
        revisions,
        working_tree,
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
    }
}

fn open(revisions: Option<&str>, working_tree: bool) -> App {
    App::new(Theme::dark(), None, false, options(revisions, working_tree))
        .expect("startup should succeed")
}

fn ids(rows: &[tuicr::vcs::CommitInfo]) -> Vec<String> {
    rows.iter().map(|row| row.id.clone()).collect()
}

fn cursor_row(app: &App) -> String {
    app.commit_list[app.commit_list_cursor].id.clone()
}

/// One test, because `App::new` discovers the repository from the process
/// cwd. This file is its own integration-test process, so changing the cwd
/// cannot race any other test.
#[test]
fn git_startup_always_shows_both_rows_and_starts_on_content() {
    let temp = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    // Sessions created at startup must not reach the real data dir.
    // SAFETY: the only test in this process, set before any thread starts.
    unsafe { std::env::set_var("XDG_DATA_HOME", data.path()) };

    let repo = Repository::init(temp.path()).unwrap();
    let first = commit_file(&repo, "first\n", "first commit");
    let second = commit_file(&repo, "second\n", "second commit");
    let original_dir = std::env::current_dir().unwrap();
    std::env::set_current_dir(temp.path()).unwrap();

    // Clean tree: both rows, cursor on the newest commit.
    let app = open(None, false);
    assert_eq!(app.input_mode, InputMode::CommitSelect);
    assert_eq!(
        ids(&app.commit_list),
        [
            UNSTAGED_SELECTION_ID.to_string(),
            STAGED_SELECTION_ID.to_string(),
            second.clone(),
            first.clone(),
        ]
    );
    assert_eq!(
        cursor_row(&app),
        second,
        "Enter must still open the newest commit"
    );
    drop(app);

    // `-r HEAD -w` on a clean tree: the inline pane carries both rows too.
    let app = open(Some("HEAD"), true);
    assert_eq!(
        ids(&app.review_commits),
        [
            STAGED_SELECTION_ID.to_string(),
            UNSTAGED_SELECTION_ID.to_string(),
            second.clone(),
        ]
    );
    drop(app);

    // Staged only: cursor on the staged row.
    fs::write(temp.path().join("fixture.txt"), "staged\n").unwrap();
    stage(&repo);
    let app = open(None, false);
    assert_eq!(app.commit_list.len(), 4);
    assert_eq!(cursor_row(&app), STAGED_SELECTION_ID);
    drop(app);

    // Staged and unstaged: cursor on the unstaged row, the first one.
    fs::write(temp.path().join("fixture.txt"), "unstaged\n").unwrap();
    let app = open(None, false);
    assert_eq!(app.commit_list.len(), 4);
    assert_eq!(cursor_row(&app), UNSTAGED_SELECTION_ID);
    drop(app);

    std::env::set_current_dir(original_dir).unwrap();
}
