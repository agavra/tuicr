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
}

impl GitCapabilities {
    fn detect(root_path: &Path) -> Self {
        let (sparse_checkout, sparse_index) = git_sparse_config(root_path);

        Self::from_config(sparse_checkout, sparse_index)
    }

    fn from_config(sparse_checkout: bool, sparse_index: bool) -> Self {
        let mode = if sparse_index {
            GitRepoMode::SparseIndex
        } else if sparse_checkout {
            GitRepoMode::SparseCheckout
        } else {
            GitRepoMode::Standard
        };

        Self { mode }
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

fn git_sparse_config(workdir: &Path) -> (bool, bool) {
    let output = run_git_command(
        workdir,
        &[
            "config",
            "--bool",
            "--get-regexp",
            r"^(core\.sparsecheckout|index\.sparse)$",
        ],
    )
    .unwrap_or_default();

    parse_sparse_config(&output)
}

fn parse_sparse_config(output: &str) -> (bool, bool) {
    let mut sparse_checkout = false;
    let mut sparse_index = false;

    for line in output.lines() {
        let mut parts = line.split_whitespace();
        let Some(key) = parts.next() else {
            continue;
        };
        let enabled = parts
            .next()
            .is_some_and(|value| matches!(value, "true" | "1" | "yes" | "on"));

        match key {
            "core.sparsecheckout" => sparse_checkout = enabled,
            "index.sparse" => sparse_index = enabled,
            _ => {}
        }
    }

    (sparse_checkout, sparse_index)
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

fn get_change_status(repo: &Repository, _capabilities: GitCapabilities) -> Result<VcsChangeStatus> {
    let mut status = crate::profile::time("vcs: git status tracked changes", || {
        get_tracked_change_status(repo)
    })?;

    if !status.unstaged {
        status.unstaged =
            crate::profile::time("vcs: git untracked scan", || has_untracked_changes(repo))?;
    }

    Ok(status)
}

fn get_tracked_change_status(repo: &Repository) -> Result<VcsChangeStatus> {
    let workdir = repo.workdir().ok_or(TuicrError::NotARepository)?;
    let mut child = Command::new("git")
        .current_dir(workdir)
        .args(["status", "--porcelain=v1", "-z", "--untracked-files=no"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| TuicrError::VcsCommand(format!("Failed to run git: {}", e)))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| TuicrError::VcsCommand("git status stdout unavailable".into()))?;

    let mut status = VcsChangeStatus::default();
    let stopped_early = parse_change_status_stream(BufReader::new(stdout), &mut status)?;

    if stopped_early {
        let _ = child.kill();
        let _ = child.wait();
        return Ok(status);
    }

    let output = child
        .wait_with_output()
        .map_err(|e| TuicrError::VcsCommand(format!("git status failed: {e}")))?;

    if !output.status.success() {
        return Err(TuicrError::VcsCommand(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }

    Ok(status)
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

fn parse_change_status_stream<R: BufRead>(
    mut reader: R,
    status: &mut VcsChangeStatus,
) -> Result<bool> {
    let mut record = Vec::new();
    loop {
        record.clear();
        let read = reader.read_until(0, &mut record)?;
        if read == 0 {
            return Ok(false);
        }
        if record.last() == Some(&0) {
            record.pop();
        }
        parse_change_status_record(&record, status);
        if status.staged && status.unstaged {
            return Ok(true);
        }
    }
}

fn parse_change_status_record(record: &[u8], status: &mut VcsChangeStatus) {
    if record.len() < 3 || record[2] != b' ' {
        return;
    }

    match (record[0], record[1]) {
        (b'?', b'?') => status.unstaged = true,
        (x, y) => {
            if x != b' ' {
                status.staged = true;
            }
            if y != b' ' {
                status.unstaged = true;
            }
        }
    }
}

impl VcsBackend for GitBackend {
    fn info(&self) -> &VcsInfo {
        &self.info
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
        if !self.capabilities.requires_git_cli() {
            return Err(TuicrError::UnsupportedOperation(
                "Git change status probe only used for sparse checkouts".into(),
            ));
        }
        get_change_status(&self.repo, self.capabilities)
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
            GitCapabilities::from_config(false, false).mode,
            GitRepoMode::Standard
        );
        assert_eq!(
            GitCapabilities::from_config(true, false).mode,
            GitRepoMode::SparseCheckout
        );
        assert_eq!(
            GitCapabilities::from_config(true, true).mode,
            GitRepoMode::SparseIndex
        );
    }

    #[test]
    fn parses_sparse_config_from_single_git_config_read() {
        let output = "core.sparsecheckout true\nindex.sparse true\n";

        assert_eq!(parse_sparse_config(output), (true, true));
    }

    #[test]
    fn parses_change_status_records() {
        let mut status = VcsChangeStatus::default();
        parse_change_status_record(b" M src/main.rs", &mut status);
        assert_eq!(
            status,
            VcsChangeStatus {
                staged: false,
                unstaged: true,
            }
        );

        let mut status = VcsChangeStatus::default();
        parse_change_status_record(b"M  src/main.rs", &mut status);
        assert_eq!(
            status,
            VcsChangeStatus {
                staged: true,
                unstaged: false,
            }
        );

        let mut status = VcsChangeStatus::default();
        parse_change_status_record(b"?? new-file.rs", &mut status);
        assert_eq!(
            status,
            VcsChangeStatus {
                staged: false,
                unstaged: true,
            }
        );

        let mut status = VcsChangeStatus::default();
        parse_change_status_record(b"R  new-name.rs", &mut status);
        parse_change_status_record(b"old-name.rs", &mut status);
        assert_eq!(
            status,
            VcsChangeStatus {
                staged: true,
                unstaged: false,
            }
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
