use std::fmt::Write;

use arboard::Clipboard;

use crate::app::DiffSource;
use crate::error::{Result, TuicrError};
use crate::model::{LineSide, ReviewSession};

/// (file_path, line_number, side, comment_type, content)
type CommentEntry<'a> = (String, Option<u32>, Option<LineSide>, &'a str, &'a str);

pub fn export_to_clipboard(session: &ReviewSession, diff_source: &DiffSource) -> Result<String> {
    // Check if there are any comments to export
    if !session.has_comments() {
        return Err(TuicrError::NoComments);
    }

    let content = generate_markdown(session, diff_source);

    let mut clipboard = Clipboard::new()
        .map_err(|e| TuicrError::Clipboard(format!("Failed to access clipboard: {}", e)))?;

    clipboard
        .set_text(content)
        .map_err(|e| TuicrError::Clipboard(format!("Failed to copy to clipboard: {}", e)))?;

    Ok("Review copied to clipboard".to_string())
}

fn generate_markdown(session: &ReviewSession, diff_source: &DiffSource) -> String {
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
    }

    let _ = writeln!(
        md,
        "Comment types: ISSUE (problems to fix), SUGGESTION (improvements), NOTE (observations), PRAISE (positive feedback)"
    );
    let _ = writeln!(md);

    // Session notes/summary
    if let Some(notes) = &session.session_notes {
        let _ = writeln!(md, "Summary: {}", notes);
        let _ = writeln!(md);
    }

    // Collect all comments into a flat list
    let mut all_comments: Vec<CommentEntry> = Vec::new();

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
                comment.comment_type.as_str(),
                &comment.content,
            ));
        }

        // Line comments (with line number, sorted)
        let mut line_comments: Vec<_> = review.line_comments.iter().collect();
        line_comments.sort_by_key(|(line, _)| *line);

        for (line, comments) in line_comments {
            for comment in comments {
                all_comments.push((
                    path_str.clone(),
                    Some(*line),
                    comment.side,
                    comment.comment_type.as_str(),
                    &comment.content,
                ));
            }
        }
    }

    // Output numbered list
    for (i, (file, line, side, comment_type, content)) in all_comments.iter().enumerate() {
        let location = match (line, side) {
            // Deleted line: use ~N to indicate old line
            (Some(l), Some(LineSide::Old)) => format!("`{}:~{}`", file, l),
            // New/context line: use normal format
            (Some(l), _) => format!("`{}:{}`", file, l),
            // File comment
            (None, _) => format!("`{}`", file),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Comment, CommentType, FileStatus, LineSide};
    use std::path::PathBuf;

    fn create_test_session() -> ReviewSession {
        let mut session =
            ReviewSession::new(PathBuf::from("/tmp/test-repo"), "abc1234def".to_string());
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
        let markdown = generate_markdown(&session, &diff_source);

        // then
        assert!(markdown.contains("I reviewed your code and have the following comments"));
        assert!(markdown.contains("Comment types:"));
        assert!(markdown.contains("[SUGGESTION]"));
        assert!(markdown.contains("`src/main.rs`"));
        assert!(markdown.contains("Consider adding documentation"));
        assert!(markdown.contains("[ISSUE]"));
        assert!(markdown.contains("`src/main.rs:42`"));
        assert!(markdown.contains("Magic number"));
    }

    #[test]
    fn should_number_comments_sequentially() {
        // given
        let session = create_test_session();
        let diff_source = DiffSource::WorkingTree;

        // when
        let markdown = generate_markdown(&session, &diff_source);

        // then
        // Should have 2 numbered comments
        assert!(markdown.contains("1. **[SUGGESTION]**"));
        assert!(markdown.contains("2. **[ISSUE]**"));
    }

    #[test]
    fn should_fail_export_when_no_comments() {
        // given
        let session = ReviewSession::new(PathBuf::from("/tmp/test-repo"), "abc1234def".to_string());
        let diff_source = DiffSource::WorkingTree;

        // when
        let result = export_to_clipboard(&session, &diff_source);

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
        let markdown = generate_markdown(&session, &diff_source);

        // then
        assert!(markdown.contains("Reviewing commits: abc1234, def4567"));
    }

    #[test]
    fn should_include_single_commit_in_markdown() {
        // given
        let session = create_test_session();
        let diff_source = DiffSource::CommitRange(vec!["abc1234567890".to_string()]);

        // when
        let markdown = generate_markdown(&session, &diff_source);

        // then
        assert!(markdown.contains("Reviewing commit: abc1234"));
    }
}
