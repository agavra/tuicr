mod app;
mod error;
mod git;
mod handler;
mod input;
mod model;
mod output;
mod persistence;
mod syntax;
mod ui;

use std::io;
use std::time::Duration;

use arboard::Clipboard; 

use crossterm::{
    event::{
        self, Event, KeyCode, KeyModifiers, KeyEventKind,
    },
    execute,
    terminal::{
        EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
    },
};
use ratatui::{Terminal, backend::CrosstermBackend};

use app::{App, FocusedPanel, InputMode};
use handler::{
    handle_command_action, handle_comment_action, handle_commit_select_action,
    handle_confirm_action, handle_diff_action, handle_file_list_action, handle_help_action,
};
use input::{Action, map_key_to_action};

fn main() -> anyhow::Result<()> {
    // Setup panic hook
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        original_hook(panic_info);
    }));

    // Initialize app
    let mut app = match App::new() {
        Ok(mut app) => {
            app.supports_keyboard_enhancement = false;
            app
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    };

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut pending_z = false;
    let mut pending_d = false;
    let mut pending_semicolon = false;

    // Main loop
    loop {
        terminal.draw(|frame| {
            ui::render(frame, &mut app);
        })?;

        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) => {
                    // Fix double input on Windows
                    if key.kind != KeyEventKind::Press {
                        continue;
                    }

                    // --- PASTE LOGIC (CTRL+P) ---
                    if app.input_mode == InputMode::Comment { 
                        if key.code == KeyCode::Char('p') && key.modifiers.contains(KeyModifiers::CONTROL) {
                            if let Ok(mut clipboard) = Clipboard::new() {
                                if let Ok(text) = clipboard.get_text() {
                                    let clean_text = text.replace("\r\n", " ").replace("\n", " ");
                                    app.comment_buffer.push_str(&clean_text); 
                                }
                            }
                            continue;
                        }
                    }

                    // Handle pending z
                    if pending_z {
                        pending_z = false;
                        if key.code == KeyCode::Char('z') {
                            app.center_cursor();
                            continue;
                        }
                    }

                    // Handle pending d
                    if pending_d {
                        pending_d = false;
                        if key.code == KeyCode::Char('d') {
                            if !app.delete_comment_at_cursor() {
                                app.set_message("No comment at cursor");
                            }
                            continue;
                        }
                    }

                    // Handle pending ;
                    if pending_semicolon {
                        pending_semicolon = false;
                        match key.code {
                            KeyCode::Char('e') => {
                                app.toggle_file_list();
                                continue;
                            }
                            KeyCode::Char('h') => {
                                app.focused_panel = app::FocusedPanel::FileList;
                                continue;
                            }
                            KeyCode::Char('l') => {
                                app.focused_panel = app::FocusedPanel::Diff;
                                continue;
                            }
                            _ => {}
                        }
                    }

                    let action = map_key_to_action(key, app.input_mode);

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

                    match app.input_mode {
                        InputMode::Help => handle_help_action(&mut app, action),
                        InputMode::Command => handle_command_action(&mut app, action),
                        InputMode::Comment => handle_comment_action(&mut app, action),
                        InputMode::Confirm => handle_confirm_action(&mut app, action),
                        InputMode::CommitSelect => handle_commit_select_action(&mut app, action),
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

    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);

    Ok(())
}