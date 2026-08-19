use crate::editor::{EditorError, LaunchState};
use crate::forge::local_git::read_blob;
use crate::forge::traits::ForgeFileLinesRequest;

use super::editor_target::{materialize, short_sha, snapshot_path, snapshot_root};
use super::*;

/// The parts of a `DiffFile` that editor-target resolution needs.
///
/// Copied out before resolving so the `&mut self` message setters stay usable
/// with `self.diff_files` no longer borrowed.
struct ReviewedFile {
    display_path: PathBuf,
    old_path: Option<PathBuf>,
    new_path: Option<PathBuf>,
    status: FileStatus,
    is_binary: bool,
}

impl App {
    pub fn can_stage(&self) -> bool {
        matches!(
            self.diff_source,
            DiffSource::Unstaged | DiffSource::StagedAndUnstaged
        )
    }

    pub fn stage_reviewed_files(&mut self) {
        if !self.can_stage() {
            self.set_error("Staging only available when viewing unstaged diffs");
            return;
        }
        let reviewed_paths: Vec<_> = self
            .session
            .files
            .iter()
            .filter(|(_, review)| review.reviewed)
            .map(|(path, _)| path.clone())
            .collect();
        if reviewed_paths.is_empty() {
            self.set_warning("No reviewed files to stage");
            return;
        }
        let mut staged = 0;
        for path in &reviewed_paths {
            if let Err(e) = self.vcs.stage_file(path) {
                self.set_error(format!("Failed to stage {}: {e}", path.display()));
                return;
            }
            staged += 1;
        }
        self.set_message(format!("Staged {} reviewed file(s)", staged));
        if let Err(TuicrError::NoChanges) = self.reload_diff_files() {
            self.diff_files.clear();
            self.diff_state = DiffState::default();
            self.file_list_state = FileListState::default();
            self.clear_expanded_gaps();
            self.rebuild_annotations();
        }
    }

    pub fn current_file(&self) -> Option<&DiffFile> {
        self.diff_files.get(self.diff_state.current_file_idx)
    }

    pub fn current_file_path(&self) -> Option<&PathBuf> {
        if self.is_cursor_in_overview() {
            return None;
        }
        self.current_file().map(|f| f.display_path())
    }

    /// Takes the queued editor target after action dispatch.
    ///
    /// The main event loop consumes this after leaving raw mode and the
    /// alternate screen,
    /// because `App` does not own terminal state.
    pub fn take_pending_editor_target(&mut self) -> Option<EditorTarget> {
        self.pending_editor_target.take()
    }

    /// Tracks a launched windowed editor so it gets cleaned up once it exits
    /// and a failed launch gets reported.
    pub fn track_editor_launch(&mut self, launch: EditorLaunch) {
        self.editor_launches.push(launch);
    }

    /// Cleans up exited windowed editors and reports the ones that never
    /// started.
    ///
    /// Returns whether a message was set.
    pub fn poll_editor_launches(&mut self) -> bool {
        let mut failures = Vec::new();
        self.editor_launches
            .retain_mut(|launch| match launch.poll() {
                LaunchState::Running => true,
                LaunchState::Exited => false,
                LaunchState::FailedToLaunch(status) => {
                    failures.push(status);
                    false
                }
            });
        let reported = !failures.is_empty();
        for status in failures {
            self.set_error(EditorError::Exit(status).to_string());
        }
        reported
    }

    /// Resolves the currently focused UI item into an editor target.
    ///
    /// The resolved target is queued on `pending_editor_target` so the main
    /// event loop can perform the terminal handoff.
    /// Invalid focus states are reported through the status bar instead.
    pub fn queue_editor_for_focused_item(&mut self) {
        match self.focused_panel {
            FocusedPanel::FileList => match self.get_selected_tree_item() {
                Some(FileTreeItem::File { file_idx, .. }) => {
                    self.queue_editor_for_file_idx(file_idx, None)
                }
                Some(FileTreeItem::Directory { .. }) => {
                    self.set_warning("Select a file to open in editor");
                }
                None => self.set_warning("No file selected"),
            },
            FocusedPanel::Diff => {
                let annotation = self.line_annotations.get(self.diff_state.cursor_line);
                let file_idx = match annotation {
                    Some(
                        AnnotatedLine::Expander { gap_id, .. }
                        | AnnotatedLine::HiddenLines { gap_id, .. }
                        | AnnotatedLine::ExpandedContext { gap_id, .. },
                    ) => gap_id.file_idx,
                    Some(annotation) => {
                        annotation_file_idx(annotation).unwrap_or(self.diff_state.current_file_idx)
                    }
                    None => self.diff_state.current_file_idx,
                };
                let line = match annotation {
                    Some(AnnotatedLine::ExpandedContext { gap_id, line_idx }) => self
                        .get_expanded_line(gap_id, *line_idx)
                        .and_then(|line| line.new_lineno.or(line.old_lineno)),
                    _ => self.get_line_at_cursor().map(|(line, _side)| line),
                };
                self.queue_editor_for_file_idx(file_idx, line);
            }
            FocusedPanel::Comments | FocusedPanel::CommitSelector => {
                self.set_warning("Focus a file or diff line to open in editor");
            }
        }
    }

    fn queue_editor_for_file_idx(&mut self, file_idx: usize, line: Option<u32>) {
        let Some(file) = self.diff_files.get(file_idx) else {
            self.set_warning("No file selected");
            return;
        };
        if file.is_commit_message {
            self.set_warning("Commit message has no local file to open");
            return;
        }

        let reviewed_file = ReviewedFile {
            display_path: file.display_path().clone(),
            old_path: file.old_path.clone(),
            new_path: file.new_path.clone(),
            status: file.status,
            is_binary: file.is_binary,
        };
        // PR mode's root_path is the synthetic forge:host/owner/repo
        // identity, never a real directory.
        let checkout = if self.vcs_info.root_path.is_absolute() {
            Some(self.vcs_info.root_path.clone())
        } else {
            self.forge_backend
                .as_deref()
                .and_then(|backend| backend.local_checkout_path())
        };

        // A PR diff describes the PR's revision, which the checkout may not
        // hold — resolving against the working tree would open the wrong text,
        // at a line number computed for other content.
        if let DiffSource::PullRequest(pr) = self.diff_source.clone() {
            match self.pr_editor_target(&pr, &reviewed_file, checkout.as_deref(), line) {
                Ok(target) => self.pending_editor_target = Some(target),
                Err(warning) => self.set_warning(warning),
            }
            return;
        }

        let Some(root) = checkout else {
            self.set_warning(format!(
                "Cannot open {}: no local checkout",
                reviewed_file.display_path.display()
            ));
            return;
        };

        let path = root.join(&reviewed_file.display_path);
        // Deleted files have diff rows, but no worktree file the external
        // editor can open.
        if !path.exists() {
            self.set_warning(format!(
                "Cannot open {}: file does not exist",
                path.display()
            ));
            return;
        }

        self.pending_editor_target = Some(EditorTarget {
            label: reviewed_file.display_path.display().to_string(),
            path,
            line,
        });
    }

    /// Resolve a PR file to something `$EDITOR` can open at the revision under
    /// review.
    ///
    /// The worktree copy wins when it *is* that revision, because it is the only
    /// file the user can edit. Otherwise the reviewed content goes to a
    /// read-only snapshot, so the text and the line the cursor sits on match the
    /// diff. `Err` carries the status-bar warning: nothing here is worth
    /// stopping the review for.
    fn pr_editor_target(
        &self,
        pr: &PullRequestDiffSource,
        file: &ReviewedFile,
        checkout: Option<&Path>,
        line: Option<u32>,
    ) -> std::result::Result<EditorTarget, String> {
        let display_path = file.display_path.display().to_string();
        let side = ForgeFileLinesRequest::side_for_status(file.status);
        let Some(revision_path) = ForgeFileLinesRequest::path_for_side(
            side,
            file.old_path.as_ref(),
            file.new_path.as_ref(),
        ) else {
            return Err(format!("Cannot open {display_path}: the diff has no path"));
        };
        let request = ForgeFileLinesRequest {
            repository: pr.key.repository.clone(),
            base_sha: pr.base_sha.clone(),
            head_sha: pr.key.head_sha.clone(),
            path: revision_path.clone(),
            status: file.status,
            side,
            start_line: 0,
            end_line: 0,
        };
        let sha = request.sha().to_string();
        let short = short_sha(&sha).to_string();
        let worktree = checkout.map(|root| root.join(&revision_path));

        // Blob reads and forge responses are lossy UTF-8, so snapshotting a
        // binary file would corrupt it; the worktree copy is the only one safe
        // to hand over.
        if file.is_binary {
            return match worktree.filter(|path| path.exists()) {
                Some(path) => Ok(EditorTarget {
                    path,
                    line,
                    label: display_path,
                }),
                None => Err(format!(
                    "Cannot open {display_path}: binary file is not in the local checkout"
                )),
            };
        }

        let content = match checkout.and_then(|root| read_blob(root, &sha, &revision_path)) {
            Some(content) => content,
            None => match self.forge_backend.as_deref() {
                Some(backend) => backend
                    .fetch_file_content(request)
                    .map_err(|err| format!("Cannot read {display_path} at {short}: {err}"))?,
                None => {
                    return Err(format!(
                        "Cannot open {display_path}: no local checkout and no forge backend"
                    ));
                }
            },
        };

        if let Some(path) = worktree
            && std::fs::read_to_string(&path).is_ok_and(|on_disk| on_disk == content)
        {
            return Ok(EditorTarget {
                path,
                line,
                label: display_path,
            });
        }

        let root = snapshot_root(&pr.key);
        let Some(path) = snapshot_path(&root, &revision_path) else {
            return Err(format!(
                "Cannot open {display_path}: the path escapes the snapshot directory"
            ));
        };
        materialize(&path, &content)
            .map_err(|err| format!("Cannot write a {short} copy of {display_path}: {err}"))?;
        Ok(EditorTarget {
            path,
            line,
            label: format!("{display_path} @ {short} (read-only PR copy)"),
        })
    }

    pub fn toggle_reviewed(&mut self) {
        let file_idx = self.diff_state.current_file_idx;
        self.toggle_reviewed_for_file_idx(file_idx, true);
    }

    pub fn toggle_reviewed_for_file_idx(&mut self, file_idx: usize, adjust_cursor: bool) {
        let Some(path) = self
            .diff_files
            .get(file_idx)
            .map(|file| file.display_path().clone())
        else {
            return;
        };

        self.revealed_reviewed_file = None;
        if let Some(review) = self.session.get_file_mut(&path) {
            review.reviewed = !review.reviewed;
            self.dirty = true;

            // Update current_file_idx before rebuilding annotations:
            // single-file view filters annotations against it.
            if adjust_cursor {
                self.diff_state.current_file_idx = file_idx;
            }
            self.rebuild_annotations();

            // With reviewed files hidden, the row just marked is gone: there is
            // no header line left to park on, and the tree selection would be
            // pointing at whatever shifted up into its place. Move to the next
            // file still in the queue instead. Runs regardless of
            // `adjust_cursor`, because the tree selection has to move either
            // way once the row it names disappears.
            if !self.file_idx_passes_filter(file_idx) {
                self.advance_past_hidden_file(file_idx);
                return;
            }

            if adjust_cursor {
                let header_line = self.calculate_file_scroll_offset(file_idx);
                self.diff_state.cursor_line = header_line;
                self.ensure_cursor_visible();
            }
        }
    }

    /// Land somewhere sensible after marking `file_idx` reviewed hid it: the
    /// next file still visible after it, wrapping to the first, or the overview
    /// when the queue is empty.
    fn advance_past_hidden_file(&mut self, file_idx: usize) {
        let visible = self.filtered_file_indices();
        let target = visible
            .iter()
            .find(|&&idx| idx > file_idx)
            .or_else(|| visible.first())
            .copied();

        match target {
            Some(idx) => self.jump_to_file(idx),
            None => {
                // Nothing left to review. Park at the overview so the diff
                // pane shows its empty state rather than a stale offset, and
                // name the command that brings the rows back.
                self.diff_state.current_file_idx = 0;
                self.diff_state.cursor_line = 0;
                self.diff_state.scroll_offset = 0;
                self.file_list_state.select(0);
                self.set_message("All files reviewed \u{00b7} :set reviewed shows them again");
            }
        }
    }

    fn hunk_at_cursor(&self) -> Option<(usize, usize)> {
        match self.line_annotations.get(self.diff_state.cursor_line)? {
            AnnotatedLine::HunkHeader { file_idx, hunk_idx }
            | AnnotatedLine::DiffLine {
                file_idx, hunk_idx, ..
            }
            | AnnotatedLine::SideBySideLine {
                file_idx, hunk_idx, ..
            } => Some((*file_idx, *hunk_idx)),
            _ => None,
        }
    }

    fn hunk_review_target(&self, file_idx: usize, hunk_idx: usize) -> Option<(PathBuf, String)> {
        let file = self.diff_files.get(file_idx)?;
        let key = file.hunk_review_key(hunk_idx)?;
        Some((file.display_path().clone(), key))
    }

    pub(in crate::app) fn hunk_header_line(
        &self,
        file_idx: usize,
        hunk_idx: usize,
    ) -> Option<usize> {
        self.line_annotations.iter().position(|line| {
            matches!(
                line,
                AnnotatedLine::HunkHeader {
                    file_idx: candidate_file_idx,
                    hunk_idx: candidate_hunk_idx
                } if *candidate_file_idx == file_idx && *candidate_hunk_idx == hunk_idx
            )
        })
    }

    pub fn is_hunk_reviewed(&self, file_idx: usize, hunk_idx: usize) -> bool {
        // Skip key computation (which hashes every hunk in the file) when this
        // file has no reviewed hunks — the common case for users not using `R`.
        let Some(file) = self.diff_files.get(file_idx) else {
            return false;
        };
        match self.session.files.get(file.display_path()) {
            Some(review) if !review.reviewed_hunks.is_empty() => {}
            _ => return false,
        }

        let Some((path, key)) = self.hunk_review_target(file_idx, hunk_idx) else {
            return false;
        };
        self.session.is_hunk_reviewed(&path, &key)
    }

    /// Whether a reviewed file should currently hide its body. Summary jumps
    /// may temporarily reveal one reviewed file in the continuous diff without
    /// changing the persisted reviewed marker.
    pub fn should_collapse_file(&self, file_idx: usize) -> bool {
        if self.is_single_file_view {
            return false;
        }

        let Some(file) = self.diff_files.get(file_idx) else {
            return false;
        };
        let path = file.display_path();
        self.session.is_file_reviewed(path) && self.revealed_reviewed_file.as_ref() != Some(path)
    }

    pub(in crate::app) fn reveal_reviewed_file(&mut self, file_idx: usize) {
        self.revealed_reviewed_file = self
            .diff_files
            .get(file_idx)
            .map(|file| file.display_path().clone());
    }

    /// Whether a reviewed hunk should currently hide its body. Summary jumps
    /// may temporarily reveal one reviewed hunk without changing the persisted
    /// reviewed marker used by [`Self::is_hunk_reviewed`].
    pub fn should_collapse_hunk(&self, file_idx: usize, hunk_idx: usize) -> bool {
        if !self.is_hunk_reviewed(file_idx, hunk_idx) {
            return false;
        }

        self.hunk_review_target(file_idx, hunk_idx)
            .is_none_or(|target| self.revealed_reviewed_hunk.as_ref() != Some(&target))
    }

    pub(in crate::app) fn reveal_reviewed_hunk(&mut self, file_idx: usize, hunk_idx: usize) {
        self.revealed_reviewed_hunk = self.hunk_review_target(file_idx, hunk_idx);
    }

    pub fn should_render_gap_before_hunk(&self, file_idx: usize, hunk_idx: usize) -> bool {
        // Reviewed hunks collapse as a complete review unit: their body and
        // adjoining hidden-context controls disappear with the header.
        !self.should_collapse_hunk(file_idx, hunk_idx)
            && (hunk_idx == 0 || !self.should_collapse_hunk(file_idx, hunk_idx - 1))
    }

    pub fn toggle_hunk_reviewed(&mut self) {
        let Some((file_idx, hunk_idx)) = self.hunk_at_cursor() else {
            self.set_warning("Move cursor to a hunk to toggle reviewed");
            return;
        };
        self.revealed_reviewed_hunk = None;

        let Some((path, key)) = self.hunk_review_target(file_idx, hunk_idx) else {
            self.set_warning("Move cursor to a hunk to toggle reviewed");
            return;
        };

        let Some(review) = self.session.get_file_mut(&path) else {
            return;
        };

        let reviewed = review.toggle_hunk_reviewed(key);
        self.dirty = true;
        self.rebuild_annotations();
        self.diff_state.current_file_idx = file_idx;
        if let Some(tree_idx) = self.file_idx_to_tree_idx(file_idx) {
            self.file_list_state.select(tree_idx);
        }
        if let Some(header_line) = self.hunk_header_line(file_idx, hunk_idx) {
            self.diff_state.cursor_line = header_line;
        }
        self.ensure_cursor_visible();

        if reviewed {
            self.set_message("Hunk marked reviewed");
        } else {
            self.set_message("Hunk marked unreviewed");
        }
    }

    /// Files in the review population: everything surviving the `i`/`e`
    /// patterns, whether or not it is currently reviewed.
    ///
    /// Deliberately *not* scoped by the reviewed-files toggle. This is the
    /// denominator of the tree title's `reviewed/total` fraction, which has to
    /// keep reporting progress while reviewed rows are hidden.
    pub fn file_count(&self) -> usize {
        if !self.file_filter_active() {
            return self.diff_files.len();
        }
        self.diff_files
            .iter()
            .filter(|file| self.file_matches_patterns(file))
            .count()
    }

    /// Total files in the diff, ignoring filters. Used to report how much a
    /// filter is hiding.
    pub fn unfiltered_file_count(&self) -> usize {
        self.diff_files.len()
    }

    /// Reviewed files within the population `file_count()` reports, so the two
    /// always form a coherent fraction.
    pub fn reviewed_count(&self) -> usize {
        if !self.file_filter_active() {
            return self.session.reviewed_count();
        }
        // Counting the whole session here would read as `12/5` next to a
        // filtered total, so count only reviewed files that survive the
        // patterns. Hiding them does not remove them from the count.
        self.diff_files
            .iter()
            .filter(|file| {
                self.file_matches_patterns(file)
                    && self.session.is_file_reviewed(file.display_path())
            })
            .count()
    }

    /// Returns `(total_files, total_additions, total_deletions)` across the
    /// files currently shown (filters applied).
    pub fn diff_stat(&self) -> (usize, usize, usize) {
        let mut additions = 0;
        let mut deletions = 0;
        let mut files = 0;
        for file in &self.diff_files {
            if !self.file_passes_filter(file) {
                continue;
            }
            files += 1;
            let (a, d) = file.stat();
            additions += a;
            deletions += d;
        }
        (files, additions, deletions)
    }

    /// Returns true when the cursor is in the review comments area above all files.
    pub fn is_cursor_in_overview(&self) -> bool {
        self.diff_state.cursor_line < self.review_comments_render_height()
    }
}
