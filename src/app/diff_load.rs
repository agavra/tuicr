use super::*;

impl App {
    pub(in crate::app) fn staged_commit_entry() -> CommitInfo {
        CommitInfo {
            id: STAGED_SELECTION_ID.to_string(),
            short_id: "STAGED".to_string(),
            branch_name: None,
            summary: "Staged changes".to_string(),
            body: None,
            author: String::new(),
            time: Utc::now(),
        }
    }

    pub(in crate::app) fn unstaged_commit_entry() -> CommitInfo {
        CommitInfo {
            id: UNSTAGED_SELECTION_ID.to_string(),
            short_id: "UNSTAGED".to_string(),
            branch_name: None,
            summary: "Unstaged changes".to_string(),
            body: None,
            author: String::new(),
            time: Utc::now(),
        }
    }

    /// If we are viewing a single commit, insert a "Commit Message" DiffFile at index 0.
    ///
    /// The synthetic path embeds the commit's short id (`Commit Message (<sha>)`)
    /// so that comments on different commits' messages get distinct session keys
    /// (the session indexes comments by path) and the exported review records
    /// which commit each commit-message comment belongs to.
    pub(in crate::app) fn insert_commit_message_if_single(&mut self) {
        self.diff_files.retain(|f| !f.is_commit_message);

        let commit = if let Some((start, end)) = self.commit_selection_range {
            if start == end {
                self.review_commits.get(start)
            } else {
                None
            }
        } else if self.review_commits.len() == 1 {
            self.review_commits.first()
        } else {
            None
        };

        let Some(commit) = commit else { return };
        if Self::is_special_commit(commit) {
            return;
        }

        let mut full_message = commit.summary.clone();
        if let Some(ref body) = commit.body {
            full_message.push('\n');
            full_message.push('\n');
            full_message.push_str(body);
        }

        let diff_lines: Vec<DiffLine> = full_message
            .lines()
            .enumerate()
            .map(|(i, line)| DiffLine {
                origin: LineOrigin::Context,
                content: line.to_string(),
                old_lineno: None,
                new_lineno: Some(i as u32 + 1),
                highlighted_spans: None,
            })
            .collect();
        let line_count = diff_lines.len() as u32;
        let hunks = vec![DiffHunk {
            header: String::new(),
            lines: diff_lines,
            old_start: 0,
            old_count: 0,
            new_start: 1,
            new_count: line_count,
        }];
        let content_hash = DiffFile::compute_content_hash(&hunks);
        let commit_msg_file = DiffFile {
            old_path: None,
            new_path: Some(PathBuf::from(format!(
                "Commit Message ({})",
                commit.short_id
            ))),
            status: FileStatus::Added,
            hunks,
            is_binary: false,
            is_too_large: false,
            is_commit_message: true,
            content_hash,
        };
        self.diff_files.insert(0, commit_msg_file);
        self.session.add_diff_file(&self.diff_files[0]);
    }

    pub(in crate::app) fn is_staged_commit(commit: &CommitInfo) -> bool {
        commit.id == STAGED_SELECTION_ID
    }

    pub(in crate::app) fn is_unstaged_commit(commit: &CommitInfo) -> bool {
        commit.id == UNSTAGED_SELECTION_ID
    }

    pub(in crate::app) fn is_special_commit(commit: &CommitInfo) -> bool {
        Self::is_staged_commit(commit) || Self::is_unstaged_commit(commit)
    }

    pub(in crate::app) fn special_commit_count(&self) -> usize {
        self.commit_list
            .iter()
            .take_while(|commit| Self::is_special_commit(commit))
            .count()
    }

    pub(in crate::app) fn loaded_history_commit_count(&self) -> usize {
        self.commit_list
            .len()
            .saturating_sub(self.special_commit_count())
    }

    pub(in crate::app) fn filter_ignored_diff_files(
        repo_root: &Path,
        diff_files: Vec<DiffFile>,
    ) -> Vec<DiffFile> {
        crate::tuicrignore::filter_diff_files(repo_root, diff_files)
    }

    fn filter_by_path(diff_files: Vec<DiffFile>, path: &str) -> Vec<DiffFile> {
        let path = path.trim_end_matches('/');
        diff_files
            .into_iter()
            .filter(|f| {
                let display = f.display_path().to_string_lossy();
                display == path || display.starts_with(&format!("{path}/"))
            })
            .collect()
    }

    fn require_non_empty_diff_files(diff_files: Vec<DiffFile>) -> Result<Vec<DiffFile>> {
        if diff_files.is_empty() {
            return Err(TuicrError::NoChanges);
        }
        Ok(diff_files)
    }

    pub(in crate::app) fn get_working_tree_diff_with_ignore(
        vcs: &dyn VcsBackend,
        repo_root: &Path,
        highlighter: &SyntaxHighlighter,
        path_filter: Option<&str>,
    ) -> Result<Vec<DiffFile>> {
        let diff_files = crate::profile::time_with(
            "diff.load_working_tree",
            || vcs.get_working_tree_diff(highlighter),
            profile_diff_result,
        )?;
        let diff_files = Self::filter_ignored_diff_files(repo_root, diff_files);
        let diff_files = if let Some(path) = path_filter {
            Self::filter_by_path(diff_files, path)
        } else {
            diff_files
        };
        Self::require_non_empty_diff_files(diff_files)
    }

    pub(in crate::app) fn get_staged_diff_with_ignore(
        vcs: &dyn VcsBackend,
        repo_root: &Path,
        highlighter: &SyntaxHighlighter,
        path_filter: Option<&str>,
    ) -> Result<Vec<DiffFile>> {
        let diff_files = crate::profile::time_with(
            "diff.load_staged",
            || vcs.get_staged_diff(highlighter),
            profile_diff_result,
        )?;
        let diff_files = Self::filter_ignored_diff_files(repo_root, diff_files);
        let diff_files = if let Some(path) = path_filter {
            Self::filter_by_path(diff_files, path)
        } else {
            diff_files
        };
        Self::require_non_empty_diff_files(diff_files)
    }

    pub(in crate::app) fn get_unstaged_diff_with_ignore(
        vcs: &dyn VcsBackend,
        repo_root: &Path,
        highlighter: &SyntaxHighlighter,
        path_filter: Option<&str>,
    ) -> Result<Vec<DiffFile>> {
        let diff_files = match crate::profile::time_with(
            "diff.load_unstaged",
            || vcs.get_unstaged_diff(highlighter),
            profile_diff_result,
        ) {
            Ok(diff_files) => diff_files,
            Err(TuicrError::UnsupportedOperation(_)) => crate::profile::time_with(
                "diff.load_unstaged_fallback_working_tree",
                || vcs.get_working_tree_diff(highlighter),
                profile_diff_result,
            )?,
            Err(e) => return Err(e),
        };
        let diff_files = Self::filter_ignored_diff_files(repo_root, diff_files);
        let diff_files = if let Some(path) = path_filter {
            Self::filter_by_path(diff_files, path)
        } else {
            diff_files
        };
        Self::require_non_empty_diff_files(diff_files)
    }

    pub(in crate::app) fn get_commit_range_diff_with_ignore(
        vcs: &dyn VcsBackend,
        repo_root: &Path,
        revision_range: &ResolvedRevisionRange<'_>,
        highlighter: &SyntaxHighlighter,
        path_filter: Option<&str>,
    ) -> Result<Vec<DiffFile>> {
        let diff_files = crate::profile::time_with(
            "diff.load_commit_range",
            || vcs.get_commit_range_diff(revision_range, highlighter),
            profile_diff_result,
        )?;
        let diff_files = Self::filter_ignored_diff_files(repo_root, diff_files);
        let diff_files = if let Some(path) = path_filter {
            Self::filter_by_path(diff_files, path)
        } else {
            diff_files
        };
        Self::require_non_empty_diff_files(diff_files)
    }

    pub(in crate::app) fn get_working_tree_with_commits_diff_with_ignore(
        vcs: &dyn VcsBackend,
        repo_root: &Path,
        commit_ids: &[String],
        highlighter: &SyntaxHighlighter,
        path_filter: Option<&str>,
    ) -> Result<Vec<DiffFile>> {
        let diff_files = crate::profile::time_with(
            "diff.load_working_tree_with_commits",
            || vcs.get_working_tree_with_commits_diff(commit_ids, highlighter),
            profile_diff_result,
        )?;
        let diff_files = Self::filter_ignored_diff_files(repo_root, diff_files);
        let diff_files = if let Some(path) = path_filter {
            Self::filter_by_path(diff_files, path)
        } else {
            diff_files
        };
        Self::require_non_empty_diff_files(diff_files)
    }

    /// Resolve the staged/unstaged status the commit selector renders.
    ///
    /// When `.gitignore`/`.tuicrignore` rules are present the cheap probe alone
    /// can't be trusted — a file the probe sees may be ignored. To verify
    /// without paying the full-diff cost, we ask the backend for just the
    /// changed paths and filter them through the same ignore rules. Backends
    /// that don't expose a path probe fall back to parsing the full diff.
    pub(in crate::app) fn get_change_status_with_ignore(
        vcs: &dyn VcsBackend,
        repo_root: &Path,
        highlighter: &SyntaxHighlighter,
        path_filter: Option<&str>,
    ) -> Result<VcsChangeStatus> {
        if path_filter.is_none() {
            match vcs.get_change_status() {
                Ok(status) => {
                    if !crate::tuicrignore::has_ignore_rules(repo_root) {
                        return Ok(status);
                    }
                    return Self::verify_status_against_ignore(
                        vcs,
                        repo_root,
                        highlighter,
                        path_filter,
                        status,
                    );
                }
                Err(TuicrError::UnsupportedOperation(_)) => {}
                Err(e) => return Err(e),
            }
        }

        Self::verify_status_against_ignore(
            vcs,
            repo_root,
            highlighter,
            path_filter,
            VcsChangeStatus {
                staged: true,
                unstaged: true,
            },
        )
    }

    /// Refine `assumed_status` by checking each side actually has at least one
    /// non-ignored, non-filtered path. Tries the cheap path probe first; falls
    /// back to parsing the full diff for backends that don't implement it.
    fn verify_status_against_ignore(
        vcs: &dyn VcsBackend,
        repo_root: &Path,
        highlighter: &SyntaxHighlighter,
        path_filter: Option<&str>,
        assumed_status: VcsChangeStatus,
    ) -> Result<VcsChangeStatus> {
        let staged = if assumed_status.staged {
            Self::side_has_visible_changes(
                vcs,
                repo_root,
                highlighter,
                path_filter,
                ChangeKind::Staged,
            )?
        } else {
            false
        };
        let unstaged = if assumed_status.unstaged {
            Self::side_has_visible_changes(
                vcs,
                repo_root,
                highlighter,
                path_filter,
                ChangeKind::Unstaged,
            )?
        } else {
            false
        };
        Ok(VcsChangeStatus { staged, unstaged })
    }

    fn side_has_visible_changes(
        vcs: &dyn VcsBackend,
        repo_root: &Path,
        highlighter: &SyntaxHighlighter,
        path_filter: Option<&str>,
        kind: ChangeKind,
    ) -> Result<bool> {
        match vcs.list_changed_paths(kind) {
            Ok(paths) => Ok(Self::any_path_survives_filters(
                paths,
                repo_root,
                path_filter,
            )),
            Err(TuicrError::UnsupportedOperation(_)) => {
                // Backend can't list paths cheaply — parse the diff to see if
                // anything survives. This still happens for jj/hg today.
                let diff_result = match kind {
                    ChangeKind::Staged => {
                        Self::get_staged_diff_with_ignore(vcs, repo_root, highlighter, path_filter)
                    }
                    ChangeKind::Unstaged => Self::get_unstaged_diff_with_ignore(
                        vcs,
                        repo_root,
                        highlighter,
                        path_filter,
                    ),
                };
                match diff_result {
                    Ok(_) => Ok(true),
                    Err(TuicrError::NoChanges) | Err(TuicrError::UnsupportedOperation(_)) => {
                        Ok(false)
                    }
                    Err(e) => Err(e),
                }
            }
            Err(e) => Err(e),
        }
    }

    fn any_path_survives_filters(
        paths: Vec<PathBuf>,
        repo_root: &Path,
        path_filter: Option<&str>,
    ) -> bool {
        let after_ignore = crate::tuicrignore::filter_paths(repo_root, paths);
        let after_path = match path_filter {
            Some(p) => Self::filter_paths_by_pathspec(after_ignore, p),
            None => after_ignore,
        };
        !after_path.is_empty()
    }

    fn filter_paths_by_pathspec(paths: Vec<PathBuf>, pathspec: &str) -> Vec<PathBuf> {
        let pathspec = pathspec.trim_end_matches('/');
        paths
            .into_iter()
            .filter(|p| {
                let display = p.to_string_lossy();
                display == pathspec || display.starts_with(&format!("{pathspec}/"))
            })
            .collect()
    }

    pub(in crate::app) fn load_staged_and_unstaged_selection(&mut self) -> Result<()> {
        let highlighter = self.theme.syntax_highlighter();
        let diff_files = match Self::get_working_tree_diff_with_ignore(
            self.vcs.as_ref(),
            &self.vcs_info.root_path,
            highlighter,
            self.path_filter.as_deref(),
        ) {
            Ok(diff_files) => diff_files,
            Err(TuicrError::NoChanges) => {
                self.set_message("No staged or unstaged changes");
                return Ok(());
            }
            Err(e) => return Err(e),
        };

        self.session =
            Self::load_or_create_session(&self.vcs_info, SessionDiffSource::StagedAndUnstaged);
        for file in &diff_files {
            self.session.add_diff_file(file);
        }
        self.reset_persisted_session_tracking();

        self.diff_files = diff_files;
        self.diff_source = DiffSource::StagedAndUnstaged;
        self.input_mode = InputMode::Normal;
        self.diff_state = DiffState::default();
        self.file_list_state = FileListState::default();
        self.clear_expanded_gaps();
        self.sort_files_by_directory(true);
        self.expand_all_dirs();
        self.rebuild_annotations();

        Ok(())
    }

    pub(in crate::app) fn load_staged_selection(&mut self) -> Result<()> {
        let highlighter = self.theme.syntax_highlighter();
        let diff_files = match Self::get_staged_diff_with_ignore(
            self.vcs.as_ref(),
            &self.vcs_info.root_path,
            highlighter,
            self.path_filter.as_deref(),
        ) {
            Ok(diff_files) => diff_files,
            Err(TuicrError::NoChanges) => {
                self.set_message("No staged changes");
                return Ok(());
            }
            Err(e) => return Err(e),
        };

        self.session = Self::load_or_create_session(&self.vcs_info, SessionDiffSource::Staged);
        for file in &diff_files {
            self.session.add_diff_file(file);
        }
        self.reset_persisted_session_tracking();

        self.diff_files = diff_files;
        self.diff_source = DiffSource::Staged;
        self.input_mode = InputMode::Normal;
        self.diff_state = DiffState::default();
        self.file_list_state = FileListState::default();
        self.clear_expanded_gaps();
        self.sort_files_by_directory(true);
        self.expand_all_dirs();
        self.rebuild_annotations();

        Ok(())
    }

    pub(in crate::app) fn load_unstaged_selection(&mut self) -> Result<()> {
        let highlighter = self.theme.syntax_highlighter();
        let diff_files = match Self::get_unstaged_diff_with_ignore(
            self.vcs.as_ref(),
            &self.vcs_info.root_path,
            highlighter,
            self.path_filter.as_deref(),
        ) {
            Ok(diff_files) => diff_files,
            Err(TuicrError::NoChanges) => {
                self.set_message("No unstaged changes");
                return Ok(());
            }
            Err(e) => return Err(e),
        };

        self.session = Self::load_or_create_session(&self.vcs_info, SessionDiffSource::Unstaged);
        for file in &diff_files {
            self.session.add_diff_file(file);
        }
        self.reset_persisted_session_tracking();

        self.diff_files = diff_files;
        self.diff_source = DiffSource::Unstaged;
        self.input_mode = InputMode::Normal;
        self.diff_state = DiffState::default();
        self.file_list_state = FileListState::default();
        self.clear_expanded_gaps();
        self.sort_files_by_directory(true);
        self.expand_all_dirs();
        self.rebuild_annotations();

        Ok(())
    }

    /// Reloads diff files from disk. Returns `(file_count, invalidated_count)` where
    /// `invalidated_count` is the number of previously reviewed files whose content changed.
    pub fn reload_diff_files(&mut self) -> Result<(usize, usize)> {
        let current_path = self.current_file_path().cloned();
        let prev_file_idx = self.diff_state.current_file_idx;
        let prev_cursor_line = self.diff_state.cursor_line;
        let prev_viewport_offset = self
            .diff_state
            .cursor_line
            .saturating_sub(self.diff_state.scroll_offset);
        let prev_relative_line = if self.diff_files.is_empty() {
            0
        } else {
            let start = self.calculate_file_scroll_offset(self.diff_state.current_file_idx);
            prev_cursor_line.saturating_sub(start)
        };

        let highlighter = self.theme.syntax_highlighter();
        let diff_files = match &self.diff_source {
            DiffSource::CommitRange(commit_ids) => Self::get_commit_range_diff_with_ignore(
                self.vcs.as_ref(),
                &self.vcs_info.root_path,
                &ResolvedRevisionRange::from_commit_ids(commit_ids, RevisionDiffTarget::CommitList),
                highlighter,
                self.path_filter.as_deref(),
            )?,
            DiffSource::StagedUnstagedAndCommits(commit_ids) => {
                let ids = commit_ids.clone();
                Self::get_working_tree_with_commits_diff_with_ignore(
                    self.vcs.as_ref(),
                    &self.vcs_info.root_path,
                    &ids,
                    highlighter,
                    self.path_filter.as_deref(),
                )?
            }
            DiffSource::Staged => Self::get_staged_diff_with_ignore(
                self.vcs.as_ref(),
                &self.vcs_info.root_path,
                highlighter,
                self.path_filter.as_deref(),
            )?,
            DiffSource::Unstaged => Self::get_unstaged_diff_with_ignore(
                self.vcs.as_ref(),
                &self.vcs_info.root_path,
                highlighter,
                self.path_filter.as_deref(),
            )?,
            DiffSource::StagedAndUnstaged | DiffSource::WorkingTree => {
                Self::get_working_tree_diff_with_ignore(
                    self.vcs.as_ref(),
                    &self.vcs_info.root_path,
                    highlighter,
                    self.path_filter.as_deref(),
                )?
            }
            DiffSource::PullRequest(_) => {
                // PR reload is a separate code path that may switch sessions
                // when the head SHA advances; callers dispatch via
                // `reload_pull_request` instead of going through this
                // local-reload helper.
                return Err(TuicrError::UnsupportedOperation(
                    "Use :reload from the command line in PR mode".to_string(),
                ));
            }
        };

        let mut invalidated = 0;
        for file in &diff_files {
            if self.session.add_diff_file(file) {
                invalidated += 1;
            }
        }

        self.diff_files = diff_files;
        self.clear_expanded_gaps();

        self.sort_files_by_directory(false);
        self.populate_file_line_count_cache();
        self.expand_all_dirs();

        if self.diff_files.is_empty() {
            self.diff_state.current_file_idx = 0;
            self.diff_state.cursor_line = 0;
            self.diff_state.scroll_offset = 0;
            self.file_list_state.select(0);
        } else {
            let target_idx = if let Some(path) = current_path {
                self.diff_files
                    .iter()
                    .position(|file| file.display_path() == &path)
                    .unwrap_or_else(|| prev_file_idx.min(self.diff_files.len().saturating_sub(1)))
            } else {
                prev_file_idx.min(self.diff_files.len().saturating_sub(1))
            };

            self.jump_to_file(target_idx);

            let file_start = self.calculate_file_scroll_offset(target_idx);
            let file_height = self.file_render_height(target_idx, &self.diff_files[target_idx]);
            let relative_line = prev_relative_line.min(file_height.saturating_sub(1));
            self.diff_state.cursor_line = file_start.saturating_add(relative_line);

            let viewport = self.diff_state.viewport_height.max(1);
            let max_relative = viewport.saturating_sub(1);
            let relative_offset = prev_viewport_offset.min(max_relative);
            if self.total_lines() == 0 {
                self.diff_state.scroll_offset = 0;
            } else {
                let max_scroll = self.max_scroll_offset();
                let desired = self
                    .diff_state
                    .cursor_line
                    .saturating_sub(relative_offset)
                    .min(max_scroll);
                self.diff_state.scroll_offset = desired;
            }

            self.ensure_cursor_visible();
            self.update_current_file_from_cursor();
        }

        self.rebuild_annotations();
        Ok((self.diff_files.len(), invalidated))
    }

    pub(in crate::app) fn load_staged_unstaged_and_commits_selection(
        &mut self,
        selected_ids: Vec<String>,
        selected_commits: Vec<CommitInfo>,
    ) -> Result<()> {
        let highlighter = self.theme.syntax_highlighter();
        let diff_files = match Self::get_working_tree_with_commits_diff_with_ignore(
            self.vcs.as_ref(),
            &self.vcs_info.root_path,
            &selected_ids,
            highlighter,
            self.path_filter.as_deref(),
        ) {
            Ok(diff_files) => diff_files,
            Err(TuicrError::NoChanges) => {
                self.set_message("No changes in selected commits + staged/unstaged");
                return Ok(());
            }
            Err(e) => return Err(e),
        };

        self.session =
            Self::load_or_create_staged_unstaged_and_commits_session(&self.vcs_info, &selected_ids);

        for file in &diff_files {
            self.session.add_diff_file(file);
        }
        self.reset_persisted_session_tracking();

        self.diff_files = diff_files;
        self.diff_source = DiffSource::StagedUnstagedAndCommits(selected_ids);
        self.input_mode = InputMode::Normal;
        self.diff_state = DiffState::default();
        self.file_list_state = FileListState::default();

        // Set up inline commit selector (newest-first display order)
        self.pr_commits.clear();
        self.pr_last_reviewed_commit_index = None;
        self.review_commits = selected_commits.into_iter().rev().collect();
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

        self.insert_commit_message_if_single();
        self.sort_files_by_directory(true);
        self.expand_all_dirs();
        self.rebuild_annotations();
        Ok(())
    }
}
