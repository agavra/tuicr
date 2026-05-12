pub mod context;
pub mod diff;
pub mod repository;
pub mod staging;

use git2::Repository;
use std::path::Path;
use std::process::Command;

use crate::error::{Result, TuicrError};
use crate::model::{DiffFile, DiffLine, FileStatus};
use crate::syntax::SyntaxHighlighter;

use super::traits::{CommitInfo, VcsBackend, VcsInfo, VcsType};

// Re-export commonly used functions
pub use context::{calculate_gap, fetch_context_lines};
pub use diff::{
    get_commit_range_diff, get_staged_diff, get_unstaged_diff, get_working_tree_diff,
    get_working_tree_with_commits_diff,
};

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

        let head_commit = run_git_command(&root_path, &["rev-parse", "--verify", "HEAD"])
            .ok()
            .map(|value| value.trim().to_string())
            .unwrap_or_else(|| "HEAD".to_string());

        let branch_name =
            run_git_command(&root_path, &["symbolic-ref", "--quiet", "--short", "HEAD"])
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty());

        let info = VcsInfo {
            root_path,
            head_commit,
            branch_name,
            vcs_type: VcsType::Git,
        };

        Ok(Self { repo, info })
    }
}

fn run_git_command(workdir: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .current_dir(workdir)
        .args(args)
        .output()
        .map_err(|e| TuicrError::VcsCommand(format!("Failed to run git: {}", e)))?;

    if !output.status.success() {
        return Err(TuicrError::VcsCommand(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

impl VcsBackend for GitBackend {
    fn info(&self) -> &VcsInfo {
        &self.info
    }

    fn get_working_tree_diff(&self, highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
        get_working_tree_diff(&self.repo, highlighter)
    }

    fn get_staged_diff(&self, highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
        get_staged_diff(&self.repo, highlighter)
    }

    fn get_unstaged_diff(&self, highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
        get_unstaged_diff(&self.repo, highlighter)
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

    fn get_recent_commits(&self, offset: usize, limit: usize) -> Result<Vec<CommitInfo>> {
        let git_commits = repository::get_recent_commits(&self.repo, offset, limit)?;
        Ok(git_commits
            .into_iter()
            .map(|c| CommitInfo {
                id: c.id,
                short_id: c.short_id,
                branch_name: c.branch_name,
                summary: c.summary,
                body: c.body,
                author: c.author,
                time: c.time,
            })
            .collect())
    }

    fn resolve_revisions(&self, revisions: &str) -> Result<Vec<String>> {
        repository::resolve_revisions(&self.repo, revisions)
    }

    fn get_commit_range_diff(
        &self,
        commit_ids: &[String],
        highlighter: &SyntaxHighlighter,
    ) -> Result<Vec<DiffFile>> {
        get_commit_range_diff(&self.repo, commit_ids, highlighter)
    }

    fn get_commits_info(&self, ids: &[String]) -> Result<Vec<CommitInfo>> {
        let git_commits = repository::get_commits_info(&self.repo, ids)?;
        Ok(git_commits
            .into_iter()
            .map(|c| CommitInfo {
                id: c.id,
                short_id: c.short_id,
                branch_name: c.branch_name,
                summary: c.summary,
                body: c.body,
                author: c.author,
                time: c.time,
            })
            .collect())
    }

    fn get_working_tree_with_commits_diff(
        &self,
        commit_ids: &[String],
        highlighter: &SyntaxHighlighter,
    ) -> Result<Vec<DiffFile>> {
        get_working_tree_with_commits_diff(&self.repo, commit_ids, highlighter)
    }

    fn stage_file(&self, path: &Path) -> Result<()> {
        staging::stage_file(&self.repo, path)
    }
}
