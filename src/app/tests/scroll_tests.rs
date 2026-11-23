use std::path::PathBuf;

use crate::app::{App, DiffSource, InputMode};
use crate::model::{
    DiffFile, DiffHunk, DiffLine, FileStatus, LineOrigin, ReviewSession, SessionDiffSource,
};
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

fn app_with_diff_lines(line_count: usize) -> App {
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
    let files = if line_count == 0 {
        Vec::new()
    } else {
        let lines = (1..=line_count)
            .map(|n| DiffLine {
                origin: LineOrigin::Addition,
                content: format!("line {n}"),
                old_lineno: None,
                new_lineno: Some(n as u32),
                raw_content: None,
                highlighted_spans: None,
            })
            .collect();
        vec![DiffFile {
            old_path: None,
            new_path: Some(PathBuf::from("src/lib.rs")),
            status: FileStatus::Added,
            hunks: vec![DiffHunk {
                header: format!("@@ -0,0 +1,{line_count} @@"),
                lines,
                old_start: 0,
                old_count: 0,
                new_start: 1,
                new_count: line_count as u32,
            }],
            is_binary: false,
            is_too_large: false,
            is_commit_message: false,
            content_hash: 0,
        }]
    };
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

#[test]
fn should_calculate_max_scroll() {
    // Last line can be scrolled to the top of the viewport: total_lines - 1
    let large = app_with_diff_lines(100);
    // FileHeader + HunkHeader + 100 DiffLines + trailing Spacing = 103
    assert_eq!(large.total_lines(), 103);
    assert_eq!(large.max_scroll_offset(), 102);

    let medium = app_with_diff_lines(17);
    assert_eq!(medium.total_lines(), 20);
    assert_eq!(medium.max_scroll_offset(), 19);
}

#[test]
fn should_handle_small_content() {
    // Even with few lines, can scroll last line to top
    let small = app_with_diff_lines(10);
    assert_eq!(small.total_lines(), 13);
    assert_eq!(small.max_scroll_offset(), 12);

    let single = app_with_diff_lines(1);
    assert_eq!(single.total_lines(), 4);
    assert_eq!(single.max_scroll_offset(), 3);
}

#[test]
fn should_handle_empty_content() {
    let empty = app_with_diff_lines(0);
    assert_eq!(empty.total_lines(), 0);
    assert_eq!(empty.max_scroll_offset(), 0);
}
