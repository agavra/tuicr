use ratatui::style::Style;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::hash::Fnv1aHasher;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
}

impl FileStatus {
    pub fn as_char(&self) -> char {
        match self {
            FileStatus::Added => 'A',
            FileStatus::Modified => 'M',
            FileStatus::Deleted => 'D',
            FileStatus::Renamed => 'R',
            FileStatus::Copied => 'C',
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineOrigin {
    Context,
    Addition,
    Deletion,
}

#[derive(Debug, Clone)]
pub struct DiffLine {
    pub origin: LineOrigin,
    pub content: String,
    pub old_lineno: Option<u32>,
    pub new_lineno: Option<u32>,
    /// Optional syntax-highlighted spans for this line
    /// If None, use the default diff coloring
    pub highlighted_spans: Option<Vec<(Style, String)>>,
}

#[derive(Debug, Clone)]
pub struct DiffHunk {
    pub header: String,
    pub lines: Vec<DiffLine>,
    /// Starting line number in the old file (from @@ header)
    #[allow(dead_code)]
    pub old_start: u32,
    /// Number of lines from the old file in this hunk
    #[allow(dead_code)]
    pub old_count: u32,
    /// Starting line number in the new file (from @@ header)
    pub new_start: u32,
    /// Number of lines from the new file in this hunk
    pub new_count: u32,
}

#[derive(Debug, Clone)]
pub struct DiffFile {
    pub old_path: Option<PathBuf>,
    pub new_path: Option<PathBuf>,
    pub status: FileStatus,
    pub hunks: Vec<DiffHunk>,
    pub is_binary: bool,
    pub is_too_large: bool,
    pub is_commit_message: bool,
    pub content_hash: u64,
}

impl DiffFile {
    /// Computes a hash of the diff content (all hunk line contents) for change detection.
    pub fn compute_content_hash(hunks: &[DiffHunk]) -> u64 {
        let mut hasher = Fnv1aHasher::new();
        for hunk in hunks {
            for line in &hunk.lines {
                hasher.write(match line.origin {
                    LineOrigin::Addition => b"+",
                    LineOrigin::Deletion => b"-",
                    LineOrigin::Context => b" ",
                });
                hasher.write(line.content.as_bytes());
                hasher.write(b"\n");
            }
        }
        hasher.finish()
    }

    pub fn display_path(&self) -> &PathBuf {
        self.new_path
            .as_ref()
            .or(self.old_path.as_ref())
            .expect("DiffFile must have at least one path")
    }

    /// Returns `(additions, deletions)` for this file.
    pub fn stat(&self) -> (usize, usize) {
        let mut additions = 0;
        let mut deletions = 0;
        for hunk in &self.hunks {
            for line in &hunk.lines {
                match line.origin {
                    LineOrigin::Addition => additions += 1,
                    LineOrigin::Deletion => deletions += 1,
                    LineOrigin::Context => {}
                }
            }
        }
        (additions, deletions)
    }
}
