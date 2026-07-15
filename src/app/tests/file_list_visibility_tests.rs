use crate::app::*;
use crate::handler::{handle_diff_action, handle_file_list_action};
use crate::input::Action;
use crate::model::FileStatus;
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

fn build_app() -> App {
    let file = DiffFile {
        old_path: None,
        new_path: Some(PathBuf::from("test.rs")),
        status: FileStatus::Modified,
        hunks: vec![],
        is_binary: false,
        is_too_large: false,
        is_commit_message: false,
        content_hash: 0,
    };

    let vcs_info = VcsInfo {
        root_path: PathBuf::from("/tmp"),
        head_commit: "abc".to_string(),
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
        vec![file],
        session,
        DiffSource::WorkingTree,
        InputMode::Normal,
        Vec::new(),
        None,
        None,
    )
    .expect("failed to build test app")
}

#[test]
fn escape_in_diff_panel_keeps_file_list_visible() {
    let mut app = build_app();
    app.show_file_list = true;
    app.focused_panel = FocusedPanel::Diff;

    handle_diff_action(&mut app, Action::ExitMode);

    assert!(
        app.show_file_list,
        "Esc in the diff panel must not hide the file list"
    );
    assert_eq!(app.focused_panel, FocusedPanel::Diff);
}

#[test]
fn escape_in_file_list_panel_keeps_file_list_visible_and_focuses_diff() {
    let mut app = build_app();
    app.show_file_list = true;
    app.focused_panel = FocusedPanel::FileList;

    handle_file_list_action(&mut app, Action::ExitMode);

    assert!(
        app.show_file_list,
        "Esc in the file list panel must not hide the file list"
    );
    assert_eq!(
        app.focused_panel,
        FocusedPanel::Diff,
        "Esc returns focus to the diff"
    );
}

#[test]
fn leader_e_toggle_still_hides_and_shows_the_file_list() {
    let mut app = build_app();
    app.show_file_list = true;

    app.toggle_file_list();
    assert!(!app.show_file_list, "toggle hides the file list");

    app.toggle_file_list();
    assert!(app.show_file_list, "toggle shows the file list again");
}
