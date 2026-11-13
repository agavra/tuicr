//! The "Staged changes" and "Unstaged changes" rows in the review target
//! selector. Git always shows both, so the rows below them never shift and an
//! empty side opens an empty review that says so. Other backends show a row
//! only while its side has changes.

use tempfile::TempDir;

use crate::app::*;
use crate::model::FileStatus;
use crate::vcs::traits::{VcsChangeStatus, VcsType};

struct TestReviewsDir {
    _dir: TempDir,
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

/// A tree with a fixed set of staged and unstaged files. Status is derived
/// from them, so the selector and the loaders always agree about what each
/// side holds.
struct TreeVcs {
    info: VcsInfo,
    staged: Vec<DiffFile>,
    unstaged: Vec<DiffFile>,
    commits: Vec<CommitInfo>,
}

fn side(files: &[DiffFile]) -> Result<Vec<DiffFile>> {
    if files.is_empty() {
        Err(TuicrError::NoChanges)
    } else {
        Ok(files.to_vec())
    }
}

impl VcsBackend for TreeVcs {
    fn info(&self) -> &VcsInfo {
        &self.info
    }

    fn get_working_tree_diff(&self, _highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
        side(&[self.staged.clone(), self.unstaged.clone()].concat())
    }

    fn get_staged_diff(&self, _highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
        side(&self.staged)
    }

    fn get_unstaged_diff(&self, _highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
        side(&self.unstaged)
    }

    fn get_change_status(&self) -> Result<VcsChangeStatus> {
        Ok(VcsChangeStatus {
            staged: !self.staged.is_empty(),
            unstaged: !self.unstaged.is_empty(),
        })
    }

    fn get_recent_commits(&self, offset: usize, limit: usize) -> Result<Vec<CommitInfo>> {
        Ok(self
            .commits
            .iter()
            .skip(offset)
            .take(limit)
            .cloned()
            .collect())
    }

    fn fetch_context_lines(
        &self,
        _file_path: &Path,
        _file_status: FileStatus,
        _ref_commit: Option<&str>,
        _start_line: u32,
        _end_line: u32,
    ) -> Result<Vec<DiffLine>> {
        Ok(Vec::new())
    }

    fn file_line_count(
        &self,
        _file_path: &Path,
        _file_status: FileStatus,
        _ref_commit: Option<&str>,
    ) -> Result<u32> {
        Ok(0)
    }
}

fn diff_file(path: &str) -> DiffFile {
    DiffFile {
        old_path: None,
        new_path: Some(PathBuf::from(path)),
        status: FileStatus::Modified,
        hunks: Vec::new(),
        is_binary: false,
        is_too_large: false,
        is_commit_message: false,
        content_hash: 0,
    }
}

fn commit(id: &str) -> CommitInfo {
    CommitInfo {
        id: id.to_string(),
        short_id: id.to_string(),
        branch_name: None,
        summary: format!("commit {id}"),
        body: None,
        author: "test".to_string(),
        time: chrono::Utc::now(),
    }
}

/// A Normal-mode app over `vcs`, rooted in a fresh temp dir so no
/// `.tuicrignore` from the host leaks into the status probe.
fn build_app(vcs_type: VcsType, staged: Vec<DiffFile>, unstaged: Vec<DiffFile>) -> (App, TempDir) {
    let root = tempfile::tempdir().expect("failed to create repo root");
    let vcs_info = VcsInfo {
        root_path: root.path().to_path_buf(),
        head_commit: "head".to_string(),
        branch_name: Some("main".to_string()),
        vcs_type,
    };
    let session = ReviewSession::new(
        vcs_info.root_path.clone(),
        vcs_info.head_commit.clone(),
        vcs_info.branch_name.clone(),
        SessionDiffSource::WorkingTree,
    );
    let app = App::build(
        Box::new(TreeVcs {
            info: vcs_info.clone(),
            staged,
            unstaged,
            commits: vec![commit("c2"), commit("c1")],
        }),
        vcs_info,
        Theme::dark(),
        None,
        false,
        Vec::new(),
        session,
        DiffSource::WorkingTree,
        InputMode::Normal,
        Vec::new(),
        None,
        None,
    )
    .expect("failed to build test app");
    (app, root)
}

fn row_ids(app: &App) -> Vec<&str> {
    app.commit_list.iter().map(|c| c.id.as_str()).collect()
}

fn message(app: &App) -> Option<&str> {
    app.message.as_ref().map(|m| m.content.as_str())
}

#[test]
fn git_selector_shows_both_rows_on_a_clean_tree() {
    let _reviews = TestReviewsDir::new();
    let (mut app, _root) = build_app(VcsType::Git, vec![], vec![]);

    app.enter_target_selector(TargetTab::Local).unwrap();

    assert_eq!(app.input_mode, InputMode::CommitSelect);
    assert_eq!(
        row_ids(&app),
        [UNSTAGED_SELECTION_ID, STAGED_SELECTION_ID, "c2", "c1"],
        "both rows stay put even with nothing staged or unstaged"
    );
    assert_eq!(app.special_commit_count(), 2);
}

#[test]
fn git_selector_shows_both_rows_when_one_side_is_empty() {
    let _reviews = TestReviewsDir::new();
    let (mut app, _root) = build_app(VcsType::Git, vec![diff_file("staged.rs")], vec![]);

    app.enter_target_selector(TargetTab::Local).unwrap();

    assert_eq!(
        row_ids(&app),
        [UNSTAGED_SELECTION_ID, STAGED_SELECTION_ID, "c2", "c1"],
        "the empty unstaged side keeps its row"
    );
}

#[test]
fn mercurial_selector_shows_only_rows_with_changes() {
    let _reviews = TestReviewsDir::new();
    let (mut app, _root) = build_app(VcsType::Mercurial, vec![], vec![diff_file("edit.rs")]);

    app.enter_target_selector(TargetTab::Local).unwrap();

    assert_eq!(
        row_ids(&app),
        [UNSTAGED_SELECTION_ID, "c2", "c1"],
        "non-Git backends keep showing a row only while its side has changes"
    );
}

#[test]
fn mercurial_selector_shows_no_rows_on_a_clean_tree() {
    let _reviews = TestReviewsDir::new();
    let (mut app, _root) = build_app(VcsType::Mercurial, vec![], vec![]);

    app.enter_target_selector(TargetTab::Local).unwrap();

    assert_eq!(row_ids(&app), ["c2", "c1"]);
}

#[test]
fn row_status_widens_only_for_git() {
    let clean = VcsChangeStatus::default();
    let both = VcsChangeStatus {
        staged: true,
        unstaged: true,
    };
    assert_eq!(App::worktree_row_status(VcsType::Git, clean), both);
    for vcs_type in [VcsType::Mercurial, VcsType::Jujutsu, VcsType::File] {
        assert_eq!(
            App::worktree_row_status(vcs_type, clean),
            clean,
            "{vcs_type:?} must keep the real status"
        );
    }
}

/// Opens the selector and confirms rows `start..=end`.
fn pick_rows(app: &mut App, start: usize, end: usize) {
    app.enter_target_selector(TargetTab::Local).unwrap();
    app.commit_list_cursor = start;
    app.commit_selection_range = Some((start, end));
    app.confirm_commit_selection().unwrap();
}

#[test]
fn picking_the_empty_staged_row_opens_an_empty_review_that_says_so() {
    let _reviews = TestReviewsDir::new();
    let (mut app, _root) = build_app(VcsType::Git, vec![], vec![diff_file("edit.rs")]);

    // Rows: [Unstaged, Staged, c2, c1]
    pick_rows(&mut app, 1, 1);

    assert_eq!(app.input_mode, InputMode::Normal, "the selector closes");
    assert_eq!(app.diff_source, DiffSource::Staged);
    assert!(app.diff_files.is_empty());
    assert_eq!(app.session.diff_source, SessionDiffSource::Staged);
    assert_eq!(message(&app), Some("No staged changes"));
    assert_eq!(app.empty_review_placeholder(), Some("No staged changes"));
}

#[test]
fn picking_the_empty_unstaged_row_opens_an_empty_review_that_says_so() {
    let _reviews = TestReviewsDir::new();
    let (mut app, _root) = build_app(VcsType::Git, vec![diff_file("staged.rs")], vec![]);

    pick_rows(&mut app, 0, 0);

    assert_eq!(app.input_mode, InputMode::Normal);
    assert_eq!(app.diff_source, DiffSource::Unstaged);
    assert!(app.diff_files.is_empty());
    assert_eq!(message(&app), Some("No unstaged changes"));
}

#[test]
fn picking_both_empty_rows_opens_an_empty_review_that_says_so() {
    let _reviews = TestReviewsDir::new();
    let (mut app, _root) = build_app(VcsType::Git, vec![], vec![]);

    pick_rows(&mut app, 0, 1);

    assert_eq!(app.input_mode, InputMode::Normal);
    assert_eq!(app.diff_source, DiffSource::StagedAndUnstaged);
    assert!(app.diff_files.is_empty());
    assert_eq!(message(&app), Some("No staged or unstaged changes"));
}

#[test]
fn picking_a_row_with_changes_opens_it_without_a_message() {
    let _reviews = TestReviewsDir::new();
    let (mut app, _root) = build_app(VcsType::Git, vec![diff_file("staged.rs")], vec![]);

    pick_rows(&mut app, 1, 1);

    assert_eq!(app.diff_source, DiffSource::Staged);
    assert_eq!(app.diff_files.len(), 1);
    assert_eq!(message(&app), None);
    assert_eq!(app.empty_review_placeholder(), None);
}

/// `-r -w`, or picking staged and unstaged together with commits, puts the
/// rows in the inline pane. Narrowing that pane to an empty side shows an
/// empty diff rather than failing.
#[test]
fn narrowing_the_inline_pane_to_an_empty_side_shows_an_empty_diff() {
    let _reviews = TestReviewsDir::new();
    let (mut app, _root) = build_app(VcsType::Git, vec![], vec![diff_file("edit.rs")]);
    app.diff_source = DiffSource::StagedUnstagedAndCommits(vec!["c1".to_string()]);
    app.review_commits = vec![
        App::unstaged_commit_entry(),
        App::staged_commit_entry(),
        commit("c1"),
    ];
    app.commit_list = app.review_commits.clone();
    app.commit_selection_range = Some((1, 1));

    app.reload_inline_selection().unwrap();

    assert!(app.diff_files.is_empty());
    assert_eq!(app.commit_list.len(), 3, "the pane keeps every row");
}

/// Swaps the app's backend for one whose tree holds `staged` and `unstaged`,
/// as if the user had committed or staged in another shell.
fn change_tree(app: &mut App, staged: Vec<DiffFile>, unstaged: Vec<DiffFile>) {
    app.vcs = Box::new(TreeVcs {
        info: app.vcs_info.clone(),
        staged,
        unstaged,
        commits: vec![commit("c2"), commit("c1")],
    });
}

/// Opens the selector and returns the id of the row the cursor starts on.
fn starting_row(vcs_type: VcsType, staged: Vec<DiffFile>, unstaged: Vec<DiffFile>) -> String {
    let (mut app, _root) = build_app(vcs_type, staged, unstaged);
    app.enter_target_selector(TargetTab::Local).unwrap();
    app.commit_list[app.commit_list_cursor].id.clone()
}

/// Git's rows can be empty, so the cursor must skip them. Otherwise `Enter`
/// on a clean tree would open an empty review instead of the newest commit.
#[test]
fn selector_cursor_starts_on_the_first_row_with_something_to_review() {
    let _reviews = TestReviewsDir::new();
    let edit = || vec![diff_file("edit.rs")];

    assert_eq!(starting_row(VcsType::Git, vec![], vec![]), "c2");
    assert_eq!(
        starting_row(VcsType::Git, edit(), vec![]),
        STAGED_SELECTION_ID
    );
    assert_eq!(
        starting_row(VcsType::Git, vec![], edit()),
        UNSTAGED_SELECTION_ID
    );
    assert_eq!(
        starting_row(VcsType::Git, edit(), edit()),
        UNSTAGED_SELECTION_ID
    );
    assert_eq!(starting_row(VcsType::Mercurial, vec![], vec![]), "c2");
    assert_eq!(
        starting_row(VcsType::Mercurial, vec![], edit()),
        UNSTAGED_SELECTION_ID
    );
}

fn run_reload_command(app: &mut App) {
    app.input_mode = InputMode::Command;
    app.command_buffer = "e".to_string();
    crate::handler::handle_command_action(app, crate::input::Action::SubmitInput);
}

/// `:e` on a staged review whose side has emptied (the user committed) shows
/// the empty review and says why, rather than a red "Reload failed".
///
/// The session is not reconciled against the empty list, so the reviewed
/// mark survives for when the side fills again.
#[test]
fn reload_on_an_emptied_staged_review_shows_an_empty_review() {
    let _reviews = TestReviewsDir::new();
    let path = PathBuf::from("staged.rs");
    let (mut app, _root) = build_app(VcsType::Git, vec![diff_file("staged.rs")], vec![]);
    pick_rows(&mut app, 1, 1);
    assert_eq!(app.diff_files.len(), 1);
    app.session.files.get_mut(&path).unwrap().reviewed = true;

    change_tree(&mut app, vec![], vec![]);
    run_reload_command(&mut app);

    assert!(app.diff_files.is_empty());
    assert_eq!(app.diff_source, DiffSource::Staged);
    let shown = app
        .message
        .as_ref()
        .expect("a message explains the empty review");
    assert_eq!(shown.content, "No staged changes");
    assert_eq!(shown.message_type, MessageType::Info, "not an error");
    assert!(
        app.session.is_file_reviewed(&path),
        "an empty reload must not wipe the reviewed mark"
    );
    assert_eq!(
        app.empty_review_placeholder(),
        Some("No staged changes"),
        "the pane keeps saying why after the message bar clears"
    );
}

/// The same for an already empty review, which is where `:e` lands most
/// often: picking an empty row and then reloading.
#[test]
fn reload_on_an_empty_unstaged_review_stays_quietly_empty() {
    let _reviews = TestReviewsDir::new();
    let (mut app, _root) = build_app(VcsType::Git, vec![], vec![]);
    pick_rows(&mut app, 0, 0);

    run_reload_command(&mut app);

    assert!(app.diff_files.is_empty());
    let shown = app.message.as_ref().unwrap();
    assert_eq!(shown.content, "No unstaged changes");
    assert_eq!(shown.message_type, MessageType::Info);
}

/// Other sources still report an empty reload as a failure.
#[test]
fn reload_with_no_changes_on_a_working_tree_review_is_still_an_error() {
    let _reviews = TestReviewsDir::new();
    let (mut app, _root) = build_app(VcsType::Git, vec![], vec![]);
    app.diff_source = DiffSource::WorkingTree;

    run_reload_command(&mut app);

    let shown = app.message.as_ref().unwrap();
    assert_eq!(shown.message_type, MessageType::Error);
}

/// The selector records the real status, which is what dims an empty row.
#[test]
fn selector_records_the_real_status_for_dimming_empty_rows() {
    let _reviews = TestReviewsDir::new();
    let (mut app, _root) = build_app(VcsType::Git, vec![diff_file("staged.rs")], vec![]);

    app.enter_target_selector(TargetTab::Local).unwrap();

    assert!(
        app.is_empty_worktree_row(&app.commit_list[0]),
        "unstaged is empty"
    );
    assert!(
        !app.is_empty_worktree_row(&app.commit_list[1]),
        "staged has a file"
    );
    assert!(
        !app.is_empty_worktree_row(&app.commit_list[2]),
        "commits never dim"
    );
}
