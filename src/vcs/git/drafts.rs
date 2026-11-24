use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::{Result, TuicrError};
use crate::model::DiffFile;
use crate::syntax::SyntaxHighlighter;

use super::{cli::build_untracked_diff_file, raw::path_buf_from_bytes};

/// Resolve CLI paths from the invocation directory and keep checkout-relative
/// paths for diff rendering, session anchors and subsequent reloads.
pub(crate) fn resolve_paths(root: &Path, paths: &[String]) -> Result<Vec<PathBuf>> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }

    let root = root.canonicalize()?;
    let mut seen = HashSet::new();
    let mut relative_paths = Vec::with_capacity(paths.len());
    for path in paths {
        let canonical = confined_file(&root, Path::new(path))?;
        let relative = canonical
            .strip_prefix(&root)
            .expect("checked by confined_file");
        if seen.insert(relative.to_path_buf()) {
            relative_paths.push(relative.to_path_buf());
        }
    }
    Ok(relative_paths)
}

fn confined_file(root: &Path, path: &Path) -> Result<PathBuf> {
    let canonical = path
        .canonicalize()
        .map_err(|err| invalid_draft(path, &format!("cannot open file ({err})")))?;
    if !canonical.starts_with(root) {
        return Err(invalid_draft(path, "path is outside the Git checkout"));
    }
    if !path.symlink_metadata()?.is_file() {
        return Err(invalid_draft(path, "expected a regular file"));
    }
    Ok(canonical)
}

pub(crate) fn load_diff(
    root: &Path,
    paths: &[PathBuf],
    highlighter: &SyntaxHighlighter,
) -> Result<Vec<DiffFile>> {
    let root = root.canonicalize()?;
    // Omitting exclude rules includes explicitly named Git-ignored drafts.
    // Literal pathspecs keep names containing Git pathspec syntax exact.
    let output = Command::new("git")
        .current_dir(&root)
        .args(["--literal-pathspecs", "ls-files", "--others", "-z", "--"])
        .args(paths)
        .output()?;
    if !output.status.success() {
        return Err(TuicrError::VcsCommand(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    let untracked: HashSet<_> = output
        .stdout
        .split(|&byte| byte == 0)
        .filter(|entry| !entry.is_empty())
        .map(path_buf_from_bytes)
        .collect();

    let mut files = Vec::with_capacity(paths.len());
    for path in paths {
        let absolute = root.join(path);
        // Check again on reload in case a parent directory was replaced by a
        // symlink after startup.
        if confined_file(&root, &absolute)? != absolute {
            return Err(invalid_draft(path, "file location changed"));
        }
        if !untracked.contains(path) {
            return Err(invalid_draft(
                path,
                "expected an untracked file in the Git checkout",
            ));
        }
        let file = build_untracked_diff_file(path, &absolute)
            .ok_or_else(|| invalid_draft(path, "cannot read file"))?;
        files.push(file);
    }

    crate::vcs::enhance_with_full_file_highlight(
        &mut files,
        highlighter,
        |_| None,
        |path| crate::vcs::read_workdir_file(&root, path),
    );
    Ok(files)
}

fn invalid_draft(path: &Path, reason: &str) -> TuicrError {
    TuicrError::InvalidInput(format!("Draft file '{}': {reason}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_explicit_ignored_drafts_and_rejects_the_same_file_once_tracked() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let git = |args: &[&str]| {
            let output = Command::new("git")
                .current_dir(root)
                .args(args)
                .output()
                .unwrap();
            assert!(output.status.success(), "{:?}", output);
        };
        git(&["init", "-q"]);
        std::fs::write(root.join(".gitignore"), "*.md\n").unwrap();
        let path = root.join("notes[1].md");
        std::fs::write(&path, "draft text\n").unwrap();

        let input = path.to_str().unwrap().to_owned();
        let paths = resolve_paths(root, &[input.clone(), input]).unwrap();
        assert_eq!(paths, [PathBuf::from("notes[1].md")]);

        let highlighter = SyntaxHighlighter::default();
        let files = load_diff(root, &paths, &highlighter).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].display_path(), &paths[0]);
        let line = &files[0].hunks[0].lines[0];
        assert_eq!(line.content, "draft text");
        assert_eq!(line.new_lineno, Some(1));

        git(&["add", "--force", "--", "notes[1].md"]);
        let err = load_diff(root, &paths, &highlighter).unwrap_err();
        assert!(err.to_string().contains("expected an untracked file"));
    }

    #[test]
    fn rejects_paths_outside_the_checkout_and_directories() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("checkout");
        std::fs::create_dir(&root).unwrap();
        let outside = dir.path().join("outside.md");
        std::fs::write(&outside, "outside\n").unwrap();

        let err = resolve_paths(&root, &[outside.to_str().unwrap().to_owned()]).unwrap_err();
        assert!(err.to_string().contains("outside the Git checkout"));
        let err = resolve_paths(&root, &[root.to_str().unwrap().to_owned()]).unwrap_err();
        assert!(err.to_string().contains("expected a regular file"));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_drafts() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("checkout");
        std::fs::create_dir(&root).unwrap();
        let outside = dir.path().join("outside.md");
        std::fs::write(&outside, "outside\n").unwrap();
        let inside = root.join("notes.md");
        std::fs::write(&inside, "notes\n").unwrap();

        let escape = root.join("escape.md");
        symlink(&outside, &escape).unwrap();
        let err = resolve_paths(&root, &[escape.to_str().unwrap().to_owned()]).unwrap_err();
        assert!(err.to_string().contains("outside the Git checkout"));

        let alias = root.join("alias.md");
        symlink(&inside, &alias).unwrap();
        let err = resolve_paths(&root, &[alias.to_str().unwrap().to_owned()]).unwrap_err();
        assert!(err.to_string().contains("expected a regular file"));
    }
}
