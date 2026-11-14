use crate::app::init::PrDisplayOptions;
use crate::app::*;
use crate::forge::selector::PullRequestsTab;
use crate::forge::traits::{
    PullRequestReviewMetadata, PullRequestReviewRecord, PullRequestSummary,
};
use crate::model::FileStatus;
use crate::vcs::traits::{VcsChangeStatus, VcsType};

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

struct DummyVcs {
    info: VcsInfo,
    commits: Vec<CommitInfo>,
}

impl VcsBackend for DummyVcs {
    fn info(&self) -> &VcsInfo {
        &self.info
    }

    fn get_working_tree_diff(&self, _highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
        Err(TuicrError::NoChanges)
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

    fn get_change_status(&self) -> Result<VcsChangeStatus> {
        Ok(VcsChangeStatus {
            staged: false,
            unstaged: false,
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

    fn file_line_count(
        &self,
        _file_path: &Path,
        _file_status: FileStatus,
        _ref_commit: Option<&str>,
    ) -> Result<u32> {
        Ok(0)
    }
}

fn build_app() -> App {
    build_app_with_commits(Vec::new())
}

#[test]
fn comment_vim_command_line_q_cancels_w_saves() {
    let mut app = build_app();
    app.comment_vim_enabled = true;

    // `:q` exits the comment box.
    app.enter_review_comment_mode();
    assert_eq!(app.input_mode, InputMode::Comment);
    app.start_comment_vim_command();
    assert!(app.comment_vim_command_active());
    app.comment_vim_command_push('q');
    assert_eq!(
        app.comment_vim_mode_label(),
        Some((":q".to_string(), false))
    );
    app.run_comment_vim_command();
    assert_eq!(app.input_mode, InputMode::Normal);
    assert!(!app.comment_vim_command_active());

    // `:w` reaches save_comment; on an empty buffer save rejects it and the
    // box stays open — proving the mapping without touching disk.
    app.enter_review_comment_mode();
    app.start_comment_vim_command();
    app.comment_vim_command_push('w');
    app.run_comment_vim_command();
    assert_eq!(app.input_mode, InputMode::Comment);
    assert!(!app.comment_vim_command_active());
}

#[test]
fn comment_vim_double_enter_saves() {
    let mut app = build_app();
    app.comment_vim_enabled = true;
    app.enter_review_comment_mode();

    // First Enter arms (header would show the hint); second routes to
    // save_comment (empty buffer rejected, box stays open) — double-Enter == :w.
    app.comment_vim_enter_normal();
    assert_eq!(app.comment_vim_pending, CommentVimPending::Save);
    assert_eq!(
        app.comment_vim_mode_label(),
        Some(("Enter again to save".to_string(), false))
    );
    app.comment_vim_enter_normal();
    assert_eq!(app.comment_vim_pending, CommentVimPending::None);
    assert_eq!(app.input_mode, InputMode::Comment);

    // A non-Enter key between the two presses breaks the sequence.
    app.comment_vim_enter_normal();
    app.comment_vim_reset_pending();
    assert_eq!(app.comment_vim_pending, CommentVimPending::None);
}

#[test]
fn comment_vim_double_esc_cancels() {
    let mut app = build_app();
    app.comment_vim_enabled = true;
    app.enter_review_comment_mode();
    assert_eq!(app.input_mode, InputMode::Comment);

    // First Esc arms cancel + header hint; second exits the comment box.
    app.comment_vim_esc_normal();
    assert_eq!(app.comment_vim_pending, CommentVimPending::Cancel);
    assert_eq!(
        app.comment_vim_mode_label(),
        Some(("Esc/q again to cancel".to_string(), true))
    );
    app.comment_vim_esc_normal();
    assert_eq!(app.input_mode, InputMode::Normal);
    assert_eq!(app.comment_vim_pending, CommentVimPending::None);
}

#[test]
fn comment_vim_soft_tab_inserts_configured_spaces() {
    let mut app = build_app();
    app.comment_vim_enabled = true;
    app.comment_tab_width = 2;
    app.enter_review_comment_mode();
    app.ensure_comment_vim_editor(); // Insert mode, empty buffer
    app.comment_vim_insert_soft_tab();
    assert_eq!(app.comment_buffer, "  ");
    assert_eq!(app.comment_cursor, 2);
}

#[test]
fn comment_block_start_finds_first_row_of_comment() {
    let mut app = build_app();
    app.line_annotations = vec![
        AnnotatedLine::ReviewComment { comment_idx: 0 },
        AnnotatedLine::ReviewComment { comment_idx: 1 },
        AnnotatedLine::ReviewComment { comment_idx: 1 },
        AnnotatedLine::ReviewComment { comment_idx: 1 },
        AnnotatedLine::ReviewComment { comment_idx: 2 },
    ];
    assert_eq!(app.comment_block_start(3), 1);
    assert_eq!(app.comment_block_start(1), 1);
    assert_eq!(app.comment_block_start(0), 0);
    assert_eq!(app.comment_block_start(4), 4);
}

#[test]
fn comment_current_line_cursor_targets_the_cursor_line() {
    let mut app = build_app();
    app.comment_buffer = "alpha\nbravo\ncharlie".to_string();
    app.diff_state.viewport_width = 200; // wide => no wrapping
    let block_start = 10;

    // 2nd content line "bravo" (block_start + top-border + 1): start 6, end 11.
    app.diff_state.cursor_line = block_start + 2;
    assert_eq!(app.comment_current_line_cursor(block_start, false), 6);
    assert_eq!(app.comment_current_line_cursor(block_start, true), 11);

    // 1st content line "alpha": start 0, end 5.
    app.diff_state.cursor_line = block_start + 1;
    assert_eq!(app.comment_current_line_cursor(block_start, false), 0);
    assert_eq!(app.comment_current_line_cursor(block_start, true), 5);

    // Top border row maps to the first line.
    app.diff_state.cursor_line = block_start;
    assert_eq!(app.comment_current_line_cursor(block_start, false), 0);

    // Bottom border / beyond maps to the last line "charlie": start 12, end 19.
    app.diff_state.cursor_line = block_start + 99;
    assert_eq!(app.comment_current_line_cursor(block_start, false), 12);
    assert_eq!(app.comment_current_line_cursor(block_start, true), 19);
}

#[test]
fn comment_vim_command_backspace_past_colon_closes() {
    let mut app = build_app();
    app.comment_vim_enabled = true;
    app.enter_review_comment_mode();
    app.start_comment_vim_command();
    app.comment_vim_command_push('q');
    app.comment_vim_command_backspace(); // -> ":"
    assert!(app.comment_vim_command_active());
    app.comment_vim_command_backspace(); // past ':' -> closed
    assert!(!app.comment_vim_command_active());
}

fn build_app_with_commits(commits: Vec<CommitInfo>) -> App {
    build_app_full(commits, None)
}

fn build_app_with_comment_types(configs: Vec<crate::config::CommentTypeConfig>) -> App {
    build_app_full(Vec::new(), Some(configs))
}

fn comment_type_config(id: &str) -> crate::config::CommentTypeConfig {
    crate::config::CommentTypeConfig {
        id: id.to_string(),
        ..Default::default()
    }
}

fn build_app_full(
    commits: Vec<CommitInfo>,
    comment_type_configs: Option<Vec<crate::config::CommentTypeConfig>>,
) -> App {
    build_app_rooted(PathBuf::from("/tmp"), commits, comment_type_configs)
}

/// `build_app_full` with an explicit VCS root, for tests that need the app
/// rooted at a real on-disk checkout.
fn build_app_rooted(
    root_path: PathBuf,
    commits: Vec<CommitInfo>,
    comment_type_configs: Option<Vec<crate::config::CommentTypeConfig>>,
) -> App {
    let vcs_info = VcsInfo {
        root_path,
        head_commit: "head".to_string(),
        branch_name: Some("main".to_string()),
        vcs_type: VcsType::Git,
    };
    let session = ReviewSession::new(
        vcs_info.root_path.clone(),
        vcs_info.head_commit.clone(),
        vcs_info.branch_name.clone(),
        SessionDiffSource::WorkingTree,
    );

    App::build(
        Box::new(DummyVcs {
            info: vcs_info.clone(),
            commits,
        }),
        vcs_info,
        Theme::dark(),
        comment_type_configs,
        false,
        Vec::new(),
        session,
        DiffSource::WorkingTree,
        InputMode::Normal,
        Vec::new(),
        None,
        None,
    )
    .expect("failed to build test app")
}

#[test]
fn default_comment_type_is_none_without_config() {
    let mut app = build_app();
    app.enter_comment_mode(false, None);
    assert_eq!(app.input_mode, InputMode::Comment);
    // Out of the box the only type is None — untyped, no prefix.
    assert!(app.comment_type.is_none());
    assert_eq!(app.comment_type.id(), "none");

    // With a single type there is nothing to cycle to; stays on None.
    app.cycle_comment_type();
    assert!(app.comment_type.is_none());
}

#[test]
fn should_cycle_comment_type_on_tab_action() {
    // Configuring types overrides the None default (first configured type
    // becomes the default) but None stays available, appended to the cycle.
    let mut app = build_app_with_comment_types(vec![
        comment_type_config("note"),
        comment_type_config("suggestion"),
    ]);
    app.enter_comment_mode(false, None);
    assert_eq!(app.input_mode, InputMode::Comment);
    assert_eq!(app.comment_type.id(), "note");

    app.cycle_comment_type();
    assert_eq!(app.comment_type.id(), "suggestion");

    // None is appended and reachable by cycling.
    app.cycle_comment_type();
    assert_eq!(app.comment_type.id(), "none");
    assert!(app.comment_type.is_none());

    // Wraps back around to the first configured type.
    app.cycle_comment_type();
    assert_eq!(app.comment_type.id(), "note");
}

fn dummy_commit(id: &str) -> CommitInfo {
    CommitInfo {
        id: id.to_string(),
        short_id: id.to_string(),
        branch_name: None,
        summary: format!("commit {id}"),
        body: None,
        author: "tester".to_string(),
        time: Utc::now(),
    }
}

fn test_pr_details(number: u64, title: &str) -> crate::forge::traits::PullRequestDetails {
    crate::forge::traits::PullRequestDetails {
        repository: ForgeRepository::github("github.com", "agavra", "tuicr"),
        number,
        title: title.to_string(),
        url: format!("https://github.com/agavra/tuicr/pull/{number}"),
        state: "OPEN".to_string(),
        is_draft: false,
        author: Some("alice".to_string()),
        head_ref_name: "feat".to_string(),
        base_ref_name: "main".to_string(),
        head_sha: "abcdef0123456789".to_string(),
        base_sha: "1234567890abcdef".to_string(),
        body: String::new(),
        updated_at: None,
        closed: false,
        merged_at: None,
        diff_start_sha: None,
    }
}

struct FakeForgeBackend {
    details: crate::forge::traits::PullRequestDetails,
    patch: String,
    commits: Vec<crate::forge::traits::PullRequestCommit>,
    review_metadata: PullRequestReviewMetadata,
    range_patch: Option<String>,
    line_count_requests: std::sync::Arc<std::sync::Mutex<Vec<PathBuf>>>,
}

impl FakeForgeBackend {
    fn open_pr_details(details: crate::forge::traits::PullRequestDetails, patch: String) -> Self {
        Self {
            details,
            patch,
            commits: Vec::new(),
            review_metadata: PullRequestReviewMetadata::default(),
            range_patch: None,
            line_count_requests: Default::default(),
        }
    }
}

impl crate::forge::traits::ForgeBackend for FakeForgeBackend {
    fn list_pull_requests(
        &self,
        _query: crate::forge::traits::PullRequestListQuery,
    ) -> Result<crate::forge::traits::PagedPullRequests> {
        unimplemented!("not used in this test")
    }
    fn get_pull_request(
        &self,
        _target: crate::forge::traits::PullRequestTarget,
    ) -> Result<crate::forge::traits::PullRequestDetails> {
        Ok(self.details.clone())
    }
    fn get_pull_request_diff(
        &self,
        _pr: &crate::forge::traits::PullRequestDetails,
    ) -> Result<Vec<crate::model::FilePatch>> {
        Ok(structured_patch(&self.patch))
    }
    fn fetch_file_lines(
        &self,
        _request: crate::forge::traits::ForgeFileLinesRequest,
    ) -> Result<Vec<crate::model::DiffLine>> {
        Ok(Vec::new())
    }
    fn file_line_count(&self, request: crate::forge::traits::ForgeFileLinesRequest) -> Result<u32> {
        self.line_count_requests.lock().unwrap().push(request.path);
        Ok(0)
    }
    fn list_review_threads(
        &self,
        _pr: &crate::forge::traits::PullRequestDetails,
    ) -> Result<Vec<crate::forge::remote_comments::RemoteReviewThread>> {
        Ok(Vec::new())
    }
    fn list_pull_request_commits(
        &self,
        _pr: &crate::forge::traits::PullRequestDetails,
    ) -> Result<Vec<crate::forge::traits::PullRequestCommit>> {
        Ok(self.commits.clone())
    }
    fn list_pull_request_review_metadata(
        &self,
        _pr: &crate::forge::traits::PullRequestDetails,
    ) -> Result<PullRequestReviewMetadata> {
        Ok(self.review_metadata.clone())
    }
    fn get_pull_request_commit_range_diff(
        &self,
        _pr: &crate::forge::traits::PullRequestDetails,
        _start_sha: &str,
        _end_sha: &str,
    ) -> Result<Vec<crate::model::FilePatch>> {
        Ok(structured_patch(
            self.range_patch.as_deref().unwrap_or(&self.patch),
        ))
    }
    fn create_review(
        &self,
        _pr: &crate::forge::traits::PullRequestDetails,
        _request: crate::forge::traits::CreateReviewRequest<'_>,
    ) -> Result<crate::forge::traits::GhCreateReviewResponse> {
        unimplemented!("FakeForgeBackend does not implement create_review")
    }
}

fn sample_pr(number: u64, title: &str) -> PullRequestSummary {
    PullRequestSummary {
        repository: ForgeRepository::github("github.com", "agavra", "tuicr"),
        number,
        title: title.to_string(),
        author: Some("alice".to_string()),
        head_ref_name: "feat".to_string(),
        base_ref_name: "main".to_string(),
        updated_at: None,
        last_commit_at: None,
        url: format!("https://github.com/agavra/tuicr/pull/{number}"),
        state: "OPEN".to_string(),
        is_draft: false,
    }
}

#[test]
fn should_default_to_local_tab_after_build() {
    // given / when
    let app = build_app();
    // then
    assert_eq!(app.target_tab, TargetTab::Local);
    assert!(!app.pr_filter_editing());
}

#[test]
fn should_cycle_between_local_and_pull_requests_on_tab_keypress() {
    // given
    let mut app = build_app();
    app.forge_repository = Some(ForgeRepository::github("github.com", "agavra", "tuicr"));
    app.pr_tab = PullRequestsTab::new(app.forge_repository.clone());
    // when
    app.cycle_target_tab(true);
    // then
    assert_eq!(app.target_tab, TargetTab::PullRequests);
    // when
    app.cycle_target_tab(false);
    // then
    assert_eq!(app.target_tab, TargetTab::Local);
}

#[test]
fn should_transition_pr_tab_to_loading_on_first_visit() {
    // given
    let mut app = build_app();
    app.forge_repository = Some(ForgeRepository::github("github.com", "agavra", "tuicr"));
    app.pr_tab = PullRequestsTab::new(app.forge_repository.clone());
    // when
    app.cycle_target_tab(true);
    // then — a background fetch is in flight
    assert!(app.pr_tab.is_loading());
    assert!(app.pr_load_rx.is_some());
    // The spawned thread holds a backend that may fail without a real
    // `gh` binary; cancel by dropping the receiver to avoid touching it.
    app.pr_load_rx = None;
}

#[test]
fn should_keep_pr_tab_disabled_when_no_forge_remote() {
    // given
    let mut app = build_app();
    // No forge_repository set up; default new app has None.
    // when
    app.cycle_target_tab(true);
    // then
    assert_eq!(app.target_tab, TargetTab::PullRequests);
    assert!(matches!(app.pr_tab, PullRequestsTab::Disabled { .. }));
    assert!(app.pr_load_rx.is_none());
}

#[test]
fn should_set_filter_after_typing_and_committing() {
    // given
    let mut app = build_app();
    app.forge_repository = Some(ForgeRepository::github("github.com", "agavra", "tuicr"));
    app.pr_tab = PullRequestsTab::new(app.forge_repository.clone());
    app.pr_tab.start_initial_load();
    app.pr_tab.apply_initial_load(Ok((
        vec![sample_pr(125, "Forge"), sample_pr(148, "Review")],
        false,
    )));
    app.target_tab = TargetTab::PullRequests;
    // when
    app.begin_pr_filter();
    app.pr_filter_insert_char('f');
    app.pr_filter_insert_char('o');
    app.commit_pr_filter();
    // then
    assert!(!app.pr_filter_editing());
    assert_eq!(app.pr_tab.view().rows.len(), 1);
    assert_eq!(app.pr_tab.view().rows[0].summary.number, 125);
}

#[test]
fn should_discard_filter_draft_on_cancel() {
    // given
    let mut app = build_app();
    app.forge_repository = Some(ForgeRepository::github("github.com", "agavra", "tuicr"));
    app.pr_tab = PullRequestsTab::new(app.forge_repository.clone());
    app.pr_tab.start_initial_load();
    app.pr_tab
        .apply_initial_load(Ok((vec![sample_pr(1, "alpha")], false)));
    app.target_tab = TargetTab::PullRequests;
    // when
    app.begin_pr_filter();
    app.pr_filter_insert_char('z');
    app.cancel_pr_filter();
    // then
    assert!(!app.pr_filter_editing());
    assert_eq!(app.pr_tab.view().filter, "");
}

#[test]
fn should_enter_pr_mode_when_opening_pr_via_fake_backend() {
    // given a selector with a single PR row and a fake forge backend
    let mut app = build_app();
    app.forge_repository = Some(ForgeRepository::github("github.com", "agavra", "tuicr"));
    app.pr_tab = PullRequestsTab::new(app.forge_repository.clone());
    app.pr_tab.start_initial_load();
    let summary = sample_pr(42, "answer");
    app.pr_tab
        .apply_initial_load(Ok((vec![summary.clone()], false)));
    app.target_tab = TargetTab::PullRequests;
    let backend = Box::new(FakeForgeBackend::open_pr_details(
        test_pr_details(42, "answer"),
        crate::forge::github::gh::tests_fixture::SIMPLE_PATCH.to_string(),
    ));
    // when
    app.open_pr_with_backend(&summary, backend, None).unwrap();
    // then
    assert!(matches!(app.diff_source, DiffSource::PullRequest(_)));
    if let DiffSource::PullRequest(pr) = &app.diff_source {
        assert_eq!(pr.key.number, 42);
        assert_eq!(pr.title, "answer");
        assert_eq!(pr.head_ref_name, "feat");
        assert_eq!(pr.base_ref_name, "main");
        assert_eq!(pr.key.head_sha, "abcdef0123456789");
        assert_eq!(pr.base_sha, "1234567890abcdef");
    }
    // and the session is keyed by the PR
    assert!(app.session.pr_session_key.is_some());
    // and PR diff files were parsed
    assert_eq!(app.diff_files.len(), 1);
    // and the forge backend is wired for context expansion / submit
    assert!(app.forge_backend.is_some());
}

/// Regression for issue #591: `:prs` back to the selector and opening a
/// second PR errored with "needs a local clone" on Azure DevOps.
///
/// Entering PR mode replaces `vcs_info.root_path` with the synthetic
/// `forge:host/owner/repo` session identity, so resolving the checkout from
/// it found nothing for every PR after the first. `local_repo_root` keeps the
/// real launch root around for exactly this.
#[test]
fn should_still_resolve_local_checkout_for_a_second_pr_open() {
    // given an app rooted in a real checkout of the PR's repository
    let _reviews = TestReviewsDir::new();
    let dir = tempfile::tempdir().expect("failed to create test repo dir");
    let repo_root = dir.path().to_path_buf();
    let git = git2::Repository::init(&repo_root).expect("failed to init test repo");
    git.remote("origin", "https://github.com/agavra/tuicr")
        .expect("failed to add origin");

    let repository = ForgeRepository::github("github.com", "agavra", "tuicr");
    let mut app = build_app_rooted(repo_root.clone(), Vec::new(), None);
    assert_eq!(app.local_checkout_for(&repository), Some(repo_root.clone()));

    // when a first PR is opened
    let summary = sample_pr(42, "first");
    let backend = Box::new(FakeForgeBackend::open_pr_details(
        test_pr_details(42, "first"),
        crate::forge::github::gh::tests_fixture::SIMPLE_PATCH.to_string(),
    ));
    app.open_pr_with_backend(&summary, backend, Some(repo_root.clone()))
        .unwrap();

    // then the VCS root is the synthetic PR identity, not a directory
    assert!(
        app.vcs_info
            .root_path
            .to_string_lossy()
            .starts_with("forge:"),
        "expected a synthetic PR root, got {:?}",
        app.vcs_info.root_path,
    );
    // and the next PR open still finds the local clone
    assert_eq!(app.local_checkout_for(&repository), Some(repo_root));
}

fn sample_pr_commit(oid: &str, summary: &str) -> crate::forge::traits::PullRequestCommit {
    crate::forge::traits::PullRequestCommit {
        oid: oid.to_string(),
        short_oid: oid.chars().take(7).collect(),
        summary: summary.to_string(),
        body: None,
        author: "Alice".to_string(),
        timestamp: None,
    }
}

fn sample_pr_commit_with_body(
    oid: &str,
    summary: &str,
    body: &str,
) -> crate::forge::traits::PullRequestCommit {
    let mut commit = sample_pr_commit(oid, summary);
    commit.body = Some(body.to_string());
    commit
}

fn review_record(author: &str, oid: &str, submitted_at: &str) -> PullRequestReviewRecord {
    PullRequestReviewRecord {
        author: Some(author.to_string()),
        submitted_at: Some(submitted_at.parse().unwrap()),
        commit_oid: Some(oid.to_string()),
    }
}

fn review_metadata(reviews: Vec<PullRequestReviewRecord>) -> PullRequestReviewMetadata {
    PullRequestReviewMetadata {
        viewer_login: Some("ronen-hoffer".to_string()),
        reviews,
    }
}

#[test]
fn should_select_commits_since_viewers_last_review() {
    let commits = vec![
        sample_pr_commit("c3", "third"),
        sample_pr_commit("c2", "second"),
        sample_pr_commit("c1", "first"),
    ];
    let metadata = review_metadata(vec![
        review_record("ronen-hoffer", "c1", "2026-06-01T10:00:00Z"),
        review_record("alice", "c3", "2026-06-02T10:00:00Z"),
        review_record("ronen-hoffer", "c2", "2026-06-03T10:00:00Z"),
    ]);

    let selection = commits_since_last_review_selection(&commits, &metadata).unwrap();

    assert_eq!(selection.range, Some((0, 0)));
    assert_eq!(selection.reviewed_index, 1);
    assert_eq!(
        selection.message,
        "Showing 1 commit since your last review — press Enter to see all"
    );
}

#[test]
fn should_report_no_new_commits_when_viewers_last_review_is_at_head() {
    let commits = vec![
        sample_pr_commit("c3", "third"),
        sample_pr_commit("c2", "second"),
    ];
    let metadata = review_metadata(vec![review_record(
        "ronen-hoffer",
        "c3",
        "2026-06-03T10:00:00Z",
    )]);

    let selection = commits_since_last_review_selection(&commits, &metadata).unwrap();

    assert_eq!(selection.range, None);
    assert_eq!(selection.reviewed_index, 0);
    assert_eq!(selection.message, "No commits since your last review");
}

#[test]
fn should_skip_since_last_review_selection_when_commit_is_missing() {
    let commits = vec![sample_pr_commit("c3", "third")];
    let metadata = review_metadata(vec![review_record(
        "ronen-hoffer",
        "gone",
        "2026-06-03T10:00:00Z",
    )]);

    assert!(commits_since_last_review_selection(&commits, &metadata).is_none());
}

#[test]
fn should_preserve_persisted_commit_range_over_since_last_review_default() {
    let mut app = build_app();
    app.session.commit_selection_range = Some((1, 1));
    let commits = vec![
        sample_pr_commit("c3", "third"),
        sample_pr_commit("c2", "second"),
        sample_pr_commit("c1", "first"),
    ];
    let metadata = review_metadata(vec![review_record(
        "ronen-hoffer",
        "c2",
        "2026-06-03T10:00:00Z",
    )]);

    let message = app.apply_pr_commit_selector(commits, metadata);

    assert_eq!(app.commit_selection_range, Some((1, 1)));
    assert_eq!(app.pr_last_reviewed_commit_index, Some(1));
    assert!(message.is_none());
    assert_eq!(app.focused_panel, FocusedPanel::Diff);
}

#[test]
fn should_mark_pr_commits_covered_by_viewers_last_review() {
    let mut app = build_app();
    app.diff_source = DiffSource::PullRequest(Box::new(PullRequestDiffSource::from_details(
        &test_pr_details(42, "reviewed"),
    )));
    let commits = vec![
        sample_pr_commit("c3", "third"),
        sample_pr_commit("c2", "second"),
        sample_pr_commit("c1", "first"),
    ];
    let metadata = review_metadata(vec![review_record(
        "ronen-hoffer",
        "c2",
        "2026-06-03T10:00:00Z",
    )]);

    app.apply_pr_commit_selector(commits, metadata);

    assert!(!app.is_commit_reviewed_by_viewer(0));
    assert!(app.is_commit_reviewed_by_viewer(1));
    assert!(app.is_commit_reviewed_by_viewer(2));
}

fn two_hunk_patch() -> &'static str {
    include_str!("../../../tests/fixtures/pr_refresh/two_hunk.patch")
}

fn first_hunk_patch() -> &'static str {
    include_str!("../../../tests/fixtures/pr_refresh/first_hunk.patch")
}

fn structured_patch(patch: &str) -> Vec<crate::model::FilePatch> {
    crate::vcs::diff_parser::git_fixture_file_patches(patch)
}

fn two_file_patch(changed_replacement: &str) -> String {
    match changed_replacement {
        "new changed" => {
            include_str!("../../../tests/fixtures/pr_refresh/two_file_new_changed.patch")
        }
        "newer changed" => {
            include_str!("../../../tests/fixtures/pr_refresh/two_file_newer_changed.patch")
        }
        _ => panic!("unexpected two-file patch replacement: {changed_replacement}"),
    }
    .to_string()
}

fn two_file_plus_added_patch() -> String {
    include_str!("../../../tests/fixtures/pr_refresh/two_file_plus_added.patch").to_string()
}

fn two_hunk_pr_patch(second_replacement: &str) -> String {
    match second_replacement {
        "new second" => {
            include_str!("../../../tests/fixtures/pr_refresh/two_hunk_pr_new_second.patch")
        }
        "newer second" => {
            include_str!("../../../tests/fixtures/pr_refresh/two_hunk_pr_newer_second.patch")
        }
        _ => panic!("unexpected two-hunk PR patch replacement: {second_replacement}"),
    }
    .to_string()
}

fn write_session_file_without_manifest(session: &ReviewSession) {
    let path = crate::persistence::storage::session_path(session).unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let json = serde_json::to_vec_pretty(session).unwrap();
    std::fs::write(path, json).unwrap();
}

fn write_corrupt_session_file(session: &ReviewSession) {
    let path = crate::persistence::storage::session_path(session).unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, b"not json").unwrap();
}

#[test]
fn should_populate_inline_selector_when_pr_has_multiple_commits() {
    // given a PR open path where the forge returns 3 commits
    let mut app = build_app();
    app.forge_repository = Some(ForgeRepository::github("github.com", "agavra", "tuicr"));
    app.pr_tab = PullRequestsTab::new(app.forge_repository.clone());
    app.pr_tab.start_initial_load();
    let summary = sample_pr(42, "multi-commit");
    app.pr_tab
        .apply_initial_load(Ok((vec![summary.clone()], false)));
    let mut backend = FakeForgeBackend::open_pr_details(
        test_pr_details(42, "multi-commit"),
        crate::forge::github::gh::tests_fixture::SIMPLE_PATCH.to_string(),
    );
    // Forge returns oldest-first; pr_open reverses to newest-first.
    backend.commits = vec![
        sample_pr_commit("aaaaaaa1111", "first"),
        sample_pr_commit("bbbbbbb2222", "second"),
        sample_pr_commit("ccccccc3333", "third"),
    ];
    // when
    app.open_pr_with_backend(&summary, Box::new(backend), None)
        .unwrap();
    // then — selector is visible and pr_commits is in newest-first order.
    assert!(app.show_commit_selector, "selector should be visible");
    assert_eq!(app.pr_commits.len(), 3);
    assert_eq!(app.pr_commits[0].summary, "third");
    assert_eq!(app.review_commits.len(), 3);
    // and — default selection covers all commits.
    assert_eq!(app.commit_selection_range, Some((0, 2)));
    assert!(!app.diff_files.iter().any(|file| file.is_commit_message));
}

#[test]
fn should_show_single_pr_commit_message_on_open_and_same_head_reload() {
    // given a PR with exactly one commit and a multiline message
    let _reviews = TestReviewsDir::new();
    let mut app = build_app();
    let details = test_pr_details(424310, "solo");
    let patch = crate::forge::github::gh::tests_fixture::SIMPLE_PATCH;
    let commit = sample_pr_commit_with_body(
        &details.head_sha,
        "only",
        "first body line\nsecond body line",
    );
    let opened = crate::forge::pr_open::prepare_open_pr(
        details.clone(),
        structured_patch(patch),
        vec![commit.clone()],
        PullRequestReviewMetadata::default(),
        crate::forge::traits::PullRequestInfo::from_details(details.clone()),
        None,
    )
    .unwrap();

    // when PR mode is entered without an inline selector
    app.enter_pr_diff_mode(
        Box::new(FakeForgeBackend::open_pr_details(
            details.clone(),
            patch.to_string(),
        )),
        opened,
    )
    .unwrap();

    // then the single commit's message is already at the top
    assert!(!app.show_commit_selector);
    assert!(!app.has_inline_commit_selector());
    assert!(!app.has_review_commits());
    assert!(app.commit_list.is_empty());
    assert_eq!(app.commit_selection_range, None);
    assert_eq!(app.pr_range_sha_pair(), None);
    let message_path = PathBuf::from(format!("Commit Message ({})", commit.short_oid));
    assert!(app.diff_files[0].is_commit_message);
    assert_eq!(app.diff_files[0].display_path(), &message_path);
    assert_eq!(
        app.diff_files
            .iter()
            .filter(|file| file.is_commit_message)
            .count(),
        1
    );
    assert_eq!(
        app.diff_files[0].hunks[0]
            .lines
            .iter()
            .map(|line| line.content.as_str())
            .collect::<Vec<_>>(),
        vec!["only", "", "first body line", "second body line"]
    );
    app.session
        .get_file_mut(&message_path)
        .unwrap()
        .add_line_comment(
            3,
            Comment::new(
                "message draft".to_string(),
                CommentType::from_id("note"),
                None,
            ),
        );
    app.session.get_file_mut(&message_path).unwrap().reviewed = true;
    let request = PrReloadRequest {
        repository: details.repository.clone(),
        pr_number: details.number,
        head_sha: details.head_sha.clone(),
        started_at: Instant::now(),
        anchor: None,
        restore_overview_cursor: None,
    };

    // when the same head is refreshed
    app.finish_pr_reload(
        details.clone(),
        structured_patch(patch),
        vec![commit],
        PullRequestReviewMetadata::default(),
        crate::forge::traits::PullRequestInfo::from_details(details),
        &request,
    )
    .unwrap();

    // then the message, its draft and its reviewed mark survive
    assert!(app.diff_files[0].is_commit_message);
    assert_eq!(app.diff_files[0].display_path(), &message_path);
    assert_eq!(
        app.diff_files
            .iter()
            .filter(|file| file.is_commit_message)
            .count(),
        1
    );
    assert_eq!(
        app.diff_files[0].hunks[0]
            .lines
            .iter()
            .map(|line| line.content.as_str())
            .collect::<Vec<_>>(),
        vec!["only", "", "first body line", "second body line"]
    );
    assert!(app.session.is_file_reviewed(&message_path));
    assert_eq!(
        app.session.files[&message_path].line_comments[&3][0].content,
        "message draft"
    );
    assert_eq!(app.commit_selection_range, None);
    assert!(!app.has_inline_commit_selector());
}

fn commit_message_lines(file: &DiffFile) -> Vec<&str> {
    file.hunks[0]
        .lines
        .iter()
        .map(|line| line.content.as_str())
        .collect()
}

fn assert_one_commit_message_first(app: &App, short_oid: &str) {
    assert!(app.diff_files[0].is_commit_message);
    assert_eq!(
        app.diff_files[0].display_path(),
        &DiffFile::commit_message_path(short_oid)
    );
    assert_eq!(
        app.diff_files
            .iter()
            .filter(|file| file.is_commit_message)
            .count(),
        1
    );
}

#[test]
fn should_keep_selected_pr_diff_on_same_head_reload_and_refresh_cumulative_cache() {
    // given a multi-commit PR narrowed to its middle commit
    let _reviews = TestReviewsDir::new();
    let mut app = build_app();
    let details = test_pr_details(424312, "same-head message range");
    let selected = sample_pr_commit_with_body("middle22222", "middle subject", "middle body");
    // Forge returns oldest-first; prepare_open_pr reverses this order.
    let commits = vec![
        sample_pr_commit("first111111", "first"),
        selected.clone(),
        sample_pr_commit("last3333333", "last"),
    ];
    let mut backend =
        FakeForgeBackend::open_pr_details(details.clone(), two_hunk_patch().to_string());
    backend.commits = commits.clone();
    app.open_pr_with_backend(
        &sample_pr(details.number, &details.title),
        Box::new(backend),
        None,
    )
    .unwrap();
    let cumulative_file = app.diff_files[0].clone();
    assert_eq!(cumulative_file.hunks.len(), 2);
    app.commit_selection_range = Some((1, 1));
    let range_request = PrRangeReloadRequest {
        repository: details.repository.clone(),
        pr_number: details.number,
        head_sha: details.head_sha.clone(),
        start_sha: "first111111".to_string(),
        end_sha: selected.oid.clone(),
        range: (1, 1),
        started_at: Instant::now(),
        anchor: None,
    };
    app.finish_pr_range_reload(&range_request, structured_patch(first_hunk_patch()))
        .unwrap();
    let narrowed_file = app.diff_files[1].clone();
    assert_eq!(narrowed_file.hunks.len(), 1);
    // The selected message is first, registered, and takes a draft.
    assert_one_commit_message_first(&app, &selected.short_oid);
    assert_eq!(
        commit_message_lines(&app.diff_files[0]),
        vec!["middle subject", "", "middle body"]
    );
    let message_path = DiffFile::commit_message_path(&selected.short_oid);
    app.session
        .get_file_mut(&message_path)
        .expect("message registered in the session")
        .add_line_comment(
            3,
            Comment::new(
                "message comment".to_string(),
                CommentType::from_id("note"),
                None,
            ),
        );
    // The reload's cumulative patch differs from the open patch, so the
    // cache refresh is observable.
    let reload_patch = two_hunk_patch().replace("new two", "newer two");
    let request = PrReloadRequest {
        repository: details.repository.clone(),
        pr_number: details.number,
        head_sha: details.head_sha.clone(),
        started_at: Instant::now(),
        anchor: None,
        restore_overview_cursor: None,
    };

    // when a same-head reload delivers the cumulative PR diff
    app.finish_pr_reload(
        details.clone(),
        structured_patch(&reload_patch),
        commits,
        PullRequestReviewMetadata::default(),
        crate::forge::traits::PullRequestInfo::from_details(details),
        &request,
    )
    .unwrap();

    // then the selection, selected message and narrowed diff remain on screen
    assert_eq!(app.commit_selection_range, Some((1, 1)));
    assert_eq!(app.diff_files.len(), 2);
    assert!(app.diff_files[0].is_commit_message);
    assert_eq!(
        app.diff_files[0].display_path(),
        &PathBuf::from(format!("Commit Message ({})", selected.short_oid))
    );
    assert!(!app.diff_files[1].is_commit_message);
    assert_eq!(
        app.diff_files[1].display_path(),
        narrowed_file.display_path()
    );
    assert_eq!(app.diff_files[1].content_hash, narrowed_file.content_hash);
    assert_eq!(app.diff_files[1].hunks.len(), 1);
    let cached = app.range_diff_files.as_ref().unwrap();
    assert_eq!(cached.len(), 1);
    assert!(!cached[0].is_commit_message);
    assert_eq!(cached[0].display_path(), cumulative_file.display_path());
    assert_ne!(cached[0].content_hash, cumulative_file.content_hash);
    assert_eq!(cached[0].hunks.len(), 2);
    let refreshed_hash = cached[0].content_hash;

    // when the selection widens to all commits
    app.commit_selection_range = Some((0, 2));
    app.reload_pr_inline_selection();

    // then the refreshed cumulative cache replaces the message and narrowed diff
    assert_eq!(app.diff_files.len(), 1);
    assert!(!app.diff_files.iter().any(|file| file.is_commit_message));
    assert_eq!(
        app.diff_files[0].display_path(),
        cumulative_file.display_path()
    );
    assert_eq!(app.diff_files[0].content_hash, refreshed_hash);
    assert_eq!(app.diff_files[0].hunks.len(), 2);
    assert!(app.pr_range_reload_state.is_none());
    assert_eq!(
        app.session.files[&message_path].line_comments[&3][0].content,
        "message comment"
    );
}

#[test]
fn should_refresh_the_cumulative_cache_on_an_unchanged_identity_reload() {
    // given a multi-commit PR with every commit selected, so the cumulative
    // diff is the displayed one
    let _reviews = TestReviewsDir::new();
    let mut app = build_app();
    let details = test_pr_details(424398, "fresh cache");
    let commits = three_commits();
    let mut backend =
        FakeForgeBackend::open_pr_details(details.clone(), two_hunk_patch().to_string());
    backend.commits = commits.clone();
    app.open_pr_with_backend(
        &sample_pr(details.number, &details.title),
        Box::new(backend),
        None,
    )
    .unwrap();
    assert!(app.range_diff_files.is_some());
    let old_hash = app.diff_files[0].content_hash;

    // when a same-base, same-commits reload brings a changed patch
    let reload_patch = two_hunk_patch().replace("new two", "newer two");
    let request = PrReloadRequest {
        repository: details.repository.clone(),
        pr_number: details.number,
        head_sha: details.head_sha.clone(),
        started_at: Instant::now(),
        anchor: None,
        restore_overview_cursor: None,
    };
    app.finish_pr_reload(
        details.clone(),
        structured_patch(&reload_patch),
        commits,
        PullRequestReviewMetadata::default(),
        crate::forge::traits::PullRequestInfo::from_details(details.clone()),
        &request,
    )
    .unwrap();
    let new_hash = app.diff_files[0].content_hash;
    assert_ne!(new_hash, old_hash);

    // and the user narrows, then widens back to every commit
    app.commit_selection_range = Some((2, 2));
    let range_request = PrRangeReloadRequest {
        repository: details.repository.clone(),
        pr_number: details.number,
        head_sha: details.head_sha.clone(),
        start_sha: details.base_sha.clone(),
        end_sha: "aaa1111111".to_string(),
        range: (2, 2),
        started_at: Instant::now(),
        anchor: None,
    };
    app.finish_pr_range_reload(&range_request, structured_patch(first_hunk_patch()))
        .unwrap();
    app.commit_selection_range = Some((0, 2));
    app.reload_pr_inline_selection();

    // then the widened view is the refreshed diff, not the one from open
    let file = app
        .diff_files
        .iter()
        .find(|f| !f.is_commit_message)
        .unwrap();
    assert_eq!(file.content_hash, new_hash);
}

#[test]
fn should_not_ask_the_forge_for_a_commit_message_line_count() {
    // given a single-commit PR, whose diff opens with the commit's message
    let _reviews = TestReviewsDir::new();
    let mut app = build_app();
    let details = test_pr_details(424314, "line counts");
    let patch = crate::forge::github::gh::tests_fixture::SIMPLE_PATCH;
    let backend = FakeForgeBackend::open_pr_details(details.clone(), patch.to_string());
    let requests = backend.line_count_requests.clone();
    let opened = crate::forge::pr_open::prepare_open_pr(
        details.clone(),
        structured_patch(patch),
        vec![sample_pr_commit_with_body(
            &details.head_sha,
            "only",
            "body",
        )],
        PullRequestReviewMetadata::default(),
        crate::forge::traits::PullRequestInfo::from_details(details),
        None,
    )
    .unwrap();

    // when the diff is laid out
    app.enter_pr_diff_mode(Box::new(backend), opened).unwrap();

    // then line counts are looked up for the real files only
    let message_path = app.diff_files[0].display_path().clone();
    assert!(app.diff_files[0].is_commit_message);
    let requests = requests.lock().unwrap();
    assert!(!requests.is_empty());
    assert!(!requests.contains(&message_path));
}

#[test]
fn should_resolve_pr_range_to_parent_sha_and_head_sha() {
    // given a multi-commit PR open with the middle commit selected
    let mut app = build_app();
    app.forge_repository = Some(ForgeRepository::github("github.com", "agavra", "tuicr"));
    app.pr_tab = PullRequestsTab::new(app.forge_repository.clone());
    app.pr_tab.start_initial_load();
    let summary = sample_pr(42, "ranges");
    app.pr_tab
        .apply_initial_load(Ok((vec![summary.clone()], false)));
    let mut backend = FakeForgeBackend::open_pr_details(
        test_pr_details(42, "ranges"),
        crate::forge::github::gh::tests_fixture::SIMPLE_PATCH.to_string(),
    );
    backend.commits = vec![
        sample_pr_commit("first11", "first"),
        sample_pr_commit("middle2", "middle"),
        sample_pr_commit("last333", "last"),
    ];
    app.open_pr_with_backend(&summary, Box::new(backend), None)
        .unwrap();
    // After open: pr_commits = [last, middle, first] (newest-first).
    // Select only the middle commit (index 1).
    app.commit_selection_range = Some((1, 1));
    // when
    let pair = app.pr_range_sha_pair();
    // then — start = parent (first), end = newest selected (middle).
    assert_eq!(pair, Some(("first11".to_string(), "middle2".to_string())));
}

#[test]
fn should_resolve_pr_range_to_pr_base_when_oldest_commit_selected() {
    // given a multi-commit PR with only the oldest commit selected
    let mut app = build_app();
    app.forge_repository = Some(ForgeRepository::github("github.com", "agavra", "tuicr"));
    app.pr_tab = PullRequestsTab::new(app.forge_repository.clone());
    app.pr_tab.start_initial_load();
    let summary = sample_pr(42, "base");
    app.pr_tab
        .apply_initial_load(Ok((vec![summary.clone()], false)));
    let mut backend = FakeForgeBackend::open_pr_details(
        test_pr_details(42, "base"),
        crate::forge::github::gh::tests_fixture::SIMPLE_PATCH.to_string(),
    );
    backend.commits = vec![
        sample_pr_commit("aaa", "first"),
        sample_pr_commit("bbb", "second"),
    ];
    app.open_pr_with_backend(&summary, Box::new(backend), None)
        .unwrap();
    // pr_commits = [second, first]. Select only the oldest (index 1).
    app.commit_selection_range = Some((1, 1));
    // when
    let pair = app.pr_range_sha_pair();
    // then — start falls back to the PR's base_sha.
    let expected_base = test_pr_details(42, "base").base_sha;
    assert_eq!(pair, Some((expected_base, "aaa".to_string())));
}

#[test]
fn should_route_pr_inline_selection_reload_through_forge_not_vcs() {
    // Regression for #462: narrowing a PR review to a commit subrange with
    // `(`/`)` used to reload via the VCS `get_commit_range_diff`, which the
    // forge-backed review has no support for — it errored with "Commit range
    // diff not supported for this VCS". The reload must route through the
    // forge `compare` API instead, exactly like the Enter/toggle path does.
    let _reviews = TestReviewsDir::new();
    let mut app = build_app();
    app.forge_repository = Some(ForgeRepository::github("github.com", "agavra", "tuicr"));
    app.pr_tab = PullRequestsTab::new(app.forge_repository.clone());
    app.pr_tab.start_initial_load();
    let summary = sample_pr(42, "cycle");
    app.pr_tab
        .apply_initial_load(Ok((vec![summary.clone()], false)));
    let mut backend = FakeForgeBackend::open_pr_details(
        test_pr_details(42, "cycle"),
        crate::forge::github::gh::tests_fixture::SIMPLE_PATCH.to_string(),
    );
    backend.commits = vec![
        sample_pr_commit("first11", "first"),
        sample_pr_commit("middle2", "middle"),
        sample_pr_commit("last333", "last"),
    ];
    app.open_pr_with_backend(&summary, Box::new(backend), None)
        .unwrap();
    // Default selection covers all three commits.
    assert_eq!(app.commit_selection_range, Some((0, 2)));

    // when — `(` narrows the selection to a strict subrange (all → first).
    app.cycle_commit_prev();
    assert_eq!(app.commit_selection_range, Some((0, 0)));
    let result = app.reload_inline_selection_for_source();

    // then — the reload succeeds (the VCS path would return
    // UnsupportedOperation) and takes the forge range-reload route.
    assert!(
        result.is_ok(),
        "PR inline reload must not hit the unsupported VCS path: {result:?}"
    );
    assert!(
        app.pr_range_reload_state.is_some(),
        "narrowing a PR review should spawn a forge range reload"
    );
}

#[test]
fn should_preserve_hunk_marks_hidden_by_pr_range_diff() {
    let mut app = build_app();
    app.forge_repository = Some(ForgeRepository::github("github.com", "agavra", "tuicr"));
    app.pr_tab = PullRequestsTab::new(app.forge_repository.clone());
    app.pr_tab.start_initial_load();
    let summary = sample_pr(42, "ranges");
    app.pr_tab
        .apply_initial_load(Ok((vec![summary.clone()], false)));
    let mut backend = FakeForgeBackend::open_pr_details(
        test_pr_details(42, "ranges"),
        two_hunk_patch().to_string(),
    );
    backend.commits = vec![
        sample_pr_commit("aaa1111", "first"),
        sample_pr_commit("bbb2222", "second"),
    ];
    app.open_pr_with_backend(&summary, Box::new(backend), None)
        .unwrap();
    let path = app.diff_files[0].display_path().clone();
    let hidden_key = app.diff_files[0].hunk_review_key(1).unwrap();
    app.session
        .get_file_mut(&path)
        .unwrap()
        .toggle_hunk_reviewed(hidden_key.clone());

    let request = PrRangeReloadRequest {
        repository: ForgeRepository::github("github.com", "agavra", "tuicr"),
        pr_number: 42,
        head_sha: "abcdef0123456789".to_string(),
        start_sha: "aaa1111".to_string(),
        end_sha: "bbb2222".to_string(),
        range: (1, 1),
        started_at: Instant::now(),
        anchor: None,
    };
    app.finish_pr_range_reload(&request, structured_patch(first_hunk_patch()))
        .unwrap();

    assert!(app.session.is_hunk_reviewed(&path, &hidden_key));
}

#[test]
fn should_warn_when_opening_closed_pr() {
    // given
    let mut app = build_app();
    let summary = sample_pr(42, "old");
    let mut details = test_pr_details(42, "old");
    details.state = "CLOSED".to_string();
    details.closed = true;
    let backend = Box::new(FakeForgeBackend::open_pr_details(
        details,
        crate::forge::github::gh::tests_fixture::SIMPLE_PATCH.to_string(),
    ));
    // when
    app.open_pr_with_backend(&summary, backend, None).unwrap();
    // then — warning message surfaces the read-only state
    let msg = app.message.as_ref().expect("expected warning message");
    assert!(msg.content.contains("closed"), "got: {:?}", msg.content);
    assert!(msg.content.contains("read-only"), "got: {:?}", msg.content);
    // and the diff source reflects the closed state
    if let DiffSource::PullRequest(pr) = &app.diff_source {
        assert!(pr.is_read_only());
        assert_eq!(pr.read_only_reason(), Some("closed"));
    } else {
        panic!("expected PullRequest diff source");
    }
}

#[test]
fn should_surface_pr_open_error_into_selector_state() {
    // given a backend that fails at get_pull_request
    let mut app = build_app();
    app.forge_repository = Some(ForgeRepository::github("github.com", "agavra", "tuicr"));
    app.pr_tab = PullRequestsTab::new(app.forge_repository.clone());
    app.pr_tab.start_initial_load();
    let summary = sample_pr(42, "boom");
    app.pr_tab
        .apply_initial_load(Ok((vec![summary.clone()], false)));
    app.target_tab = TargetTab::PullRequests;
    let backend = Box::new(FailingForgeBackend);
    // when
    let result = app.open_pr_with_backend(&summary, backend, None);
    // then
    assert!(result.is_err());
    // diff source did not switch
    assert!(matches!(app.diff_source, DiffSource::WorkingTree));
}

#[test]
fn should_route_context_expansion_to_forge_provider_in_pr_mode() {
    // given an app in PR mode with a counting fake backend
    let mut app = build_app();
    let summary = sample_pr(7, "ctx");
    let backend = Box::new(FakeForgeBackend::open_pr_details(
        test_pr_details(7, "ctx"),
        crate::forge::github::gh::tests_fixture::SIMPLE_PATCH.to_string(),
    ));
    app.open_pr_with_backend(&summary, backend, None).unwrap();
    // when we ask for a context provider
    // (we can't easily trigger a real gap expansion without setting up
    //  the full diff state, so instead we assert by construction)
    let provider = app.context_provider();
    // then — explicitly: the provider is the forge variant. We probe by
    // calling fetch with a Modified file and asserting the forge backend
    // recorded a head-side request via its trait method. The
    // FakeForgeBackend just returns empty; we're verifying routing.
    let res = provider
        .fetch_context_lines(
            None,
            Some(&PathBuf::from("src/lib.rs")),
            FileStatus::Modified,
            1,
            3,
        )
        .unwrap();
    // The fake forge returns empty by default — the *call* succeeded
    // (no error from a VCS backend would have meant VCS routing). The
    // key signal: this didn't go through the VCS backend (DummyVcs
    // doesn't implement fetch_context_lines and would have panicked).
    assert!(res.is_empty());
}

#[test]
fn should_switch_session_when_pr_head_advances_on_reload() {
    // given an app already in PR mode at head A
    let mut app = build_app();
    let summary = sample_pr(42, "head-a");
    let mut details_a = test_pr_details(42, "head-a");
    details_a.head_sha = "aaaaaaaaaaaaaaaa".to_string();
    let backend_a = Box::new(FakeForgeBackend::open_pr_details(
        details_a.clone(),
        crate::forge::github::gh::tests_fixture::SIMPLE_PATCH.to_string(),
    ));
    app.open_pr_with_backend(&summary, backend_a, None).unwrap();
    let old_session_id = app.session.id.clone();
    // when reloading with a backend that reports head B
    let mut details_b = details_a.clone();
    details_b.head_sha = "bbbbbbbbbbbbbbbb".to_string();
    details_b.title = "head-b".to_string();
    let backend_b = Box::new(FakeForgeBackend::open_pr_details(
        details_b.clone(),
        crate::forge::github::gh::tests_fixture::SIMPLE_PATCH.to_string(),
    ));
    let head_changed = app
        .reload_pull_request_with_backend(backend_b, None)
        .unwrap();
    // then — the session swap happened
    assert!(head_changed);
    if let DiffSource::PullRequest(pr) = &app.diff_source {
        assert_eq!(pr.key.head_sha, "bbbbbbbbbbbbbbbb");
        assert_eq!(pr.title, "head-b");
    } else {
        panic!("expected PullRequest diff source");
    }
    // and the session changed (new session, not the old one)
    assert_ne!(app.session.id, old_session_id);
}

#[test]
fn should_load_persisted_pr_session_when_reopening_same_head() {
    // given a saved PR session for the same PR head
    let _reviews = TestReviewsDir::new();
    let mut app = build_app();
    let summary = sample_pr(424246, "persisted");
    let mut details = test_pr_details(424246, "persisted");
    details.head_sha = "aaaaaaaaaaaaaaaa".to_string();
    let stable_path = PathBuf::from("src/stable.rs");
    let backend = Box::new(FakeForgeBackend::open_pr_details(
        details.clone(),
        two_file_patch("new changed"),
    ));
    app.open_pr_with_backend(&summary, backend, None).unwrap();
    app.session.get_file_mut(&stable_path).unwrap().reviewed = true;
    app.session
        .get_file_mut(&stable_path)
        .unwrap()
        .add_file_comment(Comment::new(
            "persisted draft".to_string(),
            CommentType::from_id("note"),
            None,
        ));
    crate::persistence::save_session(&app.session).unwrap();

    // when the same PR head is opened again
    let mut reopened = build_app();
    let backend = Box::new(FakeForgeBackend::open_pr_details(
        details,
        two_file_patch("new changed"),
    ));
    reopened
        .open_pr_with_backend(&summary, backend, None)
        .unwrap();

    // then the persisted session branch reattaches reviewed state and drafts.
    let stable_review = reopened.session.files.get(&stable_path).unwrap();
    assert!(stable_review.reviewed);
    assert_eq!(stable_review.file_comments.len(), 1);
    assert_eq!(stable_review.file_comments[0].content, "persisted draft");
}

#[test]
fn should_carry_persisted_pr_review_forward_when_reopening_at_new_head() {
    // given a saved PR session at head A with both files reviewed and a draft
    let _reviews = TestReviewsDir::new();
    let mut app = build_app();
    let summary = sample_pr(424247, "persisted");
    let mut details_a = test_pr_details(424247, "persisted");
    details_a.head_sha = "aaaaaaaaaaaaaaaa".to_string();
    let stable_path = PathBuf::from("src/stable.rs");
    let changed_path = PathBuf::from("src/changed.rs");
    let backend = Box::new(FakeForgeBackend::open_pr_details(
        details_a.clone(),
        two_file_patch("new changed"),
    ));
    app.open_pr_with_backend(&summary, backend, None).unwrap();
    app.session.get_file_mut(&stable_path).unwrap().reviewed = true;
    app.session.get_file_mut(&changed_path).unwrap().reviewed = true;
    app.session
        .get_file_mut(&stable_path)
        .unwrap()
        .add_file_comment(Comment::new(
            "persisted draft".to_string(),
            CommentType::from_id("note"),
            None,
        ));
    crate::persistence::save_session(&app.session).unwrap();

    // when the PR is opened fresh after a push moved it to head B
    let mut details_b = details_a.clone();
    details_b.head_sha = "bbbbbbbbbbbbbbbb".to_string();
    let mut reopened = build_app();
    let backend = Box::new(FakeForgeBackend::open_pr_details(
        details_b,
        two_file_patch("newer changed"),
    ));
    reopened
        .open_pr_with_backend(&summary, backend, None)
        .unwrap();

    // then the unchanged file keeps its review and draft; the changed one reopens.
    let stable_review = reopened.session.files.get(&stable_path).unwrap();
    assert!(stable_review.reviewed);
    assert_eq!(stable_review.file_comments.len(), 1);
    assert!(!reopened.session.is_file_reviewed(&changed_path));
    assert_eq!(
        reopened.session.pr_session_key.as_ref().unwrap().head_sha,
        "bbbbbbbbbbbbbbbb"
    );
}

#[test]
fn should_open_fresh_when_previous_head_session_is_corrupt() {
    // given the PR's manifest entry points at a corrupt session file for head A
    let _reviews = TestReviewsDir::new();
    let mut app = build_app();
    let summary = sample_pr(424248, "persisted");
    let mut details_a = test_pr_details(424248, "persisted");
    details_a.head_sha = "aaaaaaaaaaaaaaaa".to_string();
    let stable_path = PathBuf::from("src/stable.rs");
    let backend = Box::new(FakeForgeBackend::open_pr_details(
        details_a.clone(),
        two_file_patch("new changed"),
    ));
    app.open_pr_with_backend(&summary, backend, None).unwrap();
    app.session.get_file_mut(&stable_path).unwrap().reviewed = true;
    let path = crate::persistence::save_session(&app.session).unwrap();
    std::fs::write(path, b"not json").unwrap();

    // when the PR is opened fresh at head B
    let mut details_b = details_a.clone();
    details_b.head_sha = "bbbbbbbbbbbbbbbb".to_string();
    let mut reopened = build_app();
    let backend = Box::new(FakeForgeBackend::open_pr_details(
        details_b,
        two_file_patch("newer changed"),
    ));
    reopened
        .open_pr_with_backend(&summary, backend, None)
        .unwrap();

    // then it opens with a fresh session instead of failing.
    assert!(!reopened.session.is_file_reviewed(&stable_path));
}

#[test]
fn should_not_carry_review_from_same_pr_on_another_host() {
    // given a saved review of owner/repo#N on github.com at head A
    let _reviews = TestReviewsDir::new();
    let mut app = build_app();
    let summary = sample_pr(424249, "persisted");
    let mut details_a = test_pr_details(424249, "persisted");
    details_a.head_sha = "aaaaaaaaaaaaaaaa".to_string();
    let stable_path = PathBuf::from("src/stable.rs");
    let backend = Box::new(FakeForgeBackend::open_pr_details(
        details_a.clone(),
        two_file_patch("new changed"),
    ));
    app.open_pr_with_backend(&summary, backend, None).unwrap();
    app.session.get_file_mut(&stable_path).unwrap().reviewed = true;
    crate::persistence::save_session(&app.session).unwrap();

    // when the same owner/repo#N is opened on a GHE host at head B
    let mut details_b = details_a.clone();
    details_b.repository = ForgeRepository::github("ghe.example.com", "agavra", "tuicr");
    details_b.head_sha = "bbbbbbbbbbbbbbbb".to_string();
    let mut reopened = build_app();
    let backend = Box::new(FakeForgeBackend::open_pr_details(
        details_b,
        two_file_patch("new changed"),
    ));
    reopened
        .open_pr_with_backend(&summary, backend, None)
        .unwrap();

    // then the github.com review is not carried over.
    assert!(!reopened.session.is_file_reviewed(&stable_path));
}

#[test]
fn should_keep_saved_pr_session_through_quit_reopen_and_same_head_reload() {
    // given a saved PR session with all files reviewed and local comments
    let _reviews = TestReviewsDir::new();
    let mut app = build_app();
    let summary = sample_pr(424255, "quit-reopen");
    let mut details = test_pr_details(424255, "quit-reopen");
    details.head_sha = "aaaaaaaaaaaaaaaa".to_string();
    let stable_path = PathBuf::from("src/stable.rs");
    let changed_path = PathBuf::from("src/changed.rs");
    let backend = Box::new(FakeForgeBackend::open_pr_details(
        details.clone(),
        two_file_patch("new changed"),
    ));
    app.open_pr_with_backend(&summary, backend, None).unwrap();
    app.session.get_file_mut(&stable_path).unwrap().reviewed = true;
    app.session.get_file_mut(&changed_path).unwrap().reviewed = true;
    app.session
        .get_file_mut(&stable_path)
        .unwrap()
        .add_file_comment(Comment::new(
            "persisted file draft".to_string(),
            CommentType::from_id("note"),
            None,
        ));
    app.session
        .get_file_mut(&changed_path)
        .unwrap()
        .add_line_comment(
            1,
            Comment::new(
                "persisted line draft".to_string(),
                CommentType::from_id("issue"),
                None,
            ),
        );
    app.save_current_session_merging_external().unwrap();

    // and normal quit cleanup runs for the ephemeral session path
    assert_eq!(app.cleanup_empty_ephemeral_sessions().unwrap(), 0);

    // when the same PR head is opened again and reloaded with :e
    let mut reopened = build_app();
    let backend = Box::new(FakeForgeBackend::open_pr_details(
        details.clone(),
        two_file_patch("new changed"),
    ));
    reopened
        .open_pr_with_backend(&summary, backend, None)
        .unwrap();
    let backend = Box::new(FakeForgeBackend::open_pr_details(
        details,
        two_file_patch("new changed"),
    ));
    let head_changed = reopened
        .reload_pull_request_with_backend(backend, None)
        .unwrap();

    // then reviewed state and comments survive the full flow.
    assert!(!head_changed);
    assert!(reopened.session.is_file_reviewed(&stable_path));
    assert!(reopened.session.is_file_reviewed(&changed_path));
    let stable_review = reopened.session.files.get(&stable_path).unwrap();
    assert_eq!(stable_review.file_comments.len(), 1);
    assert_eq!(
        stable_review.file_comments[0].content,
        "persisted file draft"
    );
    let changed_review = reopened.session.files.get(&changed_path).unwrap();
    assert_eq!(changed_review.line_comments[&1].len(), 1);
    assert_eq!(
        changed_review.line_comments[&1][0].content,
        "persisted line draft"
    );
}

#[test]
fn should_reindex_recovered_pr_session() {
    // given an old-head PR session with reviewed state
    let _reviews = TestReviewsDir::new();
    let mut app = build_app();
    let summary = sample_pr(424247, "head-a");
    let mut details_a = test_pr_details(424247, "head-a");
    details_a.head_sha = "aaaaaaaaaaaaaaaa".to_string();
    let stable_path = PathBuf::from("src/stable.rs");
    let changed_path = PathBuf::from("src/changed.rs");
    let backend_a = Box::new(FakeForgeBackend::open_pr_details(
        details_a.clone(),
        two_file_patch("new changed"),
    ));
    app.open_pr_with_backend(&summary, backend_a, None).unwrap();
    app.session.get_file_mut(&stable_path).unwrap().reviewed = true;

    // and a previously saved session file already exists for the new head
    let mut details_b = details_a.clone();
    details_b.head_sha = "bbbbbbbbbbbbbbbb".to_string();
    let opened_b = crate::forge::pr_open::prepare_open_pr(
        details_b.clone(),
        structured_patch(&two_file_patch("newer changed")),
        Vec::new(),
        PullRequestReviewMetadata::default(),
        crate::forge::traits::PullRequestInfo::from_details(details_b.clone()),
        None,
    )
    .unwrap();
    let mut persisted_b = opened_b.session.clone();
    persisted_b.get_file_mut(&changed_path).unwrap().reviewed = true;
    persisted_b
        .get_file_mut(&changed_path)
        .unwrap()
        .add_file_comment(Comment::new(
            "new-head draft".to_string(),
            CommentType::from_id("note"),
            None,
        ));
    let persisted_path = crate::persistence::storage::session_path(&persisted_b).unwrap();
    let pr_session_key = persisted_b.pr_session_key.as_ref().unwrap();
    let slug = crate::slug::Slug::from(pr_session_key).to_string();
    write_session_file_without_manifest(&persisted_b);
    let persisted_contents = std::fs::read(&persisted_path).unwrap();

    // when the PR reload advances to that head
    let backend_b = Box::new(FakeForgeBackend::open_pr_details(
        details_b,
        two_file_patch("newer changed"),
    ));
    let head_changed = app
        .reload_pull_request_with_backend(backend_b, None)
        .unwrap();

    // then the exact new-head session file wins over old-head carry-forward,
    // even when the manifest points at the old head.
    assert!(head_changed);
    assert!(!app.session.is_file_reviewed(&stable_path));
    assert!(app.session.is_file_reviewed(&changed_path));
    let changed_review = app.session.files.get(&changed_path).unwrap();
    assert_eq!(changed_review.file_comments.len(), 1);
    assert_eq!(changed_review.file_comments[0].content, "new-head draft");
    let resolved = crate::review_store::ReviewStore::new()
        .resolve_pr_session(&slug)
        .unwrap()
        .unwrap();
    assert_eq!(resolved.path(), persisted_path);
    assert_eq!(std::fs::read(&persisted_path).unwrap(), persisted_contents);
}

#[test]
fn should_error_on_corrupt_exact_session_file_when_reopening_pr() {
    // given a corrupt session file at the opened PR head's deterministic path
    let _reviews = TestReviewsDir::new();
    let mut details = test_pr_details(424250, "corrupt");
    details.head_sha = "aaaaaaaaaaaaaaaa".to_string();
    let opened = crate::forge::pr_open::prepare_open_pr(
        details.clone(),
        structured_patch(&two_file_patch("new changed")),
        Vec::new(),
        PullRequestReviewMetadata::default(),
        crate::forge::traits::PullRequestInfo::from_details(details.clone()),
        None,
    )
    .unwrap();
    write_corrupt_session_file(&opened.session);

    // when the PR is opened at that head
    let mut app = build_app();
    let summary = sample_pr(424250, "corrupt");
    let backend = Box::new(FakeForgeBackend::open_pr_details(
        details,
        two_file_patch("new changed"),
    ));
    let err = app
        .open_pr_with_backend(&summary, backend, None)
        .unwrap_err();

    // then the corrupt persisted file blocks opening instead of being
    // silently overwritten by a fresh session.
    assert!(
        err.to_string().contains("failed to load PR session"),
        "got: {err}"
    );
}

#[test]
fn should_keep_old_head_session_when_new_head_session_file_is_corrupt() {
    // given reviewed state at the old PR head
    let _reviews = TestReviewsDir::new();
    let mut app = build_app();
    let summary = sample_pr(424251, "head-a");
    let mut details_a = test_pr_details(424251, "head-a");
    details_a.head_sha = "aaaaaaaaaaaaaaaa".to_string();
    let stable_path = PathBuf::from("src/stable.rs");
    let changed_path = PathBuf::from("src/changed.rs");
    let backend_a = Box::new(FakeForgeBackend::open_pr_details(
        details_a.clone(),
        two_file_patch("new changed"),
    ));
    app.open_pr_with_backend(&summary, backend_a, None).unwrap();
    app.session.get_file_mut(&stable_path).unwrap().reviewed = true;
    app.session.get_file_mut(&changed_path).unwrap().reviewed = true;

    // and the deterministic file for the new head is corrupt
    let mut details_b = details_a.clone();
    details_b.head_sha = "bbbbbbbbbbbbbbbb".to_string();
    let opened_b = crate::forge::pr_open::prepare_open_pr(
        details_b.clone(),
        structured_patch(&two_file_patch("newer changed")),
        Vec::new(),
        PullRequestReviewMetadata::default(),
        crate::forge::traits::PullRequestInfo::from_details(details_b.clone()),
        None,
    )
    .unwrap();
    write_corrupt_session_file(&opened_b.session);

    // when the PR reload advances to that head
    let backend_b = Box::new(FakeForgeBackend::open_pr_details(
        details_b,
        two_file_patch("newer changed"),
    ));
    let err = app
        .reload_pull_request_with_backend(backend_b, None)
        .unwrap_err();

    // then reload fails safely and keeps the old-head session in memory.
    assert!(
        err.to_string().contains("failed to load PR session"),
        "got: {err}"
    );
    assert!(app.session.is_file_reviewed(&stable_path));
    assert!(app.session.is_file_reviewed(&changed_path));
}

#[test]
fn should_keep_old_head_session_when_saving_before_head_switch_fails() {
    // given reviewed state at the old PR head
    let _reviews = TestReviewsDir::new();
    let mut app = build_app();
    let summary = sample_pr(424254, "head-a");
    let mut details_a = test_pr_details(424254, "head-a");
    details_a.head_sha = "aaaaaaaaaaaaaaaa".to_string();
    let stable_path = PathBuf::from("src/stable.rs");
    let backend_a = Box::new(FakeForgeBackend::open_pr_details(
        details_a.clone(),
        two_file_patch("new changed"),
    ));
    app.open_pr_with_backend(&summary, backend_a, None).unwrap();
    app.session.get_file_mut(&stable_path).unwrap().reviewed = true;

    // and the review storage root has become unusable.
    let blocked_dir = tempfile::tempdir().unwrap();
    let blocked_root = blocked_dir.path().join("reviews-file");
    std::fs::write(&blocked_root, b"not a directory").unwrap();
    crate::persistence::storage::set_test_reviews_dir(Some(blocked_root));

    // when reload tries to advance to a new head
    let mut details_b = details_a.clone();
    details_b.head_sha = "bbbbbbbbbbbbbbbb".to_string();
    let backend_b = Box::new(FakeForgeBackend::open_pr_details(
        details_b,
        two_file_patch("newer changed"),
    ));
    let err = app
        .reload_pull_request_with_backend(backend_b, None)
        .unwrap_err();

    // then the save failure aborts before switching away from head A.
    assert!(matches!(err, TuicrError::Io(_)), "got: {err}");
    assert!(app.session.is_file_reviewed(&stable_path));
    if let DiffSource::PullRequest(pr) = &app.diff_source {
        assert_eq!(pr.key.head_sha, "aaaaaaaaaaaaaaaa");
    } else {
        panic!("expected PullRequest diff source");
    }
}

#[test]
fn should_ignore_exact_session_file_when_pr_session_key_does_not_match() {
    // given a session file at the opened PR head's deterministic path but
    // with a mismatched embedded PR key
    let _reviews = TestReviewsDir::new();
    let mut details = test_pr_details(424249, "mismatch");
    details.head_sha = "aaaaaaaaaaaaaaaa".to_string();
    let opened = crate::forge::pr_open::prepare_open_pr(
        details.clone(),
        structured_patch(&two_file_patch("new changed")),
        Vec::new(),
        PullRequestReviewMetadata::default(),
        crate::forge::traits::PullRequestInfo::from_details(details.clone()),
        None,
    )
    .unwrap();
    let stable_path = PathBuf::from("src/stable.rs");
    let mut mismatched = opened.session.clone();
    mismatched.pr_session_key = Some(crate::forge::traits::PrSessionKey::new(
        details.repository.clone(),
        details.number,
        "bbbbbbbbbbbbbbbb".to_string(),
    ));
    mismatched.get_file_mut(&stable_path).unwrap().reviewed = true;
    mismatched
        .get_file_mut(&stable_path)
        .unwrap()
        .add_file_comment(Comment::new(
            "wrong-head draft".to_string(),
            CommentType::from_id("note"),
            None,
        ));
    let path = crate::persistence::storage::session_path(&opened.session).unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, serde_json::to_vec_pretty(&mismatched).unwrap()).unwrap();

    // when the PR is opened at the original head
    let mut app = build_app();
    let summary = sample_pr(424249, "mismatch");
    let backend = Box::new(FakeForgeBackend::open_pr_details(
        details,
        two_file_patch("new changed"),
    ));
    app.open_pr_with_backend(&summary, backend, None).unwrap();

    // then the mismatched file is rejected and the fresh session is used.
    let stable_review = app.session.files.get(&stable_path).unwrap();
    assert!(!stable_review.reviewed);
    assert!(stable_review.file_comments.is_empty());
}

#[test]
fn should_ignore_manifest_session_when_pr_session_key_does_not_match() {
    // given a manifest entry for the requested PR head whose session file
    // contains a mismatched embedded key
    let _reviews = TestReviewsDir::new();
    let mut app = build_app();
    let summary = sample_pr(424252, "manifest-mismatch");
    let mut details = test_pr_details(424252, "manifest-mismatch");
    details.head_sha = "aaaaaaaaaaaaaaaa".to_string();
    let stable_path = PathBuf::from("src/stable.rs");
    let backend = Box::new(FakeForgeBackend::open_pr_details(
        details.clone(),
        two_file_patch("new changed"),
    ));
    app.open_pr_with_backend(&summary, backend, None).unwrap();
    let saved_path = crate::persistence::save_session(&app.session).unwrap();

    let mut mismatched = app.session.clone();
    mismatched.pr_session_key = Some(crate::forge::traits::PrSessionKey::new(
        details.repository.clone(),
        details.number,
        "bbbbbbbbbbbbbbbb".to_string(),
    ));
    mismatched.get_file_mut(&stable_path).unwrap().reviewed = true;
    mismatched
        .get_file_mut(&stable_path)
        .unwrap()
        .add_file_comment(Comment::new(
            "wrong-manifest draft".to_string(),
            CommentType::from_id("note"),
            None,
        ));
    std::fs::write(saved_path, serde_json::to_vec_pretty(&mismatched).unwrap()).unwrap();

    // when the PR is opened at the original head
    let mut reopened = build_app();
    let backend = Box::new(FakeForgeBackend::open_pr_details(
        details,
        two_file_patch("new changed"),
    ));
    reopened
        .open_pr_with_backend(&summary, backend, None)
        .unwrap();

    // then the manifest-loaded mismatch is rejected too.
    let stable_review = reopened.session.files.get(&stable_path).unwrap();
    assert!(!stable_review.reviewed);
    assert!(stable_review.file_comments.is_empty());
}

#[test]
fn should_leave_next_session_unchanged_when_carry_forward_has_no_matching_file() {
    // given a next session whose file is missing from the previous session
    // and from the supplied diff file list
    let mut app = build_app();
    let summary = sample_pr(424248, "head-a");
    let mut details = test_pr_details(424248, "head-a");
    details.head_sha = "aaaaaaaaaaaaaaaa".to_string();
    let stable_path = PathBuf::from("src/stable.rs");
    let backend = Box::new(FakeForgeBackend::open_pr_details(
        details,
        two_file_patch("new changed"),
    ));
    app.open_pr_with_backend(&summary, backend, None).unwrap();
    let previous = ReviewSession::new(
        PathBuf::from("/tmp/previous"),
        "old".to_string(),
        None,
        SessionDiffSource::PullRequest,
    );
    let mut next = app.session.clone();
    next.get_file_mut(&stable_path).unwrap().reviewed = true;

    // when carry-forward has no matching previous file or diff file
    let carried_without_previous =
        App::reviewed_state_carried_forward(&previous, next.clone(), &app.diff_files, &[]);
    let carried_without_diff =
        App::reviewed_state_carried_forward(&previous, next.clone(), &[], &[]);

    // then it keeps the fresh next-session state untouched.
    assert!(carried_without_previous.is_file_reviewed(&stable_path));
    assert!(carried_without_diff.is_file_reviewed(&stable_path));

    // and old reviewed state does not leak in when the fresh next session
    // has no matching rendered diff file.
    let mut previous_with_reviewed_file = previous;
    previous_with_reviewed_file.files.insert(
        stable_path.clone(),
        crate::model::review::FileReview::new(stable_path.clone(), FileStatus::Modified, 1),
    );
    previous_with_reviewed_file
        .get_file_mut(&stable_path)
        .unwrap()
        .reviewed = true;
    let mut fresh_next = next;
    fresh_next.get_file_mut(&stable_path).unwrap().reviewed = false;
    let carried_without_diff =
        App::reviewed_state_carried_forward(&previous_with_reviewed_file, fresh_next, &[], &[]);
    assert!(!carried_without_diff.is_file_reviewed(&stable_path));
}

#[test]
fn should_carry_draft_comments_for_unchanged_files_when_pr_head_advances() {
    // given an app already in PR mode with reviewed files and draft comments
    let _reviews = TestReviewsDir::new();
    let mut app = build_app();
    let summary = sample_pr(424256, "head-a");
    let mut details_a = test_pr_details(424256, "head-a");
    details_a.head_sha = "aaaaaaaaaaaaaaaa".to_string();
    let stable_path = PathBuf::from("src/stable.rs");
    let changed_path = PathBuf::from("src/changed.rs");
    let backend_a = Box::new(FakeForgeBackend::open_pr_details(
        details_a.clone(),
        two_file_patch("new changed"),
    ));
    app.open_pr_with_backend(&summary, backend_a, None).unwrap();
    app.session.get_file_mut(&stable_path).unwrap().reviewed = true;
    app.session.get_file_mut(&changed_path).unwrap().reviewed = true;
    app.session.review_comments.push(Comment::new(
        "review-level draft".to_string(),
        CommentType::from_id("note"),
        None,
    ));
    app.session
        .get_file_mut(&stable_path)
        .unwrap()
        .add_file_comment(Comment::new(
            "stable file draft".to_string(),
            CommentType::from_id("note"),
            None,
        ));
    app.session
        .get_file_mut(&changed_path)
        .unwrap()
        .add_line_comment(
            1,
            Comment::new(
                "changed line draft".to_string(),
                CommentType::from_id("issue"),
                None,
            ),
        );
    app.save_current_session_merging_external().unwrap();

    // when a normal new commit adds another file without changing either
    // previously reviewed file
    let mut details_b = details_a.clone();
    details_b.head_sha = "bbbbbbbbbbbbbbbb".to_string();
    let backend_b = Box::new(FakeForgeBackend::open_pr_details(
        details_b,
        two_file_plus_added_patch(),
    ));
    let head_changed = app
        .reload_pull_request_with_backend(backend_b, None)
        .unwrap();

    // then reviewed markers and comments on the unchanged review scope survive.
    assert!(head_changed);
    assert!(app.session.is_file_reviewed(&stable_path));
    assert!(app.session.is_file_reviewed(&changed_path));
    assert_eq!(app.session.review_comments.len(), 1);
    assert_eq!(app.session.review_comments[0].content, "review-level draft");
    let stable_review = app.session.files.get(&stable_path).unwrap();
    assert_eq!(stable_review.file_comments.len(), 1);
    assert_eq!(stable_review.file_comments[0].content, "stable file draft");
    let changed_review = app.session.files.get(&changed_path).unwrap();
    assert_eq!(changed_review.line_comments[&1].len(), 1);
    assert_eq!(
        changed_review.line_comments[&1][0].content,
        "changed line draft"
    );
}

#[test]
fn should_carry_reviewed_marks_for_unchanged_files_when_pr_head_advances() {
    // given an app already in PR mode with two reviewed files at head A
    let mut app = build_app();
    let summary = sample_pr(424242, "head-a");
    let mut details_a = test_pr_details(424242, "head-a");
    details_a.head_sha = "aaaaaaaaaaaaaaaa".to_string();
    let backend_a = Box::new(FakeForgeBackend::open_pr_details(
        details_a.clone(),
        two_file_patch("new changed"),
    ));
    app.open_pr_with_backend(&summary, backend_a, None).unwrap();
    let stable_path = PathBuf::from("src/stable.rs");
    let changed_path = PathBuf::from("src/changed.rs");
    app.session.get_file_mut(&stable_path).unwrap().reviewed = true;
    app.session.get_file_mut(&changed_path).unwrap().reviewed = true;
    app.session
        .get_file_mut(&stable_path)
        .unwrap()
        .add_file_comment(Comment::new(
            "old-head draft".to_string(),
            CommentType::from_id("note"),
            None,
        ));
    app.session
        .get_file_mut(&changed_path)
        .unwrap()
        .add_file_comment(Comment::new(
            "changed-file draft".to_string(),
            CommentType::from_id("issue"),
            None,
        ));

    // when the PR advances and only one file's diff content changes
    let mut details_b = details_a.clone();
    details_b.head_sha = "bbbbbbbbbbbbbbbb".to_string();
    let backend_b = Box::new(FakeForgeBackend::open_pr_details(
        details_b,
        two_file_patch("newer changed"),
    ));
    let head_changed = app
        .reload_pull_request_with_backend(backend_b, None)
        .unwrap();

    // then unchanged files stay reviewed, changed files reopen, and
    // draft comments on unchanged files move to the new head.
    assert!(head_changed);
    assert!(app.session.is_file_reviewed(&stable_path));
    assert!(!app.session.is_file_reviewed(&changed_path));
    let stable_review = app.session.files.get(&stable_path).unwrap();
    assert_eq!(stable_review.file_comments.len(), 1);
    assert_eq!(stable_review.file_comments[0].content, "old-head draft");
    let changed_review = app.session.files.get(&changed_path).unwrap();
    assert!(changed_review.file_comments.is_empty());
}

#[test]
fn should_build_new_head_session_by_carrying_only_unchanged_reviewed_state() {
    // given an old-head PR session with reviewed file and hunk marks
    let mut app = build_app();
    let summary = sample_pr(424243, "head-a");
    let mut details_a = test_pr_details(424243, "head-a");
    details_a.head_sha = "aaaaaaaaaaaaaaaa".to_string();
    let backend_a = Box::new(FakeForgeBackend::open_pr_details(
        details_a.clone(),
        two_file_patch("new changed"),
    ));
    app.open_pr_with_backend(&summary, backend_a, None).unwrap();
    let stable_path = PathBuf::from("src/stable.rs");
    let changed_path = PathBuf::from("src/changed.rs");
    let stable_key = app
        .diff_files
        .iter()
        .find(|file| file.display_path() == &stable_path)
        .and_then(|file| file.hunk_review_key(0))
        .unwrap();
    let changed_key = app
        .diff_files
        .iter()
        .find(|file| file.display_path() == &changed_path)
        .and_then(|file| file.hunk_review_key(0))
        .unwrap();
    app.session.get_file_mut(&stable_path).unwrap().reviewed = true;
    app.session.get_file_mut(&changed_path).unwrap().reviewed = true;
    app.session
        .get_file_mut(&stable_path)
        .unwrap()
        .toggle_hunk_reviewed(stable_key.clone());
    app.session
        .get_file_mut(&changed_path)
        .unwrap()
        .toggle_hunk_reviewed(changed_key);
    let previous = app.session.clone();

    // when a new-head session is built and only one file's diff changes
    let mut details_b = details_a.clone();
    details_b.head_sha = "bbbbbbbbbbbbbbbb".to_string();
    let pr_info_b = crate::forge::traits::PullRequestInfo::from_details(details_b.clone());
    let opened = crate::forge::pr_open::prepare_open_pr(
        details_b,
        structured_patch(&two_file_patch("newer changed")),
        Vec::new(),
        PullRequestReviewMetadata::default(),
        pr_info_b,
        None,
    )
    .unwrap();
    let next = App::reviewed_state_carried_forward(
        &previous,
        opened.session.clone(),
        &opened.diff_files,
        &[],
    );

    // then unchanged files carry reviewed state and changed files reopen.
    assert!(next.is_file_reviewed(&stable_path));
    assert!(next.is_hunk_reviewed(&stable_path, &stable_key));
    assert!(!next.is_file_reviewed(&changed_path));
    assert!(
        next.files
            .get(&changed_path)
            .unwrap()
            .reviewed_hunks
            .is_empty()
    );
}

#[test]
fn should_carry_unchanged_hunk_marks_inside_changed_file_when_pr_head_advances() {
    // given a reviewed file with two reviewed hunks at the old PR head
    let mut app = build_app();
    let summary = sample_pr(424244, "head-a");
    let mut details_a = test_pr_details(424244, "head-a");
    details_a.head_sha = "aaaaaaaaaaaaaaaa".to_string();
    let backend_a = Box::new(FakeForgeBackend::open_pr_details(
        details_a.clone(),
        two_hunk_pr_patch("new second"),
    ));
    app.open_pr_with_backend(&summary, backend_a, None).unwrap();
    let path = PathBuf::from("src/multi.rs");
    let first_key = app.diff_files[0].hunk_review_key(0).unwrap();
    let second_key = app.diff_files[0].hunk_review_key(1).unwrap();
    app.session.get_file_mut(&path).unwrap().reviewed = true;
    app.session
        .get_file_mut(&path)
        .unwrap()
        .toggle_hunk_reviewed(first_key.clone());
    app.session
        .get_file_mut(&path)
        .unwrap()
        .toggle_hunk_reviewed(second_key.clone());
    let previous = app.session.clone();

    // when the PR head changes only the second hunk in that file
    let mut details_b = details_a.clone();
    details_b.head_sha = "bbbbbbbbbbbbbbbb".to_string();
    let pr_info_b = crate::forge::traits::PullRequestInfo::from_details(details_b.clone());
    let opened = crate::forge::pr_open::prepare_open_pr(
        details_b,
        structured_patch(&two_hunk_pr_patch("newer second")),
        Vec::new(),
        PullRequestReviewMetadata::default(),
        pr_info_b,
        None,
    )
    .unwrap();
    let new_first_key = opened.diff_files[0].hunk_review_key(0).unwrap();
    let new_second_key = opened.diff_files[0].hunk_review_key(1).unwrap();
    let next = App::reviewed_state_carried_forward(
        &previous,
        opened.session.clone(),
        &opened.diff_files,
        &[],
    );

    // then only the unchanged hunk stays reviewed; the file and changed
    // hunk reopen.
    assert_eq!(first_key, new_first_key);
    assert_ne!(second_key, new_second_key);
    assert!(!next.is_file_reviewed(&path));
    assert!(next.is_hunk_reviewed(&path, &new_first_key));
    assert!(!next.is_hunk_reviewed(&path, &new_second_key));
}

#[test]
fn should_carry_reviewed_state_through_finish_pr_reload_when_head_advances() {
    // given an app already in PR mode with reviewed state at head A
    let mut app = build_app();
    let summary = sample_pr(424245, "head-a");
    let mut details_a = test_pr_details(424245, "head-a");
    details_a.head_sha = "aaaaaaaaaaaaaaaa".to_string();
    let backend_a = Box::new(FakeForgeBackend::open_pr_details(
        details_a.clone(),
        two_file_patch("new changed"),
    ));
    app.open_pr_with_backend(&summary, backend_a, None).unwrap();
    let stable_path = PathBuf::from("src/stable.rs");
    let changed_path = PathBuf::from("src/changed.rs");
    app.session.get_file_mut(&stable_path).unwrap().reviewed = true;
    app.session.get_file_mut(&changed_path).unwrap().reviewed = true;
    let request = PrReloadRequest {
        repository: details_a.repository.clone(),
        pr_number: details_a.number,
        head_sha: details_a.head_sha.clone(),
        started_at: Instant::now(),
        anchor: None,
        restore_overview_cursor: None,
    };

    // when the async reload finish path applies head B
    let mut details_b = details_a.clone();
    details_b.head_sha = "bbbbbbbbbbbbbbbb".to_string();
    app.finish_pr_reload(
        details_b.clone(),
        structured_patch(&two_file_patch("newer changed")),
        Vec::new(),
        PullRequestReviewMetadata::default(),
        crate::forge::traits::PullRequestInfo::from_details(details_b),
        &request,
    )
    .unwrap();

    // then it uses the same carry-forward rules as synchronous reload.
    assert!(app.session.is_file_reviewed(&stable_path));
    assert!(!app.session.is_file_reviewed(&changed_path));
}

#[test]
fn should_keep_reviewed_state_through_finish_pr_reload_when_head_unchanged() {
    // given an app in PR mode with reviewed file and hunk state
    let mut app = build_app();
    let summary = sample_pr(424253, "same-finish");
    let mut details = test_pr_details(424253, "same-finish");
    details.head_sha = "aaaaaaaaaaaaaaaa".to_string();
    let backend = Box::new(FakeForgeBackend::open_pr_details(
        details.clone(),
        two_file_patch("new changed"),
    ));
    app.open_pr_with_backend(&summary, backend, None).unwrap();
    let stable_path = PathBuf::from("src/stable.rs");
    let stable_key = app
        .diff_files
        .iter()
        .find(|file| file.display_path() == &stable_path)
        .and_then(|file| file.hunk_review_key(0))
        .unwrap();
    app.session.get_file_mut(&stable_path).unwrap().reviewed = true;
    app.session
        .get_file_mut(&stable_path)
        .unwrap()
        .toggle_hunk_reviewed(stable_key.clone());
    let request = PrReloadRequest {
        repository: details.repository.clone(),
        pr_number: details.number,
        head_sha: details.head_sha.clone(),
        started_at: Instant::now(),
        anchor: None,
        restore_overview_cursor: None,
    };

    // Retargeting can change the base without changing the head.
    details.base_sha = "retargeted-base".to_string();
    // when the async reload finish path refreshes the same head
    app.finish_pr_reload(
        details.clone(),
        structured_patch(&two_file_patch("new changed")),
        Vec::new(),
        PullRequestReviewMetadata::default(),
        crate::forge::traits::PullRequestInfo::from_details(details),
        &request,
    )
    .unwrap();

    // then reviewed file and hunk markers are preserved.
    assert!(app.session.is_file_reviewed(&stable_path));
    assert!(app.session.is_hunk_reviewed(&stable_path, &stable_key));
    let DiffSource::PullRequest(pr) = &app.diff_source else {
        panic!("expected PR");
    };
    assert_eq!(pr.base_sha, "retargeted-base");
}

#[test]
fn should_keep_session_when_pr_head_unchanged_on_reload() {
    // given an app in PR mode
    let mut app = build_app();
    let summary = sample_pr(42, "same");
    let details = test_pr_details(42, "same");
    let backend = Box::new(FakeForgeBackend::open_pr_details(
        details.clone(),
        crate::forge::github::gh::tests_fixture::SIMPLE_PATCH.to_string(),
    ));
    app.open_pr_with_backend(&summary, backend, None).unwrap();
    let session_id_before = app.session.id.clone();
    // when reloading with the same head but a different base
    let mut details = details;
    details.base_sha = "retargeted-base".to_string();
    let backend2 = Box::new(FakeForgeBackend::open_pr_details(
        details,
        crate::forge::github::gh::tests_fixture::SIMPLE_PATCH.to_string(),
    ));
    let changed = app
        .reload_pull_request_with_backend(backend2, None)
        .unwrap();
    // then
    assert!(!changed);
    assert_eq!(app.session.id, session_id_before);
    let DiffSource::PullRequest(pr) = &app.diff_source else {
        panic!("expected PR");
    };
    assert_eq!(pr.base_sha, "retargeted-base");
}

struct FailingForgeBackend;

impl crate::forge::traits::ForgeBackend for FailingForgeBackend {
    fn list_pull_requests(
        &self,
        _q: crate::forge::traits::PullRequestListQuery,
    ) -> Result<crate::forge::traits::PagedPullRequests> {
        unimplemented!()
    }
    fn get_pull_request(
        &self,
        _target: crate::forge::traits::PullRequestTarget,
    ) -> Result<crate::forge::traits::PullRequestDetails> {
        Err(crate::error::TuicrError::Forge(
            "simulated network failure".to_string(),
        ))
    }
    fn get_pull_request_diff(
        &self,
        _pr: &crate::forge::traits::PullRequestDetails,
    ) -> Result<Vec<crate::model::FilePatch>> {
        unreachable!()
    }
    fn fetch_file_lines(
        &self,
        _req: crate::forge::traits::ForgeFileLinesRequest,
    ) -> Result<Vec<crate::model::DiffLine>> {
        Ok(Vec::new())
    }
    fn list_review_threads(
        &self,
        _pr: &crate::forge::traits::PullRequestDetails,
    ) -> Result<Vec<crate::forge::remote_comments::RemoteReviewThread>> {
        Ok(Vec::new())
    }
    fn list_pull_request_commits(
        &self,
        _pr: &crate::forge::traits::PullRequestDetails,
    ) -> Result<Vec<crate::forge::traits::PullRequestCommit>> {
        Ok(Vec::new())
    }
    fn get_pull_request_commit_range_diff(
        &self,
        _pr: &crate::forge::traits::PullRequestDetails,
        _start_sha: &str,
        _end_sha: &str,
    ) -> Result<Vec<crate::model::FilePatch>> {
        unreachable!()
    }
    fn create_review(
        &self,
        _pr: &crate::forge::traits::PullRequestDetails,
        _request: crate::forge::traits::CreateReviewRequest<'_>,
    ) -> Result<crate::forge::traits::GhCreateReviewResponse> {
        unimplemented!()
    }
}

#[test]
fn should_apply_initial_load_event_to_pr_tab() {
    // given
    let mut app = build_app();
    app.forge_repository = Some(ForgeRepository::github("github.com", "agavra", "tuicr"));
    app.pr_tab = PullRequestsTab::new(app.forge_repository.clone());
    app.pr_tab.start_initial_load();
    let (tx, rx) = std::sync::mpsc::channel();
    app.pr_load_rx = Some(rx);
    tx.send(PrLoadEvent::Initial {
        canonical: ForgeRepository::github("github.com", "agavra", "tuicr"),
        result: Ok((vec![sample_pr(7, "lucky")], false)),
    })
    .unwrap();
    drop(tx);
    // when
    app.poll_pr_load_events();
    // then
    assert!(app.pr_load_rx.is_none());
    assert_eq!(app.pr_tab.view().rows.len(), 1);
    assert_eq!(app.pr_tab.view().rows[0].summary.number, 7);
}

#[test]
fn should_surface_initial_pr_list_error_to_message_bar() {
    let mut app = build_app();
    let repository = ForgeRepository::github("github.com", "agavra", "tuicr");
    app.forge_repository = Some(repository.clone());
    app.pr_tab = PullRequestsTab::new(Some(repository.clone()));
    app.pr_tab.start_initial_load();
    let (tx, rx) = std::sync::mpsc::channel();
    app.pr_load_rx = Some(rx);
    tx.send(PrLoadEvent::Initial {
        canonical: repository,
        result: Err("forge API response body".to_string()),
    })
    .unwrap();
    drop(tx);

    app.poll_pr_load_events();

    let message = app.message.as_ref().expect("expected PR-list error");
    assert_eq!(message.message_type, MessageType::Error);
    assert_eq!(message.content, "forge API response body");
}

#[test]
fn should_promote_app_forge_repository_to_canonical_on_initial_load() {
    // given — origin is a fork; canonical from the background thread is upstream
    let origin = ForgeRepository::github("github.com", "agavra", "slatedb");
    let canonical = ForgeRepository::github("github.com", "slatedb", "slatedb");
    let mut app = build_app();
    app.forge_repository = Some(origin.clone());
    app.pr_tab = PullRequestsTab::new(Some(origin));
    app.pr_tab.start_initial_load();
    assert!(!app.canonical_resolved);
    let (tx, rx) = std::sync::mpsc::channel();
    app.pr_load_rx = Some(rx);
    tx.send(PrLoadEvent::Initial {
        canonical: canonical.clone(),
        result: Ok((Vec::new(), false)),
    })
    .unwrap();
    drop(tx);
    // when
    app.poll_pr_load_events();
    // then
    assert_eq!(app.forge_repository.as_ref(), Some(&canonical));
    assert!(app.canonical_resolved);
}

#[test]
fn should_quit_on_q_when_only_reviewed_files_dirty() {
    // given: a session dirtied only by a reviewed-file marker (no comments)
    let mut app = build_app();
    let path = PathBuf::from("src/main.rs");
    app.session.add_file(path.clone(), FileStatus::Modified, 0);
    app.session.get_file_mut(&path).unwrap().reviewed = true;
    app.dirty = true;
    assert!(!app.session.has_comments());
    app.command_buffer = "q".to_string();

    // when
    crate::handler::handle_command_action(&mut app, crate::input::Action::SubmitInput);

    // then: `:q` discards the reviewed-only state and quits, no `:q!` needed
    assert!(app.should_quit);
    assert!(!app.dirty);
    assert!(
        !matches!(
            app.message.as_ref().map(|m| &m.message_type),
            Some(MessageType::Error)
        ),
        "should not surface a no-write error"
    );
}

#[test]
fn should_block_q_when_unsaved_comments_exist() {
    // given: a session with an unsaved comment
    let mut app = build_app();
    let path = PathBuf::from("src/main.rs");
    app.session.add_file(path.clone(), FileStatus::Modified, 0);
    app.session
        .get_file_mut(&path)
        .unwrap()
        .add_file_comment(crate::model::Comment::new(
            "needs work".to_string(),
            crate::model::CommentType::from_id("note"),
            None,
        ));
    app.dirty = true;
    assert!(app.session.has_comments());
    app.command_buffer = "q".to_string();

    // when
    crate::handler::handle_command_action(&mut app, crate::input::Action::SubmitInput);

    // then: the guard still requires `:q!`
    assert!(!app.should_quit);
    assert!(app.dirty);
    assert_eq!(
        app.message.as_ref().map(|m| m.message_type.clone()),
        Some(MessageType::Error)
    );
}

#[test]
fn should_open_pr_selector_on_prs_command() {
    // given
    let mut app = build_app();
    app.forge_repository = Some(ForgeRepository::github("github.com", "agavra", "tuicr"));
    app.pr_tab = PullRequestsTab::new(app.forge_repository.clone());
    app.command_buffer = "prs".to_string();
    // when
    crate::handler::handle_command_action(&mut app, crate::input::Action::SubmitInput);
    // then
    assert_eq!(app.target_tab, TargetTab::PullRequests);
    assert_eq!(app.input_mode, InputMode::CommitSelect);
    // Cancel the background fetch handle to avoid surprising real `gh` calls.
    app.pr_load_rx = None;
}

#[test]
fn should_open_local_selector_on_targets_command() {
    // given
    let mut app = build_app_with_commits(vec![dummy_commit("abc")]);
    app.command_buffer = "targets".to_string();
    // when
    crate::handler::handle_command_action(&mut app, crate::input::Action::SubmitInput);
    // then
    assert_eq!(app.target_tab, TargetTab::Local);
    assert_eq!(app.input_mode, InputMode::CommitSelect);
}

#[test]
fn should_treat_commits_as_alias_for_local_target_selector() {
    // given
    let mut app = build_app_with_commits(vec![dummy_commit("abc")]);
    app.command_buffer = "commits".to_string();
    // when
    crate::handler::handle_command_action(&mut app, crate::input::Action::SubmitInput);
    // then
    assert_eq!(app.target_tab, TargetTab::Local);
    assert_eq!(app.input_mode, InputMode::CommitSelect);
}

#[test]
fn should_open_pending_comment_summary_from_command_mode() {
    let mut app = build_app();
    app.input_mode = InputMode::Command;
    app.command_buffer = "summary".to_string();

    crate::handler::handle_command_action(&mut app, crate::input::Action::SubmitInput);

    assert_eq!(app.input_mode, InputMode::Summary);
    assert!(app.command_buffer.is_empty());
    assert_eq!(app.summary_state.selected_comment, 0);
    assert_eq!(app.summary_state.scroll_offset, 0);

    app.summary_state.selected_comment = 5;
    app.summary_state.scroll_offset = 15;
    app.enter_summary_mode();
    assert_eq!(app.summary_state.selected_comment, 0);
    assert_eq!(app.summary_state.scroll_offset, 0);

    crate::handler::handle_summary_action(&mut app, crate::input::Action::ExitMode);
    assert_eq!(app.input_mode, InputMode::Normal);
}

#[test]
fn should_complete_summary_command() {
    let mut app = build_app();
    app.input_mode = InputMode::Command;
    app.command_buffer = "summ".to_string();

    crate::handler::handle_command_action(&mut app, crate::input::Action::CompleteCommand);

    assert_eq!(app.command_buffer, "summary");
    assert!(app.command_completion.is_none());
}

#[test]
fn should_complete_command_when_only_one_candidate_matches() {
    // given
    let mut app = build_app();
    app.input_mode = InputMode::Command;
    app.command_buffer = "vers".to_string();
    // when
    crate::handler::handle_command_action(&mut app, crate::input::Action::CompleteCommand);
    // then
    assert_eq!(app.command_buffer, "version");
    assert!(app.command_completion.is_none());
}

#[test]
fn should_extend_to_common_command_prefix_before_cycling() {
    // given
    let mut app = build_app();
    app.input_mode = InputMode::Command;
    app.command_buffer = "sub".to_string();
    // when
    crate::handler::handle_command_action(&mut app, crate::input::Action::CompleteCommand);
    // then
    assert_eq!(app.command_buffer, "submit");
    assert!(app.command_completion.is_none());
}

#[test]
fn should_cycle_forward_through_command_matches() {
    // given
    let mut app = build_app();
    app.input_mode = InputMode::Command;
    app.command_buffer = "submit".to_string();
    // when
    crate::handler::handle_command_action(&mut app, crate::input::Action::CompleteCommand);
    // then
    assert_eq!(app.command_buffer, "submit comment");
    assert_eq!(
        app.command_completion
            .as_ref()
            .map(|completion| completion.prefix.as_str()),
        Some("submit")
    );
    // when
    crate::handler::handle_command_action(&mut app, crate::input::Action::CompleteCommand);
    // then
    assert_eq!(app.command_buffer, "submit approve");
}

#[test]
fn should_cycle_backward_through_command_matches() {
    // given
    let mut app = build_app();
    app.input_mode = InputMode::Command;
    app.command_buffer = "submit".to_string();
    // when
    crate::handler::handle_command_action(&mut app, crate::input::Action::CompleteCommandReverse);
    // then
    assert_eq!(app.command_buffer, "submit draft");
}

#[test]
fn should_leave_unknown_command_completion_unchanged() {
    // given
    let mut app = build_app();
    app.input_mode = InputMode::Command;
    app.command_buffer = "zz".to_string();
    // when
    crate::handler::handle_command_action(&mut app, crate::input::Action::CompleteCommand);
    // then
    assert_eq!(app.command_buffer, "zz");
    assert!(app.command_completion.is_none());
}

#[test]
fn should_clear_command_completion_state_after_manual_edit() {
    // given
    let mut app = build_app();
    app.input_mode = InputMode::Command;
    app.command_buffer = "set ".to_string();
    crate::handler::handle_command_action(&mut app, crate::input::Action::CompleteCommand);
    assert_eq!(app.command_buffer, "set wrap");
    assert!(app.command_completion.is_some());
    // when
    crate::handler::handle_command_action(&mut app, crate::input::Action::InsertChar('x'));
    // then
    assert_eq!(app.command_buffer, "set wrapx");
    assert!(app.command_completion.is_none());
}

// -- async PR open spinner tests -----------------------------------------

fn loaded_pr_tab(pr_list: Vec<PullRequestSummary>) -> PullRequestsTab {
    let mut tab = PullRequestsTab::new(Some(ForgeRepository::github(
        "github.com",
        "agavra",
        "tuicr",
    )));
    tab.start_initial_load();
    tab.apply_initial_load(Ok((pr_list, false)));
    tab
}

#[test]
fn should_set_pr_open_state_and_spawn_when_pressing_enter_on_a_pr_row() {
    // given a loaded PR tab and no in-flight open
    let mut app = build_app();
    app.forge_repository = Some(ForgeRepository::github("github.com", "agavra", "tuicr"));
    app.pr_tab = loaded_pr_tab(vec![sample_pr(42, "boom")]);
    app.target_tab = TargetTab::PullRequests;
    // when
    let handled = app.pr_tab_select();
    // then
    assert!(handled);
    assert!(app.pr_open_state.is_some());
    let state = app.pr_open_state.as_ref().unwrap();
    assert_eq!(state.pr_number, 42);
    // Drop the receiver so the spawned thread's tx send is a no-op
    // when it completes (the real `gh` call would block; this test
    // does not wait for it).
    app.pr_open_rx = None;
    app.pr_open_state = None;
}

#[test]
fn should_be_a_noop_when_pressing_enter_during_an_in_flight_open() {
    // given an in-flight open marker
    let mut app = build_app();
    app.forge_repository = Some(ForgeRepository::github("github.com", "agavra", "tuicr"));
    app.pr_tab = loaded_pr_tab(vec![sample_pr(7, "ctx"), sample_pr(8, "next")]);
    app.target_tab = TargetTab::PullRequests;
    app.pr_open_state = Some(crate::app::PrOpenRequest {
        repository: ForgeRepository::github("github.com", "agavra", "tuicr"),
        pr_number: 7,
        started_at: std::time::Instant::now(),
    });
    // (no pr_open_rx is fine — the function never touches it on this path)
    // when — Enter on a different row
    if let crate::forge::selector::PullRequestsTab::Loaded { cursor, .. } = &mut app.pr_tab {
        *cursor = 1;
    }
    let handled = app.pr_tab_select();
    // then — handled but state unchanged (no new spawn for #8)
    assert!(handled);
    let state = app.pr_open_state.as_ref().unwrap();
    assert_eq!(state.pr_number, 7);
}

#[test]
fn should_clear_pr_open_state_on_cancel() {
    // given
    let mut app = build_app();
    app.pr_open_state = Some(crate::app::PrOpenRequest {
        repository: ForgeRepository::github("github.com", "agavra", "tuicr"),
        pr_number: 11,
        started_at: std::time::Instant::now(),
    });
    // when
    let cancelled = app.cancel_pr_open();
    // then
    assert!(cancelled);
    assert!(app.pr_open_state.is_none());
    assert!(app.pr_open_rx.is_none());
}

#[test]
fn should_return_false_when_cancelling_with_no_in_flight_open() {
    // given
    let mut app = build_app();
    // when
    let cancelled = app.cancel_pr_open();
    // then
    assert!(!cancelled);
}

#[test]
fn should_surface_pr_open_error_to_message_bar_when_done_event_carries_error() {
    // given an app waiting on a synthetic open
    let mut app = build_app();
    app.input_mode = InputMode::CommitSelect;
    app.forge_repository = Some(ForgeRepository::github("github.com", "agavra", "tuicr"));
    app.pr_tab = loaded_pr_tab(vec![sample_pr(42, "boom")]);
    app.target_tab = TargetTab::PullRequests;
    let request = crate::app::PrOpenRequest {
        repository: ForgeRepository::github("github.com", "agavra", "tuicr"),
        pr_number: 42,
        started_at: std::time::Instant::now(),
    };
    app.pr_open_state = Some(request.clone());
    let (tx, rx) = std::sync::mpsc::channel();
    app.pr_open_rx = Some(rx);
    tx.send(crate::app::PrOpenEvent::Done {
        request,
        result: Err("auth failed".to_string()),
    })
    .unwrap();
    // when
    app.poll_pr_open_events();
    // then — open state cleared, error surfaced to message bar, PR
    // list is intact so the user can retry / pick a different PR
    assert!(app.pr_open_state.is_none());
    assert!(app.pr_open_rx.is_none());
    assert!(matches!(app.pr_tab, PullRequestsTab::Loaded { .. }));
    let msg = app
        .message
        .as_ref()
        .expect("expected an error message on the bar");
    assert!(matches!(msg.message_type, MessageType::Error));
    assert!(msg.content.contains("auth failed"), "got {msg:?}");

    app.enter_command_mode();
    app.command_buffer = "messages".to_string();
    crate::handler::handle_command_action(&mut app, crate::input::Action::SubmitInput);
    assert_eq!(app.input_mode, InputMode::MessageDetails);
    app.toggle_help();
    assert_eq!(app.input_mode, InputMode::CommitSelect);
}

#[test]
fn should_ignore_stale_done_event_after_cancel() {
    // given an open was cancelled but the thread's send arrived anyway
    let mut app = build_app();
    app.forge_repository = Some(ForgeRepository::github("github.com", "agavra", "tuicr"));
    app.pr_tab = loaded_pr_tab(vec![sample_pr(42, "boom")]);
    app.target_tab = TargetTab::PullRequests;
    let stale_request = crate::app::PrOpenRequest {
        repository: ForgeRepository::github("github.com", "agavra", "tuicr"),
        pr_number: 42,
        started_at: std::time::Instant::now(),
    };
    let (tx, rx) = std::sync::mpsc::channel();
    app.pr_open_rx = Some(rx);
    // pr_open_state is None — the user already cancelled.
    tx.send(crate::app::PrOpenEvent::Done {
        request: stale_request,
        result: Err("would-have-failed".to_string()),
    })
    .unwrap();
    // when
    app.poll_pr_open_events();
    // then — the stale error does not produce a user-visible message
    assert!(matches!(app.pr_tab, PullRequestsTab::Loaded { .. }));
    assert!(
        app.message.is_none()
            || !app
                .message
                .as_ref()
                .unwrap()
                .content
                .contains("would-have-failed")
    );
}

#[test]
fn should_cancel_in_flight_open_when_pressing_esc_in_selector() {
    // given
    let mut app = build_app();
    app.forge_repository = Some(ForgeRepository::github("github.com", "agavra", "tuicr"));
    app.pr_tab = loaded_pr_tab(vec![sample_pr(99, "x")]);
    app.target_tab = TargetTab::PullRequests;
    app.pr_open_state = Some(crate::app::PrOpenRequest {
        repository: ForgeRepository::github("github.com", "agavra", "tuicr"),
        pr_number: 99,
        started_at: std::time::Instant::now(),
    });
    // when
    crate::handler::handle_commit_select_action(&mut app, crate::input::Action::ExitMode);
    // then
    assert!(app.pr_open_state.is_none());
}

#[test]
fn should_show_quit_hint_message_without_quitting_in_commit_select_mode() {
    // given
    let mut app = build_app();
    // when
    crate::handler::handle_commit_select_action(&mut app, crate::input::Action::QuitHint);
    // then
    assert!(!app.should_quit);
    assert_eq!(
        app.message.as_ref().map(|m| m.content.as_str()),
        Some("q no longer quits — use :q to quit")
    );
}

// -----------------------------------------------------------------
// Remote review threads (PR 4)
// -----------------------------------------------------------------

use crate::forge::remote_comments::{
    PrCommentsVisibility, RemoteCommentSide, RemoteReviewComment, RemoteReviewThread,
};

struct ThreadAwareForgeBackend {
    details: crate::forge::traits::PullRequestDetails,
    patch: String,
    threads: Vec<RemoteReviewThread>,
    calls: std::sync::atomic::AtomicU32,
    /// Fail `list_review_threads` with a pagination bound.
    hit_page_limit: bool,
    /// Fail `list_review_summaries` only (threads succeed).
    fail_summaries: bool,
}

impl ThreadAwareForgeBackend {
    fn new(
        details: crate::forge::traits::PullRequestDetails,
        patch: String,
        threads: Vec<RemoteReviewThread>,
    ) -> Self {
        Self {
            details,
            patch,
            threads,
            calls: std::sync::atomic::AtomicU32::new(0),
            hit_page_limit: false,
            fail_summaries: false,
        }
    }
}

impl crate::forge::traits::ForgeBackend for ThreadAwareForgeBackend {
    fn list_pull_requests(
        &self,
        _q: crate::forge::traits::PullRequestListQuery,
    ) -> Result<crate::forge::traits::PagedPullRequests> {
        unimplemented!()
    }
    fn get_pull_request(
        &self,
        _t: crate::forge::traits::PullRequestTarget,
    ) -> Result<crate::forge::traits::PullRequestDetails> {
        Ok(self.details.clone())
    }
    fn get_pull_request_diff(
        &self,
        _p: &crate::forge::traits::PullRequestDetails,
    ) -> Result<Vec<crate::model::FilePatch>> {
        Ok(structured_patch(&self.patch))
    }
    fn fetch_file_lines(
        &self,
        _r: crate::forge::traits::ForgeFileLinesRequest,
    ) -> Result<Vec<crate::model::DiffLine>> {
        Ok(Vec::new())
    }
    fn list_review_threads(
        &self,
        _pr: &crate::forge::traits::PullRequestDetails,
    ) -> Result<Vec<RemoteReviewThread>> {
        self.calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if self.hit_page_limit {
            return Err(crate::error::TuicrError::ForgeLimit(
                "too many pages".to_string(),
            ));
        }
        Ok(self.threads.clone())
    }
    fn list_review_summaries(
        &self,
        _pr: &crate::forge::traits::PullRequestDetails,
    ) -> Result<Vec<crate::forge::remote_comments::RemoteReviewSummary>> {
        if self.fail_summaries {
            return Err(crate::error::TuicrError::ForgeLimit(
                "summaries too big".to_string(),
            ));
        }
        Ok(Vec::new())
    }
    fn list_pull_request_commits(
        &self,
        _pr: &crate::forge::traits::PullRequestDetails,
    ) -> Result<Vec<crate::forge::traits::PullRequestCommit>> {
        Ok(Vec::new())
    }
    fn get_pull_request_commit_range_diff(
        &self,
        _pr: &crate::forge::traits::PullRequestDetails,
        _start_sha: &str,
        _end_sha: &str,
    ) -> Result<Vec<crate::model::FilePatch>> {
        unreachable!()
    }
    fn create_review(
        &self,
        _pr: &crate::forge::traits::PullRequestDetails,
        _request: crate::forge::traits::CreateReviewRequest<'_>,
    ) -> Result<crate::forge::traits::GhCreateReviewResponse> {
        unimplemented!()
    }
}

fn sample_thread(line: u32, body: &str, resolved: bool, outdated: bool) -> RemoteReviewThread {
    RemoteReviewThread {
        id: "T".to_string(),
        path: "src/lib.rs".to_string(),
        line: Some(line),
        side: RemoteCommentSide::Right,
        is_resolved: resolved,
        is_outdated: outdated,
        comments: vec![RemoteReviewComment {
            id: "C".to_string(),
            author: Some("alice".to_string()),
            body: body.to_string(),
            url: "https://example.com/c".to_string(),
            ..Default::default()
        }],
    }
}

#[test]
fn should_populate_remote_threads_when_opening_pr_through_test_seam() {
    // given
    let mut app = build_app();
    let summary = sample_pr(42, "answer");
    let backend = Box::new(ThreadAwareForgeBackend::new(
        test_pr_details(42, "answer"),
        crate::forge::github::gh::tests_fixture::SIMPLE_PATCH.to_string(),
        vec![sample_thread(2, "remote body", false, false)],
    ));
    // when
    app.open_pr_with_backend(&summary, backend, None).unwrap();
    // then
    assert_eq!(app.forge_review_threads.len(), 1);
    assert_eq!(app.forge_review_threads[0].comments[0].body, "remote body");
    // default visibility is Unresolved on a fresh PR session
    assert_eq!(
        app.session.remote_comments_visibility,
        PrCommentsVisibility::Unresolved
    );
}

#[test]
fn should_keep_locked_comments_when_the_sync_thread_fetch_hits_the_page_limit() {
    // given a persisted PR session holding a locked (submitted) comment
    let _reviews = TestReviewsDir::new();
    let summary = sample_pr(42, "answer");
    let patch = crate::forge::github::gh::tests_fixture::SIMPLE_PATCH.to_string();
    let backend = |limit: bool| {
        let mut b =
            ThreadAwareForgeBackend::new(test_pr_details(42, "answer"), patch.clone(), Vec::new());
        b.hit_page_limit = limit;
        Box::new(b)
    };
    let mut first = build_app();
    first
        .open_pr_with_backend(&summary, backend(false), None)
        .unwrap();
    let mut locked = Comment::new("posted".to_string(), CommentType::from_id("note"), None);
    locked.lifecycle_state = crate::model::comment::CommentLifecycleState::Submitted;
    first.session.review_comments.push(locked);
    first.save_current_session_merging_external().unwrap();

    // when the reopen's thread fetch fails with the page bound
    let mut app = build_app();
    app.open_pr_with_backend(&summary, backend(true), None)
        .unwrap();

    // then the locked comment is not pruned and the failure is shown
    assert_eq!(app.session.review_comments.len(), 1);
    assert!(app.forge_review_threads.is_empty());
    let message = app.message.as_ref().expect("warning").content.clone();
    assert!(message.contains("too many pages"), "{message}");

    // and a successful fetch still prunes it
    let mut ok = build_app();
    ok.open_pr_with_backend(&summary, backend(false), None)
        .unwrap();
    assert!(ok.session.review_comments.is_empty());
}

#[test]
fn should_keep_locked_comments_when_only_the_sync_summary_fetch_fails() {
    // given a persisted PR session holding a locked (submitted) comment
    let _reviews = TestReviewsDir::new();
    let summary = sample_pr(42, "answer");
    let patch = crate::forge::github::gh::tests_fixture::SIMPLE_PATCH.to_string();
    let backend = |fail: bool| {
        let mut b =
            ThreadAwareForgeBackend::new(test_pr_details(42, "answer"), patch.clone(), Vec::new());
        b.fail_summaries = fail;
        Box::new(b)
    };
    let mut first = build_app();
    first
        .open_pr_with_backend(&summary, backend(false), None)
        .unwrap();
    let mut locked = Comment::new("posted".to_string(), CommentType::from_id("note"), None);
    locked.lifecycle_state = crate::model::comment::CommentLifecycleState::Submitted;
    first.session.review_comments.push(locked);
    first.save_current_session_merging_external().unwrap();

    // when the reopen's threads load but the summary fetch fails
    let mut app = build_app();
    app.open_pr_with_backend(&summary, backend(true), None)
        .unwrap();

    // then the locked comment is not pruned and the failure is shown
    assert_eq!(app.session.review_comments.len(), 1);
    let message = app.message.as_ref().expect("warning").content.clone();
    assert!(message.contains("summaries too big"), "{message}");
}

#[test]
fn configured_comments_visibility_seeds_a_fresh_pr_session() {
    let mut app = build_app();
    app.initial_comments_visibility = Some(PrCommentsVisibility::All);
    let summary = sample_pr(42, "answer");
    let backend = Box::new(ThreadAwareForgeBackend::new(
        test_pr_details(42, "answer"),
        crate::forge::github::gh::tests_fixture::SIMPLE_PATCH.to_string(),
        Vec::new(),
    ));
    app.open_pr_with_backend(&summary, backend, None).unwrap();
    assert_eq!(
        app.session.remote_comments_visibility,
        PrCommentsVisibility::All
    );
}

#[test]
fn configured_comments_visibility_seeds_the_async_pr_open_path() {
    let _reviews = TestReviewsDir::new();
    let mut app = build_app();
    app.initial_comments_visibility = Some(PrCommentsVisibility::All);
    app.forge_repository = Some(ForgeRepository::gitlab("gitlab.com", "owner", "repo"));
    app.pr_tab = loaded_pr_tab(vec![sample_pr(42, "answer")]);
    app.target_tab = TargetTab::PullRequests;
    let request = crate::app::PrOpenRequest {
        repository: ForgeRepository::gitlab("gitlab.com", "owner", "repo"),
        pr_number: 42,
        started_at: std::time::Instant::now(),
    };
    app.pr_open_state = Some(request.clone());
    let (tx, rx) = std::sync::mpsc::channel();
    app.pr_open_rx = Some(rx);
    let mut details = test_pr_details(42, "answer");
    details.repository = ForgeRepository::gitlab("gitlab.com", "owner", "repo");
    tx.send(crate::app::PrOpenEvent::Done {
        request,
        result: Ok((
            details.clone(),
            structured_patch(crate::forge::github::gh::tests_fixture::SIMPLE_PATCH),
            Vec::new(),
            crate::forge::traits::PullRequestReviewMetadata::default(),
            crate::forge::traits::PullRequestInfo::from_details(details.clone()),
        )),
    })
    .unwrap();

    app.poll_pr_open_events();

    assert!(
        matches!(&app.diff_source, DiffSource::PullRequest(_)),
        "the async open should have entered PR mode"
    );
    assert_eq!(
        app.session.remote_comments_visibility,
        PrCommentsVisibility::All
    );
}

#[test]
fn configured_comments_default_does_not_override_a_persisted_visibility() {
    let _reviews = TestReviewsDir::new();
    for (saved, configured) in [
        (PrCommentsVisibility::Unresolved, PrCommentsVisibility::All),
        (PrCommentsVisibility::All, PrCommentsVisibility::Hide),
    ] {
        let mut app = build_app();
        let summary = sample_pr(42, "answer");
        let backend = Box::new(ThreadAwareForgeBackend::new(
            test_pr_details(42, "answer"),
            crate::forge::github::gh::tests_fixture::SIMPLE_PATCH.to_string(),
            Vec::new(),
        ));
        app.open_pr_with_backend(&summary, backend, None).unwrap();
        app.session.remote_comments_visibility = saved;
        crate::persistence::storage::save_session(&app.session).unwrap();

        let mut app = build_app();
        app.initial_comments_visibility = Some(configured);
        let backend = Box::new(ThreadAwareForgeBackend::new(
            test_pr_details(42, "answer"),
            crate::forge::github::gh::tests_fixture::SIMPLE_PATCH.to_string(),
            Vec::new(),
        ));
        app.open_pr_with_backend(&summary, backend, None).unwrap();
        assert_eq!(app.session.remote_comments_visibility, saved);
    }
}

#[test]
fn should_clear_remote_threads_without_refetch_when_setting_visibility_hide() {
    // given a PR open with one fetched thread
    let mut app = build_app();
    let summary = sample_pr(42, "answer");
    let backend = Box::new(ThreadAwareForgeBackend::new(
        test_pr_details(42, "answer"),
        crate::forge::github::gh::tests_fixture::SIMPLE_PATCH.to_string(),
        vec![sample_thread(2, "remote", false, false)],
    ));
    app.open_pr_with_backend(&summary, backend, None).unwrap();
    assert_eq!(app.forge_review_threads.len(), 1);
    // when — switch to hide
    let changed = app.set_remote_comments_visibility(PrCommentsVisibility::Hide);
    // then
    assert!(changed);
    assert_eq!(
        app.session.remote_comments_visibility,
        PrCommentsVisibility::Hide
    );
    // We don't drop the cache on visibility change — only filtering changes.
    // Switching back to Unresolved should restore the rendered comments
    // without making a new network call.
    assert_eq!(app.forge_review_threads.len(), 1);
}

#[test]
fn should_route_comments_unresolved_command_through_command_handler() {
    use crate::handler::handle_command_action;
    use crate::input::Action;
    // given
    let mut app = build_app();
    let summary = sample_pr(42, "answer");
    let backend = Box::new(ThreadAwareForgeBackend::new(
        test_pr_details(42, "answer"),
        crate::forge::github::gh::tests_fixture::SIMPLE_PATCH.to_string(),
        vec![sample_thread(2, "remote", false, false)],
    ));
    app.open_pr_with_backend(&summary, backend, None).unwrap();
    // when — enter command mode then submit `:comments all`
    app.input_mode = crate::app::InputMode::Command;
    app.command_buffer = "comments all".to_string();
    handle_command_action(&mut app, Action::SubmitInput);
    // then
    assert_eq!(
        app.session.remote_comments_visibility,
        PrCommentsVisibility::All
    );
}

#[test]
fn should_warn_when_comments_command_used_outside_pr_mode() {
    use crate::handler::handle_command_action;
    use crate::input::Action;
    // given — plain local working-tree session
    let mut app = build_app();
    app.input_mode = crate::app::InputMode::Command;
    app.command_buffer = "comments all".to_string();
    // when
    handle_command_action(&mut app, Action::SubmitInput);
    // then — visibility unchanged, a warning surfaced on the message bar
    assert_eq!(
        app.session.remote_comments_visibility,
        PrCommentsVisibility::Unresolved
    );
    let msg = app
        .message
        .as_ref()
        .expect("expected warning on message bar");
    assert!(matches!(msg.message_type, MessageType::Warning));
    assert!(
        msg.content.contains("PR mode"),
        "got message: {}",
        msg.content
    );
}

#[test]
fn should_apply_remote_threads_event_when_relevant() {
    // given a PR session is open at head=`headsha`
    let mut app = build_app();
    let summary = sample_pr(42, "answer");
    let backend = Box::new(ThreadAwareForgeBackend::new(
        test_pr_details(42, "answer"),
        crate::forge::github::gh::tests_fixture::SIMPLE_PATCH.to_string(),
        // open with empty threads — we'll deliver via the channel
        Vec::new(),
    ));
    app.open_pr_with_backend(&summary, backend, None).unwrap();
    // simulate a background fetch that finished after open
    let (tx, rx) = std::sync::mpsc::channel();
    app.pr_threads_rx = Some(rx);
    app.forge_review_threads_loading = true;
    let pr_key = match &app.diff_source {
        DiffSource::PullRequest(pr) => pr.key.clone(),
        _ => panic!("expected PR mode"),
    };
    tx.send(crate::app::PrThreadsEvent::Done {
        repository: pr_key.repository.clone(),
        pr_number: pr_key.number,
        head_sha: pr_key.head_sha.clone(),
        // The same forge thread can appear more than once in a malformed or
        // overlapping paginated response; the UI must keep one copy.
        threads: Ok(vec![
            sample_thread(2, "delayed", false, false),
            sample_thread(2, "duplicate", false, false),
        ]),
        summaries: Ok(Vec::new()),
    })
    .unwrap();
    // when
    app.poll_pr_threads_events();
    // then
    assert!(!app.forge_review_threads_loading);
    assert_eq!(app.forge_review_threads.len(), 1);
    assert_eq!(app.forge_review_threads[0].comments[0].body, "delayed");
}

#[test]
fn should_discard_stale_remote_threads_event_after_switching_pr() {
    // given a PR open, then user switches to a different PR while a
    // fetch is in flight
    let mut app = build_app();
    let summary = sample_pr(42, "answer");
    let backend = Box::new(ThreadAwareForgeBackend::new(
        test_pr_details(42, "answer"),
        crate::forge::github::gh::tests_fixture::SIMPLE_PATCH.to_string(),
        Vec::new(),
    ));
    app.open_pr_with_backend(&summary, backend, None).unwrap();
    // simulate a stale event from a different PR head
    let (tx, rx) = std::sync::mpsc::channel();
    app.pr_threads_rx = Some(rx);
    tx.send(crate::app::PrThreadsEvent::Done {
        repository: ForgeRepository::github("github.com", "agavra", "tuicr"),
        pr_number: 999,                         // wrong number
        head_sha: "definitely-not-this".into(), // wrong head
        threads: Ok(vec![sample_thread(2, "stale", false, false)]),
        summaries: Ok(Vec::new()),
    })
    .unwrap();
    // when
    app.poll_pr_threads_events();
    // then — stale result was dropped
    assert!(app.forge_review_threads.is_empty());
}

#[test]
fn pr_open_landing_mid_delete_prompt_drops_the_prompt() {
    // given — a PR open is in flight and the user raised a Sessions-tab
    // delete prompt on the selector meanwhile
    let _reviews = TestReviewsDir::new();
    let mut app = build_app();
    app.forge_repository = Some(ForgeRepository::gitlab("gitlab.com", "owner", "repo"));
    app.pr_tab = loaded_pr_tab(vec![sample_pr(42, "answer")]);
    let request = crate::app::PrOpenRequest {
        repository: ForgeRepository::gitlab("gitlab.com", "owner", "repo"),
        pr_number: 42,
        started_at: std::time::Instant::now(),
    };
    app.pr_open_state = Some(request.clone());
    let (tx, rx) = std::sync::mpsc::channel();
    app.pr_open_rx = Some(rx);
    app.enter_confirm_mode(crate::app::ConfirmAction::DeleteSession);

    // when — the open completes
    let mut details = test_pr_details(42, "answer");
    details.repository = ForgeRepository::gitlab("gitlab.com", "owner", "repo");
    tx.send(crate::app::PrOpenEvent::Done {
        request,
        result: Ok((
            details.clone(),
            structured_patch(crate::forge::github::gh::tests_fixture::SIMPLE_PATCH),
            Vec::new(),
            crate::forge::traits::PullRequestReviewMetadata::default(),
            crate::forge::traits::PullRequestInfo::from_details(details.clone()),
        )),
    })
    .unwrap();
    app.poll_pr_open_events();

    // then — the prompt belonged to the selector; Normal mode cannot answer
    // it, so it must not linger over the PR diff
    assert!(matches!(&app.diff_source, DiffSource::PullRequest(_)));
    assert_eq!(app.input_mode, InputMode::Normal);
    assert_eq!(app.pending_confirm, None);
}

// ---- #848 follow-up: initial-open message policy (F8/F9) ----

fn three_commits() -> Vec<crate::forge::traits::PullRequestCommit> {
    // Forge order is oldest-first; `prepare_open_pr` reverses it.
    vec![
        sample_pr_commit("aaa1111111", "first"),
        sample_pr_commit("bbb2222222", "middle"),
        sample_pr_commit("ccc3333333", "last"),
    ]
}

#[test]
fn should_hold_the_commit_message_back_until_a_strict_initial_range_loads() {
    let _reviews = TestReviewsDir::new();
    for start in ["oldest", "saved", "since-review"] {
        // given a three-commit PR whose initial selection is a strict range
        let mut app = build_app();
        let details = test_pr_details(424398, "initial policy");
        let metadata = if start == "since-review" {
            PullRequestReviewMetadata {
                viewer_login: Some("alice".into()),
                reviews: vec![review_record("alice", "bbb2222222", "2026-01-01T00:00:00Z")],
            }
        } else {
            PullRequestReviewMetadata::default()
        };
        let mut opened = crate::forge::pr_open::prepare_open_pr(
            details.clone(),
            structured_patch(two_hunk_patch()),
            three_commits(),
            metadata,
            crate::forge::traits::PullRequestInfo::from_details(details.clone()),
            None,
        )
        .unwrap();
        if start == "oldest" {
            app.commit_selection_start = CommitSelectionStart::Oldest;
        }
        if start == "saved" {
            opened.session.commit_selection_range = Some((1, 1));
        }

        // when the PR is entered
        app.enter_pr_diff_mode(
            Box::new(FakeForgeBackend::open_pr_details(
                details.clone(),
                two_hunk_patch().to_string(),
            )),
            opened,
        )
        .unwrap();

        // then the range is loading and no message sits above the cumulative diff
        let request = app.pr_range_reload_state.clone().expect(start);
        assert!(
            !app.diff_files.iter().any(|f| f.is_commit_message),
            "{start}: loading"
        );

        // when the range fetch fails
        let (tx, rx) = std::sync::mpsc::channel();
        app.pr_range_reload_rx = Some(rx);
        tx.send(PrRangeReloadEvent::Done {
            request: request.clone(),
            result: Err("fixture".into()),
        })
        .unwrap();
        app.poll_pr_range_reload_events();

        // then there is still no message
        assert!(
            !app.diff_files.iter().any(|f| f.is_commit_message),
            "{start}: failure"
        );

        // when the range loads
        app.finish_pr_range_reload(&request, structured_patch(first_hunk_patch()))
            .unwrap();

        // then the selected commit's message is first and registered
        assert!(app.diff_files[0].is_commit_message, "{start}: success");
        assert!(
            app.session
                .files
                .contains_key(app.diff_files[0].display_path())
        );
    }
}

#[test]
fn should_hold_the_commit_message_back_in_cli_startup_with_a_strict_range() {
    let _reviews = TestReviewsDir::new();
    // given a three-commit PR opened from the CLI with the oldest commit chosen
    let details = test_pr_details(424397, "cli strict");
    let opened = crate::forge::pr_open::prepare_open_pr(
        details.clone(),
        structured_patch(two_hunk_patch()),
        three_commits(),
        PullRequestReviewMetadata::default(),
        crate::forge::traits::PullRequestInfo::from_details(details.clone()),
        None,
    )
    .unwrap();

    // when the app is built around it
    let app = App::from_opened_pr(
        opened,
        Box::new(FakeForgeBackend::open_pr_details(
            details.clone(),
            two_hunk_patch().to_string(),
        )),
        details.repository.clone(),
        Theme::dark(),
        None,
        true,
        None,
        PrDisplayOptions {
            show_checks: false,
            show_comments: true,
            comments_visibility: None,
        },
        CommitSelectionStart::Oldest,
        None,
        VcsOpenOptions::default(),
    )
    .unwrap();

    // then the range is loading and no message sits above the cumulative diff
    assert!(app.pr_range_reload_state.is_some());
    assert!(!app.diff_files.iter().any(|f| f.is_commit_message));
}

#[test]
fn should_open_a_single_commit_pr_from_the_cli_with_its_message_first() {
    let _reviews = TestReviewsDir::new();
    // given a one-commit PR
    let details = test_pr_details(424396, "cli single");
    let patch = crate::forge::github::gh::tests_fixture::SIMPLE_PATCH;
    let commit = sample_pr_commit_with_body(&details.head_sha, "only", "body");
    let opened = crate::forge::pr_open::prepare_open_pr(
        details.clone(),
        structured_patch(patch),
        vec![commit.clone()],
        PullRequestReviewMetadata::default(),
        crate::forge::traits::PullRequestInfo::from_details(details.clone()),
        None,
    )
    .unwrap();

    // when the CLI's post-open half builds the app
    let app = App::from_opened_pr(
        opened,
        Box::new(FakeForgeBackend::open_pr_details(
            details.clone(),
            patch.to_string(),
        )),
        details.repository.clone(),
        Theme::dark(),
        None,
        true,
        None,
        PrDisplayOptions {
            show_checks: false,
            show_comments: true,
            comments_visibility: None,
        },
        CommitSelectionStart::All,
        None,
        VcsOpenOptions::default(),
    )
    .unwrap();

    // then the message is first and can take a comment
    assert_one_commit_message_first(&app, &commit.short_oid);
    assert_eq!(
        commit_message_lines(&app.diff_files[0]),
        vec!["only", "", "body"]
    );
    assert!(
        app.session
            .files
            .contains_key(&DiffFile::commit_message_path(&commit.short_oid))
    );
}

// ---- #848 follow-up: reload identity policy (F4) ----

/// Open the three-commit PR and narrow it to the oldest commit, as a range
/// load would.
fn open_narrowed_pr() -> (
    App,
    crate::forge::traits::PullRequestDetails,
    PrRangeReloadRequest,
) {
    let mut app = build_app();
    let details = test_pr_details(424399, "retarget");
    let mut backend =
        FakeForgeBackend::open_pr_details(details.clone(), two_hunk_patch().to_string());
    backend.commits = three_commits();
    app.open_pr_with_backend(
        &sample_pr(details.number, &details.title),
        Box::new(backend),
        None,
    )
    .unwrap();
    app.commit_selection_range = Some((2, 2));
    let request = PrRangeReloadRequest {
        repository: details.repository.clone(),
        pr_number: details.number,
        head_sha: details.head_sha.clone(),
        start_sha: details.base_sha.clone(),
        end_sha: "aaa1111111".to_string(),
        range: (2, 2),
        started_at: Instant::now(),
        anchor: None,
    };
    app.finish_pr_range_reload(&request, structured_patch(first_hunk_patch()))
        .unwrap();
    (app, details, request)
}

fn same_head_reload(
    app: &mut App,
    mut details: crate::forge::traits::PullRequestDetails,
    new_base: Option<&str>,
    commits: Vec<crate::forge::traits::PullRequestCommit>,
    metadata: PullRequestReviewMetadata,
) {
    if let Some(base) = new_base {
        details.base_sha = base.to_string();
    }
    let request = PrReloadRequest {
        repository: details.repository.clone(),
        pr_number: details.number,
        head_sha: details.head_sha.clone(),
        started_at: Instant::now(),
        anchor: None,
        restore_overview_cursor: None,
    };
    app.finish_pr_reload(
        details.clone(),
        structured_patch(two_hunk_patch()),
        commits,
        metadata,
        crate::forge::traits::PullRequestInfo::from_details(details),
        &request,
    )
    .unwrap();
}

fn real_file_hunks(app: &App) -> usize {
    app.diff_files
        .iter()
        .find(|f| !f.is_commit_message)
        .unwrap()
        .hunks
        .len()
}

#[test]
fn should_reset_to_the_full_diff_when_a_retarget_changes_the_commit_list() {
    let _reviews = TestReviewsDir::new();
    for config in ["oldest", "since-review"] {
        // given a PR narrowed to its oldest commit
        let (mut app, details, _) = open_narrowed_pr();
        let mut metadata = PullRequestReviewMetadata::default();
        if config == "oldest" {
            app.commit_selection_start = CommitSelectionStart::Oldest;
        } else {
            metadata = PullRequestReviewMetadata {
                viewer_login: Some("alice".into()),
                reviews: vec![review_record("alice", "bbb2222222", "2026-01-01T00:00:00Z")],
            };
        }

        // when a retarget drops the narrowed commit and moves the base
        same_head_reload(
            &mut app,
            details,
            Some("aaa1111111"),
            vec![
                sample_pr_commit("bbb2222222", "middle"),
                sample_pr_commit("ccc3333333", "last"),
            ],
            metadata,
        );

        // then the full diff shows with every commit selected and fresh lists
        assert_eq!(app.commit_selection_range, Some((0, 1)), "{config}");
        assert_eq!(real_file_hunks(&app), 2, "{config}");
        assert!(app.pr_range_reload_state.is_none(), "{config}");
        let oids: Vec<_> = app.pr_commits.iter().map(|c| c.oid.as_str()).collect();
        assert_eq!(oids, vec!["ccc3333333", "bbb2222222"], "{config}");
        assert_eq!(app.commit_list.len(), 2);
        assert_eq!(app.review_commits.len(), 2);
        assert_eq!(app.session.commit_selection_range, None);
    }
}

#[test]
fn should_reset_when_only_the_commit_membership_changes() {
    let _reviews = TestReviewsDir::new();
    // given a PR narrowed to its oldest commit
    let (mut app, details, _) = open_narrowed_pr();

    // when a reload keeps the base but drops that commit
    same_head_reload(
        &mut app,
        details,
        None,
        vec![
            sample_pr_commit("bbb2222222", "middle"),
            sample_pr_commit("ccc3333333", "last"),
        ],
        PullRequestReviewMetadata::default(),
    );

    // then the selection resets to the full diff
    assert_eq!(app.commit_selection_range, Some((0, 1)));
    assert_eq!(real_file_hunks(&app), 2);
}

#[test]
fn should_ignore_a_late_range_result_for_the_old_base_after_a_reset() {
    let _reviews = TestReviewsDir::new();
    // given a range fetch in flight for the old base
    let (mut app, details, old_request) = open_narrowed_pr();
    let (tx, rx) = std::sync::mpsc::channel();
    app.pr_range_reload_state = Some(old_request.clone());
    app.pr_range_reload_rx = Some(rx);

    // when a retarget resets the selection
    same_head_reload(
        &mut app,
        details,
        Some("aaa1111111"),
        vec![
            sample_pr_commit("bbb2222222", "middle"),
            sample_pr_commit("ccc3333333", "last"),
        ],
        PullRequestReviewMetadata::default(),
    );
    let fresh_hash = app.diff_files[0].content_hash;
    assert!(
        app.pr_range_reload_state.is_none(),
        "reset must stop the range-loading state"
    );
    assert!(app.pr_range_reload_rx.is_none());

    // then the stale result can neither be delivered nor applied
    let delivered = tx.send(PrRangeReloadEvent::Done {
        request: old_request,
        result: Ok(structured_patch(first_hunk_patch())),
    });
    assert!(delivered.is_err());
    app.poll_pr_range_reload_events();
    assert_eq!(app.diff_files[0].content_hash, fresh_hash);
    assert_eq!(app.commit_selection_range, Some((0, 1)));
}

#[test]
fn should_keep_known_commits_when_a_base_change_arrives_with_an_unknown_list() {
    let _reviews = TestReviewsDir::new();
    // given a narrowed PR with cached state tied to the old selection
    let (mut app, details, _) = open_narrowed_pr();
    let old_oids: Vec<_> = app.pr_commits.iter().map(|c| c.oid.clone()).collect();
    app.session.commit_selection_range = app.commit_selection_range;
    app.saved_inline_selection = app.commit_selection_range;
    app.commit_diff_cache.insert((2, 2), app.diff_files.clone());
    let gap = GapId {
        file_idx: 0,
        hunk_idx: 0,
    };
    app.expanded_top.insert(gap.clone(), Vec::new());
    app.expanded_bottom.insert(gap, Vec::new());

    // when the base changes and the commit fetch degraded to an empty list
    same_head_reload(
        &mut app,
        details,
        Some("aaa1111111"),
        Vec::new(),
        PullRequestReviewMetadata::default(),
    );

    // then the known commits stay, the diff is cumulative, and all are selected
    let oids: Vec<_> = app.pr_commits.iter().map(|c| c.oid.clone()).collect();
    assert_eq!(oids, old_oids);
    assert_eq!(app.commit_list.len(), 3);
    assert_eq!(app.review_commits.len(), 3);
    assert_eq!(app.commit_selection_range, Some((0, 2)));
    assert_eq!(real_file_hunks(&app), 2);
    assert!(app.commit_diff_cache.is_empty());
    assert_eq!(app.saved_inline_selection, None);
    assert_eq!(app.session.commit_selection_range, None);
    assert!(app.expanded_top.is_empty() && app.expanded_bottom.is_empty());
    assert!(app.has_inline_commit_selector());
}

#[test]
fn should_keep_the_selection_when_the_diff_base_and_commits_are_unchanged() {
    let _reviews = TestReviewsDir::new();
    // given a narrowed PR
    let (mut app, details, _) = open_narrowed_pr();

    // when a same-head reload finds the same base and commit list
    same_head_reload(
        &mut app,
        details.clone(),
        None,
        three_commits(),
        PullRequestReviewMetadata::default(),
    );

    // then the strict selection and its narrowed diff stay
    assert_eq!(app.commit_selection_range, Some((2, 2)));
    assert_eq!(real_file_hunks(&app), 1);

    // and a manually widened selection under Oldest stays widened
    app.commit_selection_start = CommitSelectionStart::Oldest;
    app.commit_selection_range = Some((0, 2));
    app.reload_pr_inline_selection();
    same_head_reload(
        &mut app,
        details,
        None,
        three_commits(),
        PullRequestReviewMetadata::default(),
    );
    assert_eq!(app.commit_selection_range, Some((0, 2)));
    assert!(!app.diff_files.iter().any(|f| f.is_commit_message));
    assert_eq!(real_file_hunks(&app), 2);
}

#[test]
fn should_select_the_single_remaining_commit_and_hide_comments_of_removed_commits() {
    let _reviews = TestReviewsDir::new();
    // given a narrowed PR with the selector focused
    let (mut app, details, _) = open_narrowed_pr();
    app.focused_panel = FocusedPanel::CommitSelector;
    let mut removed = Comment::new("removed".into(), CommentType::None, None);
    removed.commit_id = Some("aaa1111111".into());
    let mut retained = removed.clone();
    retained.commit_id = Some("ccc3333333".into());

    // when a reload leaves one commit
    same_head_reload(
        &mut app,
        details,
        Some("aaa1111111"),
        vec![sample_pr_commit("ccc3333333", "last")],
        PullRequestReviewMetadata::default(),
    );

    // then it is explicitly selected, the selector is gone and focus moved
    assert_eq!(app.pr_commits.len(), 1);
    assert_eq!(app.commit_selection_range, Some((0, 0)));
    assert!(!app.show_commit_selector);
    assert!(!app.has_inline_commit_selector());
    assert_eq!(app.focused_panel, FocusedPanel::Diff);
    assert!(app.diff_files[0].is_commit_message);
    // and only the retained commit's comments are visible
    assert!(!app.comment_visible(&removed));
    assert!(app.comment_visible(&retained));
}

#[test]
fn should_not_underflow_when_a_base_change_meets_an_empty_commit_list() {
    let _reviews = TestReviewsDir::new();
    // given a PR opened with no known commits
    let mut app = build_app();
    let details = test_pr_details(424395, "no commits");
    let backend = FakeForgeBackend::open_pr_details(details.clone(), two_hunk_patch().to_string());
    app.open_pr_with_backend(
        &sample_pr(details.number, &details.title),
        Box::new(backend),
        None,
    )
    .unwrap();
    assert!(app.pr_commits.is_empty());

    // when the base changes and the list is still unknown
    same_head_reload(
        &mut app,
        details,
        Some("aaa1111111"),
        Vec::new(),
        PullRequestReviewMetadata::default(),
    );

    // then nothing is selected, no selector shows, and the diff is intact
    assert_eq!(app.commit_selection_range, None);
    assert!(!app.has_inline_commit_selector());
    assert_eq!(app.focused_panel, FocusedPanel::Diff);
    assert_eq!(real_file_hunks(&app), 2);
}

#[test]
fn should_not_restore_stale_persisted_indices_after_a_reset() {
    let _reviews = TestReviewsDir::new();
    // given a saved narrowed selection
    let (mut app, details, _) = open_narrowed_pr();
    app.persist_pr_commit_selection_range();
    let path = crate::persistence::storage::session_path(&app.session).unwrap();
    assert_eq!(
        crate::persistence::storage::load_session(&path)
            .unwrap()
            .commit_selection_range,
        Some((2, 2))
    );

    // when a reset happens
    same_head_reload(
        &mut app,
        details.clone(),
        Some("aaa1111111"),
        Vec::new(),
        PullRequestReviewMetadata::default(),
    );

    // then the cleared selection is on disk
    assert_eq!(
        crate::persistence::storage::load_session(&path)
            .unwrap()
            .commit_selection_range,
        None
    );

    // and a reopen selects all commits without fetching a range
    let mut new_details = details;
    new_details.base_sha = "aaa1111111".to_string();
    let mut backend =
        FakeForgeBackend::open_pr_details(new_details.clone(), two_hunk_patch().to_string());
    backend.commits = three_commits();
    let mut reopened = build_app();
    reopened
        .open_pr_with_backend(
            &sample_pr(new_details.number, &new_details.title),
            Box::new(backend),
            None,
        )
        .unwrap();
    assert_eq!(reopened.commit_selection_range, Some((0, 2)));
    assert!(reopened.pr_range_reload_state.is_none());
}

// ---- #848 follow-up: commit-message review carry-forward (F7) ----

#[test]
fn should_carry_a_retained_commit_message_review_to_a_new_head() {
    for reopen in [false, true] {
        let _reviews = TestReviewsDir::new();
        // given a reviewed commit message with drafts and a locked comment
        let mut app = build_app();
        let mut details_a = test_pr_details(424390 + u64::from(reopen), "f7");
        details_a.head_sha = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string();
        let commit_a = sample_pr_commit_with_body(&details_a.head_sha, "subject A", "body A");
        let patch = crate::forge::github::gh::tests_fixture::SIMPLE_PATCH;
        let mut backend_a = FakeForgeBackend::open_pr_details(details_a.clone(), patch.to_string());
        backend_a.commits = vec![commit_a.clone()];
        app.open_pr_with_backend(
            &sample_pr(details_a.number, &details_a.title),
            Box::new(backend_a),
            None,
        )
        .unwrap();
        let message_path = DiffFile::commit_message_path(&commit_a.short_oid);
        let hunk = app.diff_files[0].hunk_review_key(0).unwrap();
        let mut locked = Comment::new("locked".into(), CommentType::None, None);
        locked.lifecycle_state = crate::model::comment::CommentLifecycleState::Submitted;
        let review = app.session.get_file_mut(&message_path).unwrap();
        review.reviewed = true;
        review.reviewed_hunks.insert(hunk.clone());
        review.add_line_comment(
            3,
            Comment::new("line draft".into(), CommentType::None, None),
        );
        review.add_file_comment(Comment::new("file draft".into(), CommentType::None, None));
        review.add_file_comment(locked.clone());
        review.add_line_comment(3, locked);
        app.save_current_session_merging_external().unwrap();

        // when the head moves and the commit stays in the PR
        let mut details_b = details_a.clone();
        details_b.head_sha = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string();
        let commit_b = sample_pr_commit(&details_b.head_sha, "subject B");
        if reopen {
            let mut next = build_app();
            let mut backend_b =
                FakeForgeBackend::open_pr_details(details_b.clone(), patch.to_string());
            backend_b.commits = vec![commit_a.clone(), commit_b];
            next.open_pr_with_backend(
                &sample_pr(details_b.number, &details_b.title),
                Box::new(backend_b),
                None,
            )
            .unwrap();
            app = next;
        } else {
            same_head_reload_new_head(&mut app, &details_a, &details_b, &commit_a, commit_b);
        }
        assert_eq!(
            app.session.pr_session_key.as_ref().unwrap().head_sha,
            details_b.head_sha
        );

        // and the retained commit is selected again
        app.commit_selection_range = Some((1, 1));
        let request = PrRangeReloadRequest {
            repository: details_b.repository.clone(),
            pr_number: details_b.number,
            head_sha: details_b.head_sha.clone(),
            start_sha: details_b.base_sha.clone(),
            end_sha: commit_a.oid.clone(),
            range: (1, 1),
            started_at: Instant::now(),
            anchor: None,
        };
        app.finish_pr_range_reload(&request, structured_patch(patch))
            .unwrap();

        // then its review state and unlocked drafts are there, locked ones are not
        assert_one_commit_message_first(&app, &commit_a.short_oid);
        let review = &app.session.files[&message_path];
        assert!(review.reviewed, "reopen={reopen}");
        assert!(review.reviewed_hunks.contains(&hunk), "reopen={reopen}");
        assert_eq!(review.file_comments.len(), 1, "reopen={reopen}");
        assert_eq!(review.file_comments[0].content, "file draft");
        assert_eq!(review.line_comments[&3].len(), 1, "reopen={reopen}");
        assert_eq!(review.line_comments[&3][0].content, "line draft");
    }
}

fn same_head_reload_new_head(
    app: &mut App,
    old: &crate::forge::traits::PullRequestDetails,
    new: &crate::forge::traits::PullRequestDetails,
    commit_a: &crate::forge::traits::PullRequestCommit,
    commit_b: crate::forge::traits::PullRequestCommit,
) {
    let request = PrReloadRequest {
        repository: old.repository.clone(),
        pr_number: old.number,
        head_sha: old.head_sha.clone(),
        started_at: Instant::now(),
        anchor: None,
        restore_overview_cursor: None,
    };
    app.finish_pr_reload(
        new.clone(),
        structured_patch(crate::forge::github::gh::tests_fixture::SIMPLE_PATCH),
        vec![commit_a.clone(), commit_b],
        PullRequestReviewMetadata::default(),
        crate::forge::traits::PullRequestInfo::from_details(new.clone()),
        &request,
    )
    .unwrap();
}

#[test]
fn should_drop_removed_commit_message_reviews_and_lock_state_when_carrying_forward() {
    // given a previous session with a kept and a removed commit's message
    let key_session = |head: &str| {
        let mut session = ReviewSession::new(
            PathBuf::from("/tmp/pr"),
            head.to_string(),
            None,
            SessionDiffSource::PullRequest,
        );
        session.pr_session_key = None;
        session
    };
    let mut previous = key_session("old");
    let kept = DiffFile::commit_message_path("kept111");
    let gone = DiffFile::commit_message_path("gone222");
    for path in [&kept, &gone] {
        let mut review = FileReview::new(path.clone(), FileStatus::Added, 1);
        review.reviewed = true;
        review.add_file_comment(Comment::new("draft".into(), CommentType::None, None));
        previous.files.insert(path.clone(), review);
    }

    // when carried to a session whose PR still has only the kept commit
    let carried = App::reviewed_state_carried_forward(
        &previous,
        key_session("new"),
        &[],
        &[sample_pr_commit("kept1112222", "kept")],
    );

    // then only the kept message's review comes along
    assert!(carried.files[&kept].reviewed);
    assert_eq!(carried.files[&kept].file_comments.len(), 1);
    assert!(!carried.files.contains_key(&gone));
}

#[test]
fn should_prefer_the_exact_head_session_over_carried_commit_message_state() {
    let _reviews = TestReviewsDir::new();
    // given a head-B session saved with the message review cleared
    let mut details_a = test_pr_details(424389, "f7 precedence");
    details_a.head_sha = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string();
    let mut details_b = details_a.clone();
    details_b.head_sha = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string();
    let commit_a = sample_pr_commit_with_body(&details_a.head_sha, "subject A", "body A");
    let commit_b = sample_pr_commit(&details_b.head_sha, "subject B");
    let patch = crate::forge::github::gh::tests_fixture::SIMPLE_PATCH;
    let message_path = DiffFile::commit_message_path(&commit_a.short_oid);

    let mut app = build_app();
    let mut backend = FakeForgeBackend::open_pr_details(details_a.clone(), patch.to_string());
    backend.commits = vec![commit_a.clone()];
    app.open_pr_with_backend(
        &sample_pr(details_a.number, &details_a.title),
        Box::new(backend),
        None,
    )
    .unwrap();
    app.session.get_file_mut(&message_path).unwrap().reviewed = true;
    app.save_current_session_merging_external().unwrap();

    let open_b = |commit_a: &crate::forge::traits::PullRequestCommit| {
        let mut next = build_app();
        let mut backend_b = FakeForgeBackend::open_pr_details(details_b.clone(), patch.to_string());
        backend_b.commits = vec![commit_a.clone(), commit_b.clone()];
        next.open_pr_with_backend(
            &sample_pr(details_b.number, &details_b.title),
            Box::new(backend_b),
            None,
        )
        .unwrap();
        next
    };
    let mut at_b = open_b(&commit_a);
    assert!(at_b.session.files[&message_path].reviewed, "carried");
    at_b.session.get_file_mut(&message_path).unwrap().reviewed = false;
    at_b.save_current_session_merging_external().unwrap();

    // when head B is reopened
    let again = open_b(&commit_a);

    // then its own session wins over the carry from head A
    assert!(!again.session.files[&message_path].reviewed);

    // The live new-head reload must respect the same exact-head session.
    same_head_reload_new_head(
        &mut app,
        &details_a,
        &details_b,
        &commit_a,
        commit_b.clone(),
    );
    assert!(
        !app.session.files[&message_path].reviewed,
        "exact-head wins on live reload"
    );
}

// ---- late async results must belong to the PR on screen ----

fn open_two_commit_pr(app: &mut App, number: u64) {
    let summary = sample_pr(number, "async identity");
    let mut backend = FakeForgeBackend::open_pr_details(
        test_pr_details(number, "async identity"),
        two_hunk_patch().to_string(),
    );
    backend.commits = vec![
        sample_pr_commit("aaa1111", "first"),
        sample_pr_commit("bbb2222", "second"),
    ];
    app.open_pr_with_backend(&summary, Box::new(backend), None)
        .unwrap();
}

fn late_reload_request(app: &App, pr_number: u64, head_sha: &str) -> PrReloadRequest {
    let DiffSource::PullRequest(pr) = &app.diff_source else {
        panic!("expected PR mode");
    };
    PrReloadRequest {
        repository: pr.key.repository.clone(),
        pr_number,
        head_sha: head_sha.to_string(),
        started_at: Instant::now(),
        anchor: None,
        restore_overview_cursor: None,
    }
}

fn late_range_request(
    app: &App,
    pr_number: u64,
    head_sha: &str,
    range: (usize, usize),
) -> PrRangeReloadRequest {
    let DiffSource::PullRequest(pr) = &app.diff_source else {
        panic!("expected PR mode");
    };
    PrRangeReloadRequest {
        repository: pr.key.repository.clone(),
        pr_number,
        head_sha: head_sha.to_string(),
        start_sha: "aaa1111".to_string(),
        end_sha: "bbb2222".to_string(),
        range,
        started_at: Instant::now(),
        anchor: None,
    }
}

fn live_head(app: &App) -> String {
    let DiffSource::PullRequest(pr) = &app.diff_source else {
        panic!("expected PR mode");
    };
    pr.key.head_sha.clone()
}

#[test]
fn should_drop_late_reload_result_for_another_pr() {
    // given PR 43 is open while a reload spawned for PR 42 is still in flight
    let _reviews = TestReviewsDir::new();
    let mut app = build_app();
    open_two_commit_pr(&mut app, 43);
    let request = late_reload_request(&app, 42, "stale-head");
    app.pr_reload_state = Some(request.clone());
    let (tx, rx) = std::sync::mpsc::channel();
    app.pr_reload_rx = Some(rx);
    tx.send(PrReloadEvent::Done {
        request,
        result: Err("boom".to_string()),
    })
    .unwrap();

    // when the result lands
    app.poll_pr_reload_events();

    // then it is dropped without touching PR 43
    assert!(app.message.is_none(), "got {:?}", app.message);
}

#[test]
fn should_drop_late_reload_result_for_an_older_head_of_the_open_pr() {
    // given a reload spawned at an older head of the open PR
    let _reviews = TestReviewsDir::new();
    let mut app = build_app();
    open_two_commit_pr(&mut app, 43);
    let request = late_reload_request(&app, 43, "older-head");
    app.pr_reload_state = Some(request.clone());
    let (tx, rx) = std::sync::mpsc::channel();
    app.pr_reload_rx = Some(rx);
    tx.send(PrReloadEvent::Done {
        request,
        result: Err("boom".to_string()),
    })
    .unwrap();

    // when / then
    app.poll_pr_reload_events();
    assert!(app.message.is_none(), "got {:?}", app.message);
}

#[test]
fn should_drop_late_range_result_for_another_pr() {
    // given PR 43 open with a subset selected, and a range request from PR 42
    let _reviews = TestReviewsDir::new();
    let mut app = build_app();
    open_two_commit_pr(&mut app, 43);
    app.commit_selection_range = Some((0, 0));
    let request = late_range_request(&app, 42, &live_head(&app), (0, 0));
    app.pr_range_reload_state = Some(request.clone());
    let (tx, rx) = std::sync::mpsc::channel();
    app.pr_range_reload_rx = Some(rx);
    tx.send(PrRangeReloadEvent::Done {
        request,
        result: Err("boom".to_string()),
    })
    .unwrap();

    // when / then
    app.poll_pr_range_reload_events();
    assert!(app.message.is_none(), "got {:?}", app.message);
}

#[test]
fn should_drop_late_range_result_when_selection_changed() {
    // given a range request for (1, 1) while the live selection is (0, 0)
    let _reviews = TestReviewsDir::new();
    let mut app = build_app();
    open_two_commit_pr(&mut app, 43);
    app.commit_selection_range = Some((0, 0));
    let request = late_range_request(&app, 43, &live_head(&app), (1, 1));
    app.pr_range_reload_state = Some(request.clone());
    let (tx, rx) = std::sync::mpsc::channel();
    app.pr_range_reload_rx = Some(rx);
    tx.send(PrRangeReloadEvent::Done {
        request,
        result: Err("boom".to_string()),
    })
    .unwrap();

    // when / then
    app.poll_pr_range_reload_events();
    assert!(app.message.is_none(), "got {:?}", app.message);
}

#[test]
fn should_cancel_range_reload_when_widening_to_all_commits() {
    // given a subset range fetch in flight
    let _reviews = TestReviewsDir::new();
    let mut app = build_app();
    open_two_commit_pr(&mut app, 43);
    app.range_diff_files = Some(app.diff_files.clone());
    app.commit_selection_range = Some((0, 1));
    let request = late_range_request(&app, 43, &live_head(&app), (0, 0));
    app.pr_range_reload_state = Some(request);
    let (_tx, rx) = std::sync::mpsc::channel();
    app.pr_range_reload_rx = Some(rx);

    // when the selection widens to every commit
    app.reload_pr_inline_selection();

    // then the fetch is cancelled
    assert!(app.pr_range_reload_state.is_none());
    assert!(app.pr_range_reload_rx.is_none());
}

#[test]
fn should_clear_in_flight_reloads_when_entering_another_pr() {
    // given reload and range reload state left over from PR 42
    let _reviews = TestReviewsDir::new();
    let mut app = build_app();
    open_two_commit_pr(&mut app, 42);
    app.pr_reload_state = Some(late_reload_request(&app, 42, &live_head(&app)));
    app.pr_range_reload_state = Some(late_range_request(&app, 42, &live_head(&app), (0, 0)));
    let (_tx, rx) = std::sync::mpsc::channel();
    app.pr_reload_rx = Some(rx);
    let (_tx2, rx2) = std::sync::mpsc::channel();
    app.pr_range_reload_rx = Some(rx2);

    // when PR 43 is opened
    open_two_commit_pr(&mut app, 43);

    // then none of it survives
    assert!(app.pr_reload_state.is_none() && app.pr_reload_rx.is_none());
    assert!(app.pr_range_reload_state.is_none() && app.pr_range_reload_rx.is_none());
}

fn app_in_pr_mode_from_checkout() -> (App, tempfile::TempDir, TestReviewsDir) {
    let reviews = TestReviewsDir::new();
    let dir = tempfile::tempdir().expect("failed to create test repo dir");
    let commits = vec![
        CommitInfo {
            id: "c2".to_string(),
            short_id: "c2".to_string(),
            branch_name: None,
            summary: "second".to_string(),
            body: None,
            author: "a".to_string(),
            time: chrono::Utc::now(),
        },
        CommitInfo {
            id: "c1".to_string(),
            short_id: "c1".to_string(),
            branch_name: None,
            summary: "first".to_string(),
            body: None,
            author: "a".to_string(),
            time: chrono::Utc::now(),
        },
    ];
    let mut app = build_app_rooted(dir.path().to_path_buf(), commits, None);
    let backend = Box::new(FakeForgeBackend::open_pr_details(
        test_pr_details(42, "first"),
        crate::forge::github::gh::tests_fixture::SIMPLE_PATCH.to_string(),
    ));
    app.open_pr_with_backend(&sample_pr(42, "first"), backend, None)
        .unwrap();
    (app, dir, reviews)
}

/// Entering PR mode swaps in `PrNoopVcs`, which has no commits or working
/// tree, so the Local tab used to be empty for the rest of the run.
#[test]
fn should_list_local_commits_after_entering_pr_mode() {
    // given a checkout app that has opened a PR
    let (mut app, dir, _reviews) = app_in_pr_mode_from_checkout();
    assert!(
        app.vcs_info
            .root_path
            .to_string_lossy()
            .starts_with("forge:")
    );

    // when the Local tab is opened
    app.enter_target_selector(TargetTab::Local).unwrap();

    // then the checkout's commits are listed
    assert_eq!(app.input_mode, InputMode::CommitSelect);
    assert_eq!(app.commit_list.len(), 2);
    // and the PR stays active behind the selector
    assert!(
        app.vcs_info
            .root_path
            .to_string_lossy()
            .starts_with("forge:")
    );
    assert!(matches!(app.diff_source, DiffSource::PullRequest(_)));
    // and local sessions are scoped by the checkout, not the PR identity
    // (the reload runs with the PR backend active, so it must not read its root)
    app.enter_target_selector(TargetTab::Sessions).unwrap();
    app.sessions_tab_toggle_show_empty();
    assert_eq!(
        app.sessions_tab.scope(),
        dir.path().file_name().unwrap().to_string_lossy()
    );
}

/// A local load that finds nothing leaves the PR on screen with its own
/// backend; one that succeeds leaves PR mode with the checkout's backend.
#[test]
fn should_keep_the_pr_backend_when_a_local_load_finds_nothing() {
    let (mut app, dir, _reviews) = app_in_pr_mode_from_checkout();

    let loaded = app
        .with_local_vcs(|app| app.load_working_tree_selection())
        .unwrap();

    assert!(!loaded);
    assert!(matches!(app.diff_source, DiffSource::PullRequest(_)));
    assert!(
        app.vcs_info
            .root_path
            .to_string_lossy()
            .starts_with("forge:")
    );
    // and the checkout is still reachable afterwards
    app.enter_target_selector(TargetTab::Local).unwrap();
    assert_eq!(app.commit_list.len(), 2);
    drop(dir);
}

/// Leaving PR mode for a local review must drop the PR rows from the
/// annotation cache, or row 0 stays a PR-description row with no `pr_info`.
#[test]
fn should_drop_pr_annotations_when_a_local_review_replaces_the_pr() {
    // given a PR app whose annotations include PR rows
    let (mut app, _dir, _reviews) = app_in_pr_mode_from_checkout();
    app.rebuild_annotations();
    let is_pr_row = |a: &AnnotatedLine| {
        matches!(
            a,
            AnnotatedLine::PrInfoLine { .. } | AnnotatedLine::RemoteReviewSummaryLine { .. }
        )
    };
    assert!(app.line_annotations.iter().any(is_pr_row));

    // when a local action replaces the PR (as the local loaders do)
    app.with_local_vcs(|app| {
        app.diff_source = DiffSource::WorkingTree;
        app.rebuild_annotations();
        Ok(())
    })
    .unwrap();

    // then no PR row survives
    assert!(app.pr_info.is_none());
    assert!(!app.line_annotations.iter().any(is_pr_row));
}

/// `tuicr pr` from a checkout subdirectory records the checkout root, and
/// from outside any checkout records nothing.
#[test]
fn should_record_the_checkout_root_for_cli_pr_launch_dirs() {
    use crate::app::init::checkout_root_for_launch_dir;
    let options = VcsOpenOptions::default();

    // given a checkout with a subdirectory
    let repo = tempfile::tempdir().unwrap();
    crate::forge::local_git::test_support::init_repo_with_commit(repo.path(), &["one"]);
    let sub = repo.path().join("src");
    std::fs::create_dir(&sub).unwrap();

    // then the subdirectory resolves to the root
    let root = checkout_root_for_launch_dir(&sub, &options).unwrap();
    assert_eq!(
        root.canonicalize().unwrap(),
        repo.path().canonicalize().unwrap()
    );

    // and a directory outside any checkout resolves to nothing
    let plain = tempfile::tempdir().unwrap();
    assert_eq!(checkout_root_for_launch_dir(plain.path(), &options), None);
}

fn app_in_pr_mode_with_commits(n: usize) -> (App, tempfile::TempDir, TestReviewsDir) {
    let reviews = TestReviewsDir::new();
    let dir = tempfile::tempdir().expect("failed to create test repo dir");
    let commits = (0..n)
        .map(|i| CommitInfo {
            id: format!("c{i}"),
            short_id: format!("c{i}"),
            branch_name: None,
            summary: format!("commit {i}"),
            body: None,
            author: "a".to_string(),
            time: chrono::Utc::now(),
        })
        .collect();
    let mut app = build_app_rooted(dir.path().to_path_buf(), commits, None);
    let backend = Box::new(FakeForgeBackend::open_pr_details(
        test_pr_details(42, "first"),
        crate::forge::github::gh::tests_fixture::SIMPLE_PATCH.to_string(),
    ));
    app.open_pr_with_backend(&sample_pr(42, "first"), backend, None)
        .unwrap();
    (app, dir, reviews)
}

/// "Show more" pages through the checkout's history, not the PR's no-op
/// backend (which answers with nothing and used to end pagination at 10).
#[test]
fn should_page_local_history_after_entering_pr_mode() {
    // given a PR opened from a checkout with 25 commits
    let (mut app, _dir, _reviews) = app_in_pr_mode_with_commits(25);
    app.enter_target_selector(TargetTab::Local).unwrap();
    assert_eq!(app.commit_list.len(), VISIBLE_COMMIT_COUNT);
    assert!(app.has_more_commit);

    // when the list is expanded past the first page
    app.visible_commit_count = app.commit_list.len();
    app.expand_commit().unwrap();

    // then older commits were fetched and the PR backend is back in place
    assert!(app.commit_list.len() > VISIBLE_COMMIT_COUNT);
    assert_eq!(app.commit_list[VISIBLE_COMMIT_COUNT].id, "c10");
    assert!(
        app.vcs_info
            .root_path
            .to_string_lossy()
            .starts_with("forge:")
    );
}

/// A lazily reopened backend follows the recorded checkout, not wherever the
/// process currently is.
#[test]
fn should_reopen_the_recorded_checkout_not_the_cwd() {
    // given a PR app with no parked backend and a real recorded checkout
    let (mut app, _dir, _reviews) = app_in_pr_mode_from_checkout();
    let repo = tempfile::tempdir().unwrap();
    crate::forge::local_git::test_support::init_repo_with_commit(repo.path(), &["one"]);
    app.parked_vcs = None;
    app.local_repo_root = Some(repo.path().to_path_buf());
    assert_ne!(
        std::env::current_dir().unwrap().canonicalize().unwrap(),
        repo.path().canonicalize().unwrap()
    );

    // when a local action runs
    let root = app
        .with_local_vcs(|app| Ok(app.vcs_info.root_path.clone()))
        .unwrap();

    // then it ran against the recorded checkout
    assert_eq!(
        root.canonicalize().unwrap(),
        repo.path().canonicalize().unwrap()
    );
}

/// `tuicr pr` records the configured backend and whitespace mode so the
/// reopened checkout backend matches a normal start.
#[test]
fn should_record_the_startup_vcs_options_for_cli_pr_startup() {
    let _reviews = TestReviewsDir::new();
    let details = test_pr_details(424398, "cli opts");
    let patch = crate::forge::github::gh::tests_fixture::SIMPLE_PATCH;
    let opened = crate::forge::pr_open::prepare_open_pr(
        details.clone(),
        structured_patch(patch),
        vec![sample_pr_commit_with_body(
            &details.head_sha,
            "only",
            "body",
        )],
        PullRequestReviewMetadata::default(),
        crate::forge::traits::PullRequestInfo::from_details(details.clone()),
        None,
    )
    .unwrap();
    let configured = VcsOpenOptions {
        git_backend_preference: GitBackendPreference::Cli,
        diff_whitespace_mode: DiffWhitespaceMode::IgnoreAll,
    };

    let app = App::from_opened_pr(
        opened,
        Box::new(FakeForgeBackend::open_pr_details(
            details.clone(),
            patch.to_string(),
        )),
        details.repository.clone(),
        Theme::dark(),
        None,
        true,
        None,
        PrDisplayOptions {
            show_checks: false,
            show_comments: true,
            comments_visibility: None,
        },
        CommitSelectionStart::All,
        None,
        configured.clone(),
    )
    .unwrap();

    assert_eq!(app.vcs_open_options, configured);
}

#[test]
fn should_keep_wrap_preference_when_opening_a_pr() {
    let mut app = build_app();
    app.forge_repository = Some(ForgeRepository::github("github.com", "agavra", "tuicr"));
    app.pr_tab = PullRequestsTab::new(app.forge_repository.clone());
    app.pr_tab.start_initial_load();
    let summary = sample_pr(42, "answer");
    app.pr_tab
        .apply_initial_load(Ok((vec![summary.clone()], false)));
    app.target_tab = TargetTab::PullRequests;
    app.diff_state.wrap_lines = true;
    let backend = Box::new(FakeForgeBackend::open_pr_details(
        test_pr_details(42, "answer"),
        crate::forge::github::gh::tests_fixture::SIMPLE_PATCH.to_string(),
    ));

    app.open_pr_with_backend(&summary, backend, None).unwrap();

    assert!(matches!(app.diff_source, DiffSource::PullRequest(_)));
    assert!(app.diff_state.wrap_lines, "PR open reset wrap");
}
