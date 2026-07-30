use std::collections::HashSet;
use std::path::{Path, PathBuf};

use git2::{AttrCheckFlags, AttrValue, Repository};

/// Attributes that mark a file as code-generated. GitHub's Linguist and
/// GitLab each define their own name; a file marked by either counts.
const GENERATED_ATTRS: [&str; 2] = ["linguist-generated", "gitlab-generated"];

/// Which of `paths` the repository's gitattributes mark as code-generated.
///
/// Delegating to libgit2 rather than parsing `.gitattributes` ourselves means
/// nested `.gitattributes`, `.git/info/attributes`, `core.attributesFile`, and
/// `[attr]` macros all resolve the way git resolves them, for free.
///
/// Returns an empty set — never an error — when there is no git repository to
/// ask: mercurial, non-colocated jujutsu, and pull requests without a local
/// checkout all land here, and none of them should block a review.
pub fn detect_generated(repo_root: &Path, paths: &[PathBuf]) -> HashSet<PathBuf> {
    let mut generated = HashSet::new();
    if paths.is_empty() {
        return generated;
    }
    let Ok(repo) = Repository::discover(repo_root) else {
        return generated;
    };
    for path in paths {
        if is_generated(&repo, path) {
            generated.insert(path.clone());
        }
    }
    generated
}

/// Whether either generated attribute is affirmatively set for `path`.
///
/// An explicit opt-out on *either* attribute beats an opt-in on the other, and
/// a value we don't recognize counts as "not generated". Both rules fail toward
/// showing the diff: a needlessly shown diff is an annoyance, a needlessly
/// hidden one is unreviewed code.
fn is_generated(repo: &Repository, path: &Path) -> bool {
    let mut opted_in = false;
    for name in GENERATED_ATTRS {
        let Ok(raw) = repo.get_attr(path, name, AttrCheckFlags::FILE_THEN_INDEX) else {
            continue;
        };
        // `get_attr` returns a sentinel string for a set-but-valueless
        // attribute, so the raw value has to be interpreted through
        // `AttrValue` rather than compared directly. Getting this wrong would
        // read GitLab's bare `gitlab-generated` and its `-gitlab-generated`
        // opt-out as the same thing.
        match AttrValue::from_string(raw) {
            // `attr` (GitLab's documented form) or `attr=true` (GitHub's).
            AttrValue::True | AttrValue::String("true") => opted_in = true,
            // `-attr` or `attr=false`.
            AttrValue::False | AttrValue::String("false") => return false,
            // `!attr`, absent, or a value neither forge documents.
            _ => {}
        }
    }
    opted_in
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::{TempDir, tempdir};

    use super::*;

    /// A git repository with the given `.gitattributes` content at its root.
    fn repo_with_attributes(contents: &str) -> TempDir {
        let dir = tempdir().expect("failed to create temp dir");
        Repository::init(dir.path()).expect("failed to init repo");
        fs::write(dir.path().join(".gitattributes"), contents)
            .expect("failed to write .gitattributes");
        dir
    }

    fn detect(dir: &TempDir, paths: &[&str]) -> HashSet<PathBuf> {
        let paths: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
        detect_generated(dir.path(), &paths)
    }

    #[test]
    fn detects_github_and_gitlab_attribute_forms() {
        // `attr=true` yields `AttrValue::String("true")` while bare `attr`
        // yields `AttrValue::True`. Both mean generated, and a naive
        // `is_some()` check on the raw value would conflate them with the
        // opt-out forms below.
        let dir = repo_with_attributes(
            "github.pb.go linguist-generated=true\ngitlab.pb.go gitlab-generated\n",
        );

        let generated = detect(&dir, &["github.pb.go", "gitlab.pb.go", "src/main.rs"]);

        assert_eq!(
            generated,
            HashSet::from([PathBuf::from("github.pb.go"), PathBuf::from("gitlab.pb.go"),])
        );
    }

    #[test]
    fn treats_explicit_opt_out_forms_as_not_generated() {
        let dir = repo_with_attributes(
            "dash.pb.go -linguist-generated\nvalue.pb.go linguist-generated=false\nbang.pb.go !linguist-generated\n",
        );

        let generated = detect(&dir, &["dash.pb.go", "value.pb.go", "bang.pb.go"]);

        assert!(generated.is_empty(), "unexpected: {generated:?}");
    }

    #[test]
    fn ignores_values_neither_forge_documents() {
        let dir = repo_with_attributes("weird.pb.go linguist-generated=maybe\n");

        assert!(detect(&dir, &["weird.pb.go"]).is_empty());
    }

    #[test]
    fn opt_out_on_either_attribute_beats_opt_in_on_the_other() {
        let dir = repo_with_attributes(
            "a.pb.go linguist-generated=true -gitlab-generated\nb.pb.go -linguist-generated gitlab-generated\n",
        );

        let generated = detect(&dir, &["a.pb.go", "b.pb.go"]);

        assert!(generated.is_empty(), "unexpected: {generated:?}");
    }

    #[test]
    fn resolves_nested_gitattributes() {
        // Proves libgit2 is doing the attribute resolution rather than us
        // reading only the root file.
        let dir = repo_with_attributes("");
        let nested = dir.path().join("proto");
        fs::create_dir(&nested).expect("failed to create nested dir");
        fs::write(
            nested.join(".gitattributes"),
            "*.pb.go linguist-generated=true\n",
        )
        .expect("failed to write nested .gitattributes");

        let generated = detect(&dir, &["proto/api.pb.go", "root.pb.go"]);

        assert_eq!(generated, HashSet::from([PathBuf::from("proto/api.pb.go")]));
    }

    #[test]
    fn returns_empty_set_outside_a_git_repository() {
        let dir = tempdir().expect("failed to create temp dir");
        fs::write(
            dir.path().join(".gitattributes"),
            "a.pb.go linguist-generated\n",
        )
        .expect("failed to write .gitattributes");
        let paths = vec![PathBuf::from("a.pb.go")];

        assert!(detect_generated(dir.path(), &paths).is_empty());
    }

    #[test]
    fn returns_empty_set_without_opening_a_repository_for_no_paths() {
        let dir = repo_with_attributes("*.pb.go linguist-generated=true\n");

        assert!(detect_generated(dir.path(), &[]).is_empty());
    }
}
