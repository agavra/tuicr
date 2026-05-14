use std::ffi::OsStr;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandOutputErrorKind {
    NotFound,
    SpawnFailed,
    Unsuccessful,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutputError {
    pub kind: CommandOutputErrorKind,
    pub status: Option<i32>,
    pub stderr: String,
}

pub type CommandOutputResult<T> = std::result::Result<T, CommandOutputError>;

pub fn run_command_output<I, S>(
    program: &str,
    current_dir: Option<&Path>,
    args: I,
) -> CommandOutputResult<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new(program);
    if let Some(current_dir) = current_dir {
        command.current_dir(current_dir);
    }

    let output = command.args(args).output().map_err(|err| {
        let kind = if err.kind() == std::io::ErrorKind::NotFound {
            CommandOutputErrorKind::NotFound
        } else {
            CommandOutputErrorKind::SpawnFailed
        };
        CommandOutputError {
            kind,
            status: None,
            stderr: err.to_string(),
        }
    })?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(CommandOutputError {
            kind: CommandOutputErrorKind::Unsuccessful,
            status: output.status.code(),
            stderr: combine_streams_for_error(&output.stdout, &output.stderr),
        })
    }
}

/// Build the `stderr` field of a failed-command error from the child's
/// stdout + stderr. `gh api` puts the JSON response body on stdout even on
/// non-2xx, while stderr only carries a short status line — surfacing both
/// (with a separator when both are populated) lets the caller relay the
/// real API error.
fn combine_streams_for_error(stdout: &[u8], stderr: &[u8]) -> String {
    let stderr = String::from_utf8_lossy(stderr);
    let stdout = String::from_utf8_lossy(stdout);
    match (stderr.trim(), stdout.trim()) {
        (e, s) if !e.is_empty() && !s.is_empty() => format!("{e}\n{s}"),
        (e, "") => e.to_string(),
        ("", s) => s.to_string(),
        _ => String::new(),
    }
}

/// Variant of `run_command_output` that pipes `stdin` bytes into the spawned
/// child. Used by `gh api --input -` (and any future tools that want the
/// same shape).
pub fn run_command_output_with_stdin<I, S>(
    program: &str,
    current_dir: Option<&Path>,
    args: I,
    stdin: &str,
) -> CommandOutputResult<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new(program);
    if let Some(current_dir) = current_dir {
        command.current_dir(current_dir);
    }

    let mut child = command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| {
            let kind = if err.kind() == std::io::ErrorKind::NotFound {
                CommandOutputErrorKind::NotFound
            } else {
                CommandOutputErrorKind::SpawnFailed
            };
            CommandOutputError {
                kind,
                status: None,
                stderr: err.to_string(),
            }
        })?;

    // Write the stdin payload before waiting on stdout, then drop the handle
    // so the child sees EOF and can finish.
    if let Some(mut child_stdin) = child.stdin.take() {
        child_stdin
            .write_all(stdin.as_bytes())
            .map_err(|err| CommandOutputError {
                kind: CommandOutputErrorKind::SpawnFailed,
                status: None,
                stderr: err.to_string(),
            })?;
        // `drop(child_stdin)` happens when the value goes out of scope, which
        // closes the pipe and signals EOF.
    }

    let output = child.wait_with_output().map_err(|err| CommandOutputError {
        kind: CommandOutputErrorKind::SpawnFailed,
        status: None,
        stderr: err.to_string(),
    })?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(CommandOutputError {
            kind: CommandOutputErrorKind::Unsuccessful,
            status: output.status.code(),
            stderr: combine_streams_for_error(&output.stdout, &output.stderr),
        })
    }
}
