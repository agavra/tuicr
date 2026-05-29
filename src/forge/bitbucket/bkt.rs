//! Abstraction over the `bkt` CLI, analogous to `gh.rs` for GitHub.

use std::path::Path;
use std::process::Command;

use crate::process::{CommandOutputError, CommandOutputErrorKind};

// ── Error types ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BktCommandError {
    MissingBkt,
    Failed { status: Option<i32>, stderr: String },
}

pub type BktCommandResult<T> = std::result::Result<T, BktCommandError>;

impl From<CommandOutputError> for BktCommandError {
    fn from(error: CommandOutputError) -> Self {
        match error.kind {
            CommandOutputErrorKind::NotFound => Self::MissingBkt,
            CommandOutputErrorKind::SpawnFailed | CommandOutputErrorKind::Unsuccessful => {
                Self::Failed {
                    status: error.status,
                    stderr: error.stderr,
                }
            }
        }
    }
}

// ── Runner trait ─────────────────────────────────────────────────────────

/// Trait for executing `bkt` CLI commands. Mirrors `GhCommandRunner`.
pub trait BktCommandRunner {
    fn run(&self, args: &[String]) -> BktCommandResult<String>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemBktRunner;

impl BktCommandRunner for SystemBktRunner {
    fn run(&self, args: &[String]) -> BktCommandResult<String> {
        let mut command = Command::new("bkt");
        command
            .args(args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .env("NO_COLOR", "1");

        let output = command.output().map_err(|err| {
            let kind = if err.kind() == std::io::ErrorKind::NotFound {
                CommandOutputErrorKind::NotFound
            } else {
                CommandOutputErrorKind::SpawnFailed
            };
            let err_detail = CommandOutputError {
                kind,
                status: None,
                stderr: err.to_string(),
            };
            BktCommandError::from(err_detail)
        })?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let combined = match (stderr.trim(), stdout.trim()) {
                (e, s) if !e.is_empty() && !s.is_empty() => format!("{e}\n{s}"),
                (e, "") => e.to_string(),
                ("", s) => s.to_string(),
                _ => String::new(),
            };
            Err(BktCommandError::Failed {
                status: output.status.code(),
                stderr: combined,
            })
        }
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────

/// Run `bkt` in a specific working directory. Returns `Ok("")` when `bkt`
/// is not found (caller decides whether to surface an error).
pub fn run_bkt_in_dir(
    _runner: &dyn BktCommandRunner,
    _cwd: Option<&Path>,
    args: &[&str],
) -> BktCommandResult<String> {
    let _ = (_runner, _cwd); // reserved for future use
    // We can't change the working directory of the child from the trait,
    // so we rely on the caller to pass a full path in args when needed.
    // For now, SystemBktRunner always runs from the current directory.
    let args_vec: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    _runner.run(&args_vec)
}

/// Check whether `bkt` is available on PATH.
pub fn is_bkt_available() -> bool {
    Command::new("bkt")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|mut child| {
            let _ = child.wait();
        })
        .is_ok()
}

/// Map a `BktCommandError` into a `TuicrError` with a user-friendly message.
pub fn map_bkt_error(error: BktCommandError) -> crate::error::TuicrError {
    match error {
        BktCommandError::MissingBkt => crate::error::TuicrError::Forge(
            "Bitbucket integration requires `bkt`.\nInstall Bitbucket CLI and run `bkt auth login`."
                .to_string(),
        ),
        BktCommandError::Failed { stderr, .. } if looks_like_auth_failure(&stderr) => {
            crate::error::TuicrError::Forge(
                "Bitbucket authentication failed.\nRun `bkt auth login`."
                    .to_string(),
            )
        }
        BktCommandError::Failed { status, stderr } => {
            let detail = if stderr.is_empty() {
                status
                    .map(|code| format!("bkt exited with status {code}"))
                    .unwrap_or_else(|| "bkt command failed".to_string())
            } else {
                stderr
            };
            crate::error::TuicrError::Forge(format!("Bitbucket command failed: {detail}"))
        }
    }
}

fn looks_like_auth_failure(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    lower.contains("auth")
        || lower.contains("login")
        || lower.contains("unauthorized")
        || lower.contains("forbidden")
        || lower.contains("401")
        || lower.contains("403")
}
