//! VCS abstraction layer for supporting multiple version control systems.
//!
//! Currently supports:
//! - Git
//! - Mercurial
//! - Jujutsu
//!
//! ## Detection Order
//!
//! When auto-detecting the VCS type, Jujutsu is tried first because jj repos
//! are Git-backed and contain a `.git` directory. If jj detection fails, Git
//! is tried next, then Mercurial.

mod diff_parser;
pub mod file;
pub mod git;
mod hg;
mod jj;
pub(crate) mod traits;

pub use file::FileBackend;
pub use git::GitBackend;
pub use hg::HgBackend;
pub use jj::JjBackend;
pub use traits::{CommitInfo, VcsBackend, VcsInfo};

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::error::{Result, TuicrError};
use crate::model::{DiffFile, LineSide};
use crate::syntax::{
    HighlightedLines, HighlightedSpans, SyntaxHighlighter, needs_full_file_highlight,
};

/// Boundary marker emitted between files in batched `hg cat` / `jj file show`
/// output. The long random suffix makes accidental collision with real source
/// content effectively impossible.
pub(crate) const BATCH_BOUNDARY: &str = "@@TUICR_BATCH_BOUNDARY_e97f2d44_8b1a@@";

/// Collect the unique paths of files that need full-file syntax highlighting
/// (Vue, Svelte, PHP and friends) on the given side, skipping binary, too-large,
/// or empty entries. Used by hg / jj to know which files to batch-fetch.
pub(crate) fn container_file_paths(files: &[DiffFile], side: LineSide) -> Vec<PathBuf> {
    files
        .iter()
        .filter(|f| !f.is_binary && !f.is_too_large && !f.hunks.is_empty())
        .filter_map(|f| {
            let syntax_path = f.new_path.as_deref().or(f.old_path.as_deref())?;
            if !needs_full_file_highlight(syntax_path) {
                return None;
            }
            match side {
                LineSide::Old => f.old_path.clone(),
                LineSide::New => f.new_path.clone(),
            }
        })
        .collect()
}

/// Expand tabs to spaces in diff line content so highlighted spans line up
/// with the displayed text in side-by-side and unified rendering.
pub(crate) fn tabify(s: &str) -> String {
    s.replace('\t', "    ")
}

/// Read a file from the working tree, returning `None` on any IO error.
pub(crate) fn read_workdir_file(root: &Path, rel: &Path) -> Option<String> {
    std::fs::read_to_string(root.join(rel)).ok()
}

/// Parse the output of a batched `hg cat` / `jj file show` invocation whose
/// template prefixed each file with `\n{BATCH_BOUNDARY}\n{path}\n` before
/// emitting `{data}`. Returns a `path → data` map.
pub(crate) fn parse_batched_files(output: &str) -> HashMap<PathBuf, String> {
    let sep = format!("\n{BATCH_BOUNDARY}\n");
    output
        .split(&sep)
        .filter(|s| !s.is_empty())
        .filter_map(|block| {
            let mut iter = block.splitn(2, '\n');
            let path = iter.next()?;
            let data = iter.next().unwrap_or("");
            Some((PathBuf::from(path), data.to_string()))
        })
        .collect()
}

/// Re-highlight container-grammar files (Vue, Svelte, etc) using their full
/// content at the requested revisions. `new_rev = None` reads the new side
/// from the working tree on disk instead of calling `fetch_batch`. The
/// `fetch_batch` closure is the backend-specific batched-fetch primitive
/// (`hg cat -r REV ...` or `jj file show -r REV ...`).
pub(crate) fn apply_container_full_file_highlight<F>(
    root: &Path,
    old_rev: &str,
    new_rev: Option<&str>,
    files: &mut [DiffFile],
    highlighter: &SyntaxHighlighter,
    fetch_batch: F,
) -> Result<()>
where
    F: Fn(&Path, &str, &[PathBuf]) -> Result<HashMap<PathBuf, String>>,
{
    let old_paths = container_file_paths(files, LineSide::Old);
    let new_paths = container_file_paths(files, LineSide::New);

    if old_paths.is_empty() && new_paths.is_empty() {
        return Ok(());
    }

    let old_map = fetch_batch(root, old_rev, &old_paths)?;
    let new_map = match new_rev {
        Some(rev) => fetch_batch(root, rev, &new_paths)?,
        None => HashMap::new(),
    };

    let workdir = new_rev.is_none().then(|| root.to_path_buf());

    enhance_with_full_file_highlight(
        files,
        highlighter,
        |p| old_map.get(p).cloned(),
        |p| match (new_map.get(p), workdir.as_deref()) {
            (Some(content), _) => Some(content.clone()),
            (None, Some(root)) => read_workdir_file(root, p),
            (None, None) => None,
        },
    );

    Ok(())
}

/// Files larger than this skip the full-file highlight pass and fall back to
/// per-hunk highlighting. Keeps a runaway-cost ceiling on diffs that include
/// huge generated artefacts (lockfiles, vendored bundles, fixtures).
const MAX_HIGHLIGHT_FILE_BYTES: usize = 1024 * 1024;

/// Re-highlight each diff line using full-file context, for files whose
/// grammar needs it (Vue, Svelte, Astro, MDX). Other files keep their existing
/// per-hunk highlighting unchanged.
///
/// `fetch_old`/`fetch_new` return the entire content of the file at the old
/// and new sides respectively (or `None` if unavailable). When a side is
/// available, every diff line on that side is replaced with the span at its
/// 1-based lineno from the full-file highlight. Lines whose side could not be
/// fetched keep whatever the parser already assigned.
pub(crate) fn enhance_with_full_file_highlight<F, G>(
    files: &mut [DiffFile],
    highlighter: &SyntaxHighlighter,
    mut fetch_old: F,
    mut fetch_new: G,
) where
    F: FnMut(&Path) -> Option<String>,
    G: FnMut(&Path) -> Option<String>,
{
    for file in files.iter_mut() {
        if file.is_binary || file.is_too_large || file.hunks.is_empty() {
            continue;
        }
        let Some(syntax_path) = file.new_path.as_deref().or(file.old_path.as_deref()) else {
            continue;
        };
        if !needs_full_file_highlight(syntax_path) {
            continue;
        }

        let old_highlight = file
            .old_path
            .as_deref()
            .and_then(&mut fetch_old)
            .and_then(|c| highlight_content(highlighter, syntax_path, &c));
        let new_highlight = file
            .new_path
            .as_deref()
            .and_then(&mut fetch_new)
            .and_then(|c| highlight_content(highlighter, syntax_path, &c));

        if old_highlight.is_none() && new_highlight.is_none() {
            continue;
        }

        apply_full_file_spans(
            file,
            highlighter,
            old_highlight.as_deref(),
            new_highlight.as_deref(),
        );
    }
}

fn highlight_content(
    highlighter: &SyntaxHighlighter,
    path: &Path,
    content: &str,
) -> Option<HighlightedLines> {
    if content.len() > MAX_HIGHLIGHT_FILE_BYTES || content.as_bytes().contains(&0u8) {
        return None;
    }
    let lines: Vec<String> = content.lines().map(tabify).collect();
    highlighter.highlight_file_lines(path, &lines)
}

fn apply_full_file_spans(
    file: &mut DiffFile,
    highlighter: &SyntaxHighlighter,
    old_highlight: Option<&[Option<HighlightedSpans>]>,
    new_highlight: Option<&[Option<HighlightedSpans>]>,
) {
    for hunk in &mut file.hunks {
        for line in &mut hunk.lines {
            let old_idx = line.old_lineno.map(|n| n.saturating_sub(1) as usize);
            let new_idx = line.new_lineno.map(|n| n.saturating_sub(1) as usize);
            let spans = highlighter.highlighted_line_for_diff_with_background(
                old_highlight,
                new_highlight,
                old_idx,
                new_idx,
                line.origin,
            );
            if spans.is_some() {
                line.highlighted_spans = spans;
            }
        }
    }
}

/// Detect the VCS type and return the appropriate backend.
///
/// Detection order: Jujutsu → Git → Mercurial.
/// Jujutsu is tried first because jj repos are Git-backed.
pub fn detect_vcs() -> Result<Box<dyn VcsBackend>> {
    // Try jj first since jj repos are Git-backed
    if let Ok(backend) = JjBackend::discover() {
        return Ok(Box::new(backend));
    }

    // Try git
    if let Ok(backend) = GitBackend::discover() {
        return Ok(Box::new(backend));
    }

    // Try hg
    if let Ok(backend) = HgBackend::discover() {
        return Ok(Box::new(backend));
    }

    Err(TuicrError::NotARepository)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vcs::traits::VcsType;
    use std::path::PathBuf;

    #[test]
    fn exports_are_accessible() {
        // Verify that public types are properly exported
        let _: fn() -> Result<Box<dyn VcsBackend>> = detect_vcs;

        // VcsInfo can be constructed
        let info = VcsInfo {
            root_path: PathBuf::from("/test"),
            head_commit: "abc".to_string(),
            branch_name: None,
            vcs_type: VcsType::Git,
        };
        assert_eq!(info.head_commit, "abc");

        // CommitInfo can be constructed
        let commit = CommitInfo {
            id: "abc".to_string(),
            short_id: "abc".to_string(),
            branch_name: Some("main".to_string()),
            summary: "test".to_string(),
            body: None,
            author: "author".to_string(),
            time: chrono::Utc::now(),
        };
        assert_eq!(commit.id, "abc");
    }

    #[test]
    fn detect_vcs_outside_repo_returns_error() {
        // When run outside any VCS repo, should return NotARepository
        // Note: This test may pass or fail depending on where tests are run
        // In CI or outside a repo, it should fail with NotARepository
        // Inside the tuicr repo (which is git), it will succeed
        let result = detect_vcs();

        // We just verify the function runs without panic
        // The actual result depends on the environment
        match result {
            Ok(backend) => {
                // If we're in a repo, we should get valid info
                let info = backend.info();
                assert!(!info.head_commit.is_empty());
            }
            Err(TuicrError::NotARepository) => {
                // Expected when outside a repo
            }
            Err(e) => {
                panic!("Unexpected error: {e:?}");
            }
        }
    }
}
