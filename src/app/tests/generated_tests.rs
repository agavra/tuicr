use std::fs;
use std::path::{Path, PathBuf};

use tempfile::{TempDir, tempdir};

use crate::app::*;
use crate::config::GeneratedConfig;
use crate::model::{DiffFile, DiffHunk, DiffLine, FileStatus, LineOrigin};
use crate::vcs::traits::{VcsBackend, VcsInfo, VcsType};

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
        _path: &Path,
        _status: FileStatus,
        _ref_commit: Option<&str>,
        _start: u32,
        _end: u32,
    ) -> crate::error::Result<Vec<DiffLine>> {
        Ok(Vec::new())
    }
    fn file_line_count(
        &self,
        _path: &Path,
        _status: FileStatus,
        _ref_commit: Option<&str>,
    ) -> crate::error::Result<u32> {
        Ok(0)
    }
}

fn hunk() -> DiffHunk {
    let lines = (1..=3)
        .map(|i| DiffLine {
            origin: LineOrigin::Context,
            content: format!("line {i}"),
            old_lineno: Some(i),
            new_lineno: Some(i),
            highlighted_spans: None,
            raw_content: None,
        })
        .collect();
    DiffHunk {
        header: "@@ -1,3 +1,3 @@".to_string(),
        lines,
        old_start: 1,
        old_count: 3,
        new_start: 1,
        new_count: 3,
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

/// A git repository whose root `.gitattributes` marks `api.pb.go` generated.
fn repo() -> TempDir {
    let dir = tempdir().expect("tempdir");
    git2::Repository::init(dir.path()).expect("init repo");
    fs::write(
        dir.path().join(".gitattributes"),
        "*.pb.go linguist-generated=true\n",
    )
    .expect("write .gitattributes");
    dir
}

fn app_in(dir: &TempDir, paths: &[&str]) -> App {
    let vcs_info = VcsInfo {
        root_path: dir.path().to_path_buf(),
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
        paths.iter().copied().map(file).collect(),
        session,
        DiffSource::WorkingTree,
        InputMode::Normal,
        Vec::new(),
        None,
        None,
    )
    .expect("build app")
}

fn collapsing_app(dir: &TempDir, paths: &[&str]) -> App {
    let mut app = app_in(dir, paths);
    app.apply_generated_config(&GeneratedConfig {
        collapse: Some(true),
        count: None,
    });
    app
}

fn collapsed(app: &App, path: &str) -> bool {
    let idx = app
        .diff_files
        .iter()
        .position(|f| f.display_path() == Path::new(path))
        .expect("file in diff set");
    app.should_collapse_file(idx)
}

fn opted_out_app(dir: &TempDir, paths: &[&str]) -> App {
    let mut app = app_in(dir, paths);
    app.apply_generated_config(&GeneratedConfig {
        collapse: Some(false),
        count: None,
    });
    app
}

#[test]
fn default_config_collapses_generated_files() {
    let dir = repo();
    let mut app = app_in(&dir, &["api.pb.go", "src/main.rs"]);
    app.apply_generated_config(&GeneratedConfig::default());

    assert!(collapsed(&app, "api.pb.go"));
    assert!(!collapsed(&app, "src/main.rs"));
}

#[test]
fn opting_out_shows_generated_files_undecorated() {
    let dir = repo();
    let app = opted_out_app(&dir, &["api.pb.go", "src/main.rs"]);

    assert!(!collapsed(&app, "api.pb.go"));
    assert!(!app.is_generated_file(Path::new("api.pb.go")));
    assert_eq!(app.generated_file_count(), 0);
}

#[test]
fn collapse_hides_generated_files_only() {
    let dir = repo();
    let app = collapsing_app(&dir, &["api.pb.go", "src/main.rs"]);

    assert!(app.is_generated_file(Path::new("api.pb.go")));
    assert!(!app.is_generated_file(Path::new("src/main.rs")));
    assert!(collapsed(&app, "api.pb.go"));
    assert!(!collapsed(&app, "src/main.rs"));
}

#[test]
fn space_expansion_overrides_collapse() {
    let dir = repo();
    let mut app = collapsing_app(&dir, &["api.pb.go"]);
    app.diff_state.current_file_idx = 0;

    assert!(app.toggle_file_collapse());
    assert!(!collapsed(&app, "api.pb.go"));

    assert!(app.toggle_file_collapse());
    assert!(collapsed(&app, "api.pb.go"));
}

#[test]
fn space_manually_collapses_a_file_that_is_not_generated() {
    // Space is not scoped to generated files: it toggles any file's body,
    // manually collapsing one that defaulted to expanded.
    let dir = repo();
    let mut app = collapsing_app(&dir, &["src/main.rs"]);
    app.diff_state.current_file_idx = 0;

    assert!(!collapsed(&app, "src/main.rs"));
    assert!(app.toggle_file_collapse());
    assert!(collapsed(&app, "src/main.rs"));
}

#[test]
fn turning_collapse_off_stops_collapsing_already_detected_files() {
    // The detected set is not cleared when the feature is switched off, so
    // the predicate has to gate on the runtime flag rather than on the set
    // being empty.
    let dir = repo();
    let mut app = collapsing_app(&dir, &["api.pb.go"]);
    assert!(collapsed(&app, "api.pb.go"));

    app.toggle_collapse_generated();

    assert!(!app.generated_files.is_empty());
    assert!(!collapsed(&app, "api.pb.go"));
}

#[test]
fn toggling_collapse_off_returns_to_the_starting_appearance() {
    // The detected set is kept when the feature is switched off so that
    // re-enabling is free, which means the decoration must not be keyed off
    // it: `:generated` twice from an opted-out start has to land back exactly
    // where it began, labels and counter included.
    let dir = repo();
    let mut app = opted_out_app(&dir, &["api.pb.go", "src/main.rs"]);

    let initial = (
        app.is_generated_file(Path::new("api.pb.go")),
        app.generated_file_count(),
        app.review_progress(),
        collapsed(&app, "api.pb.go"),
    );
    assert_eq!(initial, (false, 0, (0, 2), false));

    app.toggle_collapse_generated();
    assert_eq!(
        (
            app.is_generated_file(Path::new("api.pb.go")),
            app.generated_file_count(),
            app.review_progress(),
            collapsed(&app, "api.pb.go"),
        ),
        (true, 1, (0, 2), true)
    );

    app.toggle_collapse_generated();
    assert_eq!(
        (
            app.is_generated_file(Path::new("api.pb.go")),
            app.generated_file_count(),
            app.review_progress(),
            collapsed(&app, "api.pb.go"),
        ),
        initial,
        "toggling collapse off must restore the starting appearance"
    );
    // The set itself is retained, so re-enabling costs no libgit2 work.
    assert!(!app.generated_files.is_empty());
}

#[test]
fn count_exclusion_alone_still_labels_without_being_collapsed() {
    // `count = false, collapse = false` is a legitimate configuration: the
    // file is surfaced and dropped from progress, but nothing is hidden by
    // default.
    let dir = repo();
    let mut app = app_in(&dir, &["api.pb.go", "src/main.rs"]);
    app.apply_generated_config(&GeneratedConfig {
        collapse: Some(false),
        count: Some(false),
    });
    app.diff_state.current_file_idx = app
        .diff_files
        .iter()
        .position(|f| f.display_path() == Path::new("api.pb.go"))
        .expect("generated file");

    assert!(app.is_generated_file(Path::new("api.pb.go")));
    assert_eq!(app.generated_file_count(), 1);
    assert!(!collapsed(&app, "api.pb.go"));
}

#[test]
fn expansion_survives_a_diff_file_rebuild_that_shifts_indices() {
    // Reloads, watch ticks, and commit-selection changes all replace
    // `diff_files` wholesale. State keyed by index would follow the wrong
    // file afterwards.
    let dir = repo();
    let mut app = collapsing_app(&dir, &["api.pb.go"]);
    app.diff_state.current_file_idx = 0;
    app.toggle_file_collapse();

    app.diff_files = vec![file("src/main.rs"), file("api.pb.go")];
    app.rebuild_annotations();

    assert_eq!(
        app.diff_files[1].display_path(),
        &PathBuf::from("api.pb.go")
    );
    assert!(!collapsed(&app, "api.pb.go"));
}

#[test]
fn space_expands_a_reviewed_file_without_un_reviewing_it() {
    // The whole point of a per-file override: peeking at a reviewed file's
    // diff must not touch its reviewed status, unlike the old behavior where
    // `r` was the only way to see it again.
    let dir = repo();
    let mut app = app_in(&dir, &["src/main.rs"]);
    app.diff_state.current_file_idx = 0;
    app.toggle_reviewed();
    assert!(collapsed(&app, "src/main.rs"));

    assert!(app.toggle_file_collapse());
    assert!(!collapsed(&app, "src/main.rs"));
    assert!(app.session.is_file_reviewed(&PathBuf::from("src/main.rs")));

    assert!(app.toggle_file_collapse());
    assert!(collapsed(&app, "src/main.rs"));
    assert!(app.session.is_file_reviewed(&PathBuf::from("src/main.rs")));
}

#[test]
fn detection_runs_for_count_exclusion_without_collapse() {
    // `count` is deliberately not gated by `collapse`: excluding generated
    // files from review progress is useful on its own.
    let dir = repo();
    let mut app = app_in(&dir, &["api.pb.go", "src/main.rs"]);
    app.apply_generated_config(&GeneratedConfig {
        collapse: Some(false),
        count: Some(false),
    });

    assert_eq!(app.generated_file_count(), 1);
    assert!(!collapsed(&app, "api.pb.go"));
    assert_eq!(app.review_progress(), (0, 1));
}

#[test]
fn progress_counts_generated_files_by_default() {
    let dir = repo();
    let app = collapsing_app(&dir, &["api.pb.go", "src/main.rs"]);

    assert_eq!(app.generated_file_count(), 1);
    assert_eq!(app.review_progress(), (0, 2));
}

#[test]
fn reviewing_an_excluded_generated_file_cannot_overrun_the_total() {
    // Both halves of the fraction have to drop the file, or marking it
    // reviewed would report 2/1.
    let dir = repo();
    let mut app = app_in(&dir, &["api.pb.go", "src/main.rs"]);
    app.apply_generated_config(&GeneratedConfig {
        collapse: Some(false),
        count: Some(false),
    });

    let generated_idx = app
        .diff_files
        .iter()
        .position(|f| f.display_path() == Path::new("api.pb.go"))
        .expect("generated file");
    app.toggle_reviewed_for_file_idx(generated_idx, false);

    assert_eq!(app.review_progress(), (0, 1));
}

#[test]
fn invalidation_re_reads_edited_gitattributes() {
    let dir = repo();
    let mut app = collapsing_app(&dir, &["api.pb.go"]);
    assert!(collapsed(&app, "api.pb.go"));

    fs::write(
        dir.path().join(".gitattributes"),
        "*.pb.go -linguist-generated\n",
    )
    .expect("rewrite .gitattributes");
    app.invalidate_generated_detection();
    app.rebuild_annotations();

    assert!(!app.is_generated_file(Path::new("api.pb.go")));
    assert!(!collapsed(&app, "api.pb.go"));
}

#[test]
fn set_generated_commands_drive_the_runtime_toggle() {
    let dir = repo();
    let mut app = app_in(&dir, &["api.pb.go"]);

    for (command, expected) in [
        ("set generated", true),
        ("set nogenerated", false),
        ("set generated!", true),
        ("generated", false),
    ] {
        app.enter_command_mode();
        app.command_buffer = command.to_string();
        crate::handler::handle_command_action(&mut app, crate::input::Action::SubmitInput);

        assert_eq!(
            app.collapse_generated, expected,
            "`:{command}` should leave collapse={expected}"
        );
        assert_eq!(app.input_mode, InputMode::Normal);
    }
    // The commands have to work from the `collapse = false` default, or the
    // opt-in default would be intolerable: the very first `:set generated`
    // must trigger detection that startup skipped.
    assert!(app.generated_files.contains(Path::new("api.pb.go")));
}

#[test]
fn space_in_the_diff_panel_expands_the_generated_file_under_the_cursor() {
    let dir = repo();
    let mut app = collapsing_app(&dir, &["api.pb.go"]);
    app.focused_panel = FocusedPanel::Diff;
    app.diff_state.current_file_idx = 0;

    crate::handler::handle_diff_action(&mut app, crate::input::Action::ToggleExpand);

    assert!(!collapsed(&app, "api.pb.go"));
}

#[test]
fn detection_is_skipped_when_there_is_no_local_repository_root() {
    // PR mode's root is the synthetic `forge:host/owner/repo` identity, and
    // without a matching local checkout there is nothing to read attributes
    // from. It must degrade to "nothing is generated", not panic.
    let dir = repo();
    let mut app = collapsing_app(&dir, &["api.pb.go"]);
    app.vcs_info.root_path = PathBuf::from("forge:github.com/agavra/tuicr");
    app.invalidate_generated_detection();
    app.rebuild_annotations();

    assert!(app.generated_files.is_empty());
    assert!(!collapsed(&app, "api.pb.go"));
}

#[test]
fn gitlab_generated_files_collapse_like_linguist_generated_ones() {
    let dir = repo();
    fs::write(
        dir.path().join(".gitattributes"),
        "github.ts linguist-generated=true\ngitlab.ts gitlab-generated\n",
    )
    .expect("write .gitattributes");
    let app = collapsing_app(&dir, &["github.ts", "gitlab.ts", "src/main.rs"]);

    assert!(collapsed(&app, "github.ts"));
    assert!(collapsed(&app, "gitlab.ts"));
    assert!(!collapsed(&app, "src/main.rs"));
}

#[test]
fn space_expands_a_collapsed_gitlab_generated_file() {
    let dir = repo();
    fs::write(
        dir.path().join(".gitattributes"),
        "gitlab.ts gitlab-generated\n",
    )
    .expect("write .gitattributes");
    let mut app = collapsing_app(&dir, &["gitlab.ts"]);
    app.focused_panel = FocusedPanel::Diff;
    app.diff_state.current_file_idx = 0;
    assert!(collapsed(&app, "gitlab.ts"));

    crate::handler::handle_diff_action(&mut app, crate::input::Action::ToggleExpand);
    assert!(!collapsed(&app, "gitlab.ts"));

    crate::handler::handle_diff_action(&mut app, crate::input::Action::ToggleExpand);
    assert!(collapsed(&app, "gitlab.ts"));
}

#[test]
fn reviewing_a_space_expanded_file_collapses_it_again() {
    // `r` restates how a file should look, so an earlier `Space` override must
    // not keep a freshly reviewed file open.
    let dir = repo();
    let mut app = collapsing_app(&dir, &["api.pb.go"]);
    app.diff_state.current_file_idx = 0;
    assert!(app.toggle_file_collapse());
    assert!(!collapsed(&app, "api.pb.go"));

    app.toggle_reviewed_for_file_idx(0, false);
    assert!(collapsed(&app, "api.pb.go"));

    // Un-reviewing falls back to the generated default, which still collapses.
    app.toggle_reviewed_for_file_idx(0, false);
    assert!(collapsed(&app, "api.pb.go"));
}

#[test]
fn space_is_inert_in_single_file_view() {
    // Single-file view never collapses, so an override would claim a change
    // that never reaches the screen.
    let dir = repo();
    let mut app = collapsing_app(&dir, &["api.pb.go"]);
    app.diff_state.current_file_idx = 0;
    app.is_single_file_view = true;

    assert!(!app.toggle_file_collapse());
    assert!(app.collapse_override.is_empty());
}

#[test]
fn summary_jump_reveals_a_collapsed_generated_file() {
    let dir = repo();
    let mut app = collapsing_app(&dir, &["api.pb.go", "src/main.rs"]);
    let path = PathBuf::from("api.pb.go");
    let comment = crate::model::Comment::new(
        "check this".to_string(),
        crate::model::CommentType::from_id("note"),
        None,
    );
    let comment_id = comment.id.clone();
    app.session
        .files
        .get_mut(&path)
        .expect("registered review file")
        .file_comments
        .push(comment);
    app.rebuild_annotations();
    assert!(collapsed(&app, "api.pb.go"));

    assert!(app.jump_to_summary_comment(SummaryCommentTarget::File {
        path: path.clone(),
        comment_id,
    }));

    assert!(!collapsed(&app, "api.pb.go"));
    assert!(matches!(
        app.line_annotations.get(app.diff_state.cursor_line),
        Some(AnnotatedLine::FileComment { .. })
    ));
}

fn note(text: &str) -> crate::model::Comment {
    crate::model::Comment::new(
        text.to_string(),
        crate::model::CommentType::from_id("note"),
        None,
    )
}

/// `file(path)` with one line edited, so its content hash differs.
fn edited_file(path: &str) -> DiffFile {
    let mut edited = file(path);
    edited.hunks[0].lines[0].content = "edited".to_string();
    edited.content_hash = DiffFile::compute_content_hash(&edited.hunks);
    edited
}

#[test]
fn space_after_a_summary_jump_collapses_the_revealed_file() {
    let dir = repo();
    let mut app = collapsing_app(&dir, &["api.pb.go"]);
    let path = PathBuf::from("api.pb.go");
    let comment = note("check this");
    let comment_id = comment.id.clone();
    app.session
        .files
        .get_mut(&path)
        .expect("registered review file")
        .file_comments
        .push(comment);
    app.rebuild_annotations();
    assert!(app.jump_to_summary_comment(SummaryCommentTarget::File { path, comment_id }));
    assert!(!collapsed(&app, "api.pb.go"));

    assert!(app.toggle_file_collapse());
    assert!(collapsed(&app, "api.pb.go"));
}

#[test]
fn space_above_the_first_file_does_not_toggle_file_zero() {
    let dir = repo();
    let mut app = collapsing_app(&dir, &["api.pb.go"]);
    app.session.review_comments.push(note("overall"));
    app.rebuild_annotations();
    app.diff_state.cursor_line = 0;
    app.diff_state.current_file_idx = 0;
    assert!(app.is_cursor_in_overview());

    assert!(!app.toggle_file_collapse());
    assert!(app.collapse_override.is_empty());
    assert!(collapsed(&app, "api.pb.go"));
}

#[test]
fn start_line_reaches_a_line_in_a_collapsed_generated_file() {
    let dir = repo();
    let mut app = collapsing_app(&dir, &["api.pb.go"]);
    app.diff_state.current_file_idx = 0;
    app.diff_state.viewport_height = 20;
    app.pending_start_line = Some(2);

    assert!(app.apply_pending_start_line());

    assert!(!collapsed(&app, "api.pb.go"));
    assert!(matches!(
        app.line_annotations.get(app.diff_state.cursor_line),
        Some(AnnotatedLine::DiffLine {
            new_lineno: Some(2),
            ..
        })
    ));
}

#[test]
fn a_comment_started_on_a_collapsed_header_opens_the_file() {
    let dir = repo();
    let mut app = collapsing_app(&dir, &["api.pb.go"]);
    app.diff_state.cursor_line = 0;
    assert!(matches!(
        app.line_annotations.first(),
        Some(AnnotatedLine::FileHeader { file_idx: 0 })
    ));

    app.enter_comment_mode(true, None);
    assert!(!collapsed(&app, "api.pb.go"));

    app.comment_buffer = "generated, but wrong".to_string();
    app.save_comment();
    assert!(
        app.line_annotations
            .iter()
            .any(|annotation| matches!(annotation, AnnotatedLine::FileComment { file_idx: 0, .. }))
    );
}

#[test]
fn collapsed_header_counts_hidden_comments() {
    let dir = repo();
    let mut app = collapsing_app(&dir, &["api.pb.go", "src/main.rs"]);
    let review = app
        .session
        .files
        .get_mut(&PathBuf::from("api.pb.go"))
        .expect("registered review file");
    review.file_comments.push(note("file"));
    review
        .line_comments
        .entry(2)
        .or_default()
        .push(note("line"));
    app.rebuild_annotations();

    let idx = |app: &App, path: &str| {
        app.diff_files
            .iter()
            .position(|f| f.display_path() == Path::new(path))
            .expect("file in diff set")
    };
    assert_eq!(app.collapsed_file_comment_count(idx(&app, "api.pb.go")), 2);
    assert_eq!(
        app.collapsed_file_comment_count(idx(&app, "src/main.rs")),
        0
    );

    // Expanded, the comments render inline and the header stops counting.
    app.diff_state.current_file_idx = idx(&app, "api.pb.go");
    app.diff_state.cursor_line = app.calculate_file_scroll_offset(app.diff_state.current_file_idx);
    assert!(app.toggle_file_collapse());
    assert_eq!(app.collapsed_file_comment_count(idx(&app, "api.pb.go")), 0);
}

#[test]
fn opting_out_in_config_rebuilds_annotations_built_with_the_defaults() {
    let dir = repo();
    let mut app = app_in(&dir, &["api.pb.go"]);
    let has_body = |app: &App| {
        app.line_annotations
            .iter()
            .any(|annotation| matches!(annotation, AnnotatedLine::HunkHeader { file_idx: 0, .. }))
    };
    assert!(!has_body(&app), "built with collapse on by default");

    app.apply_generated_config(&GeneratedConfig {
        collapse: Some(false),
        count: None,
    });

    assert!(has_body(&app));
}

#[test]
fn an_override_stops_applying_once_the_file_changes() {
    let dir = repo();
    let mut app = collapsing_app(&dir, &["src/main.rs"]);
    app.diff_state.current_file_idx = 0;
    assert!(app.toggle_file_collapse());
    assert!(collapsed(&app, "src/main.rs"));

    app.diff_files = vec![edited_file("src/main.rs")];
    app.rebuild_annotations();

    assert!(!collapsed(&app, "src/main.rs"));
}

#[test]
fn clear_drops_space_overrides() {
    let dir = repo();
    let mut app = collapsing_app(&dir, &["api.pb.go"]);
    app.diff_state.current_file_idx = 0;
    assert!(app.toggle_file_collapse());
    assert!(!collapsed(&app, "api.pb.go"));

    app.clear_comments(crate::model::ClearScope::CommentsAndReviewed);

    assert!(app.collapse_override.is_empty());
    assert!(collapsed(&app, "api.pb.go"));
}
