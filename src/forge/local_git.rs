//! Local git reads shared by the forge backends.
//!
//! Every backend prefers a blob from a local checkout over a REST round trip
//! when the PR's SHAs happen to be present, and editor-target resolution needs
//! the same read to open the reviewed revision. One copy lives here so those
//! callers cannot drift apart.

use std::ffi::OsStr;
use std::path::Path;

use crate::process::{CommandOutputResult, run_command_output_with_env};

/// Run git with lazy promisor fetches disabled.
///
/// In a partial clone (`--filter=blob:none`), `git cat-file -e` on a missing
/// object does not just fail: it starts a lazy fetch with `noop` negotiation,
/// which can take tens of seconds. These helpers only check whether the
/// checkout has the object, and callers fall back to the forge when it does
/// not, so a missing object must fail fast. Git older than 2.45 ignores the
/// variable and keeps its previous behavior.
pub(crate) fn run_git_probe<I, S>(repo_root: &Path, args: I) -> CommandOutputResult<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    run_command_output_with_env("git", Some(repo_root), args, &[("GIT_NO_LAZY_FETCH", "1")])
}

/// Whether `s` can name a git object by hex: 4 to 64 ASCII hex digits.
///
/// Forges disagree on length (Bitbucket sends 12-character hashes, SHA-256
/// repositories use 64), so only the alphabet and the git minimum are checked.
/// This keeps a forge-supplied id from being read as an option or a revision
/// expression such as `HEAD`.
fn is_hex_object_id(s: &str) -> bool {
    (4..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Whether the object named by `spec` exists. No validation: `spec` must
/// already have passed [`is_hex_object_id`]; `--end-of-options` is the second
/// defence should it not have.
fn object_exists_unchecked(repo_root: &Path, spec: &str) -> bool {
    run_git_probe(repo_root, cat_file_exists_args(spec)).is_ok()
}

/// Arguments of the existence probe. Split out because no option-shaped spec
/// changes the probe's exit status (`cat-file -e` fails on unknown options
/// just as it fails on unknown objects), so only the argv can pin the guard.
fn cat_file_exists_args(spec: &str) -> Vec<&str> {
    vec!["cat-file", "-e", "--end-of-options", spec]
}

/// `git show <spec>` without validation; see [`object_exists_unchecked`].
fn show_object_unchecked(repo_root: &Path, spec: &str) -> Option<String> {
    run_git_probe(
        repo_root,
        ["show", "--end-of-options", spec]
            .iter()
            .map(|s| OsStr::new(*s)),
    )
    .ok()
}

/// `git show -s --format=%B <spec>` without validation; see
/// [`object_exists_unchecked`].
fn show_message_unchecked(repo_root: &Path, spec: &str) -> Option<String> {
    run_git_probe(
        repo_root,
        ["show", "-s", "--format=%B", "--end-of-options", spec]
            .iter()
            .map(|s| OsStr::new(*s)),
    )
    .ok()
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
    // git only resolves `<sha>:<path>` with slashes; see `forge_path`.
    let spec = format!("{}:{}", sha, crate::forge::forge_path(path));
    if !object_exists_unchecked(repo_root, &spec) {
        return None;
    }
    show_object_unchecked(repo_root, &spec)
}

/// Whether the commit `sha` exists in the checkout at `repo_root`.
pub(crate) fn commit_present(repo_root: &Path, sha: &str) -> bool {
    is_hex_object_id(sha) && object_exists_unchecked(repo_root, &format!("{sha}^{{commit}}"))
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
    show_message_unchecked(repo_root, &format!("{sha}^{{commit}}"))
}

/// Shared git fixtures for the tests of every backend that reads the clone.
#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use crate::process::run_command_output;

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

    /// Clone `origin` without blobs and without a working tree, so the clone
    /// holds its commits and trees but none of the file contents. Git fetches
    /// a missing object lazily from this remote unless the caller opts out.
    pub(crate) fn clone_without_blobs(origin: &Path, clone: &Path) {
        git(origin, &["config", "uploadpack.allowFilter", "true"]);
        let url = format!("file://{}", origin.display());
        run_command_output(
            "git",
            None,
            [
                "clone",
                "--quiet",
                "--filter=blob:none",
                "--no-checkout",
                "--no-local",
                &url,
                clone.to_str().expect("utf-8 clone path"),
            ]
            .iter()
            .map(|s| OsStr::new(*s)),
        )
        .expect("partial clone");
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{clone_without_blobs, git, init_repo_with_commit};
    use super::*;
    use crate::process::run_command_output;
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

    /// In a `blob:none` partial clone a missing blob is not an error to git:
    /// `cat-file -e` starts a lazy promisor fetch. The probe must disable that
    /// and report the blob as absent so callers use the forge API instead.
    #[test]
    fn should_not_lazy_fetch_a_missing_blob_from_a_partial_clone() {
        let dir = tempfile::tempdir().expect("tempdir");
        let origin = dir.path().join("origin");
        std::fs::create_dir(&origin).expect("origin dir");
        let sha = init_repo_with_commit(&origin, &["add file"]);
        let clone = dir.path().join("clone");
        clone_without_blobs(&origin, &clone);

        assert_eq!(read_blob(&clone, &sha, Path::new("file.txt")), None);
        assert_eq!(
            show_object_unchecked(&clone, &format!("{sha}:file.txt")),
            None
        );
    }

    #[test]
    fn should_not_lazy_fetch_a_missing_commit_message_from_a_partial_clone() {
        let dir = tempfile::tempdir().expect("tempdir");
        let origin = dir.path().join("origin");
        std::fs::create_dir(&origin).expect("origin dir");
        init_repo_with_commit(&origin, &["one"]);
        let clone = dir.path().join("clone");
        clone_without_blobs(&origin, &clone);

        std::fs::write(origin.join("file.txt"), "three\n").expect("write");
        git(&origin, &["commit", "-qam", "two"]);
        let second = git(&origin, &["rev-parse", "HEAD"]).trim().to_string();

        assert_eq!(read_commit_message(&clone, &second), None);
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

    /// `--output=<file>` makes `git show` write `<file>` when it is parsed as
    /// an option, so the unchecked helpers must not let a spec be one.
    #[test]
    fn should_put_end_of_options_right_before_the_cat_file_spec() {
        let args = cat_file_exists_args("--output=probe");

        assert_eq!(
            args[args.len() - 2..],
            ["--end-of-options", "--output=probe"]
        );
    }

    #[test]
    fn should_not_let_a_spec_write_files_through_git_show_blob() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        init_repo_with_commit(root, &["add file"]);

        let out = show_object_unchecked(root, "--output=probe");

        assert_eq!(out, None);
        assert!(!root.join("probe").exists());
    }

    #[test]
    fn should_not_let_a_spec_write_files_through_git_show_message() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        init_repo_with_commit(root, &["add file"]);

        let out = show_message_unchecked(root, "--output=probe");

        assert_eq!(out, None);
        assert!(!root.join("probe").exists());
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

    #[cfg(unix)]
    #[test]
    fn reads_a_blob_whose_name_has_a_backslash() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        git(root, &["init", "--quiet"]);
        git(root, &["config", "user.email", "test@example.com"]);
        git(root, &["config", "user.name", "test"]);
        std::fs::write(root.join("a\\b.rs"), "x\n").expect("write");
        git(root, &["add", "--", "a\\b.rs"]);
        git(root, &["commit", "--quiet", "-m", "add file"]);
        let sha = run_command_output(
            "git",
            Some(root),
            ["rev-parse", "HEAD"].iter().map(|s| OsStr::new(*s)),
        )
        .expect("rev-parse");

        assert_eq!(
            read_blob(root, sha.trim(), &PathBuf::from("a\\b.rs")),
            Some("x\n".to_string())
        );
    }
}
