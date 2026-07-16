use crate::app::*;
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

fn build_app(commit_list: Vec<CommitInfo>) -> App {
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
        InputMode::CommitSelect,
        commit_list,
        None,
        None,
    )
    .expect("failed to build test app")
}

fn normal_commit(id: &str) -> CommitInfo {
    CommitInfo {
        id: id.to_string(),
        short_id: id.to_string(),
        branch_name: None,
        summary: "Test commit".to_string(),
        body: None,
        author: "Test".to_string(),
        time: Utc::now(),
    }
}

#[test]
fn special_commit_count_counts_leading_special_entries() {
    let app = build_app(vec![
        App::staged_commit_entry(),
        App::unstaged_commit_entry(),
        normal_commit("abc123"),
    ]);

    assert_eq!(app.special_commit_count(), 2);
}

#[test]
fn special_commit_count_ignores_non_leading_special_entries() {
    let app = build_app(vec![normal_commit("abc123"), App::staged_commit_entry()]);

    assert_eq!(app.special_commit_count(), 0);
}

#[test]
fn toggle_commit_selection_from_all_selected_selects_only_cursor() {
    for cursor in 0..3 {
        let mut app = build_app(vec![
            normal_commit("abc123"),
            normal_commit("def456"),
            normal_commit("789abc"),
        ]);
        app.commit_selection_range = Some((0, 2));
        app.commit_list_cursor = cursor;

        app.toggle_commit_selection();

        assert_eq!(app.commit_selection_range, Some((cursor, cursor)));
    }
}

#[test]
fn toggle_commit_selection_keeps_partial_range_shrink_behavior() {
    let mut app = build_app(vec![
        normal_commit("abc123"),
        normal_commit("def456"),
        normal_commit("789abc"),
    ]);
    app.commit_selection_range = Some((0, 1));
    app.commit_list_cursor = 0;

    app.toggle_commit_selection();

    assert_eq!(app.commit_selection_range, Some((1, 1)));
}

#[test]
fn initial_commit_range_all_selects_full_span() {
    assert_eq!(
        App::initial_commit_range(CommitSelectionStart::All, 4),
        Some((0, 3))
    );
}

#[test]
fn initial_commit_range_oldest_selects_last_index() {
    // review_commits is stored newest-first, so the oldest commit is the last
    // index regardless of the display order.
    assert_eq!(
        App::initial_commit_range(CommitSelectionStart::Oldest, 4),
        Some((3, 3))
    );
}

#[test]
fn initial_commit_range_empty_is_none() {
    assert_eq!(
        App::initial_commit_range(CommitSelectionStart::Oldest, 0),
        None
    );
    assert_eq!(
        App::initial_commit_range(CommitSelectionStart::All, 0),
        None
    );
}

#[test]
fn commit_data_index_is_identity_when_descending() {
    let mut app = build_app(vec![
        normal_commit("a"),
        normal_commit("b"),
        normal_commit("c"),
    ]);
    app.review_commits = app.commit_list.clone();
    app.commit_order = CommitOrder::Descending;
    for i in 0..3 {
        assert_eq!(app.commit_data_index(i), i);
    }
}

#[test]
fn commit_data_index_mirrors_and_round_trips_when_ascending() {
    let mut app = build_app(vec![
        normal_commit("a"),
        normal_commit("b"),
        normal_commit("c"),
    ]);
    app.review_commits = app.commit_list.clone();
    app.commit_order = CommitOrder::Ascending;
    assert_eq!(app.commit_data_index(0), 2);
    assert_eq!(app.commit_data_index(2), 0);
    // The mapping is its own inverse (data <-> display row).
    for i in 0..3 {
        assert_eq!(app.commit_data_index(app.commit_data_index(i)), i);
    }
}

#[test]
fn toggle_commit_selector_flips_visibility_and_drops_focus() {
    let mut app = build_app(vec![normal_commit("a"), normal_commit("b")]);
    app.show_commit_selector = true;
    app.focused_panel = FocusedPanel::CommitSelector;

    app.toggle_commit_selector();
    assert!(!app.show_commit_selector);
    // Hiding the focused pane returns focus to the diff.
    assert_eq!(app.focused_panel, FocusedPanel::Diff);

    app.toggle_commit_selector();
    assert!(app.show_commit_selector);
}

#[test]
fn has_review_commits_ignores_visibility_but_requires_multiple_non_worktree() {
    let mut app = build_app(vec![normal_commit("a"), normal_commit("b")]);
    app.review_commits = app.commit_list.clone();
    app.diff_source = DiffSource::CommitRange(vec!["a".to_string(), "b".to_string()]);

    app.show_commit_selector = false;
    assert!(app.has_review_commits());
    assert!(!app.has_inline_commit_selector());

    // Working-tree reviews never cycle commits.
    app.diff_source = DiffSource::WorkingTree;
    assert!(!app.has_review_commits());
}
