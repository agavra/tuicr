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
    CommitRange,
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
        self.files.values().any(|f| f.comment_count() > 0)
    }

    pub fn clear_comments(&mut self) -> usize {
        let mut cleared = 0;
        for file in self.files.values_mut() {
            cleared += file.comment_count();
            file.file_comments.clear();
            file.line_comments.clear();
        }
        cleared
    }

    pub fn is_file_reviewed(&self, path: &PathBuf) -> bool {
        self.files.get(path).map(|r| r.reviewed).unwrap_or(false)
    }
}
