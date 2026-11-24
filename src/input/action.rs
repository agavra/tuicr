//! The action vocabulary shared by key bindings and `:` commands.
//!
//! Every user-facing behavior is an [`Action`]. Keys and commands both
//! resolve to one, and a handler for the active context performs it. Actions
//! listed in [`ACTIONS`] carry a stable kebab-case name, which is what the
//! keymap config refers to. Variants without a spec cannot be bound:
//! internal input events (typed text, paste, mouse), prefix keys and digits,
//! and the actions that repeat a listed one for a single mode
//! (`CommitSelectDown`, `SubmitPickerConfirm`, `FileTreeSearch`).

/// A single user intent, independent of the key or command that produced it.
///
/// Motion variants carry a step size. A count typed before the key multiplies
/// it (see [`CountMode::Scale`]).
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
    /// Jump to the new-side source line given by the count (`{N}G`).
    GoToLine,
    /// Jump to the old-side source line given by the count.
    GoToOldLine,
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
    SearchNext,
    SearchPrev,
    ClearSearchHighlight,

    // Visual selection mode
    EnterVisualMode,
    AddRangeComment,
    AddRangeSuggestion,

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

    // Screen
    /// Clear the terminal and repaint every cell on the next frame (`Ctrl-l`).
    Redraw,

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
    TextCursorUp,
    TextCursorDown,
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
    /// Sessions tab: show or hide sessions holding no review progress.
    SessionsTabToggleShowEmpty,
    /// Sessions tab: delete the session under the cursor, after a confirm.
    SessionsTabDelete,

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

    /// `/` inside the theme picker — open its filter prompt.
    ThemePickerFilter,
    /// `/` inside the file picker — open its filter prompt.
    FilePickerFilter,

    // No-op
    None,
}

/// How a count typed before a key (`5j`) applies to the action it selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CountMode {
    /// The count is dropped.
    Ignore,
    /// The count multiplies the action's step size.
    Scale,
    /// The action runs once per count.
    Repeat,
    /// With a count, the action becomes [`Action::GoToLine`].
    Line,
    /// The handler reads the count itself.
    Pass,
}

/// An action paired with the `{N}` count typed before it, if any.
///
/// Keys and `:` commands both produce one. `:42` is
/// `Invocation { action: GoToLine, count: Some(42) }`, the same as `42G`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    pub action: Action,
    pub count: Option<usize>,
}

impl Invocation {
    pub fn new(action: Action, count: Option<usize>) -> Self {
        Self { action, count }
    }

    /// Apply the count through the action's [`CountMode`]. Returns the
    /// invocation to run and how many times to run it. The returned count is
    /// kept only for actions that read it themselves.
    pub fn resolve(self) -> (Invocation, usize) {
        let Some(count) = self.count else {
            return (self, 1);
        };
        let count = count.max(1);
        let action = self.action;
        match action.count_mode() {
            CountMode::Ignore => (Invocation::from(action), 1),
            CountMode::Scale => (Invocation::from(action.scaled(count)), 1),
            CountMode::Repeat => (Invocation::from(action), count),
            CountMode::Line => (Invocation::new(Action::GoToLine, Some(count)), 1),
            CountMode::Pass => (Invocation::new(action, Some(count)), 1),
        }
    }
}

impl From<Action> for Invocation {
    fn from(action: Action) -> Self {
        Self::new(action, None)
    }
}

/// Registry entry for a bindable action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionSpec {
    /// The action, with its default step size where it has one.
    pub action: Action,
    /// Stable kebab-case identifier used by the keymap config.
    pub name: &'static str,
    pub count: CountMode,
    /// One-line description for the help popup.
    pub help: &'static str,
}

const fn spec(
    action: Action,
    name: &'static str,
    count: CountMode,
    help: &'static str,
) -> ActionSpec {
    ActionSpec {
        action,
        name,
        count,
        help,
    }
}

use Action as A;
use CountMode::{Ignore, Line, Pass, Repeat, Scale};

/// Every bindable action. Order is the help popup's order within a context.
#[rustfmt::skip]
pub const ACTIONS: &[ActionSpec] = &[
    // Navigation
    spec(A::CursorDown(1), "cursor-down", Scale, "Move down"),
    spec(A::CursorUp(1), "cursor-up", Scale, "Move up"),
    spec(A::ScrollViewDown(1), "scroll-view-down", Scale, "Scroll view down"),
    spec(A::ScrollViewUp(1), "scroll-view-up", Scale, "Scroll view up"),
    spec(A::HalfPageDown, "half-page-down", Ignore, "Half page down"),
    spec(A::HalfPageUp, "half-page-up", Ignore, "Half page up"),
    spec(A::PageDown, "page-down", Ignore, "Full page down"),
    spec(A::PageUp, "page-up", Ignore, "Full page up"),
    spec(A::ScrollLeft(4), "scroll-left", Scale, "Scroll left"),
    spec(A::ScrollRight(4), "scroll-right", Scale, "Scroll right"),
    spec(A::GoToTop, "go-to-top", Ignore, "Go to top"),
    spec(A::GoToBottom, "go-to-bottom", Line, "Go to bottom ({N} jumps to line N)"),
    spec(A::GoToLine, "go-to-line", Pass, "Go to new-side line {N}"),
    spec(A::GoToOldLine, "go-to-old-line", Pass, "Go to old-side line {N}"),
    spec(A::NextFile, "next-file", Repeat, "Next file"),
    spec(A::PrevFile, "prev-file", Repeat, "Previous file"),
    spec(A::NextHunk, "next-hunk", Repeat, "Next hunk"),
    spec(A::PrevHunk, "prev-hunk", Repeat, "Previous hunk"),
    spec(A::NextComment, "next-comment", Ignore, "Next comment"),
    spec(A::PrevComment, "prev-comment", Ignore, "Previous comment"),
    spec(A::CycleCommitNext, "next-commit", Ignore, "Show next commit"),
    spec(A::CycleCommitPrev, "prev-commit", Ignore, "Show previous commit"),
    // Panels and lists
    spec(A::ToggleFocus, "focus-next", Ignore, "Focus next pane"),
    spec(A::ToggleFocusReverse, "focus-prev", Ignore, "Focus previous pane"),
    spec(A::SelectFile, "select", Ignore, "Open or activate the item under the cursor"),
    spec(A::SelectFileFull, "select-full", Ignore, "Expand the whole gap under the cursor"),
    spec(A::ToggleExpand, "toggle-expand", Ignore, "Toggle directory or gap"),
    spec(A::ExpandAll, "expand-all", Ignore, "Expand all directories"),
    spec(A::CollapseAll, "collapse-all", Ignore, "Collapse all directories"),
    spec(A::FileTreeFilterInclude, "filter-include", Ignore, "Include files matching a regex"),
    spec(A::FileTreeFilterExclude, "filter-exclude", Ignore, "Exclude files matching a regex"),
    spec(A::FileTreeClearInclude, "clear-include", Ignore, "Clear the include filter"),
    spec(A::FileTreeClearExclude, "clear-exclude", Ignore, "Clear the exclude filter"),
    // Review
    spec(A::ToggleReviewed, "toggle-reviewed", Ignore, "Toggle file reviewed"),
    spec(A::ToggleHunkReviewed, "toggle-hunk-reviewed", Ignore, "Toggle hunk reviewed"),
    spec(A::AddLineComment, "comment-line", Ignore, "Comment on the cursor line"),
    spec(A::AddFileComment, "comment-file", Ignore, "Comment on the file"),
    spec(A::EditComment, "edit-comment", Ignore, "Edit the comment at cursor"),
    spec(A::EditCommentAtEnd, "edit-comment-at-end", Ignore, "Edit the comment at cursor, text cursor at end"),
    spec(A::EditFile, "edit-file", Ignore, "Open the focused file in $EDITOR"),
    spec(A::EnterVisualMode, "visual-mode", Ignore, "Start a visual selection"),
    spec(A::AddRangeComment, "comment-range", Ignore, "Comment on the selection"),
    spec(A::AddRangeSuggestion, "suggest-range", Ignore, "Suggest a change to the selection"),
    spec(A::ExportToClipboard, "export", Ignore, "Copy the review to the clipboard"),
    spec(A::CopyCommentAtCursor, "copy-comment", Ignore, "Copy the comment at cursor"),
    // Search
    spec(A::EnterSearchMode, "search", Ignore, "Search"),
    spec(A::SearchNext, "search-next", Ignore, "Next search match"),
    spec(A::SearchPrev, "search-prev", Ignore, "Previous search match"),
    spec(A::ClearSearchHighlight, "clear-search-highlight", Ignore, "Clear search highlight"),
    // Modes and session
    spec(A::EnterCommandMode, "command-mode", Ignore, "Enter command mode"),
    spec(A::ToggleHelp, "help", Ignore, "Toggle help"),
    spec(A::ExitMode, "exit", Ignore, "Close or cancel"),
    spec(A::Quit, "quit", Ignore, "Quit"),
    spec(A::Redraw, "redraw", Ignore, "Redraw the screen"),
    // Dialogs
    spec(A::ConfirmYes, "yes", Ignore, "Confirm"),
    spec(A::ConfirmNo, "no", Ignore, "Decline"),
    // Review target selector
    spec(A::TargetSelectorTabNext, "next-tab", Ignore, "Next tab"),
    spec(A::TargetSelectorTabPrev, "prev-tab", Ignore, "Previous tab"),
    spec(A::TogglePrReviewRequestedFilter, "toggle-review-requested", Ignore, "Toggle PRs awaiting your review"),
    spec(A::SessionsTabToggleShowEmpty, "toggle-empty-sessions", Ignore, "Show or hide empty sessions"),
    spec(A::SessionsTabDelete, "delete-session", Ignore, "Delete the session under the cursor"),
    // Submit flow
    spec(A::SubmitResolverAdvance, "submit-advance", Ignore, "Continue to submit confirmation"),
    spec(A::SubmitReloadPr, "reload-pr", Ignore, "Reload the pull request"),
];

impl Action {
    /// The registry entry for this action, or `None` for internal events.
    pub fn spec(&self) -> Option<&'static ActionSpec> {
        let id = std::mem::discriminant(self);
        ACTIONS
            .iter()
            .find(|spec| std::mem::discriminant(&spec.action) == id)
    }

    /// Look up a bindable action by its config name.
    pub fn from_name(name: &str) -> Option<Action> {
        ACTIONS
            .iter()
            .find(|spec| spec.name == name)
            .map(|spec| spec.action.clone())
    }

    pub fn name(&self) -> Option<&'static str> {
        self.spec().map(|spec| spec.name)
    }

    pub fn count_mode(&self) -> CountMode {
        self.spec().map_or(Ignore, |spec| spec.count)
    }

    /// Multiply a motion's step size by `count`. Other actions are unchanged.
    pub fn scaled(mut self, count: usize) -> Action {
        match &mut self {
            Action::CursorDown(n)
            | Action::CursorUp(n)
            | Action::ScrollLeft(n)
            | Action::ScrollRight(n)
            | Action::ScrollViewDown(n)
            | Action::ScrollViewUp(n) => *n = n.saturating_mul(count),
            _ => {}
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn should_give_every_action_a_unique_name() {
        let mut names = HashSet::new();
        for spec in ACTIONS {
            assert!(names.insert(spec.name), "duplicate name {}", spec.name);
        }
    }

    #[test]
    fn should_register_each_action_once() {
        let mut seen = HashSet::new();
        for spec in ACTIONS {
            assert!(
                seen.insert(std::mem::discriminant(&spec.action)),
                "{} registered twice",
                spec.name
            );
        }
    }

    #[test]
    fn should_use_kebab_case_names() {
        for spec in ACTIONS {
            assert!(
                spec.name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "{} is not kebab-case",
                spec.name
            );
        }
    }

    #[test]
    fn should_round_trip_names() {
        for spec in ACTIONS {
            let action = Action::from_name(spec.name).expect("registered name");
            assert_eq!(action.name(), Some(spec.name));
        }
    }

    #[test]
    fn should_leave_internal_events_unnamed() {
        assert_eq!(Action::InsertChar('x').name(), None);
        assert_eq!(Action::Paste("x".into()).name(), None);
        assert_eq!(Action::MouseScrollDown(3).name(), None);
        assert_eq!(Action::TextCursorLeft.name(), None);
        assert_eq!(Action::CycleCommentType.name(), None);
        assert_eq!(Action::None.name(), None);
    }

    #[test]
    fn should_run_once_without_a_count() {
        let (run, times) = Invocation::from(Action::NextFile).resolve();
        assert_eq!((run, times), (Invocation::from(Action::NextFile), 1));
    }

    #[test]
    fn should_resolve_each_count_mode() {
        let resolve = |action, count| Invocation::new(action, Some(count)).resolve();
        assert_eq!(
            resolve(Action::CursorDown(1), 5),
            (Invocation::from(Action::CursorDown(5)), 1)
        );
        assert_eq!(
            resolve(Action::NextHunk, 3),
            (Invocation::from(Action::NextHunk), 3)
        );
        assert_eq!(
            resolve(Action::GoToBottom, 42),
            (Invocation::new(Action::GoToLine, Some(42)), 1)
        );
        assert_eq!(
            resolve(Action::GoToOldLine, 7),
            (Invocation::new(Action::GoToOldLine, Some(7)), 1)
        );
        assert_eq!(
            resolve(Action::NextComment, 4),
            (Invocation::from(Action::NextComment), 1)
        );
    }

    #[test]
    fn should_treat_a_zero_count_as_one() {
        assert_eq!(
            Invocation::new(Action::NextFile, Some(0)).resolve(),
            (Invocation::from(Action::NextFile), 1)
        );
    }

    #[test]
    fn should_scale_motion_step_by_count() {
        assert_eq!(Action::CursorDown(1).scaled(5), Action::CursorDown(5));
        assert_eq!(Action::ScrollLeft(4).scaled(3), Action::ScrollLeft(12));
        assert_eq!(Action::NextFile.scaled(3), Action::NextFile);
    }
}
