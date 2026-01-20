//! Jujutsu (jj) backend implementation using CLI commands.

use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::{DateTime, Utc};

use crate::error::{Result, TuicrError};
use crate::model::{DiffFile, DiffLine, FileStatus, LineOrigin};
use crate::syntax::SyntaxHighlighter;
use crate::vcs::diff_parser::{self, DiffFormat};
use crate::vcs::traits::{CommitInfo, VcsBackend, VcsInfo, VcsType};

/// Jujutsu backend implementation using jj CLI commands
pub struct JjBackend {
    info: VcsInfo,
}

impl JjBackend {
    /// Discover a Jujutsu repository from the current directory
    pub fn discover() -> Result<Self> {
        // Use `jj root` to find the repository root
        // This handles being called from subdirectories
        let root_output = Command::new("jj")
            .args(["root"])
            .output()
            .map_err(|e| TuicrError::VcsCommand(format!("Failed to run jj: {}", e)))?;

        if !root_output.status.success() {
            return Err(TuicrError::NotARepository);
        }

        let root_path = PathBuf::from(String::from_utf8_lossy(&root_output.stdout).trim());

        Self::from_path(root_path)
    }

    /// Create backend from a known path (used by discover and tests)
    fn from_path(root_path: PathBuf) -> Result<Self> {
        // Get current change id (jj uses change IDs rather than commit hashes)
        let head_commit = run_jj_command(
            &root_path,
            &["log", "-r", "@", "--no-graph", "-T", "change_id.short()"],
        )
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "unknown".to_string());

        // jj doesn't have branches in the traditional sense, but we can show the bookmark if set
        let branch_name = run_jj_command(
            &root_path,
            &["log", "-r", "@", "--no-graph", "-T", "bookmarks"],
        )
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

        let info = VcsInfo {
            root_path,
            head_commit,
            branch_name,
            vcs_type: VcsType::Jujutsu,
        };

        Ok(Self { info })
    }
}

impl VcsBackend for JjBackend {
    fn info(&self) -> &VcsInfo {
        &self.info
    }

    fn get_working_tree_diff(&self, highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
        // Get unified diff output from jj using --git format
        let diff_output = run_jj_command(&self.info.root_path, &["diff", "--git"])?;

        if diff_output.trim().is_empty() {
            return Err(TuicrError::NoChanges);
        }

        diff_parser::parse_unified_diff(&diff_output, DiffFormat::GitStyle, highlighter)
    }

    fn fetch_context_lines(
        &self,
        file_path: &Path,
        file_status: FileStatus,
        start_line: u32,
        end_line: u32,
    ) -> Result<Vec<DiffLine>> {
        if start_line > end_line || start_line == 0 {
            return Ok(Vec::new());
        }

        let content = match file_status {
            FileStatus::Deleted => {
                // Read from jj show (parent revision)
                run_jj_command(
                    &self.info.root_path,
                    &["file", "show", "-r", "@-", &file_path.to_string_lossy()],
                )?
            }
            _ => {
                // Read from working tree
                let full_path = self.info.root_path.join(file_path);
                std::fs::read_to_string(&full_path)?
            }
        };

        let lines: Vec<&str> = content.lines().collect();
        let mut result = Vec::new();

        for line_num in start_line..=end_line {
            let idx = (line_num - 1) as usize;
            if idx < lines.len() {
                result.push(DiffLine {
                    origin: LineOrigin::Context,
                    content: lines[idx].to_string(),
                    old_lineno: Some(line_num),
                    new_lineno: Some(line_num),
                    highlighted_spans: None,
                });
            }
        }

        Ok(result)
    }

    fn get_recent_commits(&self, count: usize) -> Result<Vec<CommitInfo>> {
        // Use jj log with a template to get structured output
        // Template fields separated by \x00, records separated by \x01
        // Note: jj uses change_id for identifying changes, commit_id for the underlying git commit
        let template = r#"commit_id ++ "\x00" ++ commit_id.short() ++ "\x00" ++ description.first_line() ++ "\x00" ++ author.email() ++ "\x00" ++ committer.timestamp() ++ "\x01""#;
        let output = run_jj_command(
            &self.info.root_path,
            &[
                "log",
                "-r",
                "::@",
                "--limit",
                &count.to_string(),
                "--no-graph",
                "-T",
                template,
            ],
        )?;

        let mut commits = Vec::new();
        for record in output.split('\x01') {
            let record = record.trim();
            if record.is_empty() {
                continue;
            }

            let parts: Vec<&str> = record.split('\x00').collect();
            if parts.len() < 5 {
                continue;
            }

            let id = parts[0].to_string();
            let short_id = parts[1].to_string();
            let summary = parts[2].to_string();
            let author = parts[3].to_string();

            // jj timestamp format is ISO 8601: "2024-01-15T10:30:00.000-05:00"
            let time = DateTime::parse_from_rfc3339(parts[4])
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());

            commits.push(CommitInfo {
                id,
                short_id,
                summary,
                author,
                time,
            });
        }

        Ok(commits)
    }

    fn get_commit_range_diff(
        &self,
        commit_ids: &[String],
        highlighter: &SyntaxHighlighter,
    ) -> Result<Vec<DiffFile>> {
        if commit_ids.is_empty() {
            return Err(TuicrError::NoChanges);
        }

        // commit_ids are ordered from oldest to newest
        let oldest = &commit_ids[0];
        let newest = commit_ids.last().unwrap();

        // Get the parent of the oldest commit to include its changes
        // In jj, we use {commit}- to get the parent(s)
        let diff_output = run_jj_command(
            &self.info.root_path,
            &[
                "diff",
                "--from",
                &format!("{}-", oldest),
                "--to",
                newest,
                "--git",
            ],
        )?;

        if diff_output.trim().is_empty() {
            return Err(TuicrError::NoChanges);
        }

        diff_parser::parse_unified_diff(&diff_output, DiffFormat::GitStyle, highlighter)
    }
}

/// Run a jj command and return its stdout
fn run_jj_command(root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("jj")
        .current_dir(root)
        .args(args)
        .output()
        .map_err(|e| TuicrError::VcsCommand(format!("Failed to run jj: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TuicrError::VcsCommand(format!(
            "jj {} failed: {}",
            args.join(" "),
            stderr
        )));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Check if jj command is available
    fn jj_available() -> bool {
        Command::new("jj")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Discover a Jujutsu repository from a specific directory
    fn discover_in(path: &Path) -> Result<JjBackend> {
        let root_output = Command::new("jj")
            .args(["root"])
            .current_dir(path)
            .output()
            .map_err(|e| TuicrError::VcsCommand(format!("Failed to run jj: {}", e)))?;

        if !root_output.status.success() {
            return Err(TuicrError::NotARepository);
        }

        let root_path = PathBuf::from(String::from_utf8_lossy(&root_output.stdout).trim());

        JjBackend::from_path(root_path)
    }

    /// Create a temporary jj repo for testing.
    /// Returns None if jj is not available.
    fn setup_test_repo() -> Option<tempfile::TempDir> {
        if !jj_available() {
            return None;
        }

        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let root = temp_dir.path();

        // Initialize jj repo (jj init creates a git-backed repo by default)
        let output = Command::new("jj")
            .args(["git", "init"])
            .current_dir(root)
            .output()
            .expect("Failed to init jj repo");

        if !output.status.success() {
            eprintln!(
                "jj git init failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            return None;
        }

        // Create initial file
        fs::write(root.join("hello.txt"), "hello world\n").expect("Failed to write file");

        // Snapshot the changes (jj auto-tracks files)
        Command::new("jj")
            .args(["commit", "-m", "Initial commit"])
            .current_dir(root)
            .output()
            .expect("Failed to commit");

        // Make a modification
        fs::write(root.join("hello.txt"), "hello world\nmodified line\n")
            .expect("Failed to modify file");

        Some(temp_dir)
    }

    #[test]
    fn test_jj_discover() {
        let Some(temp) = setup_test_repo() else {
            eprintln!("Skipping test: jj command not available");
            return;
        };

        // Use discover_in to avoid set_current_dir race conditions
        let backend = discover_in(temp.path()).expect("Failed to discover jj repo");
        let info = backend.info();

        assert_eq!(info.root_path, temp.path());
        assert_eq!(info.vcs_type, VcsType::Jujutsu);
        assert!(!info.head_commit.is_empty());
    }

    #[test]
    fn test_jj_working_tree_diff() {
        let Some(temp) = setup_test_repo() else {
            eprintln!("Skipping test: jj command not available");
            return;
        };

        // Use from_path directly to avoid set_current_dir race conditions
        let backend =
            JjBackend::from_path(temp.path().to_path_buf()).expect("Failed to create jj backend");

        assert_eq!(backend.info().root_path, temp.path());
        assert_eq!(backend.info().vcs_type, VcsType::Jujutsu);

        let files = backend
            .get_working_tree_diff(&SyntaxHighlighter::default())
            .expect("Failed to get diff");

        assert_eq!(files.len(), 1);
        assert_eq!(
            files[0].new_path.as_ref().unwrap().to_str().unwrap(),
            "hello.txt"
        );
        assert_eq!(files[0].status, FileStatus::Modified);
    }

    #[test]
    fn test_jj_fetch_context_lines() {
        let Some(temp) = setup_test_repo() else {
            eprintln!("Skipping test: jj command not available");
            return;
        };

        // Use from_path directly to avoid set_current_dir race conditions
        let backend =
            JjBackend::from_path(temp.path().to_path_buf()).expect("Failed to create jj backend");

        assert_eq!(backend.info().root_path, temp.path());

        // Fetch context lines from working tree (modified file)
        let lines = backend
            .fetch_context_lines(Path::new("hello.txt"), FileStatus::Modified, 1, 2)
            .expect("Failed to fetch context lines");

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].content, "hello world");
        assert_eq!(lines[1].content, "modified line");
    }

    /// Create a test repo with multiple commits (no pending changes).
    /// Returns None if jj is not available.
    fn setup_test_repo_with_commits() -> Option<tempfile::TempDir> {
        if !jj_available() {
            return None;
        }

        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let root = temp_dir.path();

        // Initialize jj repo
        let output = Command::new("jj")
            .args(["git", "init"])
            .current_dir(root)
            .output()
            .expect("Failed to init jj repo");

        if !output.status.success() {
            eprintln!(
                "jj git init failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            return None;
        }

        // First commit
        fs::write(root.join("file1.txt"), "first file\n").expect("Failed to write file");
        Command::new("jj")
            .args(["commit", "-m", "First commit"])
            .current_dir(root)
            .output()
            .expect("Failed to commit");

        // Second commit
        fs::write(root.join("file2.txt"), "second file\n").expect("Failed to write file");
        Command::new("jj")
            .args(["commit", "-m", "Second commit"])
            .current_dir(root)
            .output()
            .expect("Failed to commit");

        // Third commit - modify first file
        fs::write(root.join("file1.txt"), "first file\nmodified\n").expect("Failed to write file");
        Command::new("jj")
            .args(["commit", "-m", "Third commit"])
            .current_dir(root)
            .output()
            .expect("Failed to commit");

        Some(temp_dir)
    }

    #[test]
    fn test_jj_get_recent_commits() {
        let Some(temp) = setup_test_repo_with_commits() else {
            eprintln!("Skipping test: jj command not available");
            return;
        };

        let backend =
            JjBackend::from_path(temp.path().to_path_buf()).expect("Failed to create jj backend");

        let commits = backend
            .get_recent_commits(5)
            .expect("Failed to get commits");

        // jj creates a working copy commit on top, so we may have 4 commits
        assert!(commits.len() >= 3, "Expected at least 3 commits");

        // All commits should have valid ids
        for commit in &commits {
            assert!(!commit.id.is_empty());
            assert!(!commit.short_id.is_empty());
        }

        // Check that our commit messages are present (may not be in exact order due to working copy)
        let summaries: Vec<_> = commits.iter().map(|c| c.summary.as_str()).collect();
        assert!(
            summaries.iter().any(|s| s.contains("First commit")),
            "Expected 'First commit' in {:?}",
            summaries
        );
        assert!(
            summaries.iter().any(|s| s.contains("Second commit")),
            "Expected 'Second commit' in {:?}",
            summaries
        );
        assert!(
            summaries.iter().any(|s| s.contains("Third commit")),
            "Expected 'Third commit' in {:?}",
            summaries
        );
    }

    #[test]
    fn test_jj_get_commit_range_diff() {
        let Some(temp) = setup_test_repo_with_commits() else {
            eprintln!("Skipping test: jj command not available");
            return;
        };

        let backend =
            JjBackend::from_path(temp.path().to_path_buf()).expect("Failed to create jj backend");

        let commits = backend
            .get_recent_commits(10)
            .expect("Failed to get commits");
        assert!(commits.len() >= 3, "Expected at least 3 commits");

        // Find the commits with our messages (skip empty working copy commit)
        let named_commits: Vec<_> = commits
            .iter()
            .filter(|c| {
                c.summary.contains("First commit")
                    || c.summary.contains("Second commit")
                    || c.summary.contains("Third commit")
            })
            .collect();

        if named_commits.len() >= 2 {
            // Get diff for two commits
            let oldest = &named_commits[named_commits.len() - 1]; // First commit
            let newest = &named_commits[0]; // Third commit

            let commit_ids = vec![oldest.id.clone(), newest.id.clone()];
            let diff = backend
                .get_commit_range_diff(&commit_ids, &SyntaxHighlighter::default())
                .expect("Failed to get commit range diff");

            // Should have changes
            assert!(!diff.is_empty(), "Expected non-empty diff");
        }
    }

    /// Create a test repo with a renamed file (no content changes).
    fn setup_test_repo_with_rename() -> Option<tempfile::TempDir> {
        if !jj_available() {
            return None;
        }

        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let root = temp_dir.path();

        // Initialize jj repo
        let output = Command::new("jj")
            .args(["git", "init"])
            .current_dir(root)
            .output()
            .expect("Failed to init jj repo");

        if !output.status.success() {
            return None;
        }

        // Create and commit a file
        fs::write(root.join("original.txt"), "file content\n").expect("Failed to write file");
        Command::new("jj")
            .args(["commit", "-m", "Add original file"])
            .current_dir(root)
            .output()
            .expect("Failed to commit");

        // Rename the file using jj file track after manual rename
        fs::rename(root.join("original.txt"), root.join("renamed.txt"))
            .expect("Failed to rename file");

        Some(temp_dir)
    }

    #[test]
    fn test_jj_renamed_file_without_content_changes() {
        let Some(temp) = setup_test_repo_with_rename() else {
            eprintln!("Skipping test: jj command not available");
            return;
        };

        let backend =
            JjBackend::from_path(temp.path().to_path_buf()).expect("Failed to create jj backend");

        let files = backend
            .get_working_tree_diff(&SyntaxHighlighter::default())
            .expect("Failed to get diff");

        // jj should detect the rename
        // Note: jj may show this as delete + add if it doesn't detect the rename
        assert!(!files.is_empty(), "Expected at least one file change");

        // Verify we can get display_path without panic (the bug we fixed)
        for file in &files {
            let _path = file.display_path();
        }
    }

    /// Create a test repo with a binary file.
    fn setup_test_repo_with_binary() -> Option<tempfile::TempDir> {
        if !jj_available() {
            return None;
        }

        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let root = temp_dir.path();

        // Initialize jj repo
        let output = Command::new("jj")
            .args(["git", "init"])
            .current_dir(root)
            .output()
            .expect("Failed to init jj repo");

        if !output.status.success() {
            return None;
        }

        // Create a binary file (PNG header bytes)
        let png_header: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        fs::write(root.join("image.png"), png_header).expect("Failed to write binary file");

        Some(temp_dir)
    }

    #[test]
    fn test_jj_binary_file_added() {
        let Some(temp) = setup_test_repo_with_binary() else {
            eprintln!("Skipping test: jj command not available");
            return;
        };

        let backend =
            JjBackend::from_path(temp.path().to_path_buf()).expect("Failed to create jj backend");

        let files = backend
            .get_working_tree_diff(&SyntaxHighlighter::default())
            .expect("Failed to get diff");

        assert_eq!(files.len(), 1, "Expected one file");

        let file = &files[0];
        // Verify we can get display_path without panic (the bug we fixed)
        let path = file.display_path();
        assert_eq!(path.to_str().unwrap(), "image.png");
        assert_eq!(file.status, FileStatus::Added);
    }

    #[test]
    fn test_jj_binary_file_deleted() {
        let Some(temp) = setup_test_repo_with_binary() else {
            eprintln!("Skipping test: jj command not available");
            return;
        };

        let root = temp.path();

        // Commit the binary file first
        Command::new("jj")
            .args(["commit", "-m", "Add binary file"])
            .current_dir(root)
            .output()
            .expect("Failed to commit");

        // Delete the binary file
        fs::remove_file(root.join("image.png")).expect("Failed to delete file");

        let backend =
            JjBackend::from_path(temp.path().to_path_buf()).expect("Failed to create jj backend");

        let files = backend
            .get_working_tree_diff(&SyntaxHighlighter::default())
            .expect("Failed to get diff");

        assert_eq!(files.len(), 1, "Expected one file");

        let file = &files[0];
        // Verify we can get display_path without panic (the bug we fixed)
        let path = file.display_path();
        assert_eq!(path.to_str().unwrap(), "image.png");
        assert_eq!(file.status, FileStatus::Deleted);
    }
}
