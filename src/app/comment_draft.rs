use crate::comment_draft::{self, DraftContext};

use super::*;

impl App {
    /// Queues the comment draft for the external editor.
    ///
    /// The rendered buffer is left on `pending_comment_draft` for the main
    /// event loop, which owns the terminal handoff, exactly like
    /// `pending_editor_target`.
    pub fn queue_comment_draft_editor(&mut self) {
        if self.input_mode != InputMode::Comment {
            return;
        }
        let context = self.comment_draft_context();
        self.pending_comment_draft =
            Some(comment_draft::render_draft(&self.comment_buffer, &context));
    }

    /// Takes the queued draft buffer after action dispatch.
    pub fn take_pending_comment_draft(&mut self) -> Option<String> {
        self.pending_comment_draft.take()
    }

    /// Adopts the comment body the editor wrote back.
    ///
    /// An empty body leaves the draft alone: an editor that was quit without
    /// saving, or whose buffer was emptied, should not silently discard what
    /// the user had already typed in the comment box.
    pub fn apply_comment_draft(&mut self, edited: &str) {
        if self.input_mode != InputMode::Comment {
            return;
        }
        let body = comment_draft::parse_draft(edited);
        if body.is_empty() {
            self.set_warning("Draft unchanged: the editor returned an empty comment");
            return;
        }
        if body == self.comment_buffer {
            self.set_message("Draft unchanged");
            return;
        }
        self.comment_buffer = body;
        self.comment_cursor = self.comment_buffer.len();
        // The vim overlay holds its own copy of the text; drop it so the next
        // key reseeds it from what the editor returned.
        self.comment_vim_editor = None;
        self.comment_vim_command = None;
        self.comment_vim_pending = CommentVimPending::None;
        self.set_message("Comment draft updated from editor");
    }

    /// Describes what the draft is attached to, for the context block below
    /// the scissors line.
    fn comment_draft_context(&self) -> DraftContext {
        if self.comment_is_review_level {
            return DraftContext::targeting("the whole review");
        }
        let Some(path) = self.current_file_path().cloned() else {
            return DraftContext::default();
        };
        let target = self.comment_line_range.or_else(|| {
            self.comment_line
                .map(|(line, side)| (LineRange::single(line), side))
        });
        let Some((range, side)) = target.filter(|_| !self.comment_is_file_level) else {
            return DraftContext::targeting(format!("{} (whole file)", path.display()));
        };

        let side_label = match side {
            LineSide::Old => "old",
            LineSide::New => "new",
        };
        let target = if range.is_single() {
            format!("{}:{} ({side_label})", path.display(), range.end)
        } else {
            format!(
                "{}:{}-{} ({side_label})",
                path.display(),
                range.start,
                range.end
            )
        };
        DraftContext {
            target,
            lines: self.comment_draft_context_lines(range, side),
        }
    }

    /// Diff rows around the commented lines, taken from the hunk that holds
    /// them. Lines with no matching hunk (a comment carried over from an
    /// earlier diff, say) simply get no context.
    fn comment_draft_context_lines(&self, range: LineRange, side: LineSide) -> Vec<String> {
        self.current_file()
            .into_iter()
            .flat_map(|file| file.hunks.iter())
            .map(|hunk| comment_draft::context_lines(hunk, range, side))
            .find(|lines| !lines.is_empty())
            .unwrap_or_default()
    }
}
