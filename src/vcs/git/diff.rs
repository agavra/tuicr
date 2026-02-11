use git2::{BranchType, Delta, Diff, DiffOptions, Oid, Repository};
use std::path::PathBuf;

use crate::error::{Result, TuicrError};
use crate::model::{DiffFile, DiffHunk, DiffLine, FileStatus, LineOrigin};
use crate::syntax::SyntaxHighlighter;
use crate::vcs::{PullRequestDiff, PullRequestInfo};

pub fn get_working_tree_diff(
    repo: &Repository,
    highlighter: &SyntaxHighlighter,
) -> Result<Vec<DiffFile>> {
    let head = repo.head()?.peel_to_tree()?;

    let mut opts = DiffOptions::new();
    opts.include_untracked(true);
    opts.show_untracked_content(true);
    opts.recurse_untracked_dirs(true);

    let diff = repo.diff_tree_to_workdir_with_index(Some(&head), Some(&mut opts))?;

    parse_diff(&diff, highlighter)
}

/// Get the diff for a range of commits.
/// `commit_ids` should be ordered from oldest to newest.
/// The diff compares the oldest commit's parent to the newest commit.
pub fn get_commit_range_diff(
    repo: &Repository,
    commit_ids: &[String],
    highlighter: &SyntaxHighlighter,
) -> Result<Vec<DiffFile>> {
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

    parse_diff(&diff, highlighter)
}

/// Get a combined diff from the parent of the oldest commit through to the working tree.
/// This shows both committed and uncommitted changes in a single diff.
pub fn get_working_tree_with_commits_diff(
    repo: &Repository,
    commit_ids: &[String],
    highlighter: &SyntaxHighlighter,
) -> Result<Vec<DiffFile>> {
    if commit_ids.is_empty() {
        return Err(TuicrError::NoChanges);
    }

    // Find the oldest commit (first in our list since commits are oldest to newest)
    let oldest_id = git2::Oid::from_str(&commit_ids[0])?;
    let oldest_commit = repo.find_commit(oldest_id)?;

    // Get the parent of the oldest commit, or use an empty tree if it's the initial commit
    let old_tree = if oldest_commit.parent_count() > 0 {
        Some(oldest_commit.parent(0)?.tree()?)
    } else {
        None
    };

    let mut opts = DiffOptions::new();
    opts.include_untracked(true);
    opts.show_untracked_content(true);
    opts.recurse_untracked_dirs(true);

    let diff = repo.diff_tree_to_workdir_with_index(old_tree.as_ref(), Some(&mut opts))?;

    parse_diff(&diff, highlighter)
}

/// Get a PR-style diff from merge-base(base_ref, HEAD) to HEAD.
pub fn get_pull_request_diff(
    repo: &Repository,
    base_ref: Option<&str>,
    highlighter: &SyntaxHighlighter,
) -> Result<PullRequestDiff> {
    let head_commit = repo.head()?.peel_to_commit()?;
    let head_oid = head_commit.id();
    let head_tree = head_commit.tree()?;

    let (resolved_base_ref, base_oid) = resolve_base_reference(repo, base_ref)?;
    let merge_base_oid = repo.merge_base(base_oid, head_oid).map_err(|_| {
        TuicrError::VcsCommand(format!(
            "Failed to find merge-base between {resolved_base_ref} and HEAD"
        ))
    })?;

    if merge_base_oid == head_oid {
        return Err(TuicrError::NoChanges);
    }

    let merge_base_commit = repo.find_commit(merge_base_oid)?;
    let merge_base_tree = merge_base_commit.tree()?;
    let diff = repo.diff_tree_to_tree(Some(&merge_base_tree), Some(&head_tree), None)?;
    let files = parse_diff(&diff, highlighter)?;

    let commit_count = count_commits_between(repo, merge_base_oid, head_oid)?;

    Ok(PullRequestDiff {
        files,
        info: PullRequestInfo {
            base_ref: resolved_base_ref,
            merge_base_commit: merge_base_oid.to_string(),
            head_commit: head_oid.to_string(),
            commit_count,
        },
    })
}

fn count_commits_between(repo: &Repository, merge_base_oid: Oid, head_oid: Oid) -> Result<usize> {
    let mut revwalk = repo.revwalk()?;
    revwalk.push(head_oid)?;
    revwalk.hide(merge_base_oid)?;

    let mut count = 0;
    for oid_result in revwalk {
        oid_result?;
        count += 1;
    }

    Ok(count)
}

fn resolve_base_reference(repo: &Repository, explicit_base: Option<&str>) -> Result<(String, Oid)> {
    if let Some(base_ref) = explicit_base {
        let oid = resolve_ref_to_oid(repo, base_ref).map_err(|_| {
            TuicrError::VcsCommand(format!("Could not resolve base reference '{base_ref}'"))
        })?;
        return Ok((base_ref.to_string(), oid));
    }

    if let Ok(head) = repo.head()
        && head.is_branch()
        && let Some(branch_name) = head.shorthand()
        && let Ok(branch) = repo.find_branch(branch_name, BranchType::Local)
        && let Ok(upstream) = branch.upstream()
        && let Some(upstream_ref_name) = upstream.get().name()
        && let Ok(oid) = resolve_ref_to_oid(repo, upstream_ref_name)
    {
        // Skip if the upstream is just the remote copy of the current branch
        // (e.g. origin/feat-x for branch feat-x), since it's not a useful PR base.
        let upstream_short = upstream.get().shorthand().unwrap_or(upstream_ref_name);
        let is_same_branch = upstream_short
            .split_once('/')
            .map(|(_, upstream_branch_name)| upstream_branch_name == branch_name)
            .unwrap_or(false);
        if !is_same_branch {
            return Ok((upstream_ref_name.to_string(), oid));
        }
    }

    if let Ok(origin_head) = repo.find_reference("refs/remotes/origin/HEAD")
        && let Some(target) = origin_head.symbolic_target()
        && let Ok(oid) = resolve_ref_to_oid(repo, target)
    {
        return Ok((target.to_string(), oid));
    }

    for candidate in ["origin/main", "origin/master", "main", "master"] {
        if let Ok(oid) = resolve_ref_to_oid(repo, candidate) {
            return Ok((candidate.to_string(), oid));
        }
    }

    Err(TuicrError::VcsCommand(
        "Could not determine PR base reference. Set an upstream branch or pass --base <ref>."
            .to_string(),
    ))
}

fn resolve_ref_to_oid(repo: &Repository, reference: &str) -> Result<Oid> {
    let object = repo.revparse_single(reference)?;
    let commit = object.peel_to_commit()?;
    Ok(commit.id())
}

fn parse_diff(diff: &Diff, highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
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

        // Use new_path for highlighting (the current version of the file)
        let file_path = new_path.as_ref().or(old_path.as_ref());

        let hunks = if is_binary {
            Vec::new()
        } else {
            parse_hunks(diff, delta_idx, file_path, highlighter)?
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

fn parse_hunks(
    diff: &Diff,
    delta_idx: usize,
    file_path: Option<&PathBuf>,
    highlighter: &SyntaxHighlighter,
) -> Result<Vec<DiffHunk>> {
    let mut hunks: Vec<DiffHunk> = Vec::new();

    let patch = git2::Patch::from_diff(diff, delta_idx)?;

    if let Some(patch) = patch {
        for hunk_idx in 0..patch.num_hunks() {
            let (hunk, _) = patch.hunk(hunk_idx)?;

            let header = String::from_utf8_lossy(hunk.header()).trim().to_string();
            let old_start = hunk.old_start();
            let old_count = hunk.old_lines();
            let new_start = hunk.new_start();
            let new_count = hunk.new_lines();

            let mut lines: Vec<DiffLine> = Vec::new();

            // First, collect all line content for syntax highlighting
            let mut line_contents: Vec<String> = Vec::new();
            let mut line_origins: Vec<LineOrigin> = Vec::new();

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
                    .replace('\t', "    ")
                    .to_string();

                line_contents.push(content);
                line_origins.push(origin);
            }

            // Apply syntax highlighting if we have a file path
            let highlight_sequences =
                SyntaxHighlighter::split_diff_lines_for_highlighting(&line_contents, &line_origins);
            let (old_highlighted_lines, new_highlighted_lines) = if let Some(path) = file_path {
                (
                    highlighter.highlight_file_lines(path, &highlight_sequences.old_lines),
                    highlighter.highlight_file_lines(path, &highlight_sequences.new_lines),
                )
            } else {
                (None, None)
            };

            // Now create DiffLines with syntax highlighting applied
            for line_idx in 0..patch.num_lines_in_hunk(hunk_idx)? {
                let line = patch.line_in_hunk(hunk_idx, line_idx)?;
                let old_lineno = line.old_lineno();
                let new_lineno = line.new_lineno();
                let content = line_contents[line_idx].clone();
                let origin = line_origins[line_idx];

                // Get highlighted spans and apply diff background
                let highlighted_spans = highlighter.highlighted_line_for_diff_with_background(
                    old_highlighted_lines.as_deref(),
                    new_highlighted_lines.as_deref(),
                    highlight_sequences.old_line_indices[line_idx],
                    highlight_sequences.new_line_indices[line_idx],
                    origin,
                );

                lines.push(DiffLine {
                    origin,
                    content,
                    old_lineno,
                    new_lineno,
                    highlighted_spans,
                });
            }

            hunks.push(DiffHunk {
                header,
                lines,
                old_start,
                old_count,
                new_start,
                new_count,
            });
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
        let highlighter = SyntaxHighlighter::default();

        // when
        let result = parse_diff(&diff, &highlighter);

        // then
        assert!(matches!(result, Err(TuicrError::NoChanges)));
    }
}
