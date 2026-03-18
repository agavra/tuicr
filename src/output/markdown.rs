use std::fmt::Write;
use std::io::Write as IoWrite;

use arboard::Clipboard;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

use crate::app::{CommentTypeDefinition, DiffSource};
use crate::error::{Result, TuicrError};
use crate::model::{CommentType, LineRange, LineSide, ReviewSession};

/// (file_path, line_range, side, comment_type, content)
type CommentEntry<'a> = (String, Option<LineRange>, Option<LineSide>, String, &'a str);

/// Generate markdown content from the review session.
/// Returns the markdown string or an error if there are no comments.
pub fn generate_export_content(
    session: &ReviewSession,
    diff_source: &DiffSource,
    comment_types: &[CommentTypeDefinition],
) -> Result<String> {
    if !session.has_comments() {
        return Err(TuicrError::NoComments);
    }
    Ok(generate_markdown(session, diff_source, comment_types))
}

pub fn export_to_clipboard(
    session: &ReviewSession,
    diff_source: &DiffSource,
    comment_types: &[CommentTypeDefinition],
) -> Result<String> {
    let content = generate_export_content(session, diff_source, comment_types)?;

    // Prefer OSC 52 in tmux/SSH where arboard may silently fail
    if should_prefer_osc52() {
        copy_osc52(&content)?;
        return Ok("Review copied to clipboard (via terminal)".to_string());
    }

    // Try arboard (system clipboard) first, fall back to OSC 52 for SSH/remote sessions
    match Clipboard::new().and_then(|mut cb| cb.set_text(&content)) {
        Ok(_) => Ok("Review copied to clipboard".to_string()),
        Err(_) => {
            // Fall back to OSC 52 escape sequence (works over SSH)
            copy_osc52(&content)?;
            Ok("Review copied to clipboard (via terminal)".to_string())
        }
    }
}

/// Returns true if we should prefer OSC 52 over the system clipboard.
///
/// In tmux or SSH sessions, arboard may "succeed" but copy to an inaccessible
/// X11 clipboard, so we use OSC 52 which works reliably in these environments.
fn should_prefer_osc52() -> bool {
    std::env::var("TMUX").is_ok()
        || std::env::var("SSH_TTY").is_ok()
        || std::env::var("ZELLIJ").is_ok()
}

/// Copy text to clipboard using OSC 52 escape sequence.
/// In tmux, raw OSC 52 is intercepted and may not reach the outer terminal.
/// We use `tmux load-buffer -w` which tells tmux to handle the clipboard copy itself.
fn copy_osc52(text: &str) -> Result<()> {
    if std::env::var("TMUX").is_ok() {
        copy_via_tmux(text)
    } else {
        let mut stdout = std::io::stdout().lock();
        write_osc52(&mut stdout, text)
    }
}

/// Copy text to the system clipboard via `tmux load-buffer -w -`.
/// The `-w` flag tells tmux to also forward to the outer terminal's clipboard via OSC 52.
fn copy_via_tmux(text: &str) -> Result<()> {
    use std::process::{Command, Stdio};

    let mut child = Command::new("tmux")
        .args(["load-buffer", "-w", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| TuicrError::Clipboard(format!("Failed to run tmux: {e}")))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(text.as_bytes())
            .map_err(|e| TuicrError::Clipboard(format!("Failed to write to tmux: {e}")))?;
    }

    let status = child
        .wait()
        .map_err(|e| TuicrError::Clipboard(format!("tmux load-buffer failed: {e}")))?;

    if !status.success() {
        return Err(TuicrError::Clipboard(
            "tmux load-buffer exited with error".to_string(),
        ));
    }

    Ok(())
}

/// Write OSC 52 escape sequence to the given writer.
/// Separated for testability.
fn write_osc52<W: IoWrite>(writer: &mut W, text: &str) -> Result<()> {
    let encoded = BASE64.encode(text);
    write!(writer, "\x1b]52;c;{encoded}\x07")
        .map_err(|e| TuicrError::Clipboard(format!("Failed to write OSC 52: {e}")))?;
    writer
        .flush()
        .map_err(|e| TuicrError::Clipboard(format!("Failed to flush: {e}")))?;
    Ok(())
}

fn review_scope_label(diff_source: &DiffSource) -> String {
    let scope = match diff_source {
        DiffSource::WorkingTree => "working tree changes".to_string(),
        DiffSource::CommitRange(_) => "selected commit range".to_string(),
        DiffSource::WorkingTreeAndCommits(_) => {
            "selected commit range + working tree changes".to_string()
        }
    };

    format!("Review Comment (scope: {scope})")
}

fn generate_markdown(
    session: &ReviewSession,
    diff_source: &DiffSource,
    comment_types: &[CommentTypeDefinition],
) -> String {
    let mut md = String::new();

    // Intro for agents
    let _ = writeln!(
        md,
        "I reviewed your code and have the following comments. Please address them."
    );
    let _ = writeln!(md);

    // Include commit range info if reviewing commits
    match diff_source {
        DiffSource::WorkingTree => {}
        DiffSource::CommitRange(commits) => {
            if commits.len() == 1 {
                let _ = writeln!(
                    md,
                    "Reviewing commit: {}",
                    &commits[0][..7.min(commits[0].len())]
                );
            } else {
                let short_ids: Vec<&str> = commits.iter().map(|c| &c[..7.min(c.len())]).collect();
                let _ = writeln!(md, "Reviewing commits: {}", short_ids.join(", "));
            }
            let _ = writeln!(md);
        }
        DiffSource::WorkingTreeAndCommits(commits) => {
            let short_ids: Vec<&str> = commits.iter().map(|c| &c[..7.min(c.len())]).collect();
            let _ = writeln!(
                md,
                "Reviewing working tree + commits: {}",
                short_ids.join(", ")
            );
            let _ = writeln!(md);
        }
    }

    let legend = if comment_types.is_empty() {
        "NOTE, SUGGESTION, ISSUE, PRAISE".to_string()
    } else {
        comment_types
            .iter()
            .map(|comment_type| {
                let definition = comment_type
                    .definition
                    .as_deref()
                    .unwrap_or(comment_type.id.as_str());
                format!(
                    "{} ({})",
                    comment_type.label.to_ascii_uppercase(),
                    definition
                )
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    let _ = writeln!(md, "Comment types: {legend}");
    let _ = writeln!(md);

    // Session notes/summary
    if let Some(notes) = &session.session_notes {
        let _ = writeln!(md, "Summary: {notes}");
        let _ = writeln!(md);
    }

    // Collect all comments into a flat list
    let mut all_comments: Vec<CommentEntry> = Vec::new();
    let review_comment_location = review_scope_label(diff_source);

    for comment in &session.review_comments {
        all_comments.push((
            review_comment_location.clone(),
            None,
            None,
            export_comment_type_label(&comment.comment_type, comment_types),
            &comment.content,
        ));
    }

    // Sort files by path for consistent output
    let mut files: Vec<_> = session.files.iter().collect();
    files.sort_by_key(|(path, _)| path.to_string_lossy().to_string());

    for (path, review) in files {
        let path_str = path.display().to_string();

        // File comments (no line number)
        for comment in &review.file_comments {
            all_comments.push((
                path_str.clone(),
                None,
                None,
                export_comment_type_label(&comment.comment_type, comment_types),
                &comment.content,
            ));
        }

        // Line comments (with line number, sorted)
        let mut line_comments: Vec<_> = review.line_comments.iter().collect();
        line_comments.sort_by_key(|(line, _)| *line);

        for (line, comments) in line_comments {
            for comment in comments {
                // Use comment's line_range if available, otherwise use the key line
                let line_range = comment
                    .line_range
                    .or_else(|| Some(LineRange::single(*line)));
                all_comments.push((
                    path_str.clone(),
                    line_range,
                    comment.side,
                    export_comment_type_label(&comment.comment_type, comment_types),
                    &comment.content,
                ));
            }
        }
    }

    // Output numbered list
    for (i, (file, line_range, side, comment_type, content)) in all_comments.iter().enumerate() {
        let location = match (line_range, side) {
            // Range on deleted side (old lines)
            (Some(range), Some(LineSide::Old)) if range.is_single() => {
                format!("`{}:~{}`", file, range.start)
            }
            (Some(range), Some(LineSide::Old)) => {
                format!("`{}:~{}-~{}`", file, range.start, range.end)
            }
            // Range on new/context side
            (Some(range), _) if range.is_single() => {
                format!("`{}:{}`", file, range.start)
            }
            (Some(range), _) => {
                format!("`{}:{}-{}`", file, range.start, range.end)
            }
            // File comment
            (None, _) => format!("`{file}`"),
        };
        let _ = writeln!(
            md,
            "{}. **[{}]** {} - {}",
            i + 1,
            comment_type,
            location,
            content
        );
    }

    md
}

fn export_comment_type_label(
    comment_type: &CommentType,
    comment_types: &[CommentTypeDefinition],
) -> String {
    if let Some(definition) = comment_types
        .iter()
        .find(|definition| definition.id == comment_type.id())
    {
        return definition.label.to_ascii_uppercase();
    }

    comment_type.as_str()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::CommentTypeDefinition;
    use crate::model::{Comment, CommentType, FileStatus, LineRange, LineSide, SessionDiffSource};
    use std::path::PathBuf;

    fn comment_types() -> Vec<CommentTypeDefinition> {
        vec![
            CommentTypeDefinition {
                id: "note".to_string(),
                label: "note".to_string(),
                definition: Some("observations".to_string()),
                color: None,
            },
            CommentTypeDefinition {
                id: "suggestion".to_string(),
                label: "suggestion".to_string(),
                definition: Some("improvements".to_string()),
                color: None,
            },
            CommentTypeDefinition {
                id: "issue".to_string(),
                label: "issue".to_string(),
                definition: Some("problems to fix".to_string()),
                color: None,
            },
            CommentTypeDefinition {
                id: "praise".to_string(),
                label: "praise".to_string(),
                definition: Some("positive feedback".to_string()),
                color: None,
            },
        ]
    }

    fn create_test_session() -> ReviewSession {
        let mut session = ReviewSession::new(
            PathBuf::from("/tmp/test-repo"),
            "abc1234def".to_string(),
            Some("main".to_string()),
            SessionDiffSource::WorkingTree,
        );
        session.add_file(PathBuf::from("src/main.rs"), FileStatus::Modified);

        // Add a file comment
        if let Some(review) = session.get_file_mut(&PathBuf::from("src/main.rs")) {
            review.reviewed = true;
            review.add_file_comment(Comment::new(
                "Consider adding documentation".to_string(),
                CommentType::Suggestion,
                None,
            ));
            review.add_line_comment(
                42,
                Comment::new(
                    "Magic number should be a constant".to_string(),
                    CommentType::Issue,
                    Some(LineSide::New),
                ),
            );
        }

        session
    }

    #[test]
    fn should_generate_valid_markdown() {
        // given
        let session = create_test_session();
        let diff_source = DiffSource::WorkingTree;

        // when
        let markdown = generate_markdown(&session, &diff_source, &comment_types());

        // then
        assert!(markdown.contains("I reviewed your code and have the following comments"));
        assert!(markdown.contains("Comment types:"));
        assert!(markdown.contains("SUGGESTION (improvements)"));
        assert!(markdown.contains("[SUGGESTION]"));
        assert!(markdown.contains("`src/main.rs`"));
        assert!(markdown.contains("Consider adding documentation"));
        assert!(markdown.contains("[ISSUE]"));
        assert!(markdown.contains("`src/main.rs:42`"));
        assert!(markdown.contains("Magic number"));
    }

    #[test]
    fn should_use_configured_label_and_definition_in_export() {
        let mut session = ReviewSession::new(
            PathBuf::from("/tmp/test-repo"),
            "abc1234def".to_string(),
            Some("main".to_string()),
            SessionDiffSource::WorkingTree,
        );
        session.add_file(PathBuf::from("src/main.rs"), FileStatus::Modified);
        if let Some(review) = session.get_file_mut(&PathBuf::from("src/main.rs")) {
            review.add_file_comment(Comment::new(
                "Needs clarification".to_string(),
                CommentType::Note,
                None,
            ));
        }

        let custom_types = vec![CommentTypeDefinition {
            id: "note".to_string(),
            label: "question".to_string(),
            definition: Some("ask for clarification".to_string()),
            color: None,
        }];

        let markdown = generate_markdown(&session, &DiffSource::WorkingTree, &custom_types);

        assert!(markdown.contains("Comment types: QUESTION (ask for clarification)"));
        assert!(markdown.contains("**[QUESTION]**"));
    }

    #[test]
    fn should_number_comments_sequentially() {
        // given
        let session = create_test_session();
        let diff_source = DiffSource::WorkingTree;

        // when
        let markdown = generate_markdown(&session, &diff_source, &comment_types());

        // then
        // Should have 2 numbered comments
        assert!(markdown.contains("1. **[SUGGESTION]**"));
        assert!(markdown.contains("2. **[ISSUE]**"));
    }

    #[test]
    fn should_include_review_comments_in_export() {
        let mut session = create_test_session();
        session.review_comments.push(Comment::new(
            "Please split this into smaller commits".to_string(),
            CommentType::Note,
            None,
        ));

        let markdown = generate_markdown(&session, &DiffSource::WorkingTree, &comment_types());

        assert!(markdown
            .contains("`Review Comment (scope: working tree changes)` - Please split this into smaller commits"));
    }

    #[test]
    fn should_include_commit_range_scope_for_review_comments() {
        let mut session = create_test_session();
        session.review_comments.push(Comment::new(
            "High-level concern across commits".to_string(),
            CommentType::Issue,
            None,
        ));

        let markdown = generate_markdown(
            &session,
            &DiffSource::CommitRange(vec!["abc1234567890".to_string()]),
            &comment_types(),
        );

        assert!(markdown.contains(
            "`Review Comment (scope: selected commit range)` - High-level concern across commits"
        ));
    }

    #[test]
    fn should_fail_export_when_no_comments() {
        // given
        let session = ReviewSession::new(
            PathBuf::from("/tmp/test-repo"),
            "abc1234def".to_string(),
            Some("main".to_string()),
            SessionDiffSource::WorkingTree,
        );
        let diff_source = DiffSource::WorkingTree;

        // when
        let result = export_to_clipboard(&session, &diff_source, &comment_types());

        // then
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), TuicrError::NoComments));
    }

    #[test]
    fn should_generate_export_content_with_comments() {
        // given
        let session = create_test_session();
        let diff_source = DiffSource::WorkingTree;

        // when
        let result = generate_export_content(&session, &diff_source, &comment_types());

        // then
        assert!(result.is_ok());
        let content = result.unwrap();
        assert!(content.contains("I reviewed your code"));
        assert!(content.contains("[SUGGESTION]"));
        assert!(content.contains("[ISSUE]"));
    }

    #[test]
    fn should_fail_generate_export_content_when_no_comments() {
        // given
        let session = ReviewSession::new(
            PathBuf::from("/tmp/test-repo"),
            "abc1234def".to_string(),
            Some("main".to_string()),
            SessionDiffSource::WorkingTree,
        );
        let diff_source = DiffSource::WorkingTree;

        // when
        let result = generate_export_content(&session, &diff_source, &comment_types());

        // then
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), TuicrError::NoComments));
    }

    #[test]
    fn should_include_commit_range_in_markdown() {
        // given
        let session = create_test_session();
        let diff_source = DiffSource::CommitRange(vec![
            "abc1234567890".to_string(),
            "def4567890123".to_string(),
        ]);

        // when
        let markdown = generate_markdown(&session, &diff_source, &comment_types());

        // then
        assert!(markdown.contains("Reviewing commits: abc1234, def4567"));
    }

    #[test]
    fn should_include_single_commit_in_markdown() {
        // given
        let session = create_test_session();
        let diff_source = DiffSource::CommitRange(vec!["abc1234567890".to_string()]);

        // when
        let markdown = generate_markdown(&session, &diff_source, &comment_types());

        // then
        assert!(markdown.contains("Reviewing commit: abc1234"));
    }

    #[test]
    fn should_write_osc52_escape_sequence() {
        // given
        let text = "Hello, World!";
        let mut buffer: Vec<u8> = Vec::new();

        // when
        write_osc52(&mut buffer, text).unwrap();

        // then
        let output = String::from_utf8(buffer).unwrap();
        // OSC 52 format: ESC ] 52 ; c ; <base64> BEL
        assert!(output.starts_with("\x1b]52;c;"));
        assert!(output.ends_with("\x07"));
        // Verify the base64 content
        let base64_content = &output[7..output.len() - 1];
        assert_eq!(BASE64.encode(text), base64_content);
    }

    #[test]
    fn should_encode_empty_string_in_osc52() {
        // given
        let text = "";
        let mut buffer: Vec<u8> = Vec::new();

        // when
        write_osc52(&mut buffer, text).unwrap();

        // then
        let output = String::from_utf8(buffer).unwrap();
        assert_eq!(output, "\x1b]52;c;\x07");
    }

    #[test]
    fn should_encode_unicode_in_osc52() {
        // given
        let text = "こんにちは 🦀";
        let mut buffer: Vec<u8> = Vec::new();

        // when
        write_osc52(&mut buffer, text).unwrap();

        // then
        let output = String::from_utf8(buffer).unwrap();
        let base64_content = &output[7..output.len() - 1];
        // Decode and verify it matches original
        let decoded = String::from_utf8(BASE64.decode(base64_content).unwrap()).unwrap();
        assert_eq!(decoded, text);
    }

    #[test]
    fn should_encode_markdown_content_in_osc52() {
        // given - simulate what would be copied during export
        let session = create_test_session();
        let diff_source = DiffSource::WorkingTree;
        let markdown = generate_markdown(&session, &diff_source, &comment_types());
        let mut buffer: Vec<u8> = Vec::new();

        // when
        write_osc52(&mut buffer, &markdown).unwrap();

        // then
        let output = String::from_utf8(buffer).unwrap();
        assert!(output.starts_with("\x1b]52;c;"));
        assert!(output.ends_with("\x07"));
        // Verify we can decode the base64 back to the original markdown
        let base64_content = &output[7..output.len() - 1];
        let decoded = String::from_utf8(BASE64.decode(base64_content).unwrap()).unwrap();
        assert_eq!(decoded, markdown);
    }

    #[test]
    fn should_export_single_line_range_as_single_line() {
        // given - a comment with a single-line range should display as L42, not L42-L42
        let mut session = ReviewSession::new(
            PathBuf::from("/tmp/test-repo"),
            "abc1234def".to_string(),
            Some("main".to_string()),
            SessionDiffSource::WorkingTree,
        );
        session.add_file(PathBuf::from("src/main.rs"), FileStatus::Modified);

        if let Some(review) = session.get_file_mut(&PathBuf::from("src/main.rs")) {
            let range = LineRange::single(42);
            review.add_line_comment(
                42,
                Comment::new_with_range(
                    "Single line comment".to_string(),
                    CommentType::Note,
                    Some(LineSide::New),
                    range,
                ),
            );
        }
        let diff_source = DiffSource::WorkingTree;

        // when
        let markdown = generate_markdown(&session, &diff_source, &comment_types());

        // then
        assert!(markdown.contains("`src/main.rs:42`"));
        assert!(!markdown.contains("`src/main.rs:42-42`"));
    }

    #[test]
    fn should_export_line_range_with_start_and_end() {
        // given - a comment spanning multiple lines
        let mut session = ReviewSession::new(
            PathBuf::from("/tmp/test-repo"),
            "abc1234def".to_string(),
            Some("main".to_string()),
            SessionDiffSource::WorkingTree,
        );
        session.add_file(PathBuf::from("src/main.rs"), FileStatus::Modified);

        if let Some(review) = session.get_file_mut(&PathBuf::from("src/main.rs")) {
            let range = LineRange::new(10, 15);
            review.add_line_comment(
                15, // keyed by end line
                Comment::new_with_range(
                    "Multi-line comment".to_string(),
                    CommentType::Issue,
                    Some(LineSide::New),
                    range,
                ),
            );
        }
        let diff_source = DiffSource::WorkingTree;

        // when
        let markdown = generate_markdown(&session, &diff_source, &comment_types());

        // then
        assert!(markdown.contains("`src/main.rs:10-15`"));
        assert!(markdown.contains("Multi-line comment"));
    }

    #[test]
    fn should_export_old_side_line_range_with_tilde() {
        // given - a range comment on deleted lines (old side)
        let mut session = ReviewSession::new(
            PathBuf::from("/tmp/test-repo"),
            "abc1234def".to_string(),
            Some("main".to_string()),
            SessionDiffSource::WorkingTree,
        );
        session.add_file(PathBuf::from("src/main.rs"), FileStatus::Modified);

        if let Some(review) = session.get_file_mut(&PathBuf::from("src/main.rs")) {
            let range = LineRange::new(20, 25);
            review.add_line_comment(
                25, // keyed by end line
                Comment::new_with_range(
                    "Deleted lines comment".to_string(),
                    CommentType::Suggestion,
                    Some(LineSide::Old),
                    range,
                ),
            );
        }
        let diff_source = DiffSource::WorkingTree;

        // when
        let markdown = generate_markdown(&session, &diff_source, &comment_types());

        // then
        assert!(markdown.contains("`src/main.rs:~20-~25`"));
    }

    #[test]
    fn should_export_single_old_side_line_with_tilde() {
        // given - a single line comment on a deleted line
        let mut session = ReviewSession::new(
            PathBuf::from("/tmp/test-repo"),
            "abc1234def".to_string(),
            Some("main".to_string()),
            SessionDiffSource::WorkingTree,
        );
        session.add_file(PathBuf::from("src/main.rs"), FileStatus::Modified);

        if let Some(review) = session.get_file_mut(&PathBuf::from("src/main.rs")) {
            let range = LineRange::single(30);
            review.add_line_comment(
                30,
                Comment::new_with_range(
                    "Single deleted line".to_string(),
                    CommentType::Note,
                    Some(LineSide::Old),
                    range,
                ),
            );
        }
        let diff_source = DiffSource::WorkingTree;

        // when
        let markdown = generate_markdown(&session, &diff_source, &comment_types());

        // then
        assert!(markdown.contains("`src/main.rs:~30`"));
        assert!(!markdown.contains("`src/main.rs:~30-~30`"));
    }

    #[test]
    fn should_handle_comment_without_line_range_field() {
        // given - backward compatibility: comment without line_range uses line number
        let mut session = ReviewSession::new(
            PathBuf::from("/tmp/test-repo"),
            "abc1234def".to_string(),
            Some("main".to_string()),
            SessionDiffSource::WorkingTree,
        );
        session.add_file(PathBuf::from("src/main.rs"), FileStatus::Modified);

        if let Some(review) = session.get_file_mut(&PathBuf::from("src/main.rs")) {
            // Use Comment::new which sets line_range to None
            review.add_line_comment(
                50,
                Comment::new(
                    "Old style comment".to_string(),
                    CommentType::Note,
                    Some(LineSide::New),
                ),
            );
        }
        let diff_source = DiffSource::WorkingTree;

        // when
        let markdown = generate_markdown(&session, &diff_source, &comment_types());

        // then
        assert!(markdown.contains("`src/main.rs:50`"));
    }
}
