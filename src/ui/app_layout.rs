use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    widgets::Block,
};

use crate::app::{App, InputMode};
use crate::ui::comment_navigator::render_comment_navigator;
use crate::ui::diff_view::render_diff_view;
use crate::ui::file_list::render_file_list;
use crate::ui::inline_commit_selector::render_inline_commit_selector;
use crate::ui::selector::render_commit_select;
use crate::ui::{comment_panel, help_popup, status_bar, styles, submit_modals, summary_popup};

const FILE_LIST_MIN_HEIGHT: u16 = 4;
const COMMENT_NAVIGATOR_MIN_HEIGHT: u16 = 4;
const COMMENT_NAVIGATOR_MAX_HEIGHT: u16 = 12;

pub fn render(frame: &mut Frame, app: &mut App) {
    // Color what this frame is about to show, before a single line is built. Order
    // matters here: see `App::color_visible_hunks` for why.
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

    // Render confirm dialog if in confirm mode
    if app.input_mode == InputMode::Confirm {
        comment_panel::render_confirm_dialog(frame, app, "Copy review to clipboard?");
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
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(20), // File list
                Constraint::Percentage(80), // Diff view
            ])
            .split(content_area);

        let comment_items = app.build_comment_navigator_items();
        if !comment_items.is_empty()
            && chunks[0].height >= FILE_LIST_MIN_HEIGHT + COMMENT_NAVIGATOR_MIN_HEIGHT
        {
            let available_comment_height = chunks[0].height.saturating_sub(FILE_LIST_MIN_HEIGHT);
            let max_comment_height = COMMENT_NAVIGATOR_MAX_HEIGHT.min(available_comment_height);
            let desired_comment_height = comment_items.len() as u16 + 2;
            let comment_height = desired_comment_height
                .min(max_comment_height)
                .max(COMMENT_NAVIGATOR_MIN_HEIGHT);
            let left_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Min(FILE_LIST_MIN_HEIGHT),
                    Constraint::Length(comment_height),
                ])
                .split(chunks[0]);

            app.file_list_area = Some(left_chunks[0]);
            app.comment_navigator_area = Some(left_chunks[1]);
            render_file_list(frame, app, left_chunks[0]);
            render_comment_navigator(frame, app, left_chunks[1], &comment_items);
        } else {
            app.file_list_area = Some(chunks[0]);
            app.comment_navigator_area = None;
            app.comment_navigator_inner_area = None;
            render_file_list(frame, app, chunks[0]);
        }
        render_content_view(frame, app, chunks[1]);
    } else {
        app.file_list_area = None;
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

#[cfg(test)]
mod visible_coloring_tests {
    //! Coloring must reach every hunk a frame shows before the frame is built. See
    //! `App::color_visible_hunks` for what a half-colored frame looks like.

    use super::render;
    use crate::app::{AnnotatedLine, App};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    /// Height of the `TestBackend` every test in this module renders into.
    const TEST_TERMINAL_HEIGHT: u16 = 24;

    fn draw(app: &mut App) {
        let mut terminal =
            Terminal::new(TestBackend::new(120, TEST_TERMINAL_HEIGHT)).expect("terminal");
        terminal
            .draw(|frame| render(frame, app))
            .expect("draw frame");
    }

    /// Pulls the `(file_idx, hunk_idx)` a diff annotation belongs to, or `None` for an
    /// annotation that isn't part of a hunk (headers, comments, expanders, and so on).
    fn hunk_ref(annotation: &AnnotatedLine) -> Option<(usize, usize)> {
        match annotation {
            AnnotatedLine::DiffLine {
                file_idx, hunk_idx, ..
            }
            | AnnotatedLine::SideBySideLine {
                file_idx, hunk_idx, ..
            }
            | AnnotatedLine::HunkHeader { file_idx, hunk_idx } => Some((*file_idx, *hunk_idx)),
            // Named exhaustively, mirroring `App::visible_hunk_indices` and for the
            // reason given there.
            AnnotatedLine::PrInfoLine { .. }
            | AnnotatedLine::IssueCommentsHeader
            | AnnotatedLine::IssueComment { .. }
            | AnnotatedLine::ReviewCommentsHeader
            | AnnotatedLine::ReviewComment { .. }
            | AnnotatedLine::RemoteReviewSummaryLine { .. }
            | AnnotatedLine::FileHeader { .. }
            | AnnotatedLine::ReviewedBanner { .. }
            | AnnotatedLine::FileComment { .. }
            | AnnotatedLine::Expander { .. }
            | AnnotatedLine::HiddenLines { .. }
            | AnnotatedLine::ExpandedContext { .. }
            | AnnotatedLine::LineComment { .. }
            | AnnotatedLine::RemoteThreadLine { .. }
            | AnnotatedLine::BinaryOrEmpty { .. }
            | AnnotatedLine::Spacing => None,
        }
    }

    /// Asserts every hunk-bearing annotation in `app.line_annotations[start..end]` is
    /// colored, and returns how many such annotations it found. A caller that expects
    /// coverage should also check the count is nonzero, since an empty range would pass
    /// this vacuously.
    fn assert_range_colored(app: &App, start: usize, end: usize) -> usize {
        let mut checked = 0;
        for annotation in &app.line_annotations[start..end] {
            let Some((file_idx, hunk_idx)) = hunk_ref(annotation) else {
                continue;
            };
            let hunk = &app.diff_files[file_idx].hunks[hunk_idx];
            assert!(
                hunk.lines.iter().all(|l| !l.coloring.is_pending()),
                "hunk {hunk_idx} of file {file_idx} is on screen but was not colored \
                 before the frame was built"
            );
            checked += 1;
        }
        checked
    }

    /// Regression guard for the argument `color_visible_hunks` is called with.
    /// `diff_state.viewport_height` reads zero until render writes it, so passing
    /// that instead of `frame.area().height` would color nothing on the first
    /// frame. The mutation check for this test is to make that swap and confirm
    /// the assertion below catches it.
    #[test]
    fn should_color_every_hunk_the_first_frame_shows() {
        let mut app = crate::ui::row_height::tests::make_app();
        assert!(
            app.diff_files
                .iter()
                .flat_map(|f| f.hunks.iter())
                .all(|h| h.lines.iter().all(|l| l.coloring.is_pending())),
            "fixture must start uncolored, or this test proves nothing"
        );

        draw(&mut app);

        let height = app.diff_state.viewport_height.max(1);
        let start = app.diff_state.scroll_offset.min(app.line_annotations.len());
        let end = start.saturating_add(height).min(app.line_annotations.len());
        let checked = assert_range_colored(&app, start, end);
        assert!(
            checked > 0,
            "no diff rows were visible, so the test asserted nothing"
        );
    }

    /// A diff several screens tall, so the coloring window cannot cover all of it.
    fn tall_diff(files: usize) -> Vec<crate::model::DiffFile> {
        use crate::model::{DiffFile, DiffHunk, DiffLine, FileStatus, LineColoring, LineOrigin};
        (0..files)
            .map(|i| {
                let hunks = vec![DiffHunk {
                    header: format!("@@ -1,3 +1,3 @@ file {i}"),
                    lines: (0..3)
                        .map(|n| DiffLine {
                            origin: LineOrigin::Context,
                            content: format!("pub fn item_{i}_{n}() -> u32 {{ {n} }}"),
                            old_lineno: Some(n + 1),
                            new_lineno: Some(n + 1),
                            coloring: LineColoring::Pending,
                        })
                        .collect(),
                    old_start: 1,
                    old_count: 3,
                    new_start: 1,
                    new_count: 3,
                }];
                let content_hash = DiffFile::compute_content_hash(&hunks);
                DiffFile {
                    old_path: None,
                    new_path: Some(std::path::PathBuf::from(format!("src/file_{i}.rs"))),
                    status: FileStatus::Modified,
                    hunks,
                    is_binary: false,
                    is_too_large: false,
                    is_commit_message: false,
                    content_hash,
                }
            })
            .collect()
    }

    /// Picks the hunk by its `HunkHeader` rather than by whatever sits a screen below
    /// the offset. A hunk's lines follow its header contiguously, so a header past that
    /// boundary puts the whole hunk in the margin. Picking by position can land
    /// mid-hunk, which `color_hunk` colors whole, and the assertion would then pass
    /// with no margin at all.
    #[test]
    fn should_color_one_screen_beyond_the_viewport() {
        let mut app = crate::ui::row_height::tests::make_app_with(tall_diff(60));
        draw(&mut app);

        // Matches the `TestBackend` height in `draw`, which is what `color_visible_hunks`
        // receives.
        let height = TEST_TERMINAL_HEIGHT as usize;
        let offset = app.diff_state.scroll_offset;
        let margin_start = offset.saturating_add(height);
        let margin_end = offset
            .saturating_add(height * 2)
            .min(app.line_annotations.len());
        let Some(margin) = app.line_annotations.get(margin_start..margin_end) else {
            return; // fixture is shorter than two screens; nothing to assert
        };

        let Some((file_idx, hunk_idx)) = margin.iter().find_map(|a| match a {
            AnnotatedLine::HunkHeader { file_idx, hunk_idx } => Some((*file_idx, *hunk_idx)),
            _ => None,
        }) else {
            return; // no hunk starts cleanly inside the margin; nothing to assert
        };

        assert!(
            app.diff_files[file_idx].hunks[hunk_idx]
                .lines
                .iter()
                .all(|l| !l.coloring.is_pending()),
            "hunk {hunk_idx} of file {file_idx} sits one screen below the viewport and \
             should have been colored ahead, so scrolling onto it does not stall"
        );
    }

    /// The forward margin has its own test above; this is its mirror. Scrolling up
    /// is just as much an ordinary move as scrolling down or paging, so the hunks one
    /// screen behind the offset need to already be colored, or scrolling up stalls on
    /// them the same way scrolling down would stall without the forward margin.
    ///
    /// Sets `scroll_offset` directly to a point several screens into a tall fixture,
    /// rather than reaching it by scrolling frame by frame, so the first (and only)
    /// coloring pass has to get the backward margin right on its own.
    #[test]
    fn should_color_one_screen_behind_the_scroll_offset() {
        let mut app = crate::ui::row_height::tests::make_app_with(tall_diff(80));

        let height = TEST_TERMINAL_HEIGHT as usize;
        let offset = (height * 3).min(app.line_annotations.len().saturating_sub(1));
        app.diff_state.scroll_offset = offset;

        draw(&mut app);

        let margin_start = app.diff_state.scroll_offset.saturating_sub(height);
        let margin_end = app.diff_state.scroll_offset;
        let Some(margin) = app.line_annotations.get(margin_start..margin_end) else {
            return; // fixture is shorter than the margin; nothing to assert
        };

        let Some((file_idx, hunk_idx)) = margin.iter().find_map(|a| match a {
            AnnotatedLine::HunkHeader { file_idx, hunk_idx } => Some((*file_idx, *hunk_idx)),
            _ => None,
        }) else {
            return; // no hunk starts cleanly inside the margin; nothing to assert
        };

        assert!(
            app.diff_files[file_idx].hunks[hunk_idx]
                .lines
                .iter()
                .all(|l| !l.coloring.is_pending()),
            "hunk {hunk_idx} of file {file_idx} sits one screen behind the scroll offset \
             and should have been colored ahead, so scrolling up onto it does not stall"
        );
    }

    /// Uses a fixture taller than the coloring window (viewport plus one screen either
    /// side) and checks that a hunk far outside it is still `Pending`. Every other test
    /// here only checks that visible hunks got colored, so a regression that colored every
    /// hunk unconditionally would leave them all green.
    #[test]
    fn should_leave_hunks_outside_the_viewport_uncolored() {
        let mut app = crate::ui::row_height::tests::make_app_with(tall_diff(60));
        draw(&mut app);

        let total: usize = app.diff_files.iter().map(|f| f.hunks.len()).sum();
        let colored = app
            .diff_files
            .iter()
            .flat_map(|f| f.hunks.iter())
            .filter(|h| h.lines.iter().all(|l| !l.coloring.is_pending()))
            .count();

        assert!(
            colored < total,
            "all {total} hunks were colored for one frame, so nothing is being saved"
        );
    }

    /// `scroll_comment_input_into_view` (called by both diff renderers, after
    /// `color_visible_hunks` has already run for the frame) can move `scroll_offset`
    /// on its own, to keep an open comment box on screen. Opening a file-level
    /// comment on a file far outside the window that was just colored forces exactly
    /// that jump, the same class of move a multi-line paste growing the box would
    /// cause: the box's position pulls the viewport to a file nothing had colored
    /// yet. `render_diff_view`'s retry is what is supposed to catch this before the
    /// frame is drawn.
    #[test]
    fn should_color_hunks_revealed_by_comment_input_auto_scroll() {
        let mut app = crate::ui::row_height::tests::make_app_with(tall_diff(80));
        draw(&mut app);

        let far_file = 60;
        assert!(
            app.diff_files[far_file]
                .hunks
                .iter()
                .all(|h| h.lines.iter().all(|l| l.coloring.is_pending())),
            "far file must start uncolored, or this test proves nothing"
        );
        assert_eq!(
            app.diff_state.scroll_offset, 0,
            "far file must start outside the viewport, or opening a comment there \
             would not need to auto-scroll"
        );

        // Opens a file-level comment box on `far_file`. Setting `current_file_idx`
        // directly isolates the `scroll_comment_input_into_view` path without needing a
        // real paste to grow a box that started on screen.
        app.diff_state.current_file_idx = far_file;
        app.enter_comment_mode(true, None);
        app.comment_buffer = "note".to_string();
        app.comment_cursor = app.comment_buffer.len();

        draw(&mut app);

        assert_ne!(
            app.diff_state.scroll_offset, 0,
            "comment auto-scroll did not move the viewport, so this test exercised \
             nothing"
        );

        let height = app.diff_state.viewport_height.max(1);
        let start = app.diff_state.scroll_offset.min(app.line_annotations.len());
        let end = start.saturating_add(height).min(app.line_annotations.len());
        let checked = assert_range_colored(&app, start, end);
        assert!(
            checked > 0,
            "no diff rows were visible, so the test asserted nothing"
        );
    }
}
