//! GitHub-specific submit-time concerns.
//!
//! Right now this is just the JSON payload shape for `POST repos/<owner>/<repo>/pulls/<number>/reviews`.
//! Future entries (e.g. error mapping, response parsing) belong here.

use serde_json::{Map, Value};

use crate::forge::submit::{InlineComment, SubmitEvent};

/// Build the JSON body for the GitHub create-review API call.
///
/// `commit_id` is the head SHA the comments were authored against (NOT
/// necessarily the PR's current head — see the stale-head warning in the
/// confirmation modal). `event` controls the published-vs-draft behavior:
/// `SubmitEvent::Draft` omits the `event` field entirely so GitHub creates a
/// PENDING review, per the spec.
pub fn build_review_payload(
    commit_id: &str,
    body: &str,
    event: SubmitEvent,
    comments: &[InlineComment],
) -> Value {
    let mut payload = Map::new();
    payload.insert("commit_id".to_string(), Value::from(commit_id));
    payload.insert("body".to_string(), Value::from(body));
    if let Some(event_str) = event.github_event() {
        payload.insert("event".to_string(), Value::from(event_str));
    }
    payload.insert(
        "comments".to_string(),
        Value::Array(comments.iter().map(inline_comment_json).collect()),
    );
    Value::Object(payload)
}

fn inline_comment_json(comment: &InlineComment) -> Value {
    let mut obj = Map::new();
    obj.insert(
        "path".to_string(),
        Value::from(comment.path.to_string_lossy().to_string()),
    );
    obj.insert("line".to_string(), Value::from(comment.line));
    obj.insert("side".to_string(), Value::from(comment.side.as_str()));
    obj.insert("body".to_string(), Value::from(comment.body.clone()));
    if let Some(start) = comment.start_line {
        obj.insert("start_line".to_string(), Value::from(start));
    }
    if let Some(start_side) = comment.start_side {
        obj.insert("start_side".to_string(), Value::from(start_side.as_str()));
    }
    Value::Object(obj)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::submit::GhSide;
    use std::path::PathBuf;

    fn inline(line: u32) -> InlineComment {
        InlineComment {
            path: PathBuf::from("src/lib.rs"),
            line,
            side: GhSide::Right,
            start_line: None,
            start_side: None,
            body: "**[ISSUE]** boom".to_string(),
            comment_id: "test-comment-id".to_string(),
        }
    }

    #[test]
    fn should_emit_commit_id_body_event_and_comments_for_comment_event() {
        let payload =
            build_review_payload("abc1234", "body text", SubmitEvent::Comment, &[inline(42)]);
        assert_eq!(payload["commit_id"], "abc1234");
        assert_eq!(payload["body"], "body text");
        assert_eq!(payload["event"], "COMMENT");
        let comments = payload["comments"].as_array().expect("comments array");
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0]["path"], "src/lib.rs");
        assert_eq!(comments[0]["line"], 42);
        assert_eq!(comments[0]["side"], "RIGHT");
        assert_eq!(comments[0]["body"], "**[ISSUE]** boom");
    }

    #[test]
    fn should_emit_event_approve_for_approve_submission() {
        let payload = build_review_payload("sha", "body", SubmitEvent::Approve, &[]);
        assert_eq!(payload["event"], "APPROVE");
    }

    #[test]
    fn should_emit_event_request_changes_for_request_changes_submission() {
        let payload = build_review_payload("sha", "body", SubmitEvent::RequestChanges, &[]);
        assert_eq!(payload["event"], "REQUEST_CHANGES");
    }

    #[test]
    fn should_omit_event_for_draft_submission() {
        let payload = build_review_payload("sha", "body", SubmitEvent::Draft, &[]);
        assert!(
            payload
                .as_object()
                .is_some_and(|m| !m.contains_key("event"))
        );
    }

    #[test]
    fn should_include_start_line_and_start_side_for_multi_line_comment() {
        let inline = InlineComment {
            path: PathBuf::from("src/main.rs"),
            line: 20,
            side: GhSide::Left,
            start_line: Some(15),
            start_side: Some(GhSide::Left),
            body: "ranged".to_string(),
            comment_id: "test-comment-id".to_string(),
        };
        let payload = build_review_payload("sha", "", SubmitEvent::Comment, &[inline]);
        let comment = &payload["comments"][0];
        assert_eq!(comment["start_line"], 15);
        assert_eq!(comment["start_side"], "LEFT");
        assert_eq!(comment["line"], 20);
        assert_eq!(comment["side"], "LEFT");
    }

    #[test]
    fn should_emit_empty_comments_array_when_none_supplied() {
        let payload = build_review_payload("sha", "body", SubmitEvent::Comment, &[]);
        assert_eq!(payload["comments"].as_array().map(|a| a.len()), Some(0));
    }

    #[test]
    fn should_use_left_side_when_inline_targets_old_side() {
        let inline = InlineComment {
            path: PathBuf::from("a.rs"),
            line: 7,
            side: GhSide::Left,
            start_line: None,
            start_side: None,
            body: String::new(),
            comment_id: "test-comment-id".to_string(),
        };
        let payload = build_review_payload("sha", "", SubmitEvent::Comment, &[inline]);
        assert_eq!(payload["comments"][0]["side"], "LEFT");
    }
}
