use std::io::Read;
use std::path::{Path, PathBuf};

use ignore::WalkBuilder;

use crate::error::{Result, TuicrError};
use crate::model::{DiffFile, DiffHunk, DiffLine, FileStatus, LineOrigin};
use crate::syntax::SyntaxHighlighter;

use super::traits::{VcsBackend, VcsInfo, VcsType};

/// A backend for reviewing files outside of a VCS repository (`--file`)
/// and for the whole-repo `--all-files` mode.
///
/// `Single` and `Directory` modes render each line with
/// `LineOrigin::Addition` and a `@@ -0,0 +1,N @@` hunk header (new-file
/// diff shape). `Pristine` mode renders each line with
/// `LineOrigin::Context` and a `@@ -1,N +1,N @@` hunk header.
///
/// Directory mode walks the tree with `ignore::WalkBuilder`, honoring
/// `.gitignore`, `.tuicrignore`, the user's global git excludes, hidden
/// entries, and symlink boundaries. Binary and unreadable files are
/// skipped silently. Pristine mode skips the walker and accepts a
/// pre-enumerated path list (typically from `git ls-files`).
pub struct FileBackend {
    info: VcsInfo,
    /// Absolute paths of every file the user can review, paired with the
    /// file size recorded at discovery time so `build_diff_file_for_path`
    /// does not need to re-stat each entry.
    files: Vec<(PathBuf, u64)>,
    /// Which entry point built this backend. Controls per-line rendering
    /// (addition coloring vs context) and the hunk-header math.
    mode: FileMode,
}

/// How a [`FileBackend`] was constructed. Determines rendering semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileMode {
    /// Single file passed via `--file <path>`. Renders as a new-file
    /// addition diff (`@@ -0,0 +1,N @@`, `LineOrigin::Addition`).
    Single,
    /// Directory walked via `ignore::WalkBuilder` from `--file <dir>`.
    /// Same rendering as `Single`; only the entry point differs.
    Directory,
    /// Whole-repo annotation surface from `--all-files`. Renders as
    /// context (`@@ -1,N +1,N @@`, `LineOrigin::Context`) so the user
    /// reads pristine source instead of a synthetic addition diff.
    Pristine,
}

/// Files larger than this are added to the diff list as `is_too_large` so the
/// UI can show a placeholder instead of loading them. Matches
/// `MAX_UNTRACKED_FILE_SIZE` in `vcs/git/diff.rs` so behavior is consistent
/// across backends.
const MAX_FILE_BYTES: u64 = 10 * 1_024 * 1_024;

/// Bytes inspected when classifying a file as text vs. binary.
const BINARY_SNIFF_BYTES: usize = 8192;

impl FileBackend {
    /// Create a new `FileBackend` for the given file or directory path
    /// (the `--file <path>` entry point).
    ///
    /// # Errors
    ///
    /// Returns [`TuicrError::Io`] if `path` cannot be canonicalized or is
    /// neither a file nor a directory. Returns [`TuicrError::NoChanges`]
    /// when a directory walk surfaces zero text files after binary
    /// filtering.
    pub fn new(path: &str) -> Result<Self> {
        let canonical = std::fs::canonicalize(path).map_err(|e| {
            TuicrError::Io(std::io::Error::new(
                e.kind(),
                format!("Cannot open '{}': {}", path, e),
            ))
        })?;

        let metadata = std::fs::metadata(&canonical)?;

        let (root_path, files, mode) = if metadata.is_file() {
            let root = canonical.parent().unwrap_or(Path::new("/")).to_path_buf();
            (root, vec![(canonical, metadata.len())], FileMode::Single)
        } else if metadata.is_dir() {
            let files = collect_text_files(&canonical);
            if files.is_empty() {
                return Err(TuicrError::NoChanges);
            }
            (canonical, files, FileMode::Directory)
        } else {
            return Err(TuicrError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("'{}' is not a file or directory", path),
            )));
        };

        let info = VcsInfo {
            root_path,
            head_commit: "file".to_string(),
            branch_name: None,
            vcs_type: VcsType::File,
        };

        Ok(Self { info, files, mode })
    }

    /// Create a [`FileBackend`] in pristine mode from a pre-enumerated
    /// list of absolute paths under `root` (the `--all-files` entry
    /// point).
    ///
    /// Skips the `ignore::WalkBuilder` step entirely: the caller has
    /// already decided what should be included (typically from
    /// `git ls-files`). Each path is `stat`-ed for its size, binary files
    /// are dropped via [`is_probably_binary`], and the remaining set is
    /// stored sorted. The session keying happens upstream, so this
    /// constructor only handles the backend state.
    ///
    /// # Errors
    ///
    /// Returns [`TuicrError::NoChanges`] when every path was filtered
    /// out (all binary, all unreadable, or the input list was empty).
    /// Returns [`TuicrError::Io`] when a non-recoverable filesystem
    /// error occurs while stat-ing a path that the caller claimed
    /// exists.
    pub fn new_pristine(paths: Vec<PathBuf>, root: PathBuf) -> Result<Self> {
        let mut files: Vec<(PathBuf, u64)> = Vec::with_capacity(paths.len());
        for path in paths {
            if is_probably_binary(&path) {
                continue;
            }
            let size = match std::fs::metadata(&path) {
                Ok(meta) => meta.len(),
                Err(_) => continue,
            };
            files.push((path, size));
        }

        if files.is_empty() {
            return Err(TuicrError::NoChanges);
        }

        files.sort();

        let info = VcsInfo {
            root_path: root,
            head_commit: "file".to_string(),
            branch_name: None,
            vcs_type: VcsType::File,
        };

        Ok(Self {
            info,
            files,
            mode: FileMode::Pristine,
        })
    }

    fn build_diff_file_for_path(
        &self,
        highlighter: &SyntaxHighlighter,
        abs_path: &Path,
        file_size: u64,
    ) -> Option<DiffFile> {
        // Binary check first so a too-large binary is skipped (not surfaced
        // as a misleading is_too_large text placeholder), and so single-file
        // mode (which never went through `collect_text_files`) is also guarded.
        if is_probably_binary(abs_path) {
            return None;
        }

        // Relative path from root (just the filename in single-file mode)
        let rel_path = abs_path
            .strip_prefix(&self.info.root_path)
            .unwrap_or(abs_path)
            .to_path_buf();

        if file_size > MAX_FILE_BYTES {
            let hunks: Vec<DiffHunk> = Vec::new();
            let content_hash = DiffFile::compute_content_hash(&hunks);
            return Some(DiffFile {
                old_path: None,
                new_path: Some(rel_path),
                status: FileStatus::Added,
                hunks,
                is_binary: false,
                is_too_large: true,
                is_commit_message: false,
                content_hash,
            });
        }

        let content = std::fs::read_to_string(abs_path).ok()?;
        let lines: Vec<&str> = content.lines().collect();
        if lines.is_empty() {
            return None;
        }

        let render_origin = match self.mode {
            FileMode::Pristine => LineOrigin::Context,
            FileMode::Single | FileMode::Directory => LineOrigin::Addition,
        };

        // Build line contents and origins for syntax highlighting
        let line_contents: Vec<String> = lines.iter().map(|l| super::tabify(l)).collect();
        let line_origins: Vec<LineOrigin> = vec![render_origin; line_contents.len()];

        // Apply syntax highlighting
        let highlight_sequences =
            SyntaxHighlighter::split_diff_lines_for_highlighting(&line_contents, &line_origins);
        let new_highlighted_lines =
            highlighter.highlight_file_lines(abs_path, &highlight_sequences.new_lines);

        // Build DiffLines
        let mut diff_lines = Vec::with_capacity(lines.len());
        for (i, content) in line_contents.iter().enumerate() {
            let line_num = (i + 1) as u32;

            let highlighted_spans = highlighter.highlighted_line_for_diff_with_background(
                None,
                new_highlighted_lines.as_deref(),
                None,
                highlight_sequences.new_line_indices[i],
                render_origin,
            );

            // Pristine context lines need both old_lineno and new_lineno
            // populated so the side-by-side and unified renderers walk the
            // gutter math correctly.
            let old_lineno = match self.mode {
                FileMode::Pristine => Some(line_num),
                FileMode::Single | FileMode::Directory => None,
            };

            diff_lines.push(DiffLine {
                origin: render_origin,
                content: content.clone(),
                old_lineno,
                new_lineno: Some(line_num),
                highlighted_spans,
            });
        }

        let total_lines = lines.len() as u32;
        let (hunk_header, old_start, old_count) = match self.mode {
            FileMode::Pristine => (
                format!("@@ -1,{0} +1,{0} @@", total_lines),
                1u32,
                total_lines,
            ),
            FileMode::Single | FileMode::Directory => {
                (format!("@@ -0,0 +1,{} @@", total_lines), 0u32, 0u32)
            }
        };

        let file_status = match self.mode {
            FileMode::Pristine => FileStatus::Modified,
            FileMode::Single | FileMode::Directory => FileStatus::Added,
        };

        let hunk = DiffHunk {
            header: hunk_header,
            lines: diff_lines,
            old_start,
            old_count,
            new_start: 1,
            new_count: total_lines,
        };

        let hunks = vec![hunk];
        let content_hash = DiffFile::compute_content_hash(&hunks);
        Some(DiffFile {
            old_path: None,
            new_path: Some(rel_path),
            status: file_status,
            hunks,
            is_binary: false,
            is_too_large: false,
            is_commit_message: false,
            content_hash,
        })
    }
}

impl VcsBackend for FileBackend {
    fn info(&self) -> &VcsInfo {
        &self.info
    }

    fn get_working_tree_diff(&self, highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
        let diff_files: Vec<DiffFile> = self
            .files
            .iter()
            .filter_map(|(p, size)| self.build_diff_file_for_path(highlighter, p, *size))
            .collect();

        if diff_files.is_empty() {
            return Err(TuicrError::NoChanges);
        }

        Ok(diff_files)
    }

    fn fetch_context_lines(
        &self,
        file_path: &Path,
        _file_status: FileStatus,
        start_line: u32,
        end_line: u32,
    ) -> Result<Vec<DiffLine>> {
        if start_line > end_line || start_line == 0 {
            return Ok(Vec::new());
        }

        // Canonicalize the joined path and confine it to root_path.
        // Without this guard a malformed session pointing at `../../etc/passwd`
        // (or a renamed repo where a stored path now escapes the new root)
        // could read arbitrary files. Pristine mode is multi-file, so the
        // attack surface is genuinely larger here than in single-file mode.
        let joined = self.info.root_path.join(file_path);
        let canonical = match std::fs::canonicalize(&joined) {
            Ok(p) => p,
            Err(_) => return Ok(Vec::new()),
        };
        if !canonical.starts_with(&self.info.root_path) {
            return Ok(Vec::new());
        }
        if !canonical.is_file() {
            return Ok(Vec::new());
        }

        let content = std::fs::read_to_string(&canonical)?;
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
}

fn collect_text_files(root: &Path) -> Vec<(PathBuf, u64)> {
    let mut builder = WalkBuilder::new(root);
    builder
        .require_git(false)
        .add_custom_ignore_filename(".tuicrignore");

    let mut files: Vec<(PathBuf, u64)> = builder
        .build()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_some_and(|ft| ft.is_file()))
        .filter_map(|entry| {
            let size = entry.metadata().ok()?.len();
            let path = entry.into_path();
            if is_probably_binary(&path) {
                return None;
            }
            Some((path, size))
        })
        .collect();

    files.sort();
    files
}

fn is_probably_binary(path: &Path) -> bool {
    let Ok(mut file) = std::fs::File::open(path) else {
        return true;
    };
    let mut buf = [0u8; BINARY_SNIFF_BYTES];
    let n = match file.read(&mut buf) {
        Ok(n) => n,
        Err(_) => return true,
    };
    buf[..n].contains(&0)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::syntax::SyntaxHighlighter;

    fn highlighter() -> SyntaxHighlighter {
        SyntaxHighlighter::default()
    }

    #[test]
    fn single_file_mode_returns_one_diff_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hello.txt");
        fs::write(&path, "alpha\nbeta\n").unwrap();

        let backend = FileBackend::new(path.to_str().unwrap()).unwrap();
        let diffs = backend.get_working_tree_diff(&highlighter()).unwrap();

        assert_eq!(diffs.len(), 1);
        assert_eq!(
            diffs[0].new_path.as_deref().unwrap(),
            Path::new("hello.txt")
        );
        assert_eq!(diffs[0].hunks[0].lines.len(), 2);
    }

    #[test]
    fn directory_mode_walks_and_respects_gitignore() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".gitignore"), "ignored.txt\n").unwrap();
        fs::write(dir.path().join("kept.txt"), "hello\n").unwrap();
        fs::write(dir.path().join("ignored.txt"), "skip me\n").unwrap();

        let backend = FileBackend::new(dir.path().to_str().unwrap()).unwrap();
        let diffs = backend.get_working_tree_diff(&highlighter()).unwrap();

        let names: Vec<_> = diffs
            .iter()
            .map(|d| d.new_path.as_ref().unwrap().to_string_lossy().into_owned())
            .collect();

        assert_eq!(names, vec!["kept.txt"]);
    }

    #[test]
    fn directory_mode_skips_binary_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("binary.bin"), [0u8, 1, 2, 3, 0, 4]).unwrap();

        let result = FileBackend::new(dir.path().to_str().unwrap());
        assert!(matches!(result, Err(TuicrError::NoChanges)));
    }

    #[test]
    fn directory_mode_preserves_relative_paths_for_nested_files() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("nested");
        fs::create_dir(&sub).unwrap();
        fs::write(sub.join("inner.txt"), "x\n").unwrap();

        let backend = FileBackend::new(dir.path().to_str().unwrap()).unwrap();
        let diffs = backend.get_working_tree_diff(&highlighter()).unwrap();

        assert_eq!(diffs.len(), 1);
        assert_eq!(
            diffs[0].new_path.as_deref().unwrap(),
            Path::new("nested/inner.txt")
        );
    }

    // ---------- Pristine-mode regression tests ----------

    #[test]
    fn pristine_diff_uses_context_origin_and_one_indexed_hunk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hello.rs").canonicalize_lazy(dir.path());
        fs::write(
            &path,
            "fn main() {}\nlet x = 1;\nlet y = 2;\nlet z = 3;\nlet w = 4;\n",
        )
        .unwrap();
        let root = dir.path().canonicalize().unwrap();
        let backend = FileBackend::new_pristine(vec![path.clone()], root).unwrap();
        let diffs = backend.get_working_tree_diff(&highlighter()).unwrap();

        assert_eq!(diffs.len(), 1);
        let hunk = &diffs[0].hunks[0];
        assert_eq!(hunk.header, "@@ -1,5 +1,5 @@");
        assert_eq!(hunk.old_start, 1);
        assert_eq!(hunk.old_count, 5);
        assert_eq!(hunk.new_start, 1);
        assert_eq!(hunk.new_count, 5);
        assert!(hunk.lines.iter().all(|l| l.origin == LineOrigin::Context));
        assert!(hunk.lines.iter().all(|l| l.old_lineno.is_some()));
        assert!(hunk.lines.iter().all(|l| l.new_lineno.is_some()));
        assert_eq!(diffs[0].status, FileStatus::Modified);
    }

    #[test]
    fn directory_mode_still_uses_addition_origin() {
        // Regression: pristine mode must not bleed back into the existing
        // `--file <dir>` directory-mode rendering.
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "alpha\nbeta\n").unwrap();

        let backend = FileBackend::new(dir.path().to_str().unwrap()).unwrap();
        let diffs = backend.get_working_tree_diff(&highlighter()).unwrap();

        assert_eq!(diffs.len(), 1);
        let hunk = &diffs[0].hunks[0];
        assert_eq!(hunk.header, "@@ -0,0 +1,2 @@");
        assert!(hunk.lines.iter().all(|l| l.origin == LineOrigin::Addition));
        assert!(hunk.lines.iter().all(|l| l.old_lineno.is_none()));
        assert_eq!(diffs[0].status, FileStatus::Added);
    }

    #[test]
    fn pristine_filters_binary_files() {
        let dir = tempfile::tempdir().unwrap();
        let text_path = dir.path().join("text.txt").canonicalize_lazy(dir.path());
        let bin_path = dir.path().join("bin.bin").canonicalize_lazy(dir.path());
        fs::write(&text_path, "hello\n").unwrap();
        fs::write(&bin_path, [0u8, 1, 2, 3, 0, 4]).unwrap();

        let root = dir.path().canonicalize().unwrap();
        let backend = FileBackend::new_pristine(vec![text_path.clone(), bin_path], root).unwrap();
        let diffs = backend.get_working_tree_diff(&highlighter()).unwrap();

        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].new_path.as_deref().unwrap(), Path::new("text.txt"));
    }

    #[test]
    fn pristine_fetch_context_resolves_per_file() {
        use crate::model::FileStatus;

        let dir = tempfile::tempdir().unwrap();
        let path_a = dir.path().join("a.txt").canonicalize_lazy(dir.path());
        let path_b = dir.path().join("b.txt").canonicalize_lazy(dir.path());
        fs::write(&path_a, "alpha-1\nalpha-2\nalpha-3\n").unwrap();
        fs::write(&path_b, "beta-1\nbeta-2\nbeta-3\n").unwrap();

        let root = dir.path().canonicalize().unwrap();
        let backend = FileBackend::new_pristine(vec![path_a, path_b], root).unwrap();

        let lines = backend
            .fetch_context_lines(Path::new("b.txt"), FileStatus::Modified, 1, 3)
            .unwrap();

        assert_eq!(lines.len(), 3);
        assert!(lines.iter().all(|l| l.content.starts_with("beta-")));
        assert!(lines.iter().all(|l| l.origin == LineOrigin::Context));
    }

    #[test]
    fn pristine_fetch_context_rejects_path_escape() {
        use crate::model::FileStatus;

        let outer = tempfile::tempdir().unwrap();
        let outer_canon = outer.path().canonicalize().unwrap();
        let secret = outer_canon.join("secret.txt");
        fs::write(&secret, "TOP_SECRET\n").unwrap();

        let repo = outer_canon.join("repo");
        fs::create_dir(&repo).unwrap();
        let kept = repo.join("kept.txt");
        fs::write(&kept, "ok\n").unwrap();

        let backend = FileBackend::new_pristine(vec![kept], repo.clone()).unwrap();
        let lines = backend
            .fetch_context_lines(Path::new("../secret.txt"), FileStatus::Modified, 1, 1)
            .unwrap();
        assert!(
            lines.is_empty(),
            "fetch_context_lines must not read files outside the configured root"
        );
    }

    // ---------- Property test for is_probably_binary ----------

    #[test]
    fn is_probably_binary_detects_any_null_in_sniff_window() {
        for trial in [0usize, 1, 100, 8000, BINARY_SNIFF_BYTES, 16_384] {
            // No nulls anywhere within sniff window -> text.
            let dir = tempfile::tempdir().unwrap();
            let text = dir.path().join(format!("text-{trial}.bin"));
            fs::write(&text, vec![b'a'; trial]).unwrap();
            assert!(
                !is_probably_binary(&text) || trial == 0 && text.metadata().unwrap().len() == 0,
                "no-null content of length {trial} must be classified text (got binary)"
            );
        }

        // A null at any position within the sniff window classifies as binary.
        for null_at in [0usize, 1, 100, BINARY_SNIFF_BYTES - 1] {
            let dir = tempfile::tempdir().unwrap();
            let bin = dir.path().join(format!("null-at-{null_at}.bin"));
            let mut content = vec![b'a'; BINARY_SNIFF_BYTES];
            content[null_at] = 0;
            fs::write(&bin, &content).unwrap();
            assert!(
                is_probably_binary(&bin),
                "null at offset {null_at} must classify as binary"
            );
        }

        // A null AFTER the sniff window is NOT detected (by design).
        let dir = tempfile::tempdir().unwrap();
        let bin_after = dir.path().join("null-after-window.bin");
        let mut content = vec![b'a'; BINARY_SNIFF_BYTES + 10];
        content[BINARY_SNIFF_BYTES + 5] = 0;
        fs::write(&bin_after, &content).unwrap();
        assert!(
            !is_probably_binary(&bin_after),
            "null past the {BINARY_SNIFF_BYTES}-byte sniff window must NOT be detected"
        );
    }

    // Small helper: turn `dir/foo.txt` into its post-create canonical form,
    // since `new_pristine` expects absolute paths.
    trait CanonicalizeLazy {
        fn canonicalize_lazy(self, base: &Path) -> PathBuf;
    }
    impl CanonicalizeLazy for PathBuf {
        fn canonicalize_lazy(self, base: &Path) -> PathBuf {
            // create parent + file if missing so canonicalize succeeds
            if !self.exists() {
                if let Some(parent) = self.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                let _ = fs::write(&self, "");
            }
            self.canonicalize().unwrap_or_else(|_| base.join(self))
        }
    }
}
