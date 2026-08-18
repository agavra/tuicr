//! Deserialization models for the Bitbucket Cloud REST 2.0 API.
//!
//! Only Bitbucket Cloud is supported. Data Center speaks an unrelated REST
//! 1.0 API (project keys, `/rest/api/1.0`, comment versioning) and is
//! rejected during remote-URL parsing rather than modelled here.
//!
//! Two Cloud quirks shape these types:
//! - Pull request payloads carry *abbreviated* (12-char) commit hashes while
//!   the commits endpoint returns full 40-char hashes. `bkt.rs` promotes the
//!   short ones before they reach a `PrSessionKey`.
//! - `user.username` is always empty (Atlassian removed usernames), so
//!   `display_name` is what gets rendered and `uuid` is used for identity.

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::error::{Result, TuicrError};
use crate::forge::remote_comments::{
    RemoteCommentSide, RemoteReviewComment, RemoteReviewState, RemoteReviewSummary,
    RemoteReviewThread,
};
use crate::forge::traits::{
    ForgeRepository, PullRequestCommit, PullRequestDetails, PullRequestReviewRecord,
    PullRequestSummary,
};
use crate::model::FileStatus;
use crate::vcs::git::raw::FileMetadata;

#[derive(Debug, Deserialize, Default)]
pub struct BbDiffStatFile {
    #[serde(default)]
    pub path: String,
}

/// One machine-readable entry from Bitbucket Cloud's diffstat endpoint.
#[derive(Debug, Deserialize)]
pub struct BbDiffStat {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub old: Option<BbDiffStatFile>,
    #[serde(default)]
    pub new: Option<BbDiffStatFile>,
}

impl BbDiffStat {
    pub(crate) fn into_metadata(self) -> Result<FileMetadata> {
        let old_path = self
            .old
            .filter(|file| !file.path.is_empty())
            .map(|file| std::path::PathBuf::from(file.path));
        let new_path = self
            .new
            .filter(|file| !file.path.is_empty())
            .map(|file| std::path::PathBuf::from(file.path));
        let status = match self.status.to_ascii_lowercase().as_str() {
            "added" => FileStatus::Added,
            "removed" => FileStatus::Deleted,
            "renamed" => FileStatus::Renamed,
            "copied" => FileStatus::Copied,
            "modified" => FileStatus::Modified,
            status => {
                return Err(TuicrError::Forge(format!(
                    "Bitbucket returned unsupported diffstat status `{status}`"
                )));
            }
        };
        Ok(FileMetadata {
            old_path,
            new_path,
            status,
        })
    }
}

/// Envelope Bitbucket Cloud wraps every paginated collection in. `next` is
/// absent on the final page, which is how callers detect the end.
#[derive(Debug, Deserialize)]
pub struct BbPaged<T> {
    #[serde(default = "Vec::new")]
    pub values: Vec<T>,
    #[serde(default)]
    pub next: Option<String>,
}

impl<T> BbPaged<T> {
    pub fn has_more(&self) -> bool {
        self.next.is_some()
    }
}

#[derive(Debug, Deserialize, Default)]
pub struct BbUser {
    /// Always empty on Cloud; kept explicit rather than silently omitted.
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub uuid: String,
    #[serde(default)]
    pub account_id: String,
}

impl BbUser {
    /// Human-facing handle. Cloud leaves `username` blank, so fall through to
    /// `display_name` and finally the opaque uuid rather than rendering "".
    pub fn label(&self) -> Option<String> {
        [&self.username, &self.display_name, &self.uuid]
            .into_iter()
            .find(|candidate| !candidate.is_empty())
            .cloned()
    }
}

#[derive(Debug, Deserialize, Default)]
pub struct BbLink {
    #[serde(default)]
    pub href: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct BbLinks {
    #[serde(default)]
    pub html: Option<BbLink>,
}

impl BbLinks {
    pub fn html_href(&self) -> String {
        self.html
            .as_ref()
            .map(|link| link.href.clone())
            .unwrap_or_default()
    }
}

#[derive(Debug, Deserialize, Default)]
pub struct BbBranch {
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct BbCommitRef {
    #[serde(default)]
    pub hash: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct BbEndpoint {
    #[serde(default)]
    pub branch: BbBranch,
    #[serde(default)]
    pub commit: Option<BbCommitRef>,
}

impl BbEndpoint {
    pub fn hash(&self) -> String {
        self.commit
            .as_ref()
            .map(|commit| commit.hash.clone())
            .unwrap_or_default()
    }
}

/// A participant on a pull request. Reviewers who have acted carry
/// `approved` / `state` / `participated_on`.
#[derive(Debug, Deserialize, Default)]
pub struct BbParticipant {
    #[serde(default)]
    pub user: BbUser,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub approved: bool,
    /// `approved`, `changes_requested`, or null.
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub participated_on: Option<DateTime<Utc>>,
}

/// `GET /2.0/repositories/{ws}/{repo}/pullrequests[/{id}]` — the same shape
/// serves both the list and the detail endpoints.
#[derive(Debug, Deserialize)]
pub struct BbPullRequest {
    pub id: u64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub author: Option<BbUser>,
    #[serde(default)]
    pub source: BbEndpoint,
    #[serde(default)]
    pub destination: BbEndpoint,
    #[serde(default)]
    pub links: BbLinks,
    #[serde(default)]
    pub updated_on: Option<DateTime<Utc>>,
    #[serde(default)]
    pub created_on: Option<DateTime<Utc>>,
    #[serde(default)]
    pub participants: Vec<BbParticipant>,
}

impl BbPullRequest {
    pub fn into_summary(self, repo: &ForgeRepository) -> PullRequestSummary {
        PullRequestSummary {
            repository: repo.clone(),
            number: self.id,
            title: self.title,
            author: self.author.and_then(|author| author.label()),
            head_ref_name: self.source.branch.name.clone(),
            base_ref_name: self.destination.branch.name.clone(),
            updated_at: self.updated_on,
            url: self.links.html_href(),
            state: normalize_state(&self.state),
            is_draft: self.draft,
        }
    }

    /// Reviewer approvals, for "commits since my last review" inference.
    ///
    /// `commit_oid` is always `None`: Bitbucket does not record which commit
    /// an approval was against, so the App shows the full cumulative diff.
    pub fn review_records(&self) -> Vec<PullRequestReviewRecord> {
        self.participants
            .iter()
            .filter(|participant| participant.approved || participant.state.is_some())
            .map(|participant| PullRequestReviewRecord {
                author: participant.user.label(),
                submitted_at: participant.participated_on,
                commit_oid: None,
            })
            .collect()
    }

    /// SHAs are taken verbatim; the caller promotes Cloud's abbreviated
    /// hashes to full length before they reach a `PrSessionKey`.
    pub fn into_details(self, repo: &ForgeRepository) -> Result<PullRequestDetails> {
        let head_sha = self.source.hash();
        if head_sha.is_empty() {
            return Err(TuicrError::Forge(format!(
                "Bitbucket pull request #{} response is missing a source commit hash",
                self.id
            )));
        }
        let base_sha = self.destination.hash();
        let state = normalize_state(&self.state);
        let merged = state == "MERGED";
        Ok(PullRequestDetails {
            repository: repo.clone(),
            number: self.id,
            title: self.title,
            url: self.links.html_href(),
            is_draft: self.draft,
            author: self.author.and_then(|author| author.label()),
            head_ref_name: self.source.branch.name.clone(),
            base_ref_name: self.destination.branch.name.clone(),
            head_sha,
            base_sha,
            body: self.description,
            updated_at: self.updated_on,
            closed: state != "OPEN",
            // Cloud leaves `closed_on` null even on merged PRs, so
            // `updated_on` is the best available merge timestamp.
            merged_at: if merged { self.updated_on } else { None },
            state,
            // GitLab-only anchoring field; Bitbucket positions inline
            // comments by path plus line alone.
            diff_start_sha: None,
        })
    }
}

/// `GET /2.0/repositories/{ws}/{repo}/pullrequests/{id}/commits`, and
/// `GET /2.0/repositories/{ws}/{repo}/commit/{sha}` for SHA promotion.
#[derive(Debug, Deserialize)]
pub struct BbCommit {
    pub hash: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub date: Option<DateTime<Utc>>,
    #[serde(default)]
    pub author: Option<BbCommitAuthor>,
}

#[derive(Debug, Deserialize, Default)]
pub struct BbCommitAuthor {
    /// `"Name <email>"` form.
    #[serde(default)]
    pub raw: String,
    #[serde(default)]
    pub user: Option<BbUser>,
}

impl BbCommitAuthor {
    fn label(&self) -> String {
        if let Some(user) = self.user.as_ref()
            && let Some(label) = user.label()
        {
            return label;
        }
        // Strip the `<email>` part from the raw git author.
        match self.raw.split_once('<') {
            Some((name, _)) => name.trim().to_string(),
            None => self.raw.trim().to_string(),
        }
    }
}

impl BbCommit {
    pub fn into_pull_request_commit(self) -> PullRequestCommit {
        let short_oid: String = self.hash.chars().take(7).collect();
        let summary = self
            .message
            .lines()
            .next()
            .unwrap_or_default()
            .trim()
            .to_string();
        PullRequestCommit {
            oid: self.hash,
            short_oid,
            summary,
            author: self.author.map(|author| author.label()).unwrap_or_default(),
            timestamp: self.date,
        }
    }
}

#[derive(Debug, Deserialize, Default)]
pub struct BbCommentContent {
    #[serde(default)]
    pub raw: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct BbCommentParent {
    pub id: u64,
}

/// Anchor for an inline comment. `from` is a line in the old (base) file,
/// `to` a line in the new (head) file. `start_from` / `start_to` mark the
/// first line of a multi-line selection.
#[derive(Debug, Deserialize, Default)]
pub struct BbCommentInline {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub from: Option<u32>,
    #[serde(default)]
    pub to: Option<u32>,
    #[serde(default)]
    pub start_from: Option<u32>,
    #[serde(default)]
    pub start_to: Option<u32>,
    /// Set when the anchored line no longer exists in the current diff. Not
    /// present in every Cloud response, so it defaults to false.
    #[serde(default)]
    pub outdated: bool,
}

/// `GET /2.0/repositories/{ws}/{repo}/pullrequests/{id}/comments` — a flat
/// list; threading is expressed through `parent`.
#[derive(Debug, Deserialize)]
pub struct BbComment {
    pub id: u64,
    #[serde(default)]
    pub content: BbCommentContent,
    #[serde(default)]
    pub user: BbUser,
    #[serde(default)]
    pub created_on: Option<DateTime<Utc>>,
    #[serde(default)]
    pub deleted: bool,
    /// True for unpublished draft comments the author has not submitted yet.
    #[serde(default)]
    pub pending: bool,
    /// Non-null once the thread has been marked resolved.
    #[serde(default)]
    pub resolution: Option<serde_json::Value>,
    #[serde(default)]
    pub parent: Option<BbCommentParent>,
    /// Absent for general (non-line-anchored) comments.
    #[serde(default)]
    pub inline: Option<BbCommentInline>,
    #[serde(default)]
    pub links: BbLinks,
}

impl BbComment {
    fn into_remote_comment(self) -> RemoteReviewComment {
        RemoteReviewComment {
            id: self.id.to_string(),
            author: self.user.label(),
            body: self.content.raw,
            created_at: self.created_on,
            in_reply_to: self.parent.map(|parent| parent.id.to_string()),
            url: self.links.html_href(),
        }
    }

    fn is_renderable(&self) -> bool {
        !self.deleted && !self.content.raw.is_empty()
    }
}

/// Group a flat comment list into line-anchored threads.
///
/// Roots are comments with no `parent` and an `inline` anchor; replies attach
/// to their root by walking `parent` links, so a reply-to-a-reply still lands
/// on the correct top-level thread. General comments are excluded — they are
/// surfaced separately by [`review_summaries`].
pub fn group_into_review_threads(comments: Vec<BbComment>) -> Vec<RemoteReviewThread> {
    use std::collections::{HashMap, HashSet};

    // comment id -> root id, so nested replies resolve to a top-level thread.
    let mut root_of: HashMap<u64, u64> = HashMap::new();
    for comment in &comments {
        let root = match comment.parent.as_ref() {
            None => comment.id,
            Some(parent) => root_of.get(&parent.id).copied().unwrap_or(parent.id),
        };
        root_of.insert(comment.id, root);
    }

    // Thread order follows the roots' order in the API response.
    let mut threads: Vec<RemoteReviewThread> = Vec::new();
    let mut index_of_root: HashMap<u64, usize> = HashMap::new();
    let mut skipped_roots: HashSet<u64> = HashSet::new();

    for comment in comments {
        let root_id = root_of.get(&comment.id).copied().unwrap_or(comment.id);

        if comment.parent.is_none() {
            // A root without an inline anchor is a general comment, handled
            // by `review_summaries`; record it so replies drop out too.
            let anchored = comment
                .inline
                .as_ref()
                .filter(|inline| !inline.path.is_empty())
                .and_then(|inline| anchor(inline).map(|(line, side)| (inline, line, side)));
            let Some((inline, line, side)) = anchored else {
                skipped_roots.insert(comment.id);
                continue;
            };
            if !comment.is_renderable() {
                skipped_roots.insert(comment.id);
                continue;
            }
            let (path, is_outdated) = (inline.path.clone(), inline.outdated);
            let is_resolved = comment.resolution.is_some();
            index_of_root.insert(root_id, threads.len());
            threads.push(RemoteReviewThread {
                id: comment.id.to_string(),
                path,
                line: Some(line),
                side,
                is_resolved,
                is_outdated,
                comments: vec![comment.into_remote_comment()],
            });
        } else {
            if skipped_roots.contains(&root_id) || !comment.is_renderable() {
                continue;
            }
            if let Some(index) = index_of_root.get(&root_id).copied() {
                // A reply can carry the resolution that closed the thread.
                if comment.resolution.is_some() {
                    threads[index].is_resolved = true;
                }
                threads[index].comments.push(comment.into_remote_comment());
            }
        }
    }

    threads
}

/// General (non-inline) comments, surfaced as review-level summaries so they
/// render in the same top-of-diff area as GitHub review bodies.
///
/// Bitbucket has no review object, so every summary reports
/// `RemoteReviewState::Commented`; approvals travel through participants
/// instead — see [`BbPullRequest::review_records`].
pub fn review_summaries(comments: &[BbComment]) -> Vec<RemoteReviewSummary> {
    comments
        .iter()
        .filter(|comment| {
            comment.inline.is_none() && comment.parent.is_none() && comment.is_renderable()
        })
        .map(|comment| RemoteReviewSummary {
            id: comment.id.to_string(),
            author: comment.user.label(),
            body: comment.content.raw.clone(),
            state: RemoteReviewState::Commented,
            created_at: comment.created_on,
            url: comment.links.html_href(),
        })
        .collect()
}

/// Resolve an inline anchor to a line plus a diff side.
///
/// `to` (new file) wins when present so comments on context lines — which
/// carry both `from` and `to` — display on the head side, matching how
/// Bitbucket's own UI anchors them.
fn anchor(inline: &BbCommentInline) -> Option<(u32, RemoteCommentSide)> {
    if let Some(to) = inline.to {
        return Some((to, RemoteCommentSide::Right));
    }
    inline.from.map(|from| (from, RemoteCommentSide::Left))
}

fn normalize_state(state: &str) -> String {
    match state.to_ascii_lowercase().as_str() {
        "open" => "OPEN".to_string(),
        "merged" => "MERGED".to_string(),
        // Bitbucket declines rather than closes; report it as CLOSED so the
        // shared read-only handling treats it like the other forges.
        "declined" | "superseded" | "closed" => "CLOSED".to_string(),
        other => other.to_ascii_uppercase(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trimmed from a live `GET .../pullrequests` response.
    const PR_LIST_JSON: &str = r#"{
      "values": [{
        "id": 833,
        "title": "fix(runtime): Fixed runtime crashes",
        "description": "body text",
        "state": "OPEN",
        "draft": false,
        "created_on": "2026-07-28T16:42:45.350166+00:00",
        "updated_on": "2026-07-28T16:42:45.952608+00:00",
        "author": { "display_name": "Example User", "username": "", "uuid": "{viewer-uuid}" },
        "source": { "branch": { "name": "task/PPPAND-1614" }, "commit": { "hash": "7d9bf1fa670a" } },
        "destination": { "branch": { "name": "Release_Dev/Sprint-6" }, "commit": { "hash": "b7e0a737bb8c" } },
        "links": { "html": { "href": "https://bitbucket.org/example-workspace/repo/pull-requests/833" } }
      }],
      "next": "https://api.bitbucket.org/2.0/next-page"
    }"#;

    fn repo() -> ForgeRepository {
        ForgeRepository::bitbucket("bitbucket.org", "example-workspace", "repo")
    }

    fn first_pr(json: &str) -> BbPullRequest {
        let page: BbPaged<BbPullRequest> = serde_json::from_str(json).unwrap();
        page.values.into_iter().next().unwrap()
    }

    #[test]
    fn should_deserialize_paged_pull_request_list() {
        // given / when
        let page: BbPaged<BbPullRequest> = serde_json::from_str(PR_LIST_JSON).unwrap();
        // then
        assert!(page.has_more());
        let summary = page
            .values
            .into_iter()
            .next()
            .unwrap()
            .into_summary(&repo());
        assert_eq!(summary.number, 833);
        assert_eq!(summary.head_ref_name, "task/PPPAND-1614");
        assert_eq!(summary.base_ref_name, "Release_Dev/Sprint-6");
        assert_eq!(summary.state, "OPEN");
        assert!(!summary.is_draft);
        assert_eq!(
            summary.url,
            "https://bitbucket.org/example-workspace/repo/pull-requests/833"
        );
    }

    #[test]
    fn should_report_no_more_pages_when_next_absent() {
        // given — Cloud omits `next` on the final page
        let json = r#"{ "values": [], "page": 1, "pagelen": 10, "size": 0 }"#;
        // when
        let page: BbPaged<BbPullRequest> = serde_json::from_str(json).unwrap();
        // then
        assert!(!page.has_more());
        assert!(page.values.is_empty());
    }

    #[test]
    fn should_fall_back_to_display_name_when_username_is_blank() {
        // given — Cloud always returns an empty username
        // when
        let summary = first_pr(PR_LIST_JSON).into_summary(&repo());
        // then
        assert_eq!(summary.author.as_deref(), Some("Example User"));
    }

    #[test]
    fn should_build_details_from_pull_request_payload() {
        // given / when
        let details = first_pr(PR_LIST_JSON).into_details(&repo()).unwrap();
        // then
        assert_eq!(details.head_sha, "7d9bf1fa670a");
        assert_eq!(details.base_sha, "b7e0a737bb8c");
        assert_eq!(details.state, "OPEN");
        assert!(!details.closed);
        assert!(details.merged_at.is_none());
        assert_eq!(details.diff_start_sha, None);
        assert_eq!(details.body, "body text");
    }

    #[test]
    fn should_reject_pull_request_without_source_commit() {
        // given — no source commit to anchor the head SHA
        let json = r#"{ "id": 7, "state": "OPEN", "source": { "branch": { "name": "x" } } }"#;
        let pr: BbPullRequest = serde_json::from_str(json).unwrap();
        // when
        let err = pr.into_details(&repo()).unwrap_err();
        // then
        assert!(
            err.to_string().contains("missing a source commit hash"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn should_treat_merged_pull_request_as_read_only() {
        // given — Cloud leaves closed_on null even when merged
        let json = r#"{
          "id": 830, "state": "MERGED",
          "updated_on": "2026-07-17T15:42:15.315910+00:00",
          "source": { "branch": { "name": "f" }, "commit": { "hash": "aaaaaaaaaaaa" } },
          "destination": { "branch": { "name": "main" }, "commit": { "hash": "bbbbbbbbbbbb" } }
        }"#;
        let pr: BbPullRequest = serde_json::from_str(json).unwrap();
        // when
        let details = pr.into_details(&repo()).unwrap();
        // then
        assert_eq!(details.state, "MERGED");
        assert!(details.closed);
        assert!(details.merged_at.is_some());
        assert!(details.is_read_only());
        assert_eq!(details.read_only_reason(), Some("merged"));
    }

    #[test]
    fn should_normalize_declined_state_to_closed() {
        // given
        let json = r#"{
          "id": 1, "state": "DECLINED",
          "source": { "branch": { "name": "f" }, "commit": { "hash": "aaaaaaaaaaaa" } },
          "destination": { "branch": { "name": "main" }, "commit": { "hash": "bbbbbbbbbbbb" } }
        }"#;
        let pr: BbPullRequest = serde_json::from_str(json).unwrap();
        // when
        let details = pr.into_details(&repo()).unwrap();
        // then
        assert_eq!(details.state, "CLOSED");
        assert!(details.closed);
        assert!(details.merged_at.is_none());
        assert_eq!(details.read_only_reason(), Some("closed"));
    }

    #[test]
    fn should_extract_review_records_from_participants() {
        // given — one reviewer who approved, one who has not acted
        let json = r#"{
          "id": 1, "state": "OPEN",
          "source": { "branch": { "name": "f" }, "commit": { "hash": "aaaaaaaaaaaa" } },
          "destination": { "branch": { "name": "main" }, "commit": { "hash": "bbbbbbbbbbbb" } },
          "participants": [
            { "role": "REVIEWER", "approved": true, "state": "approved",
              "participated_on": "2026-07-17T15:42:15.315910+00:00",
              "user": { "display_name": "Reviewer One", "username": "", "uuid": "{reviewer-one-uuid}" } },
            { "role": "REVIEWER", "approved": false, "state": null, "participated_on": null,
              "user": { "display_name": "Reviewer Two", "username": "", "uuid": "{reviewer-two-uuid}" } }
          ]
        }"#;
        let pr: BbPullRequest = serde_json::from_str(json).unwrap();
        // when
        let records = pr.review_records();
        // then — only the reviewer who acted is recorded
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].author.as_deref(), Some("Reviewer One"));
        assert!(records[0].submitted_at.is_some());
        // Bitbucket does not say which commit an approval covered.
        assert_eq!(records[0].commit_oid, None);
    }

    #[test]
    fn should_convert_commit_and_strip_email_from_author() {
        // given
        let json = r#"{
          "hash": "7d9bf1fa670a02d075ee60b7e2034bced095e096",
          "date": "2026-07-28T15:46:54+00:00",
          "message": "fix(runtime): stop the crash\n\nLonger body here.\n",
          "author": { "raw": "Example User <user@example.com>" }
        }"#;
        let commit: BbCommit = serde_json::from_str(json).unwrap();
        // when
        let converted = commit.into_pull_request_commit();
        // then — summary is the subject line only, author loses the email
        assert_eq!(converted.summary, "fix(runtime): stop the crash");
        assert_eq!(converted.short_oid, "7d9bf1f");
        assert_eq!(converted.author, "Example User");
        assert!(converted.timestamp.is_some());
    }

    #[test]
    fn should_prefer_linked_user_over_raw_commit_author() {
        // given
        let json = r#"{
          "hash": "abc1234567890000000000000000000000000000",
          "message": "subject",
          "author": {
            "raw": "example-user <user@example.com>",
            "user": { "display_name": "Example User", "username": "", "uuid": "{viewer-uuid}" }
          }
        }"#;
        let commit: BbCommit = serde_json::from_str(json).unwrap();
        // when / then
        assert_eq!(commit.into_pull_request_commit().author, "Example User");
    }

    /// Trimmed from a live `GET .../pullrequests/830/comments` response: a
    /// root inline comment plus one reply on the same line.
    const COMMENTS_JSON: &str = r#"{ "values": [
      {
        "id": 828148600,
        "content": { "raw": "is this new varaible needed?" },
        "user": { "display_name": "Example User", "username": "", "uuid": "{viewer-uuid}" },
        "created_on": "2026-07-17T15:09:56.632702+00:00",
        "deleted": false, "pending": false, "resolution": null, "parent": null,
        "inline": { "path": "app/src/Home.kt", "from": null, "to": 502,
                    "start_from": null, "start_to": null },
        "links": { "html": { "href": "https://bitbucket.org/x/y/pull-requests/830/_/diff#comment-828148600" } }
      },
      {
        "id": 828160388,
        "content": { "raw": "removed it" },
        "user": { "display_name": "Reviewer Two", "username": "", "uuid": "{reviewer-two-uuid}" },
        "created_on": "2026-07-17T15:42:15.315910+00:00",
        "deleted": false, "pending": false, "resolution": null,
        "parent": { "id": 828148600 },
        "inline": { "path": "app/src/Home.kt", "from": null, "to": 502 },
        "links": { "html": { "href": "https://bitbucket.org/x/y/pull-requests/830/_/diff#comment-828160388" } }
      }
    ] }"#;

    fn comments_of(json: &str) -> Vec<BbComment> {
        let page: BbPaged<BbComment> = serde_json::from_str(json).unwrap();
        page.values
    }

    #[test]
    fn should_group_root_and_reply_into_one_thread() {
        // given / when
        let threads = group_into_review_threads(comments_of(COMMENTS_JSON));
        // then
        assert_eq!(threads.len(), 1);
        let thread = &threads[0];
        assert_eq!(thread.id, "828148600");
        assert_eq!(thread.path, "app/src/Home.kt");
        assert_eq!(thread.line, Some(502));
        assert_eq!(thread.side, RemoteCommentSide::Right);
        assert!(!thread.is_resolved);
        assert!(!thread.is_outdated);
        assert_eq!(thread.comments.len(), 2);
        assert_eq!(
            thread.root().unwrap().author.as_deref(),
            Some("Example User")
        );
        assert_eq!(thread.replies().count(), 1);
        assert_eq!(thread.comments[1].in_reply_to.as_deref(), Some("828148600"));
        assert!(thread.comments[0].url.ends_with("#comment-828148600"));
    }

    #[test]
    fn should_anchor_to_left_side_when_only_from_is_set() {
        // given — a comment on a deleted line
        let json = r#"{ "values": [{
          "id": 1, "content": { "raw": "why remove this?" },
          "user": { "display_name": "A" }, "parent": null,
          "inline": { "path": "src/lib.rs", "from": 12, "to": null }
        }] }"#;
        // when
        let threads = group_into_review_threads(comments_of(json));
        // then
        assert_eq!(threads[0].side, RemoteCommentSide::Left);
        assert_eq!(threads[0].line, Some(12));
    }

    #[test]
    fn should_anchor_context_line_comment_to_head_side() {
        // given — context lines carry both from and to
        let json = r#"{ "values": [{
          "id": 1, "content": { "raw": "note" },
          "user": { "display_name": "A" }, "parent": null,
          "inline": { "path": "src/lib.rs", "from": 10, "to": 14 }
        }] }"#;
        // when
        let threads = group_into_review_threads(comments_of(json));
        // then — `to` wins, matching Bitbucket's own rendering
        assert_eq!(threads[0].side, RemoteCommentSide::Right);
        assert_eq!(threads[0].line, Some(14));
    }

    #[test]
    fn should_mark_thread_resolved_when_a_reply_carries_the_resolution() {
        // given
        let json = r#"{ "values": [
          { "id": 1, "content": { "raw": "root" }, "user": { "display_name": "A" },
            "parent": null, "inline": { "path": "a.rs", "to": 3 } },
          { "id": 2, "content": { "raw": "fixed" }, "user": { "display_name": "B" },
            "parent": { "id": 1 }, "inline": { "path": "a.rs", "to": 3 },
            "resolution": { "type": "resolution" } }
        ] }"#;
        // when
        let threads = group_into_review_threads(comments_of(json));
        // then
        assert_eq!(threads.len(), 1);
        assert!(threads[0].is_resolved);
        assert!(!threads[0].is_active());
    }

    #[test]
    fn should_attach_nested_replies_to_the_top_level_thread() {
        // given — a reply to a reply
        let json = r#"{ "values": [
          { "id": 1, "content": { "raw": "root" }, "user": { "display_name": "A" },
            "parent": null, "inline": { "path": "a.rs", "to": 3 } },
          { "id": 2, "content": { "raw": "reply" }, "user": { "display_name": "B" },
            "parent": { "id": 1 }, "inline": { "path": "a.rs", "to": 3 } },
          { "id": 3, "content": { "raw": "reply to reply" }, "user": { "display_name": "C" },
            "parent": { "id": 2 }, "inline": { "path": "a.rs", "to": 3 } }
        ] }"#;
        // when
        let threads = group_into_review_threads(comments_of(json));
        // then — one thread holding all three, not a second root
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].comments.len(), 3);
    }

    #[test]
    fn should_skip_deleted_comments_and_their_replies() {
        // given
        let json = r#"{ "values": [
          { "id": 1, "content": { "raw": "gone" }, "user": { "display_name": "A" },
            "parent": null, "deleted": true, "inline": { "path": "a.rs", "to": 3 } },
          { "id": 2, "content": { "raw": "orphan reply" }, "user": { "display_name": "B" },
            "parent": { "id": 1 }, "inline": { "path": "a.rs", "to": 3 } },
          { "id": 3, "content": { "raw": "kept" }, "user": { "display_name": "C" },
            "parent": null, "inline": { "path": "b.rs", "to": 9 } }
        ] }"#;
        // when
        let threads = group_into_review_threads(comments_of(json));
        // then — the deleted root and its orphaned reply both drop out
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].path, "b.rs");
    }

    #[test]
    fn should_exclude_general_comments_from_threads() {
        // given — one general comment, one inline
        let json = r#"{ "values": [
          { "id": 1, "content": { "raw": "LGTM overall" }, "user": { "display_name": "A" },
            "parent": null },
          { "id": 2, "content": { "raw": "nit" }, "user": { "display_name": "B" },
            "parent": null, "inline": { "path": "a.rs", "to": 3 } }
        ] }"#;
        // when
        let threads = group_into_review_threads(comments_of(json));
        // then
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].id, "2");
    }

    #[test]
    fn should_surface_general_comments_as_review_summaries() {
        // given
        let json = r#"{ "values": [
          { "id": 1, "content": { "raw": "LGTM overall" }, "user": { "display_name": "Alice" },
            "created_on": "2026-07-17T15:09:56.632702+00:00", "parent": null,
            "links": { "html": { "href": "https://bitbucket.org/x/y/pull-requests/1#comment-1" } } },
          { "id": 2, "content": { "raw": "nit" }, "user": { "display_name": "B" },
            "parent": null, "inline": { "path": "a.rs", "to": 3 } },
          { "id": 3, "content": { "raw": "deleted general" }, "user": { "display_name": "C" },
            "parent": null, "deleted": true }
        ] }"#;
        // when
        let summaries = review_summaries(&comments_of(json));
        // then — only the live general comment
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].id, "1");
        assert_eq!(summaries[0].body, "LGTM overall");
        assert_eq!(summaries[0].author.as_deref(), Some("Alice"));
        assert_eq!(summaries[0].state, RemoteReviewState::Commented);
    }

    #[test]
    fn should_mark_thread_outdated_when_inline_says_so() {
        // given
        let json = r#"{ "values": [{
          "id": 1, "content": { "raw": "stale" }, "user": { "display_name": "A" },
          "parent": null, "inline": { "path": "a.rs", "to": 3, "outdated": true }
        }] }"#;
        // when
        let threads = group_into_review_threads(comments_of(json));
        // then
        assert!(threads[0].is_outdated);
        assert!(!threads[0].is_active());
    }

    #[test]
    fn should_skip_inline_comment_without_a_resolvable_anchor() {
        // given — file-level comment with neither from nor to
        let json = r#"{ "values": [{
          "id": 1, "content": { "raw": "whole file" }, "user": { "display_name": "A" },
          "parent": null, "inline": { "path": "a.rs", "from": null, "to": null }
        }] }"#;
        // when / then
        assert!(group_into_review_threads(comments_of(json)).is_empty());
    }

    #[test]
    fn should_preserve_api_order_across_threads() {
        // given — two roots, replies interleaved
        let json = r#"{ "values": [
          { "id": 1, "content": { "raw": "first root" }, "user": { "display_name": "A" },
            "parent": null, "inline": { "path": "a.rs", "to": 3 } },
          { "id": 2, "content": { "raw": "second root" }, "user": { "display_name": "B" },
            "parent": null, "inline": { "path": "b.rs", "to": 9 } },
          { "id": 3, "content": { "raw": "reply to first" }, "user": { "display_name": "C" },
            "parent": { "id": 1 }, "inline": { "path": "a.rs", "to": 3 } }
        ] }"#;
        // when
        let threads = group_into_review_threads(comments_of(json));
        // then
        assert_eq!(threads.len(), 2);
        assert_eq!(threads[0].id, "1");
        assert_eq!(threads[0].comments.len(), 2);
        assert_eq!(threads[1].id, "2");
        assert_eq!(threads[1].comments.len(), 1);
    }

    #[test]
    fn diffstat_metadata_preserves_renamed_unicode_paths() {
        let row: BbDiffStat = serde_json::from_str(
            r#"{
              "status":"renamed",
              "old":{"path":"旧 b/left and side.txt"},
              "new":{"path":"新 b/right and side.txt"}
            }"#,
        )
        .unwrap();
        let metadata = row.into_metadata().unwrap();

        assert_eq!(metadata.status, FileStatus::Renamed);
        assert_eq!(
            metadata.old_path.as_deref(),
            Some(std::path::Path::new("旧 b/left and side.txt"))
        );
        assert_eq!(
            metadata.new_path.as_deref(),
            Some(std::path::Path::new("新 b/right and side.txt"))
        );
    }
}
