use std::ffi::OsStr;
use std::fmt;
use std::io::{Cursor, Read};
use std::path::Path;

use flate2::read::GzDecoder;

use super::UpdateError;

pub(super) fn release_asset_name(
    version: &str,
    os: &str,
    arch: &str,
) -> Result<String, UpdateError> {
    let target = match (os, arch) {
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        _ => {
            return Err(UpdateError::UnsupportedPlatform {
                os: os.to_string(),
                arch: arch.to_string(),
            });
        }
    };
    let extension = if os == "windows" { "zip" } else { "tar.gz" };
    Ok(format!("tuicr-{version}-{target}.{extension}"))
}

pub(super) fn extract_binary(asset_name: &str, archive: &[u8]) -> Result<Vec<u8>, UpdateError> {
    match asset_name {
        name if name.ends_with(".tar.gz") => extract_tar_gz(name, archive),
        name if name.ends_with(".zip") => extract_zip(name, archive),
        name => Err(archive_error(name, "unsupported archive format")),
    }
}

fn extract_tar_gz(asset_name: &str, archive: &[u8]) -> Result<Vec<u8>, UpdateError> {
    let mut tar = tar::Archive::new(GzDecoder::new(Cursor::new(archive)));
    for entry in tar
        .entries()
        .map_err(|error| archive_error(asset_name, error))?
    {
        let mut entry = entry.map_err(|error| archive_error(asset_name, error))?;
        let path = entry
            .path()
            .map_err(|error| archive_error(asset_name, error))?;
        if entry.header().entry_type().is_file() && path.file_name() == Some(OsStr::new("tuicr")) {
            let mut binary = Vec::new();
            entry
                .read_to_end(&mut binary)
                .map_err(|error| archive_error(asset_name, error))?;
            return Ok(binary);
        }
    }
    Err(archive_error(
        asset_name,
        "archive does not contain the tuicr binary",
    ))
}

fn extract_zip(asset_name: &str, archive: &[u8]) -> Result<Vec<u8>, UpdateError> {
    let mut zip = zip::ZipArchive::new(Cursor::new(archive))
        .map_err(|error| archive_error(asset_name, error))?;
    for index in 0..zip.len() {
        let mut file = zip
            .by_index(index)
            .map_err(|error| archive_error(asset_name, error))?;
        if !file.is_dir() && Path::new(file.name()).file_name() == Some(OsStr::new("tuicr.exe")) {
            let mut binary = Vec::new();
            file.read_to_end(&mut binary)
                .map_err(|error| archive_error(asset_name, error))?;
            return Ok(binary);
        }
    }
    Err(archive_error(
        asset_name,
        "archive does not contain the tuicr.exe binary",
    ))
}

fn archive_error(asset_name: &str, error: impl fmt::Display) -> UpdateError {
    UpdateError::Archive {
        asset: asset_name.to_string(),
        detail: error.to_string(),
    }
}
