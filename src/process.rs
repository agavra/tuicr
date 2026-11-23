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
    run_command_output_with_env(program, current_dir, args, &[])
}

/// Variant of `run_command_output` that sets `envs` on the child. Callers that
/// only probe for an object in a partial clone use this to disable lazy
/// promisor fetches: git < 2.45 ignores the variable, newer git fails fast.
pub(crate) fn run_command_output_with_env<I, S>(
    program: &str,
    current_dir: Option<&Path>,
    args: I,
    envs: &[(&str, &str)],
) -> CommandOutputResult<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new(program);
    if let Some(current_dir) = current_dir {
        command.current_dir(current_dir);
    }
    command.envs(envs.iter().copied());

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

    let child = command
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

    let output = wait_with_stdin(child, Some(stdin))?;

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

/// Both output streams of a finished command, captured regardless of exit
/// status.
///
/// `run_command_output` collapses a non-zero exit into an error and discards
/// stderr on success, which is the right shape for CLIs that signal failure
/// through their exit code. `tea api` does not: it exits 0 on every HTTP
/// response, writes the response body to stdout, and (under `-i`) writes the
/// status line and headers to stderr. Reading both streams is the only way to
/// tell a 200 from a 404, and the headers carry the pagination totals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandStreams {
    pub stdout: String,
    pub stderr: String,
    pub status: Option<i32>,
}

/// Run `program` and capture both streams, succeeding even when the child
/// exits non-zero. Spawn failures are still errors — the caller cannot
/// interpret output that was never produced.
///
/// `stdin` is written to the child and the pipe closed before waiting, so
/// commands that read a payload (`tea api -d @-`) see EOF.
pub fn run_command_streams<I, S>(
    program: &str,
    current_dir: Option<&Path>,
    args: I,
    stdin: Option<&str>,
) -> CommandOutputResult<CommandStreams>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new(program);
    if let Some(current_dir) = current_dir {
        command.current_dir(current_dir);
    }

    let child = command
        .args(args)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
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

    let output = wait_with_stdin(child, stdin)?;

    Ok(CommandStreams {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        status: output.status.code(),
    })
}

/// Feed `stdin` to `child` on a background thread while reaping the child and
/// draining stdout/stderr via `wait_with_output`.
///
/// Running the write concurrently prevents pipe-buffer deadlocks on large
/// payloads, ignores `BrokenPipe` when the child exits before consuming stdin,
/// and always reaps the child so no zombie leaks and CLI diagnostics on
/// stderr/stdout are preserved.
fn wait_with_stdin(
    mut child: std::process::Child,
    stdin: Option<&str>,
) -> CommandOutputResult<std::process::Output> {
    let writer = stdin.and_then(|payload| {
        let mut child_stdin = child.stdin.take()?;
        let bytes = payload.as_bytes().to_vec();
        Some(std::thread::spawn(move || {
            match child_stdin.write_all(&bytes) {
                Ok(()) => Ok(()),
                Err(err) if err.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
                Err(err) => Err(err),
            }
        }))
    });

    let output = child.wait_with_output().map_err(|err| CommandOutputError {
        kind: CommandOutputErrorKind::SpawnFailed,
        status: None,
        stderr: err.to_string(),
    });

    let write_result = writer.map(|handle| {
        handle
            .join()
            .unwrap_or_else(|_| Err(std::io::Error::other("stdin writer thread panicked")))
    });

    let output = output?;

    if output.status.success()
        && let Some(Err(err)) = write_result
    {
        return Err(CommandOutputError {
            kind: CommandOutputErrorKind::SpawnFailed,
            status: None,
            stderr: err.to_string(),
        });
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn stdin_broken_pipe_preserves_cli_stderr_in_output_and_streams() {
        // Larger than OS pipe buffer (64 KiB on macOS/Linux) so synchronous
        // write_all hits BrokenPipe when the child exits without reading stdin.
        let large_payload = "x".repeat(512 * 1024);

        let err = run_command_output_with_stdin(
            "sh",
            None,
            ["-c", "echo real-cli-error >&2; exit 2"],
            &large_payload,
        )
        .expect_err("non-zero exit should return Err");
        assert_eq!(err.kind, CommandOutputErrorKind::Unsuccessful);
        assert_eq!(err.status, Some(2));
        assert!(
            err.stderr.contains("real-cli-error"),
            "expected CLI stderr to be preserved, got: {:?}",
            err
        );

        let streams = run_command_streams(
            "sh",
            None,
            [
                "-c",
                "echo real-stream-err >&2; echo real-stream-out; exit 3",
            ],
            Some(&large_payload),
        )
        .expect("run_command_streams should capture output even on early exit");
        assert_eq!(streams.status, Some(3));
        assert!(streams.stderr.contains("real-stream-err"));
        assert!(streams.stdout.contains("real-stream-out"));
    }
}
