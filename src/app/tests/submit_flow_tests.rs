//! Tests for the `:submit*` preflight / resolver / confirmation
//! orchestration. Driven through the App methods rather than the key
//! handlers so we exercise the state machine directly.
use crate::app::*;
use crate::forge::submit::{ResolverAction, SubmitEvent, UnmappableReason};
use crate::forge::traits::{ForgeRepository, PrSessionKey};
use crate::model::comment::{Comment, CommentLifecycleState, CommentType, LineContext};
use crate::model::diff_types::{DiffHunk, DiffLine, FileStatus, LineOrigin};
use crate::vcs::traits::{VcsChangeStatus, VcsType};

struct DummyVcs {
    info: VcsInfo,
}

impl VcsBackend for DummyVcs {
    fn info(&self) -> &VcsInfo {
        &self.info
    }
    fn get_working_tree_diff(&self, _h: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
        Err(TuicrError::NoChanges)
    }
    fn fetch_context_lines(
        &self,
        _p: &Path,
        _s: FileStatus,
        _ref_commit: Option<&str>,
        _start: u32,
        _end: u32,
    ) -> Result<Vec<DiffLine>> {
        Ok(Vec::new())
    }
    fn get_change_status(&self) -> Result<VcsChangeStatus> {
        Ok(VcsChangeStatus {
            staged: false,
            unstaged: false,
        })
    }
    fn file_line_count(&self, _p: &Path, _s: FileStatus, _ref_commit: Option<&str>) -> Result<u32> {
        Ok(0)
    }
}

fn make_pr_app_with_single_modified_file(file_path: &str) -> App {
    let vcs_info = VcsInfo {
        root_path: PathBuf::from("/tmp/repo"),
        head_commit: "abcdef0123".to_string(),
        branch_name: Some("feat".to_string()),
        vcs_type: VcsType::File,
    };
    let session = ReviewSession::new(
        vcs_info.root_path.clone(),
        vcs_info.head_commit.clone(),
        vcs_info.branch_name.clone(),
        SessionDiffSource::PullRequest,
    );
    let diff_file = DiffFile {
        old_path: Some(PathBuf::from(file_path)),
        new_path: Some(PathBuf::from(file_path)),
        status: FileStatus::Modified,
        hunks: vec![DiffHunk {
            header: "@@".to_string(),
            old_start: 1,
            old_count: 0,
            new_start: 1,
            new_count: 0,
            lines: vec![
                DiffLine {
                    origin: LineOrigin::Context,
                    content: "a".to_string(),
                    old_lineno: Some(10),
                    new_lineno: Some(10),
                    highlighted_spans: None,
                },
                DiffLine {
                    origin: LineOrigin::Addition,
                    content: "b".to_string(),
                    old_lineno: None,
                    new_lineno: Some(11),
                    highlighted_spans: None,
                },
            ],
        }],
        is_binary: false,
        is_too_large: false,
        is_commit_message: false,
        content_hash: 0,
    };
    let pr_source = PullRequestDiffSource {
        key: PrSessionKey::new(
            ForgeRepository::github("github.com", "agavra", "tuicr"),
            125,
            "abcdef0123".to_string(),
        ),
        base_sha: "0000".to_string(),
        title: "test pr".to_string(),
        url: "https://github.com/agavra/tuicr/pull/125".to_string(),
        head_ref_name: "feat".to_string(),
        base_ref_name: "main".to_string(),
        state: "OPEN".to_string(),
        closed: false,
        merged: false,
    };
    let mut app = App::build(
        Box::new(DummyVcs {
            info: vcs_info.clone(),
        }),
        vcs_info,
        Theme::dark(),
        None,
        false,
        vec![diff_file],
        session,
        DiffSource::PullRequest(Box::new(pr_source)),
        InputMode::Normal,
        Vec::new(),
        None,
        None,
    )
    .expect("build app");
    app.current_pr_head = Some("abcdef0123".to_string());
    app
}

fn line_comment(side: LineSide, new: Option<u32>, old: Option<u32>) -> Comment {
    let mut c = Comment::new(
        "body".to_string(),
        CommentType::from_id("issue"),
        Some(side),
    );
    c.line_context = Some(LineContext {
        new_line: new,
        old_line: old,
        content: String::new(),
    });
    c
}

fn add_line_comment(app: &mut App, path: &str, line: u32, comment: Comment) {
    let pb = PathBuf::from(path);
    let review = app.session.get_file_mut(&pb).expect("file in session");
    review.line_comments.entry(line).or_default().push(comment);
}

fn deliver_matching_pr_threads_event(
    app: &mut App,
    threads: std::result::Result<Vec<crate::forge::remote_comments::RemoteReviewThread>, String>,
    summaries: std::result::Result<Vec<crate::forge::remote_comments::RemoteReviewSummary>, String>,
) {
    let (tx, rx) = std::sync::mpsc::channel();
    app.pr_threads_rx = Some(rx);
    app.forge_review_threads_loading = true;
    let pr_key = match &app.diff_source {
        DiffSource::PullRequest(pr) => pr.key.clone(),
        _ => panic!("expected PR mode"),
    };
    tx.send(PrThreadsEvent::Done {
        repository: pr_key.repository,
        pr_number: pr_key.number,
        head_sha: pr_key.head_sha,
        threads,
        summaries,
    })
    .unwrap();
}

#[test]
fn should_use_subset_head_sha_as_commit_id_when_inline_selector_is_strict_subset() {
    // Regression for HTTP 422 when reviewing a subset of commits: the
    // payload's `commit_id` must match the SHA the displayed diff was
    // computed against — using the cumulative PR head causes GitHub to
    // reject inline comments whose lines aren't in that diff.
    use crate::forge::traits::PullRequestCommit;

    let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
    // Newest-first: [C3, C2, C1]. PR head SHA is C3 ("abcdef0123").
    app.pr_commits = vec![
        PullRequestCommit {
            oid: "abcdef0123".to_string(),
            short_oid: "abcdef0".to_string(),
            summary: "C3".to_string(),
            author: "me".to_string(),
            timestamp: None,
        },
        PullRequestCommit {
            oid: "deadbeef02".to_string(),
            short_oid: "deadbee".to_string(),
            summary: "C2".to_string(),
            author: "me".to_string(),
            timestamp: None,
        },
        PullRequestCommit {
            oid: "facecafe01".to_string(),
            short_oid: "facecaf".to_string(),
            summary: "C1".to_string(),
            author: "me".to_string(),
            timestamp: None,
        },
    ];
    // Strict subset: only middle commit C2 selected (start=1, end=1).
    app.commit_selection_range = Some((1, 1));
    add_line_comment(
        &mut app,
        "src/lib.rs",
        11,
        Comment::new(
            "comment on C2".to_string(),
            CommentType::from_id("issue"),
            Some(LineSide::New),
        ),
    );

    app.start_submit(SubmitEvent::Comment);

    let state = app.submit_state.as_ref().expect("submit state");
    assert_eq!(
        state.commit_id, "deadbeef02",
        "subset → commit_id should be the newest selected commit (start_idx), not the PR head",
    );
}

#[test]
fn should_use_pr_head_sha_as_commit_id_when_full_commit_range_selected() {
    // Counterpart to the subset regression: full-range selection should
    // continue to use the cumulative PR head SHA.
    use crate::forge::traits::PullRequestCommit;

    let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
    app.pr_commits = vec![
        PullRequestCommit {
            oid: "abcdef0123".to_string(),
            short_oid: "abcdef0".to_string(),
            summary: "C2".to_string(),
            author: "me".to_string(),
            timestamp: None,
        },
        PullRequestCommit {
            oid: "facecafe01".to_string(),
            short_oid: "facecaf".to_string(),
            summary: "C1".to_string(),
            author: "me".to_string(),
            timestamp: None,
        },
    ];
    app.commit_selection_range = Some((0, 1));
    add_line_comment(
        &mut app,
        "src/lib.rs",
        11,
        Comment::new(
            "comment".to_string(),
            CommentType::from_id("issue"),
            Some(LineSide::New),
        ),
    );

    app.start_submit(SubmitEvent::Comment);

    let state = app.submit_state.as_ref().expect("submit state");
    assert_eq!(state.commit_id, "abcdef0123");
}

#[test]
fn should_anchor_line_comments_via_hashmap_key_when_line_context_missing() {
    // Regression: in production, comments are created via Comment::new
    // which does NOT populate line_context — the line lives only in the
    // line_comments HashMap key. Before the CommentAnchor refactor, the
    // mapper treated these as file-level and posted everything with
    // position 1 plus the "File-level:" body prefix.
    use crate::forge::submit::GhSide;

    let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
    let bare = Comment::new(
        "real line comment".to_string(),
        CommentType::from_id("issue"),
        Some(LineSide::New),
    );
    assert!(bare.line_context.is_none(), "fixture contract");
    add_line_comment(&mut app, "src/lib.rs", 11, bare);

    app.start_submit(SubmitEvent::Comment);

    assert_eq!(app.input_mode, InputMode::SubmitConfirm);
    let state = app.submit_state.as_ref().expect("submit state");
    assert_eq!(state.mappable.len(), 1);
    let inline = &state.mappable[0];
    assert_eq!(inline.line, 11);
    assert_eq!(inline.side, GhSide::Right);
    assert!(
        !inline.body.contains("File-level:"),
        "regression: body should not be prefixed File-level (got: {})",
        inline.body
    );
}

#[test]
fn should_open_confirm_directly_when_all_comments_map() {
    // given a PR session with one mappable line comment
    let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
    add_line_comment(
        &mut app,
        "src/lib.rs",
        11,
        line_comment(LineSide::New, Some(11), None),
    );
    // when
    app.start_submit(SubmitEvent::Comment);
    // then — went straight to confirmation, no resolver
    assert_eq!(app.input_mode, InputMode::SubmitConfirm);
    let state = app.submit_state.as_ref().expect("submit state");
    assert_eq!(state.mappable.len(), 1);
    assert!(state.unmappable.is_empty());
    assert_eq!(state.commit_id, "abcdef0123");
    assert_eq!(state.event, SubmitEvent::Comment);
}

#[test]
fn should_open_resolver_when_any_comment_is_unmappable() {
    // given a PR session with one mappable + one file-level on a
    // binary file (unmappable).
    let mut app = make_pr_app_with_single_modified_file("img.png");
    // mark file binary in diff_files
    app.diff_files[0].is_binary = true;
    // file-level comment in session
    let pb = PathBuf::from("img.png");
    let review = app.session.get_file_mut(&pb).expect("file in session");
    review.file_comments.push(Comment::new(
        "oof".to_string(),
        CommentType::from_id("note"),
        None,
    ));
    // when
    app.start_submit(SubmitEvent::Comment);
    // then — resolver entered with one unmappable
    assert_eq!(app.input_mode, InputMode::SubmitResolver);
    let state = app.submit_state.as_ref().expect("submit state");
    assert_eq!(state.unmappable.len(), 1);
    assert_eq!(state.unmappable[0].reason, UnmappableReason::BinaryFile);
    assert_eq!(state.resolver_choices.len(), 1);
    // Default action is MoveToSummary per spec
    assert_eq!(state.resolver_choices[0], ResolverAction::MoveToSummary);
}

#[test]
fn should_skip_locked_comments_during_preflight() {
    // given a single locked comment
    let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
    let mut c = line_comment(LineSide::New, Some(11), None);
    c.lifecycle_state = CommentLifecycleState::Submitted;
    add_line_comment(&mut app, "src/lib.rs", 11, c);
    // when
    app.start_submit(SubmitEvent::Comment);
    // then — preflight aborted with the "nothing to submit" warning
    assert_eq!(app.input_mode, InputMode::Normal);
    assert!(app.submit_state.is_none());
}

#[test]
fn should_warn_when_no_local_drafts_exist() {
    // given a PR session with zero comments
    let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
    // when
    app.start_submit(SubmitEvent::Comment);
    // then
    assert_eq!(app.input_mode, InputMode::Normal);
    assert!(app.submit_state.is_none());
}

#[test]
fn should_allow_bare_approve_through_action_picker_with_no_comments() {
    // Regression: bare `:submit` → picker → cursor on Approve → Enter
    // should NOT warn "Nothing to submit". Approve is the one event
    // meaningful with no comments. Picker uses skip_confirm = true, so
    // the flow goes straight to network dispatch.
    let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
    app.start_submit_action_picker();
    // Walk cursor to the Approve row (index 1).
    app.submit_picker_cursor_down();
    assert_eq!(app.submit_picker_cursor, 1);
    app.submit_picker_confirm();
    // No warning was emitted; the network call was dispatched.
    assert_eq!(app.input_mode, InputMode::Normal);
    assert!(app.submit_state.is_none());
    assert!(
        app.pr_submit_state.is_some(),
        "picker-confirm should dispatch when Approve is bare-allowed"
    );
}

#[test]
fn should_allow_bare_approve_without_any_comments() {
    // Approve is the one event meaningful with no comments (a plain
    // LGTM). Preflight should proceed; the user lands in the confirm
    // modal with zero mappable + zero unmappable.
    let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
    app.start_submit(SubmitEvent::Approve);
    assert_eq!(app.input_mode, InputMode::SubmitConfirm);
    let state = app.submit_state.as_ref().expect("submit state");
    assert!(state.mappable.is_empty());
    assert!(state.unmappable.is_empty());
    assert_eq!(state.event, SubmitEvent::Approve);
}

#[test]
fn should_warn_when_submitting_without_pr_mode() {
    // given an app NOT in PR mode
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
    let mut app = App::build(
        Box::new(DummyVcs {
            info: vcs_info.clone(),
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
    .expect("build app");
    // when
    app.start_submit(SubmitEvent::Comment);
    // then
    assert_eq!(app.input_mode, InputMode::Normal);
    assert!(app.submit_state.is_none());
}

#[test]
fn should_warn_when_pr_is_closed_or_merged() {
    // given a closed PR
    let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
    if let DiffSource::PullRequest(pr) = &mut app.diff_source {
        pr.closed = true;
    }
    add_line_comment(
        &mut app,
        "src/lib.rs",
        11,
        line_comment(LineSide::New, Some(11), None),
    );
    // when
    app.start_submit(SubmitEvent::Comment);
    // then
    assert_eq!(app.input_mode, InputMode::Normal);
    assert!(app.submit_state.is_none());
}

#[test]
fn should_cancel_submit_clears_state_and_returns_to_normal() {
    let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
    add_line_comment(
        &mut app,
        "src/lib.rs",
        11,
        line_comment(LineSide::New, Some(11), None),
    );
    app.start_submit(SubmitEvent::Comment);
    // when
    app.cancel_submit();
    // then
    assert_eq!(app.input_mode, InputMode::Normal);
    assert!(app.submit_state.is_none());
}

#[test]
fn should_toggle_resolver_action_between_move_and_omit() {
    let mut app = make_pr_app_with_single_modified_file("img.png");
    app.diff_files[0].is_binary = true;
    let pb = PathBuf::from("img.png");
    let review = app.session.get_file_mut(&pb).expect("file in session");
    review.file_comments.push(Comment::new(
        "a".to_string(),
        CommentType::from_id("note"),
        None,
    ));
    review.file_comments.push(Comment::new(
        "b".to_string(),
        CommentType::from_id("note"),
        None,
    ));
    app.start_submit(SubmitEvent::Comment);
    // when — toggle row 0
    app.submit_resolver_toggle();
    // then
    let state = app.submit_state.as_ref().unwrap();
    assert_eq!(state.resolver_choices[0], ResolverAction::Omit);
    assert_eq!(state.resolver_choices[1], ResolverAction::MoveToSummary);
    // when toggle again
    app.submit_resolver_toggle();
    let state = app.submit_state.as_ref().unwrap();
    assert_eq!(state.resolver_choices[0], ResolverAction::MoveToSummary);
}

#[test]
fn should_advance_from_resolver_to_confirm() {
    let mut app = make_pr_app_with_single_modified_file("img.png");
    app.diff_files[0].is_binary = true;
    let pb = PathBuf::from("img.png");
    let review = app.session.get_file_mut(&pb).expect("file in session");
    review.file_comments.push(Comment::new(
        "a".to_string(),
        CommentType::from_id("note"),
        None,
    ));
    app.start_submit(SubmitEvent::Comment);
    assert_eq!(app.input_mode, InputMode::SubmitResolver);
    // when
    app.submit_resolver_advance();
    // then
    assert_eq!(app.input_mode, InputMode::SubmitConfirm);
}

#[test]
fn should_skip_confirm_modal_when_action_picker_dispatches_with_no_unmappable() {
    // Bare `:submit` → action picker → pick Comment → directly dispatch
    // the network call without SubmitConfirm. submit_state is cleared
    // and pr_submit_state populated, same end state as the explicit
    // `:submit comment` + [y] flow.
    let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
    add_line_comment(
        &mut app,
        "src/lib.rs",
        11,
        line_comment(LineSide::New, Some(11), None),
    );

    app.start_submit_action_picker();
    assert_eq!(app.input_mode, InputMode::SubmitActionPicker);
    app.submit_picker_confirm();

    assert_eq!(app.input_mode, InputMode::Normal);
    assert!(app.submit_state.is_none());
    assert!(app.pr_submit_state.is_some());
}

#[test]
fn should_route_picker_through_resolver_then_skip_confirm() {
    // Bare `:submit` picker with one unmappable comment → resolver
    // appears → `s` advances and dispatches the network call directly,
    // bypassing SubmitConfirm. skip_confirm is the flag that makes
    // submit_resolver_advance bypass the confirm modal.
    let mut app = make_pr_app_with_single_modified_file("img.png");
    app.diff_files[0].is_binary = true;
    let pb = PathBuf::from("img.png");
    let review = app.session.get_file_mut(&pb).expect("file in session");
    review.file_comments.push(Comment::new(
        "binary art".to_string(),
        CommentType::from_id("note"),
        None,
    ));

    app.start_submit_action_picker();
    app.submit_picker_cursor = 0; // Comment
    app.submit_picker_confirm();

    // Picker dispatched → resolver visible because the comment is
    // unmappable, but the underlying skip_confirm flag is set.
    assert_eq!(app.input_mode, InputMode::SubmitResolver);
    let state = app.submit_state.as_ref().expect("submit state");
    assert!(state.skip_confirm);

    // Advance from resolver → goes straight to network call, no
    // SubmitConfirm.
    app.submit_resolver_advance();
    assert_eq!(app.input_mode, InputMode::Normal);
    assert!(app.submit_state.is_none());
    assert!(app.pr_submit_state.is_some());
}

#[test]
fn should_dispatch_async_submit_on_confirm_and_clear_modal_state() {
    // given a PR session with one mappable line comment
    let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
    add_line_comment(
        &mut app,
        "src/lib.rs",
        11,
        line_comment(LineSide::New, Some(11), None),
    );
    app.start_submit(SubmitEvent::Comment);
    // when
    app.confirm_submit();
    // then — PR 6: confirmation modal disappears immediately; the
    // background thread is running with state captured on
    // pr_submit_state so the spinner has something to render.
    assert_eq!(app.input_mode, InputMode::Normal);
    assert!(app.submit_state.is_none());
    assert!(
        app.pr_submit_state.is_some(),
        "spawn_pr_submit should populate pr_submit_state"
    );
    assert!(app.pr_submit_rx.is_some(), "rx must be present in-flight");
}

fn make_in_flight(
    event: SubmitEvent,
    comment_ids: &[&str],
    head_sha: &str,
    moved_to_summary_count: usize,
) -> SubmitInFlightState {
    let mappable = comment_ids
        .iter()
        .enumerate()
        .map(|(i, id)| crate::forge::submit::InlineComment {
            path: PathBuf::from("src/lib.rs"),
            line: 11 + i as u32,
            side: crate::forge::submit::GhSide::Right,
            counterpart_line: None,
            start_line: None,
            start_side: None,
            old_path: None,
            body: "x".to_string(),
            comment_id: (*id).to_string(),
        })
        .collect();
    SubmitInFlightState {
        event,
        mappable,
        summary_comment_ids: Vec::new(),
        review_comment_ids: Vec::new(),
        moved_to_summary_count,
        head_sha_snapshot: head_sha.to_string(),
        repository: ForgeRepository::github("github.com", "agavra", "tuicr"),
        pr_number: 125,
        started_at: Instant::now(),
    }
}

fn make_response(
    id: u64,
    html_url: &str,
    state: &str,
) -> crate::forge::traits::GhCreateReviewResponse {
    crate::forge::traits::GhCreateReviewResponse {
        id,
        html_url: html_url.to_string(),
        state: state.to_string(),
    }
}

fn pr_commit(oid: &str) -> crate::forge::traits::PullRequestCommit {
    crate::forge::traits::PullRequestCommit {
        oid: oid.to_string(),
        short_oid: oid.chars().take(7).collect(),
        summary: oid.to_string(),
        author: "me".to_string(),
        timestamp: None,
    }
}

#[test]
fn should_mark_reviewed_commits_after_successful_submit() {
    let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
    app.pr_commits = vec![
        pr_commit("abcdef0123"),
        pr_commit("deadbeef02"),
        pr_commit("facecafe01"),
    ];
    app.review_commits = app
        .pr_commits
        .iter()
        .map(pr_commit_to_commit_info)
        .collect();
    let in_flight = make_in_flight(SubmitEvent::Comment, &[], "deadbeef02", 0);
    let response = make_response(123, "https://example.com/r", "COMMENTED");

    app.finish_pr_submit(in_flight, Ok(response));

    assert!(!app.is_commit_reviewed_by_viewer(0));
    assert!(app.is_commit_reviewed_by_viewer(1));
    assert!(app.is_commit_reviewed_by_viewer(2));
}

#[test]
fn should_not_mark_reviewed_commits_after_draft_submit() {
    let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
    app.pr_commits = vec![pr_commit("abcdef0123"), pr_commit("deadbeef02")];
    app.review_commits = app
        .pr_commits
        .iter()
        .map(pr_commit_to_commit_info)
        .collect();
    let in_flight = make_in_flight(SubmitEvent::Draft, &[], "abcdef0123", 0);
    let response = make_response(123, "https://example.com/r", "PENDING");

    app.finish_pr_submit(in_flight, Ok(response));

    assert!(!app.is_commit_reviewed_by_viewer(0));
    assert!(!app.is_commit_reviewed_by_viewer(1));
}

#[test]
fn should_flip_comments_to_submitted_and_stamp_review_id_on_success() {
    // given an app with one line comment that we'll claim got submitted
    let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
    let comment = line_comment(LineSide::New, Some(11), None);
    let comment_id = comment.id.clone();
    add_line_comment(&mut app, "src/lib.rs", 11, comment);
    let in_flight = make_in_flight(
        SubmitEvent::Comment,
        &[comment_id.as_str()],
        "abcdef0123",
        0,
    );
    let response = make_response(987654, "https://example.com/r", "COMMENTED");
    // when
    app.apply_submit_success(&in_flight, &response);
    // then — the comment stays visible but is locked; remote_review_id set
    let review = app.session.files.get(&PathBuf::from("src/lib.rs")).unwrap();
    let saved = &review.line_comments.get(&11).unwrap()[0];
    assert_eq!(saved.lifecycle_state, CommentLifecycleState::Submitted);
    assert_eq!(saved.remote_review_id.as_deref(), Some("987654"));
}

#[test]
fn should_flip_comments_to_pushed_draft_for_draft_submission() {
    // given
    let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
    let comment = line_comment(LineSide::New, Some(11), None);
    let comment_id = comment.id.clone();
    add_line_comment(&mut app, "src/lib.rs", 11, comment);
    let in_flight = make_in_flight(SubmitEvent::Draft, &[comment_id.as_str()], "abcdef0123", 0);
    let response = make_response(42, "https://example.com/r", "PENDING");
    // when
    app.apply_submit_success(&in_flight, &response);
    // then
    let review = app.session.files.get(&PathBuf::from("src/lib.rs")).unwrap();
    let saved = &review.line_comments.get(&11).unwrap()[0];
    assert_eq!(saved.lifecycle_state, CommentLifecycleState::PushedDraft);
    assert_eq!(saved.remote_review_id.as_deref(), Some("42"));
}

#[test]
fn should_only_flip_comments_whose_ids_were_submitted() {
    // given — one matching id and one stray local-draft that wasn't in the submit
    let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
    let comment = line_comment(LineSide::New, Some(11), None);
    let target_id = comment.id.clone();
    add_line_comment(&mut app, "src/lib.rs", 11, comment);
    let untouched = line_comment(LineSide::New, Some(11), None);
    let untouched_id = untouched.id.clone();
    add_line_comment(&mut app, "src/lib.rs", 11, untouched);
    let in_flight = make_in_flight(SubmitEvent::Comment, &[target_id.as_str()], "abcdef0123", 0);
    let response = make_response(1, "u", "COMMENTED");
    // when
    app.apply_submit_success(&in_flight, &response);
    // then — only the target id moved to Submitted; the other stays a LocalDraft
    let review = app.session.files.get(&PathBuf::from("src/lib.rs")).unwrap();
    let comments = review.line_comments.get(&11).unwrap();
    let target = comments.iter().find(|c| c.id == target_id).unwrap();
    let other = comments.iter().find(|c| c.id == untouched_id).unwrap();
    assert_eq!(target.lifecycle_state, CommentLifecycleState::Submitted);
    assert_eq!(other.lifecycle_state, CommentLifecycleState::LocalDraft);
    assert!(other.remote_review_id.is_none());
}

#[test]
fn should_flip_review_level_comments_on_submit_success() {
    // given a PR session with a review-level comment included in the
    // review body. Its id is tracked on the in-flight state so we know
    // exactly which review-level entries went out.
    let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
    let review_comment = Comment::new("nice work".to_string(), CommentType::from_id("note"), None);
    let review_comment_id = review_comment.id.clone();
    app.session.review_comments.push(review_comment);
    let mut in_flight = make_in_flight(SubmitEvent::Comment, &[], "abcdef0123", 0);
    in_flight.review_comment_ids = vec![review_comment_id.clone()];
    let response = make_response(55, "u", "COMMENTED");
    // when
    app.apply_submit_success(&in_flight, &response);
    // then — review-level comments locked too, since they went out in the body
    let saved = app
        .session
        .review_comments
        .iter()
        .find(|c| c.id == review_comment_id)
        .unwrap();
    assert_eq!(saved.lifecycle_state, CommentLifecycleState::Submitted);
    assert_eq!(saved.remote_review_id.as_deref(), Some("55"));
}

#[test]
fn should_flip_summary_bound_unmappable_comments_on_submit_success() {
    // given an unmappable line comment the user chose to "move to summary"
    // — it was embedded in the review body, so the local copy is now stale.
    let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
    let comment = line_comment(LineSide::New, Some(11), None);
    let summary_id = comment.id.clone();
    add_line_comment(&mut app, "src/lib.rs", 11, comment);
    let mut in_flight = make_in_flight(SubmitEvent::Comment, &[], "abcdef0123", 1);
    in_flight.summary_comment_ids = vec![summary_id.clone()];
    let response = make_response(77, "u", "COMMENTED");
    // when
    app.apply_submit_success(&in_flight, &response);
    // then
    let review = app.session.files.get(&PathBuf::from("src/lib.rs")).unwrap();
    let saved = &review.line_comments.get(&11).unwrap()[0];
    assert_eq!(saved.lifecycle_state, CommentLifecycleState::Submitted);
    assert_eq!(saved.remote_review_id.as_deref(), Some("77"));
}

#[test]
fn should_prune_locked_comments_across_all_buckets() {
    // given a session populated with one locked comment in each bucket
    // (review-level, file-level, line) plus an unlocked draft that must
    // survive the prune.
    let mut app = make_pr_app_with_single_modified_file("src/lib.rs");

    let mut review_locked =
        Comment::new("body item".to_string(), CommentType::from_id("note"), None);
    review_locked.lifecycle_state = CommentLifecycleState::Submitted;
    app.session.review_comments.push(review_locked);

    let mut file_locked =
        Comment::new("file-level".to_string(), CommentType::from_id("note"), None);
    file_locked.lifecycle_state = CommentLifecycleState::PushedDraft;
    let mut line_locked = line_comment(LineSide::New, Some(11), None);
    line_locked.lifecycle_state = CommentLifecycleState::Submitted;
    let line_unlocked = line_comment(LineSide::New, Some(12), None);
    let unlocked_id = line_unlocked.id.clone();
    {
        let review = app
            .session
            .get_file_mut(&PathBuf::from("src/lib.rs"))
            .unwrap();
        review.file_comments.push(file_locked);
    }
    add_line_comment(&mut app, "src/lib.rs", 11, line_locked);
    add_line_comment(&mut app, "src/lib.rs", 12, line_unlocked);

    // when remote threads come back and pruning runs
    app.prune_locked_comments();

    // then — every locked entry is gone; the unlocked draft survives
    assert!(app.session.review_comments.is_empty());
    let review = app.session.files.get(&PathBuf::from("src/lib.rs")).unwrap();
    assert!(review.file_comments.is_empty());
    assert!(!review.line_comments.contains_key(&11));
    let surviving = review.line_comments.get(&12).unwrap();
    assert_eq!(surviving.len(), 1);
    assert_eq!(surviving[0].id, unlocked_id);
}

#[test]
fn should_keep_locked_line_comments_when_remote_thread_refetch_fails() {
    let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
    let mut line_locked = line_comment(LineSide::New, Some(11), None);
    line_locked.lifecycle_state = CommentLifecycleState::Submitted;
    add_line_comment(&mut app, "src/lib.rs", 11, line_locked);

    deliver_matching_pr_threads_event(
        &mut app,
        Err("network unavailable".to_string()),
        Ok(Vec::new()),
    );

    app.poll_pr_threads_events();

    let review = app.session.files.get(&PathBuf::from("src/lib.rs")).unwrap();
    let saved = review.line_comments.get(&11).unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].lifecycle_state, CommentLifecycleState::Submitted);
}

#[test]
fn should_keep_locked_review_comments_when_remote_summary_refetch_fails() {
    let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
    let mut review_locked = Comment::new("summary".to_string(), CommentType::from_id("note"), None);
    review_locked.lifecycle_state = CommentLifecycleState::Submitted;
    app.session.review_comments.push(review_locked);

    deliver_matching_pr_threads_event(
        &mut app,
        Ok(Vec::new()),
        Err("graphql unavailable".to_string()),
    );

    app.poll_pr_threads_events();

    assert_eq!(app.session.review_comments.len(), 1);
    assert_eq!(
        app.session.review_comments[0].lifecycle_state,
        CommentLifecycleState::Submitted
    );
}

#[test]
fn should_emit_success_message_with_review_id_and_counts_for_published_submit() {
    // given
    let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
    let comment = line_comment(LineSide::New, Some(11), None);
    let comment_id = comment.id.clone();
    add_line_comment(&mut app, "src/lib.rs", 11, comment);
    let in_flight = make_in_flight(
        SubmitEvent::Comment,
        &[comment_id.as_str()],
        "abcdef0123",
        2,
    );
    let response = make_response(123456, "https://example.com/r", "COMMENTED");
    // when
    app.finish_pr_submit(in_flight, Ok(response));
    // then
    let msg = app.message.as_ref().expect("info message");
    assert_eq!(msg.message_type, MessageType::Info);
    assert!(msg.content.contains("Submitted GitHub review #123456"));
    assert!(msg.content.contains("1 inline"));
    assert!(msg.content.contains("2 moved to summary"));
}

#[test]
fn should_emit_draft_message_with_pr_url_for_draft_submit() {
    // given
    let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
    let comment = line_comment(LineSide::New, Some(11), None);
    let comment_id = comment.id.clone();
    add_line_comment(&mut app, "src/lib.rs", 11, comment);
    let in_flight = make_in_flight(SubmitEvent::Draft, &[comment_id.as_str()], "abcdef0123", 0);
    let response = make_response(
        999,
        "https://github.com/agavra/tuicr/pull/125#pullrequestreview-999",
        "PENDING",
    );
    // when
    app.finish_pr_submit(in_flight, Ok(response));
    // then
    let msg = app.message.as_ref().expect("info message");
    assert!(msg.content.contains("Pushed pending GitHub review #999"));
    assert!(
        msg.content
            .contains("https://github.com/agavra/tuicr/pull/125"),
        "draft message should include the PR URL — got: {}",
        msg.content
    );
}

#[test]
fn should_keep_comments_as_local_draft_on_submit_failure() {
    // given a local-draft comment in the session
    let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
    let comment = line_comment(LineSide::New, Some(11), None);
    let comment_id = comment.id.clone();
    add_line_comment(&mut app, "src/lib.rs", 11, comment);
    let in_flight = make_in_flight(
        SubmitEvent::Comment,
        &[comment_id.as_str()],
        "abcdef0123",
        0,
    );
    // when — network failure
    app.finish_pr_submit(
        in_flight,
        Err("Cannot submit review: GitHub token lacks pull request write permission.".to_string()),
    );
    // then — the comment is still LocalDraft and editable
    let review = app.session.files.get(&PathBuf::from("src/lib.rs")).unwrap();
    let saved = &review.line_comments.get(&11).unwrap()[0];
    assert_eq!(saved.lifecycle_state, CommentLifecycleState::LocalDraft);
    assert!(saved.remote_review_id.is_none());
    // and — a sticky error message is set
    let msg = app.message.as_ref().expect("error message");
    assert_eq!(msg.message_type, MessageType::Error);
    assert!(msg.content.contains("Submit failed"));
    assert!(msg.content.contains("pull request write permission"));
}

#[test]
fn should_discard_stale_submit_result_when_head_sha_changed() {
    // given a PR session pretending we already swapped heads after submit dispatched.
    let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
    let comment = line_comment(LineSide::New, Some(11), None);
    let comment_id = comment.id.clone();
    add_line_comment(&mut app, "src/lib.rs", 11, comment);
    // record that a submit was in flight for an *older* head.
    let in_flight = make_in_flight(SubmitEvent::Comment, &[comment_id.as_str()], "OLD_HEAD", 0);
    app.pr_submit_state = Some(in_flight);
    let (tx, rx) = std::sync::mpsc::channel();
    app.pr_submit_rx = Some(rx);
    // simulate the bg thread coming back for the OLD head, but with
    // a fresh-head session note: poll path checks repository/pr_number/head_sha.
    tx.send(PrSubmitEvent::Done {
        repository: ForgeRepository::github("github.com", "agavra", "tuicr"),
        pr_number: 125,
        head_sha: "DIFFERENT_HEAD".to_string(),
        result: Ok(make_response(42, "u", "COMMENTED")),
    })
    .unwrap();
    drop(tx);
    // when
    app.poll_pr_submit_events();
    // then — comment lifecycle untouched, info message about discarded result.
    let review = app.session.files.get(&PathBuf::from("src/lib.rs")).unwrap();
    let saved = &review.line_comments.get(&11).unwrap()[0];
    assert_eq!(saved.lifecycle_state, CommentLifecycleState::LocalDraft);
    let msg = app.message.as_ref().expect("info message");
    assert!(
        msg.content.contains("Discarded stale submit result"),
        "got: {}",
        msg.content
    );
}

#[test]
fn should_apply_result_via_poll_when_head_sha_matches() {
    // given a session with a local-draft comment ready to be locked
    let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
    let comment = line_comment(LineSide::New, Some(11), None);
    let comment_id = comment.id.clone();
    add_line_comment(&mut app, "src/lib.rs", 11, comment);
    let in_flight = make_in_flight(
        SubmitEvent::Comment,
        &[comment_id.as_str()],
        "abcdef0123",
        0,
    );
    app.pr_submit_state = Some(in_flight);
    let (tx, rx) = std::sync::mpsc::channel();
    app.pr_submit_rx = Some(rx);
    tx.send(PrSubmitEvent::Done {
        repository: ForgeRepository::github("github.com", "agavra", "tuicr"),
        pr_number: 125,
        head_sha: "abcdef0123".to_string(),
        result: Ok(make_response(123, "u", "COMMENTED")),
    })
    .unwrap();
    drop(tx);
    // when
    app.poll_pr_submit_events();
    // then — lifecycle moved and the spinner state is cleared.
    assert!(app.pr_submit_state.is_none());
    assert!(app.pr_submit_rx.is_none());
    let review = app.session.files.get(&PathBuf::from("src/lib.rs")).unwrap();
    let saved = &review.line_comments.get(&11).unwrap()[0];
    assert_eq!(saved.lifecycle_state, CommentLifecycleState::Submitted);
    assert_eq!(saved.remote_review_id.as_deref(), Some("123"));
}

#[test]
fn should_lock_file_level_comment_via_submit_success() {
    // given — a file-level comment lives in `file_comments`, not `line_comments`
    let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
    let comment = Comment::new("file-level".to_string(), CommentType::from_id("note"), None);
    let comment_id = comment.id.clone();
    {
        let review = app
            .session
            .get_file_mut(&PathBuf::from("src/lib.rs"))
            .unwrap();
        review.file_comments.push(comment);
    }
    let in_flight = make_in_flight(
        SubmitEvent::Comment,
        &[comment_id.as_str()],
        "abcdef0123",
        0,
    );
    let response = make_response(7, "u", "COMMENTED");
    // when
    app.apply_submit_success(&in_flight, &response);
    // then
    let review = app.session.files.get(&PathBuf::from("src/lib.rs")).unwrap();
    assert_eq!(
        review.file_comments[0].lifecycle_state,
        CommentLifecycleState::Submitted
    );
    assert_eq!(
        review.file_comments[0].remote_review_id.as_deref(),
        Some("7")
    );
}

#[test]
fn should_report_stale_head_when_current_differs_from_session_head() {
    let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
    add_line_comment(
        &mut app,
        "src/lib.rs",
        11,
        line_comment(LineSide::New, Some(11), None),
    );
    // simulate a refresh having spotted a newer remote head
    app.current_pr_head = Some("ffff5678".to_string());
    app.start_submit(SubmitEvent::Comment);
    assert!(app.submit_head_is_stale());
}

#[test]
fn should_report_head_not_stale_when_current_matches_session_head() {
    let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
    add_line_comment(
        &mut app,
        "src/lib.rs",
        11,
        line_comment(LineSide::New, Some(11), None),
    );
    app.start_submit(SubmitEvent::Comment);
    assert!(!app.submit_head_is_stale());
}

#[test]
fn should_detect_locked_comment_under_cursor_for_dd_path() {
    // given an app with a locked line comment registered against the
    // diff. We just verify the App helper sees the lock — exercising
    // the handler keypath itself is covered in integration tests.
    let mut app = make_pr_app_with_single_modified_file("src/lib.rs");
    let mut c = line_comment(LineSide::New, Some(11), None);
    c.lifecycle_state = CommentLifecycleState::PushedDraft;
    add_line_comment(&mut app, "src/lib.rs", 11, c);
    // No cursor positioning here — `cursor_on_locked_comment` resolves
    // through `find_comment_at_cursor` which depends on annotations.
    // The annotation indices use 0..N; with a single line comment on
    // line 11 there's exactly one LineComment annotation. We point the
    // cursor at it via diff_state.
    app.rebuild_annotations();
    // Find the LineComment annotation index.
    let idx = app
        .line_annotations
        .iter()
        .position(|a| matches!(a, AnnotatedLine::LineComment { .. }))
        .expect("expected a LineComment annotation");
    app.diff_state.cursor_line = idx;
    assert!(app.cursor_on_locked_comment());
}
