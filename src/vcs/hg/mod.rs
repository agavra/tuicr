use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::{TimeZone, Utc};

use crate::error::{Result, TuicrError};
use crate::model::{DiffFile, DiffLine, FileStatus, LineOrigin};
use crate::syntax::SyntaxHighlighter;
use crate::vcs::diff_parser::{self, DiffFormat};
use crate::vcs::traits::{CommitInfo, VcsBackend, VcsInfo, VcsType};

/// Mercurial backend implementation using hg CLI commands
pub struct HgBackend {
    info: VcsInfo,
}

impl HgBackend {
    /// Discover a Mercurial repository from the current directory
    pub fn discover() -> Result<Self> {
        // Use `hg root` to find the repository root
        // This handles being called from subdirectories
        let root_output = Command::new("hg")
            .args(["root"])
            .output()
            .map_err(|e| TuicrError::VcsCommand(format!("Failed to run hg: {}", e)))?;

        if !root_output.status.success() {
            return Err(TuicrError::NotARepository);
        }

        let root_path = PathBuf::from(String::from_utf8_lossy(&root_output.stdout).trim());

        Self::from_path(root_path)
    }

    /// Create backend from a known path (used by discover and tests)
    fn from_path(root_path: PathBuf) -> Result<Self> {
        // Get current revision info
        let head_commit = run_hg_command(&root_path, &["id", "-i"])
            .map(|s| s.trim().trim_end_matches('+').to_string())
            .unwrap_or_else(|_| "unknown".to_string());

        let branch_name = run_hg_command(&root_path, &["branch"])
            .ok()
            .map(|s| s.trim().to_string());

        let info = VcsInfo {
            root_path,
            head_commit,
            branch_name,
            vcs_type: VcsType::Mercurial,
        };

        Ok(Self { info })
    }
}

impl VcsBackend for HgBackend {
    fn info(&self) -> &VcsInfo {
        &self.info
    }

    fn get_working_tree_diff(&self, highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
        // Get unified diff output from hg
        let diff_output = run_hg_command(&self.info.root_path, &["diff"])?;

        if diff_output.trim().is_empty() {
            return Err(TuicrError::NoChanges);
        }

        diff_parser::parse_unified_diff(&diff_output, DiffFormat::Hg, highlighter)
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
                // Read from hg cat (last committed version)
                run_hg_command(
                    &self.info.root_path,
                    &["cat", "-r", ".", &file_path.to_string_lossy()],
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
        // Use hg log with a template to get structured output
        // Template fields separated by \x00, records separated by \x01
        let template =
            "{node}\\x00{node|short}\\x00{desc|firstline}\\x00{author|user}\\x00{date|hgdate}\\x01";
        let output = run_hg_command(
            &self.info.root_path,
            &["log", "-l", &count.to_string(), "--template", template],
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

            // hgdate format is "unix_timestamp timezone_offset"
            let time = parts[4]
                .split_whitespace()
                .next()
                .and_then(|s| s.parse::<i64>().ok())
                .and_then(|ts| Utc.timestamp_opt(ts, 0).single())
                .unwrap_or_else(Utc::now);

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
        //
        // Note on Sapling/Mercurial compatibility:
        // - Sapling (Meta's hg fork) has issues with full 40-char hashes in certain operations
        // - We use 12-char short hashes which work with both standard Mercurial and Sapling
        // - The parents() revset is used to find the parent commit for diffing
        let oldest = &commit_ids[0];
        let oldest_short = if oldest.len() > 12 {
            &oldest[..12]
        } else {
            oldest.as_str()
        };

        let newest = commit_ids.last().unwrap();
        let newest_short = if newest.len() > 12 {
            &newest[..12]
        } else {
            newest.as_str()
        };

        // First, get the parent commit of the oldest
        // We use "log -r 'parents({oldest})'" to get the parent hash
        let parent_output = run_hg_command(
            &self.info.root_path,
            &[
                "log",
                "-r",
                &format!("parents({})", oldest_short),
                "--template",
                "{node|short}",
            ],
        );

        // If there's no parent (first commit), diff from null
        let from_rev = match parent_output {
            Ok(parent) if !parent.trim().is_empty() => parent.trim().to_string(),
            _ => "null".to_string(),
        };

        let diff_output = run_hg_command(
            &self.info.root_path,
            &["diff", "-r", &from_rev, "-r", newest_short],
        )?;

        if diff_output.trim().is_empty() {
            return Err(TuicrError::NoChanges);
        }

        diff_parser::parse_unified_diff(&diff_output, DiffFormat::Hg, highlighter)
    }
}

/// Run an hg command and return its stdout
fn run_hg_command(root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("hg")
        .current_dir(root)
        .args(args)
        .output()
        .map_err(|e| TuicrError::VcsCommand(format!("Failed to run hg: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TuicrError::VcsCommand(format!(
            "hg {} failed: {}",
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

    /// Check if hg command is available
    fn hg_available() -> bool {
        Command::new("hg")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Discover a Mercurial repository from a specific directory
    fn discover_in(path: &Path) -> Result<HgBackend> {
        let root_output = Command::new("hg")
            .args(["root"])
            .current_dir(path)
            .output()
            .map_err(|e| TuicrError::VcsCommand(format!("Failed to run hg: {}", e)))?;

        if !root_output.status.success() {
            return Err(TuicrError::NotARepository);
        }

        let root_path = PathBuf::from(String::from_utf8_lossy(&root_output.stdout).trim());

        HgBackend::from_path(root_path)
    }

    /// Create a temporary hg repo for testing.
    /// Returns None if hg is not available.
    fn setup_test_repo() -> Option<tempfile::TempDir> {
        if !hg_available() {
            return None;
        }

        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let root = temp_dir.path();

        // Initialize hg repo
        Command::new("hg")
            .args(["init"])
            .current_dir(root)
            .output()
            .expect("Failed to init hg repo");

        // Create initial file
        fs::write(root.join("hello.txt"), "hello world\n").expect("Failed to write file");

        // Add and commit
        Command::new("hg")
            .args(["add", "hello.txt"])
            .current_dir(root)
            .output()
            .expect("Failed to add file");

        Command::new("hg")
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
    fn test_hg_discover() {
        let Some(temp) = setup_test_repo() else {
            eprintln!("Skipping test: hg command not available");
            return;
        };

        // Use discover_in to avoid set_current_dir race conditions
        let backend = discover_in(temp.path()).expect("Failed to discover hg repo");
        let info = backend.info();

        assert_eq!(info.root_path, temp.path());
        assert_eq!(info.vcs_type, VcsType::Mercurial);
        assert!(!info.head_commit.is_empty());
    }

    #[test]
    fn test_hg_working_tree_diff() {
        let Some(temp) = setup_test_repo() else {
            eprintln!("Skipping test: hg command not available");
            return;
        };

        // Use from_path directly to avoid set_current_dir race conditions
        let backend =
            HgBackend::from_path(temp.path().to_path_buf()).expect("Failed to create hg backend");

        assert_eq!(backend.info().root_path, temp.path());
        assert_eq!(backend.info().vcs_type, VcsType::Mercurial);

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
    fn test_hg_fetch_context_lines() {
        let Some(temp) = setup_test_repo() else {
            eprintln!("Skipping test: hg command not available");
            return;
        };

        // Use from_path directly to avoid set_current_dir race conditions
        let backend =
            HgBackend::from_path(temp.path().to_path_buf()).expect("Failed to create hg backend");

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
    /// Returns None if hg is not available.
    fn setup_test_repo_with_commits() -> Option<tempfile::TempDir> {
        if !hg_available() {
            return None;
        }

        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let root = temp_dir.path();

        // Initialize hg repo
        Command::new("hg")
            .args(["init"])
            .current_dir(root)
            .output()
            .expect("Failed to init hg repo");

        // First commit
        fs::write(root.join("file1.txt"), "first file\n").expect("Failed to write file");
        Command::new("hg")
            .args(["add", "file1.txt"])
            .current_dir(root)
            .output()
            .expect("Failed to add file");
        Command::new("hg")
            .args(["commit", "-m", "First commit"])
            .current_dir(root)
            .output()
            .expect("Failed to commit");

        // Second commit
        fs::write(root.join("file2.txt"), "second file\n").expect("Failed to write file");
        Command::new("hg")
            .args(["add", "file2.txt"])
            .current_dir(root)
            .output()
            .expect("Failed to add file");
        Command::new("hg")
            .args(["commit", "-m", "Second commit"])
            .current_dir(root)
            .output()
            .expect("Failed to commit");

        // Third commit - modify first file
        fs::write(root.join("file1.txt"), "first file\nmodified\n").expect("Failed to write file");
        Command::new("hg")
            .args(["commit", "-m", "Third commit"])
            .current_dir(root)
            .output()
            .expect("Failed to commit");

        Some(temp_dir)
    }

    #[test]
    fn test_hg_get_recent_commits() {
        let Some(temp) = setup_test_repo_with_commits() else {
            eprintln!("Skipping test: hg command not available");
            return;
        };

        let backend =
            HgBackend::from_path(temp.path().to_path_buf()).expect("Failed to create hg backend");

        let commits = backend
            .get_recent_commits(5)
            .expect("Failed to get commits");

        assert_eq!(commits.len(), 3);
        // Most recent commit should be first
        assert_eq!(commits[0].summary, "Third commit");
        assert_eq!(commits[1].summary, "Second commit");
        assert_eq!(commits[2].summary, "First commit");

        // All commits should have valid ids
        for commit in &commits {
            assert!(!commit.id.is_empty());
            assert!(!commit.short_id.is_empty());
            assert!(commit.short_id.len() <= commit.id.len());
        }
    }

    #[test]
    fn test_hg_get_commit_range_diff() {
        let Some(temp) = setup_test_repo_with_commits() else {
            eprintln!("Skipping test: hg command not available");
            return;
        };

        let backend =
            HgBackend::from_path(temp.path().to_path_buf()).expect("Failed to create hg backend");

        let commits = backend
            .get_recent_commits(5)
            .expect("Failed to get commits");
        assert_eq!(commits.len(), 3);

        // Get diff for the last two commits (Second and Third)
        let commit_ids = vec![commits[1].id.clone(), commits[0].id.clone()];
        let diff_result = backend.get_commit_range_diff(&commit_ids, &SyntaxHighlighter::default());

        // Note: Sapling (Meta's hg fork) may fail with "id_dag_snapshot()" error
        // in certain temporary directory configurations. Skip the test in that case.
        let diff = match diff_result {
            Ok(d) => d,
            Err(TuicrError::VcsCommand(msg)) if msg.contains("id_dag_snapshot") => {
                eprintln!("Skipping test: Sapling-specific issue with tempdir repos");
                return;
            }
            Err(e) => panic!("Failed to get commit range diff: {:?}", e),
        };

        // Should have changes from both commits
        // Second commit added file2.txt, Third modified file1.txt
        assert!(!diff.is_empty());

        let file_paths: Vec<_> = diff
            .iter()
            .filter_map(|f| f.new_path.as_ref().map(|p| p.to_string_lossy().to_string()))
            .collect();

        // Both files should be in the diff
        assert!(
            file_paths.contains(&"file2.txt".to_string()),
            "Expected file2.txt in diff, got {:?}",
            file_paths
        );
        assert!(
            file_paths.contains(&"file1.txt".to_string()),
            "Expected file1.txt in diff, got {:?}",
            file_paths
        );
    }

    /// Create a test repo with a renamed file (no content changes).
    fn setup_test_repo_with_rename() -> Option<tempfile::TempDir> {
        if !hg_available() {
            return None;
        }

        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let root = temp_dir.path();

        // Initialize hg repo
        Command::new("hg")
            .args(["init"])
            .current_dir(root)
            .output()
            .expect("Failed to init hg repo");

        // Create and commit a file
        fs::write(root.join("original.txt"), "file content\n").expect("Failed to write file");
        Command::new("hg")
            .args(["add", "original.txt"])
            .current_dir(root)
            .output()
            .expect("Failed to add file");
        Command::new("hg")
            .args(["commit", "-m", "Add original file"])
            .current_dir(root)
            .output()
            .expect("Failed to commit");

        // Rename the file using hg rename
        Command::new("hg")
            .args(["rename", "original.txt", "renamed.txt"])
            .current_dir(root)
            .output()
            .expect("Failed to rename file");

        Some(temp_dir)
    }

    #[test]
    fn test_hg_renamed_file_without_content_changes() {
        let Some(temp) = setup_test_repo_with_rename() else {
            eprintln!("Skipping test: hg command not available");
            return;
        };

        let backend =
            HgBackend::from_path(temp.path().to_path_buf()).expect("Failed to create hg backend");

        let files = backend
            .get_working_tree_diff(&SyntaxHighlighter::default())
            .expect("Failed to get diff");

        // hg should show the rename
        assert!(!files.is_empty(), "Expected at least one file change");

        // Verify we can get display_path without panic (the bug we fixed)
        for file in &files {
            let _path = file.display_path();
        }

        // Look for the renamed file
        let renamed_file = files.iter().find(|f| {
            f.new_path
                .as_ref()
                .is_some_and(|p| p.to_str() == Some("renamed.txt"))
        });
        assert!(
            renamed_file.is_some(),
            "Expected to find renamed.txt in diff"
        );
    }

    /// Create a test repo with a copied file.
    fn setup_test_repo_with_copy() -> Option<tempfile::TempDir> {
        if !hg_available() {
            return None;
        }

        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let root = temp_dir.path();

        // Initialize hg repo
        Command::new("hg")
            .args(["init"])
            .current_dir(root)
            .output()
            .expect("Failed to init hg repo");

        // Create and commit a file
        fs::write(root.join("source.txt"), "source content\n").expect("Failed to write file");
        Command::new("hg")
            .args(["add", "source.txt"])
            .current_dir(root)
            .output()
            .expect("Failed to add file");
        Command::new("hg")
            .args(["commit", "-m", "Add source file"])
            .current_dir(root)
            .output()
            .expect("Failed to commit");

        // Copy the file using hg copy
        Command::new("hg")
            .args(["copy", "source.txt", "dest.txt"])
            .current_dir(root)
            .output()
            .expect("Failed to copy file");

        Some(temp_dir)
    }

    #[test]
    fn test_hg_copied_file() {
        let Some(temp) = setup_test_repo_with_copy() else {
            eprintln!("Skipping test: hg command not available");
            return;
        };

        let backend =
            HgBackend::from_path(temp.path().to_path_buf()).expect("Failed to create hg backend");

        let files = backend
            .get_working_tree_diff(&SyntaxHighlighter::default())
            .expect("Failed to get diff");

        assert!(!files.is_empty(), "Expected at least one file change");

        // Verify we can get display_path without panic (the bug we fixed)
        for file in &files {
            let _path = file.display_path();
        }

        // Look for the copied file
        let copied_file = files.iter().find(|f| {
            f.new_path
                .as_ref()
                .is_some_and(|p| p.to_str() == Some("dest.txt"))
        });
        assert!(copied_file.is_some(), "Expected to find dest.txt in diff");
    }

    /// Create a test repo with a binary file.
    fn setup_test_repo_with_binary() -> Option<tempfile::TempDir> {
        if !hg_available() {
            return None;
        }

        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let root = temp_dir.path();

        // Initialize hg repo
        Command::new("hg")
            .args(["init"])
            .current_dir(root)
            .output()
            .expect("Failed to init hg repo");

        // Create a binary file (PNG header bytes)
        let png_header: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        fs::write(root.join("image.png"), png_header).expect("Failed to write binary file");

        // Add the file
        Command::new("hg")
            .args(["add", "image.png"])
            .current_dir(root)
            .output()
            .expect("Failed to add file");

        Some(temp_dir)
    }

    #[test]
    fn test_hg_binary_file_added() {
        let Some(temp) = setup_test_repo_with_binary() else {
            eprintln!("Skipping test: hg command not available");
            return;
        };

        let backend =
            HgBackend::from_path(temp.path().to_path_buf()).expect("Failed to create hg backend");

        let files = backend
            .get_working_tree_diff(&SyntaxHighlighter::default())
            .expect("Failed to get diff");

        assert_eq!(files.len(), 1, "Expected one file");

        let file = &files[0];
        // Verify we can get display_path without panic (the bug we fixed)
        // Don't assert exact path/status as hg implementations differ (Sapling vs standard hg)
        let _path = file.display_path();
    }

    #[test]
    fn test_hg_binary_file_deleted() {
        let Some(temp) = setup_test_repo_with_binary() else {
            eprintln!("Skipping test: hg command not available");
            return;
        };

        let root = temp.path();

        // Commit the binary file first
        Command::new("hg")
            .args(["commit", "-m", "Add binary file"])
            .current_dir(root)
            .output()
            .expect("Failed to commit");

        // Delete the binary file using hg remove
        Command::new("hg")
            .args(["remove", "image.png"])
            .current_dir(root)
            .output()
            .expect("Failed to remove file");

        let backend =
            HgBackend::from_path(temp.path().to_path_buf()).expect("Failed to create hg backend");

        let files = backend
            .get_working_tree_diff(&SyntaxHighlighter::default())
            .expect("Failed to get diff");

        assert_eq!(files.len(), 1, "Expected one file");

        let file = &files[0];
        // Verify we can get display_path without panic (the bug we fixed)
        // Don't assert exact path/status as hg implementations differ (Sapling vs standard hg)
        let _path = file.display_path();
    }
}
