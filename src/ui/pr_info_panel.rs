use ratatui::{
    style::Style,
    text::{Line, Span},
};

use crate::app::App;
use crate::forge::traits::{
    PullRequestCheckStatus, PullRequestInfo, PullRequestIssueComment, PullRequestReviewStatus,
};
use crate::model::CommentType;
use crate::theme::Theme;
use crate::ui::comment_panel::{self, CommentTypePresentation};
use crate::ui::diff_view::{HEADER_RULE, cursor_indicator, cursor_indicator_spaced};
use crate::ui::styles;

/// Every rendered PR-info line is prefixed with a two-column cursor indicator
/// (`cursor_indicator_spaced`), so the wrapped body has that much less room.
/// The annotation/height counters and the renderer MUST wrap
/// `build_pr_info_lines` at this same width — otherwise `line_annotations`
/// desyncs from the rendered `Vec<Line>` and cursor↔line mapping drifts for
/// every row below the panel.
pub(crate) const PR_INFO_INDICATOR_WIDTH: usize = 2;

/// Width available for wrapped PR-info content at the given viewport width.
pub(crate) fn pr_info_content_width(viewport_width: usize) -> usize {
    viewport_width
        .saturating_sub(PR_INFO_INDICATOR_WIDTH)
        .max(1)
}

pub fn pr_info_render_height(app: &App) -> usize {
    app.pr_info
        .as_ref()
        .map(|info| {
            build_pr_info_lines(
                info,
                pr_info_content_width(app.diff_state.viewport_width),
                &app.theme,
            )
            .len()
        })
        .unwrap_or(0)
}

pub fn issue_comments_render_height(app: &App) -> usize {
    let Some(info) = app.pr_info.as_ref() else {
        return 0;
    };
    if info.issue_comments.is_empty() {
        return 0;
    }
    let mut height = if app.is_single_file_view { 0 } else { 1 };
    let width = app.diff_state.viewport_width.max(1);
    for comment in &info.issue_comments {
        height += issue_comment_display_lines(comment, width);
    }
    height
}

pub fn is_cursor_in_pr_info(app: &App) -> bool {
    pr_info_render_height(app) > 0 && app.diff_state.cursor_line < pr_info_render_height(app)
}

pub fn is_cursor_in_issue_comments(app: &App) -> bool {
    let start = app.issue_comments_start_line();
    let end = start + issue_comments_render_height(app);
    end > start && (start..end).contains(&app.diff_state.cursor_line)
}

pub fn append_pr_info_section(
    app: &App,
    lines: &mut Vec<Line<'static>>,
    line_idx: &mut usize,
    current_line_idx: usize,
) {
    let Some(info) = app.pr_info.as_ref() else {
        return;
    };

    let content_width = pr_info_content_width(app.diff_state.viewport_width);
    for mut pr_line in build_pr_info_lines(info, content_width, &app.theme) {
        let indicator = cursor_indicator_spaced(*line_idx, current_line_idx);
        pr_line.spans.insert(
            0,
            Span::styled(indicator, styles::current_line_indicator_style(&app.theme)),
        );
        lines.push(pr_line);
        *line_idx += 1;
    }
}

/// `visible` is the half-open logical-row range the caller is fully building
/// this frame; boxes outside it are replaced with blank placeholder rows. These
/// comments sit at the very top of the document, so for most of a review they
/// are scrolled past — and formatting them costs the same markdown pass as any
/// other comment box.
pub fn append_issue_comments_section(
    app: &App,
    lines: &mut Vec<Line<'static>>,
    line_idx: &mut usize,
    current_line_idx: usize,
    content_width: usize,
    visible: (usize, usize),
) {
    let Some(info) = app.pr_info.as_ref() else {
        return;
    };
    if info.issue_comments.is_empty() {
        return;
    }

    if !app.is_single_file_view {
        let header = format!("═══ PR #{} Comments ", info.details.number);
        lines.push(Line::from(vec![
            Span::styled(
                cursor_indicator_spaced(*line_idx, current_line_idx),
                styles::current_line_indicator_style(&app.theme),
            ),
            Span::styled(header, styles::file_header_style(&app.theme)),
            Span::styled(HEADER_RULE, styles::file_header_style(&app.theme)),
        ]));
        *line_idx += 1;
    }

    let note_type = CommentType::from_id("note");
    let presentation = CommentTypePresentation {
        label: app.comment_type_label(&note_type),
        color: app.comment_type_color(&note_type),
    };
    for comment in &info.issue_comments {
        // Derive the row count from the width the box is actually formatted at
        // — `content_width` is the viewport minus the indicator column, which is
        // exactly the offset `issue_comment_display_lines` expects.
        let rows = issue_comment_display_lines(comment, content_width.saturating_add(1));
        if !crate::ui::diff_view::comment_box_visible(*line_idx, rows, visible) {
            crate::ui::diff_view::skip_comment_box(lines, line_idx, rows);
            continue;
        }
        for mut comment_line in
            format_issue_comment_lines(&app.theme, comment, content_width, &presentation)
        {
            let indicator = cursor_indicator(*line_idx, current_line_idx);
            comment_line.spans.insert(
                0,
                Span::styled(indicator, styles::current_line_indicator_style(&app.theme)),
            );
            lines.push(comment_line);
            *line_idx += 1;
        }
    }
}

pub fn build_pr_info_lines(
    info: &PullRequestInfo,
    width: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let content_width = width.max(1);
    let details = &info.details;

    push_section_header(
        &mut lines,
        theme,
        format!("═══ PR #{} {} ", details.number, details.title),
    );

    let body = if details.body.trim().is_empty() {
        "(no description)".to_string()
    } else {
        details.body.clone()
    };
    for paragraph in body.lines() {
        push_wrapped_line(&mut lines, paragraph.to_string(), content_width);
    }

    push_blank(&mut lines);
    push_section_header(
        &mut lines,
        theme,
        format!("═══ PR #{} Status ", details.number),
    );

    let mut status_parts = Vec::new();
    if details.merged_at.is_some() {
        status_parts.push("Merged".to_string());
    } else if details.closed {
        status_parts.push("Closed".to_string());
    } else {
        status_parts.push(details.state.clone());
    }
    if details.is_draft {
        status_parts.push("Draft".to_string());
    }
    if let Some(decision) = &info.review_decision {
        status_parts.push(humanize_token(decision));
    }
    if let Some(state) = &info.merge_state {
        status_parts.push(format!("merge {}", humanize_token(state)));
    }
    if let Some(mergeable) = &info.mergeable {
        status_parts.push(humanize_token(mergeable));
    }
    push_wrapped_line(
        &mut lines,
        format!("Status: {}", status_parts.join(" · ")),
        content_width,
    );

    let head_short = details.head_sha.chars().take(8).collect::<String>();
    let mut branch_line = format!(
        "{} → {} · {}",
        details.head_ref_name, details.base_ref_name, head_short
    );
    if let Some(author) = &details.author {
        branch_line.push_str(&format!(" · @{author}"));
    }
    if let Some(updated) = details.updated_at {
        branch_line.push_str(&format!(
            " · updated {}",
            updated.format("%Y-%m-%d %H:%M UTC")
        ));
    }
    push_wrapped_line(&mut lines, branch_line, content_width);

    if !info.requested_reviewers.is_empty() {
        push_wrapped_line(
            &mut lines,
            format!("Requested: {}", format_users(&info.requested_reviewers)),
            content_width,
        );
    }

    let approved = reviews_by_state(&info.latest_reviews, "APPROVED");
    let changes = reviews_by_state(&info.latest_reviews, "CHANGES_REQUESTED");
    let commented = reviews_by_state(&info.latest_reviews, "COMMENTED");
    if !approved.is_empty() {
        push_wrapped_line(
            &mut lines,
            format!("Approved: {}", format_users(&approved)),
            content_width,
        );
    }
    if !changes.is_empty() {
        push_wrapped_line(
            &mut lines,
            format!("Changes requested: {}", format_users(&changes)),
            content_width,
        );
    }
    if !commented.is_empty() {
        push_wrapped_line(
            &mut lines,
            format!("Commented: {}", format_users(&commented)),
            content_width,
        );
    }

    if !info.checks.is_empty() {
        push_blank(&mut lines);
        push_wrapped_line(&mut lines, "Checks".to_string(), content_width);
        for check in &info.checks {
            lines.push(format_check_line(check, content_width, theme));
        }
    }

    lines
}

pub fn issue_comment_display_lines(
    comment: &PullRequestIssueComment,
    viewport_width: usize,
) -> usize {
    let content_area = viewport_width.saturating_sub(10);
    let visual_lines: usize = comment
        .body
        .split('\n')
        .map(|line| comment_panel::wrap_segments(line, content_area).len())
        .sum();
    2 + visual_lines
}

fn format_issue_comment_lines(
    theme: &Theme,
    comment: &PullRequestIssueComment,
    width: usize,
    presentation: &CommentTypePresentation,
) -> Vec<Line<'static>> {
    comment_panel::format_comment_lines(
        theme,
        presentation.clone(),
        &comment.body,
        None,
        width,
        comment.author.as_deref(),
    )
}

fn push_section_header(lines: &mut Vec<Line<'static>>, theme: &Theme, title: String) {
    lines.push(Line::from(vec![
        Span::styled(title, styles::file_header_style(theme)),
        Span::styled(HEADER_RULE, styles::file_header_style(theme)),
    ]));
}

fn push_wrapped_line(lines: &mut Vec<Line<'static>>, text: String, width: usize) {
    let style = Style::default();
    for chunk in wrap_text(&text, width) {
        lines.push(Line::from(Span::styled(chunk, style)));
    }
}

fn push_blank(lines: &mut Vec<Line<'static>>) {
    if lines.last().is_some_and(|line| !line.spans.is_empty()) {
        lines.push(Line::default());
    }
}

fn format_check_line(
    check: &PullRequestCheckStatus,
    content_width: usize,
    theme: &Theme,
) -> Line<'static> {
    let summary = format!("{} {}", check_glyph(check), format_check_summary(check));
    let mut spans = Vec::new();
    for chunk in wrap_text(&summary, content_width) {
        spans.push(Span::raw(chunk));
    }
    if let Some(url) = check.url.as_deref().filter(|url| !url.is_empty()) {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(url.to_string(), styles::dim_style(theme)));
    }
    Line::from(spans)
}

fn wrap_text(text: &str, width: usize) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_width = 0usize;
    for word in text.split_whitespace() {
        let word_width = word.chars().count();
        if current_width == 0 {
            current.push_str(word);
            current_width = word_width;
            continue;
        }
        if current_width + 1 + word_width <= width {
            current.push(' ');
            current.push_str(word);
            current_width += 1 + word_width;
        } else {
            lines.push(std::mem::take(&mut current));
            current.push_str(word);
            current_width = word_width;
        }
    }
    if !current.is_empty() || lines.is_empty() {
        lines.push(current);
    }
    lines
}

fn humanize_token(token: &str) -> String {
    token
        .split('_')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().chain(chars).collect(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn format_users(users: &[String]) -> String {
    users
        .iter()
        .map(|user| {
            if user.contains('/') {
                user.clone()
            } else {
                format!("@{user}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn reviews_by_state(reviews: &[PullRequestReviewStatus], state: &str) -> Vec<String> {
    reviews
        .iter()
        .filter(|review| review.state == state)
        .filter_map(|review| review.author.clone())
        .collect()
}

fn check_glyph(check: &PullRequestCheckStatus) -> &'static str {
    let outcome = check
        .conclusion
        .as_deref()
        .or(check.status.as_deref())
        .unwrap_or("");
    match outcome {
        "SUCCESS" | "COMPLETED" if check.conclusion.as_deref() == Some("SUCCESS") => "✓",
        "FAILURE" | "ERROR" | "TIMED_OUT" | "ACTION_REQUIRED" => "✗",
        "PENDING" | "IN_PROGRESS" | "QUEUED" | "WAITING" => "○",
        _ => "·",
    }
}

fn format_check_summary(check: &PullRequestCheckStatus) -> String {
    let mut parts = vec![check.name.clone()];
    if let Some(status) = &check.status {
        parts.push(humanize_token(status));
    }
    if let Some(conclusion) = &check.conclusion {
        parts.push(humanize_token(conclusion));
    }
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::traits::{ForgeRepository, PullRequestDetails};
    use crate::theme::Theme;

    fn sample_info() -> PullRequestInfo {
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
                body: "Ship the panel".to_string(),
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

    #[test]
    fn should_build_title_and_status_sections() {
        let lines = build_pr_info_lines(&sample_info(), 80, &Theme::dark());
        let rendered = lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        assert!(
            rendered
                .iter()
                .any(|line| line.contains("PR #42 Add panel"))
        );
        assert!(rendered.iter().any(|line| line.contains("Ship the panel")));
        assert!(rendered.iter().any(|line| line.contains("PR #42 Status")));
        assert!(
            rendered
                .iter()
                .any(|line| { line.contains("https://github.com/owner/repo/actions/runs/1") })
        );
    }
}
