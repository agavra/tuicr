use std::path::PathBuf;

use crate::error::Result;
use crate::git::{RepoInfo, get_working_tree_diff};
use crate::model::{Comment, CommentType, DiffFile, LineSide, ReviewSession};
use crate::persistence::{find_session_for_repo, load_session};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    Comment,
    Command,
    Help,
    Confirm,
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

pub struct App {
    pub repo_info: RepoInfo,
    pub session: ReviewSession,
    pub diff_files: Vec<DiffFile>,

    pub input_mode: InputMode,
    pub focused_panel: FocusedPanel,

    pub file_list_state: FileListState,
    pub diff_state: DiffState,
    pub command_buffer: String,
    pub comment_buffer: String,
    pub comment_cursor: usize,
    pub comment_type: CommentType,
    pub comment_is_file_level: bool,
    pub comment_line: Option<(u32, LineSide)>,

    pub should_quit: bool,
    pub dirty: bool,
    pub message: Option<String>,
    pub pending_confirm: Option<ConfirmAction>,
}

#[derive(Debug, Default)]
pub struct FileListState {
    pub selected: usize,
}

#[derive(Debug, Default)]
pub struct DiffState {
    pub scroll_offset: usize,
    pub scroll_x: usize,    // Horizontal scroll offset
    pub cursor_line: usize, // Absolute position in the line list
    pub current_file_idx: usize,
    pub viewport_height: usize, // Set during render
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
        let diff_files = get_working_tree_diff(&repo_info.repo)?;

        // Try to load existing session, or create new one
        let mut session = match find_session_for_repo(&repo_info.root_path) {
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
        };

        // Ensure all current diff files are in the session
        for file in &diff_files {
            let path = file.display_path().clone();
            session.add_file(path, file.status);
        }

        Ok(Self {
            repo_info,
            session,
            diff_files,
            input_mode: InputMode::Normal,
            focused_panel: FocusedPanel::Diff,
            file_list_state: FileListState::default(),
            diff_state: DiffState::default(),
            command_buffer: String::new(),
            comment_buffer: String::new(),
            comment_cursor: 0,
            comment_type: CommentType::Note,
            comment_is_file_level: true,
            comment_line: None,
            should_quit: false,
            dirty: false,
            message: None,
            pending_confirm: None,
        })
    }

    pub fn current_file(&self) -> Option<&DiffFile> {
        self.diff_files.get(self.diff_state.current_file_idx)
    }

    pub fn current_file_path(&self) -> Option<&PathBuf> {
        self.current_file().map(|f| f.display_path())
    }

    pub fn toggle_reviewed(&mut self) {
        if let Some(path) = self.current_file_path().cloned()
            && let Some(review) = self.session.get_file_mut(&path)
        {
            review.reviewed = !review.reviewed;
            self.dirty = true;
        }
    }

    pub fn file_count(&self) -> usize {
        self.diff_files.len()
    }

    pub fn reviewed_count(&self) -> usize {
        self.session.reviewed_count()
    }

    pub fn set_message(&mut self, msg: impl Into<String>) {
        self.message = Some(msg.into());
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
        let max_line = self.total_lines().saturating_sub(1);
        self.diff_state.cursor_line = (self.diff_state.cursor_line + lines).min(max_line);
        self.diff_state.scroll_offset = (self.diff_state.scroll_offset + lines).min(max_line);
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
        self.diff_state.scroll_x = self.diff_state.scroll_x.saturating_sub(cols);
    }

    pub fn scroll_right(&mut self, cols: usize) {
        self.diff_state.scroll_x = self.diff_state.scroll_x.saturating_add(cols);
    }

    fn ensure_cursor_visible(&mut self) {
        let viewport = self.diff_state.viewport_height.max(1);
        // If cursor is above the viewport, scroll up
        if self.diff_state.cursor_line < self.diff_state.scroll_offset {
            self.diff_state.scroll_offset = self.diff_state.cursor_line;
        }
        // If cursor is below the viewport, scroll down
        if self.diff_state.cursor_line >= self.diff_state.scroll_offset + viewport {
            self.diff_state.scroll_offset = self.diff_state.cursor_line - viewport + 1;
        }
    }

    pub fn center_cursor(&mut self) {
        let viewport = self.diff_state.viewport_height.max(1);
        let half_viewport = viewport / 2;
        self.diff_state.scroll_offset = self.diff_state.cursor_line.saturating_sub(half_viewport);
    }

    pub fn file_list_down(&mut self, n: usize) {
        let max_idx = self.diff_files.len().saturating_sub(1);
        let new_idx = (self.file_list_state.selected + n).min(max_idx);
        self.jump_to_file(new_idx);
    }

    pub fn file_list_up(&mut self, n: usize) {
        let new_idx = self.file_list_state.selected.saturating_sub(n);
        self.jump_to_file(new_idx);
    }

    pub fn jump_to_file(&mut self, idx: usize) {
        if idx < self.diff_files.len() {
            self.diff_state.current_file_idx = idx;
            self.diff_state.cursor_line = self.calculate_file_scroll_offset(idx);
            self.diff_state.scroll_offset = self.diff_state.cursor_line;
            self.file_list_state.selected = idx;
        }
    }

    pub fn next_file(&mut self) {
        let next =
            (self.diff_state.current_file_idx + 1).min(self.diff_files.len().saturating_sub(1));
        self.jump_to_file(next);
    }

    pub fn prev_file(&mut self) {
        let prev = self.diff_state.current_file_idx.saturating_sub(1);
        self.jump_to_file(prev);
    }

    pub fn next_hunk(&mut self) {
        // Find the next hunk header position after current cursor
        let mut cumulative = 0;
        for file in &self.diff_files {
            // File header
            cumulative += 1;
            // File comments
            if let Some(review) = self.session.files.get(file.display_path()) {
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
            cumulative += 1; // File header
            if let Some(review) = self.session.files.get(file.display_path()) {
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
            offset += self.file_render_height(file);
        }
        offset
    }

    fn file_render_height(&self, file: &DiffFile) -> usize {
        let header_lines = 2;
        let content_lines: usize = file.hunks.iter().map(|h| h.lines.len() + 1).sum();
        header_lines + content_lines.max(1)
    }

    fn update_current_file_from_cursor(&mut self) {
        let mut cumulative = 0;
        for (i, file) in self.diff_files.iter().enumerate() {
            let height = self.file_render_height(file);
            if cumulative + height > self.diff_state.cursor_line {
                self.diff_state.current_file_idx = i;
                self.file_list_state.selected = i;
                return;
            }
            cumulative += height;
        }
        if !self.diff_files.is_empty() {
            self.diff_state.current_file_idx = self.diff_files.len() - 1;
            self.file_list_state.selected = self.diff_files.len() - 1;
        }
    }

    pub fn total_lines(&self) -> usize {
        self.diff_files
            .iter()
            .map(|f| self.file_render_height(f))
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
        let mut line_idx = 0;

        for file in &self.diff_files {
            let path = file.display_path();

            // File header
            line_idx += 1;

            // File comments (now multiline with box)
            if let Some(review) = self.session.files.get(path) {
                for comment in &review.file_comments {
                    line_idx += Self::comment_display_lines(comment);
                }
            }

            if file.is_binary || file.hunks.is_empty() {
                // Binary file or no changes line
                line_idx += 1;
            } else {
                // Get line comments for counting
                let line_comments = self
                    .session
                    .files
                    .get(path)
                    .map(|r| &r.line_comments)
                    .cloned()
                    .unwrap_or_default();

                for hunk in &file.hunks {
                    // Hunk header
                    line_idx += 1;

                    // Diff lines
                    for diff_line in &hunk.lines {
                        if line_idx == target {
                            // Found cursor position - return line number and side
                            // Deleted lines use old_lineno with LineSide::Old
                            // Added/context lines use new_lineno with LineSide::New
                            return diff_line
                                .new_lineno
                                .map(|ln| (ln, LineSide::New))
                                .or_else(|| diff_line.old_lineno.map(|ln| (ln, LineSide::Old)));
                        }
                        line_idx += 1;

                        // Count line comments for both sides
                        // Old side (deleted lines)
                        if let Some(old_ln) = diff_line.old_lineno
                            && let Some(comments) = line_comments.get(&old_ln)
                        {
                            for comment in comments {
                                if comment.side == Some(LineSide::Old) {
                                    line_idx += Self::comment_display_lines(comment);
                                }
                            }
                        }
                        // New side (added/context lines)
                        if let Some(new_ln) = diff_line.new_lineno
                            && let Some(comments) = line_comments.get(&new_ln)
                        {
                            for comment in comments {
                                if comment.side != Some(LineSide::Old) {
                                    line_idx += Self::comment_display_lines(comment);
                                }
                            }
                        }
                    }
                }
            }

            // Spacing line
            line_idx += 1;
        }

        None
    }

    /// Find the comment at the current cursor position
    fn find_comment_at_cursor(&self) -> Option<CommentLocation> {
        let target = self.diff_state.cursor_line;
        let mut line_idx = 0;

        for file in &self.diff_files {
            let path = file.display_path().clone();

            // File header
            line_idx += 1;

            // File comments - check if cursor is on one
            if let Some(review) = self.session.files.get(&path) {
                for (idx, comment) in review.file_comments.iter().enumerate() {
                    let comment_lines = Self::comment_display_lines(comment);
                    if target >= line_idx && target < line_idx + comment_lines {
                        return Some(CommentLocation::FileComment { path, index: idx });
                    }
                    line_idx += comment_lines;
                }
            }

            if file.is_binary || file.hunks.is_empty() {
                line_idx += 1;
            } else {
                let line_comments = self
                    .session
                    .files
                    .get(&path)
                    .map(|r| r.line_comments.clone())
                    .unwrap_or_default();

                for hunk in &file.hunks {
                    // Hunk header
                    line_idx += 1;

                    for diff_line in &hunk.lines {
                        // Skip the diff line itself
                        line_idx += 1;

                        // Check comments on old side (deleted lines)
                        if let Some(old_ln) = diff_line.old_lineno
                            && let Some(comments) = line_comments.get(&old_ln)
                        {
                            for (idx, comment) in comments.iter().enumerate() {
                                if comment.side == Some(LineSide::Old) {
                                    let comment_lines = Self::comment_display_lines(comment);
                                    if target >= line_idx && target < line_idx + comment_lines {
                                        return Some(CommentLocation::LineComment {
                                            path,
                                            line: old_ln,
                                            side: LineSide::Old,
                                            index: idx,
                                        });
                                    }
                                    line_idx += comment_lines;
                                }
                            }
                        }

                        // Check comments on new side (added/context lines)
                        if let Some(new_ln) = diff_line.new_lineno
                            && let Some(comments) = line_comments.get(&new_ln)
                        {
                            for (idx, comment) in comments.iter().enumerate() {
                                if comment.side != Some(LineSide::Old) {
                                    let comment_lines = Self::comment_display_lines(comment);
                                    if target >= line_idx && target < line_idx + comment_lines {
                                        return Some(CommentLocation::LineComment {
                                            path,
                                            line: new_ln,
                                            side: LineSide::New,
                                            index: idx,
                                        });
                                    }
                                    line_idx += comment_lines;
                                }
                            }
                        }
                    }
                }
            }

            // Spacing line
            line_idx += 1;
        }

        None
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
                        return true;
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
            if self.comment_is_file_level {
                let comment = Comment::new(content, self.comment_type, None);
                review.add_file_comment(comment);
                self.set_message("File comment added");
            } else if let Some((line, side)) = self.comment_line {
                let comment = Comment::new(content, self.comment_type, Some(side));
                review.add_line_comment(line, comment);
                self.set_message(format!("Comment added to line {}", line));
            } else {
                // Fallback to file comment if no line specified
                let comment = Comment::new(content, self.comment_type, None);
                review.add_file_comment(comment);
                self.set_message("File comment added");
            }
            self.dirty = true;
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
        }
    }

    pub fn enter_confirm_mode(&mut self, action: ConfirmAction) {
        self.input_mode = InputMode::Confirm;
        self.pending_confirm = Some(action);
    }

    pub fn exit_confirm_mode(&mut self) {
        self.input_mode = InputMode::Normal;
        self.pending_confirm = None;
    }
}
