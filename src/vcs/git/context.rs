use git2::Repository;
use std::path::Path;

use crate::error::{Result, TuicrError};
use crate::model::{DiffLine, FileStatus, LineOrigin};

/// Fetch context lines from a file for gap expansion.
///
/// For Added/Modified files: reads from working tree
/// For Deleted files: reads from HEAD blob
pub fn fetch_context_lines(
    repo: &Repository,
    file_path: &Path,
    file_status: FileStatus,
    start_line: u32,
    end_line: u32,
) -> Result<Vec<DiffLine>> {
    if start_line > end_line || start_line == 0 {
        return Ok(Vec::new());
    }

    let content = match file_status {
        FileStatus::Deleted => {
            // Read from HEAD blob for deleted files
            fetch_blob_content(repo, file_path)?
        }
        _ => {
            // Read from working tree for all other statuses
            let workdir = repo.workdir().ok_or(TuicrError::NotARepository)?;
            let full_path = workdir.join(file_path);
            std::fs::read_to_string(&full_path)?
        }
    };

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

/// Fetch content from a git blob (for deleted files)
fn fetch_blob_content(repo: &Repository, file_path: &Path) -> Result<String> {
    let head = repo.head()?.peel_to_tree()?;
    let entry = head.get_path(file_path)?;
    let blob = repo.find_blob(entry.id())?;
    let content = std::str::from_utf8(blob.content())
        .map_err(|e| TuicrError::CorruptedSession(format!("Invalid UTF-8 in file: {}", e)))?;
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
