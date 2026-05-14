use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

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
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        })
    }
}
