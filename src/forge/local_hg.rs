//! Local hg reads shared by the forge backends.
//!
//! Symmetric to [`crate::forge::local_git`]: when a forge backend has a local
//! checkout that happens to be Mercurial (e.g. an Heptapod MR opened from a
//! matching `hg` checkout), the blob primitive must route through `hg` rather
//! than `git`. Returns `None` the same way `local_git::read_blob` does — the
//! source of truth is still the forge.
//!
//! Range-diff construction is heavier for Mercurial than for Git (it needs
//! the `hg status` + `hg diff --git` two-step the local [`HgBackend`][crate::vcs::HgBackend]
//! uses), so it lives in [`crate::vcs::hg::run_hg_diff_range`] and is re-exported
//! here for the forge layer.

use std::ffi::OsStr;
use std::path::Path;

use crate::process::run_command_output;

pub(crate) use crate::vcs::run_hg_diff_range;

/// Read a Mercurial file revision from the checkout at `repo_root` using
/// `hg cat -r <rev> <path>`.
///
/// Returns `None` on any failure — the revision is missing, `repo_root` is
/// not an `hg` checkout, or `hg` is not on PATH — so callers fall back to the
/// forge API. The content is lossy UTF-8, so binary blobs come back mangled;
/// callers that can see a file's binary flag should check it first.
///
/// The path is wrapped in a `path:` pattern prefix so a name starting with
/// `re:` / `glob:` / … is treated verbatim rather than as a Mercurial pattern.
pub(crate) fn read_blob_hg(repo_root: &Path, rev: &str, path: &Path) -> Option<String> {
    // Cheap existence check first, so a missing revision doesn't print a
    // noisy `abort:` to the user's stderr.
    let exists = run_command_output(
        "hg",
        Some(repo_root),
        ["log", "-r", rev, "--hidden", "-T", "exists"]
            .iter()
            .map(|s| OsStr::new(*s)),
    );
    if exists.is_err() || !exists.unwrap().contains("exists") {
        return None;
    }

    let pattern = hg_path_pattern(path);
    run_command_output(
        "hg",
        Some(repo_root),
        ["cat", "-r", rev, pattern.as_str()]
            .iter()
            .map(|s| OsStr::new(*s)),
    )
    .ok()
}

/// `hg cat -r` treats positional file arguments as patterns. The `path:`
/// prefix pins the rest of the argument to a verbatim repository-root-relative
/// path. Mirrors `HgBackend`'s `hg_path_pattern`.
fn hg_path_pattern(path: &Path) -> String {
    format!("path:{}", path.to_string_lossy())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::run_command_output;
    use std::ffi::OsStr;
    use std::path::PathBuf;

    fn hg_available() -> bool {
        std::process::Command::new("hg")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn hg(dir: &Path, args: &[&str]) {
        run_command_output("hg", Some(dir), args.iter().map(|s| OsStr::new(*s)))
            .expect("hg command");
    }

    #[test]
    fn reads_a_blob_at_a_revision() {
        if !hg_available() {
            return;
        }
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        hg(root, &["init", "--quiet"]);
        std::fs::write(root.join("file.txt"), "one\ntwo\n").expect("write");
        hg(root, &["add", "file.txt"]);
        // No username setup: `hg config KEY VALUE` is a read in Mercurial, not
        // a set (the git syntax), and `hg config` exits 1 on an unset key —
        // which a CI runner with no user hgrc hits. hg synthesizes a
        // `user@host` username for commits on its own, same as the existing
        // vcs::hg tests' `setup_test_repo`.
        hg(root, &["commit", "--quiet", "-m", "add file"]);
        let sha = run_command_output(
            "hg",
            Some(root),
            ["log", "-r", ".", "-T", "{node}"]
                .iter()
                .map(|s| OsStr::new(*s)),
        )
        .expect("log");

        // The worktree copy changing must not change what the blob read
        // returns — that difference is exactly what editor targets check.
        std::fs::write(root.join("file.txt"), "clobbered\n").expect("write");

        assert_eq!(
            read_blob_hg(root, sha.trim(), &PathBuf::from("file.txt")),
            Some("one\ntwo\n".to_string())
        );
        assert_eq!(
            read_blob_hg(root, sha.trim(), &PathBuf::from("missing.txt")),
            None
        );
        assert_eq!(
            read_blob_hg(root, "deadbeef", &PathBuf::from("file.txt")),
            None
        );
    }
}
