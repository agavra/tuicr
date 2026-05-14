//! GraphQL parsing for GitHub review threads.
//!
//! REST exposes review comments but not the resolved/outdated thread state
//! that v1 requires for `:comments unresolved`. The GitHub GraphQL API
//! groups comments into `reviewThreads` with `isResolved` and `isOutdated`
//! flags. We fetch via `gh api graphql` so authentication, host, and
//! enterprise routing reuse what `gh` already knows.
//!
//! Payload shape (only the fields we read are documented here):
//!
//! In GitHub's GraphQL schema, anchor fields (`path`, `line`, `originalLine`,
//! `diffSide`) live on `PullRequestReviewThread`, NOT on each comment node.
//! An earlier version of this code put them on the comment nodes; GraphQL
//! rejected the query with "Field 'side' doesn't exist on type
//! 'PullRequestReviewComment'". Hence this shape:
//!
//! ```json
//! {
//!   "data": {
//!     "repository": {
//!       "pullRequest": {
//!         "reviewThreads": {
//!           "pageInfo": { "hasNextPage": false, "endCursor": null },
//!           "nodes": [
//!             {
//!               "id": "PRRT_kw...",
//!               "isResolved": false,
//!               "isOutdated": false,
//!               "path": "src/lib.rs",
//!               "line": 42,
//!               "originalLine": 42,
//!               "diffSide": "RIGHT",
//!               "comments": {
//!                 "nodes": [
//!                   {
//!                     "id": "PRRC_kw...",
//!                     "body": "Can this be simplified?",
//!                     "author": { "login": "alice" },
//!                     "createdAt": "2026-05-12T18:30:00Z",
//!                     "url": "https://github.com/agavra/tuicr/pull/125#discussion_r1"
//!                   }
//!                 ]
//!               }
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
use crate::forge::remote_comments::{RemoteCommentSide, RemoteReviewComment, RemoteReviewThread};

#[derive(Debug, Deserialize)]
struct GhAuthor {
    #[serde(default)]
    login: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhReviewComment {
    id: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    author: Option<GhAuthor>,
    #[serde(default)]
    created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    reply_to: Option<GhReplyRef>,
}

#[derive(Debug, Deserialize)]
struct GhReplyRef {
    id: String,
}

#[derive(Debug, Deserialize)]
struct GhCommentsConn {
    #[serde(default)]
    nodes: Vec<GhReviewComment>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhReviewThread {
    id: String,
    #[serde(default)]
    is_resolved: bool,
    #[serde(default)]
    is_outdated: bool,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    line: Option<u32>,
    #[serde(default)]
    original_line: Option<u32>,
    #[serde(default)]
    diff_side: Option<String>,
    #[serde(default)]
    comments: Option<GhCommentsConn>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GhPageInfo {
    #[serde(default)]
    pub has_next_page: bool,
    #[serde(default)]
    pub end_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhReviewThreadsConn {
    #[serde(default)]
    page_info: Option<GhPageInfo>,
    #[serde(default)]
    nodes: Vec<GhReviewThread>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhPullRequest {
    #[serde(default)]
    review_threads: Option<GhReviewThreadsConn>,
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

/// Outcome of parsing a single GraphQL page.
#[derive(Debug)]
pub(crate) struct ParsedPage {
    pub threads: Vec<RemoteReviewThread>,
    pub page_info: Option<GhPageInfo>,
}

/// Parse one GraphQL response page into domain threads + pagination info.
/// Errors only on malformed JSON; missing optional fields are tolerated.
pub(crate) fn parse_graphql_page(json: &str) -> Result<ParsedPage> {
    let response: GhResponse = serde_json::from_str(json).map_err(|e| {
        TuicrError::Forge(format!(
            "Failed to parse GitHub review threads response: {e}"
        ))
    })?;

    let conn = response
        .data
        .and_then(|d| d.repository)
        .and_then(|r| r.pull_request)
        .and_then(|p| p.review_threads);

    let Some(conn) = conn else {
        return Ok(ParsedPage {
            threads: Vec::new(),
            page_info: None,
        });
    };

    let page_info = conn.page_info;
    let mut threads = Vec::with_capacity(conn.nodes.len());
    for raw in conn.nodes {
        threads.push(convert_thread(raw));
    }
    Ok(ParsedPage { threads, page_info })
}

fn convert_thread(raw: GhReviewThread) -> RemoteReviewThread {
    let comments_conn = raw.comments.unwrap_or(GhCommentsConn { nodes: Vec::new() });
    let comments: Vec<RemoteReviewComment> = comments_conn
        .nodes
        .into_iter()
        .map(convert_comment)
        .collect();

    let side = raw
        .diff_side
        .as_deref()
        .map(RemoteCommentSide::parse)
        .unwrap_or(RemoteCommentSide::Right);

    // If `line` is null on an outdated thread, fall back to `originalLine`
    // so we still know roughly where the thread was anchored. The
    // `is_outdated` flag drives muted styling + suppression from the
    // default `:comments unresolved` view.
    let line = raw.line.or(raw.original_line);

    RemoteReviewThread {
        id: raw.id,
        path: raw.path.unwrap_or_default(),
        line,
        side,
        is_resolved: raw.is_resolved,
        is_outdated: raw.is_outdated,
        comments,
    }
}

fn convert_comment(raw: GhReviewComment) -> RemoteReviewComment {
    RemoteReviewComment {
        id: raw.id,
        author: raw.author.and_then(|a| a.login),
        body: raw.body,
        created_at: raw.created_at,
        in_reply_to: raw.reply_to.map(|r| r.id),
        url: raw.url.unwrap_or_default(),
    }
}

/// Build the GraphQL query string parameterized by repository + PR number.
pub(crate) fn build_query(after_cursor: Option<&str>) -> String {
    let cursor_arg = match after_cursor {
        Some(_) => ", after: $after",
        None => "",
    };
    format!(
        r#"query($owner: String!, $name: String!, $number: Int!{cursor_param}) {{
  repository(owner: $owner, name: $name) {{
    pullRequest(number: $number) {{
      reviewThreads(first: 100{cursor_arg}) {{
        pageInfo {{ hasNextPage endCursor }}
        nodes {{
          id
          isResolved
          isOutdated
          path
          line
          originalLine
          diffSide
          comments(first: 100) {{
            nodes {{
              id
              body
              author {{ login }}
              createdAt
              url
              replyTo {{ id }}
            }}
          }}
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

    const SINGLE_THREAD_JSON: &str = r##"{
        "data": {
            "repository": {
                "pullRequest": {
                    "reviewThreads": {
                        "pageInfo": { "hasNextPage": false, "endCursor": null },
                        "nodes": [
                            {
                                "id": "PRRT_1",
                                "isResolved": false,
                                "isOutdated": false,
                                "path": "src/lib.rs",
                                "line": 42,
                                "originalLine": 42,
                                "diffSide": "RIGHT",
                                "comments": {
                                    "nodes": [
                                        {
                                            "id": "PRRC_1",
                                            "body": "Can this be simplified?",
                                            "author": { "login": "alice" },
                                            "createdAt": "2026-05-12T18:30:00Z",
                                            "url": "https://github.com/agavra/tuicr/pull/125#discussion_r1"
                                        }
                                    ]
                                }
                            }
                        ]
                    }
                }
            }
        }
    }"##;

    const MULTI_COMMENT_THREAD_JSON: &str = r##"{
        "data": {
            "repository": {
                "pullRequest": {
                    "reviewThreads": {
                        "pageInfo": { "hasNextPage": false, "endCursor": null },
                        "nodes": [
                            {
                                "id": "PRRT_1",
                                "isResolved": false,
                                "isOutdated": false,
                                "path": "src/lib.rs",
                                "line": 42,
                                "diffSide": "RIGHT",
                                "comments": {
                                    "nodes": [
                                        {
                                            "id": "PRRC_1",
                                            "body": "Root",
                                            "author": { "login": "alice" },
                                            "url": "https://example.com/1"
                                        },
                                        {
                                            "id": "PRRC_2",
                                            "body": "Reply 1",
                                            "author": { "login": "bob" },
                                            "url": "https://example.com/2",
                                            "replyTo": { "id": "PRRC_1" }
                                        },
                                        {
                                            "id": "PRRC_3",
                                            "body": "Reply 2",
                                            "author": { "login": "alice" },
                                            "url": "https://example.com/3",
                                            "replyTo": { "id": "PRRC_1" }
                                        }
                                    ]
                                }
                            }
                        ]
                    }
                }
            }
        }
    }"##;

    const RESOLVED_THREAD_JSON: &str = r##"{
        "data": {
            "repository": {
                "pullRequest": {
                    "reviewThreads": {
                        "nodes": [
                            {
                                "id": "PRRT_resolved",
                                "isResolved": true,
                                "isOutdated": false,
                                "path": "src/lib.rs",
                                "line": 7,
                                "diffSide": "RIGHT",
                                "comments": {
                                    "nodes": [
                                        {
                                            "id": "PRRC_old",
                                            "body": "Old resolved comment.",
                                            "author": { "login": "bob" },
                                            "url": "https://example.com/r"
                                        }
                                    ]
                                }
                            }
                        ]
                    }
                }
            }
        }
    }"##;

    const OUTDATED_THREAD_JSON: &str = r##"{
        "data": {
            "repository": {
                "pullRequest": {
                    "reviewThreads": {
                        "nodes": [
                            {
                                "id": "PRRT_outdated",
                                "isResolved": false,
                                "isOutdated": true,
                                "path": "src/lib.rs",
                                "line": null,
                                "originalLine": 19,
                                "diffSide": "LEFT",
                                "comments": {
                                    "nodes": [
                                        {
                                            "id": "PRRC_old",
                                            "body": "Comment on a line that has moved.",
                                            "author": { "login": "alice" },
                                            "url": "https://example.com/o"
                                        }
                                    ]
                                }
                            }
                        ]
                    }
                }
            }
        }
    }"##;

    const CROSS_FILE_THREADS_JSON: &str = r##"{
        "data": {
            "repository": {
                "pullRequest": {
                    "reviewThreads": {
                        "nodes": [
                            {
                                "id": "PRRT_a",
                                "isResolved": false,
                                "isOutdated": false,
                                "path": "src/lib.rs",
                                "line": 10,
                                "diffSide": "RIGHT",
                                "comments": {
                                    "nodes": [
                                        {
                                            "id": "PRRC_a",
                                            "body": "Comment in lib",
                                            "author": { "login": "alice" },
                                            "url": "https://example.com/a"
                                        }
                                    ]
                                }
                            },
                            {
                                "id": "PRRT_b",
                                "isResolved": false,
                                "isOutdated": false,
                                "path": "src/main.rs",
                                "line": 5,
                                "diffSide": "RIGHT",
                                "comments": {
                                    "nodes": [
                                        {
                                            "id": "PRRC_b",
                                            "body": "Comment in main",
                                            "author": { "login": "bob" },
                                            "url": "https://example.com/b"
                                        }
                                    ]
                                }
                            }
                        ]
                    }
                }
            }
        }
    }"##;

    const EMPTY_THREADS_JSON: &str = r##"{
        "data": {
            "repository": {
                "pullRequest": {
                    "reviewThreads": {
                        "pageInfo": { "hasNextPage": false, "endCursor": null },
                        "nodes": []
                    }
                }
            }
        }
    }"##;

    const NULL_REPO_JSON: &str = r##"{ "data": { "repository": null } }"##;

    #[test]
    fn should_parse_single_thread_with_one_comment() {
        // given
        let json = SINGLE_THREAD_JSON;
        // when
        let parsed = parse_graphql_page(json).unwrap();
        // then
        assert_eq!(parsed.threads.len(), 1);
        let thread = &parsed.threads[0];
        assert_eq!(thread.id, "PRRT_1");
        assert_eq!(thread.path, "src/lib.rs");
        assert_eq!(thread.line, Some(42));
        assert_eq!(thread.side, RemoteCommentSide::Right);
        assert!(!thread.is_resolved);
        assert!(!thread.is_outdated);
        assert_eq!(thread.comments.len(), 1);
        assert_eq!(thread.comments[0].author.as_deref(), Some("alice"));
        assert_eq!(thread.comments[0].body, "Can this be simplified?");
    }

    #[test]
    fn should_parse_multi_comment_thread_with_replies() {
        // given/when
        let parsed = parse_graphql_page(MULTI_COMMENT_THREAD_JSON).unwrap();
        // then
        let thread = &parsed.threads[0];
        assert_eq!(thread.comments.len(), 3);
        assert_eq!(thread.comments[0].in_reply_to, None);
        assert_eq!(thread.comments[1].in_reply_to.as_deref(), Some("PRRC_1"));
        assert_eq!(thread.comments[2].in_reply_to.as_deref(), Some("PRRC_1"));
        // root() returns the first; replies() returns the rest
        assert_eq!(thread.root().unwrap().body, "Root");
        let replies: Vec<&str> = thread.replies().map(|c| c.body.as_str()).collect();
        assert_eq!(replies, vec!["Reply 1", "Reply 2"]);
    }

    #[test]
    fn should_parse_resolved_thread_flag() {
        // given/when
        let parsed = parse_graphql_page(RESOLVED_THREAD_JSON).unwrap();
        // then
        let thread = &parsed.threads[0];
        assert!(thread.is_resolved);
        assert!(!thread.is_outdated);
    }

    #[test]
    fn should_parse_outdated_thread_flag_and_fall_back_to_original_line() {
        // given/when
        let parsed = parse_graphql_page(OUTDATED_THREAD_JSON).unwrap();
        // then
        let thread = &parsed.threads[0];
        assert!(thread.is_outdated);
        // line is null but originalLine = 19 — we surface originalLine so
        // the renderer can still place the comment at its last known anchor.
        assert_eq!(thread.line, Some(19));
        assert_eq!(thread.side, RemoteCommentSide::Left);
    }

    #[test]
    fn should_parse_cross_file_threads_preserving_paths() {
        // given/when
        let parsed = parse_graphql_page(CROSS_FILE_THREADS_JSON).unwrap();
        // then
        assert_eq!(parsed.threads.len(), 2);
        let paths: Vec<&str> = parsed.threads.iter().map(|t| t.path.as_str()).collect();
        assert_eq!(paths, vec!["src/lib.rs", "src/main.rs"]);
    }

    #[test]
    fn should_parse_empty_threads_array_without_error() {
        // given/when
        let parsed = parse_graphql_page(EMPTY_THREADS_JSON).unwrap();
        // then
        assert!(parsed.threads.is_empty());
        assert!(parsed.page_info.is_some());
        assert!(!parsed.page_info.as_ref().unwrap().has_next_page);
    }

    #[test]
    fn should_tolerate_missing_repository_object() {
        // given a PR with no review-thread payload reachable (rare)
        // when
        let parsed = parse_graphql_page(NULL_REPO_JSON).unwrap();
        // then — no error, no threads
        assert!(parsed.threads.is_empty());
        assert!(parsed.page_info.is_none());
    }

    #[test]
    fn should_error_on_malformed_json() {
        // given
        let bad = "not json";
        // when
        let err = parse_graphql_page(bad).unwrap_err();
        // then
        let msg = err.to_string();
        assert!(
            msg.contains("Failed to parse GitHub review threads response"),
            "unexpected error message: {msg}"
        );
    }

    #[test]
    fn should_build_query_without_cursor_for_first_page() {
        // given/when
        let q = build_query(None);
        // then
        assert!(q.contains("reviewThreads(first: 100)"));
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
