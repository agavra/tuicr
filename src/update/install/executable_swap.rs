use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use super::UpdateError;

pub(super) fn swap_executable(path: &Path, contents: &[u8]) -> Result<(), UpdateError> {
    let parent = path
        .parent()
        .ok_or_else(|| replacement_error(path, "missing parent directory"))?;
    let temp_path = parent.join(format!(".tuicr-update-{}", uuid::Uuid::new_v4()));
    write_new_file(path, &temp_path, contents)?;

    #[cfg(unix)]
    swap_unix(path, &temp_path)?;
    #[cfg(windows)]
    swap_windows(path, &temp_path)?;

    Ok(())
}

fn write_new_file(target: &Path, temp_path: &Path, contents: &[u8]) -> Result<(), UpdateError> {
    let result = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temp_path)
        .and_then(|mut file| file.write_all(contents));
    if let Err(error) = result {
        let _ = fs::remove_file(temp_path);
        return Err(replacement_error(target, error));
    }
    Ok(())
}

#[cfg(unix)]
fn swap_unix(target: &Path, temp_path: &Path) -> Result<(), UpdateError> {
    use std::os::unix::fs::PermissionsExt;

    let mode = fs::metadata(target)
        .map(|metadata| metadata.permissions().mode())
        .unwrap_or(0o755);
    fs::set_permissions(temp_path, fs::Permissions::from_mode(mode))
        .and_then(|()| fs::rename(temp_path, target))
        .map_err(|error| {
            let _ = fs::remove_file(temp_path);
            replacement_error(target, error)
        })
}

#[cfg(windows)]
fn swap_windows(target: &Path, temp_path: &Path) -> Result<(), UpdateError> {
    let backup_path = target.with_extension("exe.old");
    let _ = fs::remove_file(&backup_path);
    fs::rename(target, &backup_path).map_err(|error| {
        let _ = fs::remove_file(temp_path);
        replacement_error(target, error)
    })?;
    if let Err(error) = fs::rename(temp_path, target) {
        let _ = fs::rename(&backup_path, target);
        let _ = fs::remove_file(temp_path);
        return Err(replacement_error(target, error));
    }
    let _ = fs::remove_file(backup_path);
    Ok(())
}

fn replacement_error(path: &Path, error: impl std::fmt::Display) -> UpdateError {
    UpdateError::Replace {
        path: PathBuf::from(path),
        detail: error.to_string(),
    }
}
