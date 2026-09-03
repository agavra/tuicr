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
    /// Absolute path to the local worktree file.
    pub path: PathBuf,
    /// One-based source line to request from editors that support it.
    pub line: Option<u32>,
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

    /// Runs the prepared editor command and waits for it to exit.
    ///
    /// The caller owns terminal suspension and restoration around this process
    /// boundary.
    pub fn run(&self) -> std::io::Result<std::process::ExitStatus> {
        Command::new(&self.program).args(&self.args).status()
    }

    /// Spawns the prepared editor command without waiting for it to exit.
    ///
    /// Standard streams are detached so a chatty editor cannot write over the
    /// TUI, which stays on screen for the whole handoff. The caller polls the
    /// returned handle so the finished process gets cleaned up.
    pub fn spawn_detached(&self) -> std::io::Result<EditorLaunch> {
        let child = Command::new(&self.program)
            .args(&self.args)
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
    let name = Path::new(program)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(program);
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
    let name = Path::new(program)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(program);
    match name {
        "vi" | "vim" | "nvim" | "nano" | "emacs" | "emacsclient" | "hx" => EditorFamily::PlusLine,
        "code" | "code-insiders" | "codium" | "cursor" => EditorFamily::GotoLine,
        _ => EditorFamily::Plain,
    }
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
        assert_eq!(resolve_editor(Some("from-config"), "from-env"), "from-config");
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
}
