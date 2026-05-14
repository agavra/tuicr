use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::app::{App, TargetTab};
use crate::forge::selector::{PrTabStatus, PrTabView};
use crate::ui::status_bar;
use crate::ui::styles;
use crate::ui::text_utils::truncate_str;

pub(super) fn render_commit_select(frame: &mut Frame, app: &mut App) {
    let area = frame.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Header
            Constraint::Length(1), // Tab strip
            Constraint::Min(0),    // Active tab body
            Constraint::Length(1), // Footer hints
        ])
        .split(area);

    // Header
    let header = Paragraph::new(" Select a review target ")
        .style(styles::header_style(&app.theme))
        .block(Block::default().style(styles::panel_style(&app.theme)));
    frame.render_widget(header, chunks[0]);

    render_target_tab_strip(frame, app, chunks[1]);

    match app.target_tab {
        TargetTab::Local => render_local_target_tab(frame, app, chunks[2]),
        TargetTab::PullRequests => render_pull_requests_tab(frame, app, chunks[2]),
    }

    render_target_selector_footer(frame, app, chunks[3]);
}

fn render_target_tab_strip(frame: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;
    let active = app.target_tab;
    let active_style = Style::default()
        .fg(theme.fg_primary)
        .add_modifier(Modifier::BOLD | Modifier::REVERSED);
    let bracket_style = Style::default().fg(theme.fg_secondary);
    let inactive_label_style = Style::default().fg(theme.fg_primary);

    let mut spans: Vec<Span> = Vec::with_capacity(8);
    spans.push(Span::raw(" "));

    let local_active = active == TargetTab::Local;
    spans.push(Span::styled("[", bracket_style));
    if local_active {
        spans.push(Span::styled(" Local ", active_style));
    } else {
        spans.push(Span::styled(" Local ", inactive_label_style));
    }
    spans.push(Span::styled("]", bracket_style));

    spans.push(Span::raw("  "));

    let prs_active = active == TargetTab::PullRequests;
    spans.push(Span::styled("[", bracket_style));
    if prs_active {
        spans.push(Span::styled(" Pull Requests ", active_style));
    } else {
        spans.push(Span::styled(" Pull Requests ", inactive_label_style));
    }
    spans.push(Span::styled("]", bracket_style));

    let line = Line::from(spans);
    let strip = Paragraph::new(line).style(styles::panel_style(theme));
    frame.render_widget(strip, area);
}

fn render_local_target_tab(frame: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::default()
        .title(" Recent Commits ")
        .borders(Borders::ALL)
        .style(styles::panel_style(&app.theme))
        .border_style(styles::border_style(&app.theme, true));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Update viewport height for scroll calculations
    app.commit_list_viewport_height = inner.height as usize;
    app.commit_list_inner_area = Some(inner);
    app.pr_list_inner_area = None;

    // Get range info for visual indicators
    let range = app.commit_selection_range;

    // Determine commits to show
    let total_commits = app.commit_list.len();
    let visible_count = app.visible_commit_count.min(total_commits);

    let mut items: Vec<Line> = app
        .commit_list
        .iter()
        .take(visible_count)
        .enumerate()
        .map(|(i, commit)| {
            let is_selected = app.is_commit_selected(i);
            let is_cursor = i == app.commit_list_cursor;

            // Range boundary indicators
            let range_marker = match range {
                Some((start, end)) if i == start && i == end => "─",
                Some((start, _)) if i == start => "┌",
                Some((_, end)) if i == end => "└",
                Some((start, end)) if i > start && i < end => "│",
                _ => " ",
            };

            let checkbox = if is_selected { "[x]" } else { "[ ]" };
            let pointer = if is_cursor { ">" } else { " " };

            let style = if is_cursor {
                styles::selected_style(&app.theme)
            } else if is_selected {
                Style::default().fg(app.theme.fg_secondary)
            } else {
                Style::default()
            };

            let checkbox_style = if is_selected {
                styles::reviewed_style(&app.theme)
            } else {
                styles::pending_style(&app.theme)
            };

            let range_style = if is_selected {
                styles::reviewed_style(&app.theme)
            } else {
                Style::default().fg(app.theme.fg_secondary)
            };

            // Format: > ┌ [x] abc1234  Commit message (author, date)
            let time_str = commit.time.format("%Y-%m-%d").to_string();
            let mut spans = vec![
                Span::styled(format!("{pointer} "), style),
                Span::styled(format!("{range_marker} "), range_style),
                Span::styled(format!("{checkbox} "), checkbox_style),
                Span::styled(
                    format!("{} ", commit.short_id),
                    styles::hash_style(&app.theme),
                ),
            ];

            if commit.id == crate::app::STAGED_SELECTION_ID
                || commit.id == crate::app::UNSTAGED_SELECTION_ID
            {
                spans.push(Span::styled(&commit.summary, style));
                return Line::from(spans);
            }

            if let Some(branch_name) = &commit.branch_name {
                spans.push(Span::styled(
                    format!("[{}] ", truncate_str(branch_name, 20)),
                    styles::branch_style(&app.theme),
                ));
            }

            spans.push(Span::styled(truncate_str(&commit.summary, 50), style));
            spans.push(Span::styled(
                format!(" ({}, {})", commit.author, time_str),
                Style::default().fg(app.theme.fg_secondary),
            ));

            Line::from(spans)
        })
        .collect();

    // Show an expand row when commits are collapsed
    if app.can_show_more_commits() {
        let is_cursor = app.commit_list_cursor == visible_count;

        let style = if is_cursor {
            styles::selected_style(&app.theme)
        } else {
            Style::default().fg(app.theme.fg_secondary)
        };

        items.push(Line::from(vec![
            Span::styled(if is_cursor { "> " } else { "  " }, style),
            Span::styled("       ... show more commits ...", style),
        ]));
    }

    // Apply scroll offset and take only visible items
    let visible_items: Vec<Line> = items
        .into_iter()
        .skip(app.commit_list_scroll_offset)
        .take(inner.height as usize)
        .collect();

    let list = Paragraph::new(visible_items).style(styles::panel_style(&app.theme));
    frame.render_widget(list, inner);
}

fn render_pull_requests_tab(frame: &mut Frame, app: &mut App, area: Rect) {
    let title = match app.forge_repository.as_ref() {
        Some(repo) => format!(" Pull Requests ({}) ", repo.display_name()),
        None => " Pull Requests ".to_string(),
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .style(styles::panel_style(&app.theme))
        .border_style(styles::border_style(&app.theme, true));

    let inner = block.inner(area);
    frame.render_widget(block, area);
    app.commit_list_inner_area = None;
    app.pr_list_inner_area = Some(inner);
    app.pr_list_viewport_height = inner.height.saturating_sub(1) as usize;

    // Reserve one row at the top for the status / filter banner.
    let body_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(inner);
    let banner_area = body_chunks[0];
    let list_area = body_chunks[1];
    app.pr_list_viewport_height = list_area.height as usize;

    let view = app.pr_tab.view();
    render_pr_banner(frame, app, banner_area, &view);
    render_pr_list(frame, app, list_area, &view);
}

fn render_pr_banner(frame: &mut Frame, app: &App, area: Rect, view: &PrTabView<'_>) {
    let theme = &app.theme;

    if let Some(draft) = app.pr_filter_draft.as_ref() {
        let prefix = Span::styled(" filter: ", Style::default().fg(theme.fg_secondary));
        let value = Span::styled(format!("/{draft}"), Style::default().fg(theme.fg_primary));
        let hint = Span::styled(
            "   (Enter: apply  Esc: cancel)",
            Style::default().fg(theme.fg_secondary),
        );
        let line = Line::from(vec![prefix, value, hint]);
        let banner = Paragraph::new(line).style(styles::panel_style(theme));
        frame.render_widget(banner, area);
        return;
    }

    match &view.status {
        PrTabStatus::Disabled(reason) => {
            let line = Line::from(Span::styled(
                format!(" {reason} — switch back with Shift-Tab "),
                Style::default().fg(theme.fg_secondary),
            ));
            frame.render_widget(Paragraph::new(line).style(styles::panel_style(theme)), area);
        }
        PrTabStatus::Idle => {
            let line = Line::from(Span::styled(
                " Press Tab again or wait — loading…",
                Style::default().fg(theme.fg_secondary),
            ));
            frame.render_widget(Paragraph::new(line).style(styles::panel_style(theme)), area);
        }
        PrTabStatus::Loading => {
            render_pr_progress(frame, app, area, "Fetching pull requests from origin...");
        }
        PrTabStatus::LoadingMore => {
            render_pr_progress(frame, app, area, "Loading more pull requests...");
        }
        PrTabStatus::Error(msg) => {
            let line = Line::from(vec![
                Span::styled(
                    " error: ",
                    Style::default()
                        .fg(theme.message_error_fg)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled((*msg).to_string(), Style::default().fg(theme.fg_primary)),
            ]);
            frame.render_widget(Paragraph::new(line).style(styles::panel_style(theme)), area);
        }
        PrTabStatus::Ready => {
            let mut spans: Vec<Span> = vec![Span::styled(
                format!(" {} loaded", view.rows.len()),
                Style::default().fg(theme.fg_secondary),
            )];
            if !view.filter.is_empty() {
                spans.push(Span::styled(
                    format!("   filter: /{}", view.filter),
                    Style::default().fg(theme.fg_primary),
                ));
                spans.push(Span::styled(
                    "   ('/' to edit, Esc to clear)",
                    Style::default().fg(theme.fg_secondary),
                ));
            } else {
                spans.push(Span::styled(
                    "   '/' to filter",
                    Style::default().fg(theme.fg_secondary),
                ));
            }
            frame.render_widget(
                Paragraph::new(Line::from(spans)).style(styles::panel_style(theme)),
                area,
            );
        }
    }
}

fn render_pr_progress(frame: &mut Frame, app: &App, area: Rect, label: &str) {
    let theme = &app.theme;
    let width = area.width.saturating_sub(label.len() as u16 + 5).max(4) as usize;
    // Indeterminate bar: a windowed block of ~width/3 that travels using
    // wall-clock seconds, so the bar visibly moves between frames without
    // requiring per-frame state on App.
    let block_width = (width / 3).max(2);
    let travel = (width + block_width).max(1);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as usize)
        .unwrap_or(0);
    let position = (now / 80) % travel;
    let mut bar = String::with_capacity(width);
    for i in 0..width {
        if position < block_width {
            if i < position {
                bar.push(' ');
            } else if i < position + block_width && i < width {
                bar.push('#');
            } else {
                bar.push(' ');
            }
        } else {
            let start = position.saturating_sub(block_width);
            if i >= start && i < position && i < width {
                bar.push('#');
            } else {
                bar.push(' ');
            }
        }
    }
    let line = Line::from(vec![
        Span::styled(
            format!(" {label} "),
            Style::default().fg(theme.fg_secondary),
        ),
        Span::styled(
            format!("[{bar}]"),
            Style::default().fg(theme.diff_hunk_header),
        ),
    ]);
    frame.render_widget(Paragraph::new(line).style(styles::panel_style(theme)), area);
}

fn render_pr_list(frame: &mut Frame, app: &App, area: Rect, view: &PrTabView<'_>) {
    let theme = &app.theme;
    if area.height == 0 {
        return;
    }

    let mut lines: Vec<Line> = Vec::new();
    for (i, row) in view.rows.iter().enumerate() {
        let is_cursor = i == view.cursor;
        let pointer = if is_cursor { "> " } else { "  " };
        let pointer_style = if is_cursor {
            styles::selected_style(theme)
        } else {
            Style::default().fg(theme.fg_secondary)
        };
        let number = format!("#{:<5}", row.summary.number);
        let title = truncate_str(&row.summary.title, 60);
        let author = row.summary.author.as_deref().unwrap_or("?");
        let updated = row
            .summary
            .updated_at
            .map(format_relative_time)
            .unwrap_or_else(|| "—".to_string());
        let draft = if row.summary.is_draft { " [draft]" } else { "" };

        let line = Line::from(vec![
            Span::styled(pointer, pointer_style),
            Span::styled(number, styles::hash_style(theme)),
            Span::styled(" ", Style::default()),
            Span::styled(title, Style::default().fg(theme.fg_primary)),
            Span::styled(
                format!("   @{author}"),
                Style::default().fg(theme.fg_secondary),
            ),
            Span::styled(
                format!("   updated {updated}{draft}"),
                Style::default().fg(theme.fg_secondary),
            ),
        ]);
        lines.push(line);
    }

    if view.has_load_more {
        let load_idx = view.rows.len();
        let is_cursor = view.cursor == load_idx;
        let style = if is_cursor {
            styles::selected_style(theme)
        } else {
            Style::default().fg(theme.fg_secondary)
        };
        let pointer = if is_cursor { "> " } else { "  " };
        lines.push(Line::from(vec![
            Span::styled(pointer, style),
            Span::styled("... load more pull requests", style),
        ]));
    }

    if lines.is_empty() {
        // Empty state placeholder under the banner.
        if !matches!(view.status, PrTabStatus::Ready) {
            return;
        }
        let msg = if view.filter.is_empty() {
            " No open pull requests"
        } else {
            " No pull requests match the filter"
        };
        lines.push(Line::from(Span::styled(
            msg,
            Style::default().fg(theme.fg_secondary),
        )));
    }

    let visible: Vec<Line> = lines
        .into_iter()
        .skip(view.scroll_offset)
        .take(area.height as usize)
        .collect();
    let paragraph = Paragraph::new(visible).style(styles::panel_style(theme));
    frame.render_widget(paragraph, area);
}

fn format_relative_time(time: chrono::DateTime<chrono::Utc>) -> String {
    let now = chrono::Utc::now();
    let delta = now.signed_duration_since(time);
    if delta.num_seconds() < 0 {
        return "just now".to_string();
    }
    let secs = delta.num_seconds();
    if secs < 60 {
        return "just now".to_string();
    }
    let mins = delta.num_minutes();
    if mins < 60 {
        return format!("{mins}m ago");
    }
    let hours = delta.num_hours();
    if hours < 24 {
        return format!("{hours}h ago");
    }
    let days = delta.num_days();
    if days < 30 {
        return format!("{days}d ago");
    }
    let months = days / 30;
    if months < 12 {
        return format!("{months}mo ago");
    }
    let years = days / 365;
    format!("{years}y ago")
}

fn render_target_selector_footer(frame: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;
    let mode_span = Span::styled(" SELECT ", styles::mode_style(theme));

    let hints = if app.message.is_some() {
        String::new()
    } else if app.pr_filter_editing() {
        " Enter:apply  Esc:cancel ".to_string()
    } else {
        match app.target_tab {
            TargetTab::Local => {
                let selected_count = match app.commit_selection_range {
                    Some((start, end)) => end - start + 1,
                    None => 0,
                };
                let selection_info = if selected_count > 0 {
                    format!(" ({selected_count} selected)")
                } else {
                    String::new()
                };
                format!(
                    " Tab:tabs  j/k:navigate  Space:select range  Enter:confirm  q:quit{selection_info}"
                )
            }
            TargetTab::PullRequests => {
                " Tab:tabs  j/k:navigate  Enter:open  /:filter  Esc/q:back ".to_string()
            }
        }
    };
    let hints_span = Span::styled(hints, Style::default().fg(theme.fg_secondary));

    let left_spans = vec![mode_span, hints_span];

    let (message_span, message_width) = status_bar::build_message_span(app.message.as_ref(), theme);
    let spans = status_bar::build_right_aligned_spans(
        left_spans,
        message_span,
        message_width,
        area.width as usize,
    );

    let footer = Paragraph::new(Line::from(spans))
        .style(styles::status_bar_style(theme))
        .block(Block::default());
    frame.render_widget(footer, area);
}

#[cfg(test)]
mod selector_render_snapshot_tests {
    //! Render-snapshot tests for the review-target selector. We drive the
    //! real `render` against ratatui's `TestBackend` and assert on the
    //! resulting character grid (plus a few style checks for the active
    //! tab highlight). These tests caught a regression where the inactive
    //! tab rendered as bare dim text with no bracket / cue, making the
    //! Pull Requests tab functionally invisible.
    use crate::app::{App, DiffSource, InputMode};
    use crate::error::Result as TuicrResult;
    use crate::error::TuicrError;
    use crate::forge::selector::PullRequestsTab;
    use crate::forge::traits::{ForgeRepository, PullRequestSummary};
    use crate::model::{DiffFile, DiffLine, FileStatus, ReviewSession, SessionDiffSource};
    use crate::syntax::SyntaxHighlighter;
    use crate::theme::Theme;
    use crate::ui::render;
    use crate::vcs::CommitInfo;
    use crate::vcs::traits::{VcsBackend, VcsChangeStatus, VcsInfo, VcsType};
    use chrono::{TimeZone, Utc};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::style::Modifier;
    use std::path::{Path, PathBuf};

    struct SnapshotVcs {
        info: VcsInfo,
    }

    impl VcsBackend for SnapshotVcs {
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

        fn get_change_status(&self) -> TuicrResult<VcsChangeStatus> {
            Ok(VcsChangeStatus {
                staged: false,
                unstaged: false,
            })
        }
    }

    fn commit(i: usize) -> CommitInfo {
        CommitInfo {
            id: format!("abc{i}"),
            short_id: format!("abc{i}"),
            branch_name: None,
            summary: format!("commit {i}"),
            body: None,
            author: "tester".to_string(),
            time: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        }
    }

    fn make_app(commits: Vec<CommitInfo>) -> App {
        let vcs_info = VcsInfo {
            root_path: PathBuf::from("/tmp"),
            head_commit: "head".to_string(),
            branch_name: Some("main".to_string()),
            vcs_type: VcsType::Git,
        };
        let session = ReviewSession::new(
            vcs_info.root_path.clone(),
            vcs_info.head_commit.clone(),
            vcs_info.branch_name.clone(),
            SessionDiffSource::WorkingTree,
        );
        App::build(
            Box::new(SnapshotVcs {
                info: vcs_info.clone(),
            }),
            vcs_info,
            Theme::dark(),
            None,
            false,
            Vec::new(),
            session,
            DiffSource::WorkingTree,
            InputMode::CommitSelect,
            commits,
            None,
        )
        .expect("build app")
    }

    fn repo() -> ForgeRepository {
        ForgeRepository::github("github.com", "agavra", "tuicr")
    }

    fn pr(number: u64, title: &str, author: &str) -> PullRequestSummary {
        PullRequestSummary {
            repository: repo(),
            number,
            title: title.to_string(),
            author: Some(author.to_string()),
            head_ref_name: format!("feat/{number}"),
            base_ref_name: "main".to_string(),
            updated_at: Some(Utc.with_ymd_and_hms(2026, 5, 1, 0, 0, 0).unwrap()),
            url: format!("https://github.com/agavra/tuicr/pull/{number}"),
            state: "OPEN".to_string(),
            is_draft: false,
        }
    }

    fn draw(app: &mut App) -> Buffer {
        let backend = TestBackend::new(120, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render(frame, app))
            .expect("draw frame");
        terminal.backend().buffer().clone()
    }

    fn row_text(buffer: &Buffer, y: u16) -> String {
        (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol().to_string())
            .collect()
    }

    /// True when at least one cell in [x_start, x_end) on row `y` carries the
    /// REVERSED modifier — our visual marker for the active tab.
    fn any_reversed_in_range(buffer: &Buffer, y: u16, x_start: u16, x_end: u16) -> bool {
        (x_start..x_end.min(buffer.area.width)).any(|x| {
            buffer[(x, y)]
                .style()
                .add_modifier
                .contains(Modifier::REVERSED)
        })
    }

    /// Locate the inclusive x-range of a substring on row `y`. Panics if
    /// the substring is not present, with a helpful dump.
    fn locate(buffer: &Buffer, y: u16, needle: &str) -> (u16, u16) {
        let line = row_text(buffer, y);
        let byte_idx = line
            .find(needle)
            .unwrap_or_else(|| panic!("expected to find {needle:?} on row {y}, got {line:?}"));
        // Symbols are ASCII in our render so byte index == column index.
        let start = byte_idx as u16;
        let end = start + needle.len() as u16;
        (start, end)
    }

    #[test]
    fn should_render_both_tabs_with_brackets_when_local_active_and_no_forge() {
        // given — plain app with no GitHub remote
        let mut app = make_app(vec![commit(0), commit(1)]);
        // when
        let buffer = draw(&mut app);
        // then — tab strip has both bracketed labels
        let strip = row_text(&buffer, 1);
        assert!(
            strip.contains("[ Local ]") && strip.contains("[ Pull Requests ]"),
            "tab strip missing brackets: {strip:?}"
        );
        // and — the active "Local" label is REVERSED, inactive PR label is not
        let (lo, hi) = locate(&buffer, 1, "Local");
        assert!(
            any_reversed_in_range(&buffer, 1, lo, hi),
            "active Local label should be REVERSED"
        );
        let (lo, hi) = locate(&buffer, 1, "Pull Requests");
        assert!(
            !any_reversed_in_range(&buffer, 1, lo, hi),
            "inactive Pull Requests label should NOT be REVERSED"
        );
    }

    #[test]
    fn should_show_disabled_banner_when_pr_tab_active_without_forge() {
        // given
        let mut app = make_app(vec![commit(0)]);
        app.target_tab = crate::app::TargetTab::PullRequests;
        // when
        let buffer = draw(&mut app);
        // then — banner spells out the reason
        let body = (3..buffer.area.height)
            .map(|y| row_text(&buffer, y))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            body.contains("No GitHub remote on this repo"),
            "expected disabled banner, got body:\n{body}"
        );
    }

    #[test]
    fn should_highlight_pr_tab_label_when_pr_tab_active() {
        // given — forge repo configured, PR tab active in Idle state
        let mut app = make_app(vec![commit(0)]);
        app.forge_repository = Some(repo());
        app.pr_tab = PullRequestsTab::new(Some(repo()));
        app.target_tab = crate::app::TargetTab::PullRequests;
        // when
        let buffer = draw(&mut app);
        // then — Pull Requests is the highlighted (REVERSED) label
        let (lo, hi) = locate(&buffer, 1, "Pull Requests");
        assert!(
            any_reversed_in_range(&buffer, 1, lo, hi),
            "active Pull Requests label should be REVERSED"
        );
        let (lo, hi) = locate(&buffer, 1, "Local");
        assert!(
            !any_reversed_in_range(&buffer, 1, lo, hi),
            "inactive Local label should NOT be REVERSED"
        );
    }

    #[test]
    fn should_render_loaded_pr_rows_with_number_title_author() {
        // given — three loaded PRs, second with no author
        let mut app = make_app(vec![commit(0)]);
        app.forge_repository = Some(repo());
        let mut tab = PullRequestsTab::new(Some(repo()));
        tab.start_initial_load();
        tab.apply_initial_load(Ok((
            vec![
                pr(148, "Add forge-backed PR review", "alice"),
                pr(125, "Support fetching/pushing reviews", "ypares"),
            ],
            false,
        )));
        app.pr_tab = tab;
        app.target_tab = crate::app::TargetTab::PullRequests;
        // when
        let buffer = draw(&mut app);
        // then — both rows are present in the rendered body
        let body = (3..buffer.area.height)
            .map(|y| row_text(&buffer, y))
            .collect::<Vec<_>>()
            .join("\n");
        for needle in [
            "#148",
            "Add forge-backed PR review",
            "alice",
            "#125",
            "Support fetching/pushing reviews",
            "ypares",
        ] {
            assert!(body.contains(needle), "missing {needle:?} in:\n{body}");
        }
    }

    #[test]
    fn should_show_load_more_row_when_has_more_and_no_filter() {
        // given
        let mut app = make_app(vec![commit(0)]);
        app.forge_repository = Some(repo());
        let mut tab = PullRequestsTab::new(Some(repo()));
        tab.start_initial_load();
        tab.apply_initial_load(Ok((vec![pr(1, "alpha", "a")], true)));
        app.pr_tab = tab;
        app.target_tab = crate::app::TargetTab::PullRequests;
        // when
        let buffer = draw(&mut app);
        // then
        let body = (3..buffer.area.height)
            .map(|y| row_text(&buffer, y))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            body.contains("load more pull requests"),
            "expected load-more row, body:\n{body}"
        );
    }

    #[test]
    fn should_hide_load_more_row_when_filter_active() {
        // given — has_more is true but a filter is set
        let mut app = make_app(vec![commit(0)]);
        app.forge_repository = Some(repo());
        let mut tab = PullRequestsTab::new(Some(repo()));
        tab.start_initial_load();
        tab.apply_initial_load(Ok((vec![pr(1, "alpha", "a")], true)));
        tab.set_filter("alpha".to_string());
        app.pr_tab = tab;
        app.target_tab = crate::app::TargetTab::PullRequests;
        // when
        let buffer = draw(&mut app);
        // then
        let body = (3..buffer.area.height)
            .map(|y| row_text(&buffer, y))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !body.contains("load more pull requests"),
            "load-more row should be hidden while filter is active:\n{body}"
        );
    }

    #[test]
    fn should_render_filter_draft_banner_when_editing_filter() {
        // given
        let mut app = make_app(vec![commit(0)]);
        app.forge_repository = Some(repo());
        let mut tab = PullRequestsTab::new(Some(repo()));
        tab.start_initial_load();
        tab.apply_initial_load(Ok((vec![pr(1, "alpha", "a")], false)));
        app.pr_tab = tab;
        app.target_tab = crate::app::TargetTab::PullRequests;
        app.pr_filter_draft = Some("alp".to_string());
        // when
        let buffer = draw(&mut app);
        // then — the banner shows the in-progress filter expression
        let body = (3..buffer.area.height)
            .map(|y| row_text(&buffer, y))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            body.contains("/alp") && body.contains("filter"),
            "expected filter draft banner, got:\n{body}"
        );
    }

    #[test]
    fn should_render_error_banner_when_pr_load_failed() {
        // given
        let mut app = make_app(vec![commit(0)]);
        app.forge_repository = Some(repo());
        let mut tab = PullRequestsTab::new(Some(repo()));
        tab.start_initial_load();
        tab.apply_initial_load(Err("network down".to_string()));
        app.pr_tab = tab;
        app.target_tab = crate::app::TargetTab::PullRequests;
        // when
        let buffer = draw(&mut app);
        // then
        let body = (3..buffer.area.height)
            .map(|y| row_text(&buffer, y))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            body.contains("error") && body.contains("network down"),
            "expected error banner, got:\n{body}"
        );
    }

    #[test]
    fn should_render_loading_banner_when_pr_load_in_flight() {
        // given
        let mut app = make_app(vec![commit(0)]);
        app.forge_repository = Some(repo());
        let mut tab = PullRequestsTab::new(Some(repo()));
        tab.start_initial_load();
        // (stays in Loading until apply_initial_load)
        app.pr_tab = tab;
        app.target_tab = crate::app::TargetTab::PullRequests;
        // when
        let buffer = draw(&mut app);
        // then
        let body = (3..buffer.area.height)
            .map(|y| row_text(&buffer, y))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            body.contains("Fetching pull requests"),
            "expected loading banner, got:\n{body}"
        );
    }
}
