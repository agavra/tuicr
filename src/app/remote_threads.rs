//! Mutating remote discussion threads from the diff view.
//!
//! Every entry point resolves its target from the cursor, so the caller only
//! has to name the operation. Forge calls run synchronously - each is a single
//! request, and the thread list is refetched afterwards so the view reflects
//! what the forge now holds rather than a locally patched guess.

use crate::app::{AnnotatedLine, App, DiffSource, InputMode, RemoteThreadEdit};
use crate::forge::remote_comments::RemoteReviewThread;
use crate::forge::traits::{ForgeBackend, PullRequestDetails};

impl App {
    /// Resolve or unresolve the thread under the cursor.
    pub fn set_thread_resolved_at_cursor(&mut self, resolved: bool) {
        let Some((thread_idx, _)) = self.remote_thread_at_cursor() else {
            self.set_warning("Place the cursor on a remote thread first");
            return;
        };
        let thread_id = self.forge_review_threads[thread_idx].id.clone();
        let verb = if resolved { "Resolved" } else { "Unresolved" };
        self.run_thread_mutation(verb, |backend, pr| {
            backend.set_thread_resolved(pr, &thread_id, resolved)
        });
    }

    /// Open the comment editor to append a reply to the thread under the cursor.
    pub fn reply_to_thread_at_cursor(&mut self) {
        let Some((thread_idx, _)) = self.remote_thread_at_cursor() else {
            self.set_warning("Place the cursor on a remote thread first");
            return;
        };
        let thread_id = self.forge_review_threads[thread_idx].id.clone();
        self.remote_thread_edit = Some(RemoteThreadEdit {
            thread_id,
            comment_id: None,
        });
        self.open_editor_for_remote_thread(String::new());
        self.set_message("Reply to thread — Ctrl-S to post, Esc to cancel");
    }

    /// Open the comment editor on your own note under the cursor.
    pub fn edit_remote_note_at_cursor(&mut self) -> bool {
        let Some((thread_idx, note_idx)) = self.own_remote_note_at_cursor() else {
            return false;
        };
        let thread = &self.forge_review_threads[thread_idx];
        let body = thread.comments[note_idx].body.clone();
        self.remote_thread_edit = Some(RemoteThreadEdit {
            thread_id: thread.id.clone(),
            comment_id: Some(thread.comments[note_idx].id.clone()),
        });
        self.open_editor_for_remote_thread(body);
        self.set_message("Editing remote comment — Ctrl-S to save, Esc to cancel");
        true
    }

    /// Delete your own note under the cursor.
    pub fn delete_remote_note_at_cursor(&mut self) -> bool {
        let Some((thread_idx, note_idx)) = self.own_remote_note_at_cursor() else {
            return false;
        };
        let thread = &self.forge_review_threads[thread_idx];
        let thread_id = thread.id.clone();
        let comment_id = thread.comments[note_idx].id.clone();
        self.run_thread_mutation("Deleted remote comment", |backend, pr| {
            backend.delete_thread_comment(pr, &thread_id, &comment_id)
        });
        true
    }

    /// Apply the editor buffer to the pending reply or edit. Returns false when
    /// no remote write is pending, leaving the local-draft path to the caller.
    pub fn save_remote_thread_edit(&mut self) -> bool {
        let Some(pending) = self.remote_thread_edit.clone() else {
            return false;
        };
        let body = self.comment_buffer.trim().to_string();
        if body.is_empty() {
            self.set_message("Comment cannot be empty");
            return true;
        }

        let label = match &pending.comment_id {
            Some(_) => "Updated remote comment",
            None => "Replied to thread",
        };
        self.run_thread_mutation(label, |backend, pr| match &pending.comment_id {
            Some(comment_id) => {
                backend.edit_thread_comment(pr, &pending.thread_id, comment_id, &body)
            }
            None => backend.reply_to_thread(pr, &pending.thread_id, &body),
        });
        self.exit_comment_mode();
        true
    }

    /// Whether the active backend can write to remote threads at all.
    pub fn supports_remote_thread_mutations(&self) -> bool {
        self.forge_backend
            .as_deref()
            .is_some_and(|backend| backend.supports_thread_mutations())
    }

    /// Thread and note under the cursor, when both resolve and the note is
    /// yours. Warns about the reason it did not, so callers can fall through to
    /// their local-comment behavior only when the cursor is elsewhere.
    fn own_remote_note_at_cursor(&mut self) -> Option<(usize, usize)> {
        if !self.supports_remote_thread_mutations() {
            self.set_message(format!(
                "{} comment — read only in tuicr",
                self.forge_display_name()
            ));
            return None;
        }
        let (thread_idx, row) = self.remote_thread_at_cursor()?;
        let thread = &self.forge_review_threads[thread_idx];
        let Some(note_idx) = note_index_for_row(thread, row) else {
            self.set_warning("Place the cursor on a comment inside the thread");
            return None;
        };
        let author = thread.comments[note_idx].author.as_deref();
        if !self.is_own_remote_note(author) {
            let author = author.unwrap_or("someone else");
            self.set_warning(format!("Cannot modify @{author}'s comment"));
            return None;
        }
        Some((thread_idx, note_idx))
    }

    /// Index of the thread under the cursor plus the cursor's row offset within
    /// the thread's rendered block.
    fn remote_thread_at_cursor(&self) -> Option<(usize, usize)> {
        let cursor = self.diff_state.cursor_line;
        let &AnnotatedLine::RemoteThreadLine { thread_idx } = self.line_annotations.get(cursor)?
        else {
            return None;
        };
        self.forge_review_threads.get(thread_idx)?;
        let row = self.line_annotations[..cursor]
            .iter()
            .rev()
            .take_while(|candidate| {
                matches!(
                    candidate,
                    AnnotatedLine::RemoteThreadLine { thread_idx: prev } if *prev == thread_idx
                )
            })
            .count();
        Some((thread_idx, row))
    }

    fn is_own_remote_note(&self, author: Option<&str>) -> bool {
        let Some(author) = author else {
            return false;
        };
        let Some(viewer) = self.pr_viewer_login.as_deref() else {
            return false;
        };
        author.eq_ignore_ascii_case(viewer)
    }

    fn open_editor_for_remote_thread(&mut self, body: String) {
        self.input_mode = InputMode::Comment;
        self.diff_state.scroll_x = 0;
        self.comment_cursor = body.len();
        self.comment_buffer = body;
        self.comment_type = self.default_comment_type();
        self.comment_is_review_level = false;
        self.comment_is_file_level = false;
        self.comment_line = None;
        self.comment_line_range = None;
        self.editing_comment_id = None;
    }

    /// Run `call` against the active forge backend, then re-read the thread list
    /// from that same backend so the view shows what the forge now holds. Both
    /// calls are synchronous: each is one request, and the reread has to observe
    /// the write, which an out-of-band background fetch could race.
    fn run_thread_mutation(
        &mut self,
        success: &str,
        call: impl FnOnce(&dyn ForgeBackend, &PullRequestDetails) -> crate::error::Result<()>,
    ) {
        let Some(pr) = self.pr_details_for_mutation() else {
            self.set_warning("Remote threads can only be changed in PR mode");
            return;
        };
        let Some(backend) = self.forge_backend.as_deref() else {
            self.set_warning("No forge backend for this review");
            return;
        };
        if let Err(err) = call(backend, &pr) {
            self.set_error(format!("{err}"));
            return;
        }
        // The write landed, so a failed reread is a stale view rather than a
        // failed operation - say so instead of reporting the write as failed.
        match backend.list_review_threads(&pr) {
            Ok(threads) => {
                self.forge_review_threads = threads;
                self.rebuild_annotations();
                self.set_message(success.to_string());
            }
            Err(err) => self.set_warning(format!("{success}, but reloading threads failed: {err}")),
        }
    }

    /// Rebuild the `PullRequestDetails` the backend needs from the open PR.
    /// Only the fields the discussion endpoints read are meaningful here.
    fn pr_details_for_mutation(&self) -> Option<PullRequestDetails> {
        let DiffSource::PullRequest(pr) = &self.diff_source else {
            return None;
        };
        Some(PullRequestDetails {
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
        })
    }
}

/// Which comment in `thread` occupies `row` of its rendered block. Mirrors the
/// layout in `comment_panel::format_remote_thread_lines`: one header row per
/// comment, then its body rows, and a single footer row closing the thread.
/// The footer belongs to no comment.
fn note_index_for_row(thread: &RemoteReviewThread, row: usize) -> Option<usize> {
    let mut offset = 0;
    for (idx, comment) in thread.comments.iter().enumerate() {
        let block = 1 + comment.body.split('\n').count();
        if row < offset + block {
            return Some(idx);
        }
        offset += block;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::note_index_for_row;
    use crate::forge::remote_comments::{
        RemoteCommentSide, RemoteReviewComment, RemoteReviewThread,
    };

    fn comment(id: &str, body: &str) -> RemoteReviewComment {
        RemoteReviewComment {
            id: id.to_string(),
            author: Some("alice".to_string()),
            body: body.to_string(),
            created_at: None,
            in_reply_to: None,
            url: String::new(),
        }
    }

    fn thread(comments: Vec<RemoteReviewComment>) -> RemoteReviewThread {
        RemoteReviewThread {
            id: "t1".to_string(),
            path: "src/lib.rs".to_string(),
            line: Some(10),
            side: RemoteCommentSide::Right,
            is_resolved: false,
            is_outdated: false,
            comments,
        }
    }

    #[test]
    fn should_map_rows_to_the_comment_they_render() {
        // given a root with a two-line body and one single-line reply:
        // rows 0-2 are the root (header + 2 body), rows 3-4 the reply.
        let thread = thread(vec![
            comment("root", "first\nsecond"),
            comment("reply", "ok"),
        ]);
        // when/then
        assert_eq!(note_index_for_row(&thread, 0), Some(0));
        assert_eq!(note_index_for_row(&thread, 2), Some(0));
        assert_eq!(note_index_for_row(&thread, 3), Some(1));
        assert_eq!(note_index_for_row(&thread, 4), Some(1));
    }

    #[test]
    fn should_map_footer_row_to_no_comment() {
        // given a thread whose only comment occupies rows 0-1
        let thread = thread(vec![comment("root", "body")]);
        // when/then — row 2 is the closing rule
        assert_eq!(note_index_for_row(&thread, 2), None);
    }
}
