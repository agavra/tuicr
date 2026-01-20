//! Unified diff parser for text-based VCS backends (hg, jj).
//!
//! Parses unified diff format output from CLI tools into DiffFile structures.
//! Git uses the native git2 library instead and has its own parser.

use std::path::PathBuf;

use crate::error::{Result, TuicrError};
use crate::model::{DiffFile, DiffHunk, DiffLine, FileStatus, LineOrigin};
use crate::syntax::SyntaxHighlighter;

/// Diff format variants for different VCS tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffFormat {
    /// Mercurial format: "diff -r" headers, paths may have timestamps
    Hg,
    /// Git-style format: "diff --git" headers (used by jj, git patches)
    GitStyle,
}

/// Parse unified diff output into DiffFile structures.
pub fn parse_unified_diff(
    diff_text: &str,
    format: DiffFormat,
    highlighter: &SyntaxHighlighter,
) -> Result<Vec<DiffFile>> {
    let mut files: Vec<DiffFile> = Vec::new();
    let mut lines = diff_text.lines().peekable();

    let header_prefix = match format {
        DiffFormat::Hg => "diff ",
        DiffFormat::GitStyle => "diff --git ",
    };

    while let Some(line) = lines.next() {
        if line.starts_with(header_prefix) {
            let (old_path, new_path, status) = parse_file_header(&mut lines, format);

            // Check if binary - hg uses "Binary file", jj/git use just "Binary"
            if lines.peek().is_some_and(|l| l.contains("Binary")) {
                lines.next(); // consume binary message
                files.push(DiffFile {
                    old_path,
                    new_path,
                    status,
                    hunks: Vec::new(),
                    is_binary: true,
                });
                continue;
            }

            let file_path = new_path.as_ref().or(old_path.as_ref());
            let mut hunks = Vec::new();

            // Parse hunks until next file or end
            while lines.peek().is_some() {
                if let Some(peek_line) = lines.peek() {
                    if peek_line.starts_with("diff ") {
                        break;
                    } else if peek_line.starts_with("@@") {
                        if let Some(hunk) = parse_hunk(&mut lines, file_path, highlighter) {
                            hunks.push(hunk);
                        }
                    } else {
                        lines.next(); // skip non-hunk, non-diff lines
                    }
                }
            }

            files.push(DiffFile {
                old_path,
                new_path,
                status,
                hunks,
                is_binary: false,
            });
        }
    }

    if files.is_empty() {
        return Err(TuicrError::NoChanges);
    }

    Ok(files)
}

fn parse_file_header<'a, I>(
    lines: &mut std::iter::Peekable<I>,
    format: DiffFormat,
) -> (Option<PathBuf>, Option<PathBuf>, FileStatus)
where
    I: Iterator<Item = &'a str>,
{
    let mut old_path: Option<PathBuf> = None;
    let mut new_path: Option<PathBuf> = None;
    let mut status = FileStatus::Modified;

    // Parse --- and +++ lines and metadata
    while let Some(line) = lines.peek() {
        if line.starts_with("---") {
            let path_str = line.trim_start_matches("--- ").trim_start_matches("a/");
            if path_str != "/dev/null" {
                // Hg format may include timestamps after tab
                let path = if format == DiffFormat::Hg {
                    path_str.split('\t').next().unwrap_or(path_str)
                } else {
                    path_str
                };
                old_path = Some(PathBuf::from(path));
            }
            lines.next();
        } else if line.starts_with("+++") {
            let path_str = line.trim_start_matches("+++ ").trim_start_matches("b/");
            if path_str != "/dev/null" {
                let path = if format == DiffFormat::Hg {
                    path_str.split('\t').next().unwrap_or(path_str)
                } else {
                    path_str
                };
                new_path = Some(PathBuf::from(path));
            }
            lines.next();
            break; // Done with file header
        } else if line.starts_with("new file") {
            status = FileStatus::Added;
            lines.next();
        } else if line.starts_with("deleted file") {
            status = FileStatus::Deleted;
            lines.next();
        } else if let Some(path) = line.strip_prefix("rename from ") {
            status = FileStatus::Renamed;
            old_path = Some(PathBuf::from(path));
            lines.next();
        } else if let Some(path) = line.strip_prefix("rename to ") {
            new_path = Some(PathBuf::from(path));
            lines.next();
        } else if let Some(path) = line.strip_prefix("copy from ") {
            status = FileStatus::Copied;
            old_path = Some(PathBuf::from(path));
            lines.next();
        } else if let Some(path) = line.strip_prefix("copy to ") {
            new_path = Some(PathBuf::from(path));
            lines.next();
        } else if line.starts_with("@@") || line.starts_with("diff ") {
            break;
        } else if line.starts_with("Binary file") {
            // Hg format: "Binary file <path> has changed"
            // Git format: "Binary files a/<old> and b/<new> differ"
            if let Some((old, new)) = parse_binary_file_line(line) {
                if old_path.is_none() {
                    old_path = old;
                }
                if new_path.is_none() {
                    new_path = new;
                }
            }
            break;
        } else {
            lines.next(); // Skip other metadata lines (rename to, copy to, index, etc.)
        }
    }

    // Determine status from paths if not already set by metadata
    if status == FileStatus::Modified {
        if old_path.is_none() && new_path.is_some() {
            status = FileStatus::Added;
        } else if old_path.is_some() && new_path.is_none() {
            status = FileStatus::Deleted;
        }
    }

    (old_path, new_path, status)
}

fn parse_hunk<'a, I>(
    lines: &mut std::iter::Peekable<I>,
    file_path: Option<&PathBuf>,
    highlighter: &SyntaxHighlighter,
) -> Option<DiffHunk>
where
    I: Iterator<Item = &'a str>,
{
    let header_line = lines.next()?;

    // Parse @@ -old_start,old_count +new_start,new_count @@
    let (old_start, old_count, new_start, new_count) = parse_hunk_header(header_line)?;

    let mut line_contents: Vec<String> = Vec::new();
    let mut line_origins: Vec<LineOrigin> = Vec::new();
    let mut line_numbers: Vec<(Option<u32>, Option<u32>)> = Vec::new();

    let mut old_lineno = old_start;
    let mut new_lineno = new_start;

    // Collect lines until next hunk or file
    while let Some(line) = lines.peek() {
        if line.starts_with("@@") || line.starts_with("diff ") {
            break;
        }

        let line = lines.next().unwrap();

        if line.starts_with('\\') {
            // "\ No newline at end of file" - skip
            continue;
        }

        let (origin, content, old_ln, new_ln) = if let Some(stripped) = line.strip_prefix('+') {
            if line.starts_with("+++") {
                // Skip +++ header lines
                continue;
            }
            let ln = new_lineno;
            new_lineno += 1;
            (LineOrigin::Addition, stripped, None, Some(ln))
        } else if let Some(stripped) = line.strip_prefix('-') {
            if line.starts_with("---") {
                // Skip --- header lines
                continue;
            }
            let ln = old_lineno;
            old_lineno += 1;
            (LineOrigin::Deletion, stripped, Some(ln), None)
        } else if let Some(stripped) = line.strip_prefix(' ') {
            let old_ln = old_lineno;
            let new_ln = new_lineno;
            old_lineno += 1;
            new_lineno += 1;
            (LineOrigin::Context, stripped, Some(old_ln), Some(new_ln))
        } else if line.is_empty() {
            // Empty line in diff (context line with no content after space)
            let old_ln = old_lineno;
            let new_ln = new_lineno;
            old_lineno += 1;
            new_lineno += 1;
            (LineOrigin::Context, "", Some(old_ln), Some(new_ln))
        } else {
            // Unknown format, skip
            continue;
        };

        line_contents.push(content.to_string());
        line_origins.push(origin);
        line_numbers.push((old_ln, new_ln));
    }

    // Apply syntax highlighting if we have a file path
    let highlighted_lines =
        file_path.and_then(|path| highlighter.highlight_file_lines(path, &line_contents));

    // Build DiffLines
    let mut diff_lines: Vec<DiffLine> = Vec::with_capacity(line_contents.len());
    for (idx, content) in line_contents.into_iter().enumerate() {
        let origin = line_origins[idx];
        let (old_lineno, new_lineno) = line_numbers[idx];

        let highlighted_spans = highlighted_lines.as_ref().and_then(|all| {
            all.get(idx)
                .map(|spans| highlighter.apply_diff_background(spans.clone(), origin))
        });

        diff_lines.push(DiffLine {
            origin,
            content,
            old_lineno,
            new_lineno,
            highlighted_spans,
        });
    }

    Some(DiffHunk {
        header: header_line.to_string(),
        lines: diff_lines,
        old_start,
        old_count,
        new_start,
        new_count,
    })
}

fn parse_hunk_header(line: &str) -> Option<(u32, u32, u32, u32)> {
    // Format: @@ -old_start,old_count +new_start,new_count @@
    // or: @@ -old_start +new_start @@ (count defaults to 1)

    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 3 || parts[0] != "@@" {
        return None;
    }

    let old_part = parts[1].trim_start_matches('-');
    let new_part = parts[2].trim_start_matches('+');

    let (old_start, old_count) = parse_range(old_part);
    let (new_start, new_count) = parse_range(new_part);

    Some((old_start, old_count, new_start, new_count))
}

fn parse_range(s: &str) -> (u32, u32) {
    if let Some((start, count)) = s.split_once(',') {
        (start.parse().unwrap_or(1), count.parse().unwrap_or(1))
    } else {
        (s.parse().unwrap_or(1), 1)
    }
}

/// Parse paths from a binary file line.
/// Git format: "Binary files a/<old> and b/<new> differ"
/// Hg format: "Binary file <path> has changed"
/// Returns (old_path, new_path) where either can be None for /dev/null
fn parse_binary_file_line(line: &str) -> Option<(Option<PathBuf>, Option<PathBuf>)> {
    // Git format: "Binary files a/path/to/file and b/path/to/file differ"
    if let Some(content) = line.strip_prefix("Binary files ") {
        let content = content.strip_suffix(" differ")?;
        let (old_part, new_part) = content.split_once(" and ")?;

        let old_path = if old_part == "/dev/null" {
            None
        } else {
            Some(PathBuf::from(
                old_part.strip_prefix("a/").unwrap_or(old_part),
            ))
        };

        let new_path = if new_part == "/dev/null" {
            None
        } else {
            Some(PathBuf::from(
                new_part.strip_prefix("b/").unwrap_or(new_part),
            ))
        };

        return Some((old_path, new_path));
    }

    // Hg format: "Binary file image.png has changed"
    if let Some(content) = line.strip_prefix("Binary file ") {
        let path = content.strip_suffix(" has changed")?;
        // For hg, the same path is used for both old and new
        let path = PathBuf::from(path);
        return Some((Some(path.clone()), Some(path)));
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // ============ Common tests ============

    #[test]
    fn should_return_no_changes_for_empty_diff() {
        assert!(matches!(
            parse_unified_diff("", DiffFormat::Hg, &SyntaxHighlighter::default()),
            Err(TuicrError::NoChanges)
        ));
        assert!(matches!(
            parse_unified_diff("", DiffFormat::GitStyle, &SyntaxHighlighter::default()),
            Err(TuicrError::NoChanges)
        ));
    }

    #[test]
    fn should_parse_hunk_header() {
        let result = parse_hunk_header("@@ -1,3 +1,4 @@");
        assert_eq!(result, Some((1, 3, 1, 4)));

        let result = parse_hunk_header("@@ -10,5 +20,8 @@ context");
        assert_eq!(result, Some((10, 5, 20, 8)));
    }

    #[test]
    fn should_parse_hunk_header_without_count() {
        let (old_start, old_count, new_start, new_count) =
            parse_hunk_header("@@ -5 +10 @@").unwrap();
        assert_eq!(old_start, 5);
        assert_eq!(old_count, 1);
        assert_eq!(new_start, 10);
        assert_eq!(new_count, 1);
    }

    #[test]
    fn should_reject_invalid_hunk_header() {
        assert!(parse_hunk_header("not a hunk header").is_none());
        assert!(parse_hunk_header("@@ invalid").is_none());
    }

    #[test]
    fn should_parse_range_with_comma() {
        assert_eq!(parse_range("10,5"), (10, 5));
        assert_eq!(parse_range("1,100"), (1, 100));
    }

    #[test]
    fn should_parse_range_without_comma() {
        assert_eq!(parse_range("42"), (42, 1));
        assert_eq!(parse_range("1"), (1, 1));
    }

    #[test]
    fn should_handle_invalid_range() {
        assert_eq!(parse_range("abc"), (1, 1));
        assert_eq!(parse_range("abc,def"), (1, 1));
    }

    // ============ Hg format tests ============

    #[test]
    fn hg_should_parse_simple_diff() {
        let diff = r#"diff -r abc123 test.rs
--- a/test.rs	Thu Jan 01 00:00:00 1970 +0000
+++ b/test.rs	Thu Jan 01 00:00:00 1970 +0000
@@ -1,3 +1,4 @@
 fn main() {
+    println!("hello");
     println!("world");
 }
"#;

        let result =
            parse_unified_diff(diff, DiffFormat::Hg, &SyntaxHighlighter::default()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].status, FileStatus::Modified);
        assert_eq!(result[0].hunks.len(), 1);
        assert_eq!(result[0].hunks[0].lines.len(), 4);
    }

    #[test]
    fn hg_should_parse_new_file() {
        let diff = r#"diff -r 000000000000 new_file.rs
--- /dev/null
+++ b/new_file.rs
@@ -0,0 +1,2 @@
+fn new() {
+}
"#;

        let result =
            parse_unified_diff(diff, DiffFormat::Hg, &SyntaxHighlighter::default()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].status, FileStatus::Added);
        assert!(result[0].old_path.is_none());
        assert_eq!(
            result[0].new_path.as_ref().unwrap().to_str().unwrap(),
            "new_file.rs"
        );
    }

    #[test]
    fn hg_should_parse_deleted_file() {
        let diff = r#"diff -r abc123 old_file.rs
--- a/old_file.rs
+++ /dev/null
@@ -1,2 +0,0 @@
-fn old() {
-}
"#;

        let result =
            parse_unified_diff(diff, DiffFormat::Hg, &SyntaxHighlighter::default()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].status, FileStatus::Deleted);
        assert_eq!(
            result[0].old_path.as_ref().unwrap().to_str().unwrap(),
            "old_file.rs"
        );
        assert!(result[0].new_path.is_none());
    }

    #[test]
    fn hg_should_parse_multiple_files() {
        let diff = r#"diff -r abc123 file1.rs
--- a/file1.rs
+++ b/file1.rs
@@ -1,1 +1,2 @@
 line1
+line2
diff -r abc123 file2.rs
--- a/file2.rs
+++ b/file2.rs
@@ -1,2 +1,1 @@
 keep
-remove
"#;

        let result =
            parse_unified_diff(diff, DiffFormat::Hg, &SyntaxHighlighter::default()).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(
            result[0].new_path.as_ref().unwrap().to_str().unwrap(),
            "file1.rs"
        );
        assert_eq!(
            result[1].new_path.as_ref().unwrap().to_str().unwrap(),
            "file2.rs"
        );
    }

    #[test]
    fn hg_should_parse_multiple_hunks() {
        let diff = r#"diff -r abc123 multi.rs
--- a/multi.rs
+++ b/multi.rs
@@ -1,3 +1,4 @@
 fn first() {
+    // added
 }

@@ -10,3 +11,4 @@
 fn second() {
+    // also added
 }
"#;

        let result =
            parse_unified_diff(diff, DiffFormat::Hg, &SyntaxHighlighter::default()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].hunks.len(), 2);
        assert_eq!(result[0].hunks[0].old_start, 1);
        assert_eq!(result[0].hunks[1].old_start, 10);
    }

    #[test]
    fn hg_should_parse_renamed_file() {
        let diff = r#"diff -r abc123 new_name.rs
rename from old_name.rs
rename to new_name.rs
--- a/old_name.rs
+++ b/new_name.rs
@@ -1,1 +1,1 @@
-old content
+new content
"#;

        let result =
            parse_unified_diff(diff, DiffFormat::Hg, &SyntaxHighlighter::default()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].status, FileStatus::Renamed);
        assert_eq!(
            result[0].old_path.as_ref().unwrap().to_str().unwrap(),
            "old_name.rs"
        );
        assert_eq!(
            result[0].new_path.as_ref().unwrap().to_str().unwrap(),
            "new_name.rs"
        );
    }

    #[test]
    fn hg_should_parse_binary_file() {
        let diff = r#"diff -r abc123 image.png
Binary file image.png has changed
"#;

        let result =
            parse_unified_diff(diff, DiffFormat::Hg, &SyntaxHighlighter::default()).unwrap();
        assert_eq!(result.len(), 1);
        assert!(result[0].is_binary);
        assert!(result[0].hunks.is_empty());
    }

    #[test]
    fn hg_should_parse_renamed_file_without_content_changes() {
        // Pure rename with no content changes - no ---/+++ lines
        let diff = r#"diff -r abc123 new_name.rs
rename from old_name.rs
rename to new_name.rs
"#;

        let result =
            parse_unified_diff(diff, DiffFormat::Hg, &SyntaxHighlighter::default()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].status, FileStatus::Renamed);
        assert_eq!(result[0].old_path, Some(PathBuf::from("old_name.rs")));
        assert_eq!(result[0].new_path, Some(PathBuf::from("new_name.rs")));
        assert!(result[0].hunks.is_empty());
    }

    #[test]
    fn hg_should_parse_copied_file_without_content_changes() {
        // Pure copy with no content changes
        let diff = r#"diff -r abc123 dest.rs
copy from source.rs
copy to dest.rs
"#;

        let result =
            parse_unified_diff(diff, DiffFormat::Hg, &SyntaxHighlighter::default()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].status, FileStatus::Copied);
        assert_eq!(result[0].old_path, Some(PathBuf::from("source.rs")));
        assert_eq!(result[0].new_path, Some(PathBuf::from("dest.rs")));
        assert!(result[0].hunks.is_empty());
    }

    #[test]
    fn hg_should_parse_copied_file_with_content_changes() {
        // Copy with content changes
        let diff = r#"diff -r abc123 dest.rs
copy from source.rs
copy to dest.rs
--- a/source.rs	Thu Jan 01 00:00:00 1970 +0000
+++ b/dest.rs	Thu Jan 01 00:00:00 1970 +0000
@@ -1 +1,2 @@
 original
+added line
"#;

        let result =
            parse_unified_diff(diff, DiffFormat::Hg, &SyntaxHighlighter::default()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].status, FileStatus::Copied);
        assert_eq!(result[0].old_path, Some(PathBuf::from("source.rs")));
        assert_eq!(result[0].new_path, Some(PathBuf::from("dest.rs")));
        assert_eq!(result[0].hunks.len(), 1);
    }

    #[test]
    fn hg_should_handle_no_newline_marker() {
        let diff = r#"diff -r abc123 no_newline.rs
--- a/no_newline.rs
+++ b/no_newline.rs
@@ -1,1 +1,1 @@
-old
\ No newline at end of file
+new
\ No newline at end of file
"#;

        let result =
            parse_unified_diff(diff, DiffFormat::Hg, &SyntaxHighlighter::default()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].hunks[0].lines.len(), 2);
    }

    #[test]
    fn hg_should_parse_line_numbers_correctly() {
        let diff = r#"diff -r abc123 nums.rs
--- a/nums.rs
+++ b/nums.rs
@@ -5,4 +5,5 @@
 context at 5
-deleted at 6
+added at 6
+added at 7
 context at 7->8
"#;

        let result =
            parse_unified_diff(diff, DiffFormat::Hg, &SyntaxHighlighter::default()).unwrap();
        let lines = &result[0].hunks[0].lines;

        assert_eq!(lines[0].origin, LineOrigin::Context);
        assert_eq!(lines[0].old_lineno, Some(5));
        assert_eq!(lines[0].new_lineno, Some(5));

        assert_eq!(lines[1].origin, LineOrigin::Deletion);
        assert_eq!(lines[1].old_lineno, Some(6));
        assert_eq!(lines[1].new_lineno, None);

        assert_eq!(lines[2].origin, LineOrigin::Addition);
        assert_eq!(lines[2].old_lineno, None);
        assert_eq!(lines[2].new_lineno, Some(6));

        assert_eq!(lines[3].origin, LineOrigin::Addition);
        assert_eq!(lines[3].old_lineno, None);
        assert_eq!(lines[3].new_lineno, Some(7));

        assert_eq!(lines[4].origin, LineOrigin::Context);
        assert_eq!(lines[4].old_lineno, Some(7));
        assert_eq!(lines[4].new_lineno, Some(8));
    }

    // ============ Jujutsu (jj) format tests - uses DiffFormat::GitStyle ============

    #[test]
    fn jj_should_parse_simple_diff() {
        let diff = r#"diff --git a/file.txt b/file.txt
--- a/file.txt
+++ b/file.txt
@@ -1,3 +1,4 @@
 line1
+added
 line2
 line3
"#;
        let files =
            parse_unified_diff(diff, DiffFormat::GitStyle, &SyntaxHighlighter::default()).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].new_path, Some(PathBuf::from("file.txt")));
        assert_eq!(files[0].status, FileStatus::Modified);
        assert_eq!(files[0].hunks.len(), 1);
        assert_eq!(files[0].hunks[0].lines.len(), 4);
    }

    #[test]
    fn jj_should_parse_new_file() {
        let diff = r#"diff --git a/new.txt b/new.txt
new file mode 100644
--- /dev/null
+++ b/new.txt
@@ -0,0 +1,2 @@
+line1
+line2
"#;
        let files =
            parse_unified_diff(diff, DiffFormat::GitStyle, &SyntaxHighlighter::default()).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].status, FileStatus::Added);
    }

    #[test]
    fn jj_should_parse_deleted_file() {
        let diff = r#"diff --git a/old.txt b/old.txt
deleted file mode 100644
--- a/old.txt
+++ /dev/null
@@ -1,2 +0,0 @@
-line1
-line2
"#;
        let files =
            parse_unified_diff(diff, DiffFormat::GitStyle, &SyntaxHighlighter::default()).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].status, FileStatus::Deleted);
    }

    #[test]
    fn jj_should_parse_renamed_file_without_content_changes() {
        // Pure rename with no content changes - no ---/+++ lines
        let diff = r#"diff --git a/old.txt b/new.txt
rename from old.txt
rename to new.txt
"#;
        let files =
            parse_unified_diff(diff, DiffFormat::GitStyle, &SyntaxHighlighter::default()).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].status, FileStatus::Renamed);
        assert_eq!(files[0].old_path, Some(PathBuf::from("old.txt")));
        assert_eq!(files[0].new_path, Some(PathBuf::from("new.txt")));
        assert!(files[0].hunks.is_empty());
    }

    #[test]
    fn jj_should_parse_renamed_file_with_content_changes() {
        // Rename with content changes - has ---/+++ lines
        let diff = r#"diff --git a/old.txt b/new.txt
rename from old.txt
rename to new.txt
--- a/old.txt
+++ b/new.txt
@@ -1 +1 @@
-old content
+new content
"#;
        let files =
            parse_unified_diff(diff, DiffFormat::GitStyle, &SyntaxHighlighter::default()).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].status, FileStatus::Renamed);
        assert_eq!(files[0].old_path, Some(PathBuf::from("old.txt")));
        assert_eq!(files[0].new_path, Some(PathBuf::from("new.txt")));
        assert_eq!(files[0].hunks.len(), 1);
    }

    #[test]
    fn jj_should_parse_copied_file_without_content_changes() {
        // Pure copy with no content changes
        let diff = r#"diff --git a/source.txt b/dest.txt
copy from source.txt
copy to dest.txt
"#;
        let files =
            parse_unified_diff(diff, DiffFormat::GitStyle, &SyntaxHighlighter::default()).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].status, FileStatus::Copied);
        assert_eq!(files[0].old_path, Some(PathBuf::from("source.txt")));
        assert_eq!(files[0].new_path, Some(PathBuf::from("dest.txt")));
        assert!(files[0].hunks.is_empty());
    }

    #[test]
    fn jj_should_parse_copied_file_with_content_changes() {
        // Copy with content changes
        let diff = r#"diff --git a/source.txt b/dest.txt
copy from source.txt
copy to dest.txt
--- a/source.txt
+++ b/dest.txt
@@ -1 +1,2 @@
 original
+added line
"#;
        let files =
            parse_unified_diff(diff, DiffFormat::GitStyle, &SyntaxHighlighter::default()).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].status, FileStatus::Copied);
        assert_eq!(files[0].old_path, Some(PathBuf::from("source.txt")));
        assert_eq!(files[0].new_path, Some(PathBuf::from("dest.txt")));
        assert_eq!(files[0].hunks.len(), 1);
    }

    #[test]
    fn jj_should_parse_binary_file_added() {
        let diff = r#"diff --git a/image.png b/image.png
new file mode 100644
index 0000000000..abc1234567
Binary files /dev/null and b/image.png differ
"#;
        let files =
            parse_unified_diff(diff, DiffFormat::GitStyle, &SyntaxHighlighter::default()).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].is_binary);
        assert_eq!(files[0].status, FileStatus::Added);
        assert!(files[0].old_path.is_none());
        assert_eq!(files[0].new_path, Some(PathBuf::from("image.png")));
    }

    #[test]
    fn jj_should_parse_binary_file_deleted() {
        let diff = r#"diff --git a/image.png b/image.png
deleted file mode 100644
index abc1234567..0000000000
Binary files a/image.png and /dev/null differ
"#;
        let files =
            parse_unified_diff(diff, DiffFormat::GitStyle, &SyntaxHighlighter::default()).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].is_binary);
        assert_eq!(files[0].status, FileStatus::Deleted);
        assert_eq!(files[0].old_path, Some(PathBuf::from("image.png")));
        assert!(files[0].new_path.is_none());
    }

    #[test]
    fn jj_should_parse_binary_file_modified() {
        let diff = r#"diff --git a/image.png b/image.png
index abc1234567..def7890123 100644
Binary files a/image.png and b/image.png differ
"#;
        let files =
            parse_unified_diff(diff, DiffFormat::GitStyle, &SyntaxHighlighter::default()).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].is_binary);
        assert_eq!(files[0].status, FileStatus::Modified);
        assert_eq!(files[0].old_path, Some(PathBuf::from("image.png")));
        assert_eq!(files[0].new_path, Some(PathBuf::from("image.png")));
    }

    #[test]
    fn jj_should_parse_multiple_files() {
        let diff = r#"diff --git a/a.txt b/a.txt
--- a/a.txt
+++ b/a.txt
@@ -1 +1 @@
-old
+new
diff --git a/b.txt b/b.txt
--- a/b.txt
+++ b/b.txt
@@ -1 +1 @@
-foo
+bar
"#;
        let files =
            parse_unified_diff(diff, DiffFormat::GitStyle, &SyntaxHighlighter::default()).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].new_path, Some(PathBuf::from("a.txt")));
        assert_eq!(files[1].new_path, Some(PathBuf::from("b.txt")));
    }

    #[test]
    fn jj_should_calculate_line_numbers() {
        let diff = r#"diff --git a/file.txt b/file.txt
--- a/file.txt
+++ b/file.txt
@@ -5,4 +5,5 @@
 context
-deleted
+added1
+added2
 more
"#;
        let files =
            parse_unified_diff(diff, DiffFormat::GitStyle, &SyntaxHighlighter::default()).unwrap();
        let hunk = &files[0].hunks[0];

        assert_eq!(hunk.lines[0].old_lineno, Some(5));
        assert_eq!(hunk.lines[0].new_lineno, Some(5));

        assert_eq!(hunk.lines[1].old_lineno, Some(6));
        assert_eq!(hunk.lines[1].new_lineno, None);

        assert_eq!(hunk.lines[2].old_lineno, None);
        assert_eq!(hunk.lines[2].new_lineno, Some(6));

        assert_eq!(hunk.lines[3].old_lineno, None);
        assert_eq!(hunk.lines[3].new_lineno, Some(7));

        assert_eq!(hunk.lines[4].old_lineno, Some(7));
        assert_eq!(hunk.lines[4].new_lineno, Some(8));
    }
}
