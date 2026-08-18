//! Git's machine-readable `--raw -z` metadata paired with `--patch` output.
//!
//! With `-z`, path bytes are emitted verbatim and NUL-delimited; `core.quotepath`
//! and every ambiguity in `diff --git` display headers are therefore irrelevant.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::{Result, TuicrError};
use crate::model::{FilePatch, FileStatus};

/// Parse one `git diff --raw -z --patch` byte stream.
pub(crate) fn parse_raw_patch_output(output: &[u8]) -> Result<Vec<FilePatch>> {
    let (raw, patch_text) = split_raw_patch_sections(output)?;
    let metadata = parse_raw_records(raw)?;
    let patch_blocks = split_patch_blocks(patch_text);

    if metadata.len() != patch_blocks.len() {
        return Err(malformed(format!(
            "Git emitted {} raw records but {} patch blocks",
            metadata.len(),
            patch_blocks.len()
        )));
    }

    pair_metadata_with_blocks(metadata, patch_blocks)
}

/// Run Git with the structured diff flags used by non-CLI-backend callers.
pub(crate) fn run_git_diff(root: &Path, diff_args: &[&str]) -> Result<Vec<FilePatch>> {
    let output = Command::new("git")
        .current_dir(root)
        .args([
            "diff",
            "--no-ext-diff",
            "--raw",
            "-z",
            "--patch",
            "--binary",
        ])
        .args(diff_args)
        .output()
        .map_err(|error| TuicrError::VcsCommand(format!("Failed to run git: {error}")))?;
    if !output.status.success() {
        return Err(TuicrError::VcsCommand(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    parse_raw_patch_output(&output.stdout)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FileMetadata {
    pub old_path: Option<PathBuf>,
    pub new_path: Option<PathBuf>,
    pub status: FileStatus,
}

/// Read the authoritative records from the raw section of a combined diff.
///
/// This is also useful when a diff option (notably `--ignore-all-space`)
/// suppresses some patch bodies but Git still emits raw records for them.
pub(crate) fn parse_raw_metadata_from_patch_output(output: &[u8]) -> Result<Vec<FileMetadata>> {
    let (raw, _) = split_raw_patch_sections(output)?;
    parse_raw_records(raw)
}

pub(crate) fn patch_text_from_raw_patch_output(output: &[u8]) -> Result<&[u8]> {
    let (_, patch) = split_raw_patch_sections(output)?;
    Ok(patch)
}

fn split_raw_patch_sections(output: &[u8]) -> Result<(&[u8], &[u8])> {
    if output.is_empty() {
        return Ok((output, output));
    }

    // Git may emit a lone NUL for an empty `--raw -z` section when another
    // diff option (for example `--ignore-all-space`) filters every change.
    if output == b"\0" {
        return Ok((output, &output[1..]));
    }

    let separator = output
        .windows(2)
        .position(|window| window == b"\0\0")
        .ok_or_else(|| malformed("missing NUL boundary between raw metadata and patch text"))?;
    Ok((&output[..=separator], &output[separator + 2..]))
}

/// Pair authoritative metadata with file blocks from a Git-style patch.
///
/// Both inputs must use the source's file order. No path is read from the
/// patch text; a mismatch is an error rather than an invitation to guess.
pub(crate) fn pair_metadata_with_patch(
    metadata: Vec<FileMetadata>,
    patch: &[u8],
) -> Result<Vec<FilePatch>> {
    let patch_blocks = split_patch_blocks(patch);
    if metadata.len() != patch_blocks.len() {
        return Err(pairing_error(format!(
            "source emitted {} metadata records but {} patch blocks",
            metadata.len(),
            patch_blocks.len()
        )));
    }
    pair_metadata_with_blocks(metadata, patch_blocks)
}

fn pair_metadata_with_blocks(
    metadata: Vec<FileMetadata>,
    patch_blocks: Vec<&[u8]>,
) -> Result<Vec<FilePatch>> {
    Ok(metadata
        .into_iter()
        .zip(patch_blocks)
        .map(|(metadata, patch)| {
            let mut file = FilePatch::new(
                metadata.old_path,
                metadata.new_path,
                metadata.status,
                String::from_utf8_lossy(patch).into_owned(),
            );
            file.is_binary = is_binary_patch(patch);
            file
        })
        .collect())
}

fn parse_raw_records(raw: &[u8]) -> Result<Vec<FileMetadata>> {
    let mut fields = raw.split(|byte| *byte == 0).peekable();
    let mut records = Vec::new();

    while let Some(header) = fields.next() {
        if header.is_empty() {
            break;
        }
        let header = std::str::from_utf8(header)
            .map_err(|_| malformed("raw metadata header is not ASCII"))?;
        let status_token = header
            .strip_prefix(':')
            .and_then(|rest| rest.split_whitespace().nth(4))
            .ok_or_else(|| malformed(format!("invalid raw metadata header `{header}`")))?;
        let status_code = status_token
            .as_bytes()
            .first()
            .copied()
            .ok_or_else(|| malformed("raw metadata record has no status"))?;
        let first_path = fields
            .next()
            .filter(|path| !path.is_empty())
            .ok_or_else(|| malformed(format!("raw `{status_token}` record has no path")))?;
        let first_path = path_buf_from_bytes(first_path);

        let record = match status_code {
            b'A' => FileMetadata {
                old_path: None,
                new_path: Some(first_path),
                status: FileStatus::Added,
            },
            b'D' => FileMetadata {
                old_path: Some(first_path),
                new_path: None,
                status: FileStatus::Deleted,
            },
            b'R' | b'C' => {
                let second_path =
                    fields
                        .next()
                        .filter(|path| !path.is_empty())
                        .ok_or_else(|| {
                            malformed(format!("raw `{status_token}` record has one path"))
                        })?;
                FileMetadata {
                    old_path: Some(first_path),
                    new_path: Some(path_buf_from_bytes(second_path)),
                    status: if status_code == b'R' {
                        FileStatus::Renamed
                    } else {
                        FileStatus::Copied
                    },
                }
            }
            b'M' | b'T' | b'U' | b'X' | b'B' => FileMetadata {
                old_path: Some(first_path.clone()),
                new_path: Some(first_path),
                status: FileStatus::Modified,
            },
            other => {
                return Err(malformed(format!(
                    "unsupported raw Git status `{}`",
                    char::from(other)
                )));
            }
        };
        records.push(record);
    }

    Ok(records)
}

/// Split only on top-level Git file markers. Hunk body lines always carry a
/// ` `, `+`, or `-` prefix, so file content cannot produce this column-zero
/// sentinel. Git binary-patch payload lines contain no spaces.
pub(crate) fn split_patch_blocks(patch: &[u8]) -> Vec<&[u8]> {
    const MARKER: &[u8] = b"diff --git ";
    let starts: Vec<usize> = (0..patch.len())
        .filter(|&index| {
            patch[index..].starts_with(MARKER) && (index == 0 || patch[index - 1] == b'\n')
        })
        .collect();

    starts
        .iter()
        .enumerate()
        .map(|(index, start)| {
            let end = starts.get(index + 1).copied().unwrap_or(patch.len());
            &patch[*start..end]
        })
        .collect()
}

pub(crate) fn is_binary_patch(patch: &[u8]) -> bool {
    patch.split(|byte| *byte == b'\n').any(|line| {
        line.starts_with(b"Binary files ")
            || line.starts_with(b"Binary file ")
            || line == b"GIT binary patch"
    })
}

fn malformed(detail: impl Into<String>) -> TuicrError {
    TuicrError::VcsCommand(format!(
        "invalid `git diff --raw -z --patch` output: {}",
        detail.into()
    ))
}

fn pairing_error(detail: impl Into<String>) -> TuicrError {
    TuicrError::VcsCommand(format!(
        "structured diff metadata/patch mismatch: {}",
        detail.into()
    ))
}

#[cfg(unix)]
pub(crate) fn path_buf_from_bytes(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    PathBuf::from(OsString::from_vec(bytes.to_vec()))
}

#[cfg(not(unix))]
pub(crate) fn path_buf_from_bytes(bytes: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn parses_verbatim_paths_without_reading_display_headers() {
        let output = b":100644 100644 aaaaaaa bbbbbbb R100\0old b/ and name.txt\0\xe6\x97\xa5 new.txt\0\0diff --git a/wrong b/wrong\nrename from wrong\nrename to wrong\n";
        let files = parse_raw_patch_output(output).unwrap();

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].status, FileStatus::Renamed);
        assert_eq!(
            files[0].old_path.as_deref(),
            Some(Path::new("old b/ and name.txt"))
        );
        assert_eq!(files[0].new_path.as_deref(), Some(Path::new("日 new.txt")));
    }

    #[test]
    fn parses_empty_nul_terminated_raw_section() {
        assert!(parse_raw_patch_output(b"\0").unwrap().is_empty());
        assert!(
            parse_raw_metadata_from_patch_output(b"\0")
                .unwrap()
                .is_empty()
        );
        assert!(patch_text_from_raw_patch_output(b"\0").unwrap().is_empty());
    }

    #[test]
    fn maps_added_deleted_modified_and_copied_records() {
        let output = b":000000 100644 0000000 aaaaaaa A\0added\0:100644 000000 aaaaaaa 0000000 D\0deleted\0:100644 100644 aaaaaaa bbbbbbb M\0modified\0:100644 100644 aaaaaaa bbbbbbb C75\0source\0copy\0\0diff --git a/ignored b/ignored\nnew file mode 100644\ndiff --git a/ignored b/ignored\ndeleted file mode 100644\ndiff --git a/ignored b/ignored\nindex a..b\ndiff --git a/ignored b/ignored\nsimilarity index 75%\n";
        let files = parse_raw_patch_output(output).unwrap();

        assert_eq!(files.len(), 4);
        assert_eq!(files[0].status, FileStatus::Added);
        assert_eq!(files[1].status, FileStatus::Deleted);
        assert_eq!(files[2].status, FileStatus::Modified);
        assert_eq!(files[3].status, FileStatus::Copied);
        assert_eq!(files[3].old_path.as_deref(), Some(Path::new("source")));
        assert_eq!(files[3].new_path.as_deref(), Some(Path::new("copy")));
    }

    #[test]
    fn marks_binary_patch_from_body_without_parsing_its_paths() {
        let output = b":100644 100644 aaaaaaa bbbbbbb M\0photo and image.png\0\0diff --git a/nonsense b/nonsense\nBinary files a/nonsense and b/nonsense differ\n";
        let files = parse_raw_patch_output(output).unwrap();
        assert!(files[0].is_binary);
        assert_eq!(
            files[0].new_path.as_deref(),
            Some(Path::new("photo and image.png"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn preserves_non_utf8_path_bytes_on_unix() {
        use std::os::unix::ffi::OsStrExt;

        let output = b":100644 100644 aaaaaaa bbbbbbb M\0bad-\xff-name\0\0diff --git a/x b/x\n";
        let files = parse_raw_patch_output(output).unwrap();
        assert_eq!(
            files[0].new_path.as_ref().unwrap().as_os_str().as_bytes(),
            b"bad-\xff-name"
        );
    }

    #[test]
    fn rejects_metadata_and_patch_count_mismatch() {
        let output = b":100644 100644 aaaaaaa bbbbbbb M\0one\0\0";
        let error = parse_raw_patch_output(output).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("1 raw records but 0 patch blocks")
        );
    }
}
