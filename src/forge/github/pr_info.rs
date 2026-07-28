//! Parse extended `gh pr view --json` fields for the PR description panel.
//!
//! `gh` flattens some GraphQL connections to bare arrays in JSON export
//! (`reviewRequests`, `latestReviews`, `statusCheckRollup`), but older paths
//! and partial responses can still expose nested `{ nodes: [...] }` shapes.

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::error::{Result, TuicrError};
use crate::forge::github::models::GhPullRequestDetails;
use crate::forge::traits::{
    ForgeRepository, PullRequestCheckStatus, PullRequestInfo, PullRequestReviewStatus,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GhPullRequestInfoResponse {
    #[serde(flatten)]
    pub details: GhPullRequestDetails,
    #[serde(default)]
    pub review_decision: Option<String>,
    #[serde(default)]
    pub mergeable: Option<String>,
    #[serde(default)]
    pub merge_state_status: Option<String>,
    #[serde(default, deserialize_with = "deserialize_review_requests")]
    review_requests: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_latest_reviews")]
    latest_reviews: Vec<PullRequestReviewStatus>,
    #[serde(default, deserialize_with = "deserialize_status_checks")]
    status_check_rollup: Vec<PullRequestCheckStatus>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhLatestReview {
    #[serde(default)]
    author: Option<GhAuthor>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    submitted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
struct GhAuthor {
    #[serde(default)]
    login: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhReviewRequestFlat {
    #[serde(default, rename = "__typename")]
    type_name: Option<String>,
    #[serde(default)]
    login: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    slug: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct GhReviewRequestWrappedNode {
    #[serde(default, rename = "requestedReviewer")]
    requested_reviewer: GhReviewRequestFlat,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum GhReviewRequestsInput {
    Flat(Vec<GhReviewRequestFlat>),
    Wrapped {
        nodes: Vec<GhReviewRequestWrappedNode>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum GhLatestReviewsInput {
    Flat(Vec<GhLatestReview>),
    Wrapped { nodes: Vec<GhLatestReview> },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "__typename")]
enum GhStatusCheck {
    CheckRun {
        #[serde(default)]
        name: String,
        #[serde(default)]
        status: Option<String>,
        #[serde(default)]
        conclusion: Option<String>,
    },
    StatusContext {
        #[serde(default)]
        context: String,
        #[serde(default)]
        state: Option<String>,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Deserialize)]
struct GhStatusCheckRollupNested {
    #[serde(default)]
    nodes: Vec<GhStatusCheckRollupNode>,
}

#[derive(Debug, Default, Deserialize)]
struct GhStatusCheckRollupNode {
    #[serde(default)]
    commit: GhStatusCheckRollupCommit,
}

#[derive(Debug, Default, Deserialize)]
struct GhStatusCheckRollupCommit {
    #[serde(default, rename = "statusCheckRollup")]
    status_check_rollup: GhStatusCheckContexts,
}

#[derive(Debug, Default, Deserialize)]
struct GhStatusCheckContexts {
    #[serde(default)]
    contexts: GhStatusCheckContextNodes,
}

#[derive(Debug, Default, Deserialize)]
struct GhStatusCheckContextNodes {
    #[serde(default)]
    nodes: Vec<GhStatusCheck>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum GhStatusCheckRollupInput {
    Flat(Vec<GhStatusCheck>),
    Nested(GhStatusCheckRollupNested),
}

pub(crate) fn parse_pull_request_info(
    json: &str,
    repository: &ForgeRepository,
) -> Result<PullRequestInfo> {
    match serde_json::from_str::<GhPullRequestInfoResponse>(json) {
        Ok(response) => response.into_info(repository),
        Err(e) => {
            let details: GhPullRequestDetails = serde_json::from_str(json).map_err(|_| {
                TuicrError::Forge(format!("Failed to parse GitHub PR info response: {e}"))
            })?;
            Ok(PullRequestInfo::from_details(
                details.into_details(repository)?,
            ))
        }
    }
}

impl GhPullRequestInfoResponse {
    fn into_info(self, repository: &ForgeRepository) -> Result<PullRequestInfo> {
        let details = self.details.into_details(repository)?;

        Ok(PullRequestInfo {
            details,
            review_decision: non_empty(self.review_decision),
            mergeable: non_empty(self.mergeable),
            merge_state: non_empty(self.merge_state_status),
            requested_reviewers: self.review_requests,
            latest_reviews: self.latest_reviews,
            checks: self.status_check_rollup,
        })
    }
}

fn deserialize_review_requests<'de, D>(
    deserializer: D,
) -> std::result::Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<GhReviewRequestsInput>::deserialize(deserializer)?;
    Ok(match value {
        None => Vec::new(),
        Some(GhReviewRequestsInput::Flat(nodes)) => nodes
            .into_iter()
            .filter_map(review_request_flat_label)
            .collect(),
        Some(GhReviewRequestsInput::Wrapped { nodes }) => nodes
            .into_iter()
            .filter_map(|node| review_request_flat_label(node.requested_reviewer))
            .collect(),
    })
}

fn deserialize_latest_reviews<'de, D>(
    deserializer: D,
) -> std::result::Result<Vec<PullRequestReviewStatus>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<GhLatestReviewsInput>::deserialize(deserializer)?;
    Ok(match value {
        None => Vec::new(),
        Some(
            GhLatestReviewsInput::Flat(reviews) | GhLatestReviewsInput::Wrapped { nodes: reviews },
        ) => reviews
            .into_iter()
            .filter_map(latest_review_status)
            .collect(),
    })
}

fn deserialize_status_checks<'de, D>(
    deserializer: D,
) -> std::result::Result<Vec<PullRequestCheckStatus>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<GhStatusCheckRollupInput>::deserialize(deserializer)?;
    Ok(match value {
        None => Vec::new(),
        Some(GhStatusCheckRollupInput::Flat(checks)) => {
            checks.into_iter().filter_map(status_check_label).collect()
        }
        Some(GhStatusCheckRollupInput::Nested(nested)) => nested
            .nodes
            .into_iter()
            .flat_map(|node| node.commit.status_check_rollup.contexts.nodes)
            .filter_map(status_check_label)
            .collect(),
    })
}

fn review_request_flat_label(node: GhReviewRequestFlat) -> Option<String> {
    match node.type_name.as_deref() {
        Some("Team") => node
            .name
            .filter(|s| !s.is_empty())
            .or(node.slug.filter(|s| !s.is_empty())),
        Some("User") | Some("Bot") | _ => node.login.filter(|s| !s.is_empty()),
    }
}

fn latest_review_status(review: GhLatestReview) -> Option<PullRequestReviewStatus> {
    let state = review.state.filter(|s| !s.is_empty())?;
    Some(PullRequestReviewStatus {
        author: review.author.and_then(|a| a.login),
        state,
        submitted_at: review.submitted_at,
    })
}

fn status_check_label(check: GhStatusCheck) -> Option<PullRequestCheckStatus> {
    match check {
        GhStatusCheck::CheckRun {
            name,
            status,
            conclusion,
        } => {
            if name.is_empty() {
                None
            } else {
                Some(PullRequestCheckStatus {
                    name,
                    status: non_empty(status),
                    conclusion: non_empty(conclusion),
                })
            }
        }
        GhStatusCheck::StatusContext { context, state } => {
            if context.is_empty() {
                None
            } else {
                Some(PullRequestCheckStatus {
                    name: context,
                    status: None,
                    conclusion: non_empty(state),
                })
            }
        }
        GhStatusCheck::Unknown => None,
    }
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::traits::ForgeRepository;

    fn repo() -> ForgeRepository {
        ForgeRepository {
            kind: crate::forge::traits::ForgeKind::GitHub,
            host: "github.com".to_string(),
            owner: "owner".to_string(),
            name: "repo".to_string(),
        }
    }

    const BASE_FIELDS: &str = r#"
            "number": 42,
            "title": "Add panel",
            "url": "https://github.com/owner/repo/pull/42",
            "state": "OPEN",
            "isDraft": false,
            "author": { "login": "alice" },
            "headRefName": "feature",
            "baseRefName": "main",
            "headRefOid": "abc1234567890",
            "baseRefOid": "def0987654321",
            "body": "Ship the panel",
            "updatedAt": "2026-06-01T12:00:00Z",
            "closed": false,
            "mergedAt": null,
            "reviewDecision": "REVIEW_REQUIRED",
            "mergeable": "MERGEABLE",
            "mergeStateStatus": "BLOCKED"
    "#;

    #[test]
    fn should_parse_gh_flat_export_shapes() {
        let json = format!(
            r#"{{
            {BASE_FIELDS},
            "reviewRequests": [
                {{ "__typename": "User", "login": "bob" }},
                {{ "__typename": "Team", "name": "security", "slug": "org/security" }}
            ],
            "latestReviews": [
                {{
                    "author": {{ "login": "carol" }},
                    "state": "APPROVED",
                    "submittedAt": "2026-06-01T11:00:00Z"
                }}
            ],
            "statusCheckRollup": [
                {{
                    "__typename": "CheckRun",
                    "name": "build",
                    "status": "COMPLETED",
                    "conclusion": "SUCCESS"
                }}
            ]
        }}"#
        );

        let info = parse_pull_request_info(&json, &repo()).unwrap();
        assert_eq!(info.requested_reviewers, vec!["bob", "security"]);
        assert_eq!(info.latest_reviews.len(), 1);
        assert_eq!(info.checks.len(), 1);
    }

    #[test]
    fn should_parse_wrapped_graphql_shapes() {
        let json = format!(
            r#"{{
            {BASE_FIELDS},
            "reviewRequests": {{
                "nodes": [
                    {{ "requestedReviewer": {{ "__typename": "User", "login": "bob" }} }}
                ]
            }},
            "latestReviews": {{
                "nodes": [
                    {{ "author": {{ "login": "carol" }}, "state": "APPROVED" }}
                ]
            }},
            "statusCheckRollup": {{
                "nodes": [
                    {{
                        "commit": {{
                            "statusCheckRollup": {{
                                "contexts": {{
                                    "nodes": [
                                        {{
                                            "__typename": "StatusContext",
                                            "context": "ci/travis",
                                            "state": "FAILURE"
                                        }}
                                    ]
                                }}
                            }}
                        }}
                    }}
                ]
            }}
        }}"#
        );

        let info = parse_pull_request_info(&json, &repo()).unwrap();
        assert_eq!(info.requested_reviewers, vec!["bob"]);
        assert_eq!(info.latest_reviews.len(), 1);
        assert_eq!(info.checks[0].name, "ci/travis");
    }

    #[test]
    fn should_tolerate_empty_optional_sections() {
        let json = r#"{
            "number": 1,
            "title": "Empty",
            "url": "https://github.com/owner/repo/pull/1",
            "state": "OPEN",
            "headRefName": "feature",
            "baseRefName": "main",
            "headRefOid": "abc1234567890",
            "baseRefOid": "def0987654321",
            "body": "",
            "closed": false
        }"#;

        let info = parse_pull_request_info(json, &repo()).unwrap();
        assert!(info.requested_reviewers.is_empty());
        assert!(info.latest_reviews.is_empty());
        assert!(info.checks.is_empty());
    }
}
