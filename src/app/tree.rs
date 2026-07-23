use super::*;

impl App {
    pub fn file_list_down(&mut self, n: usize) {
        let visible_items = self.build_visible_items();
        let max_idx = visible_items.len().saturating_sub(1);
        let new_idx = (self.file_list_state.selected() + n).min(max_idx);
        self.file_list_state.select(new_idx);
        self.follow_file_list_in_single_file_view();
    }

    pub fn file_list_up(&mut self, n: usize) {
        let new_idx = self.file_list_state.selected().saturating_sub(n);
        self.file_list_state.select(new_idx);
        self.follow_file_list_in_single_file_view();
    }

    /// In single-file view the diff panel always shows one file at a time,
    /// so navigating the file list with j/k should reveal the highlighted
    /// file immediately instead of waiting for Enter. Skips directories
    /// (jumping there has no diff target) and no-ops outside single-file
    /// view to keep multi-file scrolling exactly as before.
    fn follow_file_list_in_single_file_view(&mut self) {
        if !self.is_single_file_view {
            return;
        }
        if let Some(FileTreeItem::File { file_idx, .. }) = self.get_selected_tree_item() {
            self.jump_to_file(file_idx);
        }
    }

    /// Scroll the file-list viewport down by `lines` without moving the
    /// selection unless it would fall off the top of the viewport.
    pub fn file_list_viewport_scroll_down(&mut self, lines: usize) {
        let total = self.build_visible_items().len();
        let viewport = self.file_list_state.viewport_height.max(1);
        let max_offset = total.saturating_sub(viewport);
        let new_offset = (self.file_list_state.list_state.offset() + lines).min(max_offset);
        *self.file_list_state.list_state.offset_mut() = new_offset;
        if self.file_list_state.selected() < new_offset {
            self.file_list_state.select(new_offset);
        }
    }

    /// Scroll the file-list viewport up by `lines` without moving the
    /// selection unless it would fall off the bottom of the viewport.
    pub fn file_list_viewport_scroll_up(&mut self, lines: usize) {
        let viewport = self.file_list_state.viewport_height.max(1);
        let new_offset = self
            .file_list_state
            .list_state
            .offset()
            .saturating_sub(lines);
        *self.file_list_state.list_state.offset_mut() = new_offset;
        let max_visible = (new_offset + viewport).saturating_sub(1);
        if self.file_list_state.selected() > max_visible {
            self.file_list_state.select(max_visible);
        }
    }

    pub fn file_list_idx_at_screen_row(&self, screen_row: u16) -> Option<usize> {
        let inner = self.file_list_inner_area?;
        if screen_row < inner.y || screen_row >= inner.y + inner.height {
            return None;
        }
        let rel = (screen_row - inner.y) as usize;
        let idx = self.file_list_state.list_state.offset() + rel;
        let total = self.build_visible_items().len();
        (idx < total).then_some(idx)
    }

    pub fn toggle_diff_view_mode(&mut self) {
        if self.is_pristine_mode {
            // Side-by-side has nothing to show in pristine mode: there is no
            // diff, so the two panes would render identical content. Keep
            // the view unified and tell the user why the toggle did nothing.
            self.set_message("side-by-side not available in pristine mode");
            return;
        }
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
        if !self.show_file_list
            && matches!(
                self.focused_panel,
                FocusedPanel::FileList | FocusedPanel::Comments
            )
        {
            self.focused_panel = FocusedPanel::Diff;
        }
        let status = if self.show_file_list {
            "visible"
        } else {
            "hidden"
        };
        self.set_message(format!("File list: {status}"));
    }

    /// Toggle single-file view. When on, the diff panel renders only the
    /// currently focused file instead of the full continuous-scroll
    /// concatenation. Annotations, navigation, and export work the same
    /// way on the rendered subset.
    pub fn toggle_single_file_view(&mut self) {
        self.is_single_file_view = !self.is_single_file_view;
        // Rebuild first so the file-range index reflects the new view mode,
        // then re-snap the viewport to the current file.
        self.rebuild_annotations();
        let start = self.calculate_file_scroll_offset(self.diff_state.current_file_idx);
        self.diff_state.scroll_offset = start;
        self.diff_state.cursor_line = start;
        let status = if self.is_single_file_view {
            "single file"
        } else {
            "all files"
        };
        self.set_message(format!("View: {status}"));
    }

    pub(in crate::app) fn sort_files_by_directory(&mut self, reset_position: bool) {
        use std::collections::BTreeMap;
        use std::path::Path;

        self.file_line_count_cache.clear();

        let current_path = if !reset_position {
            self.current_file_path().cloned()
        } else {
            None
        };

        let mut dir_map: BTreeMap<String, Vec<DiffFile>> = BTreeMap::new();
        let mut commit_msg_files: Vec<DiffFile> = Vec::new();

        for file in self.diff_files.drain(..) {
            if file.is_commit_message {
                commit_msg_files.push(file);
                continue;
            }
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

        self.diff_files.extend(commit_msg_files);
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

        // Start at the overview position (review comments header)
        // so the diff title shows total stats on launch.
        self.diff_state.cursor_line = 0;
        self.diff_state.scroll_offset = 0;
        self.diff_state.current_file_idx = 0;
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
