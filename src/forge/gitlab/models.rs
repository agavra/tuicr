use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::error::{Result, TuicrError};
use crate::forge::remote_comments::{RemoteCommentSide, RemoteReviewComment, RemoteReviewThread};
use crate::forge::traits::{
    ForgeRepository, PullRequestCommit, PullRequestDetails, PullRequestSummary,
};

#[derive(Debug, Deserialize)]
pub struct GlabMrSummary {
    pub iid: u64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub author: Option<GlabUser>,
    #[serde(default)]
    pub source_branch: String,
    #[serde(default)]
    pub target_branch: String,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub web_url: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub draft: bool,
}

impl GlabMrSummary {
    pub fn into_summary(self, repo: &ForgeRepository) -> PullRequestSummary {
        PullRequestSummary {
            repository: repo.clone(),
            number: self.iid,
            title: self.title,
            author: self.author.map(|a| a.username),
            head_ref_name: self.source_branch,
            base_ref_name: self.target_branch,
            updated_at: self.updated_at,
            url: self.web_url,
            state: normalize_state(&self.state),
            is_draft: self.draft,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct GlabMrDetails {
    pub iid: u64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub web_url: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub author: Option<GlabUser>,
    #[serde(default)]
    pub source_branch: String,
    #[serde(default)]
    pub target_branch: String,
    /// Head SHA — last commit on the source branch.
    #[serde(default)]
    pub sha: String,
    #[serde(default)]
    pub diff_refs: Option<GlabDiffRefs>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub closed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub merged_at: Option<DateTime<Utc>>,
}

impl GlabMrDetails {
    pub fn into_details(self, repo: &ForgeRepository) -> Result<PullRequestDetails> {
        let (head_sha, base_sha, diff_start_sha) = match self.diff_refs {
            Some(refs) => (refs.head_sha, refs.base_sha, Some(refs.start_sha)),
            None => {
                // Fall back to `sha` for head; base is unknown
                if self.sha.is_empty() {
                    return Err(TuicrError::Forge(
                        "GitLab MR response missing diff_refs and sha".to_string(),
                    ));
                }
                (self.sha.clone(), String::new(), None)
            }
        };
        let closed = self.state == "closed";
        let state = normalize_state(&self.state);
        Ok(PullRequestDetails {
            repository: repo.clone(),
            number: self.iid,
            title: self.title,
            url: self.web_url,
            state,
            is_draft: self.draft,
            author: self.author.map(|a| a.username),
            head_ref_name: self.source_branch,
            base_ref_name: self.target_branch,
            head_sha,
            base_sha,
            body: self.description,
            updated_at: self.updated_at,
            closed,
            merged_at: self.merged_at,
            diff_start_sha,
        })
    }
}

#[derive(Debug, Deserialize, Default)]
pub struct GlabDiffRefs {
    #[serde(default)]
    pub base_sha: String,
    #[serde(default)]
    pub head_sha: String,
    #[serde(default)]
    pub start_sha: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct GlabUser {
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct GlabMrVersion {
    #[serde(default)]
    pub head_commit_sha: String,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize, Default)]
pub struct GlabApprovalState {
    #[serde(default)]
    pub approved_by: Vec<GlabApprovedBy>,
}

#[derive(Debug, Deserialize, Default)]
pub struct GlabApprovedBy {
    #[serde(default)]
    pub user: GlabUser,
    #[serde(default)]
    pub approved_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct GlabCommit {
    pub id: String,
    #[serde(default)]
    pub short_id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub author_name: String,
    #[serde(default)]
    pub committed_date: Option<DateTime<Utc>>,
}

impl GlabCommit {
    pub fn into_pull_request_commit(self) -> PullRequestCommit {
        let short_oid = if self.short_id.is_empty() {
            self.id.chars().take(7).collect()
        } else {
            self.short_id.clone()
        };
        PullRequestCommit {
            oid: self.id,
            short_oid,
            summary: self.title,
            author: self.author_name,
            timestamp: self.committed_date,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct GlabDiscussion {
    pub id: String,
    #[serde(default)]
    pub individual_note: bool,
    #[serde(default)]
    pub notes: Vec<GlabNote>,
}

impl GlabDiscussion {
    pub fn into_review_thread(self) -> Option<RemoteReviewThread> {
        let root = self.notes.first()?;

        if self.individual_note {
            // General MR note (review summary), no diff position.
            // Skip system events (e.g. "requested review from @X") and empty bodies.
            if root.system || root.body.is_empty() {
                return None;
            }
            let comments = self
                .notes
                .into_iter()
                .filter(|n| !n.system && !n.body.is_empty())
                .map(|note| RemoteReviewComment {
                    id: note.id.to_string(),
                    author: Some(note.author.username),
                    body: note.body,
                    created_at: note.created_at,
                    in_reply_to: None,
                    url: String::new(),
                })
                .collect::<Vec<_>>();
            if comments.is_empty() {
                return None;
            }
            return Some(RemoteReviewThread {
                id: self.id,
                path: String::new(),
                line: None,
                side: RemoteCommentSide::Right,
                is_resolved: false,
                is_outdated: false,
                comments,
            });
        }

        // Only inline (positional) discussions have a position on the root note.
        let position = root.position.as_ref()?;

        // Skip non-text positions (e.g. image diffs).
        if position.position_type != "text" {
            return None;
        }

        let (path, line, side) = if let Some(new_line) = position.new_line {
            // Comment on the new (right) side.
            let path = position.new_path.clone().unwrap_or_default();
            (path, Some(new_line), RemoteCommentSide::Right)
        } else if let Some(old_line) = position.old_line {
            // Comment on the old (left) side only.
            let path = position
                .old_path
                .clone()
                .or_else(|| position.new_path.clone())
                .unwrap_or_default();
            (path, Some(old_line), RemoteCommentSide::Left)
        } else {
            return None;
        };

        if path.is_empty() {
            return None;
        }

        let is_resolved = root.resolved;
        let comments = self
            .notes
            .into_iter()
            .map(|note| RemoteReviewComment {
                id: note.id.to_string(),
                author: Some(note.author.username),
                body: note.body,
                created_at: note.created_at,
                in_reply_to: None,
                url: String::new(),
            })
            .collect();

        Some(RemoteReviewThread {
            id: self.id,
            path,
            line,
            side,
            is_resolved,
            is_outdated: false,
            comments,
        })
    }
}

#[derive(Debug, Deserialize)]
pub struct GlabNote {
    pub id: u64,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub author: GlabNoteAuthor,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub commit_id: Option<String>,
    #[serde(default)]
    pub position: Option<GlabNotePosition>,
    #[serde(default)]
    pub resolved: bool,
    #[serde(default)]
    pub system: bool,
}

#[derive(Debug, Deserialize, Default)]
pub struct GlabNoteAuthor {
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct GlabNotePosition {
    #[serde(default)]
    pub position_type: String,
    #[serde(default)]
    pub head_sha: Option<String>,
    pub new_path: Option<String>,
    pub new_line: Option<u32>,
    pub old_path: Option<String>,
    pub old_line: Option<u32>,
}

fn normalize_state(state: &str) -> String {
    match state.to_ascii_lowercase().as_str() {
        "opened" | "open" => "OPEN".to_string(),
        "merged" => "MERGED".to_string(),
        "closed" => "CLOSED".to_string(),
        other => other.to_ascii_uppercase(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::traits::ForgeRepository;

    fn gitlab_repo() -> ForgeRepository {
        ForgeRepository::gitlab("gitlab.com", "owner", "repo")
    }

    #[test]
    fn should_deserialize_glab_mr_summary() {
        let json = r#"{
            "iid": 42,
            "title": "My MR",
            "author": { "username": "alice", "name": "Alice" },
            "source_branch": "feature",
            "target_branch": "main",
            "updated_at": "2024-01-01T00:00:00Z",
            "web_url": "https://gitlab.com/owner/repo/-/merge_requests/42",
            "state": "opened",
            "draft": false
        }"#;
        let summary: GlabMrSummary = serde_json::from_str(json).unwrap();
        assert_eq!(summary.iid, 42);
        assert_eq!(summary.title, "My MR");
        assert_eq!(summary.author.as_ref().unwrap().username, "alice");
        assert_eq!(summary.source_branch, "feature");
        assert_eq!(summary.target_branch, "main");
        let pr_summary = summary.into_summary(&gitlab_repo());
        assert_eq!(pr_summary.number, 42);
        assert_eq!(pr_summary.state, "OPEN");
    }

    #[test]
    fn should_deserialize_glab_mr_details_with_diff_refs() {
        let json = r#"{
            "iid": 42,
            "title": "My MR",
            "web_url": "https://gitlab.com/owner/repo/-/merge_requests/42",
            "state": "opened",
            "draft": false,
            "author": { "username": "alice", "name": "Alice" },
            "source_branch": "feature",
            "target_branch": "main",
            "sha": "head111",
            "diff_refs": {
                "base_sha": "base000",
                "head_sha": "head111",
                "start_sha": "start222"
            },
            "description": "desc",
            "merged_at": null,
            "closed_at": null
        }"#;
        let details: GlabMrDetails = serde_json::from_str(json).unwrap();
        assert_eq!(details.iid, 42);
        let pr = details.into_details(&gitlab_repo()).unwrap();
        assert_eq!(pr.head_sha, "head111");
        assert_eq!(pr.base_sha, "base000");
        assert_eq!(pr.diff_start_sha, Some("start222".to_string()));
        assert!(!pr.closed);
    }

    #[test]
    fn should_convert_glab_discussion_to_review_thread() {
        let discussion = GlabDiscussion {
            id: "disc-1".to_string(),
            individual_note: false,
            notes: vec![GlabNote {
                id: 100,
                body: "review comment".to_string(),
                author: GlabNoteAuthor {
                    username: "bob".to_string(),
                    name: "Bob".to_string(),
                },
                created_at: None,
                commit_id: None,
                position: Some(GlabNotePosition {
                    position_type: "text".to_string(),
                    head_sha: None,
                    new_path: Some("src/lib.rs".to_string()),
                    new_line: Some(42),
                    old_path: None,
                    old_line: None,
                }),
                resolved: false,
                system: false,
            }],
        };
        let thread = discussion.into_review_thread().unwrap();
        assert_eq!(thread.id, "disc-1");
        assert_eq!(thread.path, "src/lib.rs");
        assert_eq!(thread.line, Some(42));
        assert_eq!(thread.side, RemoteCommentSide::Right);
        assert!(!thread.is_resolved);
        assert!(!thread.is_outdated);
        assert_eq!(thread.comments[0].author.as_deref(), Some("bob"));
    }

    #[test]
    fn should_skip_discussion_without_position() {
        // A non-individual_note discussion without a diff position is dropped.
        let discussion = GlabDiscussion {
            id: "disc-2".to_string(),
            individual_note: false,
            notes: vec![GlabNote {
                id: 101,
                body: "general comment".to_string(),
                author: GlabNoteAuthor::default(),
                created_at: None,
                commit_id: None,
                position: None,
                resolved: false,
                system: false,
            }],
        };
        assert!(discussion.into_review_thread().is_none());
    }

    #[test]
    fn should_convert_individual_note_discussion_to_review_level_thread() {
        let discussion = GlabDiscussion {
            id: "disc-3".to_string(),
            individual_note: true,
            notes: vec![GlabNote {
                id: 200,
                body: "[NOTE] review comments".to_string(),
                author: GlabNoteAuthor {
                    username: "alice".to_string(),
                    name: "Alice".to_string(),
                },
                created_at: None,
                commit_id: None,
                position: None,
                resolved: false,
                system: false,
            }],
        };
        let thread = discussion.into_review_thread().unwrap();
        assert_eq!(thread.id, "disc-3");
        assert_eq!(thread.path, "");
        assert_eq!(thread.line, None);
        assert_eq!(thread.side, RemoteCommentSide::Right);
        assert!(!thread.is_resolved);
        assert_eq!(thread.comments[0].author.as_deref(), Some("alice"));
        assert_eq!(thread.comments[0].body, "[NOTE] review comments");
    }

    #[test]
    fn should_skip_individual_note_system_events() {
        let discussion = GlabDiscussion {
            id: "disc-4".to_string(),
            individual_note: true,
            notes: vec![GlabNote {
                id: 201,
                body: "requested review from @bob".to_string(),
                author: GlabNoteAuthor::default(),
                created_at: None,
                commit_id: None,
                position: None,
                resolved: false,
                system: true,
            }],
        };
        assert!(discussion.into_review_thread().is_none());
    }
}
