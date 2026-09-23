//! Shared auto-whitespace policy application for local VCS diffs.
//!
//! Auto mode materializes the Normal and IgnoreAll variants of the same
//! comparison at most once each, then selects whole `DiffFile` records by
//! `(old_path, new_path, status)` in original Normal order.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::error::{Result, TuicrError};
use crate::model::{DiffFile, FileStatus};
use crate::vcs::traits::{DiffWhitespaceMode, WhitespaceAutoPolicy};

/// Single-pass comparison used when a backend materializes hunks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WhitespaceComparison {
    Normal,
    IgnoreAll,
}

impl WhitespaceComparison {
    pub(crate) fn ignores_all(self) -> bool {
        matches!(self, Self::IgnoreAll)
    }
}

/// Run a backend comparison once for Normal/IgnoreAll, or twice for Auto
/// and then select whole-file results. Only `NoChanges` is treated as an
/// empty variant; every other error is propagated.
pub(crate) fn materialize_diff<F>(mode: &DiffWhitespaceMode, mut load: F) -> Result<Vec<DiffFile>>
where
    F: FnMut(WhitespaceComparison) -> Result<Vec<DiffFile>>,
{
    match mode {
        DiffWhitespaceMode::Normal => load(WhitespaceComparison::Normal),
        DiffWhitespaceMode::IgnoreAll => load(WhitespaceComparison::IgnoreAll),
        DiffWhitespaceMode::Auto(policy) => {
            let normal = match load(WhitespaceComparison::Normal) {
                Ok(files) => files,
                Err(TuicrError::NoChanges) => Vec::new(),
                Err(err) => return Err(err),
            };
            if normal.is_empty() {
                return Err(TuicrError::NoChanges);
            }
            let ignored = match load(WhitespaceComparison::IgnoreAll) {
                Ok(files) => files,
                Err(TuicrError::NoChanges) => Vec::new(),
                Err(err) => return Err(err),
            };
            let selected = select_auto_diff_files(policy, normal, ignored)?;
            if selected.is_empty() {
                Err(TuicrError::NoChanges)
            } else {
                Ok(selected)
            }
        }
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct FileIdentity {
    old_path: Option<PathBuf>,
    new_path: Option<PathBuf>,
    status: char,
}

fn file_identity(file: &DiffFile) -> FileIdentity {
    FileIdentity {
        old_path: file.old_path.clone(),
        new_path: file.new_path.clone(),
        status: file.status.as_char(),
    }
}

/// Select whole `DiffFile` records from the Normal-order list.
///
/// Ignored-extension files take the IgnoreAll record when present. A
/// whitespace-only modified file that disappears from the ignored variant
/// is omitted. Metadata-only records (binary, too-large, empty-hunk
/// rename/mode/added/deleted) are preserved from Normal when the ignored
/// variant lacks them. Hashes and line numbers always come from the chosen
/// record; unmatched substantive files fail instead of guessing identity.
pub(crate) fn select_auto_diff_files(
    policy: &WhitespaceAutoPolicy,
    normal: Vec<DiffFile>,
    ignored: Vec<DiffFile>,
) -> Result<Vec<DiffFile>> {
    let mut ignored_by_key = HashMap::with_capacity(ignored.len());
    for file in ignored {
        let key = file_identity(&file);
        if ignored_by_key.insert(key.clone(), file).is_some() {
            return Err(TuicrError::VcsCommand(format!(
                "auto whitespace selection found duplicate ignored file {}",
                display_identity(&key)
            )));
        }
    }

    let mut selected = Vec::with_capacity(normal.len());
    for file in normal {
        if !policy.ignores_diff_file(&file) {
            selected.push(file);
            continue;
        }

        let key = file_identity(&file);
        match ignored_by_key.remove(&key) {
            Some(ignored_file) => selected.push(ignored_file),
            None if preserve_unmatched_record(&file) => selected.push(file),
            None if is_expected_whitespace_omit(&file) => {}
            None => {
                return Err(TuicrError::VcsCommand(format!(
                    "auto whitespace selection could not match {}",
                    display_identity(&key)
                )));
            }
        }
    }
    Ok(selected)
}

fn preserve_unmatched_record(file: &DiffFile) -> bool {
    file.is_binary || file.is_too_large || file.is_commit_message || file.hunks.is_empty()
}

fn is_expected_whitespace_omit(file: &DiffFile) -> bool {
    file.status == FileStatus::Modified && !file.hunks.is_empty()
}

fn display_identity(key: &FileIdentity) -> String {
    let path = key
        .new_path
        .as_ref()
        .or(key.old_path.as_ref())
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "<unknown>".to_string());
    format!("{path} ({})", key.status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DiffHunk, DiffLine, LineOrigin};
    use crate::vcs::traits::comparison_extension;
    use std::path::{Path, PathBuf};

    fn policy() -> WhitespaceAutoPolicy {
        WhitespaceAutoPolicy::builtin()
    }

    fn hunk(old_start: u32, new_start: u32, content: &str) -> DiffHunk {
        DiffHunk {
            header: format!("@@ -{old_start},1 +{new_start},1 @@"),
            lines: vec![DiffLine {
                origin: LineOrigin::Addition,
                content: content.to_string(),
                old_lineno: None,
                new_lineno: Some(new_start),
                highlighted_spans: None,
            }],
            old_start,
            old_count: 1,
            new_start,
            new_count: 1,
        }
    }

    fn file(
        old: Option<&str>,
        new: Option<&str>,
        status: FileStatus,
        hash: u64,
        hunks: Vec<DiffHunk>,
    ) -> DiffFile {
        DiffFile {
            old_path: old.map(PathBuf::from),
            new_path: new.map(PathBuf::from),
            status,
            hunks,
            is_binary: false,
            is_too_large: false,
            is_commit_message: false,
            content_hash: hash,
        }
    }

    fn paths(files: &[DiffFile]) -> Vec<String> {
        files
            .iter()
            .map(|file| file.display_path().display().to_string())
            .collect()
    }

    #[test]
    fn uppercase_and_multi_dot_names_use_final_extension() {
        assert_eq!(
            comparison_extension(None, Some(Path::new("src/App.JSON"))),
            Some("json".into())
        );
        assert_eq!(
            comparison_extension(None, Some(Path::new("archive.tar.gz"))),
            Some("gz".into())
        );
        assert_eq!(
            comparison_extension(None, Some(Path::new("Foo.test.TSX"))),
            Some("tsx".into())
        );
    }

    #[test]
    fn extensionless_and_dotfiles_compare_normally() {
        assert_eq!(
            comparison_extension(None, Some(Path::new("Makefile"))),
            None
        );
        assert_eq!(
            comparison_extension(None, Some(Path::new(".gitignore"))),
            None
        );
        assert_eq!(comparison_extension(None, Some(Path::new(".rs"))), None);
        assert!(!policy().ignores_diff_file(&file(
            None,
            Some("Makefile"),
            FileStatus::Modified,
            1,
            vec![hunk(1, 1, "x")],
        )));
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_extension_compares_normally() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let path = PathBuf::from(OsStr::from_bytes(b"file.\xff\xfe"));
        assert_eq!(comparison_extension(None, Some(&path)), None);
        let mut diff = file(
            None,
            Some("file.bin"),
            FileStatus::Modified,
            1,
            vec![hunk(1, 1, "x")],
        );
        diff.new_path = Some(path);
        assert!(!policy().ignores_diff_file(&diff));
    }

    #[test]
    fn destination_extension_wins_for_renames_and_source_for_deletions() {
        let renamed = file(
            Some("old.py"),
            Some("new.json"),
            FileStatus::Renamed,
            1,
            Vec::new(),
        );
        assert!(policy().ignores_diff_file(&renamed));
        assert_eq!(
            comparison_extension(renamed.old_path.as_deref(), renamed.new_path.as_deref()),
            Some("json".into())
        );

        let deleted_python = file(Some("gone.py"), None, FileStatus::Deleted, 2, Vec::new());
        assert!(!policy().ignores_diff_file(&deleted_python));

        let deleted_rust = file(Some("gone.rs"), None, FileStatus::Deleted, 3, Vec::new());
        assert!(policy().ignores_diff_file(&deleted_rust));
    }

    #[test]
    fn python_and_yaml_compare_normally_unless_overridden() {
        let python = file(
            Some("app.py"),
            Some("app.py"),
            FileStatus::Modified,
            1,
            vec![hunk(1, 1, "x =  1")],
        );
        let yaml = file(
            Some("cfg.yaml"),
            Some("cfg.yaml"),
            FileStatus::Modified,
            2,
            vec![hunk(1, 1, "a:  1")],
        );
        assert!(!policy().ignores_diff_file(&python));
        assert!(!policy().ignores_diff_file(&yaml));

        let overridden = WhitespaceAutoPolicy::with_overrides([("py".into(), true)]);
        assert!(overridden.ignores_diff_file(&python));
        assert!(!overridden.ignores_diff_file(&yaml));
    }

    #[test]
    fn explicit_false_override_preserves_builtin_ignored_type() {
        let rust = file(
            Some("lib.rs"),
            Some("lib.rs"),
            FileStatus::Modified,
            1,
            vec![hunk(1, 1, "fn x(){ }")],
        );
        assert!(policy().ignores_diff_file(&rust));
        let sensitive_rust = WhitespaceAutoPolicy::with_overrides([("rs".into(), false)]);
        assert!(!sensitive_rust.ignores_diff_file(&rust));
    }

    #[test]
    fn selection_keeps_normal_order_and_ignored_coordinates() {
        let normal = vec![
            file(
                Some("a.json"),
                Some("a.json"),
                FileStatus::Modified,
                10,
                vec![hunk(1, 1, "  {")],
            ),
            file(
                Some("b.py"),
                Some("b.py"),
                FileStatus::Modified,
                20,
                vec![hunk(2, 2, "x =  1")],
            ),
            file(
                Some("c.rs"),
                Some("c.rs"),
                FileStatus::Modified,
                30,
                vec![hunk(3, 3, "fn x(){ }")],
            ),
        ];
        let ignored = vec![
            file(
                Some("c.rs"),
                Some("c.rs"),
                FileStatus::Modified,
                31,
                vec![hunk(30, 30, "fn x(){}")],
            ),
            file(
                Some("a.json"),
                Some("a.json"),
                FileStatus::Modified,
                11,
                Vec::new(),
            ),
        ];

        let selected = select_auto_diff_files(&policy(), normal, ignored).unwrap();
        assert_eq!(paths(&selected), vec!["a.json", "b.py", "c.rs"]);
        assert_eq!(selected[0].content_hash, 11);
        assert!(selected[0].hunks.is_empty());
        assert_eq!(selected[1].content_hash, 20);
        assert_eq!(selected[1].hunks[0].old_start, 2);
        assert_eq!(selected[2].content_hash, 31);
        assert_eq!(selected[2].hunks[0].new_start, 30);
    }

    #[test]
    fn missing_ignored_modified_file_is_omitted() {
        let normal = vec![
            file(
                Some("a.json"),
                Some("a.json"),
                FileStatus::Modified,
                10,
                vec![hunk(1, 1, "  {")],
            ),
            file(
                Some("b.py"),
                Some("b.py"),
                FileStatus::Modified,
                20,
                vec![hunk(2, 2, "x =  1")],
            ),
        ];

        let selected = select_auto_diff_files(&policy(), normal, Vec::new()).unwrap();
        assert_eq!(paths(&selected), vec!["b.py"]);
        assert_eq!(selected[0].content_hash, 20);
    }

    #[test]
    fn metadata_only_records_are_preserved_when_ignored_variant_lacks_them() {
        let mut binary = file(
            Some("data.json"),
            Some("data.json"),
            FileStatus::Modified,
            1,
            Vec::new(),
        );
        binary.is_binary = true;
        let mut too_large = file(None, Some("dump.js"), FileStatus::Added, 2, Vec::new());
        too_large.is_too_large = true;
        let renamed = file(
            Some("old.rs"),
            Some("new.rs"),
            FileStatus::Renamed,
            3,
            Vec::new(),
        );
        let added = file(None, Some("new.ts"), FileStatus::Added, 4, Vec::new());
        let deleted = file(Some("gone.rs"), None, FileStatus::Deleted, 5, Vec::new());
        let mode_only = file(
            Some("tool.rs"),
            Some("tool.rs"),
            FileStatus::Modified,
            6,
            Vec::new(),
        );

        let normal = vec![
            binary,
            too_large,
            renamed,
            added,
            deleted,
            mode_only,
            file(
                Some("keep.py"),
                Some("keep.py"),
                FileStatus::Modified,
                7,
                vec![hunk(1, 1, "x")],
            ),
        ];
        let selected = select_auto_diff_files(&policy(), normal, Vec::new()).unwrap();
        assert_eq!(
            paths(&selected),
            vec![
                "data.json",
                "dump.js",
                "new.rs",
                "new.ts",
                "gone.rs",
                "tool.rs",
                "keep.py"
            ]
        );
    }

    #[test]
    fn unmatched_substantive_non_modified_file_fails() {
        let normal = vec![file(
            None,
            Some("new.rs"),
            FileStatus::Added,
            1,
            vec![hunk(1, 1, "fn x() {}")],
        )];
        let err = select_auto_diff_files(&policy(), normal, Vec::new()).unwrap_err();
        assert!(
            matches!(err, TuicrError::VcsCommand(ref message) if message.contains("new.rs")),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn materialize_diff_propagates_non_nochanges_errors() {
        let mode = DiffWhitespaceMode::Auto(policy());
        let err = materialize_diff(&mode, |comparison| {
            if comparison == WhitespaceComparison::IgnoreAll {
                Err(TuicrError::VcsCommand("boom".into()))
            } else {
                Ok(vec![file(
                    Some("a.json"),
                    Some("a.json"),
                    FileStatus::Modified,
                    1,
                    vec![hunk(1, 1, "{")],
                )])
            }
        })
        .unwrap_err();
        assert!(matches!(err, TuicrError::VcsCommand(message) if message == "boom"));
    }

    #[test]
    fn materialize_diff_treats_only_nochanges_as_empty() {
        let mode = DiffWhitespaceMode::Auto(policy());
        let err = materialize_diff(&mode, |_| Err(TuicrError::NoChanges)).unwrap_err();
        assert!(matches!(err, TuicrError::NoChanges));
    }

    #[test]
    fn materialize_diff_single_pass_for_legacy_modes() {
        let mut calls = 0;
        materialize_diff(&DiffWhitespaceMode::Normal, |_| {
            calls += 1;
            Ok(vec![file(
                Some("a.py"),
                Some("a.py"),
                FileStatus::Modified,
                1,
                vec![hunk(1, 1, "x")],
            )])
        })
        .unwrap();
        assert_eq!(calls, 1);

        calls = 0;
        materialize_diff(&DiffWhitespaceMode::IgnoreAll, |_| {
            calls += 1;
            Ok(vec![file(
                Some("a.py"),
                Some("a.py"),
                FileStatus::Modified,
                1,
                vec![hunk(1, 1, "x")],
            )])
        })
        .unwrap();
        assert_eq!(calls, 1);
    }
}
