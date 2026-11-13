//! Local git reads shared by the forge backends.
//!
//! Every backend prefers a blob from a local checkout over a REST round trip
//! when the PR's SHAs happen to be present, and editor-target resolution needs
//! the same read to open the reviewed revision. One copy lives here so those
//! callers cannot drift apart.

use std::ffi::OsStr;
use std::path::Path;

use crate::process::run_command_output;

/// Whether `s` can name a git object by hex: 4 to 64 ASCII hex digits.
///
/// Forges disagree on length (Bitbucket sends 12-character hashes, SHA-256
/// repositories use 64), so only the alphabet and the git minimum are checked.
/// This keeps a forge-supplied id from being read as an option or a revision
/// expression such as `HEAD`.
fn is_hex_object_id(s: &str) -> bool {
    (4..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Read a git blob from the checkout at `repo_root` using
/// `git show <sha>:<path>`.
///
/// Returns `None` on any failure — `sha` is not a hex object id, the object is
/// missing, `repo_root` is not a checkout, or `git` is not on PATH — so
/// callers fall back to the forge API.
/// The content is lossy UTF-8, so binary blobs come back mangled; callers that
/// can see a file's binary flag should check it first.
pub(crate) fn read_blob(repo_root: &Path, sha: &str, path: &Path) -> Option<String> {
    if !is_hex_object_id(sha) {
        return None;
    }
    // Diff paths arrive with forward slashes, but a path joined on Windows can
    // carry backslashes, and git only resolves `<sha>:<path>` with slashes.
    let spec = format!("{}:{}", sha, path.to_string_lossy().replace('\\', "/"));
    let exists = run_command_output(
        "git",
        Some(repo_root),
        ["cat-file", "-e", "--end-of-options", spec.as_str()]
            .iter()
            .map(|s| OsStr::new(*s)),
    );
    if exists.is_err() {
        return None;
    }
    run_command_output(
        "git",
        Some(repo_root),
        ["show", "--end-of-options", spec.as_str()]
            .iter()
            .map(|s| OsStr::new(*s)),
    )
    .ok()
}

/// Whether the commit `sha` exists in the checkout at `repo_root`.
pub(crate) fn commit_present(repo_root: &Path, sha: &str) -> bool {
    if !is_hex_object_id(sha) {
        return false;
    }
    let spec = format!("{sha}^{{commit}}");
    run_command_output(
        "git",
        Some(repo_root),
        ["cat-file", "-e", "--end-of-options", spec.as_str()]
            .iter()
            .map(|s| OsStr::new(*s)),
    )
    .is_ok()
}

/// Read a commit's full raw message from the checkout at `repo_root` using
/// `git show -s --format=%B <sha>^{commit}`.
///
/// Returns `None` on any failure — `sha` is not a hex object id, the object is
/// missing or not a commit, `repo_root` is not a checkout, or `git` is not on
/// PATH — so callers fall back to the forge API. The message is lossy UTF-8.
pub(crate) fn read_commit_message(repo_root: &Path, sha: &str) -> Option<String> {
    if !is_hex_object_id(sha) {
        return None;
    }
    let spec = format!("{sha}^{{commit}}");
    run_command_output(
        "git",
        Some(repo_root),
        [
            "show",
            "-s",
            "--format=%B",
            "--end-of-options",
            spec.as_str(),
        ]
        .iter()
        .map(|s| OsStr::new(*s)),
    )
    .ok()
}

/// Shared git fixtures for the tests of every backend that reads the clone.
#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    pub(crate) fn git(dir: &Path, args: &[&str]) -> String {
        run_command_output("git", Some(dir), args.iter().map(|s| OsStr::new(*s)))
            .expect("git command")
    }

    /// `git init` with a committer identity and signing off.
    pub(crate) fn init_repo(root: &Path) {
        git(root, &["init", "--quiet"]);
        git(root, &["config", "user.email", "test@example.com"]);
        git(root, &["config", "user.name", "test"]);
        git(root, &["config", "commit.gpgsign", "false"]);
    }

    /// Initialise `root` and commit `file.txt` (`"one\ntwo\n"`), one `-m` per
    /// entry of `messages`. Returns the commit's full sha.
    pub(crate) fn init_repo_with_commit(root: &Path, messages: &[&str]) -> String {
        init_repo(root);
        std::fs::write(root.join("file.txt"), "one\ntwo\n").expect("write");
        git(root, &["add", "file.txt"]);
        let mut args = vec!["commit", "--quiet"];
        for message in messages {
            args.extend(["-m", message]);
        }
        git(root, &args);
        git(root, &["rev-parse", "HEAD"]).trim().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::init_repo_with_commit;
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn reads_a_blob_at_a_commit() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let sha = init_repo_with_commit(root, &["add file"]);

        // The worktree copy changing must not change what the blob read
        // returns — that difference is exactly what editor targets check.
        std::fs::write(root.join("file.txt"), "clobbered\n").expect("write");

        assert_eq!(
            read_blob(root, &sha, &PathBuf::from("file.txt")),
            Some("one\ntwo\n".to_string())
        );
        assert_eq!(
            read_blob(root, &sha[..12], &PathBuf::from("file.txt")),
            Some("one\ntwo\n".to_string())
        );
        assert_eq!(read_blob(root, &sha, &PathBuf::from("missing.txt")), None);
        assert_eq!(
            read_blob(root, "deadbeef", &PathBuf::from("file.txt")),
            None
        );
    }

    #[test]
    fn should_reject_anything_but_a_hex_object_id() {
        // given
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        init_repo_with_commit(root, &["add file"]);
        let path = PathBuf::from("file.txt");

        // when / then
        for bad in ["", "HEAD", "-x", "abc", "zzzz1234"] {
            assert_eq!(read_blob(root, bad, &path), None, "blob {bad:?}");
            assert_eq!(read_commit_message(root, bad), None, "message {bad:?}");
            assert!(!commit_present(root, bad), "present {bad:?}");
        }
    }

    #[test]
    fn should_not_let_a_sha_write_files_through_git_options() {
        // given
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        init_repo_with_commit(root, &["add file"]);

        // when
        let blob = read_blob(root, "--output=probe", &PathBuf::from("file.txt"));
        let message = read_commit_message(root, "--output=probe");

        // then
        assert_eq!(blob, None);
        assert_eq!(message, None);
        assert!(!root.join("probe").exists());
        assert!(!root.join("probe^{commit}").exists());
    }

    #[test]
    fn should_read_a_full_commit_message() {
        // given
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let sha = init_repo_with_commit(
            root,
            &[
                "Fix the parser",
                "Handle nested expressions.",
                "Keep context.",
            ],
        );

        // when
        let message = read_commit_message(root, &sha).expect("commit message");
        let abbreviated = read_commit_message(root, &sha[..12]).expect("abbreviated");
        let missing = read_commit_message(root, "deadbeef");

        // then
        let expected = "Fix the parser\n\nHandle nested expressions.\n\nKeep context.";
        assert_eq!(message.trim_end_matches('\n'), expected);
        assert_eq!(abbreviated.trim_end_matches('\n'), expected);
        assert_eq!(missing, None);
    }
}
