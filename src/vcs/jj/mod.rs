//! Jujutsu (jj) backend implementation using CLI commands.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::{Result, TuicrError};
use crate::model::{DiffFile, DiffLine, FileStatus, LineOrigin};
use crate::vcs::diff_parser::{self, DiffFormat};
use crate::vcs::traits::{VcsBackend, VcsInfo, VcsType};

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

    fn get_working_tree_diff(&self) -> Result<Vec<DiffFile>> {
        // Get unified diff output from jj using --git format
        let diff_output = run_jj_command(&self.info.root_path, &["diff", "--git"])?;

        if diff_output.trim().is_empty() {
            return Err(TuicrError::NoChanges);
        }

        diff_parser::parse_unified_diff(&diff_output, DiffFormat::GitStyle)
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

    // Note: get_recent_commits and get_commit_range_diff use default
    // implementations that return empty/error, since we only support
    // working tree diff for jj initially
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

        let files = backend.get_working_tree_diff().expect("Failed to get diff");

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
}
