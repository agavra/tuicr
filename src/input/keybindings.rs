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
    ScrollLeft(usize),
    ScrollRight(usize),

    // Panel focus
    ToggleFocus,
    SelectFile,

    // Review actions
    ToggleReviewed,
    AddLineComment,
    AddFileComment,
    EditComment,
    PendingDCommand,
    ToggleDiffView,

    // Session
    Quit,
    ExportToClipboard,

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
    TextCursorLeft,
    TextCursorRight,

    // Comment type
    CycleCommentType,

    // Confirm dialog
    ConfirmYes,
    ConfirmNo,

    // Commit selection
    CommitSelectUp,
    CommitSelectDown,
    ToggleCommitSelect,
    ConfirmCommitSelect,

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
        InputMode::CommitSelect => map_commit_select_mode(key),
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
        (KeyCode::Enter, KeyModifiers::NONE) => Action::SelectFile,

        // Horizontal scrolling
        (KeyCode::Char('h') | KeyCode::Left, KeyModifiers::NONE) => Action::ScrollLeft(4),
        (KeyCode::Char('l') | KeyCode::Right, KeyModifiers::NONE) => Action::ScrollRight(4),

        // Review actions
        (KeyCode::Char('r'), KeyModifiers::NONE) => Action::ToggleReviewed,
        (KeyCode::Char('c'), KeyModifiers::NONE) => Action::AddLineComment,
        (KeyCode::Char('C'), _) => Action::AddFileComment,
        (KeyCode::Char('i'), KeyModifiers::NONE) => Action::EditComment,
        (KeyCode::Char('d'), KeyModifiers::NONE) => Action::PendingDCommand,
        (KeyCode::Char('v'), KeyModifiers::NONE) => Action::ToggleDiffView,
        (KeyCode::Char('y'), KeyModifiers::NONE) => Action::ExportToClipboard,

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
        // Cursor movement
        (KeyCode::Left, KeyModifiers::NONE) => Action::TextCursorLeft,
        (KeyCode::Right, KeyModifiers::NONE) => Action::TextCursorRight,
        // Editing
        (KeyCode::Backspace, KeyModifiers::NONE) => Action::DeleteChar,
        (KeyCode::Char('w'), KeyModifiers::CONTROL) => Action::DeleteWord,
        (KeyCode::Char('u'), KeyModifiers::CONTROL) => Action::ClearLine,
        (KeyCode::Char(c), _) => Action::InsertChar(c),
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

fn map_commit_select_mode(key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => Action::CommitSelectDown,
        KeyCode::Char('k') | KeyCode::Up => Action::CommitSelectUp,
        KeyCode::Char(' ') => Action::ToggleCommitSelect,
        KeyCode::Enter => Action::ConfirmCommitSelect,
        KeyCode::Char('q') | KeyCode::Esc => Action::Quit,
        _ => Action::None,
    }
}
