use super::*;

impl App {
    /// Slug for the currently active session, derived from the session's
    /// embedded fields. Returns `None` if derivation fails (e.g., a local
    /// session pointing at a non-existent path). The slug is cheap to derive
    /// from PR sessions (no I/O) and a few stat calls for local sessions.
    pub fn session_slug(&self) -> Option<String> {
        crate::persistence::storage::slug_for_session(&self.session)
            .ok()
            .map(|s| s.to_string())
    }

    pub fn set_review_watch_interval_ms(&mut self, interval_ms: u64) {
        if interval_ms == 0 {
            self.review_watch_interval = None;
        } else {
            let interval = Duration::from_millis(interval_ms);
            self.review_watch_interval = Some(interval);
            self.next_review_watch_at = Instant::now() + interval;
        }
    }

    pub(in crate::app) fn reset_persisted_session_tracking(&mut self) {
        self.session_path = crate::persistence::storage::session_path(&self.session).ok();
        self.session_file_state = self
            .session_path
            .as_deref()
            .filter(|path| path.exists())
            .and_then(|path| SessionFileState::from_path(path).ok());
        self.persisted_session_snapshot = self.session.clone();
        if let Err(e) = self.ensure_ephemeral_session_file() {
            self.set_warning(format!("Failed to initialize review session file: {e}"));
        }
    }

    fn mark_session_saved(&mut self, path: PathBuf, saved: ReviewSession) {
        self.session = saved.clone();
        self.persisted_session_snapshot = saved;
        self.session_path = Some(path.clone());
        self.session_file_state = SessionFileState::from_path(&path).ok();
        self.dirty = false;
    }

    pub fn ensure_ephemeral_session_file(&mut self) -> Result<Option<PathBuf>> {
        let path = match self.session_path.clone() {
            Some(path) => path,
            None => {
                let path = crate::persistence::storage::session_path(&self.session)?;
                self.session_path = Some(path.clone());
                path
            }
        };

        if path.exists() {
            if self.ephemeral_session_paths.contains(&path) {
                let saved_path = self.save_current_session_merging_external()?;
                return Ok(Some(saved_path));
            }
            self.session_file_state = SessionFileState::from_path(&path).ok();
            self.mark_current_session_active_at(&path);
            return Ok(None);
        }

        let saved_path = self.save_current_session_merging_external()?;
        self.ephemeral_session_paths.insert(saved_path.clone());
        Ok(Some(saved_path))
    }

    pub fn cleanup_empty_ephemeral_sessions(&mut self) -> Result<usize> {
        let mut deleted = 0;
        for path in self.ephemeral_session_paths.clone() {
            if crate::persistence::storage::delete_session_if_empty(&path)? {
                deleted += 1;
                if self.session_path.as_ref() == Some(&path) {
                    self.session_file_state = None;
                }
            }
            self.ephemeral_session_paths.remove(&path);
        }
        Ok(deleted)
    }

    pub fn clear_active_session_marker(&mut self) -> Result<()> {
        crate::persistence::storage::clear_active_session_for_pid()
    }

    /// Discard this session's persisted state and quit.
    ///
    /// Used when the only unsaved change is reviewed-file markers (no
    /// comments): rather than forcing `:q!`, we drop the persisted session so
    /// reopening starts clean, then quit. Reviewed markers are persisted
    /// eagerly, so the on-disk file is removed too.
    pub fn discard_session_and_quit(&mut self) {
        let path = self
            .session_path
            .clone()
            .or_else(|| crate::persistence::storage::session_path(&self.session).ok());
        if let Some(path) = path {
            let _ = crate::persistence::storage::delete_session(&path);
            self.ephemeral_session_paths.remove(&path);
            if self.session_path.as_ref() == Some(&path) {
                self.session_file_state = None;
            }
        }
        self.dirty = false;
        self.should_quit = true;
    }

    pub fn save_current_session_merging_external(&mut self) -> Result<PathBuf> {
        let identity = self.session.clone();
        let current = self.session.clone();
        let base = self.persisted_session_snapshot.clone();
        let (path, saved, _changed) =
            crate::persistence::storage::save_session_by_identity(&identity, |persisted| {
                let mut merged = current.clone();
                if let Some(latest) = persisted.as_ref() {
                    Self::merge_external_session_changes(&mut merged, &base, latest);
                }
                merged.updated_at = Utc::now();
                Ok((merged, ()))
            })?;
        self.mark_session_saved(path.clone(), saved);
        self.mark_current_session_active_at(&path);
        self.rebuild_annotations();
        Ok(path)
    }

    fn mark_current_session_active_at(&mut self, path: &Path) {
        if let Err(e) = crate::persistence::storage::mark_session_active(&self.session, path) {
            self.set_warning(format!("Failed to mark active review session: {e}"));
        }
    }

    /// Returns `true` if visible state changed (external comments merged or a
    /// warning was raised) so the main loop can schedule a redraw without an
    /// input event.
    pub fn poll_persisted_session_changes(&mut self) -> bool {
        let Some(interval) = self.review_watch_interval else {
            return false;
        };
        let now = Instant::now();
        if now < self.next_review_watch_at {
            return false;
        }
        self.next_review_watch_at = now + interval;

        // Do not mutate the session while the user is composing or editing a
        // comment. The next poll after the editor closes will merge changes.
        if self.input_mode == InputMode::Comment {
            return false;
        }

        match self.reload_persisted_session_if_changed(false) {
            Ok(0) => false,
            Ok(_) => true,
            Err(err) => {
                self.set_warning(format!("Review reload failed: {err}"));
                true
            }
        }
    }

    /// True while any forge background fetch (PR list/open/reload/threads/
    /// submit) is in flight. Used by the main loop to keep redrawing so
    /// spinners animate and results land without waiting for input.
    pub fn has_pending_pr_work(&self) -> bool {
        self.pr_load_rx.is_some()
            || self.pr_open_rx.is_some()
            || self.pr_reload_rx.is_some()
            || self.pr_range_reload_rx.is_some()
            || self.pr_threads_rx.is_some()
            || self.pr_submit_rx.is_some()
    }

    pub fn reload_persisted_session_if_changed(&mut self, force: bool) -> Result<usize> {
        let path = match self.session_path.clone() {
            Some(path) => path,
            None => match crate::persistence::storage::session_path(&self.session) {
                Ok(path) => {
                    self.session_path = Some(path.clone());
                    path
                }
                Err(_) => return Ok(0),
            },
        };

        if !path.exists() {
            self.session_file_state = None;
            return Ok(0);
        }

        let state = SessionFileState::from_path(&path)?;
        if !force && self.session_file_state == Some(state) {
            return Ok(0);
        }

        let latest = crate::persistence::storage::load_session(&path)?;
        let before_count = Self::comment_count(&self.session);
        let changed = Self::merge_external_session_changes(
            &mut self.session,
            &self.persisted_session_snapshot,
            &latest,
        );
        self.persisted_session_snapshot = latest;
        self.session_file_state = Some(state);
        if changed > 0 {
            self.rebuild_annotations();
        }
        let after_count = Self::comment_count(&self.session);
        Ok(after_count.saturating_sub(before_count))
    }

    pub(in crate::app) fn merge_external_session_changes(
        current: &mut ReviewSession,
        base: &ReviewSession,
        latest: &ReviewSession,
    ) -> usize {
        let mut changed = 0;

        for (path, latest_review) in &latest.files {
            if !current.files.contains_key(path) {
                current.files.insert(path.clone(), latest_review.clone());
                changed += latest_review.comment_count();
                continue;
            }

            let base_reviewed = base.files.get(path).map(|review| review.reviewed);
            if let Some(current_review) = current.files.get_mut(path)
                && Some(current_review.reviewed) == base_reviewed
                && current_review.reviewed != latest_review.reviewed
            {
                current_review.reviewed = latest_review.reviewed;
                changed += 1;
            }
        }

        let base_comments = Self::collect_stored_comments(base);
        let current_comments = Self::collect_stored_comments(current);
        let latest_comments = Self::collect_stored_comments(latest);

        for (id, latest_comment) in &latest_comments {
            match base_comments.get(id) {
                None => {
                    if !current_comments.contains_key(id) {
                        Self::upsert_stored_comment(current, latest_comment.clone());
                        changed += 1;
                    }
                }
                Some(base_comment) if latest_comment != base_comment => {
                    match current_comments.get(id) {
                        Some(current_comment) if current_comment == base_comment => {
                            Self::upsert_stored_comment(current, latest_comment.clone());
                            changed += 1;
                        }
                        None => {
                            // Local deletion wins over an external edit.
                        }
                        Some(_) => {
                            // Local edit wins over an external edit of the same comment.
                        }
                    }
                }
                Some(_) => {}
            }
        }

        for (id, base_comment) in &base_comments {
            if !latest_comments.contains_key(id)
                && current_comments
                    .get(id)
                    .is_some_and(|current_comment| current_comment == base_comment)
                && Self::remove_stored_comment(current, id)
            {
                changed += 1;
            }
        }

        changed
    }

    fn comment_count(session: &ReviewSession) -> usize {
        session.review_comments.len()
            + session
                .files
                .values()
                .map(|review| review.comment_count())
                .sum::<usize>()
    }

    fn collect_stored_comments(session: &ReviewSession) -> HashMap<String, StoredComment> {
        let mut comments = HashMap::new();
        for comment in &session.review_comments {
            comments.insert(
                comment.id.clone(),
                StoredComment {
                    location: StoredCommentLocation::Review,
                    comment: comment.clone(),
                },
            );
        }

        for (path, review) in &session.files {
            for comment in &review.file_comments {
                comments.insert(
                    comment.id.clone(),
                    StoredComment {
                        location: StoredCommentLocation::File { path: path.clone() },
                        comment: comment.clone(),
                    },
                );
            }

            for (line, line_comments) in &review.line_comments {
                for comment in line_comments {
                    comments.insert(
                        comment.id.clone(),
                        StoredComment {
                            location: StoredCommentLocation::Line {
                                path: path.clone(),
                                line: *line,
                            },
                            comment: comment.clone(),
                        },
                    );
                }
            }
        }

        comments
    }

    fn upsert_stored_comment(session: &mut ReviewSession, stored: StoredComment) {
        Self::remove_stored_comment(session, &stored.comment.id);
        match stored.location {
            StoredCommentLocation::Review => {
                session.review_comments.push(stored.comment);
            }
            StoredCommentLocation::File { path } => {
                if let Some(review) = session.files.get_mut(&path) {
                    review.file_comments.push(stored.comment);
                }
            }
            StoredCommentLocation::Line { path, line } => {
                if let Some(review) = session.files.get_mut(&path) {
                    review
                        .line_comments
                        .entry(line)
                        .or_default()
                        .push(stored.comment);
                }
            }
        }
    }

    fn remove_stored_comment(session: &mut ReviewSession, id: &str) -> bool {
        if let Some(index) = session
            .review_comments
            .iter()
            .position(|comment| comment.id == id)
        {
            session.review_comments.remove(index);
            return true;
        }

        for review in session.files.values_mut() {
            if let Some(index) = review
                .file_comments
                .iter()
                .position(|comment| comment.id == id)
            {
                review.file_comments.remove(index);
                return true;
            }

            let mut emptied_line = None;
            for (line, comments) in &mut review.line_comments {
                if let Some(index) = comments.iter().position(|comment| comment.id == id) {
                    comments.remove(index);
                    if comments.is_empty() {
                        emptied_line = Some(*line);
                    }
                    break;
                }
            }
            if let Some(line) = emptied_line {
                review.line_comments.remove(&line);
                return true;
            }
        }

        false
    }

    /// Load or create a session for a commit range (used by revisions and commit selection).
    pub(in crate::app) fn load_or_create_commit_range_session(
        vcs_info: &VcsInfo,
        commit_ids: &[String],
    ) -> ReviewSession {
        let newest_commit_id = commit_ids.last().unwrap().clone();
        let loaded = load_latest_session_for_context(
            &vcs_info.root_path,
            vcs_info.branch_name.as_deref(),
            &newest_commit_id,
            SessionDiffSource::CommitRange,
            Some(commit_ids),
        )
        .ok()
        .and_then(|found| found.map(|(_path, session)| session));

        let mut session = loaded.unwrap_or_else(|| {
            let mut s = ReviewSession::new(
                vcs_info.root_path.clone(),
                newest_commit_id,
                vcs_info.branch_name.clone(),
                SessionDiffSource::CommitRange,
            );
            s.commit_range = Some(commit_ids.to_vec());
            s
        });

        if session.commit_range.is_none() {
            session.commit_range = Some(commit_ids.to_vec());
            session.updated_at = chrono::Utc::now();
        }
        session
    }

    pub(in crate::app) fn load_or_create_staged_unstaged_and_commits_session(
        vcs_info: &VcsInfo,
        commit_ids: &[String],
    ) -> ReviewSession {
        let newest_commit_id = commit_ids.last().unwrap().clone();
        let loaded = load_latest_session_for_context(
            &vcs_info.root_path,
            vcs_info.branch_name.as_deref(),
            &newest_commit_id,
            SessionDiffSource::StagedUnstagedAndCommits,
            Some(commit_ids),
        )
        .ok()
        .and_then(|found| found.map(|(_path, session)| session));

        let mut session = loaded.unwrap_or_else(|| {
            let mut s = ReviewSession::new(
                vcs_info.root_path.clone(),
                newest_commit_id,
                vcs_info.branch_name.clone(),
                SessionDiffSource::StagedUnstagedAndCommits,
            );
            s.commit_range = Some(commit_ids.to_vec());
            s
        });

        if session.commit_range.is_none() {
            session.commit_range = Some(commit_ids.to_vec());
            session.updated_at = chrono::Utc::now();
        }
        session
    }

    pub(in crate::app) fn load_or_create_session(
        vcs_info: &VcsInfo,
        diff_source: SessionDiffSource,
    ) -> ReviewSession {
        let new_session = || {
            ReviewSession::new(
                vcs_info.root_path.clone(),
                vcs_info.head_commit.clone(),
                vcs_info.branch_name.clone(),
                diff_source,
            )
        };

        let Ok(found) = load_latest_session_for_context(
            &vcs_info.root_path,
            vcs_info.branch_name.as_deref(),
            &vcs_info.head_commit,
            diff_source,
            None,
        ) else {
            return new_session();
        };

        let Some((_path, mut session)) = found else {
            return new_session();
        };

        let mut updated = false;
        if session.branch_name.is_none() && vcs_info.branch_name.is_some() {
            session.branch_name = vcs_info.branch_name.clone();
            updated = true;
        }

        if vcs_info.branch_name.is_some() && session.base_commit != vcs_info.head_commit {
            session.base_commit = vcs_info.head_commit.clone();
            updated = true;
        }

        if updated {
            session.updated_at = chrono::Utc::now();
        }

        session
    }

    /// Materialize a PR session from an already-opened PR. Reattaches the
    /// most recent persisted session for the same head SHA when present so
    /// reviewed markers and local comments survive a reopen.
    fn load_pr_session_for_opened(
        opened: &crate::forge::pr_open::OpenedPullRequest,
    ) -> Result<Option<ReviewSession>> {
        let key = opened.key.clone();
        let mut persisted = match crate::persistence::load_pr_session(&key)? {
            Some((_path, persisted)) if persisted.pr_session_key.as_ref() == Some(&key) => {
                persisted
            }
            _ => {
                let path = crate::persistence::storage::session_path(&opened.session)?;
                if !path.exists() {
                    return Ok(None);
                }
                let persisted = crate::persistence::storage::load_session(&path).map_err(|e| {
                    TuicrError::CorruptedSession(format!(
                        "failed to load PR session {}: {e}",
                        path.display()
                    ))
                })?;
                if persisted.pr_session_key.as_ref() != Some(&key) {
                    return Ok(None);
                }
                persisted
            }
        };

        // Re-register diff files against the loaded session so any new files
        // in the PR appear with content_hash tracking, and any deleted files
        // simply stop appearing in the file list.
        // Strict subset sessions are reloaded through a full PR diff first, so
        // pruning here would discard hunk keys hidden by the active selector.
        let preserve_hunks = Self::is_strict_commit_selection(
            persisted.commit_selection_range,
            opened.commits.len(),
        );
        Self::register_diff_files(&mut persisted, &opened.diff_files, preserve_hunks);
        Ok(Some(ReviewSession {
            pr_session_key: Some(key),
            diff_source: SessionDiffSource::PullRequest,
            updated_at: chrono::Utc::now(),
            ..persisted
        }))
    }

    pub(in crate::app) fn opened_pr_with_persisted_session(
        opened: crate::forge::pr_open::OpenedPullRequest,
    ) -> Result<crate::forge::pr_open::OpenedPullRequest> {
        match Self::load_pr_session_for_opened(&opened)? {
            Some(session) => Ok(crate::forge::pr_open::OpenedPullRequest { session, ..opened }),
            None => Ok(opened),
        }
    }

    pub(in crate::app) fn opened_pr_with_new_head_session(
        &mut self,
        opened: crate::forge::pr_open::OpenedPullRequest,
    ) -> Result<crate::forge::pr_open::OpenedPullRequest> {
        self.save_current_session_merging_external()?;
        let previous_session = self.session.clone();
        let session = match Self::load_pr_session_for_opened(&opened)? {
            Some(session) => session,
            None => Self::reviewed_state_carried_forward(
                &previous_session,
                opened.session.clone(),
                &opened.diff_files,
            ),
        };
        Ok(crate::forge::pr_open::OpenedPullRequest { session, ..opened })
    }

    pub(in crate::app) fn reviewed_state_carried_forward(
        previous: &ReviewSession,
        next: ReviewSession,
        diff_files: &[DiffFile],
    ) -> ReviewSession {
        let file_by_path: HashMap<_, _> = diff_files
            .iter()
            .map(|file| (file.display_path().clone(), file))
            .collect();
        let files = next
            .files
            .into_iter()
            .map(|(path, review)| {
                Self::file_review_carried_forward(path, review, previous, &file_by_path)
            })
            .collect();
        let review_comments = previous
            .review_comments
            .iter()
            .filter(|comment| !comment.is_locked())
            .cloned()
            .collect();

        ReviewSession {
            files,
            review_comments,
            ..next
        }
    }

    fn file_review_carried_forward(
        path: PathBuf,
        review: FileReview,
        previous: &ReviewSession,
        file_by_path: &HashMap<PathBuf, &DiffFile>,
    ) -> (PathBuf, FileReview) {
        let Some(file) = file_by_path.get(&path) else {
            return (path, review);
        };
        let Some(previous_review) = previous.files.get(&path) else {
            return (path, review);
        };

        let unchanged_file = previous_review.content_hash == Some(file.content_hash);
        let valid_hunks: HashSet<_> = file.hunk_review_keys().into_iter().collect();
        let reviewed_hunks = previous_review
            .reviewed_hunks
            .iter()
            .filter(|key| valid_hunks.contains(*key))
            .cloned()
            .collect();
        let (file_comments, line_comments) = if unchanged_file {
            (
                previous_review
                    .file_comments
                    .iter()
                    .filter(|comment| !comment.is_locked())
                    .cloned()
                    .collect(),
                Self::line_draft_comments_carried_forward(previous_review),
            )
        } else {
            (review.file_comments, review.line_comments)
        };

        (
            path,
            FileReview {
                reviewed: unchanged_file && previous_review.reviewed,
                reviewed_hunks,
                file_comments,
                line_comments,
                ..review
            },
        )
    }

    fn line_draft_comments_carried_forward(
        previous_review: &FileReview,
    ) -> HashMap<u32, Vec<Comment>> {
        previous_review
            .line_comments
            .iter()
            .filter_map(|(line, comments)| {
                let drafts: Vec<_> = comments
                    .iter()
                    .filter(|comment| !comment.is_locked())
                    .cloned()
                    .collect();
                (!drafts.is_empty()).then_some((*line, drafts))
            })
            .collect()
    }
}
