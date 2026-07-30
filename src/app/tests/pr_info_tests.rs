use std::path::PathBuf;

use crate::app::{App, DiffSource, InputMode, PullRequestDiffSource};
use crate::forge::traits::{
    ForgeRepository, PrSessionKey, PullRequestCheckStatus, PullRequestDetails, PullRequestInfo,
    PullRequestIssueComment, PullRequestReviewStatus,
};
use crate::model::{DiffFile, FileStatus, ReviewSession, SessionDiffSource};
use crate::theme::Theme;
use crate::vcs::traits::VcsType;
use crate::vcs::{PrNoopVcs, VcsInfo};

fn sample_pr_info() -> PullRequestInfo {
    PullRequestInfo {
        details: PullRequestDetails {
            repository: ForgeRepository::github("github.com", "owner", "repo"),
            number: 42,
            title: "Add panel".to_string(),
            url: "https://github.com/owner/repo/pull/42".to_string(),
            state: "OPEN".to_string(),
            is_draft: false,
            author: Some("alice".to_string()),
            head_ref_name: "feature".to_string(),
            base_ref_name: "main".to_string(),
            head_sha: "abc1234567890".to_string(),
            base_sha: "def0987654321".to_string(),
            body: "Ship it".to_string(),
            updated_at: None,
            closed: false,
            merged_at: None,
            diff_start_sha: None,
        },
        review_decision: Some("REVIEW_REQUIRED".to_string()),
        mergeable: Some("MERGEABLE".to_string()),
        merge_state: Some("BLOCKED".to_string()),
        requested_reviewers: vec!["bob".to_string()],
        latest_reviews: vec![PullRequestReviewStatus {
            author: Some("carol".to_string()),
            state: "APPROVED".to_string(),
            submitted_at: None,
        }],
        checks: vec![PullRequestCheckStatus {
            name: "build".to_string(),
            status: Some("COMPLETED".to_string()),
            conclusion: Some("SUCCESS".to_string()),
            url: Some("https://github.com/owner/repo/actions/runs/1".to_string()),
        }],
        issue_comments: vec![PullRequestIssueComment {
            author: Some("dave".to_string()),
            body: "Looks good".to_string(),
            url: Some("https://github.com/owner/repo/pull/42#issuecomment-1".to_string()),
            created_at: None,
        }],
    }
}

fn build_pr_app() -> App {
    let pr = PullRequestDiffSource {
        key: PrSessionKey::new(
            ForgeRepository::github("github.com", "owner", "repo"),
            42,
            "abc1234567890",
        ),
        base_sha: "def0987654321".to_string(),
        title: "Add panel".to_string(),
        url: "https://github.com/owner/repo/pull/42".to_string(),
        head_ref_name: "feature".to_string(),
        base_ref_name: "main".to_string(),
        state: "OPEN".to_string(),
        closed: false,
        merged: false,
    };
    let vcs_info = VcsInfo {
        root_path: PathBuf::from("forge:github.com/owner/repo"),
        head_commit: pr.key.head_sha.clone(),
        branch_name: Some(pr.head_ref_name.clone()),
        vcs_type: VcsType::File,
    };
    let mut session = ReviewSession::new(
        vcs_info.root_path.clone(),
        pr.key.head_sha.clone(),
        Some(pr.head_ref_name.clone()),
        SessionDiffSource::PullRequest,
    );
    session.pr_session_key = Some(pr.key.clone());
    let mut app = App::build(
        Box::new(PrNoopVcs::new(vcs_info.clone())),
        vcs_info,
        Theme::dark(),
        None,
        false,
        vec![DiffFile {
            old_path: None,
            new_path: Some("src/lib.rs".into()),
            status: FileStatus::Modified,
            hunks: vec![],
            is_binary: false,
            is_too_large: false,
            is_commit_message: false,
            content_hash: 0,
        }],
        session,
        DiffSource::PullRequest(Box::new(pr)),
        InputMode::Normal,
        Vec::new(),
        None,
        None,
    )
    .expect("build pr app");
    app.pr_info = Some(sample_pr_info());
    app.rebuild_annotations();
    app
}

#[test]
fn should_not_add_pr_info_to_file_tree() {
    let app = build_pr_app();
    assert!(
        app.line_annotations
            .iter()
            .any(|line| matches!(line, crate::app::AnnotatedLine::PrInfoLine { .. }))
    );
    assert!(app.build_visible_items().iter().all(|item| matches!(
        item,
        crate::app::FileTreeItem::Directory { .. } | crate::app::FileTreeItem::File { .. }
    )));
}

#[test]
fn should_order_overview_sections_before_file_diffs() {
    let app = build_pr_app();
    assert!(matches!(
        app.line_annotations.first(),
        Some(crate::app::AnnotatedLine::PrInfoLine { line_idx: 0 })
    ));

    let review_header_idx = app
        .line_annotations
        .iter()
        .position(|line| matches!(line, crate::app::AnnotatedLine::ReviewCommentsHeader));
    let issue_header_idx = app
        .line_annotations
        .iter()
        .position(|line| matches!(line, crate::app::AnnotatedLine::IssueCommentsHeader));
    let first_file_idx = app
        .line_annotations
        .iter()
        .position(|line| matches!(line, crate::app::AnnotatedLine::FileHeader { .. }));

    assert!(review_header_idx.is_some());
    assert!(issue_header_idx.is_some());
    assert!(first_file_idx.is_some());
    assert!(review_header_idx.unwrap() < issue_header_idx.unwrap());
    assert!(issue_header_idx.unwrap() < first_file_idx.unwrap());
}

#[test]
fn should_start_overview_at_top_of_main_view() {
    let mut app = build_pr_app();
    app.jump_to_file(0);
    assert!(app.diff_state.cursor_line > 0);

    app.diff_state.cursor_line = 0;
    app.ensure_cursor_visible();
    assert!(crate::ui::pr_info_panel::is_cursor_in_pr_info(&app));
}

#[test]
fn should_walk_from_overview_to_first_file_with_next_file() {
    let mut app = build_pr_app();
    app.diff_state.cursor_line = 0;
    app.next_file();
    assert_eq!(app.diff_state.current_file_idx, 0);
    assert!(!crate::ui::pr_info_panel::is_cursor_in_pr_info(&app));
}

#[test]
fn should_build_pr_info_panel_lines() {
    let lines =
        crate::ui::pr_info_panel::build_pr_info_lines(&sample_pr_info(), 80, &Theme::dark());
    assert!(lines.len() > 5);
}

#[test]
fn should_keep_pr_info_annotations_in_sync_with_rendered_lines_at_wrap_boundary() {
    // A description line exactly `width` columns wide fits on one line at
    // `width` but wraps at `width - 1`. The counter (line_annotations) and
    // the renderer must agree on the wrap width, or every row below the
    // panel maps to the wrong annotation. Regression guard for the desync.
    let width = 60usize;
    let mut info = sample_pr_info();
    info.details.body = format!("{} {}", "X".repeat(30), "Y".repeat(29)); // 30 + 1 + 29 = 60

    let mut app = build_pr_app();
    app.pr_info = Some(info);
    app.diff_state.viewport_width = width;
    app.rebuild_annotations();

    let annotated = app
        .line_annotations
        .iter()
        .filter(|line| matches!(line, crate::app::AnnotatedLine::PrInfoLine { .. }))
        .count();

    let mut lines = Vec::new();
    let mut line_idx = 0usize;
    crate::ui::pr_info_panel::append_pr_info_section(&app, &mut lines, &mut line_idx, usize::MAX);

    assert!(annotated > 0, "expected PR-info annotations");
    assert_eq!(
        annotated,
        lines.len(),
        "PrInfoLine annotation count must equal the rendered PR-info line count"
    );
}
