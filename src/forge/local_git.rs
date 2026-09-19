//! Local git reads shared by the forge backends.
//!
//! Every backend prefers a blob from a local checkout over a REST round trip
//! when the PR's SHAs happen to be present, and editor-target resolution needs
//! the same read to open the reviewed revision. One copy lives here so those
//! callers cannot drift apart.

use std::ffi::OsStr;
use std::path::Path;

use crate::process::run_command_output;

/// Read a git blob from the checkout at `repo_root` using
/// `git show <sha>:<path>`.
///
/// Returns `None` on any failure — the object is missing, `repo_root` is not a
/// checkout, or `git` is not on PATH — so callers fall back to the forge API.
/// The content is lossy UTF-8, so binary blobs come back mangled; callers that
/// can see a file's binary flag should check it first.
pub(crate) fn read_blob(repo_root: &Path, sha: &str, path: &Path) -> Option<String> {
    // Diff paths arrive with forward slashes, but a path joined on Windows can
    // carry backslashes, and git only resolves `<sha>:<path>` with slashes.
    let spec = format!("{}:{}", sha, path.to_string_lossy().replace('\\', "/"));
    let exists = run_command_output(
        "git",
        Some(repo_root),
        ["cat-file", "-e", spec.as_str()]
            .iter()
            .map(|s| OsStr::new(*s)),
    );
    if exists.is_err() {
        return None;
    }
    run_command_output(
        "git",
        Some(repo_root),
        ["show", spec.as_str()].iter().map(|s| OsStr::new(*s)),
    )
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::run_command_output;
    use std::path::PathBuf;

    fn git(dir: &Path, args: &[&str]) {
        run_command_output("git", Some(dir), args.iter().map(|s| OsStr::new(*s)))
            .expect("git command");
    }

    #[test]
    fn reads_a_blob_at_a_commit() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        git(root, &["init", "--quiet"]);
        git(root, &["config", "user.email", "test@example.com"]);
        git(root, &["config", "user.name", "test"]);
        std::fs::write(root.join("file.txt"), "one\ntwo\n").expect("write");
        git(root, &["add", "file.txt"]);
        git(root, &["commit", "--quiet", "-m", "add file"]);
        let sha = run_command_output(
            "git",
            Some(root),
            ["rev-parse", "HEAD"].iter().map(|s| OsStr::new(*s)),
        )
        .expect("rev-parse");
        let sha = sha.trim();

        // The worktree copy changing must not change what the blob read
        // returns — that difference is exactly what editor targets check.
        std::fs::write(root.join("file.txt"), "clobbered\n").expect("write");

        assert_eq!(
            read_blob(root, sha, &PathBuf::from("file.txt")),
            Some("one\ntwo\n".to_string())
        );
        assert_eq!(read_blob(root, sha, &PathBuf::from("missing.txt")), None);
        assert_eq!(
            read_blob(root, "deadbeef", &PathBuf::from("file.txt")),
            None
        );
    }
}
