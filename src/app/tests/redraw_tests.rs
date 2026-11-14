use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::render_perf_tests::{app_with, file};
use crate::app::*;
use crate::handler::handle_global_key;

fn ctrl_l() -> KeyEvent {
    KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL)
}

#[test]
fn should_not_request_a_full_redraw_by_default() {
    let mut app = app_with(vec![file("a.rs", 3)]);

    assert!(!app.take_full_redraw());
}

#[test]
fn should_request_a_full_redraw_once_on_ctrl_l() {
    let mut app = app_with(vec![file("a.rs", 3)]);

    assert!(handle_global_key(&mut app, ctrl_l()));

    assert!(app.take_full_redraw());
    assert!(
        !app.take_full_redraw(),
        "the flag must clear once taken, or every frame repaints in full"
    );
}

#[test]
fn should_redraw_on_ctrl_l_without_typing_into_a_comment() {
    // Comment mode's catch-all arm inserts any modified char, so without the
    // global check Ctrl-l would type an `l`.
    let mut app = app_with(vec![file("a.rs", 3)]);
    app.enter_comment_mode(true, None);
    app.comment_buffer = "nit".to_string();
    app.comment_cursor = 3;

    assert!(handle_global_key(&mut app, ctrl_l()));

    assert_eq!(app.input_mode, InputMode::Comment);
    assert_eq!(app.comment_buffer, "nit");
    assert!(app.take_full_redraw());
}

#[test]
fn should_leave_other_keys_to_the_mode_maps() {
    let mut app = app_with(vec![file("a.rs", 3)]);

    let plain_l = KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE);
    assert!(!handle_global_key(&mut app, plain_l));

    assert!(!app.take_full_redraw());
}
