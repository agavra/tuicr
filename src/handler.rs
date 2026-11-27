use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Position;

use crate::app::{
    self, App, CommandCompletionState, ExpandDirection, FileTreeItem, FileTreePrompt, FocusedPanel,
    GapCursorHit, InputMode, TargetTab, VisualSelection,
};
use crate::error::TuicrError;
use crate::forge::remote_comments::PrCommentsVisibility;
use crate::forge::submit::SubmitEvent;
use crate::input::{
    Action, Invocation, map_file_picker_filter_mode, map_file_tree_mode_with_q_quits,
    map_file_tree_prompt_mode, map_global_key, map_key_to_action_with_q_quits,
    map_target_filter_mode, map_theme_picker_filter_mode,
};
use crate::model::{ClearScope, LineSide};
use crate::output::{
    copy_text_to_clipboard_with, export_to_clipboard_with, export_to_file, generate_export_content,
};
use crate::text_edit::{
    delete_char_before, delete_word_before, next_char_boundary, prev_char_boundary,
};

/// Columns scrolled per horizontal mouse wheel tick. Matches the default
/// step for keyboard arrow scrolling so the two input methods feel
/// interchangeable.
const WHEEL_COLS: usize = 4;

/// Shown when a bare `q` is pressed in a mode that used to quit on it.
/// Transitional — drop this a few releases after the `q` removal has landed.
const QUIT_HINT_MESSAGE: &str = "q no longer quits — use :q to quit";

const COMMAND_SPECS: &[CommandSpec] = &[
    CommandSpec::new(&["q", "quit"], CommandKind::Quit),
    CommandSpec::new(&["q!", "quit!"], CommandKind::ForceQuit),
    CommandSpec::new(&["w", "write"], CommandKind::Write),
    CommandSpec::new(&["x", "wq"], CommandKind::WriteQuit),
    CommandSpec::new(&["e", "reload"], CommandKind::Reload),
    CommandSpec::new(&["edit"], CommandKind::Edit),
    CommandSpec::new(&["clip", "export"], CommandKind::Export),
    CommandSpec::new(&["clipq"], CommandKind::ExportForceQuit),
    CommandSpec::new(&["copy-url"], CommandKind::CopyUrl),
    CommandSpec::new(
        &["clear"],
        CommandKind::Clear(ClearScope::CommentsAndReviewed),
    ),
    CommandSpec::new(&["clearc"], CommandKind::Clear(ClearScope::CommentsOnly)),
    CommandSpec::new(&["help", "h"], CommandKind::Help),
    CommandSpec::new(&["messages"], CommandKind::MessageDetails),
    CommandSpec::new(&["version"], CommandKind::Version),
    CommandSpec::new(&["update"], CommandKind::Update),
    CommandSpec::new(&["set wrap"], CommandKind::SetWrap),
    CommandSpec::new(&["set wrap!"], CommandKind::ToggleWrap),
    CommandSpec::new(&["wrap"], CommandKind::ToggleWrap),
    CommandSpec::new(
        &["set relativenumber"],
        CommandKind::SetRelativeLineNumbers(true),
    ),
    CommandSpec::new(
        &["set norelativenumber"],
        CommandKind::SetRelativeLineNumbers(false),
    ),
    CommandSpec::new(
        &["set relativenumber!"],
        CommandKind::ToggleRelativeLineNumbers,
    ),
    CommandSpec::new(&["set worddiff"], CommandKind::SetWordDiff(true)),
    CommandSpec::new(&["set noworddiff"], CommandKind::SetWordDiff(false)),
    CommandSpec::new(&["set worddiff!"], CommandKind::ToggleWordDiff),
    CommandSpec::new(&["vim", "set vim!"], CommandKind::ToggleVim),
    CommandSpec::new(&["set vim"], CommandKind::SetVim(true)),
    CommandSpec::new(&["novim", "set novim"], CommandKind::SetVim(false)),
    CommandSpec::new(&["set commits"], CommandKind::SetCommitsVisible(true)),
    CommandSpec::new(&["set nocommits"], CommandKind::SetCommitsVisible(false)),
    CommandSpec::new(&["set commits!"], CommandKind::ToggleCommits),
    CommandSpec::new(&["set reviewed"], CommandKind::SetShowReviewed(true)),
    CommandSpec::new(&["set noreviewed"], CommandKind::SetShowReviewed(false)),
    CommandSpec::new(
        &["reviewed", "set reviewed!"],
        CommandKind::ToggleShowReviewed,
    ),
    CommandSpec::new(&["set minimal"], CommandKind::SetMinimalUi(true)),
    CommandSpec::new(&["set nominimal"], CommandKind::SetMinimalUi(false)),
    CommandSpec::new(&["set minimal!"], CommandKind::ToggleMinimalUi),
    CommandSpec::new(&["set generated"], CommandKind::SetGeneratedCollapsed(true)),
    CommandSpec::new(
        &["set nogenerated"],
        CommandKind::SetGeneratedCollapsed(false),
    ),
    CommandSpec::new(
        &["generated", "set generated!"],
        CommandKind::ToggleGenerated,
    ),
    CommandSpec::new(&["diff"], CommandKind::Diff),
    CommandSpec::new(&["focus", "f"], CommandKind::Focus),
    CommandSpec::new(&["stage"], CommandKind::Stage),
    CommandSpec::new(
        &["commits", "targets"],
        CommandKind::Targets(TargetTab::Local),
    ),
    CommandSpec::new(&["prs"], CommandKind::Targets(TargetTab::PullRequests)),
    CommandSpec::new(
        &["sessions", "reviews"],
        CommandKind::Targets(TargetTab::Sessions),
    ),
    CommandSpec::new(&["submit"], CommandKind::SubmitPicker),
    CommandSpec::new(
        &["submit comment"],
        CommandKind::Submit(SubmitEvent::Comment),
    ),
    CommandSpec::new(
        &["submit approve"],
        CommandKind::Submit(SubmitEvent::Approve),
    ),
    CommandSpec::new(
        &["submit request-changes"],
        CommandKind::Submit(SubmitEvent::RequestChanges),
    ),
    CommandSpec::new(&["submit draft"], CommandKind::Submit(SubmitEvent::Draft)),
    CommandSpec::new(&["summary"], CommandKind::Summary),
    CommandSpec::new(&["theme"], CommandKind::ThemePicker),
    CommandSpec::new(
        &["comments unresolved"],
        CommandKind::Comments(PrCommentsVisibility::Unresolved),
    ),
    CommandSpec::new(
        &["comments all"],
        CommandKind::Comments(PrCommentsVisibility::All),
    ),
    CommandSpec::new(
        &["comments hide"],
        CommandKind::Comments(PrCommentsVisibility::Hide),
    ),
];

/// CommandSpec is the single registry entry used by both completion and
/// command-mode dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CommandSpec {
    /// Accepted command strings, including aliases.
    names: &'static [&'static str],
    /// Behavior shared by every alias in `names`.
    kind: CommandKind,
}

impl CommandSpec {
    const fn new(names: &'static [&'static str], kind: CommandKind) -> Self {
        Self { names, kind }
    }
}

/// CommandKind is the parsed meaning of one command-mode input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommandKind {
    Quit,
    ForceQuit,
    Write,
    WriteQuit,
    Reload,
    Edit,
    Export,
    ExportForceQuit,
    CopyUrl,
    Clear(ClearScope),
    Help,
    MessageDetails,
    Summary,
    Version,
    Update,
    SetWrap,
    ToggleWrap,
    SetRelativeLineNumbers(bool),
    ToggleRelativeLineNumbers,
    SetWordDiff(bool),
    ToggleWordDiff,
    ToggleVim,
    SetVim(bool),
    SetCommitsVisible(bool),
    ToggleCommits,
    SetShowReviewed(bool),
    ToggleShowReviewed,
    SetMinimalUi(bool),
    ToggleMinimalUi,
    SetGeneratedCollapsed(bool),
    ToggleGenerated,
    Diff,
    Focus,
    Stage,
    Targets(TargetTab),
    SubmitPicker,
    Submit(SubmitEvent),
    Comments(PrCommentsVisibility),
    ThemePicker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommandAfterDispatch {
    ExitCommandMode,
    KeepMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompletionDirection {
    Forward,
    Reverse,
}

/// How long a first Ctrl+C waits for the second one that exits.
const CTRL_C_EXIT_TIMEOUT: Duration = Duration::from_secs(2);

/// Drop a Ctrl+C warning that has timed out.
/// Returns `true` when one was dropped, so the main loop can schedule a redraw.
pub fn clear_expired_ctrl_c(app: &mut App) -> bool {
    let expired = app
        .pending_ctrl_c
        .is_some_and(|first_press| first_press.elapsed() >= CTRL_C_EXIT_TIMEOUT);
    if expired {
        app.pending_ctrl_c = None;
        app.message = None;
    }
    expired
}

/// Handle a key release.
///
/// Down/Up Release flips the `*_released_since_arm` flag so the
/// primed two-press file walk in single-file view requires a
/// deliberate release + press; held-key auto-repeat (Repeat
/// events) never satisfies the gate. Terminals without kitty
/// REPORT_EVENT_TYPES support never emit Release, in which case
/// `supports_keyboard_enhancement` is false and `cursor_down` /
/// `cursor_up` never walk: j/k stop at the file's edges.
pub fn handle_key_release(app: &mut App, key: KeyEvent) {
    if matches!(key.code, KeyCode::Down | KeyCode::Char('j')) {
        app.down_released_since_arm = true;
    }
    if matches!(key.code, KeyCode::Up | KeyCode::Char('k')) {
        app.up_released_since_arm = true;
    }
}

/// Handle one key press or repeat.
///
/// Returns `true` when the frame must be drawn before the next queued key is read.
pub fn handle_key(app: &mut App, key: KeyEvent) -> bool {
    // Handle Ctrl+C twice to exit (works across all input modes)
    // In Comment mode, first Ctrl+C also cancels the comment
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        // If in comment mode, cancel the comment first
        if app.input_mode == InputMode::Comment {
            app.exit_comment_mode();
        }

        if let Some(first_press) = app.pending_ctrl_c
            && first_press.elapsed() < CTRL_C_EXIT_TIMEOUT
        {
            // Second Ctrl+C within timeout - exit immediately
            app.should_quit = true;
            return false;
        }
        // First Ctrl+C (or timeout expired) - show warning and start timer
        app.pending_ctrl_c = Some(Instant::now());
        app.set_message("Press Ctrl+C again to exit");
        return false;
    }

    // Any other key clears the pending Ctrl+C state and message
    if app.pending_ctrl_c.is_some() {
        app.pending_ctrl_c = None;
        app.message = None;
    }

    // Keys that work the same in every mode (`Ctrl-l` redraw).
    // Checked before the pending-prefix and vim-editor routing
    // so they never reach a text buffer.
    if handle_global_key(app, key) {
        return false;
    }

    // Handle pending z command for zz/zt/zb viewport positioning
    if app.pending_prefix.z {
        app.pending_prefix.z = false;
        match key.code {
            KeyCode::Char('z') => {
                app.center_cursor();
                return false;
            }
            KeyCode::Char('t') => {
                app.cursor_to_top();
                return false;
            }
            KeyCode::Char('b') => {
                app.cursor_to_bottom();
                return false;
            }
            _ => {} // Fall through to normal handling
        }
    }

    // Handle pending Z command for ZZ (export+quit) / ZQ (quit)
    if app.pending_prefix.shift_z {
        app.pending_prefix.shift_z = false;
        match key.code {
            KeyCode::Char('Z') => {
                // ZZ: save session, export, and quit (same as :wq)
                handle_write_and_quit(app);
                return false;
            }
            KeyCode::Char('Q') => {
                // ZQ: quit without exporting (same as q)
                app.should_quit = true;
                return false;
            }
            _ => {} // Fall through to normal handling
        }
    }

    // Handle pending d command for dd delete comment
    if app.pending_prefix.d {
        app.pending_prefix.d = false;
        if key.code == KeyCode::Char('d') && key.modifiers == KeyModifiers::NONE {
            // In the target selector `dd` deletes the saved
            // review under the cursor, not a comment.
            if app.input_mode == InputMode::CommitSelect {
                handle_commit_select_action(app, Action::SessionsTabDelete);
                return false;
            }
            if app.input_mode == InputMode::Summary {
                if !app.delete_selected_summary_comment() {
                    app.set_warning("Selected comment is no longer editable or available");
                }
                // Refresh IDs and clamp selection before another queued dd.
                return true;
            }
            if app.cursor_on_locked_comment() {
                let forge = app.forge_display_name();
                app.set_message(format!(
                    "Comment already pushed to {forge} — read only in tuicr"
                ));
            } else if !app.delete_comment_at_cursor() {
                if app.begin_remote_draft_delete() {
                    // Waiting on the confirmation prompt.
                } else if app.cursor_on_remote_thread() {
                    let forge = app.forge_display_name();
                    app.set_message(format!("{forge} comment — read only in tuicr"));
                } else {
                    app.set_message("No comment at cursor");
                }
            }
            return false;
        }
        // Otherwise fall through to normal handling
    }

    // Handle pending leader command for panel focus, file list toggle, and review comments.
    if app.pending_prefix.leader {
        app.pending_prefix.leader = false;
        match key.code {
            KeyCode::Char('e') => {
                app.toggle_file_list();
                return false;
            }
            KeyCode::Char('h') => {
                app.focus_pane_left();
                return false;
            }
            KeyCode::Char('l') => {
                app.focus_pane_right();
                return false;
            }
            KeyCode::Char('k') => {
                if app.focused_panel == FocusedPanel::Comments {
                    app.focused_panel = FocusedPanel::FileList;
                } else if app.has_inline_commit_selector() {
                    app.focused_panel = FocusedPanel::CommitSelector;
                }
                return false;
            }
            KeyCode::Char('j') => {
                if app.focused_panel == FocusedPanel::FileList && app.has_comment_navigator_items()
                {
                    app.focused_panel = FocusedPanel::Comments;
                } else {
                    app.focused_panel = FocusedPanel::Diff;
                }
                return false;
            }
            KeyCode::Char('c') => {
                app.enter_review_comment_mode();
                return false;
            }
            // `<leader>s` toggles the commit selector pane.
            KeyCode::Char('s') => {
                app.toggle_commit_selector();
                return false;
            }
            KeyCode::Char('f') => {
                app.toggle_single_file_view();
                return false;
            }
            KeyCode::Char('n') => {
                app.relative_line_numbers = !app.relative_line_numbers;
                return false;
            }
            KeyCode::Char('p') => {
                app.enter_file_picker_mode();
                return false;
            }
            _ => {}
        }
        // Otherwise fall through to normal handling
    }

    // Vim modal editing: route comment-box keys to the edtui
    // overlay (app-level keys handled inside).
    if app.input_mode == InputMode::Comment && app.comment_vim_enabled {
        app.ensure_comment_vim_editor();
        if handle_comment_vim_key(app, key) {
            return false;
        }
    }

    // Editing the PR-tab filter is a sub-state of CommitSelect;
    // route through the filter-specific key map so typed
    // characters update the filter buffer rather than driving
    // commit-list navigation.
    let action = map_normal_path_key(app, key);

    // Handle pending command setters (these work in any mode)
    match action {
        Action::PendingZCommand => {
            app.pending_prefix.z = true;
            app.pending_count = None;
            return false;
        }
        Action::PendingShiftZCommand => {
            app.pending_prefix.shift_z = true;
            app.pending_count = None;
            return false;
        }
        Action::PendingDCommand => {
            app.pending_prefix.d = true;
            app.pending_count = None;
            return false;
        }
        Action::PendingLeaderCommand => {
            app.pending_prefix.leader = true;
            app.pending_count = None;
            return false;
        }
        _ => {}
    }

    // Vim-style {count}{motion} (Normal mode only): digits accumulate
    // into `pending_count`, and the next action consumes it.
    let mut count = None;
    if app.input_mode == InputMode::Normal {
        if let Action::Digit(d) = action {
            let n = app.pending_count.unwrap_or(0);
            app.pending_count =
                Some((n.saturating_mul(10).saturating_add(d as usize)).min(999_999));
            return false;
        }
        count = app.pending_count.take();
    }

    let previous_mode = app.input_mode;
    dispatch_action(app, Invocation::new(action, count));
    if previous_mode != InputMode::Summary && app.input_mode == InputMode::Summary {
        // Summary identities are populated by rendering before queued input.
        return true;
    }
    false
}

/// Handle a comment-mode key while vim is active: app-level keys keep their
/// semantics, everything else feeds the overlay. Always returns `true`.
fn handle_comment_vim_key(app: &mut App, key: KeyEvent) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    // An open `:` command-line captures all input until Enter/Esc.
    if app.comment_vim_command_active() {
        match key.code {
            KeyCode::Enter => app.run_comment_vim_command(),
            KeyCode::Esc => app.comment_vim_command_cancel(),
            KeyCode::Backspace => app.comment_vim_command_backspace(),
            KeyCode::Char(c) if !ctrl && !key.modifiers.contains(KeyModifiers::ALT) => {
                app.comment_vim_command_push(c)
            }
            _ => {}
        }
        return true;
    }

    // Alt+Enter (Option+Enter) accepts (save) and Alt+Esc discards (cancel)
    // directly, in any mode — no double-press. Alt is the one modified Enter/Esc
    // that reaches the app across terminals, including browser/web terminals
    // like zellij web (where Shift/Cmd+Enter get stripped or grabbed). Plain
    // Enter still inserts a newline in Insert mode, so Alt+Enter is free here.
    if key.modifiers.contains(KeyModifiers::ALT) {
        match key.code {
            KeyCode::Enter => {
                app.save_comment();
                return true;
            }
            KeyCode::Esc => {
                app.exit_comment_mode();
                return true;
            }
            _ => {}
        }
    }

    let normal = app.comment_vim_in_normal_mode();

    // In Normal mode a first plain Enter/Esc arms a confirm (header shows the
    // hint); a second consecutive press saves (`:w`) / cancels (`:q`).
    if normal && key.modifiers.is_empty() {
        match key.code {
            KeyCode::Enter => {
                app.comment_vim_enter_normal();
                return true;
            }
            // `q` behaves exactly like Esc here (arm/confirm cancel).
            KeyCode::Esc | KeyCode::Char('q') => {
                app.comment_vim_esc_normal();
                return true;
            }
            _ => {}
        }
    }
    // Any other key breaks a pending double-press sequence.
    app.comment_vim_reset_pending();

    match key.code {
        // Save shortcut in any mode (Ctrl-C cancel is handled earlier).
        KeyCode::Char('s') if ctrl => app.save_comment(),
        KeyCode::Enter if ctrl => app.save_comment(),
        // Tab cycles the comment type in Normal mode; in Insert it inserts a
        // soft tab (`comment_tab_width` spaces).
        KeyCode::Tab | KeyCode::Char('\t') if normal => app.cycle_comment_type(),
        KeyCode::Tab | KeyCode::Char('\t') => app.comment_vim_insert_soft_tab(),
        KeyCode::BackTab if normal => app.cycle_comment_type_reverse(),
        // `:` opens the command-line in Normal mode (`:w` saves, `:q` cancels).
        // Esc/Enter in Normal fall through to edtui (Esc is a no-op there).
        KeyCode::Char(':') if normal => app.start_comment_vim_command(),
        _ => app.comment_vim_feed_key(key),
    }
    true
}

/// Run an invocation in the active context, applying its count.
pub fn dispatch_action(app: &mut App, invocation: impl Into<Invocation>) {
    let (invocation, times) = invocation.into().resolve();
    // A count repeats the action; the last run takes the invocation itself.
    for _ in 1..times {
        dispatch_once(app, invocation.clone());
    }
    dispatch_once(app, invocation);
}

fn dispatch_once(app: &mut App, invocation: Invocation) {
    if handle_mode_independent_action(app, &invocation) {
        return;
    }
    let action = invocation.action;
    match app.input_mode {
        InputMode::Help | InputMode::MessageDetails => handle_help_action(app, action),
        InputMode::Summary => handle_summary_action(app, action),
        InputMode::Command => handle_command_action(app, action),
        InputMode::Search => handle_search_action(app, action),
        InputMode::Comment => handle_comment_action(app, action),
        InputMode::Confirm => handle_confirm_action(app, action),
        InputMode::CommitSelect => handle_commit_select_action(app, action),
        InputMode::VisualSelect => handle_visual_action(app, action),
        InputMode::SubmitResolver => handle_submit_resolver_action(app, action),
        InputMode::SubmitConfirm => handle_submit_confirm_action(app, action),
        InputMode::SubmitActionPicker => handle_submit_action_picker_action(app, action),
        InputMode::ThemePicker => handle_theme_picker_action(app, action),
        InputMode::FilePicker => handle_file_picker_action(app, action),
        InputMode::Normal => handle_normal_action(app, action),
    }
}

/// Handle actions that behave the same in every mode.
/// Returns `true` when `action` was one of them.
fn handle_mode_independent_action(app: &mut App, invocation: &Invocation) -> bool {
    let Invocation { action, count } = invocation;
    match action {
        Action::GoToLine | Action::GoToOldLine => {
            let side = if *action == Action::GoToLine {
                LineSide::New
            } else {
                LineSide::Old
            };
            match count {
                Some(line) => app.go_to_source_line(*line as u32, side),
                None => app.set_message("Type a line number first, e.g. 42G"),
            }
        }
        _ => return false,
    }
    true
}

/// Handle a key that means the same thing in every input mode (see
/// `map_global_key`). Returns `true` when the key was consumed, so the caller
/// skips the per-mode maps.
pub fn handle_global_key(app: &mut App, key: KeyEvent) -> bool {
    match map_global_key(key) {
        Action::Redraw => {
            app.request_full_redraw();
            true
        }
        _ => false,
    }
}

/// Map a key press to an `Action` for the current mode. An open text prompt
/// (PR filter, file-tree prompt, theme/file picker filter) owns the keys first,
/// whatever panel has focus; the file tree has its own map otherwise.
pub fn map_normal_path_key(app: &App, key: KeyEvent) -> Action {
    if app.input_mode == InputMode::CommitSelect && app.pr_filter_editing() {
        map_target_filter_mode(key)
    } else if app.input_mode == InputMode::Normal && app.file_tree_prompt_editing() {
        map_file_tree_prompt_mode(key)
    } else if app.input_mode == InputMode::ThemePicker && app.theme_picker_filtering() {
        map_theme_picker_filter_mode(key)
    } else if app.input_mode == InputMode::FilePicker && app.file_picker_filtering() {
        map_file_picker_filter_mode(key)
    } else if app.input_mode == InputMode::Normal && app.focused_panel == FocusedPanel::FileList {
        map_file_tree_mode_with_q_quits(key, app.leader_key, app.q_quits)
    } else {
        map_key_to_action_with_q_quits(key, app.input_mode, app.leader_key, app.q_quits)
    }
}

pub fn handle_mouse_event(app: &mut App, event: MouseEvent) {
    let pos = Position::new(event.column, event.row);
    match event.kind {
        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
            let scroll_up = matches!(event.kind, MouseEventKind::ScrollUp);
            let action = if scroll_up {
                Action::MouseScrollUp(app.mouse_scroll_lines)
            } else {
                Action::MouseScrollDown(app.mouse_scroll_lines)
            };
            let over_file_list = app.file_list_area.is_some_and(|r| r.contains(pos));
            let over_diff = app.diff_area.is_some_and(|r| r.contains(pos));
            let over_commit_list = app.commit_list_inner_area.is_some_and(|r| r.contains(pos));
            match app.input_mode {
                InputMode::Help | InputMode::MessageDetails => handle_help_action(app, action),
                InputMode::Summary => handle_summary_action(app, action),
                InputMode::ThemePicker => {
                    app.move_theme_picker_selection(if scroll_up { -1 } else { 1 });
                }
                InputMode::FilePicker => {
                    app.move_file_picker_selection(if scroll_up { -1 } else { 1 });
                }
                InputMode::CommitSelect | InputMode::Normal if over_commit_list => {
                    wheel_commit_list(app, scroll_up);
                }
                InputMode::Normal if over_file_list => handle_file_list_action(app, action),
                InputMode::Normal
                    if app.comment_navigator_area.is_some_and(|r| r.contains(pos)) =>
                {
                    handle_comment_navigator_action(app, action)
                }
                InputMode::Normal if over_diff => handle_diff_action(app, action),
                InputMode::VisualSelect if over_diff => handle_diff_action(app, action),
                _ => {}
            }
            clear_visual_if_cursor_offscreen(app);
        }
        MouseEventKind::ScrollLeft | MouseEventKind::ScrollRight => {
            // Bypass the Action layer: trackpad swipes dispatch directly to
            // the diff viewport so they don't trigger keyboard focus-slide
            // (FEAT-0012) or abort an active VisualSelect.
            let scroll_left = matches!(event.kind, MouseEventKind::ScrollLeft);
            let over_diff = app.diff_area.is_some_and(|r| r.contains(pos));
            if over_diff && matches!(app.input_mode, InputMode::Normal | InputMode::VisualSelect) {
                if scroll_left {
                    app.scroll_left(WHEEL_COLS);
                } else {
                    app.scroll_right(WHEEL_COLS);
                }
            }
        }
        MouseEventKind::Down(MouseButton::Left)
            if matches!(app.input_mode, InputMode::Normal | InputMode::VisualSelect) =>
        {
            // Set focus based on the outer panel area (includes the
            // border) so clicks on dead space still move focus.
            if app.file_list_area.is_some_and(|r| r.contains(pos)) {
                app.focused_panel = FocusedPanel::FileList;
            } else if app.diff_area.is_some_and(|r| r.contains(pos)) {
                app.focused_panel = FocusedPanel::Diff;
            }
            // Reset back to Normal so an Up without drag falls through to handle_left_click.
            if app.input_mode == InputMode::VisualSelect {
                app.exit_visual_mode();
            }
            app.mouse_drag_active = false;
            if app.diff_inner_area.is_some_and(|r| r.contains(pos))
                && let Some(point) = app.cell_to_sel_point(pos.x, pos.y)
            {
                app.visual_selection = Some(VisualSelection::collapsed(point));
                if let Some(idx) = app.diff_annotation_at_screen_row(pos.y) {
                    app.move_cursor_to_annotation(idx);
                }
            } else {
                app.visual_selection = None;
                handle_left_click(app, pos);
            }
        }
        MouseEventKind::Down(MouseButton::Left) if app.input_mode == InputMode::CommitSelect => {
            if let Some(idx) = app.commit_list_idx_at_screen_row(pos.y) {
                app.commit_list_cursor = idx;
                handle_commit_select_action(app, Action::ToggleCommitSelect);
            }
        }
        MouseEventKind::Drag(MouseButton::Left)
            if matches!(app.input_mode, InputMode::Normal | InputMode::VisualSelect) =>
        {
            // An open file-tree prompt owns input; a drag must not start a
            // visual selection (it would strand the draft in VisualSelect).
            if app.file_tree_prompt_editing() {
                return;
            }
            let Some(sel) = app.visual_selection else {
                return;
            };
            let Some(mut head) = app.cell_to_sel_point(pos.x, pos.y) else {
                return;
            };
            // Pin head to the anchor's pane so cross-side drags don't escape.
            head.side = sel.anchor.side;
            if head == sel.head && app.mouse_drag_active {
                return;
            }
            let moved = head != sel.head;
            let promoted_now = moved && !app.mouse_drag_active;
            app.visual_selection = Some(VisualSelection {
                anchor: sel.anchor,
                head,
            });
            if moved {
                app.mouse_drag_active = true;
            }
            if promoted_now && app.input_mode == InputMode::Normal {
                app.input_mode = InputMode::VisualSelect;
            }
            if app.input_mode == InputMode::VisualSelect
                && head.annotation_idx != sel.head.annotation_idx
            {
                app.move_cursor_to_annotation(head.annotation_idx);
            }
        }
        MouseEventKind::Up(MouseButton::Left)
            if matches!(app.input_mode, InputMode::Normal | InputMode::VisualSelect) =>
        {
            if app.visual_selection.is_none() {
                return;
            }
            if !app.mouse_drag_active {
                app.visual_selection = None;
                if app.input_mode == InputMode::VisualSelect {
                    app.exit_visual_mode();
                }
                handle_left_click(app, pos);
            }
            app.mouse_drag_active = false;
        }
        _ => {}
    }
}

/// Helix-style: scrolling the cursor off-viewport drops the selection.
pub fn clear_visual_if_cursor_offscreen(app: &mut App) {
    if app.input_mode == InputMode::VisualSelect && !app.is_cursor_visible() {
        app.exit_visual_mode();
    }
}

/// Wheel scroll inside a commit list (full-screen picker or inline selector).
/// The list is short and selection-oriented, so each tick moves the cursor
/// rather than the viewport, matching how arrow keys behave.
fn wheel_commit_list(app: &mut App, scroll_up: bool) {
    // Match screen direction: the inline pane (Normal mode) reverses its
    // display in ascending order, so wheel-up moves toward newer (lower) data
    // indices there. The full-screen picker (CommitSelect) is always
    // newest-first.
    let scroll_up = if app.input_mode == InputMode::Normal && app.commits_ascending() {
        !scroll_up
    } else {
        scroll_up
    };
    for _ in 0..app.mouse_scroll_lines {
        if scroll_up {
            app.commit_select_up();
        } else {
            app.commit_select_down();
        }
    }
}

fn handle_left_click(app: &mut App, pos: Position) {
    if app.file_list_inner_area.is_some_and(|r| r.contains(pos))
        && let Some(idx) = app.file_list_idx_at_screen_row(pos.y)
    {
        app.focused_panel = FocusedPanel::FileList;
        app.file_list_state.select(idx);
        if let Some(item) = app.build_visible_items().get(idx).cloned() {
            match item {
                FileTreeItem::Directory { path, .. } => app.toggle_directory(&path),
                FileTreeItem::File { file_idx, .. } => {
                    app.jump_to_file(file_idx);
                    app.focused_panel = FocusedPanel::Diff;
                }
            }
        }
        return;
    }

    if app
        .comment_navigator_inner_area
        .is_some_and(|r| r.contains(pos))
        && let Some(idx) = app.comment_navigator_idx_at_screen_row(pos.y)
    {
        app.focused_panel = FocusedPanel::Comments;
        app.comment_navigator_state.select(idx);
        app.jump_to_selected_comment();
        return;
    }

    if app.has_inline_commit_selector()
        && app.commit_list_inner_area.is_some_and(|r| r.contains(pos))
        && let Some(idx) = app.commit_list_idx_at_screen_row(pos.y)
    {
        app.focused_panel = FocusedPanel::CommitSelector;
        app.commit_list_cursor = idx;
        handle_commit_selector_action(app, Action::ToggleExpand);
        return;
    }

    if app.diff_inner_area.is_some_and(|r| r.contains(pos))
        && let Some(idx) = app.diff_annotation_at_screen_row(pos.y)
    {
        app.focused_panel = FocusedPanel::Diff;
        app.move_cursor_to_annotation(idx);
        handle_diff_action(app, Action::SelectFile);
    }
}

fn handle_copy_pr_url(app: &mut App) {
    let app::DiffSource::PullRequest(pr) = &app.diff_source else {
        app.set_warning(":copy-url only applies in PR mode");
        return;
    };
    let url = pr.url.clone();

    match copy_text_to_clipboard_with(
        &url,
        app.clipboard_override.as_deref(),
        app.clipboard_command.as_deref(),
    ) {
        Ok(copy) => app.set_copy_status(&copy, copy.message("PR URL")),
        Err(e) => app.set_warning(format!("Failed to copy PR URL: {e}")),
    }
}

/// Export review: either to clipboard or set pending stdout output based on app.output_to_stdout.
/// When output_to_stdout is true, stores the content and sets should_quit.
/// Returns false when nothing was exported; the reason is then on the status line.
fn handle_export(app: &mut App) -> bool {
    export_review(app, false)
}

/// Like `handle_export`, but with `quitting` a clipboard fallback warning is
/// kept for after exit, since the status bar would never show it.
fn export_review(app: &mut App, quitting: bool) -> bool {
    let slug = app.session_slug();
    if app.output_to_stdout {
        match generate_export_content(
            &app.session,
            &app.diff_source,
            &app.comment_types,
            &app.export,
            &app.forge_review_threads,
            slug.as_deref(),
            &app.export_diff(),
        ) {
            Ok(content) => {
                app.pending_stdout_output = Some(content);
                app.should_quit = true;
                true
            }
            Err(e) => {
                app.set_warning(format!("{e}"));
                false
            }
        }
    } else {
        match export_to_clipboard_with(
            &app.session,
            &app.diff_source,
            &app.comment_types,
            &app.export,
            &app.forge_review_threads,
            slug.as_deref(),
            &app.export_diff(),
            app.clipboard_override.as_deref(),
            app.clipboard_command.as_deref(),
        ) {
            Ok(copy) => {
                if quitting {
                    app.pending_exit_warning = copy.fallback_warning;
                } else {
                    app.set_copy_status(&copy, copy.message("Review"));
                }
                true
            }
            Err(e) => {
                app.set_warning(format!("{e}"));
                false
            }
        }
    }
}

/// The path given to `:file <path>`, or `Some("")` for a bare `:file`.
fn file_command_path(cmd: &str) -> Option<&str> {
    let rest = cmd.strip_prefix("file")?;
    (rest.is_empty() || rest.starts_with(' ')).then(|| rest.trim())
}

/// `:file <path>`: write the review to a file. A leading `~/` means home.
fn handle_export_to_file(app: &mut App, path: &str) {
    if path.is_empty() {
        app.set_warning("Usage: :file <path>");
        return;
    }
    let path = match path.strip_prefix("~/") {
        Some(rest) => directories::BaseDirs::new()
            .map(|dirs| dirs.home_dir().join(rest))
            .unwrap_or_else(|| path.into()),
        None => path.into(),
    };
    let slug = app.session_slug();
    match export_to_file(
        &app.session,
        &app.diff_source,
        &app.comment_types,
        &app.export,
        &app.forge_review_threads,
        slug.as_deref(),
        &path,
        &app.export_diff(),
    ) {
        Ok(msg) => app.set_message(msg),
        Err(TuicrError::Io(e)) => {
            app.set_warning(format!("Could not write {}: {e}", path.display()))
        }
        Err(TuicrError::NoComments) => app.set_warning("No comments to write"),
        Err(e) => app.set_warning(format!("{e}")),
    }
}

/// Copy just the comment under the cursor (`Y`). Unlike `y`, the rest of the
/// review stays out of the clipboard, so a single comment can go straight into
/// a chat message or an agent prompt.
fn handle_copy_comment_at_cursor(app: &mut App) {
    let Some(content) = app
        .comment_content_at_cursor()
        .or_else(|| app.remote_comment_content_at_cursor())
    else {
        if app.cursor_on_remote_thread() {
            let forge = app.forge_display_name();
            app.set_message(format!("No copyable comment in this {forge} thread"));
        } else {
            app.set_message("No comment at cursor");
        }
        return;
    };
    match copy_text_to_clipboard_with(
        &content,
        app.clipboard_override.as_deref(),
        app.clipboard_command.as_deref(),
    ) {
        Ok(copy) => app.set_copy_status(&copy, copy.message("Comment")),
        Err(e) => app.set_warning(format!("{e}")),
    }
}

/// Which key asked to save and quit. They differ only under
/// `copy_on_quit = "ask"`: `:wq` prompts, `ZZ` copies without asking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WriteQuitKey {
    /// `:wq` / `:x`
    Command,
    /// `ZZ`
    ZZ,
}

/// Tail of `:wq` / `:x` / `ZZ` once the session is saved: decide whether the
/// review reaches the clipboard (or stdout) before quitting, per config
/// `copy_on_quit`. Returns `KeepMode` when a confirmation prompt was opened
/// and the caller must stay in the new mode.
pub(crate) fn finish_write_quit(app: &mut App, key: WriteQuitKey) -> CommandAfterDispatch {
    if !app.session.has_comments() {
        app.should_quit = true;
        return CommandAfterDispatch::ExitCommandMode;
    }
    // `--stdout` is an explicit request for the export, so it wins over config.
    if app.output_to_stdout {
        match key {
            // `:wq` stays open to show an export failure; `ZZ` has always quit.
            WriteQuitKey::Command => {
                handle_export(app);
            }
            WriteQuitKey::ZZ => handle_export_and_quit(app),
        }
        return CommandAfterDispatch::ExitCommandMode;
    }
    match quit_copy_plan(app.copy_on_quit, key) {
        QuitCopyPlan::Quit => app.should_quit = true,
        QuitCopyPlan::CopyAndQuit => handle_export_and_quit(app),
        QuitCopyPlan::Prompt => {
            app.exit_command_mode();
            app.enter_confirm_mode(app::ConfirmAction::CopyAndQuit);
            return CommandAfterDispatch::KeepMode;
        }
    }
    CommandAfterDispatch::ExitCommandMode
}

/// What a save-and-quit does about the clipboard once comments exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QuitCopyPlan {
    Quit,
    CopyAndQuit,
    Prompt,
}

fn quit_copy_plan(policy: app::CopyOnQuit, key: WriteQuitKey) -> QuitCopyPlan {
    use app::CopyOnQuit;

    match (policy, key) {
        (CopyOnQuit::Never, _) => QuitCopyPlan::Quit,
        (CopyOnQuit::Always, _) | (CopyOnQuit::Ask, WriteQuitKey::ZZ) => QuitCopyPlan::CopyAndQuit,
        (CopyOnQuit::Ask, WriteQuitKey::Command) => QuitCopyPlan::Prompt,
    }
}

/// Export and quit (used by ZZ keybinding).
/// When --stdout is set, stores export content and quits.
/// Otherwise, exports to clipboard and quits.
pub fn handle_export_and_quit(app: &mut App) {
    handle_export(app);
    app.should_quit = true;
}

/// ZZ: save the session, then quit the way `:wq` would (see
/// `finish_write_quit`). A failed save reports the error and stays open, like
/// `:wq`.
pub fn handle_write_and_quit(app: &mut App) {
    match app.save_current_session_merging_external() {
        Ok(_) => {
            finish_write_quit(app, WriteQuitKey::ZZ);
        }
        Err(e) => app.set_error(format!("Save failed: {e}")),
    }
}

fn comment_line_start(buffer: &str, cursor: usize) -> usize {
    let cursor = cursor.min(buffer.len());
    match buffer[..cursor].rfind('\n') {
        Some(pos) => pos + 1,
        None => 0,
    }
}

fn comment_line_end(buffer: &str, cursor: usize) -> usize {
    let cursor = cursor.min(buffer.len());
    match buffer[cursor..].find('\n') {
        Some(pos) => cursor + pos,
        None => buffer.len(),
    }
}

fn comment_word_left(buffer: &str, cursor: usize) -> usize {
    let cursor = cursor.min(buffer.len());
    if cursor == 0 {
        return 0;
    }
    let before = &buffer[..cursor];
    let mut idx = 0;
    let mut found_word = false;
    for (pos, ch) in before.char_indices().rev() {
        if !ch.is_whitespace() {
            idx = pos;
            found_word = true;
            break;
        }
    }

    if !found_word {
        return 0;
    }

    for (pos, ch) in before[..idx].char_indices().rev() {
        if ch.is_whitespace() {
            return pos + ch.len_utf8();
        }
        idx = pos;
    }

    idx
}

fn comment_word_right(buffer: &str, cursor: usize) -> usize {
    let cursor = cursor.min(buffer.len());
    if cursor >= buffer.len() {
        return buffer.len();
    }

    let mut chars = buffer[cursor..].char_indices();
    if let Some((_, ch)) = chars.next()
        && ch.is_whitespace()
    {
        for (pos, ch) in buffer[cursor..].char_indices() {
            if !ch.is_whitespace() {
                return cursor + pos;
            }
        }
        return buffer.len();
    }

    let mut word_end = buffer.len();
    for (pos, ch) in buffer[cursor..].char_indices() {
        if ch.is_whitespace() {
            word_end = cursor + pos;
            break;
        }
    }

    if word_end >= buffer.len() {
        return buffer.len();
    }

    for (pos, ch) in buffer[word_end..].char_indices() {
        if !ch.is_whitespace() {
            return word_end + pos;
        }
    }

    buffer.len()
}

/// Append a pasted payload into a single-line buffer, dropping embedded
/// newlines so a multi-line paste doesn't smear across the prompt.
fn push_single_line(buffer: &mut String, text: &str) {
    for ch in text.chars() {
        if matches!(ch, '\n' | '\r') {
            continue;
        }
        buffer.push(ch);
    }
}

/// Handle actions in Help mode (scrolling only)
pub fn handle_help_action(app: &mut App, action: Action) {
    match action {
        Action::CursorDown(n) => app.help_scroll_down(n),
        Action::CursorUp(n) => app.help_scroll_up(n),
        Action::ScrollLeft(n) => app.help_scroll_left(n),
        Action::ScrollRight(n) => app.help_scroll_right(n),
        Action::HalfPageDown => app.help_scroll_down(app.help_state.viewport_height / 2),
        Action::HalfPageUp => app.help_scroll_up(app.help_state.viewport_height / 2),
        Action::PageDown => app.help_scroll_down(app.help_state.viewport_height),
        Action::PageUp => app.help_scroll_up(app.help_state.viewport_height),
        Action::GoToTop => app.help_scroll_to_top(),
        Action::GoToBottom => app.help_scroll_to_bottom(),
        Action::MouseScrollDown(n) => app.help_scroll_down(n),
        Action::MouseScrollUp(n) => app.help_scroll_up(n),
        Action::EnterSearchMode => app.enter_search_mode(),
        Action::SearchNext => {
            app.search_next_in_help();
        }
        Action::SearchPrev => {
            app.search_prev_in_help();
        }
        Action::ToggleHelp => app.toggle_help(),
        Action::Quit => app.should_quit = true,
        _ => {}
    }
}

/// Handle selection, scrolling, activation, and dismissal in the pending-comments summary view.
pub fn handle_summary_action(app: &mut App, action: Action) {
    match action {
        Action::CursorDown(n) => app.summary_select_down(n),
        Action::CursorUp(n) => app.summary_select_up(n),
        Action::HalfPageDown => {
            app.summary_scroll_down(app.summary_state.viewport_height / 2);
        }
        Action::HalfPageUp => {
            app.summary_scroll_up(app.summary_state.viewport_height / 2);
        }
        Action::PageDown => app.summary_scroll_down(app.summary_state.viewport_height),
        Action::PageUp => app.summary_scroll_up(app.summary_state.viewport_height),
        Action::GoToTop => app.summary_select_first(),
        Action::GoToBottom => app.summary_select_last(),
        Action::MouseScrollDown(n) => app.summary_scroll_down(n),
        Action::MouseScrollUp(n) => app.summary_scroll_up(n),
        Action::SubmitInput => {
            let selected_target = app
                .summary_state
                .targets
                .get(app.summary_state.selected_comment)
                .cloned()
                .flatten();
            if let Some(target) = selected_target {
                app.jump_to_summary_comment(target);
            } else if !app.summary_state.targets.is_empty() {
                app.set_warning("That comment is hidden by the current diff or filters");
            }
        }
        Action::ExitMode => app.exit_summary_mode(),
        Action::Quit => app.should_quit = true,
        _ => {}
    }
}

/// Handle actions in Command mode (text input for :commands)
pub fn handle_command_action(app: &mut App, action: Action) {
    match action {
        Action::InsertChar(c) => {
            app.command_completion = None;
            app.command_buffer.push(c);
        }
        Action::Paste(text) => {
            app.command_completion = None;
            push_single_line(&mut app.command_buffer, &text);
        }
        Action::DeleteChar => {
            app.command_completion = None;
            app.command_buffer.pop();
        }
        Action::DeleteWord => {
            app.command_completion = None;
            let cursor = app.command_buffer.len();
            delete_word_before(&mut app.command_buffer, cursor);
        }
        Action::ClearLine => {
            app.command_completion = None;
            app.command_buffer.clear();
        }
        Action::CompleteCommand => complete_command(app, CompletionDirection::Forward),
        Action::CompleteCommandReverse => complete_command(app, CompletionDirection::Reverse),
        Action::ExitMode => app.exit_command_mode(),
        Action::SubmitInput => {
            app.command_completion = None;
            let cmd = app.command_buffer.trim().to_string();
            let after_dispatch = if let Some(spec) = command_spec_for(&cmd) {
                dispatch_command(app, spec.kind)
            } else if let Some((lineno, side)) = parse_lineno_command(&cmd) {
                app.go_to_source_line(lineno, side);
                CommandAfterDispatch::ExitCommandMode
            } else if let Some(path) = file_command_path(&cmd) {
                handle_export_to_file(app, path);
                CommandAfterDispatch::ExitCommandMode
            } else if let Some(name) = cmd.strip_prefix("theme ") {
                let name = name.trim();
                if name.is_empty() {
                    app.set_message(format!("Unknown command: {cmd}"));
                } else {
                    app.apply_theme(name);
                }
                CommandAfterDispatch::ExitCommandMode
            } else {
                app.set_message(format!("Unknown command: {cmd}"));
                CommandAfterDispatch::ExitCommandMode
            };
            if matches!(after_dispatch, CommandAfterDispatch::ExitCommandMode) {
                app.exit_command_mode();
            }
        }
        Action::Quit => app.should_quit = true,
        _ => {}
    }
}

fn command_spec_for(cmd: &str) -> Option<&'static CommandSpec> {
    COMMAND_SPECS.iter().find(|spec| spec.names.contains(&cmd))
}

/// CommandCompleter computes command-buffer replacements without mutating App.
struct CommandCompleter<'a> {
    /// Registry whose command names are exposed as completion candidates.
    specs: &'a [CommandSpec],
}

impl<'a> CommandCompleter<'a> {
    /// Build a completer over the command registry it should expose.
    pub fn new(specs: &'a [CommandSpec]) -> Self {
        Self { specs }
    }

    /// Return the next command-completion state for the current prompt state.
    ///
    /// `buffer` is the command text currently shown to the user. `active` is
    /// the previous completion cycle, if repeated Tab presses are cycling an
    /// existing match set. The result describes the next buffer contents and
    /// active completion cycle without mutating App.
    pub fn complete(
        &self,
        buffer: &str,
        active: Option<&CommandCompletionState>,
        direction: CompletionDirection,
    ) -> CompletionResult {
        if let Some(result) = self.cycle_existing(buffer, active, direction) {
            return result;
        }

        let prefix = buffer.trim_start().to_string();
        let matches: Vec<&'static str> = self
            .command_names()
            .filter(|command| command.starts_with(&prefix))
            .collect();

        match matches.len() {
            0 => CompletionResult::NoMatch,
            1 => CompletionResult::replace(matches[0], None),
            _ => {
                let common_prefix = Self::common_prefix(&matches);
                if common_prefix.len() > prefix.len() {
                    CompletionResult::replace(common_prefix, None)
                } else {
                    self.start_cycle(buffer, prefix, matches, direction)
                }
            }
        }
    }

    fn command_names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.specs
            .iter()
            .flat_map(|spec| spec.names.iter().copied())
    }

    fn cycle_existing(
        &self,
        buffer: &str,
        active: Option<&CommandCompletionState>,
        direction: CompletionDirection,
    ) -> Option<CompletionResult> {
        let completion = active?;
        let current = completion.matches.get(completion.selected)?;
        if buffer != *current {
            return None;
        }

        let selected = Self::next_index(completion.selected, completion.matches.len(), direction);
        Some(CompletionResult::replace(
            completion.matches[selected],
            Some(CommandCompletionState {
                prefix: completion.prefix.clone(),
                matches: completion.matches.clone(),
                selected,
            }),
        ))
    }

    fn start_cycle(
        &self,
        buffer: &str,
        prefix: String,
        matches: Vec<&'static str>,
        direction: CompletionDirection,
    ) -> CompletionResult {
        let selected = matches
            .iter()
            .position(|command| *command == buffer)
            .map(|idx| Self::next_index(idx, matches.len(), direction))
            .unwrap_or_else(|| match direction {
                CompletionDirection::Forward => 0,
                CompletionDirection::Reverse => matches.len() - 1,
            });

        CompletionResult::replace(
            matches[selected],
            Some(CommandCompletionState {
                prefix,
                matches,
                selected,
            }),
        )
    }

    fn next_index(selected: usize, match_count: usize, direction: CompletionDirection) -> usize {
        match direction {
            CompletionDirection::Forward => (selected + 1) % match_count,
            CompletionDirection::Reverse => (selected + match_count - 1) % match_count,
        }
    }

    fn common_prefix(matches: &[&str]) -> String {
        let mut prefix = matches
            .first()
            .map(|command| (*command).to_string())
            .unwrap_or_default();
        for command in &matches[1..] {
            while !command.starts_with(&prefix) {
                prefix.pop();
            }
        }
        prefix
    }
}

/// CompletionResult tells the handler how to update command-mode state.
enum CompletionResult {
    /// Leave the buffer unchanged and surface a no-match message.
    NoMatch,
    /// Replace the buffer and optionally keep a cycling state active.
    Replace {
        /// New command-buffer contents.
        buffer: String,
        /// Completion state used by the next Tab press, if cycling started.
        completion: Option<CommandCompletionState>,
    },
}

impl CompletionResult {
    fn replace(
        buffer: impl Into<String>,
        completion: Option<CommandCompletionState>,
    ) -> CompletionResult {
        CompletionResult::Replace {
            buffer: buffer.into(),
            completion,
        }
    }
}

fn dispatch_command(app: &mut App, kind: CommandKind) -> CommandAfterDispatch {
    match kind {
        CommandKind::Quit => {
            if app.dirty && app.session.has_comments() {
                app.set_error("No write since last change (add ! to override)");
            } else if app.dirty {
                // Dirty from reviewed-file markers only: discard the state and
                // quit instead of requiring `:q!`.
                app.discard_session_and_quit();
            } else {
                app.should_quit = true;
            }
            CommandAfterDispatch::ExitCommandMode
        }
        CommandKind::ForceQuit => {
            app.should_quit = true;
            CommandAfterDispatch::ExitCommandMode
        }
        CommandKind::Write => {
            match app.save_current_session_merging_external() {
                Ok(path) => {
                    app.set_message(format!("Saved to {}", path.display()));
                }
                Err(e) => app.set_error(format!("Save failed: {e}")),
            }
            CommandAfterDispatch::ExitCommandMode
        }
        CommandKind::WriteQuit => {
            match app.save_current_session_merging_external() {
                Ok(_) => return finish_write_quit(app, WriteQuitKey::Command),
                Err(e) => app.set_error(format!("Save failed: {e}")),
            }
            CommandAfterDispatch::ExitCommandMode
        }
        CommandKind::Reload => {
            reload_review(app);
            CommandAfterDispatch::ExitCommandMode
        }
        CommandKind::Edit => {
            app.queue_editor_for_focused_item();
            CommandAfterDispatch::ExitCommandMode
        }
        CommandKind::Export => {
            handle_export(app);
            CommandAfterDispatch::ExitCommandMode
        }
        CommandKind::ExportForceQuit => {
            if export_review(app, true) {
                app.should_quit = true;
            }
            CommandAfterDispatch::ExitCommandMode
        }
        CommandKind::CopyUrl => {
            handle_copy_pr_url(app);
            CommandAfterDispatch::ExitCommandMode
        }
        CommandKind::Clear(scope) => {
            app.clear_comments(scope);
            CommandAfterDispatch::ExitCommandMode
        }
        CommandKind::Help => {
            // Leave command mode by hand: the dispatch loop's ExitCommandMode
            // path resets `input_mode` to Normal, which would clobber Help.
            app.exit_command_mode();
            app.toggle_help();
            CommandAfterDispatch::KeepMode
        }
        CommandKind::MessageDetails => {
            app.exit_command_mode();
            app.open_message_details();
            CommandAfterDispatch::KeepMode
        }
        CommandKind::Summary => {
            // See Help above: leave command mode before opening the view so
            // the common post-dispatch cleanup cannot clobber Summary.
            app.exit_command_mode();
            app.enter_summary_mode();
            CommandAfterDispatch::KeepMode
        }
        CommandKind::ThemePicker => {
            app.exit_command_mode();
            app.enter_theme_picker_mode();
            CommandAfterDispatch::KeepMode
        }
        CommandKind::Version => {
            app.set_message(format!("tuicr v{}", env!("CARGO_PKG_VERSION")));
            CommandAfterDispatch::ExitCommandMode
        }
        CommandKind::Update => {
            check_for_updates(app);
            CommandAfterDispatch::ExitCommandMode
        }
        CommandKind::SetWrap => {
            app.set_diff_wrap(true);
            CommandAfterDispatch::ExitCommandMode
        }
        CommandKind::ToggleWrap => {
            app.toggle_diff_wrap();
            CommandAfterDispatch::ExitCommandMode
        }
        CommandKind::SetRelativeLineNumbers(enabled) => {
            app.relative_line_numbers = enabled;
            CommandAfterDispatch::ExitCommandMode
        }
        CommandKind::ToggleRelativeLineNumbers => {
            app.relative_line_numbers = !app.relative_line_numbers;
            CommandAfterDispatch::ExitCommandMode
        }
        CommandKind::SetWordDiff(enabled) => {
            app.set_word_diff(enabled);
            CommandAfterDispatch::ExitCommandMode
        }
        CommandKind::ToggleWordDiff => {
            app.set_word_diff(!app.word_diff);
            CommandAfterDispatch::ExitCommandMode
        }
        CommandKind::ToggleVim => {
            app.toggle_comment_vim();
            CommandAfterDispatch::ExitCommandMode
        }
        CommandKind::SetVim(enabled) => {
            app.set_comment_vim(enabled);
            CommandAfterDispatch::ExitCommandMode
        }
        CommandKind::SetCommitsVisible(visible) => {
            set_commit_selector_visible(app, visible);
            CommandAfterDispatch::ExitCommandMode
        }
        CommandKind::ToggleCommits => {
            app.toggle_commit_selector();
            CommandAfterDispatch::ExitCommandMode
        }
        CommandKind::SetShowReviewed(show) => {
            app.set_show_reviewed(show);
            CommandAfterDispatch::ExitCommandMode
        }
        CommandKind::ToggleShowReviewed => {
            app.toggle_show_reviewed();
            CommandAfterDispatch::ExitCommandMode
        }
        CommandKind::SetMinimalUi(enabled) => {
            app.set_minimal_ui(enabled);
            CommandAfterDispatch::ExitCommandMode
        }
        CommandKind::ToggleMinimalUi => {
            app.toggle_minimal_ui();
            CommandAfterDispatch::ExitCommandMode
        }
        CommandKind::SetGeneratedCollapsed(collapse) => {
            app.set_collapse_generated(collapse);
            CommandAfterDispatch::ExitCommandMode
        }
        CommandKind::ToggleGenerated => {
            app.toggle_collapse_generated();
            CommandAfterDispatch::ExitCommandMode
        }
        CommandKind::Diff => {
            app.toggle_diff_view_mode();
            CommandAfterDispatch::ExitCommandMode
        }
        CommandKind::Focus => {
            app.toggle_single_file_view();
            CommandAfterDispatch::ExitCommandMode
        }
        CommandKind::Stage => {
            app.stage_reviewed_files();
            CommandAfterDispatch::ExitCommandMode
        }
        CommandKind::Targets(tab) => {
            let result = app.enter_target_selector(tab);
            match (tab, result) {
                (_, Ok(())) => CommandAfterDispatch::KeepMode,
                (TargetTab::Local, Err(e)) => {
                    app.set_error(format!("Failed to load commits: {e}"));
                    CommandAfterDispatch::ExitCommandMode
                }
                (TargetTab::PullRequests, Err(e)) => {
                    app.set_error(format!("Failed to open PR selector: {e}"));
                    CommandAfterDispatch::ExitCommandMode
                }
                (TargetTab::Sessions, Err(e)) => {
                    app.set_error(format!("Failed to open session selector: {e}"));
                    CommandAfterDispatch::ExitCommandMode
                }
            }
        }
        CommandKind::SubmitPicker => {
            app.exit_command_mode();
            app.start_submit_action_picker();
            CommandAfterDispatch::KeepMode
        }
        CommandKind::Submit(event) => {
            app.exit_command_mode();
            app.start_submit(event);
            CommandAfterDispatch::KeepMode
        }
        CommandKind::Comments(visibility) => {
            set_remote_comments_visibility(app, visibility);
            CommandAfterDispatch::ExitCommandMode
        }
    }
}

fn reload_review(app: &mut App) {
    let comment_reload = app.reload_persisted_session_if_changed(true);
    if matches!(app.diff_source, app::DiffSource::PullRequest(_)) {
        if let Err(e) = comment_reload {
            app.set_warning(format!("Comment reload failed: {e}"));
        }
        // Async: shows a spinner in the status bar; result is applied in
        // `poll_pr_reload_events` and the cursor is restored to the captured
        // anchor.
        if let Err(e) = app.spawn_pr_reload() {
            app.set_error(format!("Reload failed: {e}"));
        }
    } else {
        match app.reload_diff_files() {
            Ok((count, invalidated)) => {
                let comment_suffix = match comment_reload {
                    Ok(added) if added > 0 => {
                        format!(", loaded {added} external comments")
                    }
                    Ok(_) => String::new(),
                    Err(e) => format!(", comment reload failed: {e}"),
                };
                if invalidated > 0 {
                    app.set_message(format!(
                        "Reloaded {count} files, {invalidated} changed since last review{comment_suffix}"
                    ));
                } else {
                    app.set_message(format!("Reloaded {count} files{comment_suffix}"));
                }
            }
            // A staged or unstaged review can empty out under it, for example
            // after a commit. That is an empty review, not a failed reload.
            Err(crate::error::TuicrError::NoChanges) if app.reload_can_empty() => {
                app.show_emptied_review();
            }
            Err(e) => app.set_error(format!("Reload failed: {e}")),
        }
    }
}

fn check_for_updates(app: &mut App) {
    match crate::update::check_for_updates() {
        crate::update::UpdateCheckResult::UpdateAvailable(info) => {
            app.set_message(format!(
                "Update available: v{} -> v{}",
                info.current_version, info.latest_version
            ));
        }
        crate::update::UpdateCheckResult::UpToDate(info) => {
            app.set_message(format!("tuicr v{} is up to date", info.current_version));
        }
        crate::update::UpdateCheckResult::AheadOfRelease(info) => {
            app.set_message(format!(
                "You're from the future! v{} > v{}",
                info.current_version, info.latest_version
            ));
        }
        crate::update::UpdateCheckResult::Failed(err) => {
            app.set_warning(format!("Update check failed: {err}"));
        }
    }
}

fn set_commit_selector_visible(app: &mut App, visible: bool) {
    app.show_commit_selector = visible;
    if !visible && app.focused_panel == FocusedPanel::CommitSelector {
        app.focused_panel = FocusedPanel::Diff;
    }
    let status = if visible { "visible" } else { "hidden" };
    app.set_message(format!("Commit selector: {status}"));
}

fn set_remote_comments_visibility(app: &mut App, visibility: PrCommentsVisibility) {
    if !matches!(app.diff_source, app::DiffSource::PullRequest(_)) {
        app.set_warning(":comments only applies in PR mode");
        return;
    }

    let changed = app.set_remote_comments_visibility(visibility);
    let label = visibility.label();
    if changed {
        app.set_message(format!("Remote comments: {label}"));
    } else {
        app.set_message(format!("Remote comments: already {label}"));
    }
}

fn complete_command(app: &mut App, direction: CompletionDirection) {
    let completer = CommandCompleter::new(COMMAND_SPECS);
    match completer.complete(
        &app.command_buffer,
        app.command_completion.as_ref(),
        direction,
    ) {
        CompletionResult::NoMatch => {
            app.command_completion = None;
            app.set_message("No command matches");
        }
        CompletionResult::Replace { buffer, completion } => {
            app.command_buffer = buffer;
            app.command_completion = completion;
        }
    }
}

/// Parse `:<n>` (new-side) or `:o<n>` (old-side) jump targets. The leading `:`
/// has already been stripped by the time we get here.
fn parse_lineno_command(cmd: &str) -> Option<(u32, LineSide)> {
    if let Some(rest) = cmd.strip_prefix('o') {
        rest.parse::<u32>().ok().map(|n| (n, LineSide::Old))
    } else {
        cmd.parse::<u32>().ok().map(|n| (n, LineSide::New))
    }
}

/// Handle actions in Search mode (text input for /pattern)
pub fn handle_search_action(app: &mut App, action: Action) {
    match action {
        Action::InsertChar(c) => app.search_buffer.push(c),
        Action::Paste(text) => push_single_line(&mut app.search_buffer, &text),
        Action::DeleteChar => {
            app.search_buffer.pop();
        }
        Action::DeleteWord if !app.search_buffer.is_empty() => {
            while app
                .search_buffer
                .chars()
                .last()
                .map(|c| c.is_whitespace())
                .unwrap_or(false)
            {
                app.search_buffer.pop();
            }
            while app
                .search_buffer
                .chars()
                .last()
                .map(|c| !c.is_whitespace())
                .unwrap_or(false)
            {
                app.search_buffer.pop();
            }
        }
        Action::ClearLine => {
            app.search_buffer.clear();
        }
        Action::ExitMode => app.exit_search_mode(),
        Action::SubmitInput => {
            if app.searching_help() {
                app.search_in_help_from_scroll();
            } else {
                app.search_in_diff_from_cursor();
            }
            app.exit_search_mode();
        }
        Action::Quit => app.should_quit = true,
        _ => {}
    }
}

/// Handle actions in Comment mode (text input for comments)
pub fn handle_comment_action(app: &mut App, action: Action) {
    if matches!(
        action,
        Action::InsertChar(_)
            | Action::Paste(_)
            | Action::DeleteChar
            | Action::DeleteWord
            | Action::ClearLine
            | Action::TextCursorLeft
            | Action::TextCursorRight
            | Action::TextCursorLineStart
            | Action::TextCursorLineEnd
            | Action::TextCursorWordLeft
            | Action::TextCursorWordRight
    ) {
        app.comment_preferred_column = None;
    }
    match action {
        Action::InsertChar(c) => {
            app.comment_buffer.insert(app.comment_cursor, c);
            app.comment_cursor += c.len_utf8();
        }
        Action::Paste(text) => {
            let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
            app.comment_buffer
                .insert_str(app.comment_cursor, &normalized);
            app.comment_cursor += normalized.len();
        }
        Action::DeleteChar => {
            app.comment_cursor = delete_char_before(&mut app.comment_buffer, app.comment_cursor);
        }
        Action::ExitMode => app.exit_comment_mode(),
        Action::SubmitInput => app.save_comment(),
        Action::CycleCommentType => app.cycle_comment_type(),
        Action::CycleCommentTypeReverse => app.cycle_comment_type_reverse(),
        Action::TextCursorUp | Action::TextCursorDown => {
            app.comment_cursor = crate::ui::comment_panel::move_comment_cursor(
                &app.comment_buffer,
                app.comment_cursor,
                app.diff_state.viewport_width.saturating_sub(1),
                matches!(action, Action::TextCursorDown),
                &mut app.comment_preferred_column,
            );
        }
        Action::TextCursorLeft => {
            app.comment_cursor = prev_char_boundary(&app.comment_buffer, app.comment_cursor);
        }
        Action::TextCursorRight => {
            app.comment_cursor = next_char_boundary(&app.comment_buffer, app.comment_cursor);
        }
        Action::TextCursorLineStart => {
            app.comment_cursor = comment_line_start(&app.comment_buffer, app.comment_cursor);
        }
        Action::TextCursorLineEnd => {
            app.comment_cursor = comment_line_end(&app.comment_buffer, app.comment_cursor);
        }
        Action::TextCursorWordLeft => {
            app.comment_cursor = comment_word_left(&app.comment_buffer, app.comment_cursor);
        }
        Action::TextCursorWordRight => {
            app.comment_cursor = comment_word_right(&app.comment_buffer, app.comment_cursor);
        }
        Action::DeleteWord => {
            app.comment_cursor = delete_word_before(&mut app.comment_buffer, app.comment_cursor);
        }
        Action::ClearLine => {
            app.comment_buffer.clear();
            app.comment_cursor = 0;
        }
        Action::Quit => app.should_quit = true,
        _ => {}
    }
}

/// Handle actions in Confirm mode (Y/N prompts)
pub fn handle_confirm_action(app: &mut App, action: Action) {
    // Only the quit prompts end the session. A Sessions-tab delete answers
    // in place and returns to the selector.
    let quits = !matches!(
        app.pending_confirm,
        Some(app::ConfirmAction::DeleteSession | app::ConfirmAction::DeleteRemoteDraft)
    );
    match action {
        Action::ConfirmYes => {
            if let Some(app::ConfirmAction::DeleteSession) = app.pending_confirm {
                app.sessions_tab_delete_selected();
            }
            if let Some(app::ConfirmAction::DeleteRemoteDraft) = app.pending_confirm {
                app.spawn_remote_draft_delete();
            }
            if let Some(app::ConfirmAction::CopyAndQuit) = app.pending_confirm {
                let slug = app.session_slug();
                if app.output_to_stdout {
                    match generate_export_content(
                        &app.session,
                        &app.diff_source,
                        &app.comment_types,
                        &app.export,
                        &app.forge_review_threads,
                        slug.as_deref(),
                        &app.export_diff(),
                    ) {
                        Ok(content) => app.pending_stdout_output = Some(content),
                        Err(e) => app.set_warning(format!("{e}")),
                    }
                } else {
                    match export_to_clipboard_with(
                        &app.session,
                        &app.diff_source,
                        &app.comment_types,
                        &app.export,
                        &app.forge_review_threads,
                        slug.as_deref(),
                        &app.export_diff(),
                        app.clipboard_override.as_deref(),
                        app.clipboard_command.as_deref(),
                    ) {
                        // tuicr quits next, so the status bar would never show it.
                        Ok(copy) => app.pending_exit_warning = copy.fallback_warning,
                        Err(e) => app.set_warning(format!("{e}")),
                    }
                }
            }
            app.exit_confirm_mode();
            app.should_quit = quits;
        }
        Action::ConfirmNo => {
            app.exit_confirm_mode();
            app.should_quit = quits;
        }
        Action::Quit => app.should_quit = true,
        _ => {}
    }
}

/// Handle actions in CommitSelect mode.
///
/// CommitSelect actually drives the review target selector, which has two
/// tabs (Local and Pull Requests). Tab-shared actions (switch tab, quit) are
/// handled first; per-tab dispatch follows.
pub fn handle_commit_select_action(app: &mut App, action: Action) {
    // Filter-editing sub-state on the PR tab. Routed before tab dispatch
    // because typed characters must go to the filter buffer rather than
    // to local-commit movement.
    if app.pr_filter_editing() {
        handle_pr_filter_action(app, action);
        return;
    }

    match action {
        Action::TargetSelectorTabNext => app.cycle_target_tab(true),
        Action::TargetSelectorTabPrev => app.cycle_target_tab(false),
        Action::EnterCommandMode => app.enter_command_mode(),
        Action::Quit => app.should_quit = true,
        Action::QuitHint => app.set_message(QUIT_HINT_MESSAGE),
        Action::ExitMode => {
            // Esc during an in-flight PR open aborts the load and stays
            // in the selector. Takes precedence over the
            // commit-selection-range exit so the user isn't stuck staring
            // at a spinner.
            if app.cancel_pr_open() {
                return;
            }
            if !app.selector_opened_from_review && app.commit_selection_range.is_none() {
                return;
            }
            if let Err(e) = app.exit_commit_select_mode() {
                app.set_error(format!("Failed to reload changes: {e}"));
            }
        }
        other => match app.target_tab {
            TargetTab::Local => handle_local_target_action(app, other),
            TargetTab::PullRequests => handle_pr_target_action(app, other),
            TargetTab::Sessions => handle_sessions_target_action(app, other),
        },
    }
}

fn handle_sessions_target_action(app: &mut App, action: Action) {
    match action {
        Action::CommitSelectUp => app.sessions_tab_cursor_up(),
        Action::CommitSelectDown => app.sessions_tab_cursor_down(),
        Action::ConfirmCommitSelect => {
            if let Err(e) = app.sessions_tab_select() {
                app.set_error(format!("Failed to open session: {e}"));
            }
        }
        Action::SessionsTabToggleShowEmpty => app.sessions_tab_toggle_show_empty(),
        Action::SessionsTabDelete => {
            if app.sessions_tab.cursor_session().is_some() {
                app.enter_confirm_mode(app::ConfirmAction::DeleteSession);
            }
        }
        // Space is a no-op: sessions are picked, not multi-selected.
        Action::ToggleCommitSelect => {}
        _ => {}
    }
}

fn handle_local_target_action(app: &mut App, action: Action) {
    match action {
        Action::CommitSelectUp => app.commit_select_up(),
        Action::CommitSelectDown => app.commit_select_down(),
        Action::ToggleCommitSelect => {
            // If on expand row, expand commits instead of toggling selection
            if app.is_on_expand_row() {
                if let Err(e) = app.expand_commit() {
                    app.set_error(format!("Failed to load commits: {e}"));
                }
            } else {
                // Toggle + auto-advance so repeated Space sweeps a range.
                app.toggle_commit_selection_and_advance();
            }
        }
        Action::ConfirmCommitSelect => {
            // if on expand row, expand commit instead of confirming
            if app.is_on_expand_row() {
                if let Err(e) = app.expand_commit() {
                    app.set_error(format!("Failed to load commits: {e}"));
                }
            } else if let Err(e) = app.confirm_commit_selection() {
                app.set_error(format!("Failed to load commits: {e}"));
            }
        }
        _ => {}
    }
}

fn handle_pr_target_action(app: &mut App, action: Action) {
    match action {
        Action::CommitSelectUp => app.pr_tab_cursor_up(),
        Action::CommitSelectDown => app.pr_tab_cursor_down(),
        Action::ConfirmCommitSelect => {
            app.pr_tab_select();
        }
        Action::ToggleCommitSelect => {
            // Space is a no-op on the PR tab (spec).
        }
        Action::BeginTargetFilter => {
            app.begin_pr_filter();
        }
        Action::TogglePrReviewRequestedFilter => {
            app.toggle_pr_review_requested_filter();
        }
        _ => {}
    }
}

fn handle_pr_filter_action(app: &mut App, action: Action) {
    match action {
        Action::InsertChar(c) => app.pr_filter_insert_char(c),
        Action::Paste(text) => {
            for ch in text.chars().filter(|c| !matches!(*c, '\n' | '\r')) {
                app.pr_filter_insert_char(ch);
            }
        }
        Action::DeleteChar => app.pr_filter_delete_char(),
        Action::DeleteWord => {
            // Soft word-delete: collapses to clear-line for the v1 cut.
            app.pr_filter_clear();
        }
        Action::ClearLine => app.pr_filter_clear(),
        Action::SubmitInput => app.commit_pr_filter(),
        Action::ExitMode => app.cancel_pr_filter(),
        Action::Quit => app.should_quit = true,
        _ => {}
    }
}

/// Handle actions when inline commit selector panel is focused
pub fn handle_commit_selector_action(app: &mut App, action: Action) {
    match action {
        // `j`/`k` track screen direction: in ascending (oldest-first) order the
        // display is reversed, so moving down the screen moves toward newer
        // (lower) data indices.
        Action::CursorDown(_) => {
            if app.commits_ascending() {
                app.commit_select_up();
            } else {
                app.commit_select_down();
            }
        }
        Action::CursorUp(_) => {
            if app.commits_ascending() {
                app.commit_select_down();
            } else {
                app.commit_select_up();
            }
        }
        // Space toggles + auto-advances so repeated presses sweep a contiguous
        // run; Enter selects only the cursor commit, unselecting the others.
        Action::ToggleExpand | Action::ToggleCommitSelect | Action::SelectFile => {
            if action == Action::SelectFile {
                app.select_only_cursor_commit();
            } else {
                app.toggle_commit_selection_and_advance();
            }
            // PR mode reloads via the forge `compare` API on a background
            // thread (persisting the new range so it survives a restart);
            // local reviews reload from the VCS.
            if let Err(e) = app.reload_inline_selection_for_source() {
                app.set_error(format!("Failed to load diff: {e}"));
            }
        }
        // Normal mode maps Esc to `ClearSearchHighlight`; in this pane it
        // also returns focus to the diff.
        Action::ClearSearchHighlight => {
            app.clear_search_highlight();
            app.focused_panel = FocusedPanel::Diff;
        }
        _ => handle_shared_normal_action(app, action),
    }
}

/// Handle actions in VisualSelect mode
pub fn handle_visual_action(app: &mut App, action: Action) {
    match action {
        Action::CursorDown(n) => app.visual_cursor_down(n),
        Action::CursorUp(n) => app.visual_cursor_up(n),
        Action::AddRangeComment | Action::AddRangeSuggestion => {
            if action == Action::AddRangeSuggestion {
                app.enter_suggestion_from_visual();
            } else if app.visual_selection_line_range().is_some() {
                app.enter_comment_from_visual();
            } else {
                app.set_warning("Invalid selection - move cursor to a diff line");
                app.exit_visual_mode();
            }
        }
        Action::ExportToClipboard => {
            match app.copy_visual_selection() {
                Ok((0, _)) => app.set_message("Nothing to copy"),
                Ok((_, Some(warning))) => app.set_warning(warning),
                Ok((n, None)) => app.set_message(format!("Copied {n} chars")),
                Err(e) => app.set_warning(format!("{e}")),
            }
            app.exit_visual_mode();
        }
        Action::ExitMode => app.exit_visual_mode(),
        Action::Quit => app.should_quit = true,
        Action::QuitHint => app.set_message(QUIT_HINT_MESSAGE),
        Action::ScrollViewDown(n) | Action::MouseScrollDown(n) => app.scroll_view_down(n),
        Action::ScrollViewUp(n) | Action::MouseScrollUp(n) => app.scroll_view_up(n),
        Action::HalfPageDown => app.visual_page_down(app.diff_state.viewport_height / 2),
        Action::HalfPageUp => app.visual_page_up(app.diff_state.viewport_height / 2),
        Action::PageDown => app.visual_page_down(app.diff_state.viewport_height),
        Action::PageUp => app.visual_page_up(app.diff_state.viewport_height),
        _ => {}
    }
    clear_visual_if_cursor_offscreen(app);
}

/// Route an action in `InputMode::Normal`. An open file-tree prompt owns
/// input regardless of which panel has focus (a mouse click can move focus
/// off the tree while the prompt is still open); otherwise focus decides.
pub fn handle_normal_action(app: &mut App, action: Action) {
    if app.file_tree_prompt_editing() {
        return handle_file_list_action(app, action);
    }
    match app.focused_panel {
        FocusedPanel::FileList => handle_file_list_action(app, action),
        FocusedPanel::Comments => handle_comment_navigator_action(app, action),
        FocusedPanel::Diff => handle_diff_action(app, action),
        FocusedPanel::CommitSelector => handle_commit_selector_action(app, action),
    }
}

/// Route a bracketed-paste payload to whichever input is accepting text.
/// Everything else ignores it, so pasted text never drives normal-mode keys.
pub fn handle_paste(app: &mut App, text: String) {
    // Vim comment box: feed the overlay so the buffer stays synced.
    if app.input_mode == InputMode::Comment && app.comment_vim_enabled {
        app.ensure_comment_vim_editor();
        app.comment_vim_feed_paste(text);
        return;
    }
    let action = Action::Paste(text);
    match app.input_mode {
        InputMode::Comment => handle_comment_action(app, action),
        InputMode::Command => handle_command_action(app, action),
        InputMode::Search => handle_search_action(app, action),
        InputMode::CommitSelect if app.pr_filter_editing() => {
            handle_commit_select_action(app, action)
        }
        InputMode::Normal if app.file_tree_prompt_editing() => handle_file_list_action(app, action),
        InputMode::ThemePicker if app.theme_picker_filtering() => {
            handle_theme_picker_action(app, action)
        }
        InputMode::FilePicker if app.file_picker_filtering() => {
            handle_file_picker_action(app, action)
        }
        _ => {}
    }
}

/// Handle actions when file list panel is focused
pub fn handle_file_list_action(app: &mut App, action: Action) {
    // An open `i`/`e`/`/` prompt owns keyboard input until it is submitted or
    // cancelled. The wheel still scrolls the list underneath it.
    if app.file_tree_prompt_editing()
        && !matches!(
            action,
            Action::MouseScrollDown(_) | Action::MouseScrollUp(_)
        )
    {
        handle_file_tree_prompt_action(app, action);
        return;
    }
    match action {
        Action::FileTreeFilterInclude => app.begin_file_tree_prompt(FileTreePrompt::Include),
        Action::FileTreeFilterExclude => app.begin_file_tree_prompt(FileTreePrompt::Exclude),
        Action::FileTreeClearInclude => app.clear_include_filter(),
        Action::FileTreeClearExclude => app.clear_exclude_filter(),
        Action::FileTreeSearch => app.begin_file_tree_prompt(FileTreePrompt::Search),
        // With a tree search active, n/N step file matches. Otherwise they
        // fall through to the diff search so the shared binding still works.
        Action::SearchNext if app.file_tree_search_active() => app.file_tree_search_next(),
        Action::SearchPrev if app.file_tree_search_active() => app.file_tree_search_prev(),
        Action::CursorDown(n) => app.file_list_down(n),
        Action::CursorUp(n) => app.file_list_up(n),
        Action::ScrollLeft(n) => app.file_list_state.scroll_left(n),
        Action::ScrollRight(n) => app.file_list_state.scroll_right(n),
        Action::MouseScrollDown(n) => app.file_list_viewport_scroll_down(n),
        Action::MouseScrollUp(n) => app.file_list_viewport_scroll_up(n),
        Action::SelectFile | Action::ToggleExpand => {
            if let Some(item) = app.get_selected_tree_item() {
                match item {
                    FileTreeItem::Directory { path, .. } => app.toggle_directory(&path),
                    FileTreeItem::File { file_idx, .. } => {
                        app.jump_to_file(file_idx);
                        app.focused_panel = FocusedPanel::Diff;
                    }
                }
            }
        }
        Action::ToggleReviewed => {
            if let Some(FileTreeItem::File { file_idx, .. }) = app.get_selected_tree_item() {
                app.toggle_reviewed_for_file_idx(file_idx, false);
            } else {
                app.set_warning("Select a file to toggle reviewed");
            }
        }
        _ => handle_shared_normal_action(app, action),
    }
}

/// Handle input while a file-tree prompt (`i` include, `e` exclude, `/`
/// search) is open. Mirrors `handle_pr_filter_action` for the target selector.
fn handle_file_tree_prompt_action(app: &mut App, action: Action) {
    match action {
        Action::InsertChar(c) => app.file_tree_prompt_insert_char(c),
        Action::Paste(text) => app.file_tree_prompt_insert_str(&text),
        Action::DeleteChar => app.file_tree_prompt_delete_char(),
        Action::DeleteWord => app.file_tree_prompt_delete_word(),
        Action::ClearLine => app.file_tree_prompt_clear_line(),
        Action::SubmitInput => app.commit_file_tree_prompt(),
        Action::ExitMode => app.cancel_file_tree_prompt(),
        _ => {}
    }
}

/// Handle actions in `InputMode::FilePicker`, with the same draft gate as
/// `handle_theme_picker_action`.
pub fn handle_file_picker_action(app: &mut App, action: Action) {
    if app.file_picker_filtering() {
        match action {
            Action::InsertChar(c) => app.file_picker_filter_insert_char(c),
            Action::Paste(text) => text
                .chars()
                .filter(|c| !matches!(c, '\n' | '\r'))
                .for_each(|c| app.file_picker_filter_insert_char(c)),
            Action::DeleteChar => app.file_picker_filter_delete_char(),
            Action::ClearLine => app.file_picker_filter_clear_line(),
            Action::CursorDown(n) => app.move_file_picker_selection(n as isize),
            Action::CursorUp(n) => app.move_file_picker_selection(-(n as isize)),
            Action::SubmitInput => app.submit_file_picker_filter(),
            Action::ExitMode => app.cancel_file_picker_filter(),
            _ => {}
        }
        return;
    }
    match action {
        Action::CursorDown(n) => app.move_file_picker_selection(n as isize),
        Action::CursorUp(n) => app.move_file_picker_selection(-(n as isize)),
        Action::FilePickerFilter => app.begin_file_picker_filter(),
        Action::SubmitInput => app.confirm_file_picker(),
        Action::ExitMode => app.cancel_file_picker(),
        _ => {}
    }
}

/// Handle actions in `InputMode::ThemePicker`. Mirrors
/// `handle_file_list_action`'s prompt-editing gate: while the `/` filter
/// draft is open, input goes to `handle_theme_picker_filter_action` instead
/// of list navigation.
pub fn handle_theme_picker_action(app: &mut App, action: Action) {
    if app.theme_picker_filtering() {
        handle_theme_picker_filter_action(app, action);
        return;
    }
    match action {
        Action::CursorDown(n) => app.move_theme_picker_selection(n as isize),
        Action::CursorUp(n) => app.move_theme_picker_selection(-(n as isize)),
        Action::ThemePickerFilter => app.begin_theme_picker_filter(),
        Action::SubmitInput => app.confirm_theme_picker(),
        Action::ExitMode => app.cancel_theme_picker(),
        Action::Quit => app.should_quit = true,
        _ => {}
    }
}

/// Handle input while the theme picker's `/` filter draft is open. Mirrors
/// `handle_file_tree_prompt_action`.
fn handle_theme_picker_filter_action(app: &mut App, action: Action) {
    match action {
        Action::InsertChar(c) => app.theme_picker_filter_insert_char(c),
        Action::Paste(text) => text
            .chars()
            .filter(|c| !matches!(c, '\n' | '\r'))
            .for_each(|c| app.theme_picker_filter_insert_char(c)),
        Action::DeleteChar => app.theme_picker_filter_delete_char(),
        Action::ClearLine => app.theme_picker_filter_clear_line(),
        Action::SubmitInput => app.commit_theme_picker_filter(),
        Action::ExitMode => app.cancel_theme_picker_filter(),
        _ => {}
    }
}

/// Handle actions when the comment navigator panel is focused
pub fn handle_comment_navigator_action(app: &mut App, action: Action) {
    match action {
        Action::CursorDown(n) => {
            app.comment_navigator_down(n);
            // Auto-follow: scroll the diff to the selected comment without
            // stealing focus from the navigator (Enter still jumps + focuses).
            let saved_focus = app.focused_panel;
            app.jump_to_selected_comment();
            app.focused_panel = saved_focus;
        }
        Action::CursorUp(n) => {
            app.comment_navigator_up(n);
            let saved_focus = app.focused_panel;
            app.jump_to_selected_comment();
            app.focused_panel = saved_focus;
        }
        Action::ScrollLeft(n) => app.comment_navigator_state.scroll_left(n),
        Action::ScrollRight(n) => app.comment_navigator_state.scroll_right(n),
        Action::MouseScrollDown(n) => app.comment_navigator_viewport_scroll_down(n),
        Action::MouseScrollUp(n) => app.comment_navigator_viewport_scroll_up(n),
        Action::SelectFile => {
            app.jump_to_selected_comment();
        }
        _ => handle_shared_normal_action(app, action),
    }
}

/// Enter edit mode for the comment under the cursor, placing the text cursor at
/// the end (vim `A` / non-vim default) or beginning (vim `i`). Surfaces the
/// right message when the comment is read-only or absent.
fn edit_comment_at_cursor(app: &mut App, cursor_at_end: bool) {
    if app.cursor_on_locked_comment() {
        let forge = app.forge_display_name();
        app.set_message(format!(
            "Comment already pushed to {forge} — read only in tuicr"
        ));
    } else if !app.enter_edit_mode(cursor_at_end) {
        if app.cursor_on_remote_thread() {
            let forge = app.forge_display_name();
            app.set_message(format!("{forge} comment — read only in tuicr"));
        } else {
            app.set_message("No comment at cursor");
        }
    }
}

/// Handle actions when diff panel is focused
pub fn handle_diff_action(app: &mut App, action: Action) {
    match action {
        Action::CursorDown(n) => app.cursor_down(n),
        Action::CursorUp(n) => app.cursor_up(n),
        Action::ScrollViewDown(n) => app.scroll_view_down(n),
        Action::ScrollViewUp(n) => app.scroll_view_up(n),
        Action::ScrollLeft(n) => app.scroll_left(n),
        Action::ScrollRight(n) => app.scroll_right(n),
        Action::MouseScrollDown(n) => app.scroll_view_down(n),
        Action::MouseScrollUp(n) => app.scroll_view_up(n),
        Action::SelectFile => {
            if let Some(hit) = app.get_gap_at_cursor() {
                match hit {
                    GapCursorHit::Expander(gap_id, dir) => {
                        let limit = if dir == ExpandDirection::Both {
                            None
                        } else {
                            Some(20)
                        };
                        if let Err(e) = app.expand_gap(gap_id, dir, limit) {
                            app.set_error(format!("Failed to expand: {e}"));
                        }
                    }
                    GapCursorHit::HiddenLines(gap_id) => {
                        if let Err(e) = app.expand_gap(gap_id, ExpandDirection::Both, None) {
                            app.set_error(format!("Failed to expand: {e}"));
                        }
                    }
                    GapCursorHit::ExpandedContent(gap_id) => {
                        app.collapse_gap(gap_id);
                    }
                }
            }
        }
        // Space already means "expand what's under the cursor" in the file
        // list and the commit selector; in the diff panel it was unbound, so
        // it takes on the same meaning here: toggle whether the current
        // file's diff body is shown, regardless of why it defaulted to
        // collapsed or expanded.
        Action::ToggleExpand => {
            if !app.toggle_file_collapse() {
                handle_shared_normal_action(app, action);
            }
        }
        Action::SelectFileFull => {
            if let Some(hit) = app.get_gap_at_cursor() {
                match hit {
                    GapCursorHit::Expander(gap_id, _) | GapCursorHit::HiddenLines(gap_id) => {
                        if let Err(e) = app.expand_gap(gap_id, ExpandDirection::Both, None) {
                            app.set_error(format!("Failed to expand: {e}"));
                        }
                    }
                    GapCursorHit::ExpandedContent(gap_id) => {
                        app.collapse_gap(gap_id);
                    }
                }
            }
        }
        _ => handle_shared_normal_action(app, action),
    }
}

/// Handle actions shared between file list and diff panels in Normal mode
fn handle_shared_normal_action(app: &mut App, action: Action) {
    // Reset quit_warned on any non-quit action
    if !matches!(action, Action::Quit) {
        app.quit_warned = false;
    }

    match action {
        Action::Quit => {
            if app.dirty && app.session.has_comments() && !app.quit_warned {
                app.set_sticky_warning("Unsaved changes. Press q again to quit.");
                app.quit_warned = true;
            } else if app.dirty && !app.session.has_comments() {
                // Dirty from reviewed-file markers only: discard the state and
                // quit instead of warning about unsaved changes.
                app.discard_session_and_quit();
            } else {
                app.should_quit = true;
            }
        }
        Action::QuitHint => app.set_message(QUIT_HINT_MESSAGE),
        Action::ExitMode => {
            app.show_file_list = false;
            app.focused_panel = FocusedPanel::Diff;
        }
        Action::HalfPageDown => app.page_down(app.diff_state.viewport_height / 2),
        Action::HalfPageUp => app.page_up(app.diff_state.viewport_height / 2),
        Action::PageDown => app.page_down(app.diff_state.viewport_height),
        Action::PageUp => app.page_up(app.diff_state.viewport_height),
        Action::GoToTop => app.jump_to_top(),
        Action::GoToBottom => app.jump_to_bottom(),
        Action::NextFile => app.next_file(),
        Action::PrevFile => app.prev_file(),
        Action::NextHunk => app.next_hunk(),
        Action::PrevHunk => app.prev_hunk(),
        Action::NextComment => app.next_comment(),
        Action::PrevComment => app.prev_comment(),
        Action::ToggleReviewed => app.toggle_reviewed(),
        Action::ToggleHunkReviewed => app.toggle_hunk_reviewed(),
        Action::ToggleFocus => {
            let has_selector = app.has_inline_commit_selector();
            let has_comments = app.has_comment_navigator_items();
            // Cycle: FileList -> Diff -> CommitSelector -> Comments -> FileList,
            // skipping panels that aren't present.
            app.focused_panel = match (app.focused_panel, has_selector, has_comments) {
                (FocusedPanel::FileList, _, _) => FocusedPanel::Diff,
                (FocusedPanel::Diff, true, _) => FocusedPanel::CommitSelector,
                (FocusedPanel::Diff, false, true) => FocusedPanel::Comments,
                (FocusedPanel::Diff, false, false) => FocusedPanel::FileList,
                (FocusedPanel::CommitSelector, _, true) => FocusedPanel::Comments,
                (FocusedPanel::CommitSelector, _, false) => FocusedPanel::FileList,
                (FocusedPanel::Comments, _, _) => FocusedPanel::FileList,
            };
            if matches!(
                app.focused_panel,
                FocusedPanel::FileList | FocusedPanel::Comments
            ) {
                app.show_file_list = true;
            }
        }
        Action::ToggleFocusReverse => {
            let has_selector = app.has_inline_commit_selector();
            let has_comments = app.has_comment_navigator_items();
            app.focused_panel = match (app.focused_panel, has_selector, has_comments) {
                (FocusedPanel::FileList, _, true) => FocusedPanel::Comments,
                (FocusedPanel::FileList, true, false) => FocusedPanel::CommitSelector,
                (FocusedPanel::FileList, false, false) => FocusedPanel::Diff,
                (FocusedPanel::Comments, true, _) => FocusedPanel::CommitSelector,
                (FocusedPanel::Comments, false, _) => FocusedPanel::Diff,
                (FocusedPanel::Diff, _, _) => FocusedPanel::FileList,
                (FocusedPanel::CommitSelector, _, _) => FocusedPanel::Diff,
            };
            if matches!(
                app.focused_panel,
                FocusedPanel::FileList | FocusedPanel::Comments
            ) {
                app.show_file_list = true;
            }
        }
        Action::ExpandAll => {
            app.expand_all_dirs();
            app.set_message("All directories expanded");
        }
        Action::CollapseAll => {
            app.collapse_all_dirs();
            app.set_message("All directories collapsed");
        }
        Action::ToggleHelp => app.toggle_help(),
        Action::EnterCommandMode => app.enter_command_mode(),
        Action::EnterSearchMode => app.enter_search_mode(),
        Action::AddLineComment => {
            let line = app.get_line_at_cursor();
            if line.is_some() {
                app.enter_comment_mode(false, line);
            } else {
                app.set_message("Move cursor to a diff line to add a line comment");
            }
        }
        Action::AddFileComment => app.enter_comment_mode(true, None),
        // `i` edits the comment at cursor. In vim mode the text cursor starts at
        // the beginning; otherwise (and for `A`) it starts at the end.
        Action::EditComment => edit_comment_at_cursor(app, !app.comment_vim_enabled),
        // `A` (vim only) edits with the text cursor at end-of-line.
        Action::EditCommentAtEnd if app.comment_vim_enabled => edit_comment_at_cursor(app, true),
        Action::ExportToClipboard => {
            handle_export(app);
        }
        Action::CopyCommentAtCursor => handle_copy_comment_at_cursor(app),
        Action::SearchNext => {
            app.search_next_in_diff();
        }
        Action::SearchPrev => {
            app.search_prev_in_diff();
        }
        Action::ClearSearchHighlight => app.clear_search_highlight(),
        Action::EnterVisualMode => {
            if app.get_line_at_cursor().is_some() {
                app.enter_visual_mode_at_cursor();
            } else {
                app.set_message("Move cursor to a diff line to start visual selection");
            }
        }
        // `(` moves toward the top row, `)` toward the bottom row. The cycle
        // methods walk data indices (newest-first), so ascending display order
        // swaps which one runs to keep the on-screen direction stable.
        Action::CycleCommitNext if app.has_review_commits() => {
            if app.commits_ascending() {
                app.cycle_commit_prev();
            } else {
                app.cycle_commit_next();
            }
            if let Err(e) = app.reload_inline_selection_for_source() {
                app.set_error(format!("Failed to load diff: {e}"));
            }
        }
        Action::CycleCommitPrev if app.has_review_commits() => {
            if app.commits_ascending() {
                app.cycle_commit_next();
            } else {
                app.cycle_commit_prev();
            }
            if let Err(e) = app.reload_inline_selection_for_source() {
                app.set_error(format!("Failed to load diff: {e}"));
            }
        }
        Action::EditFile => app.queue_editor_for_focused_item(),
        _ => {}
    }
}

/// Handle actions in the submit resolver modal: pick what to do with each
/// comment that did not map to an inline GitHub review comment.
pub fn handle_submit_resolver_action(app: &mut App, action: Action) {
    match action {
        Action::SubmitResolverDown => app.submit_resolver_cursor_down(),
        Action::SubmitResolverUp => app.submit_resolver_cursor_up(),
        Action::SubmitResolverToggle => app.submit_resolver_toggle(),
        Action::SubmitResolverAdvance => app.submit_resolver_advance(),
        Action::ExitMode => app.cancel_submit(),
        Action::Quit => app.should_quit = true,
        _ => {}
    }
}

/// Handle actions in the bare-`:submit` action picker. Up/down move the
/// cursor through Comment/Approve/Request changes/Draft; Enter dispatches
/// preflight with the picked event (skipping the confirmation modal); Esc
/// cancels back to normal.
pub fn handle_submit_action_picker_action(app: &mut App, action: Action) {
    match action {
        Action::SubmitPickerDown => app.submit_picker_cursor_down(),
        Action::SubmitPickerUp => app.submit_picker_cursor_up(),
        Action::SubmitPickerConfirm => app.submit_picker_confirm(),
        Action::ExitMode => app.cancel_submit_action_picker(),
        Action::Quit => app.should_quit = true,
        Action::QuitHint => app.set_message(QUIT_HINT_MESSAGE),
        _ => {}
    }
}

/// Handle actions in the final submit confirmation modal.
pub fn handle_submit_confirm_action(app: &mut App, action: Action) {
    match action {
        Action::ConfirmYes => app.confirm_submit(),
        Action::ConfirmNo => app.cancel_submit(),
        // Only meaningful when the stale-head warning is visible.
        Action::SubmitReloadPr
            if app.submit_head_is_stale()
                && matches!(app.diff_source, app::DiffSource::PullRequest(_)) =>
        {
            app.cancel_submit();
            if let Err(e) = app.spawn_pr_reload() {
                app.set_error(format!("Reload failed: {e}"));
            }
        }
        Action::Quit => app.should_quit = true,
        _ => {}
    }
}

#[cfg(test)]
mod command_tests {
    use super::{CommandKind, QuitCopyPlan, WriteQuitKey, command_spec_for, quit_copy_plan};
    use crate::app::CopyOnQuit;

    #[test]
    fn plans_the_clipboard_step_for_every_policy_and_quit_key() {
        // `ask` keeps the legacy split: `:wq` prompts, `ZZ` copies.
        assert_eq!(
            quit_copy_plan(CopyOnQuit::Ask, WriteQuitKey::Command),
            QuitCopyPlan::Prompt
        );
        assert_eq!(
            quit_copy_plan(CopyOnQuit::Ask, WriteQuitKey::ZZ),
            QuitCopyPlan::CopyAndQuit
        );
        for key in [WriteQuitKey::Command, WriteQuitKey::ZZ] {
            assert_eq!(
                quit_copy_plan(CopyOnQuit::Always, key),
                QuitCopyPlan::CopyAndQuit
            );
            assert_eq!(quit_copy_plan(CopyOnQuit::Never, key), QuitCopyPlan::Quit);
        }
    }

    #[test]
    fn parses_relative_line_number_commands() {
        assert_eq!(
            command_spec_for("set relativenumber").map(|spec| spec.kind),
            Some(CommandKind::SetRelativeLineNumbers(true))
        );
        assert_eq!(
            command_spec_for("set norelativenumber").map(|spec| spec.kind),
            Some(CommandKind::SetRelativeLineNumbers(false))
        );
        assert_eq!(
            command_spec_for("set relativenumber!").map(|spec| spec.kind),
            Some(CommandKind::ToggleRelativeLineNumbers)
        );
    }

    #[test]
    fn parses_word_diff_commands() {
        assert_eq!(
            command_spec_for("set worddiff").map(|spec| spec.kind),
            Some(CommandKind::SetWordDiff(true))
        );
        assert_eq!(
            command_spec_for("set noworddiff").map(|spec| spec.kind),
            Some(CommandKind::SetWordDiff(false))
        );
        assert_eq!(
            command_spec_for("set worddiff!").map(|spec| spec.kind),
            Some(CommandKind::ToggleWordDiff)
        );
    }

    #[test]
    fn parses_minimal_ui_commands() {
        assert_eq!(
            command_spec_for("set minimal").map(|spec| spec.kind),
            Some(CommandKind::SetMinimalUi(true))
        );
        assert_eq!(
            command_spec_for("set nominimal").map(|spec| spec.kind),
            Some(CommandKind::SetMinimalUi(false))
        );
        assert_eq!(
            command_spec_for("set minimal!").map(|spec| spec.kind),
            Some(CommandKind::ToggleMinimalUi)
        );
    }

    #[test]
    fn parses_file_command_paths() {
        assert_eq!(
            super::file_command_path("file review.md"),
            Some("review.md")
        );
        assert_eq!(super::file_command_path("file   a b.md "), Some("a b.md"));
        assert_eq!(super::file_command_path("file"), Some(""));
        assert_eq!(super::file_command_path("files"), None);
        assert_eq!(super::file_command_path("focus"), None);
    }

    #[test]
    fn parses_copy_url_command() {
        assert_eq!(
            command_spec_for("copy-url").map(|spec| spec.kind),
            Some(CommandKind::CopyUrl)
        );
    }

    #[test]
    fn parses_every_reviewed_visibility_command_form() {
        // Mirrors `:set commits` / `:set nocommits` / `:set commits!`, plus a
        // bare toggle alias in the shape of `:wrap`.
        assert_eq!(
            command_spec_for("set reviewed").map(|spec| spec.kind),
            Some(CommandKind::SetShowReviewed(true))
        );
        assert_eq!(
            command_spec_for("set noreviewed").map(|spec| spec.kind),
            Some(CommandKind::SetShowReviewed(false))
        );
        assert_eq!(
            command_spec_for("set reviewed!").map(|spec| spec.kind),
            Some(CommandKind::ToggleShowReviewed)
        );
        assert_eq!(
            command_spec_for("reviewed").map(|spec| spec.kind),
            Some(CommandKind::ToggleShowReviewed)
        );
    }
}
