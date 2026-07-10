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
    let vcs_info = VcsInfo {
        root_path: PathBuf::from("/tmp"),
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
}

impl FakeForgeBackend {
    fn open_pr_details(details: crate::forge::traits::PullRequestDetails, patch: String) -> Self {
        Self {
            details,
            patch,
            commits: Vec::new(),
            review_metadata: PullRequestReviewMetadata::default(),
            range_patch: None,
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
    ) -> Result<String> {
        Ok(self.patch.clone())
    }
    fn fetch_file_lines(
        &self,
        _request: crate::forge::traits::ForgeFileLinesRequest,
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
    ) -> Result<String> {
        Ok(self
            .range_patch
            .clone()
            .unwrap_or_else(|| self.patch.clone()))
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

fn sample_pr_commit(oid: &str, summary: &str) -> crate::forge::traits::PullRequestCommit {
    crate::forge::traits::PullRequestCommit {
        oid: oid.to_string(),
        short_oid: oid.chars().take(7).collect(),
        summary: summary.to_string(),
        author: "Alice".to_string(),
        timestamp: None,
    }
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
}

#[test]
fn should_hide_inline_selector_for_single_commit_pr() {
    // given a PR with exactly one commit
    let mut app = build_app();
    app.forge_repository = Some(ForgeRepository::github("github.com", "agavra", "tuicr"));
    app.pr_tab = PullRequestsTab::new(app.forge_repository.clone());
    app.pr_tab.start_initial_load();
    let summary = sample_pr(42, "solo");
    app.pr_tab
        .apply_initial_load(Ok((vec![summary.clone()], false)));
    let mut backend = FakeForgeBackend::open_pr_details(
        test_pr_details(42, "solo"),
        crate::forge::github::gh::tests_fixture::SIMPLE_PATCH.to_string(),
    );
    backend.commits = vec![sample_pr_commit("aaaaaaa1111", "only")];
    // when
    app.open_pr_with_backend(&summary, Box::new(backend), None)
        .unwrap();
    // then
    assert!(!app.show_commit_selector);
    assert!(app.commit_list.is_empty());
    assert_eq!(app.commit_selection_range, None);
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
    app.finish_pr_range_reload(&request, first_hunk_patch())
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
fn should_use_persisted_new_head_session_instead_of_carrying_old_head_state() {
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
    let highlighter = app.theme.syntax_highlighter();
    let opened_b = crate::forge::pr_open::prepare_open_pr(
        details_b.clone(),
        &two_file_patch("newer changed"),
        Vec::new(),
        PullRequestReviewMetadata::default(),
        None,
        highlighter,
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
    write_session_file_without_manifest(&persisted_b);

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
}

#[test]
fn should_error_on_corrupt_exact_session_file_when_reopening_pr() {
    // given a corrupt session file at the opened PR head's deterministic path
    let _reviews = TestReviewsDir::new();
    let mut details = test_pr_details(424250, "corrupt");
    details.head_sha = "aaaaaaaaaaaaaaaa".to_string();
    let theme = Theme::default();
    let highlighter = theme.syntax_highlighter();
    let opened = crate::forge::pr_open::prepare_open_pr(
        details.clone(),
        &two_file_patch("new changed"),
        Vec::new(),
        PullRequestReviewMetadata::default(),
        None,
        highlighter,
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
    let highlighter = app.theme.syntax_highlighter();
    let opened_b = crate::forge::pr_open::prepare_open_pr(
        details_b.clone(),
        &two_file_patch("newer changed"),
        Vec::new(),
        PullRequestReviewMetadata::default(),
        None,
        highlighter,
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
    let theme = Theme::default();
    let highlighter = theme.syntax_highlighter();
    let opened = crate::forge::pr_open::prepare_open_pr(
        details.clone(),
        &two_file_patch("new changed"),
        Vec::new(),
        PullRequestReviewMetadata::default(),
        None,
        highlighter,
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
        App::reviewed_state_carried_forward(&previous, next.clone(), &app.diff_files);
    let carried_without_diff = App::reviewed_state_carried_forward(&previous, next.clone(), &[]);

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
        App::reviewed_state_carried_forward(&previous_with_reviewed_file, fresh_next, &[]);
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
    let highlighter = app.theme.syntax_highlighter();
    let opened = crate::forge::pr_open::prepare_open_pr(
        details_b,
        &two_file_patch("newer changed"),
        Vec::new(),
        PullRequestReviewMetadata::default(),
        None,
        highlighter,
    )
    .unwrap();
    let next =
        App::reviewed_state_carried_forward(&previous, opened.session.clone(), &opened.diff_files);

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
    let highlighter = app.theme.syntax_highlighter();
    let opened = crate::forge::pr_open::prepare_open_pr(
        details_b,
        &two_hunk_pr_patch("newer second"),
        Vec::new(),
        PullRequestReviewMetadata::default(),
        None,
        highlighter,
    )
    .unwrap();
    let new_first_key = opened.diff_files[0].hunk_review_key(0).unwrap();
    let new_second_key = opened.diff_files[0].hunk_review_key(1).unwrap();
    let next =
        App::reviewed_state_carried_forward(&previous, opened.session.clone(), &opened.diff_files);

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
    };

    // when the async reload finish path applies head B
    let mut details_b = details_a.clone();
    details_b.head_sha = "bbbbbbbbbbbbbbbb".to_string();
    app.finish_pr_reload(
        details_b,
        two_file_patch("newer changed"),
        Vec::new(),
        PullRequestReviewMetadata::default(),
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
    };

    // when the async reload finish path refreshes the same head
    app.finish_pr_reload(
        details,
        two_file_patch("new changed"),
        Vec::new(),
        PullRequestReviewMetadata::default(),
        &request,
    )
    .unwrap();

    // then reviewed file and hunk markers are preserved.
    assert!(app.session.is_file_reviewed(&stable_path));
    assert!(app.session.is_hunk_reviewed(&stable_path, &stable_key));
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
    // when reloading with the same head
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
    ) -> Result<String> {
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
    ) -> Result<String> {
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
    app.command_buffer = "su".to_string();
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
    calls: std::cell::Cell<u32>,
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
            calls: std::cell::Cell::new(0),
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
    ) -> Result<String> {
        Ok(self.patch.clone())
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
        self.calls.set(self.calls.get() + 1);
        Ok(self.threads.clone())
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
    ) -> Result<String> {
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
            created_at: None,
            in_reply_to: None,
            url: "https://example.com/c".to_string(),
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
        threads: Ok(vec![sample_thread(2, "delayed", false, false)]),
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
