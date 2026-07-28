use std::path::PathBuf;

use crate::app::{App, DiffSource, FileTreeItem, InputMode, PullRequestDiffSource};
use crate::forge::traits::{
    ForgeRepository, PrSessionKey, PullRequestCheckStatus, PullRequestDetails, PullRequestInfo,
    PullRequestReviewStatus,
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
fn should_prepend_pr_info_tree_entry_in_pr_mode() {
    let app = build_pr_app();
    let items = app.build_visible_items();
    assert!(matches!(items.first(), Some(FileTreeItem::PrInfo)));
}

#[test]
fn should_prepend_pr_info_annotations_before_review_comments() {
    let app = build_pr_app();
    assert!(matches!(
        app.line_annotations.first(),
        Some(crate::app::AnnotatedLine::PrInfoHeader)
    ));
    assert!(
        app.line_annotations
            .iter()
            .any(|line| { matches!(line, crate::app::AnnotatedLine::PrInfoLine { .. }) })
    );
    let review_header_idx = app
        .line_annotations
        .iter()
        .position(|line| matches!(line, crate::app::AnnotatedLine::ReviewCommentsHeader));
    let first_file_idx = app
        .line_annotations
        .iter()
        .position(|line| matches!(line, crate::app::AnnotatedLine::FileHeader { .. }));
    assert!(review_header_idx.is_some());
    assert!(first_file_idx.is_some());
    assert!(review_header_idx.unwrap() < first_file_idx.unwrap());
}

#[test]
fn should_jump_to_pr_description_at_top_of_main_view() {
    let mut app = build_pr_app();
    app.jump_to_file(0);
    assert!(app.diff_state.cursor_line > 0);

    app.jump_to_pr_info();
    assert_eq!(app.diff_state.cursor_line, 0);
    assert_eq!(app.get_selected_tree_item(), Some(FileTreeItem::PrInfo));
    assert!(crate::ui::pr_info_panel::is_cursor_in_pr_info(&app));
}

#[test]
fn should_walk_from_pr_description_to_first_file_with_next_file() {
    let mut app = build_pr_app();
    app.jump_to_pr_info();
    app.next_file();
    assert_eq!(app.diff_state.current_file_idx, 0);
    assert!(!crate::ui::pr_info_panel::is_cursor_in_pr_info(&app));
}

#[test]
fn should_walk_from_first_file_to_pr_description_with_prev_file() {
    let mut app = build_pr_app();
    app.jump_to_file(0);
    app.prev_file();
    assert_eq!(app.diff_state.cursor_line, 0);
    assert!(crate::ui::pr_info_panel::is_cursor_in_pr_info(&app));
}

#[test]
fn should_build_pr_info_panel_lines() {
    let lines = crate::ui::pr_info_panel::build_pr_info_lines(&sample_pr_info(), 80);
    assert!(lines.len() > 5);
}
