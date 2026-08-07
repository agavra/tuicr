use super::*;
use crate::ui::text_utils::{contains_fold, fold_for_search};
use std::borrow::Cow;

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

        self.search_needle_lower = Some(fold_for_search(&pattern));
        self.last_search_pattern = Some(pattern);
        self.recompute_search_matches();
        if self.line_annotations.is_empty() {
            self.set_message("No diff content to search");
            return false;
        }
        self.cycle_search_match(true, true)
    }

    pub fn search_next_in_diff(&mut self) -> bool {
        if self.last_search_pattern.is_none() {
            self.set_message("No previous search");
            return false;
        }
        self.cycle_search_match(true, false)
    }

    pub fn search_prev_in_diff(&mut self) -> bool {
        if self.last_search_pattern.is_none() {
            self.set_message("No previous search");
            return false;
        }
        self.cycle_search_match(false, false)
    }

    fn cycle_search_match(&mut self, forward: bool, include_current: bool) -> bool {
        if self.search_matches_stale {
            self.recompute_search_matches();
        }
        if self.search_matches.is_empty() {
            let pattern = self.last_search_pattern.as_deref().unwrap_or_default();
            self.set_message(format!("No matches for \"{pattern}\""));
            self.search_highlight_visible = false;
            return false;
        }

        self.search_highlight_visible = true;
        let cursor = self.diff_state.cursor_line;
        let match_idx = if forward {
            let idx = self.search_matches.partition_point(|&line| {
                if include_current {
                    line < cursor
                } else {
                    line <= cursor
                }
            });
            if idx == self.search_matches.len() {
                self.set_message("search hit BOTTOM, continuing at TOP");
                0
            } else {
                idx
            }
        } else {
            let idx = self.search_matches.partition_point(|&line| {
                if include_current {
                    line <= cursor
                } else {
                    line < cursor
                }
            });
            if idx == 0 {
                self.set_message("search hit TOP, continuing at BOTTOM");
                self.search_matches.len() - 1
            } else {
                idx - 1
            }
        };
        self.move_cursor_to_search_match(match_idx)
    }

    fn move_cursor_to_search_match(&mut self, match_idx: usize) -> bool {
        let Some(&line_idx) = self.search_matches.get(match_idx) else {
            return false;
        };
        self.diff_state.cursor_line = line_idx;
        self.ensure_cursor_visible();
        self.center_cursor();
        self.update_current_file_from_cursor();
        true
    }

    pub(crate) fn refresh_search_matches(&mut self) {
        if self.search_highlight_visible {
            self.recompute_search_matches();
        } else {
            self.search_matches_stale = true;
        }
    }

    fn recompute_search_matches(&mut self) {
        self.search_matches_stale = false;
        let Some(needle) = self.search_needle_lower.as_deref() else {
            self.search_matches.clear();
            return;
        };
        let mut matches = Vec::new();
        let mut pr_info_lines = None;
        let mut last_thread_match: Option<(usize, bool)> = None;
        for line_idx in 0..self.line_annotations.len() {
            let matched = match self.line_annotations.get(line_idx) {
                Some(AnnotatedLine::RemoteThreadLine { thread_idx }) => match last_thread_match {
                    Some((last_idx, last_matched)) if last_idx == *thread_idx => last_matched,
                    _ => {
                        let matched = self.thread_matches_search(*thread_idx, needle);
                        last_thread_match = Some((*thread_idx, matched));
                        matched
                    }
                },
                _ => self
                    .line_text_for_search(line_idx, &mut pr_info_lines)
                    .is_some_and(|text| contains_fold(&text, needle)),
            };
            if matched {
                matches.push(line_idx);
            }
        }
        debug_assert!(matches.is_sorted());
        self.search_matches = matches;
    }

    fn thread_matches_search(&self, thread_idx: usize, needle: &str) -> bool {
        let Some(thread) = self.forge_review_threads.get(thread_idx) else {
            return false;
        };
        contains_fold(&format!("github {}", thread.path), needle)
            || thread
                .comments
                .iter()
                .any(|comment| contains_fold(&comment.body, needle))
    }

    pub fn clear_search_highlight(&mut self) {
        self.search_highlight_visible = false;
    }

    pub fn search_match_position(&self) -> Option<(usize, usize)> {
        if !self.search_highlight_visible || self.search_matches.is_empty() {
            return None;
        }
        let current = self
            .search_matches
            .partition_point(|&line| line <= self.diff_state.cursor_line)
            .max(1);
        Some((current, self.search_matches.len()))
    }

    pub fn active_search_needle(&self) -> Option<&str> {
        if !self.search_highlight_enabled
            || !self.search_highlight_visible
            || self.input_mode == InputMode::Comment
        {
            return None;
        }
        self.search_needle_lower.as_deref()
    }

    pub(crate) fn search_paint_at(&self, line_idx: usize) -> Option<&str> {
        let needle = self.active_search_needle()?;
        self.search_matches.binary_search(&line_idx).ok()?;
        Some(needle)
    }

    fn pr_info_search_lines(&self) -> Vec<String> {
        let Some(info) = self.pr_info.as_ref() else {
            return Vec::new();
        };
        crate::ui::pr_info_panel::build_pr_info_lines(
            info,
            crate::ui::pr_info_panel::pr_info_content_width(self.diff_state.viewport_width),
            &self.theme,
        )
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect()
    }

    fn line_text_for_search<'a>(
        &'a self,
        line_idx: usize,
        pr_info_lines: &'a mut Option<Vec<String>>,
    ) -> Option<Cow<'a, str>> {
        match self.line_annotations.get(line_idx)? {
            AnnotatedLine::PrInfoLine { line_idx } => pr_info_lines
                .get_or_insert_with(|| self.pr_info_search_lines())
                .get(*line_idx)
                .map(|line| Cow::Borrowed(line.as_str())),
            AnnotatedLine::IssueCommentsHeader => {
                let info = self.pr_info.as_ref()?;
                Some(Cow::Owned(format!("PR #{} Comments", info.details.number)))
            }
            AnnotatedLine::IssueComment { comment_idx } => {
                let info = self.pr_info.as_ref()?;
                let comment = info.issue_comments.get(*comment_idx)?;
                Some(Cow::Borrowed(comment.body.as_str()))
            }
            AnnotatedLine::ReviewCommentsHeader => Some(Cow::Borrowed("Review comments")),
            AnnotatedLine::ReviewComment { comment_idx } => {
                let comment = self.session.review_comments.get(*comment_idx)?;
                Some(Cow::Borrowed(comment.content.as_str()))
            }
            AnnotatedLine::RemoteReviewSummaryLine { summary_idx } => {
                let summary = self.forge_review_summaries.get(*summary_idx)?;
                let author = summary.author.as_deref().unwrap_or("unknown");
                Some(Cow::Owned(format!("github @{author} {}", summary.body)))
            }
            AnnotatedLine::FileHeader { file_idx } => {
                let file = self.diff_files.get(*file_idx)?;
                Some(Cow::Owned(format!(
                    "{} [{}]",
                    file.display_path().display(),
                    file.status.as_char()
                )))
            }
            AnnotatedLine::FileComment {
                file_idx,
                comment_idx,
            } => {
                let path = self.diff_files.get(*file_idx)?.display_path();
                let review = self.session.files.get(path)?;
                let comment = review.file_comments.get(*comment_idx)?;
                Some(Cow::Borrowed(comment.content.as_str()))
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
                Some(Cow::Borrowed(comment.content.as_str()))
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
                Some(Cow::Owned(format!(
                    "... {arrow} expand ({count} lines) ..."
                )))
            }
            AnnotatedLine::HiddenLines { count, .. } => {
                Some(Cow::Owned(format!("... {count} lines hidden ...")))
            }
            AnnotatedLine::ExpandedContext {
                gap_id,
                line_idx: context_idx,
            } => {
                let content = self.get_expanded_line(gap_id, *context_idx)?;
                Some(Cow::Borrowed(content.content.as_str()))
            }
            AnnotatedLine::HunkHeader { file_idx, hunk_idx } => {
                let file = self.diff_files.get(*file_idx)?;
                let hunk = file.hunks.get(*hunk_idx)?;
                Some(Cow::Borrowed(hunk.header.as_str()))
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
                Some(Cow::Borrowed(line.content.as_str()))
            }
            AnnotatedLine::BinaryOrEmpty { file_idx } => {
                let file = self.diff_files.get(*file_idx)?;
                if file.is_too_large {
                    Some(Cow::Borrowed("(file too large to display)"))
                } else if file.is_binary {
                    Some(Cow::Borrowed("(binary file)"))
                } else {
                    Some(Cow::Borrowed("(no changes)"))
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
                Some(Cow::Owned(format!("{} {}", del_content, add_content)))
            }
            AnnotatedLine::RemoteThreadLine { .. }
            | AnnotatedLine::Spacing
            | AnnotatedLine::ReviewedBanner { .. } => None,
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
