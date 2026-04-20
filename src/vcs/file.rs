use std::path::{Path, PathBuf};

use crate::error::{Result, TuicrError};
use crate::model::{DiffFile, DiffHunk, DiffLine, FileStatus, LineOrigin};
use crate::syntax::SyntaxHighlighter;

use super::traits::{VcsBackend, VcsInfo, VcsType};

/// A backend for reviewing a single file without a VCS repository.
///
/// All lines are presented as additions (like a new-file diff), allowing the
/// user to annotate any file without needing git, hg, or jj.
pub struct FileBackend {
    info: VcsInfo,
    /// Absolute path to the file being reviewed
    file_path: PathBuf,
}

impl FileBackend {
    /// Create a new `FileBackend` for the given file path.
    ///
    /// The path is resolved to an absolute path. The file must exist and be
    /// readable.
    pub fn new(path: &str) -> Result<Self> {
        let file_path = std::fs::canonicalize(path).map_err(|e| {
            TuicrError::Io(std::io::Error::new(
                e.kind(),
                format!("Cannot open file '{}': {}", path, e),
            ))
        })?;

        if !file_path.is_file() {
            return Err(TuicrError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("'{}' is not a file", path),
            )));
        }

        let root_path = file_path.parent().unwrap_or(Path::new("/")).to_path_buf();

        let info = VcsInfo {
            root_path,
            head_commit: "file".to_string(),
            branch_name: None,
            vcs_type: VcsType::File,
        };

        Ok(Self { info, file_path })
    }
}

impl VcsBackend for FileBackend {
    fn info(&self) -> &VcsInfo {
        &self.info
    }

    fn get_working_tree_diff(&self, highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
        let content = std::fs::read_to_string(&self.file_path)?;
        let lines: Vec<&str> = content.lines().collect();

        if lines.is_empty() {
            return Err(TuicrError::NoChanges);
        }

        // Build line contents and origins for syntax highlighting
        let line_contents: Vec<String> = lines.iter().map(|l| l.replace('\t', "    ")).collect();
        let line_origins: Vec<LineOrigin> = vec![LineOrigin::Addition; line_contents.len()];

        // Apply syntax highlighting
        let highlight_sequences =
            SyntaxHighlighter::split_diff_lines_for_highlighting(&line_contents, &line_origins);
        let new_highlighted_lines =
            highlighter.highlight_file_lines(&self.file_path, &highlight_sequences.new_lines);

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

        // Relative path from root (just the filename)
        let rel_path = self
            .file_path
            .file_name()
            .map(PathBuf::from)
            .unwrap_or_else(|| self.file_path.clone());

        let hunk = DiffHunk {
            header: format!("@@ -0,0 +1,{} @@", total_lines),
            lines: diff_lines,
            old_start: 0,
            old_count: 0,
            new_start: 1,
            new_count: total_lines,
        };

        let file = DiffFile {
            old_path: None,
            new_path: Some(rel_path),
            status: FileStatus::Added,
            hunks: vec![hunk],
            is_binary: false,
            is_too_large: false,
            is_commit_message: false,
        };

        Ok(vec![file])
    }

    fn fetch_context_lines(
        &self,
        _file_path: &Path,
        _file_status: FileStatus,
        start_line: u32,
        end_line: u32,
    ) -> Result<Vec<DiffLine>> {
        if start_line > end_line || start_line == 0 {
            return Ok(Vec::new());
        }

        let content = std::fs::read_to_string(&self.file_path)?;
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
