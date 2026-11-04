use git2::{Delta, Diff, DiffFindOptions, DiffOptions, Repository};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::error::{Result, TuicrError};
use crate::model::{DiffFile, DiffHunk, DiffLine, FileStatus, LineOrigin};
use crate::syntax::{SyntaxHighlighter, needs_full_file_highlight};
use crate::vcs::traits::{
    ChangeKind, DiffWhitespaceMode, ResolvedRevisionRange, RevisionDiffTarget,
};
use crate::vcs::whitespace::{WhitespaceComparison, materialize_diff};
use crate::vcs::{MAX_HIGHLIGHT_FILE_BYTES, enhance_with_full_file_highlight, tabify};

pub fn get_working_tree_diff(
    repo: &Repository,
    whitespace_mode: &DiffWhitespaceMode,
    include_untracked: bool,
    highlighter: &SyntaxHighlighter,
) -> Result<Vec<DiffFile>> {
    materialize_diff(whitespace_mode, |comparison| {
        get_working_tree_diff_once(repo, comparison, include_untracked, highlighter)
    })
}

fn get_working_tree_diff_once(
    repo: &Repository,
    comparison: WhitespaceComparison,
    include_untracked: bool,
    highlighter: &SyntaxHighlighter,
) -> Result<Vec<DiffFile>> {
    // Unborn HEAD (fresh `git init` / `git clone` of an empty remote) has no
    // tree to compare against; diff against an empty baseline so freshly
    // staged/added files still surface in the working-tree review.
    let head = repo.head().ok().and_then(|h| h.peel_to_tree().ok());

    let mut opts = diff_options(comparison);
    if include_untracked {
        opts.include_untracked(true);
        opts.show_untracked_content(true);
        opts.recurse_untracked_dirs(true);
    }

    let diff = repo.diff_tree_to_workdir_with_index(head.as_ref(), Some(&mut opts))?;
    let mut files = parse_diff(repo, diff)?;
    enhance_with_full_file_highlight(
        &mut files,
        highlighter,
        |path| {
            head.as_ref()
                .and_then(|tree| read_path_from_tree(repo, tree, path))
        },
        |path| read_path_from_workdir(repo, path),
    );
    Ok(files)
}

/// Get the staged diff (index vs HEAD)
/// On repos with no commits (unborn HEAD), diffs against an empty tree.
pub fn get_staged_diff(
    repo: &Repository,
    whitespace_mode: &DiffWhitespaceMode,
    highlighter: &SyntaxHighlighter,
) -> Result<Vec<DiffFile>> {
    materialize_diff(whitespace_mode, |comparison| {
        get_staged_diff_once(repo, comparison, highlighter)
    })
}

fn get_staged_diff_once(
    repo: &Repository,
    comparison: WhitespaceComparison,
    highlighter: &SyntaxHighlighter,
) -> Result<Vec<DiffFile>> {
    let head = repo.head().ok().and_then(|h| h.peel_to_tree().ok());
    let index = super::staging::fresh_index(repo)?;
    let mut opts = diff_options(comparison);

    let diff = repo.diff_tree_to_index(head.as_ref(), Some(&index), Some(&mut opts))?;
    let mut files = parse_diff(repo, diff)?;
    enhance_with_full_file_highlight(
        &mut files,
        highlighter,
        |path| {
            head.as_ref()
                .and_then(|tree| read_path_from_tree(repo, tree, path))
        },
        |path| read_path_from_index(repo, &index, path),
    );
    Ok(files)
}

/// Get the unstaged diff (working tree vs index)
/// List the changed paths for either the staged or unstaged diff, without
/// materializing hunks or running the syntax highlighter. Lets callers verify
/// `get_change_status` against ignore rules cheaply — full diff parsing only
/// happens once the user actually selects the staged/unstaged view.
pub fn list_changed_paths(
    repo: &Repository,
    kind: ChangeKind,
    include_untracked: bool,
) -> Result<Vec<PathBuf>> {
    let diff = match kind {
        ChangeKind::Staged => {
            let head = repo.head().ok().and_then(|h| h.peel_to_tree().ok());
            let index = super::staging::fresh_index(repo)?;
            let mut opts = diff_options(WhitespaceComparison::Normal);
            repo.diff_tree_to_index(head.as_ref(), Some(&index), Some(&mut opts))?
        }
        ChangeKind::Unstaged => {
            let index = super::staging::fresh_index(repo)?;
            let mut opts = diff_options(WhitespaceComparison::Normal);
            if include_untracked {
                opts.include_untracked(true);
                // `show_untracked_content(false)` keeps libgit2 from reading each
                // untracked file's bytes — only the paths are needed here.
                opts.show_untracked_content(false);
                opts.recurse_untracked_dirs(true);
            }
            opts.skip_binary_check(true);
            repo.diff_index_to_workdir(Some(&index), Some(&mut opts))?
        }
    };

    let mut paths: Vec<PathBuf> = Vec::with_capacity(diff.deltas().len());
    for delta in diff.deltas() {
        let path = delta
            .new_file()
            .path()
            .or_else(|| delta.old_file().path())
            .map(PathBuf::from);
        if let Some(p) = path {
            paths.push(p);
        }
    }
    Ok(paths)
}

pub fn get_unstaged_diff(
    repo: &Repository,
    whitespace_mode: &DiffWhitespaceMode,
    include_untracked: bool,
    highlighter: &SyntaxHighlighter,
) -> Result<Vec<DiffFile>> {
    materialize_diff(whitespace_mode, |comparison| {
        get_unstaged_diff_once(repo, comparison, include_untracked, highlighter)
    })
}

fn get_unstaged_diff_once(
    repo: &Repository,
    comparison: WhitespaceComparison,
    include_untracked: bool,
    highlighter: &SyntaxHighlighter,
) -> Result<Vec<DiffFile>> {
    let index = super::staging::fresh_index(repo)?;
    let mut opts = diff_options(comparison);
    opts.include_untracked(include_untracked);
    opts.show_untracked_content(include_untracked);
    opts.recurse_untracked_dirs(include_untracked);

    let diff = repo.diff_index_to_workdir(Some(&index), Some(&mut opts))?;
    let mut files = parse_diff(repo, diff)?;
    enhance_with_full_file_highlight(
        &mut files,
        highlighter,
        |path| read_path_from_index(repo, &index, path),
        |path| read_path_from_workdir(repo, path),
    );
    Ok(files)
}

/// Get the diff for a range of commits.
/// `commit_ids` should be ordered from oldest to newest.
/// The diff compares the oldest commit's parent to the newest commit.
pub fn get_commit_range_diff(
    repo: &Repository,
    revision_range: &ResolvedRevisionRange<'_>,
    whitespace_mode: &DiffWhitespaceMode,
    highlighter: &SyntaxHighlighter,
    line_counts: &RefCell<LineCounts>,
) -> Result<Vec<DiffFile>> {
    materialize_diff(whitespace_mode, |comparison| {
        get_commit_range_diff_once(repo, revision_range, comparison, highlighter, line_counts)
    })
}

fn get_commit_range_diff_once(
    repo: &Repository,
    revision_range: &ResolvedRevisionRange<'_>,
    comparison: WhitespaceComparison,
    highlighter: &SyntaxHighlighter,
    line_counts: &RefCell<LineCounts>,
) -> Result<Vec<DiffFile>> {
    let (old_tree, new_tree) = match &revision_range.diff_target {
        RevisionDiffTarget::CommitList => {
            commit_list_range_trees(repo, &revision_range.commit_ids)?
        }
        RevisionDiffTarget::Explicit { base, head } => {
            let old_tree = match base {
                Some(base) => Some(tree_for_commit(repo, base)?),
                None => None,
            };
            let new_tree = tree_for_commit(repo, head)?;
            (old_tree, new_tree)
        }
    };

    diff_commit_trees(
        repo,
        old_tree,
        new_tree,
        comparison,
        highlighter,
        line_counts,
    )
}

fn commit_list_range_trees<'repo>(
    repo: &'repo Repository,
    commit_ids: &[String],
) -> Result<(Option<git2::Tree<'repo>>, git2::Tree<'repo>)> {
    if commit_ids.is_empty() {
        return Err(TuicrError::NoChanges);
    }

    let oldest_id = git2::Oid::from_str(&commit_ids[0])?;
    let oldest_commit = repo.find_commit(oldest_id)?;

    let newest_id = git2::Oid::from_str(commit_ids.last().unwrap())?;
    let newest_commit = repo.find_commit(newest_id)?;

    let old_tree = if oldest_commit.parent_count() > 0 {
        Some(oldest_commit.parent(0)?.tree()?)
    } else {
        None
    };

    Ok((old_tree, newest_commit.tree()?))
}

// Explicit revision ranges carry commit IDs for old/new endpoints,
// but libgit2 diffs require the endpoint trees.
fn tree_for_commit<'repo>(repo: &'repo Repository, commit_id: &str) -> Result<git2::Tree<'repo>> {
    let oid = git2::Oid::from_str(commit_id)?;
    Ok(repo.find_commit(oid)?.tree()?)
}

fn diff_commit_trees(
    repo: &Repository,
    old_tree: Option<git2::Tree<'_>>,
    new_tree: git2::Tree<'_>,
    comparison: WhitespaceComparison,
    highlighter: &SyntaxHighlighter,
    line_counts: &RefCell<LineCounts>,
) -> Result<Vec<DiffFile>> {
    let mut opts = diff_options(comparison);

    let mut diff = repo.diff_tree_to_tree(old_tree.as_ref(), Some(&new_tree), Some(&mut opts))?;
    detect_renames(repo, &mut diff)?;
    let TreeDiff {
        mut files,
        mut old_contents,
        mut new_contents,
        counts,
    } = parse_tree_diff(repo, &diff, comparison)?;
    line_counts
        .borrow_mut()
        .extend(counts.into_iter().map(|(p, n)| ((new_tree.id(), p), n)));
    enhance_with_full_file_highlight(
        &mut files,
        highlighter,
        |path| {
            old_contents.remove(path).or_else(|| {
                old_tree
                    .as_ref()
                    .and_then(|tree| read_path_from_tree(repo, tree, path))
            })
        },
        |path| {
            new_contents
                .remove(path)
                .or_else(|| read_path_from_tree(repo, &new_tree, path))
        },
    );
    Ok(files)
}

/// Get a combined diff from the parent of the oldest commit through to the working tree.
/// This shows both committed and working tree changes in a single diff.
pub fn get_working_tree_with_commits_diff(
    repo: &Repository,
    commit_ids: &[String],
    whitespace_mode: &DiffWhitespaceMode,
    include_untracked: bool,
    highlighter: &SyntaxHighlighter,
) -> Result<Vec<DiffFile>> {
    materialize_diff(whitespace_mode, |comparison| {
        get_working_tree_with_commits_diff_once(
            repo,
            commit_ids,
            comparison,
            include_untracked,
            highlighter,
        )
    })
}

fn get_working_tree_with_commits_diff_once(
    repo: &Repository,
    commit_ids: &[String],
    comparison: WhitespaceComparison,
    include_untracked: bool,
    highlighter: &SyntaxHighlighter,
) -> Result<Vec<DiffFile>> {
    if commit_ids.is_empty() {
        // An empty revision range still has a meaningful working-tree diff.
        return get_working_tree_diff_once(repo, comparison, include_untracked, highlighter);
    }

    let oldest_id = git2::Oid::from_str(&commit_ids[0])?;
    let oldest_commit = repo.find_commit(oldest_id)?;

    let old_tree = if oldest_commit.parent_count() > 0 {
        Some(oldest_commit.parent(0)?.tree()?)
    } else {
        None
    };

    let mut opts = diff_options(comparison);
    opts.include_untracked(include_untracked);
    opts.show_untracked_content(include_untracked);
    opts.recurse_untracked_dirs(include_untracked);

    let diff = repo.diff_tree_to_workdir_with_index(old_tree.as_ref(), Some(&mut opts))?;
    let mut files = parse_diff(repo, diff)?;
    enhance_with_full_file_highlight(
        &mut files,
        highlighter,
        |path| {
            old_tree
                .as_ref()
                .and_then(|tree| read_path_from_tree(repo, tree, path))
        },
        |path| read_path_from_workdir(repo, path),
    );
    Ok(files)
}

fn diff_options(comparison: WhitespaceComparison) -> DiffOptions {
    let mut opts = DiffOptions::new();
    opts.patience(true).indent_heuristic(true);
    opts.ignore_whitespace(comparison.ignores_all());
    opts
}

fn read_path_from_tree(repo: &Repository, tree: &git2::Tree, path: &Path) -> Option<String> {
    let entry = tree.get_path(path).ok()?;
    let blob = repo.find_blob(entry.id()).ok()?;
    Some(String::from_utf8_lossy(blob.content()).into_owned())
}

fn read_path_from_workdir(repo: &Repository, path: &Path) -> Option<String> {
    crate::vcs::read_workdir_file(repo.workdir()?, path)
}

fn read_path_from_index(repo: &Repository, index: &git2::Index, path: &Path) -> Option<String> {
    let entry = index.get_path(path, 0)?;
    let blob = repo.find_blob(entry.id).ok()?;
    Some(String::from_utf8_lossy(blob.content()).into_owned())
}

/// Pair deleted and added files as renames, following `diff.renames`.
fn detect_renames(repo: &Repository, diff: &mut Diff<'_>) -> Result<()> {
    let config = repo.config()?;
    let copies = config.get_string("diff.renames").is_ok_and(|value| {
        value.eq_ignore_ascii_case("copies") || value.eq_ignore_ascii_case("copy")
    });
    let renames = copies || config.get_bool("diff.renames").unwrap_or(true);
    if renames {
        let mut find = DiffFindOptions::new();
        // Explicit flags bypass libgit2's diff.renames lookup. Preserve that policy
        // while including the untracked files already shown in working-tree reviews.
        find.renames(true).copies(copies).for_untracked(true);
        diff.find_similar(Some(&mut find))?;
    }
    Ok(())
}

fn parse_diff(repo: &Repository, mut diff: Diff<'_>) -> Result<Vec<DiffFile>> {
    detect_renames(repo, &mut diff)?;

    let mut files: Vec<DiffFile> = Vec::new();

    // Untracked files larger than this are shown in the file list but their
    // content is not parsed — they are likely logs, dumps, or build artefacts.
    const MAX_UNTRACKED_FILE_SIZE: u64 = 10 * 1_024 * 1_024;

    for (delta_idx, delta) in diff.deltas().enumerate() {
        let mut is_binary = delta_is_binary(&delta);
        let is_too_large =
            delta.status() == Delta::Untracked && delta.new_file().size() > MAX_UNTRACKED_FILE_SIZE;

        let hunks = if is_binary || is_too_large {
            Vec::new()
        } else {
            match parse_hunks(&diff, delta_idx)? {
                Some(hunks) => hunks,
                None => {
                    is_binary = true;
                    Vec::new()
                }
            }
        };

        let mut file = diff_file_from_delta(&delta, hunks);
        file.is_binary = is_binary;
        file.is_too_large = is_too_large;
        files.push(file);
    }

    if files.is_empty() {
        return Err(TuicrError::NoChanges);
    }

    Ok(files)
}

/// New-version line counts, keyed by the tree that holds the file. A tree's
/// content never changes, so an entry stays correct.
pub(crate) type LineCounts = HashMap<(git2::Oid, PathBuf), u32>;

/// Build the `DiffFile` for a delta, with `is_binary` and `is_too_large` false.
fn diff_file_from_delta(delta: &git2::DiffDelta<'_>, hunks: Vec<DiffHunk>) -> DiffFile {
    let status = match delta.status() {
        Delta::Added | Delta::Untracked => FileStatus::Added,
        Delta::Deleted => FileStatus::Deleted,
        Delta::Modified => FileStatus::Modified,
        Delta::Renamed => FileStatus::Renamed,
        Delta::Copied => FileStatus::Copied,
        _ => FileStatus::Modified,
    };
    let content_hash = DiffFile::compute_content_hash(&hunks);
    DiffFile {
        old_path: delta.old_file().path().map(PathBuf::from),
        new_path: delta.new_file().path().map(PathBuf::from),
        status,
        hunks,
        is_binary: false,
        is_too_large: false,
        is_commit_message: false,
        content_hash,
    }
}

/// Holds the parsed files, plus the contents and line counts that coloring and
/// `file_line_count` would otherwise read from git again.
struct TreeDiff {
    files: Vec<DiffFile>,
    old_contents: HashMap<PathBuf, String>,
    new_contents: HashMap<PathBuf, String>,
    counts: Vec<(PathBuf, u32)>,
}

/// Parse a diff between two trees, reading each file version once.
///
/// `Patch::from_diff` does not return the blobs it loads, so line counting and
/// whole-file coloring have to read them again. This function reads the blobs,
/// builds the patch from them, and reuses the bytes for those two steps.
fn parse_tree_diff(
    repo: &Repository,
    diff: &Diff,
    comparison: WhitespaceComparison,
) -> Result<TreeDiff> {
    let mut parsed = TreeDiff {
        files: Vec::new(),
        old_contents: HashMap::new(),
        new_contents: HashMap::new(),
        counts: Vec::new(),
    };

    for (delta_idx, delta) in diff.deltas().enumerate() {
        let old_path = delta.old_file().path().map(PathBuf::from);
        let new_path = delta.new_file().path().map(PathBuf::from);
        let mut is_binary = delta_is_binary(&delta);
        let path = new_path.as_ref().or(old_path.as_ref());

        let hunks = if is_binary {
            Vec::new()
        } else {
            let old_blob = read_delta_blob(repo, delta.old_file().id(), MAX_DELTA_BLOB_BYTES);
            let new_blob = read_delta_blob(repo, delta.new_file().id(), MAX_DELTA_BLOB_BYTES);
            let patch = match (&old_blob, &new_blob, path) {
                (Ok(old), Ok(new), Some(path)) => {
                    patch_from_blobs(repo, &delta, path, old.as_ref(), new.as_ref(), comparison)?
                }
                _ => None,
            };
            let hunks = match patch {
                Some(patch) => binary_checked_hunks(&patch)?,
                None => parse_hunks(diff, delta_idx)?,
            };
            let Some(hunks) = hunks else {
                is_binary = true;
                parsed.files.push(DiffFile {
                    is_binary,
                    ..diff_file_from_delta(&delta, Vec::new())
                });
                continue;
            };

            // Coloring skips files without hunks and files over its size cap, so
            // this function does not store those contents.
            let wants_contents =
                !hunks.is_empty() && path.is_some_and(|p| needs_full_file_highlight(p));
            let fits = |blob: &git2::Blob<'_>| blob.size() <= MAX_HIGHLIGHT_FILE_BYTES;
            if let (Some(p), Ok(Some(blob))) = (&old_path, &old_blob)
                && wants_contents
                && fits(blob)
            {
                parsed.old_contents.insert(
                    p.clone(),
                    String::from_utf8_lossy(blob.content()).into_owned(),
                );
            }
            if let (Some(p), Ok(Some(blob))) = (&new_path, &new_blob) {
                if wants_contents && fits(blob) {
                    parsed.new_contents.insert(
                        p.clone(),
                        String::from_utf8_lossy(blob.content()).into_owned(),
                    );
                }
                // Skip invalid UTF-8, because `file_line_count` returns an error for it.
                if delta.status() != Delta::Deleted
                    && !hunks.is_empty()
                    && let Ok(text) = std::str::from_utf8(blob.content())
                {
                    parsed.counts.push((p.clone(), text.lines().count() as u32));
                }
            }
            hunks
        };

        let mut file = diff_file_from_delta(&delta, hunks);
        file.is_binary = is_binary;
        parsed.files.push(file);
    }

    if parsed.files.is_empty() {
        return Err(TuicrError::NoChanges);
    }
    Ok(parsed)
}

/// libgit2's default diff size limit. `Patch::from_diff` treats a larger file
/// as binary without decompressing it, and `Patch::from_buffers` has no limit.
const MAX_DELTA_BLOB_BYTES: usize = 512 * 1024 * 1024;

/// Read one side of a delta. Return `Ok(None)` for a zero ID, because that side
/// does not exist. Return an error for an ID that is not a blob, such as a
/// submodule's commit, and for a blob over `max_bytes`, without decompressing it.
fn read_delta_blob(
    repo: &Repository,
    id: git2::Oid,
    max_bytes: usize,
) -> std::result::Result<Option<git2::Blob<'_>>, git2::Error> {
    if id.is_zero() {
        return Ok(None);
    }
    let (size, _) = repo.odb()?.read_header(id)?;
    if size > max_bytes {
        return Err(git2::Error::from_str("blob exceeds the diff size limit"));
    }
    repo.find_blob(id).map(Some)
}

/// Build a patch from blobs already in memory. `Patch::from_buffers` does not
/// look up the `diff` attribute, so this function looks it up and applies it.
/// Return `None` for a named diff driver, because a driver can change hunk
/// headers. The caller then uses `from_diff`.
fn patch_from_blobs<'a>(
    repo: &Repository,
    delta: &git2::DiffDelta<'_>,
    path: &Path,
    old: Option<&'a git2::Blob<'_>>,
    new: Option<&'a git2::Blob<'_>>,
    comparison: WhitespaceComparison,
) -> Result<Option<git2::Patch<'a>>> {
    let mut opts = diff_options(comparison);
    match git2::AttrValue::from_bytes(repo.get_attr_bytes(
        path,
        "diff",
        git2::AttrCheckFlags::default(),
    )?) {
        git2::AttrValue::True => {
            opts.force_text(true);
        }
        git2::AttrValue::False => {
            opts.force_binary(true);
        }
        git2::AttrValue::Unspecified => {}
        git2::AttrValue::String(_) | git2::AttrValue::Bytes(_) => return Ok(None),
    }
    let patch = git2::Patch::from_buffers(
        old.map_or(&[][..], |blob| blob.content()),
        delta.old_file().path(),
        new.map_or(&[][..], |blob| blob.content()),
        delta.new_file().path(),
        Some(&mut opts),
    )?;
    Ok(Some(patch))
}

fn delta_is_binary(delta: &git2::DiffDelta<'_>) -> bool {
    delta.flags().is_binary() || delta.old_file().is_binary() || delta.new_file().is_binary()
}

/// Return `None` for a binary file. A modified delta gets its binary flag only
/// once the patch loads the file contents, so this checks the patch's delta.
fn binary_checked_hunks(patch: &git2::Patch<'_>) -> Result<Option<Vec<DiffHunk>>> {
    match delta_is_binary(&patch.delta()) {
        true => Ok(None),
        false => hunks_from_patch(patch).map(Some),
    }
}

fn parse_hunks(diff: &Diff, delta_idx: usize) -> Result<Option<Vec<DiffHunk>>> {
    match git2::Patch::from_diff(diff, delta_idx)? {
        Some(patch) => binary_checked_hunks(&patch),
        None => Ok(Some(Vec::new())),
    }
}

fn hunks_from_patch(patch: &git2::Patch<'_>) -> Result<Vec<DiffHunk>> {
    let mut hunks: Vec<DiffHunk> = Vec::new();

    for hunk_idx in 0..patch.num_hunks() {
        let (hunk, _) = patch.hunk(hunk_idx)?;

        let header = String::from_utf8_lossy(hunk.header()).trim().to_string();
        let old_start = hunk.old_start();
        let old_count = hunk.old_lines();
        let new_start = hunk.new_start();
        let new_count = hunk.new_lines();

        let mut line_contents: Vec<String> = Vec::new();
        let mut line_origins: Vec<LineOrigin> = Vec::new();
        let mut line_numbers: Vec<(Option<u32>, Option<u32>)> = Vec::new();

        for line_idx in 0..patch.num_lines_in_hunk(hunk_idx)? {
            let line = patch.line_in_hunk(hunk_idx, line_idx)?;

            let origin = match line.origin() {
                '+' => LineOrigin::Addition,
                '-' => LineOrigin::Deletion,
                ' ' => LineOrigin::Context,
                // EOF-newline markers ('=', '>', '<'); the CLI parser skips these too.
                _ => continue,
            };

            let raw = String::from_utf8_lossy(line.content());
            let content = raw.trim_end_matches(['\n', '\r']).to_string();

            line_contents.push(content);
            line_origins.push(origin);
            line_numbers.push((line.old_lineno(), line.new_lineno()));
        }

        let mut lines: Vec<DiffLine> = Vec::with_capacity(line_contents.len());
        for (idx, content) in line_contents.into_iter().enumerate() {
            let origin = line_origins[idx];
            let (old_lineno, new_lineno) = line_numbers[idx];

            lines.push(DiffLine {
                origin,
                raw_content: content.contains('\t').then(|| content.clone()),
                content: tabify(&content),
                old_lineno,
                new_lineno,
                highlighted_spans: None,
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

    Ok(hunks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    use crate::vcs::git::{GitCliBackend, Libgit2Backend};
    use crate::vcs::traits::VcsBackend;

    fn create_initial_commit(repo: &Repository, file_name: &str, content: &str) {
        fs::write(repo.workdir().unwrap().join(file_name), content)
            .expect("failed to write initial file");

        let mut index = repo.index().expect("failed to open index");
        index
            .add_path(Path::new(file_name))
            .expect("failed to add file to index");
        index.write().expect("failed to write index");

        let tree_id = index.write_tree().expect("failed to write tree");
        let tree = repo.find_tree(tree_id).expect("failed to find tree");
        let sig = git2::Signature::now("Test User", "test@example.com")
            .expect("failed to create signature");

        repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
            .expect("failed to create commit");
    }

    fn alignment_diff(old: &str, new: &str) -> DiffFile {
        assert_ne!(old, new, "fixture must contain a change");
        let temp_dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(temp_dir.path()).unwrap();
        create_initial_commit(&repo, "fixture.rs", old);
        fs::write(temp_dir.path().join("fixture.rs"), new).unwrap();
        let mut files = get_working_tree_diff(
            &repo,
            &DiffWhitespaceMode::Normal,
            true,
            &SyntaxHighlighter::default(),
        )
        .unwrap();
        assert_eq!(files.len(), 1);
        let file = files.remove(0);
        // Hunk alignment must preserve source coordinates used by comment anchors.
        let old_lines: Vec<_> = old.lines().collect();
        let new_lines: Vec<_> = new.lines().collect();
        for line in file.hunks.iter().flat_map(|hunk| &hunk.lines) {
            if let Some(number) = line.old_lineno {
                assert_eq!(line.content, old_lines[number as usize - 1]);
            }
            if let Some(number) = line.new_lineno {
                assert_eq!(line.content, new_lines[number as usize - 1]);
            }
        }
        file
    }

    fn changed_lines(file: &DiffFile, origin: LineOrigin) -> Vec<&str> {
        file.hunks
            .iter()
            .flat_map(|hunk| &hunk.lines)
            .filter(|line| line.origin == origin)
            .map(|line| line.content.as_str())
            .collect()
    }

    #[test]
    fn reordered_functions_keep_their_bodies_together() {
        let old = include_str!("../../../tests/fixtures/diff_alignment/reordered_functions.old.rs");
        let new = include_str!("../../../tests/fixtures/diff_alignment/reordered_functions.new.rs");
        let file = alignment_diff(old, new);
        let moved: Vec<_> = old.lines().take(6).collect();
        assert_eq!(changed_lines(&file, LineOrigin::Deletion), moved);
        assert_eq!(
            changed_lines(&file, LineOrigin::Addition),
            [
                "",
                "fn first() {",
                "    shared();",
                "    shared();",
                "    shared();",
                "}"
            ]
        );
        assert!(
            file.hunks.iter().flat_map(|hunk| &hunk.lines).any(|line| {
                line.origin == LineOrigin::Context && line.content == "fn second() {"
            })
        );
    }

    #[test]
    fn inserted_and_removed_blocks_keep_their_braces_together() {
        let old = include_str!("../../../tests/fixtures/diff_alignment/inserted_block.old.rs");
        let new = include_str!("../../../tests/fixtures/diff_alignment/inserted_block.new.rs");
        let block = [
            "    if ready {",
            "        extra();",
            "        work();",
            "    }",
        ];
        for (before, after, origin, absent) in [
            (old, new, LineOrigin::Addition, LineOrigin::Deletion),
            (new, old, LineOrigin::Deletion, LineOrigin::Addition),
        ] {
            let file = alignment_diff(before, after);
            assert_eq!(changed_lines(&file, origin), block);
            assert!(changed_lines(&file, absent).is_empty());
        }
    }

    #[test]
    fn alignment_keeps_reindentation_and_edits_visible() {
        let old = "fn run() {\n    work();\n}\n";
        let wrapped = "fn run() {\n    if ready {\n        work();\n    }\n}\n";
        for (before, after) in [(old, wrapped), (wrapped, old)] {
            let file = alignment_diff(before, after);
            assert!(!changed_lines(&file, LineOrigin::Addition).is_empty());
            assert!(!changed_lines(&file, LineOrigin::Deletion).is_empty());
        }
        let old = include_str!("../../../tests/fixtures/diff_alignment/reordered_functions.old.rs");
        let moved_and_edited =
            include_str!("../../../tests/fixtures/diff_alignment/reordered_functions.new.rs")
                .replace(
                    "fn first() {\n    shared();",
                    "fn first() {\n    changed();",
                );
        let file = alignment_diff(old, &moved_and_edited);
        assert!(changed_lines(&file, LineOrigin::Addition).contains(&"    changed();"));
        assert!(!changed_lines(&file, LineOrigin::Deletion).is_empty());
    }

    #[test]
    fn should_return_no_changes_for_clean_repo() {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let repo = Repository::init(temp_dir.path()).expect("failed to init repo");
        create_initial_commit(&repo, "file.txt", "content\n");
        let head = repo.head().unwrap().peel_to_tree().unwrap();
        let diff = repo
            .diff_tree_to_tree(Some(&head), Some(&head), None)
            .unwrap();

        let result = parse_diff(&repo, diff);

        assert!(matches!(result, Err(TuicrError::NoChanges)));
    }

    #[test]
    fn empty_commit_range_still_returns_working_tree_changes() {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let repo = Repository::init(temp_dir.path()).expect("failed to init repo");
        create_initial_commit(&repo, "fixture.txt", "base\n");
        fs::write(
            temp_dir.path().join("fixture.txt"),
            "synthetic working-tree edit\n",
        )
        .expect("failed to update synthetic fixture");
        let range = crate::vcs::git::repository::resolve_revision_range(&repo, "HEAD..HEAD")
            .expect("empty synthetic commit range should resolve");
        assert!(range.commit_ids.is_empty());

        let files = get_working_tree_with_commits_diff(
            &repo,
            &range.commit_ids,
            &DiffWhitespaceMode::Normal,
            true,
            &SyntaxHighlighter::default(),
        )
        .expect("an empty commit range must still show working-tree changes");

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].display_path(), Path::new("fixture.txt"));
        assert!(
            files[0].hunks[0]
                .lines
                .iter()
                .any(|line| line.content == "synthetic working-tree edit")
        );
    }

    #[test]
    fn should_expand_tabs_to_spaces_in_git_hunks() {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let repo = Repository::init(temp_dir.path()).expect("failed to init repo");

        create_initial_commit(
            &repo, "file.txt", r#"old
"#,
        );

        fs::write(
            temp_dir.path().join("file.txt"),
            r#"	new
"#,
        )
        .expect("failed to update file");

        let files = get_working_tree_diff(
            &repo,
            &DiffWhitespaceMode::Normal,
            true,
            &SyntaxHighlighter::default(),
        )
        .expect("failed to get diff");

        assert_eq!(files.len(), 1);
        let lines = &files[0].hunks[0].lines;

        assert!(
            lines.iter().any(|l| l.content == "    new"),
            "expected tab-expanded content in git diff lines"
        );
        assert!(lines.iter().all(|l| !l.content.contains('\t')));
        assert!(
            lines
                .iter()
                .any(|l| l.raw_content.as_deref() == Some("\tnew"))
        );
    }

    #[test]
    fn should_highlight_vue_script_hunk_using_full_file_context() {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let repo = Repository::init(temp_dir.path()).expect("failed to init repo");

        let initial = "<template>\n  <div>{{ msg }}</div>\n</template>\n\n<script setup>\nimport { ref } from 'vue'\nconst msg = ref('hi')\nconst other = 1\n</script>\n";
        create_initial_commit(&repo, "App.vue", initial);

        let edited = "<template>\n  <div>{{ msg }}</div>\n</template>\n\n<script setup>\nimport { ref } from 'vue'\nconst msg = ref('hello')\nconst other = 1\n</script>\n";
        fs::write(temp_dir.path().join("App.vue"), edited).expect("failed to update file");

        let files = get_working_tree_diff(
            &repo,
            &DiffWhitespaceMode::Normal,
            true,
            &SyntaxHighlighter::default(),
        )
        .expect("failed to get diff");
        assert_eq!(files.len(), 1);

        let changed_lines: Vec<_> = files[0].hunks[0]
            .lines
            .iter()
            .filter(|l| matches!(l.origin, LineOrigin::Addition | LineOrigin::Deletion))
            .collect();
        assert!(!changed_lines.is_empty(), "expected change lines in hunk");

        for line in changed_lines {
            let spans = line
                .highlighted_spans
                .as_ref()
                .unwrap_or_else(|| panic!("vue line should be highlighted: {line:?}"));
            let unique_fgs: std::collections::HashSet<_> =
                spans.iter().filter_map(|(s, _)| s.fg).collect();
            assert!(
                unique_fgs.len() >= 2,
                "vue hunk line {line:?} should have varied fg colors, got {unique_fgs:?}"
            );
        }
    }

    #[test]
    fn should_separate_staged_and_unstaged_diffs() {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let repo = Repository::init(temp_dir.path()).expect("failed to init repo");

        create_initial_commit(&repo, "file.txt", "base\n");

        fs::write(temp_dir.path().join("file.txt"), "unstaged\n").expect("failed to update file");

        let highlighter = SyntaxHighlighter::default();

        let unstaged = get_unstaged_diff(&repo, &DiffWhitespaceMode::Normal, true, &highlighter)
            .expect("unstaged diff failed");
        assert_eq!(unstaged.len(), 1);
        assert!(matches!(
            get_staged_diff(&repo, &DiffWhitespaceMode::Normal, &highlighter),
            Err(TuicrError::NoChanges)
        ));

        let mut index = repo.index().expect("failed to open index");
        index
            .add_path(Path::new("file.txt"))
            .expect("failed to add file to index");
        index.write().expect("failed to write index");

        let staged = get_staged_diff(&repo, &DiffWhitespaceMode::Normal, &highlighter)
            .expect("staged diff failed");
        assert_eq!(staged.len(), 1);
        assert!(matches!(
            get_unstaged_diff(&repo, &DiffWhitespaceMode::Normal, true, &highlighter),
            Err(TuicrError::NoChanges)
        ));
    }

    #[test]
    fn should_exclude_untracked_files_from_unstaged_diff_when_disabled() {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let repo = Repository::init(temp_dir.path()).expect("failed to init repo");

        create_initial_commit(&repo, "file.txt", "base\n");
        fs::write(temp_dir.path().join("file.txt"), "changed\n")
            .expect("failed to update tracked file");
        fs::write(temp_dir.path().join("untracked.txt"), "new\n")
            .expect("failed to write untracked file");

        let highlighter = SyntaxHighlighter::default();

        // Untracked files are excluded when include_untracked is false; the
        // tracked modification still surfaces.
        let unstaged = get_unstaged_diff(&repo, &DiffWhitespaceMode::Normal, false, &highlighter)
            .expect("unstaged diff failed");
        assert_eq!(unstaged.len(), 1);
        assert_eq!(unstaged[0].display_path(), Path::new("file.txt"));

        // The default keeps surfacing untracked files as additions.
        let unstaged = get_unstaged_diff(&repo, &DiffWhitespaceMode::Normal, true, &highlighter)
            .expect("unstaged diff failed");
        assert_eq!(unstaged.len(), 2);
    }

    #[test]
    fn should_report_no_changes_for_untracked_only_repo_when_untracked_disabled() {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let repo = Repository::init(temp_dir.path()).expect("failed to init repo");

        create_initial_commit(&repo, "file.txt", "base\n");
        fs::write(temp_dir.path().join("untracked.txt"), "new\n")
            .expect("failed to write untracked file");

        let highlighter = SyntaxHighlighter::default();

        assert!(matches!(
            get_unstaged_diff(&repo, &DiffWhitespaceMode::Normal, false, &highlighter),
            Err(TuicrError::NoChanges)
        ));
        assert!(matches!(
            get_working_tree_diff(&repo, &DiffWhitespaceMode::Normal, false, &highlighter),
            Err(TuicrError::NoChanges)
        ));
    }

    #[test]
    fn should_list_changed_paths_without_untracked_when_disabled() {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let repo = Repository::init(temp_dir.path()).expect("failed to init repo");

        create_initial_commit(&repo, "file.txt", "base\n");
        fs::write(temp_dir.path().join("file.txt"), "changed\n")
            .expect("failed to update tracked file");
        fs::write(temp_dir.path().join("untracked.txt"), "new\n")
            .expect("failed to write untracked file");

        let paths = list_changed_paths(&repo, ChangeKind::Unstaged, false)
            .expect("failed to list changed paths");
        assert_eq!(paths, vec![PathBuf::from("file.txt")]);

        let paths = list_changed_paths(&repo, ChangeKind::Unstaged, true)
            .expect("failed to list changed paths");
        assert_eq!(paths.len(), 2);
    }

    #[test]
    fn should_surface_staged_files_on_unborn_head() {
        // given a freshly initialised repo with a staged file but no commits
        // (the "naked clone" / `git init` state — HEAD is unborn)
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let repo = Repository::init(temp_dir.path()).expect("failed to init repo");
        fs::write(temp_dir.path().join("file.txt"), "hello\n").expect("write file");
        let mut index = repo.index().expect("open index");
        index.add_path(Path::new("file.txt")).expect("stage file");
        index.write().expect("write index");

        // when
        let files = get_working_tree_diff(
            &repo,
            &DiffWhitespaceMode::Normal,
            true,
            &SyntaxHighlighter::default(),
        )
        .expect("unborn HEAD should produce a diff against an empty tree");

        // then the staged file shows up as an addition rather than crashing
        // with `reference 'refs/heads/main' not found`
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].display_path(), Path::new("file.txt"));
        assert!(matches!(files[0].status, FileStatus::Added));
    }

    #[test]
    fn should_surface_noop_file_when_whitespace_only_diff_is_empty() {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let repo = Repository::init(temp_dir.path()).expect("failed to init repo");

        create_initial_commit(&repo, "file.txt", "alpha\nbeta\n");
        fs::write(temp_dir.path().join("file.txt"), " alpha \n beta\n")
            .expect("failed to update file");

        let files = get_working_tree_diff(
            &repo,
            &DiffWhitespaceMode::IgnoreAll,
            true,
            &SyntaxHighlighter::default(),
        )
        .expect("whitespace-only edit may surface as a no-op diff file");
        assert_eq!(files.len(), 1);
        assert!(files[0].hunks.is_empty());

        fs::write(temp_dir.path().join("file.txt"), " alpha \ngamma\n")
            .expect("failed to update file");

        let files = get_working_tree_diff(
            &repo,
            &DiffWhitespaceMode::IgnoreAll,
            true,
            &SyntaxHighlighter::default(),
        )
        .expect("non-whitespace edit should still produce a diff");
        assert_eq!(files.len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn should_keep_mode_only_changes_when_ignoring_whitespace() {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let repo = Repository::init(temp_dir.path()).expect("failed to init repo");
        create_initial_commit(&repo, "file.txt", "alpha\n");

        let path = temp_dir.path().join("file.txt");
        let mut permissions = fs::metadata(&path)
            .expect("failed to stat file")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).expect("failed to update mode");

        let files = get_working_tree_diff(
            &repo,
            &DiffWhitespaceMode::IgnoreAll,
            true,
            &SyntaxHighlighter::default(),
        )
        .expect("mode-only edit should still produce a diff");
        assert_eq!(files.len(), 1);
        assert!(files[0].hunks.is_empty());
    }

    /// Temporary git repo for the plain against highlighted test below.
    /// `create_initial_commit` above adds one commit to an already-open
    /// `Repository`. This builds the whole repo instead, so both the libgit2 and
    /// Git CLI backends can open it from its path.
    struct TempRepo {
        dir: tempfile::TempDir,
    }

    impl TempRepo {
        /// Creates a git repo with each `(path, content)` pair committed once
        /// with empty content, then rewritten to `content` as an unstaged
        /// change. Starts from a real commit rather than an unborn HEAD, which
        /// keeps both backends on their ordinary "diff against HEAD" path.
        fn with_changes(files: &[(&str, &str)]) -> Self {
            let dir = tempfile::tempdir().expect("failed to create temp dir");
            run_git(dir.path(), &["init"]);
            run_git(dir.path(), &["config", "user.name", "Tuicr Test"]);
            run_git(dir.path(), &["config", "user.email", "tuicr@example.com"]);
            for (path, _) in files {
                fs::write(dir.path().join(path), "")
                    .unwrap_or_else(|e| panic!("failed to write {path}: {e}"));
            }
            run_git(dir.path(), &["add", "-A"]);
            run_git(dir.path(), &["commit", "-m", "base"]);
            for (path, content) in files {
                fs::write(dir.path().join(path), content)
                    .unwrap_or_else(|e| panic!("failed to update {path}: {e}"));
            }
            Self { dir }
        }

        fn path(&self) -> &Path {
            self.dir.path()
        }
    }

    fn run_git(dir: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .args([
                "-c",
                "commit.gpgsign=false",
                "-c",
                "init.defaultRefFormat=files",
            ])
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap_or_else(|e| panic!("failed to run git {args:?}: {e}"));
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// Compares two parses of the same working tree on the fields the diff-watch
    /// gate (`App::fetch_changed_diff_files`) keys on: path, status, binary and
    /// too-large flags, and content hash. Sorted so the comparison does not depend
    /// on backend return order.
    fn assert_files_match(highlighted: &[DiffFile], plain: &[DiffFile], context: &str) {
        // Two empty parses match, so the comparison below passes if the fixture
        // stops producing a diff.
        assert_eq!(
            highlighted.len(),
            2,
            "{context}: fixture should produce both changed files"
        );

        assert_eq!(
            highlighted.len(),
            plain.len(),
            "{context}: file count differs between highlighted and plain parses"
        );

        let key = |f: &DiffFile| {
            (
                f.display_path().clone(),
                f.status.as_char(),
                f.is_binary,
                f.is_too_large,
                f.content_hash,
            )
        };
        let mut highlighted_keys: Vec<_> = highlighted.iter().map(key).collect();
        let mut plain_keys: Vec<_> = plain.iter().map(key).collect();
        highlighted_keys.sort();
        plain_keys.sort();

        assert_eq!(
            highlighted_keys, plain_keys,
            "{context}: plain and highlighted parses must match"
        );
    }

    /// The gate in `App::fetch_changed_diff_files` (`src/app/diff_load.rs`) is sound
    /// only because parsing without a syntax set produces the same per-file result as
    /// parsing with one. Highlighting assigns spans; `content_hash` comes from line
    /// text at parse time. If that stops being true the watcher breaks silently, so
    /// pin it here.
    ///
    /// The `.vue` file is deliberate. Extensions accepted by
    /// `needs_full_file_highlight` are the only ones where highlighting does extra
    /// work after parsing, via `enhance_with_full_file_highlight`. Without one, the
    /// test never exercises the path most likely to break the equality.
    ///
    /// Runs against both Git backends: `content_hash` is computed separately from
    /// styling at each backend's own parse site, so the equality is a property of
    /// each backend, not a shared guarantee. Mercurial and Jujutsu are out of scope.
    /// Exercising them needs those CLIs installed, which this test suite does not
    /// assume.
    #[test]
    fn should_match_plain_and_highlighted_parses_for_both_git_backends() {
        let repo = TempRepo::with_changes(&[
            ("a.rs", "fn main() { let x = 1; }\n"),
            ("b.vue", "<template><div>{{ x }}</div></template>\n"),
        ]);

        let backends: Vec<(&str, Box<dyn VcsBackend>)> = vec![
            (
                "libgit2",
                Box::new(
                    Libgit2Backend::discover_from(repo.path(), DiffWhitespaceMode::Normal, true)
                        .expect("failed to open libgit2 backend"),
                ),
            ),
            (
                "git cli",
                Box::new(
                    GitCliBackend::discover_from(repo.path(), DiffWhitespaceMode::Normal, true)
                        .expect("failed to open git cli backend"),
                ),
            ),
        ];

        for (label, backend) in backends {
            let highlighted = backend
                .get_working_tree_diff(&SyntaxHighlighter::default())
                .unwrap_or_else(|e| panic!("{label}: highlighted fetch failed: {e}"));
            let plain = backend
                .get_working_tree_diff(&SyntaxHighlighter::plain())
                .unwrap_or_else(|e| panic!("{label}: plain fetch failed: {e}"));

            assert_files_match(&highlighted, &plain, label);
        }
    }

    fn file_named<'a>(files: &'a [DiffFile], path: &str) -> Option<&'a DiffFile> {
        files
            .iter()
            .find(|file| file.display_path() == Path::new(path))
    }

    fn assert_ignored_whitespace(files: &[DiffFile], path: &str, context: &str) {
        match file_named(files, path) {
            None => {}
            Some(file) => assert!(
                file.hunks.is_empty(),
                "{context}: {path} should omit whitespace hunks"
            ),
        }
    }

    fn assert_has_hunks(files: &[DiffFile], path: &str, context: &str) {
        let file = file_named(files, path)
            .unwrap_or_else(|| panic!("{context}: expected {path} to remain visible"));
        assert!(
            !file.hunks.is_empty(),
            "{context}: {path} should retain hunks"
        );
    }

    fn assert_visible(files: &[DiffFile], path: &str, context: &str) {
        assert!(
            file_named(files, path).is_some(),
            "{context}: expected {path} to remain visible, got {:?}",
            files
                .iter()
                .map(|file| file.display_path().display().to_string())
                .collect::<Vec<_>>()
        );
    }

    fn auto_mode() -> DiffWhitespaceMode {
        DiffWhitespaceMode::Auto(crate::vcs::WhitespaceAutoPolicy::builtin())
    }

    fn auto_with_overrides() -> DiffWhitespaceMode {
        DiffWhitespaceMode::Auto(crate::vcs::WhitespaceAutoPolicy::with_overrides([
            ("rs".into(), false),
            ("custom".into(), true),
        ]))
    }

    fn setup_mixed_whitespace_git_repo() -> (tempfile::TempDir, Vec<String>) {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let root = dir.path();
        run_git(root, &["init"]);
        run_git(root, &["config", "user.name", "Tuicr Test"]);
        run_git(root, &["config", "user.email", "tuicr@example.com"]);
        fs::write(root.join("data.json"), "{\"a\":1}\n").unwrap();
        fs::write(root.join("lib.rs"), "fn x(){}\n").unwrap();
        fs::write(root.join("app.py"), "x = 1\n").unwrap();
        fs::write(root.join("cfg.yaml"), "a: 1\n").unwrap();
        fs::write(root.join("keep.txt"), "hello\n").unwrap();
        fs::write(root.join("gone.txt"), "bye\n").unwrap();
        fs::write(root.join("renamed.txt"), "same\n").unwrap();
        fs::write(root.join("mode.txt"), "alpha\n").unwrap();
        fs::write(root.join("extra.custom"), "foo\n").unwrap();
        fs::write(root.join("binary.bin"), [0, 1, 2, 3]).unwrap();
        run_git(root, &["add", "-A"]);
        run_git(root, &["commit", "-m", "base"]);
        let first = run_git_rev_parse(root);

        fs::write(root.join("keep.txt"), "hello\nrange\n").unwrap();
        run_git(root, &["add", "keep.txt"]);
        run_git(root, &["commit", "-m", "subset"]);
        let second = run_git_rev_parse(root);

        fs::write(root.join("data.json"), "{ \"a\" : 1 }\n").unwrap();
        fs::write(root.join("lib.rs"), "fn x(){ }\n").unwrap();
        fs::write(root.join("app.py"), "x =  1\n").unwrap();
        fs::write(root.join("cfg.yaml"), "a:  1\n").unwrap();
        fs::write(root.join("extra.custom"), " foo \n").unwrap();
        fs::write(root.join("keep.txt"), "hello\nrange\nworld\n").unwrap();
        fs::remove_file(root.join("gone.txt")).unwrap();
        fs::write(root.join("added.txt"), "new\n").unwrap();
        run_git(root, &["add", "added.txt"]);
        run_git(root, &["mv", "renamed.txt", "renamed-new.txt"]);
        fs::write(root.join("binary.bin"), [0, 1, 9, 3]).unwrap();
        fs::write(root.join("untracked.json"), "{ }\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let path = root.join("mode.txt");
            let mut permissions = fs::metadata(&path).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(path, permissions).unwrap();
        }

        (dir, vec![first, second])
    }

    fn run_git_rev_parse(dir: &Path) -> String {
        let output = std::process::Command::new("git")
            .args([
                "-c",
                "commit.gpgsign=false",
                "-c",
                "init.defaultRefFormat=files",
                "rev-parse",
                "HEAD",
            ])
            .current_dir(dir)
            .output()
            .expect("failed to run git rev-parse");
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    }

    fn mixed_backends(
        path: &Path,
        mode: DiffWhitespaceMode,
    ) -> Vec<(&'static str, Box<dyn VcsBackend>)> {
        vec![
            (
                "libgit2",
                Box::new(
                    Libgit2Backend::discover_from(path, mode.clone(), true)
                        .expect("failed to open libgit2 backend"),
                ),
            ),
            (
                "git cli",
                Box::new(
                    GitCliBackend::discover_from(path, mode, true)
                        .expect("failed to open git cli backend"),
                ),
            ),
        ]
    }

    fn assert_auto_working_tree(files: &[DiffFile], context: &str) {
        assert_ignored_whitespace(files, "data.json", context);
        assert_ignored_whitespace(files, "lib.rs", context);
        assert_has_hunks(files, "app.py", context);
        assert_has_hunks(files, "cfg.yaml", context);
        assert_has_hunks(files, "keep.txt", context);
        assert_visible(files, "gone.txt", context);
        assert_visible(files, "added.txt", context);
        assert_visible(files, "binary.bin", context);
        assert_visible(files, "untracked.json", context);
        assert!(
            file_named(files, "renamed-new.txt").is_some()
                || file_named(files, "renamed.txt").is_some(),
            "{context}: rename-only change should remain visible"
        );
        #[cfg(unix)]
        assert_visible(files, "mode.txt", context);
    }

    #[test]
    fn auto_whitespace_selects_per_extension_for_both_git_backends() {
        let (repo, _ids) = setup_mixed_whitespace_git_repo();
        let highlighter = SyntaxHighlighter::default();

        for (label, backend) in mixed_backends(repo.path(), auto_mode()) {
            let files = backend
                .get_working_tree_diff(&highlighter)
                .unwrap_or_else(|e| panic!("{label}: auto working tree failed: {e}"));
            assert_auto_working_tree(&files, label);
            assert_has_hunks(&files, "extra.custom", label);
        }

        for (label, backend) in mixed_backends(repo.path(), DiffWhitespaceMode::IgnoreAll) {
            let files = backend
                .get_working_tree_diff(&highlighter)
                .unwrap_or_else(|e| panic!("{label}: ignore-all working tree failed: {e}"));
            assert_ignored_whitespace(&files, "data.json", label);
            assert_ignored_whitespace(&files, "app.py", label);
            assert_ignored_whitespace(&files, "cfg.yaml", label);
            assert_has_hunks(&files, "keep.txt", label);
        }

        for (label, backend) in mixed_backends(repo.path(), DiffWhitespaceMode::Normal) {
            let files = backend
                .get_working_tree_diff(&highlighter)
                .unwrap_or_else(|e| panic!("{label}: normal working tree failed: {e}"));
            assert_has_hunks(&files, "data.json", label);
            assert_has_hunks(&files, "lib.rs", label);
            assert_has_hunks(&files, "app.py", label);
        }

        for (label, backend) in mixed_backends(repo.path(), auto_with_overrides()) {
            let files = backend
                .get_working_tree_diff(&highlighter)
                .unwrap_or_else(|e| panic!("{label}: override working tree failed: {e}"));
            assert_ignored_whitespace(&files, "data.json", label);
            assert_has_hunks(&files, "lib.rs", label);
            assert_ignored_whitespace(&files, "extra.custom", label);
            assert_has_hunks(&files, "app.py", label);
        }
    }

    #[test]
    fn auto_whitespace_covers_git_diff_endpoints() {
        let (repo, ids) = setup_mixed_whitespace_git_repo();
        let highlighter = SyntaxHighlighter::default();
        let range = ResolvedRevisionRange::from_owned_commit_ids(
            vec![ids[1].clone()],
            crate::vcs::RevisionDiffTarget::CommitList,
        );

        for (label, backend) in mixed_backends(repo.path(), auto_mode()) {
            let staged = backend
                .get_staged_diff(&highlighter)
                .unwrap_or_else(|e| panic!("{label}: staged failed: {e}"));
            assert_visible(&staged, "added.txt", &format!("{label} staged"));

            let unstaged = backend
                .get_unstaged_diff(&highlighter)
                .unwrap_or_else(|e| panic!("{label}: unstaged failed: {e}"));
            assert_ignored_whitespace(&unstaged, "data.json", &format!("{label} unstaged"));
            assert_has_hunks(&unstaged, "app.py", &format!("{label} unstaged"));

            let range_files = backend
                .get_commit_range_diff(&range, &highlighter)
                .unwrap_or_else(|e| panic!("{label}: commit range failed: {e}"));
            assert_has_hunks(&range_files, "keep.txt", &format!("{label} range"));

            let combined = backend
                .get_working_tree_with_commits_diff(&[ids[1].clone()], &highlighter)
                .unwrap_or_else(|e| panic!("{label}: combined failed: {e}"));
            assert_ignored_whitespace(&combined, "data.json", &format!("{label} combined"));
            assert_has_hunks(&combined, "keep.txt", &format!("{label} combined"));
        }
    }

    #[test]
    fn auto_whitespace_surfaces_unborn_head_and_strict_subset() {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let repo = Repository::init(temp_dir.path()).expect("failed to init repo");
        fs::write(temp_dir.path().join("data.json"), "{ \"a\" : 1 }\n").unwrap();
        fs::write(temp_dir.path().join("app.py"), "x =  1\n").unwrap();
        let mut index = repo.index().expect("open index");
        index.add_path(Path::new("data.json")).unwrap();
        index.add_path(Path::new("app.py")).unwrap();
        index.write().unwrap();

        let files = get_working_tree_diff(&repo, &auto_mode(), true, &SyntaxHighlighter::default())
            .expect("unborn HEAD auto diff");
        assert_visible(&files, "data.json", "unborn");
        assert_visible(&files, "app.py", "unborn");

        let staged = get_staged_diff(&repo, &auto_mode(), &SyntaxHighlighter::default())
            .expect("unborn staged auto diff");
        assert_visible(&staged, "data.json", "unborn staged");

        let (repo, ids) = setup_mixed_whitespace_git_repo();
        let subset = ResolvedRevisionRange::from_owned_commit_ids(
            vec![ids[1].clone()],
            crate::vcs::RevisionDiffTarget::CommitList,
        );
        for (label, backend) in mixed_backends(repo.path(), auto_mode()) {
            let files = backend
                .get_commit_range_diff(&subset, &SyntaxHighlighter::default())
                .unwrap_or_else(|e| panic!("{label}: subset failed: {e}"));
            assert_eq!(
                files.len(),
                1,
                "{label}: strict subset should only include keep.txt"
            );
            assert_has_hunks(&files, "keep.txt", &format!("{label} subset"));
        }
    }

    fn git_init_commit(dir: &Path, files: &[(&str, &[u8])]) {
        run_git(dir, &["init", "-q"]);
        run_git(dir, &["config", "user.email", "t@example.com"]);
        run_git(dir, &["config", "user.name", "T"]);
        for (name, content) in files {
            fs::write(dir.join(name), content).unwrap();
        }
        run_git(dir, &["add", "."]);
        run_git(dir, &["commit", "-q", "-m", "init"]);
    }

    #[test]
    fn should_see_externally_staged_files_and_keep_them_when_staging() {
        let dir = tempfile::tempdir().unwrap();
        git_init_commit(dir.path(), &[("a.txt", b"a\n"), ("b.txt", b"b\n")]);
        let backend =
            Libgit2Backend::discover_from(dir.path(), DiffWhitespaceMode::Normal, true).unwrap();
        fs::write(dir.path().join("a.txt"), "a2\n").unwrap();
        fs::write(dir.path().join("b.txt"), "b2\n").unwrap();
        let hl = SyntaxHighlighter::default();
        backend.get_unstaged_diff(&hl).unwrap();

        run_git(dir.path(), &["add", "b.txt"]);

        let staged = backend.get_staged_diff(&hl).unwrap();
        assert_eq!(staged.len(), 1);
        backend.stage_file(Path::new("a.txt")).unwrap();
        let out = std::process::Command::new("git")
            .args(["diff", "--cached", "--name-only"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "a.txt\nb.txt\n");
    }

    #[test]
    fn should_match_cli_for_files_without_trailing_newline() {
        let dir = tempfile::tempdir().unwrap();
        git_init_commit(
            dir.path(),
            &[
                ("both.txt", b"a\nb"),
                ("gain.txt", b"x"),
                ("lose.txt", b"y\n"),
            ],
        );
        fs::write(dir.path().join("both.txt"), "a\nc").unwrap();
        fs::write(dir.path().join("gain.txt"), "x\n").unwrap();
        fs::write(dir.path().join("lose.txt"), "y").unwrap();
        fs::write(dir.path().join("new.txt"), "n").unwrap();
        run_git(dir.path(), &["add", "new.txt"]);

        let mut results = Vec::new();
        for (label, backend) in mixed_backends(dir.path(), DiffWhitespaceMode::Normal) {
            let mut files = backend
                .get_working_tree_diff(&SyntaxHighlighter::default())
                .unwrap();
            files.sort_by_key(|f| f.display_path().to_path_buf());
            results.push((label, files));
        }
        let (_, lg) = &results[0];
        let (_, cli) = &results[1];
        assert_eq!(lg.len(), cli.len());
        for (a, b) in lg.iter().zip(cli) {
            assert_eq!(a.hunks.len(), b.hunks.len(), "{:?}", a.display_path());
            for (ha, hb) in a.hunks.iter().zip(&b.hunks) {
                let sig = |h: &DiffHunk| {
                    h.lines
                        .iter()
                        .map(|l| (l.origin, l.content.clone(), l.old_lineno, l.new_lineno))
                        .collect::<Vec<_>>()
                };
                assert_eq!(sig(ha), sig(hb), "{:?}", a.display_path());
            }
            assert_eq!(a.content_hash, b.content_hash, "{:?}", a.display_path());
        }
    }

    #[test]
    fn should_flag_modified_binary_files_in_libgit2() {
        let dir = tempfile::tempdir().unwrap();
        git_init_commit(dir.path(), &[("img.bin", &[0, 1, 2, 3])]);
        fs::write(dir.path().join("img.bin"), [0, 1, 9, 3]).unwrap();
        let backend =
            Libgit2Backend::discover_from(dir.path(), DiffWhitespaceMode::Normal, true).unwrap();

        let files = backend
            .get_working_tree_diff(&SyntaxHighlighter::default())
            .unwrap();

        assert!(files[0].is_binary);
        assert!(files[0].hunks.is_empty());
    }

    /// Create a repo with two commits, where each file changes from its old
    /// bytes to its new bytes.
    fn attribute_repo(
        files: &[(&str, &[u8], &[u8])],
        attributes: &str,
    ) -> (tempfile::TempDir, Repository) {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        let commit = |repo: &Repository, side: usize, parents: &[&git2::Commit<'_>]| {
            for (name, old, new) in files {
                fs::write(dir.path().join(name), if side == 0 { *old } else { *new }).unwrap();
            }
            fs::write(dir.path().join(".gitattributes"), attributes).unwrap();
            let mut index = repo.index().unwrap();
            index
                .add_all(["*"], git2::IndexAddOption::FORCE, None)
                .unwrap();
            index.write().unwrap();
            let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
            let sig = git2::Signature::now("Test", "test@example.com").unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "c", &tree, parents)
                .unwrap()
        };
        let first = commit(&repo, 0, &[]);
        let first = repo.find_commit(first).unwrap();
        commit(&repo, 1, &[&first]);
        drop(first);
        (dir, repo)
    }

    fn head_diff(repo: &Repository) -> Diff<'_> {
        let new = repo.head().unwrap().peel_to_commit().unwrap();
        let old = new.parent(0).unwrap();
        repo.diff_tree_to_tree(
            Some(&old.tree().unwrap()),
            Some(&new.tree().unwrap()),
            Some(&mut DiffOptions::new()),
        )
        .unwrap()
    }

    fn hunk_text(files: &[DiffFile]) -> Vec<(Option<PathBuf>, Vec<String>)> {
        files
            .iter()
            .map(|file| {
                let lines = file
                    .hunks
                    .iter()
                    .flat_map(|hunk| {
                        std::iter::once(hunk.header.clone()).chain(hunk.lines.iter().map(|line| {
                            format!(
                                "{:?} {:?} {:?} {}",
                                line.origin, line.old_lineno, line.new_lineno, line.content
                            )
                        }))
                    })
                    .collect();
                (file.new_path.clone(), lines)
            })
            .collect()
    }

    /// `parse_tree_diff` must produce the same hunks as `from_diff`. Each file
    /// here uses a `.gitattributes` rule that can change a diff.
    #[test]
    fn should_match_from_diff_when_gitattributes_change_how_files_diff() {
        let (_dir, repo) = attribute_repo(
            &[
                ("plain.txt", b"one\n", b"two\n"),
                ("marked.dat", b"one\n", b"two\n"),
                ("hidden.txt2", b"one\n", b"two\n"),
                ("real.bin", b"a\0b\n", b"a\0c\n"),
                ("forced.bin2", b"one\n", b"x\0two\n"),
                (
                    "named.py",
                    b"def f():\n    a\n    b\n    c\n    d\n    e\n    g\n",
                    b"def f():\n    a\n    b\n    c\n    d\n    e\n    h\n",
                ),
            ],
            "*.dat binary\n*.txt2 -diff\n*.bin2 diff\n*.py diff=python\n",
        );
        let diff = head_diff(&repo);

        let expected = parse_diff(&repo, diff).unwrap();
        // Use a second diff. `from_diff` writes the binary flag into the delta it
        // reads, so `parse_tree_diff` would skip these files on a shared diff.
        let fresh = head_diff(&repo);
        let actual = parse_tree_diff(&repo, &fresh, WhitespaceComparison::Normal).unwrap();

        assert_eq!(hunk_text(&actual.files), hunk_text(&expected));
    }

    #[test]
    fn should_count_new_version_lines_while_reading_for_the_patch() {
        let (_dir, repo) = attribute_repo(&[("a.txt", b"one\n", b"one\ntwo\nthree\n")], "");
        let diff = head_diff(&repo);

        let parsed = parse_tree_diff(&repo, &diff, WhitespaceComparison::Normal).unwrap();

        assert_eq!(parsed.counts, vec![(PathBuf::from("a.txt"), 3)]);
    }

    #[test]
    fn should_keep_whole_file_contents_only_for_container_grammars() {
        let (_dir, repo) = attribute_repo(
            &[
                ("a.svelte", b"<p>a</p>\n", b"<p>b</p>\n"),
                ("b.rs", b"a\n", b"b\n"),
            ],
            "",
        );
        let diff = head_diff(&repo);

        let parsed = parse_tree_diff(&repo, &diff, WhitespaceComparison::Normal).unwrap();

        assert_eq!(
            parsed.new_contents.keys().collect::<Vec<_>>(),
            vec![&PathBuf::from("a.svelte")]
        );
        assert_eq!(
            parsed.old_contents[&PathBuf::from("a.svelte")],
            "<p>a</p>\n"
        );
    }

    #[test]
    fn should_refuse_to_read_a_blob_over_the_size_limit() {
        let (_dir, repo) = attribute_repo(&[("a.txt", b"one\n", b"twelve bytes")], "");
        let diff = head_diff(&repo);
        let id = diff.get_delta(0).unwrap().new_file().id();

        assert!(read_delta_blob(&repo, id, 11).is_err());
        assert!(read_delta_blob(&repo, id, 12).unwrap().is_some());
    }

    #[test]
    fn should_not_keep_contents_of_a_container_file_over_the_highlight_cap() {
        let big = vec![b'a'; MAX_HIGHLIGHT_FILE_BYTES + 1];
        let (_dir, repo) = attribute_repo(&[("a.svelte", b"<p>a</p>\n", &big)], "");
        let diff = head_diff(&repo);

        let parsed = parse_tree_diff(&repo, &diff, WhitespaceComparison::Normal).unwrap();

        assert!(parsed.new_contents.is_empty());
        assert!(parsed.old_contents.contains_key(&PathBuf::from("a.svelte")));
    }
}
