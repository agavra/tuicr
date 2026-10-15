use super::*;
use crate::input::mappings::MapMode;

impl App {
    pub fn mapping_mode(&self) -> Option<MapMode> {
        match self.input_mode {
            InputMode::Normal if self.file_tree_prompt_editing() => Some(MapMode::CommandLine),
            InputMode::Normal => Some(MapMode::Normal),
            InputMode::VisualSelect => Some(MapMode::Visual),
            InputMode::Comment if self.comment_vim_command_active() => Some(MapMode::CommandLine),
            InputMode::Comment if self.comment_vim_enabled => {
                self.comment_vim_in_insert_mode().then_some(MapMode::Insert)
            }
            InputMode::Comment => Some(MapMode::Insert),
            InputMode::Command | InputMode::Search => Some(MapMode::CommandLine),
            InputMode::CommitSelect if self.pr_filter_editing() => Some(MapMode::CommandLine),
            InputMode::ThemePicker if self.theme_picker_filtering() => Some(MapMode::CommandLine),
            _ => None,
        }
    }
}
