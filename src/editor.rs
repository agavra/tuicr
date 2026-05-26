use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

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
    /// Builds an invocation from `$EDITOR`.
    ///
    /// An unset, empty, or unparsable value falls back to `vi` so the caller
    /// always gets a concrete command to run.
    pub fn from_env(target: &EditorTarget) -> Self {
        let editor = std::env::var("EDITOR").unwrap_or_default();
        Self::from_editor(&editor, target)
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

fn editor_family(program: &str) -> EditorFamily {
    let name = Path::new(program)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(program);
    match name {
        "vi" | "vim" | "nvim" | "nano" => EditorFamily::PlusLine,
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

/// Opens `target` in the user's editor.
///
/// The caller owns terminal restoration before displaying any returned error.
pub fn run_editor(target: &EditorTarget) -> Result<(), EditorError> {
    let command = EditorCommand::from_env(target);
    match command.run() {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(EditorError::Exit(status)),
        Err(err) => Err(EditorError::Launch(err)),
    }
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
        for editor in ["vi", "vim", "nvim", "nano"] {
            let command = EditorCommand::from_editor(editor, &target(Some(42)));
            assert_eq!(command.program, editor);
            assert_eq!(args(&command), vec!["+42", "/repo/src/main.rs"]);
        }
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
    fn empty_editor_falls_back_to_vi() {
        let command = EditorCommand::from_editor("", &target(None));
        assert_eq!(command.program, "vi");
        assert_eq!(args(&command), vec!["/repo/src/main.rs"]);
    }
}
