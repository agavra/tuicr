use crate::app::*;
use crate::handler::handle_theme_picker_action;
use crate::input::keybindings::Action;
use crate::model::{DiffFile, DiffHunk, DiffLine, FileStatus, LineOrigin};
use crate::vcs::traits::{VcsBackend, VcsInfo, VcsType};
use ratatui::style::{Color, Style};
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
    app_with_files(Vec::new())
}

/// A diff file with one addition-line hunk whose `highlighted_spans` carry an
/// artificial marker background, standing in for "highlighted under whatever
/// theme was active at load time." Real syntax highlighters never produce
/// this exact color, so tests can tell a stale cached span apart from a
/// freshly rehighlighted one without depending on real syntect output.
fn diff_file_with_marker_bg(path: &str, marker_bg: Color) -> DiffFile {
    let hunk = DiffHunk {
        header: "@@ -0,0 +1,1 @@".to_string(),
        lines: vec![DiffLine {
            origin: LineOrigin::Addition,
            content: "let x = 1;".to_string(),
            old_lineno: None,
            new_lineno: Some(1),
            highlighted_spans: Some(vec![(
                Style::default().bg(marker_bg),
                "let x = 1;".to_string(),
            )]),
        }],
        old_start: 1,
        old_count: 0,
        new_start: 1,
        new_count: 1,
    };
    let content_hash = DiffFile::compute_content_hash(std::slice::from_ref(&hunk));
    DiffFile {
        old_path: None,
        new_path: Some(PathBuf::from(path)),
        status: FileStatus::Modified,
        hunks: vec![hunk],
        is_binary: false,
        is_too_large: false,
        is_commit_message: false,
        content_hash,
    }
}

/// The marker background stashed by `diff_file_with_marker_bg`, read back
/// from a file's first hunk/line, if still present.
fn addition_bg(file: &DiffFile) -> Option<Color> {
    file.hunks[0].lines[0]
        .highlighted_spans
        .as_ref()
        .and_then(|spans| spans.first())
        .and_then(|(style, _)| style.bg)
}

fn app_with_files(diff_files: Vec<DiffFile>) -> App {
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
        diff_files,
        session,
        DiffSource::WorkingTree,
        InputMode::Normal,
        Vec::new(),
        None,
        None,
    )
    .expect("build app")
}

// NOTE: these tests deliberately never call `App::confirm_theme_picker` or
// `App::apply_and_persist_theme` -- both call `config::set_theme_in_config`,
// which resolves the *real* user config path (XDG/HOME/APPDATA) and would
// write to the developer's actual `config.toml` if exercised here.
// Persistence correctness is covered by the path-parameterized
// `config::tests::set_theme_in_config_*` unit tests instead.

#[test]
fn entering_picker_populates_all_built_in_candidates_and_snapshots_original() {
    let mut app = app();
    app.theme = crate::theme::Theme::dark();

    app.enter_theme_picker_mode();

    assert_eq!(app.input_mode, InputMode::ThemePicker);
    assert!(app.theme_picker.candidates.contains(&"dark".to_string()));
    assert!(
        app.theme_picker
            .candidates
            .contains(&"catppuccin-mocha".to_string())
    );
    assert!(app.theme_picker.candidates.len() >= 24);
    assert!(app.theme_picker.original.is_some());
}

#[test]
fn navigation_previews_theme_live() {
    let mut app = app();
    app.enter_theme_picker_mode();
    let dark_panel_bg = app.theme.panel_bg;

    handle_theme_picker_action(&mut app, Action::CursorDown(1));

    // Selecting the next candidate ("light", index 1) must repaint app.theme.
    assert_eq!(app.theme_picker.selected(), 1);
    assert_eq!(app.theme_picker.selected_name(), Some("light"));
    assert_ne!(app.theme.panel_bg, dark_panel_bg);
}

#[test]
fn cursor_up_at_top_clamps_instead_of_wrapping() {
    let mut app = app();
    app.enter_theme_picker_mode();

    handle_theme_picker_action(&mut app, Action::CursorUp(1));

    assert_eq!(app.theme_picker.selected(), 0);
}

#[test]
fn esc_on_picker_reverts_preview_and_closes() {
    let mut app = app();
    app.enter_theme_picker_mode();
    let original_panel_bg = app.theme.panel_bg;
    handle_theme_picker_action(&mut app, Action::CursorDown(1));
    assert_ne!(app.theme.panel_bg, original_panel_bg);

    handle_theme_picker_action(&mut app, Action::ExitMode);

    assert_eq!(app.input_mode, InputMode::Normal);
    assert_eq!(app.theme.panel_bg, original_panel_bg);
    assert!(app.theme_picker.candidates.is_empty());
}

#[test]
fn slash_opens_filter_draft_and_typing_does_not_move_selection_until_committed() {
    let mut app = app();
    app.enter_theme_picker_mode();

    handle_theme_picker_action(&mut app, Action::ThemePickerFilter);
    assert!(app.theme_picker_filtering());

    for ch in "gruvbox".chars() {
        handle_theme_picker_action(&mut app, Action::InsertChar(ch));
    }
    assert_eq!(app.theme_picker.draft.as_deref(), Some("gruvbox"));
    // Still filtering: the applied filter and selection haven't changed yet.
    assert_eq!(app.theme_picker.filter, None);
}

#[test]
fn committing_filter_narrows_candidates_and_previews_first_match() {
    let mut app = app();
    app.enter_theme_picker_mode();
    handle_theme_picker_action(&mut app, Action::ThemePickerFilter);
    for ch in "gruvbox".chars() {
        handle_theme_picker_action(&mut app, Action::InsertChar(ch));
    }

    handle_theme_picker_action(&mut app, Action::SubmitInput);

    assert!(!app.theme_picker_filtering());
    assert_eq!(app.theme_picker.filter.as_deref(), Some("gruvbox"));
    let filtered = app.theme_picker.filtered_indices();
    assert!(
        filtered
            .iter()
            .all(|&idx| app.theme_picker.candidates[idx].contains("gruvbox"))
    );
    assert_eq!(app.theme_picker.selected(), 0);
    assert_eq!(app.theme_picker.selected_name(), Some("gruvbox-dark"));
}

#[test]
fn esc_while_filtering_discards_draft_without_changing_applied_filter() {
    let mut app = app();
    app.enter_theme_picker_mode();
    // Apply a filter first via the normal commit path.
    handle_theme_picker_action(&mut app, Action::ThemePickerFilter);
    for ch in "nord".chars() {
        handle_theme_picker_action(&mut app, Action::InsertChar(ch));
    }
    handle_theme_picker_action(&mut app, Action::SubmitInput);
    assert_eq!(app.theme_picker.filter.as_deref(), Some("nord"));

    // Reopen the draft and abandon a different edit.
    handle_theme_picker_action(&mut app, Action::ThemePickerFilter);
    handle_theme_picker_action(&mut app, Action::InsertChar('x'));
    handle_theme_picker_action(&mut app, Action::ExitMode);

    assert!(!app.theme_picker_filtering());
    assert_eq!(app.theme_picker.filter.as_deref(), Some("nord"));
    // The picker itself must still be open (Esc only closed the draft).
    assert_eq!(app.input_mode, InputMode::ThemePicker);
}

#[test]
fn empty_filter_commit_clears_the_filter() {
    let mut app = app();
    app.enter_theme_picker_mode();
    handle_theme_picker_action(&mut app, Action::ThemePickerFilter);
    for ch in "nord".chars() {
        handle_theme_picker_action(&mut app, Action::InsertChar(ch));
    }
    handle_theme_picker_action(&mut app, Action::SubmitInput);
    assert!(app.theme_picker.filter.is_some());

    handle_theme_picker_action(&mut app, Action::ThemePickerFilter);
    handle_theme_picker_action(&mut app, Action::ClearLine);
    handle_theme_picker_action(&mut app, Action::SubmitInput);

    assert_eq!(app.theme_picker.filter, None);
    assert_eq!(
        app.theme_picker.filtered_indices().len(),
        app.theme_picker.candidates.len()
    );
}

#[test]
fn list_scrolls_to_keep_selection_visible_in_a_short_viewport() {
    use ratatui::{Terminal, backend::TestBackend};

    let mut app = app();
    app.enter_theme_picker_mode();
    assert!(app.theme_picker.candidates.len() > 10);

    let mut terminal = Terminal::new(TestBackend::new(60, 12)).expect("terminal");
    terminal
        .draw(|frame| crate::ui::theme_picker::render_theme_picker(frame, &mut app))
        .expect("draw theme picker");
    assert_eq!(app.theme_picker.list_state.offset(), 0);

    // Move well past the first render's visible window.
    for _ in 0..15 {
        handle_theme_picker_action(&mut app, Action::CursorDown(1));
    }
    terminal
        .draw(|frame| crate::ui::theme_picker::render_theme_picker(frame, &mut app))
        .expect("draw theme picker");

    // The list must have scrolled so the far-down selection stayed visible.
    assert!(app.theme_picker.list_state.offset() > 0);
}

// ---- rehighlight scoping (foreground colors during preview/confirm/cancel) ----

const MARKER_BG: Color = Color::Rgb(1, 2, 3);

#[test]
fn preview_rehighlights_only_the_currently_visible_file() {
    let mut app = app_with_files(vec![
        diff_file_with_marker_bg("a.rs", MARKER_BG),
        diff_file_with_marker_bg("b.rs", MARKER_BG),
    ]);
    app.diff_state.current_file_idx = 0;
    app.enter_theme_picker_mode();

    // Move to a different built-in theme; "light" is index 1, right after "dark".
    handle_theme_picker_action(&mut app, Action::CursorDown(1));
    assert_eq!(app.theme_picker.selected_name(), Some("light"));

    // The visible file (index 0) was rehighlighted under the new theme, so
    // its marker background is gone, replaced by "light"'s real syntax_add_bg.
    assert_ne!(addition_bg(&app.diff_files[0]), Some(MARKER_BG));
    assert_eq!(
        addition_bg(&app.diff_files[0]),
        Some(app.theme.syntax_add_bg)
    );

    // The other file was never on screen, so its cached spans are left
    // exactly as they were -- proportional-to-visible-file scoping, not a
    // full-diff rehighlight, is what keeps scrubbing cheap.
    assert_eq!(addition_bg(&app.diff_files[1]), Some(MARKER_BG));
}

#[test]
fn esc_reverts_the_rehighlighted_file_back_to_the_original_theme() {
    let mut app = app_with_files(vec![diff_file_with_marker_bg("a.rs", MARKER_BG)]);
    app.diff_state.current_file_idx = 0;
    let original_syntax_add_bg = app.theme.syntax_add_bg;
    app.enter_theme_picker_mode();

    handle_theme_picker_action(&mut app, Action::CursorDown(1));
    assert_ne!(
        addition_bg(&app.diff_files[0]),
        Some(original_syntax_add_bg)
    );

    handle_theme_picker_action(&mut app, Action::ExitMode);

    // Cancelling restores `app.theme` to the original and must also undo the
    // scrub-time rehighlight, or the file would stay recolored under a theme
    // that was never actually chosen.
    assert_eq!(
        addition_bg(&app.diff_files[0]),
        Some(original_syntax_add_bg)
    );
}

#[test]
fn confirm_rehighlights_every_loaded_file_not_just_the_visible_one() {
    let mut app = app_with_files(vec![
        diff_file_with_marker_bg("a.rs", MARKER_BG),
        diff_file_with_marker_bg("b.rs", MARKER_BG),
    ]);
    app.diff_state.current_file_idx = 0;
    app.enter_theme_picker_mode();
    handle_theme_picker_action(&mut app, Action::CursorDown(1));
    let chosen_syntax_add_bg = app.theme.syntax_add_bg;

    // Before confirming, the non-visible file is still stale (previous test
    // already covers this) -- confirming must catch it up.
    assert_eq!(addition_bg(&app.diff_files[1]), Some(MARKER_BG));

    // `confirm_theme_picker`/`apply_and_persist_theme` both write to the
    // real user config path via `config::set_theme_in_config`, so tests call
    // the underlying rehighlight step directly rather than going through
    // `Action::SubmitInput`. Persistence itself is covered by the
    // path-parameterized `config::tests::set_theme_in_config_*` tests.
    app.rehighlight_all_files();

    assert_eq!(addition_bg(&app.diff_files[0]), Some(chosen_syntax_add_bg));
    assert_eq!(addition_bg(&app.diff_files[1]), Some(chosen_syntax_add_bg));
}
