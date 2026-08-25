use crate::app::*;
use crate::comment_draft::SCISSORS;
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

/// A three-line hunk: context at 10, a deletion of old 11, an addition at 11.
fn hunk() -> DiffHunk {
    let lines = vec![
        DiffLine {
            origin: LineOrigin::Context,
            content: "    let cfg = load();".to_string(),
            old_lineno: Some(10),
            new_lineno: Some(10),
            highlighted_spans: None,
        },
        DiffLine {
            origin: LineOrigin::Deletion,
            content: "    let old = 1;".to_string(),
            old_lineno: Some(11),
            new_lineno: None,
            highlighted_spans: None,
        },
        DiffLine {
            origin: LineOrigin::Addition,
            content: "    let new = 2;".to_string(),
            old_lineno: None,
            new_lineno: Some(11),
            highlighted_spans: None,
        },
    ];
    DiffHunk {
        header: "@@ -10,2 +10,2 @@".to_string(),
        lines,
        old_start: 10,
        old_count: 2,
        new_start: 10,
        new_count: 2,
    }
}

fn app_with(files: Vec<DiffFile>) -> App {
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
        files,
        session,
        DiffSource::WorkingTree,
        InputMode::Normal,
        Vec::new(),
        None,
        None,
    )
    .expect("build app")
}

fn app_on_a_diff_line() -> App {
    let hunks = vec![hunk()];
    let content_hash = DiffFile::compute_content_hash(&hunks);
    let mut app = app_with(vec![DiffFile {
        old_path: None,
        new_path: Some(PathBuf::from("src/main.rs")),
        status: FileStatus::Modified,
        hunks,
        is_binary: false,
        is_too_large: false,
        is_commit_message: false,
        content_hash,
    }]);
    // Park the cursor on a diff row so the comment target resolves to the file
    // rather than the review overview.
    app.diff_state.cursor_line = app
        .line_annotations
        .iter()
        .position(|line| matches!(line, AnnotatedLine::DiffLine { .. }))
        .expect("a diff row");
    app
}

/// The context block, with its `# ` prefix stripped.
fn context_block(buffer: &str) -> Vec<String> {
    buffer
        .lines()
        .skip_while(|line| *line != SCISSORS)
        .skip(1)
        .map(|line| {
            line.trim_start_matches('#')
                .trim_start_matches(' ')
                .to_string()
        })
        .collect()
}

#[test]
fn a_line_comment_hands_over_the_commented_hunk() {
    let mut app = app_on_a_diff_line();
    app.enter_comment_mode(false, Some((11, LineSide::New)));
    app.comment_buffer = "this can be null".to_string();

    app.queue_comment_draft_editor();
    let buffer = app.take_pending_comment_draft().expect("queued draft");

    assert!(buffer.starts_with("this can be null\n"), "{buffer}");
    let context = context_block(&buffer);
    assert!(
        context.contains(&"Commenting on src/main.rs:11 (new)".to_string()),
        "{context:?}"
    );
    assert!(
        context.iter().any(|line| line.ends_with("let new = 2;")),
        "{context:?}"
    );
    assert!(
        context
            .iter()
            .any(|line| line.ends_with("let cfg = load();")),
        "{context:?}"
    );
}

#[test]
fn a_range_comment_names_both_ends() {
    let mut app = app_on_a_diff_line();
    app.enter_comment_mode(false, Some((11, LineSide::New)));
    app.comment_line_range = Some((LineRange::new(10, 11), LineSide::New));

    app.queue_comment_draft_editor();
    let buffer = app.take_pending_comment_draft().expect("queued draft");

    assert!(
        context_block(&buffer).contains(&"Commenting on src/main.rs:10-11 (new)".to_string()),
        "{buffer}"
    );
}

#[test]
fn a_file_comment_has_no_diff_context() {
    let mut app = app_on_a_diff_line();
    app.enter_comment_mode(true, None);

    app.queue_comment_draft_editor();
    let buffer = app.take_pending_comment_draft().expect("queued draft");

    let context = context_block(&buffer);
    assert!(
        context.contains(&"Commenting on src/main.rs (whole file)".to_string()),
        "{context:?}"
    );
    assert!(
        !context.iter().any(|line| line.contains("let new = 2;")),
        "{context:?}"
    );
}

#[test]
fn a_review_comment_targets_the_review() {
    let mut app = app_on_a_diff_line();
    app.enter_review_comment_mode();

    app.queue_comment_draft_editor();
    let buffer = app.take_pending_comment_draft().expect("queued draft");

    assert!(
        context_block(&buffer).contains(&"Commenting on the whole review".to_string()),
        "{buffer}"
    );
}

#[test]
fn nothing_is_queued_outside_comment_mode() {
    let mut app = app_on_a_diff_line();
    app.queue_comment_draft_editor();
    assert!(app.take_pending_comment_draft().is_none());
}

#[test]
fn the_edited_body_becomes_the_draft() {
    let mut app = app_on_a_diff_line();
    app.enter_comment_mode(false, Some((11, LineSide::New)));
    app.comment_buffer = "rough note".to_string();
    app.comment_cursor = app.comment_buffer.len();
    app.queue_comment_draft_editor();
    let buffer = app.take_pending_comment_draft().expect("queued draft");

    let edited = buffer.replace("rough note", "# Heading\n\npolished note");
    app.apply_comment_draft(&edited);

    assert_eq!(app.comment_buffer, "# Heading\n\npolished note");
    assert_eq!(app.comment_cursor, app.comment_buffer.len());
}

#[test]
fn an_emptied_buffer_leaves_the_draft_alone() {
    let mut app = app_on_a_diff_line();
    app.enter_comment_mode(false, Some((11, LineSide::New)));
    app.comment_buffer = "keep me".to_string();

    app.apply_comment_draft(&format!(
        "\n{SCISSORS}\n# Commenting on src/main.rs:11 (new)\n"
    ));

    assert_eq!(app.comment_buffer, "keep me");
}

#[test]
fn the_vim_overlay_is_reseeded_from_the_edited_body() {
    let mut app = app_on_a_diff_line();
    app.comment_vim_enabled = true;
    app.enter_comment_mode(false, Some((11, LineSide::New)));
    app.ensure_comment_vim_editor();
    assert!(app.comment_vim_editor.is_some());

    app.apply_comment_draft(&format!("from the editor\n{SCISSORS}\n"));

    assert_eq!(app.comment_buffer, "from the editor");
    assert!(
        app.comment_vim_editor.is_none(),
        "the stale overlay must be dropped so it reseeds"
    );
    app.ensure_comment_vim_editor();
    app.comment_vim_feed_key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('!'),
        crossterm::event::KeyModifiers::NONE,
    ));
    assert!(
        app.comment_buffer.starts_with("from the editor"),
        "{}",
        app.comment_buffer
    );
}
