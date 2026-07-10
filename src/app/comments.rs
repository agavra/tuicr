use super::*;

impl App {
    pub fn comment_navigator_idx_at_screen_row(&self, screen_row: u16) -> Option<usize> {
        let inner = self.comment_navigator_inner_area?;
        if screen_row < inner.y || screen_row >= inner.y + inner.height {
            return None;
        }
        let rel = (screen_row - inner.y) as usize;
        let idx = self.comment_navigator_state.list_state.offset() + rel;
        let total = self.build_comment_navigator_items().len();
        (idx < total).then_some(idx)
    }

    fn comment_navigator_key(annotation: &AnnotatedLine) -> Option<CommentNavigatorKey> {
        match annotation {
            AnnotatedLine::ReviewComment { comment_idx } => Some(CommentNavigatorKey::Review {
                comment_idx: *comment_idx,
            }),
            AnnotatedLine::FileComment {
                file_idx,
                comment_idx,
            } => Some(CommentNavigatorKey::File {
                file_idx: *file_idx,
                comment_idx: *comment_idx,
            }),
            AnnotatedLine::LineComment {
                file_idx,
                line,
                side,
                comment_idx,
            } => Some(CommentNavigatorKey::Line {
                file_idx: *file_idx,
                line: *line,
                side: *side,
                comment_idx: *comment_idx,
            }),
            AnnotatedLine::RemoteThreadLine { thread_idx } => Some(CommentNavigatorKey::Remote {
                thread_idx: *thread_idx,
            }),
            AnnotatedLine::RemoteReviewSummaryLine { summary_idx } => {
                Some(CommentNavigatorKey::RemoteReview {
                    summary_idx: *summary_idx,
                })
            }
            _ => None,
        }
    }

    fn comment_navigator_item_for_key(
        &self,
        key: CommentNavigatorKey,
        target_annotation: usize,
    ) -> Option<CommentNavigatorItem> {
        match key {
            CommentNavigatorKey::Review { comment_idx } => {
                let comment = self.session.review_comments.get(comment_idx)?;
                Some(CommentNavigatorItem {
                    key: CommentNavigatorKey::Review { comment_idx },
                    kind: CommentNavigatorKind::Local(comment.comment_type.clone()),
                    target_annotation,
                    path: None,
                    line: None,
                    side: None,
                    author: Some(comment.author.clone()),
                })
            }
            CommentNavigatorKey::File {
                file_idx,
                comment_idx,
            } => {
                let path = self.diff_files.get(file_idx)?.display_path();
                let review = self.session.files.get(path)?;
                let comment = review.file_comments.get(comment_idx)?;
                Some(CommentNavigatorItem {
                    key: CommentNavigatorKey::File {
                        file_idx,
                        comment_idx,
                    },
                    kind: CommentNavigatorKind::Local(comment.comment_type.clone()),
                    target_annotation,
                    path: Some(path.display().to_string()),
                    line: None,
                    side: None,
                    author: Some(comment.author.clone()),
                })
            }
            CommentNavigatorKey::Line {
                file_idx,
                line,
                side,
                comment_idx,
            } => {
                let path = self.diff_files.get(file_idx)?.display_path();
                let review = self.session.files.get(path)?;
                let comments = review.line_comments.get(&line)?;
                let comment = comments.get(comment_idx)?;
                Some(CommentNavigatorItem {
                    key: CommentNavigatorKey::Line {
                        file_idx,
                        line,
                        side,
                        comment_idx,
                    },
                    kind: CommentNavigatorKind::Local(comment.comment_type.clone()),
                    target_annotation,
                    path: Some(path.display().to_string()),
                    line: Some(line),
                    side: Some(side),
                    author: Some(comment.author.clone()),
                })
            }
            CommentNavigatorKey::Remote { thread_idx } => {
                let thread = self.forge_review_threads.get(thread_idx)?;
                let muted = self
                    .session
                    .remote_comments_visibility
                    .render_decision(thread)?;
                let side = match thread.side {
                    crate::forge::remote_comments::RemoteCommentSide::Right => LineSide::New,
                    crate::forge::remote_comments::RemoteCommentSide::Left => LineSide::Old,
                };
                let author = thread.root().and_then(|c| c.author.clone());
                Some(CommentNavigatorItem {
                    key: CommentNavigatorKey::Remote { thread_idx },
                    kind: CommentNavigatorKind::Remote { muted },
                    target_annotation,
                    path: Some(thread.path.clone()),
                    line: thread.line,
                    side: Some(side),
                    author,
                })
            }
            CommentNavigatorKey::RemoteReview { summary_idx } => {
                let summary = self.forge_review_summaries.get(summary_idx)?;
                Some(CommentNavigatorItem {
                    key: CommentNavigatorKey::RemoteReview { summary_idx },
                    kind: CommentNavigatorKind::Remote { muted: false },
                    target_annotation,
                    path: None,
                    line: None,
                    side: None,
                    author: summary.author.clone(),
                })
            }
        }
    }

    pub fn build_comment_navigator_items(&self) -> Vec<CommentNavigatorItem> {
        let mut items = Vec::new();
        let mut last_key: Option<CommentNavigatorKey> = None;

        for (idx, annotation) in self.line_annotations.iter().enumerate() {
            let Some(key) = Self::comment_navigator_key(annotation) else {
                last_key = None;
                continue;
            };

            if last_key.as_ref() == Some(&key) {
                continue;
            }

            if let Some(item) = self.comment_navigator_item_for_key(key.clone(), idx) {
                items.push(item);
                last_key = Some(key);
            }
        }

        items
    }

    pub fn has_comment_navigator_items(&self) -> bool {
        !self.build_comment_navigator_items().is_empty()
    }

    pub fn sync_comment_navigator_selection(&mut self, items: &[CommentNavigatorItem]) {
        if items.is_empty() {
            self.comment_navigator_state.list_state.select(None);
            return;
        }

        if self.focused_panel == FocusedPanel::Diff
            && let Some(annotation) = self.line_annotations.get(self.diff_state.cursor_line)
            && let Some(key) = Self::comment_navigator_key(annotation)
            && let Some(idx) = items.iter().position(|item| item.key == key)
        {
            self.comment_navigator_state.select(idx);
            return;
        }

        let selected = self
            .comment_navigator_state
            .selected()
            .min(items.len().saturating_sub(1));
        self.comment_navigator_state.select(selected);
    }

    pub fn comment_navigator_down(&mut self, n: usize) {
        let items = self.build_comment_navigator_items();
        let max_idx = items.len().saturating_sub(1);
        let new_idx = (self.comment_navigator_state.selected() + n).min(max_idx);
        self.comment_navigator_state.select(new_idx);
    }

    pub fn comment_navigator_up(&mut self, n: usize) {
        let new_idx = self.comment_navigator_state.selected().saturating_sub(n);
        self.comment_navigator_state.select(new_idx);
    }

    pub fn comment_navigator_viewport_scroll_down(&mut self, lines: usize) {
        let total = self.build_comment_navigator_items().len();
        let viewport = self.comment_navigator_state.viewport_height.max(1);
        let max_offset = total.saturating_sub(viewport);
        let new_offset = (self.comment_navigator_state.list_state.offset() + lines).min(max_offset);
        *self.comment_navigator_state.list_state.offset_mut() = new_offset;
        if self.comment_navigator_state.selected() < new_offset {
            self.comment_navigator_state.select(new_offset);
        }
    }

    pub fn comment_navigator_viewport_scroll_up(&mut self, lines: usize) {
        let viewport = self.comment_navigator_state.viewport_height.max(1);
        let new_offset = self
            .comment_navigator_state
            .list_state
            .offset()
            .saturating_sub(lines);
        *self.comment_navigator_state.list_state.offset_mut() = new_offset;
        let max_visible = (new_offset + viewport).saturating_sub(1);
        if self.comment_navigator_state.selected() > max_visible {
            self.comment_navigator_state.select(max_visible);
        }
    }

    pub fn jump_to_selected_comment(&mut self) -> bool {
        let items = self.build_comment_navigator_items();
        let Some(item) = items.get(self.comment_navigator_state.selected()) else {
            self.set_message("No comments to navigate");
            return false;
        };
        self.move_cursor_to_annotation(item.target_annotation);
        self.center_cursor();
        self.focused_panel = FocusedPanel::Diff;
        true
    }

    /// True when the cursor sits on a local comment whose lifecycle state
    /// has been pushed/submitted to the forge. Such comments are locked from
    /// edit/delete in tuicr to prevent the local state from drifting from
    /// what GitHub now stores.
    pub fn cursor_on_locked_comment(&self) -> bool {
        let Some(location) = self.find_comment_at_cursor() else {
            return false;
        };
        match location {
            CommentLocation::Review { index } => self
                .session
                .review_comments
                .get(index)
                .is_some_and(|c| c.is_locked()),
            CommentLocation::File { path, index } => self
                .session
                .files
                .get(&path)
                .and_then(|review| review.file_comments.get(index))
                .is_some_and(|c| c.is_locked()),
            CommentLocation::Line {
                path,
                line,
                side,
                index,
            } => self
                .session
                .files
                .get(&path)
                .and_then(|review| review.line_comments.get(&line))
                .and_then(|comments| {
                    let mut side_idx = 0;
                    for c in comments {
                        if c.side.unwrap_or(LineSide::New) == side {
                            if side_idx == index {
                                return Some(c);
                            }
                            side_idx += 1;
                        }
                    }
                    None
                })
                .is_some_and(|c| c.is_locked()),
        }
    }

    /// Find the comment at the current cursor position
    /// True when the cursor is on a row that belongs to a fetched-from-GitHub
    /// review thread. Remote threads are read-only in v1; surfaced as a
    /// distinct condition so the handler can produce a clearer message than
    /// the generic "no comment at cursor".
    pub fn cursor_on_remote_thread(&self) -> bool {
        matches!(
            self.line_annotations.get(self.diff_state.cursor_line),
            Some(AnnotatedLine::RemoteThreadLine { .. })
        )
    }

    fn find_comment_at_cursor(&self) -> Option<CommentLocation> {
        let target = self.diff_state.cursor_line;
        let commit_set = self.selected_commit_set();
        match self.line_annotations.get(target) {
            Some(AnnotatedLine::ReviewComment { comment_idx }) => Some(CommentLocation::Review {
                index: *comment_idx,
            }),
            Some(AnnotatedLine::FileComment {
                file_idx,
                comment_idx,
            }) => {
                let path = self.diff_files.get(*file_idx)?.display_path().clone();
                // Guard against stale annotations from an async commit-selection
                // reload: if the comment is no longer visible under the current
                // selection, treat the cursor as not on a comment.
                let visible = self
                    .session
                    .files
                    .get(&path)
                    .and_then(|r| r.file_comments.get(*comment_idx))
                    .is_some_and(|c| Self::comment_visible_with(c, commit_set.as_ref()));
                if !visible {
                    return None;
                }
                Some(CommentLocation::File {
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
                let visible = self
                    .session
                    .files
                    .get(&path)
                    .and_then(|r| r.line_comments.get(line))
                    .and_then(|c| c.get(*comment_idx))
                    .is_some_and(|c| Self::comment_visible_with(c, commit_set.as_ref()));
                if !visible {
                    return None;
                }
                Some(CommentLocation::Line {
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
            Some(CommentLocation::Review { index })
                if index < self.session.review_comments.len() =>
            {
                self.session.review_comments.remove(index);
                self.dirty = true;
                self.set_message("Review comment deleted");
                self.rebuild_annotations();
                return true;
            }
            Some(CommentLocation::File { path, index }) => {
                if let Some(review) = self.session.get_file_mut(&path) {
                    review.file_comments.remove(index);
                    self.dirty = true;
                    self.set_message("Comment deleted");
                    self.rebuild_annotations();
                    return true;
                }
            }
            Some(CommentLocation::Line {
                path,
                line,
                side,
                index,
            }) => {
                if let Some(review) = self.session.get_file_mut(&path)
                    && let Some(comments) = review.line_comments.get_mut(&line)
                {
                    // `comment_idx` from the annotation is the absolute index
                    // into the stored Vec (see `push_comments`), so delete
                    // directly — no side-filtered re-count.
                    if index < comments.len() {
                        let comment_side = comments[index].side.unwrap_or(LineSide::New);
                        if comment_side == side {
                            comments.remove(index);
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
            }
            Some(CommentLocation::Review { .. }) | None => {}
        }

        false
    }

    pub fn clear_comments(&mut self, scope: ClearScope) {
        let (cleared, unreviewed) = self.session.clear_comments(scope);
        if cleared == 0 && unreviewed == 0 {
            self.set_message("No comments to clear");
            return;
        }

        self.dirty = true;
        self.rebuild_annotations();
        let msg = match (cleared, unreviewed) {
            (0, n) => format!("Unreviewed {n} files"),
            (c, 0) => format!("Cleared {c} comments"),
            (c, n) => format!("Cleared {c} comments, unreviewed {n} files"),
        };
        self.set_message(msg);
    }

    /// True if two annotation rows belong to the same rendered comment.
    /// `AnnotatedLine` is not `Eq`, so compare the identifying fields.
    fn same_comment(a: &AnnotatedLine, b: &AnnotatedLine) -> bool {
        use AnnotatedLine::{FileComment, LineComment, ReviewComment};
        match (a, b) {
            (ReviewComment { comment_idx: x }, ReviewComment { comment_idx: y }) => x == y,
            (
                FileComment {
                    file_idx: f1,
                    comment_idx: c1,
                },
                FileComment {
                    file_idx: f2,
                    comment_idx: c2,
                },
            ) => f1 == f2 && c1 == c2,
            (
                LineComment {
                    file_idx: f1,
                    line: l1,
                    side: s1,
                    comment_idx: c1,
                },
                LineComment {
                    file_idx: f2,
                    line: l2,
                    side: s2,
                    comment_idx: c2,
                },
            ) => f1 == f2 && l1 == l2 && s1 == s2 && c1 == c2,
            _ => false,
        }
    }

    /// First annotation row of the comment rendered at `cursor_line` (or
    /// `cursor_line` itself when it isn't on a comment).
    pub(in crate::app) fn comment_block_start(&self, cursor_line: usize) -> usize {
        let Some(cur) = self.line_annotations.get(cursor_line) else {
            return cursor_line;
        };
        let mut start = cursor_line;
        while start > 0
            && self
                .line_annotations
                .get(start - 1)
                .is_some_and(|prev| Self::same_comment(prev, cur))
        {
            start -= 1;
        }
        start
    }

    /// Byte offset in the loaded `comment_buffer` for the start
    /// (`cursor_at_end == false`) or end of the comment line the diff cursor is
    /// on. The comment's block begins at annotation row `block_start` (row 0 =
    /// top border, then one row per wrapped segment, then the bottom border).
    pub(in crate::app) fn comment_current_line_cursor(
        &self,
        block_start: usize,
        cursor_at_end: bool,
    ) -> usize {
        let content = &self.comment_buffer;
        let content_area = self.diff_state.viewport_width.saturating_sub(10);
        // Visual content row under the cursor (skip the top border at row 0).
        let visual_target = self
            .diff_state
            .cursor_line
            .saturating_sub(block_start)
            .saturating_sub(1);

        let mut visual = 0usize;
        let mut byte = 0usize;
        let mut line_start = 0usize;
        let mut line_len = 0usize;
        for line in content.split('\n') {
            line_start = byte;
            line_len = line.len();
            let segs = crate::ui::comment_panel::wrap_segments(line, content_area)
                .len()
                .max(1);
            if visual_target < visual + segs {
                return if cursor_at_end {
                    line_start + line_len
                } else {
                    line_start
                };
            }
            visual += segs;
            byte += line.len() + 1;
        }
        // Cursor on the bottom border or past the content: use the last line.
        if cursor_at_end {
            line_start + line_len
        } else {
            line_start
        }
    }

    /// Enter edit mode for the comment at the current cursor position.
    /// `cursor_at_end` places the text cursor at the end of the current comment
    /// line (vim `A` / the default non-vim behavior); otherwise at its start
    /// (vim `i`). Returns true if a comment was found and edit mode entered.
    pub fn enter_edit_mode(&mut self, cursor_at_end: bool) -> bool {
        let location = self.find_comment_at_cursor();
        // First annotation row of the comment under the cursor, so we can place
        // the text cursor on the line the diff cursor is actually pointing at.
        let block_start = self.comment_block_start(self.diff_state.cursor_line);

        match location {
            Some(CommentLocation::Review { index }) => {
                if let Some(comment) = self.session.review_comments.get(index) {
                    self.input_mode = InputMode::Comment;
                    self.diff_state.scroll_x = 0;
                    self.comment_buffer = comment.content.clone();
                    self.comment_cursor =
                        self.comment_current_line_cursor(block_start, cursor_at_end);
                    self.comment_type = comment.comment_type.clone();
                    self.comment_is_review_level = true;
                    self.comment_is_file_level = false;
                    self.comment_line = None;
                    self.editing_comment_id = Some(comment.id.clone());
                    return true;
                }
            }
            Some(CommentLocation::File { path, index }) => {
                if let Some(review) = self.session.files.get(&path)
                    && let Some(comment) = review.file_comments.get(index)
                {
                    self.input_mode = InputMode::Comment;
                    self.diff_state.scroll_x = 0;
                    self.comment_buffer = comment.content.clone();
                    self.comment_cursor =
                        self.comment_current_line_cursor(block_start, cursor_at_end);
                    self.comment_type = comment.comment_type.clone();
                    self.comment_is_review_level = false;
                    self.comment_is_file_level = true;
                    self.comment_line = None;
                    self.editing_comment_id = Some(comment.id.clone());
                    return true;
                }
            }
            Some(CommentLocation::Line {
                path,
                line,
                side,
                index,
            }) => {
                if let Some(review) = self.session.files.get(&path)
                    && let Some(comments) = review.line_comments.get(&line)
                {
                    // `comment_idx` from the annotation is the absolute index
                    // into the stored Vec (see `push_comments`); look it up
                    // directly, verifying the side matches.
                    if let Some(comment) = comments.get(index)
                        && comment.side.unwrap_or(LineSide::New) == side
                    {
                        self.input_mode = InputMode::Comment;
                        self.diff_state.scroll_x = 0;
                        self.comment_buffer = comment.content.clone();
                        self.comment_cursor =
                            self.comment_current_line_cursor(block_start, cursor_at_end);
                        self.comment_type = comment.comment_type.clone();
                        self.comment_is_review_level = false;
                        self.comment_is_file_level = false;
                        self.comment_line = Some((line, side));
                        self.editing_comment_id = Some(comment.id.clone());
                        return true;
                    }
                }
            }
            None => {}
        }

        false
    }

    pub fn enter_comment_mode(&mut self, file_level: bool, line: Option<(u32, LineSide)>) {
        self.input_mode = InputMode::Comment;
        // Snap horizontal scroll back to the left edge so the inline input
        // box renders inside the viewport on long lines.
        self.diff_state.scroll_x = 0;
        self.comment_buffer.clear();
        self.comment_cursor = 0;
        self.comment_type = self.default_comment_type();
        self.comment_is_review_level = false;
        self.comment_is_file_level = file_level;
        self.comment_line = line;
    }

    pub fn enter_review_comment_mode(&mut self) {
        self.input_mode = InputMode::Comment;
        self.diff_state.scroll_x = 0;
        self.comment_buffer.clear();
        self.comment_cursor = 0;
        self.comment_type = self.default_comment_type();
        self.comment_is_review_level = true;
        self.comment_is_file_level = false;
        self.comment_line = None;
        self.comment_line_range = None;
        self.editing_comment_id = None;
    }

    pub fn exit_comment_mode(&mut self) {
        self.input_mode = InputMode::Normal;
        self.comment_buffer.clear();
        self.comment_cursor = 0;
        self.comment_vim_editor = None;
        self.comment_vim_command = None;
        self.comment_vim_pending = CommentVimPending::None;
        self.comment_is_review_level = false;
        self.editing_comment_id = None;
        self.comment_line_range = None;
    }

    pub fn save_comment(&mut self) {
        if self.comment_buffer.trim().is_empty() {
            self.set_message("Comment cannot be empty");
            return;
        }

        let content = self.comment_buffer.trim().to_string();

        let mut message = "Error: Could not save comment".to_string();
        let mut autosave_error = None;

        // Check if we're editing an existing comment
        if let Some(editing_id) = &self.editing_comment_id {
            if let Some(comment) = self
                .session
                .review_comments
                .iter_mut()
                .find(|c| &c.id == editing_id)
            {
                comment.content = content.clone();
                comment.comment_type = self.comment_type.clone();
                message = "Review comment updated".to_string();
            } else if let Some(path) = self.current_file_path().cloned()
                && let Some(review) = self.session.get_file_mut(&path)
            {
                if let Some(comment) = review
                    .file_comments
                    .iter_mut()
                    .find(|c| &c.id == editing_id)
                {
                    comment.content = content.clone();
                    comment.comment_type = self.comment_type.clone();
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
                        comment.comment_type = self.comment_type.clone();
                        message = if let Some((line, _)) = self.comment_line {
                            format!("Comment on line {line} updated")
                        } else {
                            "Comment updated".to_string()
                        };
                    } else {
                        message = "Error: Comment to edit not found".to_string();
                    }
                }
            }
        } else if self.comment_is_review_level {
            let request = AddCommentRequest {
                target: CommentTarget::Review,
                content,
                comment_type: self.comment_type.clone(),
                author: self.username.clone(),
                commit_id: None,
            };
            message = match add_comment_to_session(&mut self.session, request) {
                Ok(_) => "Review comment added".to_string(),
                Err(e) => format!("Error: Could not save comment: {e}"),
            };
        } else if let Some(path) = self.current_file_path().cloned() {
            let (target, success_message) = if self.comment_is_file_level {
                (
                    CommentTarget::File { path },
                    "File comment added".to_string(),
                )
            } else if let Some((range, side)) = self.comment_line_range {
                let message = if range.is_single() {
                    format!("Comment added to line {}", range.end)
                } else {
                    format!("Comment added to lines {}-{}", range.start, range.end)
                };
                (CommentTarget::LineRange { path, range, side }, message)
            } else if let Some((line, side)) = self.comment_line {
                (
                    CommentTarget::Line { path, line, side },
                    format!("Comment added to line {line}"),
                )
            } else {
                (
                    CommentTarget::File { path },
                    "File comment added".to_string(),
                )
            };

            let request = AddCommentRequest {
                target,
                content,
                comment_type: self.comment_type.clone(),
                author: self.username.clone(),
                commit_id: self.commit_id_for_new_comment(),
            };
            message = match add_comment_to_session(&mut self.session, request) {
                Ok(_) => success_message,
                Err(e) => format!("Error: Could not save comment: {e}"),
            };
        }

        if !message.starts_with("Error:") {
            self.dirty = true;
            if let Err(e) = self.save_current_session_merging_external() {
                autosave_error = Some(format!("{message}; autosave failed: {e}"));
            }
        }
        if let Some(error) = autosave_error {
            self.set_error(error);
        } else {
            self.set_message(message);
        }
        self.rebuild_annotations();

        self.exit_comment_mode();
    }

    pub fn cycle_comment_type(&mut self) {
        if self.comment_types.is_empty() {
            return;
        }
        if self.comment_types.len() == 1 {
            self.set_message("Only one comment type configured");
            return;
        }

        let current_id = self.comment_type.id().to_string();
        let current_index = self
            .comment_types
            .iter()
            .position(|comment_type| comment_type.id == current_id)
            .unwrap_or(0);
        let next_index = (current_index + 1) % self.comment_types.len();
        let next_id = self.comment_types[next_index].id.clone();
        self.comment_type = CommentType::from_id(&next_id);
        self.announce_comment_type();
    }

    pub fn cycle_comment_type_reverse(&mut self) {
        if self.comment_types.is_empty() {
            return;
        }
        if self.comment_types.len() == 1 {
            self.set_message("Only one comment type configured");
            return;
        }

        let current_id = self.comment_type.id();
        let current_index = self
            .comment_types
            .iter()
            .position(|comment_type| comment_type.id == current_id)
            .unwrap_or(0);
        let prev_index = if current_index == 0 {
            self.comment_types.len() - 1
        } else {
            current_index - 1
        };
        self.comment_type = CommentType::from_id(&self.comment_types[prev_index].id);
        self.announce_comment_type();
    }

    /// Emit a status message naming the current comment type. `None` has an
    /// empty label, so fall back to its id (`none`) for legible feedback.
    fn announce_comment_type(&mut self) {
        let comment_type = self.comment_type.clone();
        let label = self.comment_type_label(&comment_type);
        let display = if label.is_empty() {
            comment_type.id().to_string()
        } else {
            label
        };
        self.set_message(format!("Comment type: {display}"));
    }
}
