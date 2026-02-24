mod app;
mod config;
mod error;
mod handler;
#[cfg(feature = "ide-integration")]
mod ide;
mod input;
mod model;
mod output;
mod persistence;
mod syntax;
mod text_edit;
mod theme;
mod tuicrignore;
mod ui;
mod update;
mod vcs;

use std::fs::File;
use std::io::{self, Write};
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[cfg(feature = "ide-integration")]
use std::sync::Arc;

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
    handle_commit_selector_action, handle_confirm_action, handle_diff_action,
    handle_file_list_action, handle_help_action, handle_search_action, handle_visual_action,
};
use input::{Action, map_key_to_action};
use theme::{parse_cli_args, resolve_theme_with_config};

/// Timeout for the "press Ctrl+C again to exit" feature
const CTRL_C_EXIT_TIMEOUT: Duration = Duration::from_secs(2);
/// Hide the file list by default on narrow terminals.
const MIN_WIDTH_FOR_FILE_LIST: u16 = 100;

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
    let mut startup_warnings = Vec::new();
    let config_outcome = match config::load_config() {
        Ok(outcome) => outcome,
        Err(e) => {
            startup_warnings.push(format!("Failed to load config: {e}"));
            config::ConfigLoadOutcome::default()
        }
    };
    startup_warnings.extend(config_outcome.warnings);
    let (theme, theme_warnings) = resolve_theme_with_config(
        cli_args.theme,
        config_outcome
            .config
            .as_ref()
            .and_then(|cfg| cfg.theme.as_deref()),
    );
    startup_warnings.extend(theme_warnings);

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
            if let Some(message) = startup_warnings.first() {
                app.set_warning(message.clone());
            }
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

    // IDE integration setup
    #[cfg(feature = "ide-integration")]
    let ide_command_rx: Option<std::sync::mpsc::Receiver<ide::IdeCommand>> =
        if cli_args.ide_integration {
            let workspace_path = app.vcs_info.root_path.to_string_lossy().to_string();
            let ide_state = ide::new_shared_state();
            let ide_state_clone = ide_state.clone();

            // Create a channel for IDE commands (sync receiver for main loop)
            let (cmd_tx, cmd_rx) = std::sync::mpsc::channel();

            // Spawn tokio runtime in background thread
            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
                rt.block_on(async move {
                    // Create tokio channel for internal async communication
                    let (async_cmd_tx, mut async_cmd_rx) = tokio::sync::mpsc::channel(32);

                    // Start the IDE server
                    let mut server = ide::IdeServer::new(ide_state_clone, async_cmd_tx);
                    match server.start(&workspace_path).await {
                        Ok(port) => {
                            eprintln!("IDE integration server started on port {port}");
                        }
                        Err(e) => {
                            eprintln!("Failed to start IDE server: {e}");
                            return;
                        }
                    }

                    // Forward async commands to sync channel
                    while let Some(cmd) = async_cmd_rx.recv().await {
                        if cmd_tx.send(cmd).is_err() {
                            // Main loop has exited
                            break;
                        }
                    }
                });
            });

            // Sync initial state to IDE
            sync_app_to_ide_state(&app, &ide_state);

            // Store the IDE state for later syncing
            app.ide_state = Some(ide_state);

            Some(cmd_rx)
        } else {
            None
        };

    #[cfg(not(feature = "ide-integration"))]
    let _ide_command_rx: Option<std::sync::mpsc::Receiver<()>> = None;

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

    // On narrow terminals, start with only the diff panel visible.
    if let Ok((width, _)) = crossterm::terminal::size()
        && width < MIN_WIDTH_FOR_FILE_LIST
    {
        app.show_file_list = false;
        app.focused_panel = FocusedPanel::Diff;
    }

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

        // Handle IDE commands (non-blocking, limited to avoid blocking the event loop)
        #[cfg(feature = "ide-integration")]
        if let Some(ref rx) = ide_command_rx {
            const MAX_IDE_COMMANDS_PER_FRAME: usize = 10;
            for _ in 0..MAX_IDE_COMMANDS_PER_FRAME {
                match rx.try_recv() {
                    Ok(cmd) => handle_ide_command(&mut app, cmd),
                    Err(_) => break,
                }
            }
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

                    // Handle pending ; command for ;e toggle file list, ;h/;l/;k/;j panel focus
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
                            crossterm::event::KeyCode::Char('k') => {
                                if app.has_inline_commit_selector() {
                                    app.focused_panel = app::FocusedPanel::CommitSelector;
                                }
                                continue;
                            }
                            crossterm::event::KeyCode::Char('j') => {
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
                            FocusedPanel::CommitSelector => {
                                handle_commit_selector_action(&mut app, action)
                            }
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

/// Sync app state to IDE state for tool queries.
///
/// Uses `try_write()` to avoid blocking the main event loop. If the lock
/// cannot be acquired (e.g., a tool query is in progress), sync is skipped
/// for this call and will be retried on the next state change. This is
/// acceptable because IDE tools query the state on-demand and brief staleness
/// during active queries has no user-visible impact.
#[cfg(feature = "ide-integration")]
fn sync_app_to_ide_state(app: &App, ide_state: &ide::SharedIdeState) {
    use ide::{DiagnosticInfo, OpenFileInfo, Selection};

    // Use try_write to avoid blocking - if lock is held, skip this sync
    // (state will be refreshed on next call)
    let Ok(mut state) = ide_state.try_write() else {
        return;
    };

    // Set workspace info
    state.set_workspace(
        app.vcs_info.root_path.to_string_lossy().to_string(),
        app.vcs_info
            .root_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string()),
    );

    // Set open files from diff_files
    let open_files: Vec<OpenFileInfo> = app
        .diff_files
        .iter()
        .enumerate()
        .map(|(idx, file)| {
            let path = file
                .new_path
                .as_ref()
                .or(file.old_path.as_ref())
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();

            let extension = std::path::Path::new(&path)
                .extension()
                .map(|e| e.to_string_lossy().to_string())
                .unwrap_or_default();

            let language_id = match extension.as_str() {
                "rs" => "rust",
                "py" => "python",
                "js" => "javascript",
                "ts" => "typescript",
                "tsx" => "typescriptreact",
                "jsx" => "javascriptreact",
                "go" => "go",
                "java" => "java",
                "c" | "h" => "c",
                "cpp" | "hpp" | "cc" | "cxx" => "cpp",
                "rb" => "ruby",
                "php" => "php",
                "swift" => "swift",
                "kt" | "kts" => "kotlin",
                "scala" => "scala",
                "lua" => "lua",
                "sh" | "bash" => "shellscript",
                "json" => "json",
                "yaml" | "yml" => "yaml",
                "toml" => "toml",
                "xml" => "xml",
                "html" => "html",
                "css" => "css",
                "scss" => "scss",
                "md" => "markdown",
                _ => "plaintext",
            }
            .to_string();

            let reviewed = app.session.is_file_reviewed(
                &file
                    .new_path
                    .clone()
                    .or_else(|| file.old_path.clone())
                    .unwrap_or_default(),
            );

            OpenFileInfo {
                file_path: path,
                language_id,
                is_dirty: !reviewed,
                is_active: idx == app.diff_state.current_file_idx,
                status: format!("{:?}", file.status),
                reviewed,
            }
        })
        .collect();
    state.set_open_files(open_files);
    state.set_active_file(app.diff_state.current_file_idx);

    // Set diagnostics from comments
    let mut diagnostics = Vec::new();
    for (path, file_review) in &app.session.files {
        let path_str = path.to_string_lossy().to_string();

        // File-level comments
        for comment in &file_review.file_comments {
            diagnostics.push(DiagnosticInfo {
                file_path: path_str.clone(),
                start_line: 1,
                end_line: 1,
                message: comment.content.clone(),
                severity: comment_type_to_severity(&comment.comment_type),
                comment_type: format!("{:?}", comment.comment_type),
            });
        }

        // Line comments
        for (line, comments) in &file_review.line_comments {
            for comment in comments {
                let (start, end) = match &comment.line_range {
                    Some(range) => (range.start, range.end),
                    None => (*line, *line),
                };
                diagnostics.push(DiagnosticInfo {
                    file_path: path_str.clone(),
                    start_line: start,
                    end_line: end,
                    message: comment.content.clone(),
                    severity: comment_type_to_severity(&comment.comment_type),
                    comment_type: format!("{:?}", comment.comment_type),
                });
            }
        }
    }
    state.set_diagnostics(diagnostics);

    // Set selection if in visual mode
    if app.input_mode == InputMode::VisualSelect {
        if let Some((anchor_line, _anchor_side)) = app.visual_anchor {
            // Get current cursor position
            if let Some(current_file) = app.diff_files.get(app.diff_state.current_file_idx) {
                let path = current_file
                    .new_path
                    .as_ref()
                    .or(current_file.old_path.as_ref())
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default();

                // For now, just use the anchor line as both start and end
                // A more complete implementation would track the current cursor line
                state.set_selection(Some(Selection {
                    file_path: path,
                    text: String::new(), // Would need to extract actual text
                    start_line: anchor_line,
                    end_line: anchor_line,
                }));
            }
        }
    } else {
        state.set_selection(None);
    }
}

#[cfg(feature = "ide-integration")]
fn comment_type_to_severity(comment_type: &model::CommentType) -> String {
    match comment_type {
        model::CommentType::Issue => "error".to_string(),
        model::CommentType::Suggestion => "warning".to_string(),
        model::CommentType::Note => "information".to_string(),
        model::CommentType::Praise => "hint".to_string(),
    }
}

/// Handle an IDE command from the MCP server.
#[cfg(feature = "ide-integration")]
fn handle_ide_command(app: &mut App, cmd: ide::IdeCommand) {
    match cmd {
        ide::IdeCommand::OpenFile { path, line } => {
            // Find the file index
            let file_idx = app.diff_files.iter().position(|f| {
                f.new_path
                    .as_ref()
                    .or(f.old_path.as_ref())
                    .map(|p| p.to_string_lossy().contains(&path))
                    .unwrap_or(false)
            });

            if let Some(idx) = file_idx {
                // Select the file in the file list
                app.file_list_state.select(idx);
                app.diff_state.current_file_idx = idx;

                // Jump to line if specified
                if let Some(target_line) = line {
                    // Find the annotation line that corresponds to this source line
                    for (anno_idx, anno) in app.line_annotations.iter().enumerate() {
                        match anno {
                            app::AnnotatedLine::DiffLine {
                                file_idx: f_idx,
                                new_lineno,
                                old_lineno,
                                ..
                            } if *f_idx == idx => {
                                if new_lineno == &Some(target_line)
                                    || old_lineno == &Some(target_line)
                                {
                                    app.diff_state.cursor_line = anno_idx;
                                    // Adjust scroll to show cursor
                                    if anno_idx < app.diff_state.scroll_offset {
                                        app.diff_state.scroll_offset = anno_idx;
                                    } else if anno_idx
                                        >= app.diff_state.scroll_offset
                                            + app.diff_state.viewport_height.saturating_sub(1)
                                    {
                                        app.diff_state.scroll_offset = anno_idx
                                            .saturating_sub(app.diff_state.viewport_height / 2);
                                    }
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }
                }

                // Sync state back to IDE
                if let Some(ref ide_state) = app.ide_state {
                    sync_app_to_ide_state(app, ide_state);
                }
            }
        }
    }
}
