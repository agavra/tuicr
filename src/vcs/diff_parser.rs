//! Materialize structured file patches into the diff model.
//!
//! File identity is deliberately outside this module. Each backend obtains
//! paths and status from a machine-readable source (`git --raw -z`, jj
//! templates, Mercurial status templates, or forge JSON) and supplies them in
//! [`FilePatch`]. This module only parses unified-diff hunks, where the `@@`
//! header's line counts make the grammar unambiguous.

use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;

use crate::error::{Result, TuicrError};
#[cfg(test)]
use crate::model::FileStatus;
use crate::model::{DiffFile, DiffHunk, DiffLine, FilePatch, LineOrigin};
use crate::syntax::{SyntaxHighlighter, needs_full_file_highlight};

/// Convert backend-structured file patches into renderable diff files.
pub fn parse_file_patches(
    patches: Vec<FilePatch>,
    highlighter: &SyntaxHighlighter,
) -> Result<Vec<DiffFile>> {
    if patches.is_empty() {
        return Err(TuicrError::NoChanges);
    }

    patches
        .into_iter()
        .map(|patch| materialize_file_patch(patch, highlighter))
        .collect()
}

fn materialize_file_patch(patch: FilePatch, highlighter: &SyntaxHighlighter) -> Result<DiffFile> {
    let file_path = patch.display_path().ok_or_else(|| {
        TuicrError::VcsCommand("structured diff entry has neither an old nor a new path".into())
    })?;
    let hunks = if patch.is_binary || patch.is_too_large {
        Vec::new()
    } else {
        parse_hunks(&patch.patch, file_path, highlighter)?
    };
    let content_hash = DiffFile::compute_content_hash(&hunks);

    Ok(DiffFile {
        old_path: patch.old_path,
        new_path: patch.new_path,
        status: patch.status,
        hunks,
        is_binary: patch.is_binary,
        is_too_large: patch.is_too_large,
        commit_message_sha: None,
        content_hash,
    })
}

fn parse_hunks(
    patch: &str,
    file_path: &Path,
    highlighter: &SyntaxHighlighter,
) -> Result<Vec<DiffHunk>> {
    let mut hunks = Vec::new();
    let mut lines = patch.lines().peekable();
    let mut parsed_hunk = false;

    while let Some(line) = lines.next() {
        if line.starts_with("@@ ") {
            hunks.push(parse_hunk(line, &mut lines, file_path, highlighter)?);
            parsed_hunk = true;
        } else if line.starts_with("@@") {
            return Err(invalid_patch(
                file_path,
                format!("unsupported or malformed hunk header `{line}`"),
            ));
        } else if parsed_hunk
            && !line.starts_with('\\')
            && matches!(line.as_bytes().first(), Some(b' ' | b'+' | b'-'))
        {
            return Err(invalid_patch(
                file_path,
                format!("body line appears outside its declared hunk range: `{line}`"),
            ));
        }
    }

    Ok(hunks)
}

fn parse_hunk<'a, I>(
    header: &str,
    lines: &mut std::iter::Peekable<I>,
    file_path: &Path,
    highlighter: &SyntaxHighlighter,
) -> Result<DiffHunk>
where
    I: Iterator<Item = &'a str>,
{
    let (old_start, old_count, new_start, new_count) = parse_hunk_header(header)
        .ok_or_else(|| invalid_patch(file_path, format!("malformed hunk header `{header}`")))?;
    if !range_fits_u32(old_start, old_count) || !range_fits_u32(new_start, new_count) {
        return Err(invalid_patch(
            file_path,
            format!("hunk range overflows a 32-bit line number in `{header}`"),
        ));
    }

    let mut line_contents = Vec::new();
    let mut line_origins = Vec::new();
    let mut line_numbers = Vec::new();
    let mut old_lineno = old_start;
    let mut new_lineno = new_start;
    let mut old_remaining = old_count;
    let mut new_remaining = new_count;

    // Counts from the hunk header are the grammar. Prefix-looking content is
    // still content: deleting `-- comment` yields `--- comment`, and adding
    // `++i` yields `+++i`. We consume until both declared sides are complete,
    // rather than looking for path-like sentinel strings inside the body.
    while old_remaining > 0 || new_remaining > 0 {
        let line = lines.next().ok_or_else(|| {
            invalid_patch(
                file_path,
                format!(
                    "hunk `{header}` ended early (missing {old_remaining} old and {new_remaining} new lines)"
                ),
            )
        })?;

        if line.starts_with('\\') {
            // `\ No newline at end of file` does not consume either side.
            continue;
        }

        let (origin, content, old_ln, new_ln) = if let Some(content) = line.strip_prefix('+') {
            if new_remaining == 0 {
                return Err(invalid_patch(
                    file_path,
                    format!("hunk `{header}` contains too many new-side lines"),
                ));
            }
            let line_number = new_lineno;
            new_lineno += 1;
            new_remaining -= 1;
            (LineOrigin::Addition, content, None, Some(line_number))
        } else if let Some(content) = line.strip_prefix('-') {
            if old_remaining == 0 {
                return Err(invalid_patch(
                    file_path,
                    format!("hunk `{header}` contains too many old-side lines"),
                ));
            }
            let line_number = old_lineno;
            old_lineno += 1;
            old_remaining -= 1;
            (LineOrigin::Deletion, content, Some(line_number), None)
        } else if let Some(content) = line.strip_prefix(' ') {
            if old_remaining == 0 || new_remaining == 0 {
                return Err(invalid_patch(
                    file_path,
                    format!("hunk `{header}` contains too many context lines"),
                ));
            }
            let old_line_number = old_lineno;
            let new_line_number = new_lineno;
            old_lineno += 1;
            new_lineno += 1;
            old_remaining -= 1;
            new_remaining -= 1;
            (
                LineOrigin::Context,
                content,
                Some(old_line_number),
                Some(new_line_number),
            )
        } else {
            return Err(invalid_patch(
                file_path,
                format!("hunk `{header}` contains an unprefixed body line `{line}`"),
            ));
        };

        line_contents.push(super::tabify(content));
        line_origins.push(origin);
        line_numbers.push((old_ln, new_ln));
    }

    let highlight_sequences =
        SyntaxHighlighter::split_diff_lines_for_highlighting(&line_contents, &line_origins);
    let (old_highlighted_lines, new_highlighted_lines) = if !needs_full_file_highlight(file_path) {
        (
            highlighter.highlight_file_lines(file_path, &highlight_sequences.old_lines),
            highlighter.highlight_file_lines(file_path, &highlight_sequences.new_lines),
        )
    } else {
        (None, None)
    };

    let lines = line_contents
        .into_iter()
        .enumerate()
        .map(|(index, content)| {
            let origin = line_origins[index];
            let (old_lineno, new_lineno) = line_numbers[index];
            let highlighted_spans = highlighter.highlighted_line_for_diff_with_background(
                old_highlighted_lines.as_deref(),
                new_highlighted_lines.as_deref(),
                highlight_sequences.old_line_indices[index],
                highlight_sequences.new_line_indices[index],
                origin,
            );
            DiffLine {
                origin,
                content,
                old_lineno,
                new_lineno,
                highlighted_spans,
            }
        })
        .collect();

    Ok(DiffHunk {
        header: header.to_string(),
        lines,
        old_start,
        old_count,
        new_start,
        new_count,
    })
}

fn invalid_patch(path: &Path, detail: String) -> TuicrError {
    TuicrError::VcsCommand(format!(
        "invalid unified diff for {}: {detail}",
        path.display()
    ))
}

fn parse_hunk_header(line: &str) -> Option<(u32, u32, u32, u32)> {
    let tail = line.strip_prefix("@@ ")?;
    let (ranges, _) = tail.split_once(" @@")?;
    let mut parts = ranges.split_whitespace();
    let old = parts.next()?.strip_prefix('-')?;
    let new = parts.next()?.strip_prefix('+')?;
    if parts.next().is_some() {
        return None;
    }
    let (old_start, old_count) = parse_range(old)?;
    let (new_start, new_count) = parse_range(new)?;
    Some((old_start, old_count, new_start, new_count))
}

fn parse_range(range: &str) -> Option<(u32, u32)> {
    match range.split_once(',') {
        Some((start, count)) => Some((start.parse().ok()?, count.parse().ok()?)),
        None => Some((range.parse().ok()?, 1)),
    }
}

fn range_fits_u32(start: u32, count: u32) -> bool {
    count == 0 || start.checked_add(count - 1).is_some()
}

/// Adapt compact hand-written Git fixtures used throughout the unit tests.
/// Production code must obtain metadata from its backend instead.
#[cfg(test)]
pub(crate) fn git_fixture_file_patches(patch: &str) -> Vec<FilePatch> {
    crate::vcs::git::raw::split_patch_blocks(patch.as_bytes())
        .into_iter()
        .map(|block| {
            let text = String::from_utf8(block.to_vec()).expect("test patch must be UTF-8");
            let mut lines = text.lines();
            let header = lines.next().expect("test patch block needs a header");
            let paths = header
                .strip_prefix("diff --git a/")
                .and_then(|rest| rest.split_once(" b/"))
                .expect("test patch header must use simple a/ and b/ paths");
            let mut old_path = Some(PathBuf::from(paths.0));
            let mut new_path = Some(PathBuf::from(paths.1));
            let mut status = FileStatus::Modified;
            for line in text.lines() {
                if line.starts_with("new file mode") {
                    old_path = None;
                    status = FileStatus::Added;
                } else if line.starts_with("deleted file mode") {
                    new_path = None;
                    status = FileStatus::Deleted;
                } else if let Some(path) = line.strip_prefix("rename from ") {
                    old_path = Some(PathBuf::from(path));
                    status = FileStatus::Renamed;
                } else if let Some(path) = line.strip_prefix("rename to ") {
                    new_path = Some(PathBuf::from(path));
                } else if let Some(path) = line.strip_prefix("copy from ") {
                    old_path = Some(PathBuf::from(path));
                    status = FileStatus::Copied;
                } else if let Some(path) = line.strip_prefix("copy to ") {
                    new_path = Some(PathBuf::from(path));
                }
            }
            FilePatch::new(old_path, new_path, status, text)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(patch: &str) -> Result<Vec<DiffFile>> {
        parse_file_patches(
            vec![FilePatch::new(
                Some(PathBuf::from("file.txt")),
                Some(PathBuf::from("file.txt")),
                FileStatus::Modified,
                patch,
            )],
            &SyntaxHighlighter::default(),
        )
    }

    #[test]
    fn returns_no_changes_for_empty_patch_set() {
        assert!(matches!(
            parse_file_patches(Vec::new(), &SyntaxHighlighter::default()),
            Err(TuicrError::NoChanges)
        ));
    }

    #[test]
    fn uses_structured_metadata_instead_of_display_headers() {
        let patch = r#"diff --git \"a/\\346\\227\\245.txt\" \"b/\\346\\227\\245.txt\"
--- \"a/\\346\\227\\245.txt\"
+++ \"b/\\346\\227\\245.txt\"
@@ -1 +1 @@
-old
+new
"#;
        let expected = PathBuf::from("日.txt");
        let files = parse_file_patches(
            vec![FilePatch::new(
                Some(expected.clone()),
                Some(expected.clone()),
                FileStatus::Modified,
                patch,
            )],
            &SyntaxHighlighter::default(),
        )
        .unwrap();

        assert_eq!(files[0].old_path.as_ref(), Some(&expected));
        assert_eq!(files[0].new_path.as_ref(), Some(&expected));
    }

    #[test]
    fn keeps_deleted_lines_whose_content_looks_like_headers() {
        let files =
            parse("@@ -1,5 +1,2 @@\n----\n-title: hello\n----\n intro\n-old body\n+new body\n")
                .unwrap();
        let lines = &files[0].hunks[0].lines;

        assert_eq!(lines.len(), 6);
        assert_eq!(lines[0].content, "---");
        assert_eq!(lines[0].old_lineno, Some(1));
        assert_eq!(lines[3].old_lineno, Some(4));
        assert_eq!(lines[4].old_lineno, Some(5));
    }

    #[test]
    fn keeps_added_lines_whose_content_starts_with_plus_signs() {
        let files = parse("@@ -1,2 +1,3 @@\n line\n-old\n+++i;\n+ tail\n").unwrap();
        let lines = &files[0].hunks[0].lines;

        assert_eq!(lines.len(), 4);
        assert_eq!(lines[2].content, "++i;");
        assert_eq!(lines[2].new_lineno, Some(2));
        assert_eq!(lines[3].new_lineno, Some(3));
    }

    #[test]
    fn parses_multiple_hunks_by_declared_counts() {
        let files = parse(
            "metadata\n@@ -1 +1 @@ first\n-old\n+new\n@@ -10,2 +10,2 @@ second\n a\n-b\n+c\n",
        )
        .unwrap();

        assert_eq!(files[0].hunks.len(), 2);
        assert_eq!(files[0].hunks[1].lines[1].old_lineno, Some(11));
        assert_eq!(files[0].hunks[1].lines[2].new_lineno, Some(11));
    }

    #[test]
    fn skips_no_newline_markers_without_consuming_a_side() {
        let files = parse(
            "@@ -1 +1 @@\n-old\n\\ No newline at end of file\n+new\n\\ No newline at end of file\n",
        )
        .unwrap();

        assert_eq!(files[0].hunks[0].lines.len(), 2);
    }

    #[test]
    fn rejects_truncated_hunks_instead_of_stealing_the_next_header() {
        let error = parse("@@ -1,2 +1,2 @@\n a\n@@ -5 +5 @@\n-b\n+c\n").unwrap_err();
        assert!(error.to_string().contains("unprefixed body line"));
    }

    #[test]
    fn rejects_malformed_ranges_instead_of_defaulting_to_line_one() {
        let error = parse("@@ -wat +1 @@\n-old\n+new\n").unwrap_err();
        assert!(error.to_string().contains("malformed hunk header"));
    }

    #[test]
    fn rejects_body_lines_beyond_the_declared_counts() {
        let error = parse("@@ -1 +1 @@\n-old\n+new\n+extra\n").unwrap_err();
        assert!(
            error
                .to_string()
                .contains("outside its declared hunk range")
        );
    }

    #[test]
    fn rejects_line_ranges_that_would_overflow() {
        let error = parse("@@ -4294967295,2 +1 @@\n-a\n-b\n+c\n").unwrap_err();
        assert!(error.to_string().contains("overflows"));
    }

    #[test]
    fn binary_and_too_large_files_do_not_parse_patch_text() {
        let mut binary = FilePatch::new(
            Some(PathBuf::from("image.png")),
            Some(PathBuf::from("image.png")),
            FileStatus::Modified,
            "GIT binary patch\n@@ malformed",
        );
        binary.is_binary = true;
        let mut large = FilePatch::new(
            Some(PathBuf::from("large.txt")),
            Some(PathBuf::from("large.txt")),
            FileStatus::Modified,
            "@@ malformed",
        );
        large.is_too_large = true;

        let files = parse_file_patches(vec![binary, large], &SyntaxHighlighter::default()).unwrap();
        assert!(files[0].is_binary);
        assert!(files[0].hunks.is_empty());
        assert!(files[1].is_too_large);
        assert!(files[1].hunks.is_empty());
    }
}
