use std::collections::HashSet;

use crate::app::*;
use crate::forge::traits::{ForgeRepository, PrSessionKey, PullRequestCommit};
use crate::model::{DiffFile, DiffHunk, DiffLine, ReviewSession, SessionDiffSource};
use crate::theme::Theme;
use crate::vcs::traits::VcsType;
use crate::vcs::{PrNoopVcs, VcsInfo};

fn request(status: FileStatus) -> PrFileHighlightRequest {
    PrFileHighlightRequest {
        repository: ForgeRepository::github("github.com", "owner", "repo"),
        pr_number: 42,
        session_head_sha: "session-head".to_string(),
        key: PrFileHighlightKey {
            generation: 7,
            old_sha: "range-start".to_string(),
            new_sha: "range-end".to_string(),
            old_path: Some(PathBuf::from("old/name.rs")),
            new_path: Some(PathBuf::from("new/name.rs")),
            content_hash: 99,
        },
        status,
    }
}

fn rust_comment_diff() -> DiffFile {
    DiffFile {
        old_path: Some(PathBuf::from("src/example.rs")),
        new_path: Some(PathBuf::from("src/example.rs")),
        status: FileStatus::Modified,
        hunks: vec![DiffHunk {
            header: "@@ -11 +11 @@".to_string(),
            lines: vec![DiffLine {
                origin: LineOrigin::Addition,
                content: "comment 9 changed".to_string(),
                old_lineno: None,
                new_lineno: Some(11),
                highlighted_spans: None,
            }],
            old_start: 11,
            old_count: 0,
            new_start: 11,
            new_count: 1,
        }],
        is_binary: false,
        is_too_large: false,
        is_commit_message: false,
        content_hash: 99,
    }
}

fn build_pr_app() -> App {
    let repository = ForgeRepository::github("github.com", "owner", "repo");
    let pr_source = PullRequestDiffSource {
        key: PrSessionKey::new(repository, 42, "session-head"),
        base_sha: "base-sha".to_string(),
        title: "PR".to_string(),
        url: "https://github.com/owner/repo/pull/42".to_string(),
        head_ref_name: "feature".to_string(),
        base_ref_name: "main".to_string(),
        state: "OPEN".to_string(),
        closed: false,
        merged: false,
    };
    let vcs_info = VcsInfo {
        root_path: PathBuf::from("forge:github.com/owner/repo"),
        head_commit: "session-head".to_string(),
        branch_name: Some("feature".to_string()),
        vcs_type: VcsType::File,
    };
    let session = ReviewSession::new(
        vcs_info.root_path.clone(),
        vcs_info.head_commit.clone(),
        vcs_info.branch_name.clone(),
        SessionDiffSource::PullRequest,
    );
    App::build(
        Box::new(PrNoopVcs::new(vcs_info.clone())),
        vcs_info,
        Theme::dark(),
        None,
        false,
        vec![rust_comment_diff()],
        session,
        DiffSource::PullRequest(Box::new(pr_source)),
        InputMode::Normal,
        Vec::new(),
        None,
        None,
    )
    .expect("PR app should build")
}

fn long_comment_content() -> String {
    [
        "fn before() {}",
        "/*",
        "comment 1",
        "comment 2",
        "comment 3",
        "comment 4",
        "comment 5",
        "comment 6",
        "comment 7",
        "comment 8",
        "comment 9 changed",
        "*/",
        "fn after() {}",
    ]
    .join("\n")
}

#[test]
fn modified_pr_file_hydration_uses_active_exact_endpoints_and_paths() {
    let request = request(FileStatus::Renamed);

    let old = request
        .old_content_request()
        .expect("rename should fetch its old side");
    let new = request
        .new_content_request()
        .expect("rename should fetch its new side");

    assert_eq!(old.sha, "range-start");
    assert_eq!(old.path, PathBuf::from("old/name.rs"));
    assert_eq!(new.sha, "range-end");
    assert_eq!(new.path, PathBuf::from("new/name.rs"));
}

#[test]
fn added_and_deleted_pr_files_fetch_only_their_existing_side() {
    let added = request(FileStatus::Added);
    assert!(added.old_content_request().is_none());
    assert_eq!(added.new_content_request().unwrap().sha, "range-end");

    let deleted = request(FileStatus::Deleted);
    assert_eq!(deleted.old_content_request().unwrap().sha, "range-start");
    assert!(deleted.new_content_request().is_none());
}

#[test]
fn completed_pr_file_hydration_applies_full_comment_state() {
    let mut app = build_pr_app();
    app.install_pr_diff_endpoints("base-sha".to_string(), "session-head".to_string());
    let request = app
        .current_pr_file_highlight_request()
        .expect("current PR file should be hydratable");
    let key = request.key.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    app.pr_file_highlight_rx = Some(rx);
    tx.send(PrFileHighlightEvent {
        request,
        old_content: None,
        new_content: Some(long_comment_content()),
    })
    .unwrap();

    assert!(app.poll_pr_file_highlight_events());
    assert!(app.pr_file_highlight_finished.contains(&key));
    let spans = app.diff_files[0].hunks[0].lines[0]
        .highlighted_spans
        .as_ref()
        .expect("hydration should add highlighted spans");
    let colors: HashSet<_> = spans.iter().filter_map(|(style, _)| style.fg).collect();
    assert_eq!(colors.len(), 1);
}

#[test]
fn mismatched_remote_content_keeps_provisional_diff_text() {
    let mut app = build_pr_app();
    app.install_pr_diff_endpoints("base-sha".to_string(), "session-head".to_string());
    let request = app
        .current_pr_file_highlight_request()
        .expect("current PR file should be hydratable");
    let mismatched_content = long_comment_content().replace("comment 9 changed", "wrong source");
    let (tx, rx) = std::sync::mpsc::channel();
    app.pr_file_highlight_rx = Some(rx);
    tx.send(PrFileHighlightEvent {
        request,
        old_content: None,
        new_content: Some(mismatched_content),
    })
    .unwrap();

    assert!(app.poll_pr_file_highlight_events());
    assert!(
        app.diff_files[0].hunks[0].lines[0]
            .highlighted_spans
            .is_none(),
        "mismatched fetched text must not replace the patch line"
    );
    assert_eq!(
        app.diff_files[0].hunks[0].lines[0].content,
        "comment 9 changed"
    );
}

#[test]
fn hydration_for_a_different_pr_is_discarded() {
    let mut app = build_pr_app();
    app.install_pr_diff_endpoints("base-sha".to_string(), "session-head".to_string());
    let request = app
        .current_pr_file_highlight_request()
        .expect("current PR file should be hydratable");
    let key = request.key.clone();
    let DiffSource::PullRequest(pr) = &mut app.diff_source else {
        panic!("test app should be in PR mode");
    };
    pr.key.number = 43;
    let (tx, rx) = std::sync::mpsc::channel();
    app.pr_file_highlight_rx = Some(rx);
    tx.send(PrFileHighlightEvent {
        request,
        old_content: None,
        new_content: Some(long_comment_content()),
    })
    .unwrap();

    assert!(!app.poll_pr_file_highlight_events());
    assert!(!app.pr_file_highlight_finished.contains(&key));
    assert!(
        app.diff_files[0].hunks[0].lines[0]
            .highlighted_spans
            .is_none()
    );
}

#[test]
fn failed_pr_file_hydration_is_terminal_for_the_active_generation() {
    let mut app = build_pr_app();
    app.install_pr_diff_endpoints("base-sha".to_string(), "session-head".to_string());
    let request = app
        .current_pr_file_highlight_request()
        .expect("current PR file should be hydratable");
    let key = request.key.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    app.pr_file_highlight_rx = Some(rx);
    tx.send(PrFileHighlightEvent {
        request,
        old_content: None,
        new_content: None,
    })
    .unwrap();

    assert!(app.poll_pr_file_highlight_events());
    assert!(app.pr_file_highlight_finished.contains(&key));
    assert!(
        app.diff_files[0].hunks[0].lines[0]
            .highlighted_spans
            .is_none()
    );
}

#[test]
fn successful_range_diff_install_switches_active_hydration_endpoints() {
    let mut app = build_pr_app();
    app.install_pr_diff_endpoints("base-sha".to_string(), "session-head".to_string());
    let previous_generation = app.pr_diff_endpoints.as_ref().unwrap().generation;
    let request = PrRangeReloadRequest {
        repository: ForgeRepository::github("github.com", "owner", "repo"),
        pr_number: 42,
        head_sha: "session-head".to_string(),
        start_sha: "range-start".to_string(),
        end_sha: "range-end".to_string(),
        range: (0, 0),
        started_at: Instant::now(),
        anchor: None,
    };
    let patch = "diff --git a/src/example.rs b/src/example.rs\n\
index 1111111..2222222 100644\n\
--- a/src/example.rs\n\
+++ b/src/example.rs\n\
@@ -11 +11 @@\n\
-old comment\n\
+new comment\n";

    app.finish_pr_range_reload(
        &request,
        crate::vcs::diff_parser::git_fixture_file_patches(patch),
    )
    .expect("range diff should install");

    let endpoints = app.pr_diff_endpoints.as_ref().unwrap();
    assert_eq!(endpoints.old_sha, "range-start");
    assert_eq!(endpoints.new_sha, "range-end");
    assert_ne!(endpoints.generation, previous_generation);
}

#[test]
fn cached_full_pr_diff_restore_reinstalls_cumulative_endpoints() {
    let mut app = build_pr_app();
    app.range_diff_files = Some(app.diff_files.clone());
    app.pr_commits = vec![
        PullRequestCommit {
            oid: "session-head".to_string(),
            short_oid: "session".to_string(),
            summary: "new".to_string(),
            author: "author".to_string(),
            timestamp: None,
        },
        PullRequestCommit {
            oid: "older".to_string(),
            short_oid: "older".to_string(),
            summary: "old".to_string(),
            author: "author".to_string(),
            timestamp: None,
        },
    ];
    app.commit_selection_range = Some((0, 1));
    app.install_pr_diff_endpoints("range-start".to_string(), "range-end".to_string());

    app.reload_pr_inline_selection();

    let endpoints = app.pr_diff_endpoints.as_ref().unwrap();
    assert_eq!(endpoints.old_sha, "base-sha");
    assert_eq!(endpoints.new_sha, "session-head");
}

#[test]
fn installing_new_diff_endpoints_cancels_stale_range_reload() {
    let mut app = build_pr_app();
    let range_request = PrRangeReloadRequest {
        repository: ForgeRepository::github("github.com", "owner", "repo"),
        pr_number: 42,
        head_sha: "session-head".to_string(),
        start_sha: "range-start".to_string(),
        end_sha: "range-end".to_string(),
        range: (0, 0),
        started_at: Instant::now(),
        anchor: None,
    };
    let (_tx, rx) = std::sync::mpsc::channel();
    app.pr_range_reload_state = Some(range_request);
    app.pr_range_reload_rx = Some(rx);

    app.install_pr_diff_endpoints("new-base".to_string(), "new-head".to_string());

    assert!(app.pr_range_reload_state.is_none());
    assert!(app.pr_range_reload_rx.is_none());
}

#[test]
fn cumulative_reload_cache_reports_that_strict_range_must_be_refetched() {
    let mut app = build_pr_app();
    app.pr_commits = vec![
        PullRequestCommit {
            oid: "session-head".to_string(),
            short_oid: "session".to_string(),
            summary: "new".to_string(),
            author: "author".to_string(),
            timestamp: None,
        },
        PullRequestCommit {
            oid: "older".to_string(),
            short_oid: "older".to_string(),
            summary: "old".to_string(),
            author: "author".to_string(),
            timestamp: None,
        },
    ];
    app.commit_selection_range = Some((0, 0));

    assert!(app.cache_cumulative_pr_diff());
    let cached = app
        .range_diff_files
        .as_ref()
        .expect("cumulative diff should be cached");
    assert_eq!(cached.len(), app.diff_files.len());
    assert_eq!(cached[0].old_path, app.diff_files[0].old_path);
    assert_eq!(cached[0].new_path, app.diff_files[0].new_path);
    assert_eq!(cached[0].content_hash, app.diff_files[0].content_hash);
}

#[test]
fn stale_pr_file_hydration_is_discarded_after_endpoint_generation_changes() {
    let mut app = build_pr_app();
    app.install_pr_diff_endpoints("base-sha".to_string(), "session-head".to_string());
    let stale_request = app
        .current_pr_file_highlight_request()
        .expect("current PR file should be hydratable");
    let stale_key = stale_request.key.clone();
    app.install_pr_diff_endpoints("range-start".to_string(), "range-end".to_string());

    let (tx, rx) = std::sync::mpsc::channel();
    app.pr_file_highlight_rx = Some(rx);
    tx.send(PrFileHighlightEvent {
        request: stale_request,
        old_content: None,
        new_content: Some(long_comment_content()),
    })
    .unwrap();

    assert!(!app.poll_pr_file_highlight_events());
    assert!(!app.pr_file_highlight_finished.contains(&stale_key));
    assert!(
        app.diff_files[0].hunks[0].lines[0]
            .highlighted_spans
            .is_none()
    );
}

// ---------------------------------------------------------------------------
// Scheduler gates.
//
// `schedule_current_pr_file_highlight` spawns a thread that builds a real
// forge backend and hits the network, so these cover only the paths that must
// *not* spawn. `pr_file_highlight_rx` staying `None` is the observable proof
// no work started.
// ---------------------------------------------------------------------------

fn fake_backend() -> Box<dyn crate::forge::traits::ForgeBackend> {
    Box::new(super::FakeForgeBackend {
        local_checkout: None,
    })
}

#[test]
fn binary_files_are_not_hydratable() {
    let mut app = build_pr_app();
    app.install_pr_diff_endpoints("base-sha".to_string(), "session-head".to_string());
    app.diff_files[0].is_binary = true;

    assert!(app.current_pr_file_highlight_request().is_none());
}

#[test]
fn oversized_files_are_not_hydratable() {
    let mut app = build_pr_app();
    app.install_pr_diff_endpoints("base-sha".to_string(), "session-head".to_string());
    app.diff_files[0].is_too_large = true;

    assert!(app.current_pr_file_highlight_request().is_none());
}

#[test]
fn files_without_hunks_are_not_hydratable() {
    let mut app = build_pr_app();
    app.install_pr_diff_endpoints("base-sha".to_string(), "session-head".to_string());
    app.diff_files[0].hunks.clear();

    assert!(app.current_pr_file_highlight_request().is_none());
}

#[test]
fn hydration_is_not_requested_before_endpoints_are_installed() {
    // A PR whose diff endpoints haven't been pinned yet has no revision pair
    // to fetch against, so there is nothing safe to request.
    let app = build_pr_app();
    assert!(app.pr_diff_endpoints.is_none());

    assert!(app.current_pr_file_highlight_request().is_none());
}

#[test]
fn scheduling_is_skipped_without_a_forge_backend() {
    let mut app = build_pr_app();
    app.install_pr_diff_endpoints("base-sha".to_string(), "session-head".to_string());
    assert!(app.forge_backend.is_none());

    app.schedule_current_pr_file_highlight();

    assert!(app.pr_file_highlight_rx.is_none());
}

#[test]
fn scheduling_is_single_flight_while_a_fetch_is_in_progress() {
    let mut app = build_pr_app();
    app.install_pr_diff_endpoints("base-sha".to_string(), "session-head".to_string());
    app.forge_backend = Some(fake_backend());
    // An in-flight fetch owns the receiver; a second one would orphan it.
    let (tx, rx) = std::sync::mpsc::channel::<PrFileHighlightEvent>();
    app.pr_file_highlight_rx = Some(rx);

    app.schedule_current_pr_file_highlight();

    // The original receiver is still the live one: sending on the paired
    // sender is observable through it, which a replacement would have broken.
    tx.send(PrFileHighlightEvent {
        request: app
            .current_pr_file_highlight_request()
            .expect("current PR file should be hydratable"),
        old_content: None,
        new_content: Some(long_comment_content()),
    })
    .expect("original receiver must still be installed");
    assert!(app.poll_pr_file_highlight_events());
}

#[test]
fn scheduling_is_skipped_for_a_file_already_hydrated_at_this_generation() {
    let mut app = build_pr_app();
    app.install_pr_diff_endpoints("base-sha".to_string(), "session-head".to_string());
    app.forge_backend = Some(fake_backend());
    let key = app
        .current_pr_file_highlight_request()
        .expect("current PR file should be hydratable")
        .key;
    app.pr_file_highlight_finished.insert(key);

    app.schedule_current_pr_file_highlight();

    assert!(app.pr_file_highlight_rx.is_none());
}

#[test]
fn installing_new_endpoints_makes_a_finished_file_hydratable_again() {
    // The finished set is generation-scoped: a range change must not leave a
    // file permanently un-hydratable at its new revisions.
    let mut app = build_pr_app();
    app.install_pr_diff_endpoints("base-sha".to_string(), "session-head".to_string());
    let key = app
        .current_pr_file_highlight_request()
        .expect("current PR file should be hydratable")
        .key;
    app.pr_file_highlight_finished.insert(key.clone());

    app.install_pr_diff_endpoints("range-start".to_string(), "range-end".to_string());

    assert!(app.pr_file_highlight_finished.is_empty());
    let refreshed = app
        .current_pr_file_highlight_request()
        .expect("current PR file should be hydratable at the new endpoints");
    assert_ne!(refreshed.key, key);
    assert!(!app.pr_file_highlight_finished.contains(&refreshed.key));
}
