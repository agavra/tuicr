use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::model::{DiffFile, DiffLine, FileStatus};
use crate::syntax::SyntaxHighlighter;

/// Information about the VCS type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VcsType {
    Git,
    Mercurial,
    Jujutsu,
}

impl std::fmt::Display for VcsType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VcsType::Git => write!(f, "git"),
            VcsType::Mercurial => write!(f, "hg"),
            VcsType::Jujutsu => write!(f, "jj"),
        }
    }
}

/// Repository information
#[derive(Debug, Clone)]
pub struct VcsInfo {
    pub root_path: PathBuf,
    pub head_commit: String,
    pub branch_name: Option<String>,
    /// VCS type - displayed in status bar header
    pub vcs_type: VcsType,
}

/// Commit information for commit selection UI
#[derive(Debug, Clone)]
pub struct CommitInfo {
    pub id: String,
    pub short_id: String,
    /// Optional branch label for this commit in commit selection UI.
    /// For Git this is populated for commits that are branch tips.
    pub branch_name: Option<String>,
    pub summary: String,
    pub body: Option<String>,
    pub author: String,
    pub time: DateTime<Utc>,
}

/// Metadata about a pull-request style diff.
#[derive(Debug, Clone)]
pub struct PullRequestInfo {
    /// Base reference used for the PR comparison, for example `origin/main`.
    pub base_ref: String,
    /// Merge-base commit between base_ref and HEAD.
    pub merge_base_commit: String,
    /// Current HEAD commit.
    pub head_commit: String,
    /// Number of commits between merge-base and HEAD.
    pub commit_count: usize,
}

/// Pull-request diff result including files and metadata.
#[derive(Debug, Clone)]
pub struct PullRequestDiff {
    pub files: Vec<DiffFile>,
    pub info: PullRequestInfo,
}

/// Trait for VCS backend implementations
pub trait VcsBackend: Send {
    /// Get repository information
    fn info(&self) -> &VcsInfo;

    /// Get the working tree diff (uncommitted changes)
    fn get_working_tree_diff(&self, highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>>;

    /// Fetch context lines for gap expansion.
    /// For deleted files, reads from VCS; otherwise from working tree.
    fn fetch_context_lines(
        &self,
        file_path: &Path,
        file_status: FileStatus,
        start_line: u32,
        end_line: u32,
    ) -> Result<Vec<DiffLine>>;

    /// Get recent commits for commit selection UI.
    /// Returns empty vec if not supported (default).
    fn get_recent_commits(&self, _offset: usize, _limit: usize) -> Result<Vec<CommitInfo>> {
        Ok(Vec::new())
    }

    /// Resolve a revisions expression to a list of commit IDs (oldest first).
    /// Returns error if not supported (default).
    fn resolve_revisions(&self, _revisions: &str) -> Result<Vec<String>> {
        Err(crate::error::TuicrError::UnsupportedOperation(
            "Revset resolution not supported for this VCS".into(),
        ))
    }

    /// Get diff for a commit range.
    /// Returns error if not supported (default).
    fn get_commit_range_diff(
        &self,
        _commit_ids: &[String],
        _highlighter: &SyntaxHighlighter,
    ) -> Result<Vec<DiffFile>> {
        Err(crate::error::TuicrError::UnsupportedOperation(
            "Commit range diff not supported for this VCS".into(),
        ))
    }

    /// Get commit info for specific commit IDs (for inline commit selector).
    /// Returns CommitInfo for each ID, in the same order as the input.
    fn get_commits_info(&self, _ids: &[String]) -> Result<Vec<CommitInfo>> {
        Ok(Vec::new())
    }

    /// Get a combined diff from the parent of the oldest commit through to the working tree.
    /// This shows both committed and uncommitted changes in a single diff.
    /// Returns error if not supported (default).
    fn get_working_tree_with_commits_diff(
        &self,
        _commit_ids: &[String],
        _highlighter: &SyntaxHighlighter,
    ) -> Result<Vec<DiffFile>> {
        Err(crate::error::TuicrError::UnsupportedOperation(
            "Working tree + commits diff not supported for this VCS".into(),
        ))
    }

    /// Get a PR-style diff from merge-base(base_ref, HEAD) to HEAD.
    fn get_pull_request_diff(
        &self,
        _base_ref: Option<&str>,
        _highlighter: &SyntaxHighlighter,
    ) -> Result<PullRequestDiff> {
        Err(crate::error::TuicrError::UnsupportedOperation(
            "PR diff not supported for this VCS".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vcs_type_display_git() {
        assert_eq!(format!("{}", VcsType::Git), "git");
    }

    #[test]
    fn vcs_type_display_mercurial() {
        assert_eq!(format!("{}", VcsType::Mercurial), "hg");
    }

    #[test]
    fn vcs_type_display_jujutsu() {
        assert_eq!(format!("{}", VcsType::Jujutsu), "jj");
    }

    #[test]
    fn vcs_type_equality() {
        assert_eq!(VcsType::Git, VcsType::Git);
        assert_eq!(VcsType::Mercurial, VcsType::Mercurial);
        assert_ne!(VcsType::Git, VcsType::Mercurial);
        assert_eq!(VcsType::Jujutsu, VcsType::Jujutsu);
        assert_ne!(VcsType::Git, VcsType::Jujutsu);
    }

    #[test]
    fn vcs_info_clone() {
        let info = VcsInfo {
            root_path: PathBuf::from("/test/repo"),
            head_commit: "abc123".to_string(),
            branch_name: Some("main".to_string()),
            vcs_type: VcsType::Git,
        };

        let cloned = info.clone();
        assert_eq!(cloned.root_path, PathBuf::from("/test/repo"));
        assert_eq!(cloned.head_commit, "abc123");
        assert_eq!(cloned.branch_name, Some("main".to_string()));
        assert_eq!(cloned.vcs_type, VcsType::Git);
    }

    #[test]
    fn vcs_info_without_branch() {
        let info = VcsInfo {
            root_path: PathBuf::from("/detached"),
            head_commit: "def456".to_string(),
            branch_name: None,
            vcs_type: VcsType::Git,
        };

        assert!(info.branch_name.is_none());
    }

    #[test]
    fn commit_info_clone() {
        let commit = CommitInfo {
            id: "abc123def456".to_string(),
            short_id: "abc123d".to_string(),
            branch_name: Some("main".to_string()),
            summary: "Fix bug".to_string(),
            body: None,
            author: "Test User".to_string(),
            time: Utc::now(),
        };

        let cloned = commit.clone();
        assert_eq!(cloned.id, "abc123def456");
        assert_eq!(cloned.short_id, "abc123d");
        assert_eq!(cloned.branch_name, Some("main".to_string()));
        assert_eq!(cloned.summary, "Fix bug");
        assert_eq!(cloned.author, "Test User");
    }
}
