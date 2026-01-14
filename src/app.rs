use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::error::{Result, TuicrError};
use crate::git::{
    CommitInfo, RepoInfo, calculate_gap, fetch_context_lines, get_commit_range_diff,
    get_recent_commits, get_working_tree_diff,
};
use crate::model::{Comment, CommentType, DiffFile, DiffLine, LineSide, ReviewSession};
use crate::persistence::{find_session_for_repo, load_session};

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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffSource {
    WorkingTree,
    CommitRange(Vec<String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmAction {
    CopyAndQuit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusedPanel {
    FileList,
    Diff,
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
    pub repo_info: RepoInfo,
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

    // Commit selection state
    pub commit_list: Vec<CommitInfo>,
    pub commit_list_cursor: usize,
    /// Selected commit range as (start_idx, end_idx) inclusive, where start <= end.
    /// Indices refer to positions in commit_list (0 = newest/HEAD, higher = older).
    pub commit_selection_range: Option<(usize, usize)>,

    pub should_quit: bool,
    pub dirty: bool,
    pub message: Option<Message>,
    pub pending_confirm: Option<ConfirmAction>,
    pub supports_keyboard_enhancement: bool,
    pub show_file_list: bool,
    pub expanded_dirs: HashSet<String>,
    /// Tracks which hunk gaps have been expanded to show more context
    pub expanded_gaps: HashSet<GapId>,
    /// Stores the expanded context lines for each gap
    pub expanded_content: HashMap<GapId, Vec<DiffLine>>,
    /// Cached annotations describing what each rendered line represents
    pub line_annotations: Vec<AnnotatedLine>,
}

#[derive(Default)]
pub struct FileListState {
    pub list_state: ratatui::widgets::ListState,
    pub scroll_x: usize,
    pub viewport_width: usize,    // Set during render
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
    pub fn new() -> Result<Self> {
        let repo_info = RepoInfo::discover()?;

        // Try to get working tree diff first
        let diff_result = get_working_tree_diff(&repo_info.repo);

        match diff_result {
            Ok(diff_files) => {
                // We have unstaged changes - normal flow
                let mut session = Self::load_or_create_session(&repo_info);

                // Ensure all current diff files are in the session
                for file in &diff_files {
                    let path = file.display_path().clone();
                    session.add_file(path, file.status);
                }

                let mut app = Self {
                    repo_info,
                    session,
                    diff_files,
                    diff_source: DiffSource::WorkingTree,
                    input_mode: InputMode::Normal,
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
                    commit_list: Vec::new(),
                    commit_list_cursor: 0,
                    commit_selection_range: None,
                    should_quit: false,
                    dirty: false,
                    message: None,
                    pending_confirm: None,
                    supports_keyboard_enhancement: false,
                    show_file_list: true,
                    expanded_dirs: HashSet::new(),
                    expanded_gaps: HashSet::new(),
                    expanded_content: HashMap::new(),
                    line_annotations: Vec::new(),
                };
                app.sort_files_by_directory(true);
                app.expand_all_dirs();
                app.rebuild_annotations();
                Ok(app)
            }
            Err(TuicrError::NoChanges) => {
                // No unstaged changes - try to get recent commits
                let commits = get_recent_commits(&repo_info.repo, 5)?;
                if commits.is_empty() {
                    return Err(TuicrError::NoChanges);
                }

                let session =
                    ReviewSession::new(repo_info.root_path.clone(), repo_info.head_commit.clone());

                Ok(Self {
                    repo_info,
                    session,
                    diff_files: Vec::new(),
                    diff_source: DiffSource::WorkingTree,
                    input_mode: InputMode::CommitSelect,
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
                    commit_list: commits,
                    commit_list_cursor: 0,
                    commit_selection_range: None,
                    should_quit: false,
                    dirty: false,
                    message: None,
                    pending_confirm: None,
                    supports_keyboard_enhancement: false,
                    show_file_list: true,
                    expanded_dirs: HashSet::new(),
                    expanded_gaps: HashSet::new(),
                    expanded_content: HashMap::new(),
                    line_annotations: Vec::new(),
                })
            }
            Err(e) => Err(e),
        }
    }

    fn load_or_create_session(repo_info: &RepoInfo) -> ReviewSession {
        match find_session_for_repo(&repo_info.root_path) {
            Ok(Some(path)) => match load_session(&path) {
                Ok(s) => {
                    // Delete stale session file if base commit doesn't match
                    if s.base_commit != repo_info.head_commit {
                        let _ = std::fs::remove_file(&path);
                        ReviewSession::new(
                            repo_info.root_path.clone(),
                            repo_info.head_commit.clone(),
                        )
                    } else {
                        s
                    }
                }
                Err(_) => {
                    ReviewSession::new(repo_info.root_path.clone(), repo_info.head_commit.clone())
                }
            },
            _ => ReviewSession::new(repo_info.root_path.clone(), repo_info.head_commit.clone()),
        }
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

        let diff_files = get_working_tree_diff(&self.repo_info.repo)?;

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
            let total_lines = self.total_lines();
            if total_lines == 0 {
                self.diff_state.scroll_offset = 0;
            } else {
                let max_scroll = total_lines.saturating_sub(viewport);
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
        let viewport = self.diff_state.viewport_height.max(1);
        let max_line = total.saturating_sub(1);
        let max_scroll = total.saturating_sub(viewport);
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
        self.set_message(format!("Diff wrapping: {}", status));
    }

    fn ensure_cursor_visible(&mut self) {
        let viewport = self.diff_state.viewport_height.max(1);
        let max_scroll = self.total_lines().saturating_sub(viewport);
        if self.diff_state.cursor_line < self.diff_state.scroll_offset {
            self.diff_state.scroll_offset = self.diff_state.cursor_line;
        }
        if self.diff_state.cursor_line >= self.diff_state.scroll_offset + viewport {
            self.diff_state.scroll_offset =
                (self.diff_state.cursor_line - viewport + 1).min(max_scroll);
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

        self.set_message(format!("No matches for \"{}\"", pattern));
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
                Some(format!("... expand ({} lines) ...", gap))
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
                if file.is_binary {
                    Some("(binary file)".to_string())
                } else {
                    Some("(no changes)".to_string())
                }
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
        let max_scroll = self.total_lines().saturating_sub(viewport);
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
            let viewport = self.diff_state.viewport_height.max(1);
            let max_scroll = self.total_lines().saturating_sub(viewport);
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

        let header_lines = 2; // File header + spacing after
        let mut content_lines = 0;

        if file.is_binary || file.hunks.is_empty() {
            content_lines = 1;
        } else {
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
                content_lines += 1 + hunk.lines.len();
            }
        }

        header_lines + content_lines.max(1) - 1 // -1 because spacing is counted separately
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
                        self.set_message(format!("Comment on line {} deleted", line));
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
        self.set_message(format!("Cleared {} comments", cleared));
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
                            format!("Comment on line {} updated", line)
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
                } else if let Some((line, side)) = self.comment_line {
                    let comment = Comment::new(content, self.comment_type, Some(side));
                    review.add_line_comment(line, comment);
                    message = format!("Comment added to line {}", line);
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

    pub fn toggle_diff_view_mode(&mut self) {
        self.diff_view_mode = match self.diff_view_mode {
            DiffViewMode::Unified => DiffViewMode::SideBySide,
            DiffViewMode::SideBySide => DiffViewMode::Unified,
        };
        let mode_name = match self.diff_view_mode {
            DiffViewMode::Unified => "unified",
            DiffViewMode::SideBySide => "side-by-side",
        };
        self.set_message(format!("Diff view mode: {}", mode_name));
    }

    pub fn toggle_file_list(&mut self) {
        self.show_file_list = !self.show_file_list;
        let status = if self.show_file_list {
            "visible"
        } else {
            "hidden"
        };
        self.set_message(format!("File list: {}", status));
    }

    // Commit selection methods

    pub fn commit_select_up(&mut self) {
        if self.commit_list_cursor > 0 {
            self.commit_list_cursor -= 1;
        }
    }

    pub fn commit_select_down(&mut self) {
        if self.commit_list_cursor < self.commit_list.len().saturating_sub(1) {
            self.commit_list_cursor += 1;
        }
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

    pub fn confirm_commit_selection(&mut self) -> Result<()> {
        let Some((start, end)) = self.commit_selection_range else {
            self.set_message("Select at least one commit");
            return Ok(());
        };

        // Collect selected commit IDs (in order from oldest to newest, i.e., end to start)
        // commit_list[0] is newest (HEAD), commit_list[end] is oldest selected
        let selected_ids: Vec<String> = (start..=end)
            .rev()
            .filter_map(|i| self.commit_list.get(i).map(|c| c.id.clone()))
            .collect();

        if selected_ids.is_empty() {
            self.set_message("Select at least one commit");
            return Ok(());
        }

        // Get the diff for the selected commits
        let diff_files = get_commit_range_diff(&self.repo_info.repo, &selected_ids)?;

        if diff_files.is_empty() {
            self.set_message("No changes in selected commits");
            return Ok(());
        }

        // Update session with the newest commit as base
        let newest_commit_id = selected_ids.last().unwrap().clone();
        self.session = ReviewSession::new(self.repo_info.root_path.clone(), newest_commit_id);

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
        let lines = fetch_context_lines(
            &self.repo_info.repo,
            &file_path,
            file_status,
            start_line,
            end_line,
        )?;

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

                    // Diff lines
                    for (line_idx, diff_line) in hunk.lines.iter().enumerate() {
                        self.line_annotations.push(AnnotatedLine::DiffLine {
                            file_idx,
                            hunk_idx,
                            line_idx,
                            old_lineno: diff_line.old_lineno,
                            new_lineno: diff_line.new_lineno,
                        });

                        // Line comments on old side (deleted lines)
                        if let Some(old_ln) = diff_line.old_lineno
                            && let Some(comments) = line_comments.get(&old_ln)
                        {
                            for (idx, comment) in comments.iter().enumerate() {
                                if comment.side == Some(LineSide::Old) {
                                    let comment_lines = Self::comment_display_lines(comment);
                                    for _ in 0..comment_lines {
                                        self.line_annotations.push(AnnotatedLine::LineComment {
                                            file_idx,
                                            line: old_ln,
                                            side: LineSide::Old,
                                            comment_idx: idx,
                                        });
                                    }
                                }
                            }
                        }

                        // Line comments on new side (added/context lines)
                        if let Some(new_ln) = diff_line.new_lineno
                            && let Some(comments) = line_comments.get(&new_ln)
                        {
                            for (idx, comment) in comments.iter().enumerate() {
                                if comment.side != Some(LineSide::Old) {
                                    let comment_lines = Self::comment_display_lines(comment);
                                    for _ in 0..comment_lines {
                                        self.line_annotations.push(AnnotatedLine::LineComment {
                                            file_idx,
                                            line: new_ln,
                                            side: LineSide::New,
                                            comment_idx: idx,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Spacing line
            self.line_annotations.push(AnnotatedLine::Spacing);
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
