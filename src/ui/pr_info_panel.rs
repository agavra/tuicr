use ratatui::{
    style::Style,
    text::{Line, Span},
};

use crate::app::App;
use crate::forge::traits::{PullRequestCheckStatus, PullRequestInfo, PullRequestReviewStatus};
use crate::ui::diff_view::{cursor_indicator_spaced, HEADER_RULE};
use crate::ui::styles;

/// Rendered line count of the PR description block at the top of the diff view.
pub fn pr_info_render_height(app: &App) -> usize {
    let Some(info) = app.pr_info.as_ref() else {
        return 0;
    };
    let mut height = if app.is_single_file_view { 0 } else { 1 };
    height += build_pr_info_lines(info, app.diff_state.viewport_width.max(1)).len();
    height
}

pub fn is_cursor_in_pr_info(app: &App) -> bool {
    pr_info_render_height(app) > 0 && app.diff_state.cursor_line < pr_info_render_height(app)
}

/// Append PR metadata lines to the main diff scroll buffer, before review comments.
pub fn append_pr_info_section(
    app: &App,
    lines: &mut Vec<Line<'static>>,
    line_idx: &mut usize,
    current_line_idx: usize,
    content_width: usize,
) {
    let Some(info) = app.pr_info.as_ref() else {
        return;
    };

    if !app.is_single_file_view {
        let general_indicator = cursor_indicator_spaced(*line_idx, current_line_idx);
        lines.push(Line::from(vec![
            Span::styled(
                general_indicator,
                styles::current_line_indicator_style(&app.theme),
            ),
            Span::styled(
                "═══ PR Description ",
                styles::file_header_style(&app.theme),
            ),
            Span::styled(HEADER_RULE, styles::file_header_style(&app.theme)),
        ]));
        *line_idx += 1;
    }

    for mut pr_line in build_pr_info_lines(info, content_width) {
        let indicator = cursor_indicator_spaced(*line_idx, current_line_idx);
        pr_line.spans.insert(
            0,
            Span::styled(
                indicator,
                styles::current_line_indicator_style(&app.theme),
            ),
        );
        lines.push(pr_line);
        *line_idx += 1;
    }
}

pub fn build_pr_info_lines(info: &PullRequestInfo, width: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let content_width = width.max(1);

    let details = &info.details;
    push_section(
        &mut lines,
        format!("#{} · {}", details.number, details.title),
        content_width,
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

    let head_short = details
        .head_sha
        .chars()
        .take(8)
        .collect::<String>();
    let mut branch_line = format!(
        "{} → {} · {}",
        details.head_ref_name, details.base_ref_name, head_short
    );
    if let Some(author) = &details.author {
        branch_line.push_str(&format!(" · @{author}"));
    }
    if let Some(updated) = details.updated_at {
        branch_line.push_str(&format!(" · updated {}", updated.format("%Y-%m-%d %H:%M UTC")));
    }
    push_wrapped_line(&mut lines, branch_line, content_width);

    if !info.requested_reviewers.is_empty() {
        push_section(
            &mut lines,
            format!("Requested: {}", format_users(&info.requested_reviewers)),
            content_width,
        );
    }

    let approved = reviews_by_state(&info.latest_reviews, "APPROVED");
    let changes = reviews_by_state(&info.latest_reviews, "CHANGES_REQUESTED");
    let commented = reviews_by_state(&info.latest_reviews, "COMMENTED");
    if !approved.is_empty() {
        push_section(
            &mut lines,
            format!("Approved: {}", format_users(&approved)),
            content_width,
        );
    }
    if !changes.is_empty() {
        push_section(
            &mut lines,
            format!("Changes requested: {}", format_users(&changes)),
            content_width,
        );
    }
    if !commented.is_empty() {
        push_section(
            &mut lines,
            format!("Commented: {}", format_users(&commented)),
            content_width,
        );
    }

    if !info.checks.is_empty() {
        push_blank(&mut lines);
        push_wrapped_line(&mut lines, "Checks".to_string(), content_width);
        for check in &info.checks {
            push_wrapped_line(
                &mut lines,
                format!("{} {}", check_glyph(check), format_check(check)),
                content_width,
            );
        }
    }

    push_blank(&mut lines);
    push_wrapped_line(&mut lines, "Description".to_string(), content_width);
    let body = if details.body.trim().is_empty() {
        "(no description)".to_string()
    } else {
        details.body.clone()
    };
    for paragraph in body.lines() {
        push_wrapped_line(&mut lines, paragraph.to_string(), content_width);
    }

    lines
}

fn push_section(lines: &mut Vec<Line<'static>>, text: String, width: usize) {
    push_blank(lines);
    push_wrapped_line(lines, text, width);
}

fn push_blank(lines: &mut Vec<Line<'static>>) {
    if lines.last().is_some_and(|line| !line.spans.is_empty()) {
        lines.push(Line::default());
    }
}

fn push_wrapped_line(lines: &mut Vec<Line<'static>>, text: String, width: usize) {
    let style = Style::default();
    for chunk in wrap_text(&text, width) {
        lines.push(Line::from(Span::styled(chunk, style)));
    }
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

fn format_check(check: &PullRequestCheckStatus) -> String {
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
            }],
        }
    }

    #[test]
    fn should_build_non_empty_pr_info_lines() {
        let lines = build_pr_info_lines(&sample_info(), 80);
        assert!(lines.len() > 5);
    }
}
