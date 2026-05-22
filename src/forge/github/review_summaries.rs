//! GraphQL parsing for GitHub review summaries (the body text on a
//! `PullRequestReview` itself, not on its line-anchored threads).
//!
//! Reviews are a separate connection from `reviewThreads`. The summary is
//! what shows up on the PR overview as "<reviewer> commented / approved /
//! requested changes" with body markdown. We surface these in the
//! top-of-diff review area so reviewers' high-level feedback shows up
//! alongside local review-level drafts.
//!
//! Payload shape (only fields we read):
//!
//! ```json
//! {
//!   "data": {
//!     "repository": {
//!       "pullRequest": {
//!         "reviews": {
//!           "pageInfo": { "hasNextPage": false, "endCursor": null },
//!           "nodes": [
//!             {
//!               "id": "PRR_kw...",
//!               "state": "COMMENTED",
//!               "body": "Overall this looks tight ...",
//!               "author": { "login": "alice" },
//!               "submittedAt": "2026-05-12T18:30:00Z",
//!               "url": "https://github.com/agavra/tuicr/pull/356#pullrequestreview-1"
//!             }
//!           ]
//!         }
//!       }
//!     }
//!   }
//! }
//! ```

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::error::{Result, TuicrError};
use crate::forge::github::review_threads::GhPageInfo;
use crate::forge::remote_comments::{RemoteReviewState, RemoteReviewSummary};

#[derive(Debug, Deserialize)]
struct GhAuthor {
    #[serde(default)]
    login: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhReview {
    id: String,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    body: String,
    #[serde(default)]
    author: Option<GhAuthor>,
    #[serde(default)]
    submitted_at: Option<DateTime<Utc>>,
    #[serde(default)]
    url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhReviewsConn {
    #[serde(default)]
    page_info: Option<GhPageInfo>,
    #[serde(default)]
    nodes: Vec<GhReview>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhPullRequest {
    #[serde(default)]
    reviews: Option<GhReviewsConn>,
}

#[derive(Debug, Deserialize)]
struct GhRepository {
    #[serde(default, rename = "pullRequest")]
    pull_request: Option<GhPullRequest>,
}

#[derive(Debug, Deserialize)]
struct GhData {
    #[serde(default)]
    repository: Option<GhRepository>,
}

#[derive(Debug, Deserialize)]
struct GhResponse {
    #[serde(default)]
    data: Option<GhData>,
}

#[derive(Debug)]
pub(crate) struct ParsedReviewsPage {
    pub summaries: Vec<RemoteReviewSummary>,
    pub page_info: Option<GhPageInfo>,
}

/// Parse one GraphQL response page into summaries + pagination info.
/// Reviews with empty bodies (bare approvals, auto-submitted reviews from
/// API integrations) are dropped — they have nothing to display.
pub(crate) fn parse_graphql_page(json: &str) -> Result<ParsedReviewsPage> {
    let response: GhResponse = serde_json::from_str(json).map_err(|e| {
        TuicrError::Forge(format!(
            "Failed to parse GitHub review summaries response: {e}"
        ))
    })?;

    let conn = response
        .data
        .and_then(|d| d.repository)
        .and_then(|r| r.pull_request)
        .and_then(|p| p.reviews);

    let Some(conn) = conn else {
        return Ok(ParsedReviewsPage {
            summaries: Vec::new(),
            page_info: None,
        });
    };

    let page_info = conn.page_info;
    let mut summaries = Vec::with_capacity(conn.nodes.len());
    for raw in conn.nodes {
        if raw.body.trim().is_empty() {
            continue;
        }
        summaries.push(RemoteReviewSummary {
            id: raw.id,
            author: raw.author.and_then(|a| a.login),
            body: raw.body,
            state: raw
                .state
                .as_deref()
                .map(RemoteReviewState::parse)
                .unwrap_or(RemoteReviewState::Commented),
            created_at: raw.submitted_at,
            url: raw.url.unwrap_or_default(),
        });
    }
    Ok(ParsedReviewsPage {
        summaries,
        page_info,
    })
}

/// Build the GraphQL query for fetching reviews on a PR.
pub(crate) fn build_query(after_cursor: Option<&str>) -> String {
    let cursor_arg = match after_cursor {
        Some(_) => ", after: $after",
        None => "",
    };
    format!(
        r#"query($owner: String!, $name: String!, $number: Int!{cursor_param}) {{
  repository(owner: $owner, name: $name) {{
    pullRequest(number: $number) {{
      reviews(first: 100{cursor_arg}) {{
        pageInfo {{ hasNextPage endCursor }}
        nodes {{
          id
          state
          body
          author {{ login }}
          submittedAt
          url
        }}
      }}
    }}
  }}
}}"#,
        cursor_param = if after_cursor.is_some() {
            ", $after: String!"
        } else {
            ""
        },
        cursor_arg = cursor_arg,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWO_REVIEWS_JSON: &str = r##"{
        "data": {
            "repository": {
                "pullRequest": {
                    "reviews": {
                        "pageInfo": { "hasNextPage": false, "endCursor": null },
                        "nodes": [
                            {
                                "id": "PRR_1",
                                "state": "COMMENTED",
                                "body": "Overall this looks tight.",
                                "author": { "login": "alice" },
                                "submittedAt": "2026-05-12T18:30:00Z",
                                "url": "https://example.com/r/1"
                            },
                            {
                                "id": "PRR_2",
                                "state": "APPROVED",
                                "body": "LGTM",
                                "author": { "login": "bob" },
                                "submittedAt": "2026-05-13T09:00:00Z",
                                "url": "https://example.com/r/2"
                            }
                        ]
                    }
                }
            }
        }
    }"##;

    const EMPTY_BODY_JSON: &str = r##"{
        "data": {
            "repository": {
                "pullRequest": {
                    "reviews": {
                        "nodes": [
                            {
                                "id": "PRR_empty",
                                "state": "APPROVED",
                                "body": "",
                                "author": { "login": "ci-bot" },
                                "url": "https://example.com/r/empty"
                            },
                            {
                                "id": "PRR_whitespace",
                                "state": "COMMENTED",
                                "body": "   \n\t",
                                "author": { "login": "ci-bot" },
                                "url": "https://example.com/r/ws"
                            },
                            {
                                "id": "PRR_real",
                                "state": "COMMENTED",
                                "body": "Real feedback.",
                                "author": { "login": "alice" },
                                "url": "https://example.com/r/real"
                            }
                        ]
                    }
                }
            }
        }
    }"##;

    const NULL_REPO_JSON: &str = r##"{ "data": { "repository": null } }"##;

    #[test]
    fn should_parse_two_reviews_with_state_and_author() {
        // given/when
        let parsed = parse_graphql_page(TWO_REVIEWS_JSON).unwrap();
        // then
        assert_eq!(parsed.summaries.len(), 2);
        assert_eq!(parsed.summaries[0].id, "PRR_1");
        assert_eq!(parsed.summaries[0].author.as_deref(), Some("alice"));
        assert_eq!(parsed.summaries[0].state, RemoteReviewState::Commented);
        assert_eq!(parsed.summaries[1].state, RemoteReviewState::Approved);
    }

    #[test]
    fn should_drop_reviews_with_empty_or_whitespace_body() {
        // given/when
        let parsed = parse_graphql_page(EMPTY_BODY_JSON).unwrap();
        // then — only the review with a real body survives
        assert_eq!(parsed.summaries.len(), 1);
        assert_eq!(parsed.summaries[0].id, "PRR_real");
    }

    #[test]
    fn should_tolerate_missing_repository_object() {
        // given/when
        let parsed = parse_graphql_page(NULL_REPO_JSON).unwrap();
        // then
        assert!(parsed.summaries.is_empty());
        assert!(parsed.page_info.is_none());
    }

    #[test]
    fn should_build_query_without_cursor_for_first_page() {
        // given/when
        let q = build_query(None);
        // then
        assert!(q.contains("reviews(first: 100)"));
        assert!(!q.contains("after: $after"));
    }

    #[test]
    fn should_build_query_with_cursor_for_subsequent_pages() {
        // given/when
        let q = build_query(Some("CURSOR"));
        // then
        assert!(q.contains("$after: String!"));
        assert!(q.contains("after: $after"));
    }
}
