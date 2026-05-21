//! Modal renderers for `:submit*`: the unmappable-comment resolver and the
//! final confirmation. Driven off `App::submit_state`.

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Flex, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::app::{App, SUBMIT_PICKER_EVENTS};
use crate::forge::submit::{ResolverAction, SubmitEvent, UnmappableItem};
use crate::ui::styles;

/// Render the bare-`:submit` action picker. Lists the available review
/// events plus a Cancel row; the user picks one with j/k + Enter (or
/// Esc to cancel).
pub fn render_submit_action_picker(frame: &mut Frame, app: &App) {
    let theme = &app.theme;
    let area = centered_rect(40, 50, modal_anchor(app, frame.area()));
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" Submit review to GitHub? ")
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .style(styles::popup_style(theme))
        .border_style(styles::border_style(theme, true));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(""));
    for (i, (label, _)) in SUBMIT_PICKER_EVENTS.iter().enumerate() {
        let style = if i == app.submit_picker_cursor {
            Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
        } else {
            Style::default()
        };
        let cursor = if i == app.submit_picker_cursor {
            ">"
        } else {
            " "
        };
        lines.push(Line::from(Span::styled(format!("{cursor} {label}"), style)));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Enter: submit   Esc: cancel",
        Style::default().fg(theme.fg_secondary),
    )));

    let paragraph = Paragraph::new(lines).style(styles::popup_style(theme));
    frame.render_widget(paragraph, inner);
}

/// Render the modal listing comments the mapper could not place inline,
/// with per-row toggle between "Move to summary" / "Omit". Centered overlay
/// with a 70%-wide / 50%-tall area to give the rows breathing room.
pub fn render_submit_resolver(frame: &mut Frame, app: &App) {
    let theme = &app.theme;
    let Some(state) = app.submit_state.as_ref() else {
        return;
    };
    let area = centered_rect(70, 50, modal_anchor(app, frame.area()));

    frame.render_widget(Clear, area);

    let n = state.unmappable.len();
    let title = format!(
        " {n} comment{s} cannot be posted inline ",
        s = if n == 1 { "" } else { "s" }
    );

    let block = Block::default()
        .title(title)
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .style(styles::popup_style(theme))
        .border_style(styles::border_style(theme, true));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::with_capacity(n + 4);
    lines.push(Line::from(""));
    for (i, (item, action)) in state
        .unmappable
        .iter()
        .zip(state.resolver_choices.iter())
        .enumerate()
    {
        let cursor = if i == state.resolver_cursor { ">" } else { " " };
        let action_label = match action {
            ResolverAction::MoveToSummary => "[x] Move to summary",
            ResolverAction::Omit => "[ ] Omit             ",
        };
        let style = if i == state.resolver_cursor {
            Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(
            format!("{cursor} {action_label}  {row}", row = describe_row(item)),
            style,
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Enter: toggle action   s: submit   Esc: cancel",
        Style::default().fg(theme.fg_secondary),
    )));

    let paragraph = Paragraph::new(lines)
        .style(styles::popup_style(theme))
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, inner);
}

/// Render the final confirmation modal before submit. Shows the counts and
/// surface the stale-head warning when the open-time head and the latest
/// known PR head disagree.
pub fn render_submit_confirm(frame: &mut Frame, app: &App) {
    let theme = &app.theme;
    let Some(state) = app.submit_state.as_ref() else {
        return;
    };
    let area = centered_rect(60, 70, modal_anchor(app, frame.area()));

    frame.render_widget(Clear, area);

    let title = match state.event {
        SubmitEvent::Draft => " Push pending GitHub review? ".to_string(),
        _ => " Submit review to GitHub? ".to_string(),
    };

    let block = Block::default()
        .title(title)
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .style(styles::popup_style(theme))
        .border_style(styles::border_style(theme, true));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let moved_count = state
        .resolver_choices
        .iter()
        .filter(|a| **a == ResolverAction::MoveToSummary)
        .count();
    let omit_count = state.resolver_choices.len() - moved_count;
    let body_summary = body_summary_label(app, moved_count);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(""));
    if !matches!(state.event, SubmitEvent::Draft) {
        lines.push(Line::from(format!(
            "Event: {label}",
            label = state.event.human_label()
        )));
    }
    lines.push(Line::from(format!("Inline: {n}", n = state.mappable.len())));
    lines.push(Line::from(format!("Moved to summary: {moved_count}")));
    lines.push(Line::from(format!("Omitted: {omit_count}")));
    lines.push(Line::from(format!("Body: {body_summary}")));
    lines.push(Line::from(format!(
        "Head: {sha}",
        sha = short_sha(&state.commit_id)
    )));

    let stale = app.submit_head_is_stale();
    if stale {
        lines.push(Line::from(""));
        let current = app
            .current_pr_head
            .as_deref()
            .map(short_sha)
            .unwrap_or_else(|| "?".to_string());
        lines.push(Line::from(Span::styled(
            format!("Current PR head: {current}"),
            Style::default().fg(theme.pending),
        )));
        lines.push(Line::from(Span::styled(
            "Warning: this review targets an older PR revision.",
            Style::default().fg(theme.pending),
        )));
        lines.push(Line::from(Span::styled(
            "Some comments may appear outdated on GitHub.",
            Style::default().fg(theme.pending),
        )));
    }

    if matches!(state.event, SubmitEvent::Draft) {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "This will create a pending GitHub review.",
            Style::default().fg(theme.fg_secondary),
        )));
        lines.push(Line::from(Span::styled(
            "It will not publish until you finish it in GitHub.",
            Style::default().fg(theme.fg_secondary),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(prompt_spans(stale, state.event)));

    let paragraph = Paragraph::new(lines)
        .style(styles::popup_style(theme))
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, inner);
}

fn describe_row(item: &UnmappableItem) -> String {
    let kind = item.comment.comment_type.as_str();
    let path = item.file.display();
    let preview: String = item.comment.content.chars().take(40).collect();
    let ellipsis = if item.comment.content.chars().count() > 40 {
        "…"
    } else {
        ""
    };
    let reason = item.reason.human_label();
    format!("{path} [{kind}] {preview}{ellipsis}  ({reason})")
}

fn body_summary_label(app: &App, moved_count: usize) -> String {
    let review_level = app.session.review_comments.len();
    match (review_level, moved_count) {
        (0, 0) => "empty".to_string(),
        (n, 0) => format!("{n} review comment{s}", s = plural(n)),
        (0, m) => format!("{m} unplaced comment{s}", s = plural(m)),
        (n, m) => format!("{n} review comment{sn} + {m} unplaced", sn = plural(n)),
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

fn short_sha(sha: &str) -> String {
    sha.chars().take(7).collect()
}

fn prompt_spans(stale: bool, event: SubmitEvent) -> Vec<Span<'static>> {
    let primary = match event {
        SubmitEvent::Draft => "push draft",
        _ => "submit",
    };
    let mut spans = vec![
        Span::styled("[y] ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(format!("{primary}    ")),
        Span::styled("[n] ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("cancel"),
    ];
    if stale {
        spans.push(Span::raw("    "));
        spans.push(Span::styled(
            "[r] ",
            Style::default().add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw("reload"));
    }
    spans
}

/// The submit modals should center over the diff pane when one is visible
/// (file list off to the side would otherwise tug the visual centre left).
/// Falls back to the full frame when no diff area is laid out yet.
fn modal_anchor(app: &App, fallback: Rect) -> Rect {
    app.diff_area.unwrap_or(fallback)
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::vertical([Constraint::Percentage(percent_y)]).flex(Flex::Center);
    let horizontal = Layout::horizontal([Constraint::Percentage(percent_x)]).flex(Flex::Center);
    let [area] = vertical.areas(area);
    let [area] = horizontal.areas(area);
    area
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, DiffSource, InputMode, PullRequestDiffSource, SubmitState};
    use crate::error::Result as TuicrResult;
    use crate::error::TuicrError;
    use crate::forge::submit::{GhSide, InlineComment};
    use crate::forge::traits::{ForgeRepository, PrSessionKey};
    use crate::model::ReviewSession;
    use crate::model::comment::{Comment, CommentType};
    use crate::model::diff_types::FileStatus;
    use crate::model::{DiffFile, DiffLine, SessionDiffSource};
    use crate::syntax::SyntaxHighlighter;
    use crate::theme::Theme;
    use crate::vcs::traits::{VcsBackend, VcsChangeStatus, VcsInfo, VcsType};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use std::path::{Path, PathBuf};

    struct SnapshotVcs {
        info: VcsInfo,
    }

    impl VcsBackend for SnapshotVcs {
        fn info(&self) -> &VcsInfo {
            &self.info
        }
        fn get_working_tree_diff(&self, _h: &SyntaxHighlighter) -> TuicrResult<Vec<DiffFile>> {
            Err(TuicrError::NoChanges)
        }
        fn fetch_context_lines(
            &self,
            _p: &Path,
            _s: FileStatus,
            _ref_commit: Option<&str>,
            _start: u32,
            _end: u32,
        ) -> TuicrResult<Vec<DiffLine>> {
            Ok(Vec::new())
        }
        fn get_change_status(&self) -> TuicrResult<VcsChangeStatus> {
            Ok(VcsChangeStatus {
                staged: false,
                unstaged: false,
            })
        }
        fn file_line_count(
            &self,
            _p: &Path,
            _s: FileStatus,
            _ref_commit: Option<&str>,
        ) -> TuicrResult<u32> {
            Ok(0)
        }
    }

    fn make_pr_app() -> App {
        let vcs_info = VcsInfo {
            root_path: PathBuf::from("/tmp"),
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
        let pr_source = PullRequestDiffSource {
            key: PrSessionKey::new(
                ForgeRepository::github("github.com", "agavra", "tuicr"),
                125,
                "abcdef0123".to_string(),
            ),
            base_sha: "0000".to_string(),
            title: "test".to_string(),
            url: "https://example".to_string(),
            head_ref_name: "feat".to_string(),
            base_ref_name: "main".to_string(),
            state: "OPEN".to_string(),
            closed: false,
            merged: false,
        };
        let mut app = App::build(
            Box::new(SnapshotVcs {
                info: vcs_info.clone(),
            }),
            vcs_info,
            Theme::dark(),
            None,
            false,
            Vec::new(),
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

    fn inline(line: u32) -> InlineComment {
        InlineComment {
            path: PathBuf::from("src/lib.rs"),
            line,
            side: GhSide::Right,
            start_line: None,
            start_side: None,
            body: "x".to_string(),
            comment_id: "test-comment-id".to_string(),
        }
    }

    fn unmappable_item(
        file: &str,
        ty: CommentType,
        body: &str,
        reason: crate::forge::submit::UnmappableReason,
    ) -> UnmappableItem {
        UnmappableItem {
            comment: Comment::new(body.to_string(), ty, None),
            file: PathBuf::from(file),
            reason,
        }
    }

    fn buffer_text(buffer: &Buffer) -> String {
        let mut out = String::new();
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                out.push_str(buffer[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    fn draw_resolver(app: &App) -> Buffer {
        let backend = TestBackend::new(120, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_submit_resolver(frame, app))
            .expect("draw");
        terminal.backend().buffer().clone()
    }

    fn draw_confirm(app: &App) -> Buffer {
        let backend = TestBackend::new(120, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_submit_confirm(frame, app))
            .expect("draw");
        terminal.backend().buffer().clone()
    }

    fn draw_picker(app: &App) -> Buffer {
        let backend = TestBackend::new(120, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_submit_action_picker(frame, app))
            .expect("draw");
        terminal.backend().buffer().clone()
    }

    #[test]
    fn picker_renders_all_events_and_cursor_marker() {
        let mut app = make_pr_app();
        app.submit_picker_cursor = 0;
        let buffer = draw_picker(&app);
        let text = buffer_text(&buffer);
        assert!(text.contains("Submit review to GitHub?"));
        assert!(text.contains("Comment"));
        assert!(text.contains("Approve"));
        assert!(text.contains("Request changes"));
        assert!(text.contains("Draft"));
        assert!(text.contains("Enter: submit"));
        assert!(text.contains("Esc: cancel"));
        assert!(text.contains("> Comment"));
    }

    #[test]
    fn picker_cursor_moves_with_marker() {
        let mut app = make_pr_app();
        app.submit_picker_cursor = 2;
        let buffer = draw_picker(&app);
        let text = buffer_text(&buffer);
        assert!(text.contains("> Request changes"));
        assert!(!text.contains("> Comment"));
    }

    #[test]
    fn resolver_renders_each_row_with_cursor_marker() {
        use crate::forge::submit::UnmappableReason;
        let mut app = make_pr_app();
        app.submit_state = Some(SubmitState {
            event: SubmitEvent::Comment,
            mappable: Vec::new(),
            unmappable: vec![
                unmappable_item(
                    "src/lib.rs",
                    CommentType::Issue,
                    "needs fixing",
                    UnmappableReason::FileLevelNoAnchor,
                ),
                unmappable_item(
                    "img.png",
                    CommentType::Note,
                    "binary art",
                    UnmappableReason::BinaryFile,
                ),
            ],
            resolver_choices: vec![ResolverAction::MoveToSummary, ResolverAction::MoveToSummary],
            resolver_cursor: 0,
            commit_id: "abcdef0123".to_string(),
            skip_confirm: false,
        });
        let buffer = draw_resolver(&app);
        let text = buffer_text(&buffer);
        assert!(
            text.contains("2 comments cannot be posted inline"),
            "title: {text}"
        );
        assert!(text.contains("src/lib.rs"));
        assert!(text.contains("img.png"));
        assert!(text.contains("Enter: toggle action"));
        // Cursor marker on row 0
        assert!(text.contains("> [x] Move to summary"));
    }

    #[test]
    fn resolver_marks_omit_rows_with_empty_brackets() {
        use crate::forge::submit::UnmappableReason;
        let mut app = make_pr_app();
        app.submit_state = Some(SubmitState {
            event: SubmitEvent::Comment,
            mappable: Vec::new(),
            unmappable: vec![unmappable_item(
                "x.rs",
                CommentType::Note,
                "n",
                UnmappableReason::FileLevelNoAnchor,
            )],
            resolver_choices: vec![ResolverAction::Omit],
            resolver_cursor: 0,
            commit_id: "abcdef0123".to_string(),
            skip_confirm: false,
        });
        let buffer = draw_resolver(&app);
        let text = buffer_text(&buffer);
        assert!(text.contains("[ ] Omit"), "omit marker: {text}");
    }

    #[test]
    fn confirm_renders_event_and_counts_for_comment_submission() {
        let mut app = make_pr_app();
        app.submit_state = Some(SubmitState {
            event: SubmitEvent::Comment,
            mappable: vec![inline(11), inline(12)],
            unmappable: Vec::new(),
            resolver_choices: Vec::new(),
            resolver_cursor: 0,
            commit_id: "abcdef0123".to_string(),
            skip_confirm: false,
        });
        let buffer = draw_confirm(&app);
        let text = buffer_text(&buffer);
        assert!(text.contains("Submit review to GitHub?"), "title: {text}");
        assert!(text.contains("Event: Comment"), "event line: {text}");
        assert!(text.contains("Inline: 2"));
        assert!(text.contains("Head: abcdef0"));
        assert!(text.contains("[y]"));
        assert!(text.contains("[n]"));
        // No stale-head warning when current_pr_head matches
        assert!(!text.contains("Warning: this review targets"));
    }

    #[test]
    fn confirm_shows_stale_head_warning_with_reload_option() {
        let mut app = make_pr_app();
        app.current_pr_head = Some("ffff5678".to_string());
        app.submit_state = Some(SubmitState {
            event: SubmitEvent::RequestChanges,
            mappable: vec![inline(11)],
            unmappable: Vec::new(),
            resolver_choices: Vec::new(),
            resolver_cursor: 0,
            commit_id: "abcdef0123".to_string(),
            skip_confirm: false,
        });
        let buffer = draw_confirm(&app);
        let text = buffer_text(&buffer);
        assert!(text.contains("Event: Request changes"));
        assert!(text.contains("Current PR head: ffff567"));
        assert!(text.contains("Warning: this review targets"));
        assert!(text.contains("[r]"));
    }

    #[test]
    fn confirm_shows_pending_review_title_and_no_event_line_for_draft() {
        let mut app = make_pr_app();
        app.submit_state = Some(SubmitState {
            event: SubmitEvent::Draft,
            mappable: vec![inline(11)],
            unmappable: Vec::new(),
            resolver_choices: Vec::new(),
            resolver_cursor: 0,
            commit_id: "abcdef0123".to_string(),
            skip_confirm: false,
        });
        let buffer = draw_confirm(&app);
        let text = buffer_text(&buffer);
        assert!(text.contains("Push pending GitHub review?"));
        // The "Event:" prefix should not appear for draft submissions
        assert!(!text.contains("Event: "), "draft body: {text}");
        assert!(text.contains("[y]"));
        assert!(text.contains("push draft"));
    }

    #[test]
    fn confirm_reflects_moved_to_summary_counts() {
        use crate::forge::submit::UnmappableReason;
        let mut app = make_pr_app();
        app.submit_state = Some(SubmitState {
            event: SubmitEvent::Comment,
            mappable: vec![inline(11)],
            unmappable: vec![
                unmappable_item(
                    "a.rs",
                    CommentType::Note,
                    "x",
                    UnmappableReason::FileLevelNoAnchor,
                ),
                unmappable_item(
                    "b.rs",
                    CommentType::Note,
                    "y",
                    UnmappableReason::FileLevelNoAnchor,
                ),
            ],
            resolver_choices: vec![ResolverAction::MoveToSummary, ResolverAction::Omit],
            resolver_cursor: 0,
            commit_id: "abcdef0123".to_string(),
            skip_confirm: false,
        });
        let buffer = draw_confirm(&app);
        let text = buffer_text(&buffer);
        assert!(text.contains("Moved to summary: 1"), "counts: {text}");
        assert!(text.contains("Omitted: 1"));
    }
}
