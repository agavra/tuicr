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
