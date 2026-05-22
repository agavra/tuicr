use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Position;

use crate::app::{
    self, App, ExpandDirection, FileTreeItem, FocusedPanel, GapCursorHit, InputMode, TargetTab,
    VisualSelection,
};
use crate::input::Action;
use crate::model::{ClearScope, LineSide};
use crate::output::{export_to_clipboard, generate_export_content};
use crate::text_edit::{
    delete_char_before, delete_word_before, next_char_boundary, prev_char_boundary,
};

const WHEEL_LINES: usize = 3;

pub fn handle_mouse_event(app: &mut App, event: MouseEvent) {
    let pos = Position::new(event.column, event.row);
    match event.kind {
        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
            let scroll_up = matches!(event.kind, MouseEventKind::ScrollUp);
            let action = if scroll_up {
                Action::MouseScrollUp(WHEEL_LINES)
            } else {
                Action::MouseScrollDown(WHEEL_LINES)
            };
            let over_file_list = app.file_list_area.is_some_and(|r| r.contains(pos));
            let over_diff = app.diff_area.is_some_and(|r| r.contains(pos));
            let over_commit_list = app.commit_list_inner_area.is_some_and(|r| r.contains(pos));
            match app.input_mode {
                InputMode::Help => handle_help_action(app, action),
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
        MouseEventKind::Down(MouseButton::Left)
            if matches!(app.input_mode, InputMode::Normal | InputMode::VisualSelect) =>
        {
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
    for _ in 0..WHEEL_LINES {
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
        handle_commit_selector_action(app, Action::SelectFile);
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

/// Export review: either to clipboard or set pending stdout output based on app.output_to_stdout.
/// When output_to_stdout is true, stores the content and sets should_quit.
fn handle_export(app: &mut App) {
    let slug = app.session_slug();
    if app.output_to_stdout {
        match generate_export_content(
            &app.session,
            &app.diff_source,
            &app.comment_types,
            app.export_legend,
            &app.forge_review_threads,
            slug.as_deref(),
        ) {
            Ok(content) => {
                app.pending_stdout_output = Some(content);
                app.should_quit = true;
            }
            Err(e) => app.set_warning(format!("{e}")),
        }
    } else {
        match export_to_clipboard(
            &app.session,
            &app.diff_source,
            &app.comment_types,
            app.export_legend,
            &app.forge_review_threads,
            slug.as_deref(),
        ) {
            Ok(msg) => app.set_message(msg),
            Err(e) => app.set_warning(format!("{e}")),
        }
    }
}

/// Export and quit (used by ZZ keybinding).
/// When --stdout is set, stores export content and quits.
/// Otherwise, exports to clipboard and quits.
pub fn handle_export_and_quit(app: &mut App) {
    handle_export(app);
    app.should_quit = true;
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
        Action::HalfPageDown => app.help_scroll_down(app.help_state.viewport_height / 2),
        Action::HalfPageUp => app.help_scroll_up(app.help_state.viewport_height / 2),
        Action::PageDown => app.help_scroll_down(app.help_state.viewport_height),
        Action::PageUp => app.help_scroll_up(app.help_state.viewport_height),
        Action::GoToTop => app.help_scroll_to_top(),
        Action::GoToBottom => app.help_scroll_to_bottom(),
        Action::MouseScrollDown(n) => app.help_scroll_down(n),
        Action::MouseScrollUp(n) => app.help_scroll_up(n),
        Action::ToggleHelp => app.toggle_help(),
        Action::Quit => app.should_quit = true,
        _ => {}
    }
}

/// Handle actions in Command mode (text input for :commands)
pub fn handle_command_action(app: &mut App, action: Action) {
    match action {
        Action::InsertChar(c) => app.command_buffer.push(c),
        Action::Paste(text) => push_single_line(&mut app.command_buffer, &text),
        Action::DeleteChar => {
            app.command_buffer.pop();
        }
        Action::ExitMode => app.exit_command_mode(),
        Action::SubmitInput => {
            let cmd = app.command_buffer.trim().to_string();
            match cmd.as_str() {
                "q" | "quit" => {
                    if app.dirty {
                        app.set_error("No write since last change (add ! to override)");
                    } else {
                        app.should_quit = true;
                    }
                }
                "q!" | "quit!" => app.should_quit = true,
                "w" | "write" => match app.save_current_session_merging_external() {
                    Ok(path) => {
                        app.set_message(format!("Saved to {}", path.display()));
                    }
                    Err(e) => app.set_error(format!("Save failed: {e}")),
                },
                "x" | "wq" => match app.save_current_session_merging_external() {
                    Ok(_) => {
                        if app.session.has_comments() {
                            if app.output_to_stdout {
                                // Skip confirmation dialog, export directly
                                handle_export(app);
                                return;
                            }
                            app.exit_command_mode();
                            app.enter_confirm_mode(app::ConfirmAction::CopyAndQuit);
                            return;
                        } else {
                            app.should_quit = true;
                        }
                    }
                    Err(e) => app.set_error(format!("Save failed: {e}")),
                },
                "e" | "reload" => {
                    let comment_reload = app.reload_persisted_session_if_changed(true);
                    if matches!(app.diff_source, app::DiffSource::PullRequest(_)) {
                        if let Err(e) = comment_reload {
                            app.set_warning(format!("Comment reload failed: {e}"));
                        }
                        // Async: shows a spinner in the status bar; result
                        // is applied in `poll_pr_reload_events` and the
                        // cursor is restored to the captured anchor.
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
                                    app.set_message(format!(
                                        "Reloaded {count} files{comment_suffix}"
                                    ));
                                }
                            }
                            Err(e) => app.set_error(format!("Reload failed: {e}")),
                        }
                    }
                }
                "clip" | "export" => handle_export(app),
                "clear" => app.clear_comments(ClearScope::CommentsAndReviewed),
                "clearc" => app.clear_comments(ClearScope::CommentsOnly),
                "version" => {
                    app.set_message(format!("tuicr v{}", env!("CARGO_PKG_VERSION")));
                }
                "update" => match crate::update::check_for_updates() {
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
                },
                "set wrap" => app.set_diff_wrap(true),
                "set wrap!" => app.toggle_diff_wrap(),
                "set commits" => {
                    app.show_commit_selector = true;
                    app.set_message("Commit selector: visible");
                }
                "set nocommits" => {
                    app.show_commit_selector = false;
                    if app.focused_panel == FocusedPanel::CommitSelector {
                        app.focused_panel = FocusedPanel::Diff;
                    }
                    app.set_message("Commit selector: hidden");
                }
                "set commits!" => {
                    app.show_commit_selector = !app.show_commit_selector;
                    if !app.show_commit_selector
                        && app.focused_panel == FocusedPanel::CommitSelector
                    {
                        app.focused_panel = FocusedPanel::Diff;
                    }
                    let status = if app.show_commit_selector {
                        "visible"
                    } else {
                        "hidden"
                    };
                    app.set_message(format!("Commit selector: {status}"));
                }
                "diff" => app.toggle_diff_view_mode(),
                "focus" | "f" => app.toggle_single_file_view(),
                "stage" => app.stage_reviewed_files(),
                "commits" | "targets" => {
                    if let Err(e) = app.enter_target_selector(TargetTab::Local) {
                        app.set_error(format!("Failed to load commits: {e}"));
                    } else {
                        return;
                    }
                }
                "prs" => {
                    if let Err(e) = app.enter_target_selector(TargetTab::PullRequests) {
                        app.set_error(format!("Failed to open PR selector: {e}"));
                    } else {
                        return;
                    }
                }
                "submit" => {
                    app.exit_command_mode();
                    app.start_submit_action_picker();
                    return;
                }
                "submit comment" => {
                    app.exit_command_mode();
                    app.start_submit(crate::forge::submit::SubmitEvent::Comment);
                    return;
                }
                "submit approve" => {
                    app.exit_command_mode();
                    app.start_submit(crate::forge::submit::SubmitEvent::Approve);
                    return;
                }
                "submit request-changes" => {
                    app.exit_command_mode();
                    app.start_submit(crate::forge::submit::SubmitEvent::RequestChanges);
                    return;
                }
                "submit draft" => {
                    app.exit_command_mode();
                    app.start_submit(crate::forge::submit::SubmitEvent::Draft);
                    return;
                }
                "comments unresolved" | "comments all" | "comments hide" => {
                    use crate::forge::remote_comments::PrCommentsVisibility;
                    if !matches!(app.diff_source, app::DiffSource::PullRequest(_)) {
                        app.set_warning(":comments only applies in PR mode");
                    } else {
                        let new_visibility = match cmd.as_str() {
                            "comments unresolved" => PrCommentsVisibility::Unresolved,
                            "comments all" => PrCommentsVisibility::All,
                            "comments hide" => PrCommentsVisibility::Hide,
                            _ => unreachable!(),
                        };
                        let changed = app.set_remote_comments_visibility(new_visibility);
                        let label = new_visibility.label();
                        if changed {
                            app.set_message(format!("Remote comments: {label}"));
                        } else {
                            app.set_message(format!("Remote comments: already {label}"));
                        }
                    }
                }
                _ => {
                    if let Some((lineno, side)) = parse_lineno_command(&cmd) {
                        app.go_to_source_line(lineno, side);
                    } else {
                        app.set_message(format!("Unknown command: {cmd}"));
                    }
                }
            }
            app.exit_command_mode();
        }
        Action::Quit => app.should_quit = true,
        _ => {}
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
            app.search_in_diff_from_cursor();
            app.exit_search_mode();
        }
        Action::Quit => app.should_quit = true,
        _ => {}
    }
}

/// Handle actions in Comment mode (text input for comments)
pub fn handle_comment_action(app: &mut App, action: Action) {
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
    match action {
        Action::ConfirmYes => {
            if let Some(app::ConfirmAction::CopyAndQuit) = app.pending_confirm {
                let slug = app.session_slug();
                if app.output_to_stdout {
                    match generate_export_content(
                        &app.session,
                        &app.diff_source,
                        &app.comment_types,
                        app.export_legend,
                        &app.forge_review_threads,
                        slug.as_deref(),
                    ) {
                        Ok(content) => app.pending_stdout_output = Some(content),
                        Err(e) => app.set_warning(format!("{e}")),
                    }
                } else {
                    match export_to_clipboard(
                        &app.session,
                        &app.diff_source,
                        &app.comment_types,
                        app.export_legend,
                        &app.forge_review_threads,
                        slug.as_deref(),
                    ) {
                        Ok(msg) => app.set_message(msg),
                        Err(e) => app.set_warning(format!("{e}")),
                    }
                }
            }
            app.exit_confirm_mode();
            app.should_quit = true;
        }
        Action::ConfirmNo => {
            app.exit_confirm_mode();
            app.should_quit = true;
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
        Action::Quit => app.should_quit = true,
        Action::ExitMode => {
            // Esc during an in-flight PR open aborts the load and stays
            // in the selector. Takes precedence over the
            // commit-selection-range exit so the user isn't stuck staring
            // at a spinner.
            if app.cancel_pr_open() {
                return;
            }
            if app.commit_selection_range.is_none() {
                return;
            }
            if let Err(e) = app.exit_commit_select_mode() {
                app.set_error(format!("Failed to reload changes: {e}"));
            }
        }
        other => match app.target_tab {
            TargetTab::Local => handle_local_target_action(app, other),
            TargetTab::PullRequests => handle_pr_target_action(app, other),
        },
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
        Action::CursorDown(_) => app.commit_select_down(),
        Action::CursorUp(_) => app.commit_select_up(),
        // Toggle + auto-advance so repeated presses sweep a contiguous run.
        Action::ToggleExpand | Action::ToggleCommitSelect | Action::SelectFile => {
            app.toggle_commit_selection_and_advance();
            if matches!(app.diff_source, crate::app::DiffSource::PullRequest(_)) {
                // PR mode reloads via the forge `compare` API on a
                // background thread; persist the new range so it survives
                // a restart.
                app.persist_pr_commit_selection_range();
                app.reload_pr_inline_selection();
            } else if let Err(e) = app.reload_inline_selection() {
                app.set_error(format!("Failed to load diff: {e}"));
            }
        }
        Action::ExitMode => {
            app.focused_panel = FocusedPanel::Diff;
        }
        _ => handle_shared_normal_action(app, action),
    }
}

/// Handle actions in VisualSelect mode
pub fn handle_visual_action(app: &mut App, action: Action) {
    match action {
        Action::CursorDown(n) => {
            app.cursor_down(n);
            app.extend_visual_to_cursor();
        }
        Action::CursorUp(n) => {
            app.cursor_up(n);
            app.extend_visual_to_cursor();
        }
        Action::AddRangeComment => {
            if app.visual_selection_line_range().is_some() {
                app.enter_comment_from_visual();
            } else {
                app.set_warning("Invalid selection - move cursor to a diff line");
                app.exit_visual_mode();
            }
        }
        Action::ExportToClipboard => {
            match app.copy_visual_selection() {
                Ok(0) => app.set_message("Nothing to copy"),
                Ok(n) => app.set_message(format!("Copied {n} chars")),
                Err(e) => app.set_warning(format!("{e}")),
            }
            app.exit_visual_mode();
        }
        Action::ExitMode => app.exit_visual_mode(),
        Action::Quit => app.should_quit = true,
        Action::ScrollViewDown(n) | Action::MouseScrollDown(n) => app.scroll_view_down(n),
        Action::ScrollViewUp(n) | Action::MouseScrollUp(n) => app.scroll_view_up(n),
        Action::HalfPageDown => app.scroll_down(app.diff_state.viewport_height / 2),
        Action::HalfPageUp => app.scroll_up(app.diff_state.viewport_height / 2),
        Action::PageDown => app.scroll_down(app.diff_state.viewport_height),
        Action::PageUp => app.scroll_up(app.diff_state.viewport_height),
        _ => {}
    }
    clear_visual_if_cursor_offscreen(app);
}

/// Handle actions when file list panel is focused
pub fn handle_file_list_action(app: &mut App, action: Action) {
    match action {
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

/// Handle actions when the comment navigator panel is focused
pub fn handle_comment_navigator_action(app: &mut App, action: Action) {
    match action {
        Action::CursorDown(n) => app.comment_navigator_down(n),
        Action::CursorUp(n) => app.comment_navigator_up(n),
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
            if app.dirty && !app.quit_warned {
                app.set_sticky_warning("Unsaved changes. Press q again to quit.");
                app.quit_warned = true;
            } else {
                app.should_quit = true;
            }
        }
        Action::HalfPageDown => app.scroll_down(app.diff_state.viewport_height / 2),
        Action::HalfPageUp => app.scroll_up(app.diff_state.viewport_height / 2),
        Action::PageDown => app.scroll_down(app.diff_state.viewport_height),
        Action::PageUp => app.scroll_up(app.diff_state.viewport_height),
        Action::GoToTop => app.jump_to_file(0),
        Action::GoToBottom => app.jump_to_bottom(),
        Action::NextFile => app.next_file(),
        Action::PrevFile => app.prev_file(),
        Action::NextHunk => app.next_hunk(),
        Action::PrevHunk => app.prev_hunk(),
        Action::ToggleReviewed => app.toggle_reviewed(),
        Action::ToggleFocus => {
            let has_selector = app.has_inline_commit_selector();
            let has_comments = app.has_comment_navigator_items();
            app.focused_panel = match (app.focused_panel, has_selector, has_comments) {
                (FocusedPanel::FileList, _, true) => FocusedPanel::Comments,
                (FocusedPanel::FileList, _, false) => FocusedPanel::Diff,
                (FocusedPanel::Comments, _, _) => FocusedPanel::Diff,
                (FocusedPanel::Diff, true, _) => FocusedPanel::CommitSelector,
                (FocusedPanel::Diff, false, _) => FocusedPanel::FileList,
                (FocusedPanel::CommitSelector, _, _) => FocusedPanel::FileList,
            };
        }
        Action::ToggleFocusReverse => {
            let has_selector = app.has_inline_commit_selector();
            let has_comments = app.has_comment_navigator_items();
            app.focused_panel = match (app.focused_panel, has_selector, has_comments) {
                (FocusedPanel::FileList, true, _) => FocusedPanel::CommitSelector,
                (FocusedPanel::FileList, false, _) => FocusedPanel::Diff,
                (FocusedPanel::Comments, _, _) => FocusedPanel::FileList,
                (FocusedPanel::Diff, _, true) => FocusedPanel::Comments,
                (FocusedPanel::Diff, _, false) => FocusedPanel::FileList,
                (FocusedPanel::CommitSelector, _, _) => FocusedPanel::Diff,
            };
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
        Action::EditComment if app.cursor_on_locked_comment() => {
            app.set_message("Comment already pushed to GitHub — read only in tuicr");
        }
        Action::EditComment if !app.enter_edit_mode() => {
            if app.cursor_on_remote_thread() {
                app.set_message("GitHub comment — read only in tuicr");
            } else {
                app.set_message("No comment at cursor");
            }
        }
        Action::ExportToClipboard => handle_export(app),
        Action::SearchNext => {
            app.search_next_in_diff();
        }
        Action::SearchPrev => {
            app.search_prev_in_diff();
        }
        Action::EnterVisualMode => {
            if app.get_line_at_cursor().is_some() {
                app.enter_visual_mode_at_cursor();
            } else {
                app.set_message("Move cursor to a diff line to start visual selection");
            }
        }
        Action::CycleCommitNext if app.has_inline_commit_selector() => {
            app.cycle_commit_next();
            if let Err(e) = app.reload_inline_selection() {
                app.set_error(format!("Failed to load diff: {e}"));
            }
        }
        Action::CycleCommitPrev if app.has_inline_commit_selector() => {
            app.cycle_commit_prev();
            if let Err(e) = app.reload_inline_selection() {
                app.set_error(format!("Failed to load diff: {e}"));
            }
        }
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
