mod app;
mod error;
mod handler;
mod input;
mod model;
mod output;
mod persistence;
mod syntax;
mod theme;
mod ui;
mod vcs;

use std::fs::File;
use std::io::{self, Write};
use std::time::{Duration, Instant};

use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind,
        KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{
        EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
        supports_keyboard_enhancement,
    },
};
use ratatui::{Terminal, backend::CrosstermBackend};

use app::{App, FileTreeItem, FocusedPanel, InputMode};
use handler::{
    handle_command_action, handle_comment_action, handle_commit_select_action,
    handle_confirm_action, handle_diff_action, handle_file_list_action, handle_help_action,
    handle_search_action, handle_visual_action,
};
use input::{Action, map_key_to_action};
use output::markdown::export_to_clipboard;
use persistence::storage::save_session;
use theme::{parse_cli_args, resolve_theme};

/// Timeout for the "press Ctrl+C again to exit" feature
const CTRL_C_EXIT_TIMEOUT: Duration = Duration::from_secs(2);

fn main() -> anyhow::Result<()> {
    // Setup panic hook to restore terminal on panic
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = execute!(io::stdout(), PopKeyboardEnhancementFlags);
        let _ = execute!(io::stdout(), DisableMouseCapture);
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        original_hook(panic_info);
    }));

    // Check keyboard enhancement support before enabling raw mode
    let keyboard_enhancement_supported = matches!(supports_keyboard_enhancement(), Ok(true));

    // Parse CLI arguments and resolve theme
    // This also configures syntax highlighting colors before diff parsing
    let cli_args = parse_cli_args();
    let theme = resolve_theme(cli_args.theme);

    // Initialize app
    let mut app = match App::new(theme, cli_args.output_to_stdout) {
        Ok(mut app) => {
            app.supports_keyboard_enhancement = keyboard_enhancement_supported;
            app
        }
        Err(e) => {
            eprintln!("Error: {e}");
            eprintln!(
                "\nMake sure you're in a git, jujutsu, or mercurial repository with uncommitted changes."
            );
            std::process::exit(1);
        }
    };

    // Setup terminal
    // When --stdout is used, render TUI to /dev/tty so stdout is free for export output
    enable_raw_mode()?;
    let mut tty_output: Box<dyn Write> = if cli_args.output_to_stdout {
        Box::new(File::options().write(true).open("/dev/tty")?)
    } else {
        Box::new(io::stdout())
    };
    execute!(tty_output, EnterAlternateScreen, EnableMouseCapture)?;

    // Enable keyboard enhancement for better modifier key detection (e.g., Alt+Enter)
    // This is supported by modern terminals like Kitty, iTerm2, WezTerm, etc.
    if keyboard_enhancement_supported {
        let _ = execute!(
            tty_output,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        );
    }
    let backend = CrosstermBackend::new(tty_output);
    let mut terminal = Terminal::new(backend)?;

    // Track pending z command for zz centering
    let mut pending_z = false;
    // Track pending d command for dd delete
    let mut pending_d = false;
    // Track pending ; command for ;e toggle file list
    let mut pending_semicolon = false;
    // Track pending Ctrl+C for "press twice to exit" (with timestamp for 2s timeout)
    let mut pending_ctrl_c: Option<Instant> = None;

    // Main loop
    loop {
        // Render
        terminal.draw(|frame| {
            ui::render(frame, &mut app);
        })?;

        // Auto-clear expired pending Ctrl+C state and message
        if let Some(first_press) = pending_ctrl_c
            && first_press.elapsed() >= CTRL_C_EXIT_TIMEOUT
        {
            pending_ctrl_c = None;
            app.message = None;
        }

        // Handle events
        if event::poll(Duration::from_millis(100))? {
            let event = event::read()?;
            match event {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    // Handle Ctrl+C twice to exit (works across all input modes)
                    // In Comment mode, first Ctrl+C also cancels the comment
                    if key.code == crossterm::event::KeyCode::Char('c')
                        && key
                            .modifiers
                            .contains(crossterm::event::KeyModifiers::CONTROL)
                    {
                        // If in comment mode, cancel the comment first
                        if app.input_mode == InputMode::Comment {
                            app.exit_comment_mode();
                        }

                        if let Some(first_press) = pending_ctrl_c
                            && first_press.elapsed() < CTRL_C_EXIT_TIMEOUT
                        {
                            // Second Ctrl+C within timeout - exit immediately
                            app.should_quit = true;
                            continue;
                        }
                        // First Ctrl+C (or timeout expired) - show warning and start timer
                        pending_ctrl_c = Some(Instant::now());
                        app.set_message("Press Ctrl+C again to exit");
                        continue;
                    }

                    // Any other key clears the pending Ctrl+C state and message
                    if pending_ctrl_c.is_some() {
                        pending_ctrl_c = None;
                        app.message = None;
                    }

                    // Handle pending z command for zz centering
                    if pending_z {
                        pending_z = false;
                        if key.code == crossterm::event::KeyCode::Char('z') {
                            app.center_cursor();
                            continue;
                        }
                        // Otherwise fall through to normal handling
                    }

                    // Handle pending d command for dd delete comment
                    if pending_d {
                        pending_d = false;
                        if key.code == crossterm::event::KeyCode::Char('d') {
                            if !app.delete_comment_at_cursor() {
                                app.set_message("No comment at cursor");
                            }
                            continue;
                        }
                        // Otherwise fall through to normal handling
                    }

                    // Handle pending ; command for ;e toggle file list, ;h/;l panel focus
                    if pending_semicolon {
                        pending_semicolon = false;
                        match key.code {
                            crossterm::event::KeyCode::Char('e') => {
                                app.toggle_file_list();
                                continue;
                            }
                            crossterm::event::KeyCode::Char('h') => {
                                app.focused_panel = app::FocusedPanel::FileList;
                                continue;
                            }
                            crossterm::event::KeyCode::Char('l') => {
                                app.focused_panel = app::FocusedPanel::Diff;
                                continue;
                            }
                            _ => {}
                        }
                        // Otherwise fall through to normal handling
                    }

                    let action = map_key_to_action(key, app.input_mode);

                    // Handle pending command setters (these work in any mode)
                    match action {
                        Action::PendingZCommand => {
                            pending_z = true;
                            continue;
                        }
                        Action::PendingDCommand => {
                            pending_d = true;
                            continue;
                        }
                        Action::PendingSemicolonCommand => {
                            pending_semicolon = true;
                            continue;
                        }
                        _ => {}
                    }

                    // Dispatch by input mode
                    match app.input_mode {
                        InputMode::Help => handle_help_action(&mut app, action),
                        InputMode::Command => handle_command_action(&mut app, action),
                        InputMode::Search => handle_search_action(&mut app, action),
                        InputMode::Comment => handle_comment_action(&mut app, action),
                        InputMode::Confirm => handle_confirm_action(&mut app, action),
                        InputMode::CommitSelect => handle_commit_select_action(&mut app, action),
                        InputMode::VisualSelect => handle_visual_action(&mut app, action),
                        InputMode::Normal => match app.focused_panel {
                            FocusedPanel::FileList => handle_file_list_action(&mut app, action),
                            FocusedPanel::Diff => handle_diff_action(&mut app, action),
                        },
                    }
                }
                Event::Mouse(mouse_event) => {
                    use crossterm::event::MouseEventKind;

                    // Helper to determine which panel the mouse is over
                    let mouse_col = mouse_event.column;
                    let mouse_row = mouse_event.row;

                    let over_file_list = app
                        .file_list_area
                        .map(|area| {
                            mouse_col >= area.x
                                && mouse_col < area.x + area.width
                                && mouse_row >= area.y
                                && mouse_row < area.y + area.height
                        })
                        .unwrap_or(false);

                    let over_diff = app
                        .diff_area
                        .map(|area| {
                            mouse_col >= area.x
                                && mouse_col < area.x + area.width
                                && mouse_row >= area.y
                                && mouse_row < area.y + area.height
                        })
                        .unwrap_or(false);

                    match mouse_event.kind {
                        MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
                            // Click to focus panel
                            if app.input_mode == InputMode::Normal {
                                if over_file_list {
                                    app.focused_panel = FocusedPanel::FileList;
                                } else if over_diff {
                                    app.focused_panel = FocusedPanel::Diff;
                                }
                            }
                        }
                        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                            let action = match mouse_event.kind {
                                MouseEventKind::ScrollUp => Action::MouseScrollUp(3),
                                MouseEventKind::ScrollDown => Action::MouseScrollDown(3),
                                _ => unreachable!(),
                            };

                            // Dispatch action based on which panel the mouse is over
                            match app.input_mode {
                                InputMode::Help => handle_help_action(&mut app, action),
                                InputMode::Normal => {
                                    if over_file_list {
                                        handle_file_list_action(&mut app, action);
                                    } else if over_diff {
                                        handle_diff_action(&mut app, action);
                                    }
                                }
                                _ => {}
                            }
                        }
                        _ => {}
                    }
                }
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
                        app::FocusedPanel::FileList => app::FocusedPanel::Diff,
                        app::FocusedPanel::Diff => app::FocusedPanel::FileList,
                    };
                }
                Action::SelectFile => {
                    if app.focused_panel == app::FocusedPanel::FileList
                        && let Some(item) = app.get_selected_tree_item()
                    {
                        match item {
                            FileTreeItem::Directory { path, .. } => {
                                app.toggle_directory(&path);
                            }
                            FileTreeItem::File { file_idx, .. } => {
                                app.jump_to_file(file_idx);
                            }
                        }
                    }
                }
                Action::ToggleExpand => {
                    if app.focused_panel == app::FocusedPanel::FileList
                        && let Some(item) = app.get_selected_tree_item()
                    {
                        match item {
                            FileTreeItem::Directory { path, .. } => {
                                app.toggle_directory(&path);
                            }
                            FileTreeItem::File { file_idx, .. } => {
                                app.jump_to_file(file_idx);
                            }
                        }
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
                Action::ExitMode => {
                    if app.input_mode == app::InputMode::Command {
                        app.exit_command_mode();
                    } else if app.input_mode == app::InputMode::Comment {
                        app.exit_comment_mode();
                    }
                }
                Action::AddLineComment => {
                    let line = app.get_line_at_cursor();
                    if line.is_some() {
                        app.enter_comment_mode(false, line);
                    } else {
                        app.set_message("Move cursor to a diff line to add a line comment");
                    }
                }
                Action::AddFileComment => {
                    app.enter_comment_mode(true, None);
                }
                Action::EditComment => {
                    if !app.enter_edit_mode() {
                        app.set_message("No comment at cursor");
                    }
                }
                Action::InsertChar(c) => {
                    if app.input_mode == app::InputMode::Command {
                        app.command_buffer.push(c);
                    } else if app.input_mode == app::InputMode::Comment {
                        app.comment_buffer.insert(app.comment_cursor, c);
                        app.comment_cursor += 1;
                    }
                }
                Action::DeleteChar => {
                    if app.input_mode == app::InputMode::Command {
                        app.command_buffer.pop();
                    } else if app.input_mode == app::InputMode::Comment && app.comment_cursor > 0 {
                        app.comment_cursor -= 1;
                        app.comment_buffer.remove(app.comment_cursor);
                    }
                }
                Action::CycleCommentType => {
                    app.cycle_comment_type();
                }
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
                Action::DeleteWord => {
                    if app.input_mode == app::InputMode::Comment && app.comment_cursor > 0 {
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
                    if app.input_mode == app::InputMode::Comment {
                        app.comment_buffer.clear();
                        app.comment_cursor = 0;
                    }
                }
                Action::SubmitInput => {
                    if app.input_mode == app::InputMode::Command {
                        let cmd = app.command_buffer.trim().to_string();
                        match cmd.as_str() {
                            "q" | "quit" => app.should_quit = true,
                            "w" | "write" => match save_session(&app.session) {
                                Ok(path) => {
                                    app.dirty = false;
                                    app.set_message(format!("Saved to {}", path.display()));
                                }
                                Err(e) => {
                                    app.set_error(format!("Save failed: {}", e));
                                }
                            },
                            "x" | "wq" => match save_session(&app.session) {
                                Ok(_) => {
                                    app.dirty = false;
                                    // Only prompt if there are comments to copy
                                    if app.session.has_comments() {
                                        app.exit_command_mode();
                                        app.enter_confirm_mode(app::ConfirmAction::CopyAndQuit);
                                        continue;
                                    } else {
                                        app.should_quit = true;
                                    }
                                }
                                Err(e) => {
                                    app.set_error(format!("Save failed: {}", e));
                                }
                            },
                            "e" | "reload" => match app.reload_diff_files() {
                                Ok(count) => {
                                    app.set_message(format!("Reloaded {} files", count));
                                }
                                Err(e) => {
                                    app.set_error(format!("Reload failed: {}", e));
                                }
                            },
                            "clip" | "export" => {
                                match export_to_clipboard(&app.session, &app.diff_source) {
                                    Ok(msg) => app.set_message(msg),
                                    Err(e) => app.set_warning(format!("{}", e)),
                                }
                            }
                            _ => {
                                app.set_message(format!("Unknown command: {}", cmd));
                            }
                        }
                        app.exit_command_mode();
                    } else if app.input_mode == app::InputMode::Comment {
                        app.save_comment();
                    }
                }
                Action::ConfirmYes => {
                    if app.input_mode == app::InputMode::Confirm {
                        if let Some(app::ConfirmAction::CopyAndQuit) = app.pending_confirm {
                            match export_to_clipboard(&app.session, &app.diff_source) {
                                Ok(msg) => app.set_message(msg),
                                Err(e) => app.set_warning(format!("{}", e)),
                            }
                        }
                        app.exit_confirm_mode();
                        app.should_quit = true;
                    }
                }
                Action::ConfirmNo => {
                    if app.input_mode == app::InputMode::Confirm {
                        app.exit_confirm_mode();
                        app.should_quit = true;
                    }
                }
                Action::ExportToClipboard => {
                    match export_to_clipboard(&app.session, &app.diff_source) {
                        Ok(msg) => app.set_message(msg),
                        Err(e) => app.set_warning(format!("{}", e)),
                    }
                }
                Action::CommitSelectUp => app.commit_select_up(),
                Action::CommitSelectDown => app.commit_select_down(),
                Action::ToggleCommitSelect => app.toggle_commit_selection(),
                Action::ConfirmCommitSelect => {
                    if let Err(e) = app.confirm_commit_selection() {
                        app.set_error(format!("Failed to load commits: {}", e));
                    }
                }
                _ => {}
            }
        }

        if app.should_quit {
            break;
        }
    }

    // Restore terminal
    let _ = execute!(terminal.backend_mut(), PopKeyboardEnhancementFlags);
    execute!(terminal.backend_mut(), DisableMouseCapture)?;
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;

    // Print pending stdout output if --stdout was used
    if let Some(output) = app.pending_stdout_output {
        print!("{output}");
    }

    Ok(())
}
