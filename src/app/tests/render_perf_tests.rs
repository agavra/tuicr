//! Measures one `ui::render` frame against diffs of increasing size, so the
//! per-frame cost can be compared across branches and checked for O(total
//! diff) rather than O(visible rows) scaling.
//!
//! `cargo test --release render_perf -- --ignored --nocapture`

use crate::app::*;
use crate::model::{
    Comment, CommentType, DiffFile, DiffHunk, DiffLine, FileStatus, LineOrigin, LineRange, LineSide,
};
use crate::vcs::traits::{VcsBackend, VcsInfo, VcsType};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Style;
use std::path::PathBuf;
use std::time::Instant;

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

/// Four spans per line, matching what the syntax highlighter produces for
/// ordinary code: a bare `content` string would understate real frame cost.
fn line(idx: usize) -> DiffLine {
    let content = format!("    let value_{idx} = compute(input, {idx}); // measured line");
    let spans = vec![
        (Style::default(), "    let ".to_string()),
        (Style::default(), format!("value_{idx}")),
        (Style::default(), format!(" = compute(input, {idx});")),
        (Style::default(), " // measured line".to_string()),
    ];
    DiffLine {
        origin: if idx.is_multiple_of(3) {
            LineOrigin::Addition
        } else {
            LineOrigin::Context
        },
        content,
        old_lineno: Some(idx as u32 + 1),
        new_lineno: Some(idx as u32 + 1),
        highlighted_spans: Some(spans),
    }
}

fn file(path: &str, lines_per_file: usize) -> DiffFile {
    let lines: Vec<DiffLine> = (0..lines_per_file).map(line).collect();
    let hunks = vec![DiffHunk {
        header: format!("@@ -1,{lines_per_file} +1,{lines_per_file} @@"),
        lines,
        old_start: 1,
        old_count: lines_per_file as u32,
        new_start: 1,
        new_count: lines_per_file as u32,
    }];
    let content_hash = DiffFile::compute_content_hash(&hunks);
    DiffFile {
        old_path: Some(PathBuf::from(path)),
        new_path: Some(PathBuf::from(path)),
        status: FileStatus::Modified,
        hunks,
        is_binary: false,
        is_too_large: false,
        is_commit_message: false,
        content_hash,
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

/// A multi-paragraph body with a fenced code block, matching what a real
/// review thread carries: the markdown highlighter's cost tracks body lines,
/// not comment count.
fn comment_body(idx: usize) -> String {
    format!(
        "This looks wrong to me (#{idx}).\n\n\
         The `compute` call ignores its second argument, so every branch\n\
         collapses to the same value.\n\n\
         ```rust\n\
         let value = compute(input, {idx});\n\
         assert_eq!(value, expected);\n\
         ```\n\n\
         Can you confirm before we merge?"
    )
}

/// Median frame time in microseconds for a diff of `file_count` ×
/// `lines_per_file`, with `comments_per_file` review comments attached,
/// drawn at a realistic terminal size.
fn frame_micros_with_comments(
    file_count: usize,
    lines_per_file: usize,
    comments_per_file: usize,
) -> u128 {
    let files: Vec<DiffFile> = (0..file_count)
        .map(|i| file(&format!("src/module_{i}/file_{i}.rs"), lines_per_file))
        .collect();
    let mut app = app_with(files.clone());

    for (i, f) in files.iter().enumerate() {
        let path = f.display_path().clone();
        app.session.add_diff_file(f);
        let Some(review) = app.session.get_file_mut(&path) else {
            continue;
        };
        for c in 0..comments_per_file {
            let mut comment = Comment::new(
                comment_body(i * comments_per_file + c),
                CommentType::default(),
                Some(LineSide::New),
            );
            comment.line_range = Some(LineRange::single(c as u32 + 1));
            review.add_line_comment(c as u32 + 1, comment);
        }
    }
    app.rebuild_annotations();

    let mut terminal = Terminal::new(TestBackend::new(180, 50)).unwrap();
    let mut samples = Vec::new();
    for _ in 0..21 {
        let start = Instant::now();
        terminal
            .draw(|frame| crate::ui::render(frame, &mut app))
            .expect("draw frame");
        samples.push(start.elapsed().as_micros());
    }
    samples.sort_unstable();
    samples[samples.len() / 2]
}

/// Median frame time in microseconds for a diff of `file_count` ×
/// `lines_per_file`, drawn at a realistic terminal size.
fn frame_micros(file_count: usize, lines_per_file: usize) -> u128 {
    let files = (0..file_count)
        .map(|i| file(&format!("src/module_{i}/file_{i}.rs"), lines_per_file))
        .collect();
    let mut app = app_with(files);
    let mut terminal = Terminal::new(TestBackend::new(180, 50)).unwrap();

    let mut samples = Vec::new();
    for _ in 0..21 {
        let start = Instant::now();
        terminal
            .draw(|frame| crate::ui::render(frame, &mut app))
            .expect("draw frame");
        samples.push(start.elapsed().as_micros());
    }
    samples.sort_unstable();
    samples[samples.len() / 2]
}

#[test]
#[ignore = "timing measurement, run explicitly"]
fn render_perf_scaling() {
    for (files, lines) in [(1, 200), (10, 200), (50, 200), (100, 200), (200, 200)] {
        let micros = frame_micros(files, lines);
        println!(
            "{files:>4} files x {lines} lines = {:>7} diff lines: {:>8.2} ms/frame",
            files * lines,
            micros as f64 / 1000.0
        );
    }
}

#[test]
#[ignore = "timing measurement, run explicitly"]
fn render_perf_with_comments() {
    for (files, comments) in [(20, 0), (20, 1), (20, 3), (20, 10)] {
        let micros = frame_micros_with_comments(files, 200, comments);
        println!(
            "{files} files x 200 lines, {:>4} comments total: {:>8.2} ms/frame",
            files * comments,
            micros as f64 / 1000.0
        );
    }
}
