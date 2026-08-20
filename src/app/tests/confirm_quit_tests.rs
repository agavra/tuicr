//! Behavior of the `confirm_quit` config setting: whether a bare `q` in the
//! review view asks first, and the guarantee that `:q` is never gated by it.

use crate::app::*;
use crate::handler::{handle_command_action, handle_confirm_action, handle_diff_action};
use crate::input::Action;
use crate::model::{Comment, CommentType, FileStatus};
use crate::vcs::traits::VcsType;

struct DummyVcs {
    info: VcsInfo,
}

impl VcsBackend for DummyVcs {
    fn info(&self) -> &VcsInfo {
        &self.info
    }

    fn get_working_tree_diff(&self, _highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
        Err(TuicrError::NoChanges)
    }

    fn fetch_context_lines(
        &self,
        _file_path: &Path,
        _file_status: FileStatus,
        _ref_commit: Option<&str>,
        _start_line: u32,
        _end_line: u32,
    ) -> Result<Vec<DiffLine>> {
        Ok(Vec::new())
    }

    fn file_line_count(
        &self,
        _file_path: &Path,
        _file_status: FileStatus,
        _ref_commit: Option<&str>,
    ) -> Result<u32> {
        Ok(0)
    }
}

/// A review app with `confirm_quit` left at whatever `App::build` defaults it
/// to, so the default itself stays under test.
fn build_app() -> App {
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
        Box::new(DummyVcs {
            info: vcs_info.clone(),
        }),
        vcs_info,
        Theme::dark(),
        None,
        false,
        Vec::new(),
        session,
        DiffSource::WorkingTree,
        InputMode::Normal,
        Vec::new(),
        None,
        None,
    )
    .expect("failed to build test app")
}

fn build_app_with_confirm_quit() -> App {
    let mut app = build_app();
    app.confirm_quit = true;
    app
}

/// Type `command` at the `:` prompt and submit it, the way a user would.
fn run_command(app: &mut App, command: &str) {
    app.enter_command_mode();
    app.command_buffer.push_str(command);
    handle_command_action(app, Action::SubmitInput);
}

fn add_review_comment(app: &mut App) {
    app.session.review_comments.push(Comment::new(
        "note".to_string(),
        CommentType::from_id("note"),
        None,
    ));
}

#[test]
fn confirm_quit_defaults_to_off() {
    assert!(!build_app().confirm_quit);
}

#[test]
fn off_quits_on_the_first_bare_q() {
    // given: the default setting and a clean session
    let mut app = build_app();

    // when
    handle_diff_action(&mut app, Action::Quit);

    // then
    assert!(app.should_quit);
    assert_eq!(app.input_mode, InputMode::Normal);
}

#[test]
fn off_still_warns_once_when_unsaved_comments_exist() {
    // given: the one state the historical guard reacts to
    let mut app = build_app();
    add_review_comment(&mut app);
    app.dirty = true;

    // when: first `q`
    handle_diff_action(&mut app, Action::Quit);

    // then: warned, not quit
    assert!(!app.should_quit);
    assert!(app.quit_warned);
    assert_eq!(
        app.message.as_ref().map(|m| m.content.as_str()),
        Some("Unsaved changes. Press q again to quit.")
    );

    // when: second `q`
    handle_diff_action(&mut app, Action::Quit);

    // then
    assert!(app.should_quit);
}

#[test]
fn on_opens_the_quit_prompt_instead_of_quitting() {
    // given
    let mut app = build_app_with_confirm_quit();

    // when
    handle_diff_action(&mut app, Action::Quit);

    // then
    assert!(!app.should_quit);
    assert_eq!(app.input_mode, InputMode::Confirm);
    assert_eq!(app.pending_confirm, Some(ConfirmAction::Quit));
    assert_eq!(app.pending_confirm.unwrap().prompt(), "Quit tuicr?");
}

#[test]
fn on_quits_on_yes() {
    // given: the quit prompt is open
    let mut app = build_app_with_confirm_quit();
    handle_diff_action(&mut app, Action::Quit);

    // when
    handle_confirm_action(&mut app, Action::ConfirmYes);

    // then
    assert!(app.should_quit);
    assert_eq!(app.pending_confirm, None);
}

#[test]
fn on_returns_to_the_review_on_no() {
    // given: the quit prompt is open
    let mut app = build_app_with_confirm_quit();
    handle_diff_action(&mut app, Action::Quit);

    // when
    handle_confirm_action(&mut app, Action::ConfirmNo);

    // then: unlike the copy-on-exit prompt, "no" here means "don't quit"
    assert!(!app.should_quit);
    assert_eq!(app.input_mode, InputMode::Normal);
    assert_eq!(app.pending_confirm, None);
}

#[test]
fn copy_and_quit_prompt_still_quits_on_no() {
    // given: the `:wq` prompt, which only asks whether to export
    let mut app = build_app_with_confirm_quit();
    app.enter_confirm_mode(ConfirmAction::CopyAndQuit);

    // when
    handle_confirm_action(&mut app, Action::ConfirmNo);

    // then
    assert!(app.should_quit);
}

#[test]
fn colon_q_still_quits_when_confirm_quit_is_on() {
    // given
    let mut app = build_app_with_confirm_quit();

    // when
    run_command(&mut app, "q");

    // then: the explicit command is never gated
    assert!(app.should_quit);
}
