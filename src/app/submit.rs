use super::*;

impl App {
    /// Drive `:submit*` preflight: walk every local-draft comment in the
    /// current PR session, map each one against the displayed diff, bucket
    /// the results, and transition into the resolver (when there are
    /// unmappable comments) or the final-confirmation modal.
    ///
    /// PR 5 does not call the network; `[y]` in the confirmation modal
    /// stubs a "PR 6 will wire the network call" info message.
    pub fn start_submit(&mut self, event: crate::forge::submit::SubmitEvent) {
        self.start_submit_with(event, false);
    }

    /// Like `start_submit`, but when `skip_confirm` is `true` the flow
    /// bypasses `SubmitConfirm`. The action-picker path uses this because
    /// picking IS the confirmation; the resolver (if any unmappable
    /// comments) still runs first, then dispatches the network call
    /// directly. `:submit <event>` callers should pass `false`.
    pub fn start_submit_with(
        &mut self,
        event: crate::forge::submit::SubmitEvent,
        skip_confirm: bool,
    ) {
        use crate::forge::submit::{
            CommentAnchor, InlineComment, ResolverAction, UnmappableItem, map_comment,
        };

        let DiffSource::PullRequest(pr) = &self.diff_source else {
            self.set_warning(":submit only applies in PR mode");
            return;
        };
        if pr.is_read_only() {
            let reason = pr.read_only_reason().unwrap_or("read only");
            self.set_warning(format!("Cannot submit: PR is {reason}"));
            return;
        }
        // When the inline commit selector shows a strict subset, comments
        // anchor to the displayed (subset) diff, so `commit_id` must be the
        // SHA the diff was computed against — otherwise GitHub rejects with
        // 422 because the line/position isn't present in the diff against
        // the cumulative PR head. `pr_commits` is stored newest-first, so
        // the head of a (start_idx..=end_idx) range is `pr_commits[start_idx]`.
        let commit_id = match self.commit_selection_range {
            Some((start_idx, end_idx))
                if !self.pr_commits.is_empty()
                    && start_idx <= end_idx
                    && end_idx < self.pr_commits.len()
                    && !(start_idx == 0 && end_idx + 1 == self.pr_commits.len()) =>
            {
                self.pr_commits[start_idx].oid.clone()
            }
            _ => pr.key.head_sha.clone(),
        };

        // Source of truth for the diff: when the inline commit selector is
        // showing a strict subset, `range_diff_files` carries the merged
        // subset diff; otherwise `diff_files` is canonical.
        let files: Vec<&DiffFile> = match self.range_diff_files.as_ref() {
            Some(range) => range.iter().collect(),
            None => self.diff_files.iter().collect(),
        };

        let mut mappable: Vec<InlineComment> = Vec::new();
        let mut unmappable: Vec<UnmappableItem> = Vec::new();
        let mut total_local_drafts = 0_usize;

        // Walk file-level and line comments in display order. Review-level
        // comments (session.review_comments) are NOT inline-mapped; they
        // appear in the body via `build_review_body`.
        for file in &files {
            let Some(review) = self.session.files.get(file.display_path()) else {
                continue;
            };
            for comment in &review.file_comments {
                if comment.is_locked() || !self.comment_visible(comment) {
                    continue;
                }
                total_local_drafts += 1;
                bucket_mapping(
                    map_comment(comment, CommentAnchor::FileLevel, file, &self.forge_config),
                    &mut mappable,
                    &mut unmappable,
                );
            }
            let mut keys: Vec<&u32> = review.line_comments.keys().collect();
            keys.sort();
            for key in keys {
                for comment in &review.line_comments[key] {
                    if comment.is_locked() || !self.comment_visible(comment) {
                        continue;
                    }
                    total_local_drafts += 1;
                    let anchor = if comment.line_range.is_some() {
                        CommentAnchor::Range
                    } else {
                        CommentAnchor::Line {
                            line: *key,
                            side: comment.side.unwrap_or_default(),
                        }
                    };
                    bucket_mapping(
                        map_comment(comment, anchor, file, &self.forge_config),
                        &mut mappable,
                        &mut unmappable,
                    );
                }
            }
        }

        // Approve is the one event that's meaningful with no comments — a
        // bare "LGTM" approval. Every other event needs at least one local
        // draft comment or a review-level comment, otherwise there's
        // nothing to submit.
        let bare_allowed = matches!(event, crate::forge::submit::SubmitEvent::Approve);
        if !bare_allowed && total_local_drafts == 0 && self.session.review_comments.is_empty() {
            self.set_warning("Nothing to submit — no local-draft comments");
            return;
        }

        let resolver_choices = vec![ResolverAction::default(); unmappable.len()];
        let has_unmappable = !unmappable.is_empty();
        self.submit_state = Some(SubmitState {
            event,
            mappable,
            unmappable,
            resolver_choices,
            resolver_cursor: 0,
            commit_id,
            skip_confirm,
        });

        if has_unmappable {
            self.input_mode = InputMode::SubmitResolver;
        } else if skip_confirm {
            self.input_mode = InputMode::Normal;
            self.confirm_submit();
        } else {
            self.input_mode = InputMode::SubmitConfirm;
        }
    }

    /// Open the bare-`:submit` action picker. The user picks
    /// Comment/Approve/Request changes/Draft (or cancels); the picked event
    /// then runs through preflight with `skip_confirm = true` so no extra
    /// confirmation modal follows.
    pub fn start_submit_action_picker(&mut self) {
        if !matches!(self.diff_source, DiffSource::PullRequest(_)) {
            self.set_warning(":submit only applies in PR mode");
            return;
        }
        self.submit_picker_cursor = 0;
        self.input_mode = InputMode::SubmitActionPicker;
    }

    /// Move the action-picker cursor down by one row, wrapping at the end.
    pub fn submit_picker_cursor_down(&mut self) {
        let total = SUBMIT_PICKER_EVENTS.len();
        if total > 0 {
            self.submit_picker_cursor = (self.submit_picker_cursor + 1) % total;
        }
    }

    /// Move the action-picker cursor up by one row, wrapping at the start.
    pub fn submit_picker_cursor_up(&mut self) {
        let total = SUBMIT_PICKER_EVENTS.len();
        if total > 0 {
            self.submit_picker_cursor = (self.submit_picker_cursor + total - 1) % total;
        }
    }

    /// Confirm the action picker selection: dispatch into preflight with the
    /// chosen event and `skip_confirm = true`.
    pub fn submit_picker_confirm(&mut self) {
        let Some(event) = SUBMIT_PICKER_EVENTS
            .get(self.submit_picker_cursor)
            .map(|(_, ev)| *ev)
        else {
            self.cancel_submit_action_picker();
            return;
        };
        self.input_mode = InputMode::Normal;
        self.start_submit_with(event, true);
    }

    /// Cancel the action picker without entering preflight.
    pub fn cancel_submit_action_picker(&mut self) {
        self.input_mode = InputMode::Normal;
        self.submit_picker_cursor = 0;
    }

    pub fn cancel_submit(&mut self) {
        self.submit_state = None;
        self.input_mode = InputMode::Normal;
    }

    /// Move the resolver cursor down by one row, clamped to the last row.
    pub fn submit_resolver_cursor_down(&mut self) {
        if let Some(state) = self.submit_state.as_mut()
            && state.resolver_cursor + 1 < state.unmappable.len()
        {
            state.resolver_cursor += 1;
        }
    }

    pub fn submit_resolver_cursor_up(&mut self) {
        if let Some(state) = self.submit_state.as_mut()
            && state.resolver_cursor > 0
        {
            state.resolver_cursor -= 1;
        }
    }

    pub fn submit_resolver_toggle(&mut self) {
        use crate::forge::submit::ResolverAction;
        if let Some(state) = self.submit_state.as_mut()
            && let Some(choice) = state.resolver_choices.get_mut(state.resolver_cursor)
        {
            *choice = match choice {
                ResolverAction::MoveToSummary => ResolverAction::Omit,
                ResolverAction::Omit => ResolverAction::MoveToSummary,
            };
        }
    }

    /// Advance from the resolver. When `skip_confirm` is set (action-picker
    /// path), dispatch the network call directly; otherwise route to
    /// `SubmitConfirm` for the final confirmation modal.
    pub fn submit_resolver_advance(&mut self) {
        let Some(state) = self.submit_state.as_ref() else {
            return;
        };
        if state.skip_confirm {
            self.input_mode = InputMode::Normal;
            self.confirm_submit();
        } else {
            self.input_mode = InputMode::SubmitConfirm;
        }
    }

    /// True iff the original review head and the latest known PR head
    /// disagree. PR 5 cannot trigger this (the open-time head equals
    /// `current_pr_head`), but the field is exposed so the renderer can
    /// fold the warning in once PR 6 refreshes the remote head.
    pub fn submit_head_is_stale(&self) -> bool {
        let Some(state) = self.submit_state.as_ref() else {
            return false;
        };
        match self.current_pr_head.as_deref() {
            Some(latest) => latest != state.commit_id,
            None => false,
        }
    }

    /// Confirm submit — PR 6 dispatches the async `gh api .../reviews` call.
    /// Builds the body + payload on the main thread, saves the session, then
    /// hands off to `spawn_pr_submit`. The modal disappears immediately; a
    /// status-bar spinner takes over until the result lands in
    /// `poll_pr_submit_events`.
    pub fn confirm_submit(&mut self) {
        if let Err(e) = self.spawn_pr_submit() {
            self.set_error(format!("Submit failed: {e}"));
            self.submit_state = None;
            self.input_mode = InputMode::Normal;
        }
    }

    /// Kick off the create-review call asynchronously. Pre-submit-saves the
    /// session, builds the JSON payload on the main thread, then runs the
    /// network round-trip on a background thread. The result is applied
    /// later in `poll_pr_submit_events`.
    pub fn spawn_pr_submit(&mut self) -> Result<()> {
        use crate::forge::submit::{MovedToSummaryItem, ResolverAction, build_review_body};
        use crate::forge::traits::{CreateReviewRequest, PullRequestTarget};

        // Snapshot identity from the PR diff source first so the borrow on
        // `submit_state` below doesn't conflict.
        let DiffSource::PullRequest(pr) = self.diff_source.clone() else {
            return Err(TuicrError::UnsupportedOperation(
                "Not in PR mode".to_string(),
            ));
        };
        if self.pr_submit_state.is_some() {
            return Ok(()); // already in flight; ignore
        }

        let Some(state) = self.submit_state.take() else {
            return Ok(());
        };

        let summary_items: Vec<MovedToSummaryItem> = state
            .unmappable
            .iter()
            .zip(state.resolver_choices.iter())
            .filter_map(|(item, action)| {
                if *action == ResolverAction::MoveToSummary {
                    Some(MovedToSummaryItem {
                        comment: item.comment.clone(),
                        file: item.file.clone(),
                    })
                } else {
                    None
                }
            })
            .collect();
        let summary_comment_ids: Vec<String> =
            summary_items.iter().map(|i| i.comment.id.clone()).collect();
        let review_comment_ids: Vec<String> = self
            .session
            .review_comments
            .iter()
            .map(|c| c.id.clone())
            .collect();
        let body = build_review_body(
            &self.session.review_comments,
            &summary_items,
            &self.forge_config,
        );

        // Save the session BEFORE the network call — keeps the user's
        // local-draft work durable if anything goes sideways below.
        let _ = self.save_current_session_merging_external();

        let in_flight = SubmitInFlightState {
            event: state.event,
            mappable: state.mappable.clone(),
            summary_comment_ids,
            review_comment_ids,
            moved_to_summary_count: summary_items.len(),
            head_sha_snapshot: state.commit_id.clone(),
            repository: pr.key.repository.clone(),
            pr_number: pr.key.number,
            started_at: Instant::now(),
        };
        self.pr_submit_state = Some(in_flight.clone());
        self.input_mode = InputMode::Normal;

        let local_checkout = self
            .forge_backend
            .as_deref()
            .and_then(|backend| backend.local_checkout_path());

        let (tx, rx) = std::sync::mpsc::channel();
        self.pr_submit_rx = Some(rx);

        let repository = in_flight.repository.clone();
        let pr_number = in_flight.pr_number;
        let head_sha = in_flight.head_sha_snapshot.clone();
        let event = in_flight.event;
        let mappable = in_flight.mappable.clone();
        let commit_id = state.commit_id.clone();

        std::thread::spawn(move || {
            let backend = create_forge_backend(&repository, local_checkout);
            // Need PR details for repo/owner routing; refetch lightly via
            // the same target the user opened with.
            let target = PullRequestTarget::with_repository(
                repository.clone(),
                pr_number,
                pr_number.to_string(),
            );
            let result = match backend.get_pull_request(target) {
                Ok(details) => backend
                    .create_review(
                        &details,
                        CreateReviewRequest {
                            event,
                            commit_id: &commit_id,
                            body: &body,
                            comments: &mappable,
                        },
                    )
                    .map_err(|e| e.to_string()),
                Err(e) => Err(e.to_string()),
            };
            let _ = tx.send(PrSubmitEvent::Done {
                repository,
                pr_number,
                head_sha,
                result,
            });
        });
        Ok(())
    }

    /// Pump a pending create-review result. Applies lifecycle writes + the
    /// success message, or surfaces a sticky error.
    pub fn poll_pr_submit_events(&mut self) {
        let Some(rx) = self.pr_submit_rx.as_ref() else {
            return;
        };
        let event = match rx.try_recv() {
            Ok(e) => e,
            Err(_) => return,
        };
        self.pr_submit_rx = None;
        let in_flight = self.pr_submit_state.take();
        let PrSubmitEvent::Done {
            repository,
            pr_number,
            head_sha,
            result,
        } = event;

        // Stale-result discard: if the user reloaded the PR mid-submit, the
        // active head SHA may have moved. Drop the result rather than
        // silently mutating the wrong session.
        let Some(in_flight) = in_flight else {
            return;
        };
        let stale = in_flight.repository != repository
            || in_flight.pr_number != pr_number
            || in_flight.head_sha_snapshot != head_sha;
        if stale {
            self.set_message("Discarded stale submit result (PR was reloaded)".to_string());
            return;
        }

        self.finish_pr_submit(in_flight, result);
    }

    /// Human-readable name of the forge backing the current PR/MR review.
    /// Used to keep submit messaging accurate across GitHub and GitLab.
    pub fn forge_display_name(&self) -> &'static str {
        match &self.diff_source {
            DiffSource::PullRequest(pr) => match pr.key.repository.kind {
                crate::forge::traits::ForgeKind::GitHub => "GitHub",
                crate::forge::traits::ForgeKind::GitLab => "GitLab",
            },
            _ => "forge",
        }
    }

    /// Apply the create-review result on the main thread. On success: flip
    /// each included `Comment` to `Submitted` (or `PushedDraft` for the
    /// draft event), stamp `remote_review_id`, save the session again, and
    /// publish a success message. On failure: keep everything as
    /// `LocalDraft` and set a sticky error.
    pub fn finish_pr_submit(
        &mut self,
        in_flight: SubmitInFlightState,
        result: std::result::Result<crate::forge::traits::GhCreateReviewResponse, String>,
    ) {
        use crate::forge::submit::SubmitEvent;

        let response = match result {
            Ok(r) => r,
            Err(e) => {
                self.set_error(format!("Submit failed: {e}"));
                return;
            }
        };

        self.apply_submit_success(&in_flight, &response);

        // Post-submit save — captures the lifecycle transitions.
        let _ = self.save_current_session_merging_external();

        let inline_count = in_flight.mappable.len();
        let summary_count = in_flight.moved_to_summary_count;
        let forge_name = self.forge_display_name();
        let message = match in_flight.event {
            SubmitEvent::Draft => {
                let pr_url = match &self.diff_source {
                    DiffSource::PullRequest(pr) => pr.url.clone(),
                    _ => String::new(),
                };
                if pr_url.is_empty() {
                    format!(
                        "Pushed pending {forge_name} review #{}: {} inline, {} moved to summary",
                        response.id, inline_count, summary_count,
                    )
                } else {
                    format!(
                        "Pushed pending {forge_name} review #{}: {} inline, {} moved to summary — Finish it in {forge_name}: {}",
                        response.id, inline_count, summary_count, pr_url,
                    )
                }
            }
            _ => format!(
                "Submitted {forge_name} review #{}: {} inline, {} moved to summary",
                response.id, inline_count, summary_count,
            ),
        };
        if in_flight.event != SubmitEvent::Draft {
            self.mark_pr_commits_reviewed_through(&in_flight.head_sha_snapshot);
        }
        self.set_message(message);

        // Refetch remote threads so the just-submitted comments appear immediately.
        self.refetch_pr_threads();
    }

    /// Flip every comment that was sent — inline, summary-bound, and review-
    /// level — from `LocalDraft` to `Submitted` (or `PushedDraft` for
    /// `:submit draft`) and stamp `remote_review_id`. The comments stay in
    /// the session so the user keeps seeing their work; they're pruned by
    /// `prune_locked_comments` when remote threads are next fetched.
    pub fn apply_submit_success(
        &mut self,
        in_flight: &SubmitInFlightState,
        response: &crate::forge::traits::GhCreateReviewResponse,
    ) {
        use crate::forge::submit::SubmitEvent;
        use crate::model::comment::CommentLifecycleState;

        let new_state = match in_flight.event {
            SubmitEvent::Draft => CommentLifecycleState::PushedDraft,
            _ => CommentLifecycleState::Submitted,
        };
        let review_id = response.id.to_string();

        let target_ids: std::collections::HashSet<&str> = in_flight
            .mappable
            .iter()
            .map(|c| c.comment_id.as_str())
            .chain(in_flight.summary_comment_ids.iter().map(String::as_str))
            .chain(in_flight.review_comment_ids.iter().map(String::as_str))
            .collect();
        if target_ids.is_empty() {
            return;
        }

        for comment in self.session.review_comments.iter_mut() {
            if target_ids.contains(comment.id.as_str()) {
                comment.lifecycle_state = new_state;
                comment.remote_review_id = Some(review_id.clone());
            }
        }
        for review in self.session.files.values_mut() {
            for comment in review.file_comments.iter_mut() {
                if target_ids.contains(comment.id.as_str()) {
                    comment.lifecycle_state = new_state;
                    comment.remote_review_id = Some(review_id.clone());
                }
            }
            for comments in review.line_comments.values_mut() {
                for comment in comments.iter_mut() {
                    if target_ids.contains(comment.id.as_str()) {
                        comment.lifecycle_state = new_state;
                        comment.remote_review_id = Some(review_id.clone());
                    }
                }
            }
        }
        self.rebuild_annotations();
    }

    /// Drop locked (`Submitted`/`PushedDraft`) comments from the session.
    /// Called after a successful `forge_review_threads` fetch: anything that
    /// was published to the forge is now represented by the fresh remote
    /// threads, so keeping the locals would double-render every line.
    pub fn prune_locked_comments(&mut self) {
        self.session.review_comments.retain(|c| !c.is_locked());
        for review in self.session.files.values_mut() {
            review.file_comments.retain(|c| !c.is_locked());
            for comments in review.line_comments.values_mut() {
                comments.retain(|c| !c.is_locked());
            }
            review.line_comments.retain(|_, v| !v.is_empty());
        }
    }
}
