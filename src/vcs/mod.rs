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
    ChangeKind, CommitInfo, DiffWhitespaceMode, ResolvedRevisionRange, RevisionDiffTarget,
    VcsBackend, VcsChangeStatus, VcsInfo,
};

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::error::{Result, TuicrError};
use crate::model::{DiffFile, DiffHunk, DiffLine, LineColoring, LineOrigin, LineSide};
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

/// Slice `[start_line, end_line]` (1-indexed, inclusive) into `DiffLine`s.
pub(crate) fn slice_context_lines(content: &str, start_line: u32, end_line: u32) -> Vec<DiffLine> {
    if start_line > end_line || start_line == 0 {
        return Vec::new();
    }

    let lines: Vec<&str> = content.lines().collect();
    let mut result = Vec::new();
    for line_num in start_line..=end_line {
        let idx = (line_num - 1) as usize;
        if idx >= lines.len() {
            break;
        }
        result.push(DiffLine {
            origin: LineOrigin::Context,
            content: tabify(lines[idx]),
            old_lineno: Some(line_num),
            new_lineno: Some(line_num),
            coloring: LineColoring::Pending,
        });
    }
    result
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
/// grammar needs it (Vue, Svelte, Astro, MDX). This pass leaves other files alone, so
/// their lines stay `LineColoring::Pending` for the render-time pass.
///
/// `fetch_old`/`fetch_new` return the entire content of the file at the old
/// and new sides respectively (or `None` if unavailable). When a side is
/// available, every diff line on that side is replaced with the span at its
/// 1-based lineno from the full-file highlight. Lines whose side could not be
/// fetched stay `Pending`.
///
/// Runs in three phases: fetch (serial, since fetch closures may close over
/// `!Send` state such as `git2::Repository`), highlight (parallel via
/// `std::thread::scope`, since syntect's `SyntaxSet` and `Theme` are `Sync`
/// and each file's syntect work is independent), and apply spans (serial,
/// needs `&mut files`).
pub(crate) fn enhance_with_full_file_highlight<F, G>(
    files: &mut [DiffFile],
    highlighter: &SyntaxHighlighter,
    mut fetch_old: F,
    mut fetch_new: G,
) where
    F: FnMut(&Path) -> Option<String>,
    G: FnMut(&Path) -> Option<String>,
{
    let mut jobs: Vec<HighlightJob> = Vec::new();
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
            old_content,
            new_content,
        });
    }

    if jobs.is_empty() {
        return;
    }

    let results = highlight_jobs_parallel(&jobs, highlighter);

    for (idx, old, new) in results {
        if old.is_none() && new.is_none() {
            continue;
        }
        apply_full_file_spans(&mut files[idx], highlighter, old.as_deref(), new.as_deref());
    }
}

struct HighlightJob {
    file_idx: usize,
    syntax_path: PathBuf,
    old_content: Option<String>,
    new_content: Option<String>,
}

type HighlightResult = (usize, Option<HighlightedLines>, Option<HighlightedLines>);

fn highlight_jobs_parallel(
    jobs: &[HighlightJob],
    highlighter: &SyntaxHighlighter,
) -> Vec<HighlightResult> {
    let parallelism = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(jobs.len());

    if parallelism <= 1 {
        return jobs
            .iter()
            .map(|j| highlight_single_job(j, highlighter))
            .collect();
    }

    let chunk_size = jobs.len().div_ceil(parallelism);
    std::thread::scope(|scope| {
        let handles: Vec<_> = jobs
            .chunks(chunk_size)
            .map(|chunk| {
                scope.spawn(move || {
                    chunk
                        .iter()
                        .map(|j| highlight_single_job(j, highlighter))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("highlight thread panicked"))
            .collect()
    })
}

fn highlight_single_job(job: &HighlightJob, highlighter: &SyntaxHighlighter) -> HighlightResult {
    let old = job
        .old_content
        .as_deref()
        .and_then(|c| highlight_content(highlighter, &job.syntax_path, c));
    let new = job
        .new_content
        .as_deref()
        .and_then(|c| highlight_content(highlighter, &job.syntax_path, c));
    (job.file_idx, old, new)
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
            // Mark a line this pass produced nothing for as `Plain`, so a later per-hunk
            // pass does not retry it with only hunk-local context. That retry is wrong
            // for the container grammars this pass exists to serve.
            line.coloring = match spans {
                Some(spans) => LineColoring::Spans(spans),
                None => LineColoring::Plain,
            };
        }
    }
}

/// Colors one hunk in place, marking every line done even when no grammar matched so
/// a plain file is not retried on every frame. Idempotent.
///
/// This pass deliberately does not skip container grammars. The libgit2, Git CLI, Mercurial,
/// and Jujutsu diff builds each follow up with a whole-file pass that overwrites this with
/// a better result. Forge pull requests and `FileBackend` do not, so this is the only
/// coloring a Vue, Svelte, or MDX file gets in pull request, whole-file, and single-file
/// review.
pub(crate) fn color_hunk(hunk: &mut DiffHunk, syntax_path: &Path, highlighter: &SyntaxHighlighter) {
    let contents: Vec<String> = hunk.lines.iter().map(|l| l.content.clone()).collect();
    let origins: Vec<LineOrigin> = hunk.lines.iter().map(|l| l.origin).collect();
    let seq = SyntaxHighlighter::split_diff_lines_for_highlighting(&contents, &origins);

    // Only a deletion reads the old-side sequence. Whole-file review is all context
    // lines, which read the new side, so coloring both there would double the work
    // for byte-identical output.
    let wants_old = origins.contains(&LineOrigin::Deletion);
    let old = wants_old
        .then(|| highlighter.highlight_file_lines(syntax_path, &seq.old_lines))
        .flatten();
    let new = highlighter.highlight_file_lines(syntax_path, &seq.new_lines);

    for (idx, line) in hunk.lines.iter_mut().enumerate() {
        line.coloring = match highlighter.highlighted_line_for_diff_with_background(
            old.as_deref(),
            new.as_deref(),
            seq.old_line_indices[idx],
            seq.new_line_indices[idx],
            line.origin,
        ) {
            Some(spans) => LineColoring::Spans(spans),
            None => LineColoring::Plain,
        };
    }
}

/// Colors every hunk in `files` at once, for the characterization tests that compare spans
/// against golden literals. Rendering never calls this.
#[cfg(test)]
pub(crate) fn color_all_hunks(files: &mut [DiffFile], highlighter: &SyntaxHighlighter) {
    for file in files.iter_mut() {
        let Some(path) = file.new_path.clone().or_else(|| file.old_path.clone()) else {
            continue;
        };
        for hunk in &mut file.hunks {
            color_hunk(hunk, &path, highlighter);
        }
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

    #[test]
    fn slice_context_lines_expands_tabs() {
        let content = "fn main() {\n\tprintln!(\"hi\");\n}";

        let lines = slice_context_lines(content, 2, 2);

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].content, "    println!(\"hi\");");
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
                    coloring: LineColoring::Pending,
                },
                DiffLine {
                    origin: LineOrigin::Addition,
                    content: added_line.to_string(),
                    old_lineno: None,
                    new_lineno: Some(target_line),
                    coloring: LineColoring::Pending,
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

        let mut files = Vec::with_capacity(n);
        let mut content_map: HashMap<PathBuf, (String, String)> = HashMap::new();
        for i in 0..n {
            let (file, old, new) = make_vue_file(i);
            let path = file.new_path.clone().unwrap();
            content_map.insert(path, (old, new));
            files.push(file);
        }

        let highlighter = SyntaxHighlighter::default();
        enhance_with_full_file_highlight(
            &mut files,
            &highlighter,
            |p| content_map.get(p).map(|(o, _)| o.clone()),
            |p| content_map.get(p).map(|(_, n)| n.clone()),
        );
        files
    }

    fn assert_all_lines_highlighted(files: &[DiffFile]) {
        for (i, file) in files.iter().enumerate() {
            for line in &file.hunks[0].lines {
                let spans = line.coloring.spans().unwrap_or_else(|| {
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
    fn enhance_full_file_highlight_serial_path_one_file() {
        // Single file takes the serial branch in highlight_jobs_parallel.
        let files = highlight_n_vue_files(1);
        assert_all_lines_highlighted(&files);
    }

    #[test]
    fn enhance_full_file_highlight_parallel_path_many_files() {
        // 12 files exceeds typical parallelism, forcing chunked thread::scope.
        let files = highlight_n_vue_files(12);
        assert_eq!(files.len(), 12);
        assert_all_lines_highlighted(&files);
    }

    fn synth_vue_file(idx: usize) -> (DiffFile, String, String) {
        let mut html = String::from("<template>\n  <div class=\"app\">\n");
        for i in 0..80 {
            html.push_str(&format!("    <span class=\"item-{i}\">item {i}</span>\n"));
        }
        html.push_str("  </div>\n</template>\n\n");

        let mut script =
            String::from("<script setup lang=\"ts\">\nimport { ref, computed } from 'vue'\n\n");
        for i in 0..90 {
            script.push_str(&format!("const value{i} = ref({i})\n"));
        }
        script.push_str("</script>\n\n");

        let mut style = String::from("<style scoped>\n");
        for i in 0..30 {
            style.push_str(&format!(".item-{i} {{ color: rgb({i}, 0, 0); }}\n"));
        }
        style.push_str("</style>\n");

        let old = format!("{html}{script}{style}");
        let new = old.replace("const value0 = ref(0)", "const value0 = ref(42)");

        let target_line = new
            .lines()
            .position(|l| l.starts_with("const value0 = ref(42)"))
            .expect("synth content must contain target line") as u32
            + 1;
        let file = vue_diff_file(
            idx,
            "const value0 = ref(0)",
            "const value0 = ref(42)",
            target_line,
        );
        (file, old, new)
    }

    /// Manual bench: parallel vs serial highlight at realistic scales.
    /// Run with: `cargo test --release vcs::tests::bench_highlight_parallel_vs_serial -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn bench_highlight_parallel_vs_serial() {
        use crate::syntax::SyntaxHighlighter;
        use std::time::Instant;

        let highlighter = SyntaxHighlighter::default();
        let scales = [1usize, 5, 12, 25, 50];
        let runs = 5;

        for &n in &scales {
            let mut files_template = Vec::with_capacity(n);
            let mut content_map: HashMap<PathBuf, (String, String)> = HashMap::new();
            for i in 0..n {
                let (file, old, new) = synth_vue_file(i);
                content_map.insert(file.new_path.clone().unwrap(), (old, new));
                files_template.push(file);
            }

            let jobs: Vec<HighlightJob> = files_template
                .iter()
                .enumerate()
                .map(|(idx, f)| {
                    let path = f.new_path.clone().unwrap();
                    let (old, new) = content_map.get(&path).unwrap();
                    HighlightJob {
                        file_idx: idx,
                        syntax_path: path,
                        old_content: Some(old.clone()),
                        new_content: Some(new.clone()),
                    }
                })
                .collect();

            // Warmup
            let _ = highlight_jobs_parallel(&jobs, &highlighter);

            let mut par_total = std::time::Duration::ZERO;
            for _ in 0..runs {
                let t = Instant::now();
                let _ = highlight_jobs_parallel(&jobs, &highlighter);
                par_total += t.elapsed();
            }
            let par_mean = par_total / runs as u32;

            let mut ser_total = std::time::Duration::ZERO;
            for _ in 0..runs {
                let t = Instant::now();
                let _: Vec<HighlightResult> = jobs
                    .iter()
                    .map(|j| highlight_single_job(j, &highlighter))
                    .collect();
                ser_total += t.elapsed();
            }
            let ser_mean = ser_total / runs as u32;

            let speedup = ser_mean.as_secs_f64() / par_mean.as_secs_f64().max(1e-9);
            println!(
                "N={n:>3}: serial={ser_mean:>10.2?}  parallel={par_mean:>10.2?}  speedup={speedup:.2}x"
            );
        }
    }

    #[test]
    fn enhance_full_file_highlight_results_match_input_order() {
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
                added.coloring.spans().is_some(),
                "file {i} addition unhighlighted"
            );
        }
    }

    fn hunk_of(lines: &[(LineOrigin, &str)]) -> DiffHunk {
        DiffHunk {
            header: "@@ -1,3 +1,3 @@".to_string(),
            lines: lines
                .iter()
                .enumerate()
                .map(|(idx, (origin, content))| DiffLine {
                    origin: *origin,
                    content: (*content).to_string(),
                    old_lineno: Some(idx as u32 + 1),
                    new_lineno: Some(idx as u32 + 1),
                    coloring: LineColoring::Pending,
                })
                .collect(),
            old_start: 1,
            old_count: lines.len() as u32,
            new_start: 1,
            new_count: lines.len() as u32,
        }
    }

    #[test]
    fn should_mark_a_hunk_colored_even_when_no_grammar_matches() {
        // Coloring still has to mark a file with no matching grammar done, or the
        // render-time pass sees `is_pending()` still true and reruns the same
        // split-and-resolve work on every frame the hunk is visible.
        let mut hunk = hunk_of(&[(LineOrigin::Context, "no grammar knows this")]);

        color_hunk(
            &mut hunk,
            Path::new("mystery.zzzzz"),
            &SyntaxHighlighter::default(),
        );

        assert!(
            hunk.lines.iter().all(|l| !l.coloring.is_pending()),
            "every line must be marked done, or the render-time pass retries this hunk every frame"
        );
        assert!(
            hunk.lines[0].coloring.spans().is_none(),
            "no grammar matched, so there are no spans to show"
        );
    }

    /// Release build only. The guard changes cost, not output: both runs produce
    /// the same spans by construction, and a debug build makes syntect slow enough
    /// that the two measurements stop separating cleanly.
    ///
    /// Run with:
    /// `cargo test --release vcs::tests::should_skip_the_old_side_when_a_hunk_has_no_deletion -- --ignored`
    #[test]
    #[ignore = "cost assertion; needs a release build to separate cleanly"]
    fn should_skip_the_old_side_when_a_hunk_has_no_deletion() {
        // The saving is larger than the line count suggests:
        // `split_diff_lines_for_highlighting` puts context into both sequences, so a
        // hunk of pure context builds an old side nothing reads. Compared against the
        // same hunk holding one deletion, by cost rather than equality, because the
        // visible output is identical either way.
        const LINES: usize = 400;
        let source = "pub fn alpha(x: u32) -> u32 { x + 1 }";
        let context_only: Vec<(LineOrigin, &str)> =
            (0..LINES).map(|_| (LineOrigin::Context, source)).collect();
        let mut one_deletion = context_only.clone();
        one_deletion[0] = (LineOrigin::Deletion, source);

        let highlighter = SyntaxHighlighter::default();
        let time = |lines: &[(LineOrigin, &str)]| {
            let mut total = std::time::Duration::ZERO;
            for _ in 0..5 {
                let mut hunk = hunk_of(lines);
                let start = std::time::Instant::now();
                color_hunk(&mut hunk, Path::new("sample.rs"), &highlighter);
                total += start.elapsed();
            }
            total / 5
        };

        let skipped = time(&context_only);
        let both = time(&one_deletion);

        assert!(
            both > skipped + skipped / 2,
            "one deletion forces the old side to be colored too, so it should cost far \
             more than a context-only hunk of the same size; got {both:.1?} against \
             {skipped:.1?}"
        );
    }

    #[test]
    fn should_produce_the_same_spans_when_run_twice() {
        // The render-time pass runs every frame and skips hunks already marked
        // done, but a future change that clears the mark must not change what a
        // reader sees.
        let mut hunk = hunk_of(&[
            (LineOrigin::Context, "pub fn alpha(x: u32) -> u32 {"),
            (LineOrigin::Deletion, "    x + 1"),
            (LineOrigin::Addition, "    x * 2"),
        ]);
        let highlighter = SyntaxHighlighter::default();

        color_hunk(&mut hunk, Path::new("sample.rs"), &highlighter);
        let first: Vec<String> = hunk
            .lines
            .iter()
            .map(crate::model::diff_types::span_signature)
            .collect();

        color_hunk(&mut hunk, Path::new("sample.rs"), &highlighter);
        let second: Vec<String> = hunk
            .lines
            .iter()
            .map(crate::model::diff_types::span_signature)
            .collect();

        assert_eq!(first, second, "coloring a hunk twice changed its spans");
        assert!(
            first.iter().all(|s| s != "<uncolored>"),
            "fixture should resolve a grammar; got {first:?}"
        );
    }

    /// Terminal rows a diff pane shows on open. A smaller viewport reports a better ratio,
    /// so this is set near what a maximized terminal fits.
    const VIEWPORT_ROWS: usize = 50;

    /// Repeats an operation and keeps the fastest wall time, not the mean. An identical
    /// call reads slower only from interference, so the minimum is the cleanest sample.
    fn fastest_of(reps: u32, mut op: impl FnMut()) -> std::time::Duration {
        let mut best = std::time::Duration::MAX;
        for _ in 0..reps {
            let start = std::time::Instant::now();
            op();
            best = best.min(start.elapsed());
        }
        best
    }

    fn syntax_path_of(file: &DiffFile) -> Option<PathBuf> {
        file.new_path.clone().or_else(|| file.old_path.clone())
    }

    /// Manual benchmark: does a screen's worth of hunks bound the coloring work?
    ///
    /// `TUICR_BENCH_REPO` names a repository with a working tree and `TUICR_BENCH_RANGE` a
    /// revision range within it. Both unset skips the benchmark, so no repository path is
    /// committed and none is guessed. Only counts and durations are printed: no path, file
    /// name, or commit message from the target repository reaches the output.
    ///
    /// Read the ratio, not the absolute milliseconds. Two runs over the same repository and
    /// range differed threefold when other work competed for cores, yet every column moved
    /// together and the ratio held near eleven to one. `--release` is required: a debug build
    /// makes syntect slow enough that the measurements stop separating.
    ///
    /// Run with:
    /// `TUICR_BENCH_REPO=… TUICR_BENCH_RANGE=… cargo test --release \
    ///     vcs::tests::bench_viewport_versus_whole_diff -- --ignored --nocapture`
    #[test]
    #[ignore = "reads TUICR_BENCH_REPO and TUICR_BENCH_RANGE; never runs by default"]
    fn bench_viewport_versus_whole_diff() {
        let (Ok(repo_path), Ok(range_spec)) = (
            std::env::var("TUICR_BENCH_REPO"),
            std::env::var("TUICR_BENCH_RANGE"),
        ) else {
            println!("skipped: set TUICR_BENCH_REPO and TUICR_BENCH_RANGE to run this benchmark");
            return;
        };

        const REPS: u32 = 5;

        let repo_root = Path::new(&repo_path);
        let backend = GitBackend::discover_from(
            repo_root,
            GitBackendPreference::Libgit2,
            DiffWhitespaceMode::Normal,
        )
        .expect("TUICR_BENCH_REPO did not open as a git repository");
        let range = backend
            .resolve_revision_range(&range_spec)
            .expect("TUICR_BENCH_RANGE did not resolve to a revision range");

        let full = SyntaxHighlighter::default();
        let plain = SyntaxHighlighter::plain();

        // Fetched with the plain highlighter, so every line arrives uncolored and the
        // coloring timings below measure coloring alone. Timed separately because a
        // reader waits on fetch plus coloring, and a coloring ratio quoted without it
        // overstates what they gain.
        let fetch_time = fastest_of(REPS, || {
            backend
                .get_commit_range_diff(&range, &plain)
                .expect("failed to fetch the diff for TUICR_BENCH_RANGE");
        });
        let files = backend
            .get_commit_range_diff(&range, &plain)
            .expect("failed to fetch the diff for TUICR_BENCH_RANGE");

        let total_files = files.len();
        let total_hunks: usize = files.iter().map(|f| f.hunks.len()).sum();
        let total_lines: usize = files
            .iter()
            .flat_map(|f| &f.hunks)
            .map(|h| h.lines.len())
            .sum();

        // The same window production coloring uses, rather than a benchmark-only
        // approximation. Building the `App` this way never renders a frame.
        let mut app = crate::ui::row_height::tests::make_app_with(files);
        app.diff_state.scroll_offset = 0; // top of the diff: what a reader sees on open
        let picked = app.visible_hunk_indices(VIEWPORT_ROWS);

        if picked.is_empty() {
            println!("skipped: TUICR_BENCH_RANGE produced no hunks to color");
            return;
        }

        let viewport_lines: usize = picked
            .iter()
            .map(|&(f, h)| app.diff_files[f].hunks[h].lines.len())
            .sum();

        // Recoloring redoes the work rather than short-circuiting, so repeats on one
        // `App` measure real cost each time rather than a cache warming up.
        let viewport_time = fastest_of(REPS, || {
            for &(f, h) in &picked {
                let Some(path) = syntax_path_of(&app.diff_files[f]) else {
                    continue;
                };
                color_hunk(&mut app.diff_files[f].hunks[h], &path, &full);
            }
        });
        let whole_diff_time = fastest_of(REPS, || {
            color_all_hunks(&mut app.diff_files, &full);
        });

        let before = fetch_time + whole_diff_time;
        let after = fetch_time + viewport_time;

        println!("target: {total_files} files, {total_hunks} hunks, {total_lines} diff lines");
        println!(
            "viewport: {} hunks, {viewport_lines} lines ({:.1}% of the diff)",
            picked.len(),
            100.0 * viewport_lines as f64 / total_lines.max(1) as f64
        );
        println!("({REPS} runs each, fastest kept)");
        println!();
        println!("fetch and parse    {fetch_time:>10.1?}   the same either way");
        println!("color whole diff   {whole_diff_time:>10.1?}   coloring the whole diff up front");
        println!("color viewport     {viewport_time:>10.1?}   coloring one screen at render time");
        println!();
        println!("open path before   {before:>10.1?}");
        println!("open path after    {after:>10.1?}");
        println!(
            "open path is {:.1}x faster; coloring alone is {:.1}x",
            before.as_secs_f64() / after.as_secs_f64().max(f64::EPSILON),
            whole_diff_time.as_secs_f64() / viewport_time.as_secs_f64().max(f64::EPSILON)
        );
        println!();
        println!(
            "Both ratios depend on how much of the diff one screen covers, here {:.1}%.",
            100.0 * viewport_lines as f64 / total_lines.max(1) as f64
        );
        println!("A diff with fewer, larger files at the top shows a smaller ratio.");
    }
}
