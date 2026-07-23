use super::*;
use crate::ui::row_height::annotation_row_height;

impl App {
    pub fn cursor_down(&mut self, lines: usize) {
        let max_line = self.max_cursor_line();
        let prev_cursor = self.diff_state.cursor_line;
        let prev_scroll = self.diff_state.scroll_offset;
        let target = self.diff_state.cursor_line + lines;
        // Single-file view: first overflow press arms `primed_walk_next`
        // and parks the cursor on max. On kitty terminals the walk
        // consumes only when the Down key was released between the two
        // presses (`down_released_since_arm`) -- a held-j sequence emits
        // Press, Repeat, Repeat, ..., Release and never satisfies the
        // gate. Non-kitty terminals don't emit Release, so the gate
        // bypasses on `!supports_keyboard_enhancement` to keep the
        // two-press walk usable without modern keyboard reporting.
        self.primed_walk_prev = false;
        if self.is_single_file_view && target > max_line {
            let release_gate_ok =
                !self.supports_keyboard_enhancement || self.down_released_since_arm;
            if self.primed_walk_next && release_gate_ok {
                let next_idx = self.diff_state.current_file_idx + 1;
                if next_idx < self.diff_files.len() {
                    let overflow = target - max_line;
                    self.primed_walk_next = false;
                    self.down_released_since_arm = false;
                    self.jump_to_file(next_idx);
                    let new_top = self.diff_state.cursor_line;
                    let new_max = self.max_cursor_line();
                    self.diff_state.cursor_line =
                        (new_top + overflow.saturating_sub(1)).min(new_max);
                    self.ensure_cursor_visible();
                    return;
                }
                self.primed_walk_next = false;
                self.down_released_since_arm = false;
            } else {
                self.primed_walk_next = true;
                self.down_released_since_arm = false;
                self.diff_state.cursor_line = max_line;
                self.ensure_cursor_visible();
                return;
            }
        } else if target != prev_cursor {
            self.primed_walk_next = false;
            self.down_released_since_arm = false;
        }
        self.diff_state.cursor_line = target.min(max_line);
        if self.diff_state.cursor_line != prev_cursor {
            self.ensure_cursor_visible();
            // Cap scroll change to cursor movement to prevent multi-line jumps
            // when the view is catching up from a non-steady-state position.
            let cursor_moved = self.diff_state.cursor_line - prev_cursor;
            if self.diff_state.scroll_offset > prev_scroll + cursor_moved {
                self.diff_state.scroll_offset = prev_scroll + cursor_moved;
            }
        }
        self.update_current_file_from_cursor();
    }

    pub fn cursor_up(&mut self, lines: usize) {
        // Symmetric to cursor_down: first underflow arms `primed_walk_prev`,
        // second underflow consumes only after a Up/k release event has set
        // `up_released_since_arm`. See cursor_down for the gate rationale.
        self.primed_walk_next = false;
        if self.is_single_file_view {
            let file_top = self.calculate_file_scroll_offset(self.diff_state.current_file_idx);
            if self.diff_state.cursor_line < file_top + lines
                && self.diff_state.current_file_idx > 0
            {
                let release_gate_ok =
                    !self.supports_keyboard_enhancement || self.up_released_since_arm;
                if self.primed_walk_prev && release_gate_ok {
                    let underflow = (file_top + lines) - self.diff_state.cursor_line;
                    let prev_idx = self.diff_state.current_file_idx - 1;
                    self.primed_walk_prev = false;
                    self.up_released_since_arm = false;
                    self.jump_to_file(prev_idx);
                    let new_max = self.max_cursor_line();
                    self.diff_state.cursor_line =
                        new_max.saturating_sub(underflow.saturating_sub(1));
                    self.ensure_cursor_visible();
                    return;
                }
                self.primed_walk_prev = true;
                self.up_released_since_arm = false;
                self.diff_state.cursor_line = file_top;
                self.ensure_cursor_visible();
                return;
            }
            self.primed_walk_prev = false;
            self.up_released_since_arm = false;
        }
        self.diff_state.cursor_line = self.diff_state.cursor_line.saturating_sub(lines);
        let visible_lines = self.diff_state.effective_visible_lines();
        let scroll_margin = self.diff_state.effective_scroll_margin(self.scroll_offset);
        // Enforce top margin
        if self.diff_state.cursor_line < self.diff_state.scroll_offset + scroll_margin {
            self.diff_state.scroll_offset =
                self.diff_state.cursor_line.saturating_sub(scroll_margin);
        }
        // Ensure cursor is at least within the viewport (no bottom margin enforcement,
        // just basic visibility — handles viewport shrink or wrap-mode changes).
        if self.diff_state.cursor_line >= self.diff_state.scroll_offset + visible_lines {
            self.diff_state.scroll_offset = self.diff_state.cursor_line - visible_lines + 1;
        }
        self.update_current_file_from_cursor();
    }

    pub fn scroll_down(&mut self, lines: usize) {
        // For half-page/page scrolling, move both cursor and scroll
        let max_line = self.max_cursor_line();
        let max_scroll = self.max_scroll_offset();
        self.diff_state.cursor_line = (self.diff_state.cursor_line + lines).min(max_line);
        self.diff_state.cursor_line = skip_decoration_forward(
            &self.line_annotations,
            self.diff_state.cursor_line,
            max_line,
        );
        self.diff_state.scroll_offset = (self.diff_state.scroll_offset + lines).min(max_scroll);
        self.ensure_cursor_visible();
        self.update_current_file_from_cursor();
    }

    pub fn scroll_up(&mut self, lines: usize) {
        // For half-page/page scrolling, move both cursor and scroll
        self.diff_state.cursor_line = self.diff_state.cursor_line.saturating_sub(lines);
        self.diff_state.cursor_line =
            skip_decoration_backward(&self.line_annotations, self.diff_state.cursor_line);
        self.diff_state.scroll_offset = self.diff_state.scroll_offset.saturating_sub(lines);
        self.ensure_cursor_visible();
        self.update_current_file_from_cursor();
    }

    pub fn scroll_view_down(&mut self, lines: usize) {
        let max_scroll = self.max_scroll_offset();
        self.diff_state.scroll_offset = (self.diff_state.scroll_offset + lines).min(max_scroll);
        let scroll_margin = self.diff_state.effective_scroll_margin(self.scroll_offset);
        let min_cursor =
            (self.diff_state.scroll_offset + scroll_margin).min(self.max_cursor_line());
        if self.diff_state.cursor_line < min_cursor {
            self.diff_state.cursor_line = min_cursor;
            self.update_current_file_from_cursor();
        }
    }

    pub fn scroll_view_up(&mut self, lines: usize) {
        self.diff_state.scroll_offset = self.diff_state.scroll_offset.saturating_sub(lines);
        let visible_lines = if self.diff_state.visible_line_count > 0 {
            self.diff_state.visible_line_count
        } else {
            self.diff_state.viewport_height.max(1)
        };
        let bottom = self.diff_state.scroll_offset + visible_lines.saturating_sub(1);
        if self.diff_state.cursor_line > bottom {
            self.diff_state.cursor_line = bottom;
            self.update_current_file_from_cursor();
        }
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

    /// Adjusts scroll_offset so the cursor stays within the visible viewport,
    /// respecting the configured scroll margin (minimum lines from edge).
    pub(in crate::app) fn ensure_cursor_visible(&mut self) {
        // Use visible_line_count which is computed during render based on actual line widths.
        // Falls back to viewport_height if not yet set (before first render).
        let visible_lines = self.diff_state.effective_visible_lines();
        let max_scroll = self.max_scroll_offset();
        let scroll_margin = self.diff_state.effective_scroll_margin(self.scroll_offset);
        // Cursor too close to the top edge — scroll up
        if self.diff_state.cursor_line < self.diff_state.scroll_offset + scroll_margin {
            self.diff_state.scroll_offset =
                self.diff_state.cursor_line.saturating_sub(scroll_margin);
        }
        // Cursor too close to the bottom edge — scroll down.
        // Reduce the margin near EOF so we don't scroll to show empty space
        // when the last line is already visible (matches Vim behavior).
        let lines_below = self
            .max_cursor_line()
            .saturating_sub(self.diff_state.cursor_line);
        let bottom_margin = scroll_margin.min(lines_below);
        if self.diff_state.cursor_line + bottom_margin
            >= self.diff_state.scroll_offset + visible_lines
        {
            self.diff_state.scroll_offset =
                (self.diff_state.cursor_line + bottom_margin - visible_lines + 1).min(max_scroll);
        }
    }

    pub(in crate::app) fn scroll_offset_for_rows_above(
        &self,
        anchor: usize,
        row_budget: usize,
    ) -> usize {
        let mut result = anchor;
        let mut acc: usize = 0;
        let mut k = anchor;
        while k > 0 {
            k -= 1;
            let h = annotation_row_height(self, k);
            if acc + h > row_budget {
                break;
            }
            acc += h;
            result = k;
        }
        result
    }

    pub(crate) fn page_lines_down(&self, row_budget: usize) -> usize {
        let anchor = self.diff_state.cursor_line;
        let total = self.line_annotations.len();
        let mut acc: usize = 0;
        let mut count: usize = 0;
        let mut k = anchor + 1;
        while k < total {
            let h = annotation_row_height(self, k);
            if acc + h > row_budget {
                break;
            }
            acc += h;
            count += 1;
            k += 1;
        }
        count.max(1)
    }

    pub(crate) fn page_lines_up(&self, row_budget: usize) -> usize {
        let anchor = self.diff_state.cursor_line;
        (anchor - self.scroll_offset_for_rows_above(anchor, row_budget)).max(1)
    }

    pub fn center_cursor(&mut self) {
        let viewport = self.diff_state.viewport_height.max(1);
        let max_scroll = self.max_scroll_offset();
        self.diff_state.scroll_offset = self
            .scroll_offset_for_rows_above(self.diff_state.cursor_line, viewport / 2)
            .min(max_scroll);
    }

    pub fn cursor_to_top(&mut self) {
        let scroll_margin = self.diff_state.effective_scroll_margin(self.scroll_offset);
        let max_scroll = self.max_scroll_offset();
        self.diff_state.scroll_offset = self
            .scroll_offset_for_rows_above(self.diff_state.cursor_line, scroll_margin)
            .min(max_scroll);
    }

    pub fn cursor_to_bottom(&mut self) {
        let viewport = self.diff_state.viewport_height.max(1);
        let scroll_margin = self.diff_state.effective_scroll_margin(self.scroll_offset);
        let cursor = self.diff_state.cursor_line;
        let budget = viewport.saturating_sub(scroll_margin + annotation_row_height(self, cursor));
        let max_scroll = self.max_scroll_offset();
        self.diff_state.scroll_offset = self
            .scroll_offset_for_rows_above(cursor, budget)
            .min(max_scroll);
    }

    pub fn go_to_source_line(&mut self, target_lineno: u32, side: LineSide) {
        let current_file = self.diff_state.current_file_idx;
        let mut result = self.find_source_line_in_diff(target_lineno, side);
        let side_label = match side {
            LineSide::New => "",
            LineSide::Old => " (old)",
        };

        // If the line isn't already annotated, see whether it falls inside a
        // collapsed (or partially collapsed) gap between hunks. If so, expand
        // *toward* the target from whichever side the cursor is on: cursor
        // above the gap expands `Down` from the previous hunk, cursor at or
        // below the gap expands `Up` from the next hunk. Either way the
        // unreached half of the gap stays collapsed behind an expander.
        if !matches!(result, FindSourceLineResult::Exact(_))
            && let Some(gap_id) = self.find_gap_containing_lineno(current_file, target_lineno, side)
        {
            let (direction, limit) = self.expand_plan_to_reach(&gap_id, target_lineno, side);
            if let Err(e) = self.expand_gap(gap_id, direction, limit) {
                self.set_error(format!("Expand failed: {e}"));
                return;
            }
            result = self.find_source_line_in_diff(target_lineno, side);
        }

        match result {
            FindSourceLineResult::Exact(idx) | FindSourceLineResult::Nearest(idx) => {
                self.diff_state.cursor_line = idx;
                self.ensure_cursor_visible();
                self.center_cursor();
                self.update_current_file_from_cursor();
                if matches!(result, FindSourceLineResult::Nearest(_)) {
                    self.set_message(format!(
                        "Line {target_lineno}{side_label} not in diff, jumped to nearest"
                    ));
                }
            }
            FindSourceLineResult::NotFound => {
                self.set_warning(format!(
                    "Line {target_lineno}{side_label} not found in current file"
                ));
            }
        }
    }

    /// Like the free `find_source_line` but also resolves `ExpandedContext`
    /// annotations through `get_expanded_line` so newly-revealed context lines
    /// count toward the match.
    fn find_source_line_in_diff(&self, target_lineno: u32, side: LineSide) -> FindSourceLineResult {
        let current_file = self.diff_state.current_file_idx;
        let mut best: Option<(usize, u32)> = None;

        for (idx, annotation) in self.line_annotations.iter().enumerate() {
            let (file_idx, candidate) = match annotation {
                AnnotatedLine::DiffLine {
                    file_idx,
                    old_lineno,
                    new_lineno,
                    ..
                }
                | AnnotatedLine::SideBySideLine {
                    file_idx,
                    old_lineno,
                    new_lineno,
                    ..
                } => {
                    let c = match side {
                        LineSide::New => *new_lineno,
                        LineSide::Old => *old_lineno,
                    };
                    (*file_idx, c)
                }
                AnnotatedLine::ExpandedContext { gap_id, line_idx } => {
                    let Some(line) = self.get_expanded_line(gap_id, *line_idx) else {
                        continue;
                    };
                    let c = match side {
                        LineSide::New => line.new_lineno,
                        LineSide::Old => line.old_lineno,
                    };
                    (gap_id.file_idx, c)
                }
                _ => continue,
            };
            if file_idx != current_file {
                continue;
            }
            if let Some(ln) = candidate {
                let dist = ln.abs_diff(target_lineno);
                if dist == 0 {
                    return FindSourceLineResult::Exact(idx);
                }
                if best.is_none() || dist < best.unwrap().1 {
                    best = Some((idx, dist));
                }
            }
        }

        match best {
            Some((idx, _)) => FindSourceLineResult::Nearest(idx),
            None => FindSourceLineResult::NotFound,
        }
    }

    pub fn diff_annotation_at_screen_row(&self, screen_row: u16) -> Option<usize> {
        let inner = self.diff_inner_area?;
        if screen_row < inner.y || screen_row >= inner.y + inner.height {
            return None;
        }
        let rel = (screen_row - inner.y) as usize;
        self.diff_row_to_annotation.get(rel).copied()
    }

    /// Syncs `current_file_idx` so the file list selection follows when the
    /// new cursor lands on an annotation belonging to a file.
    pub fn move_cursor_to_annotation(&mut self, idx: usize) {
        if idx >= self.line_annotations.len() {
            return;
        }
        self.diff_state.cursor_line = idx;
        if let Some(file_idx) = annotation_file_idx(&self.line_annotations[idx]) {
            self.diff_state.current_file_idx = file_idx;
        }
        let viewport = self.diff_state.viewport_height.max(1);
        if idx < self.diff_state.scroll_offset {
            self.diff_state.scroll_offset = idx;
        } else {
            let bottom_start = self.scroll_offset_for_rows_above(
                idx,
                viewport.saturating_sub(annotation_row_height(self, idx)),
            );
            if self.diff_state.scroll_offset < bottom_start {
                self.diff_state.scroll_offset = bottom_start;
            }
        }
    }

    /// In SBS, picks Old or New per `side`, falling back to the other pane
    /// if the requested one is empty. Unified diff rows ignore `side`.
    pub fn content_for_side(&self, ann_idx: usize, side: LineSide) -> Option<&str> {
        let ann = self.line_annotations.get(ann_idx)?;
        match ann {
            AnnotatedLine::DiffLine {
                file_idx,
                hunk_idx,
                line_idx,
                ..
            } => {
                let line = self
                    .diff_files
                    .get(*file_idx)?
                    .hunks
                    .get(*hunk_idx)?
                    .lines
                    .get(*line_idx)?;
                Some(line.content.as_str())
            }
            AnnotatedLine::SideBySideLine {
                file_idx,
                hunk_idx,
                del_line_idx,
                add_line_idx,
                ..
            } => {
                let hunk = self.diff_files.get(*file_idx)?.hunks.get(*hunk_idx)?;
                let add = add_line_idx
                    .and_then(|i| hunk.lines.get(i))
                    .map(|l| l.content.as_str());
                let del = del_line_idx
                    .and_then(|i| hunk.lines.get(i))
                    .map(|l| l.content.as_str());
                match side {
                    LineSide::New => add.or(del),
                    LineSide::Old => del.or(add),
                }
            }
            AnnotatedLine::ExpandedContext { gap_id, line_idx } => self
                .get_expanded_line(gap_id, *line_idx)
                .map(|l| l.content.as_str()),
            _ => None,
        }
    }

    /// For annotations rendered outside the content gutter (hunk headers,
    /// file headers): returns a clean copy text. The selection's char range
    /// is meaningless for these — they're emitted whole or not at all.
    pub(in crate::app) fn atomic_text_for_annotation(&self, ann_idx: usize) -> Option<String> {
        match self.line_annotations.get(ann_idx)? {
            AnnotatedLine::HunkHeader { file_idx, hunk_idx } => {
                let hunk = self.diff_files.get(*file_idx)?.hunks.get(*hunk_idx)?;
                Some(hunk.header.clone())
            }
            AnnotatedLine::FileHeader { file_idx } => {
                let file = self.diff_files.get(*file_idx)?;
                if file.is_commit_message {
                    Some(file.display_path().display().to_string())
                } else {
                    Some(format!(
                        "{} [{}]",
                        file.display_path().display(),
                        file.status.as_char()
                    ))
                }
            }
            _ => None,
        }
    }

    pub fn lineno_width(&self) -> usize {
        let hunk_max = self
            .diff_files
            .iter()
            .map(|f| f.max_lineno())
            .max()
            .unwrap_or(0);
        let cache_max = self
            .file_line_count_cache
            .values()
            .copied()
            .max()
            .unwrap_or(0);
        lineno_width(hunk_max.max(cache_max))
    }

    pub fn pane_geometry(&self, inner: ratatui::layout::Rect, side: LineSide) -> PaneGeom {
        let w = self.lineno_width();
        match self.diff_view_mode {
            DiffViewMode::Unified => {
                let gutter = unified_gutter(w);
                let content_width = (inner.width as usize).saturating_sub(gutter as usize);
                PaneGeom {
                    content_x_start: inner.x + gutter,
                    content_x_end: inner.x + inner.width,
                    content_width,
                }
            }
            DiffViewMode::SideBySide => {
                let overhead = sbs_overhead(w);
                let left_gutter = sbs_left_gutter(w);
                let half_w = (inner.width.saturating_sub(overhead) / 2) as usize;
                match side {
                    LineSide::Old => PaneGeom {
                        content_x_start: inner.x + left_gutter,
                        content_x_end: inner.x + left_gutter + half_w as u16,
                        content_width: half_w,
                    },
                    LineSide::New => {
                        let start = inner.x + overhead + half_w as u16;
                        PaneGeom {
                            content_x_start: start,
                            content_x_end: start + half_w as u16,
                            content_width: half_w,
                        }
                    }
                }
            }
        }
    }

    pub fn side_at_x(
        &self,
        inner: ratatui::layout::Rect,
        x: u16,
        ann_default: LineSide,
    ) -> LineSide {
        let w = self.lineno_width();
        match self.diff_view_mode {
            DiffViewMode::Unified => ann_default,
            DiffViewMode::SideBySide => {
                let half_w = inner.width.saturating_sub(sbs_overhead(w)) / 2;
                let divider = inner.x + sbs_left_gutter(w) + half_w;
                if x < divider {
                    LineSide::Old
                } else {
                    LineSide::New
                }
            }
        }
    }

    pub fn cell_to_sel_point(&self, screen_col: u16, screen_row: u16) -> Option<SelPoint> {
        let idx = self.diff_annotation_at_screen_row(screen_row)?;
        let inner = self.diff_inner_area?;
        let ann = self.line_annotations.get(idx)?;
        let side = self.side_at_x(inner, screen_col, annotation_side_default(ann));

        let zero_point = SelPoint {
            annotation_idx: idx,
            char_offset: 0,
            side,
        };
        let Some(content) = self.content_for_side(idx, side) else {
            return Some(zero_point);
        };
        let geom = self.pane_geometry(inner, side);
        if geom.content_width == 0 {
            return Some(zero_point);
        }
        let last_col = geom.content_x_end.saturating_sub(1);
        let col = screen_col.clamp(geom.content_x_start, last_col);
        let col_in_row = (col - geom.content_x_start) as usize;

        let rel = (screen_row - inner.y) as usize;
        let mut walker = rel;
        while walker > 0 && self.diff_row_to_annotation.get(walker - 1).copied() == Some(idx) {
            walker -= 1;
        }
        let which_row = rel - walker;
        let total_chars = content.chars().count();
        let char_offset = (which_row * geom.content_width + col_in_row).min(total_chars);
        Some(SelPoint {
            annotation_idx: idx,
            char_offset,
            side,
        })
    }

    /// Mirrors `ensure_cursor_visible`'s notion of visibility (uses the
    /// renderer's `visible_line_count` when present so wrapping is honored).
    pub fn is_cursor_visible(&self) -> bool {
        let visible = if self.diff_state.visible_line_count > 0 {
            self.diff_state.visible_line_count
        } else {
            self.diff_state.viewport_height.max(1)
        };
        let cursor = self.diff_state.cursor_line;
        cursor >= self.diff_state.scroll_offset && cursor < self.diff_state.scroll_offset + visible
    }

    pub fn jump_to_file(&mut self, idx: usize) {
        use std::path::Path;

        if idx < self.diff_files.len() {
            // Deliberate jump cancels any in-flight two-press walk arming.
            self.primed_walk_next = false;
            self.primed_walk_prev = false;
            self.down_released_since_arm = false;
            self.up_released_since_arm = false;
            self.diff_state.current_file_idx = idx;
            self.diff_state.cursor_line = self.calculate_file_scroll_offset(idx);
            self.diff_state.cursor_line = skip_decoration_forward(
                &self.line_annotations,
                self.diff_state.cursor_line,
                self.line_annotations.len().saturating_sub(1),
            );
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

            // Single-file view filters `line_annotations` by
            // `current_file_idx`, so a file switch must rebuild them or
            // click hit-testing would resolve against the previous file.
            if self.is_single_file_view {
                self.rebuild_annotations();
            }
        }
    }

    pub fn jump_to_bottom(&mut self) {
        let max_line = self.max_cursor_line();
        self.diff_state.cursor_line = max_line;
        // Position so the last navigable line is at the bottom of the viewport
        let viewport = self.diff_state.viewport_height.max(1);
        self.diff_state.scroll_offset = self.scroll_offset_for_rows_above(
            max_line,
            viewport.saturating_sub(annotation_row_height(self, max_line)),
        );
        self.update_current_file_from_cursor();
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

    pub(in crate::app) fn file_idx_to_tree_idx(&self, target_file_idx: usize) -> Option<usize> {
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

    /// Render-line indices of every visible hunk header. Respects
    /// single-file view (only the current file's hunks) and the
    /// reviewed-collapse behavior in multi-file view (skipped entirely)
    /// versus single-file view (body rendered under a banner).
    pub(in crate::app) fn hunk_positions(&self) -> Vec<usize> {
        let single = self.is_single_file_view;
        let current_idx = self.diff_state.current_file_idx;
        let mut positions = Vec::new();
        let mut cumulative = self.review_comments_render_height();
        for (file_idx, file) in self.diff_files.iter().enumerate() {
            if single && file_idx != current_idx {
                continue;
            }
            let path = file.display_path();
            let is_reviewed = self.session.is_file_reviewed(path);

            if !single {
                cumulative += 1; // File header
            }
            if !single && is_reviewed {
                // multi-file collapsed: no body, no trailing spacing
                continue;
            }
            if single && is_reviewed {
                cumulative += 1; // banner
            }
            if let Some(review) = self.session.files.get(path) {
                cumulative += review.file_comments.len();
            }
            if file.is_binary || file.hunks.is_empty() {
                cumulative += 1;
            } else {
                for (hunk_idx, hunk) in file.hunks.iter().enumerate() {
                    positions.push(cumulative);
                    cumulative += 1;
                    if !self.is_hunk_reviewed(file_idx, hunk_idx) {
                        cumulative += hunk.lines.len();
                    }
                }
            }
            cumulative += 1; // trailing spacing or "next file" hint
        }
        positions
    }

    pub fn next_hunk(&mut self) {
        // Hunk navigation is a deliberate move, not a continuation of a
        // boundary walk. Clear any in-flight cursor-walk arming.
        self.primed_walk_next = false;
        self.primed_walk_prev = false;
        self.down_released_since_arm = false;
        self.up_released_since_arm = false;
        for pos in self.hunk_positions() {
            if pos > self.diff_state.cursor_line {
                self.diff_state.cursor_line = pos;
                self.ensure_cursor_visible();
                self.update_current_file_from_cursor();
                return;
            }
        }
        // No further hunks in the current frame. Single-file view crosses
        // into the next file's first hunk so `]` can step the codebase
        // hunk-by-hunk without breaking on file boundaries.
        if self.is_single_file_view {
            let next_idx = self.diff_state.current_file_idx + 1;
            if next_idx < self.diff_files.len() {
                self.jump_to_file(next_idx);
                if let Some(&first) = self.hunk_positions().first() {
                    self.diff_state.cursor_line = first;
                    self.ensure_cursor_visible();
                    self.update_current_file_from_cursor();
                }
            }
        }
    }

    pub fn prev_hunk(&mut self) {
        self.primed_walk_next = false;
        self.primed_walk_prev = false;
        self.down_released_since_arm = false;
        self.up_released_since_arm = false;
        let positions = self.hunk_positions();
        for &pos in positions.iter().rev() {
            if pos < self.diff_state.cursor_line {
                self.diff_state.cursor_line = pos;
                self.ensure_cursor_visible();
                self.update_current_file_from_cursor();
                return;
            }
        }
        // Symmetric to next_hunk: in single-file view, fall through to the
        // previous file's last hunk so `[` keeps stepping backward across
        // files.
        if self.is_single_file_view && self.diff_state.current_file_idx > 0 {
            let prev_idx = self.diff_state.current_file_idx - 1;
            self.jump_to_file(prev_idx);
            if let Some(&last) = self.hunk_positions().last() {
                self.diff_state.cursor_line = last;
                self.ensure_cursor_visible();
                self.update_current_file_from_cursor();
                return;
            }
        }
        self.diff_state.cursor_line = 0;
        self.ensure_cursor_visible();
        self.update_current_file_from_cursor();
    }

    pub(in crate::app) fn calculate_file_scroll_offset(&self, file_idx: usize) -> usize {
        let mut offset = self.review_comments_render_height();
        for (i, file) in self.diff_files.iter().enumerate() {
            if i == file_idx {
                break;
            }
            offset += self.effective_file_height(i, file);
        }
        offset
    }

    pub(in crate::app) fn review_comments_render_height(&self) -> usize {
        // Header line is only rendered in multi-file view. See the guards
        // in `src/ui/diff_unified.rs` and `src/ui/diff_side_by_side.rs`.
        let mut height = if self.is_single_file_view { 0 } else { 1 };
        for summary in &self.forge_review_summaries {
            height += crate::forge::remote_comments::summary_display_lines(summary);
        }
        for comment in &self.session.review_comments {
            height += Self::comment_display_lines(comment, self.diff_state.viewport_width);
        }
        // Review-level remote threads (line: None) — must mirror the filter
        // in `rebuild_annotations` or scroll offsets fall out of sync.
        {
            use crate::forge::remote_comments::{PrCommentsVisibility, thread_display_lines};
            let visibility = self.session.remote_comments_visibility;
            if !matches!(visibility, PrCommentsVisibility::Hide) {
                for thread in &self.forge_review_threads {
                    if thread.line.is_some() {
                        continue;
                    }
                    if visibility.render_decision(thread).is_none() {
                        continue;
                    }
                    height += thread_display_lines(thread);
                }
            }
        }
        if self.input_mode == InputMode::Comment
            && self.comment_is_review_level
            && self.editing_comment_id.is_none()
        {
            // Header + one content line + footer
            height += 3;
        }
        height
    }

    pub(in crate::app) fn file_render_height(&self, file_idx: usize, file: &DiffFile) -> usize {
        if self.session.is_file_reviewed(file.display_path()) {
            return 1; // collapsed: header only
        }
        1 + self.file_render_body_height(file_idx, file) // header + body
    }

    /// File body height in lines (comments + content + trailing spacing),
    /// excluding the file header and ignoring the reviewed-collapse
    /// short-circuit. Used by `effective_file_height` to size the body of
    /// the focused file in single-file view, where the header is hidden
    /// and reviewed files render the body under a banner.
    fn file_render_body_height(&self, file_idx: usize, file: &DiffFile) -> usize {
        let path = file.display_path();

        let spacing_lines = 1; // Trailing blank or "next file" hint
        let mut content_lines = 0;
        let mut comment_lines = 0;

        // Pre-aggregate remote-thread rows by (line, side) for this file. Must
        // mirror the filter/anchor logic in build_remote_thread_index — the
        // rebuild_annotations renderer uses the same filter, and the two must
        // emit identical row counts or scroll math goes out of sync.
        let remote_thread_rows: HashMap<(u32, LineSide), usize> = {
            use crate::forge::remote_comments::{RemoteCommentSide, thread_display_lines};
            let mut map: HashMap<(u32, LineSide), usize> = HashMap::new();
            let path_str = path.to_string_lossy();
            let visibility = self.session.remote_comments_visibility;
            for thread in &self.forge_review_threads {
                if thread.path != *path_str {
                    continue;
                }
                if visibility.render_decision(thread).is_none() {
                    continue;
                }
                let Some(line) = thread.line else { continue };
                let side = match thread.side {
                    RemoteCommentSide::Right => LineSide::New,
                    RemoteCommentSide::Left => LineSide::Old,
                };
                *map.entry((line, side)).or_default() += thread_display_lines(thread);
            }
            map
        };

        // Commit-selection filter — must mirror rebuild_annotations and
        // both renderers exactly, or total_lines() disagrees with
        // line_annotations.len() and scroll/cursor math drifts.
        let commit_set = self.selected_commit_set();

        if let Some(review) = self.session.files.get(path) {
            for comment in &review.file_comments {
                if !Self::comment_visible_with(comment, commit_set.as_ref()) {
                    continue;
                }
                comment_lines +=
                    Self::comment_display_lines(comment, self.diff_state.viewport_width);
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

                if gap > 0 && self.should_render_gap_before_hunk(file_idx, hunk_idx) {
                    let top_len = self.expanded_top.get(&gap_id).map_or(0, |v| v.len());
                    let bot_len = self.expanded_bottom.get(&gap_id).map_or(0, |v| v.len());
                    let remaining = (gap as usize).saturating_sub(top_len + bot_len);
                    content_lines += top_len + bot_len;
                    content_lines += gap_annotation_line_count(hunk_idx == 0, false, remaining);
                }

                // Hunk header + diff lines
                content_lines += 1; // Hunk header
                if self.is_hunk_reviewed(file_idx, hunk_idx) {
                    continue;
                }

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
                                        if comment.side == Some(LineSide::Old)
                                            && Self::comment_visible_with(
                                                comment,
                                                commit_set.as_ref(),
                                            )
                                        {
                                            comment_lines += Self::comment_display_lines(
                                                comment,
                                                self.diff_state.viewport_width,
                                            );
                                        }
                                    }
                                }

                                if let Some(new_ln) = diff_line.new_lineno
                                    && let Some(comments) = line_comments.get(&new_ln)
                                {
                                    for comment in comments {
                                        if comment.side != Some(LineSide::Old)
                                            && Self::comment_visible_with(
                                                comment,
                                                commit_set.as_ref(),
                                            )
                                        {
                                            comment_lines += Self::comment_display_lines(
                                                comment,
                                                self.diff_state.viewport_width,
                                            );
                                        }
                                    }
                                }
                            }

                            if let Some(old_ln) = diff_line.old_lineno {
                                comment_lines += remote_thread_rows
                                    .get(&(old_ln, LineSide::Old))
                                    .copied()
                                    .unwrap_or(0);
                            }
                            if let Some(new_ln) = diff_line.new_lineno {
                                comment_lines += remote_thread_rows
                                    .get(&(new_ln, LineSide::New))
                                    .copied()
                                    .unwrap_or(0);
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
                                            if comment.side != Some(LineSide::Old)
                                                && Self::comment_visible_with(
                                                    comment,
                                                    commit_set.as_ref(),
                                                )
                                            {
                                                comment_lines += Self::comment_display_lines(
                                                    comment,
                                                    self.diff_state.viewport_width,
                                                );
                                            }
                                        }
                                    }
                                    if let Some(new_ln) = diff_line.new_lineno {
                                        comment_lines += remote_thread_rows
                                            .get(&(new_ln, LineSide::New))
                                            .copied()
                                            .unwrap_or(0);
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
                                                    if comment.side == Some(LineSide::Old)
                                                        && Self::comment_visible_with(
                                                            comment,
                                                            commit_set.as_ref(),
                                                        )
                                                    {
                                                        comment_lines +=
                                                            Self::comment_display_lines(
                                                                comment,
                                                                self.diff_state.viewport_width,
                                                            );
                                                    }
                                                }
                                            }
                                        }

                                        for line in &lines[add_start..add_end] {
                                            if let Some(new_ln) = line.new_lineno
                                                && let Some(comments) = line_comments.get(&new_ln)
                                            {
                                                for comment in comments {
                                                    if comment.side != Some(LineSide::Old)
                                                        && Self::comment_visible_with(
                                                            comment,
                                                            commit_set.as_ref(),
                                                        )
                                                    {
                                                        comment_lines +=
                                                            Self::comment_display_lines(
                                                                comment,
                                                                self.diff_state.viewport_width,
                                                            );
                                                    }
                                                }
                                            }
                                        }
                                    }

                                    for line in &lines[del_start..del_end] {
                                        if let Some(old_ln) = line.old_lineno {
                                            comment_lines += remote_thread_rows
                                                .get(&(old_ln, LineSide::Old))
                                                .copied()
                                                .unwrap_or(0);
                                        }
                                    }
                                    for line in &lines[add_start..add_end] {
                                        if let Some(new_ln) = line.new_lineno {
                                            comment_lines += remote_thread_rows
                                                .get(&(new_ln, LineSide::New))
                                                .copied()
                                                .unwrap_or(0);
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
                                            if comment.side != Some(LineSide::Old)
                                                && Self::comment_visible_with(
                                                    comment,
                                                    commit_set.as_ref(),
                                                )
                                            {
                                                comment_lines += Self::comment_display_lines(
                                                    comment,
                                                    self.diff_state.viewport_width,
                                                );
                                            }
                                        }
                                    }
                                    if let Some(new_ln) = diff_line.new_lineno {
                                        comment_lines += remote_thread_rows
                                            .get(&(new_ln, LineSide::New))
                                            .copied()
                                            .unwrap_or(0);
                                    }

                                    i += 1;
                                }
                            }
                        }
                    }
                }
            }

            // End-of-file gap (not for deleted files)
            if file.status != FileStatus::Deleted
                && let Some(last_hunk) = file.hunks.last()
            {
                let eof_start = last_hunk.new_start + last_hunk.new_count;
                if let Some(&total) = self.file_line_count_cache.get(&file_idx)
                    && eof_start <= total
                {
                    let gap = (total - eof_start + 1) as usize;
                    let eof_gap_id = GapId {
                        file_idx,
                        hunk_idx: file.hunks.len(),
                    };
                    let top_len = self.expanded_top.get(&eof_gap_id).map_or(0, |v| v.len());
                    let bot_len = self.expanded_bottom.get(&eof_gap_id).map_or(0, |v| v.len());
                    let remaining = gap.saturating_sub(top_len + bot_len);
                    content_lines += top_len + bot_len;
                    content_lines += gap_annotation_line_count(false, true, remaining);
                }
            }
        }

        comment_lines + content_lines + spacing_lines
    }

    /// Render-aware file height that knows about `is_single_file_view`.
    /// Multi-file view: same as `file_render_height`. Single-file view:
    /// non-current files return 0; the current file returns the body
    /// (no header) plus a 1-line banner when the file is reviewed.
    pub(in crate::app) fn effective_file_height(&self, file_idx: usize, file: &DiffFile) -> usize {
        if !self.is_single_file_view {
            return self.file_render_height(file_idx, file);
        }
        if file_idx != self.diff_state.current_file_idx {
            return 0;
        }
        let banner = if self.session.is_file_reviewed(file.display_path()) {
            1
        } else {
            0
        };
        banner + self.file_render_body_height(file_idx, file)
    }

    pub(in crate::app) fn update_current_file_from_cursor(&mut self) {
        // Single-file view renders one file at a time. `cursor_line` is
        // interpreted relative to that single file's content, so mapping
        // it back through cumulative-file-heights would wrongly resolve
        // every position to file 0. The "which file is visible" decision
        // is owned by `jump_to_file` / toggle / file-list-follow.
        if self.is_single_file_view {
            return;
        }
        let mut cumulative = self.review_comments_render_height();
        if self.diff_state.cursor_line < cumulative {
            if !self.diff_files.is_empty() {
                self.diff_state.current_file_idx = 0;
                self.file_list_state.select(0);
            }
            return;
        }
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
        self.review_comments_render_height()
            + self
                .diff_files
                .iter()
                .enumerate()
                .map(|(i, f)| self.effective_file_height(i, f))
                .sum::<usize>()
    }

    /// Last line the cursor can occupy. If the final annotation is a Spacing
    /// separator it is not navigable content and is excluded.
    pub fn max_cursor_line(&self) -> usize {
        let total = self.total_lines();
        if matches!(self.line_annotations.last(), Some(AnnotatedLine::Spacing)) {
            total.saturating_sub(2)
        } else {
            total.saturating_sub(1)
        }
    }

    /// Calculate the maximum scroll offset.
    ///
    /// Allows scrolling until the last line of content is at the top of the viewport.
    /// This permits empty space below content (e.g. when centering the cursor near EOF)
    /// while ensuring there is always at least one line of content visible at the top.
    pub fn max_scroll_offset(&self) -> usize {
        self.total_lines().saturating_sub(1)
    }

    /// Calculate the number of display lines a comment takes (header + content + footer).
    /// Uses viewport_width to account for pre-wrapped visual segments so the
    /// annotation count stays in sync with what format_comment_lines renders.
    pub(crate) fn comment_display_lines(comment: &Comment, viewport_width: usize) -> usize {
        // Mirrors the content_area calculation in format_comment_lines:
        // indicator(1) + border_prefix(7) + safety_margin(2) = 10
        let content_area = viewport_width.saturating_sub(10);
        let visual_lines: usize = comment
            .content
            .split('\n')
            .map(|line| crate::ui::comment_panel::wrap_segments(line, content_area).len())
            .sum();
        2 + visual_lines // top border + visual segments + bottom border
    }

    /// Update viewport_width and trigger annotation rebuild if it changed.
    /// Called from render functions when inner.width is computed — keeps
    /// line_annotations.len() in sync with the rendered Vec<Line>.
    pub fn sync_viewport_width(&mut self, new_width: usize) {
        if self.diff_state.viewport_width != new_width {
            self.diff_state.viewport_width = new_width;
            self.rebuild_annotations();
        }
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
}
