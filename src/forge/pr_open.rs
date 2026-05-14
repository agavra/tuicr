//! PR open path.
//!
//! Given a `ForgeBackend` and a `PullRequestTarget`, produce the materials
//! the App needs to enter PR review mode: parsed diff files, a session, and
//! a `PrSessionKey` that scopes persistence and remote context fetches.
//!
//! Key invariants enforced here:
//! - The current local checkout is never treated as the source of truth.
//!   Diffs are parsed from `gh pr diff`; SHAs are captured from PR metadata.
//! - `.tuicrignore` is applied only when the caller supplies a local
//!   checkout path. Outside a checkout, the unfiltered diff is shown.
//! - No checkout mutation. We never spawn `git checkout/fetch/reset/stash`
//!   or branch-creation commands here.

use std::path::{Path, PathBuf};

use crate::error::{Result, TuicrError};
use crate::forge::traits::{ForgeBackend, PrSessionKey, PullRequestDetails, PullRequestTarget};
use crate::model::{DiffFile, FileStatus, ReviewSession, SessionDiffSource};
use crate::syntax::SyntaxHighlighter;
use crate::tuicrignore;
use crate::vcs::diff_parser::{DiffFormat, parse_unified_diff};

/// Everything the App needs to enter PR review mode.
#[derive(Debug)]
pub struct OpenedPullRequest {
    pub details: PullRequestDetails,
    pub diff_files: Vec<DiffFile>,
    pub session: ReviewSession,
    pub key: PrSessionKey,
}

/// Open a PR target through a forge backend and prepare review state.
///
/// `local_checkout` is optional: when provided, `.tuicrignore` rules at the
/// root are applied. When absent (PR opened via URL outside a checkout, or
/// for a different repo), no filtering happens.
pub fn open_pull_request(
    backend: &dyn ForgeBackend,
    target: PullRequestTarget,
    local_checkout: Option<&Path>,
    highlighter: &SyntaxHighlighter,
) -> Result<OpenedPullRequest> {
    let (details, patch) = fetch_pr_data(backend, target)?;
    prepare_open_pr(details, &patch, local_checkout, highlighter)
}

/// Network-only half of the PR open path: fetch PR metadata and the raw
/// patch text. Safe to run on a background thread because it does no
/// syntax parsing and holds nothing that isn't `Send`.
pub fn fetch_pr_data(
    backend: &dyn ForgeBackend,
    target: PullRequestTarget,
) -> Result<(PullRequestDetails, String)> {
    let details = backend.get_pull_request(target)?;
    let patch = backend.get_pull_request_diff(&details)?;
    Ok((details, patch))
}

/// CPU-only half of the PR open path: parse the patch, apply
/// `.tuicrignore`, and build the session. Runs on the main thread because
/// `SyntaxHighlighter` is not trivially `Send`-cloneable.
pub fn prepare_open_pr(
    details: PullRequestDetails,
    patch: &str,
    local_checkout: Option<&Path>,
    highlighter: &SyntaxHighlighter,
) -> Result<OpenedPullRequest> {
    let parsed = match parse_unified_diff(patch, DiffFormat::GitStyle, highlighter) {
        Ok(files) => files,
        Err(TuicrError::NoChanges) => {
            return Err(TuicrError::Forge(format!(
                "Pull request #{} has no file changes",
                details.number
            )));
        }
        Err(e) => return Err(e),
    };

    let diff_files = match local_checkout {
        Some(root) => tuicrignore::filter_diff_files(root, parsed),
        None => parsed,
    };

    let key = PrSessionKey::from_details(&details);
    let session = build_session(&details, &key, &diff_files);

    Ok(OpenedPullRequest {
        details,
        diff_files,
        session,
        key,
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
        let path: PathBuf = file.display_path().clone();
        let status: FileStatus = file.status;
        session.add_file(path, status, file.content_hash);
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
    use std::cell::RefCell;

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
        }
    }

    struct StaticBackend {
        details: PullRequestDetails,
        patch: String,
        calls: RefCell<Vec<&'static str>>,
    }

    impl ForgeBackend for StaticBackend {
        fn list_pull_requests(&self, _query: PullRequestListQuery) -> Result<PagedPullRequests> {
            unimplemented!()
        }
        fn get_pull_request(&self, _target: PullRequestTarget) -> Result<PullRequestDetails> {
            self.calls.borrow_mut().push("get_pull_request");
            Ok(self.details.clone())
        }
        fn get_pull_request_diff(&self, _pr: &PullRequestDetails) -> Result<String> {
            self.calls.borrow_mut().push("get_pull_request_diff");
            Ok(self.patch.clone())
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
        let backend = StaticBackend {
            details: details(),
            patch: SIMPLE_PATCH.to_string(),
            calls: RefCell::new(Vec::new()),
        };
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
        // and — both forge calls were made, in order
        assert_eq!(
            backend.calls.borrow().as_slice(),
            &["get_pull_request", "get_pull_request_diff"],
        );
    }

    /// Patch fixture covering add/modify/delete/rename in a single PR
    /// diff, mirroring what `gh pr diff --patch --color never` would
    /// emit. Acts as a regression guard against future changes to the
    /// shared diff parser.
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
        let backend = StaticBackend {
            details: details(),
            patch: MULTI_STATUS_PATCH.to_string(),
            calls: RefCell::new(Vec::new()),
        };
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
        let backend = StaticBackend {
            details: details(),
            patch: String::new(),
            calls: RefCell::new(Vec::new()),
        };
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
}
