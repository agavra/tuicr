//! Tests for choosing the comment side in side-by-side view: `<leader>h` /
//! `<leader>l` walk the panes and set `cursor_side`, and `get_line_at_cursor`
//! resolves that preference (clamped to the sides a line actually has). The
//! plain horizontal keys keep scrolling in every view.

use crate::app::*;
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
    let vcs_info = VcsInfo {
        root_path: PathBuf::from("/tmp"),
        head_commit: "head".to_string(),
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
        Vec::new(),
        session,
        DiffSource::WorkingTree,
        InputMode::Normal,
        Vec::new(),
        None,
        None,
    )
    .expect("failed to build test app")
}

fn sbs_line(old_lineno: Option<u32>, new_lineno: Option<u32>) -> AnnotatedLine {
    AnnotatedLine::SideBySideLine {
        file_idx: 0,
        hunk_idx: 0,
        del_line_idx: old_lineno.map(|_| 0),
        add_line_idx: new_lineno.map(|_| 0),
        old_lineno,
        new_lineno,
    }
}

#[test]
fn context_line_honors_cursor_side_in_side_by_side() {
    let mut app = build_app();
    app.diff_view_mode = DiffViewMode::SideBySide;
    app.line_annotations = vec![sbs_line(Some(10), Some(20))];
    app.diff_state.cursor_line = 0;

    app.cursor_side = LineSide::New;
    assert_eq!(app.get_line_at_cursor(), Some((20, LineSide::New)));

    app.cursor_side = LineSide::Old;
    assert_eq!(app.get_line_at_cursor(), Some((10, LineSide::Old)));
}

#[test]
fn addition_line_clamps_to_new_even_when_old_preferred() {
    let mut app = build_app();
    app.diff_view_mode = DiffViewMode::SideBySide;
    app.line_annotations = vec![sbs_line(None, Some(20))];
    app.diff_state.cursor_line = 0;

    app.cursor_side = LineSide::Old; // no old side on this line
    assert_eq!(app.get_line_at_cursor(), Some((20, LineSide::New)));
}

#[test]
fn deletion_line_clamps_to_old_even_when_new_preferred() {
    let mut app = build_app();
    app.diff_view_mode = DiffViewMode::SideBySide;
    app.line_annotations = vec![sbs_line(Some(10), None)];
    app.diff_state.cursor_line = 0;

    app.cursor_side = LineSide::New; // no new side on this line
    assert_eq!(app.get_line_at_cursor(), Some((10, LineSide::Old)));
}

#[test]
fn unified_view_ignores_cursor_side_and_prefers_new() {
    let mut app = build_app();
    app.diff_view_mode = DiffViewMode::Unified;
    app.line_annotations = vec![sbs_line(Some(10), Some(20))];
    app.diff_state.cursor_line = 0;

    app.cursor_side = LineSide::Old; // ignored in unified
    assert_eq!(app.effective_cursor_side(), LineSide::New);
    assert_eq!(app.get_line_at_cursor(), Some((20, LineSide::New)));
}

#[test]
fn horizontal_keys_scroll_in_side_by_side_and_leave_side_unchanged() {
    let mut app = build_app();
    app.diff_view_mode = DiffViewMode::SideBySide;
    app.cursor_side = LineSide::New;
    app.diff_state.wrap_lines = false;
    app.diff_state.max_content_width = 200;
    app.diff_state.viewport_width = 80;

    // Reviewers expect the horizontal keys to scroll in every view; the side is
    // switched with the `<leader>` panel walk instead.
    crate::handler::handle_diff_action(&mut app, Action::ScrollRight(4));
    assert_eq!(app.diff_state.scroll_x, 4);
    assert_eq!(app.cursor_side, LineSide::New);

    crate::handler::handle_diff_action(&mut app, Action::ScrollLeft(4));
    assert_eq!(app.diff_state.scroll_x, 0);
    assert_eq!(app.cursor_side, LineSide::New);
}

#[test]
fn line_comment_preserves_horizontal_scroll() {
    let mut app = build_app();
    app.diff_view_mode = DiffViewMode::SideBySide;
    app.diff_state.scroll_x = 12;

    app.enter_comment_mode(false, Some((20, LineSide::New)));

    assert_eq!(app.diff_state.scroll_x, 12);
}

#[test]
fn range_comment_preserves_horizontal_scroll() {
    let mut app = build_app();
    app.diff_view_mode = DiffViewMode::SideBySide;
    app.diff_state.scroll_x = 12;
    app.input_mode = InputMode::VisualSelect;
    app.line_annotations = vec![sbs_line(Some(10), Some(20))];
    app.visual_selection = Some(VisualSelection::collapsed(SelPoint {
        annotation_idx: 0,
        char_offset: 0,
        side: LineSide::New,
    }));

    app.enter_comment_from_visual();

    assert_eq!(app.diff_state.scroll_x, 12);
    assert_eq!(app.input_mode, InputMode::Comment);
}

#[test]
fn leader_walk_steps_through_the_side_by_side_panes() {
    let mut app = build_app();
    app.diff_view_mode = DiffViewMode::SideBySide;
    app.show_file_list = true;
    app.focused_panel = FocusedPanel::Diff;
    app.cursor_side = LineSide::New;

    // new -> old inside the diff, then out to the file list.
    app.focus_pane_left();
    assert_eq!(app.focused_panel, FocusedPanel::Diff);
    assert_eq!(app.cursor_side, LineSide::Old);

    app.focus_pane_left();
    assert_eq!(app.focused_panel, FocusedPanel::FileList);
    assert_eq!(app.cursor_side, LineSide::Old);

    // ... and back: file list -> diff (side kept) -> new.
    app.focus_pane_right();
    assert_eq!(app.focused_panel, FocusedPanel::Diff);
    assert_eq!(app.cursor_side, LineSide::Old);

    app.focus_pane_right();
    assert_eq!(app.cursor_side, LineSide::New);

    // Already rightmost: stays put.
    app.focus_pane_right();
    assert_eq!(app.focused_panel, FocusedPanel::Diff);
    assert_eq!(app.cursor_side, LineSide::New);
}

#[test]
fn leader_walk_ignores_the_sides_in_unified_view() {
    let mut app = build_app();
    app.diff_view_mode = DiffViewMode::Unified;
    app.show_file_list = true;
    app.focused_panel = FocusedPanel::Diff;
    app.cursor_side = LineSide::New;

    // Unified has a single pane, so the walk leaves the diff immediately.
    app.focus_pane_left();
    assert_eq!(app.focused_panel, FocusedPanel::FileList);
    assert_eq!(app.cursor_side, LineSide::New);

    app.focus_pane_right();
    assert_eq!(app.focused_panel, FocusedPanel::Diff);
    assert_eq!(app.cursor_side, LineSide::New);
}

#[test]
fn horizontal_keys_scroll_in_unified_and_leave_side_unchanged() {
    let mut app = build_app();
    app.diff_view_mode = DiffViewMode::Unified;
    app.cursor_side = LineSide::New;
    app.diff_state.wrap_lines = false;
    app.diff_state.max_content_width = 100;
    app.diff_state.viewport_width = 10;

    crate::handler::handle_diff_action(&mut app, Action::ScrollRight(4));
    assert_eq!(app.diff_state.scroll_x, 4);
    assert_eq!(app.cursor_side, LineSide::New);
}

#[test]
fn visual_suggestion_prefills_selected_side_in_both_views() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    for view in [DiffViewMode::Unified, DiffViewMode::SideBySide] {
        for side in [LineSide::Old, LineSide::New] {
            for reverse in [false, true] {
                let mut app = build_app();
                app.diff_view_mode = view;
                let lines = [
                    (LineOrigin::Deletion, Some(10), None, "    old α"),
                    (LineOrigin::Deletion, Some(11), None, ""),
                    (LineOrigin::Addition, None, Some(20), "    new β"),
                    (LineOrigin::Addition, None, Some(21), ""),
                ]
                .into_iter()
                .map(|(origin, old_lineno, new_lineno, content)| DiffLine {
                    origin,
                    old_lineno,
                    new_lineno,
                    content: content.into(),
                    raw_content: content
                        .starts_with("    ")
                        .then(|| content.replacen("    ", "\t", 1)),
                    highlighted_spans: None,
                })
                .collect();
                app.diff_files = vec![DiffFile {
                    old_path: Some("test.rs".into()),
                    new_path: Some("test.rs".into()),
                    status: FileStatus::Modified,
                    hunks: vec![DiffHunk {
                        header: "@@ -10,2 +20,2 @@".into(),
                        old_start: 10,
                        old_count: 2,
                        new_start: 20,
                        new_count: 2,
                        lines,
                    }],
                    is_binary: false,
                    is_too_large: false,
                    is_commit_message: false,
                    content_hash: 0,
                }];
                app.line_annotations = if view == DiffViewMode::SideBySide {
                    (0..2)
                        .map(|i| AnnotatedLine::SideBySideLine {
                            file_idx: 0,
                            hunk_idx: 0,
                            del_line_idx: Some(i),
                            add_line_idx: Some(i + 2),
                            old_lineno: Some(10 + i as u32),
                            new_lineno: Some(20 + i as u32),
                        })
                        .collect()
                } else {
                    app.diff_files[0].hunks[0]
                        .lines
                        .iter()
                        .enumerate()
                        .map(|(i, line)| AnnotatedLine::DiffLine {
                            file_idx: 0,
                            hunk_idx: 0,
                            line_idx: i,
                            old_lineno: line.old_lineno,
                            new_lineno: line.new_lineno,
                        })
                        .collect()
                };
                let first = if view == DiffViewMode::Unified && side == LineSide::New {
                    2
                } else {
                    0
                };
                // A partial mouse selection still needs the full replacement lines.
                let start = SelPoint {
                    annotation_idx: first,
                    char_offset: 4,
                    side,
                };
                let end = SelPoint {
                    annotation_idx: first + 1,
                    char_offset: 0,
                    side,
                };
                app.visual_selection = Some(if reverse {
                    VisualSelection {
                        anchor: end,
                        head: start,
                    }
                } else {
                    VisualSelection {
                        anchor: start,
                        head: end,
                    }
                });
                app.input_mode = InputMode::VisualSelect;
                let action = crate::input::map_key_to_action(
                    KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE),
                    InputMode::VisualSelect,
                    crate::config::DEFAULT_LEADER_KEY,
                );
                crate::handler::handle_visual_action(&mut app, action);

                if side == LineSide::Old {
                    assert_eq!(app.input_mode, InputMode::VisualSelect);
                    assert!(app.comment_buffer.is_empty());
                    assert!(app.comment_line_range.is_none());
                    continue;
                }

                let (code, line) = if side == LineSide::Old {
                    ("\told α", 10)
                } else {
                    ("\tnew β", 20)
                };
                assert_eq!(
                    app.comment_buffer,
                    format!("\n```suggestion\n{code}\n\n```")
                );
                assert_eq!(app.comment_cursor, 0);
                assert_eq!(
                    app.comment_line_range,
                    Some((LineRange::new(line, line + 1), side))
                );
                assert_eq!(app.comment_line, Some((line + 1, side)));
                assert_eq!(app.input_mode, InputMode::Comment);
                assert!(app.visual_selection.is_none());
            }
        }
    }
}

#[test]
fn visual_suggestion_rejects_gaps_expanded_context_and_other_hunks() {
    let gap_id = GapId {
        file_idx: 0,
        hunk_idx: 1,
    };
    for middle in [
        AnnotatedLine::HiddenLines {
            gap_id: gap_id.clone(),
            count: 3,
        },
        AnnotatedLine::ExpandedContext {
            gap_id,
            line_idx: 0,
        },
        AnnotatedLine::HunkHeader {
            file_idx: 0,
            hunk_idx: 1,
        },
        AnnotatedLine::SideBySideLine {
            file_idx: 0,
            hunk_idx: 1,
            del_line_idx: None,
            add_line_idx: Some(0),
            old_lineno: None,
            new_lineno: Some(21),
        },
        AnnotatedLine::SideBySideLine {
            file_idx: 1,
            hunk_idx: 0,
            del_line_idx: None,
            add_line_idx: Some(0),
            old_lineno: None,
            new_lineno: Some(21),
        },
    ] {
        let mut app = build_app();
        app.line_annotations = vec![sbs_line(None, Some(20)), middle, sbs_line(None, Some(24))];
        app.input_mode = InputMode::VisualSelect;
        app.visual_selection = Some(VisualSelection {
            anchor: SelPoint {
                annotation_idx: 0,
                char_offset: 0,
                side: LineSide::New,
            },
            head: SelPoint {
                annotation_idx: 2,
                char_offset: 1,
                side: LineSide::New,
            },
        });
        crate::handler::handle_visual_action(&mut app, Action::AddRangeSuggestion);
        assert_eq!(app.input_mode, InputMode::VisualSelect);
        assert!(app.comment_buffer.is_empty());
        assert!(app.comment_line_range.is_none());
    }
}

fn suggestion_app_from_patch(patch: &str) -> App {
    suggestion_app_from_patch_in_view(patch, DiffViewMode::Unified)
}

fn suggestion_app_from_patch_in_view(patch: &str, view: DiffViewMode) -> App {
    let mut app = build_app();
    app.diff_view_mode = view;
    app.diff_files = crate::vcs::diff_parser::parse_file_patches(
        vec![crate::model::FilePatch::new(
            Some("test.rs".into()),
            Some("test.rs".into()),
            FileStatus::Modified,
            patch,
        )],
        &SyntaxHighlighter::default(),
    )
    .unwrap();
    app.rebuild_annotations();
    let indices: Vec<_> = app
        .line_annotations
        .iter()
        .enumerate()
        .filter_map(|(i, ann)| {
            matches!(
                ann,
                AnnotatedLine::DiffLine {
                    new_lineno: Some(_),
                    ..
                } | AnnotatedLine::SideBySideLine {
                    new_lineno: Some(_),
                    ..
                }
            )
            .then_some(i)
        })
        .collect();
    app.input_mode = InputMode::VisualSelect;
    app.visual_selection = Some(VisualSelection {
        anchor: SelPoint {
            annotation_idx: indices[0],
            char_offset: 0,
            side: LineSide::New,
        },
        head: SelPoint {
            annotation_idx: *indices.last().unwrap(),
            char_offset: 0,
            side: LineSide::New,
        },
    });
    app
}

#[test]
fn visual_suggestion_rejects_context_between_added_blocks() {
    for view in [DiffViewMode::Unified, DiffViewMode::SideBySide] {
        for context in ["", "unchanged"] {
            for reverse in [false, true] {
                let patch = format!("@@ -1 +1,3 @@\n+first\n {context}\n+second\n");
                let mut app = suggestion_app_from_patch_in_view(&patch, view);
                if reverse {
                    let sel = app.visual_selection.as_mut().unwrap();
                    std::mem::swap(&mut sel.anchor, &mut sel.head);
                }
                crate::handler::handle_visual_action(&mut app, Action::AddRangeSuggestion);
                assert_eq!(app.input_mode, InputMode::VisualSelect);
                assert!(app.visual_selection.is_some());
                assert!(app.comment_buffer.is_empty());
                assert!(app.comment_line_range.is_none());
            }
        }
    }
}

#[test]
fn visual_suggestion_allows_context_around_one_added_block() {
    for view in [DiffViewMode::Unified, DiffViewMode::SideBySide] {
        let mut app = suggestion_app_from_patch_in_view(
            "@@ -1,2 +1,5 @@\n before\n+first\n+\n+second\n after\n",
            view,
        );
        app.enter_suggestion_from_visual();
        assert_eq!(app.input_mode, InputMode::Comment);
        assert_eq!(
            app.comment_buffer,
            "\n```suggestion\nbefore\nfirst\n\nsecond\nafter\n```"
        );
    }
}

#[test]
fn visual_suggestion_rejects_deleted_line_gaps_inside_a_hunk() {
    for view in [DiffViewMode::Unified, DiffViewMode::SideBySide] {
        let mut app = suggestion_app_from_patch("@@ -1,3 +1,2 @@\n before\n-deleted\n after\n");
        app.diff_view_mode = view;
        app.rebuild_annotations();
        let indices: Vec<_> = app
            .line_annotations
            .iter()
            .enumerate()
            .filter_map(|(idx, ann)| {
                matches!(
                    ann,
                    AnnotatedLine::DiffLine {
                        new_lineno: Some(_),
                        ..
                    } | AnnotatedLine::SideBySideLine {
                        new_lineno: Some(_),
                        ..
                    }
                )
                .then_some(idx)
            })
            .collect();
        let selection = app.visual_selection.as_mut().unwrap();
        selection.anchor.annotation_idx = indices[0];
        selection.head.annotation_idx = *indices.last().unwrap();

        crate::handler::handle_visual_action(&mut app, Action::AddRangeSuggestion);

        assert_eq!(app.input_mode, InputMode::VisualSelect);
        assert!(app.visual_selection.is_some());
        assert!(app.comment_buffer.is_empty());
        assert!(app.comment_line_range.is_none());
    }
}
