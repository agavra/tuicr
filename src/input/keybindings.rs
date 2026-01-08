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
    NextFile,
    PrevFile,
    NextHunk,
    PrevHunk,
    PendingZCommand,

    // Panel focus
    FocusFileList,
    FocusDiff,
    ToggleFocus,
    SelectFile,

    // Review actions
    ToggleReviewed,
    AddLineComment,
    AddFileComment,
    EditComment,
    DeleteComment,

    // Session
    Quit,

    // Mode changes
    EnterCommandMode,
    ExitMode,
    ToggleHelp,

    // Text input
    InsertChar(char),
    DeleteChar,
    DeleteWord,
    ClearLine,
    SubmitInput,

    // Confirm dialog
    ConfirmYes,
    ConfirmNo,

    // No-op
    None,
}

pub fn map_key_to_action(key: KeyEvent, mode: InputMode) -> Action {
    match mode {
        InputMode::Normal => map_normal_mode(key),
        InputMode::Command => map_command_mode(key),
        InputMode::Comment => map_comment_mode(key),
        InputMode::Help => map_help_mode(key),
        InputMode::Confirm => map_confirm_mode(key),
    }
}

fn map_normal_mode(key: KeyEvent) -> Action {
    match (key.code, key.modifiers) {
        // Cursor movement (vim-like: cursor moves, scroll follows when needed)
        (KeyCode::Char('j') | KeyCode::Down, KeyModifiers::NONE) => Action::CursorDown(1),
        (KeyCode::Char('k') | KeyCode::Up, KeyModifiers::NONE) => Action::CursorUp(1),
        (KeyCode::Char('d'), KeyModifiers::CONTROL) => Action::HalfPageDown,
        (KeyCode::Char('u'), KeyModifiers::CONTROL) => Action::HalfPageUp,
        (KeyCode::Char('f'), KeyModifiers::CONTROL) => Action::PageDown,
        (KeyCode::Char('b'), KeyModifiers::CONTROL) => Action::PageUp,
        (KeyCode::Char('g'), KeyModifiers::NONE) => Action::GoToTop,
        (KeyCode::Char('G'), _) => Action::GoToBottom,
        (KeyCode::Char('z'), KeyModifiers::NONE) => Action::PendingZCommand,

        // File navigation (use _ for modifiers since shift is implicit in the character)
        (KeyCode::Char('}'), _) => Action::NextFile,
        (KeyCode::Char('{'), _) => Action::PrevFile,
        (KeyCode::Char(']'), _) => Action::NextHunk,
        (KeyCode::Char('['), _) => Action::PrevHunk,

        // Panel focus
        (KeyCode::Tab, KeyModifiers::NONE) => Action::ToggleFocus,
        (KeyCode::Char('h') | KeyCode::Left, KeyModifiers::NONE) => Action::FocusFileList,
        (KeyCode::Char('l') | KeyCode::Right, KeyModifiers::NONE) => Action::FocusDiff,
        (KeyCode::Enter, KeyModifiers::NONE) => Action::SelectFile,

        // Review actions
        (KeyCode::Char('r'), KeyModifiers::NONE) => Action::ToggleReviewed,
        (KeyCode::Char('c'), KeyModifiers::NONE) => Action::AddLineComment,
        (KeyCode::Char('C'), _) => Action::AddFileComment,
        (KeyCode::Char('e'), KeyModifiers::NONE) => Action::EditComment,
        (KeyCode::Char('d'), KeyModifiers::NONE) => Action::DeleteComment,

        // Mode changes (use _ for shifted characters like : and ?)
        (KeyCode::Char(':'), _) => Action::EnterCommandMode,
        (KeyCode::Char('?'), _) => Action::ToggleHelp,
        (KeyCode::Esc, KeyModifiers::NONE) => Action::ExitMode,

        // Quick quit
        (KeyCode::Char('q'), KeyModifiers::NONE) => Action::Quit,

        _ => Action::None,
    }
}

fn map_command_mode(key: KeyEvent) -> Action {
    match (key.code, key.modifiers) {
        (KeyCode::Esc, KeyModifiers::NONE) => Action::ExitMode,
        (KeyCode::Enter, KeyModifiers::NONE) => Action::SubmitInput,
        (KeyCode::Backspace, KeyModifiers::NONE) => Action::DeleteChar,
        (KeyCode::Char('w'), KeyModifiers::CONTROL) => Action::DeleteWord,
        (KeyCode::Char('u'), KeyModifiers::CONTROL) => Action::ClearLine,
        (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => Action::InsertChar(c),
        _ => Action::None,
    }
}

fn map_comment_mode(key: KeyEvent) -> Action {
    match (key.code, key.modifiers) {
        // Cancel: Esc, Ctrl+C, Ctrl+D
        (KeyCode::Esc, KeyModifiers::NONE) => Action::ExitMode,
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => Action::ExitMode,
        (KeyCode::Char('d'), KeyModifiers::CONTROL) => Action::ExitMode,
        // Submit: Ctrl+Enter, Ctrl+S, Shift+Enter
        (KeyCode::Enter, KeyModifiers::CONTROL) => Action::SubmitInput,
        (KeyCode::Char('s'), KeyModifiers::CONTROL) => Action::SubmitInput,
        (KeyCode::Enter, KeyModifiers::SHIFT) => Action::SubmitInput,
        // Editing
        (KeyCode::Backspace, KeyModifiers::NONE) => Action::DeleteChar,
        (KeyCode::Char('w'), KeyModifiers::CONTROL) => Action::DeleteWord,
        (KeyCode::Char('u'), KeyModifiers::CONTROL) => Action::ClearLine,
        (KeyCode::Char(c), _) => Action::InsertChar(c),
        (KeyCode::Enter, KeyModifiers::NONE) => Action::InsertChar('\n'),
        _ => Action::None,
    }
}

fn map_help_mode(key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => Action::ToggleHelp,
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
