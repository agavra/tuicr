use std::io::Read;
use std::path::{Path, PathBuf};

use ignore::WalkBuilder;

use crate::error::{Result, TuicrError};
use crate::model::{DiffFile, DiffHunk, DiffLine, FileStatus, LineOrigin};
use crate::syntax::SyntaxHighlighter;

use super::traits::{VcsBackend, VcsInfo, VcsType};

/// A backend for reviewing files outside of a VCS repository.
///
/// Accepts either a single file path or a directory. In directory mode the
/// tree is walked with `ignore::WalkBuilder`:
///
/// - `.gitignore`, `.tuicrignore`, and the user's global git excludes
///   (e.g. `~/.config/git/ignore`) are honored
/// - hidden entries (dot-files and `.git`/`.hg`/`.jj`) are skipped
/// - symlinks are not followed
/// - binary and unreadable files are skipped silently
///
/// Every remaining text file is surfaced as a new-file diff so the user can
/// annotate an entire codebase without git, hg, or jj.
pub struct FileBackend {
    info: VcsInfo,
    /// Absolute paths of every file the user can review, paired with the
    /// file size recorded at discovery time so `build_diff_file_for_path`
    /// does not need to re-stat each entry.
    files: Vec<(PathBuf, u64)>,
}

/// Files larger than this are added to the diff list as `is_too_large` so the
/// UI can show a placeholder instead of loading them. Matches
/// `MAX_UNTRACKED_FILE_SIZE` in `vcs/git/diff.rs` so behavior is consistent
/// across backends.
const MAX_FILE_BYTES: u64 = 10 * 1_024 * 1_024;

/// Bytes inspected when classifying a file as text vs. binary.
const BINARY_SNIFF_BYTES: usize = 8192;

impl FileBackend {
    /// Create a new `FileBackend` for the given file or directory path.
    pub fn new(path: &str) -> Result<Self> {
        let canonical = std::fs::canonicalize(path).map_err(|e| {
            TuicrError::Io(std::io::Error::new(
                e.kind(),
                format!("Cannot open '{}': {}", path, e),
            ))
        })?;

        let metadata = std::fs::metadata(&canonical)?;

        let (root_path, files) = if metadata.is_file() {
            let root = canonical.parent().unwrap_or(Path::new("/")).to_path_buf();
            (root, vec![(canonical, metadata.len())])
        } else if metadata.is_dir() {
            let files = collect_text_files(&canonical);
            if files.is_empty() {
                return Err(TuicrError::NoChanges);
            }
            (canonical, files)
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

        Ok(Self { info, files })
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

        // Build line contents and origins for syntax highlighting
        let line_contents: Vec<String> = lines.iter().map(|l| super::tabify(l)).collect();
        let line_origins: Vec<LineOrigin> = vec![LineOrigin::Addition; line_contents.len()];

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
                LineOrigin::Addition,
            );

            diff_lines.push(DiffLine {
                origin: LineOrigin::Addition,
                content: content.clone(),
                old_lineno: None,
                new_lineno: Some(line_num),
                highlighted_spans,
            });
        }

        let total_lines = lines.len() as u32;
        let hunk = DiffHunk {
            header: format!("@@ -0,0 +1,{} @@", total_lines),
            lines: diff_lines,
            old_start: 0,
            old_count: 0,
            new_start: 1,
            new_count: total_lines,
        };

        let hunks = vec![hunk];
        let content_hash = DiffFile::compute_content_hash(&hunks);
        Some(DiffFile {
            old_path: None,
            new_path: Some(rel_path),
            status: FileStatus::Added,
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

        let abs_path = self.info.root_path.join(file_path);
        if !abs_path.is_file() {
            return Ok(Vec::new());
        }

        let content = std::fs::read_to_string(&abs_path)?;
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
}
