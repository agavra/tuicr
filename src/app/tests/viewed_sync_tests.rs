//! Guards on when a file-reviewed toggle is allowed to reach the forge.
//!
//! The positive path is covered at the backend seam
//! (`forge::github::gh::tests`); here we only prove the worker never starts
//! where it must not, because starting it means shelling out to `gh`.

use crate::app::*;
use crate::forge::traits::{ForgeRepository, PullRequestDetails, PullRequestInfo};
use crate::model::{DiffFile, DiffHunk, DiffLine, FileStatus, LineOrigin};
use crate::vcs::traits::{VcsBackend, VcsInfo, VcsType};
use std::path::PathBuf;

struct StubVcs(VcsInfo);
impl VcsBackend for StubVcs {
    fn info(&self) -> &VcsInfo {
        &self.0
    }
    fn get_working_tree_diff(
        &self,
        _hl: &crate::syntax::SyntaxHighlighter,
    ) -> crate::error::Result<Vec<DiffFile>> {
        Ok(Vec::new())
    }
    fn fetch_context_lines(
        &self,
        _path: &std::path::Path,
        _status: FileStatus,
        _ref_commit: Option<&str>,
        _start: u32,
        _end: u32,
    ) -> crate::error::Result<Vec<DiffLine>> {
        Ok(Vec::new())
    }
    fn file_line_count(
        &self,
        _path: &std::path::Path,
        _status: FileStatus,
        _ref_commit: Option<&str>,
    ) -> crate::error::Result<u32> {
        Ok(0)
    }
}

fn file(path: &str) -> DiffFile {
    let hunks = vec![DiffHunk {
        header: "@@ -1,1 +1,1 @@".to_string(),
        lines: vec![DiffLine {
            origin: LineOrigin::Addition,
            content: "added".to_string(),
            old_lineno: None,
            new_lineno: Some(1),
            highlighted_spans: None,
        }],
        old_start: 1,
        old_count: 0,
        new_start: 1,
        new_count: 1,
    }];
    let content_hash = DiffFile::compute_content_hash(&hunks);
    DiffFile {
        old_path: None,
        new_path: Some(PathBuf::from(path)),
        status: FileStatus::Added,
        hunks,
        is_binary: false,
        is_too_large: false,
        is_commit_message: false,
        content_hash,
    }
}

fn app() -> App {
    app_with(&["src/main.rs"])
}

fn app_with(paths: &[&str]) -> App {
    let vcs_info = VcsInfo {
        root_path: PathBuf::from("/tmp"),
        head_commit: "head".into(),
        branch_name: Some("main".into()),
        vcs_type: VcsType::Git,
    };
    let session = ReviewSession::new(
        vcs_info.root_path.clone(),
        vcs_info.head_commit.clone(),
        vcs_info.branch_name.clone(),
        SessionDiffSource::WorkingTree,
    );
    App::build(
        Box::new(StubVcs(vcs_info.clone())),
        vcs_info,
        crate::theme::Theme::dark(),
        None,
        false,
        paths.iter().map(|p| file(p)).collect(),
        session,
        DiffSource::WorkingTree,
        InputMode::Normal,
        Vec::new(),
        None,
        None,
    )
    .expect("build app")
}

fn details(repository: ForgeRepository) -> PullRequestDetails {
    PullRequestDetails {
        repository,
        number: 125,
        title: "Add a thing".into(),
        url: "https://example.test/pull/125".into(),
        state: "OPEN".into(),
        is_draft: false,
        author: None,
        head_ref_name: "feature".into(),
        base_ref_name: "main".into(),
        head_sha: "headsha".into(),
        base_sha: "basesha".into(),
        body: String::new(),
        updated_at: None,
        closed: false,
        merged_at: None,
        diff_start_sha: None,
    }
}

/// Put `app` into PR mode against `repository` without touching the network.
fn enter_pr_mode(app: &mut App, repository: ForgeRepository) {
    let details = details(repository);
    app.diff_source =
        DiffSource::PullRequest(Box::new(PullRequestDiffSource::from_details(&details)));
    app.pr_info = Some(PullRequestInfo::from_details(details));
}

#[test]
fn should_not_start_viewed_sync_for_local_diffs() {
    // given — a working-tree review with the sync switched on
    let mut app = app();
    app.forge_config.sync_viewed = true;

    // when
    app.toggle_reviewed_for_file_idx(0, false);

    // then — nothing to push to; there is no pull request
    assert!(app.session.is_file_reviewed(&PathBuf::from("src/main.rs")));
    assert!(app.viewed_sync.is_none());
}

#[test]
fn should_not_start_viewed_sync_when_disabled_in_config() {
    // given — a GitHub PR, sync left at its default (off)
    let mut app = app();
    enter_pr_mode(
        &mut app,
        ForgeRepository::github("github.com", "agavra", "tuicr"),
    );

    // when
    app.toggle_reviewed_for_file_idx(0, false);

    // then
    assert!(app.session.is_file_reviewed(&PathBuf::from("src/main.rs")));
    assert!(app.viewed_sync.is_none());
}

#[test]
fn should_not_start_viewed_sync_on_forges_without_viewed_state() {
    // given — GitLab has no per-viewer file state, so the backend would only
    // answer `UnsupportedOperation`; don't spawn a worker to find that out.
    let mut app = app();
    app.forge_config.sync_viewed = true;
    enter_pr_mode(
        &mut app,
        ForgeRepository::gitlab("gitlab.com", "agavra", "tuicr"),
    );

    // when
    app.toggle_reviewed_for_file_idx(0, false);

    // then
    assert!(app.viewed_sync.is_none());
}

#[test]
fn should_ignore_viewed_sync_results_with_no_worker() {
    // given — polling is unconditional in the main loop
    let mut app = app();

    // when/then — no channel, no repaint, no panic
    assert!(!app.poll_viewed_sync_events());
}

#[test]
fn should_not_read_viewed_state_for_local_diffs() {
    // given — the sync is on, but there is no pull request behind this review
    let mut app = app();
    app.forge_config.sync_viewed = true;

    // when — the main loop pumps the sync on every tick
    let redraw = app.poll_viewed_sync_events();

    // then — no read was started
    assert!(!redraw);
    assert!(app.viewed_seeded.is_none());
    assert!(app.viewed_seed_rx.is_none());
}

#[test]
fn should_not_read_viewed_state_when_disabled_in_config() {
    // given — a GitHub PR with the sync left off
    let mut app = app();
    enter_pr_mode(
        &mut app,
        ForgeRepository::github("github.com", "agavra", "tuicr"),
    );

    // when
    app.poll_viewed_sync_events();

    // then
    assert!(app.viewed_seeded.is_none());
}

#[test]
fn should_mark_files_the_forge_reports_as_viewed() {
    // given — two files, neither reviewed locally
    let mut app = app_with(&["src/main.rs", "src/lib.rs"]);

    // when — GitHub says one of them is already ticked
    let redraw = app.apply_remote_viewed_state(&[PathBuf::from("src/lib.rs")]);

    // then
    assert!(redraw);
    assert!(app.session.is_file_reviewed(&PathBuf::from("src/lib.rs")));
    assert!(!app.session.is_file_reviewed(&PathBuf::from("src/main.rs")));
    // and — the seeded markers are worth persisting
    assert!(app.dirty);
}

#[test]
fn should_not_clear_local_markers_the_forge_does_not_report() {
    // given — a file reviewed here but not ticked on GitHub, which is what a
    // failed push, or a marker made before the sync was switched on, leaves
    // behind
    let mut app = app_with(&["src/main.rs", "src/lib.rs"]);
    app.toggle_reviewed_for_file_idx(0, false);
    assert!(app.session.is_file_reviewed(&PathBuf::from("src/main.rs")));

    // when — the forge reports only the other file
    app.apply_remote_viewed_state(&[PathBuf::from("src/lib.rs")]);

    // then — the read adds markers, it never takes them away
    assert!(app.session.is_file_reviewed(&PathBuf::from("src/main.rs")));
    assert!(app.session.is_file_reviewed(&PathBuf::from("src/lib.rs")));
}

#[test]
fn should_ignore_viewed_paths_outside_the_review() {
    // given — the PR covers files this session filtered out, so GitHub
    // reports paths the review knows nothing about
    let mut app = app_with(&["src/main.rs"]);

    // when
    let redraw = app.apply_remote_viewed_state(&[PathBuf::from("docs/ignored.md")]);

    // then — nothing marked, nothing to repaint, no session entry invented
    assert!(!redraw);
    assert!(!app.dirty);
    assert!(
        !app.session
            .files
            .contains_key(&PathBuf::from("docs/ignored.md"))
    );
}
