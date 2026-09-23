//! Runtime `:theme` picker: live-preview list with a `/`-filter draft, the
//! same shape as the file tree's `i`/`e`/`/` prompts (`app/file_filter.rs`).

use super::*;
use crate::config::themes_dir;
use crate::theme::{built_in_theme_names, list_local_theme_names, resolve_theme_name};

impl App {
    /// Open the picker: snapshot the current theme (for `Esc`-revert) and
    /// build the candidate catalog (built-ins first, then local themes).
    pub fn enter_theme_picker_mode(&mut self) {
        let mut candidates = built_in_theme_names();
        if let Ok(dir) = themes_dir() {
            candidates.extend(list_local_theme_names(&dir));
        }

        self.theme_picker.original = Some(self.theme.clone());
        self.theme_picker.candidates = candidates;
        self.theme_picker.filter = None;
        self.theme_picker.draft = None;
        self.theme_picker.list_state = ratatui::widgets::ListState::default();
        self.theme_picker.select(0);
        self.input_mode = InputMode::ThemePicker;
    }

    /// `Esc` on the picker itself (not the filter draft): revert the live
    /// preview and close.
    pub fn cancel_theme_picker(&mut self) {
        if let Some(original) = self.theme_picker.original.take() {
            self.theme = original;
            // Undo the scrub-time rehighlight of the current file (see
            // `preview_theme`) so nothing is left recolored under a theme
            // that was never actually chosen.
            self.rehighlight_current_file();
        }
        self.exit_theme_picker_mode();
    }

    /// `Enter` on the picker itself: keep the currently previewed theme for
    /// this session and close.
    pub fn confirm_theme_picker(&mut self) {
        let name = self.theme_picker.selected_name().map(str::to_string);
        self.exit_theme_picker_mode();
        let Some(name) = name else {
            self.set_warning("No theme selected");
            return;
        };
        self.announce_session_theme(&name);
        self.rehighlight_all_files();
    }

    fn exit_theme_picker_mode(&mut self) {
        self.input_mode = InputMode::Normal;
        self.theme_picker.candidates.clear();
        self.theme_picker.filter = None;
        self.theme_picker.draft = None;
        self.theme_picker.list_state = ratatui::widgets::ListState::default();
        self.theme_picker.original = None;
    }

    /// Move the highlighted row within the filtered view and live-preview
    /// it. `delta` is typically `1`/`-1`; clamps rather than wraps.
    pub fn move_theme_picker_selection(&mut self, delta: isize) {
        let len = self.theme_picker.filtered_indices().len();
        if len == 0 {
            return;
        }
        let current = self.theme_picker.selected() as isize;
        let next = (current + delta).clamp(0, len as isize - 1);
        self.theme_picker.select(next as usize);
        self.preview_selected_theme();
    }

    fn preview_selected_theme(&mut self) {
        let Some(name) = self.theme_picker.selected_name().map(str::to_string) else {
            return;
        };
        self.preview_theme(&name);
    }

    /// Resolve `name` and assign it as the live preview. Leaves the current
    /// preview untouched (with a warning) if `name` doesn't resolve.
    ///
    /// Only rehighlights the file currently on screen (`rehighlight_current_file`),
    /// not the whole diff -- this runs on every picker navigation, so it has to
    /// stay cheap enough for rapid `j`/`k` scrubbing. Other loaded files keep
    /// their foreground syntax colors from whatever theme was active when they
    /// were last highlighted until `rehighlight_all_files` runs (on confirm).
    pub fn preview_theme(&mut self, name: &str) {
        let theme_dir = match themes_dir() {
            Ok(dir) => dir,
            Err(err) => {
                self.set_warning(format!("Could not determine theme directory: {err}"));
                return;
            }
        };
        match resolve_theme_name(name, &theme_dir) {
            Ok(Some((theme, warnings))) => {
                self.theme = theme;
                self.rehighlight_current_file();
                if let Some(first) = warnings.into_iter().next() {
                    self.set_warning(first);
                }
            }
            Ok(None) => {
                self.set_warning(format!("Unknown theme '{name}'"));
            }
            Err(err) => {
                self.set_warning(format!("Could not load theme '{name}': {err}"));
            }
        }
    }

    /// Apply `name` for this session directly (`:theme <name>`, no picker).
    pub fn apply_theme(&mut self, name: &str) {
        let theme_dir = match themes_dir() {
            Ok(dir) => dir,
            Err(err) => {
                self.set_error(format!("Could not determine theme directory: {err}"));
                return;
            }
        };
        match resolve_theme_name(name, &theme_dir) {
            Ok(Some((theme, warnings))) => {
                self.theme = theme;
                for warning in warnings {
                    self.set_warning(warning);
                }
                self.announce_session_theme(name);
                self.rehighlight_all_files();
            }
            Ok(None) => {
                self.set_error(format!(
                    "Unknown theme '{name}'. Bundled themes: {}",
                    crate::theme::built_in_theme_names_display()
                ));
            }
            Err(err) => {
                self.set_error(format!("Could not load theme '{name}': {err}"));
            }
        }
    }

    /// Theme changes are session-only, like `:set`/`:wrap`; point the user at
    /// the config key that makes the choice stick.
    fn announce_session_theme(&mut self, name: &str) {
        self.set_message(format!(
            "Theme: {name} (this session; set theme = \"{name}\" in config.toml to keep it)"
        ));
    }

    /// Re-derive `highlighted_spans` for the file currently on screen under
    /// `self.theme`, without touching the VCS. Cheap: proportional to one
    /// file's size, safe to call on every picker keystroke.
    ///
    /// `pub(crate)` (rather than private) so tests can exercise the
    /// rehighlight scoping directly.
    pub(crate) fn rehighlight_current_file(&mut self) {
        let highlighter = self.theme.syntax_highlighter();
        if let Some(file) = self.diff_files.get_mut(self.diff_state.current_file_idx) {
            highlighter.rehighlight_file_in_place(file);
        }
    }

    /// Re-derive `highlighted_spans` for every loaded file under `self.theme`,
    /// without touching the VCS. Proportional to the whole diff's size, so
    /// this is reserved for one-time actions (confirming a theme), not
    /// per-keystroke preview. No-op for container-grammar files, which need
    /// real full-file content this cache doesn't retain -- those stay stale
    /// until an actual reload (`:e`).
    pub(crate) fn rehighlight_all_files(&mut self) {
        let highlighter = self.theme.syntax_highlighter();
        for file in &mut self.diff_files {
            highlighter.rehighlight_file_in_place(file);
        }
    }

    // ---- `/` filter draft ---------------------------------------------

    pub fn theme_picker_filtering(&self) -> bool {
        self.theme_picker.draft.is_some()
    }

    /// `/` inside the picker: open the filter draft, pre-seeded with the
    /// currently applied filter so it can be refined rather than retyped.
    pub fn begin_theme_picker_filter(&mut self) {
        let seed = self.theme_picker.filter.clone().unwrap_or_default();
        self.theme_picker.draft = Some(seed);
    }

    pub fn theme_picker_filter_insert_char(&mut self, ch: char) {
        if let Some(draft) = self.theme_picker.draft.as_mut() {
            draft.push(ch);
        }
    }

    pub fn theme_picker_filter_delete_char(&mut self) {
        if let Some(draft) = self.theme_picker.draft.as_mut() {
            draft.pop();
        }
    }

    pub fn theme_picker_filter_clear_line(&mut self) {
        if let Some(draft) = self.theme_picker.draft.as_mut() {
            draft.clear();
        }
    }

    /// `Esc` while the filter draft is open: discard the draft without
    /// changing the applied filter or current selection/preview.
    pub fn cancel_theme_picker_filter(&mut self) {
        self.theme_picker.draft = None;
    }

    /// `Enter` while the filter draft is open: commit it (empty clears the
    /// filter), select the first match, and live-preview it. The picker
    /// itself stays open.
    pub fn commit_theme_picker_filter(&mut self) {
        let Some(draft) = self.theme_picker.draft.take() else {
            return;
        };
        let pattern = draft.trim().to_string();
        self.theme_picker.filter = if pattern.is_empty() {
            None
        } else {
            Some(pattern)
        };
        self.theme_picker.select(0);
        self.preview_selected_theme();
    }
}
