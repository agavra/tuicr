use git2::{Delta, Diff, DiffOptions, Repository};
use std::path::PathBuf;

use crate::error::{Result, TuicrError};
use crate::model::{DiffFile, DiffHunk, DiffLine, FileStatus, LineOrigin};

pub fn get_working_tree_diff(repo: &Repository) -> Result<Vec<DiffFile>> {
    let head = repo.head()?.peel_to_tree()?;

    let mut opts = DiffOptions::new();
    opts.include_untracked(true);
    opts.show_untracked_content(true);
    opts.recurse_untracked_dirs(true);

    let diff = repo.diff_tree_to_workdir_with_index(Some(&head), Some(&mut opts))?;

    parse_diff(&diff)
}

/// Get the diff for a range of commits.
/// `commit_ids` should be ordered from oldest to newest.
/// The diff compares the oldest commit's parent to the newest commit.
pub fn get_commit_range_diff(repo: &Repository, commit_ids: &[String]) -> Result<Vec<DiffFile>> {
    if commit_ids.is_empty() {
        return Err(TuicrError::NoChanges);
    }

    // Find the oldest commit (last in our list since commits are oldest to newest)
    let oldest_id = git2::Oid::from_str(&commit_ids[0])?;
    let oldest_commit = repo.find_commit(oldest_id)?;

    // Find the newest commit (first in our list)
    let newest_id = git2::Oid::from_str(commit_ids.last().unwrap())?;
    let newest_commit = repo.find_commit(newest_id)?;

    // Get the parent of the oldest commit, or use an empty tree if it's the initial commit
    let old_tree = if oldest_commit.parent_count() > 0 {
        Some(oldest_commit.parent(0)?.tree()?)
    } else {
        None
    };

    let new_tree = newest_commit.tree()?;

    let diff = repo.diff_tree_to_tree(old_tree.as_ref(), Some(&new_tree), None)?;

    parse_diff(&diff)
}

fn parse_diff(diff: &Diff) -> Result<Vec<DiffFile>> {
    let mut files: Vec<DiffFile> = Vec::new();

    for (delta_idx, delta) in diff.deltas().enumerate() {
        let status = match delta.status() {
            Delta::Added | Delta::Untracked => FileStatus::Added,
            Delta::Deleted => FileStatus::Deleted,
            Delta::Modified => FileStatus::Modified,
            Delta::Renamed => FileStatus::Renamed,
            Delta::Copied => FileStatus::Copied,
            _ => FileStatus::Modified,
        };

        let old_path = delta.old_file().path().map(PathBuf::from);
        let new_path = delta.new_file().path().map(PathBuf::from);
        let is_binary = delta.old_file().is_binary() || delta.new_file().is_binary();

        let hunks = if is_binary {
            Vec::new()
        } else {
            parse_hunks(diff, delta_idx)?
        };

        files.push(DiffFile {
            old_path,
            new_path,
            status,
            hunks,
            is_binary,
        });
    }

    if files.is_empty() {
        return Err(TuicrError::NoChanges);
    }

    Ok(files)
}

fn parse_hunks(diff: &Diff, delta_idx: usize) -> Result<Vec<DiffHunk>> {
    let mut hunks: Vec<DiffHunk> = Vec::new();

    let patch = git2::Patch::from_diff(diff, delta_idx)?;

    if let Some(patch) = patch {
        for hunk_idx in 0..patch.num_hunks() {
            let (hunk, _) = patch.hunk(hunk_idx)?;

            let header = String::from_utf8_lossy(hunk.header()).trim().to_string();

            let mut lines: Vec<DiffLine> = Vec::new();

            for line_idx in 0..patch.num_lines_in_hunk(hunk_idx)? {
                let line = patch.line_in_hunk(hunk_idx, line_idx)?;

                let origin = match line.origin() {
                    '+' => LineOrigin::Addition,
                    '-' => LineOrigin::Deletion,
                    ' ' => LineOrigin::Context,
                    _ => LineOrigin::Context,
                };

                let content = String::from_utf8_lossy(line.content())
                    .trim_end_matches('\n')
                    .trim_end_matches('\r')
                    .to_string();

                let old_lineno = line.old_lineno();
                let new_lineno = line.new_lineno();

                lines.push(DiffLine {
                    origin,
                    content,
                    old_lineno,
                    new_lineno,
                });
            }

            hunks.push(DiffHunk { header, lines });
        }
    }

    Ok(hunks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_return_no_changes_for_clean_repo() {
        // given
        let repo = Repository::discover(".").unwrap();
        let head = repo.head().unwrap().peel_to_tree().unwrap();
        let diff = repo
            .diff_tree_to_tree(Some(&head), Some(&head), None)
            .unwrap();

        // when
        let result = parse_diff(&diff);

        // then
        assert!(matches!(result, Err(TuicrError::NoChanges)));
    }
}
