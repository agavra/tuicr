use crate::app::*;
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
                highlighted_spans: None,
            },
            DiffLine {
                origin: LineOrigin::Addition,
                content: "added".to_string(),
                old_lineno: None,
                new_lineno: Some(2),
                highlighted_spans: None,
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

/// Paths of the file rows currently in the tree, in order.
fn visible_paths(app: &App) -> Vec<String> {
    app.build_visible_items()
        .into_iter()
        .filter_map(|item| match item {
            FileTreeItem::File { file_idx, .. } => Some(
                app.diff_files[file_idx]
                    .display_path()
                    .display()
                    .to_string(),
            ),
            FileTreeItem::Directory { .. } => None,
        })
        .collect()
}

fn visible_dirs(app: &App) -> Vec<String> {
    app.build_visible_items()
        .into_iter()
        .filter_map(|item| match item {
            FileTreeItem::Directory { path, .. } => Some(path),
            FileTreeItem::File { .. } => None,
        })
        .collect()
}

/// Type `pattern` into the prompt opened by `prompt` and press Enter.
fn apply(app: &mut App, prompt: FileTreePrompt, pattern: &str) {
    app.begin_file_tree_prompt(prompt);
    for ch in pattern.chars() {
        app.file_tree_prompt_insert_char(ch);
    }
    app.commit_file_tree_prompt();
}

#[test]
fn should_keep_only_files_matching_the_include_pattern() {
    let mut app = app_with(&["src/main.rs", "README.md", "src/app/tree.rs"]);

    apply(&mut app, FileTreePrompt::Include, r"\.rs$");

    assert_eq!(visible_paths(&app), vec!["src/main.rs", "src/app/tree.rs"]);
}

#[test]
fn should_drop_files_matching_the_exclude_pattern() {
    let mut app = app_with(&["src/main.rs", "tests/smoke.rs", "README.md"]);

    apply(&mut app, FileTreePrompt::Exclude, "^tests/");

    // Rows come back in tree order (root files first, then directories),
    // not in the order the diff was loaded.
    assert_eq!(visible_paths(&app), vec!["README.md", "src/main.rs"]);
}

#[test]
fn should_intersect_include_and_exclude_patterns() {
    let mut app = app_with(&["src/main.rs", "src/app/tree.rs", "tests/smoke.rs"]);

    apply(&mut app, FileTreePrompt::Include, r"\.rs$");
    apply(&mut app, FileTreePrompt::Exclude, "^tests/");

    assert_eq!(visible_paths(&app), vec!["src/main.rs", "src/app/tree.rs"]);
}

#[test]
fn should_match_patterns_case_insensitively() {
    let mut app = app_with(&["src/Main.rs", "README.md"]);

    apply(&mut app, FileTreePrompt::Include, "main");

    assert_eq!(visible_paths(&app), vec!["src/Main.rs"]);
}

#[test]
fn should_match_against_the_full_relative_path_not_just_the_file_name() {
    let mut app = app_with(&["src/app/tree.rs", "src/ui/tree.rs"]);

    apply(&mut app, FileTreePrompt::Include, "^src/ui/");

    assert_eq!(visible_paths(&app), vec!["src/ui/tree.rs"]);
}

#[test]
fn should_hide_directories_whose_children_are_all_filtered_out() {
    let mut app = app_with(&["src/main.rs", "docs/guide.md"]);

    apply(&mut app, FileTreePrompt::Include, r"\.rs$");

    assert_eq!(visible_dirs(&app), vec!["src"]);
}

#[test]
fn should_remove_filtered_files_from_the_diff_render_height() {
    let mut app = app_with(&["src/main.rs", "README.md"]);
    let unfiltered = app.total_lines();

    apply(&mut app, FileTreePrompt::Include, r"\.rs$");

    let filtered = app.total_lines();
    assert!(
        filtered < unfiltered,
        "filtered diff should be shorter: {filtered} vs {unfiltered}"
    );
    // Filtering to 1 of 2 identical files removes exactly one file's worth
    // of render lines.
    let per_file = unfiltered - filtered;
    assert_eq!(filtered, unfiltered - per_file);
}

#[test]
fn should_exclude_filtered_files_from_counts_and_stats() {
    let mut app = app_with(&["src/main.rs", "README.md", "docs/guide.md"]);
    assert_eq!(app.file_count(), 3);

    apply(&mut app, FileTreePrompt::Include, r"\.md$");

    assert_eq!(app.file_count(), 2);
    assert_eq!(app.unfiltered_file_count(), 3);
    let (files, _, _) = app.diff_stat();
    assert_eq!(files, 2);
}

#[test]
fn should_move_the_current_file_off_a_row_the_filter_just_hid() {
    let mut app = app_with(&["README.md", "src/main.rs"]);
    app.jump_to_file(0);
    assert_eq!(app.diff_state.current_file_idx, 0);

    apply(&mut app, FileTreePrompt::Include, r"\.rs$");

    assert_eq!(app.diff_state.current_file_idx, 1);
}

#[test]
fn should_park_at_the_overview_when_nothing_matches() {
    let mut app = app_with(&["src/main.rs", "README.md"]);

    apply(&mut app, FileTreePrompt::Include, "no-such-file");

    assert!(visible_paths(&app).is_empty());
    assert_eq!(app.diff_state.cursor_line, 0);
    assert_eq!(app.diff_state.scroll_offset, 0);
}

#[test]
fn should_clear_only_the_requested_filter() {
    let mut app = app_with(&["src/main.rs", "tests/smoke.rs", "README.md"]);
    apply(&mut app, FileTreePrompt::Include, r"\.rs$");
    apply(&mut app, FileTreePrompt::Exclude, "^tests/");

    app.clear_include_filter();
    assert_eq!(visible_paths(&app), vec!["README.md", "src/main.rs"]);

    app.clear_exclude_filter();
    assert_eq!(
        visible_paths(&app),
        vec!["README.md", "src/main.rs", "tests/smoke.rs"]
    );
    assert!(!app.file_filter_active());
}

#[test]
fn should_treat_an_empty_pattern_as_clearing_the_filter() {
    let mut app = app_with(&["src/main.rs", "README.md"]);
    apply(&mut app, FileTreePrompt::Include, r"\.rs$");
    assert_eq!(visible_paths(&app).len(), 1);

    // Reopening seeds the buffer with the applied pattern, so emptying it
    // (ctrl-u) is what expresses "no include filter".
    app.begin_file_tree_prompt(FileTreePrompt::Include);
    app.file_tree_prompt_clear_line();
    app.commit_file_tree_prompt();

    assert_eq!(visible_paths(&app).len(), 2);
    assert!(!app.file_filter_active());
}

#[test]
fn should_keep_the_prompt_open_and_apply_nothing_on_an_invalid_regex() {
    let mut app = app_with(&["src/main.rs", "README.md"]);

    apply(&mut app, FileTreePrompt::Include, "[unclosed");

    assert!(
        app.file_tree_prompt_editing(),
        "prompt should stay open so the pattern can be fixed"
    );
    assert!(!app.file_filter_active());
    assert_eq!(visible_paths(&app).len(), 2);
    // The reason, not the `regex parse error:` header, is what reaches the
    // status bar. Pinned against the live crate output.
    let message = app.message.as_ref().expect("error message").content.clone();
    assert_eq!(message, "Invalid regex: unclosed character class");
}

#[test]
fn should_seed_the_prompt_with_the_pattern_already_applied() {
    let mut app = app_with(&["src/main.rs"]);
    apply(&mut app, FileTreePrompt::Include, r"\.rs$");

    app.begin_file_tree_prompt(FileTreePrompt::Include);

    assert_eq!(app.file_tree_draft().expect("draft").buffer, r"\.rs$");
}

#[test]
fn should_discard_the_draft_when_the_prompt_is_cancelled() {
    let mut app = app_with(&["src/main.rs", "README.md"]);
    app.begin_file_tree_prompt(FileTreePrompt::Include);
    app.file_tree_prompt_insert_char('x');

    app.cancel_file_tree_prompt();

    assert!(!app.file_tree_prompt_editing());
    assert!(!app.file_filter_active());
}

#[test]
fn should_select_the_matching_file_on_search_without_moving_the_diff() {
    let mut app = app_with(&["README.md", "src/main.rs"]);
    app.jump_to_file(0);
    let cursor_before = app.diff_state.cursor_line;

    apply(&mut app, FileTreePrompt::Search, "main");

    let selected = app.get_selected_tree_item().expect("selection");
    let FileTreeItem::File { file_idx, .. } = selected else {
        panic!("expected a file row to be selected, got {selected:?}");
    };
    assert_eq!(
        app.diff_files[file_idx]
            .display_path()
            .display()
            .to_string(),
        "src/main.rs"
    );
    assert_eq!(
        app.diff_state.cursor_line, cursor_before,
        "search should not move the diff viewport"
    );
    assert_eq!(app.diff_state.current_file_idx, 0);
}

#[test]
fn should_step_and_wrap_through_search_matches() {
    let mut app = app_with(&["a_test.rs", "b_test.rs", "README.md"]);

    apply(&mut app, FileTreePrompt::Search, "_test");
    assert_eq!(selected_path(&app), "a_test.rs");

    app.file_tree_search_next();
    assert_eq!(selected_path(&app), "b_test.rs");

    // Past the last match, wrap to the first.
    app.file_tree_search_next();
    assert_eq!(selected_path(&app), "a_test.rs");

    // And backwards off the front wraps to the last.
    app.file_tree_search_prev();
    assert_eq!(selected_path(&app), "b_test.rs");
}

#[test]
fn should_expand_collapsed_parents_to_reveal_a_search_match() {
    let mut app = app_with(&["src/deep/nested/target.rs", "README.md"]);
    app.collapse_all_dirs();

    apply(&mut app, FileTreePrompt::Search, "target");

    assert_eq!(selected_path(&app), "src/deep/nested/target.rs");
}

#[test]
fn should_not_search_into_files_hidden_by_a_filter() {
    let mut app = app_with(&["src/main.rs", "tests/main.rs"]);
    apply(&mut app, FileTreePrompt::Exclude, "^tests/");

    apply(&mut app, FileTreePrompt::Search, "main");
    assert_eq!(selected_path(&app), "src/main.rs");

    // The only other "main" match is excluded, so stepping wraps back.
    app.file_tree_search_next();
    assert_eq!(selected_path(&app), "src/main.rs");
}

#[test]
fn should_report_when_no_file_matches_the_search() {
    let mut app = app_with(&["src/main.rs"]);

    apply(&mut app, FileTreePrompt::Search, "nothing-here");

    let message = app.message.as_ref().expect("message").content.clone();
    assert!(
        message.contains("nothing-here"),
        "expected a no-match message, got: {message}"
    );
}

fn selected_path(app: &App) -> String {
    match app.get_selected_tree_item().expect("selection") {
        FileTreeItem::File { file_idx, .. } => app.diff_files[file_idx]
            .display_path()
            .display()
            .to_string(),
        other => panic!("expected a file row, got {other:?}"),
    }
}

// ---- hiding reviewed files (`H` / `:set noreviewed`) ---------------------

/// `diff_files` index of `path`. Looked up rather than hardcoded because
/// `App::build` sorts the files, so load order is not index order.
fn index_of(app: &App, path: &str) -> usize {
    app.diff_files
        .iter()
        .position(|file| file.display_path().display().to_string() == path)
        .unwrap_or_else(|| panic!("no diff file for {path}"))
}

/// Mark `path` reviewed the way the tree's `r` does (without moving the diff
/// cursor), so these tests exercise the real toggle rather than poking the
/// session directly.
fn mark_reviewed(app: &mut App, path: &str) {
    let idx = index_of(app, path);
    app.toggle_reviewed_for_file_idx(idx, false);
}

#[test]
fn should_hide_reviewed_files_from_the_tree() {
    let mut app = app_with(&["README.md", "src/main.rs"]);
    mark_reviewed(&mut app, "README.md");

    app.set_show_reviewed(false);

    assert_eq!(visible_paths(&app), vec!["src/main.rs"]);
}

#[test]
fn should_bring_reviewed_files_back_when_toggled_on_again() {
    let mut app = app_with(&["README.md", "src/main.rs"]);
    mark_reviewed(&mut app, "README.md");
    app.set_show_reviewed(false);

    app.toggle_show_reviewed();

    assert_eq!(visible_paths(&app), vec!["README.md", "src/main.rs"]);
}

#[test]
fn should_remove_hidden_reviewed_files_from_the_diff_render_height() {
    let mut app = app_with(&["README.md", "src/main.rs"]);
    mark_reviewed(&mut app, "README.md");
    // Marking reviewed already collapses the body in multi-file view, leaving
    // the header row behind. Measure from there so this asserts the hiding,
    // not the pre-existing collapse.
    let collapsed = app.total_lines();

    app.set_show_reviewed(false);

    let hidden = app.total_lines();
    assert!(
        hidden < collapsed,
        "hiding a reviewed file should drop its remaining rows: {hidden} vs {collapsed}"
    );
}

#[test]
fn should_keep_hunk_positions_aligned_with_the_rendered_rows_while_hiding() {
    let mut app = app_with(&["a.rs", "b.rs", "c.rs"]);
    mark_reviewed(&mut app, "a.rs");

    app.set_show_reviewed(false);

    // `hunk_positions` walks `diff_files` computing its own cumulative row
    // offsets, so it has to drop exactly the rows the renderer drops —
    // including the file header that a reviewed-but-visible file still gets.
    for pos in app.hunk_positions() {
        assert!(
            matches!(
                app.line_annotations.get(pos),
                Some(AnnotatedLine::HunkHeader { .. })
            ),
            "hunk position {pos} should land on a hunk header, got {:?}",
            app.line_annotations.get(pos)
        );
    }
}

#[test]
fn should_keep_the_progress_fraction_over_the_whole_population_while_hiding() {
    let mut app = app_with(&["a.rs", "b.rs", "c.rs"]);
    mark_reviewed(&mut app, "a.rs");

    app.set_show_reviewed(false);

    // The tree title reads `Files · reviewed/total`. Counting only the rows on
    // screen would collapse it to `0/2` exactly when progress matters most.
    assert_eq!(app.reviewed_count(), 1);
    assert_eq!(app.file_count(), 3);
}

#[test]
fn should_still_scope_the_progress_fraction_to_the_include_exclude_filters() {
    let mut app = app_with(&["src/a.rs", "src/b.rs", "docs/guide.md"]);
    mark_reviewed(&mut app, "src/a.rs");
    apply(&mut app, FileTreePrompt::Include, "^src/");

    app.set_show_reviewed(false);

    assert_eq!(app.reviewed_count(), 1);
    assert_eq!(app.file_count(), 2);
}

#[test]
fn should_not_search_into_hidden_reviewed_files() {
    let mut app = app_with(&["src/main.rs", "tests/main.rs"]);
    mark_reviewed(&mut app, "tests/main.rs");
    app.set_show_reviewed(false);

    apply(&mut app, FileTreePrompt::Search, "main");
    assert_eq!(selected_path(&app), "src/main.rs");

    // The only other "main" match is hidden, so stepping wraps back.
    app.file_tree_search_next();
    assert_eq!(selected_path(&app), "src/main.rs");
}

#[test]
fn should_advance_to_the_next_unreviewed_file_when_marking_hides_the_current_one() {
    let mut app = app_with(&["a.rs", "b.rs", "c.rs"]);
    app.set_show_reviewed(false);
    app.jump_to_file(index_of(&app, "b.rs"));

    mark_reviewed(&mut app, "b.rs");

    assert_eq!(app.diff_state.current_file_idx, index_of(&app, "c.rs"));
    assert_eq!(selected_path(&app), "c.rs");
}

#[test]
fn should_wrap_to_the_first_remaining_file_when_marking_the_last_one() {
    let mut app = app_with(&["a.rs", "b.rs", "c.rs"]);
    app.set_show_reviewed(false);
    app.jump_to_file(index_of(&app, "c.rs"));

    mark_reviewed(&mut app, "c.rs");

    assert_eq!(app.diff_state.current_file_idx, index_of(&app, "a.rs"));
    assert_eq!(selected_path(&app), "a.rs");
}

#[test]
fn should_park_at_the_overview_when_the_last_unreviewed_file_is_marked() {
    let mut app = app_with(&["a.rs"]);
    app.set_show_reviewed(false);

    mark_reviewed(&mut app, "a.rs");

    assert!(visible_paths(&app).is_empty());
    assert_eq!(app.diff_state.cursor_line, 0);
    assert_eq!(app.diff_state.scroll_offset, 0);
    let message = app.message.as_ref().expect("message").content.clone();
    assert!(
        message.contains("reviewed"),
        "expected the exhausted-queue message to name the way back, got: {message}"
    );
}

#[test]
fn should_leave_the_cursor_alone_when_marking_reviewed_while_showing_everything() {
    let mut app = app_with(&["a.rs", "b.rs", "c.rs"]);
    let b = index_of(&app, "b.rs");
    app.jump_to_file(b);

    mark_reviewed(&mut app, "b.rs");

    // Without hiding there is nothing to recover from: the reviewed file keeps
    // its row and the diff stays where it was.
    assert_eq!(app.diff_state.current_file_idx, b);
    assert_eq!(visible_paths(&app), vec!["a.rs", "b.rs", "c.rs"]);
}

#[test]
fn should_leave_no_stale_annotations_when_the_startup_default_hides_reviewed_files() {
    // `App::build` builds annotations before `main.rs` applies the config, so
    // the startup path has to re-derive them. `total_lines()` recomputes from
    // the filter, but `line_annotations` — what the diff renderer and the
    // cursor read — does not.
    let mut app = app_with(&["a.rs", "b.rs"]);
    mark_reviewed(&mut app, "a.rs");

    app.init_show_reviewed(false);

    assert_eq!(
        app.line_annotations.len(),
        app.total_lines(),
        "rendered rows and the scroll math must agree after the startup default"
    );
}

#[test]
fn should_mark_the_selected_tree_row_reviewed_through_the_file_list_handler() {
    let mut app = app_with(&["a.rs", "b.rs"]);
    app.focused_panel = FocusedPanel::FileList;
    app.file_list_state.select(0);

    crate::handler::handle_file_list_action(&mut app, Action::ToggleReviewed);

    assert_eq!(app.reviewed_count(), 1);
}

#[test]
fn should_burn_down_through_the_file_list_handler_while_hiding() {
    let mut app = app_with(&["a.rs", "b.rs", "c.rs"]);
    app.focused_panel = FocusedPanel::FileList;
    app.set_show_reviewed(false);
    app.file_list_state.select(0);

    crate::handler::handle_file_list_action(&mut app, Action::ToggleReviewed);

    assert_eq!(visible_paths(&app), vec!["b.rs", "c.rs"]);
    assert_eq!(selected_path(&app), "b.rs");
}

#[test]
fn should_toggle_hiding_through_the_file_list_handler() {
    let mut app = app_with(&["a.rs", "b.rs"]);
    app.focused_panel = FocusedPanel::FileList;
    mark_reviewed(&mut app, "a.rs");

    crate::handler::handle_file_list_action(&mut app, Action::ToggleShowReviewed);

    assert!(!app.show_reviewed());
    assert_eq!(visible_paths(&app), vec!["b.rs"]);
}

#[test]
fn should_not_hide_a_file_whose_hunks_are_merely_all_hunk_reviewed() {
    let mut app = app_with(&["a.rs", "b.rs"]);
    // `R` on every hunk is not the same as `r` on the file: only the
    // file-level flag hides.
    let a = index_of(&app, "a.rs");
    app.jump_to_file(a);
    app.toggle_hunk_reviewed();
    assert!(
        app.is_hunk_reviewed(a, 0),
        "precondition: the hunk should be marked reviewed"
    );

    app.set_show_reviewed(false);

    assert_eq!(visible_paths(&app), vec!["a.rs", "b.rs"]);
}
