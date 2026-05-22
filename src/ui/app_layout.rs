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
use crate::ui::{comment_panel, help_popup, status_bar, styles, submit_modals};

const FILE_LIST_MIN_HEIGHT: u16 = 4;
const COMMENT_NAVIGATOR_MIN_HEIGHT: u16 = 4;
const COMMENT_NAVIGATOR_MAX_HEIGHT: u16 = 12;

pub fn render(frame: &mut Frame, app: &mut App) {
    frame.render_widget(
        Block::default().style(styles::panel_style(&app.theme)),
        frame.area(),
    );

    // Special handling for commit selection mode
    if app.input_mode == InputMode::CommitSelect {
        render_commit_select(frame, app);
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

    // Render help popup on top if in help mode
    if app.input_mode == InputMode::Help {
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
    let content_area = if app.has_inline_commit_selector() {
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
        app.diff_area = Some(chunks[1]);

        render_diff_view(frame, app, chunks[1]);
    } else {
        app.file_list_area = None;
        app.comment_navigator_area = None;
        app.comment_navigator_inner_area = None;
        app.diff_area = Some(content_area);

        render_diff_view(frame, app, content_area);
    }
}
