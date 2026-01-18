use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Which side of the diff a line comment belongs to
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LineSide {
    /// Comment on a deleted line (keyed by old_lineno)
    Old,
    /// Comment on an added or context line (keyed by new_lineno)
    #[default]
    New,
}

/// A range of lines for a comment (inclusive)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineRange {
    pub start: u32,
    pub end: u32,
}

impl LineRange {
    /// Create a new line range
    pub fn new(start: u32, end: u32) -> Self {
        Self {
            start: start.min(end),
            end: start.max(end),
        }
    }

    /// Create a single-line range
    pub fn single(line: u32) -> Self {
        Self {
            start: line,
            end: line,
        }
    }

    /// Check if this is a single-line range
    pub fn is_single(&self) -> bool {
        self.start == self.end
    }

    /// Check if this range contains a given line
    pub fn contains(&self, line: u32) -> bool {
        line >= self.start && line <= self.end
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CommentType {
    Note,
    Suggestion,
    Issue,
    Praise,
}

impl CommentType {
    pub fn as_str(&self) -> &'static str {
        match self {
            CommentType::Note => "NOTE",
            CommentType::Suggestion => "SUGGESTION",
            CommentType::Issue => "ISSUE",
            CommentType::Praise => "PRAISE",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineContext {
    pub new_line: Option<u32>,
    pub old_line: Option<u32>,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Comment {
    pub id: String,
    pub content: String,
    pub comment_type: CommentType,
    pub created_at: DateTime<Utc>,
    pub line_context: Option<LineContext>,
    /// Which side of the diff this comment belongs to (for line comments)
    /// None for file-level comments, defaults to New for backward compatibility
    #[serde(default)]
    pub side: Option<LineSide>,
    /// Line range for multi-line comments (for line comments)
    /// None for file-level comments or single-line comments (backward compatibility)
    #[serde(default)]
    pub line_range: Option<LineRange>,
}

impl Comment {
    pub fn new(content: String, comment_type: CommentType, side: Option<LineSide>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            content,
            comment_type,
            created_at: Utc::now(),
            line_context: None,
            side,
            line_range: None,
        }
    }

    /// Create a new comment with a line range
    pub fn new_with_range(
        content: String,
        comment_type: CommentType,
        side: Option<LineSide>,
        line_range: LineRange,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            content,
            comment_type,
            created_at: Utc::now(),
            line_context: None,
            side,
            line_range: Some(line_range),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod line_range_tests {
        use super::*;

        #[test]
        fn new_creates_range_with_correct_bounds() {
            let range = LineRange::new(10, 20);
            assert_eq!(range.start, 10);
            assert_eq!(range.end, 20);
        }

        #[test]
        fn new_normalizes_reversed_bounds() {
            // When start > end, new() should normalize them
            let range = LineRange::new(20, 10);
            assert_eq!(range.start, 10);
            assert_eq!(range.end, 20);
        }

        #[test]
        fn single_creates_single_line_range() {
            let range = LineRange::single(42);
            assert_eq!(range.start, 42);
            assert_eq!(range.end, 42);
        }

        #[test]
        fn is_single_returns_true_for_single_line() {
            let range = LineRange::single(10);
            assert!(range.is_single());
        }

        #[test]
        fn is_single_returns_false_for_multi_line() {
            let range = LineRange::new(10, 15);
            assert!(!range.is_single());
        }

        #[test]
        fn contains_returns_true_for_start_line() {
            let range = LineRange::new(10, 20);
            assert!(range.contains(10));
        }

        #[test]
        fn contains_returns_true_for_end_line() {
            let range = LineRange::new(10, 20);
            assert!(range.contains(20));
        }

        #[test]
        fn contains_returns_true_for_middle_line() {
            let range = LineRange::new(10, 20);
            assert!(range.contains(15));
        }

        #[test]
        fn contains_returns_false_for_line_before_range() {
            let range = LineRange::new(10, 20);
            assert!(!range.contains(5));
        }

        #[test]
        fn contains_returns_false_for_line_after_range() {
            let range = LineRange::new(10, 20);
            assert!(!range.contains(25));
        }

        #[test]
        fn single_line_range_contains_only_that_line() {
            let range = LineRange::single(42);
            assert!(!range.contains(41));
            assert!(range.contains(42));
            assert!(!range.contains(43));
        }

        #[test]
        fn line_range_serializes_correctly() {
            let range = LineRange::new(10, 20);
            let json = serde_json::to_string(&range).unwrap();
            assert!(json.contains("\"start\":10"));
            assert!(json.contains("\"end\":20"));
        }

        #[test]
        fn line_range_deserializes_correctly() {
            let json = r#"{"start":10,"end":20}"#;
            let range: LineRange = serde_json::from_str(json).unwrap();
            assert_eq!(range.start, 10);
            assert_eq!(range.end, 20);
        }
    }

    mod comment_tests {
        use super::*;

        #[test]
        fn new_creates_comment_without_line_range() {
            let comment = Comment::new(
                "Test comment".to_string(),
                CommentType::Note,
                Some(LineSide::New),
            );
            assert!(comment.line_range.is_none());
            assert_eq!(comment.content, "Test comment");
            assert_eq!(comment.comment_type, CommentType::Note);
            assert_eq!(comment.side, Some(LineSide::New));
        }

        #[test]
        fn new_with_range_creates_comment_with_line_range() {
            let range = LineRange::new(10, 15);
            let comment = Comment::new_with_range(
                "Range comment".to_string(),
                CommentType::Issue,
                Some(LineSide::Old),
                range,
            );
            assert!(comment.line_range.is_some());
            let stored_range = comment.line_range.unwrap();
            assert_eq!(stored_range.start, 10);
            assert_eq!(stored_range.end, 15);
            assert_eq!(comment.side, Some(LineSide::Old));
        }

        #[test]
        fn comment_with_line_range_serializes_correctly() {
            let range = LineRange::new(10, 15);
            let comment = Comment::new_with_range(
                "Test".to_string(),
                CommentType::Note,
                Some(LineSide::New),
                range,
            );
            let json = serde_json::to_string(&comment).unwrap();
            assert!(json.contains("\"line_range\""));
            assert!(json.contains("\"start\":10"));
            assert!(json.contains("\"end\":15"));
        }

        #[test]
        fn comment_without_line_range_deserializes_with_none() {
            // Simulate old format without line_range field
            let json = r#"{
                "id": "test-id",
                "content": "Test comment",
                "comment_type": "note",
                "created_at": "2024-01-01T00:00:00Z",
                "line_context": null,
                "side": "new"
            }"#;
            let comment: Comment = serde_json::from_str(json).unwrap();
            assert!(comment.line_range.is_none());
            assert_eq!(comment.content, "Test comment");
        }

        #[test]
        fn comment_with_line_range_deserializes_correctly() {
            let json = r#"{
                "id": "test-id",
                "content": "Range comment",
                "comment_type": "issue",
                "created_at": "2024-01-01T00:00:00Z",
                "line_context": null,
                "side": "old",
                "line_range": {"start": 10, "end": 15}
            }"#;
            let comment: Comment = serde_json::from_str(json).unwrap();
            assert!(comment.line_range.is_some());
            let range = comment.line_range.unwrap();
            assert_eq!(range.start, 10);
            assert_eq!(range.end, 15);
        }
    }
}
