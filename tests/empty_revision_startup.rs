use std::{fs, path::Path};

use git2::{Repository, Signature};
use tuicr::{
    TuicrError,
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

fn startup_options(working_tree: bool) -> AppStartupOptions<'static> {
    AppStartupOptions {
        revisions: Some("HEAD..HEAD~1"),
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

#[test]
fn empty_revision_range_is_no_changes_unless_working_tree_is_requested() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::init(temp.path()).unwrap();
    commit_file(&repo, "first\n", "first commit");
    commit_file(&repo, "second\n", "second commit");
    fs::write(temp.path().join("fixture.txt"), "working tree change\n").unwrap();
    drop(repo);

    // App::new discovers the repository from the process cwd. This is one test
    // in its own integration-test process, so the temporary cwd cannot race
    // other test binaries or parallel tests in this binary.
    let original_dir = std::env::current_dir().unwrap();
    std::env::set_current_dir(temp.path()).unwrap();

    let without_working_tree = App::new(Theme::dark(), None, false, startup_options(false));
    assert!(matches!(without_working_tree, Err(TuicrError::NoChanges)));

    let with_working_tree = App::new(Theme::dark(), None, false, startup_options(true));
    assert!(
        with_working_tree.is_ok(),
        "-w should open the working-tree diff for an empty commit range"
    );

    std::env::set_current_dir(original_dir).unwrap();
}
