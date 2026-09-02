//! Resuming a persisted review through the Sessions tab.
//!
//! These drive `sessions_tab_select` — the production entry point — against
//! sessions persisted in an isolated reviews dir. An earlier version called the
//! loaders directly through test-only wrappers; that passed while the dispatch
//! routed `WorkingTree` to the unstaged-only loader, so it guarded nothing.

use crate::app::*;
use crate::model::FileStatus;
use crate::review_store::{SessionKind, SessionSummary};
use crate::vcs::traits::{ResolvedRevisionRange, VcsType};
use std::sync::{Arc, Mutex};

/// Redirects persisted-session storage to a temp dir for the current thread.
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

/// What the app asked the VCS for, so tests can assert the diff was requested
/// the right way round rather than trusting the app's own bookkeeping.
#[derive(Default)]
struct VcsCalls {
    range_ids: Option<Vec<String>>,
    working_tree: usize,
    unstaged: usize,
    staged: usize,
}

struct RecordingVcs {
    info: VcsInfo,
    commits: Vec<CommitInfo>,
    calls: Arc<Mutex<VcsCalls>>,
}

impl VcsBackend for RecordingVcs {
    fn info(&self) -> &VcsInfo {
        &self.info
    }

    /// Non-empty: a `-w` review of a fully staged tree still has a diff.
    fn get_working_tree_diff(&self, _highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
        self.calls.lock().unwrap().working_tree += 1;
        Ok(vec![diff_file("worktree.py")])
    }

    /// Empty: the unstaged diff and the whole-working-tree diff differ, so
    /// routing to the wrong one is visible.
    fn get_unstaged_diff(&self, _highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
        self.calls.lock().unwrap().unstaged += 1;
        Err(TuicrError::NoChanges)
    }

    fn get_staged_diff(&self, _highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
        self.calls.lock().unwrap().staged += 1;
        Ok(vec![diff_file("staged.py")])
    }

    fn get_commits_info(&self, ids: &[String]) -> Result<Vec<CommitInfo>> {
        let mut out = Vec::new();
        for id in ids {
            match self.commits.iter().find(|c| &c.id == id) {
                Some(commit) => out.push(commit.clone()),
                None => {
                    return Err(TuicrError::VcsCommand(format!("Commit not found {id}")));
                }
            }
        }
        Ok(out)
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

    fn get_commit_range_diff(
        &self,
        revision_range: &ResolvedRevisionRange<'_>,
        _highlighter: &SyntaxHighlighter,
    ) -> Result<Vec<DiffFile>> {
        self.calls.lock().unwrap().range_ids = Some(revision_range.commit_ids.to_vec());
        Ok(vec![diff_file("ranged.py")])
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
        old_path: Some(PathBuf::from(path)),
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
        short_id: id.chars().take(7).collect(),
        branch_name: None,
        summary: format!("commit {id}"),
        body: None,
        author: "Test".to_string(),
        time: Utc::now(),
    }
}

/// App over `commits` (newest-first, as the selector displays them).
fn build_app(commits: Vec<CommitInfo>) -> (App, Arc<Mutex<VcsCalls>>) {
    let vcs_info = VcsInfo {
        root_path: PathBuf::from("/tmp"),
        head_commit: commits.first().map(|c| c.id.clone()).unwrap_or_default(),
        branch_name: Some("main".to_string()),
        vcs_type: VcsType::Git,
    };
    let session = ReviewSession::new(
        vcs_info.root_path.clone(),
        vcs_info.head_commit.clone(),
        vcs_info.branch_name.clone(),
        SessionDiffSource::WorkingTree,
    );
    let calls = Arc::new(Mutex::new(VcsCalls::default()));
    let app = App::build(
        Box::new(RecordingVcs {
            info: vcs_info.clone(),
            commits: commits.clone(),
            calls: Arc::clone(&calls),
        }),
        vcs_info,
        Theme::dark(),
        None,
        false,
        Vec::new(),
        session,
        DiffSource::WorkingTree,
        InputMode::CommitSelect,
        commits,
        None,
        None,
    )
    .expect("failed to build test app");
    (app, calls)
}

/// Persist a session and put its listing row in the Sessions tab, so
/// `sessions_tab_select` resolves it exactly as it would in the TUI.
fn stage_session(app: &mut App, session: ReviewSession) -> SessionSummary {
    let store = crate::review_store::ReviewStore::new();
    let session_ref = store.save_review(&session).expect("persist session");
    let summary = SessionSummary {
        session_ref,
        slug: "test@main/session".to_string(),
        kind: SessionKind::Local,
        updated_at: Utc::now(),
        comment_count: 1,
        reviewed_count: 0,
        file_count: 1,
        anchor: "main".to_string(),
        active: false,
    };
    app.sessions_tab.apply_load(Ok(vec![summary.clone()]));
    summary
}

fn range_session(commit_ids: &[&str], branch: Option<&str>) -> ReviewSession {
    let mut session = ReviewSession::new(
        PathBuf::from("/tmp"),
        commit_ids.last().expect("non-empty range").to_string(),
        branch.map(str::to_string),
        SessionDiffSource::CommitRange,
    );
    session.commit_range = Some(commit_ids.iter().map(|id| id.to_string()).collect());
    session
}

#[test]
fn should_pass_stored_commit_order_through_unchanged_when_resuming() {
    // a range persisted oldest-first, as confirm_commit_selection_inner
    // writes it
    let _reviews = TestReviewsDir::new();
    let (mut app, calls) = build_app(vec![commit("ccc"), commit("bbb"), commit("aaa")]);
    stage_session(
        &mut app,
        range_session(&["aaa", "bbb", "ccc"], Some("main")),
    );

    // resumed through the production entry point
    app.sessions_tab_select().expect("resume should succeed");

    // the VCS sees oldest-first, so [0] is the base and last() the head.
    // Reversed, the diff renders additions as deletions.
    assert_eq!(
        calls.lock().unwrap().range_ids.clone(),
        Some(vec![
            "aaa".to_string(),
            "bbb".to_string(),
            "ccc".to_string()
        ]),
        "resume must not reverse the stored range"
    );
}

#[test]
fn should_resume_working_tree_session_through_the_working_tree_loader() {
    // a `-w` review whose changes are all staged, so the unstaged-only
    // diff is empty while the working-tree diff is not
    let _reviews = TestReviewsDir::new();
    let (mut app, calls) = build_app(vec![commit("ccc")]);
    stage_session(
        &mut app,
        ReviewSession::new(
            PathBuf::from("/tmp"),
            "ccc".to_string(),
            Some("main".to_string()),
            SessionDiffSource::WorkingTree,
        ),
    );

    app.sessions_tab_select().expect("resume should succeed");

    // the working-tree diff loaded, not the unstaged one
    let calls = calls.lock().unwrap();
    assert_eq!(calls.working_tree, 1, "expected the working-tree diff");
    assert_eq!(calls.unstaged, 0, "unstaged-only diff is the wrong source");
    assert_eq!(app.diff_files.len(), 1);
    assert!(
        matches!(app.session.diff_source, SessionDiffSource::WorkingTree),
        "session source must survive resume or a different session is written"
    );
    assert_eq!(
        app.message, None,
        "a resumable review must not report an empty diff"
    );
}

#[test]
fn should_install_the_selected_session_not_one_matching_current_head() {
    // a review saved on another branch. The loaders resolve a session
    // from the *current* branch and HEAD, so resume must override their choice.
    let _reviews = TestReviewsDir::new();
    let (mut app, _) = build_app(vec![commit("ccc"), commit("bbb"), commit("aaa")]);
    let mut saved = range_session(&["aaa", "bbb", "ccc"], Some("feature/old"));
    saved.session_notes = Some("notes from the saved review".to_string());
    let summary = stage_session(&mut app, saved);

    app.sessions_tab_select().expect("resume should succeed");

    // the app holds the row that was picked, not a fresh session under
    // the current anchor
    assert_eq!(
        app.session.branch_name.as_deref(),
        Some("feature/old"),
        "resume installed a different session than the selected row"
    );
    assert_eq!(
        app.session.session_notes.as_deref(),
        Some("notes from the saved review"),
        "the selected session's saved state must survive resume"
    );
    assert_eq!(
        app.session_path.as_deref(),
        Some(summary.session_ref.path()),
        "further edits must write back to the selected session file"
    );
}

#[test]
fn should_report_a_range_whose_commits_are_gone() {
    // a session referencing a commit that no longer resolves, which is
    // what an amend or rebase leaves behind
    let _reviews = TestReviewsDir::new();
    let (mut app, calls) = build_app(vec![commit("ccc"), commit("bbb")]);
    stage_session(
        &mut app,
        range_session(&["aaa", "bbb", "ccc"], Some("main")),
    );

    app.sessions_tab_select().expect("resume should not error");

    // refused with a message, and no partial range loaded
    let message = app
        .message
        .as_ref()
        .expect("expected a user-facing refusal");
    assert_eq!(message.message_type, MessageType::Error);
    assert!(
        message.content.contains("aren't in the current history"),
        "unexpected refusal text: {:?}",
        message.content
    );
    assert!(
        calls.lock().unwrap().range_ids.is_none(),
        "a partially resolvable range must not reach the VCS"
    );
}

#[test]
fn should_resolve_commits_outside_the_recent_history_page() {
    // a range older than any page of recent commits. Direct id lookup
    // must find it; a paged history walk would report it as pruned.
    let _reviews = TestReviewsDir::new();
    let mut history: Vec<CommitInfo> = (0..40).map(|i| commit(&format!("c{i:02}"))).collect();
    history.push(commit("ancient"));
    let (mut app, calls) = build_app(history);
    stage_session(&mut app, range_session(&["ancient"], Some("main")));

    app.sessions_tab_select().expect("resume should succeed");

    assert_eq!(
        calls.lock().unwrap().range_ids.clone(),
        Some(vec!["ancient".to_string()]),
        "a commit outside the recent page must still resume"
    );
    assert!(app.message.is_none(), "{:?}", app.message);
}

#[test]
fn should_resume_a_pr_session_through_the_forge_open_path() {
    // A PR diff is fetched, not read from the checkout, so resume must reuse
    // the Pull Requests tab's open path rather than refusing.
    let _reviews = TestReviewsDir::new();
    let (mut app, calls) = build_app(vec![commit("ccc")]);
    let repository = crate::forge::traits::ForgeRepository::github("github.com", "owner", "repo");
    let mut session = ReviewSession::new(
        PathBuf::from("/tmp"),
        "deadbee".to_string(),
        None,
        SessionDiffSource::PullRequest,
    );
    session.pr_session_key = Some(crate::forge::traits::PrSessionKey::new(
        repository.clone(),
        7,
        "deadbee",
    ));
    let store = crate::review_store::ReviewStore::new();
    let session_ref = store.save_review(&session).expect("persist PR session");
    app.sessions_tab.apply_load(Ok(vec![SessionSummary {
        session_ref,
        slug: "gh:owner/repo/pr/7".to_string(),
        kind: SessionKind::Pr,
        updated_at: Utc::now(),
        comment_count: 1,
        reviewed_count: 0,
        file_count: 1,
        anchor: "pr/7".to_string(),
        active: false,
    }]));

    app.sessions_tab_select().expect("resume should not error");

    // The forge fetch is in flight for the right PR, and no local diff was read.
    let request = app
        .pr_open_state
        .as_ref()
        .expect("expected a PR open to start");
    assert_eq!(request.pr_number, 7);
    assert_eq!(request.repository, repository);
    let calls = calls.lock().unwrap();
    assert_eq!(
        (calls.working_tree, calls.staged, calls.unstaged),
        (0, 0, 0)
    );
    assert!(calls.range_ids.is_none());
}

#[test]
fn should_report_a_pr_session_with_no_saved_pull_request() {
    let _reviews = TestReviewsDir::new();
    let (mut app, _) = build_app(vec![commit("ccc")]);
    app.sessions_tab.apply_load(Ok(vec![SessionSummary {
        session_ref: crate::review_store::SessionRef::from_path("/tmp/missing-pr.json"),
        slug: "gh:owner/repo/pr/9".to_string(),
        kind: SessionKind::Pr,
        updated_at: Utc::now(),
        comment_count: 1,
        reviewed_count: 0,
        file_count: 1,
        anchor: "pr/9".to_string(),
        active: false,
    }]));

    app.sessions_tab_select().expect("select should not error");

    // A session file that cannot be read is reported, not silently ignored.
    assert!(app.message.is_some(), "expected a user-facing message");
    assert!(app.pr_open_state.is_none());
}

#[test]
fn should_point_pristine_sessions_at_the_flag_that_opens_them() {
    let _reviews = TestReviewsDir::new();
    let (mut app, calls) = build_app(vec![commit("ccc")]);
    stage_session(
        &mut app,
        ReviewSession::new(
            PathBuf::from("/tmp"),
            "pristine:ccc".to_string(),
            Some("main".to_string()),
            SessionDiffSource::Pristine,
        ),
    );

    app.sessions_tab_select().expect("select should not error");

    assert!(
        app.message
            .as_ref()
            .is_some_and(|m| m.content.contains("--all-files")),
        "expected guidance naming the flag, got {:?}",
        app.message
    );
    let calls = calls.lock().unwrap();
    assert_eq!(
        (calls.working_tree, calls.staged, calls.unstaged),
        (0, 0, 0)
    );
}

#[test]
fn should_hide_sessions_with_no_review_progress() {
    // an empty session, as a crashed TUI leaves behind, beside one
    // holding comments and one holding reviewed state
    let empty = SessionSummary {
        session_ref: crate::review_store::SessionRef::from_path("/tmp/empty.json"),
        slug: "repo@main/worktree/ccc".to_string(),
        kind: SessionKind::Local,
        updated_at: Utc::now(),
        comment_count: 0,
        reviewed_count: 0,
        file_count: 1,
        anchor: "main".to_string(),
        active: false,
    };
    let commented = SessionSummary {
        comment_count: 7,
        slug: "repo@main/commits/aaa..ccc".to_string(),
        ..empty.clone()
    };
    let reviewed = SessionSummary {
        reviewed_count: 2,
        slug: "repo@main/worktree/bbb".to_string(),
        ..empty.clone()
    };

    // the production predicate decides
    let kept: Vec<&str> = [&empty, &commented, &reviewed]
        .into_iter()
        .filter(|s| App::is_resumable(s))
        .map(|s| s.slug.as_str())
        .collect();

    assert_eq!(
        kept,
        vec!["repo@main/commits/aaa..ccc", "repo@main/worktree/bbb"],
        "a session with no comments and no reviewed files holds no progress"
    );
}
