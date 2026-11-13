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
    let mut app = build_pr_app();
    // The review-comments header only renders once the section has content.
    app.session.review_comments.push(crate::model::Comment::new(
        "review-level".to_string(),
        crate::model::CommentType::from_id("note"),
        None,
    ));
    app.rebuild_annotations();
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

fn pr_info_annotation_count(app: &App) -> usize {
    app.line_annotations
        .iter()
        .filter(|line| matches!(line, crate::app::AnnotatedLine::PrInfoLine { .. }))
        .count()
}

#[test]
fn should_collapse_pr_description_to_one_row_when_marked_reviewed() {
    let mut app = build_pr_app();
    let expanded_rows = pr_info_annotation_count(&app);
    assert!(expanded_rows > 1);
    let file_header_line = app.calculate_file_scroll_offset(0);
    app.diff_state.cursor_line = 2;

    app.toggle_reviewed();

    assert!(app.session.pr_description_reviewed);
    assert!(app.dirty);
    assert_eq!(pr_info_annotation_count(&app), 1);
    assert_eq!(app.diff_state.cursor_line, 0);
    assert_eq!(
        app.calculate_file_scroll_offset(0),
        file_header_line + 1 - expanded_rows,
        "rows below the panel shift up by the collapsed rows"
    );
    // The file under the panel is untouched.
    assert!(!app.session.is_file_reviewed(&PathBuf::from("src/lib.rs")));

    let mut lines = Vec::new();
    let mut line_idx = 0usize;
    crate::ui::pr_info_panel::append_pr_info_section(&app, &mut lines, &mut line_idx, usize::MAX);
    assert_eq!(lines.len(), 1);
    let text: String = lines[0]
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect();
    assert!(text.contains("✓ PR #42 Add panel"), "collapsed row: {text}");
}

#[test]
fn should_expand_reviewed_pr_description_when_toggled_again() {
    let mut app = build_pr_app();
    let expanded_rows = pr_info_annotation_count(&app);
    app.diff_state.cursor_line = 0;

    app.toggle_reviewed();
    app.toggle_reviewed();

    assert!(!app.session.pr_description_reviewed);
    assert_eq!(pr_info_annotation_count(&app), expanded_rows);
    assert_eq!(app.diff_state.cursor_line, 0);
}

#[test]
fn should_toggle_file_not_description_when_cursor_is_on_a_file() {
    let mut app = build_pr_app();
    app.jump_to_file(0);

    app.toggle_reviewed();

    assert!(!app.session.pr_description_reviewed);
    assert!(app.session.is_file_reviewed(&PathBuf::from("src/lib.rs")));
}

#[test]
fn should_carry_pr_description_reviewed_to_new_head_session() {
    let mut previous = build_pr_app().session.clone();
    previous.pr_description_reviewed = true;
    let next = ReviewSession::new(
        previous.repo_path.clone(),
        "fedcba9876543210".to_string(),
        previous.branch_name.clone(),
        SessionDiffSource::PullRequest,
    );

    let carried = App::reviewed_state_carried_forward(&previous, next, &[], &[]);

    assert!(carried.pr_description_reviewed);
}

#[test]
fn should_unreview_pr_description_on_clear_without_counting_it_as_a_file() {
    let mut app = build_pr_app();
    app.diff_state.cursor_line = 0;
    app.toggle_reviewed();
    assert!(app.session.pr_description_reviewed);

    app.clear_comments(crate::model::ClearScope::CommentsAndReviewed);

    assert!(!app.session.pr_description_reviewed);
    assert!(
        pr_info_annotation_count(&app) > 1,
        "description expands again"
    );
    let message = app.message.as_ref().map(|m| m.content.clone());
    assert_eq!(message.as_deref(), Some("Unreviewed the PR description"));
}

fn draw_full(app: &mut App) -> String {
    let backend = ratatui::backend::TestBackend::new(120, 40);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| crate::ui::app_layout::render(frame, app))
        .expect("draw frame");
    let buffer = terminal.backend().buffer().clone();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn should_lay_out_pr_description_at_real_width_on_first_frame() {
    let body = "Ship it with a reasonably long description line";
    for (mode, stale_width) in [
        (crate::app::DiffViewMode::Unified, 0),
        (crate::app::DiffViewMode::Unified, 8),
        (crate::app::DiffViewMode::SideBySide, 0),
        (crate::app::DiffViewMode::SideBySide, 8),
    ] {
        let mut app = build_pr_app();
        let mut info = sample_pr_info();
        info.details.body = body.to_string();
        app.pr_info = Some(info);
        app.diff_view_mode = mode;
        app.diff_state.wrap_lines = true;
        // What the first frame (0) or a pre-resize frame (8) leaves behind.
        app.diff_state.viewport_width = stale_width;
        app.rebuild_annotations();

        let screen = draw_full(&mut app);

        assert!(
            screen.contains(body),
            "{mode:?} stale width {stale_width}: description must not be wrapped at the stale width\n{screen}"
        );
    }
}

#[test]
fn should_reuse_pr_info_rows_until_width_metadata_or_theme_changes() {
    use crate::ui::pr_info_panel::pr_info_display_lines;
    use std::sync::Arc;

    let mut app = build_pr_app();
    app.diff_state.viewport_width = 80;
    let first = pr_info_display_lines(&app);
    assert!(
        Arc::ptr_eq(&first, &pr_info_display_lines(&app)),
        "same width and metadata reuse the built rows"
    );

    app.diff_state.viewport_width = 60;
    let narrower = pr_info_display_lines(&app);
    assert!(!Arc::ptr_eq(&first, &narrower), "width change rebuilds");

    app.pr_info.as_mut().unwrap().details.body = "Changed body".to_string();
    let edited = pr_info_display_lines(&app);
    assert!(!Arc::ptr_eq(&narrower, &edited), "metadata change rebuilds");
    assert!(edited.iter().any(|line| {
        line.spans
            .iter()
            .any(|s| s.content.contains("Changed body"))
    }));

    app.set_theme(Theme::light());
    assert!(
        !Arc::ptr_eq(&edited, &pr_info_display_lines(&app)),
        "theme change rebuilds"
    );

    app.session.pr_description_reviewed = true;
    assert_eq!(pr_info_display_lines(&app).len(), 1, "reviewed collapses");
    app.session.pr_description_reviewed = false;
    assert!(pr_info_display_lines(&app).len() > 1, "expands again");
}
