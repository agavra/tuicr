use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::error::Result;
use crate::forge::remote_comments::RemoteReviewThread;
use crate::forge::submit::SubmitEvent;
use crate::model::{DiffLine, FileStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForgeKind {
    GitHub,
    GitLab,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForgeRepository {
    pub kind: ForgeKind,
    pub host: String,
    pub owner: String,
    pub name: String,
}

impl ForgeRepository {
    pub fn github(
        host: impl Into<String>,
        owner: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        Self {
            kind: ForgeKind::GitHub,
            host: host.into(),
            owner: owner.into(),
            name: name.into(),
        }
    }

    pub fn gitlab(
        host: impl Into<String>,
        owner: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        Self {
            kind: ForgeKind::GitLab,
            host: host.into(),
            owner: owner.into(),
            name: name.into(),
        }
    }

    pub fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }

    pub fn display_name(&self) -> String {
        if self.host == "github.com" || self.host == "gitlab.com" {
            self.slug()
        } else {
            format!("{}/{}", self.host, self.slug())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequestTarget {
    pub repository: Option<ForgeRepository>,
    pub number: u64,
    pub original: String,
}

impl PullRequestTarget {
    pub fn number(number: u64, original: impl Into<String>) -> Self {
        Self {
            repository: None,
            number,
            original: original.into(),
        }
    }

    pub fn with_repository(
        repository: ForgeRepository,
        number: u64,
        original: impl Into<String>,
    ) -> Self {
        Self {
            repository: Some(repository),
            number,
            original: original.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PullRequestListScope {
    #[default]
    Open,
    ReviewRequested,
}

impl PullRequestListScope {
    pub fn toggled(self) -> Self {
        match self {
            Self::Open => Self::ReviewRequested,
            Self::ReviewRequested => Self::Open,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Open => "all",
            Self::ReviewRequested => "requested",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestListQuery {
    pub repository: ForgeRepository,
    pub already_loaded: usize,
    pub page_size: usize,
    pub scope: PullRequestListScope,
}

impl PullRequestListQuery {
    pub fn first_page(repository: ForgeRepository, page_size: usize) -> Self {
        Self::first_page_with_scope(repository, page_size, PullRequestListScope::Open)
    }

    pub fn first_page_with_scope(
        repository: ForgeRepository,
        page_size: usize,
        scope: PullRequestListScope,
    ) -> Self {
        Self {
            repository,
            already_loaded: 0,
            page_size,
            scope,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequestSummary {
    pub repository: ForgeRepository,
    pub number: u64,
    pub title: String,
    pub author: Option<String>,
    pub head_ref_name: String,
    pub base_ref_name: String,
    pub updated_at: Option<DateTime<Utc>>,
    pub url: String,
    pub state: String,
    pub is_draft: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PagedPullRequests {
    pub pull_requests: Vec<PullRequestSummary>,
    pub has_more: bool,
    pub total_loaded: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequestDetails {
    pub repository: ForgeRepository,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub state: String,
    pub is_draft: bool,
    pub author: Option<String>,
    pub head_ref_name: String,
    pub base_ref_name: String,
    pub head_sha: String,
    pub base_sha: String,
    pub body: String,
    pub updated_at: Option<DateTime<Utc>>,
    pub closed: bool,
    pub merged_at: Option<DateTime<Utc>>,
    /// GitLab diff start SHA for inline comment position anchoring.
    /// None for GitHub; populated from `diff_refs.start_sha` for GitLab.
    #[serde(default)]
    pub diff_start_sha: Option<String>,
}

impl PullRequestDetails {
    pub fn is_read_only(&self) -> bool {
        self.closed || self.merged_at.is_some()
    }

    pub fn read_only_reason(&self) -> Option<&'static str> {
        if self.merged_at.is_some() {
            Some("merged")
        } else if self.closed {
            Some("closed")
        } else {
            None
        }
    }
}

/// Stable identity for a PR review session.
///
/// Sessions are keyed by forge kind + host + owner/repo + PR number + head
/// SHA per the spec. Two opens of the same PR at the same head SHA must
/// produce equal keys so persistence reattaches local comments and reviewed
/// markers; a PR that advances to a new head opens a new session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrSessionKey {
    pub repository: ForgeRepository,
    pub number: u64,
    pub head_sha: String,
}

impl PrSessionKey {
    pub fn new(repository: ForgeRepository, number: u64, head_sha: impl Into<String>) -> Self {
        Self {
            repository,
            number,
            head_sha: head_sha.into(),
        }
    }

    pub fn from_details(details: &PullRequestDetails) -> Self {
        Self::new(
            details.repository.clone(),
            details.number,
            details.head_sha.clone(),
        )
    }

    /// Short, human-recognizable head SHA prefix used in filenames and UI.
    pub fn short_head(&self) -> String {
        self.head_sha
            .chars()
            .take(8.min(self.head_sha.len()))
            .collect()
    }
}

/// Which side of a pull request diff the caller wants to read from.
///
/// Maps to a concrete SHA + path: for added/modified/copied/renamed files
/// the caller wants the head side; for deleted files the base side. Renames
/// pick the old path on the base side and the new path on the head side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForgeFileSide {
    Base,
    Head,
}

/// A single request to read file lines from a forge for context expansion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForgeFileLinesRequest {
    pub repository: ForgeRepository,
    /// Base SHA captured when the PR was opened.
    pub base_sha: String,
    /// Head SHA captured when the PR was opened.
    pub head_sha: String,
    /// File path relative to the repository root.
    pub path: PathBuf,
    /// File status, used to choose the appropriate side without forcing the
    /// caller to also compute it. Renames use `Renamed`; `path` should already
    /// reflect the chosen side.
    pub status: FileStatus,
    /// Which side to read from. The caller is responsible for picking the
    /// right side per the spec mapping rules.
    pub side: ForgeFileSide,
    /// Inclusive 1-based line range. Caller is responsible for clamping.
    pub start_line: u32,
    pub end_line: u32,
}

impl ForgeFileLinesRequest {
    /// Resolve the side and path for a given file based on its status.
    /// Helper for callers that have a `DiffFile` and want to fetch context.
    pub fn side_for_status(status: FileStatus) -> ForgeFileSide {
        match status {
            FileStatus::Deleted => ForgeFileSide::Base,
            FileStatus::Added | FileStatus::Modified | FileStatus::Renamed | FileStatus::Copied => {
                ForgeFileSide::Head
            }
        }
    }

    /// Pick the right path for a forge fetch given old/new paths and the
    /// side. Renamed files use `old_path` on the base side, `new_path` on
    /// the head side.
    pub fn path_for_side(
        side: ForgeFileSide,
        old_path: Option<&PathBuf>,
        new_path: Option<&PathBuf>,
    ) -> Option<PathBuf> {
        match side {
            ForgeFileSide::Base => old_path.or(new_path).cloned(),
            ForgeFileSide::Head => new_path.or(old_path).cloned(),
        }
    }

    /// Return the SHA matching `side`.
    pub fn sha(&self) -> &str {
        match self.side {
            ForgeFileSide::Base => &self.base_sha,
            ForgeFileSide::Head => &self.head_sha,
        }
    }
}

/// Response returned by `ForgeBackend::create_review` after a successful
/// `POST .../pulls/<n>/reviews`. Carries enough state to drive lifecycle
/// writes on the source comments and the success message in the status bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GhCreateReviewResponse {
    /// GitHub's numeric review ID. Stored on each included `Comment` as
    /// `remote_review_id` (stringified).
    pub id: u64,
    /// Web URL of the created review — used in the draft success message so
    /// users can finish the pending review in GitHub.
    pub html_url: String,
    /// Review state as reported by GitHub (`PENDING`, `COMMENTED`,
    /// `APPROVED`, `CHANGES_REQUESTED`). Kept for debugging/logging.
    pub state: String,
}

/// Request to create a review against a PR. The payload is the forge-agnostic
/// shape of the JSON body the backend will POST; downstream the GitHub
/// backend reshapes it via `build_review_payload` and writes it on stdin.
#[derive(Debug, Clone)]
pub struct CreateReviewRequest<'a> {
    pub event: SubmitEvent,
    pub commit_id: &'a str,
    pub body: &'a str,
    pub comments: &'a [crate::forge::submit::InlineComment],
}

/// A single commit on a pull request, as returned by the forge.
///
/// Fields mirror what the inline commit selector needs to render a row.
/// Backends populate `oid`, `summary`, and `author`; `timestamp` is best-
/// effort (None when the forge does not expose a parseable value).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequestCommit {
    pub oid: String,
    pub short_oid: String,
    pub summary: String,
    pub author: String,
    pub timestamp: Option<DateTime<Utc>>,
}

pub trait ForgeBackend {
    fn list_pull_requests(&self, query: PullRequestListQuery) -> Result<PagedPullRequests>;
    fn get_pull_request(&self, target: PullRequestTarget) -> Result<PullRequestDetails>;
    fn get_pull_request_diff(&self, pr: &PullRequestDetails) -> Result<String>;
    /// Fetch the requested file lines from the forge for context expansion.
    /// Implementations may optimize by reading from a local checkout when
    /// available; the trait does not require that path.
    fn fetch_file_lines(&self, request: ForgeFileLinesRequest) -> Result<Vec<DiffLine>>;
    /// Fetch existing review discussions for a PR, including their resolved
    /// and outdated state. Implementations should return all threads in
    /// posted order; filtering by visibility happens in the App.
    fn list_review_threads(&self, pr: &PullRequestDetails) -> Result<Vec<RemoteReviewThread>>;
    /// Fetch review-level summary comments — the body text on each
    /// `PullRequestReview`, distinct from line-anchored threads. Default
    /// returns an empty list; only forges with review-summary semantics
    /// (GitHub) need to override.
    fn list_review_summaries(
        &self,
        _pr: &PullRequestDetails,
    ) -> Result<Vec<crate::forge::remote_comments::RemoteReviewSummary>> {
        Ok(Vec::new())
    }
    /// List the commits that make up a pull request, in chronological order
    /// (oldest first; the App reverses to newest-first display order). The
    /// list scopes the inline commit selector so users can narrow a PR's
    /// cumulative diff down to a contiguous subrange.
    fn list_pull_request_commits(&self, pr: &PullRequestDetails) -> Result<Vec<PullRequestCommit>>;
    /// Fetch the cumulative diff between two commit SHAs that both belong to
    /// `pr`. `start_sha` is the *parent* of the first commit in the
    /// subrange; `end_sha` is the last commit. Implementations may use a
    /// local checkout when both SHAs are present locally, but the source of
    /// truth is the forge.
    fn get_pull_request_commit_range_diff(
        &self,
        pr: &PullRequestDetails,
        start_sha: &str,
        end_sha: &str,
    ) -> Result<String>;
    /// Optional path to a local checkout the backend may consult as an
    /// optimization. The default returns `None`; callers must never treat
    /// this path as the source of truth for PR contents.
    fn local_checkout_path(&self) -> Option<PathBuf> {
        None
    }

    /// Create a review on the PR. The payload-building details (event field
    /// mapping, comment serialization) are the backend's responsibility — the
    /// caller only supplies the high-level inputs. Returns a minimal response
    /// describing the created review (id, html_url, state).
    fn create_review(
        &self,
        pr: &PullRequestDetails,
        request: CreateReviewRequest<'_>,
    ) -> Result<GhCreateReviewResponse>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_round_trip_pr_session_key_via_serde() {
        // given
        let key = PrSessionKey::new(
            ForgeRepository::github("github.com", "agavra", "tuicr"),
            125,
            "abcdef0123456789".to_string(),
        );
        // when
        let serialized = serde_json::to_string(&key).unwrap();
        let restored: PrSessionKey = serde_json::from_str(&serialized).unwrap();
        // then
        assert_eq!(key, restored);
    }

    #[test]
    fn should_truncate_long_head_sha_for_short_head() {
        // given
        let key = PrSessionKey::new(
            ForgeRepository::github("github.com", "a", "b"),
            1,
            "1234567890abcdef1234567890abcdef".to_string(),
        );
        // when/then
        assert_eq!(key.short_head(), "12345678");
    }

    #[test]
    fn should_handle_short_head_sha_gracefully() {
        // given
        let key = PrSessionKey::new(
            ForgeRepository::github("github.com", "a", "b"),
            1,
            "abc".to_string(),
        );
        // when/then
        assert_eq!(key.short_head(), "abc");
    }

    #[test]
    fn should_pick_head_side_for_added_modified_renamed_copied() {
        for status in [
            FileStatus::Added,
            FileStatus::Modified,
            FileStatus::Renamed,
            FileStatus::Copied,
        ] {
            assert_eq!(
                ForgeFileLinesRequest::side_for_status(status),
                ForgeFileSide::Head,
                "{status:?} should pick head"
            );
        }
    }

    #[test]
    fn should_pick_base_side_for_deleted_files() {
        assert_eq!(
            ForgeFileLinesRequest::side_for_status(FileStatus::Deleted),
            ForgeFileSide::Base,
        );
    }
}
