use chrono::{DateTime, Utc};
use std::borrow::Cow;
use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::model::{DiffFile, DiffLine, FileStatus};
use crate::syntax::SyntaxHighlighter;

/// Information about the VCS type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VcsType {
    Git,
    Mercurial,
    Jujutsu,
    File,
}

impl std::fmt::Display for VcsType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VcsType::Git => write!(f, "git"),
            VcsType::Mercurial => write!(f, "hg"),
            VcsType::Jujutsu => write!(f, "jj"),
            VcsType::File => write!(f, "file"),
        }
    }
}

/// Repository information
#[derive(Debug, Clone)]
pub struct VcsInfo {
    pub root_path: PathBuf,
    pub head_commit: String,
    pub branch_name: Option<String>,
    /// VCS type - displayed in status bar header
    pub vcs_type: VcsType,
}

/// Whitespace comparison policy used when a backend materializes diff hunks.
///
/// This is a local-diff setting,
/// not a review-session identity or forge option.
/// Cheap change probes intentionally ignore it so selectors can still show
/// staged/unstaged choices before the full diff is loaded.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DiffWhitespaceMode {
    /// Compare whitespace normally.
    #[default]
    Normal,
    /// Ignore all whitespace while comparing lines.
    IgnoreAll,
}

impl DiffWhitespaceMode {
    pub fn ignores_all(self) -> bool {
        matches!(self, Self::IgnoreAll)
    }
}

/// Commit information for commit selection UI
#[derive(Debug, Clone)]
pub struct CommitInfo {
    pub id: String,
    pub short_id: String,
    /// Optional branch label for this commit in commit selection UI.
    /// For Git this is populated for commits that are branch tips.
    pub branch_name: Option<String>,
    pub summary: String,
    pub body: Option<String>,
    pub author: String,
    pub time: DateTime<Utc>,
}

/// ResolvedRevisionRange is the VCS boundary's parsed form of a user-provided
/// revision expression.
///
/// It preserves the knowledge gained while resolving the expression so callers
/// do not need to keep passing the raw revision string through lower layers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRevisionRange<'a> {
    /// Commit IDs selected by the expression, oldest first.
    pub commit_ids: Cow<'a, [String]>,

    /// Boundary to use when materializing the file diff for this range.
    pub diff_target: RevisionDiffTarget,
}

impl<'a> ResolvedRevisionRange<'a> {
    /// Build a range from borrowed commit IDs and a diff boundary.
    ///
    /// Selection and reload paths use this when application state already
    /// stores the selected commits and no clone is needed.
    pub fn from_commit_ids(commit_ids: &'a [String], diff_target: RevisionDiffTarget) -> Self {
        Self {
            commit_ids: Cow::Borrowed(commit_ids),
            diff_target,
        }
    }

    /// Build a range from owned commit IDs and a diff boundary.
    ///
    /// VCS adapters use this after resolving both the commit list and the
    /// old/new diff target that should be used to materialize the range.
    pub fn from_owned_commit_ids(commit_ids: Vec<String>, diff_target: RevisionDiffTarget) -> Self {
        Self {
            commit_ids: Cow::Owned(commit_ids),
            diff_target,
        }
    }
}

/// RevisionDiffTarget describes the old and new sides of a revision diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevisionDiffTarget {
    /// Infer diff endpoints from the selected commit list.
    ///
    /// This is used when the user selected commits as commits,
    /// for example through the inline commit selector,
    /// reloading an active commit-range session,
    /// or a backend that only reports a resolved commit list.
    /// In this mode,
    /// backends infer the old side from the oldest selected commit.
    CommitList,

    /// Compare the resolved base revision to the resolved head revision.
    ///
    /// This is used when the user supplied a revision expression,
    /// for example `tuicr -r A..B`,
    /// `tuicr -r A...B`,
    /// or `tuicr -r REV`.
    /// In this mode,
    /// the parsed expression supplies the diff boundary directly.
    Explicit {
        /// Old side of the diff, or None for the empty tree.
        base: Option<String>,

        /// New side of the diff.
        head: String,
    },
}

/// Cheap repository change summary used by selection UIs before loading full diffs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VcsChangeStatus {
    pub staged: bool,
    pub unstaged: bool,
}

/// Which side of the working tree to enumerate for the cheap path probes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// HEAD vs index — paths that would land in `get_staged_diff`.
    Staged,
    /// Index vs workdir, including untracked — paths that would land in
    /// `get_unstaged_diff`.
    Unstaged,
}

/// Trait for VCS backend implementations
pub trait VcsBackend: Send {
    /// Get repository information
    fn info(&self) -> &VcsInfo;

    /// Non-fatal notices that should be shown after startup.
    fn startup_warnings(&self) -> Vec<String> {
        Vec::new()
    }

    /// Whether this concrete backend can operate on Git sparse-checkout repos.
    ///
    /// Non-Git VCS implementations keep the default `false`; Git backends
    /// override this based on the selected implementation.
    fn supports_sparse_checkout(&self) -> bool {
        false
    }

    /// Get the working tree diff (staged + unstaged changes)
    fn get_working_tree_diff(&self, highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>>;

    /// Get the staged diff (index vs HEAD)
    fn get_staged_diff(&self, _highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
        Err(crate::error::TuicrError::UnsupportedOperation(
            "Staged diff not supported for this VCS".into(),
        ))
    }

    /// Get the unstaged diff (working tree vs index)
    fn get_unstaged_diff(&self, _highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
        Err(crate::error::TuicrError::UnsupportedOperation(
            "Unstaged diff not supported for this VCS".into(),
        ))
    }

    /// Get a cheap staged/unstaged summary without parsing or highlighting diffs.
    fn get_change_status(&self) -> Result<VcsChangeStatus> {
        Err(crate::error::TuicrError::UnsupportedOperation(
            "Change status not supported for this VCS".into(),
        ))
    }

    /// List the paths that would appear in the corresponding diff, without
    /// parsing hunks or highlighting. Used to verify `get_change_status`
    /// against `.gitignore`/`.tuicrignore` rules without paying the diff cost.
    fn list_changed_paths(&self, _kind: ChangeKind) -> Result<Vec<PathBuf>> {
        Err(crate::error::TuicrError::UnsupportedOperation(
            "Listing changed paths not supported for this VCS".into(),
        ))
    }

    /// Fetch context lines for gap expansion.
    /// When `ref_commit` is `Some`, reads from that commit; otherwise reads
    /// from the working tree (or VCS HEAD for deleted files).
    fn fetch_context_lines(
        &self,
        file_path: &Path,
        file_status: FileStatus,
        ref_commit: Option<&str>,
        start_line: u32,
        end_line: u32,
    ) -> Result<Vec<DiffLine>>;

    /// Get the total number of lines in a file.
    /// When `ref_commit` is `Some`, reads from that commit; otherwise reads
    /// from the working tree (or VCS HEAD for deleted files).
    fn file_line_count(
        &self,
        file_path: &Path,
        file_status: FileStatus,
        ref_commit: Option<&str>,
    ) -> Result<u32>;

    /// Get recent commits for commit selection UI.
    /// Returns empty vec if not supported (default).
    fn get_recent_commits(&self, _offset: usize, _limit: usize) -> Result<Vec<CommitInfo>> {
        Ok(Vec::new())
    }

    /// Resolve a revisions expression into the VCS boundary's parsed range.
    /// Returns error if not supported (default).
    fn resolve_revision_range(&self, _revisions: &str) -> Result<ResolvedRevisionRange<'static>> {
        Err(crate::error::TuicrError::UnsupportedOperation(
            "Revision range resolution not supported for this VCS".into(),
        ))
    }

    /// Get diff for a commit range.
    /// Returns error if not supported (default).
    fn get_commit_range_diff(
        &self,
        _revision_range: &ResolvedRevisionRange<'_>,
        _highlighter: &SyntaxHighlighter,
    ) -> Result<Vec<DiffFile>> {
        Err(crate::error::TuicrError::UnsupportedOperation(
            "Commit range diff not supported for this VCS".into(),
        ))
    }

    /// Get commit info for specific commit IDs (for inline commit selector).
    /// Returns CommitInfo for each ID, in the same order as the input.
    fn get_commits_info(&self, _ids: &[String]) -> Result<Vec<CommitInfo>> {
        Ok(Vec::new())
    }

    /// Get a combined diff from the parent of the oldest commit through to the working tree.
    /// This shows both committed and working tree changes in a single diff.
    /// Returns error if not supported (default).
    fn get_working_tree_with_commits_diff(
        &self,
        _commit_ids: &[String],
        _highlighter: &SyntaxHighlighter,
    ) -> Result<Vec<DiffFile>> {
        Err(crate::error::TuicrError::UnsupportedOperation(
            "Working tree + commits diff not supported for this VCS".into(),
        ))
    }

    /// Stage a file (add to index).
    fn stage_file(&self, _path: &Path) -> Result<()> {
        Err(crate::error::TuicrError::UnsupportedOperation(
            "Staging not supported for this VCS".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vcs_type_display_git() {
        assert_eq!(format!("{}", VcsType::Git), "git");
    }

    #[test]
    fn vcs_type_display_mercurial() {
        assert_eq!(format!("{}", VcsType::Mercurial), "hg");
    }

    #[test]
    fn vcs_type_display_jujutsu() {
        assert_eq!(format!("{}", VcsType::Jujutsu), "jj");
    }

    #[test]
    fn vcs_type_equality() {
        assert_eq!(VcsType::Git, VcsType::Git);
        assert_eq!(VcsType::Mercurial, VcsType::Mercurial);
        assert_ne!(VcsType::Git, VcsType::Mercurial);
        assert_eq!(VcsType::Jujutsu, VcsType::Jujutsu);
        assert_ne!(VcsType::Git, VcsType::Jujutsu);
    }

    #[test]
    fn vcs_info_clone() {
        let info = VcsInfo {
            root_path: PathBuf::from("/test/repo"),
            head_commit: "abc123".to_string(),
            branch_name: Some("main".to_string()),
            vcs_type: VcsType::Git,
        };

        let cloned = info.clone();
        assert_eq!(cloned.root_path, PathBuf::from("/test/repo"));
        assert_eq!(cloned.head_commit, "abc123");
        assert_eq!(cloned.branch_name, Some("main".to_string()));
        assert_eq!(cloned.vcs_type, VcsType::Git);
    }

    #[test]
    fn vcs_info_without_branch() {
        let info = VcsInfo {
            root_path: PathBuf::from("/detached"),
            head_commit: "def456".to_string(),
            branch_name: None,
            vcs_type: VcsType::Git,
        };

        assert!(info.branch_name.is_none());
    }

    #[test]
    fn commit_info_clone() {
        let commit = CommitInfo {
            id: "abc123def456".to_string(),
            short_id: "abc123d".to_string(),
            branch_name: Some("main".to_string()),
            summary: "Fix bug".to_string(),
            body: None,
            author: "Test User".to_string(),
            time: Utc::now(),
        };

        let cloned = commit.clone();
        assert_eq!(cloned.id, "abc123def456");
        assert_eq!(cloned.short_id, "abc123d");
        assert_eq!(cloned.branch_name, Some("main".to_string()));
        assert_eq!(cloned.summary, "Fix bug");
        assert_eq!(cloned.author, "Test User");
    }
}
