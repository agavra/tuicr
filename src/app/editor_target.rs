//! Read-only snapshots of a reviewed revision for the external editor.
//!
//! In PR review the diff is the PR's revision, but `$EDITOR` can only open a
//! file on disk. When the local checkout does not hold that revision — wrong
//! branch, file added by the PR, file deleted by the PR — the reviewed content
//! is written to a snapshot under the per-user cache dir and the editor is pointed there,
//! so the text and the line the cursor sits on agree with the diff.
//!
//! Snapshots are keyed by the full revision SHA, so their contents never change;
//! they are left in the cache dir for the OS to reclaim rather than deleted on exit,
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

/// Per-user directory holding every snapshot, created `0700` on Unix.
///
/// It lives in the user's cache dir rather than the shared temp dir, so another
/// local user cannot pre-create it, plant content in it, or swap in symlinks.
fn snapshot_base() -> io::Result<PathBuf> {
    #[cfg(test)]
    if let Some(base) = TEST_BASE.with(|b| b.borrow().clone()) {
        private_dir(&base)?;
        return Ok(base);
    }
    let dirs = directories::ProjectDirs::from("", "", "tuicr")
        .ok_or_else(|| io::Error::other("could not determine the cache directory"))?;
    let base = dirs.cache_dir().join("snapshots");
    private_dir(&base)?;
    Ok(base)
}

#[cfg(test)]
thread_local! {
    /// Redirects snapshots away from the real cache dir in tests.
    pub(in crate::app) static TEST_BASE: std::cell::RefCell<Option<PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

/// Create `dir` if needed and insist it is a real directory only we can use.
fn private_dir(dir: &Path) -> io::Result<()> {
    if let Some(parent) = dir.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match std::fs::symlink_metadata(dir) {
        Ok(_) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            let mut builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
            builder.create(dir)?;
        }
        Err(err) => return Err(err),
    }
    let meta = std::fs::symlink_metadata(dir)?;
    if !meta.is_dir() {
        return Err(io::Error::other(format!(
            "{} is not a directory",
            dir.display()
        )));
    }
    #[cfg(unix)]
    {
        // SAFETY: geteuid has no preconditions and cannot fail.
        check_owner_only(dir, &meta, unsafe { libc::geteuid() })?;
    }
    Ok(())
}

#[cfg(unix)]
fn check_owner_only(dir: &Path, meta: &std::fs::Metadata, euid: u32) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    if meta.uid() != euid || meta.mode() & 0o077 != 0 {
        return Err(io::Error::other(format!(
            "{} is not private to the current user",
            dir.display()
        )));
    }
    Ok(())
}

/// Directory holding one revision's snapshots.
///
/// Keyed by repository (host included), PR number and the full SHA of the
/// revision being shown, so two PRs, two pushes, or a retargeted base never
/// collide, even when their abbreviated SHAs agree.
pub(in crate::app) fn snapshot_root(key: &PrSessionKey, sha: &str) -> io::Result<PathBuf> {
    Ok(snapshot_root_in(&snapshot_base()?, key, sha))
}

fn snapshot_root_in(base: &Path, key: &PrSessionKey, sha: &str) -> PathBuf {
    use sha2::{Digest, Sha256};
    let repository = &key.repository;
    let identity = format!("{}/{}", repository.host, repository.slug());
    // Hosts, namespaces and SHAs are unbounded, but one path component is capped
    // at NAME_MAX (255 bytes): keep a truncated readable part and let a hash of
    // the whole identity keep the name unique.
    let digest = Sha256::digest(format!("{identity}\0{}\0{sha}", key.number));
    let hash: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    let readable: String = sanitize_component(&identity).chars().take(60).collect();
    let revision: String = sanitize_component(sha).chars().take(64).collect();
    base.join(format!("{readable}-{hash}-pr{}-{revision}", key.number))
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
/// Nothing under `root` is followed through a symlink: directories are created
/// one component at a time, and the file is written to an exclusively created
/// temp file that is linked into place only if `path` is still free, so a
/// reader never sees a partial snapshot.
pub(in crate::app) fn materialize(root: &Path, path: &Path, content: &str) -> io::Result<()> {
    use std::io::Write;

    let rel_parent = path
        .parent()
        .and_then(|parent| parent.strip_prefix(root).ok())
        .ok_or_else(|| io::Error::other("snapshot path is outside its root"))?;
    let mut dir = root.to_path_buf();
    for component in std::iter::once(None).chain(rel_parent.components().map(Some)) {
        if let Some(part) = component {
            dir.push(part);
        }
        match std::fs::symlink_metadata(&dir) {
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => {
                return Err(io::Error::other(format!(
                    "{} is not a directory",
                    dir.display()
                )));
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => std::fs::create_dir(&dir)?,
            Err(err) => return Err(err),
        }
    }
    if std::fs::symlink_metadata(path).is_ok_and(|meta| meta.is_file()) {
        return Ok(());
    }
    let mut temp = tempfile::NamedTempFile::new_in(&dir)?;
    temp.write_all(content.as_bytes())?;
    // Best-effort: a snapshot the user can edit invites losing that work, but
    // failing to set the bit is no reason not to open the file.
    let mut permissions = temp.as_file().metadata()?.permissions();
    permissions.set_readonly(true);
    let _ = temp.as_file().set_permissions(permissions);
    match temp.persist_noclobber(path) {
        Ok(_) => Ok(()),
        Err(err) if err.error.kind() == io::ErrorKind::AlreadyExists => {
            // A symlink (even a dangling one) in the way is not a snapshot.
            if std::fs::symlink_metadata(path).is_ok_and(|meta| meta.is_file()) {
                Ok(())
            } else {
                Err(err.error)
            }
        }
        Err(err) => Err(err.error),
    }
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
    fn snapshot_root_names_the_host_repo_pr_and_full_revision() {
        let base = PathBuf::from("/base");
        let root = snapshot_root_in(&base, &key(), "1a2b3c4d5e6f7a8b");
        let name = root.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with("github.com-agavra-tuicr-"), "{name}");
        assert!(name.ends_with("-pr624-1a2b3c4d5e6f7a8b"), "{name}");
    }

    #[test]
    fn snapshot_root_stays_under_name_max_for_a_long_namespace() {
        let base = PathBuf::from("/base");
        let sha = "a".repeat(64);
        let repo = |ns: &str| ForgeRepository::github("gitlab.example.com", ns, "project");
        let long = "n".repeat(300);
        let a = snapshot_root_in(&base, &PrSessionKey::new(repo(&long), 1, &sha), &sha);
        let other = format!("{}x", "n".repeat(299));
        let b = snapshot_root_in(&base, &PrSessionKey::new(repo(&other), 1, &sha), &sha);
        for root in [&a, &b] {
            assert_eq!(root.parent().unwrap(), base);
            let len = root.file_name().unwrap().len();
            assert!(len <= 255, "component is {len} bytes");
        }
        assert_ne!(a, b, "truncated names must still be unique");
    }

    #[cfg(unix)]
    #[test]
    fn check_owner_only_rejects_a_directory_owned_by_someone_else() {
        use std::os::unix::fs::MetadataExt;
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let meta = std::fs::symlink_metadata(dir.path()).unwrap();
        assert!(check_owner_only(dir.path(), &meta, meta.uid()).is_ok());
        assert!(check_owner_only(dir.path(), &meta, meta.uid().wrapping_add(1)).is_err());
    }

    #[test]
    fn snapshot_root_separates_revisions_sharing_a_short_prefix() {
        let base = PathBuf::from("/base");
        let a = snapshot_root_in(&base, &key(), "e97cf6ffeb99cab4");
        let b = snapshot_root_in(&base, &key(), "e97cf6f3e9ed7f5a");
        assert_ne!(a, b);
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

        materialize(dir.path(), &path, "resource {}\n").expect("write snapshot");

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
        let siblings = std::fs::read_dir(path.parent().unwrap()).unwrap().count();
        assert_eq!(siblings, 1, "no temp file may be left behind");
    }

    #[test]
    fn materialize_keeps_an_existing_snapshot() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("main.tf");
        materialize(dir.path(), &path, "first\n").expect("write snapshot");

        // Same SHA, same content: rewriting a read-only file would fail on
        // Windows, so the second call must be a no-op.
        materialize(dir.path(), &path, "second\n").expect("reuse snapshot");

        assert_eq!(
            std::fs::read_to_string(&path).expect("read back"),
            "first\n"
        );
    }

    #[test]
    fn materialize_rejects_a_path_outside_its_root() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("root");
        std::fs::create_dir(&root).unwrap();
        assert!(materialize(&root, &dir.path().join("x.rs"), "x").is_err());
        assert!(!dir.path().join("x.rs").exists());
    }

    #[cfg(unix)]
    #[test]
    fn materialize_does_not_follow_a_symlinked_parent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("root");
        let outside = dir.path().join("outside");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("sub")).unwrap();

        let result = materialize(&root, &root.join("sub").join("a.rs"), "planted\n");

        assert!(result.is_err());
        assert!(!outside.join("a.rs").exists());
    }

    #[cfg(unix)]
    #[test]
    fn materialize_does_not_follow_a_dangling_symlink() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("outside.rs");
        let path = dir.path().join("a.rs");
        std::os::unix::fs::symlink(&target, &path).unwrap();

        let result = materialize(dir.path(), &path, "planted\n");

        assert!(result.is_err());
        assert!(!target.exists());
    }

    #[cfg(unix)]
    #[test]
    fn materialize_does_not_trust_a_symlink_to_a_real_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("fake.rs");
        let path = dir.path().join("a.rs");
        std::fs::write(&target, "fake\n").unwrap();
        std::os::unix::fs::symlink(&target, &path).unwrap();

        assert!(materialize(dir.path(), &path, "real\n").is_err());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "fake\n");
    }

    #[test]
    fn private_dir_creates_a_missing_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("a").join("snapshots");
        private_dir(&path).expect("create");
        assert!(path.is_dir());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700);
        }
    }

    #[test]
    fn private_dir_rejects_a_plain_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("snapshots");
        std::fs::write(&path, "").unwrap();
        #[cfg(unix)]
        std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o700))
            .unwrap();
        assert!(private_dir(&path).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn private_dir_rejects_a_symlink() {
        let dir = tempfile::tempdir().expect("tempdir");
        let real = dir.path().join("real");
        let link = dir.path().join("snapshots");
        private_dir(&real).unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(private_dir(&link).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn private_dir_rejects_a_group_or_world_accessible_directory() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("snapshots");
        private_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(private_dir(&path).is_err());
    }
}
