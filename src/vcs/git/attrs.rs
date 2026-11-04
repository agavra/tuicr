//! Check whether any Git attribute source can set `diff` on a changed file.

use git2::Repository;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Holds whether any attribute source can set `diff` on a changed file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DiffAttrs {
    NoRules,
    LookUpEachFile,
}

/// libgit2 reads these file names inside its system and global search paths.
const SYSTEM_ATTRIBUTES_FILE: &str = "gitattributes";
const GLOBAL_ATTRIBUTES_FILE: &str = "attributes";

/// Report whether attribute text can set `diff`.
///
/// Match plain text, because a false match costs only time. Match `diff` for the
/// attribute, and `binary` and `[attr]` for macros that set it elsewhere.
fn may_set_diff(bytes: &[u8]) -> bool {
    [&b"diff"[..], b"binary", b"[attr]"]
        .iter()
        .any(|word| bytes.windows(word.len()).any(|w| w == *word))
}

/// Return true when the file exists and may set `diff`, or cannot be read.
fn file_may_set_diff(path: &Path) -> bool {
    match std::fs::read(path) {
        Ok(bytes) => may_set_diff(&bytes),
        Err(err) => !matches!(
            err.kind(),
            std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
        ),
    }
}

/// Read every attribute source libgit2 reads for `changed`, once. Return `LookUpEachFile`
/// if tuicr cannot read a source.
pub(super) fn diff_attrs_for(repo: &Repository, changed: &[&Path]) -> DiffAttrs {
    let Some(workdir) = repo.workdir() else {
        return DiffAttrs::LookUpEachFile;
    };
    let (Some(outside), Ok(index)) = (attributes_files_outside_tree(repo), repo.index()) else {
        return DiffAttrs::LookUpEachFile;
    };
    if outside.iter().any(|path| file_may_set_diff(path)) {
        return DiffAttrs::LookUpEachFile;
    }

    let dirs: HashSet<&Path> = changed
        .iter()
        .flat_map(|path| path.ancestors().skip(1))
        .collect();
    let any_rule = dirs.into_iter().any(|dir| {
        let name = dir.join(".gitattributes");
        file_may_set_diff(&workdir.join(&name))
            || index.get_path(&name, 0).is_some_and(|entry| {
                repo.find_blob(entry.id)
                    .map_or(true, |blob| may_set_diff(blob.content()))
            })
    });
    if any_rule {
        DiffAttrs::LookUpEachFile
    } else {
        DiffAttrs::NoRules
    }
}

/// List the attribute files outside the working tree. Return `None` if libgit2 returns an
/// error for its search paths.
fn attributes_files_outside_tree(repo: &Repository) -> Option<Vec<PathBuf>> {
    use git2::ConfigLevel;

    let search_dirs = |level: ConfigLevel| -> Option<Vec<PathBuf>> {
        // SAFETY: this reads a process-wide libgit2 setting, and tuicr never writes that
        // setting.
        let joined = unsafe { git2::opts::get_search_path(level) }.ok()?;
        Some(std::env::split_paths(joined.to_str().ok()?).collect())
    };
    let mut files: Vec<PathBuf> = search_dirs(ConfigLevel::System)?
        .into_iter()
        .map(|dir| dir.join(SYSTEM_ATTRIBUTES_FILE))
        .collect();
    files.extend(
        search_dirs(ConfigLevel::XDG)?
            .into_iter()
            .map(|dir| dir.join(GLOBAL_ATTRIBUTES_FILE)),
    );
    files.push(repo.commondir().join("info/attributes"));
    if let Ok(path) = repo.config().ok()?.get_path("core.attributesFile") {
        files.push(path);
    }
    Some(files)
}

#[cfg(test)]
mod tests {
    use super::super::diff::tests::attribute_repo;
    use super::*;
    use std::fs;

    fn attrs_with_rules(attributes: &str) -> (tempfile::TempDir, Repository) {
        attribute_repo(&[("a.txt", b"one\n", b"two\n")], attributes)
    }

    fn attrs_for_a_txt(repo: &Repository) -> DiffAttrs {
        diff_attrs_for(repo, &[Path::new("a.txt")])
    }

    #[test]
    fn should_report_no_rules_when_sources_never_mention_diff() {
        let (_dir, repo) = attrs_with_rules("*.svelte linguist-detectable\n");

        assert_eq!(attrs_for_a_txt(&repo), DiffAttrs::NoRules);
    }

    #[test]
    fn should_look_up_each_file_for_rules_that_can_change_a_diff() {
        for rule in [
            "*.txt -diff\n",
            "*.txt binary\n",
            "[attr]raw -text\n*.txt raw\n",
        ] {
            let (_dir, repo) = attrs_with_rules(rule);

            assert_eq!(attrs_for_a_txt(&repo), DiffAttrs::LookUpEachFile, "{rule}");
        }
    }

    #[test]
    fn should_honor_a_rule_in_the_index_when_the_working_copy_is_deleted() {
        let (dir, repo) = attrs_with_rules("*.txt -diff\n");
        fs::remove_file(dir.path().join(".gitattributes")).unwrap();

        assert_eq!(attrs_for_a_txt(&repo), DiffAttrs::LookUpEachFile);
    }

    #[test]
    fn should_honor_a_nested_rule_above_a_changed_file() {
        let (dir, repo) = attrs_with_rules("");
        fs::create_dir_all(dir.path().join("x/y")).unwrap();
        fs::write(dir.path().join("x/.gitattributes"), "*.txt -diff\n").unwrap();

        assert_eq!(
            diff_attrs_for(&repo, &[Path::new("x/y/a.txt")]),
            DiffAttrs::LookUpEachFile
        );
        assert_eq!(attrs_for_a_txt(&repo), DiffAttrs::NoRules);
    }

    #[test]
    fn should_honor_a_rule_in_info_attributes() {
        let (_dir, repo) = attrs_with_rules("");
        fs::create_dir_all(repo.commondir().join("info")).unwrap();
        fs::write(repo.commondir().join("info/attributes"), "*.txt -diff\n").unwrap();

        assert_eq!(attrs_for_a_txt(&repo), DiffAttrs::LookUpEachFile);
    }

    #[test]
    fn should_honor_a_rule_in_the_configured_attributes_file() {
        let (dir, repo) = attrs_with_rules("");
        let file = dir.path().join("outside-attributes");
        fs::write(&file, "*.txt -diff\n").unwrap();
        repo.config()
            .unwrap()
            .set_str("core.attributesFile", file.to_str().unwrap())
            .unwrap();

        assert_eq!(attrs_for_a_txt(&repo), DiffAttrs::LookUpEachFile);
    }

    /// Assert the file names libgit2 reads, because a wrong name changes how a file
    /// displays.
    #[test]
    fn should_name_the_system_file_libgit2_reads() {
        assert_eq!(SYSTEM_ATTRIBUTES_FILE, "gitattributes");
        assert_eq!(GLOBAL_ATTRIBUTES_FILE, "attributes");
    }

    #[test]
    fn should_look_up_each_file_when_a_source_cannot_be_read() {
        let (dir, repo) = attrs_with_rules("");
        fs::create_dir_all(dir.path().join("x/.gitattributes")).unwrap();

        assert_eq!(
            diff_attrs_for(&repo, &[Path::new("x/a.txt")]),
            DiffAttrs::LookUpEachFile
        );
    }
}
