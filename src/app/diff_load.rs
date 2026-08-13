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
                    if !crate::tuicrignore::has_tuicrignore(repo_root) {
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

    /// Fetches diff files for the current `diff_source`.
    ///
    /// Returns `Err(UnsupportedOperation)` for `DiffSource::PullRequest`: PR
    /// reload is a separate code path that may switch sessions when the head
    /// SHA advances, so callers dispatch via `reload_pull_request` instead of
    /// going through this local-reload helper.
    fn fetch_diff_files(&self) -> Result<Vec<DiffFile>> {
        self.fetch_diff_files_with(self.theme.syntax_highlighter())
    }

    /// Same as `fetch_diff_files`, with the highlighter as a parameter so the
    /// diff-watch gate can run a cheap parse against `probe_highlighter()` before
    /// deciding to pay for a highlighted one.
    fn fetch_diff_files_with(&self, highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
        let fetch_source = Self::narrowed_fetch_source(
            &self.diff_source,
            &self.review_commits,
            self.commit_selection_range,
        );
        Self::fetch_diff_files_for_source(
            self.vcs.as_ref(),
            &self.vcs_info.root_path,
            &fetch_source,
            highlighter,
            self.path_filter.as_deref(),
        )
    }

    /// Which `DiffSource` a fetch should run against.
    ///
    /// A narrowed commit selection has to survive a reload. Fetching
    /// `diff_source` directly would use its whole commit list and widen the
    /// review back out while the selector still shows the subset. Anything not
    /// narrowed gets `diff_source` back untouched.
    ///
    /// It returns a value instead of fetching so the caller can resolve it on
    /// the main thread and hand the result to a worker, rather than sharing
    /// `App` across threads.
    fn narrowed_fetch_source(
        diff_source: &DiffSource,
        review_commits: &[CommitInfo],
        commit_selection_range: Option<(usize, usize)>,
    ) -> DiffSource {
        if !matches!(
            diff_source,
            DiffSource::CommitRange(_) | DiffSource::StagedUnstagedAndCommits(_)
        ) {
            return diff_source.clone();
        }
        if !Self::is_strict_commit_selection(commit_selection_range, review_commits.len()) {
            return diff_source.clone();
        }
        let (start, end) =
            commit_selection_range.expect("is_strict_commit_selection guarantees Some(range)");
        Self::source_for_commit_subrange(review_commits, start, end)
    }

    /// Where the current commit selection lands in a rebuilt pane.
    ///
    /// Rebuilding renumbers `review_commits`, and `commit_selection_range`
    /// holds raw indices into that list. Carrying those indices over leaves
    /// them in bounds but pointing at different commits, so the review scope
    /// changes with no error and no warning. Both endpoints are therefore
    /// re-found by commit id, which the two synthetic rows carry as well
    /// (`STAGED_SELECTION_ID`, `UNSTAGED_SELECTION_ID`).
    ///
    /// A commit landing between the endpoints joins the selection. A range
    /// means both ends and everything between them, and the next tick's diff
    /// covers the new commit and says so on screen.
    pub(in crate::app) fn reanchored_commit_selection(
        commit_selection_range: Option<(usize, usize)>,
        current: &[CommitInfo],
        rebuilt: &[CommitInfo],
    ) -> CommitSelectionAnchor {
        if rebuilt.is_empty() {
            return CommitSelectionAnchor::Moved(None);
        }
        let last = rebuilt.len() - 1;
        // No selection, or one covering every row. Either way the range means
        // "everything", and everything is now a different number of rows, so
        // there are no endpoints worth preserving. `filter` keeps the range and
        // the verdict about it in one value, so the narrowed branch below
        // cannot run without the pair it needs.
        let narrowed = commit_selection_range
            .filter(|range| Self::is_strict_commit_selection(Some(*range), current.len()));
        let Some((start, end)) = narrowed else {
            return CommitSelectionAnchor::Moved(Some((0, last)));
        };
        let (Some(start_row), Some(end_row)) = (current.get(start), current.get(end)) else {
            return CommitSelectionAnchor::Lost;
        };
        let position = |id: &str| rebuilt.iter().position(|row| row.id == id);
        match (position(&start_row.id), position(&end_row.id)) {
            // `commit_pane_rows` always keeps commits newest-first, synthetic
            // rows in front. An inverted pair means that broke. Swapping the
            // ends would give back a range over whatever sits between them now.
            (Some(new_start), Some(new_end)) if new_start <= new_end => {
                CommitSelectionAnchor::Moved(Some((new_start, new_end)))
            }
            _ => CommitSelectionAnchor::Lost,
        }
    }

    /// Which diff a commit-selector subrange means.
    ///
    /// The selector can hold real commits plus two synthetic entries, one for
    /// staged changes and one for unstaged. Which combination the user picked
    /// decides which diff to fetch.
    ///
    /// Two callers share it: `narrowed_fetch_source` asks what a reload should
    /// fetch, and `reload_inline_selection` (`src/app/commits.rs`) asks what
    /// the selector should load. Keep it that way. A second copy would have to
    /// agree with this one about every case, including that the selector
    /// stores commits newest-first while every commit list runs oldest-first.
    pub(in crate::app) fn source_for_commit_subrange(
        review_commits: &[CommitInfo],
        start: usize,
        end: usize,
    ) -> DiffSource {
        let has_staged =
            (start..=end).any(|i| review_commits.get(i).is_some_and(Self::is_staged_commit));
        let has_unstaged =
            (start..=end).any(|i| review_commits.get(i).is_some_and(Self::is_unstaged_commit));
        let selected_ids: Vec<String> = (start..=end)
            .rev() // oldest to newest, matching every other DiffSource commit list
            .filter_map(|i| review_commits.get(i))
            .filter(|c| !Self::is_special_commit(c))
            .map(|c| c.id.clone())
            .collect();

        // Matched on the three facts that decide the answer, so every case is
        // visible at once and the compiler rejects a missing one.
        match (has_staged, has_unstaged, selected_ids.is_empty()) {
            // No staged or unstaged entry in the selection, so it is only commits.
            (false, false, _) => DiffSource::CommitRange(selected_ids),
            // A special entry plus real commits.
            (_, _, false) => DiffSource::StagedUnstagedAndCommits(selected_ids),
            // Special entries only, no commits alongside them.
            (true, true, true) => DiffSource::StagedAndUnstaged,
            (true, false, true) => DiffSource::Staged,
            (false, true, true) => DiffSource::Unstaged,
        }
    }

    /// Backend-agnostic core of `fetch_diff_files_with`: takes `vcs` and
    /// `root_path` as parameters instead of reading `self.vcs`/`self.vcs_info`
    /// so the diff-watch worker thread (`diff_watch_fetch`) can call it
    /// against a backend it opened itself, without borrowing `App`.
    pub(in crate::app) fn fetch_diff_files_for_source(
        vcs: &dyn VcsBackend,
        root_path: &Path,
        diff_source: &DiffSource,
        highlighter: &SyntaxHighlighter,
        path_filter: Option<&str>,
    ) -> Result<Vec<DiffFile>> {
        match diff_source {
            DiffSource::CommitRange(commit_ids) => Self::get_commit_range_diff_with_ignore(
                vcs,
                root_path,
                &ResolvedRevisionRange::from_commit_ids(commit_ids, RevisionDiffTarget::CommitList),
                highlighter,
                path_filter,
            ),
            DiffSource::StagedUnstagedAndCommits(commit_ids) => {
                Self::get_working_tree_with_commits_diff_with_ignore(
                    vcs,
                    root_path,
                    commit_ids,
                    highlighter,
                    path_filter,
                )
            }
            DiffSource::Staged => {
                Self::get_staged_diff_with_ignore(vcs, root_path, highlighter, path_filter)
            }
            DiffSource::Unstaged => {
                Self::get_unstaged_diff_with_ignore(vcs, root_path, highlighter, path_filter)
            }
            DiffSource::StagedAndUnstaged | DiffSource::WorkingTree => {
                Self::get_working_tree_diff_with_ignore(vcs, root_path, highlighter, path_filter)
            }
            DiffSource::PullRequest(_) => Err(TuicrError::UnsupportedOperation(
                "Use :reload from the command line in PR mode".to_string(),
            )),
        }
    }

    /// Applies fetched diff files: session bookkeeping, tree/gap reset, and
    /// best-effort cursor restoration. Returns `(file_count, invalidated_count)`
    /// where `invalidated_count` is the number of previously reviewed files
    /// whose content changed.
    ///
    /// The cursor-capture at the top reads `self.diff_files` and
    /// `self.diff_state` as they stood before this call. The synchronous
    /// callers get that for free: the fetch that produced `diff_files` took
    /// `&self` and could not have changed them in between. The diff-watch
    /// worker fetches off-thread, so it has no such guarantee. Callers
    /// reached from a background result must check relevance first, which
    /// `poll_diff_watch_changes` does via `diff_watch_result_is_stale`.
    fn apply_diff_files(&mut self, diff_files: Vec<DiffFile>) -> (usize, usize) {
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
        (self.diff_files.len(), invalidated)
    }

    /// Reloads diff files from disk. Returns `(file_count, invalidated_count)` where
    /// `invalidated_count` is the number of previously reviewed files whose content changed.
    pub fn reload_diff_files(&mut self) -> Result<(usize, usize)> {
        let diff_files = self.fetch_diff_files()?;
        Ok(self.apply_diff_files(diff_files))
    }

    /// Returns the freshly fetched diff only when it differs from what is
    /// currently on screen. `None` means identical: there is nothing to apply.
    ///
    /// Binds `changed_diff_files_for_source`, the shared gate the diff-watch
    /// worker also runs via `diff_watch_fetch`, to this app's own backend and
    /// current fingerprint.
    ///
    /// Test-only since the watcher moved to a worker thread. The gate itself is
    /// production code; this wrapper just supplies the arguments a synchronous
    /// caller would, so `diff_reload_tests.rs` can exercise it without threads.
    #[cfg(test)]
    pub(in crate::app) fn fetch_changed_diff_files(&self) -> Result<Option<Vec<DiffFile>>> {
        let fetch_source = Self::narrowed_fetch_source(
            &self.diff_source,
            &self.review_commits,
            self.commit_selection_range,
        );
        Self::changed_diff_files_for_source(
            self.vcs.as_ref(),
            &self.vcs_info.root_path,
            &fetch_source,
            self.theme.syntax_highlighter(),
            self.path_filter.as_deref(),
            diff_files_fingerprint(&self.diff_files),
        )
    }

    /// The probe-then-real-fetch gate: returns the fetched diff only when it
    /// differs from `current`, the fingerprint of what's already on screen.
    /// Shared by `fetch_changed_diff_files` (main thread, `self.vcs`) and
    /// `diff_watch_fetch` (worker thread, its own freshly opened backend) so
    /// the decision exists once and both run identically.
    ///
    /// `fetch_source` must already be narrowed. Callers run
    /// `narrowed_fetch_source` first, so this function stays about the gate and
    /// knows nothing about commit selection.
    ///
    /// The comparison runs against a parse that skips syntax highlighting first.
    /// That is 98% of the cost and fingerprints identically, so an unchanged
    /// tick costs roughly 3ms instead of 195ms on a 4,000-line diff.
    fn changed_diff_files_for_source(
        vcs: &dyn VcsBackend,
        root_path: &Path,
        fetch_source: &DiffSource,
        highlighter: &SyntaxHighlighter,
        path_filter: Option<&str>,
        current: u64,
    ) -> Result<Option<Vec<DiffFile>>> {
        let probe = Self::fetch_diff_files_for_source(
            vcs,
            root_path,
            fetch_source,
            probe_highlighter(),
            path_filter,
        )?;
        if diff_files_fingerprint(&probe) == current {
            return Ok(None);
        }

        // Re-check after the real fetch. The two reads can be seconds apart, and an
        // edit that lands between them can be reverted, leaving a fetch that matches
        // the screen again. Applying it would reset collapsed folders and expanded
        // gaps with nothing new to show.
        let fetched = Self::fetch_diff_files_for_source(
            vcs,
            root_path,
            fetch_source,
            highlighter,
            path_filter,
        )?;
        if diff_files_fingerprint(&fetched) == current {
            return Ok(None);
        }
        Ok(Some(fetched))
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
        let range =
            Self::initial_commit_range(self.commit_selection_start, self.review_commits.len());
        self.commit_selection_range = range;
        self.commit_list_cursor = range.map(|(start, _)| start).unwrap_or(0);
        self.commit_list_scroll_offset = 0;
        self.visible_commit_count = self.review_commits.len();
        self.has_more_commit = false;
        self.show_commit_selector = self.review_commits.len() > 1;
        self.commit_diff_cache.clear();
        self.saved_inline_selection = None;

        // `initial_commit_selection = oldest` opens scoped to a single commit; narrow
        // the loaded diff to it. Otherwise finalize the full-range diff.
        if Self::is_strict_commit_selection(self.commit_selection_range, self.review_commits.len())
        {
            self.reload_inline_selection()?;
        } else {
            self.insert_commit_message_if_single();
            self.sort_files_by_directory(true);
            self.expand_all_dirs();
            self.rebuild_annotations();
        }
        Ok(())
    }

    /// Sets the diff-watch interval. `0` disables it, which is the default.
    pub fn set_diff_watch_interval_ms(&mut self, interval_ms: u64) {
        match interval_ms {
            0 => self.diff_watch_interval = None,
            ms => {
                let interval = Duration::from_millis(ms);
                self.diff_watch_interval = Some(interval);
                self.next_diff_watch_at = Instant::now() + interval;
            }
        }
    }

    /// Periodically re-reads the local diff so uncommitted changes appear without
    /// pressing `:e`. Returns `true` when a redraw is needed.
    ///
    /// Multi-key sequences (`dd`, `zz`, leader chords) are tracked by locals in
    /// the event loop rather than by `input_mode`, so a tick landing between the
    /// two keys is not deferred and can move the cursor under the second one.
    /// The window is one interval and the option is off by default.
    ///
    /// Drains any landed result before evaluating whether to spawn a new one,
    /// so this single entry point covers both halves of the async round trip
    /// and `main.rs`'s per-tick call site needs no change.
    pub fn poll_diff_watch_changes(&mut self) -> bool {
        let redraw = self.poll_diff_watch_reload_events();

        let now = Instant::now();
        match self.diff_watch_tick(now) {
            DiffWatchTick::Idle | DiffWatchTick::NotDue => {}
            DiffWatchTick::Defer(interval) => self.next_diff_watch_at = now + interval,
            DiffWatchTick::Fetch(interval) => {
                self.next_diff_watch_at = now + interval;
                self.spawn_diff_watch_reload();
            }
        }
        redraw
    }

    /// Decides what this tick should do, without doing any of it. Reads
    /// `self`, writes nothing and starts no thread, so the guard order can be
    /// tested without a worker running a real diff.
    pub(in crate::app) fn diff_watch_tick(&self, now: Instant) -> DiffWatchTick {
        let Some(interval) = self.diff_watch_interval else {
            return DiffWatchTick::Idle;
        };
        // `PullRequest` has its own reload path (`reload_pull_request`).
        // `is_pristine_mode` (`--all-files`) and `VcsType::File` (`--file`)
        // both back onto `FileBackend`, which the worker cannot reopen: it
        // resolves a backend via `detect_vcs`, which only ever discovers a
        // real git/jj/hg repository at the process cwd.
        if matches!(self.diff_source, DiffSource::PullRequest(_))
            || self.is_pristine_mode
            || self.vcs_info.vcs_type == VcsType::File
        {
            return DiffWatchTick::Idle;
        }
        if now < self.next_diff_watch_at {
            return DiffWatchTick::NotDue;
        }

        // Past this point the tick was due, so every answer carries the
        // interval and the caller advances the deadline. A tick arriving while
        // the user is composing waits a full interval rather than firing the
        // moment the editor closes. Mirrors `poll_persisted_session_changes`.
        if self.input_mode != InputMode::Normal {
            return DiffWatchTick::Defer(interval);
        }

        // A fetch is already running, so skip rather than supersede it. A
        // periodic tick carries no new intent to prioritize, unlike a
        // user-triggered PR range toggle. The next tick retries.
        if self.diff_watch_reload.is_some() {
            return DiffWatchTick::Defer(interval);
        }

        DiffWatchTick::Fetch(interval)
    }

    /// Kicks off a background diff-watch fetch. The worker opens its own VCS
    /// backend via `detect_vcs` rather than sharing `self.vcs`, because
    /// `git2::Repository` is `Send` but not `Sync`. `detect_vcs` resolves the
    /// repository from the process's current working directory; tuicr never
    /// changes cwd at runtime, so calling it off the main thread is safe.
    ///
    /// `review_commits` is cloned in here (rather than sent as part of
    /// `request`) because it only feeds `narrowed_fetch_source` on the worker
    /// side. `request.diff_source` must stay the *un*narrowed source so
    /// `diff_watch_result_is_stale`'s comparison against `self.diff_source`
    /// still matches on landing.
    fn spawn_diff_watch_reload(&mut self) {
        let request = DiffWatchReloadRequest {
            diff_source: self.diff_source.clone(),
            commit_selection_range: self.commit_selection_range,
        };
        let current = diff_files_fingerprint(&self.diff_files);
        let review_commits = self.review_commits.clone();
        let path_filter = self.path_filter.clone();
        let vcs_open_options = self.vcs_open_options;
        let highlighter = self.theme.syntax_highlighter_arc();

        let (tx, rx) = std::sync::mpsc::channel();
        self.diff_watch_reload = Some(DiffWatchReload {
            request: request.clone(),
            rx,
        });
        let reporter = DiffWatchReporter::new(tx, request.clone());

        std::thread::spawn(move || {
            let outcome = Self::diff_watch_fetch(
                vcs_open_options,
                &request,
                &review_commits,
                path_filter.as_deref(),
                &highlighter,
                current,
            );
            match outcome {
                Ok(fetched) => reporter.answer(
                    normalize_diff_watch_result(Ok(fetched.diff_files)),
                    fetched.commits,
                    fetched.change_status,
                ),
                Err(e) => reporter.answer(normalize_diff_watch_result(Err(e)), None, None),
            }
        });
    }

    /// Worker-side fetch. Opens its own backend, because `App::vcs` cannot
    /// cross the thread boundary, then runs the same
    /// `changed_diff_files_for_source` gate the main thread uses. `current`
    /// is the fingerprint of what was on screen when the fetch was spawned.
    fn diff_watch_fetch(
        vcs_open_options: VcsOpenOptions,
        request: &DiffWatchReloadRequest,
        review_commits: &[CommitInfo],
        path_filter: Option<&str>,
        highlighter: &SyntaxHighlighter,
        current: u64,
    ) -> Result<DiffWatchFetched> {
        let vcs = detect_vcs(
            vcs_open_options.git_backend_preference,
            vcs_open_options.diff_whitespace_mode,
        )?;
        let root_path = &vcs.info().root_path;
        let fetch_source = Self::narrowed_fetch_source(
            &request.diff_source,
            review_commits,
            request.commit_selection_range,
        );
        let files = Self::changed_diff_files_for_source(
            vcs.as_ref(),
            root_path,
            &fetch_source,
            highlighter,
            path_filter,
            current,
        )?;
        // Runs on the worker, alongside the diff fetch, so the event loop never
        // pays for it.
        //
        // A failure here is dropped rather than reported, unlike the diff half
        // above. The two are independent: the diff is what the user is reading,
        // and failing the whole tick because the commit list could not be read
        // would replace a current diff with a warning. The cost is that a
        // commit fetch failing over and over stays silent. The pane just stops
        // updating, which is how it behaved before this feature.
        let commits = vcs.get_recent_commits(0, VISIBLE_COMMIT_COUNT).ok();
        // Same treatment, and for the same reason: staging a file leaves the
        // combined working-tree diff byte-identical, so the fingerprint above
        // reports nothing and only this can tell the pane that a side gained
        // or lost its row.
        let change_status = Self::get_change_status_with_ignore(
            vcs.as_ref(),
            root_path,
            probe_highlighter(),
            path_filter,
        )
        .ok();
        Ok(DiffWatchFetched {
            diff_files: files,
            commits,
            change_status,
        })
    }

    /// Pumps a pending diff-watch result and applies or discards it. Returns
    /// `true` when a redraw is needed. A no-op when nothing has landed.
    fn poll_diff_watch_reload_events(&mut self) -> bool {
        let Some(in_flight) = self.diff_watch_reload.as_ref() else {
            return false;
        };
        let event = match in_flight.rx.try_recv() {
            Ok(event) => event,
            Err(std::sync::mpsc::TryRecvError::Empty) => return false,
            // `DiffWatchReporter` answers even when the worker panics, so a
            // real failure arrives above as `Err(..)` and warns. Reaching here
            // means the sender went away with nothing sent at all, which the
            // reporter's `Drop` rules out short of process teardown. Clear the
            // in-flight record anyway: leaving it set would block every future
            // tick, killing the watcher for the rest of the session silently.
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.diff_watch_reload = None;
                return false;
            }
        };
        // Clearing here is unconditional and safe: unlike PR range reload,
        // diff-watch never lets a second fetch supersede the first (see the
        // guard in `diff_watch_tick`), so this event answers exactly
        // the request recorded here.
        self.diff_watch_reload = None;

        let DiffWatchReloadEvent::Done {
            request,
            result,
            commits,
            change_status,
        } = event;
        if diff_watch_result_is_stale(
            &request,
            &self.diff_source,
            self.commit_selection_range,
            self.input_mode,
        ) {
            return false;
        }

        let commit_pane_changed = self.apply_fetched_commits(commits, change_status);

        // Every arm yields rather than returning early, so a tick that changed
        // only the commit pane still reports that a redraw is needed.
        let diff_changed = match result {
            Ok(None) => {
                self.last_diff_watch_error = None;
                false
            }
            Ok(Some(diff_files)) => {
                self.last_diff_watch_error = None;
                self.apply_watched_diff(diff_files)
            }
            Err(err) => {
                let text = format!("Diff watch failed: {err}");
                if self.last_diff_watch_error.as_deref() == Some(text.as_str()) {
                    false
                } else {
                    self.last_diff_watch_error = Some(text.clone());
                    self.set_warning(text);
                    true
                }
            }
        };

        commit_pane_changed || diff_changed
    }

    /// Installs a landed watch diff. Returns `true` when it changed the screen.
    ///
    /// The worker compared against the diff as it stood when it was spawned. A
    /// `:e` landing in between can already have applied the same content, so
    /// this compares against what is on screen now. Applying anyway would clear
    /// expanded gaps and print a second "Reloaded" message with nothing new to
    /// show.
    fn apply_watched_diff(&mut self, diff_files: Vec<DiffFile>) -> bool {
        if diff_files_fingerprint(&diff_files) == diff_files_fingerprint(&self.diff_files) {
            return false;
        }
        let (count, invalidated) = self.apply_diff_files(diff_files);
        match invalidated {
            0 => self.set_message(format!("Reloaded {count} files")),
            changed => self.set_message(format!(
                "Reloaded {count} files, {changed} changed since last review"
            )),
        }
        true
    }

    /// Installs a watch tick's freshly fetched commits into the inline pane.
    /// Returns `true` when the pane changed and the screen needs a redraw.
    ///
    /// The selection is re-anchored here rather than at spawn time: the user
    /// can move or narrow it while the worker is running, so only the list as
    /// it stands now can say where its endpoints went.
    fn apply_fetched_commits(
        &mut self,
        fetched: Option<Vec<CommitInfo>>,
        change_status: Option<VcsChangeStatus>,
    ) -> bool {
        let Some(rebuilt) = self.rebuilt_commit_pane(fetched, change_status) else {
            return false;
        };
        match Self::reanchored_commit_selection(
            self.commit_selection_range,
            &self.review_commits,
            &rebuilt,
        ) {
            CommitSelectionAnchor::Moved(range) => {
                self.install_refreshed_commit_pane(rebuilt, range);
                true
            }
            CommitSelectionAnchor::Lost => false,
        }
    }

    /// The pane as it would look with `fetched` merged in, or `None` when
    /// there is nothing to install.
    ///
    /// An empty commit answer means "nothing to say", never "delete every
    /// row". `VcsBackend::get_recent_commits` defaults to `Ok(Vec::new())` for
    /// unsupported rather than an error (`src/vcs/traits.rs`), so without that
    /// filter a future backend that skipped it would wipe the history.
    /// Falling back to the history already on screen rather than returning
    /// early is what lets a staged/unstaged change reach the pane on a tick
    /// where the commit read found nothing or failed.
    ///
    /// `None` still comes back when the rebuilt pane matches what is on screen.
    fn rebuilt_commit_pane(
        &self,
        fetched: Option<Vec<CommitInfo>>,
        change_status: Option<VcsChangeStatus>,
    ) -> Option<Vec<CommitInfo>> {
        let history = fetched
            .filter(|commits| !commits.is_empty())
            .unwrap_or_else(|| {
                self.review_commits
                    .iter()
                    .skip_while(|commit| Self::is_special_commit(commit))
                    .cloned()
                    .collect()
            });
        let rebuilt = Self::commit_pane_rows(&self.review_commits, history, change_status);
        (rebuilt != self.review_commits).then_some(rebuilt)
    }

    /// Installs a refreshed pane and re-derives everything indexed by it.
    ///
    /// A refresh can shrink the pane. `git reset --hard HEAD~2` drops rows the
    /// cursor was sitting on, so the cursor, the scroll offset and the
    /// selection all have to be brought back inside it. Every other path that
    /// replaces this list does the same (`src/app/commits.rs`); doing it in one
    /// place here keeps the four fields from disagreeing.
    ///
    /// `range` is where the selection landed in `rows`, already worked out by
    /// `reanchored_commit_selection`. It arrives as a parameter rather than
    /// being recomputed here, so the answer and the rows it indexes into come
    /// from the same comparison rather than two readings of the list.
    fn install_refreshed_commit_pane(
        &mut self,
        rows: Vec<CommitInfo>,
        range: Option<(usize, usize)>,
    ) {
        // Read before the pane is replaced: the cursor follows the commit it is
        // on, not the index. A new commit lands at the top of a newest-first
        // pane and pushes every row below it down one, so keeping the index
        // would move the highlight onto a different commit every time the user
        // commits, which is what they are doing while this feature runs.
        let anchored = self
            .review_commits
            .get(self.commit_list_cursor)
            .and_then(|commit| {
                let id = commit.id.clone();
                rows.iter().position(|row| row.id == id)
            });

        self.review_commits = rows;
        self.commit_list = self.review_commits.clone();
        self.visible_commit_count = self.commit_list.len();
        // Keyed by row index, so every key now names a different commit. Every
        // other path that replaces `review_commits` clears it for the same
        // reason (`src/app/commits.rs`, `src/app/init.rs`). The next narrowing
        // refetches, which is the price of not showing one commit's diff under
        // another commit's name.
        self.commit_diff_cache.clear();

        match self.review_commits.len() {
            0 => {
                self.commit_list_cursor = 0;
                self.commit_list_scroll_offset = 0;
                self.commit_selection_range = None;
            }
            len => {
                let last = len - 1;
                // Falls back to clamping when the anchored commit is gone, which
                // is what `git reset --hard` does to the row under the cursor.
                self.commit_list_cursor = anchored.unwrap_or(self.commit_list_cursor).min(last);
                self.commit_list_scroll_offset = self.commit_list_scroll_offset.min(last);
                self.commit_selection_range = range;
            }
        }
    }

    /// The commit pane after a refresh: the synthetic rows the tree's current
    /// state calls for, followed by the freshly fetched commits.
    ///
    /// The pane can carry up to two synthetic rows above the real commits, one
    /// standing for staged changes and one for unstaged (`staged_commit_entry`,
    /// `unstaged_commit_entry`). They are always a prefix, which is what
    /// `special_commit_count` relies on. `get_recent_commits` does not know
    /// about them, so installing its answer verbatim would delete both.
    ///
    /// `status` decides which of the two the pane should carry, because
    /// `git add` leaves the combined working-tree diff byte-identical: the
    /// diff fingerprint reports nothing, and without this the newly staged
    /// side would gain no row until the review target was reopened. A backend
    /// that cannot report status sends `None`, and the rows are left alone.
    ///
    /// Rows already on screen keep their order, and a row that appears goes
    /// after them. The two startup paths disagree about that order: `App::new`
    /// builds staged first (`src/app/init.rs`) and `enter_target_selector`
    /// builds unstaged first (`src/app/commits.rs`). Re-deriving the order
    /// here would reorder the pane under whichever half of them the user
    /// opened from, so a tick only ever adds and removes.
    fn commit_pane_rows(
        current: &[CommitInfo],
        fetched: Vec<CommitInfo>,
        status: Option<VcsChangeStatus>,
    ) -> Vec<CommitInfo> {
        let existing = current
            .iter()
            .take_while(|commit| Self::is_special_commit(commit));
        let mut rows: Vec<CommitInfo> = match status {
            None => existing.cloned().collect(),
            Some(status) => {
                let mut kept: Vec<CommitInfo> = existing
                    .filter(|commit| Self::special_row_still_earned(commit, status))
                    .cloned()
                    .collect();
                if status.staged && !kept.iter().any(Self::is_staged_commit) {
                    kept.push(Self::staged_commit_entry());
                }
                if status.unstaged && !kept.iter().any(Self::is_unstaged_commit) {
                    kept.push(Self::unstaged_commit_entry());
                }
                kept
            }
        };
        rows.extend(fetched);
        rows
    }

    /// Whether the tree still holds what a synthetic row stands for.
    ///
    /// A row this does not recognise is kept. `is_special_commit` decides what
    /// counts as one, and reading "not staged" as "therefore unstaged" would
    /// silently drop a third kind the day one is added.
    fn special_row_still_earned(commit: &CommitInfo, status: VcsChangeStatus) -> bool {
        match commit {
            row if Self::is_staged_commit(row) => status.staged,
            row if Self::is_unstaged_commit(row) => status.unstaged,
            _ => true,
        }
    }
}

/// The worker's end of the diff-watch channel, which can only be dropped
/// having sent exactly one answer.
///
/// Without it the worker has four outcomes but only three ways to say so:
/// nothing changed, something changed, and the fetch failed all arrive as a
/// value, while a worker that dies mid-fetch arrives as no value at all. The
/// main thread then has to read that fourth case off the channel itself,
/// which is how it ended up handled separately from the other three, and
/// silently.
///
/// The request sits in an `Option` so the type enforces exactly one answer.
/// `take` is both the send and the record that it happened, so neither a
/// double send nor a silent drop can be written.
pub(in crate::app) struct DiffWatchReporter {
    tx: std::sync::mpsc::Sender<DiffWatchReloadEvent>,
    request: Option<DiffWatchReloadRequest>,
}

impl DiffWatchReporter {
    pub(in crate::app) fn new(
        tx: std::sync::mpsc::Sender<DiffWatchReloadEvent>,
        request: DiffWatchReloadRequest,
    ) -> Self {
        Self {
            tx,
            request: Some(request),
        }
    }

    /// Reports the fetch outcome. Consumes the reporter, so nothing can send
    /// after it.
    fn answer(
        mut self,
        result: std::result::Result<Option<Vec<DiffFile>>, String>,
        commits: Option<Vec<CommitInfo>>,
        change_status: Option<VcsChangeStatus>,
    ) {
        self.send(result, commits, change_status);
    }

    fn send(
        &mut self,
        result: std::result::Result<Option<Vec<DiffFile>>, String>,
        commits: Option<Vec<CommitInfo>>,
        change_status: Option<VcsChangeStatus>,
    ) {
        let Some(request) = self.request.take() else {
            return;
        };
        // The receiver is gone once the app shuts down, which is normal.
        let _ = self.tx.send(DiffWatchReloadEvent::Done {
            request,
            result,
            commits,
            change_status,
        });
    }
}

impl Drop for DiffWatchReporter {
    /// A worker that panics unwinds through here, so a crash reaches the main
    /// thread as an ordinary failed fetch and takes the existing warn-once
    /// path. The panic still propagates afterwards; this only makes sure the
    /// crash is reported rather than inferred.
    fn drop(&mut self) {
        self.send(
            Err("diff watch worker stopped without finishing".to_string()),
            None,
            None,
        );
    }
}

/// What one diff-watch worker run produced. The two halves are independent:
/// a tick can find new commits while the diff is unchanged, or the reverse.
struct DiffWatchFetched {
    diff_files: Option<Vec<DiffFile>>,
    commits: Option<Vec<CommitInfo>>,
    change_status: Option<VcsChangeStatus>,
}

/// Where a commit selection lands in a rebuilt commit pane, if it lands at
/// all. Carries the answer rather than a yes/no, so a caller cannot install a
/// rebuilt pane having forgotten to move the range that indexes into it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum CommitSelectionAnchor {
    /// Install the rebuilt pane and set the selection to this.
    Moved(Option<(usize, usize)>),
    /// An endpoint of the selection is missing from the rebuilt pane, because
    /// `git reset --hard` dropped it or it fell out of the fetch window. There
    /// is no honest place to put the range, so the pane stays as it is.
    Lost,
}

/// What one diff-watch tick should do. Only a tick that was actually due
/// carries an interval, so the caller cannot advance the deadline for a tick
/// that never came up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum DiffWatchTick {
    /// No interval set, or this diff source is never watched.
    Idle,
    /// Armed, but the interval has not elapsed yet.
    NotDue,
    /// Due, but something blocks a fetch right now. The deadline still moves,
    /// so the next attempt waits a full interval.
    Defer(Duration),
    /// Due and clear. Move the deadline and start a worker.
    Fetch(Duration),
}

/// True when a landed diff-watch result should be discarded rather than
/// applied to `current_*`: the diff source changed, the commit selection
/// changed, or the mode has left `Normal` since the fetch was spawned. See
/// `apply_diff_files`'s doc comment for why any of these invalidate the
/// cursor-capture it depends on.
pub(in crate::app) fn diff_watch_result_is_stale(
    request: &DiffWatchReloadRequest,
    current_diff_source: &DiffSource,
    current_commit_selection_range: Option<(usize, usize)>,
    current_input_mode: InputMode,
) -> bool {
    request.diff_source != *current_diff_source
        || request.commit_selection_range != current_commit_selection_range
        || current_input_mode != InputMode::Normal
}

/// Maps the worker's fetch outcome onto the channel's `String`-error wire
/// type (errors cross threads as text, not `TuicrError`, matching
/// `PrRangeReloadEvent`). `NoChanges` is folded into `Ok(None)`, because it
/// means the same thing to a caller as "fetched, nothing changed". Both are a
/// silent no-op, never a warning.
pub(in crate::app) fn normalize_diff_watch_result(
    result: Result<Option<Vec<DiffFile>>>,
) -> std::result::Result<Option<Vec<DiffFile>>, String> {
    match result {
        Ok(files) => Ok(files),
        Err(TuicrError::NoChanges) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// One shared plain highlighter. Building a `SyntaxSet` costs something even when
/// it is empty, and the watcher asks for this on every tick.
fn probe_highlighter() -> &'static SyntaxHighlighter {
    static PROBE: std::sync::OnceLock<SyntaxHighlighter> = std::sync::OnceLock::new();
    PROBE.get_or_init(SyntaxHighlighter::plain)
}

/// Fingerprint of one file. `content_hash` alone is insufficient: binary and
/// too-large files all carry the same empty-hunks hash, so a file flipping to
/// too-large would otherwise look unchanged.
///
/// Everything written after the path is fixed width, which is what keeps the
/// variable-length path unambiguous. Adding another variable-length field here
/// would need a delimiter.
fn file_fingerprint(file: &DiffFile) -> u64 {
    let mut hasher = crate::hash::Fnv1aHasher::new();
    hasher.write(file.display_path().to_string_lossy().as_bytes());
    hasher.write(&[
        file.status.as_char() as u8,
        file.is_binary as u8,
        file.is_too_large as u8,
    ]);
    hasher.write(&file.content_hash.to_le_bytes());
    hasher.finish()
}

/// Order-insensitive fingerprint of a whole diff. Sorting the per-file hashes
/// compares two lists as sets, which is required because the stored list is
/// sorted by directory and a freshly fetched one is not (see
/// `sort_files_by_directory`).
fn diff_files_fingerprint(files: &[DiffFile]) -> u64 {
    let mut per_file: Vec<u64> = files.iter().map(file_fingerprint).collect();
    per_file.sort_unstable();
    let mut hasher = crate::hash::Fnv1aHasher::new();
    for hash in per_file {
        hasher.write(&hash.to_le_bytes());
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_file(
        path: &str,
        status: FileStatus,
        content_hash: u64,
        is_binary: bool,
        is_too_large: bool,
    ) -> DiffFile {
        DiffFile {
            old_path: None,
            new_path: Some(PathBuf::from(path)),
            status,
            hunks: vec![],
            is_binary,
            is_too_large,
            is_commit_message: false,
            content_hash,
        }
    }

    fn file(path: &str) -> DiffFile {
        make_file(path, FileStatus::Modified, 0, false, false)
    }

    #[test]
    fn should_match_for_identical_input() {
        let files = vec![file("a.rs"), file("b.rs")];
        let other = files.clone();
        assert_eq!(
            diff_files_fingerprint(&files),
            diff_files_fingerprint(&other)
        );
    }

    /// `apply_diff_files` stores files sorted by directory
    /// (`sort_files_by_directory`), while a fresh fetch returns raw backend
    /// order. Order-sensitivity here would report "changed" on every tick.
    #[test]
    fn should_match_when_same_files_arrive_in_a_different_order() {
        let forward = vec![file("a.rs"), file("b.rs"), file("c.rs")];
        let reversed = vec![file("c.rs"), file("b.rs"), file("a.rs")];
        assert_eq!(
            diff_files_fingerprint(&forward),
            diff_files_fingerprint(&reversed)
        );
    }

    #[test]
    fn should_differ_when_content_hash_changes() {
        let before = vec![make_file("a.rs", FileStatus::Modified, 1, false, false)];
        let after = vec![make_file("a.rs", FileStatus::Modified, 2, false, false)];
        assert_ne!(
            diff_files_fingerprint(&before),
            diff_files_fingerprint(&after)
        );
    }

    #[test]
    fn should_differ_when_status_changes() {
        let before = vec![make_file("a.rs", FileStatus::Modified, 1, false, false)];
        let after = vec![make_file("a.rs", FileStatus::Added, 1, false, false)];
        assert_ne!(
            diff_files_fingerprint(&before),
            diff_files_fingerprint(&after)
        );
    }

    #[test]
    fn should_differ_when_path_changes() {
        let before = vec![file("a.rs")];
        let after = vec![file("z.rs")];
        assert_ne!(
            diff_files_fingerprint(&before),
            diff_files_fingerprint(&after)
        );
    }

    #[test]
    fn should_differ_when_a_file_is_added() {
        let before = vec![file("a.rs")];
        let after = vec![file("a.rs"), file("b.rs")];
        assert_ne!(
            diff_files_fingerprint(&before),
            diff_files_fingerprint(&after)
        );
    }

    #[test]
    fn should_differ_when_a_file_is_removed() {
        let before = vec![file("a.rs"), file("b.rs")];
        let after = vec![file("a.rs")];
        assert_ne!(
            diff_files_fingerprint(&before),
            diff_files_fingerprint(&after)
        );
    }

    #[test]
    fn should_differ_when_binary_flag_flips_despite_equal_content_hash() {
        let before = vec![make_file("a.rs", FileStatus::Modified, 1, false, false)];
        let after = vec![make_file("a.rs", FileStatus::Modified, 1, true, false)];
        assert_ne!(
            diff_files_fingerprint(&before),
            diff_files_fingerprint(&after)
        );
    }

    #[test]
    fn should_differ_when_too_large_flag_flips_despite_equal_content_hash() {
        let before = vec![make_file("a.rs", FileStatus::Modified, 1, false, false)];
        let after = vec![make_file("a.rs", FileStatus::Modified, 1, false, true)];
        assert_ne!(
            diff_files_fingerprint(&before),
            diff_files_fingerprint(&after)
        );
    }

    /// Two diffs holding the same characters split across paths differently
    /// must not collide, so the combine step cannot lose path boundaries.
    #[test]
    fn should_differ_when_paths_are_split_differently_across_files() {
        let a = vec![file("ab"), file("c")];
        let b = vec![file("a"), file("bc")];
        assert_ne!(diff_files_fingerprint(&a), diff_files_fingerprint(&b));
    }
}
