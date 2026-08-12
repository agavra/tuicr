//! File-tree include/exclude filters and `/` search.
//!
//! Filters are a *view* over `diff_files`, not a mutation of it: `file_idx`
//! stays an absolute index into `diff_files`, so nothing downstream needs
//! remapping. Every consumer that walks files asks `file_passes_filter`:
//!
//! - `build_visible_items` (tree, and therefore `}`/`{` file nav)
//! - `rebuild_annotations` (diff content, comment navigator)
//! - `file_render_height` / `effective_file_height` (scroll math -> 0 lines)
//! - `hunk_positions` (`]`/`[`)
//! - both diff renderers
//!
//! Search is deliberately weaker: it only moves the tree selection, leaving
//! the diff viewport where the user left it.

use super::*;
use regex::RegexBuilder;

impl App {
    /// True when `file` survives the `i`/`e` patterns, ignoring whether it is
    /// reviewed. This is the *review population*: what the tree title's
    /// `reviewed/total` fraction counts and what a `/` search walks.
    ///
    /// Commit-message pseudo-files are matched like any other row: their
    /// display path is `Commit Message (<sha>)`, so `i \.rs$` hides them
    /// along with every other non-Rust entry. That keeps "include" meaning
    /// exactly what it says instead of carving out a silent exception.
    pub fn file_matches_patterns(&self, file: &DiffFile) -> bool {
        let path = file.display_path().to_string_lossy().to_string();
        if let Some(include) = &self.file_filter.include
            && !include.regex.is_match(&path)
        {
            return false;
        }
        if let Some(exclude) = &self.file_filter.exclude
            && exclude.regex.is_match(&path)
        {
            return false;
        }
        true
    }

    /// True when a file should appear in the tree, the diff, and the
    /// navigation/scroll math. This is the predicate every consumer that walks
    /// files asks — the patterns *plus* the reviewed-files toggle.
    ///
    /// Kept distinct from `file_matches_patterns` because "what is on screen"
    /// and "how much am I reviewing" stop being the same question once
    /// reviewed files can be hidden: counting a hidden reviewed file as
    /// not-shown would collapse the progress fraction to `0/n`.
    pub fn file_passes_filter(&self, file: &DiffFile) -> bool {
        self.file_matches_patterns(file)
            && (self.file_filter.show_reviewed
                || !self.session.is_file_reviewed(file.display_path()))
    }

    /// `file_passes_filter` by index, for the loops that only carry an index.
    pub fn file_idx_passes_filter(&self, file_idx: usize) -> bool {
        self.diff_files
            .get(file_idx)
            .is_none_or(|file| self.file_passes_filter(file))
    }

    pub fn file_filter_active(&self) -> bool {
        self.file_filter.include.is_some() || self.file_filter.exclude.is_some()
    }

    // ---- hiding reviewed files -------------------------------------------

    pub fn show_reviewed(&self) -> bool {
        self.file_filter.show_reviewed
    }

    pub fn toggle_show_reviewed(&mut self) {
        self.set_show_reviewed(!self.file_filter.show_reviewed);
    }

    /// Apply the `show_reviewed` config default at startup, without the status
    /// message the interactive toggles set.
    ///
    /// Still re-derives everything: `App::build` builds annotations before the
    /// config is applied, so a bare field assignment would leave the diff
    /// renderer and the cursor reading rows the filter has since dropped.
    pub fn init_show_reviewed(&mut self, show: bool) {
        self.file_filter.show_reviewed = show;
        self.apply_file_filter_change();
    }

    pub fn set_show_reviewed(&mut self, show: bool) {
        self.file_filter.show_reviewed = show;
        self.apply_file_filter_change();
        self.report_show_reviewed();
    }

    /// Say what just happened to the rows, and — when everything is hidden —
    /// how to get them back. `H` is file-tree only, so the message names
    /// `:set reviewed` too, which works from any pane.
    fn report_show_reviewed(&mut self) {
        let total = self.file_count();
        if self.file_filter.show_reviewed {
            self.set_message(format!("Showing all {total} files"));
            return;
        }
        let hidden = self.reviewed_count();
        if hidden == 0 {
            self.set_message("Hiding reviewed files \u{00b7} none reviewed yet");
        } else if hidden == total {
            self.set_message(format!(
                "All {total} files reviewed \u{00b7} H or :set reviewed shows them again"
            ));
        } else {
            self.set_message(format!(
                "Hiding {hidden} reviewed \u{00b7} {} of {total} shown",
                total - hidden
            ));
        }
    }

    /// Indices of the files surviving the current filters, in tree order.
    pub fn filtered_file_indices(&self) -> Vec<usize> {
        self.diff_files
            .iter()
            .enumerate()
            .filter(|(_, file)| self.file_passes_filter(file))
            .map(|(idx, _)| idx)
            .collect()
    }

    // ---- prompt lifecycle -------------------------------------------------

    /// Open one of the three file-tree prompts, pre-seeded with the value
    /// already applied so the user can refine rather than retype.
    pub fn begin_file_tree_prompt(&mut self, prompt: FileTreePrompt) {
        let buffer = match prompt {
            FileTreePrompt::Include => self
                .file_filter
                .include
                .as_ref()
                .map(|p| p.source.clone())
                .unwrap_or_default(),
            FileTreePrompt::Exclude => self
                .file_filter
                .exclude
                .as_ref()
                .map(|p| p.source.clone())
                .unwrap_or_default(),
            FileTreePrompt::Search => self.file_filter.search.clone().unwrap_or_default(),
        };
        self.file_filter.draft = Some(FileTreeDraft { prompt, buffer });
    }

    /// True while a file-tree prompt owns keyboard input. `main.rs` checks
    /// this before mapping keys so typed characters reach the buffer instead
    /// of driving tree navigation.
    pub fn file_tree_prompt_editing(&self) -> bool {
        self.file_filter.draft.is_some()
    }

    pub fn file_tree_draft(&self) -> Option<&FileTreeDraft> {
        self.file_filter.draft.as_ref()
    }

    pub fn file_tree_prompt_insert_char(&mut self, ch: char) {
        if let Some(draft) = self.file_filter.draft.as_mut() {
            draft.buffer.push(ch);
        }
    }

    pub fn file_tree_prompt_insert_str(&mut self, text: &str) {
        if let Some(draft) = self.file_filter.draft.as_mut() {
            for ch in text.chars() {
                if !matches!(ch, '\n' | '\r') {
                    draft.buffer.push(ch);
                }
            }
        }
    }

    pub fn file_tree_prompt_delete_char(&mut self) {
        if let Some(draft) = self.file_filter.draft.as_mut() {
            draft.buffer.pop();
        }
    }

    pub fn file_tree_prompt_delete_word(&mut self) {
        if let Some(draft) = self.file_filter.draft.as_mut() {
            while draft
                .buffer
                .chars()
                .next_back()
                .is_some_and(char::is_whitespace)
            {
                draft.buffer.pop();
            }
            while draft
                .buffer
                .chars()
                .next_back()
                .is_some_and(|c| !c.is_whitespace())
            {
                draft.buffer.pop();
            }
        }
    }

    pub fn file_tree_prompt_clear_line(&mut self) {
        if let Some(draft) = self.file_filter.draft.as_mut() {
            draft.buffer.clear();
        }
    }

    pub fn cancel_file_tree_prompt(&mut self) {
        self.file_filter.draft = None;
    }

    /// Apply the draft. An empty buffer clears that filter, so `i`+Enter is
    /// the same as `I`. An invalid regex leaves the prompt open with an
    /// error so the pattern can be fixed instead of retyped.
    pub fn commit_file_tree_prompt(&mut self) {
        let Some(draft) = self.file_filter.draft.clone() else {
            return;
        };
        let pattern = draft.buffer.trim().to_string();

        match draft.prompt {
            FileTreePrompt::Search => {
                self.file_filter.draft = None;
                if pattern.is_empty() {
                    self.file_filter.search = None;
                    return;
                }
                self.file_filter.search = Some(pattern);
                self.step_file_tree_search(true, true);
            }
            FileTreePrompt::Include | FileTreePrompt::Exclude => {
                if pattern.is_empty() {
                    self.file_filter.draft = None;
                    match draft.prompt {
                        FileTreePrompt::Include => self.clear_include_filter(),
                        _ => self.clear_exclude_filter(),
                    }
                    return;
                }
                let compiled = match RegexBuilder::new(&pattern).case_insensitive(true).build() {
                    Ok(regex) => FilePattern {
                        source: pattern,
                        regex,
                    },
                    Err(err) => {
                        // Keep the draft open: the user is mid-pattern and
                        // retyping from scratch would be worse than fixing it.
                        self.set_error(format!(
                            "Invalid regex: {}",
                            regex_reason(&err.to_string())
                        ));
                        return;
                    }
                };
                self.file_filter.draft = None;
                let label = draft.prompt.label();
                let source = compiled.source.clone();
                match draft.prompt {
                    FileTreePrompt::Include => self.file_filter.include = Some(compiled),
                    _ => self.file_filter.exclude = Some(compiled),
                }
                self.apply_file_filter_change();
                let matched = self.filtered_file_indices().len();
                if matched == 0 {
                    self.set_warning(format!(
                        "No files match {label} \"{source}\" \u{2014} I clears include, E clears exclude"
                    ));
                } else {
                    let total = self.diff_files.len();
                    self.set_message(format!(
                        "Filter {label} \"{source}\" \u{00b7} {matched}/{total} files"
                    ));
                }
            }
        }
    }

    pub fn clear_include_filter(&mut self) {
        if self.file_filter.include.take().is_none() {
            self.set_message("No include filter set");
            return;
        }
        self.apply_file_filter_change();
        self.report_filter_cleared("include");
    }

    pub fn clear_exclude_filter(&mut self) {
        if self.file_filter.exclude.take().is_none() {
            self.set_message("No exclude filter set");
            return;
        }
        self.apply_file_filter_change();
        self.report_filter_cleared("exclude");
    }

    fn report_filter_cleared(&mut self, label: &str) {
        let shown = self.filtered_file_indices().len();
        let total = self.diff_files.len();
        if shown == total {
            self.set_message(format!("Cleared {label} filter \u{00b7} {total} files"));
        } else {
            self.set_message(format!(
                "Cleared {label} filter \u{00b7} {shown}/{total} files"
            ));
        }
    }

    /// Re-derive everything that depends on which files are visible, then
    /// make sure the cursor and tree selection aren't parked on a file that
    /// just disappeared.
    fn apply_file_filter_change(&mut self) {
        self.rebuild_annotations();

        let visible = self.filtered_file_indices();
        if visible.is_empty() {
            // Nothing to focus. Park at the overview so the diff pane shows
            // its empty state instead of a stale offset into hidden content.
            self.diff_state.current_file_idx = 0;
            self.diff_state.cursor_line = 0;
            self.diff_state.scroll_offset = 0;
            self.file_list_state.select(0);
            return;
        }

        if visible.contains(&self.diff_state.current_file_idx) {
            // Same file, but its render offset moved because earlier files
            // may now be hidden.
            self.jump_to_file(self.diff_state.current_file_idx);
        } else {
            self.jump_to_file(visible[0]);
        }
    }

    // ---- `/` search -------------------------------------------------------

    pub fn file_tree_search_active(&self) -> bool {
        self.file_filter.search.is_some()
    }

    pub fn file_tree_search_next(&mut self) {
        self.step_file_tree_search(true, false);
    }

    pub fn file_tree_search_prev(&mut self) {
        self.step_file_tree_search(false, false);
    }

    /// Move the tree selection to the next/previous file whose path contains
    /// the query (case-insensitive), wrapping around. Only the selection
    /// moves: the diff viewport stays put until the user presses Enter.
    ///
    /// `include_current` lets a freshly submitted query match the file that
    /// is already selected instead of skipping past it.
    fn step_file_tree_search(&mut self, forward: bool, include_current: bool) {
        let Some(query) = self.file_filter.search.clone() else {
            self.set_message("No file tree search active");
            return;
        };
        let needle = query.to_lowercase();

        let candidates: Vec<usize> = self
            .filtered_file_indices()
            .into_iter()
            .filter(|&idx| {
                self.diff_files[idx]
                    .display_path()
                    .to_string_lossy()
                    .to_lowercase()
                    .contains(&needle)
            })
            .collect();

        if candidates.is_empty() {
            self.set_message(format!("No files matching \"{query}\""));
            return;
        }

        let anchor = self.selected_tree_file_idx();
        let target = if forward {
            candidates
                .iter()
                .find(|&&idx| {
                    if include_current {
                        idx >= anchor
                    } else {
                        idx > anchor
                    }
                })
                .copied()
                .unwrap_or(candidates[0])
        } else {
            candidates
                .iter()
                .rev()
                .find(|&&idx| {
                    if include_current {
                        idx <= anchor
                    } else {
                        idx < anchor
                    }
                })
                .copied()
                .unwrap_or(*candidates.last().expect("candidates is non-empty"))
        };

        self.select_file_in_tree(target);
        let path = self.diff_files[target].display_path().display().to_string();
        let position = candidates
            .iter()
            .position(|&idx| idx == target)
            .unwrap_or(0)
            + 1;
        self.set_message(format!(
            "{path} \u{00b7} {position}/{} for \"{query}\"",
            candidates.len()
        ));
    }

    /// The file the tree selection is sitting on, or the diff's current file
    /// when a directory row is selected.
    fn selected_tree_file_idx(&self) -> usize {
        match self.get_selected_tree_item() {
            Some(FileTreeItem::File { file_idx, .. }) => file_idx,
            _ => self.diff_state.current_file_idx,
        }
    }

    /// Expand the ancestors of `file_idx` and select its row, without
    /// touching the diff viewport.
    pub(in crate::app) fn select_file_in_tree(&mut self, file_idx: usize) {
        use std::path::Path;

        let Some(file) = self.diff_files.get(file_idx) else {
            return;
        };
        let path = file.display_path().clone();
        let mut current = path.parent();
        while let Some(parent) = current {
            if parent != Path::new("") {
                self.expanded_dirs
                    .insert(parent.to_string_lossy().to_string());
            }
            current = parent.parent();
        }
        if let Some(tree_idx) = self.file_idx_to_tree_idx(file_idx) {
            self.file_list_state.select(tree_idx);
        }
    }
}

/// Reduce a `regex` parse error to the one line worth showing in the status
/// bar. The crate renders errors as:
///
/// ```text
/// regex parse error:
///     [unclosed
///     ^
/// error: unclosed character class
/// ```
///
/// The first line is a useless header, so prefer the trailing `error:` line
/// (which carries the actual reason) and fall back to a flattened message.
fn regex_reason(message: &str) -> String {
    if let Some(reason) = message
        .lines()
        .map(str::trim)
        .filter_map(|line| line.strip_prefix("error:"))
        .next_back()
    {
        return reason.trim().to_string();
    }
    message
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::regex_reason;

    #[test]
    fn should_take_the_trailing_reason_line_over_the_header() {
        // Shape of a real `regex` parse error. The end-to-end assertion
        // against the live crate output lives in
        // `app::tests::file_filter_tests`, which reaches the compiler through
        // the prompt instead of a literal clippy can lint.
        let raw = "regex parse error:\n    [unclosed\n    ^\nerror: unclosed character class";

        assert_eq!(regex_reason(raw), "unclosed character class");
    }

    #[test]
    fn should_flatten_errors_that_carry_no_reason_line() {
        assert_eq!(regex_reason("something\n  broke"), "something broke");
    }
}
