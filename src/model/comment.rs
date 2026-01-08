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
        }
    }
}
