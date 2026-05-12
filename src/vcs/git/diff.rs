use git2::{Delta, Diff, DiffOptions, Repository};
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::error::{Result, TuicrError};
use crate::model::{DiffFile, DiffHunk, DiffLine, FileStatus, LineOrigin, LineSide};
use crate::syntax::{SyntaxHighlighter, needs_full_file_highlight};
use crate::vcs::diff_parser::{self, DiffFormat};
use crate::vcs::{container_file_paths, enhance_with_full_file_highlight, tabify};

use super::GitCapabilities;

// Untracked files larger than this are shown in the file list but their
// content is not parsed — they are likely logs, dumps, or build artefacts.
const MAX_UNTRACKED_FILE_SIZE: u64 = 10 * 1_024 * 1_024;

pub fn get_working_tree_diff(
    repo: &Repository,
    capabilities: GitCapabilities,
    highlighter: &SyntaxHighlighter,
) -> Result<Vec<DiffFile>> {
    if capabilities.requires_git_cli() {
        return get_cli_diff(
            repo,
            &["diff", "--no-ext-diff", "--binary", "HEAD", "--"],
            true,
            GitContentSource::Revision("HEAD"),
            GitContentSource::Workdir,
            highlighter,
        );
    }

    let head = repo.head()?.peel_to_tree()?;

    let mut opts = DiffOptions::new();
    opts.include_untracked(true);
    opts.show_untracked_content(true);
    opts.recurse_untracked_dirs(true);

    let diff = repo.diff_tree_to_workdir_with_index(Some(&head), Some(&mut opts))?;
    let mut files = parse_diff(&diff, highlighter)?;
    enhance_with_full_file_highlight(
        &mut files,
        highlighter,
        |path| read_path_from_tree(repo, &head, path),
        |path| read_path_from_workdir(repo, path),
    );
    Ok(files)
}

/// Get the staged diff (index vs HEAD)
/// On repos with no commits (unborn HEAD), diffs against an empty tree.
pub fn get_staged_diff(
    repo: &Repository,
    capabilities: GitCapabilities,
    highlighter: &SyntaxHighlighter,
) -> Result<Vec<DiffFile>> {
    if capabilities.requires_git_cli() {
        let old_source = if repo.head().is_ok() {
            GitContentSource::Revision("HEAD")
        } else {
            GitContentSource::None
        };
        return get_cli_diff(
            repo,
            &["diff", "--no-ext-diff", "--binary", "--cached", "--"],
            false,
            old_source,
            GitContentSource::Index,
            highlighter,
        );
    }

    let head = repo.head().ok().and_then(|h| h.peel_to_tree().ok());
    let index = repo.index()?;
    let diff = repo.diff_tree_to_index(head.as_ref(), Some(&index), None)?;
    let mut files = parse_diff(&diff, highlighter)?;
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
pub fn get_unstaged_diff(
    repo: &Repository,
    capabilities: GitCapabilities,
    highlighter: &SyntaxHighlighter,
) -> Result<Vec<DiffFile>> {
    if capabilities.requires_git_cli() {
        return get_cli_diff(
            repo,
            &["diff", "--no-ext-diff", "--binary", "--"],
            true,
            GitContentSource::Index,
            GitContentSource::Workdir,
            highlighter,
        );
    }

    let index = repo.index()?;
    let mut opts = DiffOptions::new();
    opts.include_untracked(true);
    opts.show_untracked_content(true);
    opts.recurse_untracked_dirs(true);

    let diff = repo.diff_index_to_workdir(Some(&index), Some(&mut opts))?;
    let mut files = parse_diff(&diff, highlighter)?;
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
    capabilities: GitCapabilities,
    commit_ids: &[String],
    highlighter: &SyntaxHighlighter,
) -> Result<Vec<DiffFile>> {
    if commit_ids.is_empty() {
        return Err(TuicrError::NoChanges);
    }

    if capabilities.requires_git_cli() {
        let base_rev = parent_rev_or_empty(repo, &commit_ids[0]);
        let newest_rev = commit_ids.last().unwrap();
        return get_cli_diff(
            repo,
            &[
                "diff",
                "--no-ext-diff",
                "--binary",
                &base_rev,
                newest_rev,
                "--",
            ],
            false,
            GitContentSource::Revision(&base_rev),
            GitContentSource::Revision(newest_rev),
            highlighter,
        );
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

    let new_tree = newest_commit.tree()?;

    let diff = repo.diff_tree_to_tree(old_tree.as_ref(), Some(&new_tree), None)?;
    let mut files = parse_diff(&diff, highlighter)?;
    enhance_with_full_file_highlight(
        &mut files,
        highlighter,
        |path| {
            old_tree
                .as_ref()
                .and_then(|tree| read_path_from_tree(repo, tree, path))
        },
        |path| read_path_from_tree(repo, &new_tree, path),
    );
    Ok(files)
}

/// Get a combined diff from the parent of the oldest commit through to the working tree.
/// This shows both committed and working tree changes in a single diff.
pub fn get_working_tree_with_commits_diff(
    repo: &Repository,
    capabilities: GitCapabilities,
    commit_ids: &[String],
    highlighter: &SyntaxHighlighter,
) -> Result<Vec<DiffFile>> {
    if commit_ids.is_empty() {
        return Err(TuicrError::NoChanges);
    }

    if capabilities.requires_git_cli() {
        let base_rev = parent_rev_or_empty(repo, &commit_ids[0]);
        return get_cli_diff(
            repo,
            &["diff", "--no-ext-diff", "--binary", &base_rev, "--"],
            true,
            GitContentSource::Revision(&base_rev),
            GitContentSource::Workdir,
            highlighter,
        );
    }

    let oldest_id = git2::Oid::from_str(&commit_ids[0])?;
    let oldest_commit = repo.find_commit(oldest_id)?;

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
    let mut files = parse_diff(&diff, highlighter)?;
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

#[derive(Clone, Copy)]
enum GitContentSource<'a> {
    None,
    Workdir,
    Index,
    Revision(&'a str),
}

fn empty_tree_oid() -> git2::Oid {
    git2::Oid::from_str("4b825dc642cb6eb9a060e54bf8d69288fbee4904")
        .expect("empty tree oid should be valid")
}

fn parent_rev_or_empty(repo: &Repository, commit_id: &str) -> String {
    let parent_spec = format!("{commit_id}^");
    run_git_command(repo, &["rev-parse", &parent_spec], false)
        .map(|rev| rev.trim().to_string())
        .unwrap_or_else(|_| empty_tree_oid().to_string())
}

fn get_cli_diff(
    repo: &Repository,
    args: &[&str],
    include_untracked: bool,
    old_source: GitContentSource<'_>,
    new_source: GitContentSource<'_>,
    highlighter: &SyntaxHighlighter,
) -> Result<Vec<DiffFile>> {
    let mut files = match crate::profile::time_with(
        "vcs: git cli diff command",
        || run_git_diff_command(repo, args, highlighter),
        profile_diff_result,
    ) {
        Ok(files) => files,
        Err(TuicrError::NoChanges) => Vec::new(),
        Err(err) => return Err(err),
    };

    if include_untracked {
        crate::profile::time_with(
            "vcs: git cli untracked diff",
            || append_untracked_cli_diffs(repo, &mut files, highlighter),
            |result| match result {
                Ok(count) => format!("files={count}"),
                Err(e) => format!("error={e}"),
            },
        )?;
    }

    if files.is_empty() {
        return Err(TuicrError::NoChanges);
    }

    let old_cache = crate::profile::time("vcs: git cli old content cache", || {
        git_source_content_cache(repo, old_source, &files, LineSide::Old)
    });
    let new_cache = crate::profile::time("vcs: git cli new content cache", || {
        git_source_content_cache(repo, new_source, &files, LineSide::New)
    });

    crate::profile::time("vcs: git cli full-file highlight", || {
        enhance_with_full_file_highlight(
            &mut files,
            highlighter,
            |path| read_path_from_git_source_cached(repo, old_source, old_cache.as_ref(), path),
            |path| read_path_from_git_source_cached(repo, new_source, new_cache.as_ref(), path),
        );
    });
    Ok(files)
}

fn profile_diff_result(result: &Result<Vec<DiffFile>>) -> String {
    match result {
        Ok(files) => format!("files={}", files.len()),
        Err(e) => format!("error={e}"),
    }
}

fn run_git_diff_command(
    repo: &Repository,
    args: &[&str],
    highlighter: &SyntaxHighlighter,
) -> Result<Vec<DiffFile>> {
    let workdir = repo.workdir().ok_or(TuicrError::NotARepository)?;
    let mut child = Command::new("git")
        .current_dir(workdir)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| TuicrError::VcsCommand(format!("Failed to run git: {}", e)))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| TuicrError::VcsCommand("git diff stdout unavailable".into()))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| TuicrError::VcsCommand("git diff stderr unavailable".into()))?;
    let stderr_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        bytes
    });

    let diff_lines = BufReader::new(stdout)
        .lines()
        .map(|line| line.map_err(TuicrError::from));
    let parse_result =
        diff_parser::parse_unified_diff_lines(diff_lines, DiffFormat::GitStyle, highlighter);

    let status = child.wait()?;
    let stderr = stderr_reader
        .join()
        .map_err(|_| TuicrError::VcsCommand("git diff stderr reader panicked".into()))?;

    if !status.success() {
        let stderr = String::from_utf8_lossy(&stderr);
        return Err(TuicrError::VcsCommand(format!(
            "git {} failed: {}",
            args.join(" "),
            stderr
        )));
    }

    parse_result
}

fn append_untracked_cli_diffs(
    repo: &Repository,
    files: &mut Vec<DiffFile>,
    highlighter: &SyntaxHighlighter,
) -> Result<usize> {
    let Some(workdir) = repo.workdir() else {
        return Ok(0);
    };

    let previous_len = files.len();
    for_each_untracked_path(repo, |path| {
        let full_path = workdir.join(&path);
        let Some(file) = build_untracked_diff_file(&path, &full_path, highlighter) else {
            return Ok(());
        };
        files.push(file);
        Ok(())
    })?;
    Ok(files.len().saturating_sub(previous_len))
}

fn build_untracked_diff_file(
    path: &Path,
    full_path: &Path,
    highlighter: &SyntaxHighlighter,
) -> Option<DiffFile> {
    let metadata = full_path.metadata().ok()?;
    if metadata.len() > MAX_UNTRACKED_FILE_SIZE {
        return Some(diff_file_without_hunks(path, false, true));
    }

    let bytes = fs::read(full_path).ok()?;
    if bytes.contains(&0) {
        return Some(diff_file_without_hunks(path, true, false));
    }

    let content = String::from_utf8_lossy(&bytes);
    let lines: Vec<String> = content
        .lines()
        .map(|line| tabify(line.trim_end_matches('\r')))
        .collect();

    if lines.is_empty() {
        return Some(diff_file_without_hunks(path, false, false));
    }

    let highlighted = highlighter.highlight_file_lines(path, &lines);
    let diff_lines: Vec<DiffLine> = lines
        .into_iter()
        .enumerate()
        .map(|(idx, content)| DiffLine {
            origin: LineOrigin::Addition,
            content,
            old_lineno: None,
            new_lineno: Some((idx + 1) as u32),
            highlighted_spans: highlighter.highlighted_line_for_diff_with_background(
                None,
                highlighted.as_deref(),
                None,
                Some(idx),
                LineOrigin::Addition,
            ),
        })
        .collect();

    let new_count = diff_lines.len() as u32;
    let hunks = vec![DiffHunk {
        header: format!("@@ -0,0 +1,{} @@", new_count),
        lines: diff_lines,
        old_start: 0,
        old_count: 0,
        new_start: 1,
        new_count,
    }];
    let content_hash = DiffFile::compute_content_hash(&hunks);

    Some(DiffFile {
        old_path: None,
        new_path: Some(path.to_path_buf()),
        status: FileStatus::Added,
        hunks,
        is_binary: false,
        is_too_large: false,
        is_commit_message: false,
        content_hash,
    })
}

fn diff_file_without_hunks(path: &Path, is_binary: bool, is_too_large: bool) -> DiffFile {
    DiffFile {
        old_path: None,
        new_path: Some(path.to_path_buf()),
        status: FileStatus::Added,
        hunks: Vec::new(),
        is_binary,
        is_too_large,
        is_commit_message: false,
        content_hash: 0,
    }
}

fn for_each_untracked_path<F>(repo: &Repository, mut visit: F) -> Result<()>
where
    F: FnMut(PathBuf) -> Result<()>,
{
    let workdir = repo.workdir().ok_or(TuicrError::NotARepository)?;
    let mut child = Command::new("git")
        .current_dir(workdir)
        .args(["ls-files", "--others", "--exclude-standard", "-z"])
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| TuicrError::VcsCommand(format!("Failed to run git: {}", e)))?;

    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| TuicrError::VcsCommand("git ls-files stdout unavailable".into()))?;
    let mut buffer = [0; 8192];
    let mut path = Vec::new();

    loop {
        let read = stdout.read(&mut buffer)?;
        if read == 0 {
            break;
        }

        for &byte in &buffer[..read] {
            if byte == 0 {
                if !path.is_empty() {
                    visit(PathBuf::from(String::from_utf8_lossy(&path).into_owned()))?;
                    path.clear();
                }
            } else {
                path.push(byte);
            }
        }
    }

    if !path.is_empty() {
        visit(PathBuf::from(String::from_utf8_lossy(&path).into_owned()))?;
    }

    let status = child.wait()?;
    if !status.success() {
        return Err(TuicrError::VcsCommand(format!(
            "git ls-files failed with status {}",
            status
        )));
    }

    Ok(())
}

fn read_path_from_git_source(
    repo: &Repository,
    source: GitContentSource<'_>,
    path: &Path,
) -> Option<String> {
    match source {
        GitContentSource::None => None,
        GitContentSource::Workdir => read_path_from_workdir(repo, path),
        GitContentSource::Index => read_git_object(repo, &format!(":0:{}", path.to_string_lossy())),
        GitContentSource::Revision(rev) => {
            read_git_object(repo, &format!("{}:{}", rev, path.to_string_lossy()))
        }
    }
}

fn read_path_from_git_source_cached(
    repo: &Repository,
    source: GitContentSource<'_>,
    cache: Option<&HashMap<PathBuf, String>>,
    path: &Path,
) -> Option<String> {
    cache
        .and_then(|contents| contents.get(path).cloned())
        .or_else(|| read_path_from_git_source(repo, source, path))
}

fn git_source_content_cache(
    repo: &Repository,
    source: GitContentSource<'_>,
    files: &[DiffFile],
    side: LineSide,
) -> Option<HashMap<PathBuf, String>> {
    let paths = container_file_paths(files, side);
    match source {
        GitContentSource::Revision(rev) => {
            let requests = paths
                .into_iter()
                .map(|path| {
                    let spec = format!("{rev}:{}", path.to_string_lossy());
                    (path, spec)
                })
                .collect();
            read_git_objects(repo, requests).ok()
        }
        GitContentSource::Index => {
            let requests = paths
                .into_iter()
                .map(|path| {
                    let spec = format!(":0:{}", path.to_string_lossy());
                    (path, spec)
                })
                .collect();
            read_git_objects(repo, requests).ok()
        }
        GitContentSource::None | GitContentSource::Workdir => None,
    }
}

fn read_git_objects(
    repo: &Repository,
    requests: Vec<(PathBuf, String)>,
) -> Result<HashMap<PathBuf, String>> {
    if requests.is_empty() {
        return Ok(HashMap::new());
    }

    let workdir = repo.workdir().ok_or(TuicrError::NotARepository)?;
    let mut child = Command::new("git")
        .current_dir(workdir)
        .args(["cat-file", "--batch"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| TuicrError::VcsCommand(format!("Failed to run git: {}", e)))?;

    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| TuicrError::VcsCommand("git cat-file stdin unavailable".into()))?;
        for (_, spec) in &requests {
            writeln!(stdin, "{spec}")?;
        }
    }

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| TuicrError::VcsCommand("git cat-file stdout unavailable".into()))?;
    let mut reader = BufReader::new(stdout);
    let mut contents = HashMap::new();

    for (path, _) in requests {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 {
            break;
        }

        let header = header.trim_end();
        if header.ends_with(" missing") {
            continue;
        }

        let mut parts = header.split_whitespace();
        let _oid = parts.next();
        let kind = parts.next();
        let size = parts
            .next()
            .and_then(|value| value.parse::<usize>().ok())
            .ok_or_else(|| TuicrError::VcsCommand("invalid git cat-file header".into()))?;

        let mut bytes = vec![0; size];
        reader.read_exact(&mut bytes)?;
        let mut trailing_newline = [0; 1];
        reader.read_exact(&mut trailing_newline)?;

        if kind == Some("blob") {
            contents.insert(path, String::from_utf8_lossy(&bytes).into_owned());
        }
    }

    let status = child.wait()?;
    if !status.success() {
        return Err(TuicrError::VcsCommand(format!(
            "git cat-file failed with status {}",
            status
        )));
    }

    Ok(contents)
}

fn read_git_object(repo: &Repository, spec: &str) -> Option<String> {
    let output = run_git_command(repo, &["show", spec], false).ok()?;
    Some(output)
}

fn run_git_command(repo: &Repository, args: &[&str], allow_diff_exit: bool) -> Result<String> {
    let workdir = repo.workdir().ok_or(TuicrError::NotARepository)?;
    let output = Command::new("git")
        .current_dir(workdir)
        .args(args)
        .output()
        .map_err(|e| TuicrError::VcsCommand(format!("Failed to run git: {}", e)))?;

    if !(output.status.success() || allow_diff_exit && output.status.code() == Some(1)) {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TuicrError::VcsCommand(format!(
            "git {} failed: {}",
            args.join(" "),
            stderr
        )));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
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
        let is_too_large =
            delta.status() == Delta::Untracked && delta.new_file().size() > MAX_UNTRACKED_FILE_SIZE;

        let syntax_path = new_path.as_ref().or(old_path.as_ref()).map(|p| p.as_path());
        let hunks = if is_binary || is_too_large {
            Vec::new()
        } else {
            parse_hunks(diff, delta_idx, highlighter, syntax_path)?
        };

        let content_hash = DiffFile::compute_content_hash(&hunks);
        files.push(DiffFile {
            old_path,
            new_path,
            status,
            hunks,
            is_binary,
            is_too_large,
            is_commit_message: false,
            content_hash,
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
    highlighter: &SyntaxHighlighter,
    file_path: Option<&Path>,
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

            let mut line_contents: Vec<String> = Vec::new();
            let mut line_origins: Vec<LineOrigin> = Vec::new();
            let mut line_numbers: Vec<(Option<u32>, Option<u32>)> = Vec::new();

            for line_idx in 0..patch.num_lines_in_hunk(hunk_idx)? {
                let line = patch.line_in_hunk(hunk_idx, line_idx)?;

                let origin = match line.origin() {
                    '+' => LineOrigin::Addition,
                    '-' => LineOrigin::Deletion,
                    ' ' => LineOrigin::Context,
                    _ => LineOrigin::Context,
                };

                let raw = String::from_utf8_lossy(line.content());
                let content = tabify(raw.trim_end_matches(['\n', '\r']));

                line_contents.push(content);
                line_origins.push(origin);
                line_numbers.push((line.old_lineno(), line.new_lineno()));
            }

            let sequences =
                SyntaxHighlighter::split_diff_lines_for_highlighting(&line_contents, &line_origins);
            // Container grammars skip per-hunk highlighting; the full-file
            // post-pass overwrites these spans anyway.
            let (old_highlighted, new_highlighted) = match file_path {
                Some(path) if !needs_full_file_highlight(path) => (
                    highlighter.highlight_file_lines(path, &sequences.old_lines),
                    highlighter.highlight_file_lines(path, &sequences.new_lines),
                ),
                _ => (None, None),
            };

            let mut lines: Vec<DiffLine> = Vec::with_capacity(line_contents.len());
            for (idx, content) in line_contents.into_iter().enumerate() {
                let origin = line_origins[idx];
                let (old_lineno, new_lineno) = line_numbers[idx];

                let highlighted_spans = highlighter.highlighted_line_for_diff_with_background(
                    old_highlighted.as_deref(),
                    new_highlighted.as_deref(),
                    sequences.old_line_indices[idx],
                    sequences.new_line_indices[idx],
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
    use crate::vcs::git::GitRepoMode;
    use std::fs;
    use std::path::Path;
    use std::process::Command;
    use std::time::Instant;

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

    fn create_initial_commit_with_files(repo: &Repository, files: &[(&str, &str)]) {
        for (file_name, content) in files {
            let path = repo.workdir().unwrap().join(file_name);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("failed to create parent directory");
            }
            fs::write(path, content).expect("failed to write initial file");
        }

        let mut index = repo.index().expect("failed to open index");
        for (file_name, _) in files {
            index
                .add_path(Path::new(file_name))
                .expect("failed to add file to index");
        }
        index.write().expect("failed to write index");

        let tree_id = index.write_tree().expect("failed to write tree");
        let tree = repo.find_tree(tree_id).expect("failed to find tree");
        let sig = git2::Signature::now("Test User", "test@example.com")
            .expect("failed to create signature");

        repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
            .expect("failed to create commit");
    }

    fn commit_paths(repo: &Repository, message: &str, paths: &[&str]) -> String {
        let mut index = repo.index().expect("failed to open index");
        for path in paths {
            index
                .add_path(Path::new(path))
                .expect("failed to add file to index");
        }
        index.write().expect("failed to write index");

        let tree_id = index.write_tree().expect("failed to write tree");
        let tree = repo.find_tree(tree_id).expect("failed to find tree");
        let sig = git2::Signature::now("Test User", "test@example.com")
            .expect("failed to create signature");
        let parent = repo.head().unwrap().peel_to_commit().unwrap();
        let oid = repo
            .commit(Some("HEAD"), &sig, &sig, message, &tree, &[&parent])
            .expect("failed to create commit");
        oid.to_string()
    }

    fn git(repo: &Repository, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(repo.workdir().unwrap())
            .status()
            .expect("failed to run git");
        assert!(status.success(), "git {args:?} failed with {status}");
    }

    fn standard_capabilities() -> GitCapabilities {
        GitCapabilities {
            mode: GitRepoMode::Standard,
        }
    }

    fn sparse_index_capabilities() -> GitCapabilities {
        GitCapabilities {
            mode: GitRepoMode::SparseIndex,
        }
    }

    #[test]
    fn should_return_no_changes_for_clean_repo() {
        let repo = Repository::discover(".").unwrap();
        let head = repo.head().unwrap().peel_to_tree().unwrap();
        let diff = repo
            .diff_tree_to_tree(Some(&head), Some(&head), None)
            .unwrap();
        let highlighter = SyntaxHighlighter::default();

        let result = parse_diff(&diff, &highlighter);

        assert!(matches!(result, Err(TuicrError::NoChanges)));
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
            standard_capabilities(),
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
            standard_capabilities(),
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

        let unstaged = get_unstaged_diff(&repo, standard_capabilities(), &highlighter)
            .expect("unstaged diff failed");
        assert_eq!(unstaged.len(), 1);
        assert!(matches!(
            get_staged_diff(&repo, standard_capabilities(), &highlighter),
            Err(TuicrError::NoChanges)
        ));

        let mut index = repo.index().expect("failed to open index");
        index
            .add_path(Path::new("file.txt"))
            .expect("failed to add file to index");
        index.write().expect("failed to write index");

        let staged = get_staged_diff(&repo, standard_capabilities(), &highlighter)
            .expect("staged diff failed");
        assert_eq!(staged.len(), 1);
        assert!(matches!(
            get_unstaged_diff(&repo, standard_capabilities(), &highlighter),
            Err(TuicrError::NoChanges)
        ));
    }

    #[test]
    fn should_ignore_paths_outside_sparse_checkout_cone() {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let repo = Repository::init(temp_dir.path()).expect("failed to init repo");

        create_initial_commit_with_files(
            &repo,
            &[
                ("keep/file.txt", "keep base\n"),
                ("hidden/file.txt", "hidden base\n"),
            ],
        );

        git(&repo, &["sparse-checkout", "init", "--cone"]);
        git(&repo, &["sparse-checkout", "set", "keep"]);
        git(&repo, &["sparse-checkout", "reapply", "--sparse-index"]);

        fs::write(temp_dir.path().join("keep/file.txt"), "keep changed\n")
            .expect("failed to update included file");

        let files = get_working_tree_diff(
            &repo,
            sparse_index_capabilities(),
            &SyntaxHighlighter::default(),
        )
        .expect("failed to get sparse checkout diff");

        assert_eq!(files.len(), 1);
        assert_eq!(
            files[0].new_path.as_deref(),
            Some(Path::new("keep/file.txt"))
        );
        assert_eq!(files[0].status, FileStatus::Modified);
    }

    #[test]
    fn should_return_no_changes_for_clean_sparse_checkout() {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let repo = Repository::init(temp_dir.path()).expect("failed to init repo");

        create_initial_commit_with_files(
            &repo,
            &[
                ("keep/file.txt", "keep base\n"),
                ("hidden/file.txt", "hidden base\n"),
            ],
        );

        git(&repo, &["sparse-checkout", "init", "--cone"]);
        git(&repo, &["sparse-checkout", "set", "keep"]);
        git(&repo, &["sparse-checkout", "reapply", "--sparse-index"]);

        assert!(matches!(
            get_working_tree_diff(
                &repo,
                sparse_index_capabilities(),
                &SyntaxHighlighter::default()
            ),
            Err(TuicrError::NoChanges)
        ));
    }

    #[test]
    fn should_include_untracked_files_in_sparse_checkout() {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let repo = Repository::init(temp_dir.path()).expect("failed to init repo");

        create_initial_commit_with_files(
            &repo,
            &[
                ("keep/file.txt", "keep base\n"),
                ("hidden/file.txt", "hidden base\n"),
            ],
        );

        git(&repo, &["sparse-checkout", "init", "--cone"]);
        git(&repo, &["sparse-checkout", "set", "keep"]);
        git(&repo, &["sparse-checkout", "reapply", "--sparse-index"]);

        fs::write(temp_dir.path().join("keep/new.txt"), "new sparse file\n")
            .expect("failed to write untracked file");

        let files = get_working_tree_diff(
            &repo,
            sparse_index_capabilities(),
            &SyntaxHighlighter::default(),
        )
        .expect("failed to get sparse checkout diff");

        assert_eq!(files.len(), 1);
        assert_eq!(
            files[0].new_path.as_deref(),
            Some(Path::new("keep/new.txt"))
        );
        assert_eq!(files[0].status, FileStatus::Added);
        assert_eq!(files[0].hunks.len(), 1);
        assert_eq!(files[0].hunks[0].lines.len(), 1);
        assert_eq!(files[0].hunks[0].lines[0].content, "new sparse file");
        assert_eq!(files[0].hunks[0].lines[0].new_lineno, Some(1));
    }

    #[test]
    fn should_mark_binary_untracked_files_in_sparse_checkout() {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let repo = Repository::init(temp_dir.path()).expect("failed to init repo");

        create_initial_commit_with_files(
            &repo,
            &[
                ("keep/file.txt", "keep base\n"),
                ("hidden/file.txt", "hidden base\n"),
            ],
        );

        git(&repo, &["sparse-checkout", "init", "--cone"]);
        git(&repo, &["sparse-checkout", "set", "keep"]);
        git(&repo, &["sparse-checkout", "reapply", "--sparse-index"]);

        fs::write(temp_dir.path().join("keep/image.bin"), [0_u8, 1, 2, 3])
            .expect("failed to write binary untracked file");

        let files = get_working_tree_diff(
            &repo,
            sparse_index_capabilities(),
            &SyntaxHighlighter::default(),
        )
        .expect("failed to get sparse checkout diff");

        assert_eq!(files.len(), 1);
        assert_eq!(
            files[0].new_path.as_deref(),
            Some(Path::new("keep/image.bin"))
        );
        assert_eq!(files[0].status, FileStatus::Added);
        assert!(files[0].is_binary);
        assert!(files[0].hunks.is_empty());
    }

    #[test]
    fn should_cap_large_untracked_files_in_sparse_checkout() {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let repo = Repository::init(temp_dir.path()).expect("failed to init repo");

        create_initial_commit_with_files(
            &repo,
            &[
                ("keep/file.txt", "keep base\n"),
                ("hidden/file.txt", "hidden base\n"),
            ],
        );

        git(&repo, &["sparse-checkout", "init", "--cone"]);
        git(&repo, &["sparse-checkout", "set", "keep"]);
        git(&repo, &["sparse-checkout", "reapply", "--sparse-index"]);

        fs::write(
            temp_dir.path().join("keep/dump.txt"),
            vec![b'a'; (MAX_UNTRACKED_FILE_SIZE + 1) as usize],
        )
        .expect("failed to write large untracked file");

        let files = get_working_tree_diff(
            &repo,
            sparse_index_capabilities(),
            &SyntaxHighlighter::default(),
        )
        .expect("failed to get sparse checkout diff");

        assert_eq!(files.len(), 1);
        assert_eq!(
            files[0].new_path.as_deref(),
            Some(Path::new("keep/dump.txt"))
        );
        assert_eq!(files[0].status, FileStatus::Added);
        assert!(files[0].is_too_large);
        assert!(files[0].hunks.is_empty());
    }

    #[test]
    fn should_read_staged_diff_in_sparse_checkout() {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let repo = Repository::init(temp_dir.path()).expect("failed to init repo");

        create_initial_commit_with_files(
            &repo,
            &[
                ("keep/file.txt", "keep base\n"),
                ("hidden/file.txt", "hidden base\n"),
            ],
        );

        git(&repo, &["sparse-checkout", "init", "--cone"]);
        git(&repo, &["sparse-checkout", "set", "keep"]);
        git(&repo, &["sparse-checkout", "reapply", "--sparse-index"]);

        fs::write(temp_dir.path().join("keep/file.txt"), "keep staged\n")
            .expect("failed to update included file");
        git(&repo, &["add", "keep/file.txt"]);

        let files = get_staged_diff(
            &repo,
            sparse_index_capabilities(),
            &SyntaxHighlighter::default(),
        )
        .expect("failed to get sparse checkout staged diff");

        assert_eq!(files.len(), 1);
        assert_eq!(
            files[0].new_path.as_deref(),
            Some(Path::new("keep/file.txt"))
        );
        assert_eq!(files[0].status, FileStatus::Modified);
    }

    #[test]
    fn should_read_commit_range_diff_in_sparse_checkout() {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let repo = Repository::init(temp_dir.path()).expect("failed to init repo");

        create_initial_commit_with_files(
            &repo,
            &[
                ("keep/file.txt", "keep base\n"),
                ("hidden/file.txt", "hidden base\n"),
            ],
        );

        fs::write(temp_dir.path().join("keep/file.txt"), "keep commit\n")
            .expect("failed to update included file");
        let second_id = commit_paths(&repo, "second", &["keep/file.txt"]);

        git(&repo, &["sparse-checkout", "init", "--cone"]);
        git(&repo, &["sparse-checkout", "set", "keep"]);
        git(&repo, &["sparse-checkout", "reapply", "--sparse-index"]);

        let files = get_commit_range_diff(
            &repo,
            sparse_index_capabilities(),
            &[second_id],
            &SyntaxHighlighter::default(),
        )
        .expect("failed to get sparse checkout commit range diff");

        assert_eq!(files.len(), 1);
        assert_eq!(
            files[0].new_path.as_deref(),
            Some(Path::new("keep/file.txt"))
        );
        assert!(
            files[0]
                .hunks
                .iter()
                .flat_map(|hunk| &hunk.lines)
                .any(|line| line.content == "keep commit")
        );
    }

    #[test]
    fn should_read_working_tree_with_commits_diff_in_sparse_checkout() {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let repo = Repository::init(temp_dir.path()).expect("failed to init repo");

        create_initial_commit_with_files(
            &repo,
            &[
                ("keep/file.txt", "keep base\n"),
                ("hidden/file.txt", "hidden base\n"),
            ],
        );

        fs::write(temp_dir.path().join("keep/file.txt"), "keep commit\n")
            .expect("failed to update included file");
        let second_id = commit_paths(&repo, "second", &["keep/file.txt"]);

        git(&repo, &["sparse-checkout", "init", "--cone"]);
        git(&repo, &["sparse-checkout", "set", "keep"]);
        git(&repo, &["sparse-checkout", "reapply", "--sparse-index"]);

        fs::write(temp_dir.path().join("keep/file.txt"), "keep worktree\n")
            .expect("failed to update included file");

        let files = get_working_tree_with_commits_diff(
            &repo,
            sparse_index_capabilities(),
            &[second_id],
            &SyntaxHighlighter::default(),
        )
        .expect("failed to get sparse checkout working tree + commits diff");

        assert_eq!(files.len(), 1);
        assert_eq!(
            files[0].new_path.as_deref(),
            Some(Path::new("keep/file.txt"))
        );
        assert!(
            files[0]
                .hunks
                .iter()
                .flat_map(|hunk| &hunk.lines)
                .any(|line| line.content == "keep worktree")
        );
    }

    #[test]
    fn should_batch_read_revision_and_index_blobs() {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let repo = Repository::init(temp_dir.path()).expect("failed to init repo");

        create_initial_commit(&repo, "file.txt", "old\n");
        fs::write(temp_dir.path().join("file.txt"), "new\n").expect("failed to update file");
        git(&repo, &["add", "file.txt"]);

        let contents = read_git_objects(
            &repo,
            vec![
                (PathBuf::from("old.txt"), "HEAD:file.txt".to_string()),
                (PathBuf::from("index.txt"), ":0:file.txt".to_string()),
            ],
        )
        .expect("failed to batch read git objects");

        assert_eq!(contents.get(Path::new("old.txt")).unwrap(), "old\n");
        assert_eq!(contents.get(Path::new("index.txt")).unwrap(), "new\n");
    }

    #[test]
    #[ignore = "benchmark for sparse checkout real-path performance"]
    fn bench_sparse_checkout_real_path_many_vue_files() {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let repo = Repository::init(temp_dir.path()).expect("failed to init repo");

        git(&repo, &["config", "user.email", "test@example.com"]);
        git(&repo, &["config", "user.name", "Test User"]);

        for idx in 0..150 {
            let path = temp_dir.path().join(format!("keep/app-{idx}.vue"));
            fs::create_dir_all(path.parent().unwrap()).expect("failed to create keep dir");
            fs::write(
                path,
                format!(
                    "<template>\n  <div>{{{{ msg{idx} }}}}</div>\n</template>\n\n<script setup>\nconst msg{idx} = 'old'\n</script>\n"
                ),
            )
            .expect("failed to write vue file");
        }
        for idx in 0..12_000 {
            let path = temp_dir
                .path()
                .join(format!("hidden/dir{}/file-{idx}.txt", idx / 100));
            fs::create_dir_all(path.parent().unwrap()).expect("failed to create hidden dir");
            fs::write(path, format!("hidden {idx}\n")).expect("failed to write hidden file");
        }

        git(&repo, &["add", "."]);
        git(&repo, &["commit", "--quiet", "-m", "initial"]);
        git(&repo, &["sparse-checkout", "init", "--cone"]);
        git(&repo, &["sparse-checkout", "set", "keep"]);
        git(&repo, &["sparse-checkout", "reapply", "--sparse-index"]);

        for idx in 0..150 {
            fs::write(
                temp_dir.path().join(format!("keep/app-{idx}.vue")),
                format!(
                    "<template>\n  <div>{{{{ msg{idx} }}}}</div>\n</template>\n\n<script setup>\nconst msg{idx} = 'new'\n</script>\n"
                ),
            )
            .expect("failed to update vue file");
        }

        let highlighter = SyntaxHighlighter::default();
        let capabilities = sparse_index_capabilities();
        let warmup = get_working_tree_diff(&repo, capabilities, &highlighter)
            .expect("failed to warm up sparse diff");
        assert_eq!(warmup.len(), 150);

        let iterations = 5;
        let started = Instant::now();
        let mut files_seen = 0;
        for _ in 0..iterations {
            let files = get_working_tree_diff(&repo, capabilities, &highlighter)
                .expect("failed to get sparse diff");
            files_seen += files.len();
        }
        let elapsed = started.elapsed();
        println!(
            "bench_sparse_checkout_real_path_many_vue_files iterations={iterations} files_per_iteration={} total_ms={:.2} mean_ms={:.2}",
            files_seen / iterations,
            elapsed.as_secs_f64() * 1000.0,
            elapsed.as_secs_f64() * 1000.0 / iterations as f64,
        );
    }
}
