use std::env;
use std::fs;
#[cfg(unix)]
use std::fs::File;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum EditorError {
    #[error("neither $VISUAL nor $EDITOR is set")]
    MissingEditor,
    #[error("failed to parse editor command")]
    ParseFailed,
    #[error("editor command is empty")]
    EmptyCommand,
    #[error("editor exited with status {0}")]
    EditorFailed(std::process::ExitStatus),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

fn parse_editor_command(raw: &str) -> Result<Vec<String>, EditorError> {
    let parts = shlex::split(raw).ok_or(EditorError::ParseFailed)?;
    if parts.is_empty() {
        return Err(EditorError::EmptyCommand);
    }
    Ok(parts)
}

fn resolve_editor_command_from_env(
    visual: Option<String>,
    editor: Option<String>,
) -> Result<Vec<String>, EditorError> {
    let raw = visual.or(editor).ok_or(EditorError::MissingEditor)?;
    parse_editor_command(&raw)
}

/// Resolve the editor command from environment variables.
/// Prefers `$VISUAL` over `$EDITOR`.
pub fn resolve_editor_command() -> Result<Vec<String>, EditorError> {
    resolve_editor_command_from_env(env::var("VISUAL").ok(), env::var("EDITOR").ok())
}

/// Write `seed` to a temp file, launch the editor, and return the updated content.
///
/// The caller is responsible for suspending and restoring the TUI around this call
/// (leave alternate screen, disable raw mode, etc.).
pub fn run_editor(
    seed: &str,
    editor_cmd: &[String],
    attach_to_tty: bool,
) -> Result<String, EditorError> {
    if editor_cmd.is_empty() {
        return Err(EditorError::EmptyCommand);
    }

    // Convert to TempPath immediately so no file handle stays open while the
    // editor runs.
    let temp_path = tempfile::Builder::new()
        .suffix(".md")
        .tempfile()?
        .into_temp_path();
    fs::write(&temp_path, seed)?;

    let mut cmd = Command::new(&editor_cmd[0]);
    if editor_cmd.len() > 1 {
        cmd.args(&editor_cmd[1..]);
    }
    configure_editor_stdio(&mut cmd, attach_to_tty)?;
    let status = cmd.arg(&temp_path).status()?;

    if !status.success() {
        return Err(EditorError::EditorFailed(status));
    }

    let contents = fs::read_to_string(&temp_path)?;
    Ok(contents)
}

fn configure_editor_stdio(cmd: &mut Command, attach_to_tty: bool) -> std::io::Result<()> {
    if !attach_to_tty {
        cmd.stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        return Ok(());
    }

    #[cfg(unix)]
    {
        let tty = File::options().read(true).write(true).open("/dev/tty")?;
        let tty_out = tty.try_clone()?;
        let tty_err = tty.try_clone()?;
        cmd.stdin(Stdio::from(tty))
            .stdout(Stdio::from(tty_out))
            .stderr(Stdio::from(tty_err));
    }

    #[cfg(not(unix))]
    {
        cmd.stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
    }

    Ok(())
}

/// Normalize editor output for in-app use.
///
/// Strips one trailing newline that many editors add automatically on save.
pub fn normalize_edited_content(contents: &str) -> String {
    contents.strip_suffix('\n').unwrap_or(contents).to_owned()
}

/// Returns the short editor name for display (e.g. "nvim", "code").
pub fn editor_display_name(editor_cmd: &[String]) -> String {
    editor_cmd
        .first()
        .map(|cmd| {
            PathBuf::from(cmd)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(cmd.as_str())
                .to_owned()
        })
        .unwrap_or_else(|| "editor".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_editor_prefers_visual() {
        let cmd = resolve_editor_command_from_env(Some("vis".to_string()), Some("ed".to_string()))
            .unwrap();
        assert_eq!(cmd, vec!["vis".to_string()]);
    }

    #[test]
    fn resolve_editor_falls_back_to_editor() {
        let cmd = resolve_editor_command_from_env(None, Some("nano".to_string())).unwrap();
        assert_eq!(cmd, vec!["nano".to_string()]);
    }

    #[test]
    fn resolve_editor_errors_when_unset() {
        assert!(matches!(
            resolve_editor_command_from_env(None, None),
            Err(EditorError::MissingEditor)
        ));
    }

    #[test]
    fn resolve_editor_parses_arguments() {
        let cmd = parse_editor_command("code --wait").unwrap();
        assert_eq!(cmd, vec!["code".to_string(), "--wait".to_string()]);
    }

    #[test]
    fn editor_display_name_extracts_basename() {
        let cmd = vec!["/usr/bin/nvim".to_string()];
        assert_eq!(editor_display_name(&cmd), "nvim");
    }

    #[test]
    fn editor_display_name_handles_bare_name() {
        let cmd = vec!["nano".to_string()];
        assert_eq!(editor_display_name(&cmd), "nano");
    }

    #[test]
    fn normalize_edited_content_trims_one_newline() {
        assert_eq!(normalize_edited_content("first\n\n"), "first\n");
    }

    #[test]
    #[cfg(unix)]
    fn run_editor_returns_updated_content() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let script_path = dir.path().join("edit.sh");
        fs::write(&script_path, "#!/bin/sh\nprintf \"edited\" > \"$1\"\n").unwrap();
        let mut perms = fs::metadata(&script_path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).unwrap();

        let cmd = vec![script_path.to_string_lossy().to_string()];
        let result = run_editor("seed", &cmd, false).unwrap();
        assert_eq!(result, "edited");
    }
}
