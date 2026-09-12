//! Mirror local file-reviewed markers onto the forge's per-file viewed state.
//!
//! GitHub's "Viewed" checkbox on a pull request file is the same idea as
//! tuicr's `r`, so the two are kept in step: a toggle here ticks the checkbox
//! there, and files already ticked there open as reviewed here.
//!
//! Both directions run off the main thread. Every push is a `gh api graphql`
//! process spawn, and the diff has to stay responsive while the user walks
//! the file list marking files; the read is one more paged query on top of
//! everything a PR open already fetches.

use super::*;
use crate::forge::traits::{ForgeKind, PrSessionKey, PullRequestDetails};

impl App {
    /// Queue a viewed-state push for `path`, spawning the worker on the first
    /// toggle of a session. Does nothing when the sync is off, the review is
    /// not a GitHub pull request, or the PR details are missing.
    pub(in crate::app) fn push_viewed_state(&mut self, path: &Path, viewed: bool) {
        let Some(key) = self.viewed_sync_key().cloned() else {
            return;
        };

        // A worker is bound to the PR session it was started for; opening a
        // different PR — or the same one at a new head — replaces it.
        if self
            .viewed_sync
            .as_ref()
            .is_none_or(|worker| worker.key != key)
        {
            let Some((key, details)) = self.viewed_sync_target() else {
                return;
            };
            self.spawn_viewed_sync(key, details);
        }

        let Some(worker) = self.viewed_sync.as_ref() else {
            return;
        };
        let request = ViewedSyncRequest::Set {
            path: path.to_path_buf(),
            viewed,
        };
        if worker.tx.send(request).is_err() {
            self.viewed_sync = None;
        }
    }

    /// The PR session viewed state should sync with, if any. Borrows rather
    /// than clones: the main loop asks this on every tick.
    fn viewed_sync_key(&self) -> Option<&PrSessionKey> {
        if !self.forge_config.sync_viewed {
            return None;
        }
        let DiffSource::PullRequest(pr) = &self.diff_source else {
            return None;
        };
        // Only GitHub exposes a per-viewer file state; the other backends
        // answer `UnsupportedOperation`, so don't even start a worker.
        (pr.key.repository.kind == ForgeKind::GitHub).then_some(&pr.key)
    }

    /// That session together with the PR details a worker needs in order to
    /// talk to the forge.
    fn viewed_sync_target(&self) -> Option<(PrSessionKey, PullRequestDetails)> {
        let key = self.viewed_sync_key()?.clone();
        let details = self.pr_info.as_ref()?.details.clone();
        // `pr_info` is refreshed alongside `diff_source` on every open and
        // reload. Should the two ever disagree, this check keeps the sync off
        // a pull request the user is no longer looking at.
        (details.repository == key.repository && details.number == key.number)
            .then_some((key, details))
    }

    fn spawn_viewed_sync(&mut self, key: PrSessionKey, details: PullRequestDetails) {
        let (request_tx, request_rx) = std::sync::mpsc::channel::<ViewedSyncRequest>();
        let (event_tx, event_rx) = std::sync::mpsc::channel::<ViewedSyncEvent>();

        let local_checkout = self
            .forge_backend
            .as_deref()
            .and_then(|backend| backend.local_checkout_path());
        let show_pr_checks = self.show_pr_checks;
        let show_pr_comments = self.show_pr_comments;

        std::thread::spawn(move || {
            // The worker owns its backend: `App::forge_backend` cannot cross
            // threads, and a long-lived instance here keeps the PR node id
            // the GraphQL mutation needs cached for the whole session.
            let backend = create_forge_backend(
                &details.repository,
                local_checkout,
                show_pr_checks,
                show_pr_comments,
            );
            // Ends when the App drops the sender: session replaced, or exit.
            while let Ok(ViewedSyncRequest::Set { path, viewed }) = request_rx.recv() {
                if let Err(error) = backend.set_file_viewed(&details, &path, viewed) {
                    let failure = ViewedSyncEvent::Failed {
                        path,
                        viewed,
                        error: error.to_string(),
                    };
                    if event_tx.send(failure).is_err() {
                        return;
                    }
                }
            }
        });

        self.viewed_sync = Some(ViewedSyncWorker {
            key,
            tx: request_tx,
        });
        self.viewed_sync_rx = Some(event_rx);
    }

    /// Pump both directions of the viewed-state sync: start this session's
    /// one-time read of the forge's state, then drain whatever came back.
    /// Returns whether a repaint is needed.
    pub fn poll_viewed_sync_events(&mut self) -> bool {
        let mut needs_redraw = self.start_viewed_seed();
        needs_redraw |= self.drain_viewed_seed();
        needs_redraw |= self.drain_viewed_failures();
        needs_redraw
    }

    /// Read the forge's viewed state once per PR session.
    ///
    /// This runs from the main loop rather than from the PR-open paths
    /// because `App::forge_config` is applied *after* `App::new` returns: a
    /// read started at open time would never see `sync_viewed`.
    fn start_viewed_seed(&mut self) -> bool {
        // Cheap gate first — this is asked on every tick of the main loop.
        match self.viewed_sync_key() {
            Some(key) if self.viewed_seeded.as_ref() != Some(key) => {}
            _ => return false,
        }
        let Some((key, details)) = self.viewed_sync_target() else {
            return false;
        };
        // Claim the session before spawning: the read happens once even if
        // it fails, rather than requeueing on every tick.
        self.viewed_seeded = Some(key.clone());

        let (tx, rx) = std::sync::mpsc::channel();
        self.viewed_seed_rx = Some(rx);

        let local_checkout = self
            .forge_backend
            .as_deref()
            .and_then(|backend| backend.local_checkout_path());
        let show_pr_checks = self.show_pr_checks;
        let show_pr_comments = self.show_pr_comments;

        std::thread::spawn(move || {
            let backend = create_forge_backend(
                &details.repository,
                local_checkout,
                show_pr_checks,
                show_pr_comments,
            );
            let result = backend
                .list_viewed_files(&details)
                .map_err(|error| error.to_string());
            let _ = tx.send(ViewedSeedEvent::Done { key, result });
        });

        false
    }

    /// Apply a finished read, if one has landed.
    fn drain_viewed_seed(&mut self) -> bool {
        let Some(rx) = self.viewed_seed_rx.as_ref() else {
            return false;
        };
        let event = match rx.try_recv() {
            Ok(event) => event,
            Err(std::sync::mpsc::TryRecvError::Empty) => return false,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.viewed_seed_rx = None;
                return false;
            }
        };
        self.viewed_seed_rx = None;

        let ViewedSeedEvent::Done { key, result } = event;
        // Drop a read that landed after the user opened a different PR.
        if self.viewed_sync_key() != Some(&key) {
            return false;
        }

        match result {
            Ok(paths) => self.apply_remote_viewed_state(&paths),
            Err(error) => {
                self.set_warning(format!(
                    "GitHub: could not read viewed state \u{00b7} {error}"
                ));
                true
            }
        }
    }

    /// Mark locally the files the forge reports as already viewed.
    ///
    /// Additive on purpose. A marker the user made here is never cleared by
    /// what GitHub reports: unticking a box on the web is easy to do by
    /// accident, and silently throwing away review progress is the one
    /// direction no keystroke can undo.
    pub(in crate::app) fn apply_remote_viewed_state(&mut self, paths: &[PathBuf]) -> bool {
        let mut marked = 0;
        for path in paths {
            // Only files this review actually covers — a path filtered out by
            // `.tuicrignore` or outside the selected commit range has no
            // session entry, and `get_file_mut` does not create one.
            if let Some(review) = self.session.get_file_mut(path)
                && !review.reviewed
            {
                review.reviewed = true;
                marked += 1;
            }
        }
        if marked == 0 {
            return false;
        }

        self.dirty = true;
        // Newly reviewed files collapse, which can leave the cursor past the
        // end of the rebuilt diff.
        self.rebuild_annotations();
        self.diff_state.cursor_line = self.diff_state.cursor_line.min(self.max_cursor_line());
        self.ensure_cursor_visible();
        self.set_message(format!(
            "{marked} file(s) already viewed on GitHub \u{00b7} marked reviewed"
        ));
        true
    }

    /// Drain push failures into the status bar.
    fn drain_viewed_failures(&mut self) -> bool {
        let Some(rx) = self.viewed_sync_rx.as_ref() else {
            return false;
        };

        // Only the newest failure is shown. A dead token or a revoked scope
        // fails every queued push, and one warning says as much as twenty.
        let mut warning = None;
        loop {
            match rx.try_recv() {
                Ok(ViewedSyncEvent::Failed {
                    path,
                    viewed,
                    error,
                }) => {
                    let verb = if viewed { "mark" } else { "unmark" };
                    // The local marker stays as the user left it; `:reload`
                    // resyncs the checkbox.
                    warning = Some(format!(
                        "GitHub: could not {verb} {} as viewed \u{00b7} {error}",
                        path.display()
                    ));
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.viewed_sync_rx = None;
                    self.viewed_sync = None;
                    break;
                }
            }
        }

        match warning {
            Some(text) => {
                self.set_warning(text);
                true
            }
            None => false,
        }
    }
}
