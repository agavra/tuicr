//! Pristine review path enumeration.
//!
//! Whole-repo review mode (`tuicr --all-files`) needs a list of every file
//! the user can annotate. This module shells out to `git ls-files -z` to
//! enumerate the tracked set, so untracked build artifacts (`target/`,
//! `node_modules/`, etc.) are excluded without having to maintain a
//! deny-list and without depending on the absence of a `.gitignore`.
//!
//! The MVP is git-only; jj and mercurial support is deferred to a future
//! follow-up that hoists this responsibility into the `VcsBackend` trait.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::{Result, TuicrError};

/// The literal used in place of a HEAD SHA when the repository has no
/// commits yet. Keeps the `pristine:HEAD:hash` session key well-formed in
/// the empty-repo case.
const NO_HEAD_SENTINEL: &str = "none";

/// Enumerate every tracked file in the git repository rooted at `repo_root`,
/// returning absolute paths.
///
/// The list is taken from `git ls-files -z`, so it reflects exactly what
/// git considers tracked (post-`.gitignore`, post-`.git/info/exclude`,
/// untracked files excluded). Deleted-but-tracked entries are filtered out
/// at the boundary: a path that no longer exists on disk is dropped.
///
/// # Errors
///
/// Returns [`TuicrError::NotARepository`] if `git ls-files` cannot find a
/// repository at `repo_root` (or git is not on `PATH`). Returns
/// [`TuicrError::NoChanges`] when the repository exists but has no tracked
/// files on disk.
pub fn collect_tracked_paths(repo_root: &Path) -> Result<Vec<PathBuf>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("ls-files")
        .arg("-z")
        .output()
        .map_err(|_| TuicrError::NotARepository)?;

    if !output.status.success() {
        return Err(TuicrError::NotARepository);
    }

    let mut paths: Vec<PathBuf> = output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
        .map(|part| {
            repo_root.join(std::path::Path::new(
                std::str::from_utf8(part).unwrap_or(""),
            ))
        })
        .filter(|path| path.is_file())
        .collect();

    paths.sort();

    if paths.is_empty() {
        return Err(TuicrError::NoChanges);
    }

    Ok(paths)
}

/// Return the short SHA of HEAD for the git repo at `repo_root`, or the
/// `"none"` sentinel if HEAD is unborn (e.g. a freshly-initialized repo
/// with no commits) or any subprocess error occurs.
///
/// The result is used as a component of pristine session keys; an
/// advancing HEAD changes the key but the persistence-layer prefix-match
/// keeps comments attached across `git pull`.
#[must_use]
pub fn head_short_sha(repo_root: &Path) -> String {
    let output = match Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["rev-parse", "--short", "HEAD"])
        .output()
    {
        Ok(out) if out.status.success() => out,
        _ => return NO_HEAD_SENTINEL.to_string(),
    };
    let trimmed = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if trimmed.is_empty() {
        NO_HEAD_SENTINEL.to_string()
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::process::Command;

    use super::*;

    fn init_git_repo(dir: &Path) {
        Command::new("git")
            .args(["-C"])
            .arg(dir)
            .arg("init")
            .arg("-q")
            .output()
            .expect("git init");
        Command::new("git")
            .args(["-C"])
            .arg(dir)
            .args(["config", "user.email", "tester@example.com"])
            .output()
            .expect("git config email");
        Command::new("git")
            .args(["-C"])
            .arg(dir)
            .args(["config", "user.name", "Tester"])
            .output()
            .expect("git config name");
    }

    fn git_add_commit(dir: &Path) {
        Command::new("git")
            .args(["-C"])
            .arg(dir)
            .args(["add", "-A"])
            .output()
            .expect("git add");
        Command::new("git")
            .args(["-C"])
            .arg(dir)
            .args(["commit", "-q", "-m", "init"])
            .output()
            .expect("git commit");
    }

    #[test]
    fn errors_when_not_a_git_repo() {
        let dir = tempfile::tempdir().unwrap();
        let result = collect_tracked_paths(dir.path());
        assert!(matches!(result, Err(TuicrError::NotARepository)));
    }

    #[test]
    fn errors_when_repo_has_no_tracked_files() {
        let dir = tempfile::tempdir().unwrap();
        init_git_repo(dir.path());
        let result = collect_tracked_paths(dir.path());
        assert!(matches!(result, Err(TuicrError::NoChanges)));
    }

    #[test]
    fn lists_only_tracked_files() {
        let dir = tempfile::tempdir().unwrap();
        init_git_repo(dir.path());
        fs::write(dir.path().join("kept.txt"), "hello\n").unwrap();
        fs::write(dir.path().join(".gitignore"), "ignored.txt\n").unwrap();
        fs::write(dir.path().join("ignored.txt"), "skip me\n").unwrap();
        git_add_commit(dir.path());
        // Add an untracked file after the commit -- must NOT appear.
        fs::write(dir.path().join("untracked.txt"), "untracked\n").unwrap();

        let paths = collect_tracked_paths(dir.path()).unwrap();
        let names: Vec<String> = paths
            .iter()
            .map(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string()
            })
            .collect();

        assert!(names.contains(&"kept.txt".to_string()));
        assert!(names.contains(&".gitignore".to_string()));
        assert!(!names.contains(&"ignored.txt".to_string()));
        assert!(!names.contains(&"untracked.txt".to_string()));
    }

    #[test]
    fn drops_deleted_but_tracked_entries() {
        let dir = tempfile::tempdir().unwrap();
        init_git_repo(dir.path());
        fs::write(dir.path().join("kept.txt"), "k\n").unwrap();
        fs::write(dir.path().join("removed.txt"), "r\n").unwrap();
        git_add_commit(dir.path());
        // Remove from disk but keep in the index by skipping `git rm`.
        fs::remove_file(dir.path().join("removed.txt")).unwrap();

        let paths = collect_tracked_paths(dir.path()).unwrap();
        let names: Vec<String> = paths
            .iter()
            .filter_map(|p| p.file_name()?.to_str().map(str::to_string))
            .collect();

        assert!(names.contains(&"kept.txt".to_string()));
        assert!(!names.contains(&"removed.txt".to_string()));
    }
}
