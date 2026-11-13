//! `<leader>p` file picker: a modal list of the files on screen, with a
//! `/`-filter draft shaped like the `:theme` picker's (`app/theme_picker.rs`).
//! Jumping goes through `jump_to_file`, the path the file tree's `Enter`
//! takes, so single-file view and tree selection follow along.

use super::*;

impl App {
    /// Open the picker over the files that pass the file-tree filters, with
    /// the diff's current file preselected when it is listed.
    pub fn enter_file_picker_mode(&mut self) {
        let candidates: Vec<(usize, String)> = self
            .diff_files
            .iter()
            .enumerate()
            .filter(|(_, file)| self.file_passes_filter(file))
            .map(|(idx, file)| (idx, file.display_path().display().to_string()))
            .collect();
        if candidates.is_empty() {
            self.set_warning("No files to pick");
            return;
        }

        let selected = candidates
            .iter()
            .position(|(idx, _)| *idx == self.diff_state.current_file_idx)
            .unwrap_or(0);
        self.file_picker.candidates = candidates;
        self.file_picker.draft = None;
        self.file_picker.list_state = ratatui::widgets::ListState::default();
        self.file_picker.select(selected);
        self.input_mode = InputMode::FilePicker;
    }

    /// `Esc` on the picker itself: close without moving anything.
    pub fn cancel_file_picker(&mut self) {
        self.exit_file_picker_mode();
    }

    /// `Enter` on the picker itself: jump to the highlighted file the way the
    /// file tree's `Enter` does, then close.
    ///
    /// The target is looked up again by display path: a background reload
    /// (PR refresh, diff watch) can replace `diff_files` while the picker is
    /// open, which would leave the captured index pointing at another file.
    pub fn confirm_file_picker(&mut self) {
        let target = self.file_picker.selected_path().map(str::to_string);
        self.exit_file_picker_mode();
        let Some(path) = target else {
            return;
        };
        let file_idx = self.diff_files.iter().position(|file| {
            file.display_path().display().to_string() == path && self.file_passes_filter(file)
        });
        match file_idx {
            Some(file_idx) => {
                self.jump_to_file(file_idx);
                self.focused_panel = FocusedPanel::Diff;
            }
            None => self.set_warning(format!("{path} is no longer in the diff")),
        }
    }

    fn exit_file_picker_mode(&mut self) {
        self.input_mode = InputMode::Normal;
        self.file_picker = FilePickerState::default();
    }

    /// Move the highlighted row within the filtered view. Clamps rather than
    /// wraps, like the theme picker.
    pub fn move_file_picker_selection(&mut self, delta: isize) {
        let len = self.file_picker.filtered_indices().len();
        if len == 0 {
            return;
        }
        let current = self.file_picker.selected() as isize;
        let next = (current + delta).clamp(0, len as isize - 1);
        self.file_picker.select(next as usize);
    }

    // ---- `/` filter draft ---------------------------------------------

    /// Checks the mode too, so a code path that forces `Normal` without
    /// closing the picker cannot leave the draft's hints on screen.
    pub fn file_picker_filtering(&self) -> bool {
        self.input_mode == InputMode::FilePicker && self.file_picker.draft.is_some()
    }

    /// `/` inside the picker: open an empty filter draft, remembering the
    /// highlighted file in case the draft is discarded.
    pub fn begin_file_picker_filter(&mut self) {
        self.file_picker.file_before_draft = self.file_picker.selected_file_idx();
        self.file_picker.draft = Some(String::new());
    }

    pub fn file_picker_filter_insert_char(&mut self, ch: char) {
        if let Some(draft) = self.file_picker.draft.as_mut() {
            draft.push(ch);
            self.file_picker.select(0);
        }
    }

    pub fn file_picker_filter_delete_char(&mut self) {
        if let Some(draft) = self.file_picker.draft.as_mut() {
            draft.pop();
            self.file_picker.select(0);
        }
    }

    pub fn file_picker_filter_clear_line(&mut self) {
        if let Some(draft) = self.file_picker.draft.as_mut() {
            draft.clear();
            self.file_picker.select(0);
        }
    }

    /// `Esc` while the filter draft is open: discard the draft and put back
    /// the full list and the row that was highlighted before `/`. The picker
    /// stays open.
    pub fn cancel_file_picker_filter(&mut self) {
        self.file_picker.draft = None;
        let previous = self.file_picker.file_before_draft.take();
        let position = self
            .file_picker
            .filtered_indices()
            .iter()
            .position(|&idx| Some(self.file_picker.candidates[idx].0) == previous)
            .unwrap_or(0);
        self.file_picker.select(position);
    }

    /// `Enter` while the filter draft is open: jump to the selected match and
    /// close, like `Enter` outside the draft. With no match, stay open.
    pub fn submit_file_picker_filter(&mut self) {
        if self.file_picker.selected_path().is_none() {
            self.set_warning("No files match");
            return;
        }
        self.confirm_file_picker();
    }
}
