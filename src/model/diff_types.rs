use ratatui::style::Style;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::hash::Fnv1aHasher;
use crate::model::comment::LineSide;

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

    /// Highest line number reachable from hunk headers (old or new side).
    /// Used to size the gutter; expanded context beyond hunks is covered
    /// separately via `file_line_count_cache`.
    pub fn max_lineno(&self) -> u32 {
        self.hunks
            .iter()
            .map(|h| (h.old_start + h.old_count).max(h.new_start + h.new_count))
            .max()
            .unwrap_or(0)
    }

    pub fn display_path(&self) -> &PathBuf {
        self.new_path
            .as_ref()
            .or(self.old_path.as_ref())
            .expect("DiffFile must have at least one path")
    }

    /// First line number in display order that carries a value on `side`.
    ///
    /// On `LineSide::New`, returns the first context or addition line; on
    /// `LineSide::Old`, the first deletion line. Used by the submission
    /// mapper to anchor file-level comments per the spec (a file-level
    /// comment posts on the first valid visible line on the right side, or
    /// the first deleted line for pure-deletion files).
    ///
    /// Returns `None` for binary, too-large, or empty-hunk files, and for
    /// the requested side when the file has no lines on that side (e.g. a
    /// pure addition has no Old-side anchor).
    pub fn first_valid_line(&self, side: LineSide) -> Option<u32> {
        if self.is_binary || self.is_too_large {
            return None;
        }
        for hunk in &self.hunks {
            for line in &hunk.lines {
                let candidate = match side {
                    LineSide::New => match line.origin {
                        LineOrigin::Context | LineOrigin::Addition => line.new_lineno,
                        LineOrigin::Deletion => None,
                    },
                    LineSide::Old => match line.origin {
                        LineOrigin::Deletion => line.old_lineno,
                        _ => None,
                    },
                };
                if let Some(n) = candidate {
                    return Some(n);
                }
            }
        }
        None
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
