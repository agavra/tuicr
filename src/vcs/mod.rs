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

use std::path::Path;

use crate::error::{Result, TuicrError};
use crate::model::{DiffFile, LineOrigin};
use crate::syntax::{
    HighlightedLines, HighlightedSpans, SyntaxHighlighter, needs_full_file_highlight,
};

/// Context size used by hg / jj diff calls when we want the diff text to act
/// as the file's full content. Generous enough to cover any reasonable source
/// file end-to-end, so the unified-diff parser sees every line as a context,
/// addition, or deletion entry.
pub(crate) const FULL_FILE_CONTEXT: usize = 1_000_000;

/// Expand tabs to spaces in diff line content so highlighted spans line up
/// with the displayed text in side-by-side and unified rendering.
pub(crate) fn tabify(s: &str) -> String {
    s.replace('\t', "    ")
}

/// Read a file from the working tree, returning `None` on any IO error.
pub(crate) fn read_workdir_file(root: &Path, rel: &Path) -> Option<String> {
    std::fs::read_to_string(root.join(rel)).ok()
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

/// Re-highlight container-grammar files in place by reconstructing the old and
/// new file content directly from the diff's own hunks. Assumes the diff was
/// generated with `FULL_FILE_CONTEXT` so that every line of the file appears
/// as a context, addition, or deletion entry. Avoids any extra subprocess.
pub(crate) fn enhance_with_full_context_diff(
    files: &mut [DiffFile],
    highlighter: &SyntaxHighlighter,
) {
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

        let old_content = reconstruct_side(file, false);
        let new_content = reconstruct_side(file, true);
        let old_highlight = old_content
            .as_deref()
            .and_then(|c| highlight_content(highlighter, syntax_path, c));
        let new_highlight = new_content
            .as_deref()
            .and_then(|c| highlight_content(highlighter, syntax_path, c));

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

/// Concatenate the side-relevant lines from a DiffFile's hunks into a single
/// string. Pass `new_side = true` for additions + context (the new file),
/// `false` for deletions + context (the old file).
fn reconstruct_side(file: &DiffFile, new_side: bool) -> Option<String> {
    let mut lines: Vec<&str> = Vec::new();
    for hunk in &file.hunks {
        for line in &hunk.lines {
            let include = matches!(
                (line.origin, new_side),
                (LineOrigin::Context, _)
                    | (LineOrigin::Addition, true)
                    | (LineOrigin::Deletion, false)
            );
            if include {
                lines.push(&line.content);
            }
        }
    }
    if lines.is_empty() {
        return None;
    }
    Some(lines.join("\n"))
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
