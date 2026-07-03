//! Non-interactive review session commands.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::cli::{LineSideArg, ReviewCommand};
use crate::config;
use crate::error::{Result, TuicrError};
use crate::model::comment::{self, CommentLifecycleState};
use crate::model::{
    Comment, CommentType, CommitReview, LineRange, LineSide, ReviewSession, SessionDiffSource,
};
use crate::review_store::{
    AddCommentRequest, CommentTarget, ReviewStore, SessionRef, SessionSummary,
};
use crate::slug::Slug;

pub fn run(command: ReviewCommand) -> Result<()> {
    let mut stdout = io::stdout();
    run_with_writer(command, &mut stdout)
}

fn run_with_writer(command: ReviewCommand, out: &mut impl Write) -> Result<()> {
    match command {
        ReviewCommand::List { repo, all } => list_sessions(&repo, all, out),
        ReviewCommand::Add {
            session,
            input,
            repo,
            comment_type,
            file,
            line,
            end_line,
            side,
            username,
            content,
        } => add_comment(
            &session,
            &repo,
            AddCommentOptions {
                input,
                comment_type,
                file,
                line,
                end_line,
                side,
                username,
                content,
            },
            out,
        ),
        ReviewCommand::Comments { session, repo } => show_comments(&session, &repo, out),
    }
}

fn list_sessions(repo: &Path, all: bool, out: &mut impl Write) -> Result<()> {
    let store = ReviewStore::new();
    let summaries = if all {
        store.list_all_sessions()?
    } else {
        store.list_sessions_for_repo(repo)?
    };
    let output: Vec<_> = summaries
        .into_iter()
        .map(SessionSummaryOutput::from)
        .collect();
    serde_json::to_writer_pretty(&mut *out, &output)?;
    writeln!(out)?;
    Ok(())
}

struct AddCommentOptions {
    input: Option<String>,
    comment_type: String,
    file: Option<PathBuf>,
    line: Option<u32>,
    end_line: Option<u32>,
    side: LineSideArg,
    username: Option<String>,
    content: Option<String>,
}

fn add_comment(
    session: &str,
    repo: &Path,
    options: AddCommentOptions,
    out: &mut impl Write,
) -> Result<()> {
    let store = ReviewStore::new();
    let session_ref = resolve_session_ref(&store, repo, session)?;
    let request_parts = build_add_request_parts(options)?;
    let target = request_parts.target;
    let comment_type = CommentType::from_id(&request_parts.comment_type);
    let author = resolve_cli_author(request_parts.username);
    let comment = store.add_comment(
        &session_ref,
        AddCommentRequest {
            target: target.clone(),
            content: request_parts.content,
            comment_type,
            author,
        },
    )?;
    let output = CommentOutput::from_target(&target, &comment);
    serde_json::to_writer_pretty(&mut *out, &output)?;
    writeln!(out)?;
    Ok(())
}

struct AddRequestParts {
    target: CommentTarget,
    comment_type: String,
    content: String,
    username: Option<String>,
}

/// Resolve the author for a CLI-authored comment.
///
/// Priority: explicit `--username` / JSON `username` ► config `username` ►
/// `Comment::DEFAULT_AUTHOR`. Trims whitespace so `--username " "` doesn't
/// produce an awkward all-whitespace badge.
fn resolve_cli_author(explicit: Option<String>) -> String {
    if let Some(name) = explicit.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        return name.to_string();
    }
    if let Ok(outcome) = config::load_config()
        && let Some(name) = outcome
            .config
            .as_ref()
            .and_then(|cfg| cfg.username.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty())
    {
        return name.to_string();
    }
    comment::DEFAULT_AUTHOR.to_string()
}

fn build_add_request_parts(options: AddCommentOptions) -> Result<AddRequestParts> {
    let mut comment_type = options.comment_type;
    let mut content = options.content;
    let mut file = options.file;
    let mut line = options.line;
    let mut end_line = options.end_line;
    let mut side = options.side;
    let mut username = options.username;
    let mut target = None;

    if let Some(input) = options.input {
        let payload = parse_add_payload(&read_json_input(&input)?)?;
        if let Some(payload_comment_type) = payload.comment_type {
            comment_type = payload_comment_type;
        }
        if payload.content.is_some() {
            content = payload.content;
        }
        if payload.username.is_some() {
            username = payload.username;
        }
        if let Some(payload_target) = payload.target {
            target = Some(payload_target.into_comment_target()?);
        } else {
            if let Some(payload_file) = payload.file {
                file = Some(payload_file);
            }
            if payload.line.is_some() || payload.start_line.is_some() {
                line = payload.line.or(payload.start_line);
            }
            if let Some(payload_end_line) = payload.end_line {
                end_line = Some(payload_end_line);
            }
            if let Some(payload_side) = payload.side {
                side = parse_line_side(&payload_side)?;
            }
        }
    }

    let content = content.ok_or_else(|| {
        TuicrError::InvalidInput(
            "comment text is required either as COMMENT or JSON field `content`".to_string(),
        )
    })?;
    let target = match target {
        Some(target) => target,
        None => build_comment_target(file, line, end_line, side)?,
    };

    Ok(AddRequestParts {
        target,
        comment_type,
        content,
        username,
    })
}

fn read_json_input(input: &str) -> Result<String> {
    if input == "-" {
        let mut contents = String::new();
        io::stdin().read_to_string(&mut contents)?;
        return Ok(contents);
    }
    if let Some(path) = input.strip_prefix('@') {
        return fs::read_to_string(path).map_err(TuicrError::Io);
    }
    Ok(input.to_string())
}

fn parse_add_payload(input: &str) -> Result<AddCommentPayload> {
    serde_json::from_str(input)
        .map_err(|err| TuicrError::InvalidInput(format!("invalid JSON review payload: {err}")))
}

#[derive(Debug, Deserialize)]
struct AddCommentPayload {
    #[serde(default, alias = "type")]
    comment_type: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    target: Option<JsonCommentTarget>,
    #[serde(default)]
    file: Option<PathBuf>,
    #[serde(default)]
    line: Option<u32>,
    #[serde(default)]
    start_line: Option<u32>,
    #[serde(default)]
    end_line: Option<u32>,
    #[serde(default)]
    side: Option<String>,
    #[serde(default, alias = "author")]
    username: Option<String>,
}

#[derive(Debug, Deserialize)]
struct JsonCommentTarget {
    #[serde(default, rename = "type", alias = "kind")]
    target_type: Option<String>,
    #[serde(default)]
    file: Option<PathBuf>,
    #[serde(default)]
    line: Option<u32>,
    #[serde(default)]
    start_line: Option<u32>,
    #[serde(default)]
    end_line: Option<u32>,
    #[serde(default)]
    side: Option<String>,
}

impl JsonCommentTarget {
    fn into_comment_target(self) -> Result<CommentTarget> {
        let side = match self.side {
            Some(side) => parse_line_side(&side)?,
            None => LineSideArg::New,
        };
        let inferred_type = if self.file.is_none() {
            "review"
        } else if self.line.is_some() || self.start_line.is_some() {
            if self.end_line.is_some() {
                "line_range"
            } else {
                "line"
            }
        } else {
            "file"
        };
        let target_type = self
            .target_type
            .unwrap_or_else(|| inferred_type.to_string())
            .replace('-', "_")
            .to_ascii_lowercase();

        match target_type.as_str() {
            "review" => Ok(CommentTarget::Review),
            "file" => Ok(CommentTarget::File {
                path: required_file(self.file, "target.file")?,
            }),
            "line" => Ok(CommentTarget::Line {
                path: required_file(self.file, "target.file")?,
                line: required_line(self.line.or(self.start_line), "target.line")?,
                side: line_side_arg_to_model(side),
            }),
            "line_range" | "range" => Ok(CommentTarget::LineRange {
                path: required_file(self.file, "target.file")?,
                range: LineRange::new(
                    required_line(self.line.or(self.start_line), "target.start_line")?,
                    required_line(self.end_line, "target.end_line")?,
                ),
                side: line_side_arg_to_model(side),
            }),
            other => Err(TuicrError::InvalidInput(format!(
                "unknown JSON target type '{other}'"
            ))),
        }
    }
}

fn required_file(path: Option<PathBuf>, name: &str) -> Result<PathBuf> {
    path.ok_or_else(|| TuicrError::InvalidInput(format!("{name} is required")))
}

fn required_line(line: Option<u32>, name: &str) -> Result<u32> {
    let line = line.ok_or_else(|| TuicrError::InvalidInput(format!("{name} is required")))?;
    validate_line(line, name)?;
    Ok(line)
}

fn parse_line_side(side: &str) -> Result<LineSideArg> {
    match side.to_ascii_lowercase().as_str() {
        "old" => Ok(LineSideArg::Old),
        "new" => Ok(LineSideArg::New),
        other => Err(TuicrError::InvalidInput(format!(
            "unknown side '{other}', expected 'old' or 'new'"
        ))),
    }
}

fn line_side_arg_to_model(side: LineSideArg) -> LineSide {
    match side {
        LineSideArg::Old => LineSide::Old,
        LineSideArg::New => LineSide::New,
    }
}

fn show_comments(session: &str, repo: &Path, out: &mut impl Write) -> Result<()> {
    let store = ReviewStore::new();
    let session_ref = resolve_session_ref(&store, repo, session)?;
    let session = store.get_review(&session_ref)?;
    let comments = collect_comments(&session);
    serde_json::to_writer_pretty(&mut *out, &comments)?;
    writeln!(out)?;
    Ok(())
}

fn resolve_session_ref(store: &ReviewStore, repo: &Path, session: &str) -> Result<SessionRef> {
    let direct_path = PathBuf::from(session);
    if direct_path.exists() || direct_path.is_absolute() || session.ends_with(".json") {
        return Ok(SessionRef::from_path(direct_path));
    }

    // PR sessions are keyed by forge coordinates, not a local checkout, so they
    // resolve from the manifest by slug rather than the per-repo listing.
    if matches!(session.parse::<Slug>(), Ok(Slug::Pr(_))) {
        return match store.resolve_pr_session(session)? {
            Some(session_ref) => Ok(session_ref),
            None => Err(TuicrError::InvalidInput(format!(
                "no PR session found for '{session}'. Run `tuicr review list --all` to see available sessions."
            ))),
        };
    }

    let matches: Vec<_> = store
        .list_sessions_for_repo(repo)?
        .into_iter()
        .filter(|summary| summary.slug == session)
        .collect();
    match matches.as_slice() {
        [summary] => Ok(summary.session_ref.clone()),
        [] => Err(TuicrError::InvalidInput(format!(
            "session '{session}' was not found for repo {}. Run `tuicr review list --repo {}` to see available sessions.",
            repo.display(),
            repo.display()
        ))),
        _ => Err(TuicrError::InvalidInput(format!(
            "session '{session}' is ambiguous for repo {}",
            repo.display()
        ))),
    }
}

fn build_comment_target(
    file: Option<PathBuf>,
    line: Option<u32>,
    end_line: Option<u32>,
    side: LineSideArg,
) -> Result<CommentTarget> {
    let side = match side {
        LineSideArg::Old => LineSide::Old,
        LineSideArg::New => LineSide::New,
    };

    match (file, line, end_line) {
        (None, None, None) => Ok(CommentTarget::Review),
        (Some(path), None, None) => Ok(CommentTarget::File { path }),
        (Some(path), Some(line), None) => {
            validate_line(line, "--line")?;
            Ok(CommentTarget::Line { path, line, side })
        }
        (Some(path), Some(start), Some(end)) => {
            validate_line(start, "--line")?;
            validate_line(end, "--end-line")?;
            Ok(CommentTarget::LineRange {
                path,
                range: LineRange::new(start, end),
                side,
            })
        }
        (None, Some(_), _) => Err(TuicrError::InvalidInput(
            "--line requires --target-file for review comments".to_string(),
        )),
        (None, None, Some(_)) => Err(TuicrError::InvalidInput(
            "--end-line requires --line and --target-file".to_string(),
        )),
        (Some(_), None, Some(_)) => Err(TuicrError::InvalidInput(
            "--end-line requires --line".to_string(),
        )),
    }
}

fn validate_line(line: u32, name: &str) -> Result<()> {
    if line == 0 {
        return Err(TuicrError::InvalidInput(format!(
            "{name} must be greater than zero"
        )));
    }
    Ok(())
}

fn collect_comments(session: &ReviewSession) -> Vec<CommentOutput> {
    if session.diff_source == SessionDiffSource::PerCommitRange {
        return collect_per_commit_comments(session);
    }

    let mut comments = Vec::new();
    for comment in &session.review_comments {
        comments.push(CommentOutput::from_parts(
            "review".to_string(),
            None,
            None,
            None,
            None,
            comment,
        ));
    }

    let mut files: Vec<_> = session.files.iter().collect();
    files.sort_by_key(|(path, _)| path.as_os_str().to_os_string());
    for (path, review) in files {
        let path_display = path.to_string_lossy().to_string();
        for comment in &review.file_comments {
            comments.push(CommentOutput::from_parts(
                path_display.clone(),
                Some(path_display.clone()),
                None,
                None,
                None,
                comment,
            ));
        }

        let mut line_comments: Vec<_> = review.line_comments.iter().collect();
        line_comments.sort_by_key(|(line, _)| *line);
        for (line, line_comments) in line_comments {
            for comment in line_comments {
                let (start_line, end_line) = comment
                    .line_range
                    .map(|range| (range.start, range.end))
                    .unwrap_or((*line, *line));
                let location = line_location(&path_display, start_line, end_line, comment.side);
                comments.push(CommentOutput::from_parts(
                    location,
                    Some(path_display.clone()),
                    Some(start_line),
                    Some(end_line),
                    comment.side,
                    comment,
                ));
            }
        }
    }

    comments
}

fn collect_per_commit_comments(session: &ReviewSession) -> Vec<CommentOutput> {
    let mut comments = Vec::new();
    let mut ordered_ids = session
        .commit_range
        .clone()
        .unwrap_or_else(|| session.per_commit_reviews.keys().cloned().collect());
    if session.commit_range.is_none() {
        ordered_ids.sort();
    }

    for commit_id in ordered_ids {
        let Some(commit_review) = session.per_commit_reviews.get(&commit_id) else {
            continue;
        };
        for comment in &commit_review.review_comments {
            comments.push(
                CommentOutput::from_parts("commit".to_string(), None, None, None, None, comment)
                    .with_commit(commit_review),
            );
        }

        let mut files: Vec<_> = commit_review.files.iter().collect();
        files.sort_by_key(|(path, _)| {
            if path.as_path() == Path::new("Commit Message") {
                String::new()
            } else {
                path.to_string_lossy().to_string()
            }
        });
        for (path, review) in files {
            let path_display = if path.as_path() == Path::new("Commit Message") {
                "commit message".to_string()
            } else {
                path.to_string_lossy().to_string()
            };
            for comment in &review.file_comments {
                comments.push(
                    CommentOutput::from_parts(
                        path_display.clone(),
                        Some(path_display.clone()),
                        None,
                        None,
                        None,
                        comment,
                    )
                    .with_commit(commit_review),
                );
            }

            let mut line_comments: Vec<_> = review.line_comments.iter().collect();
            line_comments.sort_by_key(|(line, _)| *line);
            for (line, line_comments) in line_comments {
                for comment in line_comments {
                    let (start_line, end_line) = comment
                        .line_range
                        .map(|range| (range.start, range.end))
                        .unwrap_or((*line, *line));
                    let location = line_location(&path_display, start_line, end_line, comment.side);
                    comments.push(
                        CommentOutput::from_parts(
                            location,
                            Some(path_display.clone()),
                            Some(start_line),
                            Some(end_line),
                            comment.side,
                            comment,
                        )
                        .with_commit(commit_review),
                    );
                }
            }
        }
    }

    comments
}

fn line_location(path: &str, start_line: u32, end_line: u32, side: Option<LineSide>) -> String {
    let line = if start_line == end_line {
        start_line.to_string()
    } else {
        format!("{start_line}-{end_line}")
    };
    match side {
        Some(LineSide::Old) => format!("{path}:{line} [old]"),
        _ => format!("{path}:{line}"),
    }
}

fn target_location(target: &CommentTarget) -> String {
    match target {
        CommentTarget::Review => "review".to_string(),
        CommentTarget::File { path } => path.display().to_string(),
        CommentTarget::Line { path, line, side } => {
            line_location(&path.to_string_lossy(), *line, *line, Some(*side))
        }
        CommentTarget::LineRange { path, range, side } => {
            line_location(&path.to_string_lossy(), range.start, range.end, Some(*side))
        }
    }
}

fn side_id(side: Option<LineSide>) -> Option<&'static str> {
    match side {
        Some(LineSide::Old) => Some("old"),
        Some(LineSide::New) => Some("new"),
        None => None,
    }
}

fn lifecycle_id(state: CommentLifecycleState) -> &'static str {
    match state {
        CommentLifecycleState::LocalDraft => "local_draft",
        CommentLifecycleState::PushedDraft => "pushed_draft",
        CommentLifecycleState::Submitted => "submitted",
    }
}

#[derive(Debug, Serialize)]
struct SessionSummaryOutput {
    slug: String,
    kind: &'static str,
    path: String,
    updated_at: String,
    comment_count: usize,
    reviewed_count: usize,
    file_count: usize,
    anchor: String,
    active: bool,
}

impl From<SessionSummary> for SessionSummaryOutput {
    fn from(summary: SessionSummary) -> Self {
        Self {
            slug: summary.slug,
            kind: summary.kind.id(),
            path: summary.session_ref.path().display().to_string(),
            updated_at: summary.updated_at.to_rfc3339(),
            comment_count: summary.comment_count,
            reviewed_count: summary.reviewed_count,
            file_count: summary.file_count,
            anchor: summary.anchor,
            active: summary.active,
        }
    }
}

#[derive(Debug, Serialize)]
struct CommentOutput {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    commit_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    commit_short_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    commit_summary: Option<String>,
    location: String,
    path: Option<String>,
    start_line: Option<u32>,
    end_line: Option<u32>,
    side: Option<&'static str>,
    comment_type: String,
    lifecycle_state: &'static str,
    created_at: String,
    content: String,
}

impl CommentOutput {
    fn from_target(target: &CommentTarget, comment: &Comment) -> Self {
        let (path, start_line, end_line, side) = match target {
            CommentTarget::Review => (None, None, None, None),
            CommentTarget::File { path } => (Some(path.display().to_string()), None, None, None),
            CommentTarget::Line { path, line, side } => (
                Some(path.display().to_string()),
                Some(*line),
                Some(*line),
                Some(*side),
            ),
            CommentTarget::LineRange { path, range, side } => (
                Some(path.display().to_string()),
                Some(range.start),
                Some(range.end),
                Some(*side),
            ),
        };
        Self::from_parts(
            target_location(target),
            path,
            start_line,
            end_line,
            side,
            comment,
        )
    }

    fn from_parts(
        location: String,
        path: Option<String>,
        start_line: Option<u32>,
        end_line: Option<u32>,
        side: Option<LineSide>,
        comment: &Comment,
    ) -> Self {
        Self {
            id: comment.id.clone(),
            commit_id: None,
            commit_short_id: None,
            commit_summary: None,
            location,
            path,
            start_line,
            end_line,
            side: side_id(side),
            comment_type: comment.comment_type.id().to_string(),
            lifecycle_state: lifecycle_id(comment.lifecycle_state),
            created_at: comment.created_at.to_rfc3339(),
            content: comment.content.clone(),
        }
    }

    fn with_commit(mut self, review: &CommitReview) -> Self {
        self.commit_id = Some(review.commit_id.clone());
        self.commit_short_id = Some(review.short_id.clone());
        self.commit_summary = Some(review.summary.clone());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use tempfile::tempdir;

    use crate::model::review::FileReview;
    use crate::model::{CommitReview, FileStatus, SessionDiffSource};

    fn test_session(repo_path: PathBuf) -> ReviewSession {
        let mut session = ReviewSession::new(
            repo_path,
            "abc1234".to_string(),
            Some("main".to_string()),
            SessionDiffSource::WorkingTree,
        );
        session.add_file(PathBuf::from("src/main.rs"), FileStatus::Modified, 0);
        session
    }

    #[test]
    fn should_build_review_comment_target_by_default() {
        let target = build_comment_target(None, None, None, LineSideArg::New).unwrap();
        assert!(matches!(target, CommentTarget::Review));
    }

    #[test]
    fn should_build_line_range_comment_target() {
        let target = build_comment_target(
            Some(PathBuf::from("src/main.rs")),
            Some(12),
            Some(10),
            LineSideArg::Old,
        )
        .unwrap();

        assert!(matches!(
            target,
            CommentTarget::LineRange {
                range: LineRange { start: 10, end: 12 },
                side: LineSide::Old,
                ..
            }
        ));
    }

    #[test]
    fn should_collect_per_commit_comments_with_commit_metadata() {
        let first_id = "aaa111122223333444455556666777788889999".to_string();
        let second_id = "bbb111122223333444455556666777788889999".to_string();
        let mut session = ReviewSession::new(
            PathBuf::from("/repo"),
            second_id.clone(),
            Some("main".to_string()),
            SessionDiffSource::PerCommitRange,
        );
        session.commit_range = Some(vec![first_id.clone(), second_id.clone()]);

        let mut first = CommitReview::new(
            first_id.clone(),
            "aaa1111".to_string(),
            "First commit".to_string(),
            None,
            "Alice".to_string(),
            Utc::now(),
        );
        first.review_comments.push(Comment::new(
            "Commit-level note".to_string(),
            CommentType::Note,
            None,
        ));
        session.per_commit_reviews.insert(first_id.clone(), first);

        let mut second = CommitReview::new(
            second_id.clone(),
            "bbb1111".to_string(),
            "Second commit".to_string(),
            None,
            "Bob".to_string(),
            Utc::now(),
        );
        let mut msg_review = FileReview::new(PathBuf::from("Commit Message"), FileStatus::Added, 0);
        msg_review.add_line_comment(
            1,
            Comment::new(
                "Commit message note".to_string(),
                CommentType::Suggestion,
                Some(LineSide::New),
            ),
        );
        second
            .files
            .insert(PathBuf::from("Commit Message"), msg_review);
        session.per_commit_reviews.insert(second_id.clone(), second);

        let comments = collect_comments(&session);

        assert_eq!(comments.len(), 2);
        assert_eq!(comments[0].commit_short_id.as_deref(), Some("aaa1111"));
        assert_eq!(comments[0].location, "commit");
        assert_eq!(comments[1].commit_short_id.as_deref(), Some("bbb1111"));
        assert_eq!(comments[1].commit_summary.as_deref(), Some("Second commit"));
        assert_eq!(comments[1].location, "commit message:1");
    }

    #[test]
    fn should_reject_zero_line() {
        let err = build_comment_target(
            Some(PathBuf::from("src/main.rs")),
            Some(0),
            None,
            LineSideArg::New,
        )
        .unwrap_err();
        assert!(matches!(err, TuicrError::InvalidInput(_)));
    }

    #[test]
    fn should_build_add_request_from_flat_json_payload() {
        let parts = build_add_request_parts(AddCommentOptions {
            input: Some(
                r#"{"file":"src/main.rs","line":42,"side":"old","type":"issue","content":"fix it"}"#
                    .to_string(),
            ),
            comment_type: "note".to_string(),
            file: None,
            line: None,
            end_line: None,
            side: LineSideArg::New,
            username: None,
            content: None,
        })
        .unwrap();

        assert_eq!(parts.comment_type, "issue");
        assert_eq!(parts.content, "fix it");
        assert!(matches!(
            parts.target,
            CommentTarget::Line {
                path,
                line: 42,
                side: LineSide::Old,
            } if path.as_path() == Path::new("src/main.rs")
        ));
    }

    #[test]
    fn should_build_add_request_from_nested_json_payload() {
        let parts = build_add_request_parts(AddCommentOptions {
            input: Some(
                r#"{"comment_type":"suggestion","content":"collapse this","target":{"type":"line_range","file":"src/main.rs","start_line":5,"end_line":7}}"#
                    .to_string(),
            ),
            comment_type: "note".to_string(),
            file: None,
            line: None,
            end_line: None,
            side: LineSideArg::New,
            username: None,
            content: None,
        })
        .unwrap();

        assert_eq!(parts.comment_type, "suggestion");
        assert!(matches!(
            parts.target,
            CommentTarget::LineRange {
                range: LineRange { start: 5, end: 7 },
                side: LineSide::New,
                ..
            }
        ));
    }

    fn save_pr_session(store: &ReviewStore) -> SessionRef {
        use crate::forge::traits::{ForgeRepository, PrSessionKey};

        let key = PrSessionKey::new(
            ForgeRepository::github("github.com", "slatedb", "slatedb"),
            1745,
            "43e3566924690c06a45b2177b4dd2df59a0f09c6".to_string(),
        );
        let mut session = ReviewSession::new(
            PathBuf::from("forge:github.com/slatedb/slatedb"),
            key.head_sha.clone(),
            Some("reviews".to_string()),
            SessionDiffSource::PullRequest,
        );
        session.pr_session_key = Some(key);
        store.save_review(&session).unwrap()
    }

    #[test]
    fn should_find_pr_session_by_repo_coordinate() {
        let temp = tempdir().unwrap();
        let store = ReviewStore::with_reviews_dir(temp.path().join("reviews"));
        let session_ref = save_pr_session(&store);

        // A bare repo coordinate surfaces the PR session and emits its slug.
        let listed = store
            .list_sessions_for_repo(Path::new("slatedb/slatedb"))
            .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].slug, "gh:slatedb/slatedb/pr/1745");
        assert_eq!(listed[0].kind, crate::review_store::SessionKind::Pr);

        // The emitted slug resolves the same way regardless of --repo.
        let resolved =
            resolve_session_ref(&store, Path::new("slatedb/slatedb"), &listed[0].slug).unwrap();
        assert_eq!(resolved, session_ref);
    }

    #[test]
    fn should_match_pr_session_via_forge_repo_path_coordinate() {
        let temp = tempdir().unwrap();
        let store = ReviewStore::with_reviews_dir(temp.path().join("reviews"));
        save_pr_session(&store);

        // The `forge:host/owner/repo` form (as stored on disk) also resolves.
        let listed = store
            .list_sessions_for_repo(Path::new("forge:github.com/slatedb/slatedb"))
            .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].slug, "gh:slatedb/slatedb/pr/1745");
    }

    #[test]
    fn should_not_match_pr_session_for_unrelated_repo() {
        let temp = tempdir().unwrap();
        let store = ReviewStore::with_reviews_dir(temp.path().join("reviews"));
        save_pr_session(&store);

        assert!(
            store
                .list_sessions_for_repo(Path::new("other/project"))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn should_list_pr_session_in_list_all() {
        let temp = tempdir().unwrap();
        let store = ReviewStore::with_reviews_dir(temp.path().join("reviews"));
        save_pr_session(&store);

        let all = store.list_all_sessions().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].slug, "gh:slatedb/slatedb/pr/1745");
    }

    #[test]
    fn should_resolve_pr_session_by_slug_without_repo() {
        let temp = tempdir().unwrap();
        let store = ReviewStore::with_reviews_dir(temp.path().join("reviews"));
        let session_ref = save_pr_session(&store);

        // PR slugs are self-contained: `--repo` is irrelevant.
        let resolved =
            resolve_session_ref(&store, Path::new("."), "gh:slatedb/slatedb/pr/1745").unwrap();
        assert_eq!(resolved, session_ref);
    }

    #[test]
    fn should_error_for_unknown_pr_slug() {
        let temp = tempdir().unwrap();
        let reviews = temp.path().join("reviews");
        let store = ReviewStore::with_reviews_dir(&reviews);
        let err = resolve_session_ref(&store, Path::new("."), "gh:nope/nope/pr/9999").unwrap_err();
        assert!(matches!(err, TuicrError::InvalidInput(_)));
    }

    #[test]
    fn should_list_add_and_show_comments() {
        let temp = tempdir().unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let reviews = temp.path().join("reviews");
        let store = ReviewStore::with_reviews_dir(&reviews);
        let session = test_session(repo.clone());
        let session_ref = store.save_review(&session).unwrap();

        let mut out = Vec::new();
        let sessions = store.list_sessions_for_repo(&repo).unwrap();
        assert_eq!(sessions.len(), 1);
        let slug = sessions[0].slug.clone();

        let resolved = resolve_session_ref(&store, &repo, &slug).unwrap();
        assert_eq!(resolved, session_ref);

        let comment = store
            .add_comment(
                &resolved,
                AddCommentRequest {
                    target: CommentTarget::Line {
                        path: PathBuf::from("src/main.rs"),
                        line: 42,
                        side: LineSide::New,
                    },
                    content: "check this".to_string(),
                    comment_type: CommentType::Issue,
                    author: crate::model::comment::DEFAULT_AUTHOR.to_string(),
                },
            )
            .unwrap();

        let loaded = store.get_review(&session_ref).unwrap();
        let comments = collect_comments(&loaded);
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].id, comment.id);
        assert_eq!(comments[0].location, "src/main.rs:42");
        assert_eq!(comments[0].comment_type, "issue");

        show_comments(&session_ref.path().display().to_string(), &repo, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value[0]["comment_type"], "issue");
        assert_eq!(value[0]["location"], "src/main.rs:42");
        assert_eq!(value[0]["content"], "check this");
    }
}
