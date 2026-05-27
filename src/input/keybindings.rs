use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::InputMode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    // Navigation
    CursorDown(usize),
    CursorUp(usize),
    HalfPageDown,
    HalfPageUp,
    PageDown,
    PageUp,
    GoToTop,
    GoToBottom,
    Digit(u8),
    NextFile,
    PrevFile,
    NextHunk,
    PrevHunk,
    PendingZCommand,
    PendingShiftZCommand,
    PendingLeaderCommand,
    ScrollLeft(usize),
    ScrollRight(usize),
    ScrollViewDown(usize),
    ScrollViewUp(usize),
    MouseScrollUp(usize),
    MouseScrollDown(usize),

    // Panel focus
    ToggleFocus,
    ToggleFocusReverse,
    SelectFile,

    // Review actions
    ToggleReviewed,
    AddLineComment,
    AddFileComment,
    EditComment,
    PendingDCommand,
    SearchNext,
    SearchPrev,

    // Visual selection mode
    EnterVisualMode,
    AddRangeComment,

    // Session
    Quit,
    ExportToClipboard,

    // Mode changes
    EnterCommandMode,
    EnterSearchMode,
    ExitMode,
    ToggleHelp,

    // Text input
    InsertChar(char),
    /// Bracketed-paste payload from the terminal. Carries the full pasted
    /// string so multi-line / shifted input doesn't get re-interpreted as
    /// keystrokes (Enter submitting, ':' opening command mode, etc.).
    Paste(String),
    DeleteChar,
    DeleteWord,
    ClearLine,
    SubmitInput,
    CompleteCommand,
    CompleteCommandReverse,
    TextCursorLeft,
    TextCursorRight,
    TextCursorLineStart,
    TextCursorLineEnd,
    TextCursorWordLeft,
    TextCursorWordRight,

    // Comment type
    CycleCommentType,
    CycleCommentTypeReverse,

    // Confirm dialog
    ConfirmYes,
    ConfirmNo,

    // Commit selection
    CommitSelectUp,
    CommitSelectDown,
    ToggleCommitSelect,
    ConfirmCommitSelect,
    /// Cycle inline commit selector to next individual commit (`)`)
    CycleCommitNext,
    /// Cycle inline commit selector to previous individual commit (`(`)
    CycleCommitPrev,

    // Review target selector
    /// Switch to the next selector tab (Tab key).
    TargetSelectorTabNext,
    /// Switch to the previous selector tab (Shift-Tab).
    TargetSelectorTabPrev,
    /// Begin editing the local filter inside the PR tab (`/`).
    BeginTargetFilter,

    // Submit resolver
    /// Move resolver cursor down (`j` / Down).
    SubmitResolverDown,
    /// Move resolver cursor up (`k` / Up).
    SubmitResolverUp,
    /// Toggle the action for the row under the resolver cursor (Enter).
    SubmitResolverToggle,
    /// Advance from resolver to final confirmation (`s`).
    SubmitResolverAdvance,
    /// Reload the PR in submit confirm (`r`, only when stale-head warning is
    /// active). Currently triggers the same path as `:e`.
    SubmitReloadPr,

    // Submit action picker (bare `:submit`)
    /// Move action-picker cursor down (`j` / Down).
    SubmitPickerDown,
    /// Move action-picker cursor up (`k` / Up).
    SubmitPickerUp,
    /// Confirm the picker selection (Enter).
    SubmitPickerConfirm,

    ToggleExpand,
    ExpandAll,
    CollapseAll,
    SelectFileFull,

    // No-op
    None,
}

pub fn map_key_to_action(key: KeyEvent, mode: InputMode, leader_key: char) -> Action {
    match mode {
        InputMode::Normal => map_normal_mode(key, leader_key),
        InputMode::Command => map_command_mode(key),
        InputMode::Search => map_search_mode(key),
        InputMode::Comment => map_comment_mode(key),
        InputMode::Help => map_help_mode(key),
        InputMode::Confirm => map_confirm_mode(key),
        InputMode::CommitSelect => map_commit_select_mode(key),
        InputMode::VisualSelect => map_visual_mode(key),
        InputMode::SubmitResolver => map_submit_resolver_mode(key),
        InputMode::SubmitConfirm => map_submit_confirm_mode(key),
        InputMode::SubmitActionPicker => map_submit_action_picker_mode(key),
    }
}

fn map_normal_mode(key: KeyEvent, leader_key: char) -> Action {
    match (key.code, key.modifiers) {
        (KeyCode::Char(key), KeyModifiers::NONE) if key == leader_key => {
            Action::PendingLeaderCommand
        }

        // Cursor movement (vim-like: cursor moves, scroll follows when needed)
        (KeyCode::Char('j') | KeyCode::Down, KeyModifiers::NONE) => Action::CursorDown(1),
        (KeyCode::Char('k') | KeyCode::Up, KeyModifiers::NONE) => Action::CursorUp(1),
        (KeyCode::Char('e'), KeyModifiers::CONTROL) => Action::ScrollViewDown(1),
        (KeyCode::Char('y'), KeyModifiers::CONTROL) => Action::ScrollViewUp(1),
        (KeyCode::Char('d'), KeyModifiers::CONTROL) => Action::HalfPageDown,
        (KeyCode::Char('u'), KeyModifiers::CONTROL) => Action::HalfPageUp,
        (KeyCode::Char('f'), KeyModifiers::CONTROL) => Action::PageDown,
        (KeyCode::Char('b'), KeyModifiers::CONTROL) => Action::PageUp,
        (KeyCode::PageDown, KeyModifiers::NONE) => Action::PageDown,
        (KeyCode::PageUp, KeyModifiers::NONE) => Action::PageUp,
        (KeyCode::Char('g'), KeyModifiers::NONE) => Action::GoToTop,
        (KeyCode::Char('G'), _) => Action::GoToBottom,
        (KeyCode::Char('z'), KeyModifiers::NONE) => Action::PendingZCommand,
        (KeyCode::Char('Z'), _) => Action::PendingShiftZCommand,

        // File navigation (use _ for modifiers since shift is implicit in the character)
        (KeyCode::Char('}'), _) => Action::NextFile,
        (KeyCode::Char('{'), _) => Action::PrevFile,
        (KeyCode::Char(']'), _) => Action::NextHunk,
        (KeyCode::Char('['), _) => Action::PrevHunk,
        (KeyCode::Char(')'), _) => Action::CycleCommitNext,
        (KeyCode::Char('('), _) => Action::CycleCommitPrev,

        // Panel focus
        (KeyCode::Tab, KeyModifiers::NONE) => Action::ToggleFocus,
        (KeyCode::BackTab, _) => Action::ToggleFocusReverse,
        (KeyCode::Enter, KeyModifiers::NONE) => Action::SelectFile,
        (KeyCode::Enter, KeyModifiers::SHIFT) => Action::SelectFileFull,

        // Horizontal scrolling
        (KeyCode::Char('h') | KeyCode::Left, KeyModifiers::NONE) => Action::ScrollLeft(4),
        (KeyCode::Char('l') | KeyCode::Right, KeyModifiers::NONE) => Action::ScrollRight(4),

        // Review actions
        (KeyCode::Char('r'), KeyModifiers::NONE) => Action::ToggleReviewed,
        (KeyCode::Char('c'), KeyModifiers::NONE) => Action::AddLineComment,
        (KeyCode::Char('C'), _) => Action::AddFileComment,
        (KeyCode::Char('i'), KeyModifiers::NONE) => Action::EditComment,
        (KeyCode::Char('d'), KeyModifiers::NONE) => Action::PendingDCommand,
        (KeyCode::Char('v') | KeyCode::Char('V'), _) => Action::EnterVisualMode,
        (KeyCode::Char('y'), KeyModifiers::NONE) => Action::ExportToClipboard,
        (KeyCode::Char('n'), KeyModifiers::NONE) => Action::SearchNext,
        (KeyCode::Char('N'), _) => Action::SearchPrev,

        // Mode changes (use _ for shifted characters like : and ?)
        (KeyCode::Char(':'), _) => Action::EnterCommandMode,
        (KeyCode::Char('/'), _) => Action::EnterSearchMode,
        (KeyCode::Char('?'), _) => Action::ToggleHelp,
        (KeyCode::Esc, KeyModifiers::NONE) => Action::ExitMode,

        // Quick quit
        (KeyCode::Char('q'), KeyModifiers::NONE) => Action::Quit,

        (KeyCode::Char(' '), KeyModifiers::NONE) => Action::ToggleExpand,
        (KeyCode::Char('o'), KeyModifiers::NONE) => Action::ExpandAll,
        (KeyCode::Char('O'), _) => Action::CollapseAll,

        (KeyCode::Char(c @ '0'..='9'), KeyModifiers::NONE) => Action::Digit(c as u8 - b'0'),

        _ => Action::None,
    }
}

fn map_command_mode(key: KeyEvent) -> Action {
    match (key.code, key.modifiers) {
        (KeyCode::Esc, KeyModifiers::NONE) => Action::ExitMode,
        (KeyCode::Enter, KeyModifiers::NONE) => Action::SubmitInput,
        (KeyCode::Tab, KeyModifiers::NONE) => Action::CompleteCommand,
        (KeyCode::BackTab, _) => Action::CompleteCommandReverse,
        (KeyCode::Backspace, mods) if mods.contains(KeyModifiers::ALT) => Action::DeleteWord,
        (KeyCode::Backspace, KeyModifiers::NONE) => Action::DeleteChar,
        (KeyCode::Char('w'), KeyModifiers::CONTROL) => Action::DeleteWord,
        (KeyCode::Char('u'), KeyModifiers::CONTROL) => Action::ClearLine,
        (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => Action::InsertChar(c),
        _ => Action::None,
    }
}

fn map_search_mode(key: KeyEvent) -> Action {
    match (key.code, key.modifiers) {
        (KeyCode::Esc, KeyModifiers::NONE) => Action::ExitMode,
        (KeyCode::Enter, KeyModifiers::NONE) => Action::SubmitInput,
        (KeyCode::Backspace, mods) if mods.contains(KeyModifiers::ALT) => Action::DeleteWord,
        (KeyCode::Backspace, KeyModifiers::NONE) => Action::DeleteChar,
        (KeyCode::Char('w'), KeyModifiers::CONTROL) => Action::DeleteWord,
        (KeyCode::Char('u'), KeyModifiers::CONTROL) => Action::ClearLine,
        (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => Action::InsertChar(c),
        _ => Action::None,
    }
}

fn map_comment_mode(key: KeyEvent) -> Action {
    match (key.code, key.modifiers) {
        // Cancel: Esc, Ctrl+C
        (KeyCode::Esc, KeyModifiers::NONE) => Action::ExitMode,
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => Action::ExitMode,
        // Submit: Enter without shift (Ctrl+Enter and Ctrl+S also work)
        (KeyCode::Enter, KeyModifiers::NONE) => Action::SubmitInput,
        (KeyCode::Enter, KeyModifiers::CONTROL) => Action::SubmitInput,
        (KeyCode::Char('s'), KeyModifiers::CONTROL) => Action::SubmitInput,
        // Newline: Shift+Enter (modern terminals) or Ctrl+J (universal fallback)
        (KeyCode::Enter, mods) if mods.contains(KeyModifiers::SHIFT) => Action::InsertChar('\n'),
        (KeyCode::Char('j'), KeyModifiers::CONTROL) => Action::InsertChar('\n'),
        // Comment type: Tab to cycle
        (KeyCode::Tab, KeyModifiers::NONE) => Action::CycleCommentType,
        (KeyCode::BackTab, _) => Action::CycleCommentTypeReverse,
        // Cursor movement
        (KeyCode::Char('a'), KeyModifiers::CONTROL) => Action::TextCursorLineStart,
        (KeyCode::Char('e'), KeyModifiers::CONTROL) => Action::TextCursorLineEnd,
        (KeyCode::Left, mods)
            if mods.contains(KeyModifiers::ALT) || mods.contains(KeyModifiers::CONTROL) =>
        {
            Action::TextCursorWordLeft
        }
        (KeyCode::Right, mods)
            if mods.contains(KeyModifiers::ALT) || mods.contains(KeyModifiers::CONTROL) =>
        {
            Action::TextCursorWordRight
        }
        (KeyCode::Home, _) => Action::TextCursorLineStart,
        (KeyCode::End, _) => Action::TextCursorLineEnd,
        (KeyCode::Left, mods)
            if mods.contains(KeyModifiers::SUPER) || mods.contains(KeyModifiers::META) =>
        {
            Action::TextCursorLineStart
        }
        (KeyCode::Right, mods)
            if mods.contains(KeyModifiers::SUPER) || mods.contains(KeyModifiers::META) =>
        {
            Action::TextCursorLineEnd
        }
        (KeyCode::Left, KeyModifiers::NONE) => Action::TextCursorLeft,
        (KeyCode::Right, KeyModifiers::NONE) => Action::TextCursorRight,
        // Editing
        (KeyCode::Backspace, mods)
            if mods.contains(KeyModifiers::ALT)
                || mods.contains(KeyModifiers::SUPER)
                || mods.contains(KeyModifiers::META) =>
        {
            Action::DeleteWord
        }
        (KeyCode::Backspace, KeyModifiers::NONE) => Action::DeleteChar,
        (KeyCode::Char('w'), KeyModifiers::CONTROL) => Action::DeleteWord,
        (KeyCode::Char('u'), KeyModifiers::CONTROL) => Action::ClearLine,
        (KeyCode::Char(c), _) => Action::InsertChar(c),
        _ => Action::None,
    }
}

fn map_help_mode(key: KeyEvent) -> Action {
    match (key.code, key.modifiers) {
        // Close help
        (KeyCode::Esc, KeyModifiers::NONE)
        | (KeyCode::Char('q'), KeyModifiers::NONE)
        | (KeyCode::Char('?'), _) => Action::ToggleHelp,
        // Scroll navigation
        (KeyCode::Char('j') | KeyCode::Down, KeyModifiers::NONE) => Action::CursorDown(1),
        (KeyCode::Char('k') | KeyCode::Up, KeyModifiers::NONE) => Action::CursorUp(1),
        (KeyCode::Char('d'), KeyModifiers::CONTROL) => Action::HalfPageDown,
        (KeyCode::Char('u'), KeyModifiers::CONTROL) => Action::HalfPageUp,
        (KeyCode::Char('f'), KeyModifiers::CONTROL) => Action::PageDown,
        (KeyCode::Char('b'), KeyModifiers::CONTROL) => Action::PageUp,
        (KeyCode::PageDown, KeyModifiers::NONE) => Action::PageDown,
        (KeyCode::PageUp, KeyModifiers::NONE) => Action::PageUp,
        (KeyCode::Char('g'), KeyModifiers::NONE) => Action::GoToTop,
        (KeyCode::Char('G'), _) => Action::GoToBottom,
        _ => Action::None,
    }
}

fn map_confirm_mode(key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => Action::ConfirmYes,
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => Action::ConfirmNo,
        _ => Action::None,
    }
}

fn map_submit_resolver_mode(key: KeyEvent) -> Action {
    match (key.code, key.modifiers) {
        (KeyCode::Char('j') | KeyCode::Down, KeyModifiers::NONE) => Action::SubmitResolverDown,
        (KeyCode::Char('k') | KeyCode::Up, KeyModifiers::NONE) => Action::SubmitResolverUp,
        (KeyCode::Enter, KeyModifiers::NONE) => Action::SubmitResolverToggle,
        (KeyCode::Char(' '), KeyModifiers::NONE) => Action::SubmitResolverToggle,
        (KeyCode::Char('s'), KeyModifiers::NONE) => Action::SubmitResolverAdvance,
        (KeyCode::Esc, KeyModifiers::NONE) => Action::ExitMode,
        _ => Action::None,
    }
}

fn map_submit_confirm_mode(key: KeyEvent) -> Action {
    match (key.code, key.modifiers) {
        (KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter, _) => Action::ConfirmYes,
        (KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc, _) => Action::ConfirmNo,
        (KeyCode::Char('r') | KeyCode::Char('R'), _) => Action::SubmitReloadPr,
        _ => Action::None,
    }
}

fn map_submit_action_picker_mode(key: KeyEvent) -> Action {
    match (key.code, key.modifiers) {
        (KeyCode::Char('j') | KeyCode::Down, KeyModifiers::NONE) => Action::SubmitPickerDown,
        (KeyCode::Char('k') | KeyCode::Up, KeyModifiers::NONE) => Action::SubmitPickerUp,
        (KeyCode::Enter, KeyModifiers::NONE) => Action::SubmitPickerConfirm,
        (KeyCode::Esc, KeyModifiers::NONE) => Action::ExitMode,
        (KeyCode::Char('q'), KeyModifiers::NONE) => Action::Quit,
        _ => Action::None,
    }
}

fn map_commit_select_mode(key: KeyEvent) -> Action {
    match (key.code, key.modifiers) {
        (KeyCode::Char('j') | KeyCode::Down, KeyModifiers::NONE) => Action::CommitSelectDown,
        (KeyCode::Char('k') | KeyCode::Up, KeyModifiers::NONE) => Action::CommitSelectUp,
        (KeyCode::Char(' '), KeyModifiers::NONE) => Action::ToggleCommitSelect,
        (KeyCode::Enter, KeyModifiers::NONE) => Action::ConfirmCommitSelect,
        (KeyCode::Esc, KeyModifiers::NONE) => Action::ExitMode,
        (KeyCode::Char('q'), KeyModifiers::NONE) => Action::Quit,
        (KeyCode::Tab, KeyModifiers::NONE) => Action::TargetSelectorTabNext,
        (KeyCode::BackTab, _) => Action::TargetSelectorTabPrev,
        (KeyCode::Char('/'), _) => Action::BeginTargetFilter,
        _ => Action::None,
    }
}

/// Key map used while the user is editing the PR-tab local filter.
/// This is a sub-state of `InputMode::CommitSelect`; the dispatcher in
/// `main.rs` routes here when `App::pr_filter_editing()` is true.
pub fn map_target_filter_mode(key: KeyEvent) -> Action {
    match (key.code, key.modifiers) {
        (KeyCode::Esc, KeyModifiers::NONE) => Action::ExitMode,
        (KeyCode::Enter, KeyModifiers::NONE) => Action::SubmitInput,
        (KeyCode::Backspace, KeyModifiers::NONE) => Action::DeleteChar,
        (KeyCode::Char('u'), KeyModifiers::CONTROL) => Action::ClearLine,
        (KeyCode::Char('w'), KeyModifiers::CONTROL) => Action::DeleteWord,
        (KeyCode::Backspace, mods) if mods.contains(KeyModifiers::ALT) => Action::DeleteWord,
        (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => Action::InsertChar(c),
        _ => Action::None,
    }
}

fn map_visual_mode(key: KeyEvent) -> Action {
    match (key.code, key.modifiers) {
        // Extend selection
        (KeyCode::Char('j') | KeyCode::Down, KeyModifiers::NONE) => Action::CursorDown(1),
        (KeyCode::Char('k') | KeyCode::Up, KeyModifiers::NONE) => Action::CursorUp(1),
        (KeyCode::Char('c'), KeyModifiers::NONE) => Action::AddRangeComment,
        (KeyCode::Enter, KeyModifiers::NONE) => Action::AddRangeComment,
        (KeyCode::Char('y'), KeyModifiers::NONE) => Action::ExportToClipboard,
        (KeyCode::Esc, KeyModifiers::NONE) => Action::ExitMode,
        (KeyCode::Char('v') | KeyCode::Char('V'), _) => Action::ExitMode,
        (KeyCode::Char('q'), KeyModifiers::NONE) => Action::Quit,
        _ => Action::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DEFAULT_LEADER_KEY;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn key_shift(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::SHIFT)
    }

    #[test]
    fn should_map_digit_keys_to_digit_action_in_normal_mode() {
        for d in 0..=9u8 {
            let c = (b'0' + d) as char;
            let action = map_normal_mode(key(KeyCode::Char(c)), DEFAULT_LEADER_KEY);
            assert_eq!(
                action,
                Action::Digit(d),
                "digit key '{c}' should map to Digit({d})"
            );
        }
    }

    #[test]
    fn should_map_uppercase_g_to_go_to_bottom_in_normal_mode() {
        let action = map_normal_mode(key_shift('G'), DEFAULT_LEADER_KEY);
        assert_eq!(action, Action::GoToBottom);
    }

    #[test]
    fn should_map_lowercase_g_to_go_to_top_in_normal_mode() {
        let action = map_normal_mode(key(KeyCode::Char('g')), DEFAULT_LEADER_KEY);
        assert_eq!(action, Action::GoToTop);
    }

    #[test]
    fn should_leave_lowercase_e_unbound_in_normal_mode() {
        let action = map_normal_mode(key(KeyCode::Char('e')), DEFAULT_LEADER_KEY);
        assert_eq!(action, Action::None);
    }

    #[test]
    fn should_keep_ctrl_e_as_scroll_down_in_normal_mode() {
        let action = map_normal_mode(
            KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL),
            DEFAULT_LEADER_KEY,
        );
        assert_eq!(action, Action::ScrollViewDown(1));
    }

    #[test]
    fn should_not_map_digits_in_command_mode() {
        for d in 0..=9u8 {
            let c = (b'0' + d) as char;
            let action = map_command_mode(key(KeyCode::Char(c)));
            assert_eq!(
                action,
                Action::InsertChar(c),
                "digit '{c}' in command mode should be InsertChar"
            );
        }
    }

    #[test]
    fn should_not_map_digits_in_search_mode() {
        for d in 0..=9u8 {
            let c = (b'0' + d) as char;
            let action = map_search_mode(key(KeyCode::Char(c)));
            assert_eq!(
                action,
                Action::InsertChar(c),
                "digit '{c}' in search mode should be InsertChar"
            );
        }
    }

    #[test]
    fn should_not_map_shifted_digits_to_digit_action() {
        // Shift+digit produces characters like !, @, #, etc. on most layouts,
        // but if a terminal sends the raw digit with SHIFT modifier it must not
        // be treated as Action::Digit.
        for d in 0..=9u8 {
            let c = (b'0' + d) as char;
            let action = map_normal_mode(key_shift(c), DEFAULT_LEADER_KEY);
            assert_ne!(
                action,
                Action::Digit(d),
                "Shift+'{c}' in normal mode must not produce Digit({d})"
            );
        }
    }

    #[test]
    fn should_map_backtab_to_reverse_focus_in_normal_mode() {
        let action = map_normal_mode(
            KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
            DEFAULT_LEADER_KEY,
        );
        assert_eq!(action, Action::ToggleFocusReverse);
    }

    #[test]
    fn should_map_backtab_to_reverse_comment_type_in_comment_mode() {
        let action = map_comment_mode(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
        assert_eq!(action, Action::CycleCommentTypeReverse);
    }

    #[test]
    fn should_map_configured_leader_to_pending_leader_action() {
        let action = map_key_to_action(key(KeyCode::Char(',')), InputMode::Normal, ',');
        assert_eq!(action, Action::PendingLeaderCommand);
    }

    #[test]
    fn should_leave_semicolon_unbound_when_another_leader_is_configured() {
        let action = map_key_to_action(key(KeyCode::Char(';')), InputMode::Normal, ',');
        assert_eq!(action, Action::None);
    }

    #[test]
    fn should_preserve_modified_shortcuts_when_character_is_leader() {
        let action = map_key_to_action(
            KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL),
            InputMode::Normal,
            'e',
        );
        assert_eq!(action, Action::ScrollViewDown(1));
    }

    #[test]
    fn should_keep_semicolon_as_default_leader() {
        let action = map_key_to_action(
            key(KeyCode::Char(DEFAULT_LEADER_KEY)),
            InputMode::Normal,
            DEFAULT_LEADER_KEY,
        );
        assert_eq!(action, Action::PendingLeaderCommand);
    }

    #[test]
    fn should_map_alt_backspace_to_delete_word_in_text_input_modes() {
        let alt_backspace = KeyEvent::new(KeyCode::Backspace, KeyModifiers::ALT);
        assert_eq!(map_comment_mode(alt_backspace), Action::DeleteWord);
        assert_eq!(map_command_mode(alt_backspace), Action::DeleteWord);
        assert_eq!(map_search_mode(alt_backspace), Action::DeleteWord);
    }

    #[test]
    fn should_map_tab_to_command_completion_in_command_mode() {
        let action = map_command_mode(key(KeyCode::Tab));
        assert_eq!(action, Action::CompleteCommand);
    }

    #[test]
    fn should_map_backtab_to_reverse_command_completion_in_command_mode() {
        let action = map_command_mode(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
        assert_eq!(action, Action::CompleteCommandReverse);
    }

    #[test]
    fn should_map_tab_to_target_selector_tab_next_in_commit_select_mode() {
        // given / when
        let action = map_commit_select_mode(key(KeyCode::Tab));
        // then
        assert_eq!(action, Action::TargetSelectorTabNext);
    }

    #[test]
    fn should_map_back_tab_to_target_selector_tab_prev_in_commit_select_mode() {
        // given / when
        let action = map_commit_select_mode(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
        // then
        assert_eq!(action, Action::TargetSelectorTabPrev);
    }

    #[test]
    fn should_map_slash_to_begin_target_filter_in_commit_select_mode() {
        // given / when
        let action = map_commit_select_mode(key(KeyCode::Char('/')));
        // then
        assert_eq!(action, Action::BeginTargetFilter);
    }

    #[test]
    fn should_route_typed_chars_to_insert_in_target_filter_mode() {
        // given / when
        let action = map_target_filter_mode(key(KeyCode::Char('a')));
        // then
        assert_eq!(action, Action::InsertChar('a'));
    }

    #[test]
    fn should_map_enter_to_submit_in_target_filter_mode() {
        // given / when
        let action = map_target_filter_mode(key(KeyCode::Enter));
        // then
        assert_eq!(action, Action::SubmitInput);
    }

    #[test]
    fn should_map_esc_to_exit_in_target_filter_mode() {
        // given / when
        let action = map_target_filter_mode(key(KeyCode::Esc));
        // then
        assert_eq!(action, Action::ExitMode);
    }

    #[test]
    fn no_key_should_produce_mouse_scroll_actions() {
        let codes = [
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Left,
            KeyCode::Right,
            KeyCode::PageDown,
            KeyCode::PageUp,
            KeyCode::Char('j'),
            KeyCode::Char('k'),
            KeyCode::Char('e'),
            KeyCode::Char('y'),
        ];
        let mod_sets = [
            KeyModifiers::NONE,
            KeyModifiers::CONTROL,
            KeyModifiers::ALT,
            KeyModifiers::SHIFT,
        ];
        for code in codes {
            for mods in mod_sets {
                let ev = KeyEvent::new(code, mods);
                for action in [
                    map_normal_mode(ev, DEFAULT_LEADER_KEY),
                    map_command_mode(ev),
                    map_search_mode(ev),
                    map_comment_mode(ev),
                    map_help_mode(ev),
                ] {
                    assert!(
                        !matches!(
                            action,
                            Action::MouseScrollUp(_) | Action::MouseScrollDown(_)
                        ),
                        "key {code:?} + {mods:?} produced a mouse-scroll action"
                    );
                }
            }
        }
    }
}
