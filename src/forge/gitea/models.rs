//! Serde shapes for the Gitea REST v1 responses tuicr consumes.
//!
//! Field names follow the wire format exactly; conversion into tuicr's neutral
//! forge types lives in [`super::tea`].
//!
//! Every field tuicr does not read is omitted, and every field the server may
//! legitimately omit or null is optional — a Gitea instance running an older
//! point release should degrade rather than fail to deserialize.

use chrono::{DateTime, Utc};
use serde::Deserialize;

/// A user reference. Nulled out for ghost users (deleted accounts), so every
/// call site treats the whole object as optional.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct GiteaUser {
    #[serde(default)]
    pub login: String,
}

impl GiteaUser {
    /// `None` when the server sent a ghost user or an empty login.
    pub(crate) fn login(user: Option<Self>) -> Option<String> {
        user.map(|user| user.login)
            .filter(|login| !login.is_empty())
    }
}

/// One side of a pull request.
///
/// `sha` is the immutable commit tuicr anchors a session to. `r#ref` is the
/// branch name, but Gitea substitutes `refs/pull/<n>/head` once the head
/// branch is deleted, so it is display-only.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct GiteaBranch {
    #[serde(default, rename = "ref")]
    pub ref_name: String,
    #[serde(default)]
    pub sha: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct GiteaPullRequest {
    pub number: u64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub user: Option<GiteaUser>,
    #[serde(default)]
    pub html_url: String,
    pub head: GiteaBranch,
    pub base: GiteaBranch,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub merged: bool,
    #[serde(default)]
    pub merged_at: Option<DateTime<Utc>>,
    /// Merge base of head and base. Present on the single-PR endpoint only;
    /// the list endpoint omits it.
    #[serde(default)]
    pub merge_base: Option<String>,
    #[serde(default)]
    pub mergeable: Option<bool>,
    #[serde(default)]
    pub requested_reviewers: Option<Vec<GiteaUser>>,
}

/// A row from `/repos/{owner}/{repo}/pulls/{index}/files`.
///
/// Note the absence of a `patch` field — unlike GitHub, Gitea does not inline
/// per-file diff text here, so the patch bodies come from `/pulls/{n}.diff`
/// and are paired positionally.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct GiteaChangedFile {
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub previous_filename: Option<String>,
    #[serde(default)]
    pub status: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct GiteaCommitAuthor {
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct GiteaCommitPayload {
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub author: Option<GiteaCommitAuthor>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct GiteaCommit {
    #[serde(default)]
    pub sha: String,
    #[serde(default)]
    pub commit: Option<GiteaCommitPayload>,
    /// Forge account that authored the commit. Null when the commit email
    /// matches no account.
    #[serde(default)]
    pub author: Option<GiteaUser>,
    #[serde(default)]
    pub created: Option<DateTime<Utc>>,
}

/// A submitted (or pending) review. `PENDING` reviews are only ever returned
/// to the user who created them.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct GiteaPullReview {
    pub id: u64,
    #[serde(default)]
    pub user: Option<GiteaUser>,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub commit_id: String,
    #[serde(default)]
    pub submitted_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub comments_count: u32,
}

/// A line-anchored review comment.
///
/// The read and write shapes disagree: reading gives `position` /
/// `original_position`, writing takes `new_position` / `old_position`. Both
/// carry a file line number, and exactly one is non-zero — that is how Gitea
/// encodes which side the comment sits on.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct GiteaPullReviewComment {
    pub id: u64,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub user: Option<GiteaUser>,
    /// Non-null once someone marks the conversation resolved.
    #[serde(default)]
    pub resolver: Option<GiteaUser>,
    #[serde(default)]
    pub pull_request_review_id: u64,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub commit_id: String,
    #[serde(default)]
    pub html_url: String,
    /// Line number on the new (right) side; 0 when the comment is old-side.
    #[serde(default)]
    pub position: u32,
    /// Line number on the old (left) side; 0 when the comment is new-side.
    #[serde(default)]
    pub original_position: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct GiteaCommitStatus {
    #[serde(default)]
    pub context: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub target_url: Option<String>,
}

/// `/repos/{owner}/{repo}/commits/{sha}/status` — the combined view, already
/// deduplicated to the newest status per context.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct GiteaCombinedStatus {
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub statuses: Vec<GiteaCommitStatus>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct GiteaIssueComment {
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub user: Option<GiteaUser>,
    #[serde(default)]
    pub html_url: Option<String>,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct GiteaRepositoryMeta {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub owner: Option<GiteaUser>,
    /// `"<owner>/<repo>"`. Preferred over `owner`/`name` because Gitea fills
    /// it in even when the owner object is elided.
    #[serde(default)]
    pub full_name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct GiteaPullRequestMeta {
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub merged: bool,
    #[serde(default)]
    pub html_url: Option<String>,
}

/// A row from `/repos/issues/search`, used for the review-requested scope.
///
/// This is the issue shape, not the pull shape: it carries no head/base refs,
/// so summaries built from it leave those blank.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct GiteaIssue {
    pub number: u64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub user: Option<GiteaUser>,
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub pull_request: Option<GiteaPullRequestMeta>,
    #[serde(default)]
    pub repository: Option<GiteaRepositoryMeta>,
}

/// One entry of `tea logins list --output json`.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct TeaLogin {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub url: String,
    /// `tea` renders this as the string `"true"`/`"false"` rather than a JSON
    /// bool, so it is read permissively and both spellings are accepted.
    #[serde(default)]
    pub default: Option<serde_json::Value>,
}

impl TeaLogin {
    pub(crate) fn is_default(&self) -> bool {
        self.default.as_ref().is_some_and(|value| {
            value
                .as_bool()
                .unwrap_or_else(|| value.as_str() == Some("true"))
        })
    }
}
