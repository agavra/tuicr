//! PR open path.
//!
//! Given a `ForgeBackend` and a `PullRequestTarget`, produce the materials
//! the App needs to enter PR review mode: parsed diff files, a session, and
//! a `PrSessionKey` that scopes persistence and remote context fetches.
//!
//! Key invariants enforced here:
//! - The current local checkout is never treated as the source of truth.
//!   File identity comes from forge metadata; SHAs come from PR metadata.
//! - `.tuicrignore` is applied only when the caller supplies a local
//!   checkout path. Outside a checkout, the unfiltered diff is shown.
//! - No checkout mutation. We never spawn `git checkout/fetch/reset/stash`
//!   or branch-creation commands here.

use std::path::{Path, PathBuf};

use crate::error::{Result, TuicrError};
use crate::forge::join;
use crate::forge::traits::{
    ForgeBackend, PrSessionKey, PullRequestCommit, PullRequestDetails, PullRequestInfo,
    PullRequestReviewMetadata, PullRequestTarget,
};
use crate::model::{DiffFile, FilePatch, ReviewSession, SessionDiffSource};
use crate::syntax::SyntaxHighlighter;
use crate::tuicrignore;
use crate::vcs::diff_parser::parse_file_patches;

/// Everything the App needs to enter PR review mode.
#[derive(Debug)]
pub struct OpenedPullRequest {
    pub details: PullRequestDetails,
    pub diff_files: Vec<DiffFile>,
    pub session: ReviewSession,
    pub key: PrSessionKey,
    /// PR commits in newest-first display order. Empty when the forge
    /// returned no commits (or the backend failed and we degraded
    /// gracefully — the cumulative diff stays usable).
    pub commits: Vec<PullRequestCommit>,
    /// Best-effort metadata for detecting commits since the viewer's last
    /// submitted review. Empty when unsupported or unavailable.
    pub review_metadata: PullRequestReviewMetadata,
    /// Extended PR metadata for the description panel.
    pub pr_info: PullRequestInfo,
}

/// Send-safe data fetched before the main thread materializes diff hunks.
pub type PrFetchData = (
    PullRequestDetails,
    Vec<FilePatch>,
    Vec<PullRequestCommit>,
    PullRequestReviewMetadata,
    PullRequestInfo,
);

/// Open a PR target through a forge backend and prepare review state.
///
/// `local_checkout` is optional: when provided, `.tuicrignore` rules at the
/// root are applied. When absent (PR opened via URL outside a checkout, or
/// for a different repo), no filtering happens.
pub fn open_pull_request(
    backend: &(dyn ForgeBackend + Sync),
    target: PullRequestTarget,
    local_checkout: Option<&Path>,
    highlighter: &SyntaxHighlighter,
) -> Result<OpenedPullRequest> {
    let (details, patches, commits, review_metadata, pr_info) = fetch_pr_data(backend, target)?;
    prepare_open_pr(
        details,
        patches,
        commits,
        review_metadata,
        pr_info,
        local_checkout,
        highlighter,
    )
}

/// Network-only half of the PR open path: fetch PR metadata, structured file
/// patches, and the commit list. Safe to run on a background thread
/// because it does no syntax parsing and holds nothing that isn't `Send`.
///
/// The commit list is best-effort: if the forge fails on that endpoint
/// only, we still return the diff so PR review proceeds without the
/// inline selector. The first two calls remain required.
///
/// Only the PR metadata and the diff-base refinement have to come first.
/// The diff, the commit list, and the review metadata need nothing but the
/// pinned `details`, so they run concurrently: startup waits for the slowest
/// of them, not their sum. Each one still paginates sequentially.
pub fn fetch_pr_data(
    backend: &(dyn ForgeBackend + Sync),
    target: PullRequestTarget,
) -> Result<PrFetchData> {
    let mut pr_info = backend.get_pull_request_info(target)?;
    // Pin `base_sha` to the commit the displayed patch is actually taken
    // from before anything downstream reads it. Everything that resolves the
    // old side of the diff — editor snapshots, context expansion, the
    // commit-range fallback parent — derives from this field, so refining it
    // once here keeps those consistent by construction.
    if let Some(base_sha) = backend.resolve_diff_base_sha(&pr_info.details) {
        pr_info.details.base_sha = base_sha;
    }
    let details = pr_info.details.clone();
    let (patches, (commits, review_metadata)) = join(
        || backend.get_pull_request_diff(&details),
        || {
            join(
                || backend.list_pull_request_commits(&details),
                || backend.list_pull_request_review_metadata(&details),
            )
        },
    );
    let patches = patches?;
    let commits = commits.unwrap_or_default();
    let review_metadata = review_metadata.unwrap_or_default();
    Ok((details, patches, commits, review_metadata, pr_info))
}

/// CPU-only half of the PR open path: apply `.tuicrignore` to the raw
/// patches, then parse the hunks and build the session. Filtering before the
/// parse keeps an ignored large file from being highlighted at all. Runs on
/// the main thread because `SyntaxHighlighter` is not trivially
/// `Send`-cloneable.
pub fn prepare_open_pr(
    details: PullRequestDetails,
    patches: Vec<FilePatch>,
    commits: Vec<PullRequestCommit>,
    review_metadata: PullRequestReviewMetadata,
    pr_info: PullRequestInfo,
    local_checkout: Option<&Path>,
    highlighter: &SyntaxHighlighter,
) -> Result<OpenedPullRequest> {
    let had_patches = !patches.is_empty();
    let patches = match local_checkout {
        Some(root) => tuicrignore::filter_file_patches(root, patches),
        None => patches,
    };

    let diff_files = if had_patches && patches.is_empty() {
        Vec::new()
    } else {
        match parse_file_patches(patches, highlighter) {
            Ok(files) => files,
            Err(TuicrError::NoChanges) => {
                return Err(TuicrError::Forge(format!(
                    "Pull request #{} has no file changes",
                    details.number
                )));
            }
            Err(e) => return Err(e),
        }
    };

    let key = PrSessionKey::from_details(&details);
    let session = build_session(&details, &key, &diff_files);
    // Forge returns commits oldest-first; the inline selector renders
    // newest-first so reverse here once.
    let mut commits = commits;
    commits.reverse();

    Ok(OpenedPullRequest {
        details,
        diff_files,
        session,
        key,
        commits,
        review_metadata,
        pr_info,
    })
}

fn build_session(
    details: &PullRequestDetails,
    key: &PrSessionKey,
    diff_files: &[DiffFile],
) -> ReviewSession {
    // The session's repo_path is purely a presentation/identity slot for PR
    // sessions. We use a virtual path so PR sessions don't collide with
    // local sessions stored under the same on-disk repo root.
    let repo_path = pr_session_repo_path(key);
    let branch_name = Some(details.head_ref_name.clone());
    let mut session = ReviewSession::new(
        repo_path,
        details.head_sha.clone(),
        branch_name,
        SessionDiffSource::PullRequest,
    );
    session.pr_session_key = Some(key.clone());
    for file in diff_files {
        session.add_diff_file(file);
    }
    session
}

/// Synthetic path used as `ReviewSession::repo_path` for PR sessions.
/// Keeps PR session filenames distinct from local sessions and conveys
/// enough identity (`forge:host/owner/repo`) for humans inspecting the
/// reviews directory.
pub fn pr_session_repo_path(key: &PrSessionKey) -> PathBuf {
    PathBuf::from(format!(
        "forge:{}/{}/{}",
        key.repository.host, key.repository.owner, key.repository.name,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::traits::{
        ForgeFileLinesRequest, ForgeRepository, PagedPullRequests, PullRequestDetails,
        PullRequestListQuery,
    };
    use crate::model::DiffLine;
    use chrono::Utc;
    use std::sync::Mutex;

    fn repo() -> ForgeRepository {
        ForgeRepository::github("github.com", "agavra", "tuicr")
    }

    fn details() -> PullRequestDetails {
        PullRequestDetails {
            repository: repo(),
            number: 125,
            title: "Review workflow".to_string(),
            url: "https://github.com/agavra/tuicr/pull/125".to_string(),
            state: "OPEN".to_string(),
            is_draft: false,
            author: Some("alice".to_string()),
            head_ref_name: "reviews".to_string(),
            base_ref_name: "main".to_string(),
            head_sha: "abcdef0123456789".to_string(),
            base_sha: "1234567890abcdef".to_string(),
            body: "body".to_string(),
            updated_at: Some(Utc::now()),
            closed: false,
            merged_at: None,
            diff_start_sha: None,
        }
    }

    struct StaticBackend {
        details: PullRequestDetails,
        patch: String,
        calls: Mutex<Vec<&'static str>>,
        /// Stands in for a backend whose `base_sha` is the base branch tip
        /// rather than the merge base the patch is taken from. `None` models
        /// a backend that already reports the merge base (GitLab).
        diff_base_sha: Option<String>,
    }

    impl StaticBackend {
        fn new(details: PullRequestDetails, patch: &str) -> Self {
            Self {
                details,
                patch: patch.to_string(),
                calls: Mutex::new(Vec::new()),
                diff_base_sha: None,
            }
        }

        fn with_diff_base_sha(mut self, sha: &str) -> Self {
            self.diff_base_sha = Some(sha.to_string());
            self
        }
    }

    impl ForgeBackend for StaticBackend {
        fn resolve_diff_base_sha(&self, _pr: &PullRequestDetails) -> Option<String> {
            self.calls.lock().unwrap().push("resolve_diff_base_sha");
            self.diff_base_sha.clone()
        }

        fn list_pull_requests(&self, _query: PullRequestListQuery) -> Result<PagedPullRequests> {
            unimplemented!()
        }
        fn get_pull_request(&self, _target: PullRequestTarget) -> Result<PullRequestDetails> {
            self.calls.lock().unwrap().push("get_pull_request");
            Ok(self.details.clone())
        }
        fn get_pull_request_diff(&self, _pr: &PullRequestDetails) -> Result<Vec<FilePatch>> {
            self.calls.lock().unwrap().push("get_pull_request_diff");
            Ok(crate::vcs::diff_parser::git_fixture_file_patches(
                &self.patch,
            ))
        }
        fn fetch_file_lines(&self, _req: ForgeFileLinesRequest) -> Result<Vec<DiffLine>> {
            unimplemented!()
        }
        fn list_review_threads(
            &self,
            _pr: &PullRequestDetails,
        ) -> Result<Vec<crate::forge::remote_comments::RemoteReviewThread>> {
            Ok(Vec::new())
        }
        fn list_pull_request_commits(
            &self,
            _pr: &PullRequestDetails,
        ) -> Result<Vec<crate::forge::traits::PullRequestCommit>> {
            Ok(Vec::new())
        }
        fn get_pull_request_commit_range_diff(
            &self,
            _pr: &PullRequestDetails,
            _start_sha: &str,
            _end_sha: &str,
        ) -> Result<Vec<FilePatch>> {
            Ok(crate::vcs::diff_parser::git_fixture_file_patches(
                &self.patch,
            ))
        }
        fn create_review(
            &self,
            _pr: &PullRequestDetails,
            _request: crate::forge::traits::CreateReviewRequest<'_>,
        ) -> Result<crate::forge::traits::GhCreateReviewResponse> {
            unimplemented!()
        }
    }

    const SIMPLE_PATCH: &str = r##"diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,3 +1,3 @@
 pub fn answer() -> u32 {
-    41
+    42
 }
"##;

    #[test]
    fn should_parse_pr_diff_and_build_session_keyed_by_head_sha() {
        // given
        let backend = StaticBackend::new(details(), SIMPLE_PATCH);
        let target = PullRequestTarget::with_repository(repo(), 125, "125");
        let highlighter = SyntaxHighlighter::default();
        // when
        let opened = open_pull_request(&backend, target, None, &highlighter).unwrap();
        // then
        assert_eq!(opened.diff_files.len(), 1);
        assert_eq!(opened.key.head_sha, "abcdef0123456789");
        assert_eq!(opened.key.number, 125);
        assert_eq!(opened.session.diff_source, SessionDiffSource::PullRequest);
        assert_eq!(
            opened.session.pr_session_key.as_ref().map(|k| k.number),
            Some(125),
        );
        assert_eq!(
            opened.session.repo_path,
            PathBuf::from("forge:github.com/agavra/tuicr"),
        );
        // and — the forge calls were made, in order. The diff base is
        // resolved between metadata and patch so `base_sha` is already
        // pinned to the patch's old side by the time anything reads it.
        assert_eq!(
            backend.calls.lock().unwrap().as_slice(),
            &[
                "get_pull_request",
                "resolve_diff_base_sha",
                "get_pull_request_diff"
            ],
        );
    }

    /// Patch fixture covering add/modify/delete/rename in a single PR diff.
    /// Tests pair its file blocks with explicit metadata before invoking the
    /// shared hunk parser.
    const MULTI_STATUS_PATCH: &str = r##"diff --git a/added.rs b/added.rs
new file mode 100644
index 0000000..abc1234
--- /dev/null
+++ b/added.rs
@@ -0,0 +1,2 @@
+pub fn new_thing() {}
+
diff --git a/modified.rs b/modified.rs
index 1111111..2222222 100644
--- a/modified.rs
+++ b/modified.rs
@@ -1,3 +1,3 @@
 pub fn answer() -> u32 {
-    41
+    42
 }
diff --git a/deleted.rs b/deleted.rs
deleted file mode 100644
index 3333333..0000000
--- a/deleted.rs
+++ /dev/null
@@ -1,2 +0,0 @@
-pub fn gone() {}
-
diff --git a/old_name.rs b/new_name.rs
similarity index 100%
rename from old_name.rs
rename to new_name.rs
"##;

    #[test]
    fn should_parse_multi_status_pr_patch_into_correct_diff_files() {
        // given a backend serving a patch with add/modify/delete/rename
        let backend = StaticBackend::new(details(), MULTI_STATUS_PATCH);
        let target = PullRequestTarget::with_repository(repo(), 125, "125");
        let highlighter = SyntaxHighlighter::default();
        // when
        let opened = open_pull_request(&backend, target, None, &highlighter).unwrap();
        // then — all four files are recognized with correct statuses
        assert_eq!(opened.diff_files.len(), 4);
        let statuses: Vec<(String, crate::model::FileStatus)> = opened
            .diff_files
            .iter()
            .map(|f| (f.display_path().to_string_lossy().into_owned(), f.status))
            .collect();
        // Order is not guaranteed by the parser, so look up by name.
        let by_name: std::collections::HashMap<_, _> = statuses.into_iter().collect();
        assert_eq!(
            by_name.get("added.rs"),
            Some(&crate::model::FileStatus::Added)
        );
        assert_eq!(
            by_name.get("modified.rs"),
            Some(&crate::model::FileStatus::Modified)
        );
        assert_eq!(
            by_name.get("deleted.rs"),
            Some(&crate::model::FileStatus::Deleted)
        );
        assert_eq!(
            by_name.get("new_name.rs"),
            Some(&crate::model::FileStatus::Renamed)
        );
    }

    #[test]
    fn should_surface_empty_pr_as_forge_error() {
        // given a PR with no file changes (empty patch)
        let backend = StaticBackend::new(details(), "");
        let target = PullRequestTarget::with_repository(repo(), 125, "125");
        let highlighter = SyntaxHighlighter::default();
        // when
        let err = open_pull_request(&backend, target, None, &highlighter).unwrap_err();
        // then
        let msg = err.to_string();
        assert!(
            msg.contains("Pull request #125 has no file changes"),
            "unexpected error message: {msg}"
        );
    }

    #[test]
    fn should_drop_ignored_patches_before_parsing_them() {
        // given a checkout whose .tuicrignore excludes dist/, and a PR diff
        // whose ignored patch body is malformed (a hunk header the parser
        // rejects)
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        std::fs::write(dir.path().join(".tuicrignore"), "dist/\n")
            .expect("failed to write .tuicrignore");
        let patches = vec![
            FilePatch::new(
                None,
                Some(std::path::PathBuf::from("src/main.rs")),
                crate::model::FileStatus::Modified,
                "@@ -1,3 +1,3 @@\n pub fn answer() -> u32 {\n-    41\n+    42\n }\n",
            ),
            FilePatch::new(
                None,
                Some(std::path::PathBuf::from("dist/bundle.js")),
                crate::model::FileStatus::Modified,
                "@@not-a-hunk\n+minified one-liner\n",
            ),
        ];
        let highlighter = SyntaxHighlighter::default();
        // when
        let opened = prepare_open_pr(
            details(),
            patches,
            Vec::new(),
            crate::forge::traits::PullRequestReviewMetadata::default(),
            crate::forge::traits::PullRequestInfo::from_details(details()),
            Some(dir.path()),
            &highlighter,
        )
        .expect("ignored patch must never reach the parser");
        // then only the kept file was parsed into the review
        let kept: Vec<String> = opened
            .diff_files
            .iter()
            .map(|f| f.display_path().display().to_string())
            .collect();
        assert_eq!(kept, vec!["src/main.rs"]);
    }

    #[test]
    fn should_open_an_empty_review_when_every_pr_patch_is_ignored() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        std::fs::write(dir.path().join(".tuicrignore"), "dist/\n")
            .expect("failed to write .tuicrignore");
        let highlighter = SyntaxHighlighter::default();

        let opened = prepare_open_pr(
            details(),
            vec![FilePatch::new(
                None,
                Some(std::path::PathBuf::from("dist/bundle.js")),
                crate::model::FileStatus::Modified,
                "@@ -1 +1 @@\n-old\n+new\n",
            )],
            Vec::new(),
            crate::forge::traits::PullRequestReviewMetadata::default(),
            crate::forge::traits::PullRequestInfo::from_details(details()),
            Some(dir.path()),
            &highlighter,
        )
        .expect("an ignored-only PR should open as an empty review");

        assert!(opened.diff_files.is_empty());
    }

    #[test]
    fn should_pin_base_sha_to_the_resolved_diff_base_before_downstream_reads() {
        // given — a backend reporting the base branch tip as base_sha, with
        // the real three-dot merge base available separately.
        let backend =
            StaticBackend::new(details(), SIMPLE_PATCH).with_diff_base_sha("mergebase111");
        let target = PullRequestTarget::with_repository(repo(), 125, "125");
        let highlighter = SyntaxHighlighter::default();
        // when
        let opened = open_pull_request(&backend, target, None, &highlighter).unwrap();
        // then — both the standalone details and the copy carried on pr_info
        // report the merge base, so PR-mode entry, the info panel, and the
        // reload paths can't disagree about the old side.
        assert_eq!(opened.details.base_sha, "mergebase111");
        assert_eq!(opened.pr_info.details.base_sha, "mergebase111");
        // and — resolution happened before the patch was fetched, so nothing
        // downstream ever observes the unrefined tip.
        let calls = backend.calls.lock().unwrap();
        let resolved_at = calls
            .iter()
            .position(|c| *c == "resolve_diff_base_sha")
            .expect("expected a resolve_diff_base_sha call");
        let diff_at = calls
            .iter()
            .position(|c| *c == "get_pull_request_diff")
            .expect("expected a get_pull_request_diff call");
        assert!(resolved_at < diff_at);
    }

    #[test]
    fn should_keep_reported_base_sha_when_backend_needs_no_refinement() {
        // given — a backend whose base_sha is already the merge base
        // (GitLab's diff_refs.base_sha), modeled by returning None.
        let backend = StaticBackend::new(details(), SIMPLE_PATCH);
        let target = PullRequestTarget::with_repository(repo(), 125, "125");
        let highlighter = SyntaxHighlighter::default();
        // when
        let opened = open_pull_request(&backend, target, None, &highlighter).unwrap();
        // then — untouched.
        assert_eq!(opened.details.base_sha, "1234567890abcdef");
        assert_eq!(opened.pr_info.details.base_sha, "1234567890abcdef");
    }

    /// Fake forge whose calls each sleep for `delay`, recording the call
    /// order and how many calls were in flight at once. Calls named in
    /// `fail` return a forge error after the sleep.
    struct SlowBackend {
        delay: std::time::Duration,
        fail: Vec<&'static str>,
        calls: Mutex<Vec<&'static str>>,
        in_flight: std::sync::atomic::AtomicUsize,
        peak_in_flight: std::sync::atomic::AtomicUsize,
    }

    impl SlowBackend {
        fn new(delay_ms: u64, fail: &[&'static str]) -> Self {
            Self {
                delay: std::time::Duration::from_millis(delay_ms),
                fail: fail.to_vec(),
                calls: Mutex::new(Vec::new()),
                in_flight: Default::default(),
                peak_in_flight: Default::default(),
            }
        }

        fn call<T>(&self, name: &'static str, value: impl FnOnce() -> T) -> Result<T> {
            use std::sync::atomic::Ordering::SeqCst;
            self.calls.lock().unwrap().push(name);
            let now = self.in_flight.fetch_add(1, SeqCst) + 1;
            self.peak_in_flight.fetch_max(now, SeqCst);
            std::thread::sleep(self.delay);
            self.in_flight.fetch_sub(1, SeqCst);
            if self.fail.contains(&name) {
                Err(TuicrError::Forge(format!("{name} failed")))
            } else {
                Ok(value())
            }
        }

        fn peak_in_flight(&self) -> usize {
            self.peak_in_flight
                .load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    impl ForgeBackend for SlowBackend {
        fn list_pull_requests(&self, _query: PullRequestListQuery) -> Result<PagedPullRequests> {
            unimplemented!()
        }
        fn get_pull_request(&self, _target: PullRequestTarget) -> Result<PullRequestDetails> {
            // A fixed timestamp keeps results from separate runs comparable.
            self.call("get_pull_request", || PullRequestDetails {
                updated_at: None,
                ..details()
            })
        }
        fn resolve_diff_base_sha(&self, _pr: &PullRequestDetails) -> Option<String> {
            Some("mergebase111".to_string())
        }
        fn get_pull_request_diff(&self, pr: &PullRequestDetails) -> Result<Vec<FilePatch>> {
            assert_eq!(pr.base_sha, "mergebase111", "diff saw an unpinned base");
            self.call("get_pull_request_diff", || {
                crate::vcs::diff_parser::git_fixture_file_patches(SIMPLE_PATCH)
            })
        }
        fn list_pull_request_commits(
            &self,
            pr: &PullRequestDetails,
        ) -> Result<Vec<PullRequestCommit>> {
            assert_eq!(pr.base_sha, "mergebase111", "commits saw an unpinned base");
            self.call("list_pull_request_commits", || {
                ["c1", "c2"]
                    .into_iter()
                    .map(|oid| PullRequestCommit {
                        oid: oid.to_string(),
                        short_oid: oid.to_string(),
                        summary: format!("commit {oid}"),
                        author: "alice".to_string(),
                        timestamp: None,
                    })
                    .collect()
            })
        }
        fn list_pull_request_review_metadata(
            &self,
            pr: &PullRequestDetails,
        ) -> Result<PullRequestReviewMetadata> {
            assert_eq!(pr.base_sha, "mergebase111", "metadata saw an unpinned base");
            self.call("list_pull_request_review_metadata", || {
                PullRequestReviewMetadata {
                    viewer_login: Some("alice".to_string()),
                    reviews: Vec::new(),
                }
            })
        }
        fn fetch_file_lines(&self, _req: ForgeFileLinesRequest) -> Result<Vec<DiffLine>> {
            unimplemented!()
        }
        fn list_review_threads(
            &self,
            _pr: &PullRequestDetails,
        ) -> Result<Vec<crate::forge::remote_comments::RemoteReviewThread>> {
            unimplemented!()
        }
        fn get_pull_request_commit_range_diff(
            &self,
            _pr: &PullRequestDetails,
            _start_sha: &str,
            _end_sha: &str,
        ) -> Result<Vec<FilePatch>> {
            unimplemented!()
        }
        fn create_review(
            &self,
            _pr: &PullRequestDetails,
            _request: crate::forge::traits::CreateReviewRequest<'_>,
        ) -> Result<crate::forge::traits::GhCreateReviewResponse> {
            unimplemented!()
        }
    }

    /// `fetch_pr_data` as it was before the independent calls ran
    /// concurrently: the reference the concurrent version must agree with.
    fn fetch_pr_data_sequential(
        backend: &dyn ForgeBackend,
        target: PullRequestTarget,
    ) -> Result<PrFetchData> {
        let mut pr_info = backend.get_pull_request_info(target)?;
        if let Some(base_sha) = backend.resolve_diff_base_sha(&pr_info.details) {
            pr_info.details.base_sha = base_sha;
        }
        let details = pr_info.details.clone();
        let patches = backend.get_pull_request_diff(&details)?;
        let commits = backend
            .list_pull_request_commits(&details)
            .unwrap_or_default();
        let review_metadata = backend
            .list_pull_request_review_metadata(&details)
            .unwrap_or_default();
        Ok((details, patches, commits, review_metadata, pr_info))
    }

    fn target() -> PullRequestTarget {
        PullRequestTarget::with_repository(repo(), 125, "125")
    }

    fn outcome(result: Result<PrFetchData>) -> std::result::Result<PrFetchData, String> {
        result.map_err(|e| e.to_string())
    }

    #[test]
    fn should_fetch_diff_commits_and_review_metadata_concurrently() {
        // given — every forge call takes 150ms.
        let delay_ms = 150;
        let sequential_backend = SlowBackend::new(delay_ms, &[]);
        let concurrent_backend = SlowBackend::new(delay_ms, &[]);
        // when
        let started = std::time::Instant::now();
        fetch_pr_data_sequential(&sequential_backend, target()).unwrap();
        let sequential = started.elapsed();
        let started = std::time::Instant::now();
        fetch_pr_data(&concurrent_backend, target()).unwrap();
        let concurrent = started.elapsed();
        eprintln!("4 calls x {delay_ms}ms: sequential {sequential:?}, concurrent {concurrent:?}");
        // then — the sequential path pays for all four calls; the concurrent
        // one pays for the PR metadata plus the slowest of the other three.
        assert!(sequential >= std::time::Duration::from_millis(4 * delay_ms));
        assert!(
            concurrent < std::time::Duration::from_millis(3 * delay_ms),
            "expected ~{}ms, took {concurrent:?}",
            2 * delay_ms
        );
        assert_eq!(sequential_backend.peak_in_flight(), 1);
        assert_eq!(concurrent_backend.peak_in_flight(), 3);
        // and — PR metadata is still fetched before anything that needs it.
        assert_eq!(
            concurrent_backend.calls.lock().unwrap().first(),
            Some(&"get_pull_request")
        );
    }

    #[test]
    fn should_return_the_same_results_and_errors_as_the_sequential_fetch() {
        let scenarios: &[&[&'static str]] = &[
            &[],
            &["get_pull_request"],
            &["get_pull_request_diff"],
            &["list_pull_request_commits"],
            &["list_pull_request_review_metadata"],
            &[
                "list_pull_request_commits",
                "list_pull_request_review_metadata",
            ],
            &["get_pull_request_diff", "list_pull_request_commits"],
            &[
                "get_pull_request_diff",
                "list_pull_request_commits",
                "list_pull_request_review_metadata",
            ],
        ];
        for fail in scenarios {
            // given
            let sequential = SlowBackend::new(0, fail);
            let concurrent = SlowBackend::new(0, fail);
            // when
            let expected = outcome(fetch_pr_data_sequential(&sequential, target()));
            let actual = outcome(fetch_pr_data(&concurrent, target()));
            // then
            assert_eq!(actual, expected, "failing calls: {fail:?}");
        }
    }

    #[test]
    fn should_surface_the_diff_error_and_degrade_optional_calls() {
        // given — the diff and the commit list both fail.
        let backend = SlowBackend::new(0, &["get_pull_request_diff", "list_pull_request_commits"]);
        // when
        let error = fetch_pr_data(&backend, target()).unwrap_err();
        // then — the required call's error wins; the best-effort one is dropped.
        assert_eq!(error.to_string(), "get_pull_request_diff failed");

        // given — only the best-effort calls fail.
        let backend = SlowBackend::new(
            0,
            &[
                "list_pull_request_commits",
                "list_pull_request_review_metadata",
            ],
        );
        // when
        let (_, patches, commits, review_metadata, _) = fetch_pr_data(&backend, target()).unwrap();
        // then — the diff still opens, without the commit selector.
        assert_eq!(patches.len(), 1);
        assert!(commits.is_empty());
        assert_eq!(review_metadata, PullRequestReviewMetadata::default());
    }

    #[test]
    fn should_not_start_dependent_calls_when_pr_metadata_fails() {
        // given
        let backend = SlowBackend::new(0, &["get_pull_request"]);
        // when
        let result = fetch_pr_data(&backend, target());
        // then
        assert!(result.is_err());
        assert_eq!(
            backend.calls.lock().unwrap().as_slice(),
            &["get_pull_request"]
        );
    }
}
