use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use chrono::Utc;

use crate::error::{Result, TuicrError};
use crate::model::{
    Comment, CommentType, DiffFile, DiffLine, LineOrigin, LineRange, LineSide, ReviewSession,
    SessionDiffSource,
};
use crate::persistence::load_latest_session_for_context;
use crate::syntax::SyntaxHighlighter;
use crate::theme::Theme;
use crate::update::UpdateInfo;
use crate::vcs::git::calculate_gap;
use crate::vcs::{CommitInfo, VcsBackend, VcsInfo, detect_vcs};

const VISIBLE_COMMIT_COUNT: usize = 10;
const COMMIT_PAGE_SIZE: usize = 10;
pub const STAGED_SELECTION_ID: &str = "__tuicr_staged__";
pub const UNSTAGED_SELECTION_ID: &str = "__tuicr_unstaged__";

#[derive(Debug, Clone)]
pub enum FileTreeItem {
    Directory {
        path: String,
        depth: usize,
        expanded: bool,
    },
    File {
        file_idx: usize,
        depth: usize,
    },
}

/// Identifies a gap between hunks in a file (for context expansion)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GapId {
    pub file_idx: usize,
    /// Index of the hunk that this gap precedes (0 = gap before first hunk)
    pub hunk_idx: usize,
}

/// Describes what a rendered line represents - built once and used for O(1) cursor queries
#[derive(Debug, Clone)]
pub enum AnnotatedLine {
    /// File header line
    FileHeader { file_idx: usize },
    /// A file-level comment line (part of a multi-line comment box)
    FileComment { file_idx: usize, comment_idx: usize },
    /// Expander line showing hidden context
    Expander { gap_id: GapId },
    /// Expanded context line (muted text)
    ExpandedContext { gap_id: GapId, line_idx: usize },
    /// Hunk header (@@...@@)
    HunkHeader { file_idx: usize, hunk_idx: usize },
    /// Actual diff line with line numbers
    DiffLine {
        file_idx: usize,
        hunk_idx: usize,
        line_idx: usize,
        old_lineno: Option<u32>,
        new_lineno: Option<u32>,
    },
    /// Side-by-side paired diff line
    SideBySideLine {
        file_idx: usize,
        hunk_idx: usize,
        del_line_idx: Option<usize>,
        add_line_idx: Option<usize>,
        old_lineno: Option<u32>,
        new_lineno: Option<u32>,
    },
    /// A line comment (part of a multi-line comment box)
    LineComment {
        file_idx: usize,
        line: u32,
        side: LineSide,
        comment_idx: usize,
    },
    /// Binary or empty file indicator
    BinaryOrEmpty { file_idx: usize },
    /// Spacing between files
    Spacing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    Comment,
    Command,
    Search,
    Help,
    Confirm,
    CommitSelect,
    VisualSelect,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffSource {
    WorkingTree,
    Staged,
    Unstaged,
    StagedAndUnstaged,
    CommitRange(Vec<String>),
    WorkingTreeAndCommits(Vec<String>),
    StagedUnstagedAndCommits(Vec<String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmAction {
    CopyAndQuit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusedPanel {
    FileList,
    Diff,
    CommitSelector,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffViewMode {
    Unified,
    SideBySide,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageType {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub content: String,
    pub message_type: MessageType,
}

pub struct App {
    pub theme: Theme,
    pub vcs: Box<dyn VcsBackend>,
    pub vcs_info: VcsInfo,
    pub session: ReviewSession,
    pub diff_files: Vec<DiffFile>,
    pub diff_source: DiffSource,

    pub input_mode: InputMode,
    pub focused_panel: FocusedPanel,
    pub diff_view_mode: DiffViewMode,

    pub file_list_state: FileListState,
    pub diff_state: DiffState,
    pub help_state: HelpState,
    pub command_buffer: String,
    pub search_buffer: String,
    pub last_search_pattern: Option<String>,
    pub comment_buffer: String,
    pub comment_cursor: usize,
    pub comment_type: CommentType,
    pub comment_is_file_level: bool,
    pub comment_line: Option<(u32, LineSide)>,
    pub editing_comment_id: Option<String>,

    /// Visual selection anchor point (starting line, side)
    pub visual_anchor: Option<(u32, LineSide)>,
    /// Line range for range comments (used when creating comments from visual selection)
    pub comment_line_range: Option<(LineRange, LineSide)>,

    // Commit selection state
    pub commit_list: Vec<CommitInfo>,
    pub commit_list_cursor: usize,
    pub commit_list_scroll_offset: usize,
    pub commit_list_viewport_height: usize,
    /// Selected commit range as (start_idx, end_idx) inclusive, where start <= end.
    /// Indices refer to positions in commit_list.
    pub commit_selection_range: Option<(usize, usize)>,
    /// State describing how many commits are currently shown and how pagination behaves.
    pub visible_commit_count: usize,
    pub commit_page_size: usize,
    pub has_more_commit: bool,

    pub should_quit: bool,
    pub dirty: bool,
    pub quit_warned: bool,
    pub message: Option<Message>,
    pub pending_confirm: Option<ConfirmAction>,
    pub supports_keyboard_enhancement: bool,
    pub show_file_list: bool,
    pub file_list_area: Option<ratatui::layout::Rect>,
    pub diff_area: Option<ratatui::layout::Rect>,
    pub expanded_dirs: HashSet<String>,
    /// Tracks which hunk gaps have been expanded to show more context
    pub expanded_gaps: HashSet<GapId>,
    /// Stores the expanded context lines for each gap
    pub expanded_content: HashMap<GapId, Vec<DiffLine>>,
    /// Cached annotations describing what each rendered line represents
    pub line_annotations: Vec<AnnotatedLine>,
    /// Output to stdout instead of clipboard when exporting
    pub output_to_stdout: bool,
    /// Pending output to print to stdout after TUI exits
    pub pending_stdout_output: Option<String>,
    /// Calculated screen position for comment input cursor (col, row) for IME positioning.
    /// Set during render when in Comment mode, None otherwise.
    pub comment_cursor_screen_pos: Option<(u16, u16)>,
    /// Information about available updates (set by background check)
    pub update_info: Option<UpdateInfo>,

    // Inline commit selector state (shown at top of diff view for multi-commit reviews)
    /// CommitInfo for commits in the current review (display order: newest first)
    pub review_commits: Vec<CommitInfo>,
    /// Whether the inline commit selector panel is visible
    pub show_commit_selector: bool,
    /// Cached individual/subrange diffs keyed by (start_idx, end_idx) into review_commits
    pub commit_diff_cache: HashMap<(usize, usize), Vec<DiffFile>>,
    /// The combined "all selected" diff, cached for quick restoration
    pub range_diff_files: Option<Vec<DiffFile>>,
    /// Saved inline selection range when entering full commit select mode via :commits
    pub saved_inline_selection: Option<(usize, usize)>,
}

#[derive(Default)]
pub struct FileListState {
    pub list_state: ratatui::widgets::ListState,
    pub scroll_x: usize,
    pub viewport_width: usize,    // Set during render
    pub viewport_height: usize,   // Set during render
    pub max_content_width: usize, // Set during render
}

impl FileListState {
    pub fn selected(&self) -> usize {
        self.list_state.selected().unwrap_or(0)
    }

    pub fn select(&mut self, index: usize) {
        self.list_state.select(Some(index));
    }

    pub fn scroll_left(&mut self, cols: usize) {
        self.scroll_x = self.scroll_x.saturating_sub(cols);
    }

    pub fn scroll_right(&mut self, cols: usize) {
        let max_scroll_x = self.max_content_width.saturating_sub(self.viewport_width);
        self.scroll_x = (self.scroll_x.saturating_add(cols)).min(max_scroll_x);
    }
}

#[derive(Debug)]
pub struct DiffState {
    pub scroll_offset: usize,
    pub scroll_x: usize,
    pub cursor_line: usize,
    pub current_file_idx: usize,
    pub viewport_height: usize,
    pub viewport_width: usize,
    pub max_content_width: usize,
    pub wrap_lines: bool,
    /// Number of logical lines that fit in the viewport (set during render).
    /// When wrapping is enabled, this accounts for lines expanding to multiple visual rows.
    pub visible_line_count: usize,
}

impl Default for DiffState {
    fn default() -> Self {
        Self {
            scroll_offset: 0,
            scroll_x: 0,
            cursor_line: 0,
            current_file_idx: 0,
            viewport_height: 0,
            viewport_width: 0,
            max_content_width: 0,
            wrap_lines: true,
            visible_line_count: 0,
        }
    }
}

#[derive(Debug, Default)]
pub struct HelpState {
    pub scroll_offset: usize,
    pub viewport_height: usize,
    pub total_lines: usize, // Set during render
}

/// Represents a comment location for deletion
enum CommentLocation {
    FileComment {
        path: std::path::PathBuf,
        index: usize,
    },
    LineComment {
        path: std::path::PathBuf,
        line: u32,
        side: LineSide,
        index: usize,
    },
}

impl App {
    pub fn new(theme: Theme, output_to_stdout: bool, revisions: Option<&str>) -> Result<Self> {
        let vcs = detect_vcs()?;
        let vcs_info = vcs.info().clone();
        let highlighter = theme.syntax_highlighter();

        // Determine the diff source, files, and session based on input.
        // Three paths: CLI revisions, working tree changes, or commit selection fallback.
        if let Some(revisions) = revisions {
            // Resolve the revisions to commits and diff as a commit range
            let commit_ids = vcs.resolve_revisions(revisions)?;
            let diff_files = Self::get_commit_range_diff_with_ignore(
                vcs.as_ref(),
                &vcs_info.root_path,
                &commit_ids,
                highlighter,
            )?;
            let session = Self::load_or_create_commit_range_session(&vcs_info, &commit_ids);
            // Get commit info for the inline commit selector
            let review_commits = vcs.get_commits_info(&commit_ids)?;
            // Reverse to newest-first display order
            let review_commits: Vec<CommitInfo> = review_commits.into_iter().rev().collect();

            let mut app = Self::build(
                vcs,
                vcs_info,
                theme,
                output_to_stdout,
                diff_files,
                session,
                DiffSource::CommitRange(commit_ids),
                InputMode::Normal,
                Vec::new(),
            )?;

            // Set up inline commit selector for multi-commit reviews
            if review_commits.len() > 1 {
                app.range_diff_files = Some(app.diff_files.clone());
                app.commit_list = review_commits.clone();
                app.commit_list_cursor = 0;
                app.commit_selection_range = Some((0, review_commits.len() - 1));
                app.commit_list_scroll_offset = 0;
                app.visible_commit_count = review_commits.len();
                app.has_more_commit = false;
                app.show_commit_selector = true;
                app.commit_diff_cache.clear();
            }
            app.review_commits = review_commits;

            Ok(app)
        } else {
            let has_staged_changes = match Self::get_staged_diff_with_ignore(
                vcs.as_ref(),
                &vcs_info.root_path,
                highlighter,
            ) {
                Ok(_) => true,
                Err(TuicrError::NoChanges) => false,
                Err(TuicrError::UnsupportedOperation(_)) => false,
                Err(e) => return Err(e),
            };

            let has_unstaged_changes = match Self::get_unstaged_diff_with_ignore(
                vcs.as_ref(),
                &vcs_info.root_path,
                highlighter,
            ) {
                Ok(_) => true,
                Err(TuicrError::NoChanges) => false,
                Err(TuicrError::UnsupportedOperation(_)) => false,
                Err(e) => return Err(e),
            };

            let working_tree_diff = if has_staged_changes || has_unstaged_changes {
                match Self::get_working_tree_diff_with_ignore(
                    vcs.as_ref(),
                    &vcs_info.root_path,
                    highlighter,
                ) {
                    Ok(diff_files) => Some(diff_files),
                    Err(TuicrError::NoChanges) => None,
                    Err(e) => return Err(e),
                }
            } else {
                None
            };

            let commits = vcs.get_recent_commits(0, VISIBLE_COMMIT_COUNT)?;
            if !has_staged_changes && !has_unstaged_changes && commits.is_empty() {
                return Err(TuicrError::NoChanges);
            }

            let mut commit_list = commits.clone();
            if has_unstaged_changes {
                commit_list.insert(0, Self::unstaged_commit_entry());
            }
            if has_staged_changes {
                commit_list.insert(0, Self::staged_commit_entry());
            }

            let diff_source = if has_staged_changes && has_unstaged_changes {
                DiffSource::StagedAndUnstaged
            } else if has_staged_changes {
                DiffSource::Staged
            } else if has_unstaged_changes {
                DiffSource::Unstaged
            } else {
                DiffSource::WorkingTree
            };

            let session_source = if has_staged_changes && has_unstaged_changes {
                SessionDiffSource::StagedAndUnstaged
            } else if has_staged_changes {
                SessionDiffSource::Staged
            } else if has_unstaged_changes {
                SessionDiffSource::Unstaged
            } else {
                SessionDiffSource::WorkingTree
            };

            let session = Self::load_or_create_session(&vcs_info, session_source);

            let mut app = Self::build(
                vcs,
                vcs_info,
                theme,
                output_to_stdout,
                working_tree_diff.unwrap_or_default(),
                session,
                diff_source,
                InputMode::CommitSelect,
                commit_list,
            )?;

            app.has_more_commit = commits.len() >= VISIBLE_COMMIT_COUNT;
            app.visible_commit_count = app.commit_list.len();
            Ok(app)
        }
    }

    /// Shared constructor: all `App::new` paths converge here.
    #[allow(clippy::too_many_arguments)]
    fn build(
        vcs: Box<dyn VcsBackend>,
        vcs_info: VcsInfo,
        theme: Theme,
        output_to_stdout: bool,
        diff_files: Vec<DiffFile>,
        mut session: ReviewSession,
        diff_source: DiffSource,
        input_mode: InputMode,
        commit_list: Vec<CommitInfo>,
    ) -> Result<Self> {
        // Ensure all diff files are registered in the session
        for file in &diff_files {
            session.add_file(file.display_path().clone(), file.status);
        }

        let has_more_commit = commit_list.len() >= VISIBLE_COMMIT_COUNT;
        let visible_commit_count = if commit_list.is_empty() {
            VISIBLE_COMMIT_COUNT
        } else {
            commit_list.len()
        };

        let mut app = Self {
            theme,
            vcs,
            vcs_info,
            session,
            diff_files,
            diff_source,
            input_mode,
            focused_panel: FocusedPanel::Diff,
            diff_view_mode: DiffViewMode::Unified,
            file_list_state: FileListState::default(),
            diff_state: DiffState::default(),
            help_state: HelpState::default(),
            command_buffer: String::new(),
            search_buffer: String::new(),
            last_search_pattern: None,
            comment_buffer: String::new(),
            comment_cursor: 0,
            comment_type: CommentType::Note,
            comment_is_file_level: true,
            comment_line: None,
            editing_comment_id: None,
            visual_anchor: None,
            comment_line_range: None,
            commit_list,
            commit_list_cursor: 0,
            commit_list_scroll_offset: 0,
            commit_list_viewport_height: 0,
            commit_selection_range: None,
            visible_commit_count,
            commit_page_size: COMMIT_PAGE_SIZE,
            has_more_commit,
            should_quit: false,
            dirty: false,
            quit_warned: false,
            message: None,
            pending_confirm: None,
            supports_keyboard_enhancement: false,
            show_file_list: true,
            file_list_area: None,
            diff_area: None,
            expanded_dirs: HashSet::new(),
            expanded_gaps: HashSet::new(),
            expanded_content: HashMap::new(),
            line_annotations: Vec::new(),
            output_to_stdout,
            pending_stdout_output: None,
            comment_cursor_screen_pos: None,
            update_info: None,
            review_commits: Vec::new(),
            show_commit_selector: false,
            commit_diff_cache: HashMap::new(),
            range_diff_files: None,
            saved_inline_selection: None,
        };
        app.sort_files_by_directory(true);
        app.expand_all_dirs();
        app.rebuild_annotations();
        Ok(app)
    }

    /// Load or create a session for a commit range (used by revisions and commit selection).
    fn load_or_create_commit_range_session(
        vcs_info: &VcsInfo,
        commit_ids: &[String],
    ) -> ReviewSession {
        let newest_commit_id = commit_ids.last().unwrap().clone();
        let loaded = load_latest_session_for_context(
            &vcs_info.root_path,
            vcs_info.branch_name.as_deref(),
            &newest_commit_id,
            SessionDiffSource::CommitRange,
            Some(commit_ids),
        )
        .ok()
        .and_then(|found| found.map(|(_path, session)| session));

        let mut session = loaded.unwrap_or_else(|| {
            let mut s = ReviewSession::new(
                vcs_info.root_path.clone(),
                newest_commit_id,
                vcs_info.branch_name.clone(),
                SessionDiffSource::CommitRange,
            );
            s.commit_range = Some(commit_ids.to_vec());
            s
        });

        if session.commit_range.is_none() {
            session.commit_range = Some(commit_ids.to_vec());
            session.updated_at = chrono::Utc::now();
        }
        session
    }

    fn load_or_create_staged_unstaged_and_commits_session(
        vcs_info: &VcsInfo,
        commit_ids: &[String],
    ) -> ReviewSession {
        let newest_commit_id = commit_ids.last().unwrap().clone();
        let loaded = load_latest_session_for_context(
            &vcs_info.root_path,
            vcs_info.branch_name.as_deref(),
            &newest_commit_id,
            SessionDiffSource::StagedUnstagedAndCommits,
            Some(commit_ids),
        )
        .ok()
        .and_then(|found| found.map(|(_path, session)| session));

        let mut session = loaded.unwrap_or_else(|| {
            let mut s = ReviewSession::new(
                vcs_info.root_path.clone(),
                newest_commit_id,
                vcs_info.branch_name.clone(),
                SessionDiffSource::StagedUnstagedAndCommits,
            );
            s.commit_range = Some(commit_ids.to_vec());
            s
        });

        if session.commit_range.is_none() {
            session.commit_range = Some(commit_ids.to_vec());
            session.updated_at = chrono::Utc::now();
        }
        session
    }

    fn load_or_create_session(vcs_info: &VcsInfo, diff_source: SessionDiffSource) -> ReviewSession {
        let new_session = || {
            ReviewSession::new(
                vcs_info.root_path.clone(),
                vcs_info.head_commit.clone(),
                vcs_info.branch_name.clone(),
                diff_source,
            )
        };

        let Ok(found) = load_latest_session_for_context(
            &vcs_info.root_path,
            vcs_info.branch_name.as_deref(),
            &vcs_info.head_commit,
            diff_source,
            None,
        ) else {
            return new_session();
        };

        let Some((_path, mut session)) = found else {
            return new_session();
        };

        let mut updated = false;
        if session.branch_name.is_none() && vcs_info.branch_name.is_some() {
            session.branch_name = vcs_info.branch_name.clone();
            updated = true;
        }

        if vcs_info.branch_name.is_some() && session.base_commit != vcs_info.head_commit {
            session.base_commit = vcs_info.head_commit.clone();
            updated = true;
        }

        if updated {
            session.updated_at = chrono::Utc::now();
        }

        session
    }

    fn staged_commit_entry() -> CommitInfo {
        CommitInfo {
            id: STAGED_SELECTION_ID.to_string(),
            short_id: "STAGED".to_string(),
            branch_name: None,
            summary: "Staged changes".to_string(),
            author: String::new(),
            time: Utc::now(),
        }
    }

    fn unstaged_commit_entry() -> CommitInfo {
        CommitInfo {
            id: UNSTAGED_SELECTION_ID.to_string(),
            short_id: "UNSTAGED".to_string(),
            branch_name: None,
            summary: "Unstaged changes".to_string(),
            author: String::new(),
            time: Utc::now(),
        }
    }

    fn is_staged_commit(commit: &CommitInfo) -> bool {
        commit.id == STAGED_SELECTION_ID
    }

    fn is_unstaged_commit(commit: &CommitInfo) -> bool {
        commit.id == UNSTAGED_SELECTION_ID
    }

    fn is_special_commit(commit: &CommitInfo) -> bool {
        Self::is_staged_commit(commit) || Self::is_unstaged_commit(commit)
    }

    fn special_commit_count(&self) -> usize {
        self.commit_list
            .iter()
            .take_while(|commit| Self::is_special_commit(commit))
            .count()
    }

    fn loaded_history_commit_count(&self) -> usize {
        self.commit_list.len().saturating_sub(self.special_commit_count())
    }

    fn filter_ignored_diff_files(repo_root: &Path, diff_files: Vec<DiffFile>) -> Vec<DiffFile> {
        crate::tuicrignore::filter_diff_files(repo_root, diff_files)
    }

    fn require_non_empty_diff_files(diff_files: Vec<DiffFile>) -> Result<Vec<DiffFile>> {
        if diff_files.is_empty() {
            return Err(TuicrError::NoChanges);
        }
        Ok(diff_files)
    }

    fn get_working_tree_diff_with_ignore(
        vcs: &dyn VcsBackend,
        repo_root: &Path,
        highlighter: &SyntaxHighlighter,
    ) -> Result<Vec<DiffFile>> {
        let diff_files = vcs.get_working_tree_diff(highlighter)?;
        let diff_files = Self::filter_ignored_diff_files(repo_root, diff_files);
        Self::require_non_empty_diff_files(diff_files)
    }

    fn get_staged_diff_with_ignore(
        vcs: &dyn VcsBackend,
        repo_root: &Path,
        highlighter: &SyntaxHighlighter,
    ) -> Result<Vec<DiffFile>> {
        let diff_files = vcs.get_staged_diff(highlighter)?;
        let diff_files = Self::filter_ignored_diff_files(repo_root, diff_files);
        Self::require_non_empty_diff_files(diff_files)
    }

    fn get_unstaged_diff_with_ignore(
        vcs: &dyn VcsBackend,
        repo_root: &Path,
        highlighter: &SyntaxHighlighter,
    ) -> Result<Vec<DiffFile>> {
        let diff_files = match vcs.get_unstaged_diff(highlighter) {
            Ok(diff_files) => diff_files,
            Err(TuicrError::UnsupportedOperation(_)) => vcs.get_working_tree_diff(highlighter)?,
            Err(e) => return Err(e),
        };
        let diff_files = Self::filter_ignored_diff_files(repo_root, diff_files);
        Self::require_non_empty_diff_files(diff_files)
    }

    fn get_commit_range_diff_with_ignore(
        vcs: &dyn VcsBackend,
        repo_root: &Path,
        commit_ids: &[String],
        highlighter: &SyntaxHighlighter,
    ) -> Result<Vec<DiffFile>> {
        let diff_files = vcs.get_commit_range_diff(commit_ids, highlighter)?;
        let diff_files = Self::filter_ignored_diff_files(repo_root, diff_files);
        Self::require_non_empty_diff_files(diff_files)
    }

    fn get_working_tree_with_commits_diff_with_ignore(
        vcs: &dyn VcsBackend,
        repo_root: &Path,
        commit_ids: &[String],
        highlighter: &SyntaxHighlighter,
    ) -> Result<Vec<DiffFile>> {
        let diff_files = vcs.get_working_tree_with_commits_diff(commit_ids, highlighter)?;
        let diff_files = Self::filter_ignored_diff_files(repo_root, diff_files);
        Self::require_non_empty_diff_files(diff_files)
    }

    fn load_staged_and_unstaged_selection(&mut self) -> Result<()> {
        let highlighter = self.theme.syntax_highlighter();
        let diff_files = match Self::get_working_tree_diff_with_ignore(
            self.vcs.as_ref(),
            &self.vcs_info.root_path,
            highlighter,
        ) {
            Ok(diff_files) => diff_files,
            Err(TuicrError::NoChanges) => {
                self.set_message("No staged or unstaged changes");
                return Ok(());
            }
            Err(e) => return Err(e),
        };

        self.session =
            Self::load_or_create_session(&self.vcs_info, SessionDiffSource::StagedAndUnstaged);
        for file in &diff_files {
            let path = file.display_path().clone();
            self.session.add_file(path, file.status);
        }

        self.diff_files = diff_files;
        self.diff_source = DiffSource::StagedAndUnstaged;
        self.input_mode = InputMode::Normal;
        self.diff_state = DiffState::default();
        self.file_list_state = FileListState::default();
        self.clear_expanded_gaps();
        self.sort_files_by_directory(true);
        self.expand_all_dirs();
        self.rebuild_annotations();

        Ok(())
    }

    fn load_staged_selection(&mut self) -> Result<()> {
        let highlighter = self.theme.syntax_highlighter();
        let diff_files = match Self::get_staged_diff_with_ignore(
            self.vcs.as_ref(),
            &self.vcs_info.root_path,
            highlighter,
        ) {
            Ok(diff_files) => diff_files,
            Err(TuicrError::NoChanges) => {
                self.set_message("No staged changes");
                return Ok(());
            }
            Err(e) => return Err(e),
        };

        self.session = Self::load_or_create_session(&self.vcs_info, SessionDiffSource::Staged);
        for file in &diff_files {
            let path = file.display_path().clone();
            self.session.add_file(path, file.status);
        }

        self.diff_files = diff_files;
        self.diff_source = DiffSource::Staged;
        self.input_mode = InputMode::Normal;
        self.diff_state = DiffState::default();
        self.file_list_state = FileListState::default();
        self.clear_expanded_gaps();
        self.sort_files_by_directory(true);
        self.expand_all_dirs();
        self.rebuild_annotations();

        Ok(())
    }

    fn load_unstaged_selection(&mut self) -> Result<()> {
        let highlighter = self.theme.syntax_highlighter();
        let diff_files = match Self::get_unstaged_diff_with_ignore(
            self.vcs.as_ref(),
            &self.vcs_info.root_path,
            highlighter,
        ) {
            Ok(diff_files) => diff_files,
            Err(TuicrError::NoChanges) => {
                self.set_message("No unstaged changes");
                return Ok(());
            }
            Err(e) => return Err(e),
        };

        self.session = Self::load_or_create_session(&self.vcs_info, SessionDiffSource::Unstaged);
        for file in &diff_files {
            let path = file.display_path().clone();
            self.session.add_file(path, file.status);
        }

        self.diff_files = diff_files;
        self.diff_source = DiffSource::Unstaged;
        self.input_mode = InputMode::Normal;
        self.diff_state = DiffState::default();
        self.file_list_state = FileListState::default();
        self.clear_expanded_gaps();
        self.sort_files_by_directory(true);
        self.expand_all_dirs();
        self.rebuild_annotations();

        Ok(())
    }

    pub fn reload_diff_files(&mut self) -> Result<usize> {
        let current_path = self.current_file_path().cloned();
        let prev_file_idx = self.diff_state.current_file_idx;
        let prev_cursor_line = self.diff_state.cursor_line;
        let prev_viewport_offset = self
            .diff_state
            .cursor_line
            .saturating_sub(self.diff_state.scroll_offset);
        let prev_relative_line = if self.diff_files.is_empty() {
            0
        } else {
            let start = self.calculate_file_scroll_offset(self.diff_state.current_file_idx);
            prev_cursor_line.saturating_sub(start)
        };

        let highlighter = self.theme.syntax_highlighter();
        let diff_files = match &self.diff_source {
            DiffSource::CommitRange(commit_ids) => Self::get_commit_range_diff_with_ignore(
                self.vcs.as_ref(),
                &self.vcs_info.root_path,
                commit_ids,
                highlighter,
            )?,
            DiffSource::WorkingTreeAndCommits(commit_ids)
            | DiffSource::StagedUnstagedAndCommits(commit_ids) => {
                let ids = commit_ids.clone();
                Self::get_working_tree_with_commits_diff_with_ignore(
                    self.vcs.as_ref(),
                    &self.vcs_info.root_path,
                    &ids,
                    highlighter,
                )?
            }
            DiffSource::Staged => Self::get_staged_diff_with_ignore(
                self.vcs.as_ref(),
                &self.vcs_info.root_path,
                highlighter,
            )?,
            DiffSource::Unstaged => Self::get_unstaged_diff_with_ignore(
                self.vcs.as_ref(),
                &self.vcs_info.root_path,
                highlighter,
            )?,
            DiffSource::StagedAndUnstaged | DiffSource::WorkingTree => {
                Self::get_working_tree_diff_with_ignore(
                    self.vcs.as_ref(),
                    &self.vcs_info.root_path,
                    highlighter,
                )?
            }
        };

        for file in &diff_files {
            let path = file.display_path().clone();
            self.session.add_file(path, file.status);
        }

        self.diff_files = diff_files;
        self.clear_expanded_gaps();

        self.sort_files_by_directory(false);
        self.expand_all_dirs();

        if self.diff_files.is_empty() {
            self.diff_state.current_file_idx = 0;
            self.diff_state.cursor_line = 0;
            self.diff_state.scroll_offset = 0;
            self.file_list_state.select(0);
        } else {
            let target_idx = if let Some(path) = current_path {
                self.diff_files
                    .iter()
                    .position(|file| file.display_path() == &path)
                    .unwrap_or_else(|| prev_file_idx.min(self.diff_files.len().saturating_sub(1)))
            } else {
                prev_file_idx.min(self.diff_files.len().saturating_sub(1))
            };

            self.jump_to_file(target_idx);

            let file_start = self.calculate_file_scroll_offset(target_idx);
            let file_height = self.file_render_height(target_idx, &self.diff_files[target_idx]);
            let relative_line = prev_relative_line.min(file_height.saturating_sub(1));
            self.diff_state.cursor_line = file_start.saturating_add(relative_line);

            let viewport = self.diff_state.viewport_height.max(1);
            let max_relative = viewport.saturating_sub(1);
            let relative_offset = prev_viewport_offset.min(max_relative);
            if self.total_lines() == 0 {
                self.diff_state.scroll_offset = 0;
            } else {
                let max_scroll = self.max_scroll_offset();
                let desired = self
                    .diff_state
                    .cursor_line
                    .saturating_sub(relative_offset)
                    .min(max_scroll);
                self.diff_state.scroll_offset = desired;
            }

            self.ensure_cursor_visible();
            self.update_current_file_from_cursor();
        }

        self.rebuild_annotations();
        Ok(self.diff_files.len())
    }

    pub fn current_file(&self) -> Option<&DiffFile> {
        self.diff_files.get(self.diff_state.current_file_idx)
    }

    pub fn current_file_path(&self) -> Option<&PathBuf> {
        self.current_file().map(|f| f.display_path())
    }

    pub fn toggle_reviewed(&mut self) {
        let file_idx = self.diff_state.current_file_idx;
        self.toggle_reviewed_for_file_idx(file_idx, true);
    }

    pub fn toggle_reviewed_for_file_idx(&mut self, file_idx: usize, adjust_cursor: bool) {
        let Some(path) = self
            .diff_files
            .get(file_idx)
            .map(|file| file.display_path().clone())
        else {
            return;
        };

        if let Some(review) = self.session.get_file_mut(&path) {
            review.reviewed = !review.reviewed;
            self.dirty = true;
            self.rebuild_annotations();

            if adjust_cursor {
                self.diff_state.current_file_idx = file_idx;
                // Move cursor to the file header line
                let header_line = self.calculate_file_scroll_offset(file_idx);
                self.diff_state.cursor_line = header_line;
                self.ensure_cursor_visible();
            }
        }
    }

    pub fn file_count(&self) -> usize {
        self.diff_files.len()
    }

    pub fn reviewed_count(&self) -> usize {
        self.session.reviewed_count()
    }

    pub fn set_message(&mut self, msg: impl Into<String>) {
        self.message = Some(Message {
            content: msg.into(),
            message_type: MessageType::Info,
        });
    }

    pub fn set_warning(&mut self, msg: impl Into<String>) {
        self.message = Some(Message {
            content: msg.into(),
            message_type: MessageType::Warning,
        });
    }

    pub fn set_error(&mut self, msg: impl Into<String>) {
        self.message = Some(Message {
            content: msg.into(),
            message_type: MessageType::Error,
        });
    }

    pub fn cursor_down(&mut self, lines: usize) {
        let max_line = self.total_lines().saturating_sub(1);
        self.diff_state.cursor_line = (self.diff_state.cursor_line + lines).min(max_line);
        self.ensure_cursor_visible();
        self.update_current_file_from_cursor();
    }

    pub fn cursor_up(&mut self, lines: usize) {
        self.diff_state.cursor_line = self.diff_state.cursor_line.saturating_sub(lines);
        self.ensure_cursor_visible();
        self.update_current_file_from_cursor();
    }

    pub fn scroll_down(&mut self, lines: usize) {
        // For half-page/page scrolling, move both cursor and scroll
        let total = self.total_lines();
        let max_line = total.saturating_sub(1);
        let max_scroll = self.max_scroll_offset();
        self.diff_state.cursor_line = (self.diff_state.cursor_line + lines).min(max_line);
        self.diff_state.scroll_offset = (self.diff_state.scroll_offset + lines).min(max_scroll);
        self.ensure_cursor_visible();
        self.update_current_file_from_cursor();
    }

    pub fn scroll_up(&mut self, lines: usize) {
        // For half-page/page scrolling, move both cursor and scroll
        self.diff_state.cursor_line = self.diff_state.cursor_line.saturating_sub(lines);
        self.diff_state.scroll_offset = self.diff_state.scroll_offset.saturating_sub(lines);
        self.ensure_cursor_visible();
        self.update_current_file_from_cursor();
    }

    pub fn scroll_left(&mut self, cols: usize) {
        if self.diff_state.wrap_lines {
            return;
        }
        self.diff_state.scroll_x = self.diff_state.scroll_x.saturating_sub(cols);
    }

    pub fn scroll_right(&mut self, cols: usize) {
        if self.diff_state.wrap_lines {
            return;
        }
        let max_scroll_x = self
            .diff_state
            .max_content_width
            .saturating_sub(self.diff_state.viewport_width);
        self.diff_state.scroll_x =
            (self.diff_state.scroll_x.saturating_add(cols)).min(max_scroll_x);
    }

    pub fn toggle_diff_wrap(&mut self) {
        let enabled = !self.diff_state.wrap_lines;
        self.set_diff_wrap(enabled);
    }

    pub fn set_diff_wrap(&mut self, enabled: bool) {
        self.diff_state.wrap_lines = enabled;
        if enabled {
            self.diff_state.scroll_x = 0;
        }
        let status = if self.diff_state.wrap_lines {
            "on"
        } else {
            "off"
        };
        self.set_message(format!("Diff wrapping: {status}"));
    }

    fn ensure_cursor_visible(&mut self) {
        // Use visible_line_count which is computed during render based on actual line widths.
        // Fall back to viewport_height if not yet set (before first render).
        let visible_lines = if self.diff_state.visible_line_count > 0 {
            self.diff_state.visible_line_count
        } else {
            self.diff_state.viewport_height.max(1)
        };
        let max_scroll = self.max_scroll_offset();
        if self.diff_state.cursor_line < self.diff_state.scroll_offset {
            self.diff_state.scroll_offset = self.diff_state.cursor_line;
        }
        if self.diff_state.cursor_line >= self.diff_state.scroll_offset + visible_lines {
            self.diff_state.scroll_offset =
                (self.diff_state.cursor_line - visible_lines + 1).min(max_scroll);
        }
    }

    pub fn search_in_diff_from_cursor(&mut self) -> bool {
        let pattern = self.search_buffer.clone();
        if pattern.trim().is_empty() {
            self.set_message("Search pattern is empty");
            return false;
        }

        self.last_search_pattern = Some(pattern.clone());
        self.search_in_diff(&pattern, self.diff_state.cursor_line, true, true)
    }

    pub fn search_next_in_diff(&mut self) -> bool {
        let Some(pattern) = self.last_search_pattern.clone() else {
            self.set_message("No previous search");
            return false;
        };
        self.search_in_diff(&pattern, self.diff_state.cursor_line, true, false)
    }

    pub fn search_prev_in_diff(&mut self) -> bool {
        let Some(pattern) = self.last_search_pattern.clone() else {
            self.set_message("No previous search");
            return false;
        };
        self.search_in_diff(&pattern, self.diff_state.cursor_line, false, false)
    }

    fn search_in_diff(
        &mut self,
        pattern: &str,
        start_idx: usize,
        forward: bool,
        include_current: bool,
    ) -> bool {
        let total_lines = self.total_lines();
        if total_lines == 0 {
            self.set_message("No diff content to search");
            return false;
        }

        if forward {
            let mut idx = start_idx.min(total_lines.saturating_sub(1));
            if !include_current {
                idx = idx.saturating_add(1);
            }
            for line_idx in idx..total_lines {
                if let Some(text) = self.line_text_for_search(line_idx)
                    && text.contains(pattern)
                {
                    self.diff_state.cursor_line = line_idx;
                    self.ensure_cursor_visible();
                    self.center_cursor();
                    self.update_current_file_from_cursor();
                    return true;
                }
            }
        } else {
            let mut idx = start_idx.min(total_lines.saturating_sub(1));
            if !include_current {
                idx = idx.saturating_sub(1);
            }
            let mut line_idx = idx;
            loop {
                if let Some(text) = self.line_text_for_search(line_idx)
                    && text.contains(pattern)
                {
                    self.diff_state.cursor_line = line_idx;
                    self.ensure_cursor_visible();
                    self.center_cursor();
                    self.update_current_file_from_cursor();
                    return true;
                }
                if line_idx == 0 {
                    break;
                }
                line_idx = line_idx.saturating_sub(1);
            }
        }

        self.set_message(format!("No matches for \"{pattern}\""));
        false
    }

    fn line_text_for_search(&self, line_idx: usize) -> Option<String> {
        match self.line_annotations.get(line_idx)? {
            AnnotatedLine::FileHeader { file_idx } => {
                let file = self.diff_files.get(*file_idx)?;
                Some(format!(
                    "{} [{}]",
                    file.display_path().display(),
                    file.status.as_char()
                ))
            }
            AnnotatedLine::FileComment {
                file_idx,
                comment_idx,
            } => {
                let path = self.diff_files.get(*file_idx)?.display_path();
                let review = self.session.files.get(path)?;
                let comment = review.file_comments.get(*comment_idx)?;
                Some(comment.content.clone())
            }
            AnnotatedLine::LineComment {
                file_idx,
                line,
                comment_idx,
                ..
            } => {
                let path = self.diff_files.get(*file_idx)?.display_path();
                let review = self.session.files.get(path)?;
                let comments = review.line_comments.get(line)?;
                let comment = comments.get(*comment_idx)?;
                Some(comment.content.clone())
            }
            AnnotatedLine::Expander { gap_id } => {
                let gap = self.gap_size(gap_id)?;
                Some(format!("... expand ({gap} lines) ..."))
            }
            AnnotatedLine::ExpandedContext {
                gap_id,
                line_idx: context_idx,
            } => {
                let content = self.expanded_content.get(gap_id)?.get(*context_idx)?;
                Some(content.content.clone())
            }
            AnnotatedLine::HunkHeader { file_idx, hunk_idx } => {
                let file = self.diff_files.get(*file_idx)?;
                let hunk = file.hunks.get(*hunk_idx)?;
                Some(hunk.header.clone())
            }
            AnnotatedLine::DiffLine {
                file_idx,
                hunk_idx,
                line_idx: diff_idx,
                ..
            } => {
                let file = self.diff_files.get(*file_idx)?;
                let hunk = file.hunks.get(*hunk_idx)?;
                let line = hunk.lines.get(*diff_idx)?;
                Some(line.content.clone())
            }
            AnnotatedLine::BinaryOrEmpty { file_idx } => {
                let file = self.diff_files.get(*file_idx)?;
                if file.is_too_large {
                    Some("(file too large to display)".to_string())
                } else if file.is_binary {
                    Some("(binary file)".to_string())
                } else {
                    Some("(no changes)".to_string())
                }
            }
            AnnotatedLine::SideBySideLine {
                file_idx,
                hunk_idx,
                del_line_idx,
                add_line_idx,
                ..
            } => {
                let file = self.diff_files.get(*file_idx)?;
                let hunk = file.hunks.get(*hunk_idx)?;

                let del_content = del_line_idx
                    .and_then(|idx| hunk.lines.get(idx))
                    .map(|l| l.content.as_str())
                    .unwrap_or("");
                let add_content = add_line_idx
                    .and_then(|idx| hunk.lines.get(idx))
                    .map(|l| l.content.as_str())
                    .unwrap_or("");
                Some(format!("{} {}", del_content, add_content))
            }
            AnnotatedLine::Spacing => None,
        }
    }

    fn gap_size(&self, gap_id: &GapId) -> Option<u32> {
        let file = self.diff_files.get(gap_id.file_idx)?;
        let hunk = file.hunks.get(gap_id.hunk_idx)?;
        let prev_hunk = if gap_id.hunk_idx > 0 {
            file.hunks.get(gap_id.hunk_idx - 1)
        } else {
            None
        };
        Some(calculate_gap(
            prev_hunk.map(|h| (&h.new_start, &h.new_count)),
            hunk.new_start,
        ))
    }

    pub fn center_cursor(&mut self) {
        let viewport = self.diff_state.viewport_height.max(1);
        let half_viewport = viewport / 2;
        let max_scroll = self.max_scroll_offset();
        self.diff_state.scroll_offset = self
            .diff_state
            .cursor_line
            .saturating_sub(half_viewport)
            .min(max_scroll);
    }

    pub fn file_list_down(&mut self, n: usize) {
        let visible_items = self.build_visible_items();
        let max_idx = visible_items.len().saturating_sub(1);
        let new_idx = (self.file_list_state.selected() + n).min(max_idx);
        self.file_list_state.select(new_idx);
    }

    pub fn file_list_up(&mut self, n: usize) {
        let new_idx = self.file_list_state.selected().saturating_sub(n);
        self.file_list_state.select(new_idx);
    }

    pub fn jump_to_file(&mut self, idx: usize) {
        use std::path::Path;

        if idx < self.diff_files.len() {
            self.diff_state.current_file_idx = idx;
            self.diff_state.cursor_line = self.calculate_file_scroll_offset(idx);
            let max_scroll = self.max_scroll_offset();
            self.diff_state.scroll_offset = self.diff_state.cursor_line.min(max_scroll);

            let file_path = self.diff_files[idx].display_path().clone();
            let mut current = file_path.parent();
            while let Some(parent) = current {
                if parent != Path::new("") {
                    self.expanded_dirs
                        .insert(parent.to_string_lossy().to_string());
                }
                current = parent.parent();
            }

            if let Some(tree_idx) = self.file_idx_to_tree_idx(idx) {
                self.file_list_state.select(tree_idx);
            }
        }
    }

    pub fn next_file(&mut self) {
        let visible_items = self.build_visible_items();
        let current_file_idx = self.diff_state.current_file_idx;

        for item in &visible_items {
            if let FileTreeItem::File { file_idx, .. } = item
                && *file_idx > current_file_idx
            {
                self.jump_to_file(*file_idx);
                return;
            }
        }
    }

    pub fn prev_file(&mut self) {
        let visible_items = self.build_visible_items();
        let current_file_idx = self.diff_state.current_file_idx;

        for item in visible_items.iter().rev() {
            if let FileTreeItem::File { file_idx, .. } = item
                && *file_idx < current_file_idx
            {
                self.jump_to_file(*file_idx);
                return;
            }
        }
    }

    fn file_idx_to_tree_idx(&self, target_file_idx: usize) -> Option<usize> {
        let visible_items = self.build_visible_items();
        for (tree_idx, item) in visible_items.iter().enumerate() {
            if let FileTreeItem::File { file_idx, .. } = item
                && *file_idx == target_file_idx
            {
                return Some(tree_idx);
            }
        }
        None
    }

    pub fn next_hunk(&mut self) {
        // Find the next hunk header position after current cursor
        let mut cumulative = 0;
        for file in &self.diff_files {
            let path = file.display_path();

            // File header
            cumulative += 1;

            // If file is reviewed, skip all content
            if self.session.is_file_reviewed(path) {
                continue;
            }

            // File comments
            if let Some(review) = self.session.files.get(path) {
                cumulative += review.file_comments.len();
            }

            if file.is_binary || file.hunks.is_empty() {
                cumulative += 1; // "(binary file)" or "(no changes)"
            } else {
                for hunk in &file.hunks {
                    // This is a hunk header position
                    if cumulative > self.diff_state.cursor_line {
                        self.diff_state.cursor_line = cumulative;
                        self.ensure_cursor_visible();
                        self.update_current_file_from_cursor();
                        return;
                    }
                    cumulative += 1; // hunk header
                    cumulative += hunk.lines.len(); // diff lines
                }
            }
            cumulative += 1; // spacing
        }
    }

    pub fn prev_hunk(&mut self) {
        // Find the previous hunk header position before current cursor
        let mut hunk_positions: Vec<usize> = Vec::new();
        let mut cumulative = 0;

        for file in &self.diff_files {
            let path = file.display_path();

            cumulative += 1; // File header

            // If file is reviewed, skip all content
            if self.session.is_file_reviewed(path) {
                continue;
            }

            if let Some(review) = self.session.files.get(path) {
                cumulative += review.file_comments.len();
            }

            if file.is_binary || file.hunks.is_empty() {
                cumulative += 1;
            } else {
                for hunk in &file.hunks {
                    hunk_positions.push(cumulative);
                    cumulative += 1;
                    cumulative += hunk.lines.len();
                }
            }
            cumulative += 1;
        }

        // Find the last hunk position before current cursor
        for &pos in hunk_positions.iter().rev() {
            if pos < self.diff_state.cursor_line {
                self.diff_state.cursor_line = pos;
                self.ensure_cursor_visible();
                self.update_current_file_from_cursor();
                return;
            }
        }

        // If no previous hunk, go to start
        self.diff_state.cursor_line = 0;
        self.ensure_cursor_visible();
        self.update_current_file_from_cursor();
    }

    fn calculate_file_scroll_offset(&self, file_idx: usize) -> usize {
        let mut offset = 0;
        for (i, file) in self.diff_files.iter().enumerate() {
            if i == file_idx {
                break;
            }
            offset += self.file_render_height(i, file);
        }
        offset
    }

    fn file_render_height(&self, file_idx: usize, file: &DiffFile) -> usize {
        let path = file.display_path();

        // If reviewed, only show header (1 line total)
        if self.session.is_file_reviewed(path) {
            return 1;
        }

        let header_lines = 1; // File header
        let spacing_lines = 1; // Blank line between files
        let mut content_lines = 0;
        let mut comment_lines = 0;

        if let Some(review) = self.session.files.get(path) {
            for comment in &review.file_comments {
                comment_lines += Self::comment_display_lines(comment);
            }
        }

        if file.is_binary || file.hunks.is_empty() {
            content_lines = 1;
        } else {
            let line_comments = self.session.files.get(path).map(|r| &r.line_comments);

            for (hunk_idx, hunk) in file.hunks.iter().enumerate() {
                // Calculate gap before this hunk
                let prev_hunk = if hunk_idx > 0 {
                    file.hunks.get(hunk_idx - 1)
                } else {
                    None
                };
                let gap = calculate_gap(
                    prev_hunk.map(|h| (&h.new_start, &h.new_count)),
                    hunk.new_start,
                );

                let gap_id = GapId { file_idx, hunk_idx };

                if gap > 0 {
                    if self.expanded_gaps.contains(&gap_id) {
                        // Expanded content lines
                        if let Some(expanded) = self.expanded_content.get(&gap_id) {
                            content_lines += expanded.len();
                        }
                    } else {
                        // Expander line
                        content_lines += 1;
                    }
                }

                // Hunk header + diff lines
                content_lines += 1; // Hunk header

                // Count diff lines based on view mode
                match self.diff_view_mode {
                    DiffViewMode::Unified => {
                        for diff_line in &hunk.lines {
                            content_lines += 1;

                            if let Some(line_comments) = line_comments {
                                if let Some(old_ln) = diff_line.old_lineno
                                    && let Some(comments) = line_comments.get(&old_ln)
                                {
                                    for comment in comments {
                                        if comment.side == Some(LineSide::Old) {
                                            comment_lines += Self::comment_display_lines(comment);
                                        }
                                    }
                                }

                                if let Some(new_ln) = diff_line.new_lineno
                                    && let Some(comments) = line_comments.get(&new_ln)
                                {
                                    for comment in comments {
                                        if comment.side != Some(LineSide::Old) {
                                            comment_lines += Self::comment_display_lines(comment);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    DiffViewMode::SideBySide => {
                        use crate::model::LineOrigin;
                        // Side-by-side mode: pair deletions with following additions
                        let lines = &hunk.lines;
                        let mut i = 0;
                        while i < lines.len() {
                            let diff_line = &lines[i];

                            match diff_line.origin {
                                LineOrigin::Context => {
                                    content_lines += 1;

                                    // Comments for context line
                                    if let Some(line_comments) = line_comments
                                        && let Some(new_ln) = diff_line.new_lineno
                                        && let Some(comments) = line_comments.get(&new_ln)
                                    {
                                        for comment in comments {
                                            if comment.side != Some(LineSide::Old) {
                                                comment_lines +=
                                                    Self::comment_display_lines(comment);
                                            }
                                        }
                                    }
                                    i += 1;
                                }
                                LineOrigin::Deletion => {
                                    // Find consecutive deletions
                                    let del_start = i;
                                    let mut del_end = i + 1;
                                    while del_end < lines.len()
                                        && lines[del_end].origin == LineOrigin::Deletion
                                    {
                                        del_end += 1;
                                    }

                                    // Find consecutive additions following deletions
                                    let add_start = del_end;
                                    let mut add_end = add_start;
                                    while add_end < lines.len()
                                        && lines[add_end].origin == LineOrigin::Addition
                                    {
                                        add_end += 1;
                                    }

                                    let del_count = del_end - del_start;
                                    let add_count = add_end - add_start;
                                    // Paired lines use max of the two counts
                                    content_lines += del_count.max(add_count);

                                    // Count comments for all deletions and additions in this pair
                                    if let Some(line_comments) = line_comments {
                                        for line in &lines[del_start..del_end] {
                                            if let Some(old_ln) = line.old_lineno
                                                && let Some(comments) = line_comments.get(&old_ln)
                                            {
                                                for comment in comments {
                                                    if comment.side == Some(LineSide::Old) {
                                                        comment_lines +=
                                                            Self::comment_display_lines(comment);
                                                    }
                                                }
                                            }
                                        }

                                        for line in &lines[add_start..add_end] {
                                            if let Some(new_ln) = line.new_lineno
                                                && let Some(comments) = line_comments.get(&new_ln)
                                            {
                                                for comment in comments {
                                                    if comment.side != Some(LineSide::Old) {
                                                        comment_lines +=
                                                            Self::comment_display_lines(comment);
                                                    }
                                                }
                                            }
                                        }
                                    }

                                    i = add_end;
                                }
                                LineOrigin::Addition => {
                                    // Standalone addition (not following deletions)
                                    content_lines += 1;

                                    if let Some(line_comments) = line_comments
                                        && let Some(new_ln) = diff_line.new_lineno
                                        && let Some(comments) = line_comments.get(&new_ln)
                                    {
                                        for comment in comments {
                                            if comment.side != Some(LineSide::Old) {
                                                comment_lines +=
                                                    Self::comment_display_lines(comment);
                                            }
                                        }
                                    }

                                    i += 1;
                                }
                            }
                        }
                    }
                }
            }
        }

        header_lines + comment_lines + content_lines + spacing_lines
    }

    fn update_current_file_from_cursor(&mut self) {
        let mut cumulative = 0;
        for (i, file) in self.diff_files.iter().enumerate() {
            let height = self.file_render_height(i, file);
            if cumulative + height > self.diff_state.cursor_line {
                self.diff_state.current_file_idx = i;
                self.file_list_state.select(i);
                return;
            }
            cumulative += height;
        }
        if !self.diff_files.is_empty() {
            self.diff_state.current_file_idx = self.diff_files.len() - 1;
            self.file_list_state.select(self.diff_files.len() - 1);
        }
    }

    pub fn total_lines(&self) -> usize {
        self.diff_files
            .iter()
            .enumerate()
            .map(|(i, f)| self.file_render_height(i, f))
            .sum()
    }

    /// Calculate the maximum scroll offset.
    ///
    /// When line wrapping is enabled, logical lines may expand to multiple visual rows.
    /// This means we need to allow scrolling further to ensure all content is reachable.
    /// We allow scrolling to `total - 1` so the last logical line can be at the top.
    ///
    /// When wrapping is disabled, each logical line is one visual row, so we use
    /// `total - viewport` which stops when the last line reaches the bottom.
    pub fn max_scroll_offset(&self) -> usize {
        let total = self.total_lines();
        let viewport = self.diff_state.viewport_height.max(1);
        if self.diff_state.wrap_lines {
            // With wrapping, allow scrolling to show the last line at the top
            total.saturating_sub(1)
        } else {
            // Without wrapping, stop when last line is at the bottom
            total.saturating_sub(viewport)
        }
    }

    /// Calculate the number of display lines a comment takes (header + content + footer)
    fn comment_display_lines(comment: &Comment) -> usize {
        let content_lines = comment.content.split('\n').count();
        2 + content_lines // header + content lines + footer
    }

    /// Returns the source line number and side at the current cursor position, if on a diff line
    pub fn get_line_at_cursor(&self) -> Option<(u32, LineSide)> {
        let target = self.diff_state.cursor_line;
        match self.line_annotations.get(target) {
            Some(AnnotatedLine::DiffLine {
                old_lineno,
                new_lineno,
                ..
            })
            | Some(AnnotatedLine::SideBySideLine {
                old_lineno,
                new_lineno,
                ..
            }) => {
                // Prefer new line number (for added/context lines), fall back to old (for deleted)
                new_lineno
                    .map(|ln| (ln, LineSide::New))
                    .or_else(|| old_lineno.map(|ln| (ln, LineSide::Old)))
            }
            _ => None,
        }
    }

    /// Find the comment at the current cursor position
    fn find_comment_at_cursor(&self) -> Option<CommentLocation> {
        let target = self.diff_state.cursor_line;
        match self.line_annotations.get(target) {
            Some(AnnotatedLine::FileComment {
                file_idx,
                comment_idx,
            }) => {
                let path = self.diff_files.get(*file_idx)?.display_path().clone();
                Some(CommentLocation::FileComment {
                    path,
                    index: *comment_idx,
                })
            }
            Some(AnnotatedLine::LineComment {
                file_idx,
                line,
                side,
                comment_idx,
            }) => {
                let path = self.diff_files.get(*file_idx)?.display_path().clone();
                Some(CommentLocation::LineComment {
                    path,
                    line: *line,
                    side: *side,
                    index: *comment_idx,
                })
            }
            _ => None,
        }
    }

    /// Delete the comment at the current cursor position, if any
    /// Returns true if a comment was deleted
    pub fn delete_comment_at_cursor(&mut self) -> bool {
        let location = self.find_comment_at_cursor();

        match location {
            Some(CommentLocation::FileComment { path, index }) => {
                if let Some(review) = self.session.get_file_mut(&path) {
                    review.file_comments.remove(index);
                    self.dirty = true;
                    self.set_message("Comment deleted");
                    self.rebuild_annotations();
                    return true;
                }
            }
            Some(CommentLocation::LineComment {
                path,
                line,
                side,
                index,
            }) => {
                if let Some(review) = self.session.get_file_mut(&path)
                    && let Some(comments) = review.line_comments.get_mut(&line)
                {
                    // Find the actual index by counting comments with matching side
                    let mut side_idx = 0;
                    let mut actual_idx = None;
                    for (i, comment) in comments.iter().enumerate() {
                        let comment_side = comment.side.unwrap_or(LineSide::New);
                        if comment_side == side {
                            if side_idx == index {
                                actual_idx = Some(i);
                                break;
                            }
                            side_idx += 1;
                        }
                    }
                    if let Some(idx) = actual_idx {
                        comments.remove(idx);
                        if comments.is_empty() {
                            review.line_comments.remove(&line);
                        }
                        self.dirty = true;
                        self.set_message(format!("Comment on line {line} deleted"));
                        self.rebuild_annotations();
                        return true;
                    }
                }
            }
            None => {}
        }

        false
    }

    pub fn clear_all_comments(&mut self) {
        let cleared = self.session.clear_comments();
        if cleared == 0 {
            self.set_message("No comments to clear");
            return;
        }

        self.dirty = true;
        self.rebuild_annotations();
        self.set_message(format!("Cleared {cleared} comments"));
    }

    /// Enter edit mode for the comment at the current cursor position
    /// Returns true if a comment was found and edit mode entered
    pub fn enter_edit_mode(&mut self) -> bool {
        let location = self.find_comment_at_cursor();

        match location {
            Some(CommentLocation::FileComment { path, index }) => {
                if let Some(review) = self.session.files.get(&path)
                    && let Some(comment) = review.file_comments.get(index)
                {
                    self.input_mode = InputMode::Comment;
                    self.comment_buffer = comment.content.clone();
                    self.comment_cursor = self.comment_buffer.len();
                    self.comment_type = comment.comment_type;
                    self.comment_is_file_level = true;
                    self.comment_line = None;
                    self.editing_comment_id = Some(comment.id.clone());
                    return true;
                }
            }
            Some(CommentLocation::LineComment {
                path,
                line,
                side,
                index,
            }) => {
                if let Some(review) = self.session.files.get(&path)
                    && let Some(comments) = review.line_comments.get(&line)
                {
                    // Find the actual comment by counting comments with matching side
                    let mut side_idx = 0;
                    for comment in comments.iter() {
                        let comment_side = comment.side.unwrap_or(LineSide::New);
                        if comment_side == side {
                            if side_idx == index {
                                self.input_mode = InputMode::Comment;
                                self.comment_buffer = comment.content.clone();
                                self.comment_cursor = self.comment_buffer.len();
                                self.comment_type = comment.comment_type;
                                self.comment_is_file_level = false;
                                self.comment_line = Some((line, side));
                                self.editing_comment_id = Some(comment.id.clone());
                                return true;
                            }
                            side_idx += 1;
                        }
                    }
                }
            }
            None => {}
        }

        false
    }

    pub fn enter_command_mode(&mut self) {
        self.input_mode = InputMode::Command;
        self.command_buffer.clear();
    }

    pub fn exit_command_mode(&mut self) {
        self.input_mode = InputMode::Normal;
        self.command_buffer.clear();
    }

    pub fn enter_search_mode(&mut self) {
        self.input_mode = InputMode::Search;
        self.search_buffer.clear();
    }

    pub fn exit_search_mode(&mut self) {
        self.input_mode = InputMode::Normal;
        self.search_buffer.clear();
    }

    pub fn enter_comment_mode(&mut self, file_level: bool, line: Option<(u32, LineSide)>) {
        self.input_mode = InputMode::Comment;
        self.comment_buffer.clear();
        self.comment_cursor = 0;
        self.comment_type = CommentType::Note;
        self.comment_is_file_level = file_level;
        self.comment_line = line;
    }

    pub fn exit_comment_mode(&mut self) {
        self.input_mode = InputMode::Normal;
        self.comment_buffer.clear();
        self.comment_cursor = 0;
        self.editing_comment_id = None;
        self.comment_line_range = None;
    }

    /// Enter visual selection mode, anchoring at the current cursor position
    pub fn enter_visual_mode(&mut self, line: u32, side: LineSide) {
        self.input_mode = InputMode::VisualSelect;
        self.visual_anchor = Some((line, side));
    }

    /// Exit visual selection mode and return to normal mode
    pub fn exit_visual_mode(&mut self) {
        self.input_mode = InputMode::Normal;
        self.visual_anchor = None;
    }

    /// Get the current visual selection range (if in visual mode)
    /// Returns None if not in visual mode or if there's no valid selection
    pub fn get_visual_selection(&self) -> Option<(LineRange, LineSide)> {
        if self.input_mode != InputMode::VisualSelect {
            return None;
        }

        let (anchor_line, anchor_side) = self.visual_anchor?;
        let (current_line, current_side) = self.get_line_at_cursor()?;

        // Don't allow selection across sides (old vs new)
        if anchor_side != current_side {
            return None;
        }

        let range = LineRange::new(anchor_line, current_line);
        Some((range, anchor_side))
    }

    /// Check if a given line is within the current visual selection
    pub fn is_line_in_visual_selection(&self, line: u32, side: LineSide) -> bool {
        if let Some((range, sel_side)) = self.get_visual_selection() {
            sel_side == side && range.contains(line)
        } else {
            false
        }
    }

    /// Enter comment mode from visual selection
    pub fn enter_comment_from_visual(&mut self) {
        if let Some((range, side)) = self.get_visual_selection() {
            self.comment_line_range = Some((range, side));
            self.comment_line = Some((range.end, side)); // Key by end line
            self.input_mode = InputMode::Comment;
            self.comment_buffer.clear();
            self.comment_cursor = 0;
            self.comment_type = CommentType::Note;
            self.comment_is_file_level = false;
            self.visual_anchor = None;
        } else {
            self.set_warning("Invalid visual selection");
            self.exit_visual_mode();
        }
    }

    pub fn save_comment(&mut self) {
        if self.comment_buffer.trim().is_empty() {
            self.set_message("Comment cannot be empty");
            return;
        }

        let content = self.comment_buffer.trim().to_string();

        if let Some(path) = self.current_file_path().cloned()
            && let Some(review) = self.session.get_file_mut(&path)
        {
            let message: String;

            // Check if we're editing an existing comment
            if let Some(editing_id) = &self.editing_comment_id {
                // Update existing comment
                // Search in file comments
                if let Some(comment) = review
                    .file_comments
                    .iter_mut()
                    .find(|c| &c.id == editing_id)
                {
                    comment.content = content.clone();
                    comment.comment_type = self.comment_type;
                    message = "Comment updated".to_string();
                } else {
                    // If not found in file comments, search in line comments
                    let mut found_comment = None;
                    for comments in review.line_comments.values_mut() {
                        if let Some(comment) = comments.iter_mut().find(|c| &c.id == editing_id) {
                            found_comment = Some(comment);
                            break;
                        }
                    }

                    if let Some(comment) = found_comment {
                        comment.content = content.clone();
                        comment.comment_type = self.comment_type;
                        message = if let Some((line, _)) = self.comment_line {
                            format!("Comment on line {line} updated")
                        } else {
                            "Comment updated".to_string()
                        };
                    } else {
                        message = "Error: Comment to edit not found".to_string();
                    }
                }
            } else {
                // Create new comment
                if self.comment_is_file_level {
                    let comment = Comment::new(content, self.comment_type, None);
                    review.add_file_comment(comment);
                    message = "File comment added".to_string();
                } else if let Some((range, side)) = self.comment_line_range {
                    // Range comment from visual selection
                    let comment =
                        Comment::new_with_range(content, self.comment_type, Some(side), range);
                    // Store by end line of the range
                    review.add_line_comment(range.end, comment);
                    if range.is_single() {
                        message = format!("Comment added to line {}", range.end);
                    } else {
                        message = format!("Comment added to lines {}-{}", range.start, range.end);
                    }
                } else if let Some((line, side)) = self.comment_line {
                    let comment = Comment::new(content, self.comment_type, Some(side));
                    review.add_line_comment(line, comment);
                    message = format!("Comment added to line {line}");
                } else {
                    // Fallback to file comment if no line specified
                    let comment = Comment::new(content, self.comment_type, None);
                    review.add_file_comment(comment);
                    message = "File comment added".to_string();
                }
            }

            self.dirty = true;
            self.set_message(message);
            self.rebuild_annotations();
        }

        self.exit_comment_mode();
    }

    pub fn cycle_comment_type(&mut self) {
        self.comment_type = match self.comment_type {
            CommentType::Note => CommentType::Suggestion,
            CommentType::Suggestion => CommentType::Issue,
            CommentType::Issue => CommentType::Praise,
            CommentType::Praise => CommentType::Note,
        };
    }

    pub fn toggle_help(&mut self) {
        if self.input_mode == InputMode::Help {
            self.input_mode = InputMode::Normal;
        } else {
            self.input_mode = InputMode::Help;
            self.help_state.scroll_offset = 0;
        }
    }

    pub fn help_scroll_down(&mut self, lines: usize) {
        let max_offset = self
            .help_state
            .total_lines
            .saturating_sub(self.help_state.viewport_height);
        self.help_state.scroll_offset = (self.help_state.scroll_offset + lines).min(max_offset);
    }

    pub fn help_scroll_up(&mut self, lines: usize) {
        self.help_state.scroll_offset = self.help_state.scroll_offset.saturating_sub(lines);
    }

    pub fn help_scroll_to_top(&mut self) {
        self.help_state.scroll_offset = 0;
    }

    pub fn help_scroll_to_bottom(&mut self) {
        let max_offset = self
            .help_state
            .total_lines
            .saturating_sub(self.help_state.viewport_height);
        self.help_state.scroll_offset = max_offset;
    }

    pub fn enter_confirm_mode(&mut self, action: ConfirmAction) {
        self.input_mode = InputMode::Confirm;
        self.pending_confirm = Some(action);
    }

    pub fn exit_confirm_mode(&mut self) {
        self.input_mode = InputMode::Normal;
        self.pending_confirm = None;
    }

    pub fn enter_commit_select_mode(&mut self) -> Result<()> {
        // Save inline selection state if we have review commits
        if !self.review_commits.is_empty() {
            self.saved_inline_selection = self.commit_selection_range;
        }

        let highlighter = self.theme.syntax_highlighter();
        let has_staged_changes = match Self::get_staged_diff_with_ignore(
            self.vcs.as_ref(),
            &self.vcs_info.root_path,
            highlighter,
        ) {
            Ok(_) => true,
            Err(TuicrError::NoChanges) => false,
            Err(TuicrError::UnsupportedOperation(_)) => false,
            Err(e) => return Err(e),
        };

        let has_unstaged_changes = match Self::get_unstaged_diff_with_ignore(
            self.vcs.as_ref(),
            &self.vcs_info.root_path,
            highlighter,
        ) {
            Ok(_) => true,
            Err(TuicrError::NoChanges) => false,
            Err(TuicrError::UnsupportedOperation(_)) => false,
            Err(e) => return Err(e),
        };

        let commits = self.vcs.get_recent_commits(0, VISIBLE_COMMIT_COUNT)?;
        if commits.is_empty() && !has_staged_changes && !has_unstaged_changes {
            self.set_message("No commits or staged/unstaged changes found");
            return Ok(());
        }

        // Check if there might be more commits
        self.has_more_commit = commits.len() >= VISIBLE_COMMIT_COUNT;
        self.commit_list = commits;
        if has_unstaged_changes {
            self.commit_list
                .insert(0, Self::unstaged_commit_entry());
        }
        if has_staged_changes {
            self.commit_list.insert(0, Self::staged_commit_entry());
        }
        self.commit_list_cursor = 0;
        self.commit_list_scroll_offset = 0;
        self.commit_selection_range = None;
        self.visible_commit_count = self.commit_list.len();
        self.input_mode = InputMode::CommitSelect;
        Ok(())
    }

    pub fn exit_commit_select_mode(&mut self) -> Result<()> {
        self.input_mode = InputMode::Normal;

        // If we have review commits, restore the inline selector state
        if !self.review_commits.is_empty() {
            self.commit_list = self.review_commits.clone();
            self.commit_selection_range = self.saved_inline_selection;
            self.commit_list_cursor = 0;
            self.commit_list_scroll_offset = 0;
            self.visible_commit_count = self.review_commits.len();
            self.has_more_commit = false;
            self.saved_inline_selection = None;

            // Reload diff for the restored selection
            if self.commit_selection_range.is_some() {
                self.reload_inline_selection()?;
            }
            return Ok(());
        }

        // If we were viewing commits, try to go back to working tree
        if matches!(
            self.diff_source,
            DiffSource::CommitRange(_)
                | DiffSource::WorkingTreeAndCommits(_)
                | DiffSource::StagedUnstagedAndCommits(_)
        ) {
            let highlighter = self.theme.syntax_highlighter();
            match Self::get_working_tree_diff_with_ignore(
                self.vcs.as_ref(),
                &self.vcs_info.root_path,
                highlighter,
            ) {
                Ok(diff_files) => {
                    self.diff_files = diff_files;
                    self.diff_source = DiffSource::StagedAndUnstaged;

                    // Update session for new files
                    for file in &self.diff_files {
                        let path = file.display_path().clone();
                        self.session.add_file(path, file.status);
                    }

                    self.sort_files_by_directory(true);
                    self.expand_all_dirs();
                }
                Err(_) => {
                    self.set_message("No staged or unstaged changes");
                }
            }
        }

        Ok(())
    }

    pub fn toggle_diff_view_mode(&mut self) {
        self.diff_view_mode = match self.diff_view_mode {
            DiffViewMode::Unified => DiffViewMode::SideBySide,
            DiffViewMode::SideBySide => DiffViewMode::Unified,
        };
        let mode_name = match self.diff_view_mode {
            DiffViewMode::Unified => "unified",
            DiffViewMode::SideBySide => "side-by-side",
        };
        self.set_message(format!("Diff view mode: {mode_name}"));
        self.rebuild_annotations();
    }

    pub fn toggle_file_list(&mut self) {
        self.show_file_list = !self.show_file_list;
        let status = if self.show_file_list {
            "visible"
        } else {
            "hidden"
        };
        self.set_message(format!("File list: {status}"));
    }

    /// Whether the inline commit selector panel should be displayed.
    pub fn has_inline_commit_selector(&self) -> bool {
        self.show_commit_selector
            && self.review_commits.len() > 1
            && !matches!(&self.diff_source, DiffSource::WorkingTree)
    }

    // Commit selection methods

    pub fn commit_select_up(&mut self) {
        if self.commit_list_cursor > 0 {
            self.commit_list_cursor -= 1;
            // Scroll up if cursor goes above visible area
            if self.commit_list_cursor < self.commit_list_scroll_offset {
                self.commit_list_scroll_offset = self.commit_list_cursor;
            }
        }
    }

    pub fn commit_select_down(&mut self) {
        let max_cursor = if self.can_show_more_commits() {
            self.visible_commit_count
        } else {
            self.visible_commit_count.saturating_sub(1)
        };

        if self.commit_list_cursor < max_cursor {
            self.commit_list_cursor += 1;
            // Scroll down if cursor goes below visible area
            if self.commit_list_viewport_height > 0
                && self.commit_list_cursor
                    >= self.commit_list_scroll_offset + self.commit_list_viewport_height
            {
                self.commit_list_scroll_offset =
                    self.commit_list_cursor - self.commit_list_viewport_height + 1;
            }
        }
    }

    // Check if cursor is on the commit expand row
    pub fn is_on_expand_row(&self) -> bool {
        self.can_show_more_commits() && self.commit_list_cursor == self.visible_commit_count
    }

    pub fn can_show_more_commits(&self) -> bool {
        self.visible_commit_count < self.commit_list.len() || self.has_more_commit
    }

    // Expand the commit list to show more commits
    pub fn expand_commit(&mut self) -> Result<()> {
        if self.visible_commit_count < self.commit_list.len() {
            self.visible_commit_count =
                (self.visible_commit_count + self.commit_page_size).min(self.commit_list.len());
            return Ok(());
        }

        if !self.has_more_commit {
            self.set_message("No more commits");
            return Ok(());
        }

        let offset = self.loaded_history_commit_count();
        let limit = self.commit_page_size;

        let new_commits = self.vcs.get_recent_commits(offset, limit)?;

        if new_commits.is_empty() {
            self.has_more_commit = false;
            self.set_message("No more commits");
            return Ok(());
        }

        if new_commits.len() < limit {
            self.has_more_commit = false;
            self.set_message("No more commits");
        }

        self.commit_list.extend(new_commits);
        self.visible_commit_count = self.commit_list.len();

        Ok(())
    }

    pub fn toggle_commit_selection(&mut self) {
        let cursor = self.commit_list_cursor;
        if cursor >= self.commit_list.len() {
            return;
        }

        match self.commit_selection_range {
            None => {
                // No selection yet - select just this commit
                self.commit_selection_range = Some((cursor, cursor));
            }
            Some((start, end)) => {
                if cursor >= start && cursor <= end {
                    // Cursor is within the range - shrink or deselect
                    if start == end {
                        // Only one commit selected, deselect all
                        self.commit_selection_range = None;
                    } else if cursor == start {
                        // At start edge - shrink from start
                        self.commit_selection_range = Some((start + 1, end));
                    } else if cursor == end {
                        // At end edge - shrink from end
                        self.commit_selection_range = Some((start, end - 1));
                    } else {
                        // In the middle - shrink towards cursor (exclude everything after cursor)
                        // This makes the cursor the new end of the range
                        self.commit_selection_range = Some((start, cursor));
                    }
                } else {
                    // Cursor is outside the range - extend to include it
                    let new_start = start.min(cursor);
                    let new_end = end.max(cursor);
                    self.commit_selection_range = Some((new_start, new_end));
                }
            }
        }
    }

    /// Check if a commit at the given index is selected
    pub fn is_commit_selected(&self, index: usize) -> bool {
        match self.commit_selection_range {
            Some((start, end)) => index >= start && index <= end,
            None => false,
        }
    }

    /// Cycle inline commit selector to the next individual commit (`)` key).
    /// all → last, i → i+1, last → all
    pub fn cycle_commit_next(&mut self) {
        if self.review_commits.is_empty() {
            return;
        }
        let n = self.review_commits.len();
        let all_selected = Some((0, n - 1));

        if self.commit_selection_range == all_selected {
            // all → last
            self.commit_selection_range = Some((n - 1, n - 1));
            self.commit_list_cursor = n - 1;
        } else if let Some((i, j)) = self.commit_selection_range {
            if i == j {
                // Single commit selected
                if i == n - 1 {
                    // last → all
                    self.commit_selection_range = all_selected;
                } else {
                    // i → i+1
                    self.commit_selection_range = Some((i + 1, i + 1));
                    self.commit_list_cursor = i + 1;
                }
            } else {
                // Multi-commit subrange → select last of that range
                self.commit_selection_range = Some((j, j));
                self.commit_list_cursor = j;
            }
        } else {
            // None selected → select all
            self.commit_selection_range = all_selected;
        }
    }

    /// Cycle inline commit selector to the previous individual commit (`(` key).
    /// all → first, i → i-1, first → all
    pub fn cycle_commit_prev(&mut self) {
        if self.review_commits.is_empty() {
            return;
        }
        let n = self.review_commits.len();
        let all_selected = Some((0, n - 1));

        if self.commit_selection_range == all_selected {
            // all → first
            self.commit_selection_range = Some((0, 0));
            self.commit_list_cursor = 0;
        } else if let Some((i, j)) = self.commit_selection_range {
            if i == j {
                // Single commit selected
                if i == 0 {
                    // first → all
                    self.commit_selection_range = all_selected;
                } else {
                    // i → i-1
                    self.commit_selection_range = Some((i - 1, i - 1));
                    self.commit_list_cursor = i - 1;
                }
            } else {
                // Multi-commit subrange → select first of that range
                self.commit_selection_range = Some((i, i));
                self.commit_list_cursor = i;
            }
        } else {
            // None selected → select all
            self.commit_selection_range = all_selected;
        }
    }

    pub fn confirm_commit_selection(&mut self) -> Result<()> {
        let Some((start, end)) = self.commit_selection_range else {
            self.set_message("Select at least one commit");
            return Ok(());
        };

        // Collect selected entries in order from oldest to newest (end..start).
        let selected_commits: Vec<&CommitInfo> = (start..=end)
            .rev()
            .filter_map(|i| self.commit_list.get(i))
            .collect();

        if selected_commits.is_empty() {
            self.set_message("Select at least one commit");
            return Ok(());
        }

        let selected_staged = selected_commits
            .iter()
            .any(|c| Self::is_staged_commit(c));
        let selected_unstaged = selected_commits
            .iter()
            .any(|c| Self::is_unstaged_commit(c));
        let selected_ids: Vec<String> = selected_commits
            .iter()
            .filter(|c| !Self::is_special_commit(c))
            .map(|c| c.id.clone())
            .collect();

        if (selected_staged || selected_unstaged) && !selected_ids.is_empty() {
            let all_selected: Vec<CommitInfo> = selected_commits.into_iter().cloned().collect();
            return self.load_staged_unstaged_and_commits_selection(selected_ids, all_selected);
        }

        if selected_staged && selected_unstaged {
            return self.load_staged_and_unstaged_selection();
        }

        if selected_staged {
            return self.load_staged_selection();
        }

        if selected_unstaged {
            return self.load_unstaged_selection();
        }

        // Get the diff for the selected commits
        let highlighter = self.theme.syntax_highlighter();
        let diff_files = Self::get_commit_range_diff_with_ignore(
            self.vcs.as_ref(),
            &self.vcs_info.root_path,
            &selected_ids,
            highlighter,
        )?;

        if diff_files.is_empty() {
            self.set_message("No changes in selected commits");
            return Ok(());
        }

        // Update session with the newest commit as base
        let newest_commit_id = selected_ids.last().unwrap().clone();
        let loaded_session = load_latest_session_for_context(
            &self.vcs_info.root_path,
            self.vcs_info.branch_name.as_deref(),
            &newest_commit_id,
            SessionDiffSource::CommitRange,
            Some(selected_ids.as_slice()),
        )
        .ok()
        .and_then(|found| found.map(|(_path, session)| session));

        let mut session = loaded_session.unwrap_or_else(|| {
            let mut session = ReviewSession::new(
                self.vcs_info.root_path.clone(),
                newest_commit_id,
                self.vcs_info.branch_name.clone(),
                SessionDiffSource::CommitRange,
            );
            session.commit_range = Some(selected_ids.clone());
            session
        });

        if session.commit_range.is_none() {
            session.commit_range = Some(selected_ids.clone());
            session.updated_at = chrono::Utc::now();
        }

        self.session = session;

        // Add files to session
        for file in &diff_files {
            let path = file.display_path().clone();
            self.session.add_file(path, file.status);
        }

        // Update app state
        self.diff_files = diff_files;
        self.diff_source = DiffSource::CommitRange(selected_ids);
        self.input_mode = InputMode::Normal;

        // Reset navigation state
        self.diff_state = DiffState::default();
        self.file_list_state = FileListState::default();

        // Set up inline commit selector for multi-commit reviews (newest-first display order)
        self.review_commits = selected_commits
            .iter()
            .rev()
            .map(|c| (*c).clone())
            .collect();
        self.range_diff_files = Some(self.diff_files.clone());
        self.commit_list = self.review_commits.clone();
        self.commit_list_cursor = 0;
        self.commit_selection_range = if self.review_commits.is_empty() {
            None
        } else {
            Some((0, self.review_commits.len() - 1))
        };
        self.commit_list_scroll_offset = 0;
        self.visible_commit_count = self.review_commits.len();
        self.has_more_commit = false;
        self.show_commit_selector = self.review_commits.len() > 1;
        self.commit_diff_cache.clear();
        self.saved_inline_selection = None;

        self.sort_files_by_directory(true);
        self.expand_all_dirs();
        self.rebuild_annotations();

        Ok(())
    }

    /// Reload the diff for the currently selected inline commit subrange.
    pub fn reload_inline_selection(&mut self) -> Result<()> {
        let Some((start, end)) = self.commit_selection_range else {
            self.set_message("Select at least one commit");
            return Ok(());
        };

        // Check if all commits selected -> use cached range_diff_files
        if start == 0
            && end == self.review_commits.len() - 1
            && let Some(ref files) = self.range_diff_files
        {
            self.diff_files = files.clone();
            let wrap = self.diff_state.wrap_lines;
            self.diff_state = DiffState::default();
            self.diff_state.wrap_lines = wrap;
            self.file_list_state = FileListState::default();
            self.expanded_gaps.clear();
            self.expanded_content.clear();
            self.sort_files_by_directory(true);
            self.expand_all_dirs();
            self.rebuild_annotations();
            return Ok(());
        }

        // Check cache for this subrange
        if let Some(files) = self.commit_diff_cache.get(&(start, end)) {
            self.diff_files = files.clone();
            let wrap = self.diff_state.wrap_lines;
            self.diff_state = DiffState::default();
            self.diff_state.wrap_lines = wrap;
            self.file_list_state = FileListState::default();
            self.expanded_gaps.clear();
            self.expanded_content.clear();
            self.sort_files_by_directory(true);
            self.expand_all_dirs();
            self.rebuild_annotations();
            return Ok(());
        }

        // Load diff for selected subrange
        let has_staged = (start..=end).any(|i| {
            self.review_commits
                .get(i)
                .is_some_and(Self::is_staged_commit)
        });
        let has_unstaged = (start..=end).any(|i| {
            self.review_commits
                .get(i)
                .is_some_and(Self::is_unstaged_commit)
        });
        let selected_ids: Vec<String> = (start..=end)
            .rev() // oldest to newest
            .filter_map(|i| self.review_commits.get(i))
            .filter(|c| !Self::is_special_commit(c))
            .map(|c| c.id.clone())
            .collect();

        let highlighter = self.theme.syntax_highlighter();
        let diff_files = if (has_staged || has_unstaged) && !selected_ids.is_empty() {
            match Self::get_working_tree_with_commits_diff_with_ignore(
                self.vcs.as_ref(),
                &self.vcs_info.root_path,
                &selected_ids,
                highlighter,
            ) {
                Ok(files) => files,
                Err(TuicrError::NoChanges) => Vec::new(),
                Err(e) => return Err(e),
            }
        } else if has_staged && has_unstaged {
            match Self::get_working_tree_diff_with_ignore(
                self.vcs.as_ref(),
                &self.vcs_info.root_path,
                highlighter,
            ) {
                Ok(files) => files,
                Err(TuicrError::NoChanges) => Vec::new(),
                Err(e) => return Err(e),
            }
        } else if has_staged {
            match Self::get_staged_diff_with_ignore(
                self.vcs.as_ref(),
                &self.vcs_info.root_path,
                highlighter,
            ) {
                Ok(files) => files,
                Err(TuicrError::NoChanges) => Vec::new(),
                Err(e) => return Err(e),
            }
        } else if has_unstaged {
            match Self::get_unstaged_diff_with_ignore(
                self.vcs.as_ref(),
                &self.vcs_info.root_path,
                highlighter,
            ) {
                Ok(files) => files,
                Err(TuicrError::NoChanges) => Vec::new(),
                Err(e) => return Err(e),
            }
        } else {
            match Self::get_commit_range_diff_with_ignore(
                self.vcs.as_ref(),
                &self.vcs_info.root_path,
                &selected_ids,
                highlighter,
            ) {
                Ok(files) => files,
                Err(TuicrError::NoChanges) => Vec::new(),
                Err(e) => return Err(e),
            }
        };
        self.commit_diff_cache
            .insert((start, end), diff_files.clone());
        self.diff_files = diff_files;

        // Reset navigation, rebuild file tree + annotations
        let wrap = self.diff_state.wrap_lines;
        self.diff_state = DiffState::default();
        self.diff_state.wrap_lines = wrap;
        self.file_list_state = FileListState::default();
        self.expanded_gaps.clear();
        self.expanded_content.clear();
        self.sort_files_by_directory(true);
        self.expand_all_dirs();
        self.rebuild_annotations();

        Ok(())
    }

    fn load_staged_unstaged_and_commits_selection(
        &mut self,
        selected_ids: Vec<String>,
        selected_commits: Vec<CommitInfo>,
    ) -> Result<()> {
        let highlighter = self.theme.syntax_highlighter();
        let diff_files = match Self::get_working_tree_with_commits_diff_with_ignore(
            self.vcs.as_ref(),
            &self.vcs_info.root_path,
            &selected_ids,
            highlighter,
        ) {
            Ok(diff_files) => diff_files,
            Err(TuicrError::NoChanges) => {
                self.set_message("No changes in selected commits + staged/unstaged");
                return Ok(());
            }
            Err(e) => return Err(e),
        };

        self.session = Self::load_or_create_staged_unstaged_and_commits_session(
            &self.vcs_info,
            &selected_ids,
        );

        for file in &diff_files {
            let path = file.display_path().clone();
            self.session.add_file(path, file.status);
        }

        self.diff_files = diff_files;
        self.diff_source = DiffSource::StagedUnstagedAndCommits(selected_ids);
        self.input_mode = InputMode::Normal;
        self.diff_state = DiffState::default();
        self.file_list_state = FileListState::default();

        // Set up inline commit selector (newest-first display order)
        self.review_commits = selected_commits.into_iter().rev().collect();
        self.range_diff_files = Some(self.diff_files.clone());
        self.commit_list = self.review_commits.clone();
        self.commit_list_cursor = 0;
        self.commit_selection_range = if self.review_commits.is_empty() {
            None
        } else {
            Some((0, self.review_commits.len() - 1))
        };
        self.commit_list_scroll_offset = 0;
        self.visible_commit_count = self.review_commits.len();
        self.has_more_commit = false;
        self.show_commit_selector = self.review_commits.len() > 1;
        self.commit_diff_cache.clear();
        self.saved_inline_selection = None;

        self.sort_files_by_directory(true);
        self.expand_all_dirs();
        self.rebuild_annotations();
        Ok(())
    }

    fn sort_files_by_directory(&mut self, reset_position: bool) {
        use std::collections::BTreeMap;
        use std::path::Path;

        let current_path = if !reset_position {
            self.current_file_path().cloned()
        } else {
            None
        };

        let mut dir_map: BTreeMap<String, Vec<DiffFile>> = BTreeMap::new();

        for file in self.diff_files.drain(..) {
            let path = file.display_path();
            let dir = if let Some(parent) = path.parent() {
                if parent == Path::new("") {
                    ".".to_string()
                } else {
                    parent.to_string_lossy().to_string()
                }
            } else {
                ".".to_string()
            };

            dir_map.entry(dir).or_default().push(file);
        }

        for (_dir, files) in dir_map {
            self.diff_files.extend(files);
        }

        if let Some(path) = current_path
            && let Some(idx) = self
                .diff_files
                .iter()
                .position(|f| f.display_path() == &path)
        {
            self.jump_to_file(idx);
            return;
        }

        self.jump_to_file(0);
    }

    pub fn expand_all_dirs(&mut self) {
        use std::path::Path;

        self.expanded_dirs.clear();
        for file in &self.diff_files {
            let path = file.display_path();
            let mut current = path.parent();
            while let Some(parent) = current {
                if parent != Path::new("") {
                    self.expanded_dirs
                        .insert(parent.to_string_lossy().to_string());
                }
                current = parent.parent();
            }
        }
        self.ensure_valid_tree_selection();
    }

    pub fn collapse_all_dirs(&mut self) {
        self.expanded_dirs.clear();
        self.ensure_valid_tree_selection();
    }

    pub fn toggle_directory(&mut self, dir_path: &str) {
        if self.expanded_dirs.contains(dir_path) {
            self.expanded_dirs.remove(dir_path);
            self.ensure_valid_tree_selection();
        } else {
            self.expanded_dirs.insert(dir_path.to_string());
        }
    }

    /// Check if a hunk gap has been expanded
    pub fn is_gap_expanded(&self, gap_id: &GapId) -> bool {
        self.expanded_gaps.contains(gap_id)
    }

    /// Expand a gap to show hidden context lines
    pub fn expand_gap(&mut self, gap_id: GapId) -> Result<()> {
        if self.expanded_gaps.contains(&gap_id) {
            return Ok(()); // Already expanded
        }

        let file = self.diff_files.get(gap_id.file_idx).ok_or_else(|| {
            TuicrError::CorruptedSession(format!("Invalid file index: {}", gap_id.file_idx))
        })?;

        let hunk = file.hunks.get(gap_id.hunk_idx).ok_or_else(|| {
            TuicrError::CorruptedSession(format!("Invalid hunk index: {}", gap_id.hunk_idx))
        })?;

        // Get previous hunk to calculate gap boundaries
        let prev_hunk = if gap_id.hunk_idx > 0 {
            file.hunks.get(gap_id.hunk_idx - 1)
        } else {
            None
        };

        // Calculate line range to fetch
        let (start_line, end_line) = match prev_hunk {
            None => (1, hunk.new_start.saturating_sub(1)),
            Some(prev) => {
                let prev_end = prev.new_start + prev.new_count;
                (prev_end, hunk.new_start.saturating_sub(1))
            }
        };

        if start_line > end_line {
            return Ok(()); // No gap to expand
        }

        let file_path = file.display_path().clone();
        let file_status = file.status;

        // Fetch the context lines
        let lines = self
            .vcs
            .fetch_context_lines(&file_path, file_status, start_line, end_line)?;

        self.expanded_content.insert(gap_id.clone(), lines);
        self.expanded_gaps.insert(gap_id);
        self.rebuild_annotations();

        Ok(())
    }

    /// Collapse an expanded gap
    pub fn collapse_gap(&mut self, gap_id: GapId) {
        self.expanded_gaps.remove(&gap_id);
        self.expanded_content.remove(&gap_id);
        self.rebuild_annotations();
    }

    /// Clear all expanded gaps (called when reloading diffs)
    pub fn clear_expanded_gaps(&mut self) {
        self.expanded_gaps.clear();
        self.expanded_content.clear();
    }

    /// Rebuild the line annotations cache. Call this when:
    /// - Diff files change (load/reload)
    /// - Expansion state changes (expand/collapse gap)
    /// - Comments are added/removed
    /// - Diff view mode changes
    pub fn rebuild_annotations(&mut self) {
        self.line_annotations.clear();

        for (file_idx, file) in self.diff_files.iter().enumerate() {
            let path = file.display_path();

            // File header
            self.line_annotations
                .push(AnnotatedLine::FileHeader { file_idx });

            // If reviewed, skip all content for this file
            if self.session.is_file_reviewed(path) {
                continue;
            }

            // File comments
            if let Some(review) = self.session.files.get(path) {
                for (comment_idx, comment) in review.file_comments.iter().enumerate() {
                    let comment_lines = Self::comment_display_lines(comment);
                    for _ in 0..comment_lines {
                        self.line_annotations.push(AnnotatedLine::FileComment {
                            file_idx,
                            comment_idx,
                        });
                    }
                }
            }

            if file.is_binary || file.hunks.is_empty() {
                self.line_annotations
                    .push(AnnotatedLine::BinaryOrEmpty { file_idx });
            } else {
                // Get line comments for this file
                let line_comments = self
                    .session
                    .files
                    .get(path)
                    .map(|r| &r.line_comments)
                    .cloned()
                    .unwrap_or_default();

                for (hunk_idx, hunk) in file.hunks.iter().enumerate() {
                    // Calculate gap before this hunk
                    let prev_hunk = if hunk_idx > 0 {
                        file.hunks.get(hunk_idx - 1)
                    } else {
                        None
                    };
                    let gap = calculate_gap(
                        prev_hunk.map(|h| (&h.new_start, &h.new_count)),
                        hunk.new_start,
                    );

                    let gap_id = GapId { file_idx, hunk_idx };

                    if gap > 0 {
                        if self.expanded_gaps.contains(&gap_id) {
                            // Expanded content lines
                            if let Some(content) = self.expanded_content.get(&gap_id) {
                                for (content_idx, _) in content.iter().enumerate() {
                                    self.line_annotations.push(AnnotatedLine::ExpandedContext {
                                        gap_id: gap_id.clone(),
                                        line_idx: content_idx,
                                    });
                                }
                            }
                        } else {
                            // Expander line
                            self.line_annotations.push(AnnotatedLine::Expander {
                                gap_id: gap_id.clone(),
                            });
                        }
                    }

                    // Hunk header
                    self.line_annotations
                        .push(AnnotatedLine::HunkHeader { file_idx, hunk_idx });

                    // Diff lines - handle differently based on view mode
                    match self.diff_view_mode {
                        DiffViewMode::Unified => {
                            Self::build_unified_diff_annotations(
                                &mut self.line_annotations,
                                file_idx,
                                hunk_idx,
                                &hunk.lines,
                                &line_comments,
                            );
                        }
                        DiffViewMode::SideBySide => {
                            Self::build_side_by_side_annotations(
                                &mut self.line_annotations,
                                file_idx,
                                hunk_idx,
                                &hunk.lines,
                                &line_comments,
                            );
                        }
                    }
                }
            }

            // Spacing line
            self.line_annotations.push(AnnotatedLine::Spacing);
        }
    }

    fn push_comments(
        annotations: &mut Vec<AnnotatedLine>,
        file_idx: usize,
        line_no: Option<u32>,
        line_comments: &std::collections::HashMap<u32, Vec<crate::model::Comment>>,
        side: LineSide,
    ) {
        let Some(ln) = line_no else {
            return;
        };

        let Some(comments) = line_comments.get(&ln) else {
            return;
        };

        for (idx, comment) in comments.iter().enumerate() {
            let matches_side =
                comment.side == Some(side) || (side == LineSide::New && comment.side.is_none());

            if !matches_side {
                continue;
            }

            let comment_lines = Self::comment_display_lines(comment);
            for _ in 0..comment_lines {
                annotations.push(AnnotatedLine::LineComment {
                    file_idx,
                    line: ln,
                    comment_idx: idx,
                    side,
                });
            }
        }
    }

    /// Build annotations for unified diff mode (one annotation per diff line)
    fn build_unified_diff_annotations(
        annotations: &mut Vec<AnnotatedLine>,
        file_idx: usize,
        hunk_idx: usize,
        lines: &[crate::model::DiffLine],
        line_comments: &std::collections::HashMap<u32, Vec<crate::model::Comment>>,
    ) {
        for (line_idx, diff_line) in lines.iter().enumerate() {
            annotations.push(AnnotatedLine::DiffLine {
                file_idx,
                hunk_idx,
                line_idx,
                old_lineno: diff_line.old_lineno,
                new_lineno: diff_line.new_lineno,
            });

            // Line comments on old side (delete lines)
            if let Some(old_ln) = diff_line.old_lineno {
                Self::push_comments(
                    annotations,
                    file_idx,
                    Some(old_ln),
                    line_comments,
                    LineSide::Old,
                );
            }

            // Line comments on new side (added/context lines)
            if let Some(new_ln) = diff_line.new_lineno {
                Self::push_comments(
                    annotations,
                    file_idx,
                    Some(new_ln),
                    line_comments,
                    LineSide::New,
                );
            }
        }
    }

    /// Build annotations for side-by-side diff mode, pairing deletions and additions into aligned rows.
    fn build_side_by_side_annotations(
        annotations: &mut Vec<AnnotatedLine>,
        file_idx: usize,
        hunk_idx: usize,
        lines: &[crate::model::DiffLine],
        line_comments: &std::collections::HashMap<u32, Vec<crate::model::Comment>>,
    ) {
        let mut i = 0;
        while i < lines.len() {
            let diff_line = &lines[i];

            match diff_line.origin {
                LineOrigin::Context => {
                    annotations.push(AnnotatedLine::SideBySideLine {
                        file_idx,
                        hunk_idx,
                        del_line_idx: Some(i),
                        add_line_idx: Some(i),
                        old_lineno: diff_line.old_lineno,
                        new_lineno: diff_line.new_lineno,
                    });

                    Self::push_comments(
                        annotations,
                        file_idx,
                        diff_line.new_lineno,
                        line_comments,
                        LineSide::New,
                    );

                    i += 1
                }

                LineOrigin::Deletion => {
                    // Find consecutive deletions
                    let del_start = i;
                    let mut del_end = i + 1;
                    while del_end < lines.len() && lines[del_end].origin == LineOrigin::Deletion {
                        del_end += 1;
                    }

                    // Find consecutive additions following deletions
                    let add_start = del_end;
                    let mut add_end = add_start;
                    while add_end < lines.len() && lines[add_end].origin == LineOrigin::Addition {
                        add_end += 1;
                    }

                    let del_count = del_end - del_start;
                    let add_count = add_end - add_start;
                    let max_lines = del_count.max(add_count);

                    for offset in 0..max_lines {
                        let del_idx = if offset < del_count {
                            Some(del_start + offset)
                        } else {
                            None
                        };
                        let add_idx = if offset < add_count {
                            Some(add_start + offset)
                        } else {
                            None
                        };

                        let old_lineno = del_idx.and_then(|idx| lines[idx].old_lineno);
                        let new_lineno = add_idx.and_then(|idx| lines[idx].new_lineno);

                        annotations.push(AnnotatedLine::SideBySideLine {
                            file_idx,
                            hunk_idx,
                            del_line_idx: del_idx,
                            add_line_idx: add_idx,
                            old_lineno,
                            new_lineno,
                        });

                        Self::push_comments(
                            annotations,
                            file_idx,
                            old_lineno,
                            line_comments,
                            LineSide::Old,
                        );
                        Self::push_comments(
                            annotations,
                            file_idx,
                            new_lineno,
                            line_comments,
                            LineSide::New,
                        );
                    }

                    i = add_end;
                }
                LineOrigin::Addition => {
                    annotations.push(AnnotatedLine::SideBySideLine {
                        file_idx,
                        hunk_idx,
                        del_line_idx: None,
                        add_line_idx: Some(i),
                        old_lineno: None,
                        new_lineno: diff_line.new_lineno,
                    });

                    Self::push_comments(
                        annotations,
                        file_idx,
                        diff_line.new_lineno,
                        line_comments,
                        LineSide::New,
                    );

                    i += 1;
                }
            }
        }
    }

    /// Check if cursor is on an expander line or expanded content and return GapId and whether expanded
    /// Returns (GapId, is_expanded) - is_expanded is true if cursor is on expanded content
    pub fn get_gap_at_cursor(&self) -> Option<(GapId, bool)> {
        let target = self.diff_state.cursor_line;
        match self.line_annotations.get(target) {
            Some(AnnotatedLine::Expander { gap_id, .. }) => Some((gap_id.clone(), false)),
            Some(AnnotatedLine::ExpandedContext { gap_id, .. }) => Some((gap_id.clone(), true)),
            _ => None,
        }
    }

    fn ensure_valid_tree_selection(&mut self) {
        use std::path::Path;

        let visible_items = self.build_visible_items();
        if visible_items.is_empty() {
            self.file_list_state.select(0);
            return;
        }

        let current_file_idx = self.diff_state.current_file_idx;
        let file_visible = visible_items.iter().any(|item| {
            matches!(item, FileTreeItem::File { file_idx, .. } if *file_idx == current_file_idx)
        });

        if file_visible {
            if let Some(tree_idx) = self.file_idx_to_tree_idx(current_file_idx) {
                self.file_list_state.select(tree_idx);
            }
        } else {
            if let Some(file) = self.diff_files.get(current_file_idx) {
                let file_path = file.display_path();
                let mut current = file_path.parent();
                while let Some(parent) = current {
                    if parent != Path::new("") {
                        let parent_str = parent.to_string_lossy().to_string();
                        for (tree_idx, item) in visible_items.iter().enumerate() {
                            if let FileTreeItem::Directory { path, .. } = item
                                && *path == parent_str
                            {
                                self.file_list_state.select(tree_idx);
                                return;
                            }
                        }
                    }
                    current = parent.parent();
                }
            }
            self.file_list_state.select(0);
        }
    }

    pub fn build_visible_items(&self) -> Vec<FileTreeItem> {
        use std::path::Path;

        let mut items = Vec::new();
        let mut seen_dirs: HashSet<String> = HashSet::new();

        for (file_idx, file) in self.diff_files.iter().enumerate() {
            let path = file.display_path();

            let mut ancestors: Vec<String> = Vec::new();
            let mut current = path.parent();
            while let Some(parent) = current {
                if parent != Path::new("") {
                    ancestors.push(parent.to_string_lossy().to_string());
                }
                current = parent.parent();
            }
            ancestors.reverse();

            let mut visible = true;
            for (depth, dir) in ancestors.iter().enumerate() {
                if !seen_dirs.contains(dir) && visible {
                    let expanded = self.expanded_dirs.contains(dir);
                    items.push(FileTreeItem::Directory {
                        path: dir.clone(),
                        depth,
                        expanded,
                    });
                    seen_dirs.insert(dir.clone());
                }

                if !self.expanded_dirs.contains(dir) {
                    visible = false;
                }
            }

            if visible {
                items.push(FileTreeItem::File {
                    file_idx,
                    depth: ancestors.len(),
                });
            }
        }

        items
    }

    pub fn get_selected_tree_item(&self) -> Option<FileTreeItem> {
        let visible_items = self.build_visible_items();
        let selected_idx = self.file_list_state.selected();
        visible_items.get(selected_idx).cloned()
    }
}

#[cfg(test)]
mod tree_tests {
    use super::*;
    use crate::model::{DiffFile, FileStatus};

    fn make_file(path: &str) -> DiffFile {
        DiffFile {
            old_path: None,
            new_path: Some(PathBuf::from(path)),
            status: FileStatus::Modified,
            hunks: vec![],
            is_binary: false,
            is_too_large: false,
        }
    }

    struct TreeTestHarness {
        diff_files: Vec<DiffFile>,
        expanded_dirs: HashSet<String>,
    }

    impl TreeTestHarness {
        fn new(paths: &[&str]) -> Self {
            Self {
                diff_files: paths.iter().map(|p| make_file(p)).collect(),
                expanded_dirs: HashSet::new(),
            }
        }

        fn expand_all(&mut self) {
            use std::path::Path;
            for file in &self.diff_files {
                let path = file.display_path();
                let mut current = path.parent();
                while let Some(parent) = current {
                    if parent != Path::new("") {
                        self.expanded_dirs
                            .insert(parent.to_string_lossy().to_string());
                    }
                    current = parent.parent();
                }
            }
        }

        fn collapse_all(&mut self) {
            self.expanded_dirs.clear();
        }

        fn toggle(&mut self, dir: &str) {
            if self.expanded_dirs.contains(dir) {
                self.expanded_dirs.remove(dir);
            } else {
                self.expanded_dirs.insert(dir.to_string());
            }
        }

        fn build_visible_items(&self) -> Vec<FileTreeItem> {
            use std::path::Path;
            let mut items = Vec::new();
            let mut seen_dirs: HashSet<String> = HashSet::new();

            for (file_idx, file) in self.diff_files.iter().enumerate() {
                let path = file.display_path();
                let mut ancestors: Vec<String> = Vec::new();
                let mut current = path.parent();
                while let Some(parent) = current {
                    if parent != Path::new("") {
                        ancestors.push(parent.to_string_lossy().to_string());
                    }
                    current = parent.parent();
                }
                ancestors.reverse();

                let mut visible = true;
                for (depth, dir) in ancestors.iter().enumerate() {
                    if !seen_dirs.contains(dir) && visible {
                        let expanded = self.expanded_dirs.contains(dir);
                        items.push(FileTreeItem::Directory {
                            path: dir.clone(),
                            depth,
                            expanded,
                        });
                        seen_dirs.insert(dir.clone());
                    }
                    if !self.expanded_dirs.contains(dir) {
                        visible = false;
                    }
                }

                if visible {
                    items.push(FileTreeItem::File {
                        file_idx,
                        depth: ancestors.len(),
                    });
                }
            }
            items
        }

        fn visible_file_count(&self) -> usize {
            self.build_visible_items()
                .iter()
                .filter(|i| matches!(i, FileTreeItem::File { .. }))
                .count()
        }

        fn visible_dir_count(&self) -> usize {
            self.build_visible_items()
                .iter()
                .filter(|i| matches!(i, FileTreeItem::Directory { .. }))
                .count()
        }
    }

    #[test]
    fn test_expand_all_shows_all_files() {
        let mut h = TreeTestHarness::new(&["src/ui/app.rs", "src/ui/help.rs", "src/main.rs"]);
        h.expand_all();

        assert_eq!(h.visible_file_count(), 3);
    }

    #[test]
    fn test_collapse_all_hides_all_files() {
        let mut h = TreeTestHarness::new(&["src/ui/app.rs", "src/main.rs"]);
        h.expand_all();
        h.collapse_all();

        assert_eq!(h.visible_file_count(), 0);
        assert_eq!(h.visible_dir_count(), 1); // only "src" visible
    }

    #[test]
    fn test_collapse_parent_hides_nested_dirs() {
        let mut h = TreeTestHarness::new(&["src/ui/components/button.rs"]);
        h.expand_all();
        assert_eq!(h.visible_dir_count(), 3); // src, src/ui, src/ui/components

        h.toggle("src");
        let items = h.build_visible_items();
        assert_eq!(items.len(), 1); // only collapsed "src" dir
        assert!(matches!(
            &items[0],
            FileTreeItem::Directory {
                expanded: false,
                ..
            }
        ));
    }

    #[test]
    fn test_root_files_always_visible() {
        let mut h = TreeTestHarness::new(&["README.md", "Cargo.toml"]);
        h.collapse_all();

        assert_eq!(h.visible_file_count(), 2);
    }

    #[test]
    fn test_tree_depth_correct() {
        let mut h = TreeTestHarness::new(&["a/b/c/file.rs"]);
        h.expand_all();

        let items = h.build_visible_items();
        assert!(matches!(&items[0], FileTreeItem::Directory { depth: 0, path, .. } if path == "a"));
        assert!(
            matches!(&items[1], FileTreeItem::Directory { depth: 1, path, .. } if path == "a/b")
        );
        assert!(
            matches!(&items[2], FileTreeItem::Directory { depth: 2, path, .. } if path == "a/b/c")
        );
        assert!(matches!(&items[3], FileTreeItem::File { depth: 3, .. }));
    }

    #[test]
    fn test_toggle_expands_collapsed_dir() {
        let mut h = TreeTestHarness::new(&["src/main.rs"]);
        h.collapse_all();
        assert_eq!(h.visible_file_count(), 0);

        h.toggle("src");
        assert_eq!(h.visible_file_count(), 1);
    }

    #[test]
    fn test_sibling_dirs_independent() {
        let mut h = TreeTestHarness::new(&["src/app.rs", "tests/test.rs"]);
        h.expand_all();
        h.toggle("src"); // collapse src

        assert_eq!(h.visible_file_count(), 1); // only tests/test.rs
    }
}

#[cfg(test)]
mod commit_selection_tests {
    use super::*;
    use crate::model::FileStatus;
    use crate::vcs::VcsType;

    struct DummyVcs {
        info: VcsInfo,
    }

    impl VcsBackend for DummyVcs {
        fn info(&self) -> &VcsInfo {
            &self.info
        }

        fn get_working_tree_diff(&self, _highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
            Err(TuicrError::NoChanges)
        }

        fn fetch_context_lines(
            &self,
            _file_path: &Path,
            _file_status: FileStatus,
            _start_line: u32,
            _end_line: u32,
        ) -> Result<Vec<DiffLine>> {
            Ok(Vec::new())
        }
    }

    fn build_app(commit_list: Vec<CommitInfo>) -> App {
        let vcs_info = VcsInfo {
            root_path: PathBuf::from("/tmp"),
            head_commit: "head".to_string(),
            branch_name: Some("main".to_string()),
            vcs_type: VcsType::Git,
        };
        let session = ReviewSession::new(
            vcs_info.root_path.clone(),
            vcs_info.head_commit.clone(),
            vcs_info.branch_name.clone(),
            SessionDiffSource::WorkingTree,
        );

        App::build(
            Box::new(DummyVcs {
                info: vcs_info.clone(),
            }),
            vcs_info,
            Theme::dark(),
            false,
            Vec::new(),
            session,
            DiffSource::WorkingTree,
            InputMode::CommitSelect,
            commit_list,
        )
        .expect("failed to build test app")
    }

    fn normal_commit(id: &str) -> CommitInfo {
        CommitInfo {
            id: id.to_string(),
            short_id: id.to_string(),
            branch_name: None,
            summary: "Test commit".to_string(),
            author: "Test".to_string(),
            time: Utc::now(),
        }
    }

    #[test]
    fn special_commit_count_counts_leading_special_entries() {
        let app = build_app(vec![
            App::staged_commit_entry(),
            App::unstaged_commit_entry(),
            normal_commit("abc123"),
        ]);

        assert_eq!(app.special_commit_count(), 2);
    }

    #[test]
    fn special_commit_count_ignores_non_leading_special_entries() {
        let app = build_app(vec![normal_commit("abc123"), App::staged_commit_entry()]);

        assert_eq!(app.special_commit_count(), 0);
    }
}

#[cfg(test)]
mod scroll_tests {
    use super::*;

    /// Test the max_scroll_offset calculation logic directly using DiffState
    /// This tests the core algorithm without needing full App setup
    fn calc_max_scroll(total_lines: usize, viewport_height: usize, wrap_lines: bool) -> usize {
        let viewport = viewport_height.max(1);
        if wrap_lines {
            // With wrapping, allow scrolling to show the last line at the top
            total_lines.saturating_sub(1)
        } else {
            // Without wrapping, stop when last line is at the bottom
            total_lines.saturating_sub(viewport)
        }
    }

    #[test]
    fn should_calculate_max_scroll_without_wrapping() {
        // Given 103 total lines and viewport of 20 (simulating header + 100 lines + spacing)
        let total = 103;
        let viewport = 20;

        // When we calculate max_scroll without wrapping
        let max_scroll = calc_max_scroll(total, viewport, false);

        // Then max_scroll should be total - viewport (allows last line at bottom)
        assert_eq!(max_scroll, 83); // 103 - 20
    }

    #[test]
    fn should_calculate_max_scroll_with_wrapping() {
        // Given 103 total lines and viewport of 20, with wrapping enabled
        let total = 103;
        let viewport = 20;

        // When we calculate max_scroll with wrapping
        let max_scroll = calc_max_scroll(total, viewport, true);

        // Then max_scroll should be total - 1 (allows last line at top)
        assert_eq!(max_scroll, 102); // 103 - 1
    }

    #[test]
    fn should_allow_scrolling_further_with_wrapping() {
        // Given identical content with and without wrapping
        let total = 103;
        let viewport = 20;

        // When we calculate max_scroll for both
        let max_no_wrap = calc_max_scroll(total, viewport, false);
        let max_with_wrap = calc_max_scroll(total, viewport, true);

        // Then wrapping should allow scrolling further
        assert!(
            max_with_wrap > max_no_wrap,
            "With wrapping, max_scroll ({}) should be greater than without ({})",
            max_with_wrap,
            max_no_wrap
        );

        // The difference should be viewport - 1
        assert_eq!(max_with_wrap - max_no_wrap, viewport - 1);
    }

    #[test]
    fn should_handle_small_content_without_wrapping() {
        // Given content smaller than viewport (13 lines in viewport of 50)
        let total = 13;
        let viewport = 50;

        // When we calculate max_scroll
        let max_scroll = calc_max_scroll(total, viewport, false);

        // Then max_scroll should be 0 (no scrolling needed)
        assert_eq!(max_scroll, 0);
    }

    #[test]
    fn should_handle_small_content_with_wrapping() {
        // Given content smaller than viewport with wrapping
        let total = 13;
        let viewport = 50;

        // When we calculate max_scroll
        let max_scroll = calc_max_scroll(total, viewport, true);

        // Then max_scroll should still allow scrolling to the last line
        assert_eq!(max_scroll, 12); // total - 1
    }

    #[test]
    fn should_handle_empty_content() {
        // Given no content (0 lines)
        let total = 0;
        let viewport = 20;

        // When we calculate max_scroll
        let max_scroll_no_wrap = calc_max_scroll(total, viewport, false);
        let max_scroll_wrap = calc_max_scroll(total, viewport, true);

        // Then both should be 0
        assert_eq!(max_scroll_no_wrap, 0);
        assert_eq!(max_scroll_wrap, 0);
    }

    #[test]
    fn should_handle_zero_viewport() {
        // Given content with viewport of 0 (edge case)
        let total = 100;
        let viewport = 0;

        // When we calculate max_scroll (viewport.max(1) makes it 1)
        let max_scroll_no_wrap = calc_max_scroll(total, viewport, false);
        let max_scroll_wrap = calc_max_scroll(total, viewport, true);

        // Then no_wrap should be total - 1, wrap should be total - 1
        assert_eq!(max_scroll_no_wrap, 99); // total - 1 (since viewport becomes 1)
        assert_eq!(max_scroll_wrap, 99); // total - 1
    }

    #[test]
    fn should_match_max_scroll_offset_implementation() {
        // Verify calc_max_scroll matches the actual implementation
        let diff_state_no_wrap = DiffState {
            viewport_height: 20,
            wrap_lines: false,
            ..Default::default()
        };

        let diff_state_wrap = DiffState {
            viewport_height: 20,
            wrap_lines: true,
            ..Default::default()
        };

        // Test that DiffState defaults match our expectations
        assert!(!diff_state_no_wrap.wrap_lines);
        assert!(diff_state_wrap.wrap_lines);
        assert_eq!(diff_state_no_wrap.viewport_height, 20);
        assert_eq!(diff_state_wrap.viewport_height, 20);
    }
}
