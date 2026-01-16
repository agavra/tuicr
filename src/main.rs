mod app;
mod error;
mod handler;
mod input;
mod model;
mod output;
mod persistence;
mod syntax;
mod ui;
mod vcs;

use std::io;
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
    handle_search_action,
};
use input::{Action, map_key_to_action};

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

    // Initialize app
    let mut app = match App::new() {
        Ok(mut app) => {
            app.supports_keyboard_enhancement = keyboard_enhancement_supported;
            app
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            #[cfg(all(feature = "hg", feature = "jj"))]
            eprintln!(
                "\nMake sure you're in a git, jujutsu, or mercurial repository with uncommitted changes."
            );
            #[cfg(all(feature = "hg", not(feature = "jj")))]
            eprintln!(
                "\nMake sure you're in a git or mercurial repository with uncommitted changes."
            );
            #[cfg(all(feature = "jj", not(feature = "hg")))]
            eprintln!(
                "\nMake sure you're in a git or jujutsu repository with uncommitted changes."
            );
            #[cfg(not(any(feature = "hg", feature = "jj")))]
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
        if event::poll(Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
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
                InputMode::Normal => match app.focused_panel {
                    FocusedPanel::FileList => handle_file_list_action(&mut app, action),
                    FocusedPanel::Diff => handle_diff_action(&mut app, action),
                },
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
