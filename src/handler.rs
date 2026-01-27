use crate::app::{self, App, FileTreeItem, FocusedPanel};
use crate::input::Action;
use crate::output::{export_to_clipboard, generate_export_content};
use crate::persistence::save_session;
use crate::text_edit::{
    delete_char_before, delete_word_before, next_char_boundary, prev_char_boundary,
};

/// Export review: either to clipboard or set pending stdout output based on app.output_to_stdout.
/// When output_to_stdout is true, stores the content and sets should_quit.
fn handle_export(app: &mut App) {
    if app.output_to_stdout {
        match generate_export_content(&app.session, &app.diff_source) {
            Ok(content) => {
                app.pending_stdout_output = Some(content);
                app.should_quit = true;
            }
            Err(e) => app.set_warning(format!("{e}")),
        }
    } else {
        match export_to_clipboard(&app.session, &app.diff_source) {
            Ok(msg) => app.set_message(msg),
            Err(e) => app.set_warning(format!("{e}")),
        }
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
                "w" | "write" => match save_session(&app.session) {
                    Ok(path) => {
                        app.dirty = false;
                        app.set_message(format!("Saved to {}", path.display()));
                    }
                    Err(e) => app.set_error(format!("Save failed: {e}")),
                },
                "x" | "wq" => match save_session(&app.session) {
                    Ok(_) => {
                        app.dirty = false;
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
                "e" | "reload" => match app.reload_diff_files() {
                    Ok(count) => app.set_message(format!("Reloaded {count} files")),
                    Err(e) => app.set_error(format!("Reload failed: {e}")),
                },
                "clip" | "export" => handle_export(app),
                "clear" => app.clear_all_comments(),
                "version" => {
                    app.set_message(format!("tuicr v{}", env!("CARGO_PKG_VERSION")));
                }
                "set wrap" => app.set_diff_wrap(true),
                "set wrap!" => app.toggle_diff_wrap(),
                "diff" => app.toggle_diff_view_mode(),
                "commits" => {
                    if let Err(e) = app.enter_commit_select_mode() {
                        app.set_error(format!("Failed to load commits: {e}"));
                    } else {
                        return;
                    }
                }
                _ => app.set_message(format!("Unknown command: {cmd}")),
            }
            app.exit_command_mode();
        }
        Action::Quit => app.should_quit = true,
        _ => {}
    }
}

/// Handle actions in Search mode (text input for /pattern)
pub fn handle_search_action(app: &mut App, action: Action) {
    match action {
        Action::InsertChar(c) => app.search_buffer.push(c),
        Action::DeleteChar => {
            app.search_buffer.pop();
        }
        Action::DeleteWord => {
            if !app.search_buffer.is_empty() {
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
        Action::DeleteChar => {
            app.comment_cursor = delete_char_before(&mut app.comment_buffer, app.comment_cursor);
        }
        Action::ExitMode => app.exit_comment_mode(),
        Action::SubmitInput => app.save_comment(),
        Action::CycleCommentType => app.cycle_comment_type(),
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
                if app.output_to_stdout {
                    match generate_export_content(&app.session, &app.diff_source) {
                        Ok(content) => app.pending_stdout_output = Some(content),
                        Err(e) => app.set_warning(format!("{e}")),
                    }
                } else {
                    match export_to_clipboard(&app.session, &app.diff_source) {
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

/// Handle actions in CommitSelect mode
pub fn handle_commit_select_action(app: &mut App, action: Action) {
    match action {
        Action::CommitSelectUp => app.commit_select_up(),
        Action::CommitSelectDown => app.commit_select_down(),
        Action::ToggleCommitSelect => app.toggle_commit_selection(),
        Action::ConfirmCommitSelect => {
            if let Err(e) = app.confirm_commit_selection() {
                app.set_error(format!("Failed to load commits: {e}"));
            }
        }
        Action::ExitMode => {
            if let Err(e) = app.exit_commit_select_mode() {
                app.set_error(format!("Failed to reload working tree: {e}"));
            }
        }
        Action::Quit => app.should_quit = true,
        _ => {}
    }
}

/// Handle actions in VisualSelect mode
pub fn handle_visual_action(app: &mut App, action: Action) {
    match action {
        Action::CursorDown(n) => {
            app.cursor_down(n);
            // Check if selection crosses sides
            if let Some((_, anchor_side)) = app.visual_anchor
                && let Some((_, current_side)) = app.get_line_at_cursor()
                && anchor_side != current_side
            {
                app.set_warning("Cannot select across old/new sides");
            }
        }
        Action::CursorUp(n) => {
            app.cursor_up(n);
            // Check if selection crosses sides
            if let Some((_, anchor_side)) = app.visual_anchor
                && let Some((_, current_side)) = app.get_line_at_cursor()
                && anchor_side != current_side
            {
                app.set_warning("Cannot select across old/new sides");
            }
        }
        Action::AddRangeComment => {
            if app.get_visual_selection().is_some() {
                app.enter_comment_from_visual();
            } else {
                app.set_warning("Invalid selection - cannot span old and new lines");
                app.exit_visual_mode();
            }
        }
        Action::ExitMode => app.exit_visual_mode(),
        Action::Quit => app.should_quit = true,
        _ => {}
    }
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
                    FileTreeItem::File { file_idx, .. } => app.jump_to_file(file_idx),
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

/// Handle actions when diff panel is focused
pub fn handle_diff_action(app: &mut App, action: Action) {
    match action {
        Action::CursorDown(n) => app.cursor_down(n),
        Action::CursorUp(n) => app.cursor_up(n),
        Action::ScrollLeft(n) => app.scroll_left(n),
        Action::ScrollRight(n) => app.scroll_right(n),
        Action::MouseScrollDown(n) => app.viewport_scroll_down(n),
        Action::MouseScrollUp(n) => app.viewport_scroll_up(n),
        Action::SelectFile => {
            // Check if cursor is on an expander line or expanded content
            if let Some((gap_id, is_expanded)) = app.get_gap_at_cursor() {
                if is_expanded {
                    // Collapse expanded content
                    app.collapse_gap(gap_id);
                } else {
                    // Expand the gap
                    if let Err(e) = app.expand_gap(gap_id) {
                        app.set_error(format!("Failed to expand: {e}"));
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
                app.set_warning("Unsaved changes. Press q again to quit.");
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
        Action::GoToBottom => {
            let last = app.file_count().saturating_sub(1);
            app.jump_to_file(last);
        }
        Action::NextFile => app.next_file(),
        Action::PrevFile => app.prev_file(),
        Action::NextHunk => app.next_hunk(),
        Action::PrevHunk => app.prev_hunk(),
        Action::ToggleReviewed => app.toggle_reviewed(),
        Action::ToggleFocus => {
            app.focused_panel = match app.focused_panel {
                FocusedPanel::FileList => FocusedPanel::Diff,
                FocusedPanel::Diff => FocusedPanel::FileList,
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
        Action::EditComment => {
            if !app.enter_edit_mode() {
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
            if let Some((line, side)) = app.get_line_at_cursor() {
                app.enter_visual_mode(line, side);
            } else {
                app.set_message("Move cursor to a diff line to start visual selection");
            }
        }
        _ => {}
    }
}
