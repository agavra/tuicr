//! Gerrit REST JSON structs and their mapping into tuicr's forge-agnostic
//! trait types.
//!
//! Modeled on `src/forge/azure/models.rs`. Two Gerrit quirks shape this file:
//!
//! - Timestamps are `"2013-02-21 11:16:36.775000000"` — space-separated, UTC,
//!   no zone suffix — so they need [`GerritTimestamp`] rather than serde's
//!   RFC 3339 handling for `DateTime<Utc>`.
//! - A change's revisions arrive as a map keyed by commit SHA, so the current
//!   revision is looked up through the change's `current_revision` field.

use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Deserializer};

use crate::forge::remote_comments::{RemoteCommentSide, RemoteReviewComment, RemoteReviewThread};
use crate::forge::traits::{
    ForgeRepository, PullRequestCommit, PullRequestDetails, PullRequestSummary,
};

/// Gerrit's timestamp format: UTC, space-separated, nanosecond precision.
const TIMESTAMP_FORMAT: &str = "%Y-%m-%d %H:%M:%S%.f";

/// A Gerrit timestamp. Newtype because Gerrit does not emit RFC 3339.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GerritTimestamp(pub DateTime<Utc>);

impl<'de> Deserialize<'de> for GerritTimestamp {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        let naive = NaiveDateTime::parse_from_str(raw.trim(), TIMESTAMP_FORMAT)
            .map_err(serde::de::Error::custom)?;
        Ok(GerritTimestamp(naive.and_utc()))
    }
}

fn at(value: &Option<GerritTimestamp>) -> Option<DateTime<Utc>> {
    value.map(|t| t.0)
}

/// A Gerrit account. `DETAILED_ACCOUNTS` fills in the name fields; without it
/// only `_account_id` is present.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct GerritAccount {
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
}

impl GerritAccount {
    /// Best display handle: the login, then the display name, then the email.
    fn handle(&self) -> Option<String> {
        [&self.username, &self.name, &self.email]
            .into_iter()
            .flatten()
            .find(|value| !value.is_empty())
            .cloned()
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct GerritParent {
    #[serde(default)]
    pub commit: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct GerritCommit {
    #[serde(default)]
    pub parents: Vec<GerritParent>,
    #[serde(default)]
    pub message: String,
}

/// One patch set of a change. `commit` is only populated with the
/// `CURRENT_COMMIT` option.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct GerritRevision {
    #[serde(default, rename = "_number")]
    pub number: u64,
    #[serde(default, rename = "ref")]
    pub ref_name: String,
    #[serde(default)]
    pub commit: Option<GerritCommit>,
}

/// A change, as returned by both `GET /changes/` (list) and
/// `GET /changes/{id}` (details). One struct serves both.
#[derive(Debug, Clone, Deserialize)]
pub struct GerritChange {
    #[serde(rename = "_number")]
    pub number: u64,
    #[serde(default)]
    pub project: String,
    #[serde(default)]
    pub branch: String,
    #[serde(default)]
    pub subject: String,
    /// `NEW` | `MERGED` | `ABANDONED`.
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub work_in_progress: bool,
    #[serde(default)]
    pub owner: Option<GerritAccount>,
    #[serde(default)]
    pub updated: Option<GerritTimestamp>,
    #[serde(default)]
    pub submitted: Option<GerritTimestamp>,
    #[serde(default)]
    pub current_revision: Option<String>,
    #[serde(default)]
    pub revisions: BTreeMap<String, GerritRevision>,
}

impl GerritChange {
    /// The current revision's SHA and metadata, when the request asked for
    /// `CURRENT_REVISION`.
    fn current(&self) -> Option<(&String, &GerritRevision)> {
        let sha = self.current_revision.as_ref()?;
        self.revisions.get(sha).map(|revision| (sha, revision))
    }

    /// The patch set ref (`refs/changes/65/3965/2`) of the current revision.
    /// Falls back to the change's own ref shape when revisions were not
    /// requested.
    fn head_ref(&self) -> String {
        self.current()
            .map(|(_, revision)| revision.ref_name.clone())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| format!("refs/changes/{:02}/{}", self.number % 100, self.number))
    }

    fn author_handle(&self) -> Option<String> {
        self.owner.as_ref().and_then(GerritAccount::handle)
    }

    /// Web URL of the change: `https://<host>/c/<project>/+/<number>`.
    pub fn web_url(&self, web_base: &str) -> String {
        format!(
            "{web_base}/c/{}/+/{}",
            if self.project.is_empty() {
                "-"
            } else {
                self.project.as_str()
            },
            self.number
        )
    }

    pub fn into_summary(self, repo: &ForgeRepository, web_base: &str) -> PullRequestSummary {
        let url = self.web_url(web_base);
        let author = self.author_handle();
        let head_ref_name = self.head_ref();
        PullRequestSummary {
            repository: repo.clone(),
            number: self.number,
            title: self.subject,
            author,
            head_ref_name,
            base_ref_name: self.branch,
            updated_at: at(&self.updated),
            url,
            state: normalize_state(&self.status),
            is_draft: self.work_in_progress,
        }
    }

    pub fn into_details(self, repo: &ForgeRepository, web_base: &str) -> PullRequestDetails {
        let url = self.web_url(web_base);
        let author = self.author_handle();
        let head_ref_name = self.head_ref();
        let (head_sha, base_sha, body) = match self.current() {
            Some((sha, revision)) => {
                let commit = revision.commit.as_ref();
                let base = commit
                    .and_then(|c| c.parents.first())
                    .map(|parent| parent.commit.clone())
                    .unwrap_or_default();
                // A Gerrit change is one commit, so its body is the commit
                // message minus the subject line.
                let body = commit.map(|c| commit_body(&c.message)).unwrap_or_default();
                (sha.clone(), base, body)
            }
            None => (String::new(), String::new(), String::new()),
        };
        let status = self.status.to_ascii_uppercase();
        let merged_at = (status == "MERGED").then(|| at(&self.submitted)).flatten();
        PullRequestDetails {
            repository: repo.clone(),
            number: self.number,
            title: self.subject,
            url,
            state: normalize_state(&self.status),
            is_draft: self.work_in_progress,
            author,
            head_ref_name,
            base_ref_name: self.branch,
            head_sha,
            base_sha,
            body,
            updated_at: at(&self.updated),
            closed: status == "ABANDONED",
            merged_at,
            diff_start_sha: None,
        }
    }

    /// The current revision as the change's single commit. Gerrit changes are
    /// one commit per patch set, so this list never has more than one entry.
    pub fn into_commits(self) -> Vec<PullRequestCommit> {
        let author = self.author_handle().unwrap_or_default();
        let timestamp = at(&self.updated);
        let summary = self.subject.clone();
        match self.current() {
            Some((sha, _)) => vec![PullRequestCommit {
                oid: sha.clone(),
                short_oid: sha.chars().take(8).collect(),
                summary,
                author,
                timestamp,
            }],
            None => Vec::new(),
        }
    }
}

/// Everything after the subject line of a commit message.
fn commit_body(message: &str) -> String {
    message
        .split_once('\n')
        .map(|(_, rest)| rest.trim_start_matches('\n').trim_end().to_string())
        .unwrap_or_default()
}

/// Map Gerrit change status onto the state vocabulary the UI shares across
/// forges.
fn normalize_state(status: &str) -> String {
    match status.to_ascii_uppercase().as_str() {
        "NEW" => "OPEN".to_string(),
        "ABANDONED" => "CLOSED".to_string(),
        other => other.to_string(),
    }
}

/// One inline comment. Gerrit returns these grouped by file path.
#[derive(Debug, Clone, Deserialize)]
pub struct GerritComment {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub line: Option<u32>,
    #[serde(default)]
    pub in_reply_to: Option<String>,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub updated: Option<GerritTimestamp>,
    #[serde(default)]
    pub author: Option<GerritAccount>,
    #[serde(default)]
    pub unresolved: bool,
    #[serde(default)]
    pub patch_set: u32,
    /// `PARENT` (base side) or `REVISION` (head side, the default).
    #[serde(default)]
    pub side: Option<String>,
}

impl GerritComment {
    fn side(&self) -> RemoteCommentSide {
        match self.side.as_deref() {
            Some("PARENT") => RemoteCommentSide::Left,
            _ => RemoteCommentSide::Right,
        }
    }

    /// `change_url` is the change's web URL; Gerrit comment permalinks hang
    /// off it as `<change-url>/comment/<uuid>/`.
    fn into_remote(self, change_url: &str) -> RemoteReviewComment {
        let url = format!("{change_url}/comment/{}/", self.id);
        RemoteReviewComment {
            id: self.id,
            author: self.author.as_ref().and_then(GerritAccount::handle),
            body: self.message,
            created_at: at(&self.updated),
            in_reply_to: self.in_reply_to,
            url,
        }
    }
}

/// Gerrit's magic pseudo-paths. They carry patch-set and commit-message
/// comments, which have no line in the file diff, so threads on them are
/// dropped rather than anchored to a file that does not exist.
const MAGIC_PATHS: &[&str] = &["/COMMIT_MSG", "/MERGE_LIST", "/PATCHSET_LEVEL"];

/// Fold Gerrit's `path -> [comment]` map into tuicr's thread shape.
///
/// Gerrit models a discussion as a chain of `in_reply_to` links rather than an
/// explicit thread object, so each root comment (one with no parent, or whose
/// parent is missing from this response) opens a thread and every descendant
/// joins it. `current_patch_set` marks threads left on an older patch set as
/// outdated, matching what the Gerrit web UI shows.
pub fn threads_from_comment_map(
    comments: BTreeMap<String, Vec<GerritComment>>,
    change_url: &str,
    current_patch_set: u32,
) -> Vec<RemoteReviewThread> {
    let mut threads: Vec<RemoteReviewThread> = Vec::new();
    for (path, mut file_comments) in comments {
        if MAGIC_PATHS.contains(&path.as_str()) {
            continue;
        }
        file_comments.sort_by_key(|comment| at(&comment.updated));

        // Comment id -> index of the thread it belongs to, so a reply lands in
        // the same thread as the comment it answers. An `in_reply_to` pointing
        // outside this file — or at a comment Gerrit did not return — simply
        // misses the map and opens a thread of its own.
        let mut thread_of: HashMap<String, usize> = HashMap::new();

        for comment in file_comments {
            let parent_thread = comment
                .in_reply_to
                .as_ref()
                .and_then(|parent| thread_of.get(parent).copied());
            let id = comment.id.clone();
            match parent_thread {
                Some(index) => {
                    thread_of.insert(id, index);
                    threads[index]
                        .comments
                        .push(comment.into_remote(change_url));
                }
                None => {
                    thread_of.insert(id.clone(), threads.len());
                    threads.push(RemoteReviewThread {
                        id,
                        path: path.clone(),
                        line: comment.line,
                        side: comment.side(),
                        is_resolved: !comment.unresolved,
                        is_outdated: comment.patch_set != 0
                            && comment.patch_set != current_patch_set,
                        // Moves `comment`, so it comes last.
                        comments: vec![comment.into_remote(change_url)],
                    });
                }
            }
        }
    }
    threads
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::traits::ForgeRepository;

    fn gerrit_repo() -> ForgeRepository {
        ForgeRepository::gerrit("gerrit.example.com", "platform/base")
    }

    fn change_json() -> &'static str {
        r#"{
            "_number": 3965,
            "project": "platform/base",
            "branch": "main",
            "subject": "Implement feature X",
            "status": "NEW",
            "work_in_progress": false,
            "owner": {"username": "jdoe", "name": "John Doe"},
            "updated": "2013-02-21 11:16:36.775000000",
            "current_revision": "674ac754f91e64a0efb8087e59a176484bd534d1",
            "revisions": {
                "674ac754f91e64a0efb8087e59a176484bd534d1": {
                    "_number": 2,
                    "ref": "refs/changes/65/3965/2",
                    "commit": {
                        "message": "Implement feature X\n\nLonger rationale.\n",
                        "parents": [{"commit": "1eee2c9d8f352483781e772f35dc586a69ff5646"}]
                    }
                }
            }
        }"#
    }

    #[test]
    fn should_parse_gerrit_timestamps_without_a_zone_suffix() {
        // given
        let json = r#"{"_number": 1, "updated": "2013-02-21 11:16:36.775000000"}"#;
        // when
        let change: GerritChange = serde_json::from_str(json).expect("parse");
        // then
        assert_eq!(
            at(&change.updated).map(|value| value.to_rfc3339()),
            Some("2013-02-21T11:16:36.775+00:00".to_string())
        );
    }

    #[test]
    fn should_map_change_details_from_the_current_revision() {
        // given
        let change: GerritChange = serde_json::from_str(change_json()).expect("parse");
        // when
        let details = change.into_details(&gerrit_repo(), "https://gerrit.example.com");
        // then
        assert_eq!(details.number, 3965);
        assert_eq!(details.state, "OPEN");
        assert_eq!(details.author.as_deref(), Some("jdoe"));
        assert_eq!(details.head_sha, "674ac754f91e64a0efb8087e59a176484bd534d1");
        assert_eq!(details.base_sha, "1eee2c9d8f352483781e772f35dc586a69ff5646");
        assert_eq!(details.head_ref_name, "refs/changes/65/3965/2");
        assert_eq!(details.base_ref_name, "main");
        assert_eq!(details.body, "Longer rationale.");
        assert_eq!(
            details.url,
            "https://gerrit.example.com/c/platform/base/+/3965"
        );
        assert!(!details.is_read_only());
    }

    #[test]
    fn should_mark_merged_and_abandoned_changes_read_only() {
        // given
        let merged = change_json().replace(
            r#""status": "NEW""#,
            r#""status": "MERGED", "submitted": "2013-03-01 10:00:00.000000000""#,
        );
        let abandoned = change_json().replace(r#""status": "NEW""#, r#""status": "ABANDONED""#);
        // when
        let merged: GerritChange = serde_json::from_str(&merged).expect("parse");
        let abandoned: GerritChange = serde_json::from_str(&abandoned).expect("parse");
        let merged = merged.into_details(&gerrit_repo(), "https://gerrit.example.com");
        let abandoned = abandoned.into_details(&gerrit_repo(), "https://gerrit.example.com");
        // then
        assert_eq!(merged.state, "MERGED");
        assert_eq!(merged.read_only_reason(), Some("merged"));
        assert_eq!(abandoned.state, "CLOSED");
        assert_eq!(abandoned.read_only_reason(), Some("closed"));
    }

    #[test]
    fn should_expose_the_current_revision_as_the_only_commit() {
        // given
        let change: GerritChange = serde_json::from_str(change_json()).expect("parse");
        // when
        let commits = change.into_commits();
        // then
        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].short_oid, "674ac754");
        assert_eq!(commits[0].summary, "Implement feature X");
    }

    #[test]
    fn should_open_a_thread_for_a_reply_whose_parent_is_missing() {
        // given — `in_reply_to` points at a comment on another file, so it is
        // not in this file's bucket and cannot be threaded onto anything
        let json = r#"{
            "src/main.rs": [
                {"id": "orphan", "in_reply_to": "elsewhere", "line": 3,
                 "message": "still a comment", "patch_set": 2,
                 "updated": "2013-02-21 11:00:00.000000000"}
            ]
        }"#;
        let comments: BTreeMap<String, Vec<GerritComment>> =
            serde_json::from_str(json).expect("parse");
        // when
        let threads = threads_from_comment_map(comments, "https://gerrit.example.com", 2);
        // then — dropping it instead would lose the comment entirely
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].id, "orphan");
        assert_eq!(threads[0].comments.len(), 1);
    }

    #[test]
    fn should_chain_replies_into_one_thread_per_root_comment() {
        // given
        let json = r#"{
            "src/main.rs": [
                {"id": "root", "line": 12, "message": "why?", "unresolved": true,
                 "patch_set": 2, "updated": "2013-02-21 11:00:00.000000000"},
                {"id": "reply", "in_reply_to": "root", "line": 12, "message": "because",
                 "patch_set": 2, "updated": "2013-02-21 12:00:00.000000000"},
                {"id": "other", "line": 40, "message": "nit", "side": "PARENT",
                 "patch_set": 1, "updated": "2013-02-21 13:00:00.000000000"}
            ],
            "/COMMIT_MSG": [
                {"id": "msg", "line": 1, "message": "typo", "patch_set": 2,
                 "updated": "2013-02-21 11:00:00.000000000"}
            ]
        }"#;
        let comments: BTreeMap<String, Vec<GerritComment>> =
            serde_json::from_str(json).expect("parse");
        // when
        let threads = threads_from_comment_map(
            comments,
            "https://gerrit.example.com/c/platform/base/+/3965",
            2,
        );
        // then — the magic path is dropped and the reply joins its root
        assert_eq!(threads.len(), 2);
        assert_eq!(threads[0].id, "root");
        assert_eq!(threads[0].comments.len(), 2);
        assert_eq!(threads[0].side, RemoteCommentSide::Right);
        assert!(!threads[0].is_resolved);
        assert!(!threads[0].is_outdated);
        assert_eq!(threads[1].id, "other");
        assert_eq!(threads[1].side, RemoteCommentSide::Left);
        assert!(threads[1].is_resolved);
        assert!(
            threads[1].is_outdated,
            "patch set 1 is behind the current 2"
        );
    }
}
