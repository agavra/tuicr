//! Mirror local file-reviewed markers onto the forge's per-file viewed state.
//!
//! GitHub's "Viewed" checkbox on a pull request file is the same idea as
//! tuicr's `r`, so a toggle here can tick it there. The push runs on a worker
//! thread: every update is a `gh api graphql` process spawn, and the diff has
//! to stay responsive while the user walks the file list marking files.

use super::*;
use crate::forge::traits::{ForgeKind, PrSessionKey, PullRequestDetails};

impl App {
    /// Queue a viewed-state push for `path`, spawning the worker on the first
    /// toggle of a session. Does nothing when the sync is off, the review is
    /// not a GitHub pull request, or the PR details are missing.
    pub(in crate::app) fn push_viewed_state(&mut self, path: &Path, viewed: bool) {
        let Some((key, details)) = self.viewed_sync_target() else {
            return;
        };

        // A worker is bound to the PR session it was started for; opening a
        // different PR — or the same one at a new head — replaces it.
        if self
            .viewed_sync
            .as_ref()
            .is_none_or(|worker| worker.key != key)
        {
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
            // The worker is gone; its last failure was already reported.
            self.viewed_sync = None;
        }
    }

    /// The PR this session should push viewed state to, if any.
    fn viewed_sync_target(&self) -> Option<(PrSessionKey, PullRequestDetails)> {
        if !self.forge_config.sync_viewed {
            return None;
        }
        let DiffSource::PullRequest(pr) = &self.diff_source else {
            return None;
        };
        // Only GitHub exposes a per-viewer file state; the other backends
        // return `UnsupportedOperation`, so don't even start a worker.
        if pr.key.repository.kind != ForgeKind::GitHub {
            return None;
        }
        let details = self.pr_info.as_ref()?.details.clone();
        // `pr_info` is refreshed alongside `diff_source` on every open and
        // reload. Should the two ever disagree, this check keeps the push
        // from landing on a pull request the user is no longer looking at.
        (details.repository == pr.key.repository && details.number == pr.key.number)
            .then(|| (pr.key.clone(), details))
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

    /// Drain viewed-state failures into the status bar. Returns whether a
    /// repaint is needed.
    pub fn poll_viewed_sync_events(&mut self) -> bool {
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
                    // The local marker stays as the user left it: losing a
                    // toggle under the cursor is worse than a stale checkbox
                    // on GitHub, which `:reload` will resync anyway.
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
