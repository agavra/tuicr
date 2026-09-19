//! Read-only snapshots of a reviewed revision for the external editor.
//!
//! In PR review the diff is the PR's revision, but `$EDITOR` can only open a
//! file on disk. When the local checkout does not hold that revision — wrong
//! branch, file added by the PR, file deleted by the PR — the reviewed content
//! is written to a snapshot under the temp dir and the editor is pointed there,
//! so the text and the line the cursor sits on agree with the diff.
//!
//! Snapshots are keyed by head SHA, so their contents never change; they are
//! left in the temp dir for the OS to reclaim rather than deleted on exit,
//! because a windowed editor may still have one open long after tuicr quits.

use std::io;
use std::path::{Component, Path, PathBuf};

use crate::forge::traits::PrSessionKey;

/// Length of the abbreviated SHA used in snapshot directory names and status
/// messages. Matches the short SHAs the PR panel already displays.
pub(in crate::app) const SHORT_SHA_LEN: usize = 7;

/// Abbreviate a revision for display.
pub(in crate::app) fn short_sha(sha: &str) -> &str {
    let len = sha.len().min(SHORT_SHA_LEN);
    &sha[..len]
}

/// Directory holding one revision's snapshots.
///
/// Keyed by repository, PR number and head SHA so two PRs — or two pushes to
/// the same PR — never collide.
pub(in crate::app) fn snapshot_root(key: &PrSessionKey) -> PathBuf {
    let slug = sanitize_component(&key.repository.slug());
    std::env::temp_dir()
        .join("tuicr")
        .join("snapshots")
        .join(format!(
            "{slug}-pr{}-{}",
            key.number,
            short_sha(&key.head_sha)
        ))
}

/// Fold a repository slug into one path-safe directory-name component.
///
/// Slugs carry slashes (`owner/name`, and `org/project/name` on Azure) plus
/// whatever characters the forge allows in a project name.
fn sanitize_component(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_') {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// Resolve `rel` inside `root`, or `None` when it would escape.
///
/// Diff paths come from a forge response, so a malicious or malformed one could
/// carry `..` or an absolute path; that must not turn a snapshot write into an
/// arbitrary-file write. Same intent as the confinement guard in
/// `FileBackend::fetch_context_lines`, done before the file exists so
/// `canonicalize` is not an option.
pub(in crate::app) fn snapshot_path(root: &Path, rel: &Path) -> Option<PathBuf> {
    let mut path = root.to_path_buf();
    let mut pushed = false;
    for component in rel.components() {
        match component {
            Component::Normal(part) => {
                path.push(part);
                pushed = true;
            }
            // Prefix/RootDir mean an absolute path; ParentDir escapes; CurDir
            // is noise but harmless to drop.
            Component::Prefix(_) | Component::RootDir | Component::ParentDir => return None,
            Component::CurDir => {}
        }
    }
    pushed.then_some(path)
}

/// Write `content` to `path` and mark it read-only.
///
/// An existing snapshot is left alone: contents are fixed by the SHA in the
/// directory name, and Windows refuses to open a read-only file for writing.
pub(in crate::app) fn materialize(path: &Path, content: &str) -> io::Result<()> {
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)?;
    // Best-effort: a snapshot the user can edit invites losing that work in the
    // temp dir, but failing to set the bit is no reason not to open the file.
    if let Ok(metadata) = std::fs::metadata(path) {
        let mut permissions = metadata.permissions();
        permissions.set_readonly(true);
        let _ = std::fs::set_permissions(path, permissions);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::traits::ForgeRepository;

    fn key() -> PrSessionKey {
        PrSessionKey::new(
            ForgeRepository::github("github.com", "agavra", "tuicr"),
            624,
            "1a2b3c4d5e6f7a8b",
        )
    }

    #[test]
    fn snapshot_root_names_the_repo_pr_and_revision() {
        let root = snapshot_root(&key());
        let name = root.file_name().expect("dir name").to_string_lossy();
        assert_eq!(name, "agavra-tuicr-pr624-1a2b3c4");
        assert!(root.starts_with(std::env::temp_dir().join("tuicr").join("snapshots")));
    }

    #[test]
    fn snapshot_path_joins_a_relative_path() {
        let root = PathBuf::from("/snap");
        assert_eq!(
            snapshot_path(&root, Path::new("src/app/mod.rs")),
            Some(root.join("src").join("app").join("mod.rs"))
        );
    }

    #[test]
    fn snapshot_path_rejects_paths_that_escape_the_root() {
        let root = PathBuf::from("/snap");
        for rel in ["../etc/passwd", "src/../../etc/passwd", "", "."] {
            assert_eq!(
                snapshot_path(&root, Path::new(rel)),
                None,
                "{rel} must not resolve"
            );
        }
        #[cfg(windows)]
        assert_eq!(snapshot_path(&root, Path::new(r"C:\Windows\win.ini")), None);
        #[cfg(unix)]
        assert_eq!(snapshot_path(&root, Path::new("/etc/passwd")), None);
    }

    #[test]
    fn materialize_writes_a_read_only_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("nested").join("main.tf");

        materialize(&path, "resource {}\n").expect("write snapshot");

        assert_eq!(
            std::fs::read_to_string(&path).expect("read back"),
            "resource {}\n"
        );
        assert!(
            std::fs::metadata(&path)
                .expect("metadata")
                .permissions()
                .readonly()
        );
    }

    #[test]
    fn materialize_keeps_an_existing_snapshot() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("main.tf");
        materialize(&path, "first\n").expect("write snapshot");

        // Same SHA, same content: rewriting a read-only file would fail on
        // Windows, so the second call must be a no-op.
        materialize(&path, "second\n").expect("reuse snapshot");

        assert_eq!(
            std::fs::read_to_string(&path).expect("read back"),
            "first\n"
        );
    }
}
