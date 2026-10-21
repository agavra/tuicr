//! Heptapod backend: delegates every API call to [`GitLabGlabBackend`] and
//! only overrides the local-checkout optimizations that assume Git.
//!
//! [`GitLabGlabBackend`] routes its local reads through `git show` and
//! `git diff`. On an Heptapod checkout the underlying VCS is Mercurial, so
//! those primitives fail and the backend has to fall back to the REST API —
//! for file content that's a graceful degradation, but for the commit-range
//! diff there is no REST fallback and the selector breaks.
//!
//! [`HeptapodBackend`] detects the local VCS once at construction time and
//! routes:
//!
//! - [`ForgeBackend::fetch_file_content`] → `hg cat` when the checkout is hg,
//!   otherwise the GitLab backend's `git show`/REST path.
//! - [`ForgeBackend::get_pull_request_commit_range_diff`] → `hg diff -r a:b`
//!   when the checkout is hg, otherwise the GitLab backend's `git diff` (which
//!   returns `UnsupportedOperation` without a checkout).
//!
//! Everything else — list/view/diff/discussions/submit — is identical to
//! GitLab and goes straight through.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::Result;
use crate::forge::gitlab::glab::{GitLabGlabBackend, GlabCommandRunner, SystemGlabRunner};
use crate::forge::local_hg::{read_blob_hg, run_hg_diff_range};
use crate::forge::remote_comments::RemoteReviewThread;
use crate::forge::traits::{
    CreateReviewRequest, ForgeBackend, ForgeFileLinesRequest, ForgeRepository,
    GhCreateReviewResponse, PagedPullRequests, PullRequestCommit, PullRequestDetails,
    PullRequestListQuery, PullRequestReviewMetadata, PullRequestTarget,
};
use crate::model::{DiffLine, FilePatch};
use crate::vcs::slice_context_lines;

/// Heptapod backend over `glab`.
///
/// Generic over the `glab` runner so tests can inject a recording runner the
/// same way `GitLabGlabBackend` does.
#[derive(Debug, Clone)]
pub struct HeptapodBackend<R = SystemGlabRunner> {
    gitlab: GitLabGlabBackend<R>,
    local_is_hg: bool,
}

impl HeptapodBackend<SystemGlabRunner> {
    pub fn new(default_repository: Option<ForgeRepository>) -> Self {
        Self {
            gitlab: GitLabGlabBackend::new(default_repository),
            local_is_hg: false,
        }
    }

    pub fn with_local_checkout(mut self, checkout: Option<PathBuf>) -> Self {
        // Sniff the checkout's VCS once, here, so per-call paths don't each
        // spawn `hg root`. `None` keeps the default (non-hg) routing.
        self.local_is_hg = checkout.as_deref().is_some_and(is_hg_checkout);
        self.gitlab = self.gitlab.with_local_checkout(checkout);
        self
    }
}

impl<R> HeptapodBackend<R>
where
    R: GlabCommandRunner,
{
    /// Build a backend with an explicit `glab` runner (used in tests).
    pub fn with_runner(default_repository: Option<ForgeRepository>, runner: R) -> Self {
        Self {
            gitlab: GitLabGlabBackend::with_runner(default_repository, runner),
            local_is_hg: false,
        }
    }

    /// Test-only constructor that lets a test force the hg routing without
    /// spawning `hg root`. Production code should go through
    /// [`Self::with_local_checkout`].
    #[cfg(test)]
    pub(crate) fn with_local_is_hg(mut self, local_is_hg: bool) -> Self {
        self.local_is_hg = local_is_hg;
        self
    }
}

/// Return true when `path` is the root of an `hg` checkout.
///
/// Uses `hg root` (run from `path`) so it works from subdirectories too.
/// Returns `false` on any failure — missing `hg` binary, not a checkout —
/// so a non-hg checkout silently falls back to the GitLab backend's Git path.
fn is_hg_checkout(path: &Path) -> bool {
    Command::new("hg")
        .arg("root")
        .current_dir(path)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

impl<R> ForgeBackend for HeptapodBackend<R>
where
    R: GlabCommandRunner,
{
    fn list_pull_requests(&self, query: PullRequestListQuery) -> Result<PagedPullRequests> {
        self.gitlab.list_pull_requests(query)
    }

    fn get_pull_request(&self, target: PullRequestTarget) -> Result<PullRequestDetails> {
        self.gitlab.get_pull_request(target)
    }

    fn get_pull_request_diff(&self, pr: &PullRequestDetails) -> Result<Vec<FilePatch>> {
        self.gitlab.get_pull_request_diff(pr)
    }

    fn fetch_file_lines(&self, request: ForgeFileLinesRequest) -> Result<Vec<DiffLine>> {
        if request.start_line == 0 || request.start_line > request.end_line {
            return Ok(Vec::new());
        }
        let (start_line, end_line) = (request.start_line, request.end_line);
        let content = self.fetch_file_content(request)?;
        Ok(slice_context_lines(&content, start_line, end_line))
    }

    fn fetch_file_content(&self, request: ForgeFileLinesRequest) -> Result<String> {
        // When the local checkout is hg, read the blob via `hg cat` first;
        // fall back to the GitLab backend (which tries `git show` then the
        // REST API) when hg can't resolve the revision.
        if self.local_is_hg
            && let Some(checkout) = self.gitlab.local_checkout_path()
            && let Some(content) = read_blob_hg(&checkout, request.sha(), request.path.as_path())
        {
            return Ok(content);
        }
        self.gitlab.fetch_file_content(request)
    }

    fn file_line_count(&self, request: ForgeFileLinesRequest) -> Result<u32> {
        let content = self.fetch_file_content(request)?;
        Ok(content.lines().count() as u32)
    }

    fn list_review_threads(&self, pr: &PullRequestDetails) -> Result<Vec<RemoteReviewThread>> {
        self.gitlab.list_review_threads(pr)
    }

    fn list_pull_request_commits(&self, pr: &PullRequestDetails) -> Result<Vec<PullRequestCommit>> {
        self.gitlab.list_pull_request_commits(pr)
    }

    fn list_pull_request_review_metadata(
        &self,
        pr: &PullRequestDetails,
    ) -> Result<PullRequestReviewMetadata> {
        self.gitlab.list_pull_request_review_metadata(pr)
    }

    fn get_pull_request_commit_range_diff(
        &self,
        pr: &PullRequestDetails,
        start_sha: &str,
        end_sha: &str,
    ) -> Result<Vec<FilePatch>> {
        // The GitLab backend only supports this when a local checkout has
        // both SHAs as Git objects. On an hg checkout that path fails, so
        // route through `hg diff -r a:b` instead. Without a local checkout
        // there is no local fallback (mirrors GitLab's error).
        if self.local_is_hg
            && let Some(checkout) = self.gitlab.local_checkout_path()
        {
            return run_hg_diff_range(&checkout, start_sha, end_sha);
        }
        self.gitlab
            .get_pull_request_commit_range_diff(pr, start_sha, end_sha)
    }

    // `resolve_diff_base_sha`: GitLab's `diff_refs.base_sha` already reports
    // the merge base, and Heptapod inherits that behavior. Keep the default
    // (None = no refinement needed).

    fn local_checkout_path(&self) -> Option<PathBuf> {
        self.gitlab.local_checkout_path()
    }

    fn create_review(
        &self,
        pr: &PullRequestDetails,
        request: CreateReviewRequest<'_>,
    ) -> Result<GhCreateReviewResponse> {
        self.gitlab.create_review(pr, request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::gitlab::glab::{GlabCommandError, GlabCommandRunner};
    use crate::forge::traits::PullRequestListQuery;
    use std::cell::RefCell;

    /// Mock runner that records calls and returns canned responses, mirroring
    /// `RecordingRunner` from the GitLab tests.
    struct RecordingRunner {
        calls: RefCell<Vec<Vec<String>>>,
        responses: RefCell<Vec<String>>,
    }

    impl RecordingRunner {
        fn new_with_responses(responses: Vec<String>) -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                responses: RefCell::new(responses),
            }
        }

        fn call_count(&self) -> usize {
            self.calls.borrow().len()
        }
    }

    impl GlabCommandRunner for RecordingRunner {
        fn run(&self, args: &[String]) -> std::result::Result<String, GlabCommandError> {
            self.calls.borrow_mut().push(args.to_vec());
            let resp = self
                .responses
                .borrow_mut()
                .drain(..1)
                .next()
                .unwrap_or_default();
            Ok(resp)
        }
    }

    #[test]
    fn delegates_list_pull_requests_to_gitlab_backend() {
        // A canned GitLab `mr list --output json` response with one MR.
        let canned = r#"[{
            "iid": 42,
            "title": "Test MR",
            "author": {"username": "alice"},
            "source_branch": "feature",
            "target_branch": "main",
            "web_url": "https://foss.heptapod.net/owner/repo/-/merge_requests/42",
            "state": "opened",
            "updated_at": "2026-01-01T00:00:00Z",
            "work_in_progress": false
        }]"#;
        let repo = ForgeRepository::heptapod("foss.heptapod.net", "owner", "repo");
        let runner = RecordingRunner::new_with_responses(vec![canned.to_string()]);
        let backend = HeptapodBackend::with_runner(Some(repo), runner);

        let result = backend
            .list_pull_requests(PullRequestListQuery::first_page(
                ForgeRepository::heptapod("foss.heptapod.net", "owner", "repo"),
                10,
            ))
            .unwrap();

        // Delegation: the glab response was parsed and returned unchanged.
        assert_eq!(result.pull_requests.len(), 1);
        assert_eq!(result.pull_requests[0].number, 42);
        assert_eq!(result.pull_requests[0].title, "Test MR");
    }

    #[test]
    fn local_is_hg_defaults_to_false() {
        let backend = HeptapodBackend::new(None);
        assert!(!backend.local_is_hg);
    }

    #[test]
    fn with_local_is_hg_forces_hg_routing_in_tests() {
        let backend = HeptapodBackend::new(None).with_local_is_hg(true);
        assert!(backend.local_is_hg);
    }
}
