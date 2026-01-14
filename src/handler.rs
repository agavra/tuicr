use crate::app::{self, App, FileTreeItem, FocusedPanel};
use crate::input::Action;
use crate::output::export_to_clipboard;
use crate::persistence::save_session;

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
                "q" | "quit" => app.should_quit = true,
                "w" | "write" => match save_session(&app.session) {
                    Ok(path) => {
                        app.dirty = false;
                        app.set_message(format!("Saved to {}", path.display()));
                    }
                    Err(e) => app.set_error(format!("Save failed: {}", e)),
                },
                "x" | "wq" => match save_session(&app.session) {
                    Ok(_) => {
                        app.dirty = false;
                        if app.session.has_comments() {
                            app.exit_command_mode();
                            app.enter_confirm_mode(app::ConfirmAction::CopyAndQuit);
                            return;
                        } else {
                            app.should_quit = true;
                        }
                    }
                    Err(e) => app.set_error(format!("Save failed: {}", e)),
                },
                "e" | "reload" => match app.reload_diff_files() {
                    Ok(count) => app.set_message(format!("Reloaded {} files", count)),
                    Err(e) => app.set_error(format!("Reload failed: {}", e)),
                },
                "clip" | "export" => match export_to_clipboard(&app.session, &app.diff_source) {
                    Ok(msg) => app.set_message(msg),
                    Err(e) => app.set_warning(format!("{}", e)),
                },
                "clear" => app.clear_all_comments(),
                "version" => {
                    app.set_message(format!("tuicr v{}", env!("CARGO_PKG_VERSION")));
                }
                "set wrap" => app.set_diff_wrap(true),
                "set wrap!" => app.toggle_diff_wrap(),
                _ => app.set_message(format!("Unknown command: {}", cmd)),
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
            app.comment_cursor += 1;
        }
        Action::DeleteChar => {
            if app.comment_cursor > 0 {
                app.comment_cursor -= 1;
                app.comment_buffer.remove(app.comment_cursor);
            }
        }
        Action::ExitMode => app.exit_comment_mode(),
        Action::SubmitInput => app.save_comment(),
        Action::CycleCommentType => app.cycle_comment_type(),
        Action::TextCursorLeft => {
            if app.comment_cursor > 0 {
                app.comment_cursor -= 1;
            }
        }
        Action::TextCursorRight => {
            if app.comment_cursor < app.comment_buffer.len() {
                app.comment_cursor += 1;
            }
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
            if app.comment_cursor > 0 {
                // Delete backwards to start of word or start of buffer
                while app.comment_cursor > 0
                    && app
                        .comment_buffer
                        .chars()
                        .nth(app.comment_cursor - 1)
                        .map(|c| c.is_whitespace())
                        .unwrap_or(false)
                {
                    app.comment_cursor -= 1;
                    app.comment_buffer.remove(app.comment_cursor);
                }
                while app.comment_cursor > 0
                    && app
                        .comment_buffer
                        .chars()
                        .nth(app.comment_cursor - 1)
                        .map(|c| !c.is_whitespace())
                        .unwrap_or(false)
                {
                    app.comment_cursor -= 1;
                    app.comment_buffer.remove(app.comment_cursor);
                }
            }
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
                match export_to_clipboard(&app.session, &app.diff_source) {
                    Ok(msg) => app.set_message(msg),
                    Err(e) => app.set_warning(format!("{}", e)),
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
                app.set_error(format!("Failed to load commits: {}", e));
            }
        }
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
        Action::SelectFile => {
            // Check if cursor is on an expander line or expanded content
            if let Some((gap_id, is_expanded)) = app.get_gap_at_cursor() {
                if is_expanded {
                    // Collapse expanded content
                    app.collapse_gap(gap_id);
                } else {
                    // Expand the gap
                    if let Err(e) = app.expand_gap(gap_id) {
                        app.set_error(format!("Failed to expand: {}", e));
                    }
                }
            }
        }
        _ => handle_shared_normal_action(app, action),
    }
}

/// Handle actions shared between file list and diff panels in Normal mode
fn handle_shared_normal_action(app: &mut App, action: Action) {
    match action {
        Action::Quit => app.should_quit = true,
        Action::HalfPageDown => app.scroll_down(15),
        Action::HalfPageUp => app.scroll_up(15),
        Action::PageDown => app.scroll_down(30),
        Action::PageUp => app.scroll_up(30),
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
        Action::ToggleDiffView => app.toggle_diff_view_mode(),
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
        Action::ExportToClipboard => match export_to_clipboard(&app.session, &app.diff_source) {
            Ok(msg) => app.set_message(msg),
            Err(e) => app.set_warning(format!("{}", e)),
        },
        Action::SearchNext => {
            app.search_next_in_diff();
        }
        Action::SearchPrev => {
            app.search_prev_in_diff();
        }
        _ => {}
    }
}
