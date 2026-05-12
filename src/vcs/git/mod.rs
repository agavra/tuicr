mod cli;
pub mod context;
pub mod diff;
mod libgit2;
pub mod repository;
pub mod staging;

use std::path::Path;
use std::process::Command;

use crate::error::{Result, TuicrError};
use crate::model::{DiffFile, DiffLine, FileStatus};
use crate::syntax::SyntaxHighlighter;

use super::traits::{CommitInfo, VcsBackend, VcsChangeStatus, VcsInfo};
use cli::GitCliBackend;
pub use libgit2::Libgit2Backend;

// Re-exported for UI/app gap calculations.
pub use context::calculate_gap;

/// Top-level Git backend.
///
/// This wrapper keeps Git backend selection in one place. Today it delegates to
/// the git2/libgit2 implementation; sparse-checkout support can add another
/// variant without pushing backend-specific branches into every operation.
pub enum GitBackend {
    Libgit2(Libgit2Backend),
    Cli(GitCliBackend),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitRepoMode {
    Standard,
    SparseCheckout,
    SparseIndex,
}

impl GitRepoMode {
    fn detect(root_path: &Path) -> Result<Self> {
        let output = run_git_command(
            root_path,
            &[
                "config",
                "--get-regexp",
                r"^(core\.sparsecheckout|index\.sparse)$",
            ],
        )
        .unwrap_or_default();

        Ok(Self::from_config(&output))
    }

    fn from_config(output: &str) -> Self {
        let mut sparse_checkout = false;
        let mut sparse_index = false;

        for line in output.lines() {
            let mut parts = line.splitn(2, char::is_whitespace);
            let Some(key) = parts.next() else {
                continue;
            };
            let raw_value = parts.next().unwrap_or_default();

            match key {
                "core.sparsecheckout" => sparse_checkout = git_bool_config_enabled(raw_value),
                "index.sparse" => sparse_index = git_bool_config_enabled(raw_value),
                _ => {}
            }
        }

        if sparse_index {
            Self::SparseIndex
        } else if sparse_checkout {
            Self::SparseCheckout
        } else {
            Self::Standard
        }
    }

    fn is_sparse_checkout(self) -> bool {
        matches!(self, Self::SparseCheckout | Self::SparseIndex)
    }
}

impl GitBackend {
    /// Discover a git repository from the current directory.
    pub fn discover() -> Result<Self> {
        if let Ok(cli_backend) = GitCliBackend::discover()
            && cli_backend.repo_mode().is_sparse_checkout()
        {
            return Ok(Self::Cli(cli_backend));
        }

        Ok(Self::Libgit2(Libgit2Backend::discover()?))
    }
}

fn run_git_command(workdir: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .current_dir(workdir)
        .args(args)
        .output()
        .map_err(|e| TuicrError::VcsCommand(format!("Failed to run git: {e}")))?;

    if !output.status.success() {
        return Err(TuicrError::VcsCommand(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn git_bool_config_enabled(value: &str) -> bool {
    matches!(value.trim(), "true" | "1" | "yes" | "on")
}

fn git_fsmonitor_config_enabled(value: &str) -> bool {
    let value = value.trim();
    git_bool_config_enabled(value)
        || (!value.is_empty() && !matches!(value, "false" | "0" | "no" | "off"))
}

impl VcsBackend for GitBackend {
    fn info(&self) -> &VcsInfo {
        match self {
            Self::Libgit2(backend) => backend.info(),
            Self::Cli(backend) => backend.info(),
        }
    }

    fn startup_warnings(&self) -> Vec<String> {
        match self {
            Self::Libgit2(backend) => backend.startup_warnings(),
            Self::Cli(backend) => backend.startup_warnings(),
        }
    }

    fn get_working_tree_diff(&self, highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
        match self {
            Self::Libgit2(backend) => backend.get_working_tree_diff(highlighter),
            Self::Cli(backend) => backend.get_working_tree_diff(highlighter),
        }
    }

    fn get_staged_diff(&self, highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
        match self {
            Self::Libgit2(backend) => backend.get_staged_diff(highlighter),
            Self::Cli(backend) => backend.get_staged_diff(highlighter),
        }
    }

    fn get_unstaged_diff(&self, highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
        match self {
            Self::Libgit2(backend) => backend.get_unstaged_diff(highlighter),
            Self::Cli(backend) => backend.get_unstaged_diff(highlighter),
        }
    }

    fn get_change_status(&self) -> Result<VcsChangeStatus> {
        match self {
            Self::Libgit2(backend) => backend.get_change_status(),
            Self::Cli(backend) => backend.get_change_status(),
        }
    }

    fn fetch_context_lines(
        &self,
        file_path: &Path,
        file_status: FileStatus,
        start_line: u32,
        end_line: u32,
    ) -> Result<Vec<DiffLine>> {
        match self {
            Self::Libgit2(backend) => {
                backend.fetch_context_lines(file_path, file_status, start_line, end_line)
            }
            Self::Cli(backend) => {
                backend.fetch_context_lines(file_path, file_status, start_line, end_line)
            }
        }
    }

    fn get_recent_commits(&self, offset: usize, limit: usize) -> Result<Vec<CommitInfo>> {
        match self {
            Self::Libgit2(backend) => backend.get_recent_commits(offset, limit),
            Self::Cli(backend) => backend.get_recent_commits(offset, limit),
        }
    }

    fn resolve_revisions(&self, revisions: &str) -> Result<Vec<String>> {
        match self {
            Self::Libgit2(backend) => backend.resolve_revisions(revisions),
            Self::Cli(backend) => backend.resolve_revisions(revisions),
        }
    }

    fn get_commit_range_diff(
        &self,
        commit_ids: &[String],
        highlighter: &SyntaxHighlighter,
    ) -> Result<Vec<DiffFile>> {
        match self {
            Self::Libgit2(backend) => backend.get_commit_range_diff(commit_ids, highlighter),
            Self::Cli(backend) => backend.get_commit_range_diff(commit_ids, highlighter),
        }
    }

    fn get_commits_info(&self, ids: &[String]) -> Result<Vec<CommitInfo>> {
        match self {
            Self::Libgit2(backend) => backend.get_commits_info(ids),
            Self::Cli(backend) => backend.get_commits_info(ids),
        }
    }

    fn get_working_tree_with_commits_diff(
        &self,
        commit_ids: &[String],
        highlighter: &SyntaxHighlighter,
    ) -> Result<Vec<DiffFile>> {
        match self {
            Self::Libgit2(backend) => {
                backend.get_working_tree_with_commits_diff(commit_ids, highlighter)
            }
            Self::Cli(backend) => {
                backend.get_working_tree_with_commits_diff(commit_ids, highlighter)
            }
        }
    }

    fn stage_file(&self, path: &Path) -> Result<()> {
        match self {
            Self::Libgit2(backend) => backend.stage_file(path),
            Self::Cli(backend) => backend.stage_file(path),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_git_repo_mode_from_config() {
        assert_eq!(GitRepoMode::from_config(""), GitRepoMode::Standard);
        assert_eq!(
            GitRepoMode::from_config("core.sparsecheckout true\n"),
            GitRepoMode::SparseCheckout
        );
        assert_eq!(
            GitRepoMode::from_config("core.sparsecheckout true\nindex.sparse true\n"),
            GitRepoMode::SparseIndex
        );
    }
}
