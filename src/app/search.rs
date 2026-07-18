use super::*;

fn find_search_match(
    total_lines: usize,
    start_idx: usize,
    forward: bool,
    include_current: bool,
    pattern: &str,
    mut line_text: impl FnMut(usize) -> Option<String>,
) -> Option<usize> {
    if total_lines == 0 {
        return None;
    }

    let normalized_pattern = pattern.to_lowercase();
    let mut matches = |line_idx| {
        line_text(line_idx).is_some_and(|text| text.to_lowercase().contains(&normalized_pattern))
    };
    let start_idx = start_idx.min(total_lines - 1);
    if forward {
        let first = if include_current {
            start_idx
        } else {
            start_idx.saturating_add(1)
        };
        (first..total_lines).find(|&line_idx| matches(line_idx))
    } else {
        let first = if include_current {
            Some(start_idx)
        } else {
            start_idx.checked_sub(1)
        };
        first.and_then(|line_idx| (0..=line_idx).rev().find(|&line_idx| matches(line_idx)))
    }
}

impl HelpState {
    fn search(&mut self, pattern: &str, forward: bool, include_current: bool) -> bool {
        let start_idx = self.current_match_line.unwrap_or(self.scroll_offset);
        let Some(line) = find_search_match(
            self.searchable_lines.len(),
            start_idx,
            forward,
            include_current,
            pattern,
            |line_idx| self.searchable_lines.get(line_idx).cloned(),
        ) else {
            return false;
        };

        self.current_match_line = Some(line);
        let max_offset = self
            .searchable_lines
            .len()
            .saturating_sub(self.viewport_height);
        self.scroll_offset = line
            .saturating_sub(self.viewport_height / 2)
            .min(max_offset);
        true
    }
}

impl App {
    pub fn search_in_help_from_scroll(&mut self) -> bool {
        let pattern = self.search_buffer.clone();
        if pattern.trim().is_empty() {
            self.set_message("Search pattern is empty");
            return false;
        }

        self.help_state.last_search_pattern = Some(pattern.clone());
        self.help_state.current_match_line = None;
        if self.help_state.search(&pattern, true, true) {
            true
        } else {
            self.set_message(format!("No help matches for \"{pattern}\""));
            false
        }
    }

    pub fn search_next_in_help(&mut self) -> bool {
        let Some(pattern) = self.help_state.last_search_pattern.clone() else {
            self.set_message("No previous help search");
            return false;
        };
        if self.help_state.search(&pattern, true, false) {
            true
        } else {
            self.set_message(format!("No further help matches for \"{pattern}\""));
            false
        }
    }

    pub fn search_prev_in_help(&mut self) -> bool {
        let Some(pattern) = self.help_state.last_search_pattern.clone() else {
            self.set_message("No previous help search");
            return false;
        };
        if self.help_state.search(&pattern, false, false) {
            true
        } else {
            self.set_message(format!("No earlier help matches for \"{pattern}\""));
            false
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

        let matched_line = find_search_match(
            total_lines,
            start_idx,
            forward,
            include_current,
            pattern,
            |line_idx| self.line_text_for_search(line_idx),
        );

        let Some(line_idx) = matched_line else {
            self.set_message(format!("No matches for \"{pattern}\""));
            return false;
        };

        self.diff_state.cursor_line = line_idx;
        self.ensure_cursor_visible();
        self.center_cursor();
        self.update_current_file_from_cursor();
        true
    }

    fn line_text_for_search(&self, line_idx: usize) -> Option<String> {
        match self.line_annotations.get(line_idx)? {
            AnnotatedLine::ReviewCommentsHeader => Some("Review comments".to_string()),
            AnnotatedLine::ReviewComment { comment_idx } => {
                let comment = self.session.review_comments.get(*comment_idx)?;
                Some(comment.content.clone())
            }
            AnnotatedLine::RemoteReviewSummaryLine { summary_idx } => {
                let summary = self.forge_review_summaries.get(*summary_idx)?;
                let author = summary.author.as_deref().unwrap_or("unknown");
                Some(format!("github @{author} {}", summary.body))
            }
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
            AnnotatedLine::Expander { gap_id, direction } => {
                let arrow = match direction {
                    ExpandDirection::Down => "↓",
                    ExpandDirection::Up => "↑",
                    ExpandDirection::Both => "↕",
                };
                let gap = self.gap_size(gap_id)?;
                let top_len = self.expanded_top.get(gap_id).map_or(0, |v| v.len());
                let bot_len = self.expanded_bottom.get(gap_id).map_or(0, |v| v.len());
                let remaining = (gap as usize).saturating_sub(top_len + bot_len);
                let count = remaining.min(GAP_EXPAND_BATCH);
                Some(format!("... {arrow} expand ({count} lines) ..."))
            }
            AnnotatedLine::HiddenLines { count, .. } => {
                Some(format!("... {count} lines hidden ..."))
            }
            AnnotatedLine::ExpandedContext {
                gap_id,
                line_idx: context_idx,
            } => {
                let content = self.get_expanded_line(gap_id, *context_idx)?;
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
            AnnotatedLine::RemoteThreadLine { thread_idx } => {
                let thread = self.forge_review_threads.get(*thread_idx)?;
                // Search matches any text in the thread (including replies).
                let mut bodies: Vec<String> =
                    thread.comments.iter().map(|c| c.body.clone()).collect();
                bodies.insert(0, format!("github {}", thread.path));
                Some(bodies.join(" "))
            }
            AnnotatedLine::Spacing => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::HelpState;

    fn help_state() -> HelpState {
        HelpState {
            viewport_height: 5,
            searchable_lines: vec![
                "Navigation".to_string(),
                "Scroll down/up".to_string(),
                "Review actions".to_string(),
                "Add line comment".to_string(),
                "Commands".to_string(),
                "Reload comments".to_string(),
                "Toggle this help".to_string(),
            ],
            ..HelpState::default()
        }
    }

    #[test]
    fn should_find_help_text_case_insensitively_and_center_it_in_the_viewport() {
        let mut state = help_state();

        assert!(state.search("COMMENT", true, true));
        assert_eq!(state.current_match_line, Some(3));
        assert_eq!(state.scroll_offset, 1);
    }

    #[test]
    fn should_move_to_next_and_previous_help_matches() {
        let mut state = help_state();
        assert!(state.search("comment", true, true));

        assert!(state.search("comment", true, false));
        assert_eq!(state.current_match_line, Some(5));

        assert!(state.search("comment", false, false));
        assert_eq!(state.current_match_line, Some(3));
    }

    #[test]
    fn should_keep_the_current_help_position_when_no_match_exists() {
        let mut state = help_state();
        state.scroll_offset = 2;

        assert!(!state.search("missing", true, true));
        assert_eq!(state.current_match_line, None);
        assert_eq!(state.scroll_offset, 2);
    }
}
