//! GraphQL parsing for minimal GitHub pull request review metadata.
//!
//! This intentionally differs from `review_summaries`: a bare approval with
//! an empty body has no summary to render, but it still counts as a submitted
//! review when deciding whether a PR has commits since the viewer last looked.

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::error::{Result, TuicrError};
use crate::forge::github::review_threads::GhPageInfo;
use crate::forge::traits::{PullRequestReviewMetadata, PullRequestReviewRecord};

#[derive(Debug, Deserialize)]
struct GhViewer {
    #[serde(default)]
    login: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GhAuthor {
    #[serde(default)]
    login: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GhCommit {
    #[serde(default)]
    oid: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhReview {
    #[serde(default)]
    author: Option<GhAuthor>,
    #[serde(default)]
    submitted_at: Option<DateTime<Utc>>,
    #[serde(default)]
    commit: Option<GhCommit>,
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
    viewer: Option<GhViewer>,
    #[serde(default)]
    repository: Option<GhRepository>,
}

#[derive(Debug, Deserialize)]
struct GhResponse {
    #[serde(default)]
    data: Option<GhData>,
}

#[derive(Debug)]
pub(crate) struct ParsedReviewMetadataPage {
    pub metadata: PullRequestReviewMetadata,
    pub page_info: Option<GhPageInfo>,
}

pub(crate) fn parse_graphql_page(json: &str) -> Result<ParsedReviewMetadataPage> {
    let response: GhResponse = serde_json::from_str(json).map_err(|e| {
        TuicrError::Forge(format!(
            "Failed to parse GitHub review metadata response: {e}"
        ))
    })?;

    let Some(data) = response.data else {
        return Ok(ParsedReviewMetadataPage {
            metadata: PullRequestReviewMetadata::default(),
            page_info: None,
        });
    };

    let viewer_login = data.viewer.and_then(|v| v.login);
    let conn = data
        .repository
        .and_then(|r| r.pull_request)
        .and_then(|p| p.reviews);

    let Some(conn) = conn else {
        return Ok(ParsedReviewMetadataPage {
            metadata: PullRequestReviewMetadata {
                viewer_login,
                reviews: Vec::new(),
            },
            page_info: None,
        });
    };

    let reviews = conn
        .nodes
        .into_iter()
        .map(|raw| PullRequestReviewRecord {
            author: raw.author.and_then(|a| a.login),
            submitted_at: raw.submitted_at,
            commit_oid: raw.commit.and_then(|c| c.oid),
        })
        .collect();

    Ok(ParsedReviewMetadataPage {
        metadata: PullRequestReviewMetadata {
            viewer_login,
            reviews,
        },
        page_info: conn.page_info,
    })
}

pub(crate) fn build_query(after_cursor: Option<&str>) -> String {
    let cursor_arg = match after_cursor {
        Some(_) => ", after: $after",
        None => "",
    };
    format!(
        r#"query($owner: String!, $name: String!, $number: Int!{cursor_param}) {{
  viewer {{ login }}
  repository(owner: $owner, name: $name) {{
    pullRequest(number: $number) {{
      reviews(first: 100{cursor_arg}) {{
        pageInfo {{ hasNextPage endCursor }}
        nodes {{
          author {{ login }}
          submittedAt
          commit {{ oid }}
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

    #[test]
    fn should_parse_viewer_and_reviews_with_commit_oids() {
        let json = r##"{
            "data": {
                "viewer": { "login": "ronen-hoffer" },
                "repository": {
                    "pullRequest": {
                        "reviews": {
                            "pageInfo": { "hasNextPage": false, "endCursor": null },
                            "nodes": [
                                {
                                    "author": { "login": "alice" },
                                    "submittedAt": "2026-06-01T18:59:23Z",
                                    "commit": { "oid": "aaa111" }
                                },
                                {
                                    "author": { "login": "ronen-hoffer" },
                                    "submittedAt": "2026-06-02T06:32:29Z",
                                    "commit": { "oid": "bbb222" }
                                }
                            ]
                        }
                    }
                }
            }
        }"##;

        let parsed = parse_graphql_page(json).unwrap();

        assert_eq!(
            parsed.metadata.viewer_login.as_deref(),
            Some("ronen-hoffer")
        );
        assert_eq!(parsed.metadata.reviews.len(), 2);
        assert_eq!(
            parsed.metadata.reviews[1].author.as_deref(),
            Some("ronen-hoffer")
        );
        assert_eq!(
            parsed.metadata.reviews[1].commit_oid.as_deref(),
            Some("bbb222")
        );
    }

    #[test]
    fn should_build_query_with_cursor_for_subsequent_pages() {
        let query = build_query(Some("cursor-1"));

        assert!(query.contains("$after: String!"));
        assert!(query.contains("reviews(first: 100, after: $after)"));
        assert!(query.contains("viewer { login }"));
        assert!(query.contains("commit { oid }"));
    }
}
