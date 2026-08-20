use crate::app::diff_load::{
    CommitSelectionAnchor, DiffWatchReporter, DiffWatchTick, diff_watch_result_is_stale,
    normalize_diff_watch_result,
};
use crate::app::*;
use crate::forge::traits::{ForgeRepository, PrSessionKey};
use std::sync::mpsc;

/// Minimal `VcsBackend` used only to satisfy `App::build`'s requirement for
/// one. The diff-watch worker never calls `self.vcs`. It opens its own
/// backend via `detect_vcs` (see `spawn_diff_watch_reload`) so a
/// `git2::Repository`, which is `Send` but not `Sync`, never has to be
/// shared with the worker thread. `get_working_tree_diff` panics so a
/// regression that routes the worker back through `self.vcs` fails loudly
/// instead of silently returning fabricated data.
struct StubVcs {
    info: VcsInfo,
}

impl VcsBackend for StubVcs {
    fn info(&self) -> &VcsInfo {
        &self.info
    }

    fn get_working_tree_diff(&self, _highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
        panic!("diff watch must not fetch through self.vcs; it opens its own backend")
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

fn test_vcs_info() -> VcsInfo {
    VcsInfo {
        root_path: PathBuf::from("/tmp"),
        head_commit: "abc123".to_string(),
        branch_name: Some("main".to_string()),
        vcs_type: VcsType::Git,
    }
}

fn build_app(files: Vec<DiffFile>, diff_source: DiffSource) -> App {
    let vcs_info = test_vcs_info();
    let session = ReviewSession::new(
        vcs_info.root_path.clone(),
        vcs_info.head_commit.clone(),
        vcs_info.branch_name.clone(),
        SessionDiffSource::WorkingTree,
    );

    App::build(
        Box::new(StubVcs {
            info: vcs_info.clone(),
        }),
        vcs_info,
        Theme::dark(),
        None,
        false,
        files,
        session,
        diff_source,
        InputMode::Normal,
        Vec::new(),
        None,
        None,
    )
    .expect("failed to build test app")
}

fn test_pull_request_source() -> DiffSource {
    DiffSource::PullRequest(Box::new(PullRequestDiffSource {
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
    }))
}

/// Hunks are left empty: most of these tests never render content.
fn make_diff_file(path: &str, content_hash: u64) -> DiffFile {
    DiffFile {
        old_path: None,
        new_path: Some(PathBuf::from(path)),
        status: FileStatus::Modified,
        hunks: Vec::new(),
        is_binary: false,
        is_too_large: false,
        commit_message_sha: None,
        content_hash,
    }
}

fn make_hunk(new_start: u32, new_count: u32) -> DiffHunk {
    let mut lines = Vec::new();
    for i in 0..new_count {
        lines.push(DiffLine {
            origin: LineOrigin::Context,
            content: format!("hunk line {}", new_start + i),
            old_lineno: Some(new_start + i),
            new_lineno: Some(new_start + i),
            highlighted_spans: None,
        });
    }
    DiffHunk {
        header: format!("@@ -{new_start},{new_count} +{new_start},{new_count} @@"),
        lines,
        old_start: new_start,
        old_count: new_count,
        new_start,
        new_count,
    }
}

fn make_file_with_hunks(path: &str, hunks: Vec<DiffHunk>) -> DiffFile {
    let content_hash = DiffFile::compute_content_hash(&hunks);
    DiffFile {
        old_path: None,
        new_path: Some(PathBuf::from(path)),
        status: FileStatus::Modified,
        hunks,
        is_binary: false,
        is_too_large: false,
        commit_message_sha: None,
        content_hash,
    }
}

fn working_tree_request() -> DiffWatchReloadRequest {
    DiffWatchReloadRequest {
        diff_source: DiffSource::WorkingTree,
        commit_selection_range: None,
    }
}

/// Delivers `event` on a freshly created channel and marks it as the active
/// in-flight reload, then drives `poll_diff_watch_changes` to consume it.
/// Bypasses real spawning entirely, so these tests never touch `detect_vcs`.
fn deliver(app: &mut App, request: DiffWatchReloadRequest, event: DiffWatchReloadEvent) -> bool {
    let (tx, rx) = mpsc::channel();
    tx.send(event).unwrap();
    app.diff_watch_reload = Some(DiffWatchReload { request, rx });
    app.poll_diff_watch_changes()
}

/// Forces the poll deadline into the past so the next call fires
/// immediately. Never sleep to make time pass in these tests.
fn expire_deadline(app: &mut App) {
    app.next_diff_watch_at = Instant::now() - Duration::from_millis(1);
}

// ---------------------------------------------------------------------
// The config setter. Every other test assigns `diff_watch_interval`
// directly, so without these the one function that turns a config value
// into behaviour is never exercised.
// ---------------------------------------------------------------------

/// The maintainer's stated condition for accepting this feature is that it
/// stays off unless asked for. `0` is how a user turns it back off after
/// setting it, so that branch has to keep working.
#[test]
fn should_disable_diff_watch_when_interval_is_zero() {
    let mut app = build_app(vec![], DiffSource::WorkingTree);
    app.set_diff_watch_interval_ms(500);
    assert!(app.diff_watch_interval.is_some());

    app.set_diff_watch_interval_ms(0);

    assert!(app.diff_watch_interval.is_none());
}

#[test]
fn should_arm_the_next_deadline_when_an_interval_is_set() {
    let mut app = build_app(vec![], DiffSource::WorkingTree);
    let before = Instant::now();

    app.set_diff_watch_interval_ms(500);

    assert_eq!(app.diff_watch_interval, Some(Duration::from_millis(500)));
    assert!(
        app.next_diff_watch_at >= before + Duration::from_millis(500),
        "a freshly set interval must wait one full interval before firing"
    );
}

// ---------------------------------------------------------------------
// Gating: whether a tick spawns a reload at all.
// ---------------------------------------------------------------------

#[test]
fn should_not_spawn_when_interval_unset() {
    let mut app = build_app(vec![make_diff_file("a.rs", 1)], DiffSource::WorkingTree);

    let redraw = app.poll_diff_watch_changes();

    assert!(!redraw);
    assert!(app.diff_watch_reload.is_none());
}

#[test]
fn should_not_spawn_when_deadline_not_reached() {
    let mut app = build_app(vec![make_diff_file("a.rs", 1)], DiffSource::WorkingTree);
    app.diff_watch_interval = Some(Duration::from_millis(500));
    app.next_diff_watch_at = Instant::now() + Duration::from_secs(60);

    let redraw = app.poll_diff_watch_changes();

    assert!(!redraw);
    assert!(app.diff_watch_reload.is_none());
}

#[test]
fn should_not_spawn_for_pull_request_source() {
    let mut app = build_app(vec![make_diff_file("a.rs", 1)], test_pull_request_source());
    app.diff_watch_interval = Some(Duration::from_millis(500));
    expire_deadline(&mut app);

    let redraw = app.poll_diff_watch_changes();

    assert!(!redraw);
    assert!(app.diff_watch_reload.is_none());
}

#[test]
fn should_not_spawn_in_pristine_mode() {
    let mut app = build_app(vec![make_diff_file("a.rs", 1)], DiffSource::WorkingTree);
    app.diff_watch_interval = Some(Duration::from_millis(500));
    app.is_pristine_mode = true;
    expire_deadline(&mut app);

    let redraw = app.poll_diff_watch_changes();

    assert!(!redraw);
    assert!(app.diff_watch_reload.is_none());
}

/// `--file` reviews use `FileBackend` (`vcs_type == VcsType::File`), scoped
/// to the files the user named. The worker instead resolves a backend via
/// `detect_vcs`, which only ever discovers a real git/jj/hg repository, so it
/// would silently diff the whole repository instead of the reviewed file.
/// Regression test for that mismatch.
#[test]
fn should_not_spawn_for_file_backed_vcs() {
    let mut app = build_app(vec![make_diff_file("a.rs", 1)], DiffSource::WorkingTree);
    app.diff_watch_interval = Some(Duration::from_millis(500));
    app.vcs_info.vcs_type = VcsType::File;
    expire_deadline(&mut app);

    let redraw = app.poll_diff_watch_changes();

    assert!(!redraw);
    assert!(app.diff_watch_reload.is_none());
}

#[test]
fn should_defer_outside_normal_mode_then_resume() {
    let mut app = build_app(vec![make_diff_file("a.rs", 1)], DiffSource::WorkingTree);
    app.diff_watch_interval = Some(Duration::from_millis(500));
    app.input_mode = InputMode::Comment;
    expire_deadline(&mut app);

    let deferred = app.poll_diff_watch_changes();

    assert!(!deferred);
    assert!(
        app.diff_watch_reload.is_none(),
        "a tick outside Normal mode must not spawn"
    );
    assert!(
        app.next_diff_watch_at > Instant::now(),
        "deadline should be pushed out, not left elapsed"
    );

    app.input_mode = InputMode::Normal;
    expire_deadline(&mut app);

    // Asks the decision rather than driving the poll: a real poll here would
    // start a worker that runs a full diff against whatever checkout the test
    // process happens to sit in.
    assert_eq!(
        app.diff_watch_tick(Instant::now()),
        DiffWatchTick::Fetch(Duration::from_millis(500)),
        "the resumed tick should be clear to fetch"
    );
}

/// A second tick landing while a fetch is already running must not spawn
/// another. Two results racing to apply against a since-changed `diff_files`
/// is the overlap `apply_diff_files`'s invariant rules out. Mirrors the guard
/// `pr_range_reload_state` provides for PR range re-fetches, but skips rather
/// than supersedes, since a periodic tick has no new user intent.
#[test]
fn should_not_spawn_a_second_reload_while_one_is_in_flight() {
    let mut app = build_app(vec![make_diff_file("a.rs", 1)], DiffSource::WorkingTree);
    app.diff_watch_interval = Some(Duration::from_millis(500));
    expire_deadline(&mut app);
    // A real in-flight reload: a live channel the worker has not answered on
    // yet. Nothing is sent, so the poll finds the guard set and nothing to
    // drain.
    let in_flight = working_tree_request();
    let (_tx, rx) = mpsc::channel();
    app.diff_watch_reload = Some(DiffWatchReload {
        request: in_flight.clone(),
        rx,
    });

    let redraw = app.poll_diff_watch_changes();

    assert!(!redraw);
    assert_eq!(
        app.diff_watch_reload.as_ref().map(|r| &r.request),
        Some(&in_flight),
        "the original request must survive; a second fetch must not replace it"
    );
}

/// A worker that panics must still answer, so a crash reaches the user as a
/// warning instead of being inferred from a dead channel and passed over in
/// silence. `DiffWatchReporter`'s `Drop` is what carries that guarantee, and
/// it is only reachable by actually unwinding a thread.
#[test]
fn should_report_a_failure_when_the_worker_panics() {
    let mut app = build_app(vec![make_diff_file("a.rs", 1)], DiffSource::WorkingTree);
    app.diff_watch_interval = Some(Duration::from_millis(500));

    let (tx, rx) = mpsc::channel::<DiffWatchReloadEvent>();
    app.diff_watch_reload = Some(DiffWatchReload {
        request: working_tree_request(),
        rx,
    });

    // Mirrors `spawn_diff_watch_reload`'s worker: build the reporter, then
    // panic before answering. Silenced so the suite's output stays readable.
    let reporter = DiffWatchReporter::new(tx, working_tree_request());
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let panicked = std::thread::spawn(move || {
        let _reporter = reporter;
        panic!("worker died mid-fetch");
    })
    .join();
    std::panic::set_hook(previous_hook);
    assert!(panicked.is_err(), "the worker thread should have panicked");

    app.next_diff_watch_at = Instant::now() + Duration::from_secs(60);
    let redraw = app.poll_diff_watch_changes();

    assert!(redraw, "a crashed worker must be reported, not swallowed");
    assert!(
        app.last_diff_watch_error.is_some(),
        "the crash should take the same warn-once path as any failed fetch"
    );
    assert!(
        app.diff_watch_reload.is_none(),
        "in-flight record must clear"
    );
}

/// Covers the defensive arm, not a path a worker can still take:
/// `DiffWatchReporter` answers even on a panic, so `spawn_diff_watch_reload`
/// no longer produces a sender that vanishes silently. The arm stays because
/// leaving the in-flight record set would block every later tick, so the
/// watcher would be dead for the rest of the session with nothing to say so.
/// This manufactures the dropped sender directly, since nothing else can.
#[test]
fn should_clear_in_flight_state_when_the_worker_dies_without_answering() {
    let mut app = build_app(vec![make_diff_file("a.rs", 1)], DiffSource::WorkingTree);
    app.diff_watch_interval = Some(Duration::from_millis(500));
    let (tx, rx) = mpsc::channel::<DiffWatchReloadEvent>();
    app.diff_watch_reload = Some(DiffWatchReload {
        request: working_tree_request(),
        rx,
    });
    drop(tx);

    app.next_diff_watch_at = Instant::now() + Duration::from_secs(60);
    let redraw = app.poll_diff_watch_changes();

    assert!(!redraw, "a dead worker is not a redraw");
    assert!(
        app.diff_watch_reload.is_none(),
        "in-flight record must be cleared, or the watcher never spawns again"
    );

    expire_deadline(&mut app);
    assert_eq!(
        app.diff_watch_tick(Instant::now()),
        DiffWatchTick::Fetch(Duration::from_millis(500)),
        "the next tick must be able to fetch again"
    );
}

#[test]
fn should_be_clear_to_fetch_when_due_and_none_in_flight() {
    let mut app = build_app(vec![make_diff_file("a.rs", 1)], DiffSource::WorkingTree);
    app.diff_watch_interval = Some(Duration::from_millis(500));
    expire_deadline(&mut app);

    assert_eq!(
        app.diff_watch_tick(Instant::now()),
        DiffWatchTick::Fetch(Duration::from_millis(500))
    );
}

/// The one test that lets a worker actually start, so that deleting
/// `spawn_diff_watch_reload()` from the `Fetch` arm fails something. Every
/// other test asks `diff_watch_tick` instead. The commit range is empty so the
/// worker's fetch returns immediately rather than diffing the whole checkout,
/// and its answer is discarded either way.
#[test]
fn should_spawn_a_worker_when_the_tick_says_fetch() {
    let mut app = build_app(
        vec![make_diff_file("a.rs", 1)],
        DiffSource::CommitRange(Vec::new()),
    );
    app.diff_watch_interval = Some(Duration::from_millis(500));
    expire_deadline(&mut app);

    app.poll_diff_watch_changes();

    assert!(app.diff_watch_reload.is_some());
}

// ---------------------------------------------------------------------
// Applying (or discarding) a landed result. Events are delivered directly
// through a manufactured channel so these tests never spawn a thread.
// ---------------------------------------------------------------------

/// `sort_files_by_directory` (`src/app/tree.rs:131`) drains `diff_files`
/// through a `BTreeMap` keyed by directory, so a fresh fetch reporting the
/// same files in raw backend order must not count as a change. The worker
/// already folds that comparison into `Ok(None)`; applying `None` must be a
/// silent no-op.
#[test]
fn should_not_disturb_view_on_unchanged_result() {
    let gap_id = GapId {
        file_idx: 0,
        hunk_idx: 0,
    };
    let file_a = make_file_with_hunks("a/file.rs", vec![make_hunk(11, 2)]);
    let file_b = make_file_with_hunks("b/file.rs", vec![make_hunk(1, 1)]);
    let mut app = build_app(vec![file_a, file_b], DiffSource::WorkingTree);
    app.toggle_directory("a");
    app.expand_gap(gap_id.clone(), ExpandDirection::Up, Some(5))
        .unwrap();
    let cursor_before = app.diff_state.cursor_line;

    let redraw = deliver(
        &mut app,
        working_tree_request(),
        DiffWatchReloadEvent::Done {
            request: working_tree_request(),
            result: Ok(None),
            change_status: None,
            commits: None,
        },
    );

    assert!(!redraw);
    assert!(
        !app.expanded_dirs.contains("a"),
        "collapsed directory should stay collapsed"
    );
    assert!(
        app.expanded_bottom.contains_key(&gap_id),
        "expanded context gap should stay expanded"
    );
    assert_eq!(app.diff_state.cursor_line, cursor_before);
    assert!(app.message.is_none());
}

#[test]
fn should_apply_and_report_a_changed_result() {
    let mut app = build_app(vec![make_diff_file("a.rs", 1)], DiffSource::WorkingTree);

    let redraw = deliver(
        &mut app,
        working_tree_request(),
        DiffWatchReloadEvent::Done {
            request: working_tree_request(),
            result: Ok(Some(vec![make_diff_file("a.rs", 2)])),
            change_status: None,
            commits: None,
        },
    );

    assert!(redraw);
    assert_eq!(app.diff_files.len(), 1);
    assert_eq!(app.diff_files[0].content_hash, 2);
    assert_eq!(
        app.message.as_ref().map(|m| m.content.as_str()),
        Some("Reloaded 1 files")
    );
}

// ---------------------------------------------------------------------
// Reviewed state against a watched change.
//
// The whole point of marking a file reviewed is that it means "I have read
// this content". A watch tick can replace that content while the mark is
// still showing, so the mark has to come off or it claims something untrue
// about code the user never saw. `:e` already behaves this way through
// `ReviewSession::add_file`; these pin that the watcher inherits it, since
// nothing else in the suite drives that path from a watch result.
// ---------------------------------------------------------------------

#[test]
fn should_unmark_a_reviewed_file_when_a_watch_tick_changes_it() {
    let mut app = build_app(vec![make_diff_file("a.rs", 1)], DiffSource::WorkingTree);
    app.toggle_reviewed_for_file_idx(0, false);
    assert!(
        app.session.is_file_reviewed(&PathBuf::from("a.rs")),
        "test setup: the file must start out reviewed"
    );

    deliver(
        &mut app,
        working_tree_request(),
        DiffWatchReloadEvent::Done {
            request: working_tree_request(),
            result: Ok(Some(vec![make_diff_file("a.rs", 2)])),
            change_status: None,
            commits: None,
        },
    );

    assert!(
        !app.session.is_file_reviewed(&PathBuf::from("a.rs")),
        "a watched content change must clear the reviewed mark"
    );
    assert_eq!(
        app.message.as_ref().map(|m| m.content.as_str()),
        Some("Reloaded 1 files, 1 changed since last review"),
        "the count in the message is how the user learns a mark came off"
    );
}

/// The counterpart, and what makes the test above mean anything: clearing
/// every mark on every tick would satisfy it just as well. A file the tick
/// did not touch keeps its mark.
#[test]
fn should_keep_a_reviewed_file_marked_when_a_watch_tick_changes_a_different_file() {
    let mut app = build_app(
        vec![make_diff_file("a.rs", 1), make_diff_file("b.rs", 1)],
        DiffSource::WorkingTree,
    );
    app.toggle_reviewed_for_file_idx(0, false);

    deliver(
        &mut app,
        working_tree_request(),
        DiffWatchReloadEvent::Done {
            request: working_tree_request(),
            result: Ok(Some(vec![
                make_diff_file("a.rs", 1),
                make_diff_file("b.rs", 2),
            ])),
            change_status: None,
            commits: None,
        },
    );

    assert!(
        app.session.is_file_reviewed(&PathBuf::from("a.rs")),
        "an untouched file must keep its reviewed mark"
    );
    assert_eq!(
        app.message.as_ref().map(|m| m.content.as_str()),
        Some("Reloaded 2 files"),
        "nothing was invalidated, so the message must not claim otherwise"
    );
}

/// Hunk-level marks are the other way a file gets marked reviewed, and they
/// are keyed by hunk content rather than by position. An edited hunk
/// produces a different key, so the stale one is dropped; an untouched hunk
/// in the same file keeps its key and stays marked.
///
/// This is deliberate, and it looks like a bug from the outside: the file
/// list shows the unreviewed glyph the moment the file-level mark clears,
/// while a hunk that did not change stays collapsed under it. The glyph
/// reads only `FileReview::reviewed` and never consults the hunk set, so
/// the two disagree. Preserving the surviving marks is still the better
/// trade, because the alternative discards reviews of untouched hunks.
#[test]
fn should_unmark_only_the_edited_hunk_when_a_watch_tick_changes_it() {
    let before = make_file_with_hunks("a.rs", vec![make_hunk(1, 2), make_hunk(40, 2)]);
    let keys = before.hunk_review_keys();
    let mut app = build_app(vec![before], DiffSource::WorkingTree);
    for key in &keys {
        app.session
            .get_file_mut(&PathBuf::from("a.rs"))
            .expect("file registered at build")
            .toggle_hunk_reviewed(key.clone());
    }

    // Same second hunk, different first hunk.
    let after = make_file_with_hunks("a.rs", vec![make_hunk(1, 3), make_hunk(40, 2)]);
    deliver(
        &mut app,
        working_tree_request(),
        DiffWatchReloadEvent::Done {
            request: working_tree_request(),
            result: Ok(Some(vec![after])),
            change_status: None,
            commits: None,
        },
    );

    let reviewed_hunks = &app
        .session
        .files
        .get(&PathBuf::from("a.rs"))
        .expect("file still in session")
        .reviewed_hunks;
    assert!(
        !reviewed_hunks.contains(&keys[0]),
        "the edited hunk's mark must come off"
    );
    assert!(
        reviewed_hunks.contains(&keys[1]),
        "the untouched hunk must keep its mark"
    );
}

/// Both watchers write to `self.session`, and `src/main.rs` polls them in
/// the same loop iteration. The session watcher merges whatever is on disk,
/// where this file is still recorded as reviewed. A naive merge would hand
/// the mark straight back, and the user would never learn the content moved.
/// `merge_external_session_changes` prevents that by adopting the
/// external value only when the local one still matches the snapshot it was
/// last read at. Clearing the mark breaks that match, so the local clear
/// wins.
///
/// This drives both watchers rather than calling the merge function directly.
/// The merge rule is only half of it. The other half is that the snapshot
/// still says reviewed when the diff watcher clears the live copy.
#[test]
fn should_not_restore_a_cleared_reviewed_mark_when_the_session_watcher_runs_after() {
    let mut app = build_app(vec![make_diff_file("a.rs", 1)], DiffSource::WorkingTree);
    app.toggle_reviewed_for_file_idx(0, false);

    // On disk and in the snapshot the file is reviewed, which is the state
    // that has to lose to the local clear.
    let session_dir = tempfile::tempdir().expect("temp dir");
    let session_file = session_dir.path().join("session.json");
    std::fs::write(
        &session_file,
        serde_json::to_string(&app.session).expect("serialize session"),
    )
    .expect("write session file");
    app.session_path = Some(session_file);
    app.persisted_session_snapshot = app.session.clone();

    deliver(
        &mut app,
        working_tree_request(),
        DiffWatchReloadEvent::Done {
            request: working_tree_request(),
            result: Ok(Some(vec![make_diff_file("a.rs", 2)])),
            change_status: None,
            commits: None,
        },
    );
    assert!(
        !app.session.is_file_reviewed(&PathBuf::from("a.rs")),
        "test setup: the diff watch tick must have cleared the mark first"
    );

    // main.rs polls the session watcher right after the diff watcher.
    app.review_watch_interval = Some(Duration::from_millis(1));
    app.next_review_watch_at = Instant::now() - Duration::from_millis(1);
    app.poll_persisted_session_changes();

    assert!(
        !app.session.is_file_reviewed(&PathBuf::from("a.rs")),
        "the session watcher must not restore a mark the diff watcher cleared"
    );
}

/// The worker compares against the diff as it stood when it was spawned, so
/// a `:e` that lands first leaves the worker still reporting a change it can
/// no longer see. Applying it would clear every expanded context gap and
/// print a second "Reloaded" message with nothing new on screen.
#[test]
fn should_discard_result_that_matches_what_is_already_on_screen() {
    let gap_id = GapId {
        file_idx: 0,
        hunk_idx: 0,
    };
    let file_a = make_file_with_hunks("a/file.rs", vec![make_hunk(11, 2)]);
    let file_b = make_file_with_hunks("b/file.rs", vec![make_hunk(1, 1)]);
    let mut app = build_app(
        vec![file_a.clone(), file_b.clone()],
        DiffSource::WorkingTree,
    );
    app.toggle_directory("a");
    app.expand_gap(gap_id.clone(), ExpandDirection::Up, Some(5))
        .unwrap();
    let cursor_before = app.diff_state.cursor_line;

    let redraw = deliver(
        &mut app,
        working_tree_request(),
        DiffWatchReloadEvent::Done {
            request: working_tree_request(),
            // Same files the app already holds, which is what a worker that
            // raced a `:e` reports.
            result: Ok(Some(vec![file_a, file_b])),
            change_status: None,
            commits: None,
        },
    );

    assert!(!redraw, "nothing changed on screen, so nothing to redraw");
    assert!(
        !app.expanded_dirs.contains("a"),
        "collapsed directory should stay collapsed"
    );
    assert!(
        app.expanded_bottom.contains_key(&gap_id),
        "expanded context gap should stay expanded"
    );
    assert_eq!(app.diff_state.cursor_line, cursor_before);
    assert!(
        app.message.is_none(),
        "a result matching the screen must not report a reload"
    );
}

/// `apply_diff_files`'s cursor capture is only correct when nothing has
/// mutated `self.diff_files`/`self.diff_state` between spawn and apply. A
/// fetch that outlives a diff-source switch must be discarded rather than
/// applied against the new source's state.
#[test]
fn should_discard_result_when_diff_source_changed_since_spawn() {
    let mut app = build_app(vec![make_diff_file("a.rs", 1)], DiffSource::WorkingTree);
    app.diff_source = DiffSource::Staged;

    let redraw = deliver(
        &mut app,
        working_tree_request(),
        DiffWatchReloadEvent::Done {
            request: working_tree_request(),
            result: Ok(Some(vec![make_diff_file("a.rs", 2)])),
            change_status: None,
            commits: None,
        },
    );

    assert!(!redraw);
    assert_eq!(app.diff_files[0].content_hash, 1);
    assert!(app.message.is_none());
}

#[test]
fn should_discard_result_when_commit_selection_changed_since_spawn() {
    let request = DiffWatchReloadRequest {
        diff_source: DiffSource::CommitRange(vec!["a".to_string()]),
        commit_selection_range: Some((0, 0)),
    };
    let mut app = build_app(
        vec![make_diff_file("a.rs", 1)],
        DiffSource::CommitRange(vec!["a".to_string()]),
    );
    app.commit_selection_range = Some((0, 1));

    let redraw = deliver(
        &mut app,
        request.clone(),
        DiffWatchReloadEvent::Done {
            request,
            result: Ok(Some(vec![make_diff_file("a.rs", 2)])),
            change_status: None,
            commits: None,
        },
    );

    assert!(!redraw);
    assert_eq!(app.diff_files[0].content_hash, 1);
    assert!(app.message.is_none());
}

#[test]
fn should_discard_result_delivered_outside_normal_mode() {
    let mut app = build_app(vec![make_diff_file("a.rs", 1)], DiffSource::WorkingTree);
    app.input_mode = InputMode::Comment;

    let redraw = deliver(
        &mut app,
        working_tree_request(),
        DiffWatchReloadEvent::Done {
            request: working_tree_request(),
            result: Ok(Some(vec![make_diff_file("a.rs", 2)])),
            change_status: None,
            commits: None,
        },
    );

    assert!(!redraw);
    assert_eq!(app.diff_files[0].content_hash, 1);
    assert!(app.message.is_none());
}

#[test]
fn should_warn_once_on_repeated_identical_failure() {
    let mut app = build_app(vec![make_diff_file("a.rs", 1)], DiffSource::WorkingTree);

    let first = deliver(
        &mut app,
        working_tree_request(),
        DiffWatchReloadEvent::Done {
            request: working_tree_request(),
            result: Err("boom".to_string()),
            change_status: None,
            commits: None,
        },
    );
    assert!(first, "first occurrence of an error should warn and redraw");
    let msg = app
        .message
        .as_ref()
        .expect("first failure should set a message");
    assert_eq!(msg.message_type, MessageType::Warning);
    let stored_error = app.last_diff_watch_error.clone();
    assert!(stored_error.is_some());

    let second = deliver(
        &mut app,
        working_tree_request(),
        DiffWatchReloadEvent::Done {
            request: working_tree_request(),
            result: Err("boom".to_string()),
            change_status: None,
            commits: None,
        },
    );
    assert!(!second, "identical repeat should be silent");
    assert_eq!(app.last_diff_watch_error, stored_error);

    let third = deliver(
        &mut app,
        working_tree_request(),
        DiffWatchReloadEvent::Done {
            request: working_tree_request(),
            result: Err("boom".to_string()),
            change_status: None,
            commits: None,
        },
    );
    assert!(!third, "identical repeat should stay silent");
    assert_eq!(app.last_diff_watch_error, stored_error);

    let recovered = deliver(
        &mut app,
        working_tree_request(),
        DiffWatchReloadEvent::Done {
            request: working_tree_request(),
            result: Ok(Some(vec![make_diff_file("a.rs", 2)])),
            change_status: None,
            commits: None,
        },
    );
    assert!(recovered);
    assert!(app.last_diff_watch_error.is_none());
    assert_eq!(app.diff_files[0].content_hash, 2);
}

// ---------------------------------------------------------------------
// Plain-function tests: the staleness predicate and the worker's error
// normalization, exercised with no `App`, channel, or thread at all.
// ---------------------------------------------------------------------

#[test]
fn should_not_be_stale_when_nothing_changed_since_spawn() {
    let request = working_tree_request();
    assert!(!diff_watch_result_is_stale(
        &request,
        &DiffSource::WorkingTree,
        None,
        InputMode::Normal,
    ));
}

#[test]
fn should_be_stale_when_diff_source_changed() {
    let request = working_tree_request();
    assert!(diff_watch_result_is_stale(
        &request,
        &DiffSource::Staged,
        None,
        InputMode::Normal,
    ));
}

#[test]
fn should_be_stale_when_commit_selection_changed() {
    let request = DiffWatchReloadRequest {
        diff_source: DiffSource::CommitRange(vec!["a".to_string()]),
        commit_selection_range: Some((0, 0)),
    };
    assert!(diff_watch_result_is_stale(
        &request,
        &DiffSource::CommitRange(vec!["a".to_string()]),
        Some((0, 1)),
        InputMode::Normal,
    ));
}

#[test]
fn should_be_stale_when_input_mode_left_normal() {
    let request = working_tree_request();
    assert!(diff_watch_result_is_stale(
        &request,
        &DiffSource::WorkingTree,
        None,
        InputMode::Comment,
    ));
}

#[test]
fn should_normalize_no_changes_error_to_silent_none() {
    match normalize_diff_watch_result(Err(TuicrError::NoChanges)) {
        Ok(None) => {}
        other => panic!("expected Ok(None), got {other:?}"),
    }
}

#[test]
fn should_normalize_other_errors_to_their_display_text() {
    let expected = TuicrError::VcsCommand("boom".to_string()).to_string();
    match normalize_diff_watch_result(Err(TuicrError::VcsCommand("boom".to_string()))) {
        Err(message) => assert_eq!(message, expected),
        other => panic!("expected Err({expected:?}), got {other:?}"),
    }
}

#[test]
fn should_pass_through_a_successful_fetch() {
    let files = vec![make_diff_file("a.rs", 1)];
    match normalize_diff_watch_result(Ok(Some(files))) {
        Ok(Some(returned)) => {
            assert_eq!(returned.len(), 1);
            assert_eq!(returned[0].content_hash, 1);
        }
        other => panic!("expected Ok(Some(_)), got {other:?}"),
    }
}

// ---------------------------------------------------------------------
// Refreshing the inline commit pane.
// ---------------------------------------------------------------------

/// The core of `reanchored_commit_selection`: a narrowing survives a rebuild
/// by naming its endpoints, not their positions. Without this the pane could
/// not refresh under a narrowing at all, which is the bug this replaced.
#[test]
fn should_move_a_narrowed_selection_onto_the_same_commits_after_a_rebuild() {
    let current = [watch_commit("c2"), watch_commit("c1")];
    // Narrowed to c2 alone, at index 0.
    let rebuilt = [watch_commit("c3"), watch_commit("c2"), watch_commit("c1")];

    assert_eq!(
        App::reanchored_commit_selection(Some((0, 0)), &current, &rebuilt),
        CommitSelectionAnchor::Moved(Some((1, 1))),
        "c2 moved to index 1, so the selection must follow it there"
    );
}

/// A selection covering every row means "everything", and everything is now a
/// different number of rows. There are no endpoints to preserve.
#[test]
fn should_widen_a_full_cover_selection_to_the_whole_rebuilt_pane() {
    let current = [watch_commit("c2"), watch_commit("c1")];
    let rebuilt = [watch_commit("c3"), watch_commit("c2"), watch_commit("c1")];

    assert_eq!(
        App::reanchored_commit_selection(Some((0, 1)), &current, &rebuilt),
        CommitSelectionAnchor::Moved(Some((0, 2))),
        "a selection covering every commit is not a narrowing"
    );
}

/// The other way to be unnarrowed: no selection at all. It widens the same
/// way, which is the behaviour the pane had before any of this.
#[test]
fn should_widen_an_absent_selection_to_the_whole_rebuilt_pane() {
    let current = [watch_commit("c2"), watch_commit("c1")];
    let rebuilt = [watch_commit("c3"), watch_commit("c2"), watch_commit("c1")];

    assert_eq!(
        App::reanchored_commit_selection(None, &current, &rebuilt),
        CommitSelectionAnchor::Moved(Some((0, 2))),
        "no selection at all is not a narrowing"
    );
}

/// An empty rebuilt pane has no index to anchor to, and `rebuilt.len() - 1`
/// would underflow. No caller passes one today, because an empty fetch is
/// refused earlier. This covers the function itself, not a path the watcher
/// can reach.
#[test]
fn should_clear_the_selection_when_the_rebuilt_pane_is_empty() {
    let current = [watch_commit("c1")];

    assert_eq!(
        App::reanchored_commit_selection(Some((0, 0)), &current, &[]),
        CommitSelectionAnchor::Moved(None),
        "an empty pane holds no selection"
    );
}

/// `git reset --hard` drops the commit a narrowing was anchored to. There is
/// no honest index left to put the range on, so the pane must stay as it is
/// rather than silently re-point the review at a different commit.
#[test]
fn should_report_a_narrowed_selection_lost_when_its_commit_is_gone() {
    let current = [watch_commit("c3"), watch_commit("c2"), watch_commit("c1")];
    // Narrowed to c3, which the rebuilt list no longer contains.
    let rebuilt = [watch_commit("c2"), watch_commit("c1")];

    assert_eq!(
        App::reanchored_commit_selection(Some((0, 0)), &current, &rebuilt),
        CommitSelectionAnchor::Lost,
        "a selection whose endpoint vanished has nowhere honest to go"
    );
}

/// A fixed timestamp, not `Utc::now()`. `CommitInfo` derives `PartialEq`, which
/// compares `time`, so a clock-stamped helper would make two commits that stand
/// for the same commit compare unequal and every equality assertion below
/// meaningless. Real commit times are stable, so comparing them is correct in
/// production.
fn watch_commit(id: &str) -> CommitInfo {
    CommitInfo {
        id: id.to_string(),
        short_id: id.to_string(),
        branch_name: None,
        summary: format!("commit {id}"),
        body: None,
        author: "tester".to_string(),
        time: chrono::DateTime::from_timestamp(1_700_000_000, 0).expect("valid timestamp"),
    }
}

/// The inline commit pane is a startup snapshot. Nothing in the reload path
/// rebuilds it, so a commit written while tuicr is open never appears and the
/// user has to restart. With no narrowing to invalidate, a watch result
/// carrying fresh commits should install them.
#[test]
fn should_install_fresh_commits_from_a_watch_result_when_nothing_is_narrowed() {
    let mut app = build_app(vec![], DiffSource::WorkingTree);
    app.review_commits = vec![watch_commit("c2"), watch_commit("c1")];
    // Covers every commit, so it is not a narrowing.
    app.commit_selection_range = Some((0, 1));

    let request = DiffWatchReloadRequest {
        diff_source: DiffSource::WorkingTree,
        commit_selection_range: Some((0, 1)),
    };
    deliver(
        &mut app,
        request.clone(),
        DiffWatchReloadEvent::Done {
            request,
            result: Ok(None),
            change_status: None,
            commits: Some(vec![
                watch_commit("c3"),
                watch_commit("c2"),
                watch_commit("c1"),
            ]),
        },
    );

    assert_eq!(
        app.review_commits
            .iter()
            .map(|c| c.id.as_str())
            .collect::<Vec<_>>(),
        ["c3", "c2", "c1"],
        "a commit written while tuicr is open should appear in the pane"
    );
}

/// A narrowing rides along with the rebuild rather than blocking it. The pane
/// grows, and the range moves to wherever the commit it named ended up. Both
/// halves are asserted together because the pane alone could be right while
/// the range quietly covered a different commit.
#[test]
fn should_refresh_the_commit_pane_and_move_a_narrowing_with_it() {
    let mut app = build_app(vec![], DiffSource::WorkingTree);
    app.review_commits = vec![watch_commit("c2"), watch_commit("c1")];
    // Two commits, narrowed to the newest alone.
    app.commit_selection_range = Some((0, 0));

    let request = DiffWatchReloadRequest {
        diff_source: DiffSource::WorkingTree,
        commit_selection_range: Some((0, 0)),
    };
    deliver(
        &mut app,
        request.clone(),
        DiffWatchReloadEvent::Done {
            request,
            result: Ok(None),
            change_status: None,
            commits: Some(vec![
                watch_commit("c3"),
                watch_commit("c2"),
                watch_commit("c1"),
            ]),
        },
    );

    assert_eq!(
        app.review_commits
            .iter()
            .map(|c| c.id.as_str())
            .collect::<Vec<_>>(),
        ["c3", "c2", "c1"],
        "the new commit must reach the pane even under a narrowing"
    );
    assert_eq!(
        app.commit_selection_range,
        Some((1, 1)),
        "the narrowing must still name c2, which is now at index 1"
    );
}

/// The counterpart, and what keeps the test above from being satisfied by a
/// rebuild that ignores the selection entirely: when the narrowed commit is
/// gone from the fetched list, there is nowhere honest to put the range, so
/// nothing is installed.
#[test]
fn should_leave_the_commit_pane_alone_when_the_narrowed_commit_is_gone() {
    let mut app = build_app(vec![], DiffSource::WorkingTree);
    app.review_commits = vec![watch_commit("c3"), watch_commit("c2"), watch_commit("c1")];
    // Narrowed to c3, which the tick below no longer reports, as if `git reset
    // --hard HEAD~1` ran while the review was open.
    app.commit_selection_range = Some((0, 0));

    let request = DiffWatchReloadRequest {
        diff_source: DiffSource::WorkingTree,
        commit_selection_range: Some((0, 0)),
    };
    deliver(
        &mut app,
        request.clone(),
        DiffWatchReloadEvent::Done {
            request,
            result: Ok(None),
            change_status: None,
            commits: Some(vec![watch_commit("c2"), watch_commit("c1")]),
        },
    );

    assert_eq!(
        app.review_commits
            .iter()
            .map(|c| c.id.as_str())
            .collect::<Vec<_>>(),
        ["c3", "c2", "c1"],
        "the pane must stay as it is when the selection cannot follow"
    );
    assert_eq!(
        app.commit_selection_range,
        Some((0, 0)),
        "and the range must stay on the commit the user chose"
    );
}

/// The scenario this feature exists for, found while dogfooding: staged and
/// unstaged are selected, real commits sit below them, and the user keeps
/// committing while reviewing. A commit written mid-review has to reach the
/// pane, or there is no way to navigate to it without `:commits`, the manual
/// step the watcher removes for the diff.
///
/// The selection must still name the same two rows afterwards. It is a pair
/// of indices, and a commit landing above them would otherwise slide it down
/// onto different rows without saying so.
#[test]
fn should_install_fresh_commits_when_only_staged_and_unstaged_are_selected() {
    let mut app = build_app(vec![], DiffSource::StagedAndUnstaged);
    app.review_commits = vec![
        App::unstaged_commit_entry(),
        App::staged_commit_entry(),
        watch_commit("c1"),
    ];
    // The two synthetic rows only, a strict subset of the three.
    app.commit_selection_range = Some((0, 1));

    let request = DiffWatchReloadRequest {
        diff_source: DiffSource::StagedAndUnstaged,
        commit_selection_range: Some((0, 1)),
    };
    deliver(
        &mut app,
        request.clone(),
        DiffWatchReloadEvent::Done {
            request,
            result: Ok(None),
            change_status: None,
            commits: Some(vec![watch_commit("c2"), watch_commit("c1")]),
        },
    );

    assert_eq!(
        app.review_commits
            .iter()
            .map(|c| c.summary.as_str())
            .collect::<Vec<_>>(),
        [
            "Unstaged changes",
            "Staged changes",
            "commit c2",
            "commit c1"
        ],
        "a commit written mid-review must reach the pane under a narrowing too"
    );
    assert_eq!(
        app.commit_selection_range,
        Some((0, 1)),
        "the selection must still name the staged and unstaged rows"
    );
}

/// Pins the invariant documented on `commit_pane_rows`.
#[test]
fn should_keep_the_staged_and_unstaged_rows_when_installing_fresh_commits() {
    let mut app = build_app(vec![], DiffSource::StagedAndUnstaged);
    app.review_commits = vec![
        App::unstaged_commit_entry(),
        App::staged_commit_entry(),
        watch_commit("c1"),
    ];
    // Covers every row, so it is not a narrowing.
    app.commit_selection_range = Some((0, 2));

    let request = DiffWatchReloadRequest {
        diff_source: DiffSource::StagedAndUnstaged,
        commit_selection_range: Some((0, 2)),
    };
    deliver(
        &mut app,
        request.clone(),
        DiffWatchReloadEvent::Done {
            request,
            result: Ok(None),
            change_status: None,
            commits: Some(vec![watch_commit("c2"), watch_commit("c1")]),
        },
    );

    assert_eq!(
        app.review_commits
            .iter()
            .map(|c| c.summary.as_str())
            .collect::<Vec<_>>(),
        [
            "Unstaged changes",
            "Staged changes",
            "commit c2",
            "commit c1"
        ],
        "the synthetic rows must survive a commit refresh"
    );
}

/// Names the row the pane carries at each index, so the assertions below read
/// as the pane the user is looking at.
fn pane_summaries(app: &App) -> Vec<&str> {
    app.review_commits
        .iter()
        .map(|commit| commit.summary.as_str())
        .collect()
}

/// The reason the pane needs its own signal. Running `git add` while tuicr is
/// open leaves the combined working-tree diff byte-identical, so the diff
/// fingerprint reports nothing and the tick carries no fresh commits either.
/// Without change status, the newly staged side gains no row until the review
/// target is reopened.
#[test]
fn should_add_the_staged_row_when_a_file_is_staged_mid_review() {
    let mut app = build_app(vec![], DiffSource::WorkingTree);
    app.review_commits = vec![App::unstaged_commit_entry(), watch_commit("c1")];
    app.commit_list = app.review_commits.clone();

    let request = DiffWatchReloadRequest {
        diff_source: DiffSource::WorkingTree,
        commit_selection_range: None,
    };
    let redraw = deliver(
        &mut app,
        request.clone(),
        DiffWatchReloadEvent::Done {
            request,
            result: Ok(None),
            commits: None,
            change_status: Some(VcsChangeStatus {
                staged: true,
                unstaged: true,
            }),
        },
    );

    assert!(redraw, "a pane that gained a row has to be redrawn");
    assert_eq!(
        pane_summaries(&app),
        ["Unstaged changes", "Staged changes", "commit c1"],
        "the staged row must appear, after the row already on screen"
    );
    assert_eq!(
        app.commit_list.len(),
        3,
        "the list the cursor indexes into must gain the row too"
    );
}

/// The mirror of the case above. Staging the last unstaged edit empties that
/// side, and a row left behind opens an empty diff when selected.
#[test]
fn should_drop_the_unstaged_row_when_the_last_unstaged_change_is_staged() {
    let mut app = build_app(vec![], DiffSource::WorkingTree);
    app.review_commits = vec![
        App::unstaged_commit_entry(),
        App::staged_commit_entry(),
        watch_commit("c1"),
    ];
    app.commit_list = app.review_commits.clone();

    let request = DiffWatchReloadRequest {
        diff_source: DiffSource::WorkingTree,
        commit_selection_range: None,
    };
    deliver(
        &mut app,
        request.clone(),
        DiffWatchReloadEvent::Done {
            request,
            result: Ok(None),
            commits: None,
            change_status: Some(VcsChangeStatus {
                staged: true,
                unstaged: false,
            }),
        },
    );

    assert_eq!(
        pane_summaries(&app),
        ["Staged changes", "commit c1"],
        "the unstaged row must go once that side is empty"
    );
}

/// The two startup paths disagree about which synthetic row comes first, so a
/// tick that rebuilt the order from status would reorder the pane under
/// whichever of them the user opened from. This pane opened staged-first.
#[test]
fn should_keep_the_pane_order_when_a_synthetic_row_appears() {
    let mut app = build_app(vec![], DiffSource::WorkingTree);
    app.review_commits = vec![App::staged_commit_entry(), watch_commit("c1")];
    app.commit_list = app.review_commits.clone();

    let request = DiffWatchReloadRequest {
        diff_source: DiffSource::WorkingTree,
        commit_selection_range: None,
    };
    deliver(
        &mut app,
        request.clone(),
        DiffWatchReloadEvent::Done {
            request,
            result: Ok(None),
            commits: None,
            change_status: Some(VcsChangeStatus {
                staged: true,
                unstaged: true,
            }),
        },
    );

    assert_eq!(
        pane_summaries(&app),
        ["Staged changes", "Unstaged changes", "commit c1"],
        "the row already on screen keeps its place; the new one follows it"
    );
}

/// Mercurial and Jujutsu report no change status, and neither does a git
/// backend whose status read failed. Leaving the rows exactly as they are is
/// the only honest answer: a missing report is not a report of "nothing".
#[test]
fn should_leave_the_synthetic_rows_alone_when_no_change_status_arrives() {
    let mut app = build_app(vec![], DiffSource::WorkingTree);
    app.review_commits = vec![
        App::unstaged_commit_entry(),
        App::staged_commit_entry(),
        watch_commit("c1"),
    ];
    app.commit_list = app.review_commits.clone();

    let request = DiffWatchReloadRequest {
        diff_source: DiffSource::WorkingTree,
        commit_selection_range: None,
    };
    let redraw = deliver(
        &mut app,
        request.clone(),
        DiffWatchReloadEvent::Done {
            request,
            result: Ok(None),
            commits: None,
            change_status: None,
        },
    );

    assert!(!redraw, "a tick that changed nothing is not a redraw");
    assert_eq!(
        pane_summaries(&app),
        ["Unstaged changes", "Staged changes", "commit c1"],
        "both rows must survive a tick that could not read status"
    );
}

/// `commit_diff_cache` is keyed by raw row indices, so a tick that shifts the
/// rows leaves every key naming a different commit. Narrowing to one row then
/// shows the diff of whatever used to sit at that index. Every other path that
/// replaces `review_commits` clears the cache; the watcher has to as well.
#[test]
fn should_drop_the_cached_selection_diffs_when_the_pane_shifts() {
    let mut app = build_app(vec![], DiffSource::WorkingTree);
    app.review_commits = vec![watch_commit("c2"), watch_commit("c1")];
    app.commit_list = app.review_commits.clone();
    // The diff of c2, which sits at row 0 right now.
    app.commit_diff_cache
        .insert((0, 0), vec![make_diff_file("only_in_c2.rs", 1)]);

    let request = DiffWatchReloadRequest {
        diff_source: DiffSource::WorkingTree,
        commit_selection_range: None,
    };
    // when: a commit written mid-review lands above both rows
    deliver(
        &mut app,
        request.clone(),
        DiffWatchReloadEvent::Done {
            request,
            result: Ok(None),
            commits: Some(vec![
                watch_commit("c3"),
                watch_commit("c2"),
                watch_commit("c1"),
            ]),
            change_status: None,
        },
    );

    // then
    assert_eq!(
        pane_summaries(&app),
        ["commit c3", "commit c2", "commit c1"],
        "the fresh commit must reach the pane"
    );
    assert!(
        app.commit_diff_cache.is_empty(),
        "row 0 now means c3, so a diff cached against c2's index must go"
    );
}

/// The synthetic rows stamp `Utc::now()` when built, so a tick that rebuilt
/// them from scratch would produce a pane that never compares equal to the one
/// on screen. That redraws and reinstalls the pane every interval, moving the
/// cursor under a user who is doing nothing.
#[test]
fn should_not_redraw_when_a_second_tick_reports_the_same_change_status() {
    let mut app = build_app(vec![], DiffSource::WorkingTree);
    app.review_commits = vec![App::unstaged_commit_entry(), watch_commit("c1")];
    app.commit_list = app.review_commits.clone();

    let status = Some(VcsChangeStatus {
        staged: true,
        unstaged: true,
    });
    let request = DiffWatchReloadRequest {
        diff_source: DiffSource::WorkingTree,
        commit_selection_range: None,
    };
    deliver(
        &mut app,
        request.clone(),
        DiffWatchReloadEvent::Done {
            request: request.clone(),
            result: Ok(None),
            commits: None,
            change_status: status,
        },
    );

    // when: the tree has not moved since, so the next tick says the same thing
    let redraw = deliver(
        &mut app,
        request.clone(),
        DiffWatchReloadEvent::Done {
            request,
            result: Ok(None),
            commits: None,
            change_status: status,
        },
    );

    // then
    assert!(!redraw, "an unchanged pane must not redraw every interval");
    assert_eq!(
        pane_summaries(&app),
        ["Unstaged changes", "Staged changes", "commit c1"],
        "and the rows must stay put"
    );
}

/// A tick reports both halves at once. The rows must not cost the pane its
/// fresh commits, and the commits must not cost it the row change.
#[test]
fn should_apply_fresh_commits_and_a_new_synthetic_row_from_one_tick() {
    let mut app = build_app(vec![], DiffSource::WorkingTree);
    app.review_commits = vec![App::unstaged_commit_entry(), watch_commit("c1")];
    app.commit_list = app.review_commits.clone();

    let request = DiffWatchReloadRequest {
        diff_source: DiffSource::WorkingTree,
        commit_selection_range: None,
    };
    deliver(
        &mut app,
        request.clone(),
        DiffWatchReloadEvent::Done {
            request,
            result: Ok(None),
            commits: Some(vec![watch_commit("c2"), watch_commit("c1")]),
            change_status: Some(VcsChangeStatus {
                staged: true,
                unstaged: true,
            }),
        },
    );

    assert_eq!(
        pane_summaries(&app),
        [
            "Unstaged changes",
            "Staged changes",
            "commit c2",
            "commit c1"
        ],
        "one tick has to deliver both halves"
    );
}

/// Pins the cursor half of the invariant documented on
/// `install_refreshed_commit_pane`. `git reset --hard HEAD~2` is the shrink.
#[test]
fn should_keep_the_cursor_inside_a_pane_that_shrank() {
    let mut app = build_app(vec![], DiffSource::WorkingTree);
    app.review_commits = vec![watch_commit("c3"), watch_commit("c2"), watch_commit("c1")];
    app.commit_selection_range = Some((0, 2));
    app.commit_list = app.review_commits.clone();
    app.visible_commit_count = 3;
    app.commit_list_cursor = 2;

    let request = DiffWatchReloadRequest {
        diff_source: DiffSource::WorkingTree,
        commit_selection_range: Some((0, 2)),
    };
    deliver(
        &mut app,
        request.clone(),
        DiffWatchReloadEvent::Done {
            request,
            result: Ok(None),
            change_status: None,
            commits: Some(vec![watch_commit("c1")]),
        },
    );

    assert!(
        app.commit_list_cursor < app.review_commits.len(),
        "cursor {} is past the end of a {}-row pane",
        app.commit_list_cursor,
        app.review_commits.len()
    );
}

/// Pins the selection half of the invariant documented on
/// `install_refreshed_commit_pane`.
#[test]
fn should_keep_the_selection_inside_a_pane_that_shrank() {
    let mut app = build_app(vec![], DiffSource::WorkingTree);
    app.review_commits = vec![watch_commit("c3"), watch_commit("c2"), watch_commit("c1")];
    app.commit_selection_range = Some((0, 2));
    app.commit_list = app.review_commits.clone();
    app.visible_commit_count = 3;

    let request = DiffWatchReloadRequest {
        diff_source: DiffSource::WorkingTree,
        commit_selection_range: Some((0, 2)),
    };
    deliver(
        &mut app,
        request.clone(),
        DiffWatchReloadEvent::Done {
            request,
            result: Ok(None),
            change_status: None,
            commits: Some(vec![watch_commit("c1")]),
        },
    );

    assert_eq!(
        app.commit_selection_range,
        Some((0, 0)),
        "a whole-pane selection must still span the whole pane after it shrank"
    );
}

/// An idle tick must change nothing. The watcher fires every interval whether
/// or not the repository moved, so a refresh that reported "changed" for an
/// identical answer would repaint the pane continuously and reset the cursor
/// with it. Rebuilding from an unchanged fetch has to land back exactly where
/// it started.
#[test]
fn should_report_no_change_when_the_fetched_commits_match_the_pane() {
    let mut app = build_app(vec![], DiffSource::WorkingTree);
    app.review_commits = vec![
        App::unstaged_commit_entry(),
        watch_commit("c2"),
        watch_commit("c1"),
    ];
    app.commit_selection_range = Some((0, 2));
    app.commit_list = app.review_commits.clone();
    app.visible_commit_count = 3;
    app.commit_list_cursor = 1;
    let before = app.review_commits.clone();

    let request = DiffWatchReloadRequest {
        diff_source: DiffSource::WorkingTree,
        commit_selection_range: Some((0, 2)),
    };
    let redraw = deliver(
        &mut app,
        request.clone(),
        DiffWatchReloadEvent::Done {
            request,
            result: Ok(None),
            change_status: None,
            commits: Some(vec![watch_commit("c2"), watch_commit("c1")]),
        },
    );

    assert!(!redraw, "an unchanged tick must not ask for a redraw");
    assert_eq!(app.review_commits, before, "the pane must be untouched");
    assert_eq!(app.commit_list_cursor, 1, "the cursor must not move");
}

/// An empty answer must not empty the pane. `VcsBackend::get_recent_commits`
/// defaults to `Ok(Vec::new())` for "not supported" rather than an error
/// (`src/vcs/traits.rs`), so a backend that never implements it would otherwise
/// delete every row on the first tick. All four real backends do implement it
/// and `--file` mode is already guarded out of the watcher, so this is a guard
/// against a future backend rather than a live bug.
#[test]
fn should_not_empty_the_pane_when_the_fetch_returns_no_commits() {
    let mut app = build_app(vec![], DiffSource::WorkingTree);
    app.review_commits = vec![watch_commit("c2"), watch_commit("c1")];
    app.commit_selection_range = Some((0, 1));
    app.commit_list = app.review_commits.clone();
    app.visible_commit_count = 2;
    let before = app.review_commits.clone();

    let request = DiffWatchReloadRequest {
        diff_source: DiffSource::WorkingTree,
        commit_selection_range: Some((0, 1)),
    };
    deliver(
        &mut app,
        request.clone(),
        DiffWatchReloadEvent::Done {
            request,
            result: Ok(None),
            change_status: None,
            commits: Some(Vec::new()),
        },
    );

    assert_eq!(
        app.review_commits, before,
        "an empty answer must change nothing"
    );
}

/// Pins the growth half of the invariant on `install_refreshed_commit_pane`:
/// a selection spanning the whole pane must still span it after rows arrive.
/// Without this the range stays at its old width and silently becomes a real
/// narrowing, hiding comments on the commits it no longer covers.
#[test]
fn should_widen_a_whole_pane_selection_when_commits_arrive() {
    let mut app = build_app(vec![], DiffSource::WorkingTree);
    app.review_commits = vec![watch_commit("c2"), watch_commit("c1")];
    app.commit_selection_range = Some((0, 1));
    app.commit_list = app.review_commits.clone();
    app.visible_commit_count = 2;

    let request = DiffWatchReloadRequest {
        diff_source: DiffSource::WorkingTree,
        commit_selection_range: Some((0, 1)),
    };
    deliver(
        &mut app,
        request.clone(),
        DiffWatchReloadEvent::Done {
            request,
            result: Ok(None),
            change_status: None,
            commits: Some(vec![
                watch_commit("c3"),
                watch_commit("c2"),
                watch_commit("c1"),
            ]),
        },
    );

    assert_eq!(app.review_commits.len(), 3);
    assert_eq!(
        app.commit_selection_range,
        Some((0, 2)),
        "a whole-pane selection must still span the whole pane after it grew"
    );
}

/// The cursor must keep pointing at the commit the user put it on, not at the
/// index. A new commit arrives at the top of a newest-first pane, so every row
/// below it shifts down one. An index-preserving cursor would then highlight a
/// different commit every time the user commits, in the workflow this feature
/// exists for.
#[test]
fn should_follow_the_same_commit_when_rows_shift_beneath_the_cursor() {
    let mut app = build_app(vec![], DiffSource::WorkingTree);
    app.review_commits = vec![watch_commit("c2"), watch_commit("c1")];
    app.commit_selection_range = Some((0, 1));
    app.commit_list = app.review_commits.clone();
    app.visible_commit_count = 2;
    // Sitting on c1, the oldest row.
    app.commit_list_cursor = 1;

    let request = DiffWatchReloadRequest {
        diff_source: DiffSource::WorkingTree,
        commit_selection_range: Some((0, 1)),
    };
    deliver(
        &mut app,
        request.clone(),
        DiffWatchReloadEvent::Done {
            request,
            result: Ok(None),
            change_status: None,
            commits: Some(vec![
                watch_commit("c3"),
                watch_commit("c2"),
                watch_commit("c1"),
            ]),
        },
    );

    assert_eq!(
        app.review_commits[app.commit_list_cursor].id, "c1",
        "the cursor should still be on c1, not on whatever row index 1 now holds"
    );
}
