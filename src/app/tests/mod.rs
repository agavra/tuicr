mod change_status_tests;
mod commit_scoped_comment_tests;
mod commit_selection_tests;
mod decoration_skip_tests;
mod diff_reload_tests;
mod diff_search_tests;
mod diff_source_tests;
mod diff_watch_tests;
mod expand_gap_tests;
mod file_filter_tests;
mod find_source_line_tests;
mod persistence_merge_tests;
mod pr_highlight_tests;
mod pr_info_tests;
mod render_perf_tests;
mod scroll_behavior_tests;
mod scroll_tests;
mod single_file_view_tests;
mod submit_flow_tests;
mod target_selector_tests;
mod tree_tests;
mod visual_selection_tests;

use std::path::PathBuf;

use crate::model::DiffLine;

/// Minimal `ForgeBackend` for tests that only need `App::forge_backend` to be
/// populated (PR-mode gating, editor target resolution). Every RPC panics —
/// reaching one means the code under test made a call it shouldn't have.
pub(super) struct FakeForgeBackend {
    pub(super) local_checkout: Option<PathBuf>,
}

impl crate::forge::traits::ForgeBackend for FakeForgeBackend {
    fn list_pull_requests(
        &self,
        _query: crate::forge::traits::PullRequestListQuery,
    ) -> crate::error::Result<crate::forge::traits::PagedPullRequests> {
        unimplemented!()
    }
    fn get_pull_request(
        &self,
        _target: crate::forge::traits::PullRequestTarget,
    ) -> crate::error::Result<crate::forge::traits::PullRequestDetails> {
        unimplemented!()
    }
    fn get_pull_request_diff(
        &self,
        _pr: &crate::forge::traits::PullRequestDetails,
    ) -> crate::error::Result<Vec<crate::model::FilePatch>> {
        unimplemented!()
    }
    fn fetch_file_lines(
        &self,
        _request: crate::forge::traits::ForgeFileLinesRequest,
    ) -> crate::error::Result<Vec<DiffLine>> {
        unimplemented!()
    }
    fn list_review_threads(
        &self,
        _pr: &crate::forge::traits::PullRequestDetails,
    ) -> crate::error::Result<Vec<crate::forge::remote_comments::RemoteReviewThread>> {
        unimplemented!()
    }
    fn list_pull_request_commits(
        &self,
        _pr: &crate::forge::traits::PullRequestDetails,
    ) -> crate::error::Result<Vec<crate::forge::traits::PullRequestCommit>> {
        unimplemented!()
    }
    fn get_pull_request_commit_range_diff(
        &self,
        _pr: &crate::forge::traits::PullRequestDetails,
        _start_sha: &str,
        _end_sha: &str,
    ) -> crate::error::Result<Vec<crate::model::FilePatch>> {
        unimplemented!()
    }
    fn create_review(
        &self,
        _pr: &crate::forge::traits::PullRequestDetails,
        _request: crate::forge::traits::CreateReviewRequest<'_>,
    ) -> crate::error::Result<crate::forge::traits::GhCreateReviewResponse> {
        unimplemented!()
    }
    fn local_checkout_path(&self) -> Option<PathBuf> {
        self.local_checkout.clone()
    }
}
