use super::render_perf_tests::{app_with, file};
use crate::app::*;
use crate::handler::handle_paste;
use crate::model::{LineRange, LineSide};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// An app with the diff focused and the cursor on new-side line 3 of `a.rs`.
fn app_on_line_3() -> App {
    let mut app = app_with(vec![file("a.rs", 6)]);
    app.focused_panel = FocusedPanel::Diff;
    app.go_to_source_line(3, LineSide::New);
    assert_eq!(app.get_line_at_cursor(), Some((3, LineSide::New)));
    app
}

#[test]
fn should_start_a_line_comment_with_the_pasted_text_on_a_diff_line() {
    let mut app = app_on_line_3();

    handle_paste(&mut app, "this should be a constant".to_string());

    assert_eq!(app.input_mode, InputMode::Comment);
    assert_eq!(app.comment_line, Some((3, LineSide::New)));
    assert_eq!(app.comment_line_range, None);
    assert!(!app.comment_is_file_level);
    assert_eq!(app.comment_buffer, "this should be a constant");
    assert_eq!(app.comment_cursor, app.comment_buffer.len());
}

#[test]
fn should_start_a_range_comment_with_the_pasted_text_on_a_selection() {
    let mut app = app_on_line_3();
    app.enter_visual_mode_at_cursor();
    app.cursor_down(2);
    app.extend_visual_to_cursor();

    handle_paste(&mut app, "extract these".to_string());

    assert_eq!(app.input_mode, InputMode::Comment);
    assert_eq!(
        app.comment_line_range,
        Some((LineRange { start: 3, end: 5 }, LineSide::New))
    );
    assert_eq!(app.comment_line, Some((5, LineSide::New)));
    assert_eq!(app.comment_buffer, "extract these");
    assert!(app.visual_selection.is_none());
}

#[test]
fn should_start_a_range_comment_on_a_mouse_selection() {
    // A mouse drag in the diff lands in VisualSelect with a selection that
    // need not start at column 0, which is the flow from the issue.
    let mut app = app_on_line_3();
    let anchor = app.diff_state.cursor_line;
    app.input_mode = InputMode::VisualSelect;
    app.visual_selection = Some(VisualSelection {
        anchor: SelPoint {
            annotation_idx: anchor,
            char_offset: 6,
            side: LineSide::New,
        },
        head: SelPoint {
            annotation_idx: anchor + 1,
            char_offset: 10,
            side: LineSide::New,
        },
    });

    handle_paste(&mut app, "rename".to_string());

    assert_eq!(app.input_mode, InputMode::Comment);
    assert_eq!(
        app.comment_line_range,
        Some((LineRange { start: 3, end: 4 }, LineSide::New))
    );
    assert_eq!(app.comment_buffer, "rename");
}

#[test]
fn should_keep_newlines_from_a_multi_line_paste() {
    let mut app = app_on_line_3();

    handle_paste(&mut app, "first point\r\nsecond point\nthird".to_string());

    assert_eq!(app.input_mode, InputMode::Comment);
    assert_eq!(app.comment_buffer, "first point\nsecond point\nthird");
}

#[test]
fn should_open_the_vim_comment_box_in_insert_mode_with_the_pasted_text() {
    let mut app = app_on_line_3();
    app.comment_vim_enabled = true;

    handle_paste(&mut app, "dictated".to_string());

    assert_eq!(app.input_mode, InputMode::Comment);
    assert_eq!(app.comment_buffer, "dictated");
    assert_eq!(
        app.comment_vim_mode_label(),
        Some(("INSERT".to_string(), false))
    );
    // Still in Insert mode: a typed character extends the text.
    app.comment_vim_feed_key(KeyEvent::new(KeyCode::Char('!'), KeyModifiers::NONE));
    assert_eq!(app.comment_buffer, "dictated!");
}

#[test]
fn should_ignore_a_paste_with_no_diff_line_under_the_cursor() {
    let mut app = app_on_line_3();
    // Row 0 is the file header, which `c` refuses as well.
    app.diff_state.cursor_line = 0;
    assert_eq!(app.get_line_at_cursor(), None);

    handle_paste(&mut app, "lost".to_string());

    assert_eq!(app.input_mode, InputMode::Normal);
    assert!(app.comment_buffer.is_empty());
    assert!(!app.session.has_comments());
}

#[test]
fn should_ignore_a_blank_paste() {
    let mut app = app_on_line_3();

    handle_paste(&mut app, " \n".to_string());

    assert_eq!(app.input_mode, InputMode::Normal);
}

#[test]
fn should_ignore_a_paste_while_the_file_list_is_focused() {
    let mut app = app_on_line_3();
    app.focused_panel = FocusedPanel::FileList;

    handle_paste(&mut app, "not here".to_string());

    assert_eq!(app.input_mode, InputMode::Normal);
    assert!(app.comment_buffer.is_empty());
}

#[test]
fn should_paste_into_an_open_file_tree_prompt() {
    let mut app = app_on_line_3();
    app.focused_panel = FocusedPanel::FileList;
    app.begin_file_tree_prompt(FileTreePrompt::Search);

    handle_paste(&mut app, "src/\nmain".to_string());

    assert_eq!(app.input_mode, InputMode::Normal);
    assert_eq!(app.file_tree_draft().unwrap().buffer, "src/main");
}

#[test]
fn should_keep_pasting_into_an_open_comment() {
    let mut app = app_on_line_3();
    app.enter_comment_mode(false, Some((3, LineSide::New)));
    app.comment_buffer = "typed ".to_string();
    app.comment_cursor = app.comment_buffer.len();

    handle_paste(&mut app, "then pasted".to_string());

    assert_eq!(app.comment_buffer, "typed then pasted");
    assert_eq!(app.comment_line, Some((3, LineSide::New)));
}

#[test]
fn should_leave_command_and_search_pastes_in_their_prompts() {
    let mut app = app_on_line_3();
    app.enter_command_mode();
    handle_paste(&mut app, "clip\n".to_string());
    assert_eq!(app.input_mode, InputMode::Command);
    assert_eq!(app.command_buffer, "clip");

    let mut app = app_on_line_3();
    app.enter_search_mode();
    handle_paste(&mut app, "needle".to_string());
    assert_eq!(app.input_mode, InputMode::Search);
    assert_eq!(app.search_buffer, "needle");
}

#[test]
fn should_ignore_a_paste_in_the_help_popup() {
    let mut app = app_on_line_3();
    app.toggle_help();
    assert_eq!(app.input_mode, InputMode::Help);

    handle_paste(&mut app, "ignored".to_string());

    assert_eq!(app.input_mode, InputMode::Help);
    assert!(app.comment_buffer.is_empty());
}
