//! Azure DevOps REST JSON structs and their mapping into tuicr's
//! forge-agnostic trait types.
//!
//! Modeled on `src/forge/gitlab/models.rs`. Azure wraps list responses in a
//! `{ count, value: [...] }` envelope ([`AzList`]); a single PR / commit is
//! returned bare.

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::forge::remote_comments::{RemoteCommentSide, RemoteReviewComment, RemoteReviewThread};
use crate::forge::traits::{
    ForgeRepository, PullRequestCommit, PullRequestDetails, PullRequestSummary,
};

/// Azure DevOps list envelope: `{ "count": N, "value": [ ... ] }`.
///
/// `value` is intentionally not `#[serde(default)]`: a bare field default on a
/// generic `Vec<T>` field forces serde to add a `T: Default` bound to the
/// derived `Deserialize` impl. Azure always returns `value`, so a plain
/// required field keeps the generic bound-free.
#[derive(Debug, Deserialize)]
pub struct AzList<T> {
    pub value: Vec<T>,
}

/// A commit pointer, e.g. `lastMergeSourceCommit`.
#[derive(Debug, Deserialize, Default)]
pub struct AzCommitRef {
    #[serde(default, rename = "commitId")]
    pub commit_id: String,
}

/// An Azure DevOps identity (author, reviewer, authenticated user).
#[derive(Debug, Deserialize, Default)]
pub struct AzIdentity {
    #[serde(default)]
    pub id: String,
    #[serde(default, rename = "displayName")]
    pub display_name: String,
    #[serde(default, rename = "uniqueName")]
    pub unique_name: String,
}

impl AzIdentity {
    /// Best display handle: prefer the friendly name, fall back to the unique
    /// name (usually an email / UPN).
    fn handle(&self) -> Option<String> {
        if !self.display_name.is_empty() {
            Some(self.display_name.clone())
        } else if !self.unique_name.is_empty() {
            Some(self.unique_name.clone())
        } else {
            None
        }
    }
}

#[derive(Debug, Deserialize, Default)]
pub struct AzLink {
    #[serde(default)]
    pub href: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct AzLinks {
    #[serde(default)]
    pub web: Option<AzLink>,
}

/// A pull request as returned by both `GET .../pullrequests/{id}` (details) and
/// `GET .../pullrequests` (each element of the list). One struct serves both.
#[derive(Debug, Deserialize)]
pub struct AzPullRequest {
    #[serde(rename = "pullRequestId")]
    pub pull_request_id: u64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// `active` | `completed` | `abandoned` | `notSet`.
    #[serde(default)]
    pub status: String,
    #[serde(default, rename = "isDraft")]
    pub is_draft: bool,
    #[serde(default, rename = "sourceRefName")]
    pub source_ref_name: String,
    #[serde(default, rename = "targetRefName")]
    pub target_ref_name: String,
    #[serde(default, rename = "lastMergeSourceCommit")]
    pub last_merge_source_commit: Option<AzCommitRef>,
    #[serde(default, rename = "lastMergeTargetCommit")]
    pub last_merge_target_commit: Option<AzCommitRef>,
    #[serde(default, rename = "createdBy")]
    pub created_by: Option<AzIdentity>,
    #[serde(default, rename = "creationDate")]
    pub creation_date: Option<DateTime<Utc>>,
    #[serde(default, rename = "closedDate")]
    pub closed_date: Option<DateTime<Utc>>,
    #[serde(default, rename = "_links")]
    pub links: Option<AzLinks>,
}

impl AzPullRequest {
    fn web_url(&self, repo: &ForgeRepository) -> String {
        if let Some(href) = self
            .links
            .as_ref()
            .and_then(|l| l.web.as_ref())
            .map(|w| w.href.clone())
            .filter(|href| !href.is_empty())
        {
            return href;
        }
        // Fallback: reconstruct the web URL. `owner` is `org/project`, so this
        // yields e.g. https://dev.azure.com/org/project/_git/repo/pullrequest/5
        format!(
            "https://{}/{}/_git/{}/pullrequest/{}",
            repo.host, repo.owner, repo.name, self.pull_request_id
        )
    }

    fn author_handle(&self) -> Option<String> {
        self.created_by.as_ref().and_then(AzIdentity::handle)
    }

    pub fn into_summary(self, repo: &ForgeRepository) -> PullRequestSummary {
        let url = self.web_url(repo);
        let author = self.author_handle();
        PullRequestSummary {
            repository: repo.clone(),
            number: self.pull_request_id,
            title: self.title,
            author,
            head_ref_name: strip_ref(&self.source_ref_name),
            base_ref_name: strip_ref(&self.target_ref_name),
            updated_at: self.creation_date,
            url,
            state: normalize_state(&self.status),
            is_draft: self.is_draft,
        }
    }

    pub fn into_details(self, repo: &ForgeRepository) -> PullRequestDetails {
        let url = self.web_url(repo);
        let author = self.author_handle();
        let head_sha = self
            .last_merge_source_commit
            .as_ref()
            .map(|c| c.commit_id.clone())
            .unwrap_or_default();
        let base_sha = self
            .last_merge_target_commit
            .as_ref()
            .map(|c| c.commit_id.clone())
            .unwrap_or_default();
        let status = self.status.to_ascii_lowercase();
        // Azure "completed" == merged; "abandoned" == closed without merge.
        let merged_at = if status == "completed" {
            self.closed_date
        } else {
            None
        };
        let closed = status == "abandoned";
        PullRequestDetails {
            repository: repo.clone(),
            number: self.pull_request_id,
            title: self.title,
            url,
            state: normalize_state(&self.status),
            is_draft: self.is_draft,
            author,
            head_ref_name: strip_ref(&self.source_ref_name),
            base_ref_name: strip_ref(&self.target_ref_name),
            head_sha,
            base_sha,
            body: self.description,
            updated_at: self.creation_date,
            closed,
            merged_at,
            diff_start_sha: None,
        }
    }
}

/// A commit on a PR, from `GET .../pullRequests/{id}/commits`.
#[derive(Debug, Deserialize)]
pub struct AzGitCommitRef {
    #[serde(rename = "commitId")]
    pub commit_id: String,
    #[serde(default)]
    pub comment: String,
    #[serde(default)]
    pub author: Option<AzGitUserDate>,
}

#[derive(Debug, Deserialize, Default)]
pub struct AzGitUserDate {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub date: Option<DateTime<Utc>>,
}

impl AzGitCommitRef {
    pub fn into_pull_request_commit(self) -> PullRequestCommit {
        let short_oid = self.commit_id.chars().take(8).collect();
        let summary = self.comment.lines().next().unwrap_or_default().to_string();
        let (author, timestamp) = match self.author {
            Some(a) => (a.name, a.date),
            None => (String::new(), None),
        };
        PullRequestCommit {
            oid: self.commit_id,
            short_oid,
            summary,
            author,
            timestamp,
        }
    }
}

/// `GET .../connectionData` — used to resolve the authenticated user's id for
/// casting an approve/reject vote.
#[derive(Debug, Deserialize, Default)]
pub struct AzConnectionData {
    #[serde(default, rename = "authenticatedUser")]
    pub authenticated_user: Option<AzIdentity>,
}

/// Response from creating a comment thread; we only need the numeric id.
#[derive(Debug, Deserialize, Default)]
pub struct AzThreadResponse {
    #[serde(default)]
    pub id: u64,
}

/// A single-side file position within a thread context (`{ line, offset }`).
#[derive(Debug, Deserialize, Default)]
pub struct AzFilePosition {
    #[serde(default)]
    pub line: u32,
}

/// Where a comment thread is anchored in the diff. Absent/empty `file_path`
/// means a PR-level (non-file) thread.
#[derive(Debug, Deserialize, Default)]
pub struct AzThreadContext {
    #[serde(default, rename = "filePath")]
    pub file_path: String,
    #[serde(default, rename = "rightFileStart")]
    pub right_file_start: Option<AzFilePosition>,
    #[serde(default, rename = "leftFileStart")]
    pub left_file_start: Option<AzFilePosition>,
}

/// One comment within a thread.
#[derive(Debug, Deserialize)]
pub struct AzComment {
    #[serde(default)]
    pub id: u64,
    #[serde(default, rename = "parentCommentId")]
    pub parent_comment_id: u64,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub author: Option<AzIdentity>,
    #[serde(default, rename = "publishedDate")]
    pub published_date: Option<DateTime<Utc>>,
    /// `text` | `system` | `codeChange`. We drop `system`.
    #[serde(default, rename = "commentType")]
    pub comment_type: String,
    #[serde(default, rename = "isDeleted")]
    pub is_deleted: bool,
}

/// A pull request comment thread, from `GET .../pullRequests/{id}/threads`.
#[derive(Debug, Deserialize)]
pub struct AzThread {
    #[serde(default)]
    pub id: u64,
    /// `active` | `fixed` | `wontFix` | `closed` | `byDesign` | `pending` | "".
    #[serde(default)]
    pub status: String,
    #[serde(default, rename = "isDeleted")]
    pub is_deleted: bool,
    #[serde(default, rename = "threadContext")]
    pub thread_context: Option<AzThreadContext>,
    #[serde(default)]
    pub comments: Vec<AzComment>,
}

impl AzThread {
    /// Map to a display thread, or `None` when the thread carries no
    /// human-authored content (Azure emits many `system` threads for pushes,
    /// policy updates, votes, etc.).
    pub fn into_review_thread(self) -> Option<RemoteReviewThread> {
        if self.is_deleted {
            return None;
        }
        let is_resolved = thread_status_resolved(&self.status);
        let comments: Vec<RemoteReviewComment> = self
            .comments
            .into_iter()
            .filter(|c| {
                !c.is_deleted
                    && !c.content.is_empty()
                    && !c.comment_type.eq_ignore_ascii_case("system")
            })
            .map(|c| RemoteReviewComment {
                id: c.id.to_string(),
                author: c.author.and_then(|a| a.handle()),
                body: c.content,
                created_at: c.published_date,
                in_reply_to: (c.parent_comment_id != 0).then(|| c.parent_comment_id.to_string()),
                url: String::new(),
            })
            .collect();
        if comments.is_empty() {
            return None;
        }

        let ctx = self.thread_context.unwrap_or_default();
        let file_path = ctx.file_path.trim_start_matches('/').to_string();
        let (path, line, side) = if file_path.is_empty() {
            // PR-level (conversation) comment — no diff anchor.
            (String::new(), None, RemoteCommentSide::Right)
        } else if let Some(pos) = ctx.right_file_start {
            (file_path, line_of(&pos), RemoteCommentSide::Right)
        } else if let Some(pos) = ctx.left_file_start {
            (file_path, line_of(&pos), RemoteCommentSide::Left)
        } else {
            (file_path, None, RemoteCommentSide::Right)
        };

        Some(RemoteReviewThread {
            id: self.id.to_string(),
            path,
            line,
            side,
            is_resolved,
            is_outdated: false,
            comments,
        })
    }
}

fn line_of(pos: &AzFilePosition) -> Option<u32> {
    (pos.line != 0).then_some(pos.line)
}

/// Azure thread statuses that mean "resolved" (as opposed to `active`/`pending`).
fn thread_status_resolved(status: &str) -> bool {
    matches!(
        status.to_ascii_lowercase().as_str(),
        "fixed" | "closed" | "wontfix" | "bydesign"
    )
}

/// Strip the `refs/heads/` prefix from a ref name; leave other refs intact.
fn strip_ref(ref_name: &str) -> String {
    ref_name
        .strip_prefix("refs/heads/")
        .unwrap_or(ref_name)
        .to_string()
}

/// Map Azure's PR status to tuicr's normalized state string.
fn normalize_state(status: &str) -> String {
    match status.to_ascii_lowercase().as_str() {
        "active" => "OPEN".to_string(),
        "completed" => "MERGED".to_string(),
        "abandoned" => "CLOSED".to_string(),
        other => other.to_ascii_uppercase(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::traits::ForgeRepository;

    fn azure_repo() -> ForgeRepository {
        ForgeRepository::azure("dev.azure.com", "myorg/myproject", "myrepo")
    }

    #[test]
    fn should_map_active_pr_into_open_details() {
        let json = r#"{
            "pullRequestId": 42,
            "title": "Add feature",
            "description": "body text",
            "status": "active",
            "isDraft": false,
            "sourceRefName": "refs/heads/feature",
            "targetRefName": "refs/heads/main",
            "lastMergeSourceCommit": { "commitId": "head111" },
            "lastMergeTargetCommit": { "commitId": "base000" },
            "createdBy": { "displayName": "Alice", "uniqueName": "alice@example.com", "id": "id-1" },
            "creationDate": "2026-01-01T00:00:00Z"
        }"#;
        let pr: AzPullRequest = serde_json::from_str(json).unwrap();
        let details = pr.into_details(&azure_repo());
        assert_eq!(details.number, 42);
        assert_eq!(details.state, "OPEN");
        assert_eq!(details.head_sha, "head111");
        assert_eq!(details.base_sha, "base000");
        assert_eq!(details.head_ref_name, "feature");
        assert_eq!(details.base_ref_name, "main");
        assert_eq!(details.author.as_deref(), Some("Alice"));
        assert!(!details.closed);
        assert!(details.merged_at.is_none());
        assert!(!details.is_read_only());
        assert_eq!(
            details.url,
            "https://dev.azure.com/myorg/myproject/_git/myrepo/pullrequest/42"
        );
    }

    #[test]
    fn should_mark_completed_pr_merged_and_read_only() {
        let json = r#"{
            "pullRequestId": 7,
            "title": "Done",
            "status": "completed",
            "sourceRefName": "refs/heads/feature",
            "targetRefName": "refs/heads/main",
            "closedDate": "2026-02-02T00:00:00Z"
        }"#;
        let pr: AzPullRequest = serde_json::from_str(json).unwrap();
        let details = pr.into_details(&azure_repo());
        assert_eq!(details.state, "MERGED");
        assert!(details.merged_at.is_some());
        assert!(!details.closed);
        assert!(details.is_read_only());
    }

    #[test]
    fn should_mark_abandoned_pr_closed_and_read_only() {
        let json = r#"{
            "pullRequestId": 8,
            "title": "Nope",
            "status": "abandoned",
            "sourceRefName": "refs/heads/feature",
            "targetRefName": "refs/heads/main",
            "closedDate": "2026-02-02T00:00:00Z"
        }"#;
        let pr: AzPullRequest = serde_json::from_str(json).unwrap();
        let details = pr.into_details(&azure_repo());
        assert_eq!(details.state, "CLOSED");
        assert!(details.closed);
        assert!(details.merged_at.is_none());
        assert!(details.is_read_only());
    }

    #[test]
    fn should_prefer_web_link_when_present() {
        let json = r#"{
            "pullRequestId": 9,
            "title": "x",
            "status": "active",
            "_links": { "web": { "href": "https://dev.azure.com/o/p/_git/r/pullrequest/9" } }
        }"#;
        let pr: AzPullRequest = serde_json::from_str(json).unwrap();
        let summary = pr.into_summary(&azure_repo());
        assert_eq!(summary.url, "https://dev.azure.com/o/p/_git/r/pullrequest/9");
        assert_eq!(summary.state, "OPEN");
    }

    #[test]
    fn should_parse_pr_list_envelope() {
        let json = r#"{ "count": 1, "value": [ { "pullRequestId": 1, "title": "t", "status": "active" } ] }"#;
        let list: AzList<AzPullRequest> = serde_json::from_str(json).unwrap();
        assert_eq!(list.value.len(), 1);
        assert_eq!(list.value[0].pull_request_id, 1);
    }

    #[test]
    fn should_map_file_anchored_thread_with_reply() {
        let json = r#"{
            "id": 14170,
            "status": "active",
            "threadContext": {
                "filePath": "/src/lib.rs",
                "rightFileStart": { "line": 66, "offset": 1 },
                "rightFileEnd": { "line": 66, "offset": 1 }
            },
            "comments": [
                { "id": 1, "parentCommentId": 0, "commentType": "text", "content": "NIT: rename?", "author": { "displayName": "Josh Vito" }, "publishedDate": "2026-08-04T00:00:00Z" },
                { "id": 2, "parentCommentId": 1, "commentType": "text", "content": "done", "author": { "displayName": "Rebecca Wall" } }
            ]
        }"#;
        let thread: AzThread = serde_json::from_str(json).unwrap();
        let mapped = thread.into_review_thread().unwrap();
        assert_eq!(mapped.id, "14170");
        assert_eq!(mapped.path, "src/lib.rs");
        assert_eq!(mapped.line, Some(66));
        assert_eq!(mapped.side, RemoteCommentSide::Right);
        assert!(!mapped.is_resolved);
        assert_eq!(mapped.comments.len(), 2);
        assert_eq!(mapped.comments[0].author.as_deref(), Some("Josh Vito"));
        assert_eq!(mapped.comments[0].in_reply_to, None);
        assert_eq!(mapped.comments[1].in_reply_to.as_deref(), Some("1"));
    }

    #[test]
    fn should_map_pr_level_thread_without_file_context() {
        let json = r#"{
            "id": 14120,
            "status": "fixed",
            "comments": [
                { "id": 1, "parentCommentId": 0, "commentType": "text", "content": "general note", "author": { "displayName": "Greg Awarski" } }
            ]
        }"#;
        let thread: AzThread = serde_json::from_str(json).unwrap();
        let mapped = thread.into_review_thread().unwrap();
        assert_eq!(mapped.path, "");
        assert_eq!(mapped.line, None);
        assert!(mapped.is_resolved);
    }

    #[test]
    fn should_skip_system_threads() {
        let json = r#"{
            "id": 14063,
            "status": "",
            "comments": [
                { "id": 1, "parentCommentId": 0, "commentType": "system", "content": "The reference refs/heads/x was updated.", "author": { "displayName": "TFS" } }
            ]
        }"#;
        let thread: AzThread = serde_json::from_str(json).unwrap();
        assert!(thread.into_review_thread().is_none());
    }

    #[test]
    fn should_skip_deleted_thread() {
        let json = r#"{ "id": 1, "status": "active", "isDeleted": true, "comments": [] }"#;
        let thread: AzThread = serde_json::from_str(json).unwrap();
        assert!(thread.into_review_thread().is_none());
    }

    #[test]
    fn should_map_commit_into_pull_request_commit() {
        let json = r#"{
            "commitId": "abcdef1234567890",
            "comment": "First line\n\nbody",
            "author": { "name": "Bob", "date": "2026-01-01T00:00:00Z" }
        }"#;
        let commit: AzGitCommitRef = serde_json::from_str(json).unwrap();
        let prc = commit.into_pull_request_commit();
        assert_eq!(prc.oid, "abcdef1234567890");
        assert_eq!(prc.short_oid, "abcdef12");
        assert_eq!(prc.summary, "First line");
        assert_eq!(prc.author, "Bob");
        assert!(prc.timestamp.is_some());
    }
}
