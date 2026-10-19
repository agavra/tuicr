use super::*;

impl App {
    pub fn copy_visual_selection(&mut self) -> Result<usize> {
        let Some(sel) = self.visual_selection else {
            return Ok(0);
        };
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
        if out.is_empty() {
            return Ok(0);
        }
        let count = out.chars().count();
        crate::output::copy_text_to_clipboard(&out)
            .map_err(|e| TuicrError::Clipboard(format!("{e}")))?;
        Ok(count)
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

    pub fn visual_selection_line_range(&self) -> Option<(LineRange, LineSide)> {
        let sel = self.get_visual_selection()?;
        let (start, end) = sel.ordered();
        let start_line = self.annotation_line_for_side(start.annotation_idx, start.side);
        let end_line = self.annotation_line_for_side(end.annotation_idx, end.side);
        let start_ln = start_line?;
        let end_ln = end_line?;
        Some((LineRange::new(start_ln, end_ln), start.side))
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
        // Require one hunk with a new-side source line on every selected row.
        let mut selected_hunk = None;
        let mut seen_addition = false;
        let mut context_after_addition = false;
        for idx in start.annotation_idx..=end.annotation_idx {
            let hunk = match self.line_annotations.get(idx) {
                Some(AnnotatedLine::DiffLine {
                    file_idx, hunk_idx, ..
                })
                | Some(AnnotatedLine::SideBySideLine {
                    file_idx, hunk_idx, ..
                }) => Some((*file_idx, *hunk_idx)),
                _ => None,
            };
            if hunk.is_none()
                || selected_hunk.is_some_and(|selected| Some(selected) != hunk)
                || self.annotation_line_for_side(idx, LineSide::New).is_none()
            {
                self.set_warning(
                    "Suggestions must stay within one diff hunk, without gaps or expanded context",
                );
                return;
            }
            selected_hunk = hunk;
            match self
                .diff_line_for_side(idx, LineSide::New)
                .map(|line| line.origin)
            {
                Some(LineOrigin::Addition) if context_after_addition => {
                    self.set_warning(
                        "Suggestions cannot span unchanged context between added code blocks",
                    );
                    return;
                }
                Some(LineOrigin::Addition) => seen_addition = true,
                Some(LineOrigin::Context) if seen_addition => context_after_addition = true,
                _ => {}
            }
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
        if let Some((range, side)) = self.visual_selection_line_range() {
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
