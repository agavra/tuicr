use ratatui::style::Style;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::PathBuf};

use crate::hash::Fnv1aHasher;
use crate::model::comment::LineSide;

/// Backend-provided file metadata paired with that file's opaque patch text.
///
/// Paths and status come from a machine-readable VCS or forge channel; the
/// patch is used only to materialize hunks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilePatch {
    pub old_path: Option<PathBuf>,
    pub new_path: Option<PathBuf>,
    pub status: FileStatus,
    pub patch: String,
    pub is_binary: bool,
    pub is_too_large: bool,
}

impl FilePatch {
    pub fn new(
        old_path: Option<PathBuf>,
        new_path: Option<PathBuf>,
        status: FileStatus,
        patch: impl Into<String>,
    ) -> Self {
        Self {
            old_path,
            new_path,
            status,
            patch: patch.into(),
            is_binary: false,
            is_too_large: false,
        }
    }

    pub fn display_path(&self) -> Option<&std::path::Path> {
        self.new_path.as_deref().or(self.old_path.as_deref())
    }
}

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

/// Where a diff line stands with respect to syntax coloring.
///
/// The coloring pass has to tell "not colored yet" from "no grammar matched". A renderer
/// does not, and draws both plainly.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum LineColoring {
    /// The coloring pass has not reached this line's hunk yet.
    #[default]
    Pending,
    /// The pass ran and produced nothing: no grammar matched the file, or the grammar
    /// yielded no spans for this line. Renders plainly and is never retried.
    Plain,
    /// The pass ran and produced these spans.
    Spans(Vec<(Style, String)>),
}

impl LineColoring {
    /// The spans to draw, if any.
    ///
    /// `Pending` and `Plain` both give `None`, because a renderer draws them identically.
    /// The coloring pass tells them apart with `is_pending`.
    pub fn spans(&self) -> Option<&Vec<(Style, String)>> {
        match self {
            Self::Spans(spans) => Some(spans),
            Self::Pending | Self::Plain => None,
        }
    }

    /// Whether this line still needs coloring. The pass skips everything else, which is
    /// what keeps a visible hunk from being recolored on every frame.
    pub fn is_pending(&self) -> bool {
        matches!(self, Self::Pending)
    }
}

#[derive(Debug, Clone)]
pub struct DiffLine {
    pub origin: LineOrigin,
    pub content: String,
    pub old_lineno: Option<u32>,
    pub new_lineno: Option<u32>,
    /// Syntax coloring state.
    pub coloring: LineColoring,
}

/// A line's syntax coloring as one compact, comparable string: each span's text with
/// its foreground and background color.
#[cfg(test)]
pub(crate) fn span_signature(line: &DiffLine) -> String {
    match line.coloring.spans() {
        None => "<uncolored>".to_string(),
        Some(spans) => spans
            .iter()
            .map(|(style, text)| {
                let fg = style
                    .fg
                    .map_or_else(|| "-".to_string(), |c| format!("{c:?}"));
                let bg = style
                    .bg
                    .map_or_else(|| "-".to_string(), |c| format!("{c:?}"));
                format!("{text:?}[{fg}/{bg}]")
            })
            .collect::<Vec<_>>()
            .join(" "),
    }
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

impl DiffLine {
    /// This line's review-hash input: origin marker and text, nothing else.
    ///
    /// A call site can still read the text back out of the coloring field, so this narrows
    /// the mistake rather than preventing it.
    /// `should_not_change_a_hunk_review_key_when_its_lines_are_colored` is what fails when
    /// coloring reaches the hash. Closing it properly means moving coloring off `DiffLine` entirely.
    fn for_content_hash(&self) -> (LineOrigin, &str) {
        (self.origin, self.content.as_str())
    }
}

impl DiffHunk {
    fn review_content_hash(&self) -> u64 {
        let mut hasher = Fnv1aHasher::new();
        write_hunk_content_hash(
            &mut hasher,
            self.lines.iter().map(DiffLine::for_content_hash),
        );
        hasher.finish()
    }
}

impl DiffFile {
    /// Stable key for a reviewed hunk.
    ///
    /// Unique hunk content ignores hunk header line numbers so unrelated
    /// edits above a hunk do not clear its reviewed state. Repeated identical
    /// hunks fall back to a line-aware key because a pure occurrence count can
    /// move reviewed state onto a different hunk when one duplicate changes.
    pub fn hunk_review_key(&self, hunk_idx: usize) -> Option<String> {
        let hash_counts = self.hunk_content_hash_counts();
        self.hunks
            .get(hunk_idx)
            .map(|hunk| self.hunk_review_key_with_counts(hunk, &hash_counts))
    }

    pub fn hunk_review_keys(&self) -> Vec<String> {
        let hash_counts = self.hunk_content_hash_counts();
        self.hunks
            .iter()
            .map(|hunk| self.hunk_review_key_with_counts(hunk, &hash_counts))
            .collect()
    }

    fn hunk_content_hash_counts(&self) -> HashMap<u64, usize> {
        let mut hash_counts = HashMap::new();
        for hunk in &self.hunks {
            *hash_counts.entry(hunk.review_content_hash()).or_insert(0) += 1;
        }
        hash_counts
    }

    fn hunk_review_key_with_counts(
        &self,
        hunk: &DiffHunk,
        hash_counts: &HashMap<u64, usize>,
    ) -> String {
        let hash = hunk.review_content_hash();
        if hash_counts.get(&hash).copied().unwrap_or_default() > 1 {
            format_hunk_review_span_key(hunk, hash)
        } else {
            format_hunk_review_content_key(hash)
        }
    }

    /// Computes a hash of the diff content (all hunk line contents) for change detection.
    pub fn compute_content_hash(hunks: &[DiffHunk]) -> u64 {
        let mut hasher = Fnv1aHasher::new();
        for hunk in hunks {
            write_hunk_content_hash(
                &mut hasher,
                hunk.lines.iter().map(DiffLine::for_content_hash),
            );
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

fn format_hunk_review_content_key(hash: u64) -> String {
    format!("hunk-content-v1:{hash:016x}:0")
}

fn format_hunk_review_span_key(hunk: &DiffHunk, hash: u64) -> String {
    format!(
        "hunk-span-v1:{hash:016x}:{}:{}:{}:{}",
        hunk.old_start, hunk.old_count, hunk.new_start, hunk.new_count
    )
}

/// Writes the review hash's input: each line's origin marker and text, nothing else.
///
/// Takes an iterator of `(origin, content)` pairs, not `&[DiffLine]`, so this body never
/// sees a `DiffLine` and cannot reach its coloring field. Both call sites build that
/// iterator via `DiffLine::for_content_hash`, staying allocation-free on the parse path.
fn write_hunk_content_hash<'a>(
    hasher: &mut Fnv1aHasher,
    lines: impl IntoIterator<Item = (LineOrigin, &'a str)>,
) {
    for (origin, content) in lines {
        hasher.write(match origin {
            LineOrigin::Addition => b"+",
            LineOrigin::Deletion => b"-",
            LineOrigin::Context => b" ",
        });
        hasher.write(content.as_bytes());
        hasher.write(b"\n");
    }
}

#[cfg(test)]
mod hunk_review_key_tests {
    use super::*;
    use ratatui::style::{Color, Style};

    fn line(content: &str, coloring: LineColoring) -> DiffLine {
        DiffLine {
            origin: LineOrigin::Addition,
            content: content.to_string(),
            old_lineno: None,
            new_lineno: Some(1),
            coloring,
        }
    }

    fn hunk(lines: Vec<DiffLine>) -> DiffHunk {
        DiffHunk {
            header: "@@ -1,1 +1,1 @@".to_string(),
            lines,
            old_start: 1,
            old_count: 1,
            new_start: 1,
            new_count: 1,
        }
    }

    /// Characterization: pins `review_content_hash` to a literal so a future change to
    /// `write_hunk_content_hash`'s input shape cannot silently rehash every persisted
    /// reviewed-hunk key. A failing assertion against unmodified code supplied the
    /// literal, never a computation from the function under test.
    #[test]
    fn should_not_silently_rehash_every_persisted_reviewed_hunk_key() {
        let hunk = hunk(vec![
            DiffLine {
                origin: LineOrigin::Context,
                content: "fn main() {".to_string(),
                old_lineno: Some(1),
                new_lineno: Some(1),
                coloring: LineColoring::Pending,
            },
            DiffLine {
                origin: LineOrigin::Deletion,
                content: "    old();".to_string(),
                old_lineno: Some(2),
                new_lineno: None,
                coloring: LineColoring::Pending,
            },
            DiffLine {
                origin: LineOrigin::Addition,
                content: "    new();".to_string(),
                old_lineno: None,
                new_lineno: Some(2),
                coloring: LineColoring::Pending,
            },
        ]);

        assert_eq!(hunk.review_content_hash(), 33278845365600689u64);
    }

    /// Coloring a hunk must not change the key that decides whether it is still the hunk
    /// the reviewer marked reviewed.
    ///
    /// The key hashes origins and content and leaves coloring out, but nothing enforces
    /// that. Coloring happens after parsing, so one session holds the same hunk both
    /// uncolored and colored. If coloring entered the hash, a reviewed mark would drop
    /// the moment the hunk scrolled into view.
    #[test]
    fn should_not_change_a_hunk_review_key_when_its_lines_are_colored() {
        let uncolored = DiffFile {
            old_path: None,
            new_path: Some(PathBuf::from("a.rs")),
            status: FileStatus::Modified,
            hunks: vec![hunk(vec![
                line("let x = 1;", LineColoring::Pending),
                line("let y = 2;", LineColoring::Pending),
            ])],
            is_binary: false,
            is_too_large: false,
            is_commit_message: false,
            content_hash: 0,
        };

        let colored = DiffFile {
            hunks: vec![hunk(vec![
                line(
                    "let x = 1;",
                    LineColoring::Spans(vec![(
                        Style::default().fg(Color::Red),
                        "let x = 1;".to_string(),
                    )]),
                ),
                // Plain and Spans in one hunk, because the whole-file pass produces both.
                line("let y = 2;", LineColoring::Plain),
            ])],
            ..uncolored.clone()
        };

        assert_eq!(
            uncolored.hunk_review_keys(),
            colored.hunk_review_keys(),
            "coloring changed a hunk's review key, so scrolling to a reviewed hunk would \
             silently clear its reviewed mark"
        );
    }
}
