use super::*;

impl App {
    /// Copy the selection; returns the character count and, when
    /// `clipboard_command` failed, the fallback warning.
    pub fn copy_visual_selection(&mut self) -> Result<(usize, Option<String>)> {
        let Some(sel) = self.visual_selection else {
            return Ok((0, None));
        };
        let out = self.visual_selection_text(&sel);
        if out.is_empty() {
            return Ok((0, None));
        }
        let count = out.chars().count();
        let copy = crate::output::copy_text_to_clipboard_with(
            &out,
            self.clipboard_override.as_deref(),
            self.clipboard_command.as_deref(),
        )
        .map_err(|e| TuicrError::Clipboard(format!("{e}")))?;
        Ok((count, copy.fallback_warning))
    }

    /// Text a visual selection copies, confined to the side it is anchored on:
    /// side-by-side rows that are empty there contribute nothing.
    pub(crate) fn visual_selection_text(&self, sel: &VisualSelection) -> String {
        let (start, end) = sel.ordered();
        let side = sel.anchor.side;
        let mut out = String::new();
        let mut emitted = 0usize;
        for idx in start.annotation_idx..=end.annotation_idx {
            let snippet = if let Some(content) = self.content_for_side(idx, side) {
                let total = content.chars().count();
                let (lo, hi) = sel.char_range(idx, total);
                char_slice(content, lo, Some(hi)).to_string()
            } else if let Some(text) = self.atomic_text_for_annotation(idx) {
                text
            } else {
                continue;
            };
            if emitted > 0 {
                out.push('\n');
            }
            out.push_str(&snippet);
            emitted += 1;
        }
        out
    }

    /// Visual-mode `j`: in side-by-side, steps over rows with no content on
    /// the selection's side so the range stays in its pane.
    pub fn visual_cursor_down(&mut self, lines: usize) {
        self.visual_cursor_move(lines, true);
    }

    /// Visual-mode `k`; mirror of [`Self::visual_cursor_down`].
    pub fn visual_cursor_up(&mut self, lines: usize) {
        self.visual_cursor_move(lines, false);
    }

    /// Visual-mode half/full page down: pages like normal mode, then extends
    /// the selection to the new cursor row.
    pub fn visual_page_down(&mut self, row_budget: usize) {
        self.page_down(row_budget);
        self.extend_visual_after_page(true);
    }

    /// Mirror of [`Self::visual_page_down`].
    pub fn visual_page_up(&mut self, row_budget: usize) {
        self.page_up(row_budget);
        self.extend_visual_after_page(false);
    }

    /// A page can land on a side-by-side row with nothing on the selection's
    /// side: step on in the paging direction, or back when none is left.
    fn extend_visual_after_page(&mut self, down: bool) {
        let Some(side) = self.visual_selection.map(|sel| sel.anchor.side) else {
            return;
        };
        if self.diff_view_mode == DiffViewMode::SideBySide {
            for direction in [down, !down] {
                while !self.row_covers_side(self.diff_state.cursor_line, side) {
                    let current = self.diff_state.cursor_line;
                    self.step_cursor(direction);
                    if self.diff_state.cursor_line == current {
                        break;
                    }
                }
            }
        }
        self.extend_visual_to_cursor();
    }

    fn visual_cursor_move(&mut self, lines: usize, down: bool) {
        let Some(side) = self.visual_selection.map(|sel| sel.anchor.side) else {
            return;
        };
        let skip_empty_sides = self.diff_view_mode == DiffViewMode::SideBySide;
        'steps: for _ in 0..lines {
            let before = self.diff_state.cursor_line;
            self.step_cursor(down);
            if self.diff_state.cursor_line == before {
                break;
            }
            while skip_empty_sides && !self.row_covers_side(self.diff_state.cursor_line, side) {
                let current = self.diff_state.cursor_line;
                self.step_cursor(down);
                if self.diff_state.cursor_line == current {
                    // No selectable row further along; stay on the last one
                    // that had content on this side.
                    self.move_cursor_to_annotation(before);
                    break 'steps;
                }
            }
        }
        self.extend_visual_to_cursor();
    }

    fn step_cursor(&mut self, down: bool) {
        if down {
            self.cursor_down(1);
        } else {
            self.cursor_up(1);
        }
    }

    fn row_covers_side(&self, idx: usize, side: LineSide) -> bool {
        match self.line_annotations.get(idx) {
            Some(AnnotatedLine::SideBySideLine { .. }) => {
                self.content_for_side(idx, side).is_some()
            }
            _ => true,
        }
    }

    pub fn enter_visual_mode_at_cursor(&mut self) {
        let idx = self.diff_state.cursor_line;
        let side = self
            .get_line_at_cursor()
            .map(|(_, s)| s)
            .unwrap_or(LineSide::New);
        let len = self.annotation_content_len(idx, side);
        let anchor = SelPoint {
            annotation_idx: idx,
            char_offset: 0,
            side,
        };
        let head = SelPoint {
            annotation_idx: idx,
            char_offset: len,
            side,
        };
        self.input_mode = InputMode::VisualSelect;
        self.visual_selection = Some(VisualSelection { anchor, head });
    }

    pub fn exit_visual_mode(&mut self) {
        self.input_mode = InputMode::Normal;
        self.visual_selection = None;
    }

    pub fn get_visual_selection(&self) -> Option<&VisualSelection> {
        if self.input_mode != InputMode::VisualSelect {
            return None;
        }
        self.visual_selection.as_ref()
    }

    pub fn annotation_content_len(&self, idx: usize, side: LineSide) -> usize {
        self.content_for_side(idx, side)
            .map(|s| s.chars().count())
            .unwrap_or(0)
    }

    pub fn extend_visual_to_cursor(&mut self) {
        let Some(sel) = self.visual_selection else {
            return;
        };
        let anchor_idx = sel.anchor.annotation_idx;
        let cursor_idx = self.diff_state.cursor_line;
        let side = sel.anchor.side;
        let anchor_len = self.annotation_content_len(anchor_idx, side);
        let cursor_len = self.annotation_content_len(cursor_idx, side);
        let (anchor_char, head_char) = if cursor_idx >= anchor_idx {
            (0, cursor_len)
        } else {
            (anchor_len, 0)
        };
        self.visual_selection = Some(VisualSelection {
            anchor: SelPoint {
                annotation_idx: anchor_idx,
                char_offset: anchor_char,
                side,
            },
            head: SelPoint {
                annotation_idx: cursor_idx,
                char_offset: head_char,
                side,
            },
        });
    }

    /// Line range and side of the selection, or `None` when it spans files.
    pub fn visual_selection_line_range(&self) -> Option<(LineRange, LineSide)> {
        self.visual_selection_target()
            .map(|(_, range, side)| (range, side))
    }

    /// File, line range and side of the selection. Each file's annotations
    /// are contiguous, so equal endpoint files mean one file throughout.
    fn visual_selection_target(&self) -> Option<(usize, LineRange, LineSide)> {
        let sel = self.get_visual_selection()?;
        let (start, end) = sel.ordered();
        let file_idx = annotation_file_idx(self.line_annotations.get(start.annotation_idx)?)?;
        if annotation_file_idx(self.line_annotations.get(end.annotation_idx)?)? != file_idx {
            return None;
        }
        let start_ln =
            self.edge_line_for_side(start.annotation_idx, end.annotation_idx, start.side)?;
        let end_ln =
            self.edge_line_for_side(end.annotation_idx, start.annotation_idx, start.side)?;
        Some((file_idx, LineRange::new(start_ln, end_ln), start.side))
    }

    /// Line number for a selection endpoint, walking into the selection when a
    /// side-by-side endpoint row is empty on `side` (a mouse drag can end
    /// there). Other rows keep their strict resolution.
    fn edge_line_for_side(&self, idx: usize, toward: usize, side: LineSide) -> Option<u32> {
        if !matches!(
            self.line_annotations.get(idx),
            Some(AnnotatedLine::SideBySideLine { .. })
        ) {
            return self.annotation_line_for_side(idx, side);
        }
        let mut i = idx;
        loop {
            if let Some(line) = self.annotation_line_for_side(i, side) {
                return Some(line);
            }
            if i == toward {
                return None;
            }
            i = if toward > i { i + 1 } else { i - 1 };
        }
    }

    fn annotation_line_for_side(&self, idx: usize, side: LineSide) -> Option<u32> {
        match self.line_annotations.get(idx)? {
            AnnotatedLine::DiffLine {
                old_lineno,
                new_lineno,
                ..
            }
            | AnnotatedLine::SideBySideLine {
                old_lineno,
                new_lineno,
                ..
            } => match side {
                LineSide::New => *new_lineno,
                LineSide::Old => *old_lineno,
            },
            _ => None,
        }
    }

    pub fn enter_suggestion_from_visual(&mut self) {
        let Some(sel) = self.get_visual_selection().copied() else {
            return;
        };
        let (start, end) = sel.ordered();
        // Suggestions replace whole lines, even for a partial mouse selection.
        if start.side != LineSide::New || end.side != LineSide::New {
            self.set_warning("Suggestions can only replace lines on the new side of the diff");
            return;
        }
        let Some((range, _)) = self.visual_selection_line_range() else {
            self.set_warning("Invalid visual selection");
            return;
        };
        let info = if self.forge_kind() == Some(crate::forge::traits::ForgeKind::GitLab) {
            let above = range.end - range.start;
            if above > 100 {
                self.set_warning("GitLab suggestions can cover at most 100 lines above the anchor");
                return;
            }
            format!("suggestion:-{above}+0")
        } else {
            "suggestion".to_string()
        };
        // Require one hunk with a new-side source line on every selected code
        // row. A side-by-side row for a deleted line is empty on the new pane
        // and is not a gap: it adds no line to the suggestion.
        let mut selected_hunk = None;
        for idx in start.annotation_idx..=end.annotation_idx {
            let hunk = match self.line_annotations.get(idx) {
                Some(
                    AnnotatedLine::LineComment { .. } | AnnotatedLine::RemoteThreadLine { .. },
                ) => {
                    continue;
                }
                Some(AnnotatedLine::DiffLine {
                    file_idx, hunk_idx, ..
                })
                | Some(AnnotatedLine::SideBySideLine {
                    file_idx, hunk_idx, ..
                }) => Some((*file_idx, *hunk_idx)),
                _ => None,
            };
            let deletion_only_row = matches!(
                self.line_annotations.get(idx),
                Some(AnnotatedLine::SideBySideLine {
                    old_lineno: Some(_),
                    new_lineno: None,
                    ..
                })
            );
            if hunk.is_none()
                || selected_hunk.is_some_and(|selected| Some(selected) != hunk)
                || (!deletion_only_row
                    && self.annotation_line_for_side(idx, LineSide::New).is_none())
            {
                self.set_warning(
                    "Suggestions must stay within one diff hunk, without gaps or expanded context",
                );
                return;
            }
            selected_hunk = hunk;
        }
        let code = (start.annotation_idx..=end.annotation_idx)
            .filter_map(|idx| self.diff_line_for_side(idx, start.side))
            .map(|line| line.raw_content.as_deref().unwrap_or(&line.content))
            .collect::<Vec<_>>()
            .join("\n");
        let fence_len = code.split(|c| c != '`').map(str::len).max().unwrap_or(0) + 1;
        let fence = "`".repeat(fence_len.max(3));

        self.enter_comment_from_visual();
        if self.input_mode == InputMode::Comment {
            self.comment_buffer = format!("\n{fence}{info}\n{code}\n{fence}");
            self.comment_cursor = 0;
        }
    }

    pub fn enter_comment_from_visual(&mut self) {
        if let Some((file_idx, range, side)) = self.visual_selection_target() {
            // Paging in visual mode can move the current file off the selection.
            self.diff_state.current_file_idx = file_idx;
            self.comment_line_range = Some((range, side));
            self.comment_line = Some((range.end, side));
            self.input_mode = InputMode::Comment;
            if self.diff_view_mode != DiffViewMode::SideBySide {
                self.diff_state.scroll_x = 0;
            }
            self.comment_buffer.clear();
            self.comment_cursor = 0;
            self.comment_type = self.default_comment_type();
            self.comment_is_review_level = false;
            self.comment_is_file_level = false;
            self.visual_selection = None;
        } else {
            self.set_warning("Invalid visual selection");
            self.exit_visual_mode();
        }
    }
}
