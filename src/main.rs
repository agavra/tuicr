mod app;
mod error;
mod handler;
mod input;
mod model;
mod output;
mod persistence;
mod syntax;
mod text_edit;
mod theme;
mod ui;
mod update;
mod vcs;

use std::fs::File;
use std::io::{self, Write};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crossterm::{
    event::{
        self, Event, KeyEventKind, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
        PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{
        EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
        supports_keyboard_enhancement,
    },
};
use ratatui::{Terminal, backend::CrosstermBackend};

use app::{App, FocusedPanel, InputMode};
use handler::{
    handle_command_action, handle_comment_action, handle_commit_select_action,
    handle_confirm_action, handle_diff_action, handle_file_list_action, handle_help_action,
    handle_search_action, handle_visual_action,
};
use input::{Action, map_key_to_action};
use theme::{parse_cli_args, resolve_theme};

/// Timeout for the "press Ctrl+C again to exit" feature
const CTRL_C_EXIT_TIMEOUT: Duration = Duration::from_secs(2);

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

    // Parse CLI arguments and resolve theme
    // This also configures syntax highlighting colors before diff parsing
    let cli_args = parse_cli_args();
    let theme = resolve_theme(cli_args.theme);

    // Start update check in background (non-blocking)
    let update_rx = if !cli_args.no_update_check {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = update::check_for_updates();
            let _ = tx.send(result); // Ignore send error if receiver dropped
        });
        Some(rx)
    } else {
        None
    };

    // Initialize app
    let mut app = match App::new(
        theme,
        cli_args.output_to_stdout,
        cli_args.revisions.as_deref(),
    ) {
        Ok(mut app) => {
            app.supports_keyboard_enhancement = keyboard_enhancement_supported;
            app
        }
        Err(e) => {
            eprintln!("Error: {e}");
            eprintln!(
                "\nMake sure you're in a git, jujutsu, or mercurial repository with commits or uncommitted changes."
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
    execute!(tty_output, EnterAlternateScreen)?;

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

        // Check for update result (non-blocking)
        if let Some(ref rx) = update_rx
            && let Ok(
                update::UpdateCheckResult::UpdateAvailable(info)
                | update::UpdateCheckResult::AheadOfRelease(info),
            ) = rx.try_recv()
        {
            app.update_info = Some(info);
        }

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

    // Print pending stdout output if --stdout was used
    if let Some(output) = app.pending_stdout_output {
        print!("{output}");
    }

    Ok(())
}
