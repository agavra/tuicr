use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::app::{App, TargetTab};
use crate::forge::selector::{PrTabStatus, PrTabView};
use crate::forge::traits::PullRequestListScope;
use crate::ui::commit_row::{
    CURSOR_GLYPH, CommitRowSpec, format_relative_short, render_commit_row,
};
use crate::ui::status_bar;
use crate::ui::styles;
use crate::ui::text_utils::truncate_or_pad;

const TAB_LOCAL: &str = "Local";
const TAB_PULL_REQUESTS: &str = "Pull Requests";

pub(super) fn render_commit_select(frame: &mut Frame, app: &mut App) {
    let area = frame.area();

    // Layout: top bar (brand + tabs + right-slot status), bordered body,
    // footer. The tabs live INSIDE the top bar — no separate strip row.
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // top bar (brand + tabs + status)
            Constraint::Min(0),    // body block (full borders)
            Constraint::Length(1), // footer
        ])
        .split(area);

    render_top_bar(frame, app, chunks[0]);

    let body_area = chunks[1];
    let body_block = Block::default()
        .borders(Borders::ALL)
        .border_style(styles::border_style(&app.theme, true))
        .style(styles::panel_style(&app.theme));
    let inner = body_block.inner(body_area);
    frame.render_widget(body_block, body_area);

    match app.target_tab {
        TargetTab::Local => render_local_target_tab(frame, app, inner),
        TargetTab::PullRequests => render_pull_requests_tab(frame, app, inner),
    }

    render_target_selector_footer(frame, app, chunks[2]);
}

/// Combined top bar: brand on the left, tab chips, then a right slot
/// carrying `git:<branch>` (Local tab) or the PR-tab status hint. The entire
/// row uses `status_bar_bg` so the active tab's `bg_highlight` reads as a
/// chip popping out of the strip.
fn render_top_bar(frame: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;
    let active = app.target_tab;
    let local_active = active == TargetTab::Local;
    let pr_active = active == TargetTab::PullRequests;

    let strip_bg = theme.status_bar_bg;
    let strip_style = Style::default().bg(strip_bg).fg(theme.fg_dim);
    let brand_style = Style::default()
        .bg(strip_bg)
        .fg(theme.fg_primary)
        .add_modifier(Modifier::BOLD);
    let active_chip = Style::default()
        .bg(theme.bg_highlight)
        .fg(theme.fg_primary)
        .add_modifier(Modifier::BOLD);
    let inactive_chip = Style::default().bg(strip_bg).fg(theme.fg_dim);

    let mut spans: Vec<Span<'static>> = Vec::new();
    spans.push(Span::styled(" tuicr  ", brand_style));
    spans.push(Span::styled(
        format!(" {TAB_LOCAL} "),
        if local_active {
            active_chip
        } else {
            inactive_chip
        },
    ));
    spans.push(Span::styled(" ".to_string(), strip_style));
    spans.push(Span::styled(
        format!(" {TAB_PULL_REQUESTS} "),
        if pr_active {
            active_chip
        } else {
            inactive_chip
        },
    ));

    let left_width: usize = spans.iter().map(|s| s.content.chars().count()).sum();

    let (right_span, right_width) = match active {
        TargetTab::PullRequests => pr_status_hint_span(app, strip_bg),
        TargetTab::Local => {
            let vcs_type = &app.vcs_info.vcs_type;
            let branch = app.vcs_info.branch_name.as_deref().unwrap_or("detached");
            let content = format!(" {vcs_type}:{branch} ");
            let width = content.chars().count();
            (Span::styled(content, strip_style), width)
        }
    };

    let total_width = area.width as usize;
    let pad = total_width.saturating_sub(left_width + right_width);
    spans.push(Span::styled(" ".repeat(pad), strip_style));
    if right_width > 0 {
        spans.push(right_span);
    }

    frame.render_widget(Paragraph::new(Line::from(spans)).style(strip_style), area);
}

/// Right-slot status hint for the PR tab. Idle ready states show
/// `<count> loaded · /<filter>` (filter only when set). Loading shows a
/// braille spinner + label. Error renders in error color. The caller
/// supplies the strip bg so the hint blends with the tab strip's bar.
fn pr_status_hint_span(app: &App, strip_bg: ratatui::style::Color) -> (Span<'static>, usize) {
    let theme = &app.theme;
    let view = app.pr_tab.view();
    let base = Style::default().bg(strip_bg).fg(theme.fg_secondary);
    let (content, style) = match &view.status {
        PrTabStatus::Disabled(reason) => (format!("{reason} — Shift-Tab to go back "), base),
        PrTabStatus::Idle => {
            let prefix = pr_scope_prefix(view.scope);
            (format!(" {prefix}waiting… "), base)
        }
        PrTabStatus::Loading => {
            let glyph = pr_open_spinner_glyph(std::time::Duration::from_millis(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0),
            ));
            let prefix = pr_scope_prefix(view.scope);
            (format!("{glyph} {prefix}loading… "), base)
        }
        PrTabStatus::LoadingMore => {
            let glyph = pr_open_spinner_glyph(std::time::Duration::from_millis(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0),
            ));
            let prefix = pr_scope_prefix(view.scope);
            (format!("{glyph} {prefix}loading more… "), base)
        }
        PrTabStatus::Error(msg) => (
            format!("error \u{00b7} {msg} "),
            styles::error_inline_style(theme).bg(strip_bg),
        ),
        PrTabStatus::Ready => {
            let mut s = format!("{}{} loaded", pr_scope_prefix(view.scope), view.rows.len());
            if !view.filter.is_empty() {
                s.push_str(&format!(" \u{00b7} /{}", view.filter));
            }
            s.push(' ');
            (s, base)
        }
    };
    let width = content.len();
    (Span::styled(content, style), width)
}

fn pr_scope_prefix(scope: PullRequestListScope) -> &'static str {
    match scope {
        PullRequestListScope::Open => "",
        PullRequestListScope::ReviewRequested => "requested · ",
    }
}

fn render_local_target_tab(frame: &mut Frame, app: &mut App, area: Rect) {
    // Update viewport height for scroll calculations
    app.commit_list_viewport_height = area.height as usize;
    app.commit_list_inner_area = Some(area);
    app.pr_list_inner_area = None;

    let total_commits = app.commit_list.len();
    let visible_count = app.visible_commit_count.min(total_commits);

    let mut items: Vec<Line> = app
        .commit_list
        .iter()
        .take(visible_count)
        .enumerate()
        .map(|(i, commit)| {
            render_commit_row(&CommitRowSpec {
                commit,
                is_cursor: i == app.commit_list_cursor,
                is_selected: app.is_commit_selected(i),
                theme: &app.theme,
            })
        })
        .collect();

    if app.can_show_more_commits() {
        items.push(overflow_row(
            &app.theme,
            app.commit_list_cursor == visible_count,
            "show more commits",
        ));
    }

    let visible_items: Vec<Line> = items
        .into_iter()
        .skip(app.commit_list_scroll_offset)
        .take(area.height as usize)
        .collect();

    let list = Paragraph::new(visible_items).style(styles::panel_style(&app.theme));
    frame.render_widget(list, area);
}

fn overflow_row<'a>(theme: &crate::theme::Theme, is_cursor: bool, label: &'a str) -> Line<'a> {
    let style = if is_cursor {
        styles::selected_style(theme)
    } else {
        Style::default().fg(theme.fg_dim)
    };
    let pointer = if is_cursor {
        format!("{CURSOR_GLYPH} ")
    } else {
        "  ".to_string()
    };
    Line::from(vec![
        Span::styled(pointer, style),
        Span::styled(format!("    \u{2026} {label}"), style),
    ])
}

fn render_pull_requests_tab(frame: &mut Frame, app: &mut App, area: Rect) {
    app.commit_list_inner_area = None;
    app.pr_list_inner_area = Some(area);
    app.pr_list_viewport_height = area.height as usize;

    let view = app.pr_tab.view();
    render_pr_list(frame, app, area, &view);
}

fn render_pr_list(frame: &mut Frame, app: &App, area: Rect, view: &PrTabView<'_>) {
    let theme = &app.theme;
    if area.height == 0 {
        return;
    }

    // The tab-strip right slot already surfaces loading / error / disabled
    // hints concisely; the body only fills in when there's a useful action
    // (idle → tap Tab again, error → retry guidance). Disabled / Loading /
    // LoadingMore leave the body blank so we don't echo the same text twice.
    match view.status {
        PrTabStatus::Disabled(_) | PrTabStatus::Loading | PrTabStatus::LoadingMore => return,
        PrTabStatus::Idle => {
            let line = Line::from(Span::styled(
                "  Press Tab again to load pull requests\u{2026}",
                Style::default().fg(theme.fg_dim),
            ));
            frame.render_widget(
                Paragraph::new(vec![line]).style(styles::panel_style(theme)),
                area,
            );
            return;
        }
        PrTabStatus::Error(msg) => {
            let line = Line::from(vec![
                Span::styled("  error \u{00b7} ", styles::error_inline_style(theme)),
                Span::styled(msg.to_string(), Style::default().fg(theme.fg_primary)),
            ]);
            frame.render_widget(
                Paragraph::new(vec![line]).style(styles::panel_style(theme)),
                area,
            );
            return;
        }
        PrTabStatus::Ready => {}
    }

    let spinner = app
        .pr_open_state
        .as_ref()
        .map(|s| pr_open_spinner_glyph(s.started_at.elapsed()));

    let mut lines: Vec<Line> = Vec::new();
    for (i, row) in view.rows.iter().enumerate() {
        let is_cursor = i == view.cursor;
        let is_loading = matches!(
            (spinner, app.pr_open_state.as_ref()),
            (Some(_), Some(s)) if s.matches(&row.summary.repository, row.summary.number)
        );

        let pointer_str = if is_loading {
            format!("{} ", spinner.unwrap_or("⠋"))
        } else if is_cursor {
            format!("{CURSOR_GLYPH} ")
        } else {
            "  ".to_string()
        };
        let pointer_style = if is_loading || is_cursor {
            styles::selected_style(theme)
        } else {
            Style::default().fg(theme.fg_secondary)
        };

        // PRs are single-select. Mirror the commit-row column layout for
        // visual consistency (leading cursor + a placeholder column where
        // the range bar lives on commit rows) and pad title/author so the
        // date column lands at the same x across rows.
        let number = format!("#{:<5}", row.summary.number);
        let title = truncate_or_pad(&row.summary.title, 60);
        let author = truncate_or_pad(row.summary.author.as_deref().unwrap_or("?"), 12);
        let updated = row
            .summary
            .updated_at
            .as_ref()
            .map(format_relative_short)
            .unwrap_or_else(|| "—".to_string());
        let draft = if row.summary.is_draft { " [draft]" } else { "" };

        lines.push(Line::from(vec![
            Span::styled(pointer_str, pointer_style),
            Span::styled("  ", Style::default()),
            Span::styled(number, styles::hash_style(theme)),
            Span::styled(" ", Style::default()),
            Span::styled(title, Style::default().fg(theme.fg_primary)),
            Span::styled(
                format!("  {} \u{00b7} {}{}", author, updated, draft),
                Style::default().fg(theme.fg_secondary),
            ),
        ]));
    }

    if view.has_load_more {
        let load_idx = view.rows.len();
        let is_cursor = view.cursor == load_idx;
        lines.push(overflow_row(theme, is_cursor, "load more pull requests"));
    }

    if lines.is_empty() {
        let msg = if view.filter.is_empty() {
            "  No open pull requests"
        } else {
            "  No pull requests match the filter"
        };
        lines.push(Line::from(Span::styled(
            msg,
            Style::default().fg(theme.fg_dim),
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

/// Braille spinner frames advanced every ~100ms based on elapsed time.
/// Stable across redraws because the start instant lives on `App`.
pub(crate) fn pr_open_spinner_glyph(elapsed: std::time::Duration) -> &'static str {
    const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    const FRAME_MS: u128 = 100;
    let idx = (elapsed.as_millis() / FRAME_MS) as usize % FRAMES.len();
    FRAMES[idx]
}

fn render_target_selector_footer(frame: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;

    // While editing the PR filter, the footer becomes a vim-style input
    // line: left slot carries `/<draft>|`, right slot the apply/cancel hint.
    if let Some(draft) = app.pr_filter_draft.as_ref() {
        let left = vec![Span::styled(
            format!(" /{draft}"),
            Style::default().fg(theme.fg_primary),
        )];
        let (message_span, message_width) =
            status_bar::build_message_span(app.message.as_ref(), theme);
        let right_span = if message_width > 0 {
            message_span
        } else {
            Span::styled(
                " enter apply \u{00b7} esc cancel ",
                Style::default().fg(theme.fg_secondary),
            )
        };
        let right_width = right_span.content.len();
        let spans = status_bar::build_right_aligned_spans(
            left,
            right_span,
            right_width,
            area.width as usize,
        );
        let footer = Paragraph::new(Line::from(spans)).style(styles::status_bar_style(theme));
        frame.render_widget(footer, area);

        // Set the terminal cursor at the end of the filter buffer so users
        // see where typing will land.
        let cursor_x = area.x + 2 + draft.len() as u16;
        let cursor_y = area.y;
        frame.set_cursor_position(ratatui::layout::Position {
            x: cursor_x.min(area.x + area.width.saturating_sub(1)),
            y: cursor_y,
        });
        return;
    }

    let mode_span = Span::styled(" SELECT ", styles::mode_style(theme));

    let hints = if app.message.is_some() {
        String::new()
    } else {
        match app.target_tab {
            TargetTab::Local => {
                "   j/k navigate \u{00b7} space range \u{00b7} \u{21b5} confirm \u{00b7} q quit"
                    .to_string()
            }
            TargetTab::PullRequests => {
                let scope_hint = match app.pr_tab.scope() {
                    PullRequestListScope::Open => "r requested",
                    PullRequestListScope::ReviewRequested => "r all PRs",
                };
                format!("   j/k navigate · ↵ open · {scope_hint} · / filter · esc/q back")
            }
        }
    };
    let hints_span = Span::styled(hints, Style::default().fg(theme.fg_secondary));

    let selected_count = match app.commit_selection_range {
        Some((start, end)) if app.target_tab == TargetTab::Local => end - start + 1,
        _ => 0,
    };

    let (right_span, right_width) = if let (Some(_), _) = (&app.message, ()) {
        status_bar::build_message_span(app.message.as_ref(), theme)
    } else if selected_count > 0 {
        let text = format!(" {selected_count} selected ");
        let width = text.len();
        (Span::styled(text, Style::default().fg(theme.fg_dim)), width)
    } else {
        (Span::raw(""), 0)
    };

    let spans = status_bar::build_right_aligned_spans(
        vec![mode_span, hints_span],
        right_span,
        right_width,
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
    //! tab highlight).
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
            _ref_commit: Option<&str>,
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

        fn file_line_count(
            &self,
            _file_path: &Path,
            _file_status: FileStatus,
            _ref_commit: Option<&str>,
        ) -> TuicrResult<u32> {
            Ok(0)
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

    /// True when at least one cell in [x_start, x_end) on row `y` carries
    /// the BOLD modifier — the active-label cue in the new flat design.
    fn any_bold_in_range(buffer: &Buffer, y: u16, x_start: u16, x_end: u16) -> bool {
        (x_start..x_end.min(buffer.area.width))
            .any(|x| buffer[(x, y)].style().add_modifier.contains(Modifier::BOLD))
    }

    /// Locate the inclusive x-range of a substring on row `y`. Panics if
    /// the substring is not present.
    fn locate(buffer: &Buffer, y: u16, needle: &str) -> (u16, u16) {
        let line = row_text(buffer, y);
        let byte_idx = line
            .find(needle)
            .unwrap_or_else(|| panic!("expected to find {needle:?} on row {y}, got {line:?}"));
        let start = byte_idx as u16;
        let end = start + needle.len() as u16;
        (start, end)
    }

    const TAB_STRIP_ROW: u16 = 0;

    /// True when at least one cell in [x_start, x_end) on row `y` carries
    /// the given background color.
    fn any_bg_in_range(
        buffer: &Buffer,
        y: u16,
        x_start: u16,
        x_end: u16,
        bg: ratatui::style::Color,
    ) -> bool {
        (x_start..x_end.min(buffer.area.width)).any(|x| buffer[(x, y)].style().bg == Some(bg))
    }

    #[test]
    fn should_render_both_tab_labels_with_active_chip_bg_when_local_active() {
        // given — plain app, Local is active by default
        let mut app = make_app(vec![commit(0), commit(1)]);
        let highlight_bg = app.theme.bg_highlight;
        // when
        let buffer = draw(&mut app);
        // then — tab strip shows both labels in the single bg-filled row
        let strip = row_text(&buffer, TAB_STRIP_ROW);
        assert!(
            strip.contains("Local") && strip.contains("Pull Requests"),
            "tab strip missing labels: {strip:?}"
        );
        // and — the active "Local" chip carries the highlight bg
        let (lo, hi) = locate(&buffer, TAB_STRIP_ROW, "Local");
        assert!(
            any_bg_in_range(&buffer, TAB_STRIP_ROW, lo, hi, highlight_bg),
            "active Local chip should carry bg_highlight"
        );
        // and the active label is BOLD
        assert!(
            any_bold_in_range(&buffer, TAB_STRIP_ROW, lo, hi),
            "active Local label should be BOLD"
        );
        // inactive "Pull Requests" is NOT highlighted
        let (lo, hi) = locate(&buffer, TAB_STRIP_ROW, "Pull Requests");
        assert!(
            !any_bg_in_range(&buffer, TAB_STRIP_ROW, lo, hi, highlight_bg),
            "inactive Pull Requests chip should NOT carry bg_highlight"
        );
    }

    #[test]
    fn should_show_disabled_hint_in_status_slot_when_pr_tab_active_without_forge() {
        // given
        let mut app = make_app(vec![commit(0)]);
        app.target_tab = crate::app::TargetTab::PullRequests;
        // when
        let buffer = draw(&mut app);
        // then — tab strip carries the disabled reason in its right slot
        let strip = row_text(&buffer, TAB_STRIP_ROW);
        assert!(
            strip.contains("No GitHub remote on this repo"),
            "expected disabled hint in tab strip, got: {strip:?}"
        );
    }

    #[test]
    fn should_bold_pr_tab_label_when_pr_tab_active() {
        // given
        let mut app = make_app(vec![commit(0)]);
        app.forge_repository = Some(repo());
        app.pr_tab = PullRequestsTab::new(Some(repo()));
        app.target_tab = crate::app::TargetTab::PullRequests;
        // when
        let buffer = draw(&mut app);
        // then
        let (lo, hi) = locate(&buffer, TAB_STRIP_ROW, "Pull Requests");
        assert!(any_bold_in_range(&buffer, TAB_STRIP_ROW, lo, hi));
        let (lo, hi) = locate(&buffer, TAB_STRIP_ROW, "Local");
        assert!(!any_bold_in_range(&buffer, TAB_STRIP_ROW, lo, hi));
    }

    #[test]
    fn should_render_loaded_pr_rows_with_number_title_author() {
        // given — two loaded PRs
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
        // then
        let body = (2..buffer.area.height)
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
    fn should_show_loaded_count_in_tab_strip_status_slot_when_ready() {
        // given
        let mut app = make_app(vec![commit(0)]);
        app.forge_repository = Some(repo());
        let mut tab = PullRequestsTab::new(Some(repo()));
        tab.start_initial_load();
        tab.apply_initial_load(Ok((
            vec![pr(1, "alpha", "a"), pr(2, "beta", "b"), pr(3, "gamma", "c")],
            false,
        )));
        app.pr_tab = tab;
        app.target_tab = crate::app::TargetTab::PullRequests;
        // when
        let buffer = draw(&mut app);
        // then — `3 loaded` lives in the tab-strip right slot
        let strip = row_text(&buffer, TAB_STRIP_ROW);
        assert!(
            strip.contains("3 loaded"),
            "expected '3 loaded' in tab strip, got: {strip:?}"
        );
    }

    #[test]
    fn should_show_requested_scope_in_tab_strip_status_slot() {
        // given
        let mut app = make_app(vec![commit(0)]);
        app.forge_repository = Some(repo());
        let mut tab = PullRequestsTab::new(Some(repo()));
        tab.toggle_scope_and_start_reload();
        tab.apply_initial_load(Ok((vec![pr(1, "alpha", "a")], false)));
        app.pr_tab = tab;
        app.target_tab = crate::app::TargetTab::PullRequests;
        // when
        let buffer = draw(&mut app);
        // then
        let strip = row_text(&buffer, TAB_STRIP_ROW);
        assert!(
            strip.contains("requested · 1 loaded"),
            "expected requested scope in tab strip, got: {strip:?}"
        );
        let footer = row_text(&buffer, buffer.area.height - 1);
        assert!(
            footer.contains("r all PRs"),
            "expected footer toggle hint, got: {footer:?}"
        );
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
        let body = (2..buffer.area.height)
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
        // given
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
        let body = (2..buffer.area.height)
            .map(|y| row_text(&buffer, y))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !body.contains("load more pull requests"),
            "load-more row should be hidden while filter is active:\n{body}"
        );
    }

    #[test]
    fn should_render_filter_draft_input_in_footer_when_editing() {
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
        // then — footer (last row) carries `/alp` on the left and apply/cancel on the right
        let footer = row_text(&buffer, buffer.area.height - 1);
        assert!(
            footer.contains("/alp"),
            "expected filter draft in footer, got: {footer:?}"
        );
        assert!(
            footer.contains("apply") && footer.contains("cancel"),
            "expected apply/cancel hint in footer, got: {footer:?}"
        );
    }

    #[test]
    fn should_render_error_state_in_tab_strip_status_slot_when_pr_load_failed() {
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
        let strip = row_text(&buffer, TAB_STRIP_ROW);
        assert!(
            strip.contains("error") && strip.contains("network down"),
            "expected error in tab-strip status slot, got: {strip:?}"
        );
    }

    #[test]
    fn should_render_loading_state_in_tab_strip_status_slot_when_pr_load_in_flight() {
        // given
        let mut app = make_app(vec![commit(0)]);
        app.forge_repository = Some(repo());
        let mut tab = PullRequestsTab::new(Some(repo()));
        tab.start_initial_load();
        app.pr_tab = tab;
        app.target_tab = crate::app::TargetTab::PullRequests;
        // when
        let buffer = draw(&mut app);
        // then
        let strip = row_text(&buffer, TAB_STRIP_ROW);
        assert!(
            strip.contains("loading"),
            "expected loading hint in tab strip, got: {strip:?}"
        );
    }

    #[test]
    fn should_render_spinner_glyph_in_place_of_cursor_for_loading_pr_row() {
        // given
        use crate::app::PrOpenRequest;
        use std::time::Instant;
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
        app.pr_open_state = Some(PrOpenRequest {
            repository: repo(),
            pr_number: 148,
            started_at: Instant::now(),
        });
        // when
        let buffer = draw(&mut app);
        // then
        let frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        let lines: Vec<String> = (2..buffer.area.height)
            .map(|y| row_text(&buffer, y))
            .collect();
        let loading_row = lines
            .iter()
            .find(|l| l.contains("#148"))
            .expect("#148 row missing");
        assert!(
            frames.iter().any(|g| loading_row.contains(g)),
            "loading row should contain a spinner glyph: {loading_row:?}"
        );
    }

    #[test]
    fn should_keep_cursor_pointer_on_other_rows_during_loading() {
        // given
        use crate::app::PrOpenRequest;
        use std::time::Instant;
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
        if let PullRequestsTab::Loaded { cursor, .. } = &mut tab {
            *cursor = 1;
        }
        app.pr_tab = tab;
        app.target_tab = crate::app::TargetTab::PullRequests;
        app.pr_open_state = Some(PrOpenRequest {
            repository: repo(),
            pr_number: 148,
            started_at: Instant::now(),
        });
        // when
        let buffer = draw(&mut app);
        // then — #125 row keeps the cursor arrow
        let lines: Vec<String> = (2..buffer.area.height)
            .map(|y| row_text(&buffer, y))
            .collect();
        let cursor_row = lines
            .iter()
            .find(|l| l.contains("#125"))
            .expect("#125 row missing");
        assert!(
            cursor_row.contains("\u{25b8}"),
            "cursor row should contain ▸ glyph: {cursor_row:?}"
        );
    }
}

#[cfg(test)]
mod pr_open_spinner_tests {
    use super::pr_open_spinner_glyph;
    use std::time::Duration;

    #[test]
    fn should_advance_braille_frame_every_100ms() {
        // given / when / then
        assert_eq!(pr_open_spinner_glyph(Duration::from_millis(0)), "⠋");
        assert_eq!(pr_open_spinner_glyph(Duration::from_millis(99)), "⠋");
        assert_eq!(pr_open_spinner_glyph(Duration::from_millis(100)), "⠙");
        assert_eq!(pr_open_spinner_glyph(Duration::from_millis(900)), "⠏");
        assert_eq!(pr_open_spinner_glyph(Duration::from_millis(1000)), "⠋");
    }
}
