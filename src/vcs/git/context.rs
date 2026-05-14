use git2::Repository;
use std::path::Path;

use crate::error::{Result, TuicrError};
use crate::model::{DiffLine, FileStatus, LineOrigin};

/// Fetch context lines from a file for gap expansion.
///
/// When `ref_commit` is `Some`, reads from that commit's tree.
/// Otherwise: working tree for non-deleted, HEAD blob for deleted.
pub fn fetch_context_lines(
    repo: &Repository,
    file_path: &Path,
    file_status: FileStatus,
    ref_commit: Option<&str>,
    start_line: u32,
    end_line: u32,
) -> Result<Vec<DiffLine>> {
    if start_line > end_line || start_line == 0 {
        return Ok(Vec::new());
    }

    let content = read_file_content(repo, file_path, file_status, ref_commit)?;

    let lines: Vec<&str> = content.lines().collect();
    let mut result = Vec::new();

    for line_num in start_line..=end_line {
        let idx = (line_num - 1) as usize;
        if idx < lines.len() {
            result.push(DiffLine {
                origin: LineOrigin::Context,
                content: lines[idx].to_string(),
                old_lineno: Some(line_num),
                new_lineno: Some(line_num),
                highlighted_spans: None,
            });
        }
    }

    Ok(result)
}

/// Get the total number of lines in a file.
pub fn file_line_count(
    repo: &Repository,
    file_path: &Path,
    file_status: FileStatus,
    ref_commit: Option<&str>,
) -> Result<u32> {
    let content = read_file_content(repo, file_path, file_status, ref_commit)?;
    Ok(content.lines().count() as u32)
}

fn read_file_content(
    repo: &Repository,
    file_path: &Path,
    file_status: FileStatus,
    ref_commit: Option<&str>,
) -> Result<String> {
    if let Some(commit) = ref_commit {
        fetch_commit_blob_content(repo, commit, file_path)
    } else if file_status == FileStatus::Deleted {
        fetch_blob_content(repo, file_path)
    } else {
        let workdir = repo.workdir().ok_or(TuicrError::NotARepository)?;
        Ok(std::fs::read_to_string(workdir.join(file_path))?)
    }
}

/// Fetch content from HEAD blob (for deleted files without a ref_commit).
fn fetch_blob_content(repo: &Repository, file_path: &Path) -> Result<String> {
    let head = repo.head()?.peel_to_tree()?;
    let entry = head.get_path(file_path)?;
    let blob = repo.find_blob(entry.id())?;
    let content = std::str::from_utf8(blob.content())
        .map_err(|e| TuicrError::CorruptedSession(format!("Invalid UTF-8 in file: {e}")))?;
    Ok(content.to_string())
}

/// Fetch content from an arbitrary commit's tree.
fn fetch_commit_blob_content(
    repo: &Repository,
    commit_spec: &str,
    file_path: &Path,
) -> Result<String> {
    let obj = repo.revparse_single(commit_spec)?;
    let tree = obj.peel_to_tree()?;
    let entry = tree.get_path(file_path)?;
    let blob = repo.find_blob(entry.id())?;
    let content = std::str::from_utf8(blob.content())
        .map_err(|e| TuicrError::CorruptedSession(format!("Invalid UTF-8 in file: {e}")))?;
    Ok(content.to_string())
}

/// Calculate the number of hidden lines (gap) before a hunk.
///
/// Returns the count of lines between the end of the previous hunk
/// and the start of the current hunk.
pub fn calculate_gap(
    prev_hunk: Option<(&u32, &u32)>, // (new_start, new_count)
    current_new_start: u32,
) -> u32 {
    match prev_hunk {
        None => {
            // Gap from line 1 to first hunk
            current_new_start.saturating_sub(1)
        }
        Some((prev_start, prev_count)) => {
            // Gap between end of prev hunk and start of current
            let prev_end = prev_start + prev_count;
            current_new_start.saturating_sub(prev_end)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_calculate_gap_before_first_hunk() {
        // given
        let current_start = 10;

        // when
        let gap = calculate_gap(None, current_start);

        // then
        assert_eq!(gap, 9); // Lines 1-9 are hidden
    }

    #[test]
    fn should_calculate_gap_between_hunks() {
        // given
        let prev_start = 5;
        let prev_count = 3; // Hunk covers lines 5-7
        let current_start = 15;

        // when
        let gap = calculate_gap(Some((&prev_start, &prev_count)), current_start);

        // then
        assert_eq!(gap, 7); // Lines 8-14 are hidden
    }

    #[test]
    fn should_return_zero_for_adjacent_hunks() {
        // given
        let prev_start = 5;
        let prev_count = 3; // Hunk covers lines 5-7
        let current_start = 8; // Starts immediately after

        // when
        let gap = calculate_gap(Some((&prev_start, &prev_count)), current_start);

        // then
        assert_eq!(gap, 0);
    }

    #[test]
    fn should_handle_overlapping_hunks() {
        // given
        let prev_start = 5;
        let prev_count = 10; // Hunk covers lines 5-14
        let current_start = 12; // Overlaps (shouldn't happen in practice)

        // when
        let gap = calculate_gap(Some((&prev_start, &prev_count)), current_start);

        // then
        assert_eq!(gap, 0); // saturating_sub prevents underflow
    }
}
