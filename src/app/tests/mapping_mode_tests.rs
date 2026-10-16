use crate::app::*;
use crate::input::mappings::MapMode;
use crate::model::{DiffFile, DiffLine, FileStatus};
use crate::vcs::traits::{VcsBackend, VcsInfo, VcsType};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
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

fn app() -> App {
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
    App::build(
        Box::new(StubVcs(vcs_info.clone())),
        vcs_info,
        crate::theme::Theme::dark(),
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
    .expect("build app")
}

fn comment_app(vim: bool) -> App {
    let mut app = app();
    app.comment_vim_enabled = vim;
    app.input_mode = InputMode::Comment;
    app.ensure_comment_vim_editor();
    app
}

fn press_esc(app: &mut App) {
    app.comment_vim_feed_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
}

#[test]
fn should_use_normal_mappings_in_the_review() {
    assert_eq!(app().mapping_mode(), Some(MapMode::Normal));
}

#[test]
fn should_use_insert_mappings_in_the_comment_box() {
    assert_eq!(comment_app(false).mapping_mode(), Some(MapMode::Insert));
    assert_eq!(comment_app(true).mapping_mode(), Some(MapMode::Insert));
}

#[test]
fn should_skip_mappings_in_vim_normal_mode_of_the_comment_box() {
    let mut app = comment_app(true);
    press_esc(&mut app);
    assert!(app.comment_vim_in_normal_mode());
    assert_eq!(app.mapping_mode(), None);
}

#[test]
fn should_use_command_line_mappings_in_the_vim_comment_prompt() {
    let mut app = comment_app(true);
    press_esc(&mut app);
    app.start_comment_vim_command();
    assert_eq!(app.mapping_mode(), Some(MapMode::CommandLine));
}

#[test]
fn should_skip_mappings_in_help() {
    let mut app = app();
    app.input_mode = InputMode::Help;
    assert_eq!(app.mapping_mode(), None);
}
