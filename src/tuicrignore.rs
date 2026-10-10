use std::collections::HashMap;
use std::path::{Path, PathBuf};

use ignore::Match;
use ignore::gitignore::{Gitignore, GitignoreBuilder};

use crate::model::{DiffFile, FilePatch};

/// Apply `.tuicrignore` rules from the repository root to a diff file set.
pub fn filter_diff_files(repo_root: &Path, diff_files: Vec<DiffFile>) -> Vec<DiffFile> {
    let mut matcher = Matcher::new(repo_root);
    diff_files
        .into_iter()
        .filter(|file| !matcher.is_ignored(file.display_path()))
        .collect()
}

/// Apply `.tuicrignore` rules from the repository root to raw file patches,
/// before hunks are parsed and highlighted, so an ignored file never pays
/// the syntax-highlighting cost (a large minified bundle otherwise stalls
/// PR open even though it is excluded from the review).
pub fn filter_file_patches(repo_root: &Path, patches: Vec<FilePatch>) -> Vec<FilePatch> {
    let mut matcher = Matcher::new(repo_root);
    patches
        .into_iter()
        .filter(|patch| {
            patch
                .display_path()
                .is_none_or(|path| !matcher.is_ignored(path))
        })
        .collect()
}

/// Apply `.tuicrignore` (and `.gitignore`, for `!`-unignore patterns) rules to a
/// list of paths. Used by the cheap status probe to verify that survives the
/// ignore filter without paying the full-diff cost.
pub fn filter_paths(repo_root: &Path, paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut matcher = Matcher::new(repo_root);
    paths
        .into_iter()
        .filter(|p| !matcher.is_ignored(p))
        .collect()
}

pub fn has_tuicrignore(repo_root: &Path) -> bool {
    repo_root.join(".tuicrignore").is_file()
}

/// `.tuicrignore` at the repository root, then every `.gitignore` from a
/// path's own directory up to the root. The first of those with a matching
/// rule decides, so a nested `.gitignore` can re-include what a parent one
/// ignores, as it does for git.
struct Matcher<'a> {
    repo_root: &'a Path,
    tuicrignore: Option<Gitignore>,
    /// `.gitignore` per repo-relative directory, loaded on first use.
    gitignores: HashMap<PathBuf, Option<Gitignore>>,
}

impl<'a> Matcher<'a> {
    fn new(repo_root: &'a Path) -> Self {
        Self {
            repo_root,
            tuicrignore: load(repo_root, &repo_root.join(".tuicrignore")),
            gitignores: HashMap::new(),
        }
    }

    fn is_ignored(&mut self, path: &Path) -> bool {
        if let Some(decided) = self.tuicrignore.as_ref().and_then(|m| decision(m, path)) {
            return decided;
        }
        for dir in path.ancestors().skip(1) {
            let repo_root = self.repo_root;
            let gitignore = self.gitignores.entry(dir.to_path_buf()).or_insert_with(|| {
                let dir = repo_root.join(dir);
                load(&dir, &dir.join(".gitignore"))
            });
            let Ok(relative) = path.strip_prefix(dir) else {
                continue;
            };
            if let Some(decided) = gitignore.as_ref().and_then(|m| decision(m, relative)) {
                return decided;
            }
        }
        false
    }
}

/// `Some(true)` for an ignore rule, `Some(false)` for a `!` re-include, `None`
/// when no rule in this file matches the path or one of its parents.
fn decision(matcher: &Gitignore, path: &Path) -> Option<bool> {
    match matcher.matched_path_or_any_parents(path, false) {
        Match::None => None,
        Match::Ignore(_) => Some(true),
        Match::Whitelist(_) => Some(false),
    }
}

fn load(root: &Path, file: &Path) -> Option<Gitignore> {
    if !file.is_file() {
        return None;
    }
    let mut builder = GitignoreBuilder::new(root);
    let _ = builder.add(file);
    builder.build().ok()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use tempfile::tempdir;

    use super::*;
    use crate::model::FileStatus;

    fn make_diff_file(path: &str) -> DiffFile {
        DiffFile {
            old_path: None,
            new_path: Some(PathBuf::from(path)),
            status: FileStatus::Modified,
            hunks: Vec::new(),
            is_binary: false,
            is_too_large: false,
            is_commit_message: false,
            content_hash: 0,
        }
    }

    #[test]
    fn keeps_all_files_when_tuicrignore_is_missing() {
        let dir = tempdir().expect("failed to create temp dir");
        let files = vec![
            make_diff_file("src/main.rs"),
            make_diff_file("target/debug/app"),
        ];

        let filtered = filter_diff_files(dir.path(), files);

        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn filters_matching_files() {
        let dir = tempdir().expect("failed to create temp dir");
        let ignore_path = dir.path().join(".tuicrignore");
        fs::write(&ignore_path, "target/\n*.lock\n").expect("failed to write .tuicrignore");

        let files = vec![
            make_diff_file("src/main.rs"),
            make_diff_file("target/debug/app"),
            make_diff_file("Cargo.lock"),
        ];

        let filtered = filter_diff_files(dir.path(), files);
        let kept_paths: Vec<String> = filtered
            .iter()
            .map(|f| f.display_path().display().to_string())
            .collect();

        assert_eq!(kept_paths, vec!["src/main.rs"]);
    }

    #[test]
    fn supports_unignore_rules() {
        let dir = tempdir().expect("failed to create temp dir");
        let ignore_path = dir.path().join(".tuicrignore");
        fs::write(&ignore_path, "generated/\n!generated/keep.rs\n")
            .expect("failed to write .tuicrignore");

        let files = vec![
            make_diff_file("generated/drop.rs"),
            make_diff_file("generated/keep.rs"),
            make_diff_file("src/main.rs"),
        ];

        let filtered = filter_diff_files(dir.path(), files);
        let kept_paths: Vec<String> = filtered
            .iter()
            .map(|f| f.display_path().display().to_string())
            .collect();

        assert_eq!(kept_paths, vec!["generated/keep.rs", "src/main.rs"]);
    }

    #[test]
    fn respects_gitignore() {
        let dir = tempdir().expect("failed to create temp dir");
        let gitignore = dir.path().join(".gitignore");
        fs::write(&gitignore, "target/\n*.log\n").expect("failed to write .gitignore");

        let files = vec![
            make_diff_file("src/main.rs"),
            make_diff_file("target/debug/app"),
            make_diff_file("build.log"),
        ];

        let filtered = filter_diff_files(dir.path(), files);
        let kept: Vec<String> = filtered
            .iter()
            .map(|f| f.display_path().display().to_string())
            .collect();

        assert_eq!(kept, vec!["src/main.rs"]);
    }

    #[test]
    fn tuicrignore_overrides_gitignore() {
        let dir = tempdir().expect("failed to create temp dir");
        let gitignore = dir.path().join(".gitignore");
        fs::write(&gitignore, "*.lock\n").expect("failed to write .gitignore");
        let tuicrignore = dir.path().join(".tuicrignore");
        fs::write(&tuicrignore, "!Cargo.lock\n").expect("failed to write .tuicrignore");

        let files = vec![
            make_diff_file("Cargo.lock"),
            make_diff_file("yarn.lock"),
            make_diff_file("src/lib.rs"),
        ];

        let filtered = filter_diff_files(dir.path(), files);
        let kept: Vec<String> = filtered
            .iter()
            .map(|f| f.display_path().display().to_string())
            .collect();

        // Cargo.lock is un-ignored by .tuicrignore, yarn.lock stays ignored
        assert_eq!(kept, vec!["Cargo.lock", "src/lib.rs"]);
    }

    #[test]
    fn gitignore_alone_filters_without_tuicrignore() {
        let dir = tempdir().expect("failed to create temp dir");
        let gitignore = dir.path().join(".gitignore");
        fs::write(&gitignore, "dist/\n").expect("failed to write .gitignore");

        let files = vec![
            make_diff_file("src/index.ts"),
            make_diff_file("dist/bundle.js"),
        ];

        let filtered = filter_diff_files(dir.path(), files);
        let kept: Vec<String> = filtered
            .iter()
            .map(|f| f.display_path().display().to_string())
            .collect();

        assert_eq!(kept, vec!["src/index.ts"]);
    }

    #[test]
    fn nested_gitignore_reincludes_what_the_root_one_ignores() {
        let dir = tempdir().expect("failed to create temp dir");
        fs::write(dir.path().join(".gitignore"), "**/packages/*\n")
            .expect("failed to write .gitignore");
        fs::create_dir(dir.path().join("ui")).expect("failed to create ui/");
        fs::write(dir.path().join("ui/.gitignore"), "!packages/*\n")
            .expect("failed to write ui/.gitignore");

        let files = vec![
            make_diff_file("ui/packages/utils/src/api.ts"),
            make_diff_file("server/packages/Newtonsoft.Json/lib.dll"),
        ];

        let filtered = filter_diff_files(dir.path(), files);
        let kept: Vec<String> = filtered
            .iter()
            .map(|f| f.display_path().display().to_string())
            .collect();

        assert_eq!(kept, vec!["ui/packages/utils/src/api.ts"]);
    }

    #[test]
    fn nested_gitignore_ignores_relative_to_its_directory() {
        let dir = tempdir().expect("failed to create temp dir");
        fs::create_dir(dir.path().join("web")).expect("failed to create web/");
        fs::write(dir.path().join("web/.gitignore"), "/dist\n")
            .expect("failed to write web/.gitignore");

        let files = vec![
            make_diff_file("web/dist/bundle.js"),
            make_diff_file("dist/release.txt"),
        ];

        let filtered = filter_diff_files(dir.path(), files);
        let kept: Vec<String> = filtered
            .iter()
            .map(|f| f.display_path().display().to_string())
            .collect();

        assert_eq!(kept, vec!["dist/release.txt"]);
    }

    #[test]
    fn tuicrignore_overrides_nested_gitignore() {
        let dir = tempdir().expect("failed to create temp dir");
        fs::create_dir(dir.path().join("web")).expect("failed to create web/");
        fs::write(dir.path().join("web/.gitignore"), "*.lock\n")
            .expect("failed to write web/.gitignore");
        fs::write(dir.path().join(".tuicrignore"), "!web/yarn.lock\n")
            .expect("failed to write .tuicrignore");

        let files = vec![make_diff_file("web/yarn.lock")];

        assert_eq!(filter_diff_files(dir.path(), files).len(), 1);
    }

    #[test]
    fn handles_deleted_file_paths() {
        let dir = tempdir().expect("failed to create temp dir");
        let ignore_path = dir.path().join(".tuicrignore");
        fs::write(&ignore_path, "generated/\n").expect("failed to write .tuicrignore");

        let deleted = DiffFile {
            old_path: Some(PathBuf::from("generated/old.txt")),
            new_path: None,
            status: FileStatus::Deleted,
            hunks: Vec::new(),
            is_binary: false,
            is_too_large: false,
            is_commit_message: false,
            content_hash: 0,
        };
        let kept = make_diff_file("src/lib.rs");

        let filtered = filter_diff_files(dir.path(), vec![deleted, kept]);
        let kept_paths: Vec<String> = filtered
            .iter()
            .map(|f| f.display_path().display().to_string())
            .collect();

        assert_eq!(kept_paths, vec!["src/lib.rs"]);
    }
}
