use super::*;

impl App {
    pub(in crate::app) fn is_strict_commit_selection(
        range: Option<(usize, usize)>,
        total: usize,
    ) -> bool {
        range.is_some_and(|(start, end)| {
            total > 0 && start <= end && end < total && (start > 0 || end + 1 < total)
        })
    }

    pub(in crate::app) fn apply_pr_commit_selector(
        &mut self,
        commits: Vec<crate::forge::traits::PullRequestCommit>,
        review_metadata: crate::forge::traits::PullRequestReviewMetadata,
    ) -> Option<String> {
        self.pr_last_reviewed_commit_index = None;
        if commits.len() <= 1 {
            return None;
        }

        let since_last_review = commits_since_last_review_selection(&commits, &review_metadata);
        self.set_pr_last_reviewed_commit_from_metadata(&commits, &review_metadata);

        self.pr_commits = commits.clone();
        let mapped: Vec<CommitInfo> = commits.iter().map(pr_commit_to_commit_info).collect();
        self.range_diff_files = Some(self.diff_files.clone());
        self.commit_list = mapped.clone();
        self.commit_list_cursor = 0;
        self.commit_list_scroll_offset = 0;
        self.visible_commit_count = mapped.len();
        self.has_more_commit = false;
        self.show_commit_selector = true;

        let mut range = (0, mapped.len() - 1);
        let mut auto_scoped_since_last_review = false;
        let mut since_last_review_message = None;
        // Restore any persisted range scoped to this head SHA. If the
        // restored range exceeds the current commit count (e.g., the PR
        // was rebased), fall back to "all". Only auto-scope to commits
        // since last review when no explicit per-session range exists.
        if let Some(persisted) = self.session.commit_selection_range
            && persisted.1 < mapped.len()
            && persisted.0 <= persisted.1
        {
            range = persisted;
        } else if self.session.commit_selection_range.is_none()
            && let Some(selection) = since_last_review.as_ref()
        {
            if let Some(selected_range) = selection.range {
                range = selected_range;
                auto_scoped_since_last_review = true;
            }
            since_last_review_message = Some(selection.message.clone());
        }

        self.commit_selection_range = Some(range);
        self.review_commits = mapped;

        if let Some(message) = since_last_review_message {
            if auto_scoped_since_last_review
                && Self::is_strict_commit_selection(Some(range), self.pr_commits.len())
            {
                self.focused_panel = FocusedPanel::CommitSelector;
                self.commit_list_cursor = self.pr_commits.len().saturating_sub(1);
                self.commit_list_scroll_offset = self.commit_list_cursor.saturating_sub(5);
            }
            return Some(message);
        }
        None
    }

    pub fn commit_list_idx_at_screen_row(&self, screen_row: u16) -> Option<usize> {
        let inner = self.commit_list_inner_area?;
        if screen_row < inner.y || screen_row >= inner.y + inner.height {
            return None;
        }
        let rel = (screen_row - inner.y) as usize;
        let idx = self.commit_list_scroll_offset + rel;
        let total = match self.input_mode {
            InputMode::CommitSelect => {
                self.visible_commit_count + usize::from(self.can_show_more_commits())
            }
            _ => self.review_commits.len(),
        };
        (idx < total).then_some(idx)
    }

    /// Open the review target selector on a specific tab.
    ///
    /// `Local` loads the recent-commits list (same as the historical commit
    /// selector). `PullRequests` switches the tab; the actual fetch is
    /// triggered lazily through `on_target_tab_entered`.
    pub fn enter_target_selector(&mut self, initial_tab: TargetTab) -> Result<()> {
        // Save inline selection state if we have review commits
        if !self.review_commits.is_empty() {
            self.saved_inline_selection = self.commit_selection_range;
        }

        let highlighter = self.theme.syntax_highlighter();
        let change_status = Self::get_change_status_with_ignore(
            self.vcs.as_ref(),
            &self.vcs_info.root_path,
            highlighter,
            self.path_filter.as_deref(),
        )?;
        let has_staged_changes = change_status.staged;
        let has_unstaged_changes = change_status.unstaged;

        let commits = self.vcs.get_recent_commits(0, VISIBLE_COMMIT_COUNT)?;
        let no_local_targets = commits.is_empty() && !has_staged_changes && !has_unstaged_changes;
        // Allow opening the selector on the Pull Requests tab even when there
        // are no local commits or changes — the PR tab is the user's reason
        // for being here.
        if no_local_targets && initial_tab == TargetTab::Local {
            self.set_message("No commits or staged/unstaged changes found");
            return Ok(());
        }

        // Check if there might be more commits
        self.has_more_commit = commits.len() >= VISIBLE_COMMIT_COUNT;
        self.commit_list = commits;
        if has_staged_changes {
            self.commit_list.insert(0, Self::staged_commit_entry());
        }
        if has_unstaged_changes {
            self.commit_list.insert(0, Self::unstaged_commit_entry());
        }
        self.commit_list_cursor = 0;
        self.commit_list_scroll_offset = 0;
        self.commit_selection_range = None;
        self.visible_commit_count = self.commit_list.len();
        self.input_mode = InputMode::CommitSelect;

        // Reset the PR tab to Idle each time the selector is opened so the
        // fetch happens lazily on first visit.
        self.pr_tab = PullRequestsTab::new(self.forge_repository.clone());
        self.pr_filter_draft = None;
        self.pr_load_rx = None;

        self.target_tab = initial_tab;
        if initial_tab == TargetTab::PullRequests {
            self.on_target_tab_entered();
        }
        Ok(())
    }

    pub fn exit_commit_select_mode(&mut self) -> Result<()> {
        self.input_mode = InputMode::Normal;

        // If we have review commits, restore the inline selector state
        if !self.review_commits.is_empty() {
            self.commit_list = self.review_commits.clone();
            self.commit_selection_range = self.saved_inline_selection;
            self.commit_list_cursor = 0;
            self.commit_list_scroll_offset = 0;
            self.visible_commit_count = self.review_commits.len();
            self.has_more_commit = false;
            self.saved_inline_selection = None;

            // Reload diff for the restored selection
            if self.commit_selection_range.is_some() {
                self.reload_inline_selection()?;
            }
            return Ok(());
        }

        // If we were viewing commits, try to go back to working tree
        if matches!(
            self.diff_source,
            DiffSource::CommitRange(_) | DiffSource::StagedUnstagedAndCommits(_)
        ) {
            let highlighter = self.theme.syntax_highlighter();
            match Self::get_working_tree_diff_with_ignore(
                self.vcs.as_ref(),
                &self.vcs_info.root_path,
                highlighter,
                self.path_filter.as_deref(),
            ) {
                Ok(diff_files) => {
                    self.diff_files = diff_files;
                    self.diff_source = DiffSource::StagedAndUnstaged;

                    // Update session for new files
                    for file in &self.diff_files {
                        self.session.add_diff_file(file);
                    }

                    self.sort_files_by_directory(true);
                    self.expand_all_dirs();
                }
                Err(_) => {
                    self.set_message("No staged or unstaged changes");
                }
            }
        }

        Ok(())
    }

    /// Switch to the next/previous tab in the review target selector.
    /// With only two tabs, forward and reverse are equivalent; the `_forward`
    /// arg is kept so callers can pass the natural direction without a cast.
    /// Triggers the lazy PR fetch the first time the PR tab is entered.
    pub fn cycle_target_tab(&mut self, _forward: bool) {
        let next = match self.target_tab {
            TargetTab::Local => TargetTab::PullRequests,
            TargetTab::PullRequests => TargetTab::Local,
        };
        self.target_tab = next;
        if next == TargetTab::PullRequests {
            self.on_target_tab_entered();
        } else {
            // Returning to Local: clear any half-typed PR filter draft.
            self.pr_filter_draft = None;
        }
    }

    /// Entry-point hook called when the PR tab becomes visible.
    /// Triggers the first network call lazily.
    fn on_target_tab_entered(&mut self) {
        if let Some((repo, scope)) = self.pr_tab.start_initial_load() {
            let override_repo = self.repo_url_override.clone();
            let skip_resolution = self.canonical_resolved;
            self.spawn_pr_initial_load(repo, override_repo, skip_resolution, scope);
        }
    }

    /// Whether the inline commit selector panel should be displayed.
    pub fn has_inline_commit_selector(&self) -> bool {
        self.show_commit_selector
            && self.review_commits.len() > 1
            && !matches!(&self.diff_source, DiffSource::WorkingTree)
    }

    // Commit selection methods

    pub fn commit_select_up(&mut self) {
        if self.commit_list_cursor > 0 {
            self.commit_list_cursor -= 1;
            // Scroll up if cursor goes above visible area
            if self.commit_list_cursor < self.commit_list_scroll_offset {
                self.commit_list_scroll_offset = self.commit_list_cursor;
            }
        }
    }

    pub fn commit_select_down(&mut self) {
        let max_cursor = if self.can_show_more_commits() {
            self.visible_commit_count
        } else {
            self.visible_commit_count.saturating_sub(1)
        };

        if self.commit_list_cursor < max_cursor {
            self.commit_list_cursor += 1;
            // Scroll down if cursor goes below visible area
            if self.commit_list_viewport_height > 0
                && self.commit_list_cursor
                    >= self.commit_list_scroll_offset + self.commit_list_viewport_height
            {
                self.commit_list_scroll_offset =
                    self.commit_list_cursor - self.commit_list_viewport_height + 1;
            }
        }
    }

    /// Toggle the cursor commit's membership in the selection range, then
    /// (only if the cursor commit was newly added to the selection) move the
    /// cursor past the end of the range. Lets the user press Enter/Space
    /// repeatedly to sweep a contiguous run of commits.
    ///
    /// Other toggle outcomes leave the cursor in place: edge presses
    /// (deselect the cursor commit), middle presses (truncate the range
    /// without unselecting the cursor commit), and clearing the last
    /// selection. Those aren't "sweep" actions, so advancing would surprise.
    pub fn toggle_commit_selection_and_advance(&mut self) {
        let cursor = self.commit_list_cursor;
        let was_selected = self.is_commit_selected(cursor);
        self.toggle_commit_selection();
        let now_selected = self.is_commit_selected(cursor);
        if was_selected || !now_selected {
            return;
        }
        if let Some((_, end)) = self.commit_selection_range {
            while self.commit_list_cursor <= end {
                let before = self.commit_list_cursor;
                self.commit_select_down();
                if self.commit_list_cursor == before {
                    return;
                }
            }
        }
    }

    // Check if cursor is on the commit expand row
    pub fn is_on_expand_row(&self) -> bool {
        self.can_show_more_commits() && self.commit_list_cursor == self.visible_commit_count
    }

    pub fn can_show_more_commits(&self) -> bool {
        self.visible_commit_count < self.commit_list.len() || self.has_more_commit
    }

    // Expand the commit list to show more commits
    pub fn expand_commit(&mut self) -> Result<()> {
        if self.visible_commit_count < self.commit_list.len() {
            self.visible_commit_count =
                (self.visible_commit_count + self.commit_page_size).min(self.commit_list.len());
            return Ok(());
        }

        if !self.has_more_commit {
            self.set_message("No more commits");
            return Ok(());
        }

        let offset = self.loaded_history_commit_count();
        let limit = self.commit_page_size;

        let new_commits = self.vcs.get_recent_commits(offset, limit)?;

        if new_commits.is_empty() {
            self.has_more_commit = false;
            self.set_message("No more commits");
            return Ok(());
        }

        if new_commits.len() < limit {
            self.has_more_commit = false;
            self.set_message("No more commits");
        }

        self.commit_list.extend(new_commits);
        self.visible_commit_count = self.commit_list.len();

        Ok(())
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
                let all_commits_selected =
                    self.commit_list.len() > 1 && start == 0 && end == self.commit_list.len() - 1;
                if all_commits_selected {
                    self.commit_selection_range = Some((cursor, cursor));
                    return;
                }

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
                        // In the middle - deselect cursor and everything after it
                        self.commit_selection_range = Some((start, cursor - 1));
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

    /// The set of commit SHAs currently selected in the inline commit
    /// selector. `None` when there is no selector / no selection (working
    /// tree, staged, etc.) — in that case every comment is visible. When
    /// the full range is selected the set contains every commit SHA, so
    /// all comments (including single-commit-scoped ones) show. When a
    /// strict subset is selected, only comments whose `commit_id` is in
    /// the set (or `None`) are visible.
    pub(in crate::app) fn selected_commit_set(&self) -> Option<std::collections::HashSet<String>> {
        let (start, end) = self.commit_selection_range?;
        if start > end || self.review_commits.is_empty() {
            return Some(std::collections::HashSet::new());
        }
        let end = end.min(self.review_commits.len().saturating_sub(1));
        let set: std::collections::HashSet<String> = (start..=end)
            .filter_map(|i| self.review_commits.get(i))
            .filter(|c| !Self::is_special_commit(c))
            .map(|c| c.id.clone())
            .collect();
        Some(set)
    }

    /// Whether a comment should be visible given the current commit
    /// selection. A comment with `commit_id == None` (legacy or made
    /// against the full cumulative diff) is always visible. Otherwise it
    /// is visible only when its commit is in the selected set.
    ///
    /// Allocates the commit set on every call. Callers in hot paths
    /// (per-comment-per-frame renderers, height calculation) should
    /// compute [`selected_commit_set`] once and use
    /// [`comment_visible_with`] instead.
    pub fn comment_visible(&self, comment: &crate::model::Comment) -> bool {
        Self::comment_visible_with(comment, self.selected_commit_set().as_ref())
    }

    /// Pure visibility check against a precomputed commit set — no
    /// allocation. `set == None` means "no selector", so every comment
    /// is visible. This is the shared predicate all filtering sites
    /// should converge on so height math and rendering never drift.
    pub fn comment_visible_with(
        comment: &crate::model::Comment,
        commit_set: Option<&std::collections::HashSet<String>>,
    ) -> bool {
        match (&comment.commit_id, commit_set) {
            (None, _) => true,
            (Some(_), None) => true,
            (Some(sha), Some(set)) => set.contains(sha),
        }
    }

    /// The single commit SHA to stamp on a *new* comment, when the inline
    /// selector shows exactly one commit. `None` otherwise (full range,
    /// multi-commit subset, or no selector) — those comments get
    /// `commit_id = None` so they stay visible across selections.
    pub(in crate::app) fn commit_id_for_new_comment(&self) -> Option<String> {
        let (start, end) = self.commit_selection_range?;
        if start != end {
            return None;
        }
        self.review_commits
            .get(start)
            .filter(|c| !Self::is_special_commit(c))
            .map(|c| c.id.clone())
    }

    pub(in crate::app) fn set_pr_last_reviewed_commit_from_metadata(
        &mut self,
        commits: &[crate::forge::traits::PullRequestCommit],
        review_metadata: &crate::forge::traits::PullRequestReviewMetadata,
    ) {
        self.pr_last_reviewed_commit_index = if commits.len() > 1 {
            commits_since_last_review_selection(commits, review_metadata)
                .map(|selection| selection.reviewed_index)
        } else {
            None
        };
    }

    pub(in crate::app) fn mark_pr_commits_reviewed_through(&mut self, commit_id: &str) {
        if !matches!(&self.diff_source, DiffSource::PullRequest(_)) {
            return;
        }
        if let Some(index) = self
            .pr_commits
            .iter()
            .position(|commit| commit.oid == commit_id)
        {
            self.pr_last_reviewed_commit_index = Some(index);
        }
    }

    /// Check if a PR commit is covered by the viewer's latest submitted review.
    pub fn is_commit_reviewed_by_viewer(&self, index: usize) -> bool {
        matches!(&self.diff_source, DiffSource::PullRequest(_))
            && index < self.review_commits.len()
            && self
                .pr_last_reviewed_commit_index
                .is_some_and(|reviewed_index| index >= reviewed_index)
    }

    /// Cycle inline commit selector to the next individual commit (`)` key).
    /// all → last, i → i+1, last → all
    pub fn cycle_commit_next(&mut self) {
        if self.review_commits.is_empty() {
            return;
        }
        let n = self.review_commits.len();
        let all_selected = Some((0, n - 1));

        if self.commit_selection_range == all_selected {
            // all → last
            self.commit_selection_range = Some((n - 1, n - 1));
            self.commit_list_cursor = n - 1;
        } else if let Some((i, j)) = self.commit_selection_range {
            if i == j {
                // Single commit selected
                if i == n - 1 {
                    // last → all
                    self.commit_selection_range = all_selected;
                } else {
                    // i → i+1
                    self.commit_selection_range = Some((i + 1, i + 1));
                    self.commit_list_cursor = i + 1;
                }
            } else {
                // Multi-commit subrange → select last of that range
                self.commit_selection_range = Some((j, j));
                self.commit_list_cursor = j;
            }
        } else {
            // None selected → select all
            self.commit_selection_range = all_selected;
        }
    }

    /// Cycle inline commit selector to the previous individual commit (`(` key).
    /// all → first, i → i-1, first → all
    pub fn cycle_commit_prev(&mut self) {
        if self.review_commits.is_empty() {
            return;
        }
        let n = self.review_commits.len();
        let all_selected = Some((0, n - 1));

        if self.commit_selection_range == all_selected {
            // all → first
            self.commit_selection_range = Some((0, 0));
            self.commit_list_cursor = 0;
        } else if let Some((i, j)) = self.commit_selection_range {
            if i == j {
                // Single commit selected
                if i == 0 {
                    // first → all
                    self.commit_selection_range = all_selected;
                } else {
                    // i → i-1
                    self.commit_selection_range = Some((i - 1, i - 1));
                    self.commit_list_cursor = i - 1;
                }
            } else {
                // Multi-commit subrange → select first of that range
                self.commit_selection_range = Some((i, i));
                self.commit_list_cursor = i;
            }
        } else {
            // None selected → select all
            self.commit_selection_range = all_selected;
        }
    }

    pub fn confirm_commit_selection(&mut self) -> Result<()> {
        let selection = match self.commit_selection_range {
            Some((start, end)) => format!(
                "range={start}..={end}, rows={}",
                end.saturating_sub(start) + 1
            ),
            None => "range=none, rows=0".to_string(),
        };
        crate::profile::time_with(
            "commit_select.confirm_selection",
            || self.confirm_commit_selection_inner(),
            |result| format!("{selection}, {}", profile_unit_result(result)),
        )
    }

    fn confirm_commit_selection_inner(&mut self) -> Result<()> {
        let (start, end) = match self.commit_selection_range {
            Some(range) => range,
            None => {
                let cursor = self.commit_list_cursor;
                (cursor, cursor)
            }
        };

        // Collect selected entries in order from oldest to newest (end..start).
        let selected_commits: Vec<CommitInfo> = (start..=end)
            .rev()
            .filter_map(|i| self.commit_list.get(i))
            .cloned()
            .collect();

        if selected_commits.is_empty() {
            self.set_message("Select at least one commit");
            return Ok(());
        }

        let selected_staged = selected_commits.iter().any(Self::is_staged_commit);
        let selected_unstaged = selected_commits.iter().any(Self::is_unstaged_commit);
        let selected_ids: Vec<String> = selected_commits
            .iter()
            .filter(|c| !Self::is_special_commit(c))
            .map(|c| c.id.clone())
            .collect();

        if (selected_staged || selected_unstaged) && !selected_ids.is_empty() {
            return self.load_staged_unstaged_and_commits_selection(selected_ids, selected_commits);
        }

        if selected_staged && selected_unstaged {
            return self.load_staged_and_unstaged_selection();
        }

        if selected_staged {
            return self.load_staged_selection();
        }

        if selected_unstaged {
            return self.load_unstaged_selection();
        }

        // Get the diff for the selected commits
        let highlighter = self.theme.syntax_highlighter();
        let diff_files = Self::get_commit_range_diff_with_ignore(
            self.vcs.as_ref(),
            &self.vcs_info.root_path,
            &ResolvedRevisionRange::from_commit_ids(&selected_ids, RevisionDiffTarget::CommitList),
            highlighter,
            self.path_filter.as_deref(),
        )?;

        if diff_files.is_empty() {
            self.set_message("No changes in selected commits");
            return Ok(());
        }

        // Update session with the newest commit as base
        let newest_commit_id = selected_ids.last().unwrap().clone();
        let loaded_session = load_latest_session_for_context(
            &self.vcs_info.root_path,
            self.vcs_info.branch_name.as_deref(),
            &newest_commit_id,
            SessionDiffSource::CommitRange,
            Some(selected_ids.as_slice()),
        )
        .ok()
        .and_then(|found| found.map(|(_path, session)| session));

        let mut session = loaded_session.unwrap_or_else(|| {
            let mut session = ReviewSession::new(
                self.vcs_info.root_path.clone(),
                newest_commit_id,
                self.vcs_info.branch_name.clone(),
                SessionDiffSource::CommitRange,
            );
            session.commit_range = Some(selected_ids.clone());
            session
        });

        if session.commit_range.is_none() {
            session.commit_range = Some(selected_ids.clone());
            session.updated_at = chrono::Utc::now();
        }

        self.session = session;

        // Add files to session
        for file in &diff_files {
            self.session.add_diff_file(file);
        }
        self.reset_persisted_session_tracking();

        // Update app state
        self.diff_files = diff_files;
        self.diff_source = DiffSource::CommitRange(selected_ids);
        self.input_mode = InputMode::Normal;

        // Reset navigation state
        self.diff_state = DiffState::default();
        self.file_list_state = FileListState::default();

        // Set up inline commit selector for multi-commit reviews (newest-first display order)
        self.pr_commits.clear();
        self.pr_last_reviewed_commit_index = None;
        self.review_commits = selected_commits.iter().rev().cloned().collect();
        self.range_diff_files = Some(self.diff_files.clone());
        self.commit_list = self.review_commits.clone();
        self.commit_list_cursor = 0;
        self.commit_selection_range = if self.review_commits.is_empty() {
            None
        } else {
            Some((0, self.review_commits.len() - 1))
        };
        self.commit_list_scroll_offset = 0;
        self.visible_commit_count = self.review_commits.len();
        self.has_more_commit = false;
        self.show_commit_selector = self.review_commits.len() > 1;
        self.commit_diff_cache.clear();
        self.saved_inline_selection = None;

        self.sort_files_by_directory(true);
        self.expand_all_dirs();
        self.rebuild_annotations();

        Ok(())
    }

    /// Reload the diff for the currently selected inline commit subrange.
    pub fn reload_inline_selection(&mut self) -> Result<()> {
        let Some((start, end)) = self.commit_selection_range else {
            self.set_message("Select at least one commit");
            return Ok(());
        };

        // Check if all commits selected -> use cached range_diff_files
        if start == 0
            && end == self.review_commits.len() - 1
            && let Some(ref files) = self.range_diff_files
        {
            self.diff_files = files.clone();
            let wrap = self.diff_state.wrap_lines;
            self.diff_state = DiffState::default();
            self.diff_state.wrap_lines = wrap;
            self.file_list_state = FileListState::default();
            self.expanded_top.clear();
            self.expanded_bottom.clear();
            self.insert_commit_message_if_single();
            self.sort_files_by_directory(true);
            self.expand_all_dirs();
            self.rebuild_annotations();
            return Ok(());
        }

        // Check cache for this subrange
        if let Some(files) = self.commit_diff_cache.get(&(start, end)) {
            self.diff_files = files.clone();
            let wrap = self.diff_state.wrap_lines;
            self.diff_state = DiffState::default();
            self.diff_state.wrap_lines = wrap;
            self.file_list_state = FileListState::default();
            self.expanded_top.clear();
            self.expanded_bottom.clear();
            self.insert_commit_message_if_single();
            self.sort_files_by_directory(true);
            self.expand_all_dirs();
            self.rebuild_annotations();
            return Ok(());
        }

        // Load diff for selected subrange
        let has_staged = (start..=end).any(|i| {
            self.review_commits
                .get(i)
                .is_some_and(Self::is_staged_commit)
        });
        let has_unstaged = (start..=end).any(|i| {
            self.review_commits
                .get(i)
                .is_some_and(Self::is_unstaged_commit)
        });
        let selected_ids: Vec<String> = (start..=end)
            .rev() // oldest to newest
            .filter_map(|i| self.review_commits.get(i))
            .filter(|c| !Self::is_special_commit(c))
            .map(|c| c.id.clone())
            .collect();

        let highlighter = self.theme.syntax_highlighter();
        let diff_files = if (has_staged || has_unstaged) && !selected_ids.is_empty() {
            match Self::get_working_tree_with_commits_diff_with_ignore(
                self.vcs.as_ref(),
                &self.vcs_info.root_path,
                &selected_ids,
                highlighter,
                self.path_filter.as_deref(),
            ) {
                Ok(files) => files,
                Err(TuicrError::NoChanges) => Vec::new(),
                Err(e) => return Err(e),
            }
        } else if has_staged && has_unstaged {
            match Self::get_working_tree_diff_with_ignore(
                self.vcs.as_ref(),
                &self.vcs_info.root_path,
                highlighter,
                self.path_filter.as_deref(),
            ) {
                Ok(files) => files,
                Err(TuicrError::NoChanges) => Vec::new(),
                Err(e) => return Err(e),
            }
        } else if has_staged {
            match Self::get_staged_diff_with_ignore(
                self.vcs.as_ref(),
                &self.vcs_info.root_path,
                highlighter,
                self.path_filter.as_deref(),
            ) {
                Ok(files) => files,
                Err(TuicrError::NoChanges) => Vec::new(),
                Err(e) => return Err(e),
            }
        } else if has_unstaged {
            match Self::get_unstaged_diff_with_ignore(
                self.vcs.as_ref(),
                &self.vcs_info.root_path,
                highlighter,
                self.path_filter.as_deref(),
            ) {
                Ok(files) => files,
                Err(TuicrError::NoChanges) => Vec::new(),
                Err(e) => return Err(e),
            }
        } else {
            match Self::get_commit_range_diff_with_ignore(
                self.vcs.as_ref(),
                &self.vcs_info.root_path,
                &ResolvedRevisionRange::from_commit_ids(
                    &selected_ids,
                    RevisionDiffTarget::CommitList,
                ),
                highlighter,
                self.path_filter.as_deref(),
            ) {
                Ok(files) => files,
                Err(TuicrError::NoChanges) => Vec::new(),
                Err(e) => return Err(e),
            }
        };
        self.commit_diff_cache
            .insert((start, end), diff_files.clone());
        self.diff_files = diff_files;

        // Reset navigation, rebuild file tree + annotations
        let wrap = self.diff_state.wrap_lines;
        self.diff_state = DiffState::default();
        self.diff_state.wrap_lines = wrap;
        self.file_list_state = FileListState::default();
        self.expanded_top.clear();
        self.expanded_bottom.clear();
        self.insert_commit_message_if_single();
        self.sort_files_by_directory(true);
        self.expand_all_dirs();
        self.rebuild_annotations();

        Ok(())
    }
}
