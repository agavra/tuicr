use std::path::Path;

use git2::{AttrCheckFlags, AttrValue, Repository};

const LINGUIST_GENERATED: &str = "linguist-generated";

/// Resolves `linguist-generated` with Git's own attribute matcher.
///
/// Attribute lookup is best-effort: a PR opened outside a usable local Git
/// checkout keeps its full diff rather than failing to open.
pub(crate) struct LinguistGeneratedMatcher {
    repo: Repository,
}

impl LinguistGeneratedMatcher {
    pub(crate) fn open(repo_root: &Path) -> Option<Self> {
        Repository::open(repo_root).ok().map(|repo| Self { repo })
    }

    pub(crate) fn is_generated(&self, path: &Path) -> bool {
        let Ok(value) = self
            .repo
            .get_attr(path, LINGUIST_GENERATED, AttrCheckFlags::default())
        else {
            return false;
        };

        matches!(
            AttrValue::from_string(value),
            AttrValue::True | AttrValue::String("true")
        )
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn honors_true_bare_false_and_unspecified_attribute_values() {
        let checkout = tempfile::tempdir().expect("failed to create temp dir");
        Repository::init(checkout.path()).expect("failed to initialize repository");
        fs::write(
            checkout.path().join(".gitattributes"),
            concat!(
                "generated.js linguist-generated=true\n",
                "bare.js linguist-generated\n",
                "source.js -linguist-generated\n",
            ),
        )
        .expect("failed to write .gitattributes");
        let matcher = LinguistGeneratedMatcher::open(checkout.path()).unwrap();

        assert!(matcher.is_generated(Path::new("generated.js")));
        assert!(matcher.is_generated(Path::new("bare.js")));
        assert!(!matcher.is_generated(Path::new("source.js")));
        assert!(!matcher.is_generated(Path::new("other.js")));
    }

    #[test]
    fn returns_no_matcher_outside_a_git_checkout() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");

        assert!(LinguistGeneratedMatcher::open(dir.path()).is_none());
    }
}
