use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// How long after launch a windowed editor's exit still counts as a launch
/// failure worth reporting.
///
/// A launcher that cannot reach its application dies within a moment. An
/// editor that exits later was in the user's hands, and its exit code is not
/// something tuicr should interrupt the review with.
const LAUNCH_FAILURE_WINDOW: Duration = Duration::from_secs(5);

/// Source location tuicr can hand off to an external editor.
///
/// The path is resolved before this reaches the process launcher so terminal
/// suspend/resume code does not need to know about repository-relative paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorTarget {
    /// Absolute path handed to the editor: the local worktree file, or in PR
    /// review a read-only snapshot of the reviewed revision.
    pub path: PathBuf,
    /// One-based source line to request from editors that support it.
    pub line: Option<u32>,
    /// What the status bar calls the opened file. A snapshot lives at a
    /// temp-dir path nobody wants to read, so the App supplies the repository
    /// path here, plus the revision when it is not the worktree's.
    pub label: String,
}

/// Fully expanded editor invocation.
///
/// `program` and `args` are kept separate to avoid shelling out after parsing
/// `$EDITOR`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorCommand {
    /// Executable name or path from `$EDITOR`, or the fallback editor.
    pub program: String,
    /// Arguments from `$EDITOR` plus the target file and optional line syntax.
    pub args: Vec<OsString>,
}

impl EditorCommand {
    /// Builds an invocation from the configured editor.
    /// Builds an invocation from `$EDITOR` or the config `editor` override.
    ///
    /// An unset, empty, or unparsable value falls back to `vi` so the caller
    /// always gets a concrete command to run.
    pub fn from_env(editor_override: Option<&str>, target: &EditorTarget) -> Self {
        let env_editor = std::env::var("EDITOR").unwrap_or_default();
        Self::from_editor(&resolve_editor(editor_override, &env_editor), target)
    }

    /// Builds an invocation from an editor command string.
    ///
    /// The command is split with shell-like quoting rules,
    /// but it is still executed directly without a shell.
    /// Known editors receive their line-navigation syntax;
    /// unknown editors receive only the path.
    ///
    /// For example,
    /// `vim -f` with line 42 becomes `vim -f +42 /repo/src/main.rs`,
    /// while `code` becomes `code --goto /repo/src/main.rs:42`.
    pub fn from_editor(editor: &str, target: &EditorTarget) -> Self {
        let mut parts = shlex::split(editor)
            .filter(|parts| !parts.is_empty())
            .unwrap_or_else(|| vec!["vi".to_string()]);
        let program = parts.remove(0);
        let mut args: Vec<OsString> = parts.into_iter().map(OsString::from).collect();

        match (editor_family(&program), target.line) {
            (EditorFamily::PlusLine, Some(line)) => {
                args.push(OsString::from(format!("+{line}")));
                args.push(target.path.as_os_str().to_os_string());
            }
            (EditorFamily::GotoLine, Some(line)) => {
                args.push(OsString::from("--goto"));
                args.push(OsString::from(format!("{}:{line}", target.path.display())));
            }
            _ => args.push(target.path.as_os_str().to_os_string()),
        }

        Self { program, args }
    }

    /// The program and arguments actually handed to the OS launcher.
    ///
    /// `$EDITOR` is expanded without a shell, so on Windows the command name
    /// has to be resolved here rather than by `Command`. See
    /// [`resolve_command`].
    fn spawn_spec(&self) -> (OsString, Vec<OsString>) {
        let lookup = host_command_lookup();
        let resolved = resolve_command(&self.program, &lookup);
        launch_command(&resolved, &self.args)
    }

    /// Runs the prepared editor command and waits for it to exit.
    ///
    /// The caller owns terminal suspension and restoration around this process
    /// boundary.
    pub fn run(&self) -> std::io::Result<std::process::ExitStatus> {
        let (program, args) = self.spawn_spec();
        Command::new(program).args(args).status()
    }

    /// Runs the prepared editor command with stdin/stdout/stderr re-attached
    /// to the controlling terminal at `/dev/tty`.
    ///
    /// Needed when tuicr was launched with `--stdout`: its own stdout is a
    /// file or pipe, and a terminal editor spawned via `.status()` would
    /// inherit that non-TTY stdout and refuse to render (e.g. vim's
    /// "Output is not to a terminal" warning). The TUI itself already draws
    /// on `/dev/tty` in that mode, so pointing the editor at the same device
    /// is safe.
    ///
    /// On non-Unix targets `/dev/tty` doesn't exist, so this falls back to
    /// [`Self::run`].
    pub fn run_on_tty(&self) -> std::io::Result<std::process::ExitStatus> {
        #[cfg(unix)]
        {
            use std::fs::OpenOptions;
            let stdin = OpenOptions::new().read(true).open("/dev/tty")?;
            let stdout = OpenOptions::new().write(true).open("/dev/tty")?;
            let stderr = OpenOptions::new().write(true).open("/dev/tty")?;
            let (program, args) = self.spawn_spec();
            Command::new(program)
                .args(args)
                .stdin(Stdio::from(stdin))
                .stdout(Stdio::from(stdout))
                .stderr(Stdio::from(stderr))
                .status()
        }
        #[cfg(not(unix))]
        {
            self.run()
        }
    }

    /// Spawns the prepared editor command without waiting for it to exit.
    ///
    /// Standard streams are detached so a chatty editor cannot write over the
    /// TUI, which stays on screen for the whole handoff. The caller polls the
    /// returned handle so the finished process gets cleaned up.
    pub fn spawn_detached(&self) -> std::io::Result<EditorLaunch> {
        let (program, args) = self.spawn_spec();
        let child = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        Ok(EditorLaunch {
            child,
            started: Instant::now(),
        })
    }

    /// Whether this invocation needs the terminal tuicr is drawn on.
    pub fn surface(&self) -> EditorSurface {
        // An explicit wait flag means the user wants tuicr to block until the
        // file is closed, so the terminal has to be handed over either way.
        let waits = self
            .args
            .iter()
            .any(|arg| arg == "-w" || arg == "--wait" || arg == "--block");
        if !waits && is_windowed_editor(&self.program) {
            EditorSurface::Gui
        } else {
            EditorSurface::Terminal
        }
    }
}

/// A windowed editor that was launched and may still be open.
#[derive(Debug)]
pub struct EditorLaunch {
    child: Child,
    started: Instant,
}

impl EditorLaunch {
    /// Cleans up the editor process if it has exited.
    ///
    /// Blocking editors report a bad exit status through `EditorError::Exit`;
    /// this is the equivalent for editors tuicr does not wait on.
    pub fn poll(&mut self) -> LaunchState {
        match self.child.try_wait() {
            Ok(None) => LaunchState::Running,
            Ok(Some(status))
                if !status.success() && self.started.elapsed() < LAUNCH_FAILURE_WINDOW =>
            {
                LaunchState::FailedToLaunch(status)
            }
            Ok(Some(_)) | Err(_) => LaunchState::Exited,
        }
    }
}

/// Where a launched editor has got to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchState {
    /// Still open.
    Running,
    /// Gone, with nothing worth reporting.
    Exited,
    /// Died soon enough after launch that it never reached the user.
    FailedToLaunch(ExitStatus),
}

/// Where an editor draws itself once launched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorSurface {
    /// Takes over the terminal; tuicr must suspend for the duration.
    Terminal,
    /// Opens its own window; tuicr keeps drawing.
    Gui,
}

/// Whether a program is a known editor that opens its own window.
///
/// Unrecognized editors are assumed to be terminal editors: suspending for a
/// GUI editor costs a flicker, while not suspending for a terminal editor
/// leaves two programs fighting over the same screen.
fn is_windowed_editor(program: &str) -> bool {
    let name = program_stem(program);
    matches!(
        name,
        "code"
            | "code-insiders"
            | "codium"
            | "cursor"
            | "windsurf"
            | "zed"
            | "subl"
            | "sublime_text"
            | "mate"
            | "idea"
            | "webstorm"
            | "goland"
            | "pycharm"
            | "clion"
            | "rustrover"
            | "phpstorm"
            | "rubymine"
    )
}

/// Line-navigation syntax family for a recognized editor executable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditorFamily {
    /// Opens a source line with `$editor +NN $file`.
    PlusLine,
    /// Opens a source line with `$editor --goto $file:NN`.
    GotoLine,
    /// Has no known line syntax; opens with `$editor $file`.
    Plain,
}

fn resolve_editor(editor_override: Option<&str>, env_editor: &str) -> String {
    editor_override
        .filter(|editor| !editor.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| env_editor.to_string())
}

fn editor_family(program: &str) -> EditorFamily {
    let name = program_stem(program);
    match name {
        "vi" | "vim" | "nvim" | "nano" | "emacs" | "emacsclient" | "hx" => EditorFamily::PlusLine,
        "code" | "code-insiders" | "codium" | "cursor" => EditorFamily::GotoLine,
        _ => EditorFamily::Plain,
    }
}

/// Extensions only a command interpreter can execute.
///
/// `CreateProcess` runs `.exe` and `.com` files directly but cannot execute a
/// batch file, so `.cmd` and `.bat` have to be handed to `cmd.exe /C`.
const BATCH_EXTENSIONS: [&str; 2] = ["cmd", "bat"];

/// Every suffix Windows treats as an executable command name.
const EXECUTABLE_EXTENSIONS: [&str; 4] = ["exe", "com", "cmd", "bat"];

/// Extensions Windows falls back to when `PATHEXT` is unset.
const DEFAULT_PATHEXT: &str = ".COM;.EXE;.BAT;.CMD";

/// A command name resolved the way the host would launch it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedProgram {
    /// What the OS launcher is handed: the program as typed, or the file
    /// found on the search path.
    program: OsString,
    /// Whether the file is a batch script, which needs a command interpreter.
    via_shell: bool,
}

impl ResolvedProgram {
    /// The program exactly as the user wrote it.
    fn as_typed(program: &str) -> Self {
        Self {
            program: OsString::from(program),
            via_shell: false,
        }
    }
}

/// How this host turns a bare command name into a program to run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct CommandLookup {
    /// `PATH` entries, in search order.
    search_path: Vec<PathBuf>,
    /// The `PATHEXT` list, lowercased. `None` means the host does not resolve
    /// command names through an extension list at all.
    pathext: Option<Vec<String>>,
}

/// The lookup this host performs for a bare command name.
///
/// `Command` expands no shell, and on Windows it only ever appends `.exe` to a
/// bare name: it never reads `PATHEXT`. So `code` is not found on a machine
/// where VS Code ships `code.cmd`, and the launch fails with "program not
/// found" even though the very same command works in the user's shell. Doing
/// the `PATHEXT` lookup here is what makes those editors launchable.
///
/// Unix resolves a command from `PATH` alone and has no `PATHEXT`, so it
/// reports an empty lookup and [`resolve_command`] hands the program through
/// untouched. `cfg!` rather than `#[cfg]` keeps the Windows branch compiled -
/// and linted - on every platform.
fn host_command_lookup() -> CommandLookup {
    if cfg!(windows) {
        let search_path = std::env::var_os("PATH")
            .map(|path| std::env::split_paths(&path).collect())
            .unwrap_or_default();
        let pathext = std::env::var("PATHEXT").ok();
        CommandLookup {
            search_path,
            pathext: Some(pathext_candidates(pathext.as_deref())),
        }
    } else {
        CommandLookup::default()
    }
}

/// Resolves a bare `program` name against a host lookup.
///
/// Returns the program untouched when it names a path the user spelled out,
/// when the host does not use an extension list, or when nothing on the search
/// path carries a listed extension - in that last case the OS reports its own
/// "program not found" error, exactly as before.
fn resolve_command(program: &str, lookup: &CommandLookup) -> ResolvedProgram {
    if !is_bare_command_name(program) {
        return ResolvedProgram::as_typed(program);
    }
    let Some(found) = find_command_on_path(program, lookup) else {
        return ResolvedProgram::as_typed(program);
    };
    ResolvedProgram {
        program: OsString::from(&found),
        via_shell: is_batch_file(&found),
    }
}

/// The program and arguments to hand the OS launcher.
///
/// A batch launcher is wrapped in `cmd.exe /C` because `CreateProcess` cannot
/// execute one. The editor's own arguments - its flags, and the line
/// navigation syntax chosen for it - follow the script path unchanged, so
/// `EDITOR=code` still opens `code.cmd` straight at the requested line.
fn launch_command(resolved: &ResolvedProgram, args: &[OsString]) -> (OsString, Vec<OsString>) {
    if !resolved.via_shell {
        return (resolved.program.clone(), args.to_vec());
    }
    let mut wrapped = Vec::with_capacity(args.len() + 2);
    wrapped.push(OsString::from("/C"));
    wrapped.push(resolved.program.clone());
    wrapped.extend(args.iter().cloned());
    (OsString::from("cmd.exe"), wrapped)
}

/// Whether `program` is a bare command name rather than a path the user
/// spelled out.
///
/// A path is a file the user named: rewriting it would paper over a typo.
fn is_bare_command_name(program: &str) -> bool {
    !program.is_empty()
        && !program.contains(['/', '\\'])
        && Path::new(program)
            .file_name()
            .is_some_and(|name| name == program)
}

/// The first file on the search path that can serve as `program`.
///
/// Windows ranks a directory's candidates by `PATHEXT` order, so `.EXE` ahead
/// of `.CMD` means `code` resolves to `code.exe` whenever both are installed
/// side by side. A name that already carries an executable suffix is matched
/// as written instead of gaining a second one.
fn find_command_on_path(program: &str, lookup: &CommandLookup) -> Option<PathBuf> {
    for dir in &lookup.search_path {
        if is_executable_file_name(program) {
            let candidate = dir.join(program);
            if candidate.is_file() {
                return Some(candidate);
            }
            continue;
        }
        let Some(extensions) = &lookup.pathext else {
            return None;
        };
        for extension in extensions {
            let candidate = dir.join(format!("{program}{extension}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// The `PATHEXT` entries, lowercased, in the order Windows ranks them.
fn pathext_candidates(pathext: Option<&str>) -> Vec<String> {
    pathext
        .unwrap_or(DEFAULT_PATHEXT)
        .split(';')
        .filter(|extension| !extension.trim().is_empty())
        .map(|extension| extension.trim().to_ascii_lowercase())
        .collect()
}

/// Whether only a command interpreter can execute this file.
fn is_batch_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension_is(BATCH_EXTENSIONS, extension))
}

/// Whether this name already carries a suffix Windows treats as executable.
fn is_executable_file_name(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension_is(EXECUTABLE_EXTENSIONS, extension))
}

fn extension_is(extensions: &[&str], extension: &str) -> bool {
    let extension = extension.to_ascii_lowercase();
    extensions.contains(&extension.as_str())
}

/// The editor's own name, without a directory or a launcher suffix.
///
/// `EDITOR=code` and `EDITOR=C:\...\code.cmd` have to name the same editor.
/// Without stripping the suffix the `.cmd` spelling misses the editor family
/// lookup, so it loses both its line-navigation syntax and its GUI surface,
/// and a working `code.cmd` gets suspended like a terminal editor. Only the
/// suffixes Windows itself treats as executable are stripped, so an editor
/// whose real name contains a dot still matches nothing - as before.
fn program_stem(program: &str) -> &str {
    let name = Path::new(program)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(program);
    let Some((stem, extension)) = name.rsplit_once('.') else {
        return name;
    };
    if stem.is_empty() || !extension_is(EXECUTABLE_EXTENSIONS, extension) {
        return name;
    }
    stem
}

/// Error returned when handing control to the external editor fails.
#[derive(Debug, thiserror::Error)]
pub enum EditorError {
    /// The editor process could not be spawned.
    #[error("Failed to launch editor: {0}")]
    Launch(#[source] std::io::Error),
    /// The editor process exited unsuccessfully.
    #[error("Editor exited with status {}", status_label(.0))]
    Exit(ExitStatus),
}

fn status_label(status: &ExitStatus) -> String {
    status
        .code()
        .map(|code| code.to_string())
        .unwrap_or_else(|| "signal".to_string())
}

/// Runs `command` to completion in the terminal.
///
/// The caller owns terminal restoration before displaying any returned error.
pub fn run_editor(command: &EditorCommand) -> Result<(), EditorError> {
    match command.run() {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(EditorError::Exit(status)),
        Err(err) => Err(EditorError::Launch(err)),
    }
}

/// Runs `command` with stdin/stdout/stderr wired to `/dev/tty` instead of
/// tuicr's inherited stdio. Used when tuicr was launched with `--stdout` so
/// the editor still sees a terminal even though tuicr's own stdout is a file.
pub fn run_editor_on_tty(command: &EditorCommand) -> Result<(), EditorError> {
    match command.run_on_tty() {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(EditorError::Exit(status)),
        Err(err) => Err(EditorError::Launch(err)),
    }
}

/// Hands `command` to a windowed editor without waiting for it to exit.
pub fn launch_editor(command: &EditorCommand) -> Result<EditorLaunch, EditorError> {
    command.spawn_detached().map_err(EditorError::Launch)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(line: Option<u32>) -> EditorTarget {
        EditorTarget {
            path: PathBuf::from("/repo/src/main.rs"),
            line,
            label: "src/main.rs".to_string(),
        }
    }

    fn args(command: &EditorCommand) -> Vec<String> {
        command
            .args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn plus_line_editors_receive_line_before_path() {
        for editor in ["vi", "vim", "nvim", "nano", "hx"] {
            let command = EditorCommand::from_editor(editor, &target(Some(42)));
            assert_eq!(command.program, editor);
            assert_eq!(args(&command), vec!["+42", "/repo/src/main.rs"]);
        }
    }

    #[cfg(unix)]
    #[test]
    fn run_on_tty_runs_true_and_reports_success() {
        // /bin/true exits 0 immediately without reading stdin, so this
        // exercises the /dev/tty wiring path without needing a real editor.
        // CI and other headless runners have no controlling terminal, so
        // /dev/tty returns ENXIO — skip cleanly in that case rather than
        // fail on a machine that literally cannot exercise this code path.
        if std::fs::File::open("/dev/tty").is_err() {
            return;
        }
        let command = EditorCommand {
            program: "true".to_string(),
            args: Vec::new(),
        };
        let status = command
            .run_on_tty()
            .expect("run_on_tty must open /dev/tty and spawn");
        assert!(status.success(), "true should exit 0");
    }

    #[test]
    fn emacs_receives_plus_line_before_path() {
        for editor in ["emacs", "emacsclient"] {
            let command = EditorCommand::from_editor(editor, &target(Some(42)));
            assert_eq!(command.program, editor);
            assert_eq!(args(&command), vec!["+42", "/repo/src/main.rs"]);
        }
    }

    #[test]
    fn emacs_args_are_preserved() {
        let command = EditorCommand::from_editor("emacs -nw", &target(Some(42)));
        assert_eq!(command.program, "emacs");
        assert_eq!(args(&command), vec!["-nw", "+42", "/repo/src/main.rs"]);
    }

    #[test]
    fn vscode_family_receives_goto_arg() {
        for editor in ["code", "code-insiders", "codium", "cursor"] {
            let command = EditorCommand::from_editor(editor, &target(Some(42)));
            assert_eq!(command.program, editor);
            assert_eq!(args(&command), vec!["--goto", "/repo/src/main.rs:42"]);
        }
    }

    #[test]
    fn unknown_editor_opens_file_without_line() {
        let command = EditorCommand::from_editor("zed", &target(Some(42)));
        assert_eq!(command.program, "zed");
        assert_eq!(args(&command), vec!["/repo/src/main.rs"]);
    }

    #[test]
    fn editor_args_are_preserved() {
        let command = EditorCommand::from_editor("vim -f", &target(Some(42)));
        assert_eq!(command.program, "vim");
        assert_eq!(args(&command), vec!["-f", "+42", "/repo/src/main.rs"]);
    }

    #[test]
    fn windowed_editors_do_not_claim_the_terminal() {
        for editor in [
            "code",
            "cursor",
            "zed",
            "subl",
            "/usr/local/bin/code-insiders",
        ] {
            let command = EditorCommand::from_editor(editor, &target(Some(42)));
            assert_eq!(command.surface(), EditorSurface::Gui, "{editor}");
        }
    }

    #[test]
    fn terminal_and_unknown_editors_claim_the_terminal() {
        for editor in ["vim", "nvim", "nano", "emacs", "helix", "kak"] {
            let command = EditorCommand::from_editor(editor, &target(Some(42)));
            assert_eq!(command.surface(), EditorSurface::Terminal, "{editor}");
        }
    }

    #[test]
    fn wait_flag_keeps_windowed_editors_blocking() {
        for editor in ["code --wait", "code -w", "zed --wait"] {
            let command = EditorCommand::from_editor(editor, &target(Some(42)));
            assert_eq!(command.surface(), EditorSurface::Terminal, "{editor}");
        }
    }

    #[test]
    fn launching_a_missing_program_fails_immediately() {
        let command = EditorCommand {
            program: "tuicr-no-such-editor".to_string(),
            args: vec![OsString::from("/repo/src/main.rs")],
        };
        assert!(matches!(
            launch_editor(&command),
            Err(EditorError::Launch(_))
        ));
    }

    #[cfg(unix)]
    fn shell_command(script: &str) -> EditorCommand {
        EditorCommand {
            program: "/bin/sh".to_string(),
            args: vec![OsString::from("-c"), OsString::from(script)],
        }
    }

    /// Polls until the editor is no longer running, so the assertions do not
    /// race the child's exit.
    #[cfg(unix)]
    fn poll_until_settled(launch: &mut EditorLaunch) -> LaunchState {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match launch.poll() {
                LaunchState::Running if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                state => return state,
            }
        }
    }

    #[test]
    #[cfg(unix)]
    fn an_editor_that_dies_on_launch_reports_its_status() {
        let mut launch = launch_editor(&shell_command("exit 3")).expect("spawn");
        let LaunchState::FailedToLaunch(status) = poll_until_settled(&mut launch) else {
            panic!("expected a launch failure");
        };
        assert_eq!(status.code(), Some(3));
        assert_eq!(
            EditorError::Exit(status).to_string(),
            "Editor exited with status 3"
        );
    }

    #[test]
    #[cfg(unix)]
    fn a_successful_editor_is_cleaned_up_without_a_message() {
        let mut launch = launch_editor(&shell_command("exit 0")).expect("spawn");
        assert_eq!(poll_until_settled(&mut launch), LaunchState::Exited);
    }

    #[test]
    fn empty_editor_falls_back_to_vi() {
        let command = EditorCommand::from_editor("", &target(None));
        assert_eq!(command.program, "vi");
        assert_eq!(args(&command), vec!["/repo/src/main.rs"]);
    }

    #[test]
    fn config_override_wins_over_env() {
        assert_eq!(
            resolve_editor(Some("from-config"), "from-env"),
            "from-config"
        );
    }

    #[test]
    fn env_is_used_without_config_override() {
        assert_eq!(resolve_editor(None, "from-env"), "from-env");
    }

    #[test]
    fn blank_config_override_falls_back_to_env() {
        assert_eq!(resolve_editor(Some("  "), "from-env"), "from-env");
    }

    #[test]
    fn missing_env_and_config_fall_back_to_vi() {
        let command = EditorCommand::from_editor(&resolve_editor(None, ""), &target(None));
        assert_eq!(command.program, "vi");
    }

    /// A temporary directory holding one named launcher file, standing in for
    /// a single entry of a Windows `PATH`.
    fn launcher_dir(name: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(dir.path().join(name), "@echo off\r\n").expect("write launcher");
        dir
    }

    /// The command the OS would be handed for `editor`, with the search path
    /// and `PATHEXT` supplied explicitly instead of read from the host.
    fn launch_spec_for(
        editor: &str,
        search_path: &[PathBuf],
        pathext: Option<&str>,
    ) -> (String, Vec<String>) {
        let lookup = CommandLookup {
            search_path: search_path.to_vec(),
            pathext: Some(pathext_candidates(pathext)),
        };
        let command = EditorCommand::from_editor(editor, &target(Some(42)));
        let resolved = resolve_command(&command.program, &lookup);
        let (program, argv) = launch_command(&resolved, &command.args);
        let argv = argv
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        (program.to_string_lossy().into_owned(), argv)
    }

    #[test]
    fn a_bare_vscode_command_reaches_its_cmd_launcher_and_its_line() {
        // VS Code ships `code.cmd` on Windows and `Command` never consults
        // PATHEXT, so this is the launch that reported "Failed to launch
        // editor: program not found" while `code` worked in the shell.
        let dir = launcher_dir("code.cmd");
        let launcher = dir.path().join("code.cmd");
        let (program, argv) = launch_spec_for("code", &[dir.path().to_path_buf()], None);
        assert_eq!(program, "cmd.exe");
        assert_eq!(
            argv,
            vec![
                "/C".to_string(),
                launcher.to_string_lossy().into_owned(),
                "--goto".to_string(),
                "/repo/src/main.rs:42".to_string(),
            ]
        );
    }

    #[test]
    fn a_bat_launcher_also_goes_through_the_command_interpreter() {
        let dir = launcher_dir("notepad.bat");
        let (program, _) = launch_spec_for("notepad", &[dir.path().to_path_buf()], None);
        assert_eq!(program, "cmd.exe");
    }

    #[test]
    fn pathext_order_decides_which_launcher_wins() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(dir.path().join("code.exe"), "").expect("write exe");
        std::fs::write(dir.path().join("code.cmd"), "").expect("write cmd");
        let search_path = [dir.path().to_path_buf()];
        let (program, argv) = launch_spec_for("code", &search_path, Some(".EXE;.CMD"));
        let exe = dir.path().join("code.exe").to_string_lossy().into_owned();
        assert_eq!(program, exe);
        assert_eq!(
            argv,
            vec!["--goto".to_string(), "/repo/src/main.rs:42".to_string()]
        );
    }

    #[test]
    fn a_command_missing_from_the_search_path_is_left_to_the_os() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (program, argv) = launch_spec_for("code", &[dir.path().to_path_buf()], Some(".CMD"));
        assert_eq!(program, "code");
        assert_eq!(
            argv,
            vec!["--goto".to_string(), "/repo/src/main.rs:42".to_string()]
        );
    }

    #[test]
    fn a_path_the_user_spelled_out_is_never_rewritten() {
        let dir = launcher_dir("code.cmd");
        let spelled = dir.path().join("code.cmd");
        let search_path = [dir.path().to_path_buf()];
        let (program, _) = launch_spec_for(&spelled.to_string_lossy(), &search_path, Some(".CMD"));
        assert_eq!(program, spelled.to_string_lossy().into_owned());
    }

    #[test]
    fn an_unset_pathext_falls_back_to_the_windows_default() {
        assert_eq!(pathext_candidates(None), ["com", "exe", "bat", "cmd"]);
    }

    #[test]
    fn a_launcher_suffix_does_not_hide_the_editor_family() {
        let installed = [std::path::MAIN_SEPARATOR, "code.cmd"].concat();
        for editor in ["code", "code.cmd", "code.exe", installed.as_str()] {
            let command = EditorCommand::from_editor(editor, &target(Some(42)));
            assert_eq!(args(&command), vec!["--goto", "/repo/src/main.rs:42"]);
            assert_eq!(command.surface(), EditorSurface::Gui, "{editor}");
        }
    }
}
