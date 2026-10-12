use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem},
};
use unicode_width::UnicodeWidthStr;

use crate::app::{App, CommentNavigatorItem, CommentNavigatorKind, FocusedPanel};
use crate::ui::diff_view::apply_horizontal_scroll;
use crate::ui::styles;

pub(super) fn render_comment_navigator(
    frame: &mut Frame,
    app: &mut App,
    area: Rect,
    items: &[CommentNavigatorItem],
) {
    let focused = app.focused_panel == FocusedPanel::Comments;
    let block = Block::default()
        .borders(Borders::ALL)
        .style(styles::panel_style(&app.theme))
        .border_style(styles::border_style(&app.theme, focused));

    let inner = block.inner(area);
    app.comment_navigator_inner_area = Some(inner);
    app.comment_navigator_state.viewport_width = inner.width as usize;
    app.comment_navigator_state.viewport_height = inner.height as usize;
    app.sync_comment_navigator_selection(items);
    // The title is built after the sync so it names the row drawn as selected.
    let block = block.title(navigator_title(
        app.comment_navigator_state.list_state.selected(),
        items.len(),
    ));

    let row_lines: Vec<Line> = items
        .iter()
        .map(|item| render_comment_row(app, item))
        .collect();
    let max_content_width = row_lines.iter().map(line_width).max().unwrap_or_default();
    app.comment_navigator_state.max_content_width = max_content_width;

    let max_scroll_x = max_content_width.saturating_sub(inner.width as usize);
    if app.comment_navigator_state.scroll_x > max_scroll_x {
        app.comment_navigator_state.scroll_x = max_scroll_x;
    }
    let scroll_x = app.comment_navigator_state.scroll_x;

    let rows: Vec<ListItem> = row_lines
        .into_iter()
        .map(|line| ListItem::new(apply_horizontal_scroll(line, scroll_x)))
        .collect();

    let list = List::new(rows)
        .style(styles::panel_style(&app.theme))
        .highlight_style(styles::selected_style(&app.theme))
        .block(block);

    frame.render_stateful_widget(list, area, &mut app.comment_navigator_state.list_state);
}

/// `Comments · 17/38`: the selected row's 1-based position, then the total.
/// The pane has no scrollbar, so this is the only cue for how far through
/// the list the selection is.
fn navigator_title(selected: Option<usize>, total: usize) -> String {
    match selected {
        Some(idx) if idx < total => format!(" Comments · {}/{} ", idx + 1, total),
        _ => format!(" Comments · {total} "),
    }
}

/// The widest title `total` comments can produce. The layout sizes the
/// sidebar to it, so moving the selection never resizes the panel.
pub(super) fn widest_navigator_title(total: usize) -> String {
    navigator_title(total.checked_sub(1), total)
}

fn render_comment_row(app: &App, item: &CommentNavigatorItem) -> Line<'static> {
    let (marker, marker_style) = match &item.kind {
        CommentNavigatorKind::Local(comment_type) => {
            let label = app.comment_type_label(comment_type);
            // `None` comments have an empty label; show a neutral bullet marker.
            let marker = label.chars().next().unwrap_or('•').to_string();
            (
                marker,
                styles::comment_type_style(&app.theme, app.comment_type_color(comment_type)),
            )
        }
        CommentNavigatorKind::Remote { muted } => {
            let style = if *muted {
                styles::dim_style(&app.theme)
            } else {
                Style::default()
                    .fg(app.theme.diff_hunk_header)
                    .add_modifier(Modifier::BOLD)
            };
            ("R".to_string(), style)
        }
    };

    let dim_style = styles::dim_style(&app.theme);
    let location = comment_location(item);

    let author_accent = item
        .author
        .as_deref()
        .and_then(|author| styles::author_accent(&app.username, author));

    let mut spans = vec![Span::styled(marker, marker_style), Span::raw(" ")];
    if let Some(color) = author_accent {
        spans.push(Span::styled(
            "● ".to_string(),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ));
    }
    spans.push(Span::styled(location, dim_style));
    Line::from(spans)
}

fn line_width(line: &Line<'_>) -> usize {
    line.spans.iter().map(|span| span.content.width()).sum()
}

fn comment_location(item: &CommentNavigatorItem) -> String {
    let Some(path) = item.path.as_deref() else {
        return "review".to_string();
    };

    let mut location = path.to_string();
    if let Some(line) = item.line {
        location.push(':');
        location.push_str(&line.to_string());
    }
    location
}

#[cfg(test)]
mod tests {
    use super::{navigator_title, widest_navigator_title};
    use crate::app::{App, DiffSource, FocusedPanel, InputMode};
    use crate::model::{
        Comment, CommentType, DiffFile, DiffLine, FileStatus, ReviewSession, SessionDiffSource,
    };
    use crate::vcs::traits::{VcsBackend, VcsInfo, VcsType};
    use ratatui::{Terminal, backend::TestBackend};
    use std::path::PathBuf;

    struct StubVcs(VcsInfo);
    impl VcsBackend for StubVcs {
        fn info(&self) -> &VcsInfo {
            &self.0
        }
        fn get_working_tree_diff(
            &self,
            _hl: &crate::syntax::SyntaxHighlighter,
        ) -> crate::error::Result<Vec<DiffFile>> {
            Ok(Vec::new())
        }
        fn fetch_context_lines(
            &self,
            _path: &std::path::Path,
            _status: FileStatus,
            _ref_commit: Option<&str>,
            _start: u32,
            _end: u32,
        ) -> crate::error::Result<Vec<DiffLine>> {
            Ok(Vec::new())
        }
        fn file_line_count(
            &self,
            _path: &std::path::Path,
            _status: FileStatus,
            _ref_commit: Option<&str>,
        ) -> crate::error::Result<u32> {
            Ok(0)
        }
    }

    /// An app over `paths` with one file-level comment on each, so the
    /// navigator lists one row per file.
    fn app_with_file_comments(paths: &[&str]) -> App {
        let vcs_info = VcsInfo {
            root_path: PathBuf::from("/tmp"),
            head_commit: "head".into(),
            branch_name: Some("main".into()),
            vcs_type: VcsType::Git,
        };
        let session = ReviewSession::new(
            vcs_info.root_path.clone(),
            vcs_info.head_commit.clone(),
            vcs_info.branch_name.clone(),
            SessionDiffSource::WorkingTree,
        );
        let files = paths
            .iter()
            .map(|path| DiffFile {
                old_path: None,
                new_path: Some(PathBuf::from(path)),
                status: FileStatus::Modified,
                hunks: vec![],
                is_binary: false,
                is_too_large: false,
                is_commit_message: false,
                content_hash: 0,
            })
            .collect();
        let mut app = App::build(
            Box::new(StubVcs(vcs_info.clone())),
            vcs_info,
            crate::theme::Theme::dark(),
            None,
            false,
            files,
            session,
            DiffSource::WorkingTree,
            InputMode::Normal,
            Vec::new(),
            None,
            None,
        )
        .expect("build app");
        for path in paths {
            app.session
                .files
                .get_mut(&PathBuf::from(path))
                .expect("file registered in the session")
                .file_comments
                .push(Comment::new(
                    format!("note on {path}"),
                    CommentType::from_id("note"),
                    None,
                ));
        }
        app.rebuild_annotations();
        app
    }

    #[test]
    fn should_title_the_navigator_with_the_selected_comment_position() {
        let mut app = app_with_file_comments(&["a.rs", "b.rs", "c.rs"]);
        app.focused_panel = FocusedPanel::Comments;

        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let mut screen = |app: &mut App| {
            terminal
                .draw(|frame| crate::ui::render(frame, app))
                .unwrap();
            terminal.backend().to_string()
        };

        assert!(screen(&mut app).contains("Comments · 1/3"));
        app.comment_navigator_down(1);
        assert!(screen(&mut app).contains("Comments · 2/3"));
        app.comment_navigator_down(5);
        assert!(screen(&mut app).contains("Comments · 3/3"));
    }

    #[test]
    fn should_show_the_selected_position_before_the_total() {
        assert_eq!(navigator_title(Some(0), 38), " Comments · 1/38 ");
        assert_eq!(navigator_title(Some(16), 38), " Comments · 17/38 ");
        assert_eq!(navigator_title(Some(37), 38), " Comments · 38/38 ");
    }

    #[test]
    fn should_show_only_the_total_without_a_valid_selection() {
        assert_eq!(navigator_title(None, 38), " Comments · 38 ");
        assert_eq!(navigator_title(Some(38), 38), " Comments · 38 ");
    }

    #[test]
    fn should_measure_the_title_at_the_last_position() {
        assert_eq!(widest_navigator_title(38), " Comments · 38/38 ");
        assert_eq!(widest_navigator_title(9), " Comments · 9/9 ");
    }

    #[test]
    fn should_keep_the_sidebar_width_while_the_selection_moves() {
        // ` Comments · 1/10 ` is a column narrower than ` Comments · 10/10 `,
        // and both outgrow the file rows and the ` Files · 0/10 ` title.
        let paths: Vec<String> = (0..10).map(|i| format!("{i}.rs")).collect();
        let paths: Vec<&str> = paths.iter().map(String::as_str).collect();
        let mut app = app_with_file_comments(&paths);
        app.focused_panel = FocusedPanel::Comments;
        let mut terminal = Terminal::new(TestBackend::new(200, 40)).unwrap();

        terminal
            .draw(|frame| crate::ui::render(frame, &mut app))
            .unwrap();
        let first = app.comment_navigator_area.expect("navigator drawn");
        assert!(terminal.backend().to_string().contains("Comments · 1/10"));
        app.comment_navigator_down(9);
        terminal
            .draw(|frame| crate::ui::render(frame, &mut app))
            .unwrap();

        assert!(terminal.backend().to_string().contains("Comments · 10/10"));
        assert_eq!(app.comment_navigator_area, Some(first));
        assert_eq!(first.width, " Comments · 10/10 ".chars().count() as u16 + 2);
    }
}
