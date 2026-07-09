use std::fs;

use tempfile::tempdir;

use crate::app::*;
use crate::vcs::traits::VcsType;

struct StatusProbeMock {
    info: VcsInfo,
    status: VcsChangeStatus,
    staged_files: Vec<DiffFile>,
    unstaged_files: Vec<DiffFile>,
}

impl VcsBackend for StatusProbeMock {
    fn info(&self) -> &VcsInfo {
        &self.info
    }

    fn get_working_tree_diff(&self, _highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
        Err(TuicrError::NoChanges)
    }

    fn get_staged_diff(&self, _highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
        if self.staged_files.is_empty() {
            Err(TuicrError::NoChanges)
        } else {
            Ok(self.staged_files.clone())
        }
    }

    fn get_unstaged_diff(&self, _highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
        if self.unstaged_files.is_empty() {
            Err(TuicrError::NoChanges)
        } else {
            Ok(self.unstaged_files.clone())
        }
    }

    fn get_change_status(&self) -> Result<VcsChangeStatus> {
        Ok(self.status)
    }

    fn fetch_context_lines(
        &self,
        _file_path: &Path,
        _file_status: FileStatus,
        _ref_commit: Option<&str>,
        _start_line: u32,
        _end_line: u32,
    ) -> Result<Vec<DiffLine>> {
        Ok(Vec::new())
    }

    fn file_line_count(
        &self,
        _file_path: &Path,
        _file_status: FileStatus,
        _ref_commit: Option<&str>,
    ) -> Result<u32> {
        Ok(0)
    }
}

fn diff_file(path: &str) -> DiffFile {
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

fn mock_vcs(root_path: PathBuf) -> StatusProbeMock {
    StatusProbeMock {
        info: VcsInfo {
            root_path,
            head_commit: "HEAD".to_string(),
            branch_name: Some("main".to_string()),
            vcs_type: VcsType::Git,
        },
        status: VcsChangeStatus {
            staged: true,
            unstaged: true,
        },
        staged_files: Vec::new(),
        unstaged_files: Vec::new(),
    }
}

#[test]
fn status_probe_rechecks_positive_rows_when_ignore_rules_exist() {
    let dir = tempdir().expect("failed to create temp dir");
    fs::write(dir.path().join(".tuicrignore"), "ignored/\n").expect("failed to write .tuicrignore");
    let mut vcs = mock_vcs(dir.path().to_path_buf());
    vcs.staged_files = vec![diff_file("ignored/generated.rs")];
    vcs.unstaged_files = vec![diff_file("src/lib.rs")];

    let status =
        App::get_change_status_with_ignore(&vcs, dir.path(), &SyntaxHighlighter::default(), None)
            .expect("failed to get change status");

    assert_eq!(
        status,
        VcsChangeStatus {
            staged: false,
            unstaged: true,
        }
    );
    // The mock backend's full-diff path was used (no list_changed_paths
    // override), so it returned the unstaged file. With a real backend
    // that implements list_changed_paths the same path-filter logic runs
    // without ever materializing the diff.
}

#[test]
fn status_probe_does_not_load_diffs_without_ignore_rules() {
    let dir = tempdir().expect("failed to create temp dir");
    let vcs = mock_vcs(dir.path().to_path_buf());

    let status =
        App::get_change_status_with_ignore(&vcs, dir.path(), &SyntaxHighlighter::default(), None)
            .expect("failed to get change status");

    assert_eq!(
        status,
        VcsChangeStatus {
            staged: true,
            unstaged: true,
        }
    );
    // No ignore rules → no diffs were loaded by the probe (the mock's
    // get_X_diff methods would have errored if hit, since staged_files
    // and unstaged_files are empty).
}

#[test]
fn list_changed_paths_used_to_verify_ignore_rules() {
    use std::cell::Cell;

    struct PathProbeMock {
        inner: StatusProbeMock,
        list_calls: Cell<u32>,
        staged_paths: Vec<PathBuf>,
        unstaged_paths: Vec<PathBuf>,
    }

    impl VcsBackend for PathProbeMock {
        fn info(&self) -> &VcsInfo {
            self.inner.info()
        }
        fn get_working_tree_diff(&self, _highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
            Err(TuicrError::UnsupportedOperation(
                "should not be called".into(),
            ))
        }
        fn get_staged_diff(&self, _highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
            Err(TuicrError::UnsupportedOperation(
                "should not be called".into(),
            ))
        }
        fn get_unstaged_diff(&self, _highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
            Err(TuicrError::UnsupportedOperation(
                "should not be called".into(),
            ))
        }
        fn get_change_status(&self) -> Result<VcsChangeStatus> {
            self.inner.get_change_status()
        }
        fn list_changed_paths(&self, kind: ChangeKind) -> Result<Vec<PathBuf>> {
            self.list_calls.set(self.list_calls.get() + 1);
            Ok(match kind {
                ChangeKind::Staged => self.staged_paths.clone(),
                ChangeKind::Unstaged => self.unstaged_paths.clone(),
            })
        }
        fn fetch_context_lines(
            &self,
            _file_path: &Path,
            _file_status: FileStatus,
            _ref_commit: Option<&str>,
            _start_line: u32,
            _end_line: u32,
        ) -> Result<Vec<DiffLine>> {
            Ok(Vec::new())
        }
        fn file_line_count(
            &self,
            _file_path: &Path,
            _file_status: FileStatus,
            _ref_commit: Option<&str>,
        ) -> Result<u32> {
            Ok(0)
        }
    }

    let dir = tempdir().expect("failed to create temp dir");
    fs::write(dir.path().join(".tuicrignore"), "ignored/\n").expect("failed to write .tuicrignore");

    let vcs = PathProbeMock {
        inner: mock_vcs(dir.path().to_path_buf()),
        list_calls: Cell::new(0),
        staged_paths: vec![PathBuf::from("ignored/generated.rs")],
        unstaged_paths: vec![PathBuf::from("src/lib.rs")],
    };

    let status =
        App::get_change_status_with_ignore(&vcs, dir.path(), &SyntaxHighlighter::default(), None)
            .expect("failed to get change status");

    assert_eq!(
        status,
        VcsChangeStatus {
            staged: false,
            unstaged: true,
        }
    );
    // Both sides probed via cheap path listing, not via full diff parsing.
    assert_eq!(vcs.list_calls.get(), 2);
}
