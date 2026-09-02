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
    NextComment,
    PrevComment,
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
    ToggleHunkReviewed,
    AddLineComment,
    AddFileComment,
    EditComment,
    /// Edit the comment at cursor with the text cursor at end (vim `A`).
    EditCommentAtEnd,
    PendingDCommand,
    EditFile,
    /// Comment mode: hand the draft to `$EDITOR` with the commented hunk as
    /// context.
    EditCommentDraft,
    SearchNext,
    SearchPrev,
    ClearSearchHighlight,

    // Visual selection mode
    EnterVisualMode,
    AddRangeComment,

    // Session
    Quit,
    /// Transitional hint for the removed `q`-quits binding: shows a message
    /// pointing at `:q` instead of quitting. Drop this a few releases after
    /// the `q` removal has had time to land.
    QuitHint,
    ExportToClipboard,
    /// Copy just the comment under the cursor (`Y`), not the whole review.
    CopyCommentAtCursor,

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
    /// Toggle PR tab between all open PRs and PRs awaiting your review (`r`).
    TogglePrReviewRequestedFilter,

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

    // File tree filters (only produced while the file tree is focused, see
    // `map_file_tree_mode`)
    /// `i` — open the include-regex prompt.
    FileTreeFilterInclude,
    /// `e` — open the exclude-regex prompt.
    FileTreeFilterExclude,
    /// `I` — drop the include filter.
    FileTreeClearInclude,
    /// `E` — drop the exclude filter.
    FileTreeClearExclude,
    /// `/` — open the file-tree search prompt.
    FileTreeSearch,

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
        InputMode::MessageDetails => match map_help_mode(key) {
            Action::EnterSearchMode | Action::SearchNext | Action::SearchPrev => Action::None,
            action => action,
        },
        InputMode::Summary => map_summary_mode(key),
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
        (KeyCode::Char('m'), KeyModifiers::NONE) => Action::NextComment,
        (KeyCode::Char('M'), _) => Action::PrevComment,
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
        (KeyCode::Char('R'), _) => Action::ToggleHunkReviewed,
        (KeyCode::Char('c'), KeyModifiers::NONE) => Action::AddLineComment,
        (KeyCode::Char('C'), _) => Action::AddFileComment,
        (KeyCode::Char('i'), KeyModifiers::NONE) => Action::EditComment,
        (KeyCode::Char('A'), _) => Action::EditCommentAtEnd,
        (KeyCode::Char('d'), KeyModifiers::NONE) => Action::PendingDCommand,
        (KeyCode::Char('v') | KeyCode::Char('V'), _) => Action::EnterVisualMode,
        (KeyCode::Char('y'), KeyModifiers::NONE) => Action::ExportToClipboard,
        (KeyCode::Char('Y'), _) => Action::CopyCommentAtCursor,
        (KeyCode::Char('e'), KeyModifiers::NONE) => Action::EditFile,
        (KeyCode::Char('n'), KeyModifiers::NONE) => Action::SearchNext,
        (KeyCode::Char('N'), _) => Action::SearchPrev,

        // Mode changes (use _ for shifted characters like : and ?)
        (KeyCode::Char(':'), _) => Action::EnterCommandMode,
        (KeyCode::Char('/'), _) => Action::EnterSearchMode,
        (KeyCode::Char('?'), _) => Action::ToggleHelp,
        (KeyCode::Esc, KeyModifiers::NONE) => Action::ClearSearchHighlight,

        // Transitional hint: q used to quit; now it just points at :q.
        (KeyCode::Char('q'), KeyModifiers::NONE) => Action::QuitHint,

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
        // Newline insertion: multiple aliases to survive different terminal/tmux environments.
        // - Shift+Enter: works when Kitty keyboard protocol is active (modern terminals)
        // - Alt+Enter:   works in tmux even with vim-tmux-navigator (ESC CR, not intercepted)
        // - Ctrl+J:      works in terminals without tmux (0x0A → Char('j')+CONTROL)
        // - Ctrl+K:      works without vim-tmux-navigator, intercepted when navigator is active
        // vim-tmux-navigator binds C-j (down) and C-k (up), so Alt-Enter is the reliable tmux key.
        (KeyCode::Enter, mods) if mods.contains(KeyModifiers::SHIFT) => Action::InsertChar('\n'),
        (KeyCode::Enter, mods) if mods.contains(KeyModifiers::ALT) => Action::InsertChar('\n'),
        (KeyCode::Char('j'), KeyModifiers::CONTROL) => Action::InsertChar('\n'),
        (KeyCode::Char('k'), KeyModifiers::CONTROL) => Action::InsertChar('\n'),
        // Comment type: Tab to cycle (match any modifier so terminals that
        // report Tab with unexpected flags still work; Shift+Tab comes in as
        // BackTab via the second arm).
        // Also match Char('\t'): some terminal multiplexers (e.g. tmux without
        // extended-keys) send Tab as the raw byte 0x09 which crossterm maps to
        // Char('\t') instead of KeyCode::Tab.
        (KeyCode::Tab, _) | (KeyCode::Char('\t'), _) => Action::CycleCommentType,
        (KeyCode::BackTab, _) => Action::CycleCommentTypeReverse,
        // Compose the draft in `$EDITOR`. Ctrl-o rather than Ctrl-e, which is
        // already end-of-line here.
        (KeyCode::Char('o'), KeyModifiers::CONTROL) => Action::EditCommentDraft,
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
        (KeyCode::Char('b'), KeyModifiers::ALT) => Action::TextCursorWordLeft,
        (KeyCode::Char('f'), KeyModifiers::ALT) => Action::TextCursorWordRight,
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
        (KeyCode::Char('/'), _) => Action::EnterSearchMode,
        (KeyCode::Char('n'), KeyModifiers::NONE) => Action::SearchNext,
        (KeyCode::Char('N'), _) => Action::SearchPrev,
        _ => Action::None,
    }
}

fn map_summary_mode(key: KeyEvent) -> Action {
    match (key.code, key.modifiers) {
        (KeyCode::Esc, KeyModifiers::NONE) | (KeyCode::Char('q'), KeyModifiers::NONE) => {
            Action::ExitMode
        }
        (KeyCode::Char('j') | KeyCode::Down, KeyModifiers::NONE) => Action::CursorDown(1),
        (KeyCode::Char('k') | KeyCode::Up, KeyModifiers::NONE) => Action::CursorUp(1),
        (KeyCode::Enter, KeyModifiers::NONE) => Action::SubmitInput,
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
        (KeyCode::Char('q'), KeyModifiers::NONE) => Action::QuitHint,
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
        (KeyCode::Char('q'), KeyModifiers::NONE) => Action::QuitHint,
        (KeyCode::Char(':'), _) => Action::EnterCommandMode,
        (KeyCode::Tab, KeyModifiers::NONE) => Action::TargetSelectorTabNext,
        (KeyCode::BackTab, _) => Action::TargetSelectorTabPrev,
        (KeyCode::Char('/'), _) => Action::BeginTargetFilter,
        (KeyCode::Char('r'), KeyModifiers::NONE) => Action::TogglePrReviewRequestedFilter,
        _ => Action::None,
    }
}

/// Key map used when the file tree holds focus in `InputMode::Normal`.
///
/// Only the keys the tree claims for itself are listed; everything else
/// delegates to `map_normal_mode` so shared bindings keep working. `i` is the
/// notable override — in the diff it edits the comment at the cursor, in the
/// tree it opens the include filter.
pub fn map_file_tree_mode(key: KeyEvent, leader_key: char) -> Action {
    // The leader key wins: `;e` (toggle file list) must not be swallowed by
    // the exclude-filter binding.
    if key.code == KeyCode::Char(leader_key) && key.modifiers == KeyModifiers::NONE {
        return map_normal_mode(key, leader_key);
    }
    match (key.code, key.modifiers) {
        (KeyCode::Char('i'), KeyModifiers::NONE) => Action::FileTreeFilterInclude,
        (KeyCode::Char('e'), KeyModifiers::NONE) => Action::FileTreeFilterExclude,
        (KeyCode::Char('I'), _) => Action::FileTreeClearInclude,
        (KeyCode::Char('E'), _) => Action::FileTreeClearExclude,
        (KeyCode::Char('/'), _) => Action::FileTreeSearch,
        _ => map_normal_mode(key, leader_key),
    }
}

/// Key map used while a file-tree prompt (`i`/`e`/`/`) is collecting input.
/// A sub-state of `InputMode::Normal`; the dispatcher in `main.rs` routes here
/// when `App::file_tree_prompt_editing()` is true.
pub fn map_file_tree_prompt_mode(key: KeyEvent) -> Action {
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
        (KeyCode::Char('q'), KeyModifiers::NONE) => Action::QuitHint,
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
    fn should_map_filter_keys_when_the_file_tree_is_focused() {
        assert_eq!(
            map_file_tree_mode(key(KeyCode::Char('i')), DEFAULT_LEADER_KEY),
            Action::FileTreeFilterInclude
        );
        assert_eq!(
            map_file_tree_mode(key(KeyCode::Char('e')), DEFAULT_LEADER_KEY),
            Action::FileTreeFilterExclude
        );
        assert_eq!(
            map_file_tree_mode(key_shift('I'), DEFAULT_LEADER_KEY),
            Action::FileTreeClearInclude
        );
        assert_eq!(
            map_file_tree_mode(key_shift('E'), DEFAULT_LEADER_KEY),
            Action::FileTreeClearExclude
        );
        assert_eq!(
            map_file_tree_mode(key(KeyCode::Char('/')), DEFAULT_LEADER_KEY),
            Action::FileTreeSearch
        );
    }

    #[test]
    fn should_leave_shift_h_unbound_for_vim_motions() {
        // Reviewed-file visibility is command-only (`:set reviewed!`). `H` is a
        // vim motion, and this project deliberately does not spend new
        // single-stroke keys, so nothing may claim it.
        assert_eq!(
            map_file_tree_mode(key_shift('H'), DEFAULT_LEADER_KEY),
            Action::None
        );
        assert_eq!(
            map_normal_mode(key_shift('H'), DEFAULT_LEADER_KEY),
            Action::None
        );
    }

    #[test]
    fn should_keep_lowercase_h_panning_when_the_file_tree_is_focused() {
        // `h`/`l` pan the tree horizontally; the tree must not claim them.
        assert_eq!(
            map_file_tree_mode(key(KeyCode::Char('h')), DEFAULT_LEADER_KEY),
            Action::ScrollLeft(4)
        );
    }

    #[test]
    fn should_leave_diff_bindings_alone_when_the_tree_is_not_focused() {
        // `i` edits the comment at the cursor in the diff; only the tree
        // reinterprets it as the include filter.
        assert_eq!(
            map_normal_mode(key(KeyCode::Char('i')), DEFAULT_LEADER_KEY),
            Action::EditComment
        );
        assert_eq!(
            map_normal_mode(key(KeyCode::Char('/')), DEFAULT_LEADER_KEY),
            Action::EnterSearchMode
        );
    }

    #[test]
    fn should_delegate_unclaimed_keys_to_normal_mode_in_file_tree_mode() {
        for code in [
            KeyCode::Char('j'),
            KeyCode::Char('r'),
            KeyCode::Char('c'),
            KeyCode::Char(' '),
            KeyCode::Enter,
        ] {
            assert_eq!(
                map_file_tree_mode(key(code), DEFAULT_LEADER_KEY),
                map_normal_mode(key(code), DEFAULT_LEADER_KEY),
                "{code:?} should keep its normal-mode meaning in the tree"
            );
        }
    }

    #[test]
    fn should_let_the_leader_key_win_over_a_filter_binding() {
        // `leader = "e"` would otherwise be swallowed by the exclude filter.
        assert_eq!(
            map_file_tree_mode(key(KeyCode::Char('e')), 'e'),
            Action::PendingLeaderCommand
        );
    }

    #[test]
    fn should_route_typed_chars_to_insert_in_file_tree_prompt_mode() {
        assert_eq!(
            map_file_tree_prompt_mode(key(KeyCode::Char('a'))),
            Action::InsertChar('a')
        );
        // Keys that would otherwise act (/ filters) are just text
        // while the prompt is open.
        assert_eq!(
            map_file_tree_prompt_mode(key(KeyCode::Char('q'))),
            Action::InsertChar('q')
        );
        assert_eq!(
            map_file_tree_prompt_mode(key(KeyCode::Enter)),
            Action::SubmitInput
        );
        assert_eq!(
            map_file_tree_prompt_mode(key(KeyCode::Esc)),
            Action::ExitMode
        );
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
    fn should_map_i_and_shift_a_to_edit_comment_actions() {
        assert_eq!(
            map_normal_mode(key(KeyCode::Char('i')), DEFAULT_LEADER_KEY),
            Action::EditComment
        );
        assert_eq!(
            map_normal_mode(key_shift('A'), DEFAULT_LEADER_KEY),
            Action::EditCommentAtEnd
        );
    }

    #[test]
    fn should_map_lowercase_and_uppercase_y_to_separate_yank_actions() {
        assert_eq!(
            map_normal_mode(key(KeyCode::Char('y')), DEFAULT_LEADER_KEY),
            Action::ExportToClipboard
        );
        assert_eq!(
            map_normal_mode(key_shift('Y'), DEFAULT_LEADER_KEY),
            Action::CopyCommentAtCursor
        );
    }

    #[test]
    fn should_map_uppercase_g_to_go_to_bottom_in_normal_mode() {
        let action = map_normal_mode(key_shift('G'), DEFAULT_LEADER_KEY);
        assert_eq!(action, Action::GoToBottom);
    }

    #[test]
    fn should_map_slash_and_match_navigation_in_help_mode() {
        assert_eq!(
            map_help_mode(key(KeyCode::Char('/'))),
            Action::EnterSearchMode
        );
        assert_eq!(map_help_mode(key(KeyCode::Char('n'))), Action::SearchNext);
        assert_eq!(map_help_mode(key_shift('N')), Action::SearchPrev);
    }

    #[test]
    fn should_reuse_help_navigation_without_search_for_message_details() {
        assert_eq!(
            map_key_to_action(
                key(KeyCode::Char('j')),
                InputMode::MessageDetails,
                DEFAULT_LEADER_KEY,
            ),
            Action::CursorDown(1)
        );
        for key in [
            key(KeyCode::Char('/')),
            key(KeyCode::Char('n')),
            key_shift('N'),
        ] {
            assert_eq!(
                map_key_to_action(key, InputMode::MessageDetails, DEFAULT_LEADER_KEY),
                Action::None
            );
        }
    }

    #[test]
    fn should_map_summary_navigation_and_escape() {
        assert_eq!(map_summary_mode(key(KeyCode::Esc)), Action::ExitMode);
        assert_eq!(map_summary_mode(key(KeyCode::Char('q'))), Action::ExitMode);
        assert_eq!(
            map_summary_mode(key(KeyCode::Char('j'))),
            Action::CursorDown(1)
        );
        assert_eq!(
            map_summary_mode(key(KeyCode::Char('k'))),
            Action::CursorUp(1)
        );
        assert_eq!(map_summary_mode(key(KeyCode::Enter)), Action::SubmitInput);
        assert_eq!(map_summary_mode(key_shift('G')), Action::GoToBottom);
    }

    #[test]
    fn should_map_lowercase_g_to_go_to_top_in_normal_mode() {
        let action = map_normal_mode(key(KeyCode::Char('g')), DEFAULT_LEADER_KEY);
        assert_eq!(action, Action::GoToTop);
    }

    #[test]
    fn should_map_lowercase_e_to_edit_file_in_normal_mode() {
        let action = map_normal_mode(key(KeyCode::Char('e')), DEFAULT_LEADER_KEY);
        assert_eq!(action, Action::EditFile);
    }

    #[test]
    fn should_map_ctrl_o_to_editor_handoff_in_comment_mode() {
        let action = map_comment_mode(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
        assert_eq!(action, Action::EditCommentDraft);
    }

    #[test]
    fn should_keep_ctrl_e_as_line_end_in_comment_mode() {
        let action = map_comment_mode(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL));
        assert_eq!(action, Action::TextCursorLineEnd);
    }

    #[test]
    fn should_map_escape_to_clear_search_highlight_in_normal_mode() {
        let action = map_normal_mode(key(KeyCode::Esc), DEFAULT_LEADER_KEY);
        assert_eq!(action, Action::ClearSearchHighlight);
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
    fn should_map_uppercase_r_to_toggle_hunk_reviewed_in_normal_mode() {
        let action = map_normal_mode(key_shift('R'), DEFAULT_LEADER_KEY);
        assert_eq!(action, Action::ToggleHunkReviewed);
    }

    #[test]
    fn should_map_m_to_comment_navigation_in_normal_mode() {
        let action = map_normal_mode(key(KeyCode::Char('m')), DEFAULT_LEADER_KEY);
        assert_eq!(action, Action::NextComment);

        let action = map_normal_mode(key_shift('M'), DEFAULT_LEADER_KEY);
        assert_eq!(action, Action::PrevComment);
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
    fn should_map_tab_to_cycle_comment_type_in_comment_mode() {
        let action = map_comment_mode(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(action, Action::CycleCommentType);
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
    fn should_map_colon_to_command_mode_in_commit_select_mode() {
        let action = map_commit_select_mode(key(KeyCode::Char(':')));
        assert_eq!(action, Action::EnterCommandMode);
    }

    #[test]
    fn should_map_r_to_review_requested_filter_in_commit_select_mode() {
        // given / when
        let action = map_commit_select_mode(key(KeyCode::Char('r')));
        // then
        assert_eq!(action, Action::TogglePrReviewRequestedFilter);
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

    #[test]
    fn should_map_q_to_quit_hint_instead_of_quitting() {
        // `q` only quits via `:q` now; the modes that used to bind it to
        // Action::Quit now show a transitional hint instead.
        assert_eq!(
            map_normal_mode(key(KeyCode::Char('q')), DEFAULT_LEADER_KEY),
            Action::QuitHint
        );
        assert_eq!(map_visual_mode(key(KeyCode::Char('q'))), Action::QuitHint);
        assert_eq!(
            map_commit_select_mode(key(KeyCode::Char('q'))),
            Action::QuitHint
        );
        assert_eq!(
            map_submit_action_picker_mode(key(KeyCode::Char('q'))),
            Action::QuitHint
        );
        // The overlays keep their own `q`, which closes them rather than tuicr.
        assert_eq!(map_help_mode(key(KeyCode::Char('q'))), Action::ToggleHelp);
        assert_eq!(map_summary_mode(key(KeyCode::Char('q'))), Action::ExitMode);
    }
}
