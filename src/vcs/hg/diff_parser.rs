use std::path::PathBuf;
use std::sync::LazyLock;

use crate::error::{Result, TuicrError};
use crate::model::{DiffFile, DiffHunk, DiffLine, FileStatus, LineOrigin};
use crate::syntax::SyntaxHighlighter;

static HIGHLIGHTER: LazyLock<SyntaxHighlighter> = LazyLock::new(SyntaxHighlighter::new);

/// Parse unified diff output from `hg diff` into DiffFile structures
pub fn parse_unified_diff(diff_text: &str) -> Result<Vec<DiffFile>> {
    let mut files: Vec<DiffFile> = Vec::new();
    let mut lines = diff_text.lines().peekable();

    while let Some(line) = lines.next() {
        // Look for "diff -r" or "diff --git" style headers
        if line.starts_with("diff ") {
            let (old_path, new_path, status) = parse_file_header(&mut lines);

            // Check if binary
            if lines.peek().is_some_and(|l| l.contains("Binary file")) {
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
                        if let Some(hunk) = parse_hunk(&mut lines, file_path) {
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
                // Remove trailing timestamp if present (hg format: "path\ttimestamp")
                let path = path_str.split('\t').next().unwrap_or(path_str);
                old_path = Some(PathBuf::from(path));
            }
            lines.next();
        } else if line.starts_with("+++") {
            let path_str = line.trim_start_matches("+++ ").trim_start_matches("b/");
            if path_str != "/dev/null" {
                let path = path_str.split('\t').next().unwrap_or(path_str);
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
        } else if line.starts_with("rename from") {
            status = FileStatus::Renamed;
            lines.next();
        } else if line.starts_with("copy from") {
            status = FileStatus::Copied;
            lines.next();
        } else if line.starts_with("@@") || line.starts_with("diff ") || line.contains("Binary") {
            break;
        } else {
            lines.next(); // Skip other metadata lines (rename to, copy to, etc.)
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
        file_path.and_then(|path| HIGHLIGHTER.highlight_file_lines(path, &line_contents));

    // Build DiffLines
    let diff_lines: Vec<DiffLine> = line_contents
        .into_iter()
        .enumerate()
        .map(|(idx, content)| {
            let origin = line_origins[idx];
            let (old_lineno, new_lineno) = line_numbers[idx];

            let highlighted_spans = highlighted_lines.as_ref().and_then(|all| {
                all.get(idx)
                    .map(|spans| SyntaxHighlighter::apply_diff_background(spans.clone(), origin))
            });

            DiffLine {
                origin,
                content,
                old_lineno,
                new_lineno,
                highlighted_spans,
            }
        })
        .collect();

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_parse_simple_diff() {
        let diff = r#"diff -r abc123 test.rs
--- a/test.rs	Thu Jan 01 00:00:00 1970 +0000
+++ b/test.rs	Thu Jan 01 00:00:00 1970 +0000
@@ -1,3 +1,4 @@
 fn main() {
+    println!("hello");
     println!("world");
 }
"#;

        let result = parse_unified_diff(diff).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].status, FileStatus::Modified);
        assert_eq!(result[0].hunks.len(), 1);
        assert_eq!(result[0].hunks[0].lines.len(), 4);
    }

    #[test]
    fn should_parse_new_file() {
        let diff = r#"diff -r 000000000000 new_file.rs
--- /dev/null
+++ b/new_file.rs
@@ -0,0 +1,2 @@
+fn new() {
+}
"#;

        let result = parse_unified_diff(diff).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].status, FileStatus::Added);
        assert!(result[0].old_path.is_none());
        assert_eq!(
            result[0].new_path.as_ref().unwrap().to_str().unwrap(),
            "new_file.rs"
        );
    }

    #[test]
    fn should_parse_deleted_file() {
        let diff = r#"diff -r abc123 old_file.rs
--- a/old_file.rs
+++ /dev/null
@@ -1,2 +0,0 @@
-fn old() {
-}
"#;

        let result = parse_unified_diff(diff).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].status, FileStatus::Deleted);
        assert_eq!(
            result[0].old_path.as_ref().unwrap().to_str().unwrap(),
            "old_file.rs"
        );
        assert!(result[0].new_path.is_none());
    }

    #[test]
    fn should_return_no_changes_for_empty_diff() {
        let diff = "";
        let result = parse_unified_diff(diff);
        assert!(matches!(result, Err(TuicrError::NoChanges)));
    }

    #[test]
    fn should_parse_multiple_files() {
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

        let result = parse_unified_diff(diff).unwrap();
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
    fn should_parse_multiple_hunks() {
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

        let result = parse_unified_diff(diff).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].hunks.len(), 2);
        assert_eq!(result[0].hunks[0].old_start, 1);
        assert_eq!(result[0].hunks[1].old_start, 10);
    }

    #[test]
    fn should_parse_renamed_file() {
        let diff = r#"diff -r abc123 new_name.rs
rename from old_name.rs
rename to new_name.rs
--- a/old_name.rs
+++ b/new_name.rs
@@ -1,1 +1,1 @@
-old content
+new content
"#;

        let result = parse_unified_diff(diff).unwrap();
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
    fn should_parse_binary_file() {
        let diff = r#"diff -r abc123 image.png
Binary file image.png has changed
"#;

        let result = parse_unified_diff(diff).unwrap();
        assert_eq!(result.len(), 1);
        assert!(result[0].is_binary);
        assert!(result[0].hunks.is_empty());
    }

    #[test]
    fn should_handle_no_newline_marker() {
        let diff = r#"diff -r abc123 no_newline.rs
--- a/no_newline.rs
+++ b/no_newline.rs
@@ -1,1 +1,1 @@
-old
\ No newline at end of file
+new
\ No newline at end of file
"#;

        let result = parse_unified_diff(diff).unwrap();
        assert_eq!(result.len(), 1);
        // The "\ No newline" markers should be skipped
        assert_eq!(result[0].hunks[0].lines.len(), 2);
    }

    #[test]
    fn should_parse_line_numbers_correctly() {
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

        let result = parse_unified_diff(diff).unwrap();
        let lines = &result[0].hunks[0].lines;

        // Context line at old:5, new:5
        assert_eq!(lines[0].origin, LineOrigin::Context);
        assert_eq!(lines[0].old_lineno, Some(5));
        assert_eq!(lines[0].new_lineno, Some(5));

        // Deletion at old:6
        assert_eq!(lines[1].origin, LineOrigin::Deletion);
        assert_eq!(lines[1].old_lineno, Some(6));
        assert_eq!(lines[1].new_lineno, None);

        // Addition at new:6
        assert_eq!(lines[2].origin, LineOrigin::Addition);
        assert_eq!(lines[2].old_lineno, None);
        assert_eq!(lines[2].new_lineno, Some(6));

        // Addition at new:7
        assert_eq!(lines[3].origin, LineOrigin::Addition);
        assert_eq!(lines[3].old_lineno, None);
        assert_eq!(lines[3].new_lineno, Some(7));

        // Context at old:7, new:8
        assert_eq!(lines[4].origin, LineOrigin::Context);
        assert_eq!(lines[4].old_lineno, Some(7));
        assert_eq!(lines[4].new_lineno, Some(8));
    }

    #[test]
    fn should_parse_hunk_header_without_count() {
        // When count is omitted, it defaults to 1
        let (old_start, old_count, new_start, new_count) =
            parse_hunk_header("@@ -5 +10 @@").unwrap();
        assert_eq!(old_start, 5);
        assert_eq!(old_count, 1);
        assert_eq!(new_start, 10);
        assert_eq!(new_count, 1);
    }

    #[test]
    fn should_parse_hunk_header_with_context() {
        // Hunk headers can have optional context after the second @@
        let (old_start, old_count, new_start, new_count) =
            parse_hunk_header("@@ -1,5 +1,6 @@ fn main() {").unwrap();
        assert_eq!(old_start, 1);
        assert_eq!(old_count, 5);
        assert_eq!(new_start, 1);
        assert_eq!(new_count, 6);
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
        // Invalid numbers default to 1
        assert_eq!(parse_range("abc"), (1, 1));
        assert_eq!(parse_range("abc,def"), (1, 1));
    }
}
