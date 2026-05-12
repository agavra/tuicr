pub mod context;
pub mod diff;
pub mod repository;
pub mod staging;

use git2::Repository;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};

use crate::error::{Result, TuicrError};
use crate::model::{DiffFile, DiffLine, FileStatus};
use crate::syntax::SyntaxHighlighter;

use super::traits::{CommitInfo, VcsBackend, VcsChangeStatus, VcsInfo, VcsType};

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
    capabilities: GitCapabilities,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitRepoMode {
    Standard,
    SparseCheckout,
    SparseIndex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GitCapabilities {
    pub mode: GitRepoMode,
    /// Git's untracked cache avoids re-reading unchanged directories during
    /// exact untracked-file scans. This matters most for sparse monorepos.
    pub untracked_cache: bool,
    /// `core.fsmonitor` may be `true` or a custom hook path; both mean Git can
    /// avoid broad tracked-file stat walks.
    pub fsmonitor: bool,
}

impl GitCapabilities {
    fn detect(root_path: &Path) -> Self {
        let (sparse_checkout, sparse_index, untracked_cache, fsmonitor) = git_config(root_path);

        Self::from_config(sparse_checkout, sparse_index, untracked_cache, fsmonitor)
    }

    fn from_config(
        sparse_checkout: bool,
        sparse_index: bool,
        untracked_cache: bool,
        fsmonitor: bool,
    ) -> Self {
        let mode = if sparse_index {
            GitRepoMode::SparseIndex
        } else if sparse_checkout {
            GitRepoMode::SparseCheckout
        } else {
            GitRepoMode::Standard
        };

        Self {
            mode,
            untracked_cache,
            fsmonitor,
        }
    }

    pub fn is_sparse_checkout(self) -> bool {
        matches!(
            self.mode,
            GitRepoMode::SparseCheckout | GitRepoMode::SparseIndex
        )
    }

    pub fn requires_git_cli(self) -> bool {
        self.is_sparse_checkout()
    }

    pub fn startup_warnings(self) -> Vec<String> {
        if self.is_sparse_checkout() && !self.untracked_cache {
            vec![
                "Sparse checkout without core.untrackedCache can make untracked scans slow; run `git update-index --test-untracked-cache` then `git config core.untrackedCache true` if it passes.".to_string(),
            ]
        } else {
            Vec::new()
        }
    }
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
        let capabilities = GitCapabilities::detect(&root_path);

        let info = VcsInfo {
            root_path,
            head_commit,
            branch_name,
            vcs_type: VcsType::Git,
        };

        Ok(Self {
            repo,
            info,
            capabilities,
        })
    }
}

fn git_config(workdir: &Path) -> (bool, bool, bool, bool) {
    // Keep this as a raw config read: core.fsmonitor can be a custom hook path,
    // so `git config --bool` would reject valid enabled configurations.
    let output = run_git_command(
        workdir,
        &[
            "config",
            "--get-regexp",
            r"^(core\.sparsecheckout|index\.sparse|core\.untrackedcache|core\.fsmonitor)$",
        ],
    )
    .unwrap_or_default();

    parse_git_config(&output)
}

fn parse_git_config(output: &str) -> (bool, bool, bool, bool) {
    let mut sparse_checkout = false;
    let mut sparse_index = false;
    let mut untracked_cache = false;
    let mut fsmonitor = false;

    for line in output.lines() {
        let mut parts = line.splitn(2, char::is_whitespace);
        let Some(key) = parts.next() else {
            continue;
        };
        let raw_value = parts.next().unwrap_or_default();

        match key {
            "core.sparsecheckout" => sparse_checkout = git_bool_config_enabled(raw_value),
            "index.sparse" => sparse_index = git_bool_config_enabled(raw_value),
            "core.untrackedcache" => untracked_cache = git_bool_config_enabled(raw_value),
            "core.fsmonitor" => fsmonitor = git_fsmonitor_config_enabled(raw_value),
            _ => {}
        }
    }

    (sparse_checkout, sparse_index, untracked_cache, fsmonitor)
}

fn git_bool_config_enabled(value: &str) -> bool {
    let value = value.trim();
    matches!(value, "true" | "1" | "yes" | "on")
}

fn git_fsmonitor_config_enabled(value: &str) -> bool {
    let value = value.trim();
    git_bool_config_enabled(value)
        || (!value.is_empty() && !matches!(value, "false" | "0" | "no" | "off"))
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

fn get_change_status(repo: &Repository, capabilities: GitCapabilities) -> Result<VcsChangeStatus> {
    // Tracked changes have a cheap exact probe. Untracked files require a
    // working-tree scan, so only pay that cost when no tracked unstaged changes
    // already prove the "unstaged" row should be shown.
    let staged = crate::profile::time("vcs: git staged diff probe", || {
        has_diff_changes(repo, &["diff", "--quiet", "--cached", "--"])
    })?;
    let tracked_unstaged = crate::profile::time("vcs: git unstaged diff probe", || {
        has_diff_changes(repo, &["diff", "--quiet", "--"])
    })?;
    let unstaged = if tracked_unstaged {
        true
    } else {
        crate::profile::time_with(
            "vcs: git untracked scan",
            || has_untracked_changes(repo),
            |result| match result {
                Ok(has_untracked) => format!(
                    "result={has_untracked}, untracked_cache={}, fsmonitor={}",
                    capabilities.untracked_cache, capabilities.fsmonitor
                ),
                Err(e) => format!(
                    "error={e}, untracked_cache={}, fsmonitor={}",
                    capabilities.untracked_cache, capabilities.fsmonitor
                ),
            },
        )?
    };

    Ok(VcsChangeStatus { staged, unstaged })
}

fn has_diff_changes(repo: &Repository, args: &[&str]) -> Result<bool> {
    let workdir = repo.workdir().ok_or(TuicrError::NotARepository)?;
    let output = Command::new("git")
        .current_dir(workdir)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| TuicrError::VcsCommand(format!("Failed to run git: {}", e)))?;

    match output.status.code() {
        Some(0) => Ok(false),
        Some(1) => Ok(true),
        _ => Err(TuicrError::VcsCommand(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        )),
    }
}

fn has_untracked_changes(repo: &Repository) -> Result<bool> {
    let workdir = repo.workdir().ok_or(TuicrError::NotARepository)?;
    let mut child = Command::new("git")
        .current_dir(workdir)
        .args([
            "ls-files",
            "--others",
            "--exclude-standard",
            "-z",
            "--directory",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| TuicrError::VcsCommand(format!("Failed to run git: {}", e)))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| TuicrError::VcsCommand("git ls-files stdout unavailable".into()))?;
    let mut reader = BufReader::new(stdout);
    let mut record = Vec::new();
    let read = reader.read_until(0, &mut record)?;
    if read > 0 {
        let _ = child.kill();
        let _ = child.wait();
        return Ok(true);
    }

    let output = child
        .wait_with_output()
        .map_err(|e| TuicrError::VcsCommand(format!("git ls-files failed: {e}")))?;

    if !output.status.success() {
        return Err(TuicrError::VcsCommand(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }

    Ok(false)
}

impl VcsBackend for GitBackend {
    fn info(&self) -> &VcsInfo {
        &self.info
    }

    fn startup_warnings(&self) -> Vec<String> {
        self.capabilities.startup_warnings()
    }

    fn get_working_tree_diff(&self, highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
        get_working_tree_diff(&self.repo, self.capabilities, highlighter)
    }

    fn get_staged_diff(&self, highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
        get_staged_diff(&self.repo, self.capabilities, highlighter)
    }

    fn get_unstaged_diff(&self, highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
        get_unstaged_diff(&self.repo, self.capabilities, highlighter)
    }

    fn get_change_status(&self) -> Result<VcsChangeStatus> {
        if self.capabilities.requires_git_cli() {
            return get_change_status(&self.repo, self.capabilities);
        }

        Err(TuicrError::UnsupportedOperation(
            "Git change status probe only used for sparse checkouts".into(),
        ))
    }

    fn fetch_context_lines(
        &self,
        file_path: &Path,
        file_status: FileStatus,
        start_line: u32,
        end_line: u32,
    ) -> Result<Vec<DiffLine>> {
        fetch_context_lines(
            &self.repo,
            self.capabilities,
            file_path,
            file_status,
            start_line,
            end_line,
        )
    }

    fn get_recent_commits(&self, offset: usize, limit: usize) -> Result<Vec<CommitInfo>> {
        let git_commits =
            repository::get_recent_commits(&self.repo, self.capabilities, offset, limit)?;
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
        repository::resolve_revisions(&self.repo, self.capabilities, revisions)
    }

    fn get_commit_range_diff(
        &self,
        commit_ids: &[String],
        highlighter: &SyntaxHighlighter,
    ) -> Result<Vec<DiffFile>> {
        get_commit_range_diff(&self.repo, self.capabilities, commit_ids, highlighter)
    }

    fn get_commits_info(&self, ids: &[String]) -> Result<Vec<CommitInfo>> {
        let git_commits = repository::get_commits_info(&self.repo, self.capabilities, ids)?;
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
        get_working_tree_with_commits_diff(&self.repo, self.capabilities, commit_ids, highlighter)
    }

    fn stage_file(&self, path: &Path) -> Result<()> {
        staging::stage_file(&self.repo, self.capabilities, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(workdir: &Path, args: &[&str]) {
        let output = Command::new("git")
            .current_dir(workdir)
            .args(args)
            .output()
            .expect("failed to run git");
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn detects_standard_git_repo_mode() {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        git(temp_dir.path(), &["init"]);

        let capabilities = GitCapabilities::detect(temp_dir.path());

        assert_eq!(capabilities.mode, GitRepoMode::Standard);
        assert!(!capabilities.requires_git_cli());
    }

    #[test]
    fn derives_git_repo_mode_from_sparse_config() {
        assert_eq!(
            GitCapabilities::from_config(false, false, false, false).mode,
            GitRepoMode::Standard
        );
        assert_eq!(
            GitCapabilities::from_config(true, false, false, false).mode,
            GitRepoMode::SparseCheckout
        );
        assert_eq!(
            GitCapabilities::from_config(true, true, false, false).mode,
            GitRepoMode::SparseIndex
        );
    }

    #[test]
    fn parses_git_config_from_single_git_config_read() {
        let output = "core.sparsecheckout true\nindex.sparse true\ncore.untrackedcache true\ncore.fsmonitor true\n";

        assert_eq!(parse_git_config(output), (true, true, true, true));
    }

    #[test]
    fn treats_custom_fsmonitor_hook_as_enabled() {
        let output = "core.fsmonitor .git/hooks/fsmonitor-watchman\n";

        assert_eq!(parse_git_config(output), (false, false, false, true));
    }

    #[test]
    fn warns_when_sparse_checkout_lacks_untracked_cache() {
        let capabilities = GitCapabilities::from_config(true, false, false, true);

        assert_eq!(
            capabilities.startup_warnings(),
            vec![
                "Sparse checkout without core.untrackedCache can make untracked scans slow; run `git update-index --test-untracked-cache` then `git config core.untrackedCache true` if it passes.".to_string()
            ]
        );
    }

    #[test]
    fn does_not_warn_for_standard_repos_or_sparse_repos_with_untracked_cache() {
        assert!(
            GitCapabilities::from_config(false, false, false, false)
                .startup_warnings()
                .is_empty()
        );
        assert!(
            GitCapabilities::from_config(true, false, true, false)
                .startup_warnings()
                .is_empty()
        );
    }

    #[test]
    fn detects_change_status_without_loading_diff() {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        git(temp_dir.path(), &["init"]);
        git(
            temp_dir.path(),
            &["config", "user.email", "test@example.com"],
        );
        git(temp_dir.path(), &["config", "user.name", "Test User"]);
        std::fs::write(temp_dir.path().join("file.txt"), "initial\n")
            .expect("failed to write file");
        git(temp_dir.path(), &["add", "."]);
        git(temp_dir.path(), &["commit", "-m", "initial"]);

        std::fs::write(temp_dir.path().join("file.txt"), "modified\n")
            .expect("failed to modify file");
        std::fs::write(temp_dir.path().join("new.txt"), "new\n")
            .expect("failed to write untracked file");

        let repo = Repository::discover(temp_dir.path()).expect("failed to discover repo");
        let status = get_change_status(
            &repo,
            GitCapabilities {
                mode: GitRepoMode::Standard,
                untracked_cache: false,
                fsmonitor: false,
            },
        )
        .expect("failed to get change status");

        assert_eq!(
            status,
            VcsChangeStatus {
                staged: false,
                unstaged: true,
            }
        );

        git(temp_dir.path(), &["add", "file.txt"]);
        let status = get_change_status(
            &repo,
            GitCapabilities {
                mode: GitRepoMode::Standard,
                untracked_cache: false,
                fsmonitor: false,
            },
        )
        .expect("failed to get change status");

        assert_eq!(
            status,
            VcsChangeStatus {
                staged: true,
                unstaged: true,
            }
        );
    }

    #[test]
    fn detects_sparse_index_repo_mode_once() {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        git(temp_dir.path(), &["init"]);
        git(
            temp_dir.path(),
            &["config", "user.email", "test@example.com"],
        );
        git(temp_dir.path(), &["config", "user.name", "Test User"]);
        std::fs::create_dir_all(temp_dir.path().join("keep")).expect("failed to create keep dir");
        std::fs::create_dir_all(temp_dir.path().join("hidden"))
            .expect("failed to create hidden dir");
        std::fs::write(temp_dir.path().join("keep/file.txt"), "keep\n")
            .expect("failed to write keep file");
        std::fs::write(temp_dir.path().join("hidden/file.txt"), "hidden\n")
            .expect("failed to write hidden file");
        git(temp_dir.path(), &["add", "."]);
        git(temp_dir.path(), &["commit", "-m", "initial"]);
        git(temp_dir.path(), &["sparse-checkout", "init", "--cone"]);
        git(temp_dir.path(), &["sparse-checkout", "set", "keep"]);
        git(
            temp_dir.path(),
            &["sparse-checkout", "reapply", "--sparse-index"],
        );

        let capabilities = GitCapabilities::detect(temp_dir.path());

        assert_eq!(capabilities.mode, GitRepoMode::SparseIndex);
        assert!(capabilities.is_sparse_checkout());
        assert!(capabilities.requires_git_cli());
    }
}
