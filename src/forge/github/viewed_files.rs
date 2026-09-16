//! GraphQL parsing for GitHub's per-file viewed state.
//!
//! The "Viewed" checkbox on a pull request file is per-viewer state, and REST
//! does not expose it at all — `viewerViewedState` lives on
//! `PullRequestChangedFile` in the GraphQL schema, so this reads it the same
//! way [`super::review_threads`] reads thread state.
//!
//! Payload shape (only the fields we read):
//!
//! ```json
//! {
//!   "data": {
//!     "repository": {
//!       "pullRequest": {
//!         "files": {
//!           "pageInfo": { "hasNextPage": false, "endCursor": null },
//!           "nodes": [
//!             { "path": "src/lib.rs", "viewerViewedState": "VIEWED" },
//!             { "path": "src/main.rs", "viewerViewedState": "UNVIEWED" }
//!           ]
//!         }
//!       }
//!     }
//!   }
//! }
//! ```

use std::path::PathBuf;

use serde::Deserialize;

use crate::error::{Result, TuicrError};

use super::review_threads::GhPageInfo;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhChangedFile {
    #[serde(default)]
    path: String,
    #[serde(default)]
    viewer_viewed_state: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhFilesConn {
    #[serde(default)]
    page_info: Option<GhPageInfo>,
    #[serde(default)]
    nodes: Vec<GhChangedFile>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhPullRequest {
    #[serde(default)]
    files: Option<GhFilesConn>,
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
    /// Paths the viewer has marked viewed on this page.
    pub viewed: Vec<PathBuf>,
    pub page_info: Option<GhPageInfo>,
}

/// Parse one page into the viewed paths it reports plus pagination info.
/// Errors only on malformed JSON; missing optional fields are tolerated.
pub(crate) fn parse_graphql_page(json: &str) -> Result<ParsedPage> {
    let response: GhResponse = serde_json::from_str(json).map_err(|e| {
        TuicrError::Forge(format!("Failed to parse GitHub viewed-state response: {e}"))
    })?;

    let conn = response
        .data
        .and_then(|d| d.repository)
        .and_then(|r| r.pull_request)
        .and_then(|p| p.files);

    let Some(conn) = conn else {
        return Ok(ParsedPage {
            viewed: Vec::new(),
            page_info: None,
        });
    };

    let page_info = conn.page_info;
    let viewed = conn
        .nodes
        .into_iter()
        .filter(|file| {
            // `FileViewedState` is VIEWED | UNVIEWED | DISMISSED. DISMISSED
            // means "you viewed this, then it changed underneath you", which
            // is not a reviewed file any more.
            file.viewer_viewed_state.as_deref() == Some("VIEWED") && !file.path.is_empty()
        })
        .map(|file| PathBuf::from(file.path))
        .collect();

    Ok(ParsedPage { viewed, page_info })
}

/// Build the paged query. `after_cursor` continues a previous page.
pub(crate) fn build_query(after_cursor: Option<&str>) -> String {
    let (cursor_param, cursor_arg) = match after_cursor {
        Some(_) => (", $after: String!", ", after: $after"),
        None => ("", ""),
    };
    format!(
        r#"query($owner: String!, $name: String!, $number: Int!{cursor_param}) {{
  repository(owner: $owner, name: $name) {{
    pullRequest(number: $number) {{
      files(first: 100{cursor_arg}) {{
        pageInfo {{ hasNextPage endCursor }}
        nodes {{
          path
          viewerViewedState
        }}
      }}
    }}
  }}
}}"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE_JSON: &str = r##"{
  "data": {
    "repository": {
      "pullRequest": {
        "files": {
          "pageInfo": { "hasNextPage": true, "endCursor": "Y3Vyc29yOjI=" },
          "nodes": [
            { "path": "src/lib.rs", "viewerViewedState": "VIEWED" },
            { "path": "src/main.rs", "viewerViewedState": "UNVIEWED" },
            { "path": "README.md", "viewerViewedState": "DISMISSED" }
          ]
        }
      }
    }
  }
}"##;

    #[test]
    fn should_keep_only_files_the_viewer_marked_viewed() {
        // given/when
        let page = parse_graphql_page(PAGE_JSON).unwrap();

        // then — DISMISSED means the file changed after it was viewed, so it
        // is no longer reviewed.
        assert_eq!(page.viewed, vec![PathBuf::from("src/lib.rs")]);
    }

    #[test]
    fn should_report_pagination_info() {
        // given/when
        let page = parse_graphql_page(PAGE_JSON).unwrap();

        // then
        let info = page.page_info.expect("expected pageInfo");
        assert!(info.has_next_page);
        assert_eq!(info.end_cursor.as_deref(), Some("Y3Vyc29yOjI="));
    }

    #[test]
    fn should_tolerate_a_pull_request_with_no_files_connection() {
        // given — a PR the token cannot see returns nulls rather than an error
        let json = r##"{ "data": { "repository": { "pullRequest": null } } }"##;

        // when
        let page = parse_graphql_page(json).unwrap();

        // then
        assert!(page.viewed.is_empty());
        assert!(page.page_info.is_none());
    }

    #[test]
    fn should_declare_the_cursor_variable_only_when_paging() {
        // given/when
        let first = build_query(None);
        let next = build_query(Some("Y3Vyc29yOjI="));

        // then — GraphQL rejects a declared-but-unused variable, and an
        // undeclared one used in the body.
        assert!(!first.contains("$after"));
        assert!(next.contains("$after: String!"));
        assert!(next.contains("after: $after"));
    }
}
