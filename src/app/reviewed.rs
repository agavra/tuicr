use super::*;

impl App {
    pub fn can_stage(&self) -> bool {
        matches!(
            self.diff_source,
            DiffSource::Unstaged | DiffSource::StagedAndUnstaged
        )
    }

    pub fn stage_reviewed_files(&mut self) {
        if !self.can_stage() {
            self.set_error("Staging only available when viewing unstaged diffs");
            return;
        }
        let reviewed_paths: Vec<_> = self
            .session
            .files
            .iter()
            .filter(|(_, review)| review.reviewed)
            .map(|(path, _)| path.clone())
            .collect();
        if reviewed_paths.is_empty() {
            self.set_warning("No reviewed files to stage");
            return;
        }
        let mut staged = 0;
        for path in &reviewed_paths {
            if let Err(e) = self.vcs.stage_file(path) {
                self.set_error(format!("Failed to stage {}: {e}", path.display()));
                return;
            }
            staged += 1;
        }
        self.set_message(format!("Staged {} reviewed file(s)", staged));
        if let Err(TuicrError::NoChanges) = self.reload_diff_files() {
            self.diff_files.clear();
            self.diff_state = DiffState::default();
            self.file_list_state = FileListState::default();
            self.clear_expanded_gaps();
            self.rebuild_annotations();
        }
    }

    pub fn current_file(&self) -> Option<&DiffFile> {
        self.diff_files.get(self.diff_state.current_file_idx)
    }

    pub fn current_file_path(&self) -> Option<&PathBuf> {
        self.current_file().map(|f| f.display_path())
    }

    /// Takes the queued editor target after action dispatch.
    ///
    /// The main event loop consumes this after leaving raw mode and the
    /// alternate screen,
    /// because `App` does not own terminal state.
    pub fn take_pending_editor_target(&mut self) -> Option<EditorTarget> {
        self.pending_editor_target.take()
    }

    /// Resolves the currently focused UI item into an editor target.
    ///
    /// The resolved target is queued on `pending_editor_target` so the main
    /// event loop can perform the terminal handoff.
    /// Invalid focus states are reported through the status bar instead.
    pub fn queue_editor_for_focused_item(&mut self) {
        match self.focused_panel {
            FocusedPanel::FileList => match self.get_selected_tree_item() {
                Some(FileTreeItem::File { file_idx, .. }) => {
                    self.queue_editor_for_file_idx(file_idx, None)
                }
                Some(FileTreeItem::Directory { .. }) => {
                    self.set_warning("Select a file to open in editor");
                }
                None => self.set_warning("No file selected"),
            },
            FocusedPanel::Diff => {
                let annotation = self.line_annotations.get(self.diff_state.cursor_line);
                let file_idx = match annotation {
                    Some(
                        AnnotatedLine::Expander { gap_id, .. }
                        | AnnotatedLine::HiddenLines { gap_id, .. }
                        | AnnotatedLine::ExpandedContext { gap_id, .. },
                    ) => gap_id.file_idx,
                    Some(annotation) => {
                        annotation_file_idx(annotation).unwrap_or(self.diff_state.current_file_idx)
                    }
                    None => self.diff_state.current_file_idx,
                };
                let line = match annotation {
                    Some(AnnotatedLine::ExpandedContext { gap_id, line_idx }) => self
                        .get_expanded_line(gap_id, *line_idx)
                        .and_then(|line| line.new_lineno.or(line.old_lineno)),
                    _ => self.get_line_at_cursor().map(|(line, _side)| line),
                };
                self.queue_editor_for_file_idx(file_idx, line);
            }
            FocusedPanel::Comments | FocusedPanel::CommitSelector => {
                self.set_warning("Focus a file or diff line to open in editor");
            }
        }
    }

    fn queue_editor_for_file_idx(&mut self, file_idx: usize, line: Option<u32>) {
        let Some(file) = self.diff_files.get(file_idx) else {
            self.set_warning("No file selected");
            return;
        };
        if file.is_commit_message {
            self.set_warning("Commit message has no local file to open");
            return;
        }

        let display_path = file.display_path().clone();
        // PR mode uses a synthetic forge root when there is no local checkout
        // to hand to an editor.
        if !self.vcs_info.root_path.is_absolute() {
            self.set_warning(format!(
                "Cannot open {}: no local checkout",
                display_path.display()
            ));
            return;
        }

        let path = self.vcs_info.root_path.join(&display_path);
        // Deleted files and remote-only PR files have diff rows,
        // but no worktree file the external editor can open.
        if !path.exists() {
            self.set_warning(format!(
                "Cannot open {}: file does not exist",
                path.display()
            ));
            return;
        }

        self.pending_editor_target = Some(EditorTarget { path, line });
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

            // Update current_file_idx before rebuilding annotations:
            // single-file view filters annotations against it.
            if adjust_cursor {
                self.diff_state.current_file_idx = file_idx;
            }
            self.rebuild_annotations();

            if adjust_cursor {
                let header_line = self.calculate_file_scroll_offset(file_idx);
                self.diff_state.cursor_line = header_line;
                self.ensure_cursor_visible();
            }
        }
    }

    fn hunk_at_cursor(&self) -> Option<(usize, usize)> {
        match self.line_annotations.get(self.diff_state.cursor_line)? {
            AnnotatedLine::HunkHeader { file_idx, hunk_idx }
            | AnnotatedLine::DiffLine {
                file_idx, hunk_idx, ..
            }
            | AnnotatedLine::SideBySideLine {
                file_idx, hunk_idx, ..
            } => Some((*file_idx, *hunk_idx)),
            _ => None,
        }
    }

    fn hunk_review_target(&self, file_idx: usize, hunk_idx: usize) -> Option<(PathBuf, String)> {
        let file = self.diff_files.get(file_idx)?;
        let key = file.hunk_review_key(hunk_idx)?;
        Some((file.display_path().clone(), key))
    }

    pub(in crate::app) fn hunk_header_line(
        &self,
        file_idx: usize,
        hunk_idx: usize,
    ) -> Option<usize> {
        self.line_annotations.iter().position(|line| {
            matches!(
                line,
                AnnotatedLine::HunkHeader {
                    file_idx: candidate_file_idx,
                    hunk_idx: candidate_hunk_idx
                } if *candidate_file_idx == file_idx && *candidate_hunk_idx == hunk_idx
            )
        })
    }

    pub fn is_hunk_reviewed(&self, file_idx: usize, hunk_idx: usize) -> bool {
        // Skip key computation (which hashes every hunk in the file) when this
        // file has no reviewed hunks — the common case for users not using `R`.
        let Some(file) = self.diff_files.get(file_idx) else {
            return false;
        };
        match self.session.files.get(file.display_path()) {
            Some(review) if !review.reviewed_hunks.is_empty() => {}
            _ => return false,
        }

        let Some((path, key)) = self.hunk_review_target(file_idx, hunk_idx) else {
            return false;
        };
        self.session.is_hunk_reviewed(&path, &key)
    }

    pub fn should_render_gap_before_hunk(&self, file_idx: usize, hunk_idx: usize) -> bool {
        // Reviewed hunks collapse as a complete review unit: their body and
        // adjoining hidden-context controls disappear with the header.
        !self.is_hunk_reviewed(file_idx, hunk_idx)
            && (hunk_idx == 0 || !self.is_hunk_reviewed(file_idx, hunk_idx - 1))
    }

    pub fn toggle_hunk_reviewed(&mut self) {
        let Some((file_idx, hunk_idx)) = self.hunk_at_cursor() else {
            self.set_warning("Move cursor to a hunk to toggle reviewed");
            return;
        };

        let Some((path, key)) = self.hunk_review_target(file_idx, hunk_idx) else {
            self.set_warning("Move cursor to a hunk to toggle reviewed");
            return;
        };

        let Some(review) = self.session.get_file_mut(&path) else {
            return;
        };

        let reviewed = review.toggle_hunk_reviewed(key);
        self.dirty = true;
        self.rebuild_annotations();
        self.diff_state.current_file_idx = file_idx;
        if let Some(tree_idx) = self.file_idx_to_tree_idx(file_idx) {
            self.file_list_state.select(tree_idx);
        }
        if let Some(header_line) = self.hunk_header_line(file_idx, hunk_idx) {
            self.diff_state.cursor_line = header_line;
        }
        self.ensure_cursor_visible();

        if reviewed {
            self.set_message("Hunk marked reviewed");
        } else {
            self.set_message("Hunk marked unreviewed");
        }
    }

    pub fn file_count(&self) -> usize {
        self.diff_files.len()
    }

    pub fn reviewed_count(&self) -> usize {
        self.session.reviewed_count()
    }

    /// Returns `(total_files, total_additions, total_deletions)` across all diff files.
    pub fn diff_stat(&self) -> (usize, usize, usize) {
        let mut additions = 0;
        let mut deletions = 0;
        for file in &self.diff_files {
            let (a, d) = file.stat();
            additions += a;
            deletions += d;
        }
        (self.diff_files.len(), additions, deletions)
    }

    /// Returns true when the cursor is in the review comments area above all files.
    pub fn is_cursor_in_overview(&self) -> bool {
        self.diff_state.cursor_line < self.review_comments_render_height()
    }
}
