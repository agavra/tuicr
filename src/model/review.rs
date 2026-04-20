use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use super::comment::Comment;
use super::diff_types::FileStatus;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileReview {
    pub path: PathBuf,
    pub reviewed: bool,
    pub status: FileStatus,
    pub file_comments: Vec<Comment>,
    pub line_comments: HashMap<u32, Vec<Comment>>,
}

impl FileReview {
    pub fn new(path: PathBuf, status: FileStatus) -> Self {
        Self {
            path,
            reviewed: false,
            status,
            file_comments: Vec::new(),
            line_comments: HashMap::new(),
        }
    }

    pub fn comment_count(&self) -> usize {
        self.file_comments.len() + self.line_comments.values().map(|v| v.len()).sum::<usize>()
    }

    pub fn add_file_comment(&mut self, comment: Comment) {
        self.file_comments.push(comment);
    }

    pub fn add_line_comment(&mut self, line: u32, comment: Comment) {
        self.line_comments.entry(line).or_default().push(comment);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum SessionDiffSource {
    #[default]
    WorkingTree,
    Staged,
    Unstaged,
    StagedAndUnstaged,
    CommitRange,
    WorkingTreeAndCommits,
    StagedUnstagedAndCommits,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewSession {
    pub id: String,
    pub version: String,
    pub repo_path: PathBuf,
    #[serde(default)]
    pub branch_name: Option<String>,
    pub base_commit: String,
    #[serde(default)]
    pub diff_source: SessionDiffSource,
    #[serde(default)]
    pub commit_range: Option<Vec<String>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub review_comments: Vec<Comment>,
    pub files: HashMap<PathBuf, FileReview>,
    pub session_notes: Option<String>,
}

impl ReviewSession {
    pub fn new(
        repo_path: PathBuf,
        base_commit: String,
        branch_name: Option<String>,
        diff_source: SessionDiffSource,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            version: "1.2".to_string(),
            repo_path,
            branch_name,
            base_commit,
            diff_source,
            commit_range: None,
            created_at: now,
            updated_at: now,
            review_comments: Vec::new(),
            files: HashMap::new(),
            session_notes: None,
        }
    }

    pub fn reviewed_count(&self) -> usize {
        self.files.values().filter(|f| f.reviewed).count()
    }

    pub fn add_file(&mut self, path: PathBuf, status: FileStatus) {
        self.files
            .entry(path.clone())
            .or_insert_with(|| FileReview::new(path, status));
    }

    pub fn get_file_mut(&mut self, path: &PathBuf) -> Option<&mut FileReview> {
        self.files.get_mut(path)
    }

    pub fn has_comments(&self) -> bool {
        !self.review_comments.is_empty() || self.files.values().any(|f| f.comment_count() > 0)
    }

    pub fn clear_comments(&mut self) -> (usize, usize) {
        let mut cleared = self.review_comments.len();
        let mut unreviewed = 0;
        self.review_comments.clear();
        for file in self.files.values_mut() {
            cleared += file.comment_count();
            file.file_comments.clear();
            file.line_comments.clear();
            if file.reviewed {
                file.reviewed = false;
                unreviewed += 1;
            }
        }
        (cleared, unreviewed)
    }

    pub fn is_file_reviewed(&self, path: &PathBuf) -> bool {
        self.files.get(path).map(|r| r.reviewed).unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::comment::{Comment, CommentType};

    fn test_session() -> ReviewSession {
        ReviewSession::new(
            PathBuf::from("/repo"),
            "abc123".to_string(),
            None,
            SessionDiffSource::WorkingTree,
        )
    }

    #[test]
    fn should_return_zero_when_clearing_empty_session() {
        let mut session = test_session();
        let (cleared, unreviewed) = session.clear_comments();
        assert_eq!(cleared, 0);
        assert_eq!(unreviewed, 0);
    }

    #[test]
    fn should_clear_review_level_comments() {
        let mut session = test_session();
        session
            .review_comments
            .push(Comment::new("note".to_string(), CommentType::Note, None));
        session
            .review_comments
            .push(Comment::new("issue".to_string(), CommentType::Issue, None));

        let (cleared, unreviewed) = session.clear_comments();
        assert_eq!(cleared, 2);
        assert_eq!(unreviewed, 0);
        assert!(session.review_comments.is_empty());
    }

    #[test]
    fn should_clear_file_and_line_comments() {
        let mut session = test_session();
        let path = PathBuf::from("src/main.rs");
        session.add_file(path.clone(), FileStatus::Modified);
        let file = session.get_file_mut(&path).unwrap();
        file.add_file_comment(Comment::new("comment".to_string(), CommentType::Note, None));
        file.add_line_comment(
            10,
            Comment::new("line".to_string(), CommentType::Note, None),
        );

        let (cleared, _) = session.clear_comments();
        assert_eq!(cleared, 2);

        let file = session.files.get(&path).unwrap();
        assert!(file.file_comments.is_empty());
        assert!(file.line_comments.is_empty());
    }

    #[test]
    fn should_reset_reviewed_status_on_all_files() {
        let mut session = test_session();
        let path_a = PathBuf::from("a.rs");
        let path_b = PathBuf::from("b.rs");
        session.add_file(path_a.clone(), FileStatus::Modified);
        session.add_file(path_b.clone(), FileStatus::Added);

        session.get_file_mut(&path_a).unwrap().reviewed = true;
        session.get_file_mut(&path_b).unwrap().reviewed = true;

        let (cleared, unreviewed) = session.clear_comments();
        assert_eq!(cleared, 0);
        assert_eq!(unreviewed, 2);
        assert!(!session.is_file_reviewed(&path_a));
        assert!(!session.is_file_reviewed(&path_b));
    }

    #[test]
    fn should_only_count_reviewed_files_as_unreviewed() {
        let mut session = test_session();
        let reviewed = PathBuf::from("reviewed.rs");
        let pending = PathBuf::from("pending.rs");
        session.add_file(reviewed.clone(), FileStatus::Modified);
        session.add_file(pending.clone(), FileStatus::Modified);

        session.get_file_mut(&reviewed).unwrap().reviewed = true;

        let (_, unreviewed) = session.clear_comments();
        assert_eq!(unreviewed, 1);
    }

    #[test]
    fn should_clear_both_comments_and_reviewed_status() {
        let mut session = test_session();
        let path = PathBuf::from("src/lib.rs");
        session.add_file(path.clone(), FileStatus::Modified);
        let file = session.get_file_mut(&path).unwrap();
        file.reviewed = true;
        file.add_file_comment(Comment::new("comment".to_string(), CommentType::Note, None));

        session
            .review_comments
            .push(Comment::new("review".to_string(), CommentType::Note, None));

        let (cleared, unreviewed) = session.clear_comments();
        assert_eq!(cleared, 2);
        assert_eq!(unreviewed, 1);
        assert!(!session.is_file_reviewed(&path));
    }
}
