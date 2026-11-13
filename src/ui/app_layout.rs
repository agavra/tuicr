use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    widgets::Block,
};

use crate::app::{App, ConfirmAction, FileListPosition, InputMode};
use crate::ui::comment_navigator::render_comment_navigator;
use crate::ui::diff_view::render_diff_view;
use crate::ui::file_list::render_file_list;
use crate::ui::inline_commit_selector::render_inline_commit_selector;
use crate::ui::selector::render_commit_select;
use crate::ui::{
    comment_panel, file_picker, help_popup, status_bar, styles, submit_modals, summary_popup,
    theme_picker,
};

const FILE_LIST_MIN_HEIGHT: u16 = 4;
const COMMENT_NAVIGATOR_MIN_HEIGHT: u16 = 4;
const COMMENT_NAVIGATOR_MAX_HEIGHT: u16 = 12;

pub fn render(frame: &mut Frame, app: &mut App) {
    // Color what this frame shows before any line is built; see
    // `App::color_visible_hunks`.
    app.color_visible_hunks(frame.area().height as usize);

    frame.render_widget(
        Block::default().style(styles::panel_style(&app.theme)),
        frame.area(),
    );

    if app.input_mode == InputMode::MessageDetails {
        help_popup::render_message_details(frame, app);
        return;
    }

    let selector_background = app.input_mode == InputMode::CommitSelect
        || (app.input_mode == InputMode::Confirm
            && app.pending_confirm.map(ConfirmAction::return_mode)
                == Some(InputMode::CommitSelect))
        || (app.input_mode == InputMode::Command
            && app.command_return_mode == InputMode::CommitSelect)
        || (app.input_mode == InputMode::Help
            && app.overlay_return_mode == InputMode::CommitSelect)
        || (app.searching_help() && app.overlay_return_mode == InputMode::CommitSelect);
    if selector_background {
        render_commit_select(frame, app);
        let area = frame.area();
        let footer = Rect::new(
            area.x,
            area.bottom().saturating_sub(1),
            area.width,
            area.height.min(1),
        );
        if matches!(app.input_mode, InputMode::Command | InputMode::Search) {
            status_bar::render_status_bar(frame, app, footer);
            status_bar::render_command_completion_popup(frame, app, footer);
        }
        if app.input_mode == InputMode::Help || app.searching_help() {
            help_popup::render_help(frame, app);
        }
        if let Some(confirm) = active_confirm(app) {
            comment_panel::render_confirm_dialog(frame, app, confirm.prompt());
        }
        return;
    }

    // Clear cursor position before rendering (will be set if in Comment mode)
    app.comment_cursor_screen_pos = None;

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![
            Constraint::Length(1), // Header
            Constraint::Min(0),    // Main content
            Constraint::Length(1), // Status bar (also shows command input in command mode)
        ])
        .split(frame.area());

    status_bar::render_header(frame, app, chunks[0]);
    render_main_content(frame, app, chunks[1]);
    status_bar::render_status_bar(frame, app, chunks[2]);
    status_bar::render_command_completion_popup(frame, app, chunks[2]);

    // Keep help visible while its search prompt is active.
    if app.input_mode == InputMode::Help || app.searching_help() {
        help_popup::render_help(frame, app);
    }

    // Comment input is now rendered inline in the diff view

    if let Some(confirm) = active_confirm(app) {
        comment_panel::render_confirm_dialog(frame, app, confirm.prompt());
    }

    // Submit-flow modals.
    if app.input_mode == InputMode::SubmitResolver {
        submit_modals::render_submit_resolver(frame, app);
    }
    if app.input_mode == InputMode::SubmitConfirm {
        submit_modals::render_submit_confirm(frame, app);
    }
    if app.input_mode == InputMode::SubmitActionPicker {
        submit_modals::render_submit_action_picker(frame, app);
    }
    if app.input_mode == InputMode::ThemePicker {
        theme_picker::render_theme_picker(frame, app);
    }
    if app.input_mode == InputMode::FilePicker {
        file_picker::render_file_picker(frame, app);
    }

    // Position terminal cursor for IME when in Comment mode
    // Always set a cursor position to prevent IME from showing at (0,0)
    if app.input_mode == InputMode::Comment {
        let (col, row) = app.comment_cursor_screen_pos.unwrap_or_else(|| {
            // Fallback: position cursor in the diff area or at a reasonable default
            // Use the diff area if available, otherwise use the main content area
            if let Some(diff_area) = app.diff_area {
                // Position at the start of the diff inner area (after border)
                (diff_area.x + 1, diff_area.y + 1)
            } else {
                // Last resort: position at the main content area
                (chunks[1].x + 1, chunks[1].y + 1)
            }
        });
        frame.set_cursor_position(ratatui::layout::Position { x: col, y: row });
    }
}

fn render_main_content(frame: &mut Frame, app: &mut App, area: Rect) {
    let content_area = if app.input_mode != InputMode::Summary && app.has_inline_commit_selector() {
        let selector_height = (app.review_commits.len() as u16 + 2).min(8); // N items + 2 borders, capped
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(selector_height), Constraint::Min(0)])
            .split(area);
        render_inline_commit_selector(frame, app, chunks[0]);
        chunks[1]
    } else {
        app.commit_list_inner_area = None;
        area
    };

    if app.show_file_list {
        const SIDEBAR: Constraint = Constraint::Percentage(20);
        const DIFF: Constraint = Constraint::Percentage(80);
        let (sidebar_area, diff_area) = match app.file_list_position {
            FileListPosition::Left => {
                let chunks = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([SIDEBAR, DIFF])
                    .split(content_area);
                (chunks[0], chunks[1])
            }
            FileListPosition::Right => {
                let chunks = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([DIFF, SIDEBAR])
                    .split(content_area);
                (chunks[1], chunks[0])
            }
        };

        let comment_items = app.build_comment_navigator_items();
        if !comment_items.is_empty()
            && sidebar_area.height >= FILE_LIST_MIN_HEIGHT + COMMENT_NAVIGATOR_MIN_HEIGHT
        {
            let available_comment_height = sidebar_area.height.saturating_sub(FILE_LIST_MIN_HEIGHT);
            let max_comment_height = COMMENT_NAVIGATOR_MAX_HEIGHT.min(available_comment_height);
            let desired_comment_height = comment_items.len() as u16 + 2;
            let comment_height = desired_comment_height
                .min(max_comment_height)
                .max(COMMENT_NAVIGATOR_MIN_HEIGHT);
            let sidebar_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Min(FILE_LIST_MIN_HEIGHT),
                    Constraint::Length(comment_height),
                ])
                .split(sidebar_area);

            app.file_list_area = Some(sidebar_chunks[0]);
            app.comment_navigator_area = Some(sidebar_chunks[1]);
            render_file_list(frame, app, sidebar_chunks[0]);
            render_comment_navigator(frame, app, sidebar_chunks[1], &comment_items);
        } else {
            app.file_list_area = Some(sidebar_area);
            app.comment_navigator_area = None;
            app.comment_navigator_inner_area = None;
            render_file_list(frame, app, sidebar_area);
        }
        render_content_view(frame, app, diff_area);
    } else {
        app.file_list_area = None;
        app.file_list_inner_area = None;
        app.comment_navigator_area = None;
        app.comment_navigator_inner_area = None;
        render_content_view(frame, app, content_area);
    }
}

fn render_content_view(frame: &mut Frame, app: &mut App, area: Rect) {
    app.diff_area = Some(area);
    if app.input_mode == InputMode::Summary {
        summary_popup::render_summary(frame, app, area);
    } else {
        render_diff_view(frame, app, area);
    }
}

/// The prompt to draw, if keys are currently routed to it. Gating on the mode
/// as well as `pending_confirm` keeps a prompt that outlived its mode (e.g. a
/// PR open landing mid-prompt) from being drawn where Y/N cannot reach it.
fn active_confirm(app: &App) -> Option<ConfirmAction> {
    (app.input_mode == InputMode::Confirm)
        .then_some(app.pending_confirm)
        .flatten()
}

#[cfg(test)]
mod tests {
    //! Layout checks for `file_list_position`: the sidebar renders on the
    //! configured side, and mouse hit-testing follows it.
    use crate::app::{App, DiffSource, DiffViewMode, FileListPosition, FocusedPanel, InputMode};
    use crate::handler::handle_mouse_event;
    use crate::model::{DiffFile, DiffLine, FileStatus, ReviewSession, SessionDiffSource};
    use crate::vcs::traits::{VcsBackend, VcsInfo, VcsType};
    use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
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

    fn file(path: &str) -> DiffFile {
        DiffFile {
            old_path: None,
            new_path: Some(PathBuf::from(path)),
            status: FileStatus::Modified,
            hunks: vec![],
            is_binary: false,
            is_too_large: false,
            is_commit_message: false,
            content_hash: 0,
        }
    }

    fn app_with(position: FileListPosition) -> App {
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
        let mut app = App::build(
            Box::new(StubVcs(vcs_info.clone())),
            vcs_info,
            crate::theme::Theme::dark(),
            None,
            false,
            ["a.rs", "b.rs", "c.rs"].iter().map(|p| file(p)).collect(),
            session,
            DiffSource::WorkingTree,
            InputMode::Normal,
            Vec::new(),
            None,
            None,
        )
        .expect("build app");
        app.show_file_list = true;
        app.file_list_position = position;
        app.focused_panel = FocusedPanel::Diff;
        app
    }

    fn draw(app: &mut App) -> Buffer {
        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| crate::ui::render(frame, app))
            .expect("draw frame");
        terminal.backend().buffer().clone()
    }

    fn column_of(buffer: &Buffer, needle: &str) -> u16 {
        for y in 0..buffer.area.height {
            let row: String = (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect();
            if let Some(byte) = row.find(needle) {
                return row[..byte].chars().count() as u16;
            }
        }
        panic!("{needle:?} not rendered");
    }

    fn mouse(app: &mut App, kind: MouseEventKind, x: u16, y: u16) {
        handle_mouse_event(
            app,
            MouseEvent {
                kind,
                column: x,
                row: y,
                modifiers: KeyModifiers::NONE,
            },
        );
    }

    #[test]
    fn file_list_renders_left_of_the_diff_by_default() {
        let mut app = app_with(FileListPosition::default());
        let buffer = draw(&mut app);

        let files = app.file_list_area.expect("file list drawn");
        let diff = app.diff_area.expect("diff drawn");
        assert_eq!(files.x, 0);
        assert_eq!(files.right(), diff.x);
        assert_eq!(diff.right(), 100);
        assert!(column_of(&buffer, "Files") < diff.x);
    }

    #[test]
    fn file_list_renders_right_of_the_diff_when_configured() {
        for mode in [DiffViewMode::Unified, DiffViewMode::SideBySide] {
            let mut app = app_with(FileListPosition::Right);
            app.diff_view_mode = mode;
            let buffer = draw(&mut app);

            let files = app.file_list_area.expect("file list drawn");
            let diff = app.diff_area.expect("diff drawn");
            assert_eq!(diff.x, 0, "{mode:?}");
            assert_eq!(diff.right(), files.x, "{mode:?}");
            assert_eq!(files.right(), 100, "{mode:?}");
            // Same 20/80 split as the left layout, just mirrored.
            assert_eq!(files.width, 20, "{mode:?}");
            assert!(column_of(&buffer, "Files") >= files.x, "{mode:?}");
            let inner = app.diff_inner_area.expect("diff inner drawn");
            assert!(inner.right() <= files.x, "{mode:?}");
        }
    }

    #[test]
    fn hidden_file_list_gives_the_diff_full_width_on_either_side() {
        for position in [FileListPosition::Left, FileListPosition::Right] {
            let mut app = app_with(position);
            draw(&mut app);
            assert!(app.file_list_inner_area.is_some(), "{position:?}");
            app.show_file_list = false;
            draw(&mut app);

            assert_eq!(app.file_list_area, None, "{position:?}");
            assert_eq!(app.file_list_inner_area, None, "{position:?}");
            assert_eq!(
                app.diff_area,
                Some(Rect::new(0, 1, 100, 22)),
                "{position:?}"
            );
        }
    }

    #[test]
    fn mouse_targets_follow_the_file_list_on_the_right() {
        let mut app = app_with(FileListPosition::Right);
        draw(&mut app);
        let files = app.file_list_area.expect("file list drawn");
        let diff = app.diff_area.expect("diff drawn");

        // A press on the right-hand sidebar's border focuses the file list ...
        let mid = files.y + files.height / 2;
        mouse(
            &mut app,
            MouseEventKind::Down(MouseButton::Left),
            files.x,
            mid,
        );
        assert_eq!(app.focused_panel, FocusedPanel::FileList);

        // ... and one on the left-hand diff's border focuses the diff.
        mouse(
            &mut app,
            MouseEventKind::Down(MouseButton::Left),
            diff.x,
            mid,
        );
        assert_eq!(app.focused_panel, FocusedPanel::Diff);

        // Clicking a file row in the right-hand tree selects that row.
        let inner = app.file_list_inner_area.expect("file list inner drawn");
        assert!(inner.x >= files.x);
        mouse(
            &mut app,
            MouseEventKind::Down(MouseButton::Left),
            inner.x + 1,
            inner.y + 1,
        );
        assert_eq!(app.file_list_state.selected(), 1);
    }

    // ---- Empty staged/unstaged reviews and rows ----

    fn row_text(buffer: &Buffer, y: u16) -> String {
        (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol().to_string())
            .collect()
    }

    /// The row holding `needle`, and the column it starts at.
    fn find(buffer: &Buffer, needle: &str) -> Option<(u16, u16)> {
        (0..buffer.area.height).find_map(|y| {
            let row = row_text(buffer, y);
            row.find(needle)
                .map(|byte| (row[..byte].chars().count() as u16, y))
        })
    }

    fn empty_review(source: DiffSource, mode: DiffViewMode) -> App {
        let mut app = app_with(FileListPosition::Left);
        app.diff_files.clear();
        app.diff_source = source;
        app.diff_view_mode = mode;
        app.rebuild_annotations();
        app
    }

    #[test]
    fn empty_staged_review_shows_a_dimmed_placeholder_in_both_layouts() {
        for mode in [DiffViewMode::Unified, DiffViewMode::SideBySide] {
            for (source, text) in [
                (DiffSource::Staged, "No staged changes"),
                (DiffSource::Unstaged, "No unstaged changes"),
                (
                    DiffSource::StagedAndUnstaged,
                    "No staged or unstaged changes",
                ),
            ] {
                let mut app = empty_review(source.clone(), mode);
                let buffer = draw(&mut app);

                let (x, y) = find(&buffer, text)
                    .unwrap_or_else(|| panic!("{mode:?} {source:?}: {text:?} not drawn"));
                let diff = app.diff_area.expect("diff drawn");
                assert!(
                    diff.x < x && x < diff.right(),
                    "{mode:?}: inside the diff pane"
                );
                assert_eq!(y, diff.y + diff.height / 2, "{mode:?}: on the middle row");
                assert_eq!(
                    buffer[(x, y)].fg,
                    app.theme.fg_dim,
                    "{mode:?}: the placeholder is dimmed"
                );
            }
        }
    }

    #[test]
    fn placeholder_is_not_drawn_when_the_review_has_files_or_another_source() {
        let mut app = app_with(FileListPosition::Left);
        app.diff_source = DiffSource::Staged;
        let buffer = draw(&mut app);
        assert!(find(&buffer, "No staged changes").is_none());

        // An empty working-tree review has no placeholder; only the staged
        // and unstaged rows can open empty.
        let mut app = empty_review(DiffSource::WorkingTree, DiffViewMode::Unified);
        let buffer = draw(&mut app);
        assert!(find(&buffer, "changes").is_none());
    }

    fn worktree_row(id: &str, summary: &str) -> crate::vcs::CommitInfo {
        crate::vcs::CommitInfo {
            id: id.to_string(),
            short_id: id.to_string(),
            branch_name: None,
            summary: summary.to_string(),
            body: None,
            author: String::new(),
            time: chrono::Utc::now(),
        }
    }

    fn pane_rows() -> Vec<crate::vcs::CommitInfo> {
        vec![
            worktree_row(crate::app::UNSTAGED_SELECTION_ID, "Unstaged changes"),
            worktree_row(crate::app::STAGED_SELECTION_ID, "Staged changes"),
            crate::vcs::CommitInfo {
                author: "alice".into(),
                ..worktree_row("abc1234", "add notes")
            },
        ]
    }

    /// Unstaged is empty, staged is not: only the unstaged row is dimmed and
    /// hinted.
    fn assert_only_unstaged_dimmed(app: &App, buffer: &Buffer) {
        let (x, y) = find(buffer, "Unstaged changes").expect("unstaged row drawn");
        assert!(
            row_text(buffer, y).contains(crate::ui::commit_row::EMPTY_ROW_HINT.trim()),
            "the empty row carries the hint"
        );
        assert_eq!(
            buffer[(x, y)].fg,
            app.theme.fg_dim,
            "the empty row is dimmed"
        );

        let (x, y) = find(buffer, "Staged changes").expect("staged row drawn");
        assert!(
            !row_text(buffer, y).contains("no changes"),
            "a row with changes has no hint"
        );
        assert_ne!(buffer[(x, y)].fg, app.theme.fg_dim);
    }

    fn status(staged: bool, unstaged: bool) -> Option<crate::vcs::traits::VcsChangeStatus> {
        Some(crate::vcs::traits::VcsChangeStatus { staged, unstaged })
    }

    #[test]
    fn target_selector_dims_and_hints_an_empty_worktree_row() {
        let mut app = app_with(FileListPosition::Left);
        app.commit_list = pane_rows();
        app.commit_list_cursor = 2;
        app.input_mode = InputMode::CommitSelect;
        app.worktree_status = status(true, false);

        let buffer = draw(&mut app);

        assert_only_unstaged_dimmed(&app, &buffer);
    }

    #[test]
    fn inline_commit_pane_dims_and_hints_an_empty_worktree_row() {
        let mut app = app_with(FileListPosition::Left);
        app.diff_source = DiffSource::StagedUnstagedAndCommits(vec!["abc1234".into()]);
        app.review_commits = pane_rows();
        app.commit_list = pane_rows();
        app.commit_list_cursor = 2;
        app.show_commit_selector = true;
        app.worktree_status = status(true, false);

        let buffer = draw(&mut app);

        assert!(app.has_inline_commit_selector());
        assert_only_unstaged_dimmed(&app, &buffer);
    }

    #[test]
    fn worktree_rows_are_not_dimmed_without_a_status_read() {
        let mut app = app_with(FileListPosition::Left);
        app.commit_list = pane_rows();
        app.input_mode = InputMode::CommitSelect;
        app.worktree_status = None;

        let buffer = draw(&mut app);

        assert!(find(&buffer, "no changes").is_none());
    }
}
