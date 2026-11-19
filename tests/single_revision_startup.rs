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

#[test]
fn should_select_the_only_commit_when_started_with_one_revision() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::init(temp.path()).unwrap();
    commit_file(&repo, "first\n", "first commit");
    commit_file(&repo, "second\n", "second commit");
    drop(repo);

    // App::new discovers the repository from the process cwd and writes its
    // session under the user's data dir. This is the only test in its own
    // process, so redirecting both cannot race anything else.
    let home = tempfile::tempdir().unwrap();
    let original_dir = std::env::current_dir().unwrap();
    std::env::set_current_dir(temp.path()).unwrap();
    // SAFETY: single-threaded test process; nothing else reads the environment.
    unsafe { std::env::set_var("HOME", home.path()) };

    let app = App::new(
        Theme::dark(),
        None,
        false,
        AppStartupOptions {
            revisions: Some("HEAD~1..HEAD"),
            working_tree: false,
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
            pr_current_branch: false,
            repo_url_override: None,
        },
    )
    .unwrap();
    std::env::set_current_dir(original_dir).unwrap();

    // The selector path selects the lone commit, which is what stamps new
    // comments with its sha. The pane itself stays hidden.
    assert_eq!(app.review_commits.len(), 1);
    assert_eq!(app.commit_selection_range, Some((0, 0)));
    assert!(!app.show_commit_selector);
}
