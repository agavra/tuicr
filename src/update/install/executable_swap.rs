use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use super::UpdateError;

pub(super) fn swap_executable(target: &Path, contents: &[u8]) -> Result<(), UpdateError> {
    validate_target(target)?;
    let running_executable = std::env::current_exe()
        .and_then(fs::canonicalize)
        .map_err(|error| replacement_error(target, error))?;
    let target_executable =
        fs::canonicalize(target).map_err(|error| replacement_error(target, error))?;
    if target_executable != running_executable {
        return Err(replacement_error(
            target,
            "refusing to replace an executable other than the running tuicr binary",
        ));
    }

    stage_and_swap(target, contents, |staged| {
        self_replace::self_replace(staged)
    })
}

pub(super) fn stage_and_swap(
    target: &Path,
    contents: &[u8],
    swap: impl FnOnce(&Path) -> std::io::Result<()>,
) -> Result<(), UpdateError> {
    let metadata = validate_target(target)?;
    if contents.is_empty() {
        return Err(replacement_error(target, "downloaded executable is empty"));
    }
    let parent = target
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| replacement_error(target, "missing parent directory"))?;
    let mut staged = tempfile::Builder::new()
        .prefix(".tuicr-update-")
        .tempfile_in(parent)
        .map_err(|error| replacement_error(target, error))?;
    staged
        .as_file_mut()
        .write_all(contents)
        .and_then(|()| staged.as_file().set_permissions(metadata.permissions()))
        .and_then(|()| staged.as_file().sync_all())
        .map_err(|error| replacement_error(target, error))?;

    swap(staged.path()).map_err(|error| replacement_error(target, error))
}

fn validate_target(target: &Path) -> Result<fs::Metadata, UpdateError> {
    if !target.is_absolute() {
        return Err(replacement_error(target, "executable path is not absolute"));
    }
    let metadata =
        fs::symlink_metadata(target).map_err(|error| replacement_error(target, error))?;
    if metadata.file_type().is_symlink() {
        return Err(replacement_error(target, "executable path is a symlink"));
    }
    if !metadata.is_file() {
        return Err(replacement_error(
            target,
            "executable path is not a regular file",
        ));
    }
    Ok(metadata)
}

fn replacement_error(path: &Path, error: impl std::fmt::Display) -> UpdateError {
    UpdateError::Replace {
        path: PathBuf::from(path),
        detail: error.to_string(),
    }
}
