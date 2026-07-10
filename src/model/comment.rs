use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Which side of the diff a line comment belongs to
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
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

/// Lifecycle state of a local comment relative to the remote forge.
///
/// `LocalDraft` is editable in tuicr. `PushedDraft` and `Submitted` are locked:
/// they have been written to GitHub and edits/deletions in tuicr would diverge
/// from the remote. PR 5 introduces the field and the lock check; PR 6 wires
/// the transitions on successful submit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CommentLifecycleState {
    #[default]
    LocalDraft,
    PushedDraft,
    Submitted,
}

impl CommentLifecycleState {
    /// True for any state that has already been written to the remote forge.
    pub fn is_locked(self) -> bool {
        !matches!(self, CommentLifecycleState::LocalDraft)
    }
}

/// Classification of a review comment.
///
/// `None` is the out-of-the-box default: a comment with no type. It carries no
/// `[TYPE]` prefix on submit or markdown export and shows no badge in the UI.
/// Any other type is user-defined through the `comment_types` config (see #211)
/// and represented as `Custom`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum CommentType {
    /// No type — the default. Emits no prefix or badge anywhere.
    #[default]
    None,
    /// A user-configured type, identified by its id.
    Custom(String),
}

impl CommentType {
    /// Reserved id for the typeless [`CommentType::None`] variant. A config
    /// entry (or CLI `--type`) of `"none"` — or an empty value — resolves here.
    pub const NONE_ID: &'static str = "none";

    pub fn from_id(id: &str) -> Self {
        let trimmed = id.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case(Self::NONE_ID) {
            Self::None
        } else {
            Self::Custom(trimmed.to_string())
        }
    }

    pub fn id(&self) -> &str {
        match self {
            CommentType::None => Self::NONE_ID,
            CommentType::Custom(id) => id.as_str(),
        }
    }

    pub fn as_str(&self) -> String {
        self.id().to_ascii_uppercase()
    }

    /// True for the typeless default. Callers use this to suppress the
    /// `[TYPE]` prefix / badge for untyped comments.
    pub fn is_none(&self) -> bool {
        matches!(self, CommentType::None)
    }
}

impl Serialize for CommentType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.id())
    }
}

impl<'de> Deserialize<'de> for CommentType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Ok(Self::from_id(&value))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineContext {
    pub new_line: Option<u32>,
    pub old_line: Option<u32>,
    pub content: String,
}

/// Default author used when a comment is created or deserialized without
/// an explicit author. Distinguishes human-authored comments from agent /
/// remote comments which set their own author string.
pub const DEFAULT_AUTHOR: &str = "user";

fn default_author() -> String {
    DEFAULT_AUTHOR.to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Who wrote this comment. Free-form; agents pass `--username "Claude …"`,
    /// humans default to `"user"`. Old session JSON predates this field and
    /// rehydrates as `"user"`.
    #[serde(default = "default_author")]
    pub author: String,
    /// Where this comment sits in its remote forge lifecycle. Old session
    /// JSON predates this field and rehydrates as `LocalDraft`.
    #[serde(default)]
    pub lifecycle_state: CommentLifecycleState,
    /// Remote review ID this comment belongs to once submitted/pushed.
    /// `None` while still `LocalDraft`.
    #[serde(default)]
    pub remote_review_id: Option<String>,
    /// Remote review-comment ID once GitHub assigns one. Only meaningful for
    /// inline comments; review-level / summary comments don't get one.
    #[serde(default)]
    pub remote_comment_id: Option<String>,
    /// The commit SHA this comment was made against, when it was created
    /// while the inline commit selector showed exactly one commit. `None`
    /// for review-level comments, for comments made against the full
    /// cumulative diff (all commits selected), and for legacy session JSON
    /// predating this field. Filtering by the active commit selection hides
    /// comments whose `commit_id` does not intersect the selection; `None`
    /// comments are always shown.
    #[serde(default)]
    pub commit_id: Option<String>,
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
            author: default_author(),
            lifecycle_state: CommentLifecycleState::default(),
            remote_review_id: None,
            remote_comment_id: None,
            commit_id: None,
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
            author: default_author(),
            lifecycle_state: CommentLifecycleState::default(),
            remote_review_id: None,
            remote_comment_id: None,
            commit_id: None,
        }
    }

    /// Builder: set the author and return self. Useful at the boundary where
    /// we know the username (TUI startup config, CLI `--username` flag) so
    /// the existing `Comment::new` call sites can stay untouched.
    pub fn with_author(mut self, author: impl Into<String>) -> Self {
        self.author = author.into();
        self
    }

    /// Builder: set `commit_id` and return self. Called by
    /// `App::save_comment` when the inline commit selector shows exactly
    /// one commit, so the comment is scoped to that commit and hidden when
    /// a different commit (or subset) is selected.
    pub fn with_commit_id(mut self, commit_id: impl Into<String>) -> Self {
        self.commit_id = Some(commit_id.into());
        self
    }

    /// True if this comment has been pushed/submitted to the forge and is
    /// therefore locked from local edits/deletions.
    pub fn is_locked(&self) -> bool {
        self.lifecycle_state.is_locked()
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
        fn comment_type_serializes_custom_type_as_string() {
            let comment_type = CommentType::from_id("question");
            let json = serde_json::to_string(&comment_type).unwrap();
            assert_eq!(json, "\"question\"");
        }

        #[test]
        fn comment_type_deserializes_custom_type_from_string() {
            let json = "\"question\"";
            let comment_type: CommentType = serde_json::from_str(json).unwrap();
            assert_eq!(comment_type.id(), "question");
        }

        #[test]
        fn comment_type_defaults_to_none() {
            assert_eq!(CommentType::default(), CommentType::None);
            assert!(CommentType::default().is_none());
        }

        #[test]
        fn comment_type_from_id_maps_none_and_empty_to_none() {
            assert!(CommentType::from_id("none").is_none());
            assert!(CommentType::from_id("NONE").is_none());
            assert!(CommentType::from_id("").is_none());
            assert!(CommentType::from_id("   ").is_none());
            assert!(!CommentType::from_id("issue").is_none());
        }

        #[test]
        fn comment_type_none_roundtrips_via_serde() {
            let json = serde_json::to_string(&CommentType::None).unwrap();
            assert_eq!(json, "\"none\"");
            let restored: CommentType = serde_json::from_str(&json).unwrap();
            assert_eq!(restored, CommentType::None);
        }

        #[test]
        fn new_creates_comment_without_line_range() {
            let comment = Comment::new(
                "Test comment".to_string(),
                CommentType::from_id("note"),
                Some(LineSide::New),
            );
            assert!(comment.line_range.is_none());
            assert_eq!(comment.content, "Test comment");
            assert_eq!(comment.comment_type, CommentType::from_id("note"));
            assert_eq!(comment.side, Some(LineSide::New));
        }

        #[test]
        fn new_with_range_creates_comment_with_line_range() {
            let range = LineRange::new(10, 15);
            let comment = Comment::new_with_range(
                "Range comment".to_string(),
                CommentType::from_id("issue"),
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
                CommentType::from_id("note"),
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

        #[test]
        fn should_default_lifecycle_state_to_local_draft_for_new_comment() {
            // given/when
            let comment = Comment::new("hi".to_string(), CommentType::from_id("note"), None);
            // then
            assert_eq!(comment.lifecycle_state, CommentLifecycleState::LocalDraft);
            assert!(!comment.is_locked());
            assert!(comment.remote_review_id.is_none());
            assert!(comment.remote_comment_id.is_none());
        }

        #[test]
        fn should_report_pushed_and_submitted_comments_as_locked() {
            // given
            let mut pushed = Comment::new("p".to_string(), CommentType::from_id("note"), None);
            pushed.lifecycle_state = CommentLifecycleState::PushedDraft;
            let mut submitted = Comment::new("s".to_string(), CommentType::from_id("note"), None);
            submitted.lifecycle_state = CommentLifecycleState::Submitted;
            // then
            assert!(pushed.is_locked());
            assert!(submitted.is_locked());
        }

        #[test]
        fn should_roundtrip_lifecycle_fields_via_serde() {
            // given
            let mut original =
                Comment::new("body".to_string(), CommentType::from_id("issue"), None);
            original.lifecycle_state = CommentLifecycleState::Submitted;
            original.remote_review_id = Some("R_kgDOEx".to_string());
            original.remote_comment_id = Some("RC_kgDOEx".to_string());
            // when
            let json = serde_json::to_string(&original).unwrap();
            let restored: Comment = serde_json::from_str(&json).unwrap();
            // then
            assert_eq!(restored.lifecycle_state, CommentLifecycleState::Submitted);
            assert_eq!(restored.remote_review_id.as_deref(), Some("R_kgDOEx"));
            assert_eq!(restored.remote_comment_id.as_deref(), Some("RC_kgDOEx"));
        }

        #[test]
        fn should_default_lifecycle_fields_for_pre_pr5_comment_json() {
            // given — JSON saved before PR 5 introduced lifecycle fields.
            let json = r#"{
                "id": "legacy",
                "content": "pre-pr5",
                "comment_type": "note",
                "created_at": "2024-01-01T00:00:00Z",
                "line_context": null
            }"#;
            // when
            let comment: Comment =
                serde_json::from_str(json).expect("pre-PR-5 comment JSON should parse");
            // then
            assert_eq!(comment.lifecycle_state, CommentLifecycleState::LocalDraft);
            assert!(comment.remote_review_id.is_none());
            assert!(comment.remote_comment_id.is_none());
            // and the rest of the comment survived
            assert_eq!(comment.id, "legacy");
            assert_eq!(comment.content, "pre-pr5");
        }
    }
}
