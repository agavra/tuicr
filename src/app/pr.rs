use super::*;

impl App {
    /// Re-enter PR mode after we've already opened a PR via the selector.
    /// Used by the selector → PR open path and by `:reload` in PR mode.
    pub fn enter_pr_diff_mode(
        &mut self,
        backend: Box<dyn ForgeBackend>,
        opened: crate::forge::pr_open::OpenedPullRequest,
    ) -> Result<()> {
        let crate::forge::pr_open::OpenedPullRequest {
            details,
            diff_files,
            session,
            key,
            commits,
            review_metadata,
        } = opened;

        // Save the current session before transitioning so local-mode work
        // isn't lost.
        self.save_current_session_merging_external()?;

        let pr_source = PullRequestDiffSource::from_details(&details);
        let read_only_reason = pr_source.read_only_reason();
        let virtual_root = session.repo_path.clone();

        self.vcs_info = VcsInfo {
            root_path: virtual_root.clone(),
            head_commit: details.head_sha.clone(),
            branch_name: Some(details.head_ref_name.clone()),
            vcs_type: VcsType::File,
        };
        self.vcs = Box::new(PrNoopVcs::new(self.vcs_info.clone()));
        self.session = session;
        self.diff_files = diff_files;
        self.reset_persisted_session_tracking();
        self.diff_source = DiffSource::PullRequest(Box::new(pr_source));
        self.forge_backend = Some(backend);
        self.forge_repository = Some(key.repository.clone());
        // Reset remote-comment state on every PR mode entry; the new PR's
        // threads will be fetched separately by spawn_pr_threads_fetch.
        self.forge_review_threads = Vec::new();
        self.forge_review_summaries = Vec::new();
        self.forge_review_threads_loading = false;
        self.pr_threads_rx = None;
        // Latest known remote head — equal to the session head at open time;
        // refreshed by future `gh pr view` calls in PR 6.
        self.current_pr_head = Some(details.head_sha.clone());
        self.input_mode = InputMode::Normal;
        self.focused_panel = FocusedPanel::Diff;
        self.clear_expanded_gaps();
        self.commit_list.clear();
        self.commit_selection_range = None;
        self.review_commits.clear();
        self.pr_commits.clear();
        self.pr_last_reviewed_commit_index = None;
        self.show_commit_selector = false;
        self.range_diff_files = None;
        self.saved_inline_selection = None;
        self.diff_state = DiffState::default();

        // PR mode populates the inline selector with the PR's commits when
        // there are at least two. Single-commit PRs hide the selector to
        // match the local-mode UX. We mirror `commit_list` and
        // `review_commits` into shared App state so the existing
        // inline_commit_selector renderer Just Works.
        let since_last_review_message = self.apply_pr_commit_selector(commits, review_metadata);

        // Ensure session has all files registered after the swap. A strict
        // selector range is a filtered view, not a new review scope.
        let preserve_hunks =
            Self::is_strict_commit_selection(self.commit_selection_range, self.pr_commits.len());
        Self::register_diff_files(&mut self.session, &self.diff_files, preserve_hunks);

        self.sort_files_by_directory(true);
        self.expand_all_dirs();
        self.rebuild_annotations();

        if let Some(reason) = read_only_reason {
            self.set_warning(format!("This PR is {reason} — review is read-only"));
        } else if let Some(message) = since_last_review_message {
            self.set_message(message);
        }

        // If the restored selection is a strict subset, fire an initial
        // range re-fetch so the diff matches the persisted scope.
        if matches!(&self.diff_source, DiffSource::PullRequest(_))
            && let Some(range) = self.commit_selection_range
            && !self.pr_commits.is_empty()
            && (range.0 > 0 || range.1 + 1 < self.pr_commits.len())
        {
            self.spawn_pr_range_reload();
        }

        Ok(())
    }

    /// Reload the PR's head from the forge. If the head SHA changed, this
    /// switches sessions so old-head draft comments stay with the old
    /// session and the new session starts clean.
    /// Capture the cursor's current file + line numbers so we can try to
    /// land back here after `:e` rebuilds the diff. Returns `None` when
    /// the cursor isn't on a diff line (e.g., it's on a header / comment
    /// / hunk header / expander).
    fn capture_pr_cursor_anchor(&self) -> Option<PrCursorAnchor> {
        let annotation = self.line_annotations.get(self.diff_state.cursor_line)?;
        let (file_idx, old_lineno, new_lineno) = match annotation {
            AnnotatedLine::DiffLine {
                file_idx,
                old_lineno,
                new_lineno,
                ..
            } => (*file_idx, *old_lineno, *new_lineno),
            AnnotatedLine::SideBySideLine {
                file_idx,
                old_lineno,
                new_lineno,
                ..
            } => (*file_idx, *old_lineno, *new_lineno),
            AnnotatedLine::ExpandedContext { gap_id, .. } => {
                // Approximate: drop back to the file index from the gap.
                let file_idx = gap_id.file_idx;
                (file_idx, None, None)
            }
            _ => {
                let file_idx = annotation_file_idx(annotation)?;
                (file_idx, None, None)
            }
        };
        let path = self.diff_files.get(file_idx)?.display_path().clone();
        Some(PrCursorAnchor {
            path,
            new_lineno,
            old_lineno,
        })
    }

    /// Move the cursor to a sensible spot after a reload that may have
    /// shifted file ordering / hunk boundaries. Best-effort: match the
    /// exact `(path, new_lineno)` if it still exists, else the same
    /// `(path, old_lineno)` on the LEFT side, else the file's first
    /// annotation, else stay at line 0.
    fn restore_pr_cursor_to_anchor(&mut self, anchor: &PrCursorAnchor) {
        let mut best: Option<usize> = None;
        let mut file_first: Option<usize> = None;
        for (idx, ann) in self.line_annotations.iter().enumerate() {
            let file_idx = match ann {
                AnnotatedLine::DiffLine { file_idx, .. }
                | AnnotatedLine::SideBySideLine { file_idx, .. }
                | AnnotatedLine::HunkHeader { file_idx, .. }
                | AnnotatedLine::FileHeader { file_idx, .. } => *file_idx,
                _ => continue,
            };
            let Some(file) = self.diff_files.get(file_idx) else {
                continue;
            };
            if file.display_path() != &anchor.path {
                continue;
            }
            file_first.get_or_insert(idx);
            let (line_new, line_old) = match ann {
                AnnotatedLine::DiffLine {
                    old_lineno,
                    new_lineno,
                    ..
                }
                | AnnotatedLine::SideBySideLine {
                    old_lineno,
                    new_lineno,
                    ..
                } => (*new_lineno, *old_lineno),
                _ => (None, None),
            };
            if anchor.new_lineno.is_some() && line_new == anchor.new_lineno {
                best = Some(idx);
                break;
            }
            if anchor.old_lineno.is_some() && line_old == anchor.old_lineno {
                best = Some(idx);
                // Don't break — a later RIGHT-side match may still be better.
            }
        }
        let target = best.or(file_first).unwrap_or(0);
        // `move_cursor_to_annotation` updates cursor_line AND adjusts
        // `scroll_offset` so the cursor stays in the viewport. Without
        // it the viewport snaps back to the top of the diff after the
        // reload.
        self.move_cursor_to_annotation(target);
    }

    /// Persist the active inline selection on the session (PR mode only).
    /// `None` is written when the range covers all commits so re-open
    /// doesn't trigger an unnecessary subset re-fetch.
    pub fn persist_pr_commit_selection_range(&mut self) {
        if !matches!(self.diff_source, DiffSource::PullRequest(_)) {
            return;
        }
        let total = self.pr_commits.len();
        let value = match self.commit_selection_range {
            Some((s, e)) if total > 0 && (s > 0 || e + 1 < total) => Some((s, e)),
            _ => None,
        };
        self.session.commit_selection_range = value;
        self.session.updated_at = chrono::Utc::now();
        let _ = self.save_current_session_merging_external();
    }

    /// Resolve the active inline selection (PR mode) to (start_sha,
    /// end_sha). `start_sha` is the parent of the *oldest* selected
    /// commit; `end_sha` is the *newest*. Because `pr_commits` is stored
    /// newest-first, the oldest selected commit is at `range.1` and the
    /// newest at `range.0`.
    ///
    /// Returns `None` outside PR mode, when the selection is empty, or
    /// when the resolved parent isn't available — in that case the
    /// caller falls back to the cached cumulative PR diff.
    pub fn pr_range_sha_pair(&self) -> Option<(String, String)> {
        let DiffSource::PullRequest(ref pr) = self.diff_source else {
            return None;
        };
        let (start_idx, end_idx) = self.commit_selection_range?;
        if self.pr_commits.is_empty() || start_idx > end_idx || end_idx >= self.pr_commits.len() {
            return None;
        }
        // Newest-first: `end_idx` is the oldest, `start_idx` is the newest.
        let newest = self.pr_commits.get(start_idx)?;
        // Parent of the oldest selected commit. If the oldest selected commit
        // is the PR's first commit (oldest commit overall, at the bottom of
        // the list), its parent is the PR's base SHA.
        let parent_sha = if end_idx + 1 < self.pr_commits.len() {
            self.pr_commits[end_idx + 1].oid.clone()
        } else {
            pr.base_sha.clone()
        };
        Some((parent_sha, newest.oid.clone()))
    }

    /// Reload the PR diff for the currently selected inline commit
    /// subrange. Uses the cached cumulative diff when the selection
    /// covers all commits; spawns a background `compare` fetch otherwise.
    pub fn reload_pr_inline_selection(&mut self) {
        // No-op outside PR mode.
        if !matches!(self.diff_source, DiffSource::PullRequest(_)) {
            return;
        }
        let Some(range) = self.commit_selection_range else {
            return;
        };
        let total = self.pr_commits.len();
        if total == 0 {
            return;
        }

        // Full-range selection: restore the cached cumulative diff
        // without hitting the network.
        if range.0 == 0 && range.1 + 1 == total {
            self.apply_cached_full_pr_diff();
            return;
        }

        // Strict subset → range re-fetch on a background thread.
        self.spawn_pr_range_reload();
    }

    /// Restore the cached cumulative PR diff into the diff view. Used when
    /// the user toggles the selector back to "all commits".
    fn apply_cached_full_pr_diff(&mut self) {
        let Some(files) = self.range_diff_files.clone() else {
            return;
        };
        let anchor = self.capture_pr_cursor_anchor();
        self.diff_files = files;
        self.clear_expanded_gaps();
        for file in &self.diff_files {
            self.session.add_diff_file(file);
        }
        self.sort_files_by_directory(true);
        self.expand_all_dirs();
        self.rebuild_annotations();
        if let Some(anchor) = anchor {
            self.restore_pr_cursor_to_anchor(&anchor);
        }
    }

    /// Kick off a background fetch of `compare/<start>...<end>` and apply
    /// it on the main thread. Cancels any in-flight range reload (a fresh
    /// toggle invalidates the previous request).
    pub fn spawn_pr_range_reload(&mut self) {
        let DiffSource::PullRequest(current) = self.diff_source.clone() else {
            return;
        };
        let Some((start_sha, end_sha)) = self.pr_range_sha_pair() else {
            return;
        };
        let Some(range) = self.commit_selection_range else {
            return;
        };

        let anchor = self.capture_pr_cursor_anchor();
        let request = PrRangeReloadRequest {
            repository: current.key.repository.clone(),
            pr_number: current.key.number,
            head_sha: current.key.head_sha.clone(),
            start_sha: start_sha.clone(),
            end_sha: end_sha.clone(),
            range,
            started_at: Instant::now(),
            anchor,
        };
        // A fresh toggle supersedes any in-flight fetch.
        self.pr_range_reload_state = Some(request.clone());

        let local_checkout = self
            .forge_backend
            .as_deref()
            .and_then(|backend| backend.local_checkout_path());

        let (tx, rx) = std::sync::mpsc::channel();
        self.pr_range_reload_rx = Some(rx);

        let repository = current.key.repository.clone();
        let pr_number = current.key.number;
        let head_sha = current.key.head_sha.clone();
        let base_sha = current.base_sha.clone();
        std::thread::spawn(move || {
            let backend = create_forge_backend(&repository, local_checkout);
            let details = crate::forge::traits::PullRequestDetails {
                repository: repository.clone(),
                number: pr_number,
                title: String::new(),
                url: String::new(),
                state: "OPEN".to_string(),
                is_draft: false,
                author: None,
                head_ref_name: String::new(),
                base_ref_name: String::new(),
                head_sha,
                base_sha,
                body: String::new(),
                updated_at: None,
                closed: false,
                merged_at: None,
                diff_start_sha: None,
            };
            let outcome = backend
                .get_pull_request_commit_range_diff(&details, &start_sha, &end_sha)
                .map_err(|e| e.to_string());
            let _ = tx.send(PrRangeReloadEvent::Done {
                request,
                result: outcome,
            });
        });
    }

    /// Pump any pending range-reload result, parse on the main thread, and
    /// apply. Stale results (the user toggled again, or left PR mode) are
    /// silently dropped.
    pub fn poll_pr_range_reload_events(&mut self) {
        let Some(rx) = self.pr_range_reload_rx.as_ref() else {
            return;
        };
        let event = match rx.try_recv() {
            Ok(e) => e,
            Err(_) => return,
        };
        self.pr_range_reload_rx = None;

        let PrRangeReloadEvent::Done { request, result } = event;
        let in_flight = self.pr_range_reload_state.clone();
        // Only apply if this result still matches the active in-flight
        // request — toggling again, switching PRs, or reloading the head
        // before this lands invalidates it.
        let still_active = in_flight.as_ref().is_some_and(|s| {
            s.start_sha == request.start_sha
                && s.end_sha == request.end_sha
                && s.repository == request.repository
                && s.pr_number == request.pr_number
                && s.head_sha == request.head_sha
                && s.range == request.range
        });
        if !still_active {
            return;
        }
        self.pr_range_reload_state = None;

        match result {
            Ok(patch) => {
                if let Err(e) = self.finish_pr_range_reload(&request, &patch) {
                    self.set_error(format!("Range diff failed: {e}"));
                }
            }
            Err(e) => {
                self.set_error(format!("Range diff failed: {e}"));
            }
        }
    }

    pub(in crate::app) fn finish_pr_range_reload(
        &mut self,
        request: &PrRangeReloadRequest,
        patch: &str,
    ) -> Result<()> {
        use crate::vcs::diff_parser::{DiffFormat, parse_unified_diff};

        let highlighter = self.theme.syntax_highlighter();
        let parsed = match parse_unified_diff(patch, DiffFormat::GitStyle, highlighter) {
            Ok(files) => files,
            Err(TuicrError::NoChanges) => Vec::new(),
            Err(e) => return Err(e),
        };

        let local_checkout = self
            .forge_backend
            .as_deref()
            .and_then(|b| b.local_checkout_path());
        let files = match local_checkout.as_deref() {
            Some(root) => crate::tuicrignore::filter_diff_files(root, parsed),
            None => parsed,
        };

        self.diff_files = files;
        self.clear_expanded_gaps();
        // Range diffs can hide hunks that are still reviewed in the broader
        // PR session, so registration must not prune them.
        Self::register_diff_files(&mut self.session, &self.diff_files, true);
        self.sort_files_by_directory(true);
        self.expand_all_dirs();
        self.rebuild_annotations();

        if let Some(anchor) = &request.anchor {
            self.restore_pr_cursor_to_anchor(anchor);
        }
        Ok(())
    }

    /// Kick off `:e` asynchronously. Captures the cursor anchor, sets
    /// the reload state for the spinner, and spawns the network fetch
    /// on a background thread. Returns immediately. The result is
    /// applied later in `poll_pr_reload_events`.
    pub fn spawn_pr_reload(&mut self) -> Result<()> {
        use crate::forge::pr_open::fetch_pr_data;
        use crate::forge::traits::PullRequestTarget;

        let DiffSource::PullRequest(current) = self.diff_source.clone() else {
            return Err(TuicrError::UnsupportedOperation(
                "Not in PR mode".to_string(),
            ));
        };
        if self.pr_reload_state.is_some() {
            return Ok(()); // already in flight; the existing spinner is enough
        }

        let anchor = self.capture_pr_cursor_anchor();
        let request = PrReloadRequest {
            repository: current.key.repository.clone(),
            pr_number: current.key.number,
            head_sha: current.key.head_sha.clone(),
            started_at: Instant::now(),
            anchor,
        };
        self.pr_reload_state = Some(request.clone());

        let local_checkout = self
            .forge_backend
            .as_deref()
            .and_then(|backend| backend.local_checkout_path());

        let (tx, rx) = std::sync::mpsc::channel();
        self.pr_reload_rx = Some(rx);

        let repository = current.key.repository.clone();
        let pr_number = current.key.number;
        std::thread::spawn(move || {
            let backend = create_forge_backend(&repository, local_checkout);
            let target =
                PullRequestTarget::with_repository(repository, pr_number, pr_number.to_string());
            let outcome = fetch_pr_data(backend.as_ref(), target).map_err(|e| e.to_string());
            let _ = tx.send(PrReloadEvent::Done {
                request,
                result: outcome,
            });
        });
        Ok(())
    }

    /// Pump a pending reload result. Parses + applies on the main thread,
    /// then restores the cursor to the remembered anchor.
    pub fn poll_pr_reload_events(&mut self) {
        let Some(rx) = self.pr_reload_rx.as_ref() else {
            return;
        };
        let event = match rx.try_recv() {
            Ok(e) => e,
            Err(_) => return,
        };
        self.pr_reload_rx = None;
        let in_flight = self.pr_reload_state.clone();
        self.pr_reload_state = None;
        let PrReloadEvent::Done { request, result } = event;
        if !in_flight
            .as_ref()
            .is_some_and(|s| s.pr_number == request.pr_number && s.repository == request.repository)
        {
            return;
        }
        match result {
            Ok((details, patch, commits, review_metadata)) => {
                if let Err(e) =
                    self.finish_pr_reload(details, patch, commits, review_metadata, &request)
                {
                    self.set_error(format!("Reload failed: {e}"));
                }
            }
            Err(e) => {
                self.set_error(format!("Reload failed: {e}"));
            }
        }
    }

    pub(in crate::app) fn finish_pr_reload(
        &mut self,
        details: crate::forge::traits::PullRequestDetails,
        patch: String,
        commits: Vec<crate::forge::traits::PullRequestCommit>,
        review_metadata: crate::forge::traits::PullRequestReviewMetadata,
        request: &PrReloadRequest,
    ) -> Result<()> {
        use crate::forge::pr_open::prepare_open_pr;

        let local_checkout = self
            .forge_backend
            .as_deref()
            .and_then(|backend| backend.local_checkout_path());
        let highlighter = self.theme.syntax_highlighter();
        let opened = prepare_open_pr(
            details,
            &patch,
            commits,
            review_metadata,
            local_checkout.as_deref(),
            highlighter,
        )?;

        let head_changed = opened.details.head_sha != request.head_sha;
        if head_changed {
            let details_for_threads = opened.details.clone();
            let opened = self.opened_pr_with_new_head_session(opened)?;
            let backend = create_forge_backend(&request.repository, local_checkout.clone());
            let previous_message = self.message.clone();
            self.enter_pr_diff_mode(backend, opened)?;
            self.spawn_pr_threads_fetch(&details_for_threads, local_checkout);
            if self.message == previous_message {
                self.set_message("Reloaded PR at new head".to_string());
            }
        } else {
            self.set_pr_last_reviewed_commit_from_metadata(
                &opened.commits,
                &opened.review_metadata,
            );
            self.diff_files = opened.diff_files;
            self.clear_expanded_gaps();
            for file in &self.diff_files {
                self.session.add_diff_file(file);
            }
            self.sort_files_by_directory(true);
            self.expand_all_dirs();
            self.rebuild_annotations();
            self.refetch_pr_threads();
            self.set_message("Reloaded PR (no new commits)".to_string());
        }

        if let Some(anchor) = &request.anchor {
            self.restore_pr_cursor_to_anchor(anchor);
        }
        Ok(())
    }

    /// Synchronous reload. Production code uses `spawn_pr_reload` for the
    /// async path; kept as a seam for tests that need to drive a reload
    /// in one call without an mpsc round-trip.
    #[allow(dead_code)]
    pub fn reload_pull_request(&mut self) -> Result<bool> {
        let DiffSource::PullRequest(current) = self.diff_source.clone() else {
            return Err(TuicrError::UnsupportedOperation(
                "Not in PR mode".to_string(),
            ));
        };

        let local_checkout = self
            .forge_backend
            .as_deref()
            .and_then(|backend| backend.local_checkout_path());
        let backend = create_forge_backend(&current.key.repository, local_checkout.clone());
        self.reload_pull_request_with_backend(backend, local_checkout)
    }

    /// Inner reload path. Takes the forge backend as a parameter so tests
    /// can inject a fake without going through `gh`.
    #[allow(dead_code)]
    pub fn reload_pull_request_with_backend(
        &mut self,
        backend: Box<dyn ForgeBackend>,
        local_checkout: Option<std::path::PathBuf>,
    ) -> Result<bool> {
        use crate::forge::pr_open::open_pull_request;

        let DiffSource::PullRequest(current) = self.diff_source.clone() else {
            return Err(TuicrError::UnsupportedOperation(
                "Not in PR mode".to_string(),
            ));
        };

        let target = crate::forge::traits::PullRequestTarget::with_repository(
            current.key.repository.clone(),
            current.key.number,
            current.key.number.to_string(),
        );
        let highlighter = self.theme.syntax_highlighter();
        let opened = open_pull_request(
            backend.as_ref(),
            target,
            local_checkout.as_deref(),
            highlighter,
        )?;

        let head_changed = opened.details.head_sha != current.key.head_sha;
        if head_changed {
            // Save the old-head session before switching so drafts persist.
            let details_for_threads = opened.details.clone();
            let opened = self.opened_pr_with_new_head_session(opened)?;
            self.enter_pr_diff_mode(backend, opened)?;
            // Fetch threads against the new head; old-head threads stay
            // tied to the old session and are dropped here.
            self.spawn_pr_threads_fetch(&details_for_threads, local_checkout.clone());
        } else {
            // Same head: re-parse the diff to pick up any side-channel
            // changes (rare), but keep the session intact.
            self.set_pr_last_reviewed_commit_from_metadata(
                &opened.commits,
                &opened.review_metadata,
            );
            self.diff_files = opened.diff_files;
            self.clear_expanded_gaps();
            for file in &self.diff_files {
                self.session.add_diff_file(file);
            }
            self.sort_files_by_directory(true);
            self.expand_all_dirs();
            self.rebuild_annotations();
        }

        Ok(head_changed)
    }

    /// Spawn a background thread that fetches the initial PR list. The
    /// resulting `PrLoadEvent::Initial` is delivered through `pr_load_rx`
    /// and applied in the main loop via `poll_pr_load_events`.
    ///
    /// On the first PR-tab visit (`skip_resolution=false`) the thread first
    /// resolves the detected origin to its canonical (fork-parent) repo via
    /// `gh api`, then lists PRs against that canonical. Subsequent visits
    /// reuse the already-resolved repo and skip the resolver.
    pub(in crate::app) fn spawn_pr_initial_load(
        &mut self,
        origin: ForgeRepository,
        override_repo: Option<ForgeRepository>,
        skip_resolution: bool,
        scope: crate::forge::traits::PullRequestListScope,
    ) {
        use crate::forge::selector::PR_PAGE_SIZE;
        use crate::forge::traits::{ForgeKind, PullRequestListQuery};

        let (tx, rx) = std::sync::mpsc::channel();
        self.pr_load_rx = Some(rx);

        std::thread::spawn(move || {
            // Canonical resolution (fork parent lookup) is GitHub-only.
            let canonical = if skip_resolution || origin.kind != ForgeKind::GitHub {
                override_repo.unwrap_or(origin)
            } else {
                use crate::forge::canonical::resolve_canonical_repository;
                use crate::forge::github::gh::SystemGhRunner;
                let runner = SystemGhRunner;
                resolve_canonical_repository(&origin, override_repo.as_ref(), &runner)
            };
            let backend = create_forge_backend(&canonical, None);
            let query =
                PullRequestListQuery::first_page_with_scope(canonical.clone(), PR_PAGE_SIZE, scope);
            let result = backend
                .list_pull_requests(query)
                .map(|page| (page.pull_requests, page.has_more))
                .map_err(|err| err.to_string());
            let _ = tx.send(PrLoadEvent::Initial { canonical, result });
        });
    }

    /// Spawn a background thread that fetches the next page of PRs.
    fn spawn_pr_load_more(
        &mut self,
        repository: ForgeRepository,
        scope: crate::forge::traits::PullRequestListScope,
        already_loaded: usize,
    ) {
        use crate::forge::selector::PR_PAGE_SIZE;
        use crate::forge::traits::PullRequestListQuery;

        let (tx, rx) = std::sync::mpsc::channel();
        self.pr_load_rx = Some(rx);

        std::thread::spawn(move || {
            let backend = create_forge_backend(&repository, None);
            let query = PullRequestListQuery {
                repository,
                already_loaded,
                page_size: PR_PAGE_SIZE,
                scope,
            };
            let result = backend
                .list_pull_requests(query)
                .map(|page| (page.pull_requests, page.has_more))
                .map_err(|err| err.to_string());
            let _ = tx.send(PrLoadEvent::LoadMore(result));
        });
    }

    /// Pump any pending PR fetch events into the tab state.
    /// Called from the main loop each tick; non-blocking.
    pub fn poll_pr_load_events(&mut self) {
        let Some(rx) = self.pr_load_rx.as_ref() else {
            return;
        };
        let mut events = Vec::new();
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }
        if events.is_empty() {
            return;
        }
        // The channel is single-use per fetch; drop the receiver once a
        // result has arrived so we don't keep checking it.
        self.pr_load_rx = None;
        for event in events {
            match event {
                PrLoadEvent::Initial { canonical, result } => {
                    // The background thread may have resolved a fork's origin
                    // to its upstream parent. Promote App state to the
                    // resolved repo so every PR-related call (open, threads,
                    // submit, load-more) targets the canonical from here on.
                    self.pr_tab.apply_canonical(canonical.clone());
                    self.forge_repository = Some(canonical);
                    self.canonical_resolved = true;
                    self.pr_tab.apply_initial_load(result);
                }
                PrLoadEvent::LoadMore(result) => self.pr_tab.apply_load_more(result),
            }
        }
        self.pr_tab.clamp_cursor();
    }

    pub fn pr_tab_cursor_up(&mut self) {
        self.pr_tab.cursor_up();
        self.pr_tab
            .ensure_cursor_visible(self.pr_list_viewport_height);
    }

    pub fn pr_tab_cursor_down(&mut self) {
        self.pr_tab.cursor_down();
        self.pr_tab
            .ensure_cursor_visible(self.pr_list_viewport_height);
    }

    pub fn toggle_pr_review_requested_filter(&mut self) {
        let Some((repo, scope)) = self.pr_tab.toggle_scope_and_start_reload() else {
            return;
        };
        self.pr_filter_draft = None;
        let override_repo = self.repo_url_override.clone();
        let skip_resolution = self.canonical_resolved;
        self.spawn_pr_initial_load(repo, override_repo, skip_resolution, scope);
        self.set_message(format!("PR list: {}", scope.label()));
    }

    /// Handle Enter on the PR tab. Returns true when the action was handled
    /// (load more triggered, PR open kicked off, error surfaced, etc).
    pub fn pr_tab_select(&mut self) -> bool {
        // Block re-entry while a previous open is still resolving — the
        // spinner glyph on the row already tells the user something is in
        // flight.
        if self.pr_open_state.is_some() {
            return true;
        }
        if self.pr_tab.cursor_on_load_more() {
            if let Some((repo, scope, already)) = self.pr_tab.start_load_more() {
                self.spawn_pr_load_more(repo, scope, already);
            }
            return true;
        }
        // Clone the summary so we drop the immutable borrow before mutating
        // the app to enter PR mode.
        let Some(summary) = self.pr_tab.cursor_pr().cloned() else {
            return false;
        };
        self.spawn_pr_open(&summary);
        true
    }

    /// Kick off the background fetch for a PR open. The main thread keeps
    /// rendering and pumping events; the resulting `PrOpenEvent::Done` is
    /// drained in `poll_pr_open_events` where parsing happens and PR mode
    /// is entered.
    fn spawn_pr_open(&mut self, summary: &crate::forge::traits::PullRequestSummary) {
        use crate::forge::pr_open::fetch_pr_data;
        use crate::forge::traits::PullRequestTarget;

        let local_checkout = Some(self.vcs_info.root_path.clone());
        let request = PrOpenRequest {
            repository: summary.repository.clone(),
            pr_number: summary.number,
            started_at: Instant::now(),
        };
        self.pr_open_state = Some(request.clone());

        let (tx, rx) = std::sync::mpsc::channel();
        self.pr_open_rx = Some(rx);

        let summary_repo = summary.repository.clone();
        let pr_number = summary.number;
        let thread_local_checkout = local_checkout.clone();
        std::thread::spawn(move || {
            let backend = create_forge_backend(&summary_repo, thread_local_checkout);
            let target =
                PullRequestTarget::with_repository(summary_repo, pr_number, pr_number.to_string());
            let outcome = fetch_pr_data(backend.as_ref(), target).map_err(|e| e.to_string());
            let _ = tx.send(PrOpenEvent::Done {
                request,
                result: outcome,
            });
        });
    }

    /// Drain any pending PR-open result and apply it. On success, parses
    /// the diff and enters PR diff mode; on failure, routes the error
    /// into the selector banner. Either way, clears `pr_open_state` and
    /// the receiver so the spinner stops animating.
    pub fn poll_pr_open_events(&mut self) {
        let Some(rx) = self.pr_open_rx.as_ref() else {
            return;
        };
        let event = match rx.try_recv() {
            Ok(e) => e,
            Err(_) => return,
        };
        self.pr_open_rx = None;
        let in_flight = self.pr_open_state.clone();
        self.pr_open_state = None;
        match event {
            PrOpenEvent::Done { request, result } => {
                // If the user cancelled (cleared pr_open_state) but the
                // background thread sent a result before being torn down,
                // ignore the result rather than entering PR mode.
                if !in_flight
                    .as_ref()
                    .map(|s| s.matches(&request.repository, request.pr_number))
                    .unwrap_or(false)
                {
                    return;
                }
                match result {
                    Ok((details, patch, commits, review_metadata)) => {
                        if let Err(e) =
                            self.finish_pr_open(details, patch, commits, review_metadata, &request)
                        {
                            self.set_error(format!(
                                "Failed to open PR #{}: {}",
                                request.pr_number, e
                            ));
                        }
                    }
                    Err(e) => {
                        self.set_error(format!("Failed to open PR #{}: {}", request.pr_number, e));
                    }
                }
            }
        }
    }

    /// Main-thread half of the PR open: parse the patch, build the
    /// session, and enter PR diff mode. Mirrors what the previous synchronous
    /// `open_pr_with_backend` did, but the network fetch has already
    /// happened on the background thread.
    fn finish_pr_open(
        &mut self,
        details: crate::forge::traits::PullRequestDetails,
        patch: String,
        commits: Vec<crate::forge::traits::PullRequestCommit>,
        review_metadata: crate::forge::traits::PullRequestReviewMetadata,
        request: &PrOpenRequest,
    ) -> Result<()> {
        use crate::forge::pr_open::prepare_open_pr;

        let local_checkout = Some(self.vcs_info.root_path.clone());
        let highlighter = self.theme.syntax_highlighter();
        let opened = prepare_open_pr(
            details.clone(),
            &patch,
            commits,
            review_metadata,
            local_checkout.as_deref(),
            highlighter,
        )?;
        let opened = Self::opened_pr_with_persisted_session(opened)?;
        let backend = create_forge_backend(&request.repository, local_checkout.clone());
        let previous_message = self.message.clone();
        self.enter_pr_diff_mode(backend, opened)?;
        // Kick the remote-thread fetch off on a fresh background thread.
        // The diff view is already up; threads fade in once they land.
        self.spawn_pr_threads_fetch(&details, local_checkout);
        if self.message == previous_message {
            self.set_message(format!(
                "Opened PR {}#{}",
                request.repository.display_name(),
                request.pr_number,
            ));
        }
        Ok(())
    }

    /// Kick off a background fetch of remote review threads for `details`.
    /// Replaces any in-flight fetch — we don't try to merge results across
    /// concurrent fetches because the head SHA scopes everything.
    pub(in crate::app) fn spawn_pr_threads_fetch(
        &mut self,
        details: &crate::forge::traits::PullRequestDetails,
        local_checkout: Option<std::path::PathBuf>,
    ) {
        self.forge_review_threads.clear();
        self.forge_review_threads_loading = true;

        let (tx, rx) = std::sync::mpsc::channel();
        self.pr_threads_rx = Some(rx);

        let details_clone = details.clone();
        let repository = details.repository.clone();
        let pr_number = details.number;
        let head_sha = details.head_sha.clone();

        std::thread::spawn(move || {
            let backend = create_forge_backend(&repository, local_checkout);
            let threads = backend
                .list_review_threads(&details_clone)
                .map_err(|e| e.to_string());
            let summaries = backend
                .list_review_summaries(&details_clone)
                .map_err(|e| e.to_string());
            let _ = tx.send(PrThreadsEvent::Done {
                repository,
                pr_number,
                head_sha,
                threads,
                summaries,
            });
        });
    }

    /// Drain any pending remote-thread fetch result and apply it. Stale
    /// results (a result that arrived after the user switched to a
    /// different PR) are discarded.
    pub fn poll_pr_threads_events(&mut self) {
        let Some(rx) = self.pr_threads_rx.as_ref() else {
            return;
        };
        let event = match rx.try_recv() {
            Ok(e) => e,
            Err(_) => return,
        };
        self.pr_threads_rx = None;
        self.forge_review_threads_loading = false;

        match event {
            PrThreadsEvent::Done {
                repository,
                pr_number,
                head_sha,
                threads,
                summaries,
            } => {
                // Validate against the currently open PR. If the user has
                // opened a different PR (or left PR mode) while the fetch
                // was in flight, drop the result silently.
                let current = match &self.diff_source {
                    DiffSource::PullRequest(pr) => Some((
                        pr.key.repository.clone(),
                        pr.key.number,
                        pr.key.head_sha.clone(),
                    )),
                    _ => None,
                };
                let still_relevant = current
                    .as_ref()
                    .map(|(r, n, sha)| *r == repository && *n == pr_number && *sha == head_sha)
                    .unwrap_or(false);
                if !still_relevant {
                    return;
                }
                let mut had_error = false;
                let mut threads_loaded = false;
                match threads {
                    Ok(t) => {
                        self.forge_review_threads = t;
                        threads_loaded = true;
                    }
                    Err(e) => {
                        self.forge_review_threads = Vec::new();
                        self.set_warning(format!("Failed to load remote comments: {e}"));
                        had_error = true;
                    }
                }
                match summaries {
                    Ok(s) => {
                        self.forge_review_summaries = s;
                    }
                    Err(e) => {
                        self.forge_review_summaries = Vec::new();
                        had_error = true;
                        // Only surface the summary error if the threads call
                        // succeeded — otherwise the user already got a
                        // warning for the broader failure.
                        if threads_loaded {
                            self.set_warning(format!("Failed to load remote reviews: {e}"));
                        }
                    }
                }
                if !had_error {
                    self.prune_locked_comments();
                    let _ = self.save_current_session_merging_external();
                }
                self.rebuild_annotations();
            }
        }
    }

    /// Update the per-session remote comments visibility and repaint.
    /// Returns `true` if the visibility actually changed.
    pub fn set_remote_comments_visibility(
        &mut self,
        visibility: crate::forge::remote_comments::PrCommentsVisibility,
    ) -> bool {
        if self.session.remote_comments_visibility == visibility {
            return false;
        }
        self.session.remote_comments_visibility = visibility;
        self.rebuild_annotations();
        true
    }

    /// Abort an in-flight PR open. Drops the receiver so the eventual
    /// thread send becomes a no-op; clears the spinner state.
    pub fn cancel_pr_open(&mut self) -> bool {
        if self.pr_open_state.is_none() {
            return false;
        }
        self.pr_open_state = None;
        self.pr_open_rx = None;
        self.set_message("PR open cancelled".to_string());
        true
    }

    /// Re-fetch remote review threads for the currently open PR. Called
    /// from `:e` so users can pull the latest discussions without
    /// reopening the PR. No-op outside PR mode.
    pub fn refetch_pr_threads(&mut self) {
        let local_checkout = self
            .forge_backend
            .as_deref()
            .and_then(|b| b.local_checkout_path());
        let details = match &self.diff_source {
            DiffSource::PullRequest(pr) => crate::forge::traits::PullRequestDetails {
                repository: pr.key.repository.clone(),
                number: pr.key.number,
                title: pr.title.clone(),
                url: pr.url.clone(),
                state: pr.state.clone(),
                is_draft: false,
                author: None,
                head_ref_name: pr.head_ref_name.clone(),
                base_ref_name: pr.base_ref_name.clone(),
                head_sha: pr.key.head_sha.clone(),
                base_sha: pr.base_sha.clone(),
                body: String::new(),
                updated_at: None,
                closed: pr.closed,
                merged_at: None,
                diff_start_sha: None,
            },
            _ => return,
        };
        self.spawn_pr_threads_fetch(&details, local_checkout);
    }

    /// Open a PR using the provided forge backend, synchronously. Exists
    /// as a seam for tests that want to drive the open without spinning
    /// up a background thread + mpsc round-trip. Production paths go
    /// through `spawn_pr_open` (selector) or `new_from_pr_target` (CLI).
    ///
    /// Synchronously fetches `list_review_threads` from the same backend
    /// and applies it before returning. This is the convenient seam for
    /// integration tests; the production async path uses
    /// `spawn_pr_threads_fetch` instead.
    #[allow(dead_code)]
    pub fn open_pr_with_backend(
        &mut self,
        summary: &crate::forge::traits::PullRequestSummary,
        backend: Box<dyn ForgeBackend>,
        local_checkout: Option<std::path::PathBuf>,
    ) -> Result<()> {
        use crate::forge::pr_open::open_pull_request;
        use crate::forge::traits::PullRequestTarget;

        let target = PullRequestTarget::with_repository(
            summary.repository.clone(),
            summary.number,
            summary.number.to_string(),
        );
        let highlighter = self.theme.syntax_highlighter();
        let opened = open_pull_request(
            backend.as_ref(),
            target,
            local_checkout.as_deref(),
            highlighter,
        )?;
        let opened = Self::opened_pr_with_persisted_session(opened)?;
        // Sync thread + summary fetch — tests assert on
        // `app.forge_review_threads`/`forge_review_summaries` immediately
        // after this returns.
        let threads = backend
            .list_review_threads(&opened.details)
            .unwrap_or_default();
        let summaries = backend
            .list_review_summaries(&opened.details)
            .unwrap_or_default();
        self.enter_pr_diff_mode(backend, opened)?;
        self.forge_review_threads = threads;
        self.forge_review_summaries = summaries;
        self.prune_locked_comments();
        self.rebuild_annotations();
        Ok(())
    }

    pub fn begin_pr_filter(&mut self) {
        if !self.pr_tab.is_loaded() {
            return;
        }
        // Seed the draft from the current applied filter so the user can
        // refine it. Starting from empty is also reasonable; preserving the
        // current filter feels less surprising when re-opening.
        let current = match &self.pr_tab {
            PullRequestsTab::Loaded { filter, .. } => filter.clone(),
            _ => String::new(),
        };
        self.pr_filter_draft = Some(current);
    }

    pub fn commit_pr_filter(&mut self) {
        if let Some(draft) = self.pr_filter_draft.take() {
            self.pr_tab.set_filter(draft);
        }
    }

    pub fn cancel_pr_filter(&mut self) {
        self.pr_filter_draft = None;
    }

    pub fn pr_filter_insert_char(&mut self, ch: char) {
        if let Some(draft) = self.pr_filter_draft.as_mut() {
            draft.push(ch);
        }
    }

    pub fn pr_filter_delete_char(&mut self) {
        if let Some(draft) = self.pr_filter_draft.as_mut() {
            draft.pop();
        }
    }

    pub fn pr_filter_clear(&mut self) {
        if let Some(draft) = self.pr_filter_draft.as_mut() {
            draft.clear();
        }
    }

    pub fn pr_filter_editing(&self) -> bool {
        self.pr_filter_draft.is_some()
    }
}
