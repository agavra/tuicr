use std::path::Path;

use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::app::{App, SummaryCommentTarget};
use crate::model::{Comment, LineSide};
use crate::slug::short_sha;
use crate::ui::styles;
use crate::ui::text_utils::wrap_spans;

struct PendingComment<'a> {
    location: String,
    comment: &'a Comment,
    target: Option<SummaryCommentTarget>,
}

struct SummaryRenderedLine {
    line: Line<'static>,
    comment_idx: Option<usize>,
}

impl SummaryRenderedLine {
    fn plain(line: Line<'static>) -> Self {
        Self {
            line,
            comment_idx: None,
        }
    }

    fn for_comment(line: Line<'static>, comment_idx: usize) -> Self {
        Self {
            line,
            comment_idx: Some(comment_idx),
        }
    }
}

struct SummaryContent {
    lines: Vec<SummaryRenderedLine>,
    comment_ranges: Vec<(usize, usize)>,
    targets: Vec<Option<SummaryCommentTarget>>,
}

/// Render every pending local draft in the active review session. Unlike the
/// diff and comment navigator, this intentionally ignores file-tree and commit
/// filters: the summary is the place to audit all work that is still local.
pub fn render_summary(frame: &mut Frame, app: &mut App, area: Rect) {
    let pending = collect_pending_comments(app);
    let pending_count = pending.len();
    let title =
        format!(" Pending Comments ({pending_count}) — j/k select, Enter to jump, Esc to return ");
    let block = Block::default()
        .title(title)
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .style(styles::panel_style(&app.theme))
        .border_style(styles::border_style(&app.theme, true));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let SummaryContent {
        lines,
        comment_ranges,
        targets,
    } = summary_lines(app, &pending, inner.width as usize);
    drop(pending);

    // The summary replaces the diff renderer, so keep the shared view geometry
    // current for annotation wrapping and the subsequent Enter jump.
    app.diff_state.viewport_height = inner.height as usize;
    app.diff_inner_area = Some(inner);
    app.sync_viewport_width(inner.width as usize);

    app.update_summary_layout(comment_ranges, targets, lines.len(), inner.height as usize);

    let can_scroll_up = app.summary_state.scroll_offset > 0;
    let can_scroll_down = app.summary_state.scroll_offset + app.summary_state.viewport_height
        < app.summary_state.total_lines;
    let selected_comment = app.summary_state.selected_comment;
    let selected_style = styles::selected_style(&app.theme);
    let visible_lines: Vec<Line> = lines
        .into_iter()
        .skip(app.summary_state.scroll_offset)
        .take(app.summary_state.viewport_height)
        .map(|rendered| {
            if rendered.comment_idx == Some(selected_comment) {
                rendered.line.style(selected_style)
            } else {
                rendered.line
            }
        })
        .collect();

    frame.render_widget(
        Paragraph::new(visible_lines).style(styles::panel_style(&app.theme)),
        inner,
    );

    let indicator_style = styles::help_indicator_style(&app.theme);
    if can_scroll_up {
        let indicator = Paragraph::new(Line::from(Span::styled("▲ more", indicator_style)));
        frame.render_widget(
            indicator,
            Rect {
                x: inner.x + inner.width.saturating_sub(8),
                y: inner.y,
                width: 7,
                height: 1,
            },
        );
    }
    if can_scroll_down {
        let indicator = Paragraph::new(Line::from(Span::styled("▼ more", indicator_style)));
        frame.render_widget(
            indicator,
            Rect {
                x: inner.x + inner.width.saturating_sub(8),
                y: inner.y + inner.height.saturating_sub(1),
                width: 7,
                height: 1,
            },
        );
    }
}

fn collect_pending_comments(app: &App) -> Vec<PendingComment<'_>> {
    let mut pending = Vec::new();

    for comment in &app.session.review_comments {
        push_if_pending(
            &mut pending,
            "Review summary".to_string(),
            comment,
            app.comment_visible(comment)
                .then(|| SummaryCommentTarget::Review {
                    comment_id: comment.id.clone(),
                }),
        );
    }

    let mut files: Vec<_> = app.session.files.iter().collect();
    files.sort_by_key(|(path, _)| path.as_os_str().to_os_string());
    for (path, review) in files {
        let path_display = path.to_string_lossy().into_owned();
        let diff_file = app
            .diff_files
            .iter()
            .find(|file| file.display_path() == path);
        let file_is_visible = diff_file.is_some_and(|file| app.file_passes_filter(file));
        for comment in &review.file_comments {
            push_if_pending(
                &mut pending,
                path_display.clone(),
                comment,
                (file_is_visible && app.comment_visible(comment)).then(|| {
                    SummaryCommentTarget::File {
                        path: path.clone(),
                        comment_id: comment.id.clone(),
                    }
                }),
            );
        }

        let mut line_comments: Vec<_> = review.line_comments.iter().collect();
        line_comments.sort_by_key(|(line, _)| *line);
        for (line, comments) in line_comments {
            for comment in comments {
                let side = comment.side.unwrap_or_default();
                let line_is_visible = diff_file.is_some_and(|file| {
                    file.hunks.iter().any(|hunk| {
                        hunk.lines.iter().any(|diff_line| match side {
                            LineSide::Old => diff_line.old_lineno == Some(*line),
                            LineSide::New => diff_line.new_lineno == Some(*line),
                        })
                    })
                });
                push_if_pending(
                    &mut pending,
                    line_location(path, *line, comment),
                    comment,
                    (file_is_visible && line_is_visible && app.comment_visible(comment)).then(
                        || SummaryCommentTarget::Line {
                            path: path.clone(),
                            line: *line,
                            side,
                            comment_id: comment.id.clone(),
                        },
                    ),
                );
            }
        }
    }

    pending
}

fn push_if_pending<'a>(
    pending: &mut Vec<PendingComment<'a>>,
    location: String,
    comment: &'a Comment,
    target: Option<SummaryCommentTarget>,
) {
    if !comment.is_locked() {
        pending.push(PendingComment {
            location,
            comment,
            target,
        });
    }
}

fn line_location(path: &Path, line: u32, comment: &Comment) -> String {
    let range = comment
        .line_range
        .unwrap_or_else(|| crate::model::LineRange::single(line));
    let line = match (comment.side.unwrap_or_default(), range.is_single()) {
        (LineSide::Old, true) => format!("~{}", range.start),
        (LineSide::Old, false) => format!("~{}-~{}", range.start, range.end),
        (_, true) => range.start.to_string(),
        (_, false) => format!("{}-{}", range.start, range.end),
    };
    format!("{}:{line}", path.display())
}

fn summary_lines(app: &App, pending: &[PendingComment<'_>], width: usize) -> SummaryContent {
    if pending.is_empty() {
        return SummaryContent {
            lines: vec![SummaryRenderedLine::plain(Line::from(Span::styled(
                "No pending comments.",
                styles::dim_style(&app.theme),
            )))],
            comment_ranges: Vec::new(),
            targets: Vec::new(),
        };
    }

    let mut lines = Vec::new();
    let mut comment_ranges = Vec::with_capacity(pending.len());
    let mut targets = Vec::with_capacity(pending.len());
    for (idx, item) in pending.iter().enumerate() {
        let start_line = lines.len();
        let mut header = vec![Span::styled(
            format!("{}. ", idx + 1),
            Style::default()
                .fg(app.theme.fg_secondary)
                .add_modifier(Modifier::BOLD),
        )];
        let label = app.comment_type_label(&item.comment.comment_type);
        if !label.is_empty() {
            header.push(Span::styled(
                format!("[{label}] "),
                styles::comment_type_style(
                    &app.theme,
                    app.comment_type_color(&item.comment.comment_type),
                ),
            ));
        }
        let location_style = if item.target.is_some() {
            Style::default()
                .fg(app.theme.comment_note)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(app.theme.fg_primary)
                .add_modifier(Modifier::BOLD)
        };
        header.push(Span::styled(item.location.clone(), location_style));
        if item.comment.author != app.username {
            header.push(Span::styled(
                format!(" · @{}", item.comment.author),
                styles::dim_style(&app.theme),
            ));
        }
        if let Some(commit_id) = item.comment.commit_id.as_deref() {
            header.push(Span::styled(
                format!(" · commit {}", short_sha(commit_id)),
                styles::dim_style(&app.theme),
            ));
        }
        push_wrapped_header(&mut lines, header, width, idx);
        push_comment_body(&mut lines, &item.comment.content, width, idx);
        comment_ranges.push((start_line, lines.len()));
        targets.push(item.target.clone());
        lines.push(SummaryRenderedLine::plain(Line::default()));
    }
    SummaryContent {
        lines,
        comment_ranges,
        targets,
    }
}

fn push_comment_body(
    lines: &mut Vec<SummaryRenderedLine>,
    content: &str,
    width: usize,
    comment_idx: usize,
) {
    let body_width = width.saturating_sub(2).max(1);
    for source_line in content.split('\n') {
        let body = [Span::raw(source_line.to_string())];
        for mut row in wrap_spans(&body, body_width) {
            let mut spans = vec![Span::raw("  ")];
            spans.append(&mut row);
            lines.push(SummaryRenderedLine::for_comment(
                Line::from(spans),
                comment_idx,
            ));
        }
    }
}

fn push_wrapped_header(
    lines: &mut Vec<SummaryRenderedLine>,
    spans: Vec<Span<'static>>,
    width: usize,
    comment_idx: usize,
) {
    for row in wrap_spans(&spans, width.max(1)) {
        lines.push(SummaryRenderedLine::for_comment(
            Line::from(row),
            comment_idx,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};

    use crate::app::{AnnotatedLine, DiffSource, DiffViewMode, FocusedPanel, InputMode};
    use crate::error::{Result as TuicrResult, TuicrError};
    use crate::handler::{handle_mouse_event, handle_summary_action};
    use crate::input::Action;
    use crate::model::comment::CommentLifecycleState;
    use crate::model::review::FileReview;
    use crate::model::{
        CommentType, DiffFile, DiffHunk, DiffLine, FileStatus, LineOrigin, LineRange,
        ReviewSession, SessionDiffSource,
    };
    use crate::syntax::SyntaxHighlighter;
    use crate::theme::Theme;
    use crate::vcs::traits::{CommitInfo, VcsBackend, VcsChangeStatus, VcsInfo, VcsType};

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
            _path: &Path,
            _status: FileStatus,
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
            _path: &Path,
            _status: FileStatus,
            _ref_commit: Option<&str>,
        ) -> TuicrResult<u32> {
            Ok(0)
        }
    }

    fn make_app_with_files(diff_files: Vec<DiffFile>) -> App {
        let info = VcsInfo {
            root_path: PathBuf::from("/tmp"),
            head_commit: "abcdef0123".to_string(),
            branch_name: Some("main".to_string()),
            vcs_type: VcsType::File,
        };
        let session = ReviewSession::new(
            info.root_path.clone(),
            info.head_commit.clone(),
            info.branch_name.clone(),
            SessionDiffSource::WorkingTree,
        );
        App::build(
            Box::new(SnapshotVcs { info: info.clone() }),
            info,
            Theme::dark(),
            None,
            false,
            diff_files,
            session,
            DiffSource::WorkingTree,
            InputMode::Summary,
            Vec::new(),
            None,
            None,
        )
        .expect("build app")
    }

    fn make_app() -> App {
        make_app_with_files(Vec::new())
    }

    fn make_hunk(start: u32, count: u32) -> DiffHunk {
        let lines = (0..count)
            .map(|offset| DiffLine {
                origin: LineOrigin::Context,
                content: format!("line {}", start + offset),
                old_lineno: Some(start + offset),
                new_lineno: Some(start + offset),
                highlighted_spans: None,
            })
            .collect();
        DiffHunk {
            header: format!("@@ -{start},{count} +{start},{count} @@"),
            lines,
            old_start: start,
            old_count: count,
            new_start: start,
            new_count: count,
        }
    }

    fn make_file(path: &str, hunks: Vec<DiffHunk>) -> DiffFile {
        let content_hash = DiffFile::compute_content_hash(&hunks);
        DiffFile {
            old_path: None,
            new_path: Some(PathBuf::from(path)),
            status: FileStatus::Modified,
            hunks,
            is_binary: false,
            is_too_large: false,
            is_commit_message: false,
            content_hash,
        }
    }

    fn app_with_line_comments(contents: &[&str]) -> (App, PathBuf, Vec<String>) {
        let path = PathBuf::from("src/app/mod.rs");
        let mut app = make_app_with_files(vec![make_file(
            path.to_str().expect("test path"),
            vec![make_hunk(1482, 2)],
        )]);
        let comments: Vec<_> = contents
            .iter()
            .map(|content| {
                Comment::new(
                    (*content).to_string(),
                    CommentType::from_id("note"),
                    Some(LineSide::New),
                )
            })
            .collect();
        let ids = comments.iter().map(|comment| comment.id.clone()).collect();
        app.session
            .files
            .get_mut(&path)
            .expect("registered review file")
            .line_comments
            .insert(1482, comments);
        app.rebuild_annotations();
        (app, path, ids)
    }

    fn draw_summary(app: &mut App, width: u16, height: u16) -> Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| render_summary(frame, app, frame.area()))
            .expect("draw summary");
        terminal.backend().buffer().clone()
    }

    fn draw_app(app: &mut App, width: u16, height: u16) -> Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| crate::ui::render(frame, app))
            .expect("draw app");
        terminal.backend().buffer().clone()
    }

    fn buffer_text(buffer: &Buffer) -> String {
        let mut output = String::new();
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                output.push_str(buffer[(x, y)].symbol());
            }
            output.push('\n');
        }
        output
    }

    fn find_text_cell(buffer: &Buffer, needle: &str) -> (u16, u16) {
        let symbols: Vec<String> = needle.chars().map(|ch| ch.to_string()).collect();
        for y in buffer.area.y..buffer.area.y + buffer.area.height {
            for x in buffer.area.x..buffer.area.x + buffer.area.width {
                if x as usize + symbols.len() > (buffer.area.x + buffer.area.width) as usize {
                    break;
                }
                if symbols
                    .iter()
                    .enumerate()
                    .all(|(offset, symbol)| buffer[(x + offset as u16, y)].symbol() == symbol)
                {
                    return (x, y);
                }
            }
        }
        panic!("could not find {needle:?} in rendered buffer");
    }

    fn assert_selected_comment_visible(app: &App) {
        let &(start, end) = app
            .summary_state
            .comment_ranges
            .get(app.summary_state.selected_comment)
            .expect("selected comment range");
        let viewport_height = app.summary_state.viewport_height;
        if end.saturating_sub(start) >= viewport_height {
            assert_eq!(app.summary_state.scroll_offset, start);
        } else {
            assert!(start >= app.summary_state.scroll_offset);
            assert!(end <= app.summary_state.scroll_offset + viewport_height);
        }
    }

    fn assert_selected_comment_intersects_viewport(app: &App) {
        let &(start, end) = app
            .summary_state
            .comment_ranges
            .get(app.summary_state.selected_comment)
            .expect("selected comment range");
        let viewport_start = app.summary_state.scroll_offset;
        let viewport_end = viewport_start + app.summary_state.viewport_height;
        assert!(end > viewport_start && start < viewport_end);
    }

    #[test]
    fn renders_empty_state() {
        let mut app = make_app();

        let text = buffer_text(&draw_summary(&mut app, 100, 24));

        assert!(text.contains("Pending Comments (0)"));
        assert!(text.contains("No pending comments."));
        assert_eq!(app.summary_state.selected_comment, 0);
        assert!(app.summary_state.comment_ranges.is_empty());
        assert!(app.summary_state.targets.is_empty());

        handle_summary_action(&mut app, Action::CursorDown(1));
        handle_summary_action(&mut app, Action::CursorUp(1));
        handle_summary_action(&mut app, Action::SubmitInput);
        assert_eq!(app.input_mode, InputMode::Summary);
        assert_eq!(app.summary_state.scroll_offset, 0);
    }

    #[test]
    fn summary_replaces_diff_pane_and_preserves_open_file_sidebar() {
        let (mut app, _path, _ids) = app_with_line_comments(&["pending note"]);
        app.show_file_list = true;
        app.diff_state.max_content_width = usize::MAX;
        app.diff_row_to_annotation = vec![usize::MAX];

        let buffer = draw_app(&mut app, 120, 30);
        let text = buffer_text(&buffer);

        assert_eq!(app.diff_area, Some(Rect::new(24, 1, 96, 28)));
        assert_eq!(app.diff_inner_area, Some(Rect::new(25, 2, 94, 26)));
        assert_eq!(app.summary_state.viewport_height, 26);
        let (files_x, files_y) = find_text_cell(&buffer, "Files");
        assert!(files_x < 24);
        assert_eq!(files_y, 1);
        let (summary_x, summary_y) = find_text_cell(&buffer, "Pending Comments (1)");
        assert!(summary_x >= 24);
        assert_eq!(summary_y, 1);
        assert!(!text.contains("line 1483"));
        // These are normally rewritten by either diff renderer. Keeping them
        // proves the summary replaced the diff rather than covering it later.
        assert_eq!(app.diff_state.max_content_width, usize::MAX);
        assert_eq!(app.diff_row_to_annotation, vec![usize::MAX]);
    }

    #[test]
    fn summary_uses_entire_content_area_when_file_sidebar_is_hidden() {
        let mut app = make_app_with_files(vec![make_file("src/lib.rs", vec![make_hunk(42, 2)])]);
        app.show_file_list = false;
        app.session.review_comments.push(Comment::new(
            "x".repeat(90),
            CommentType::from_id("note"),
            None,
        ));

        let buffer = draw_app(&mut app, 100, 24);
        let text = buffer_text(&buffer);

        assert_eq!(app.file_list_area, None);
        assert_eq!(app.diff_area, Some(Rect::new(0, 1, 100, 22)));
        assert_eq!(app.diff_inner_area, Some(Rect::new(1, 2, 98, 20)));
        assert_eq!(app.diff_state.viewport_width, 98);
        assert_eq!(app.diff_state.viewport_height, 20);
        assert_eq!(app.summary_state.viewport_height, 20);
        let (_, summary_y) = find_text_cell(&buffer, "Pending Comments (1)");
        assert_eq!(summary_y, 1);
        assert_eq!(app.summary_state.comment_ranges[0], (0, 2));
        assert!(!text.contains("line 43"));
    }

    #[test]
    fn summary_replaces_inline_commit_selector() {
        let mut app = make_app();
        app.show_file_list = false;
        app.show_commit_selector = true;
        app.diff_source = DiffSource::CommitRange(vec!["aaa".to_string(), "bbb".to_string()]);
        app.review_commits = ["aaa", "bbb"]
            .into_iter()
            .map(|id| CommitInfo {
                id: id.to_string(),
                short_id: id.to_string(),
                branch_name: None,
                summary: format!("commit {id}"),
                body: None,
                author: "tester".to_string(),
                time: chrono::Utc::now(),
            })
            .collect();
        app.commit_list_inner_area = Some(Rect::new(1, 2, 10, 3));

        let buffer = draw_app(&mut app, 100, 24);
        let text = buffer_text(&buffer);

        assert!(app.has_inline_commit_selector());
        assert_eq!(app.commit_list_inner_area, None);
        assert_eq!(app.diff_area, Some(Rect::new(0, 1, 100, 22)));
        let (_, summary_y) = find_text_cell(&buffer, "Pending Comments (0)");
        assert_eq!(summary_y, 1);
        assert!(!text.contains("Commits"));
        assert!(!text.contains("commit aaa"));
    }

    #[test]
    fn renders_all_pending_buckets_in_deterministic_order_and_skips_locked_comments() {
        let mut app = make_app();
        app.session.review_comments.push(Comment::new(
            "overall review".to_string(),
            CommentType::from_id("praise"),
            None,
        ));

        let mut z_review = FileReview::new(PathBuf::from("z.rs"), FileStatus::Modified, 0);
        z_review.file_comments.push(Comment::new(
            "file note".to_string(),
            CommentType::from_id("note"),
            None,
        ));
        app.session.files.insert(PathBuf::from("z.rs"), z_review);

        let mut a_review = FileReview::new(PathBuf::from("a.rs"), FileStatus::Modified, 0);
        let mut range = Comment::new_with_range(
            "old range".to_string(),
            CommentType::from_id("issue"),
            Some(LineSide::Old),
            LineRange::new(10, 12),
        );
        range.commit_id = Some("0123456789abcdef".to_string());
        a_review.line_comments.insert(10, vec![range]);
        let mut locked = Comment::new(
            "already pushed".to_string(),
            CommentType::from_id("suggestion"),
            Some(LineSide::New),
        );
        locked.lifecycle_state = CommentLifecycleState::PushedDraft;
        a_review.line_comments.insert(5, vec![locked]);
        app.session.files.insert(PathBuf::from("a.rs"), a_review);

        let text = buffer_text(&draw_summary(&mut app, 120, 30));

        assert!(text.contains("Pending Comments (3)"), "popup: {text}");
        assert!(text.contains("[PRAISE] Review summary"));
        assert!(text.contains("[ISSUE] a.rs:~10-~12 · commit 0123456"));
        assert!(text.contains("[NOTE] z.rs"));
        assert!(!text.contains("already pushed"));
        let review_pos = text.find("overall review").expect("review comment");
        let line_pos = text.find("old range").expect("line comment");
        let file_pos = text.find("file note").expect("file comment");
        assert!(
            review_pos < line_pos && line_pos < file_pos,
            "popup: {text}"
        );
    }

    #[test]
    fn updates_scroll_metrics_and_renders_more_indicator() {
        let mut app = make_app();
        for idx in 0..20 {
            app.session.review_comments.push(Comment::new(
                format!("comment {idx}"),
                CommentType::from_id("note"),
                None,
            ));
        }

        let text = buffer_text(&draw_summary(&mut app, 80, 12));

        assert!(app.summary_state.total_lines > app.summary_state.viewport_height);
        assert!(text.contains("▼ more"));
    }

    #[test]
    fn opening_summary_selects_and_highlights_first_comment() {
        let (mut app, _path, _ids) = app_with_line_comments(&["first body", "second body"]);
        app.summary_state.selected_comment = 1;
        app.summary_state.scroll_offset = usize::MAX;

        app.enter_summary_mode();
        let buffer = draw_summary(&mut app, 120, 30);

        assert_eq!(app.summary_state.selected_comment, 0);
        assert_eq!(app.summary_state.scroll_offset, 0);
        assert_eq!(app.summary_state.targets.len(), 2);
        let first = find_text_cell(&buffer, "first body");
        let second = find_text_cell(&buffer, "second body");
        assert_eq!(buffer[first].bg, app.theme.bg_highlight);
        assert_ne!(buffer[second].bg, app.theme.bg_highlight);
    }

    #[test]
    fn j_and_k_select_comments_and_scroll_variable_height_entries_into_view() {
        let mut app = make_app();
        for content in [
            "first line\nsecond line\nthird line\nfourth line".to_string(),
            "short second".to_string(),
            "a wrapped third comment ".repeat(12),
            "last comment".to_string(),
        ] {
            app.session.review_comments.push(Comment::new(
                content,
                CommentType::from_id("note"),
                None,
            ));
        }
        app.enter_summary_mode();
        draw_summary(&mut app, 70, 12);
        assert_eq!(app.summary_state.selected_comment, 0);
        assert!(app.summary_state.comment_ranges[0].1 - app.summary_state.comment_ranges[0].0 > 2);

        for expected in 1..app.summary_state.comment_ranges.len() {
            handle_summary_action(&mut app, Action::CursorDown(1));
            assert_eq!(app.summary_state.selected_comment, expected);
            assert_selected_comment_visible(&app);
            draw_summary(&mut app, 70, 12);
        }
        assert!(app.summary_state.scroll_offset > 0);
        handle_summary_action(&mut app, Action::CursorDown(1));
        assert_eq!(app.summary_state.selected_comment, 3);

        for expected in (0..3).rev() {
            handle_summary_action(&mut app, Action::CursorUp(1));
            assert_eq!(app.summary_state.selected_comment, expected);
            assert_selected_comment_visible(&app);
            draw_summary(&mut app, 70, 12);
        }
        assert_eq!(app.summary_state.scroll_offset, 0);
        handle_summary_action(&mut app, Action::CursorUp(1));
        assert_eq!(app.summary_state.selected_comment, 0);
    }

    #[test]
    fn page_wheel_and_top_bottom_navigation_keep_a_visible_selection() {
        let mut app = make_app();
        for idx in 0..8 {
            app.session.review_comments.push(Comment::new(
                format!("comment {idx}"),
                CommentType::from_id("note"),
                None,
            ));
        }
        app.enter_summary_mode();
        draw_summary(&mut app, 70, 12);

        handle_summary_action(&mut app, Action::PageDown);
        assert!(app.summary_state.selected_comment > 0);
        assert_selected_comment_intersects_viewport(&app);

        let after_page = app.summary_state.selected_comment;
        handle_summary_action(&mut app, Action::MouseScrollDown(3));
        assert!(app.summary_state.selected_comment >= after_page);
        assert_selected_comment_intersects_viewport(&app);

        handle_summary_action(&mut app, Action::PageUp);
        assert_selected_comment_intersects_viewport(&app);

        handle_summary_action(&mut app, Action::GoToTop);
        handle_summary_action(&mut app, Action::CursorDown(2));
        assert_eq!(app.summary_state.selected_comment, 2);
        handle_summary_action(&mut app, Action::MouseScrollDown(1));
        assert_eq!(app.summary_state.selected_comment, 2);
        assert_selected_comment_intersects_viewport(&app);

        handle_summary_action(&mut app, Action::GoToBottom);
        assert_eq!(app.summary_state.selected_comment, 7);
        assert_selected_comment_visible(&app);

        handle_summary_action(&mut app, Action::GoToTop);
        assert_eq!(app.summary_state.selected_comment, 0);
        assert_selected_comment_visible(&app);
    }

    #[test]
    fn enter_on_second_comment_at_same_line_jumps_to_exact_comment() {
        let (mut app, _path, ids) = app_with_line_comments(&["first", "second"]);
        // Simulate a resize while the summary is open. The summary renderer
        // must refresh annotation wrapping even though the diff is not drawn.
        app.diff_state.viewport_width = 13;
        app.rebuild_annotations();
        draw_summary(&mut app, 120, 30);
        assert_eq!(app.diff_state.viewport_width, 118);
        handle_summary_action(&mut app, Action::CursorDown(1));
        assert!(matches!(
            app.summary_state.targets.get(1),
            Some(Some(SummaryCommentTarget::Line { comment_id, .. })) if comment_id == &ids[1]
        ));

        handle_summary_action(&mut app, Action::SubmitInput);

        assert_eq!(app.input_mode, InputMode::Normal);
        assert_eq!(app.focused_panel, FocusedPanel::Diff);
        assert!(matches!(
            app.line_annotations.get(app.diff_state.cursor_line),
            Some(AnnotatedLine::LineComment {
                file_idx: 0,
                line: 1482,
                side: LineSide::New,
                comment_idx: 1,
            })
        ));
    }

    #[test]
    fn enter_on_comment_in_reviewed_file_and_hunk_reveals_without_clearing_marks() {
        for diff_view_mode in [DiffViewMode::Unified, DiffViewMode::SideBySide] {
            for file_reviewed in [false, true] {
                let (mut app, path, _ids) = app_with_line_comments(&["reviewed comment"]);
                app.diff_view_mode = diff_view_mode;
                app.is_single_file_view = true;
                let hunk_key = app.diff_files[0]
                    .hunk_review_key(0)
                    .expect("hunk review key");
                {
                    let review = app
                        .session
                        .files
                        .get_mut(&path)
                        .expect("registered review file");
                    review.reviewed = file_reviewed;
                    review.reviewed_hunks.insert(hunk_key.clone());
                }
                app.rebuild_annotations();
                assert!(
                    !app.line_annotations
                        .iter()
                        .any(|annotation| matches!(annotation, AnnotatedLine::LineComment { .. }))
                );
                draw_summary(&mut app, 120, 30);

                handle_summary_action(&mut app, Action::SubmitInput);

                assert_eq!(app.input_mode, InputMode::Normal);
                assert!(!app.is_single_file_view);
                assert_eq!(app.session.is_file_reviewed(&path), file_reviewed);
                assert!(app.session.is_hunk_reviewed(&path, &hunk_key));
                assert_eq!(
                    app.revealed_reviewed_file.as_ref(),
                    file_reviewed.then_some(&path)
                );
                assert!(app.line_annotations.iter().any(|annotation| matches!(
                    annotation,
                    AnnotatedLine::FileHeader { file_idx: 0 }
                )));
                assert!(!app.line_annotations.iter().any(|annotation| matches!(
                    annotation,
                    AnnotatedLine::ReviewedBanner { file_idx: 0 }
                )));
                assert!(matches!(
                    app.line_annotations.get(app.diff_state.cursor_line),
                    Some(AnnotatedLine::LineComment {
                        file_idx: 0,
                        line: 1482,
                        side: LineSide::New,
                        comment_idx: 0,
                    })
                ));
                assert_eq!(app.line_annotations.len(), app.total_lines());
            }
        }
    }

    #[test]
    fn stale_target_failure_restores_view_state_and_hidden_comments_cannot_activate() {
        let (mut app, path, ids) = app_with_line_comments(&["scoped comment"]);
        let hunk_key = app.diff_files[0]
            .hunk_review_key(0)
            .expect("hunk review key");
        {
            let review = app
                .session
                .files
                .get_mut(&path)
                .expect("registered review file");
            review.reviewed = true;
            review.reviewed_hunks.insert(hunk_key.clone());
        }
        app.is_single_file_view = true;
        draw_summary(&mut app, 120, 30);
        assert!(matches!(
            app.summary_state.targets.first(),
            Some(Some(SummaryCommentTarget::Line { comment_id, .. })) if comment_id == &ids[0]
        ));

        app.session
            .files
            .get_mut(&path)
            .unwrap()
            .line_comments
            .get_mut(&1482)
            .unwrap()[0]
            .commit_id = Some("bbb".to_string());
        app.review_commits = vec![CommitInfo {
            id: "aaa".to_string(),
            short_id: "aaa".to_string(),
            branch_name: None,
            summary: "selected".to_string(),
            body: None,
            author: "tester".to_string(),
            time: chrono::Utc::now(),
        }];
        app.commit_selection_range = Some((0, 0));

        handle_summary_action(&mut app, Action::SubmitInput);

        assert_eq!(app.input_mode, InputMode::Summary);
        assert!(app.is_single_file_view);
        assert!(app.revealed_reviewed_file.is_none());
        assert!(app.revealed_reviewed_hunk.is_none());
        assert!(app.session.is_file_reviewed(&path));
        assert!(app.session.is_hunk_reviewed(&path, &hunk_key));

        draw_summary(&mut app, 120, 30);
        assert!(matches!(app.summary_state.targets.first(), Some(None)));
        handle_summary_action(&mut app, Action::SubmitInput);
        assert_eq!(app.input_mode, InputMode::Summary);
    }

    #[test]
    fn left_click_on_summary_location_does_not_activate_comment() {
        let (mut app, _path, _ids) = app_with_line_comments(&["first"]);
        let buffer = draw_summary(&mut app, 120, 30);
        let (column, row) = find_text_cell(&buffer, "src/app/mod.rs:1482");
        let cursor_before = app.diff_state.cursor_line;
        let focus_before = app.focused_panel;

        handle_mouse_event(
            &mut app,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column,
                row,
                modifiers: KeyModifiers::NONE,
            },
        );

        assert_eq!(app.input_mode, InputMode::Summary);
        assert_eq!(app.summary_state.selected_comment, 0);
        assert_eq!(app.diff_state.cursor_line, cursor_before);
        assert_eq!(app.focused_panel, focus_before);
    }
}
