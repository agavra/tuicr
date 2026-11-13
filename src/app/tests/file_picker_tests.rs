use crate::app::*;
use crate::handler::handle_file_picker_action;
use crate::input::keybindings::Action;
use crate::model::{DiffFile, DiffHunk, DiffLine, FileStatus, LineOrigin};
use crate::vcs::traits::{VcsBackend, VcsInfo, VcsType};
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

fn hunk() -> DiffHunk {
    DiffHunk {
        header: "@@ -1,2 +1,2 @@".to_string(),
        lines: vec![
            DiffLine {
                origin: LineOrigin::Context,
                content: "context".to_string(),
                old_lineno: Some(1),
                new_lineno: Some(1),
                raw_content: None,
                highlighted_spans: None,
                stage: None,
            },
            DiffLine {
                origin: LineOrigin::Addition,
                content: "added".to_string(),
                old_lineno: None,
                new_lineno: Some(2),
                raw_content: None,
                highlighted_spans: None,
                stage: None,
            },
        ],
        old_start: 1,
        old_count: 1,
        new_start: 1,
        new_count: 2,
    }
}

fn file(path: &str) -> DiffFile {
    let hunks = vec![hunk()];
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

fn app_with(paths: &[&str]) -> App {
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
        paths.iter().map(|p| file(p)).collect(),
        session,
        DiffSource::WorkingTree,
        InputMode::Normal,
        Vec::new(),
        None,
        None,
    )
    .expect("build app");
    // The tree starts collapsed; every assertion here is about which files
    // survive the filter, not about expand state.
    app.expand_all_dirs();
    app
}

fn index_of(app: &App, path: &str) -> usize {
    app.diff_files
        .iter()
        .position(|file| file.display_path().display().to_string() == path)
        .unwrap_or_else(|| panic!("no diff file for {path}"))
}

/// Display paths of the picker rows currently listed, in order.
fn listed(app: &App) -> Vec<String> {
    app.file_picker
        .filtered_indices()
        .into_iter()
        .map(|idx| app.file_picker.candidates[idx].1.clone())
        .collect()
}

/// Press `/`, type `pattern`, and leave the draft open.
fn type_filter(app: &mut App, pattern: &str) {
    handle_file_picker_action(app, Action::FilePickerFilter);
    for ch in pattern.chars() {
        handle_file_picker_action(app, Action::InsertChar(ch));
    }
}

fn apply_tree_filter(app: &mut App, prompt: FileTreePrompt, pattern: &str) {
    app.begin_file_tree_prompt(prompt);
    for ch in pattern.chars() {
        app.file_tree_prompt_insert_char(ch);
    }
    app.commit_file_tree_prompt();
}

const PATHS: &[&str] = &[
    "README.md",
    "src/main.rs",
    "src/app/tree.rs",
    "src/ui/file_list.rs",
    "tests/smoke.rs",
];

#[test]
fn should_open_on_every_visible_file_with_the_current_file_selected() {
    let mut app = app_with(PATHS);
    app.jump_to_file(index_of(&app, "src/app/tree.rs"));

    app.enter_file_picker_mode();

    assert_eq!(app.input_mode, InputMode::FilePicker);
    assert_eq!(listed(&app).len(), PATHS.len());
    assert_eq!(
        app.file_picker.selected_file_idx(),
        Some(index_of(&app, "src/app/tree.rs"))
    );
}

#[test]
fn should_narrow_the_list_while_the_filter_is_typed() {
    let mut app = app_with(PATHS);
    app.enter_file_picker_mode();

    type_filter(&mut app, "SRC/UI");

    assert!(app.file_picker_filtering());
    assert_eq!(listed(&app), vec!["src/ui/file_list.rs"]);
    assert_eq!(
        app.file_picker.selected_file_idx(),
        Some(index_of(&app, "src/ui/file_list.rs"))
    );
}

#[test]
fn should_jump_to_the_top_match_and_close_on_enter_in_the_filter() {
    let mut app = app_with(PATHS);
    app.enter_file_picker_mode();

    type_filter(&mut app, "tree");
    handle_file_picker_action(&mut app, Action::SubmitInput);

    assert_eq!(app.input_mode, InputMode::Normal);
    assert_eq!(
        app.diff_state.current_file_idx,
        index_of(&app, "src/app/tree.rs")
    );
}

#[test]
fn should_move_the_selection_with_arrows_while_filtering() {
    let mut app = app_with(PATHS);
    app.enter_file_picker_mode();
    type_filter(&mut app, "src");

    handle_file_picker_action(&mut app, Action::CursorDown(1));
    handle_file_picker_action(&mut app, Action::SubmitInput);

    assert_eq!(app.input_mode, InputMode::Normal);
    assert_eq!(
        app.diff_state.current_file_idx,
        index_of(&app, "src/app/tree.rs")
    );
}

#[test]
fn should_stay_open_on_enter_when_the_filter_matches_nothing() {
    let mut app = app_with(PATHS);
    app.enter_file_picker_mode();
    let file = app.diff_state.current_file_idx;

    type_filter(&mut app, "zzz");
    handle_file_picker_action(&mut app, Action::SubmitInput);

    assert_eq!(app.input_mode, InputMode::FilePicker);
    assert!(app.file_picker_filtering());
    assert_eq!(app.diff_state.current_file_idx, file);
    assert!(app.message.is_some());
}

#[test]
fn should_restore_the_previous_selection_when_the_draft_is_discarded() {
    let mut app = app_with(PATHS);
    app.enter_file_picker_mode();
    handle_file_picker_action(&mut app, Action::CursorDown(2));
    let before = app.file_picker.selected_file_idx();

    type_filter(&mut app, "smoke");
    handle_file_picker_action(&mut app, Action::ExitMode);

    assert_eq!(app.input_mode, InputMode::FilePicker);
    assert!(!app.file_picker_filtering());
    assert_eq!(listed(&app).len(), PATHS.len());
    assert_eq!(app.file_picker.selected_file_idx(), before);
}

#[test]
fn should_clamp_selection_movement_to_the_listed_rows() {
    let mut app = app_with(PATHS);
    app.enter_file_picker_mode();

    handle_file_picker_action(&mut app, Action::CursorUp(1));
    assert_eq!(app.file_picker.selected(), 0);
    handle_file_picker_action(&mut app, Action::CursorDown(99));
    assert_eq!(app.file_picker.selected(), PATHS.len() - 1);
}

#[test]
fn should_jump_to_the_selected_file_and_close_on_enter() {
    let mut app = app_with(PATHS);
    app.diff_state.viewport_height = 10;
    app.enter_file_picker_mode();
    handle_file_picker_action(&mut app, Action::CursorDown(99));

    handle_file_picker_action(&mut app, Action::SubmitInput);

    let target = index_of(&app, "tests/smoke.rs");
    assert_eq!(app.input_mode, InputMode::Normal);
    assert_eq!(app.focused_panel, FocusedPanel::Diff);
    assert_eq!(app.diff_state.current_file_idx, target);
    assert!(app.diff_state.cursor_line >= app.calculate_file_scroll_offset(target));
    assert!(app.diff_state.scroll_offset > 0);
    // The tree selection follows, as it does for the tree's own Enter.
    assert!(matches!(
        app.get_selected_tree_item(),
        Some(FileTreeItem::File { file_idx, .. }) if file_idx == target
    ));
}

#[test]
fn should_switch_the_shown_file_in_single_file_view() {
    let mut app = app_with(PATHS);
    app.toggle_single_file_view();
    app.enter_file_picker_mode();
    type_filter(&mut app, "main.rs");

    handle_file_picker_action(&mut app, Action::SubmitInput);

    assert!(app.is_single_file_view);
    assert_eq!(
        app.diff_state.current_file_idx,
        index_of(&app, "src/main.rs")
    );
}

#[test]
fn should_close_without_moving_anything_on_escape() {
    let mut app = app_with(PATHS);
    app.diff_state.viewport_height = 10;
    app.jump_to_file(index_of(&app, "src/main.rs"));
    let (cursor, scroll, file) = (
        app.diff_state.cursor_line,
        app.diff_state.scroll_offset,
        app.diff_state.current_file_idx,
    );
    app.enter_file_picker_mode();
    handle_file_picker_action(&mut app, Action::CursorDown(2));

    handle_file_picker_action(&mut app, Action::ExitMode);

    assert_eq!(app.input_mode, InputMode::Normal);
    assert_eq!(app.diff_state.cursor_line, cursor);
    assert_eq!(app.diff_state.scroll_offset, scroll);
    assert_eq!(app.diff_state.current_file_idx, file);
    assert!(app.file_picker.candidates.is_empty());
}

#[test]
fn should_leave_out_files_hidden_by_the_include_and_exclude_filters() {
    let mut app = app_with(PATHS);
    apply_tree_filter(&mut app, FileTreePrompt::Include, r"\.rs$");
    apply_tree_filter(&mut app, FileTreePrompt::Exclude, "^tests/");

    app.enter_file_picker_mode();

    assert_eq!(
        listed(&app),
        vec!["src/main.rs", "src/app/tree.rs", "src/ui/file_list.rs"]
    );
    // Candidates keep absolute `diff_files` indices.
    assert_eq!(
        app.file_picker.candidates[0].0,
        index_of(&app, "src/main.rs")
    );
}

#[test]
fn should_leave_out_reviewed_files_while_they_are_hidden() {
    let mut app = app_with(PATHS);
    app.toggle_reviewed_for_file_idx(index_of(&app, "src/main.rs"), false);
    app.set_show_reviewed(false);

    app.enter_file_picker_mode();

    assert!(!listed(&app).contains(&"src/main.rs".to_string()));
    assert_eq!(listed(&app).len(), PATHS.len() - 1);
}

#[test]
fn should_not_open_when_no_file_is_visible() {
    let mut app = app_with(PATHS);
    apply_tree_filter(&mut app, FileTreePrompt::Include, "nothing-matches");

    app.enter_file_picker_mode();

    assert_eq!(app.input_mode, InputMode::Normal);
    assert!(app.message.is_some());
}

#[test]
fn should_render_the_popup_with_paths_counts_and_filter() {
    use ratatui::{Terminal, backend::TestBackend};

    let mut app = app_with(PATHS);
    app.enter_file_picker_mode();
    type_filter(&mut app, "src");

    let mut terminal = Terminal::new(TestBackend::new(80, 16)).expect("terminal");
    terminal
        .draw(|frame| crate::ui::file_picker::render_file_picker(frame, &mut app))
        .expect("draw file picker");
    let screen: String = terminal
        .backend()
        .buffer()
        .content()
        .chunks(80)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>() + "\n")
        .collect();

    assert!(screen.contains("Go to File (3/5)"), "{screen}");
    assert!(screen.contains("/src"), "{screen}");
    assert!(screen.contains("M src/main.rs"), "{screen}");
    assert!(screen.contains("M src/ui/file_list.rs"), "{screen}");
    assert!(!screen.contains("README.md"), "{screen}");
    assert!(screen.contains("open"), "{screen}");
}

fn render_screen(app: &mut App, width: u16, height: u16) -> String {
    use ratatui::{Terminal, backend::TestBackend};

    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal
        .draw(|frame| crate::ui::file_picker::render_file_picker(frame, app))
        .expect("draw file picker");
    terminal
        .backend()
        .buffer()
        .content()
        .chunks(usize::from(width))
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>() + "\n")
        .collect()
}

#[test]
fn should_keep_basenames_visible_in_a_narrow_pane() {
    let mut app = app_with(&[
        "src/forge/github/review_threads.rs",
        "src/forge/gitlab/models.rs",
    ]);
    app.enter_file_picker_mode();

    let screen = render_screen(&mut app, 30, 12);

    // The popup spans the whole 30-column pane instead of 70% of it.
    assert!(
        screen.contains("\u{2502}M \u{2026}/github/review_threads.rs\u{2502}"),
        "{screen}"
    );
    assert!(screen.contains("src/forge/gitlab/models.rs"), "{screen}");
}

#[test]
fn should_jump_by_path_after_a_reload_reorders_the_diff() {
    let mut app = app_with(PATHS);
    app.enter_file_picker_mode();
    type_filter(&mut app, "smoke");

    // A background reload replaces `diff_files` while the picker is open; the
    // captured index now points at a different file.
    let mut reloaded: Vec<DiffFile> = std::mem::take(&mut app.diff_files);
    reloaded.reverse();
    app.diff_files = reloaded;
    app.rebuild_annotations();
    handle_file_picker_action(&mut app, Action::SubmitInput);

    assert_eq!(app.input_mode, InputMode::Normal);
    assert_eq!(
        app.diff_state.current_file_idx,
        index_of(&app, "tests/smoke.rs")
    );
}

#[test]
fn should_warn_instead_of_jumping_when_a_reload_dropped_the_file() {
    let mut app = app_with(PATHS);
    app.enter_file_picker_mode();
    type_filter(&mut app, "smoke");
    let file = app.diff_state.current_file_idx;

    app.diff_files
        .retain(|f| f.display_path().display().to_string() != "tests/smoke.rs");
    app.rebuild_annotations();
    handle_file_picker_action(&mut app, Action::SubmitInput);

    assert_eq!(app.input_mode, InputMode::Normal);
    assert_eq!(app.diff_state.current_file_idx, file);
    assert!(
        app.message
            .as_ref()
            .is_some_and(|m| m.content.contains("no longer in the diff"))
    );
}

#[test]
fn should_not_report_filtering_once_the_mode_is_forced_back_to_normal() {
    let mut app = app_with(PATHS);
    app.enter_file_picker_mode();
    type_filter(&mut app, "src");

    app.input_mode = InputMode::Normal;

    assert!(!app.file_picker_filtering());
}
