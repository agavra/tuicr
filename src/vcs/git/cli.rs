use std::borrow::Cow;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use chrono::{TimeZone, Utc};

use crate::error::{Result, TuicrError};
use crate::model::{DiffFile, DiffHunk, DiffLine, FileStatus, LineOrigin, LineSide};
use crate::syntax::SyntaxHighlighter;
use crate::vcs::diff_parser::{self, DiffFormat};
use crate::vcs::{ChangeKind, CommitInfo, VcsBackend, VcsChangeStatus, VcsInfo};
use crate::vcs::{container_file_paths, enhance_with_full_file_highlight, tabify};

use super::{
    GitRepoMode, git_bool_config_enabled, git_command_error, git_fsmonitor_config_enabled,
    run_git_command,
};

// Untracked files larger than this are shown in the file list but their
// content is not parsed: they are likely logs, dumps, or build artefacts.
const MAX_UNTRACKED_FILE_SIZE: u64 = 10 * 1_024 * 1_024;
const EMPTY_TREE_OID: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
const COMMIT_FORMAT: &str = "--format=%H%x00%h%x00%an%x00%ct%x00%B%x1e";

#[derive(Debug)]
pub struct GitCliBackend {
    root_path: PathBuf,
    info: VcsInfo,
    repo_mode: GitRepoMode,
    untracked_cache: bool,
    fsmonitor: bool,
}

#[derive(Clone, Copy)]
enum GitContentSource<'a> {
    None,
    Workdir,
    Index,
    Revision(&'a str),
}

impl GitCliBackend {
    pub(super) fn discover_from(cwd: &Path) -> Result<Self> {
        let root_path =
            PathBuf::from(run_git_command(cwd, &["rev-parse", "--show-toplevel"])?.trim());
        let repo_mode = GitRepoMode::detect(&root_path)?;
        let head_commit = run_git_command(&root_path, &["rev-parse", "HEAD"])
            .map(|head| head.trim().to_string())
            .unwrap_or_else(|_| "HEAD".to_string());
        let branch_name =
            run_git_command(&root_path, &["symbolic-ref", "--quiet", "--short", "HEAD"])
                .ok()
                .map(|branch| branch.trim().to_string())
                .filter(|branch| !branch.is_empty());
        let (untracked_cache, fsmonitor) = detect_git_runtime_flags(&root_path);

        let info = VcsInfo {
            root_path: root_path.clone(),
            head_commit,
            branch_name,
            vcs_type: crate::vcs::traits::VcsType::Git,
        };

        Ok(Self {
            root_path,
            info,
            repo_mode,
            untracked_cache,
            fsmonitor,
        })
    }

    pub fn repo_mode(&self) -> GitRepoMode {
        self.repo_mode
    }

    fn get_cli_diff(
        &self,
        args: Vec<String>,
        include_untracked: bool,
        old_source: GitContentSource<'_>,
        new_source: GitContentSource<'_>,
        highlighter: &SyntaxHighlighter,
    ) -> Result<Vec<DiffFile>> {
        let mut files = match run_git_diff_command(&self.root_path, args, highlighter) {
            Ok(files) => files,
            Err(TuicrError::NoChanges) => Vec::new(),
            Err(err) => return Err(err),
        };

        if include_untracked {
            append_untracked_cli_diffs(&self.root_path, &mut files, highlighter)?;
        }
        normalize_git_cli_paths(&mut files);

        if files.is_empty() {
            return Err(TuicrError::NoChanges);
        }

        let old_cache =
            git_source_content_cache(&self.root_path, old_source, &files, LineSide::Old);
        let new_cache =
            git_source_content_cache(&self.root_path, new_source, &files, LineSide::New);

        enhance_with_full_file_highlight(
            &mut files,
            highlighter,
            |path| {
                read_path_from_git_source_cached(
                    &self.root_path,
                    old_source,
                    old_cache.as_ref(),
                    path,
                )
            },
            |path| {
                read_path_from_git_source_cached(
                    &self.root_path,
                    new_source,
                    new_cache.as_ref(),
                    path,
                )
            },
        );
        Ok(files)
    }

    fn read_file_content(
        &self,
        file_path: &Path,
        file_status: FileStatus,
        ref_commit: Option<&str>,
    ) -> Result<String> {
        let path_str = file_path.to_string_lossy();
        if let Some(commit) = ref_commit {
            read_git_object(&self.root_path, &format!("{commit}:{path_str}")).ok_or_else(|| {
                TuicrError::VcsCommand(format!("failed to read {path_str} at {commit}"))
            })
        } else if file_status == FileStatus::Deleted {
            read_git_object(&self.root_path, &format!("HEAD:{path_str}")).ok_or_else(|| {
                TuicrError::VcsCommand("failed to read deleted file from HEAD".into())
            })
        } else {
            Ok(fs::read_to_string(self.root_path.join(file_path))?)
        }
    }
}

impl VcsBackend for GitCliBackend {
    fn info(&self) -> &VcsInfo {
        &self.info
    }

    fn startup_warnings(&self) -> Vec<String> {
        if !self.repo_mode().is_sparse_checkout() {
            return Vec::new();
        }

        let mut warnings = vec!["Sparse checkout detected; using Git CLI backend.".to_string()];

        if !self.untracked_cache {
            let fsmonitor_state = if self.fsmonitor {
                "enabled"
            } else {
                "not enabled"
            };
            warnings.push(format!(
                "Sparse checkout without core.untrackedCache can make untracked scans slow; run `git update-index --test-untracked-cache` then `git config core.untrackedCache true` if it passes (fsmonitor: {fsmonitor_state})."
            ));
        }

        warnings
    }

    fn supports_sparse_checkout(&self) -> bool {
        true
    }

    fn get_working_tree_diff(&self, highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
        self.get_cli_diff(
            strings(["diff", "--no-ext-diff", "--binary", "HEAD", "--"]),
            true,
            GitContentSource::Revision("HEAD"),
            GitContentSource::Workdir,
            highlighter,
        )
    }

    fn get_staged_diff(&self, highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
        let old_source =
            if run_git_command(&self.root_path, &["rev-parse", "--verify", "HEAD"]).is_ok() {
                GitContentSource::Revision("HEAD")
            } else {
                GitContentSource::None
            };
        self.get_cli_diff(
            strings(["diff", "--no-ext-diff", "--binary", "--cached", "--"]),
            false,
            old_source,
            GitContentSource::Index,
            highlighter,
        )
    }

    fn get_unstaged_diff(&self, highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
        self.get_cli_diff(
            strings(["diff", "--no-ext-diff", "--binary", "--"]),
            true,
            GitContentSource::Index,
            GitContentSource::Workdir,
            highlighter,
        )
    }

    fn get_change_status(&self) -> Result<VcsChangeStatus> {
        // Tracked changes have cheap exact probes. Untracked files require a
        // working-tree scan, so only pay that cost when tracked unstaged changes
        // have not already proven the "unstaged" row should be shown.
        let staged = has_diff_changes(&self.root_path, &["diff", "--quiet", "--cached", "--"])?;
        let tracked_unstaged = has_diff_changes(&self.root_path, &["diff", "--quiet", "--"])?;
        let untracked_pathspecs = if tracked_unstaged {
            Vec::new()
        } else {
            sparse_checkout_untracked_pathspecs(&self.root_path)?
        };
        let unstaged =
            tracked_unstaged || has_untracked_changes(&self.root_path, &untracked_pathspecs)?;

        Ok(VcsChangeStatus { staged, unstaged })
    }

    fn list_changed_paths(&self, kind: ChangeKind) -> Result<Vec<PathBuf>> {
        match kind {
            ChangeKind::Staged => list_diff_paths(
                &self.root_path,
                &["diff", "--cached", "--name-only", "-z", "--"],
            ),
            ChangeKind::Unstaged => {
                let mut paths =
                    list_diff_paths(&self.root_path, &["diff", "--name-only", "-z", "--"])?;
                let untracked_pathspecs = sparse_checkout_untracked_pathspecs(&self.root_path)?;
                paths.extend(list_untracked_paths(&self.root_path, &untracked_pathspecs)?);
                Ok(paths)
            }
        }
    }

    fn fetch_context_lines(
        &self,
        file_path: &Path,
        file_status: FileStatus,
        ref_commit: Option<&str>,
        start_line: u32,
        end_line: u32,
    ) -> Result<Vec<DiffLine>> {
        if start_line > end_line || start_line == 0 {
            return Ok(Vec::new());
        }

        let content = self.read_file_content(file_path, file_status, ref_commit)?;

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

    fn file_line_count(
        &self,
        file_path: &Path,
        file_status: FileStatus,
        ref_commit: Option<&str>,
    ) -> Result<u32> {
        let content = self.read_file_content(file_path, file_status, ref_commit)?;
        Ok(content.lines().count() as u32)
    }

    fn get_recent_commits(&self, offset: usize, limit: usize) -> Result<Vec<CommitInfo>> {
        // Unborn HEAD (fresh `git init` / `git clone` of an empty remote):
        // `git log` returns 128 with "does not have any commits yet". Detect
        // that up-front and return an empty list so startup can fall through
        // to the staged/unstaged paths.
        if run_git_command(&self.root_path, &["rev-parse", "--verify", "HEAD"]).is_err() {
            return Ok(Vec::new());
        }
        let branch_tip_names = get_branch_tip_names(&self.root_path);
        let output = run_git_command_args(
            &self.root_path,
            [
                OsStr::new("log"),
                OsStr::new(&format!("--skip={offset}")),
                OsStr::new(&format!("--max-count={limit}")),
                OsStr::new(COMMIT_FORMAT),
            ],
        )?;

        Ok(parse_commit_records(&output, &branch_tip_names))
    }

    fn resolve_revisions(&self, revisions: &str) -> Result<Vec<String>> {
        let commit_ids = if revisions.contains("..") {
            let output = run_git_command(
                &self.root_path,
                &["rev-list", "--topo-order", "--reverse", revisions],
            )?;
            output
                .lines()
                .filter(|line| !line.is_empty())
                .map(str::to_string)
                .collect()
        } else {
            let revision = format!("{revisions}^{{commit}}");
            let output = run_git_command(&self.root_path, &["rev-parse", "--verify", &revision])?;
            vec![output.trim().to_string()]
        };

        if commit_ids.is_empty() {
            return Err(TuicrError::NoChanges);
        }

        Ok(commit_ids)
    }

    fn get_commit_range_diff(
        &self,
        commit_ids: &[String],
        highlighter: &SyntaxHighlighter,
    ) -> Result<Vec<DiffFile>> {
        if commit_ids.is_empty() {
            return Err(TuicrError::NoChanges);
        }

        let base_rev = parent_rev_or_empty(&self.root_path, &commit_ids[0]);
        let newest_rev = commit_ids.last().unwrap();
        self.get_cli_diff(
            vec![
                "diff".into(),
                "--no-ext-diff".into(),
                "--binary".into(),
                base_rev.clone(),
                newest_rev.clone(),
                "--".into(),
            ],
            false,
            GitContentSource::Revision(&base_rev),
            GitContentSource::Revision(newest_rev),
            highlighter,
        )
    }

    fn get_commits_info(&self, ids: &[String]) -> Result<Vec<CommitInfo>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        let branch_tip_names = get_branch_tip_names(&self.root_path);
        let mut args = vec![
            "show".to_string(),
            "-s".to_string(),
            COMMIT_FORMAT.to_string(),
        ];
        args.extend(ids.iter().cloned());
        let output = run_git_command_strings(&self.root_path, args)?;

        Ok(parse_commit_records(&output, &branch_tip_names))
    }

    fn get_working_tree_with_commits_diff(
        &self,
        commit_ids: &[String],
        highlighter: &SyntaxHighlighter,
    ) -> Result<Vec<DiffFile>> {
        if commit_ids.is_empty() {
            return Err(TuicrError::NoChanges);
        }

        let base_rev = parent_rev_or_empty(&self.root_path, &commit_ids[0]);
        self.get_cli_diff(
            vec![
                "diff".into(),
                "--no-ext-diff".into(),
                "--binary".into(),
                base_rev.clone(),
                "--".into(),
            ],
            true,
            GitContentSource::Revision(&base_rev),
            GitContentSource::Workdir,
            highlighter,
        )
    }

    fn stage_file(&self, path: &Path) -> Result<()> {
        let output = Command::new("git")
            .current_dir(&self.root_path)
            .arg("add")
            .arg("--")
            .arg(path)
            .output()
            .map_err(|e| TuicrError::VcsCommand(format!("Failed to run git: {e}")))?;

        if !output.status.success() {
            return Err(TuicrError::VcsCommand(
                String::from_utf8_lossy(&output.stderr).trim().to_string(),
            ));
        }

        Ok(())
    }
}

fn strings<const N: usize>(args: [&str; N]) -> Vec<String> {
    args.into_iter().map(str::to_string).collect()
}

fn detect_git_runtime_flags(workdir: &Path) -> (bool, bool) {
    let output = run_git_command(
        workdir,
        &[
            "config",
            "--get-regexp",
            r"^(core\.untrackedcache|core\.fsmonitor|feature\.manyfiles)$",
        ],
    )
    .unwrap_or_default();

    parse_git_runtime_flags(&output)
}

fn parse_git_runtime_flags(output: &str) -> (bool, bool) {
    let mut untracked_cache = None;
    let mut fsmonitor = false;
    let mut many_files = false;

    for line in output.lines() {
        let mut parts = line.splitn(2, char::is_whitespace);
        let Some(key) = parts.next() else {
            continue;
        };
        let raw_value = parts.next().unwrap_or_default();

        match key {
            "core.untrackedcache" => untracked_cache = Some(git_bool_config_enabled(raw_value)),
            "core.fsmonitor" => fsmonitor = git_fsmonitor_config_enabled(raw_value),
            "feature.manyfiles" => many_files = git_bool_config_enabled(raw_value),
            _ => {}
        }
    }

    // `feature.manyFiles` makes core.untrackedCache default to true, but
    // `git config --get core.untrackedCache` does not print that implied value.
    (untracked_cache.unwrap_or(many_files), fsmonitor)
}

fn has_diff_changes(workdir: &Path, args: &[&str]) -> Result<bool> {
    let output = Command::new("git")
        .current_dir(workdir)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| TuicrError::VcsCommand(format!("Failed to run git: {e}")))?;

    match output.status.code() {
        Some(0) => Ok(false),
        Some(1) => Ok(true),
        _ => Err(TuicrError::VcsCommand(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        )),
    }
}

/// Parse a `\0`-separated NUL-byte stream of paths (the output of e.g.
/// `git diff -z --name-only`) into a Vec<PathBuf>. Empty paths skipped.
fn split_nul_paths(bytes: &[u8]) -> Vec<PathBuf> {
    bytes
        .split(|b| *b == 0)
        .filter(|chunk| !chunk.is_empty())
        .map(|chunk| PathBuf::from(String::from_utf8_lossy(chunk).into_owned()))
        .collect()
}

fn list_diff_paths(workdir: &Path, args: &[&str]) -> Result<Vec<PathBuf>> {
    let output = Command::new("git")
        .current_dir(workdir)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| TuicrError::VcsCommand(format!("Failed to run git: {e}")))?;

    if !output.status.success() {
        return Err(TuicrError::VcsCommand(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }

    Ok(split_nul_paths(&output.stdout))
}

fn list_untracked_paths(workdir: &Path, pathspecs: &[String]) -> Result<Vec<PathBuf>> {
    let mut args: Vec<&str> = vec!["ls-files", "--others", "--exclude-standard", "-z"];
    if !pathspecs.is_empty() {
        args.push("--");
        args.extend(pathspecs.iter().map(String::as_str));
    }

    let output = Command::new("git")
        .current_dir(workdir)
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| TuicrError::VcsCommand(format!("Failed to run git: {e}")))?;

    if !output.status.success() {
        return Err(TuicrError::VcsCommand(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }

    Ok(split_nul_paths(&output.stdout))
}

fn has_untracked_changes(workdir: &Path, pathspecs: &[String]) -> Result<bool> {
    let mut args = vec![
        "ls-files",
        "--others",
        "--exclude-standard",
        "-z",
        "--directory",
    ];
    if !pathspecs.is_empty() {
        args.push("--");
        args.extend(pathspecs.iter().map(String::as_str));
    }

    let mut child = Command::new("git")
        .current_dir(workdir)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| TuicrError::VcsCommand(format!("Failed to run git: {e}")))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| TuicrError::VcsCommand("git ls-files stdout unavailable".into()))?;
    let mut reader = BufReader::new(stdout);
    let mut record = Vec::new();
    if reader.read_until(0, &mut record)? > 0 {
        let _ = child.kill();
        let _ = child.wait();
        return Ok(true);
    }

    let output = child
        .wait_with_output()
        .map_err(|e| TuicrError::VcsCommand(format!("git ls-files failed: {e}")))?;

    if !output.status.success() {
        return Err(TuicrError::VcsCommand(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }

    Ok(false)
}

fn run_git_diff_command(
    workdir: &Path,
    args: Vec<String>,
    highlighter: &SyntaxHighlighter,
) -> Result<Vec<DiffFile>> {
    let mut child = Command::new("git")
        .current_dir(workdir)
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| TuicrError::VcsCommand(format!("Failed to run git: {e}")))?;

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
        .map(|line| line.map(Cow::Owned).map_err(TuicrError::from));
    let parse_result =
        diff_parser::parse_unified_diff_lines(diff_lines, DiffFormat::GitStyle, highlighter);

    let status = child.wait()?;
    let stderr = stderr_reader
        .join()
        .map_err(|_| TuicrError::VcsCommand("git diff stderr reader panicked".into()))?;

    if !status.success() {
        return Err(TuicrError::VcsCommand(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&stderr)
        )));
    }

    parse_result
}

fn append_untracked_cli_diffs(
    workdir: &Path,
    files: &mut Vec<DiffFile>,
    highlighter: &SyntaxHighlighter,
) -> Result<usize> {
    let pathspecs = sparse_checkout_untracked_pathspecs(workdir)?;
    let previous_len = files.len();
    for_each_untracked_path(workdir, &pathspecs, |path| {
        let full_path = workdir.join(&path);
        let Some(file) = build_untracked_diff_file(&path, &full_path, highlighter) else {
            return Ok(());
        };
        files.push(file);
        Ok(())
    })?;
    Ok(files.len().saturating_sub(previous_len))
}

fn sparse_checkout_untracked_pathspecs(workdir: &Path) -> Result<Vec<String>> {
    // Simple cone sparse patterns can narrow `git ls-files --others` to the
    // checked-out cones. Complex patterns fall back to Git's full scan so we do
    // not accidentally hide valid untracked files.
    let output = Command::new("git")
        .current_dir(workdir)
        .args(["sparse-checkout", "list"])
        .output()
        .map_err(|e| TuicrError::VcsCommand(format!("Failed to run git: {e}")))?;

    if !output.status.success() {
        return Ok(Vec::new());
    }

    let mut pathspecs = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let pattern = line.trim();
        if pattern.is_empty() {
            continue;
        }

        if !is_simple_sparse_path(pattern) {
            return Ok(Vec::new());
        }

        let pathspec = pattern.trim_start_matches('/').trim_end_matches('/');
        if !pathspec.is_empty() {
            pathspecs.push(pathspec.to_string());
        }
    }

    Ok(pathspecs)
}

fn is_simple_sparse_path(pattern: &str) -> bool {
    !pattern.starts_with('!')
        && !pattern.contains('*')
        && !pattern.contains('?')
        && !pattern.contains('[')
        && !pattern.contains('\\')
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
        header: format!("@@ -0,0 +1,{new_count} @@"),
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

fn normalize_git_cli_paths(files: &mut [DiffFile]) {
    for file in files {
        match file.status {
            FileStatus::Added if file.old_path.is_none() => {
                file.old_path = file.new_path.clone();
            }
            FileStatus::Deleted if file.new_path.is_none() => {
                file.new_path = file.old_path.clone();
            }
            _ => {}
        }
    }
}

fn for_each_untracked_path<F>(workdir: &Path, pathspecs: &[String], mut visit: F) -> Result<()>
where
    F: FnMut(PathBuf) -> Result<()>,
{
    let mut args = vec!["ls-files", "--others", "--exclude-standard", "-z"];
    if !pathspecs.is_empty() {
        args.push("--");
        args.extend(pathspecs.iter().map(String::as_str));
    }

    let mut child = Command::new("git")
        .current_dir(workdir)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| TuicrError::VcsCommand(format!("Failed to run git: {e}")))?;

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

    let output = child
        .wait_with_output()
        .map_err(|e| TuicrError::VcsCommand(format!("git ls-files failed: {e}")))?;
    if !output.status.success() {
        return Err(TuicrError::VcsCommand(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }

    Ok(())
}

fn read_path_from_git_source(
    workdir: &Path,
    source: GitContentSource<'_>,
    path: &Path,
) -> Option<String> {
    match source {
        GitContentSource::None => None,
        GitContentSource::Workdir => crate::vcs::read_workdir_file(workdir, path),
        GitContentSource::Index => {
            read_git_object(workdir, &format!(":0:{}", path.to_string_lossy()))
        }
        GitContentSource::Revision(rev) => {
            read_git_object(workdir, &format!("{rev}:{}", path.to_string_lossy()))
        }
    }
}

fn read_path_from_git_source_cached(
    workdir: &Path,
    source: GitContentSource<'_>,
    cache: Option<&HashMap<PathBuf, String>>,
    path: &Path,
) -> Option<String> {
    cache
        .and_then(|contents| contents.get(path).cloned())
        .or_else(|| read_path_from_git_source(workdir, source, path))
}

fn git_source_content_cache(
    workdir: &Path,
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
            read_git_objects(workdir, requests).ok()
        }
        GitContentSource::Index => {
            let requests = paths
                .into_iter()
                .map(|path| {
                    let spec = format!(":0:{}", path.to_string_lossy());
                    (path, spec)
                })
                .collect();
            read_git_objects(workdir, requests).ok()
        }
        GitContentSource::None | GitContentSource::Workdir => None,
    }
}

fn read_git_objects(
    workdir: &Path,
    requests: Vec<(PathBuf, String)>,
) -> Result<HashMap<PathBuf, String>> {
    if requests.is_empty() {
        return Ok(HashMap::new());
    }

    let mut child = Command::new("git")
        .current_dir(workdir)
        .args(["cat-file", "--batch"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| TuicrError::VcsCommand(format!("Failed to run git: {e}")))?;

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
            "git cat-file failed with status {status}"
        )));
    }

    Ok(contents)
}

fn read_git_object(workdir: &Path, spec: &str) -> Option<String> {
    run_git_command(workdir, &["show", spec]).ok()
}

fn get_branch_tip_names(workdir: &Path) -> HashMap<String, Vec<String>> {
    let output = run_git_command(
        workdir,
        &[
            "for-each-ref",
            "--format=%(objectname)%00%(refname:short)",
            "refs/heads",
        ],
    )
    .unwrap_or_default();
    let mut names_by_tip: HashMap<String, Vec<String>> = HashMap::new();

    for line in output.lines() {
        if let Some((oid, name)) = line.split_once('\0') {
            names_by_tip
                .entry(oid.to_string())
                .or_default()
                .push(name.to_string());
        }
    }

    for names in names_by_tip.values_mut() {
        names.sort_unstable();
    }

    names_by_tip
}

fn parse_commit_records(
    output: &str,
    branch_tip_names: &HashMap<String, Vec<String>>,
) -> Vec<CommitInfo> {
    output
        .split('\x1e')
        .filter_map(|record| parse_commit_record(record, branch_tip_names))
        .collect()
}

fn parse_commit_record(
    record: &str,
    branch_tip_names: &HashMap<String, Vec<String>>,
) -> Option<CommitInfo> {
    let record = record.trim_start_matches('\n').trim_end_matches('\n');
    if record.is_empty() {
        return None;
    }

    let mut fields = record.splitn(5, '\0');
    let id = fields.next()?.to_string();
    let short_id = fields.next()?.to_string();
    let author = fields.next().unwrap_or("Unknown").to_string();
    let timestamp = fields
        .next()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or_default();
    let full_message = fields.next().unwrap_or("(no message)");
    let (summary, body) = parse_commit_message(full_message);
    let branch_name = branch_tip_names
        .get(&id)
        .and_then(|names| names.first().cloned());
    let time = Utc
        .timestamp_opt(timestamp, 0)
        .single()
        .unwrap_or_else(Utc::now);

    Some(CommitInfo {
        id,
        short_id,
        branch_name,
        summary,
        body,
        author,
        time,
    })
}

fn parse_commit_message(message: &str) -> (String, Option<String>) {
    let mut lines = message.lines();
    let summary = lines.next().unwrap_or("(no message)").to_string();
    let body_text: String = lines
        .skip_while(|l| l.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    let body = if body_text.trim().is_empty() {
        None
    } else {
        Some(body_text)
    };
    (summary, body)
}

fn parent_rev_or_empty(workdir: &Path, commit_id: &str) -> String {
    let parent_spec = format!("{commit_id}^");
    run_git_command(workdir, &["rev-parse", &parent_spec])
        .map(|rev| rev.trim().to_string())
        .unwrap_or_else(|_| EMPTY_TREE_OID.to_string())
}

fn run_git_command_strings(workdir: &Path, args: Vec<String>) -> Result<String> {
    run_git_command_args(workdir, args.iter().map(String::as_str))
}

fn run_git_command_args<I, S>(workdir: &Path, args: I) -> Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    crate::process::run_command_output("git", Some(workdir), args).map_err(git_command_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vcs::git::{diff, repository};

    fn git(workdir: &Path, args: &[&str]) {
        let output = Command::new("git")
            .current_dir(workdir)
            .args(args)
            .output()
            .expect("failed to run git");
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn write_file(workdir: &Path, path: &str, content: &str) {
        let full_path = workdir.join(path);
        fs::create_dir_all(full_path.parent().expect("test path should have parent"))
            .expect("failed to create parent");
        fs::write(full_path, content).expect("failed to write file");
    }

    fn remove_file(workdir: &Path, path: &str) {
        fs::remove_file(workdir.join(path)).expect("failed to remove file");
    }

    fn summarize_files(
        files: Vec<DiffFile>,
    ) -> Vec<(Option<PathBuf>, Option<PathBuf>, FileStatus)> {
        let mut summary: Vec<_> = files
            .into_iter()
            .map(|file| (file.old_path, file.new_path, file.status))
            .collect();
        summary.sort_by(|left, right| {
            left.0
                .cmp(&right.0)
                .then_with(|| left.1.cmp(&right.1))
                .then_with(|| left.2.as_char().cmp(&right.2.as_char()))
        });
        summary
    }

    fn setup_sparse_index_repo() -> (tempfile::TempDir, GitCliBackend, Vec<String>) {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let workdir = temp_dir.path();

        git(workdir, &["init"]);
        git(workdir, &["config", "user.email", "test@example.com"]);
        git(workdir, &["config", "user.name", "Test User"]);
        write_file(workdir, "keep/file.txt", "keep base\n");
        write_file(workdir, "hidden/file.txt", "hidden base\n");
        git(workdir, &["add", "."]);
        git(workdir, &["commit", "-m", "initial"]);
        let first_id = run_git_command(workdir, &["rev-parse", "HEAD"])
            .expect("failed to resolve first commit")
            .trim()
            .to_string();

        write_file(workdir, "keep/file.txt", "keep next\n");
        git(workdir, &["add", "."]);
        git(workdir, &["commit", "-m", "second"]);
        let second_id = run_git_command(workdir, &["rev-parse", "HEAD"])
            .expect("failed to resolve second commit")
            .trim()
            .to_string();

        git(workdir, &["sparse-checkout", "init", "--cone"]);
        git(workdir, &["sparse-checkout", "set", "keep"]);
        git(workdir, &["sparse-checkout", "reapply", "--sparse-index"]);
        git(workdir, &["config", "advice.sparseIndexExpanded", "false"]);

        let backend = GitCliBackend::discover_from(workdir).expect("failed to discover backend");
        (temp_dir, backend, vec![first_id, second_id])
    }

    fn setup_standard_parity_repo() -> (
        tempfile::TempDir,
        GitCliBackend,
        git2::Repository,
        Vec<String>,
    ) {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let workdir = temp_dir.path();

        git(workdir, &["init"]);
        git(workdir, &["config", "user.email", "test@example.com"]);
        git(workdir, &["config", "user.name", "Test User"]);
        write_file(workdir, "modified.txt", "modified base\n");
        write_file(workdir, "deleted.txt", "deleted base\n");
        write_file(workdir, "staged.txt", "staged base\n");
        write_file(workdir, "range.txt", "range base\n");
        git(workdir, &["add", "."]);
        git(workdir, &["commit", "-m", "initial"]);
        let first_id = run_git_command(workdir, &["rev-parse", "HEAD"])
            .expect("failed to resolve first commit")
            .trim()
            .to_string();

        write_file(workdir, "range.txt", "range changed\n");
        git(workdir, &["add", "range.txt"]);
        git(workdir, &["commit", "-m", "second"]);
        let second_id = run_git_command(workdir, &["rev-parse", "HEAD"])
            .expect("failed to resolve second commit")
            .trim()
            .to_string();

        write_file(workdir, "modified.txt", "modified changed\n");
        remove_file(workdir, "deleted.txt");
        write_file(workdir, "staged.txt", "staged changed\n");
        git(workdir, &["add", "staged.txt"]);
        write_file(workdir, "untracked.txt", "untracked\n");

        let cli_backend =
            GitCliBackend::discover_from(workdir).expect("failed to discover cli backend");
        let repo = git2::Repository::open(workdir).expect("failed to open git2 repo");
        (temp_dir, cli_backend, repo, vec![first_id, second_id])
    }

    #[test]
    fn parses_runtime_flags_from_single_config_read() {
        let output = "core.untrackedcache true\ncore.fsmonitor .git/hooks/fsmonitor-watchman\n";

        assert_eq!(parse_git_runtime_flags(output), (true, true));
    }

    #[test]
    fn treats_feature_many_files_as_untracked_cache_default() {
        assert_eq!(
            parse_git_runtime_flags("feature.manyfiles true\n"),
            (true, false)
        );
        assert_eq!(
            parse_git_runtime_flags("feature.manyfiles true\ncore.untrackedcache keep\n"),
            (false, false)
        );
    }

    #[test]
    fn discovers_sparse_index_repo_mode() {
        let (_temp_dir, backend, _ids) = setup_sparse_index_repo();

        assert_eq!(backend.repo_mode(), GitRepoMode::SparseIndex);
    }

    #[test]
    fn gets_recent_commits_in_sparse_index() {
        let (_temp_dir, backend, _ids) = setup_sparse_index_repo();

        let commits = backend
            .get_recent_commits(0, 10)
            .expect("failed to get commits");

        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].summary, "second");
        assert_eq!(commits[1].summary, "initial");
    }

    #[test]
    fn resolves_revisions_in_sparse_index() {
        let (_temp_dir, backend, ids) = setup_sparse_index_repo();
        let revset = format!("{}..{}", ids[0], ids[1]);

        let resolved = backend
            .resolve_revisions(&revset)
            .expect("failed to resolve revisions");

        assert_eq!(resolved, vec![ids[1].clone()]);
    }

    #[test]
    fn reads_commit_range_diff_in_sparse_index() {
        let (_temp_dir, backend, ids) = setup_sparse_index_repo();

        let files = backend
            .get_commit_range_diff(&[ids[1].clone()], &SyntaxHighlighter::default())
            .expect("failed to get sparse commit range diff");

        assert_eq!(files.len(), 1);
        assert_eq!(
            files[0].new_path.as_deref(),
            Some(Path::new("keep/file.txt"))
        );
    }

    #[test]
    fn returns_no_changes_for_clean_sparse_index() {
        let (_temp_dir, backend, _ids) = setup_sparse_index_repo();

        assert!(matches!(
            backend.get_working_tree_diff(&SyntaxHighlighter::default()),
            Err(TuicrError::NoChanges)
        ));
    }

    #[test]
    fn reads_working_tree_diff_and_untracked_files_in_sparse_index() {
        let (temp_dir, backend, _ids) = setup_sparse_index_repo();
        let workdir = temp_dir.path();
        write_file(workdir, "keep/file.txt", "keep changed\n");
        write_file(workdir, "keep/new.txt", "new sparse file\n");
        write_file(workdir, "hidden/outside.txt", "outside cone\n");

        let files = backend
            .get_working_tree_diff(&SyntaxHighlighter::default())
            .expect("failed to get sparse working tree diff");

        let paths: Vec<_> = files
            .iter()
            .filter_map(|file| file.new_path.as_deref())
            .collect();
        assert!(paths.contains(&Path::new("keep/file.txt")));
        assert!(paths.contains(&Path::new("keep/new.txt")));
        assert!(!paths.contains(&Path::new("hidden/outside.txt")));
    }

    #[test]
    fn reads_staged_diff_and_stages_files_in_sparse_index() {
        let (temp_dir, backend, _ids) = setup_sparse_index_repo();
        let workdir = temp_dir.path();
        write_file(workdir, "keep/file.txt", "keep staged\n");

        backend
            .stage_file(Path::new("keep/file.txt"))
            .expect("failed to stage file");
        let files = backend
            .get_staged_diff(&SyntaxHighlighter::default())
            .expect("failed to get sparse staged diff");

        assert_eq!(files.len(), 1);
        assert_eq!(
            files[0].new_path.as_deref(),
            Some(Path::new("keep/file.txt"))
        );
    }

    #[test]
    fn detects_change_status_without_loading_diff() {
        let (temp_dir, backend, _ids) = setup_sparse_index_repo();
        write_file(temp_dir.path(), "keep/file.txt", "keep modified\n");

        let status = backend
            .get_change_status()
            .expect("failed to get change status");

        assert_eq!(
            status,
            VcsChangeStatus {
                staged: false,
                unstaged: true,
            }
        );
    }

    #[test]
    fn ignores_untracked_files_outside_sparse_cone_in_change_status() {
        let (temp_dir, backend, _ids) = setup_sparse_index_repo();
        write_file(temp_dir.path(), "hidden/outside.txt", "outside cone\n");

        let status = backend
            .get_change_status()
            .expect("failed to get change status");

        assert_eq!(status, VcsChangeStatus::default());
    }

    #[test]
    fn detects_untracked_files_inside_sparse_cone_in_change_status() {
        let (temp_dir, backend, _ids) = setup_sparse_index_repo();
        write_file(temp_dir.path(), "keep/new.txt", "inside cone\n");

        let status = backend
            .get_change_status()
            .expect("failed to get change status");

        assert_eq!(
            status,
            VcsChangeStatus {
                staged: false,
                unstaged: true,
            }
        );
    }

    #[test]
    fn cli_diff_outputs_match_libgit2_for_shared_git_operations() {
        let (_temp_dir, cli_backend, repo, ids) = setup_standard_parity_repo();
        let highlighter = SyntaxHighlighter::default();

        assert_eq!(
            summarize_files(cli_backend.get_working_tree_diff(&highlighter).unwrap()),
            summarize_files(diff::get_working_tree_diff(&repo, &highlighter).unwrap())
        );
        assert_eq!(
            summarize_files(cli_backend.get_staged_diff(&highlighter).unwrap()),
            summarize_files(diff::get_staged_diff(&repo, &highlighter).unwrap())
        );
        assert_eq!(
            summarize_files(cli_backend.get_unstaged_diff(&highlighter).unwrap()),
            summarize_files(diff::get_unstaged_diff(&repo, &highlighter).unwrap())
        );
        assert_eq!(
            summarize_files(
                cli_backend
                    .get_commit_range_diff(&[ids[1].clone()], &highlighter)
                    .unwrap()
            ),
            summarize_files(
                diff::get_commit_range_diff(&repo, &[ids[1].clone()], &highlighter).unwrap()
            )
        );
        assert_eq!(
            summarize_files(
                cli_backend
                    .get_working_tree_with_commits_diff(&[ids[1].clone()], &highlighter)
                    .unwrap()
            ),
            summarize_files(
                diff::get_working_tree_with_commits_diff(&repo, &[ids[1].clone()], &highlighter)
                    .unwrap()
            )
        );

        let cli_commits = cli_backend.get_recent_commits(0, 10).unwrap();
        let libgit2_commits = repository::get_recent_commits(&repo, 0, 10).unwrap();
        assert_eq!(
            cli_commits
                .iter()
                .map(|commit| (&commit.id, &commit.summary, &commit.body))
                .collect::<Vec<_>>(),
            libgit2_commits
                .iter()
                .map(|commit| (&commit.id, &commit.summary, &commit.body))
                .collect::<Vec<_>>()
        );

        let revset = format!("{}..{}", ids[0], ids[1]);
        assert_eq!(
            cli_backend.resolve_revisions(&revset).unwrap(),
            repository::resolve_revisions(&repo, &revset).unwrap()
        );

        let cli_commit_info = cli_backend.get_commits_info(&ids).unwrap();
        let libgit2_commit_info = repository::get_commits_info(&repo, &ids).unwrap();
        assert_eq!(
            cli_commit_info
                .iter()
                .map(|commit| (&commit.id, &commit.summary, &commit.body))
                .collect::<Vec<_>>(),
            libgit2_commit_info
                .iter()
                .map(|commit| (&commit.id, &commit.summary, &commit.body))
                .collect::<Vec<_>>()
        );
    }
}
