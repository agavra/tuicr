mod app;
mod error;
mod git;
mod input;
mod model;
mod output;
mod persistence;
mod ui;

use std::io;
use std::time::Duration;

use crossterm::{
    event::{
        self, Event, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
        PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{
        EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
        supports_keyboard_enhancement,
    },
};
use ratatui::{Terminal, backend::CrosstermBackend};

use app::App;
use input::{Action, map_key_to_action};
use output::export_to_clipboard;
use persistence::save_session;

fn main() -> anyhow::Result<()> {
    // Setup panic hook to restore terminal on panic
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = execute!(io::stdout(), PopKeyboardEnhancementFlags);
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        original_hook(panic_info);
    }));

    // Check keyboard enhancement support before enabling raw mode
    let keyboard_enhancement_supported = matches!(supports_keyboard_enhancement(), Ok(true));

    // Initialize app
    let mut app = match App::new() {
        Ok(mut app) => {
            app.supports_keyboard_enhancement = keyboard_enhancement_supported;
            app
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            eprintln!("\nMake sure you're in a git repository with uncommitted changes.");
            std::process::exit(1);
        }
    };

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;

    // Enable keyboard enhancement for better modifier key detection (e.g., Alt+Enter)
    // This is supported by modern terminals like Kitty, iTerm2, WezTerm, etc.
    if keyboard_enhancement_supported {
        let _ = execute!(
            stdout,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        );
    }
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Track pending z command for zz centering
    let mut pending_z = false;
    // Track pending d command for dd delete
    let mut pending_d = false;

    // Main loop
    loop {
        // Render
        terminal.draw(|frame| {
            ui::render(frame, &mut app);
        })?;

        // Handle events
        if event::poll(Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
        {
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

            let action = map_key_to_action(key, app.input_mode);

            match action {
                Action::Quit => {
                    app.should_quit = true;
                }
                Action::CursorDown(n) => match app.focused_panel {
                    app::FocusedPanel::FileList => app.file_list_down(n),
                    app::FocusedPanel::Diff => app.cursor_down(n),
                },
                Action::CursorUp(n) => match app.focused_panel {
                    app::FocusedPanel::FileList => app.file_list_up(n),
                    app::FocusedPanel::Diff => app.cursor_up(n),
                },
                Action::HalfPageDown => app.scroll_down(15),
                Action::HalfPageUp => app.scroll_up(15),
                Action::PageDown => app.scroll_down(30),
                Action::PageUp => app.scroll_up(30),
                Action::ScrollLeft(n) => app.scroll_left(n),
                Action::ScrollRight(n) => app.scroll_right(n),
                Action::PendingZCommand => {
                    pending_z = true;
                }
                Action::PendingDCommand => {
                    pending_d = true;
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
                    if app.focused_panel == app::FocusedPanel::FileList {
                        app.jump_to_file(app.file_list_state.selected);
                    }
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
                            "clip" | "export" => match export_to_clipboard(&app.session) {
                                Ok(msg) => app.set_message(msg),
                                Err(e) => app.set_warning(format!("{}", e)),
                            },
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
                            match export_to_clipboard(&app.session) {
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
                Action::ExportToClipboard => match export_to_clipboard(&app.session) {
                    Ok(msg) => app.set_message(msg),
                    Err(e) => app.set_warning(format!("{}", e)),
                },
                _ => {}
            }
        }

        if app.should_quit {
            break;
        }
    }

    // Restore terminal
    let _ = execute!(terminal.backend_mut(), PopKeyboardEnhancementFlags);
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;

    Ok(())
}
