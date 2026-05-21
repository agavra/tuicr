mod app;
mod config;
mod error;
mod forge;
mod handler;
mod hash;
mod input;
mod model;
mod output;
mod persistence;
mod process;
mod profile;
mod slug;
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

use crossterm::{
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, KeyEventKind, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
        PushKeyboardEnhancementFlags,
    },
    execute, queue,
    terminal::{
        BeginSynchronizedUpdate, EndSynchronizedUpdate, EnterAlternateScreen, LeaveAlternateScreen,
        disable_raw_mode, enable_raw_mode, supports_keyboard_enhancement,
    },
};
use ratatui::{Terminal, backend::CrosstermBackend};

use app::{App, AppStartupOptions, FocusedPanel, InputMode};
use handler::{
    handle_command_action, handle_comment_action, handle_commit_select_action,
    handle_commit_selector_action, handle_confirm_action, handle_diff_action,
    handle_file_list_action, handle_help_action, handle_mouse_event, handle_search_action,
    handle_submit_action_picker_action, handle_submit_confirm_action,
    handle_submit_resolver_action, handle_visual_action,
};
use input::{Action, map_key_to_action, map_target_filter_mode};
use theme::{parse_cli_args, resolve_theme_with_config};
use vcs::GitBackendPreference;

/// Timeout for the "press Ctrl+C again to exit" feature
const CTRL_C_EXIT_TIMEOUT: Duration = Duration::from_secs(2);
/// Hide the file list by default on narrow terminals.
const MIN_WIDTH_FOR_FILE_LIST: u16 = 100;

fn main() -> anyhow::Result<()> {
    profile::init_from_env();

    // Setup panic hook to restore terminal on panic
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = execute!(io::stdout(), PopKeyboardEnhancementFlags);
        let _ = execute!(io::stdout(), DisableBracketedPaste);
        let _ = execute!(io::stdout(), DisableMouseCapture);
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        original_hook(panic_info);
    }));

    // Parse CLI arguments and resolve theme
    // This also configures syntax highlighting colors before diff parsing
    let mut cli_args = profile::time("startup.parse_cli_args", parse_cli_args);

    // Check keyboard enhancement support before enabling raw mode.
    // Skip when --stdout is used because the probe writes escape sequences to stdout,
    // which would leak into the captured export output.
    let keyboard_enhancement_supported = if cli_args.output_to_stdout {
        false
    } else {
        matches!(supports_keyboard_enhancement(), Ok(true))
    };

    // --file is mutually exclusive with --path, -r, -w, and --all-files
    if cli_args.file_path.is_some() {
        if cli_args.path_filter.is_some() {
            eprintln!("Error: --file cannot be combined with --path");
            std::process::exit(2);
        }
        if cli_args.revisions.is_some() {
            eprintln!("Error: --file cannot be combined with -r/--revisions");
            std::process::exit(2);
        }
        if cli_args.working_tree {
            eprintln!("Error: --file cannot be combined with -w/--working-tree");
            std::process::exit(2);
        }
        if cli_args.all_files {
            eprintln!("Error: --file cannot be combined with -A/--all-files");
            std::process::exit(2);
        }
    }

    // --all-files is mutually exclusive with --path, -r, and -w (--file
    // already covered above)
    if cli_args.all_files {
        if cli_args.path_filter.is_some() {
            eprintln!("Error: --all-files cannot be combined with --path");
            std::process::exit(2);
        }
        if cli_args.revisions.is_some() {
            eprintln!("Error: --all-files cannot be combined with -r/--revisions");
            std::process::exit(2);
        }
        if cli_args.working_tree {
            eprintln!("Error: --all-files cannot be combined with -w/--working-tree");
            std::process::exit(2);
        }
    }

    // --path implies --working-tree unless -r is explicitly provided
    if cli_args.path_filter.is_some() && !cli_args.working_tree && cli_args.revisions.is_none() {
        cli_args.working_tree = true;
    }
    let mut startup_warnings = Vec::new();
    let config_outcome = profile::time("startup.load_config", || match config::load_config() {
        Ok(outcome) => outcome,
        Err(e) => {
            startup_warnings.push(format!("Failed to load config: {e}"));
            config::ConfigLoadOutcome::default()
        }
    });
    startup_warnings.extend(config_outcome.warnings);
    let (mut theme, theme_warnings) = profile::time("startup.resolve_theme", || {
        resolve_theme_with_config(
            cli_args.theme,
            cli_args.appearance,
            config_outcome
                .config
                .as_ref()
                .and_then(|cfg| cfg.theme.as_deref()),
            config_outcome
                .config
                .as_ref()
                .and_then(|cfg| cfg.theme_dark.as_deref()),
            config_outcome
                .config
                .as_ref()
                .and_then(|cfg| cfg.theme_light.as_deref()),
            config_outcome
                .config
                .as_ref()
                .and_then(|cfg| cfg.appearance.as_deref()),
        )
    });
    startup_warnings.extend(theme_warnings);

    let transparent = config_outcome
        .config
        .as_ref()
        .and_then(|cfg| cfg.transparent_background)
        .unwrap_or(true);
    if transparent {
        theme.panel_bg = ratatui::style::Color::Reset;
    }

    let no_update_check = cli_args.no_update_check
        || config_outcome
            .config
            .as_ref()
            .and_then(|cfg| cfg.no_update_check)
            .unwrap_or(false);

    // Start update check in background (non-blocking)
    let update_rx = if !no_update_check {
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
    let git_backend_preference = GitBackendPreference::from_config(
        config_outcome
            .config
            .as_ref()
            .and_then(|cfg| cfg.backend.as_deref()),
    );

    let mut app = match profile::time("startup.app_init", || {
        App::new(
            theme,
            config_outcome
                .config
                .as_ref()
                .and_then(|cfg| cfg.comment_types.clone()),
            cli_args.output_to_stdout,
            AppStartupOptions {
                revisions: cli_args.revisions.as_deref(),
                working_tree: cli_args.working_tree,
                path_filter: cli_args.path_filter.as_deref(),
                file_path: cli_args.file_path.as_deref(),
                all_files: cli_args.all_files,
                git_backend_preference,
                pr_target: cli_args.pr_target.as_deref(),
                repo_url_override: cli_args
                    .repo_url
                    .as_deref()
                    .and_then(crate::forge::github::gh::parse_github_remote_url),
            },
        )
    }) {
        Ok(mut app) => {
            app.supports_keyboard_enhancement = keyboard_enhancement_supported;
            startup_warnings.extend(app.vcs.startup_warnings());
            if let Some(cfg) = config_outcome.config.as_ref() {
                if let Some(forge_cfg) = cfg.forge.clone() {
                    app.forge_config = forge_cfg;
                }
                if let Some(leader) = cfg.leader {
                    app.leader_key = leader;
                }
            }
            app
        }
        Err(e) => {
            eprintln!("Error: {e}");
            // The "you need to be in a git repo" hint is only meaningful
            // when the failure was the absence of a repo. For other
            // startup errors — `tuicr pr <bad-url>`, forge auth issues,
            // missing PR, `--file <missing-path>` — the hint is wrong.
            if matches!(e, crate::error::TuicrError::NotARepository) {
                if cli_args.all_files {
                    eprintln!(
                        "\ntuicr --all-files requires a git repository with tracked files. Run `git init && git add -A` to bootstrap one."
                    );
                } else {
                    eprintln!(
                        "\nMake sure you're in a git, jujutsu, or mercurial repository with commits or staged/unstaged changes."
                    );
                }
            }
            std::process::exit(1);
        }
    };

    // Announce the slug for the active session so agents and wrapper scripts
    // can discover it without parsing the markdown export. This is emitted to
    // stderr before the alt-screen swap so the line stays on the user's
    // scrollback after tuicr exits.
    if let Some(slug) = app.session_slug() {
        eprintln!("tuicr-session: {slug}");
    }

    // Setup terminal
    // When --stdout is used, render TUI to /dev/tty so stdout is free for export output
    enable_raw_mode()?;
    let mut tty_output: Box<dyn Write> = if cli_args.output_to_stdout {
        Box::new(File::options().write(true).open("/dev/tty")?)
    } else {
        Box::new(io::stdout())
    };
    execute!(tty_output, EnterAlternateScreen)?;
    let mouse_enabled = config_outcome
        .config
        .as_ref()
        .and_then(|cfg| cfg.mouse)
        .unwrap_or(true);
    if mouse_enabled {
        execute!(tty_output, EnableMouseCapture)?;
    }

    // Bracketed paste so multi-line / control-char pastes arrive as a single
    // Event::Paste, instead of each character driving Normal-mode actions
    // (Enter submitting, ':' opening command mode, etc.) mid-paste.
    execute!(tty_output, EnableBracketedPaste)?;

    // Enable keyboard enhancement for better modifier key detection (e.g., Alt+Enter)
    // This is supported by modern terminals like Kitty, iTerm2, WezTerm, etc.
    if keyboard_enhancement_supported {
        // REPORT_EVENT_TYPES distinguishes Press from Repeat from Release so
        // the two-press file walk (j/k at file boundary) can require an
        // actual key release between the two presses. Without it terminals
        // emit Press for every auto-repeat tick and held-j would walk past
        // file boundaries.
        let _ = execute!(
            tty_output,
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_EVENT_TYPES,
            )
        );
    }
    let backend = CrosstermBackend::new(tty_output);
    let mut terminal = Terminal::new(backend)?;

    // Apply config-driven defaults
    if let Some(ref cfg) = config_outcome.config {
        if cfg.show_file_list == Some(false) {
            app.show_file_list = false;
            app.focused_panel = FocusedPanel::Diff;
        }
        // Pristine mode has no diff, so side-by-side would render two
        // identical panes. Honor the config for every other mode.
        if cfg.diff_view.as_deref() == Some("side-by-side") && !app.is_pristine_mode {
            app.diff_view_mode = app::DiffViewMode::SideBySide;
        }
        if let Some(wrap) = cfg.wrap {
            app.diff_state.wrap_lines = wrap;
        }
        // Open in single-file view when the user opts in. Pristine
        // `--all-files` already turned it on inside `App::new`, so we
        // only toggle if it's still off.
        if cfg.single_file_view == Some(true) && !app.is_single_file_view {
            app.toggle_single_file_view();
        }
        if cfg.export_legend == Some(false) {
            app.export_legend = false;
        }
        if cfg.cursor_line == Some(false) {
            app.cursor_line_highlight = false;
        }
        if let Some(scroll_offset) = cfg.scroll_offset {
            app.scroll_offset = scroll_offset;
        }
    }

    // On narrow terminals, start with only the diff panel visible.
    if let Ok((width, _)) = crossterm::terminal::size()
        && width < MIN_WIDTH_FOR_FILE_LIST
    {
        app.show_file_list = false;
        app.focused_panel = FocusedPanel::Diff;
    }

    if let Some(message) = startup_warnings.first() {
        app.set_warning(message.clone());
    }

    // Track pending z command for zz centering
    let mut pending_z = false;
    // Track pending Z command for ZZ export+quit / ZQ quit
    let mut pending_shift_z = false;
    // Track pending d command for dd delete
    let mut pending_d = false;
    // Track pending leader command for leader-prefixed actions.
    let mut pending_leader = false;
    // Track pending Ctrl+C for "press twice to exit" (with timestamp for 2s timeout)
    let mut pending_ctrl_c: Option<Instant> = None;

    // Main loop
    loop {
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

        app.clear_expired_message();
        app.poll_pr_load_events();
        app.poll_pr_open_events();
        app.poll_pr_reload_events();
        app.poll_pr_range_reload_events();
        app.poll_pr_threads_events();
        app.poll_pr_submit_events();

        // Render. Bracket the frame in a synchronized-output pair
        // (CSI ?2026h/l) so terminals (and zellij) buffer the whole repaint
        // and present it atomically. Without this, scrolling over a slow link
        // visibly tears as escape sequences arrive in chunks. Terminals that
        // do not support DEC 2026 ignore it.
        queue!(terminal.backend_mut(), BeginSynchronizedUpdate)?;
        terminal.draw(|frame| {
            ui::render(frame, &mut app);
        })?;
        execute!(terminal.backend_mut(), EndSynchronizedUpdate)?;

        // Handle events
        if event::poll(Duration::from_millis(100))? {
            let event = event::read()?;
            // Down/Up Release flips the `*_released_since_arm` flag so the
            // primed two-press file walk in single-file view requires a
            // deliberate release + press; held-key auto-repeat (Repeat
            // events) never satisfies the gate. Terminals without kitty
            // REPORT_EVENT_TYPES support never emit Release, in which case
            // `supports_keyboard_enhancement` is false and `cursor_down` /
            // `cursor_up` skip the gate entirely.
            if let Event::Key(key) = &event
                && key.kind == KeyEventKind::Release
            {
                if matches!(
                    key.code,
                    crossterm::event::KeyCode::Down | crossterm::event::KeyCode::Char('j')
                ) {
                    app.down_released_since_arm = true;
                }
                if matches!(
                    key.code,
                    crossterm::event::KeyCode::Up | crossterm::event::KeyCode::Char('k')
                ) {
                    app.up_released_since_arm = true;
                }
            }
            match event {
                Event::Key(key)
                    if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) =>
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

                    // Handle pending z command for zz/zt/zb viewport positioning
                    if pending_z {
                        pending_z = false;
                        match key.code {
                            crossterm::event::KeyCode::Char('z') => {
                                app.center_cursor();
                                continue;
                            }
                            crossterm::event::KeyCode::Char('t') => {
                                app.cursor_to_top();
                                continue;
                            }
                            crossterm::event::KeyCode::Char('b') => {
                                app.cursor_to_bottom();
                                continue;
                            }
                            _ => {} // Fall through to normal handling
                        }
                    }

                    // Handle pending Z command for ZZ (export+quit) / ZQ (quit)
                    if pending_shift_z {
                        pending_shift_z = false;
                        match key.code {
                            crossterm::event::KeyCode::Char('Z') => {
                                // ZZ: save session, export, and quit (same as :wq)
                                let _ = persistence::save_session(&app.session);
                                app.dirty = false;
                                if app.session.has_comments() {
                                    handler::handle_export_and_quit(&mut app);
                                } else {
                                    app.should_quit = true;
                                }
                                continue;
                            }
                            crossterm::event::KeyCode::Char('Q') => {
                                // ZQ: quit without exporting (same as q)
                                app.should_quit = true;
                                continue;
                            }
                            _ => {} // Fall through to normal handling
                        }
                    }

                    // Handle pending d command for dd delete comment
                    if pending_d {
                        pending_d = false;
                        if key.code == crossterm::event::KeyCode::Char('d') {
                            if app.cursor_on_locked_comment() {
                                app.set_message(
                                    "Comment already pushed to GitHub — read only in tuicr",
                                );
                            } else if !app.delete_comment_at_cursor() {
                                if app.cursor_on_remote_thread() {
                                    app.set_message("GitHub comment — read only in tuicr");
                                } else {
                                    app.set_message("No comment at cursor");
                                }
                            }
                            continue;
                        }
                        // Otherwise fall through to normal handling
                    }

                    // Handle pending leader command for panel focus, file list toggle, and review comments.
                    if pending_leader {
                        pending_leader = false;
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
                            crossterm::event::KeyCode::Char('c') => {
                                app.enter_review_comment_mode();
                                continue;
                            }
                            crossterm::event::KeyCode::Char('f') => {
                                app.toggle_single_file_view();
                                continue;
                            }
                            _ => {}
                        }
                        // Otherwise fall through to normal handling
                    }

                    // Editing the PR-tab filter is a sub-state of CommitSelect;
                    // route through the filter-specific key map so typed
                    // characters update the filter buffer rather than driving
                    // commit-list navigation.
                    let mut action =
                        if app.input_mode == InputMode::CommitSelect && app.pr_filter_editing() {
                            map_target_filter_mode(key)
                        } else {
                            map_key_to_action(key, app.input_mode, app.leader_key)
                        };

                    // Handle pending command setters (these work in any mode)
                    match action {
                        Action::PendingZCommand => {
                            pending_z = true;
                            app.pending_count = None;
                            continue;
                        }
                        Action::PendingShiftZCommand => {
                            pending_shift_z = true;
                            app.pending_count = None;
                            continue;
                        }
                        Action::PendingDCommand => {
                            pending_d = true;
                            app.pending_count = None;
                            continue;
                        }
                        Action::PendingLeaderCommand => {
                            pending_leader = true;
                            app.pending_count = None;
                            continue;
                        }
                        _ => {}
                    }

                    // Vim-style {count}{motion} (Normal mode only): digits accumulate
                    // into `pending_count`, then a following motion either scales its
                    // inner count parameter or is dispatched repeatedly. `{count}G`
                    // jumps to source line `count` (already existing behaviour).
                    if app.input_mode == InputMode::Normal {
                        match action {
                            Action::Digit(d) => {
                                let n = app.pending_count.unwrap_or(0);
                                app.pending_count = Some(
                                    (n.saturating_mul(10).saturating_add(d as usize)).min(999_999),
                                );
                                continue;
                            }
                            Action::GoToBottom if app.pending_count.is_some() => {
                                let count = app.pending_count.unwrap().max(1);
                                app.pending_count = None;
                                app.go_to_source_line(count as u32, crate::model::LineSide::New);
                                continue;
                            }
                            _ => {
                                if let Some(count) = app.pending_count.take() {
                                    let count = count.max(1);
                                    match &mut action {
                                        Action::CursorDown(n)
                                        | Action::CursorUp(n)
                                        | Action::ScrollLeft(n)
                                        | Action::ScrollRight(n)
                                        | Action::ScrollViewDown(n)
                                        | Action::ScrollViewUp(n) => {
                                            *n = n.saturating_mul(count);
                                        }
                                        Action::NextFile
                                        | Action::PrevFile
                                        | Action::NextHunk
                                        | Action::PrevHunk => {
                                            // Dispatch `count - 1` extra times; the
                                            // last one runs through normal dispatch
                                            // below.
                                            for _ in 1..count {
                                                dispatch_action(&mut app, action.clone());
                                            }
                                        }
                                        _ => {
                                            // Count silently discarded for non-motion
                                            // actions (mode changes, edits, etc.).
                                        }
                                    }
                                }
                            }
                        }
                    }

                    dispatch_action(&mut app, action);
                }
                Event::Mouse(mouse_event) => handle_mouse_event(&mut app, mouse_event),
                Event::Paste(text) => {
                    // Bracketed-paste payload — route to whichever handler is
                    // currently accepting text input. Other modes ignore.
                    let action = Action::Paste(text);
                    match app.input_mode {
                        InputMode::Comment => handle_comment_action(&mut app, action),
                        InputMode::Command => handle_command_action(&mut app, action),
                        InputMode::Search => handle_search_action(&mut app, action),
                        InputMode::CommitSelect if app.pr_filter_editing() => {
                            handle_commit_select_action(&mut app, action)
                        }
                        _ => {}
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
    let _ = execute!(terminal.backend_mut(), DisableBracketedPaste);
    if mouse_enabled {
        let _ = execute!(terminal.backend_mut(), DisableMouseCapture);
    }
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;

    // Print pending stdout output if --stdout was used
    if let Some(output) = app.pending_stdout_output {
        print!("{output}");
    }

    Ok(())
}

fn dispatch_action(app: &mut App, action: Action) {
    match app.input_mode {
        InputMode::Help => handle_help_action(app, action),
        InputMode::Command => handle_command_action(app, action),
        InputMode::Search => handle_search_action(app, action),
        InputMode::Comment => handle_comment_action(app, action),
        InputMode::Confirm => handle_confirm_action(app, action),
        InputMode::CommitSelect => handle_commit_select_action(app, action),
        InputMode::VisualSelect => handle_visual_action(app, action),
        InputMode::SubmitResolver => handle_submit_resolver_action(app, action),
        InputMode::SubmitConfirm => handle_submit_confirm_action(app, action),
        InputMode::SubmitActionPicker => handle_submit_action_picker_action(app, action),
        InputMode::Normal => match app.focused_panel {
            FocusedPanel::FileList => handle_file_list_action(app, action),
            FocusedPanel::Diff => handle_diff_action(app, action),
            FocusedPanel::CommitSelector => handle_commit_selector_action(app, action),
        },
    }
}
