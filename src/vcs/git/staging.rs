use git2::Repository;
use std::path::Path;
use std::process::Command;

use crate::error::{Result, TuicrError};

pub fn stage_file(repo: &Repository, path: &Path) -> Result<()> {
    let workdir = repo.workdir().ok_or(TuicrError::NotARepository)?;
    let output = Command::new("git")
        .current_dir(workdir)
        .arg("add")
        .arg("--")
        .arg(path)
        .output()
        .map_err(|e| TuicrError::VcsCommand(format!("Failed to run git: {}", e)))?;

    if !output.status.success() {
        return Err(TuicrError::VcsCommand(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn stage_file_adds_to_index() {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let repo = Repository::init(temp_dir.path()).expect("failed to init repo");

        let file_path = temp_dir.path().join("test.txt");
        fs::write(&file_path, "hello\n").unwrap();

        stage_file(&repo, Path::new("test.txt")).unwrap();

        let output = Command::new("git")
            .current_dir(temp_dir.path())
            .args(["diff", "--cached", "--name-only"])
            .output()
            .expect("failed to run git");
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "test.txt");
    }

    #[test]
    fn stage_file_supports_sparse_index() {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let repo = Repository::init(temp_dir.path()).expect("failed to init repo");

        for args in [
            &["config", "user.email", "test@example.com"][..],
            &["config", "user.name", "Test User"],
        ] {
            let status = Command::new("git")
                .current_dir(temp_dir.path())
                .args(args)
                .status()
                .expect("failed to run git");
            assert!(status.success());
        }

        fs::create_dir_all(temp_dir.path().join("keep")).unwrap();
        fs::create_dir_all(temp_dir.path().join("hidden")).unwrap();
        fs::write(temp_dir.path().join("keep/file.txt"), "keep base\n").unwrap();
        fs::write(temp_dir.path().join("hidden/file.txt"), "hidden base\n").unwrap();
        for args in [
            &["add", "."][..],
            &["commit", "-m", "initial"],
            &["sparse-checkout", "init", "--cone"],
            &["sparse-checkout", "set", "keep"],
            &["sparse-checkout", "reapply", "--sparse-index"],
        ] {
            let status = Command::new("git")
                .current_dir(temp_dir.path())
                .args(args)
                .status()
                .expect("failed to run git");
            assert!(status.success(), "git {args:?} failed");
        }

        fs::write(temp_dir.path().join("keep/file.txt"), "keep staged\n").unwrap();

        stage_file(&repo, Path::new("keep/file.txt")).unwrap();

        let output = Command::new("git")
            .current_dir(temp_dir.path())
            .args(["diff", "--cached", "--name-only"])
            .output()
            .expect("failed to run git");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "keep/file.txt"
        );
    }
}
