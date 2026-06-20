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

pub(crate) mod diff_parser;
pub mod file;
pub mod git;
mod hg;
mod jj;
pub mod pr_noop;
pub mod pristine;
pub(crate) mod traits;

pub use file::FileBackend;
pub use git::{GitBackend, GitBackendPreference};
pub use hg::HgBackend;
pub use jj::JjBackend;
pub use pr_noop::PrNoopVcs;
pub use traits::{
    ChangeKind, CommitInfo, DiffWhitespaceMode, DiffWithJobs, ResolvedRevisionRange,
    RevisionDiffTarget, VcsBackend, VcsChangeStatus, VcsInfo,
};

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::error::{Result, TuicrError};
use crate::model::{DiffFile, LineSide};
use crate::syntax::{HighlightJob, HighlightJobKind, needs_full_file_highlight};

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

/// Drop files for which `keep` returns false, then re-index highlight jobs to
/// match the kept files' new positions. Jobs whose file was dropped are also
/// dropped. Order of kept files (and remaining jobs) is preserved.
pub fn filter_diff_with_jobs(diff: DiffWithJobs, keep: impl Fn(&DiffFile) -> bool) -> DiffWithJobs {
    let (files, mut jobs) = diff;
    let mut remap: HashMap<usize, usize> = HashMap::with_capacity(files.len());
    let mut kept: Vec<DiffFile> = Vec::with_capacity(files.len());
    for (old_idx, file) in files.into_iter().enumerate() {
        if keep(&file) {
            remap.insert(old_idx, kept.len());
            kept.push(file);
        }
    }
    jobs.retain_mut(|job| match remap.get(&job.file_idx) {
        Some(&new_idx) => {
            job.file_idx = new_idx;
            true
        }
        None => false,
    });
    (kept, jobs)
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

/// Batch-fetch container-grammar files from a VCS at a given revision, then
/// emit a `FullFile` highlight job for each. `new_rev = None` reads the new
/// side from disk instead of calling `fetch_batch`. The `fetch_batch` closure
/// is the backend-specific batched-fetch primitive (`hg cat -r REV ...` or
/// `jj file show -r REV ...`).
///
/// The fetch happens on the parse thread because backends here drive
/// subprocesses; the actual highlighting runs later on the worker thread.
pub(crate) fn append_container_full_file_jobs_from_rev<F>(
    root: &Path,
    old_rev: &str,
    new_rev: Option<&str>,
    files: &[DiffFile],
    jobs: &mut Vec<HighlightJob>,
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

    append_container_full_file_jobs(
        files,
        jobs,
        |p| old_map.get(p).cloned(),
        |p| match (new_map.get(p), workdir.as_deref()) {
            (Some(content), _) => Some(content.clone()),
            (None, Some(root)) => read_workdir_file(root, p),
            (None, None) => None,
        },
    );

    Ok(())
}

/// Walk `files` and append one `FullFile` highlight job per container-grammar
/// file (Vue, Svelte, ...). The closures fetch the file's full content on
/// each side; either may return `None` if that side is unavailable.
///
/// Fetching happens here, in the parse phase, because the closures may close
/// over VCS state that is `!Send` (e.g. `git2::Repository`, an `hg` child
/// process). The highlighting itself runs later on the worker thread, against
/// the `String`s embedded in the job.
pub(crate) fn append_container_full_file_jobs<F, G>(
    files: &[DiffFile],
    jobs: &mut Vec<HighlightJob>,
    mut fetch_old: F,
    mut fetch_new: G,
) where
    F: FnMut(&Path) -> Option<String>,
    G: FnMut(&Path) -> Option<String>,
{
    for (idx, file) in files.iter().enumerate() {
        if file.is_binary || file.is_too_large || file.hunks.is_empty() {
            continue;
        }
        let Some(syntax_path) = file.new_path.as_deref().or(file.old_path.as_deref()) else {
            continue;
        };
        if !needs_full_file_highlight(syntax_path) {
            continue;
        }
        let old_content = file.old_path.as_deref().and_then(&mut fetch_old);
        let new_content = file.new_path.as_deref().and_then(&mut fetch_new);
        if old_content.is_none() && new_content.is_none() {
            continue;
        }
        jobs.push(HighlightJob {
            file_idx: idx,
            syntax_path: syntax_path.to_path_buf(),
            kind: HighlightJobKind::FullFile {
                old_content,
                new_content,
            },
        });
    }
}

/// Detect the VCS type and return the appropriate backend.
///
/// Detection order: Jujutsu → Git → Mercurial.
/// Jujutsu is tried first because jj repos are Git-backed.
pub fn detect_vcs(
    git_backend_preference: GitBackendPreference,
    whitespace_mode: DiffWhitespaceMode,
) -> Result<Box<dyn VcsBackend>> {
    // Try jj first since jj repos are Git-backed
    if let Ok(backend) = JjBackend::discover(whitespace_mode) {
        return Ok(Box::new(backend));
    }

    // Try git
    if let Ok(backend) = GitBackend::discover(git_backend_preference, whitespace_mode) {
        return Ok(Box::new(backend));
    }

    // Try hg
    if let Ok(backend) = HgBackend::discover(whitespace_mode) {
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
        let _: fn(GitBackendPreference, DiffWhitespaceMode) -> Result<Box<dyn VcsBackend>> =
            detect_vcs;

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
        let result = detect_vcs(GitBackendPreference::Libgit2, DiffWhitespaceMode::Normal);

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

    fn vue_diff_file(
        idx: usize,
        deleted_line: &str,
        added_line: &str,
        target_line: u32,
    ) -> DiffFile {
        use crate::model::diff_types::{DiffHunk, DiffLine, FileStatus, LineOrigin};
        let path = PathBuf::from(format!("Comp{idx}.vue"));
        let hunk = DiffHunk {
            header: format!("@@ -{target_line} +{target_line} @@"),
            lines: vec![
                DiffLine {
                    origin: LineOrigin::Deletion,
                    content: deleted_line.to_string(),
                    old_lineno: Some(target_line),
                    new_lineno: None,
                    highlighted_spans: None,
                },
                DiffLine {
                    origin: LineOrigin::Addition,
                    content: added_line.to_string(),
                    old_lineno: None,
                    new_lineno: Some(target_line),
                    highlighted_spans: None,
                },
            ],
            old_start: target_line,
            old_count: 1,
            new_start: target_line,
            new_count: 1,
        };
        DiffFile {
            old_path: Some(path.clone()),
            new_path: Some(path),
            status: FileStatus::Modified,
            hunks: vec![hunk],
            is_binary: false,
            is_too_large: false,
            is_commit_message: false,
            content_hash: 0,
        }
    }

    fn make_vue_file(idx: usize) -> (DiffFile, String, String) {
        let old = "<template>\n  <div>{{ msg }}</div>\n</template>\n\n<script setup>\n\
                   import { ref } from 'vue'\nconst msg = ref('hi')\nconst other = 1\n</script>\n";
        let new = "<template>\n  <div>{{ msg }}</div>\n</template>\n\n<script setup>\n\
                   import { ref } from 'vue'\nconst msg = ref('hello')\nconst other = 1\n</script>\n";
        let file = vue_diff_file(idx, "const msg = ref('hi')", "const msg = ref('hello')", 7);
        (file, old.to_string(), new.to_string())
    }

    fn highlight_n_vue_files(n: usize) -> Vec<DiffFile> {
        use crate::syntax::SyntaxHighlighter;
        use crate::syntax::streaming::{apply_update, run_blocking};

        let mut files = Vec::with_capacity(n);
        let mut content_map: HashMap<PathBuf, (String, String)> = HashMap::new();
        for i in 0..n {
            let (file, old, new) = make_vue_file(i);
            let path = file.new_path.clone().unwrap();
            content_map.insert(path, (old, new));
            files.push(file);
        }

        let highlighter = SyntaxHighlighter::default();
        let mut jobs = Vec::new();
        append_container_full_file_jobs(
            &files,
            &mut jobs,
            |p| content_map.get(p).map(|(o, _)| o.clone()),
            |p| content_map.get(p).map(|(_, n)| n.clone()),
        );
        for update in run_blocking(jobs, &highlighter) {
            apply_update(&mut files, &highlighter, update);
        }
        files
    }

    fn assert_all_lines_highlighted(files: &[DiffFile]) {
        for (i, file) in files.iter().enumerate() {
            for line in &file.hunks[0].lines {
                let spans = line.highlighted_spans.as_ref().unwrap_or_else(|| {
                    panic!(
                        "file {i} line {:?} should have highlighted spans",
                        line.content
                    )
                });
                let unique_fgs: std::collections::HashSet<_> =
                    spans.iter().filter_map(|(s, _)| s.fg).collect();
                assert!(
                    unique_fgs.len() > 1,
                    "file {i} line {:?} should have multiple distinct fg colors, got {unique_fgs:?}",
                    line.content
                );
            }
        }
    }

    #[test]
    fn streaming_full_file_highlight_serial_one_file() {
        // Single file takes the serial branch in the streaming worker.
        let files = highlight_n_vue_files(1);
        assert_all_lines_highlighted(&files);
    }

    #[test]
    fn streaming_full_file_highlight_parallel_many_files() {
        // 12 files exceeds typical parallelism, forcing the scoped pool.
        let files = highlight_n_vue_files(12);
        assert_eq!(files.len(), 12);
        assert_all_lines_highlighted(&files);
    }

    #[test]
    fn streaming_full_file_highlight_results_match_input_order() {
        // Each file's highlighted spans must land on that file's hunk lines,
        // not a neighbour's. Distinguishable by line content.
        let files = highlight_n_vue_files(6);
        for (i, file) in files.iter().enumerate() {
            let path = file.new_path.as_ref().unwrap();
            assert_eq!(path.to_str().unwrap(), format!("Comp{i}.vue"));
            let added = file
                .hunks
                .iter()
                .flat_map(|h| &h.lines)
                .find(|l| l.origin == crate::model::diff_types::LineOrigin::Addition)
                .expect("addition line");
            assert!(
                added.highlighted_spans.is_some(),
                "file {i} addition unhighlighted"
            );
        }
    }
}
