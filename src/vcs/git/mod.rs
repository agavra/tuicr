pub mod context;
pub mod diff;
pub mod repository;

use git2::Repository;
use std::path::Path;

use crate::error::{Result, TuicrError};
use crate::model::{DiffFile, DiffLine, FileStatus};
use crate::syntax::SyntaxHighlighter;

use super::traits::{CommitInfo, VcsBackend, VcsInfo, VcsType};

// Re-export commonly used functions
pub use context::{calculate_gap, fetch_context_lines};
pub use diff::{get_commit_range_diff, get_working_tree_diff};

/// Git backend implementation using git2 library
pub struct GitBackend {
    repo: Repository,
    info: VcsInfo,
}

impl GitBackend {
    /// Discover a git repository from the current directory
    pub fn discover() -> Result<Self> {
        let cwd = std::env::current_dir().map_err(|_| TuicrError::NotARepository)?;
        let repo = Repository::discover(&cwd).map_err(|_| TuicrError::NotARepository)?;

        let root_path = repo
            .workdir()
            .ok_or(TuicrError::NotARepository)?
            .to_path_buf();

        let head_commit = repo
            .head()
            .ok()
            .and_then(|h| h.peel_to_commit().ok())
            .map(|c| c.id().to_string())
            .unwrap_or_else(|| "HEAD".to_string());

        let branch_name = repo.head().ok().and_then(|h| {
            if h.is_branch() {
                h.shorthand().map(|s| s.to_string())
            } else {
                None
            }
        });

        let info = VcsInfo {
            root_path,
            head_commit,
            branch_name,
            vcs_type: VcsType::Git,
        };

        Ok(Self { repo, info })
    }
}

impl VcsBackend for GitBackend {
    fn info(&self) -> &VcsInfo {
        &self.info
    }

    fn get_working_tree_diff(&self, highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
        get_working_tree_diff(&self.repo, highlighter)
    }

    fn fetch_context_lines(
        &self,
        file_path: &Path,
        file_status: FileStatus,
        start_line: u32,
        end_line: u32,
    ) -> Result<Vec<DiffLine>> {
        fetch_context_lines(&self.repo, file_path, file_status, start_line, end_line)
    }

    fn get_recent_commits(&self, count: usize) -> Result<Vec<CommitInfo>> {
        let git_commits = repository::get_recent_commits(&self.repo, count)?;
        Ok(git_commits
            .into_iter()
            .map(|c| CommitInfo {
                id: c.id,
                short_id: c.short_id,
                summary: c.summary,
                author: c.author,
                time: c.time,
            })
            .collect())
    }

    fn get_commit_range_diff(
        &self,
        commit_ids: &[String],
        highlighter: &SyntaxHighlighter,
    ) -> Result<Vec<DiffFile>> {
        get_commit_range_diff(&self.repo, commit_ids, highlighter)
    }
}
