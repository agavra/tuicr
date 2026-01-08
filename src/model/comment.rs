use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

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
}

impl Comment {
    pub fn new(content: String, comment_type: CommentType) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            content,
            comment_type,
            created_at: Utc::now(),
            line_context: None,
        }
    }
}
