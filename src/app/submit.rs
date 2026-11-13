use super::*;
use crate::forge::traits::SubmitReceipts;

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
            CommentAnchor, InlineComment, ResolverAction, SubmitContext, UnmappableItem,
            UnmappableReason, map_comment,
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

        // A strict subset's old side is the parent of its oldest commit.
        let base_commit_id = match (self.commit_selection_range, self.pr_range_sha_pair()) {
            (Some((start_idx, end_idx)), Some((parent, _)))
                if !(start_idx == 0 && end_idx + 1 == self.pr_commits.len()) =>
            {
                Some(parent)
            }
            _ => None,
        };

        // Map against the displayed diff: a narrowed selection's anchors live
        // there, and `commit_id` above is that selection's head.
        // `range_diff_files` is only the full-range cache, so it is not used.
        let files: Vec<&DiffFile> = self.diff_files.iter().collect();

        let submit_ctx = SubmitContext::new(&self.forge_config, &self.comment_types);

        let mut mappable: Vec<InlineComment> = Vec::new();
        let mut unmappable: Vec<UnmappableItem> = Vec::new();
        let mut total_local_drafts = 0_usize;

        // Walk file-level and line comments in display order. Review-level
        // comments (session.review_comments) are NOT inline-mapped; they
        // appear in the body via `build_review_body`.
        for file in &files {
            if file.is_commit_message {
                continue;
            }
            let Some(review) = self.session.files.get(file.display_path()) else {
                continue;
            };
            for comment in &review.file_comments {
                if comment.is_locked() || !self.comment_visible(comment) {
                    continue;
                }
                total_local_drafts += 1;
                bucket_mapping(
                    map_comment(comment, CommentAnchor::FileLevel, file, submit_ctx),
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
                        map_comment(comment, anchor, file, submit_ctx),
                        &mut mappable,
                        &mut unmappable,
                    );
                }
            }
        }

        // Comments on a commit message are never posted inline: the message
        // is not a file of the PR diff, and most forges have nowhere to
        // anchor one. The resolver can move them into the review summary.
        for commit in self.pr_commits.iter().rev() {
            let path = DiffFile::commit_message_path(&commit.short_oid);
            let Some(review) = self.session.files.get(&path) else {
                continue;
            };
            let mut keys: Vec<&u32> = review.line_comments.keys().collect();
            keys.sort();
            for comment in review
                .file_comments
                .iter()
                .chain(keys.into_iter().flat_map(|key| &review.line_comments[key]))
            {
                if comment.is_locked() || !self.comment_visible(comment) {
                    continue;
                }
                total_local_drafts += 1;
                unmappable.push(UnmappableItem {
                    comment: comment.clone(),
                    file: path.clone(),
                    reason: UnmappableReason::CommitMessage,
                });
            }
        }

        // Approve is the one event that's meaningful with no comments — a
        // bare "LGTM" approval. Every other event needs at least one local
        // draft comment or a review-level comment, otherwise there's
        // nothing to submit.
        // A request-changes vote can also go out bare on forges that cast it
        // as its own call after the comments: a partial submit locks the
        // comments that landed, and the retry must still be able to vote.
        // GitHub rejects an empty request-changes review, so it is excluded.
        let bare_allowed = match event {
            crate::forge::submit::SubmitEvent::Approve => true,
            crate::forge::submit::SubmitEvent::RequestChanges => {
                self.forge_kind().is_some_and(|kind| {
                    use crate::forge::traits::ForgeKind;
                    matches!(
                        kind,
                        ForgeKind::GitLab | ForgeKind::Bitbucket | ForgeKind::AzureDevOps
                    )
                })
            }
            _ => false,
        };
        if !bare_allowed
            && total_local_drafts == 0
            && !self.session.review_comments.iter().any(|c| !c.is_locked())
        {
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
            base_commit_id,
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
        use crate::forge::submit::{MovedToSummaryItem, ResolverAction};
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
        let (body, review_comment_ids) = self.build_submit_body(&summary_items);

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
            pr_head_sha: pr.key.head_sha.clone(),
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
        let base_commit_id = state.base_commit_id.clone();
        let show_pr_checks = self.show_pr_checks;
        let show_pr_comments = self.show_pr_comments;

        std::thread::spawn(move || {
            let backend = create_forge_backend(
                &repository,
                local_checkout,
                show_pr_checks,
                show_pr_comments,
            );
            // Need PR details for repo/owner routing; refetch lightly via
            // the same target the user opened with.
            let target = PullRequestTarget::with_repository(
                repository.clone(),
                pr_number,
                pr_number.to_string(),
            );
            let (result, partial) = match backend.get_pull_request(target) {
                Ok(details) => match backend.create_review(
                    &details,
                    CreateReviewRequest {
                        event,
                        commit_id: &commit_id,
                        base_commit_id: base_commit_id.as_deref(),
                        body: &body,
                        comments: &mappable,
                    },
                ) {
                    Ok(response) => (Ok(response), SubmitReceipts::default()),
                    Err(e) => {
                        let (message, receipts) = split_partial_submit(e);
                        (Err(message), receipts)
                    }
                },
                Err(e) => (Err(e.to_string()), SubmitReceipts::default()),
            };
            let _ = tx.send(PrSubmitEvent::Done {
                repository,
                pr_number,
                head_sha,
                result,
                partial,
            });
        });
        Ok(())
    }

    /// The review body and the ids of the review-level comments it carries.
    /// Locked comments already went out in an earlier (partial) submit, so
    /// they are neither repeated in the body nor re-marked.
    pub(super) fn build_submit_body(
        &self,
        summary_items: &[crate::forge::submit::MovedToSummaryItem],
    ) -> (String, Vec<String>) {
        let unlocked: Vec<_> = self
            .session
            .review_comments
            .iter()
            .filter(|c| !c.is_locked())
            .cloned()
            .collect();
        let ids = unlocked.iter().map(|c| c.id.clone()).collect();
        let body = crate::forge::submit::build_review_body(
            &unlocked,
            summary_items,
            crate::forge::submit::SubmitContext::new(&self.forge_config, &self.comment_types),
        );
        (body, ids)
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
            result, partial, ..
        } = event;

        // The review is already posted, so a result for a PR that is no
        // longer open must still reach its own session: otherwise those
        // drafts stay `LocalDraft` and get submitted a second time.
        let Some(in_flight) = in_flight else {
            return;
        };
        if !self.pr_result_is_current(
            &in_flight.repository,
            in_flight.pr_number,
            &in_flight.pr_head_sha,
        ) {
            match result {
                Ok(response) => self.persist_submit_receipt_to_origin(&in_flight, &response),
                // Comments that posted before the failure belong to their own
                // PR's session too, or a retry there would post them again.
                Err(_) => {
                    if let Some(posted) = Self::partially_posted(&in_flight, &partial) {
                        self.persist_submit_receipt_to_origin(&posted, &Self::partial_response());
                    }
                }
            }
            self.set_message("Discarded stale submit result (PR was reloaded)".to_string());
            return;
        }

        if result.is_err() {
            self.lock_partially_submitted(&in_flight, &partial);
        }
        self.finish_pr_submit(in_flight, result);
    }

    /// A submit that failed midway left some comments live on the forge.
    /// Lock those so a retry does not post them again; the rest stay editable
    /// local drafts.
    fn lock_partially_submitted(
        &mut self,
        in_flight: &SubmitInFlightState,
        partial: &SubmitReceipts,
    ) {
        let Some(posted) = Self::partially_posted(in_flight, partial) else {
            return;
        };
        self.apply_submit_success(&posted, &Self::partial_response());
        let _ = self.save_current_session_merging_external();
    }

    /// The part of `in_flight` that the receipts say went out, if any.
    fn partially_posted(
        in_flight: &SubmitInFlightState,
        partial: &SubmitReceipts,
    ) -> Option<SubmitInFlightState> {
        let mut posted = SubmitInFlightState {
            mappable: in_flight
                .mappable
                .iter()
                .filter(|c| partial.comment_ids.contains(&c.comment_id))
                .cloned()
                .collect(),
            summary_comment_ids: Vec::new(),
            review_comment_ids: Vec::new(),
            ..in_flight.clone()
        };
        if partial.body_posted {
            posted.summary_comment_ids = in_flight.summary_comment_ids.clone();
            posted.review_comment_ids = in_flight.review_comment_ids.clone();
        }
        (!posted.mappable.is_empty() || partial.body_posted).then_some(posted)
    }

    /// No review id exists for these forges' partial posts.
    fn partial_response() -> crate::forge::traits::GhCreateReviewResponse {
        crate::forge::traits::GhCreateReviewResponse {
            id: 0,
            html_url: String::new(),
            state: String::new(),
        }
    }

    /// Human-readable name of the forge backing the current PR/MR review.
    /// Used to keep submit messaging accurate across GitHub, GitLab, Bitbucket,
    /// and Azure DevOps.
    pub fn forge_display_name(&self) -> &'static str {
        match &self.diff_source {
            DiffSource::PullRequest(pr) => pr.key.repository.kind.display_name(),
            _ => "forge",
        }
    }

    /// Return the forge backing the active PR/MR diff, when reviewing one.
    pub fn forge_kind(&self) -> Option<crate::forge::traits::ForgeKind> {
        match &self.diff_source {
            DiffSource::PullRequest(pr) => Some(pr.key.repository.kind),
            _ => None,
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
        Self::apply_submit_lifecycle(&mut self.session, in_flight, response);
        self.rebuild_annotations();
    }

    /// Write the submit receipt into the session of the PR it was posted to,
    /// when that is not the PR now on screen. Best-effort: a missing or
    /// unreadable session leaves the drafts as they were.
    fn persist_submit_receipt_to_origin(
        &self,
        in_flight: &SubmitInFlightState,
        response: &crate::forge::traits::GhCreateReviewResponse,
    ) {
        let key = crate::forge::traits::PrSessionKey::new(
            in_flight.repository.clone(),
            in_flight.pr_number,
            in_flight.pr_head_sha.clone(),
        );
        let Ok(Some((_path, origin))) = crate::persistence::storage::load_pr_session(&key) else {
            return;
        };
        let _ = crate::persistence::storage::save_session_by_identity(&origin.clone(), |latest| {
            let mut session = latest.unwrap_or(origin);
            Self::apply_submit_lifecycle(&mut session, in_flight, response);
            session.updated_at = chrono::Utc::now();
            Ok((session, ()))
        });
    }

    fn apply_submit_lifecycle(
        session: &mut crate::model::ReviewSession,
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

        for comment in session.review_comments.iter_mut() {
            if target_ids.contains(comment.id.as_str()) {
                comment.lifecycle_state = new_state;
                comment.remote_review_id = Some(review_id.clone());
            }
        }
        for review in session.files.values_mut() {
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

/// Separate a `create_review` error into the user-facing message and what was
/// already posted (nothing, for a plain error).
pub(super) fn split_partial_submit(err: TuicrError) -> (String, SubmitReceipts) {
    match err {
        TuicrError::PartialSubmit { message, receipts } => (message, receipts),
        e => (e.to_string(), SubmitReceipts::default()),
    }
}
