use std::fmt;
use std::io::{Cursor, Read};
use std::path::Path;

use flate2::read::GzDecoder;

use super::UpdateError;

pub(super) fn extract_binary(asset_name: &str, archive: &[u8]) -> Result<Vec<u8>, UpdateError> {
    match asset_name {
        name if name.ends_with(".tar.gz") => extract_tar_gz(name, archive),
        name if name.ends_with(".zip") => extract_zip(name, archive),
        name => Err(archive_error(name, "unsupported archive format")),
    }
}

fn extract_tar_gz(asset_name: &str, archive: &[u8]) -> Result<Vec<u8>, UpdateError> {
    let mut tar = tar::Archive::new(GzDecoder::new(Cursor::new(archive)));
    let mut entries = tar
        .entries()
        .map_err(|error| archive_error(asset_name, error))?;
    let Some(entry) = entries.next() else {
        return Err(missing_binary(asset_name, "tuicr"));
    };
    let mut entry = entry.map_err(|error| archive_error(asset_name, error))?;
    let path = entry
        .path()
        .map_err(|error| archive_error(asset_name, error))?
        .into_owned();
    if !entry.header().entry_type().is_file() || path != Path::new("tuicr") {
        return Err(missing_binary(asset_name, "tuicr"));
    }
    read_binary(asset_name, &mut entry)
}

fn extract_zip(asset_name: &str, archive: &[u8]) -> Result<Vec<u8>, UpdateError> {
    let mut zip = zip::ZipArchive::new(Cursor::new(archive))
        .map_err(|error| archive_error(asset_name, error))?;
    let mut file = zip
        .by_name("tuicr.exe")
        .map_err(|_| missing_binary(asset_name, "tuicr.exe"))?;
    read_binary(asset_name, &mut file)
}

fn read_binary(asset_name: &str, reader: &mut impl Read) -> Result<Vec<u8>, UpdateError> {
    let mut binary = Vec::new();
    reader
        .read_to_end(&mut binary)
        .map_err(|error| archive_error(asset_name, error))?;
    Ok(binary)
}

fn missing_binary(asset_name: &str, binary_name: &str) -> UpdateError {
    archive_error(
        asset_name,
        format!("archive does not contain {binary_name} at its root"),
    )
}

fn archive_error(asset_name: &str, error: impl fmt::Display) -> UpdateError {
    UpdateError::Archive {
        asset: asset_name.to_string(),
        detail: error.to_string(),
    }
}
