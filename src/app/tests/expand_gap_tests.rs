use crate::app::*;
use crate::model::{DiffHunk, DiffLine, FileStatus, LineOrigin};
use crate::vcs::traits::VcsType;

struct MockVcs {
    info: VcsInfo,
    /// Total lines available in the "file" (1-indexed)
    total_lines: u32,
}

impl VcsBackend for MockVcs {
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
        start_line: u32,
        end_line: u32,
    ) -> Result<Vec<DiffLine>> {
        let mut result = Vec::new();
        for line_num in start_line..=end_line.min(self.total_lines) {
            result.push(DiffLine {
                origin: LineOrigin::Context,
                content: format!("line {line_num}"),
                old_lineno: Some(line_num),
                new_lineno: Some(line_num),
                highlighted_spans: None,
            });
        }
        Ok(result)
    }

    fn file_line_count(
        &self,
        _file_path: &Path,
        _file_status: FileStatus,
        _ref_commit: Option<&str>,
    ) -> Result<u32> {
        Ok(self.total_lines)
    }
}

fn make_hunk(new_start: u32, new_count: u32) -> DiffHunk {
    let mut lines = Vec::new();
    for i in 0..new_count {
        lines.push(DiffLine {
            origin: LineOrigin::Context,
            content: format!("hunk line {}", new_start + i),
            old_lineno: Some(new_start + i),
            new_lineno: Some(new_start + i),
            highlighted_spans: None,
        });
    }
    DiffHunk {
        header: format!("@@ -{new_start},{new_count} +{new_start},{new_count} @@"),
        lines,
        old_start: new_start,
        old_count: new_count,
        new_start,
        new_count,
    }
}

fn build_app_with_files(files: Vec<DiffFile>, total_lines: u32) -> App {
    let vcs_info = VcsInfo {
        root_path: PathBuf::from("/tmp"),
        head_commit: "abc123".to_string(),
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
        Box::new(MockVcs {
            info: vcs_info.clone(),
            total_lines,
        }),
        vcs_info,
        Theme::dark(),
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
    .expect("failed to build test app")
}

fn make_file_with_hunks(path: &str, hunks: Vec<DiffHunk>) -> DiffFile {
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

fn hunk_diff_line(app: &App, file_idx: usize, hunk_idx: usize) -> usize {
    app.line_annotations
        .iter()
        .position(|line| {
            matches!(
                line,
                AnnotatedLine::DiffLine {
                    file_idx: candidate_file_idx,
                    hunk_idx: candidate_hunk_idx,
                    ..
                } if *candidate_file_idx == file_idx && *candidate_hunk_idx == hunk_idx
            )
        })
        .expect("missing hunk diff annotation")
}

#[test]
fn should_toggle_hunk_reviewed_from_header() {
    let file = make_file_with_hunks("test.rs", vec![make_hunk(1, 3)]);
    let mut app = build_app_with_files(vec![file], 20);
    let path = app.diff_files[0].display_path().clone();
    let key = app.diff_files[0].hunk_review_key(0).unwrap();

    app.diff_state.cursor_line = app.hunk_header_line(0, 0).expect("missing hunk header");
    app.toggle_hunk_reviewed();

    assert!(app.session.is_hunk_reviewed(&path, &key));
    assert_eq!(
        app.message.as_ref().unwrap().content,
        "Hunk marked reviewed"
    );
    assert!(matches!(
        app.line_annotations[app.diff_state.cursor_line],
        AnnotatedLine::HunkHeader {
            file_idx: 0,
            hunk_idx: 0
        }
    ));
}

#[test]
fn should_toggle_hunk_reviewed_from_diff_line() {
    let file = make_file_with_hunks("test.rs", vec![make_hunk(1, 3)]);
    let mut app = build_app_with_files(vec![file], 20);
    let path = app.diff_files[0].display_path().clone();
    let key = app.diff_files[0].hunk_review_key(0).unwrap();

    app.diff_state.cursor_line = hunk_diff_line(&app, 0, 0);
    app.toggle_hunk_reviewed();

    assert!(app.session.is_hunk_reviewed(&path, &key));
}

#[test]
fn should_warn_when_toggling_hunk_outside_hunk() {
    let file = make_file_with_hunks("test.rs", vec![make_hunk(1, 3)]);
    let mut app = build_app_with_files(vec![file], 20);
    let path = app.diff_files[0].display_path().clone();
    let key = app.diff_files[0].hunk_review_key(0).unwrap();

    app.diff_state.cursor_line = 0;
    app.toggle_hunk_reviewed();

    assert!(!app.session.is_hunk_reviewed(&path, &key));
    assert_eq!(
        app.message.as_ref().unwrap().content,
        "Move cursor to a hunk to toggle reviewed"
    );
    assert_eq!(
        app.message.as_ref().unwrap().message_type,
        MessageType::Warning
    );
}

#[test]
fn should_fold_reviewed_hunk_body() {
    let file = make_file_with_hunks("test.rs", vec![make_hunk(1, 3), make_hunk(10, 2)]);
    let mut app = build_app_with_files(vec![file], 20);
    let before = app.total_lines();

    app.diff_state.cursor_line = app.hunk_header_line(0, 0).expect("missing hunk header");
    app.toggle_hunk_reviewed();

    assert_eq!(app.total_lines(), before - 4);
    assert!(!app.line_annotations.iter().any(|line| {
        matches!(
            line,
            AnnotatedLine::DiffLine {
                file_idx: 0,
                hunk_idx: 0,
                ..
            }
        )
    }));
    assert!(!app.line_annotations.iter().any(|line| {
        matches!(
            line,
            AnnotatedLine::Expander {
                gap_id: GapId {
                    file_idx: 0,
                    hunk_idx: 1
                },
                ..
            } | AnnotatedLine::HiddenLines {
                gap_id: GapId {
                    file_idx: 0,
                    hunk_idx: 1
                },
                ..
            }
        )
    }));
    assert!(app.line_annotations.iter().any(|line| {
        matches!(
            line,
            AnnotatedLine::DiffLine {
                file_idx: 0,
                hunk_idx: 1,
                ..
            }
        )
    }));
}

#[test]
fn should_keep_file_and_hunk_reviewed_state_independent() {
    let file = make_file_with_hunks("test.rs", vec![make_hunk(1, 3)]);
    let mut app = build_app_with_files(vec![file], 20);
    let path = app.diff_files[0].display_path().clone();
    let key = app.diff_files[0].hunk_review_key(0).unwrap();

    app.diff_state.cursor_line = app.hunk_header_line(0, 0).expect("missing hunk header");
    app.toggle_hunk_reviewed();
    app.toggle_reviewed_for_file_idx(0, false);

    assert!(app.session.is_file_reviewed(&path));
    assert!(app.session.is_hunk_reviewed(&path, &key));
}

#[test]
fn should_expand_up_from_first_hunk() {
    // given: file with 50-line gap before first hunk (hunk starts at line 51)
    let file = make_file_with_hunks("test.rs", vec![make_hunk(51, 5)]);
    let mut app = build_app_with_files(vec![file], 100);
    let gap_id = GapId {
        file_idx: 0,
        hunk_idx: 0,
    };

    // when: expand Up with limit 20 (reveals lines closest to hunk)
    app.expand_gap(gap_id.clone(), ExpandDirection::Up, Some(20))
        .unwrap();

    // then: 20 lines expanded from the bottom of the gap (lines 31-50)
    let content = app.expanded_bottom.get(&gap_id).unwrap();
    assert_eq!(content.len(), 20);
    assert_eq!(content[0].new_lineno, Some(31));
    assert_eq!(content[19].new_lineno, Some(50));
}

#[test]
fn should_expand_all_lines_with_both_direction() {
    // given: file with 50-line gap before first hunk
    let file = make_file_with_hunks("test.rs", vec![make_hunk(51, 5)]);
    let mut app = build_app_with_files(vec![file], 100);
    let gap_id = GapId {
        file_idx: 0,
        hunk_idx: 0,
    };

    // when: expand Both (all remaining)
    app.expand_gap(gap_id.clone(), ExpandDirection::Both, None)
        .unwrap();

    // then: all 50 lines in expanded_top
    let content = app.expanded_top.get(&gap_id).unwrap();
    assert_eq!(content.len(), 50);
    assert_eq!(content[0].new_lineno, Some(1));
    assert_eq!(content[49].new_lineno, Some(50));
}

#[test]
fn should_expand_down_from_upper_hunk() {
    // given: file with two hunks, gap of 24 lines (6..29) between them
    let file = make_file_with_hunks("test.rs", vec![make_hunk(1, 5), make_hunk(30, 5)]);
    let mut app = build_app_with_files(vec![file], 100);
    let gap_id = GapId {
        file_idx: 0,
        hunk_idx: 1,
    };

    // when: expand Down with limit 10
    app.expand_gap(gap_id.clone(), ExpandDirection::Down, Some(10))
        .unwrap();

    // then: 10 lines from top of gap (lines 6-15)
    let content = app.expanded_top.get(&gap_id).unwrap();
    assert_eq!(content.len(), 10);
    assert_eq!(content[0].new_lineno, Some(6));
    assert_eq!(content[9].new_lineno, Some(15));
}

#[test]
fn should_expand_up_from_lower_hunk() {
    // given: file with two hunks, gap of 24 lines (6..29) between them
    let file = make_file_with_hunks("test.rs", vec![make_hunk(1, 5), make_hunk(30, 5)]);
    let mut app = build_app_with_files(vec![file], 100);
    let gap_id = GapId {
        file_idx: 0,
        hunk_idx: 1,
    };

    // when: expand Up with limit 10
    app.expand_gap(gap_id.clone(), ExpandDirection::Up, Some(10))
        .unwrap();

    // then: 10 lines from bottom of gap (lines 20-29)
    let content = app.expanded_bottom.get(&gap_id).unwrap();
    assert_eq!(content.len(), 10);
    assert_eq!(content[0].new_lineno, Some(20));
    assert_eq!(content[9].new_lineno, Some(29));
}

#[test]
fn should_append_on_subsequent_down_expand() {
    // given: already expanded 20 lines down
    let file = make_file_with_hunks("test.rs", vec![make_hunk(1, 5), make_hunk(50, 5)]);
    let mut app = build_app_with_files(vec![file], 100);
    let gap_id = GapId {
        file_idx: 0,
        hunk_idx: 1,
    };
    app.expand_gap(gap_id.clone(), ExpandDirection::Down, Some(20))
        .unwrap();

    // when: expand Down 20 more
    app.expand_gap(gap_id.clone(), ExpandDirection::Down, Some(20))
        .unwrap();

    // then: 40 lines total in top
    let content = app.expanded_top.get(&gap_id).unwrap();
    assert_eq!(content.len(), 40);
    assert_eq!(content[0].new_lineno, Some(6));
    assert_eq!(content[39].new_lineno, Some(45));
}

#[test]
fn should_prepend_on_subsequent_up_expand() {
    // given: already expanded 10 lines up from bottom
    let file = make_file_with_hunks("test.rs", vec![make_hunk(1, 5), make_hunk(50, 5)]);
    let mut app = build_app_with_files(vec![file], 100);
    let gap_id = GapId {
        file_idx: 0,
        hunk_idx: 1,
    };
    app.expand_gap(gap_id.clone(), ExpandDirection::Up, Some(10))
        .unwrap();

    // when: expand Up 10 more
    app.expand_gap(gap_id.clone(), ExpandDirection::Up, Some(10))
        .unwrap();

    // then: 20 lines total in bottom, in ascending order
    let content = app.expanded_bottom.get(&gap_id).unwrap();
    assert_eq!(content.len(), 20);
    assert_eq!(content[0].new_lineno, Some(30));
    assert_eq!(content[19].new_lineno, Some(49));
}

#[test]
fn should_cap_at_gap_boundaries() {
    // given: file with 50-line gap, already expanded 40 up
    let file = make_file_with_hunks("test.rs", vec![make_hunk(51, 5)]);
    let mut app = build_app_with_files(vec![file], 100);
    let gap_id = GapId {
        file_idx: 0,
        hunk_idx: 0,
    };
    app.expand_gap(gap_id.clone(), ExpandDirection::Up, Some(40))
        .unwrap();

    // when: expand Up 20 more (only 10 remain)
    app.expand_gap(gap_id.clone(), ExpandDirection::Up, Some(20))
        .unwrap();

    // then: all 50 lines in bottom
    let content = app.expanded_bottom.get(&gap_id).unwrap();
    assert_eq!(content.len(), 50);
    assert_eq!(content[0].new_lineno, Some(1));
}

#[test]
fn should_show_up_expander_for_top_of_file_partial() {
    // given: file with 50-line gap, expanded 20 lines up
    let file = make_file_with_hunks("test.rs", vec![make_hunk(51, 5)]);
    let mut app = build_app_with_files(vec![file], 100);
    let gap_id = GapId {
        file_idx: 0,
        hunk_idx: 0,
    };
    app.expand_gap(gap_id.clone(), ExpandDirection::Up, Some(20))
        .unwrap();

    // then: should have ↑ expander + hidden lines annotation
    let expander_count = app
            .line_annotations
            .iter()
            .filter(|a| matches!(a, AnnotatedLine::Expander { gap_id: g, direction: ExpandDirection::Up } if *g == gap_id))
            .count();
    assert_eq!(expander_count, 1);

    let hidden_count = app
        .line_annotations
        .iter()
        .filter(|a| matches!(a, AnnotatedLine::HiddenLines { gap_id: g, .. } if *g == gap_id))
        .count();
    assert_eq!(hidden_count, 1, "should show hidden lines count");

    let expanded_count = app
        .line_annotations
        .iter()
        .filter(|a| matches!(a, AnnotatedLine::ExpandedContext { gap_id: g, .. } if *g == gap_id))
        .count();
    assert_eq!(expanded_count, 20);
}

#[test]
fn should_not_show_expander_when_fully_expanded() {
    // given: file with 50-line gap, fully expanded
    let file = make_file_with_hunks("test.rs", vec![make_hunk(51, 5)]);
    let mut app = build_app_with_files(vec![file], 100);
    let gap_id = GapId {
        file_idx: 0,
        hunk_idx: 0,
    };
    app.expand_gap(gap_id.clone(), ExpandDirection::Both, None)
        .unwrap();

    // then: no expander or hidden lines
    let expander_count = app
        .line_annotations
        .iter()
        .filter(|a| matches!(a, AnnotatedLine::Expander { gap_id: g, .. } if *g == gap_id))
        .count();
    assert_eq!(expander_count, 0);
}

#[test]
fn should_show_merged_expander_for_small_between_hunk_gap() {
    // given: file with two hunks and a 15-line gap between them
    let file = make_file_with_hunks("test.rs", vec![make_hunk(1, 5), make_hunk(21, 5)]);
    let app = build_app_with_files(vec![file], 100);
    let gap_id = GapId {
        file_idx: 0,
        hunk_idx: 1,
    };

    // then: should show single ↕ expander (gap=15, < 20)
    let both_count = app
            .line_annotations
            .iter()
            .filter(|a| matches!(a, AnnotatedLine::Expander { gap_id: g, direction: ExpandDirection::Both } if *g == gap_id))
            .count();
    assert_eq!(both_count, 1, "small gap should show merged ↕ expander");
}

#[test]
fn should_show_split_expanders_for_large_between_hunk_gap() {
    // given: file with two hunks and a 30-line gap between them
    let file = make_file_with_hunks("test.rs", vec![make_hunk(1, 5), make_hunk(36, 5)]);
    let app = build_app_with_files(vec![file], 100);
    let gap_id = GapId {
        file_idx: 0,
        hunk_idx: 1,
    };

    // then: should show ↓ + hidden + ↑
    let down_count = app
            .line_annotations
            .iter()
            .filter(|a| matches!(a, AnnotatedLine::Expander { gap_id: g, direction: ExpandDirection::Down } if *g == gap_id))
            .count();
    let up_count = app
            .line_annotations
            .iter()
            .filter(|a| matches!(a, AnnotatedLine::Expander { gap_id: g, direction: ExpandDirection::Up } if *g == gap_id))
            .count();
    let hidden_count = app
        .line_annotations
        .iter()
        .filter(|a| matches!(a, AnnotatedLine::HiddenLines { gap_id: g, .. } if *g == gap_id))
        .count();
    assert_eq!(down_count, 1);
    assert_eq!(up_count, 1);
    assert_eq!(hidden_count, 1);
}

#[test]
fn should_expand_gap_in_correct_file_not_adjacent_file() {
    // given: two files, each with a gap before the first hunk
    let file0 = make_file_with_hunks("a.rs", vec![make_hunk(31, 5)]);
    let file1 = make_file_with_hunks("b.rs", vec![make_hunk(21, 5)]);
    let mut app = build_app_with_files(vec![file0, file1], 100);

    let gap_id_file1 = GapId {
        file_idx: 1,
        hunk_idx: 0,
    };

    // when: expand gap in file1
    app.expand_gap(gap_id_file1.clone(), ExpandDirection::Up, Some(10))
        .unwrap();

    // then: expanded content is for file1's gap (10 lines from bottom)
    let content = app.expanded_bottom.get(&gap_id_file1).unwrap();
    assert_eq!(content.len(), 10);
    assert_eq!(content[9].new_lineno, Some(20));

    // and file0's gap should not be expanded
    let gap_id_file0 = GapId {
        file_idx: 0,
        hunk_idx: 0,
    };
    assert!(
        !app.expanded_top.contains_key(&gap_id_file0)
            && !app.expanded_bottom.contains_key(&gap_id_file0)
    );
}

#[test]
fn should_noop_when_already_fully_expanded() {
    // given: file with 10-line gap, fully expanded
    let file = make_file_with_hunks("test.rs", vec![make_hunk(11, 5)]);
    let mut app = build_app_with_files(vec![file], 100);
    let gap_id = GapId {
        file_idx: 0,
        hunk_idx: 0,
    };
    app.expand_gap(gap_id.clone(), ExpandDirection::Both, None)
        .unwrap();
    let len_before = app.expanded_top.get(&gap_id).unwrap().len();

    // when: try to expand again
    app.expand_gap(gap_id.clone(), ExpandDirection::Up, Some(20))
        .unwrap();

    // then: no change
    let len_after = app.expanded_top.get(&gap_id).unwrap().len();
    assert_eq!(len_before, len_after);
}

#[test]
fn should_expand_small_gap_fully_even_with_large_limit() {
    // given: file with 5-line gap
    let file = make_file_with_hunks("test.rs", vec![make_hunk(6, 5)]);
    let mut app = build_app_with_files(vec![file], 100);
    let gap_id = GapId {
        file_idx: 0,
        hunk_idx: 0,
    };

    // when: expand Up with limit 20 (gap is only 5 lines)
    app.expand_gap(gap_id.clone(), ExpandDirection::Up, Some(20))
        .unwrap();

    // then: all 5 lines expanded, no expander remaining
    let content = app.expanded_bottom.get(&gap_id).unwrap();
    assert_eq!(content.len(), 5);

    let expander_count = app
        .line_annotations
        .iter()
        .filter(|a| matches!(a, AnnotatedLine::Expander { gap_id: g, .. } if *g == gap_id))
        .count();
    assert_eq!(expander_count, 0);
}

#[test]
fn should_merge_to_both_when_remaining_drops_below_batch() {
    // given: 30-line between-hunk gap, expand 20 down => 10 remaining
    let file = make_file_with_hunks("test.rs", vec![make_hunk(1, 5), make_hunk(36, 5)]);
    let mut app = build_app_with_files(vec![file], 100);
    let gap_id = GapId {
        file_idx: 0,
        hunk_idx: 1,
    };
    app.expand_gap(gap_id.clone(), ExpandDirection::Down, Some(20))
        .unwrap();

    // then: remaining=10, should show ↕ merged expander
    let both_count = app
            .line_annotations
            .iter()
            .filter(|a| matches!(a, AnnotatedLine::Expander { gap_id: g, direction: ExpandDirection::Both } if *g == gap_id))
            .count();
    assert_eq!(both_count, 1, "should merge to ↕ when <20 remaining");
}

fn cursor_new_lineno(app: &App) -> Option<u32> {
    match &app.line_annotations[app.diff_state.cursor_line] {
        AnnotatedLine::DiffLine { new_lineno, .. }
        | AnnotatedLine::SideBySideLine { new_lineno, .. } => *new_lineno,
        AnnotatedLine::ExpandedContext { gap_id, line_idx } => app
            .get_expanded_line(gap_id, *line_idx)
            .and_then(|l| l.new_lineno),
        _ => None,
    }
}

fn cursor_old_lineno(app: &App) -> Option<u32> {
    match &app.line_annotations[app.diff_state.cursor_line] {
        AnnotatedLine::DiffLine { old_lineno, .. }
        | AnnotatedLine::SideBySideLine { old_lineno, .. } => *old_lineno,
        AnnotatedLine::ExpandedContext { gap_id, line_idx } => app
            .get_expanded_line(gap_id, *line_idx)
            .and_then(|l| l.old_lineno),
        _ => None,
    }
}

#[test]
fn should_expand_collapsed_gap_when_jumping_into_it() {
    // given: file with a 50-line gap before the first hunk (hunk @ line 51)
    let file = make_file_with_hunks("test.rs", vec![make_hunk(51, 5)]);
    let mut app = build_app_with_files(vec![file], 100);
    let gap_id = GapId {
        file_idx: 0,
        hunk_idx: 0,
    };
    assert!(!app.expanded_top.contains_key(&gap_id));
    assert!(!app.expanded_bottom.contains_key(&gap_id));

    // when: jump to line 30, which lives inside the collapsed gap
    app.go_to_source_line(30, LineSide::New);

    // then: the gap was expanded and the cursor sits on the line whose
    // new_lineno is exactly 30
    assert_eq!(cursor_new_lineno(&app), Some(30));
}

#[test]
fn should_not_expand_when_line_is_already_in_a_hunk() {
    // Line 52 lives inside the hunk's own range, not in a gap — no
    // expansion should occur.
    let file = make_file_with_hunks("test.rs", vec![make_hunk(51, 5)]);
    let mut app = build_app_with_files(vec![file], 100);
    let gap_id = GapId {
        file_idx: 0,
        hunk_idx: 0,
    };

    app.go_to_source_line(52, LineSide::New);

    assert!(!app.expanded_top.contains_key(&gap_id));
    assert!(!app.expanded_bottom.contains_key(&gap_id));
}

#[test]
fn should_expand_old_side_gap_when_jumping_with_o_prefix() {
    // Symmetric gap (offset = 0): `make_hunk` keeps old_start == new_start
    // and the mock VCS returns context lines with old_lineno == new_lineno.
    // We verify the side=Old path of go_to_source_line works end-to-end:
    // the gap auto-expands and the cursor lands on old line 30.
    let file = make_file_with_hunks("test.rs", vec![make_hunk(51, 5)]);
    let mut app = build_app_with_files(vec![file], 100);

    app.go_to_source_line(30, LineSide::Old);

    assert_eq!(cursor_old_lineno(&app), Some(30));
}

#[test]
fn should_expand_up_when_cursor_is_below_the_gap() {
    // Two hunks: hunk0 at lines 1-5, hunk1 at lines 50-54. Gap between
    // them spans new lines 6..=49. Move the cursor onto hunk1, then
    // jump to line 30 — expansion should come from the bottom of the
    // gap (Up) since the cursor sits below it.
    let file = make_file_with_hunks("test.rs", vec![make_hunk(1, 5), make_hunk(50, 5)]);
    let mut app = build_app_with_files(vec![file], 100);
    let gap_id = GapId {
        file_idx: 0,
        hunk_idx: 1,
    };

    let hunk1_header_idx = app
        .line_annotations
        .iter()
        .enumerate()
        .find_map(|(i, a)| match a {
            AnnotatedLine::HunkHeader { hunk_idx: 1, .. } => Some(i),
            _ => None,
        })
        .expect("hunk 1 header should exist");
    app.diff_state.cursor_line = hunk1_header_idx + 1;

    app.go_to_source_line(30, LineSide::New);

    assert_eq!(cursor_new_lineno(&app), Some(30));
    assert!(
        !app.expanded_top.contains_key(&gap_id),
        "no top expansion when cursor is below the gap"
    );
    // Gap covers new lines 6..=49 (44 lines). Up expansion to reach
    // line 30 reveals lines 30..=49 = 20 lines.
    let bot_len = app.expanded_bottom.get(&gap_id).map_or(0, |v| v.len());
    assert_eq!(bot_len, 20);
    let has_down_expander = app.line_annotations.iter().any(|a| {
        matches!(
            a,
            AnnotatedLine::Expander {
                gap_id: g,
                direction: ExpandDirection::Down,
            } if *g == gap_id
        )
    });
    assert!(
        has_down_expander,
        "remaining hidden lines need a `↓` expander above the cursor"
    );
}

#[test]
fn should_expand_only_up_to_target_line_not_full_gap() {
    // Gap before hunk spans new lines 1..=50. Jumping to line 20 should
    // reveal lines 1..=20 and leave 21..=50 collapsed behind an
    // `↑` expander.
    let file = make_file_with_hunks("test.rs", vec![make_hunk(51, 5)]);
    let mut app = build_app_with_files(vec![file], 100);
    let gap_id = GapId {
        file_idx: 0,
        hunk_idx: 0,
    };

    app.go_to_source_line(20, LineSide::New);

    assert_eq!(cursor_new_lineno(&app), Some(20));
    let top_len = app.expanded_top.get(&gap_id).map_or(0, |v| v.len());
    assert_eq!(top_len, 20, "only the lines up to the target should expand");
    assert!(
        !app.expanded_bottom.contains_key(&gap_id),
        "no bottom expansion should happen for a downward jump"
    );
    // The unexpanded remainder (30 lines) should still be reachable through
    // an `↑` expander between the cursor and the next hunk.
    let has_up_expander = app.line_annotations.iter().any(|a| {
        matches!(
            a,
            AnnotatedLine::Expander {
                gap_id: g,
                direction: ExpandDirection::Up,
            } if *g == gap_id
        )
    });
    assert!(
        has_up_expander,
        "remaining hidden lines need an `↑` expander"
    );
}

#[test]
fn should_show_end_of_file_expander() {
    // given: file with hunk at lines 1-5 and total 100 lines (95 lines after hunk)
    let file = make_file_with_hunks("test.rs", vec![make_hunk(1, 5)]);
    let app = build_app_with_files(vec![file], 100);
    let eof_gap_id = GapId {
        file_idx: 0,
        hunk_idx: 1, // hunks.len() == 1
    };

    // then: should have ↓ expander for end-of-file gap
    let expander_count = app
            .line_annotations
            .iter()
            .filter(|a| matches!(a, AnnotatedLine::Expander { gap_id: g, direction: ExpandDirection::Down } if *g == eof_gap_id))
            .count();
    assert_eq!(expander_count, 1, "should show ↓ expander at end of file");

    // and: should have HiddenLines (95 lines > 20)
    let hidden_count = app
            .line_annotations
            .iter()
            .filter(|a| matches!(a, AnnotatedLine::HiddenLines { gap_id: g, count } if *g == eof_gap_id && *count == 95))
            .count();
    assert_eq!(hidden_count, 1, "should show hidden lines count");

    // and: total_lines() must match line_annotations.len()
    assert_eq!(
        app.total_lines(),
        app.line_annotations.len(),
        "file_render_height sum must match annotation count"
    );
}

#[test]
fn should_expand_down_at_end_of_file() {
    // given: file with hunk at lines 1-5, total 100 lines
    let file = make_file_with_hunks("test.rs", vec![make_hunk(1, 5)]);
    let mut app = build_app_with_files(vec![file], 100);
    let eof_gap_id = GapId {
        file_idx: 0,
        hunk_idx: 1,
    };

    // when: expand Down 20 lines
    app.expand_gap(eof_gap_id.clone(), ExpandDirection::Down, Some(20))
        .unwrap();

    // then: 20 lines expanded from start of EOF gap (lines 6-25)
    let content = app.expanded_top.get(&eof_gap_id).unwrap();
    assert_eq!(content.len(), 20);
    assert_eq!(content[0].new_lineno, Some(6));
    assert_eq!(content[19].new_lineno, Some(25));
}

#[test]
fn should_not_show_eof_gap_when_hunk_ends_at_file_end() {
    // given: file with hunk at lines 1-100 and total 100 lines (no gap)
    let file = make_file_with_hunks("test.rs", vec![make_hunk(1, 100)]);
    let app = build_app_with_files(vec![file], 100);
    let eof_gap_id = GapId {
        file_idx: 0,
        hunk_idx: 1,
    };

    // then: no expander for end-of-file
    let expander_count = app
        .line_annotations
        .iter()
        .filter(|a| matches!(a, AnnotatedLine::Expander { gap_id: g, .. } if *g == eof_gap_id))
        .count();
    assert_eq!(
        expander_count, 0,
        "no EOF expander when hunk covers entire file"
    );
}

#[test]
fn should_handle_subsequent_eof_expansions() {
    // given: file with hunk at lines 1-5, total 50 lines, already expanded 20
    let file = make_file_with_hunks("test.rs", vec![make_hunk(1, 5)]);
    let mut app = build_app_with_files(vec![file], 50);
    let eof_gap_id = GapId {
        file_idx: 0,
        hunk_idx: 1,
    };
    app.expand_gap(eof_gap_id.clone(), ExpandDirection::Down, Some(20))
        .unwrap();

    // when: expand Down 20 more
    app.expand_gap(eof_gap_id.clone(), ExpandDirection::Down, Some(20))
        .unwrap();

    // then: 40 lines total (lines 6-45)
    let content = app.expanded_top.get(&eof_gap_id).unwrap();
    assert_eq!(content.len(), 40);
    assert_eq!(content[0].new_lineno, Some(6));
    assert_eq!(content[39].new_lineno, Some(45));
}

#[test]
fn should_show_small_eof_gap_expander_without_hidden_lines() {
    // given: file with hunk at lines 1-90 and total 100 lines (10-line gap)
    let file = make_file_with_hunks("test.rs", vec![make_hunk(1, 90)]);
    let app = build_app_with_files(vec![file], 100);
    let eof_gap_id = GapId {
        file_idx: 0,
        hunk_idx: 1,
    };

    // then: should have ↓ expander
    let expander_count = app
            .line_annotations
            .iter()
            .filter(|a| matches!(a, AnnotatedLine::Expander { gap_id: g, direction: ExpandDirection::Down } if *g == eof_gap_id))
            .count();
    assert_eq!(expander_count, 1);

    // and: should NOT have HiddenLines (10 <= 20)
    let hidden_count = app
        .line_annotations
        .iter()
        .filter(|a| matches!(a, AnnotatedLine::HiddenLines { gap_id: g, .. } if *g == eof_gap_id))
        .count();
    assert_eq!(
        hidden_count, 0,
        "small EOF gap should not show hidden lines"
    );
}

#[test]
fn total_lines_must_match_annotations_with_remote_threads() {
    // Regression: file_render_height previously ignored RemoteThreadLine
    // rows pushed by rebuild_annotations, so max_scroll_offset clamped
    // above the tail of a remote thread on the last line of the last file.
    use crate::forge::remote_comments::{
        RemoteCommentSide, RemoteReviewComment, RemoteReviewThread,
    };

    let files = vec![
        make_file_with_hunks("a.rs", vec![make_hunk(1, 5)]),
        make_file_with_hunks("b.rs", vec![make_hunk(1, 1)]),
    ];
    let mut app = build_app_with_files(files, 1);
    app.sync_viewport_width(80);

    app.forge_review_threads = vec![RemoteReviewThread {
        id: "T1".into(),
        path: "b.rs".into(),
        line: Some(1),
        side: RemoteCommentSide::Right,
        is_resolved: false,
        is_outdated: false,
        comments: vec![RemoteReviewComment {
            id: "C1".into(),
            author: Some("alice".into()),
            body: "first\nsecond\nthird\nfourth".into(),
            created_at: None,
            in_reply_to: None,
            url: "https://example.com/c1".into(),
        }],
    }];
    app.rebuild_annotations();

    assert_eq!(app.total_lines(), app.line_annotations.len());
}

#[test]
fn total_lines_must_match_annotations_when_commit_scoped_comment_is_hidden() {
    // Regression: file_render_body_height counted all comments unconditionally
    // while rebuild_annotations and the renderers filtered commit-hidden ones,
    // so total_lines() exceeded line_annotations.len() and scroll math drifted.
    let files = vec![make_file_with_hunks("a.rs", vec![make_hunk(1, 5)])];
    let mut app = build_app_with_files(files, 5);
    app.sync_viewport_width(80);

    // Add a line comment scoped to commit "aaa" on line 1.
    let mut comment = Comment::new(
        "scoped to aaa".to_string(),
        CommentType::Note,
        Some(LineSide::New),
    );
    comment.commit_id = Some("aaa".to_string());
    app.session
        .files
        .get_mut(&PathBuf::from("a.rs"))
        .unwrap()
        .add_line_comment(1, comment);

    // Select a different commit "bbb" so the comment is hidden.
    app.review_commits = vec![
        crate::vcs::traits::CommitInfo {
            id: "aaa".to_string(),
            short_id: "aaa".to_string(),
            branch_name: None,
            summary: "commit aaa".to_string(),
            body: None,
            author: "tester".to_string(),
            time: chrono::Utc::now(),
        },
        crate::vcs::traits::CommitInfo {
            id: "bbb".to_string(),
            short_id: "bbb".to_string(),
            branch_name: None,
            summary: "commit bbb".to_string(),
            body: None,
            author: "tester".to_string(),
            time: chrono::Utc::now(),
        },
    ];
    app.commit_selection_range = Some((1, 1)); // only "bbb"

    app.rebuild_annotations();

    assert_eq!(
        app.total_lines(),
        app.line_annotations.len(),
        "total_lines must match annotations when a commit-scoped comment is filtered out"
    );
}

#[test]
fn comment_navigator_items_follow_rendered_comment_order() {
    use crate::forge::remote_comments::{
        RemoteCommentSide, RemoteReviewComment, RemoteReviewThread,
    };

    let files = vec![
        make_file_with_hunks("a.rs", vec![make_hunk(1, 1)]),
        make_file_with_hunks("b.rs", vec![make_hunk(1, 2)]),
    ];
    let mut app = build_app_with_files(files, 10);
    app.sync_viewport_width(80);

    app.session.review_comments.push(Comment::new(
        "review-level".to_string(),
        CommentType::Note,
        None,
    ));
    app.session
        .files
        .get_mut(&PathBuf::from("a.rs"))
        .unwrap()
        .file_comments
        .push(Comment::new(
            "file-level".to_string(),
            CommentType::Suggestion,
            None,
        ));
    app.session
        .files
        .get_mut(&PathBuf::from("b.rs"))
        .unwrap()
        .line_comments
        .entry(2)
        .or_default()
        .push(Comment::new(
            "line-level".to_string(),
            CommentType::Issue,
            Some(LineSide::New),
        ));
    app.forge_review_threads = vec![RemoteReviewThread {
        id: "T1".into(),
        path: "b.rs".into(),
        line: Some(2),
        side: RemoteCommentSide::Right,
        is_resolved: false,
        is_outdated: false,
        comments: vec![RemoteReviewComment {
            id: "C1".into(),
            author: Some("alice".into()),
            body: "remote-thread".into(),
            created_at: None,
            in_reply_to: None,
            url: "https://example.com/c1".into(),
        }],
    }];
    app.rebuild_annotations();

    let items = app.build_comment_navigator_items();

    assert_eq!(items.len(), 4);
    assert!(matches!(
        items[0].key,
        CommentNavigatorKey::Review { comment_idx: 0 }
    ));
    assert!(matches!(
        items[1].key,
        CommentNavigatorKey::File {
            file_idx: 0,
            comment_idx: 0
        }
    ));
    assert!(matches!(
        items[2].key,
        CommentNavigatorKey::Line {
            file_idx: 1,
            line: 2,
            side: LineSide::New,
            comment_idx: 0
        }
    ));
    assert!(matches!(
        items[3].key,
        CommentNavigatorKey::Remote { thread_idx: 0 }
    ));
    assert_eq!(items[3].author.as_deref(), Some("alice"));
}

#[test]
fn comment_navigator_includes_remote_review_summary() {
    use crate::forge::remote_comments::{RemoteReviewState, RemoteReviewSummary};

    let file = make_file_with_hunks("a.rs", vec![make_hunk(1, 1)]);
    let mut app = build_app_with_files(vec![file], 10);
    app.sync_viewport_width(80);

    app.forge_review_summaries = vec![RemoteReviewSummary {
        id: "PRR_1".into(),
        author: Some("alice".into()),
        body: "Overall LGTM".into(),
        state: RemoteReviewState::Commented,
        created_at: None,
        url: "https://example.com/r/1".into(),
    }];
    app.rebuild_annotations();

    let items = app.build_comment_navigator_items();

    // Summary appears as the first navigator item — review-scope, before files.
    assert!(matches!(
        items[0].key,
        CommentNavigatorKey::RemoteReview { summary_idx: 0 }
    ));
    assert_eq!(items[0].author.as_deref(), Some("alice"));
    assert!(items[0].path.is_none());
    assert!(items[0].line.is_none());

    // Scroll math stays consistent: annotation count matches total_lines.
    assert_eq!(app.total_lines(), app.line_annotations.len());
}

#[test]
fn jump_to_selected_comment_uses_comment_annotation_target() {
    let file = make_file_with_hunks("a.rs", vec![make_hunk(1, 2)]);
    let mut app = build_app_with_files(vec![file], 10);
    app.sync_viewport_width(80);
    app.diff_state.viewport_height = 5;
    app.session
        .files
        .get_mut(&PathBuf::from("a.rs"))
        .unwrap()
        .line_comments
        .entry(2)
        .or_default()
        .push(Comment::new(
            "line-level".to_string(),
            CommentType::Issue,
            Some(LineSide::New),
        ));
    app.rebuild_annotations();
    let items = app.build_comment_navigator_items();
    let target = items[0].target_annotation;
    let expected_scroll = target
        .saturating_sub(app.diff_state.viewport_height / 2)
        .min(app.max_scroll_offset());

    app.focused_panel = FocusedPanel::Comments;
    app.comment_navigator_state.select(0);

    assert!(app.jump_to_selected_comment());
    assert_eq!(app.diff_state.cursor_line, target);
    assert_eq!(app.diff_state.scroll_offset, expected_scroll);
    assert_eq!(app.focused_panel, FocusedPanel::Diff);
}

#[test]
fn total_lines_must_match_annotations_with_eof_gaps() {
    // Multiple files, each with EOF gaps of different sizes
    let files = vec![
        make_file_with_hunks("a.rs", vec![make_hunk(10, 5)]), // gap before (9) + gap after
        make_file_with_hunks("b.rs", vec![make_hunk(1, 5), make_hunk(30, 5)]), // between gap + EOF gap
        make_file_with_hunks("c.rs", vec![make_hunk(1, 100)]), // no EOF gap (hunk covers all)
    ];
    let app = build_app_with_files(files, 100);

    assert_eq!(
        app.total_lines(),
        app.line_annotations.len(),
        "total_lines() must equal line_annotations.len()\n\
             total_lines={}, annotations={}",
        app.total_lines(),
        app.line_annotations.len()
    );
}

#[test]
fn should_not_show_eof_gap_for_deleted_files() {
    // given: a deleted file with hunks (old-side content)
    let hunks = vec![make_hunk(1, 5)];
    let content_hash = DiffFile::compute_content_hash(&hunks);
    let file = DiffFile {
        old_path: Some(PathBuf::from("deleted.rs")),
        new_path: None,
        status: FileStatus::Deleted,
        hunks,
        is_binary: false,
        is_too_large: false,
        is_commit_message: false,
        content_hash,
    };
    let app = build_app_with_files(vec![file], 100);
    let eof_gap_id = GapId {
        file_idx: 0,
        hunk_idx: 1,
    };

    // then: no EOF expander for deleted files
    let expander_count = app
        .line_annotations
        .iter()
        .filter(|a| matches!(a, AnnotatedLine::Expander { gap_id: g, .. } if *g == eof_gap_id))
        .count();
    assert_eq!(
        expander_count, 0,
        "deleted files should not have EOF expander"
    );

    // and: total_lines must match annotations
    assert_eq!(app.total_lines(), app.line_annotations.len());
}
