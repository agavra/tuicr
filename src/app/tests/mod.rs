mod change_status_tests;
mod comment_navigation_tests;
mod commit_scoped_comment_tests;
mod commit_selection_tests;
mod decoration_skip_tests;
mod diff_reload_tests;
mod diff_search_tests;
mod diff_source_tests;
mod diff_watch_tests;
mod expand_gap_tests;
mod file_filter_tests;
mod file_session_identity_tests;
mod find_source_line_tests;
mod locked_comment_tests;
mod persistence_merge_tests;
mod pr_info_tests;
mod redraw_tests;
pub(in crate::app) mod render_perf_tests;
mod sbs_comment_side_tests;
mod scroll_behavior_tests;
mod scroll_tests;
mod sessions_resume_tests;
mod single_file_view_tests;
mod submit_flow_tests;
mod target_selector_tests;
mod theme_picker_tests;
mod tree_tests;
mod visual_selection_tests;

/// Holds the commit each `file_line_count` call received.
pub(in crate::app) type LineCountRefs = std::sync::Arc<std::sync::Mutex<Vec<Option<String>>>>;

/// Return a fixed line count and optional fixed file contents, and record calls.
pub(in crate::app) struct StubVcs {
    pub(in crate::app) info: crate::vcs::VcsInfo,
    pub(in crate::app) line_count: u32,
    pub(in crate::app) line_count_refs: LineCountRefs,
    pub(in crate::app) uncolored_contents: Option<crate::vcs::traits::OldAndNewContents>,
    pub(in crate::app) uncolored_calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl crate::vcs::VcsBackend for StubVcs {
    fn info(&self) -> &crate::vcs::VcsInfo {
        &self.info
    }

    fn get_working_tree_diff(
        &self,
        _highlighter: &crate::syntax::SyntaxHighlighter,
    ) -> crate::error::Result<Vec<crate::model::DiffFile>> {
        Err(crate::error::TuicrError::NoChanges)
    }

    fn fetch_context_lines(
        &self,
        _file_path: &std::path::Path,
        _file_status: crate::model::FileStatus,
        _ref_commit: Option<&str>,
        _start_line: u32,
        _end_line: u32,
    ) -> crate::error::Result<Vec<crate::model::DiffLine>> {
        Ok(Vec::new())
    }

    fn file_line_count(
        &self,
        _file_path: &std::path::Path,
        _file_status: crate::model::FileStatus,
        ref_commit: Option<&str>,
    ) -> crate::error::Result<u32> {
        self.line_count_refs
            .lock()
            .unwrap()
            .push(ref_commit.map(str::to_string));
        Ok(self.line_count)
    }

    fn uncolored_file_contents(
        &self,
        _file: &crate::model::DiffFile,
    ) -> Option<crate::vcs::traits::OldAndNewContents> {
        self.uncolored_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.uncolored_contents.clone()
    }
}
