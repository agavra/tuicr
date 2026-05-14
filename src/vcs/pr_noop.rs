//! Placeholder VCS backend used in PR diff mode.
//!
//! When the App enters PR mode, the diff comes from the forge (`gh pr diff`),
//! not from a local working tree. The `VcsBackend` slot still needs to be
//! filled because the App and a number of other call sites assume one is
//! always present. `PrNoopVcs` satisfies that requirement without doing any
//! real work — every method either succeeds with an empty result or returns
//! `UnsupportedOperation`. PR-mode code paths route through
//! `ForgeContextProvider` for the operations that matter (context
//! expansion); they never call into the VCS backend.

use std::path::Path;

use crate::error::{Result, TuicrError};
use crate::model::{DiffFile, DiffLine, FileStatus};
use crate::syntax::SyntaxHighlighter;

use super::traits::{VcsBackend, VcsInfo};

pub struct PrNoopVcs {
    info: VcsInfo,
}

impl PrNoopVcs {
    pub fn new(info: VcsInfo) -> Self {
        Self { info }
    }
}

impl VcsBackend for PrNoopVcs {
    fn info(&self) -> &VcsInfo {
        &self.info
    }

    fn get_working_tree_diff(&self, _highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
        Err(TuicrError::UnsupportedOperation(
            "PR mode does not read from the local working tree".to_string(),
        ))
    }

    fn fetch_context_lines(
        &self,
        _file_path: &Path,
        _file_status: FileStatus,
        _start_line: u32,
        _end_line: u32,
    ) -> Result<Vec<DiffLine>> {
        // PR-mode context expansion routes through `ForgeContextProvider`.
        // If anything reaches this backend, it's a routing bug — return an
        // empty result rather than panicking so the UI degrades gracefully.
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vcs::traits::VcsType;
    use std::path::PathBuf;

    fn info() -> VcsInfo {
        VcsInfo {
            root_path: PathBuf::from("forge:github.com/a/b"),
            head_commit: "abc".to_string(),
            branch_name: Some("main".to_string()),
            vcs_type: VcsType::File,
        }
    }

    #[test]
    fn should_return_empty_context_lines_from_noop_backend() {
        // given
        let vcs = PrNoopVcs::new(info());
        // when
        let lines = vcs
            .fetch_context_lines(&PathBuf::from("x"), FileStatus::Modified, 1, 5)
            .unwrap();
        // then
        assert!(lines.is_empty());
    }

    #[test]
    fn should_reject_working_tree_diff_on_noop_backend() {
        // given
        let vcs = PrNoopVcs::new(info());
        // when
        let err = vcs
            .get_working_tree_diff(&SyntaxHighlighter::default())
            .unwrap_err();
        // then
        assert!(
            err.to_string()
                .contains("PR mode does not read from the local working tree"),
            "unexpected error: {err}"
        );
    }
}
