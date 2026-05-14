//! Context provider abstraction for gap expansion.
//!
//! Gap expansion needs file content at an exact revision: working tree for
//! local review, base/head SHAs for PR review. `ContextProvider` factors
//! that lookup so the App can route expansion through the right source
//! without knowing whether the diff is local or remote.
//!
//! Implementations:
//! - `VcsContextProvider`: delegates to `VcsBackend::fetch_context_lines`.
//! - `ForgeContextProvider`: builds a `ForgeFileLinesRequest` for the given
//!   file/status and delegates to `ForgeBackend::fetch_file_lines`.

use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::forge::traits::{ForgeBackend, ForgeFileLinesRequest, ForgeRepository, PrSessionKey};
use crate::model::{DiffLine, FileStatus};
use crate::vcs::VcsBackend;

/// Source of context lines for gap expansion. Implementations either read
/// from a local VCS working tree / blob, or fetch the exact base/head SHA
/// from a remote forge.
pub trait ContextProvider {
    /// Fetch context lines for a file in `[start_line, end_line]` inclusive.
    /// `old_path` and `new_path` come straight from the parsed diff and are
    /// used to map rename/copy sides correctly.
    fn fetch_context_lines(
        &self,
        old_path: Option<&PathBuf>,
        new_path: Option<&PathBuf>,
        file_status: FileStatus,
        start_line: u32,
        end_line: u32,
    ) -> Result<Vec<DiffLine>>;
}

/// Adapter over a `VcsBackend`. Picks the appropriate display path (new on
/// the head side, old on the base side) and forwards to the backend.
pub struct VcsContextProvider<'a> {
    pub vcs: &'a dyn VcsBackend,
}

impl ContextProvider for VcsContextProvider<'_> {
    fn fetch_context_lines(
        &self,
        old_path: Option<&PathBuf>,
        new_path: Option<&PathBuf>,
        file_status: FileStatus,
        start_line: u32,
        end_line: u32,
    ) -> Result<Vec<DiffLine>> {
        // Mirror the existing display_path rule: prefer new_path, fall back
        // to old_path. Local VCS backends pick the side internally based on
        // file status.
        let path: &Path = match new_path.or(old_path) {
            Some(p) => p.as_path(),
            None => return Ok(Vec::new()),
        };
        self.vcs
            .fetch_context_lines(path, file_status, start_line, end_line)
    }
}

/// Adapter over a `ForgeBackend` for PR review mode.
///
/// Built from the PR's `PrSessionKey` (carries head SHA and repository) and
/// the captured `base_sha`. Each `fetch_context_lines` call constructs the
/// appropriate `ForgeFileLinesRequest` and delegates to the backend.
pub struct ForgeContextProvider<'a> {
    pub forge: &'a dyn ForgeBackend,
    pub repository: ForgeRepository,
    pub base_sha: String,
    pub head_sha: String,
}

impl<'a> ForgeContextProvider<'a> {
    pub fn for_pr(
        forge: &'a dyn ForgeBackend,
        key: &PrSessionKey,
        base_sha: impl Into<String>,
    ) -> Self {
        Self {
            forge,
            repository: key.repository.clone(),
            base_sha: base_sha.into(),
            head_sha: key.head_sha.clone(),
        }
    }
}

impl ContextProvider for ForgeContextProvider<'_> {
    fn fetch_context_lines(
        &self,
        old_path: Option<&PathBuf>,
        new_path: Option<&PathBuf>,
        file_status: FileStatus,
        start_line: u32,
        end_line: u32,
    ) -> Result<Vec<DiffLine>> {
        let side = ForgeFileLinesRequest::side_for_status(file_status);
        let Some(path) = ForgeFileLinesRequest::path_for_side(side, old_path, new_path) else {
            return Ok(Vec::new());
        };
        let request = ForgeFileLinesRequest {
            repository: self.repository.clone(),
            base_sha: self.base_sha.clone(),
            head_sha: self.head_sha.clone(),
            path,
            status: file_status,
            side,
            start_line,
            end_line,
        };
        self.forge.fetch_file_lines(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::traits::{
        ForgeFileSide, PagedPullRequests, PullRequestDetails, PullRequestListQuery,
        PullRequestTarget,
    };
    use crate::model::LineOrigin;
    use std::cell::RefCell;

    struct CapturingForge {
        seen: RefCell<Vec<ForgeFileLinesRequest>>,
        response: Vec<DiffLine>,
    }

    impl ForgeBackend for CapturingForge {
        fn list_pull_requests(&self, _query: PullRequestListQuery) -> Result<PagedPullRequests> {
            unimplemented!()
        }
        fn get_pull_request(&self, _target: PullRequestTarget) -> Result<PullRequestDetails> {
            unimplemented!()
        }
        fn get_pull_request_diff(&self, _pr: &PullRequestDetails) -> Result<String> {
            unimplemented!()
        }
        fn fetch_file_lines(&self, request: ForgeFileLinesRequest) -> Result<Vec<DiffLine>> {
            self.seen.borrow_mut().push(request);
            Ok(self.response.clone())
        }
        fn list_review_threads(
            &self,
            _pr: &PullRequestDetails,
        ) -> Result<Vec<crate::forge::remote_comments::RemoteReviewThread>> {
            Ok(Vec::new())
        }
    }

    fn repo() -> ForgeRepository {
        ForgeRepository::github("github.com", "agavra", "tuicr")
    }

    fn key() -> PrSessionKey {
        PrSessionKey::new(repo(), 125, "headsha".to_string())
    }

    fn make_line(text: &str) -> DiffLine {
        DiffLine {
            origin: LineOrigin::Context,
            content: text.to_string(),
            old_lineno: Some(1),
            new_lineno: Some(1),
            highlighted_spans: None,
        }
    }

    #[test]
    fn should_use_head_side_for_modified_file() {
        // given
        let forge = CapturingForge {
            seen: RefCell::new(Vec::new()),
            response: vec![make_line("a")],
        };
        let provider = ForgeContextProvider::for_pr(&forge, &key(), "basesha");
        let new_path = PathBuf::from("src/lib.rs");
        // when
        let _ = provider
            .fetch_context_lines(None, Some(&new_path), FileStatus::Modified, 1, 1)
            .unwrap();
        // then
        let seen = forge.seen.borrow();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].side, ForgeFileSide::Head);
        assert_eq!(seen[0].sha(), "headsha");
        assert_eq!(seen[0].path, PathBuf::from("src/lib.rs"));
    }

    #[test]
    fn should_use_base_side_for_deleted_file() {
        // given
        let forge = CapturingForge {
            seen: RefCell::new(Vec::new()),
            response: vec![],
        };
        let provider = ForgeContextProvider::for_pr(&forge, &key(), "basesha");
        let old_path = PathBuf::from("src/gone.rs");
        // when
        let _ = provider
            .fetch_context_lines(Some(&old_path), None, FileStatus::Deleted, 1, 1)
            .unwrap();
        // then
        let seen = forge.seen.borrow();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].side, ForgeFileSide::Base);
        assert_eq!(seen[0].sha(), "basesha");
        assert_eq!(seen[0].path, PathBuf::from("src/gone.rs"));
    }

    #[test]
    fn should_pick_old_path_for_rename_base_side() {
        // when called for a renamed file's base side (we'd never call that in
        // expansion today, but the helper must do it), the old path wins.
        let old = PathBuf::from("old.rs");
        let new = PathBuf::from("new.rs");
        assert_eq!(
            ForgeFileLinesRequest::path_for_side(ForgeFileSide::Base, Some(&old), Some(&new),),
            Some(old.clone()),
        );
        assert_eq!(
            ForgeFileLinesRequest::path_for_side(ForgeFileSide::Head, Some(&old), Some(&new),),
            Some(new),
        );
    }

    #[test]
    fn should_short_circuit_when_path_missing() {
        // given a file with no paths at all (degenerate case)
        let forge = CapturingForge {
            seen: RefCell::new(Vec::new()),
            response: vec![make_line("x")],
        };
        let provider = ForgeContextProvider::for_pr(&forge, &key(), "basesha");
        // when
        let result = provider
            .fetch_context_lines(None, None, FileStatus::Modified, 1, 5)
            .unwrap();
        // then — empty result, no backend call
        assert!(result.is_empty());
        assert!(forge.seen.borrow().is_empty());
    }
}
