use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph},
};

use crate::app::{App, DiffSource, InputMode, Message, MessageType};
use crate::theme::Theme;
use crate::ui::styles;

pub fn build_message_span(message: Option<&Message>, theme: &Theme) -> (Span<'static>, usize) {
    if let Some(msg) = message {
        let (fg, bg) = match msg.message_type {
            MessageType::Info => (theme.message_info_fg, theme.message_info_bg),
            MessageType::Warning => (theme.message_warning_fg, theme.message_warning_bg),
            MessageType::Error => (theme.message_error_fg, theme.message_error_bg),
        };
        let content = format!(" {} ", msg.content);
        let width = content.len();
        (
            Span::styled(
                content,
                Style::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD),
            ),
            width,
        )
    } else {
        (Span::raw(""), 0)
    }
}

pub fn build_right_aligned_spans<'a>(
    mut left_spans: Vec<Span<'a>>,
    message_span: Span<'a>,
    message_width: usize,
    total_width: usize,
) -> Vec<Span<'a>> {
    let left_width: usize = left_spans.iter().map(|s| s.content.len()).sum();
    let padding_width = total_width.saturating_sub(left_width + message_width);
    let padding = Span::raw(" ".repeat(padding_width));

    left_spans.push(padding);
    if message_width > 0 {
        left_spans.push(message_span);
    }
    left_spans
}

pub fn render_header(frame: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;
    let vcs_type = &app.vcs_info.vcs_type;
    let branch = app.vcs_info.branch_name.as_deref().unwrap_or("detached");

    let in_pr_mode = matches!(app.diff_source, DiffSource::PullRequest(_));

    let title = if in_pr_mode {
        " tuicr - PR Review ".to_string()
    } else {
        " tuicr - Code Review ".to_string()
    };
    let vcs_info = if in_pr_mode {
        // PR mode header is anchored on the PR identity, not the local VCS.
        String::new()
    } else {
        format!("[{vcs_type}:{branch}] ")
    };

    // Show diff source info
    let source_info = match &app.diff_source {
        DiffSource::WorkingTree => String::new(),
        DiffSource::Staged => "[staged] ".to_string(),
        DiffSource::Unstaged => "[unstaged] ".to_string(),
        DiffSource::StagedAndUnstaged => "[staged + unstaged] ".to_string(),
        DiffSource::CommitRange(commits) => {
            if commits.len() == 1 {
                format!("[commit {}] ", &commits[0][..7.min(commits[0].len())])
            } else {
                match app.commit_selection_range {
                    Some((start, end)) if end - start + 1 < app.review_commits.len() => {
                        format!(
                            "[{}/{} commits] ",
                            end - start + 1,
                            app.review_commits.len()
                        )
                    }
                    _ => format!("[{} commits] ", commits.len()),
                }
            }
        }
        DiffSource::StagedUnstagedAndCommits(commits) => {
            if commits.len() == 1 {
                format!(
                    "[staged + unstaged + commit {}] ",
                    &commits[0][..7.min(commits[0].len())]
                )
            } else {
                format!("[staged + unstaged + {} commits] ", commits.len())
            }
        }
        DiffSource::PullRequest(pr) => {
            let slug = pr.key.repository.display_name();
            let trimmed_title = if pr.title.len() > 60 {
                format!("{}…", &pr.title[..59])
            } else {
                pr.title.clone()
            };
            let mut info = format!(
                "[PR mode · {slug}#{number} · {trimmed_title}] ",
                number = pr.key.number,
            );
            if let Some(state) = pr.read_only_reason() {
                info.push_str(&format!("[{state} — read only] "));
            }
            info
        }
    };

    let progress = format!("{}/{} reviewed ", app.reviewed_count(), app.file_count());

    let title_span = Span::styled(title, styles::header_style(theme));
    let vcs_span = Span::styled(vcs_info, Style::default().fg(theme.fg_secondary));
    let source_span = Span::styled(source_info, Style::default().fg(theme.diff_hunk_header));
    let progress_span = Span::styled(
        progress,
        if app.reviewed_count() == app.file_count() {
            styles::reviewed_style(theme)
        } else {
            styles::pending_style(theme)
        },
    );

    let (update_span, update_width) = if let Some(ref info) = app.update_info {
        if info.update_available {
            let text = format!(" v{} available ", info.latest_version);
            let width = text.len();
            (
                Span::styled(
                    text,
                    Style::default()
                        .fg(theme.update_badge_fg)
                        .bg(theme.update_badge_bg)
                        .add_modifier(Modifier::BOLD),
                ),
                width,
            )
        } else if info.is_ahead {
            let text = format!(" unreleased v{} ", info.current_version);
            let width = text.len();
            (
                Span::styled(
                    text,
                    Style::default()
                        .fg(theme.update_badge_fg)
                        .bg(theme.update_badge_bg)
                        .add_modifier(Modifier::BOLD),
                ),
                width,
            )
        } else {
            (Span::raw(""), 0)
        }
    } else {
        (Span::raw(""), 0)
    };

    let left_spans = vec![title_span, vcs_span, source_span, progress_span];
    let left_width: usize = left_spans.iter().map(|s| s.content.len()).sum();
    let total_width = area.width as usize;
    let padding_width = total_width.saturating_sub(left_width + update_width);

    let mut spans = left_spans;
    spans.push(Span::raw(" ".repeat(padding_width)));
    if update_width > 0 {
        spans.push(update_span);
    }

    let line = Line::from(spans);

    let header = Paragraph::new(line)
        .style(styles::status_bar_style(theme))
        .block(Block::default());

    frame.render_widget(header, area);
}

pub fn render_status_bar(frame: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;

    // In command/search mode, show the input on the left (vim-style)
    let left_spans = if matches!(app.input_mode, InputMode::Command | InputMode::Search) {
        let prefix = if app.input_mode == InputMode::Command {
            ":"
        } else {
            "/"
        };
        let buffer = if app.input_mode == InputMode::Command {
            &app.command_buffer
        } else {
            &app.search_buffer
        };
        let command_text = format!("{prefix}{buffer}");
        vec![Span::styled(
            command_text,
            Style::default().fg(theme.fg_primary),
        )]
    } else {
        let mode_str = match app.input_mode {
            InputMode::Normal => {
                if let Some(count) = app.pending_count {
                    format!(" NORMAL {count} ")
                } else {
                    " NORMAL ".to_string()
                }
            }
            InputMode::Command => " COMMAND ".to_string(),
            InputMode::Search => " SEARCH ".to_string(),
            InputMode::Comment => " COMMENT ".to_string(),
            InputMode::Help => " HELP ".to_string(),
            InputMode::Confirm => " CONFIRM ".to_string(),
            InputMode::CommitSelect => " SELECT ".to_string(),
            InputMode::VisualSelect => {
                if let Some((range, _)) = app.visual_selection_line_range() {
                    if range.is_single() {
                        format!(" VISUAL L{} ", range.start)
                    } else {
                        format!(" VISUAL L{}-L{} ", range.start, range.end)
                    }
                } else {
                    " VISUAL ".to_string()
                }
            }
        };

        let mode_span = Span::styled(mode_str, styles::mode_style(theme));

        let hints = if app.message.is_some() {
            ""
        } else {
            match app.input_mode {
                InputMode::Normal => {
                    " j/k:scroll  {/}:file  r:reviewed  c:comment  ;c:review  V:visual  /:search  ?:help  :q:quit "
                }
                InputMode::Command => " Enter:execute  Esc:cancel ",
                InputMode::Search => " Enter:search  Esc:cancel ",
                InputMode::Comment => " Ctrl-S:save  Esc:cancel ",
                InputMode::Help => " q/?/Esc:close ",
                InputMode::Confirm => " y:yes  n:no ",
                InputMode::CommitSelect => {
                    " j/k:navigate  Space:select  Enter:confirm  Esc:back  q:quit "
                }
                InputMode::VisualSelect => " j/k:extend  c/Enter:comment  y:yank  Esc/V:cancel ",
            }
        };
        let hints_span = Span::styled(hints, Style::default().fg(theme.fg_secondary));

        let dirty_indicator = if app.dirty {
            Span::styled(" [modified] ", Style::default().fg(theme.pending))
        } else {
            Span::raw("")
        };

        // Inline hint while remote review threads are still loading in PR
        // mode. The diff is already visible; this just tells the user that
        // existing GitHub discussions are still on the way.
        let remote_loading = if app.forge_review_threads_loading {
            Span::styled(
                " Loading remote comments… ",
                Style::default().fg(theme.fg_dim),
            )
        } else {
            Span::raw("")
        };

        vec![mode_span, hints_span, dirty_indicator, remote_loading]
    };

    // Right-aligned slot: while `:e` is in flight we put the spinner
    // there instead of any pending message, mirroring how messages are
    // placed. The user sees one prominent right-aligned indicator at a
    // time. After the reload lands, the success message takes the slot
    // back.
    let (right_span, right_width) = if let Some(reload) = app.pr_reload_state.as_ref() {
        let glyph = crate::ui::selector::pr_open_spinner_glyph(reload.started_at.elapsed());
        let content = format!(" {glyph} Reloading PR… ");
        let width = content.len();
        (
            Span::styled(
                content,
                Style::default()
                    .fg(theme.message_info_fg)
                    .bg(theme.message_info_bg)
                    .add_modifier(Modifier::BOLD),
            ),
            width,
        )
    } else {
        build_message_span(app.message.as_ref(), theme)
    };
    let total_width = area.width as usize;
    let spans = build_right_aligned_spans(left_spans, right_span, right_width, total_width);

    let line = Line::from(spans);

    let status = Paragraph::new(line)
        .style(styles::status_bar_style(theme))
        .block(Block::default());

    frame.render_widget(status, area);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_message(message_type: MessageType) -> Message {
        Message {
            content: "hello".to_string(),
            message_type,
            expires_at: None,
        }
    }

    #[test]
    fn should_style_info_message_using_theme_fields() {
        let theme = Theme::dark();
        let (span, width) = build_message_span(Some(&test_message(MessageType::Info)), &theme);
        assert_eq!(span.style.fg, Some(theme.message_info_fg));
        assert_eq!(span.style.bg, Some(theme.message_info_bg));
        assert_eq!(width, " hello ".len());
    }

    #[test]
    fn should_return_empty_span_when_message_is_none() {
        let theme = Theme::dark();
        let (span, width) = build_message_span(None, &theme);
        assert_eq!(span.content.as_ref(), "");
        assert_eq!(width, 0);
    }

    #[test]
    fn should_style_warning_message_using_theme_fields() {
        let theme = Theme::dark();
        let (span, _) = build_message_span(Some(&test_message(MessageType::Warning)), &theme);
        assert_eq!(span.style.fg, Some(theme.message_warning_fg));
        assert_eq!(span.style.bg, Some(theme.message_warning_bg));
    }

    #[test]
    fn should_style_error_message_using_theme_fields() {
        let theme = Theme::dark();
        let (span, _) = build_message_span(Some(&test_message(MessageType::Error)), &theme);
        assert_eq!(span.style.fg, Some(theme.message_error_fg));
        assert_eq!(span.style.bg, Some(theme.message_error_bg));
    }
}

#[cfg(test)]
mod pr_header_snapshot_tests {
    //! Render-snapshot coverage for the status bar header in PR mode.
    //! Drives the full `render_header` against ratatui's `TestBackend`
    //! and asserts on the produced character grid.

    use crate::app::{App, DiffSource, InputMode, PullRequestDiffSource};
    use crate::error::Result as TuicrResult;
    use crate::error::TuicrError;
    use crate::forge::traits::{ForgeRepository, PrSessionKey};
    use crate::model::{DiffFile, DiffLine, FileStatus, ReviewSession, SessionDiffSource};
    use crate::syntax::SyntaxHighlighter;
    use crate::theme::Theme;
    use crate::vcs::traits::{VcsBackend, VcsInfo, VcsType};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use std::path::{Path, PathBuf};

    struct NoopVcs {
        info: VcsInfo,
    }
    impl VcsBackend for NoopVcs {
        fn info(&self) -> &VcsInfo {
            &self.info
        }
        fn get_working_tree_diff(
            &self,
            _highlighter: &SyntaxHighlighter,
        ) -> TuicrResult<Vec<DiffFile>> {
            Err(TuicrError::NoChanges)
        }
        fn fetch_context_lines(
            &self,
            _file_path: &Path,
            _file_status: FileStatus,
            _start_line: u32,
            _end_line: u32,
        ) -> TuicrResult<Vec<DiffLine>> {
            Ok(Vec::new())
        }
    }

    fn pr_source(closed: bool, merged: bool) -> PullRequestDiffSource {
        PullRequestDiffSource {
            key: PrSessionKey::new(
                ForgeRepository::github("github.com", "agavra", "tuicr"),
                125,
                "abcdef0123456789".to_string(),
            ),
            base_sha: "1234567890abcdef".to_string(),
            title: "Add forge-backed PR review".to_string(),
            url: "https://github.com/agavra/tuicr/pull/125".to_string(),
            head_ref_name: "reviews".to_string(),
            base_ref_name: "main".to_string(),
            state: if closed { "CLOSED" } else { "OPEN" }.to_string(),
            closed,
            merged,
        }
    }

    fn build_pr_app(pr: PullRequestDiffSource) -> App {
        let vcs_info = VcsInfo {
            root_path: PathBuf::from("forge:github.com/agavra/tuicr"),
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
        App::build(
            Box::new(NoopVcs {
                info: vcs_info.clone(),
            }),
            vcs_info,
            Theme::dark(),
            None,
            false,
            Vec::new(),
            session,
            DiffSource::PullRequest(Box::new(pr)),
            InputMode::Normal,
            Vec::new(),
            None,
        )
        .expect("build pr app")
    }

    fn draw_header(app: &App) -> Buffer {
        let backend = TestBackend::new(140, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                super::render_header(frame, app, area);
            })
            .expect("draw frame");
        terminal.backend().buffer().clone()
    }

    fn row_text(buffer: &Buffer, y: u16) -> String {
        (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol().to_string())
            .collect()
    }

    #[test]
    fn should_render_pr_mode_header_with_slug_number_and_title() {
        // given a PR-mode app for agavra/tuicr#125
        let app = build_pr_app(pr_source(false, false));
        // when
        let buffer = draw_header(&app);
        // then
        let line = row_text(&buffer, 0);
        assert!(line.contains("tuicr - PR Review"), "got: {line:?}");
        assert!(line.contains("PR mode"), "got: {line:?}");
        assert!(line.contains("agavra/tuicr#125"), "got: {line:?}");
        assert!(line.contains("Add forge-backed PR review"), "got: {line:?}");
    }

    #[test]
    fn should_render_read_only_badge_for_closed_pr() {
        // given a closed PR
        let app = build_pr_app(pr_source(true, false));
        // when
        let buffer = draw_header(&app);
        // then
        let line = row_text(&buffer, 0);
        assert!(line.contains("closed — read only"), "got: {line:?}");
    }

    #[test]
    fn should_render_read_only_badge_for_merged_pr() {
        // given a merged PR
        let app = build_pr_app(pr_source(false, true));
        // when
        let buffer = draw_header(&app);
        // then
        let line = row_text(&buffer, 0);
        assert!(line.contains("merged — read only"), "got: {line:?}");
    }

    #[test]
    fn should_omit_read_only_badge_for_open_pr() {
        // given
        let app = build_pr_app(pr_source(false, false));
        // when
        let buffer = draw_header(&app);
        // then
        let line = row_text(&buffer, 0);
        assert!(!line.contains("read only"), "got: {line:?}");
    }
}
