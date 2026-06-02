use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

use crate::error::{Result, TuicrError};
use crate::model::{Comment, CommentType, LineRange, LineSide, ReviewSession};
use crate::persistence::manifest::{ManifestEntry, ManifestKind};
use crate::persistence::storage;

/// File-backed access to persisted tuicr review sessions.
#[derive(Debug, Clone, Default)]
pub struct ReviewStore {
    reviews_dir: Option<PathBuf>,
}

impl ReviewStore {
    /// Use tuicr's platform data directory.
    pub fn new() -> Self {
        Self::default()
    }

    /// Use an explicit reviews directory. This is primarily useful for
    /// wrappers, tests, and tools that want isolated session storage.
    pub fn with_reviews_dir(reviews_dir: impl Into<PathBuf>) -> Self {
        Self {
            reviews_dir: Some(reviews_dir.into()),
        }
    }

    /// List persisted sessions for a repo selector — a checkout path or a
    /// forge coordinate like `owner/repo`. A checkout path matches its own
    /// local sessions and, via its `origin` remote, any PR sessions for the
    /// same repo; a coordinate matches local and PR sessions by `owner/repo`.
    pub fn list_sessions_for_repo(
        &self,
        selector: impl AsRef<Path>,
    ) -> Result<Vec<SessionSummary>> {
        let reviews_dir = self.reviews_dir()?;
        let entries = storage::list_sessions_for_selector_in_dir(&reviews_dir, selector.as_ref())?;
        let active_paths = storage::active_session_paths_in_dir(&reviews_dir)?;
        Ok(entries
            .into_iter()
            .map(|(slug, entry)| summary_from_entry(&reviews_dir, &active_paths, slug, entry))
            .collect())
    }

    /// List every persisted session, local and PR, newest first. Backs
    /// `tuicr review list --all` for when the caller does not know the repo.
    pub fn list_all_sessions(&self) -> Result<Vec<SessionSummary>> {
        let reviews_dir = self.reviews_dir()?;
        let entries = storage::list_all_sessions_in_dir(&reviews_dir)?;
        let active_paths = storage::active_session_paths_in_dir(&reviews_dir)?;
        Ok(entries
            .into_iter()
            .map(|(slug, entry)| summary_from_entry(&reviews_dir, &active_paths, slug, entry))
            .collect())
    }

    /// Resolve a PR session to its [`SessionRef`] from a PR slug
    /// (`gh:owner/repo/pr/<n>`). Returns `None` when no PR session is
    /// persisted for that slug.
    pub fn resolve_pr_session(&self, slug: &str) -> Result<Option<SessionRef>> {
        let reviews_dir = self.reviews_dir()?;
        Ok(storage::pr_session_path_in_dir(&reviews_dir, slug)?.map(SessionRef::from_path))
    }

    /// Load a persisted review session.
    pub fn get_review(&self, session_ref: &SessionRef) -> Result<ReviewSession> {
        storage::load_session(session_ref.path())
    }

    /// Add a local draft comment to a persisted session and save it.
    pub fn add_comment(
        &self,
        session_ref: &SessionRef,
        request: AddCommentRequest,
    ) -> Result<Comment> {
        let reviews_dir = self.reviews_dir()?;
        let (_session, comment) =
            storage::update_session_in_dir(session_ref.path(), &reviews_dir, |session| {
                add_comment_to_session(session, request)
            })?;
        Ok(comment)
    }

    /// Save a session through this store's storage root.
    pub fn save_review(&self, session: &ReviewSession) -> Result<SessionRef> {
        let reviews_dir = self.reviews_dir()?;
        storage::save_session_in_dir(session, &reviews_dir).map(SessionRef::from_path)
    }

    fn reviews_dir(&self) -> Result<PathBuf> {
        match &self.reviews_dir {
            Some(path) => Ok(path.clone()),
            None => storage::get_reviews_dir(),
        }
    }
}

/// Build a [`SessionSummary`] from a manifest entry, resolving its absolute
/// path and active state. Shared by the per-repo and `--all` listings.
fn summary_from_entry(
    reviews_dir: &Path,
    active_paths: &std::collections::HashSet<PathBuf>,
    slug: String,
    entry: ManifestEntry,
) -> SessionSummary {
    let path = reviews_dir.join(entry.path);
    let active = active_paths.contains(&storage::normalize_path_for_comparison(&path));
    let kind = match entry.kind {
        ManifestKind::Local => SessionKind::Local,
        ManifestKind::Pr { .. } => SessionKind::Pr,
    };
    SessionSummary {
        session_ref: SessionRef::from_path(path),
        slug,
        kind,
        updated_at: entry.updated_at,
        comment_count: entry.display.comment_count,
        reviewed_count: entry.display.reviewed_count,
        file_count: entry.display.file_count,
        anchor: entry.display.anchor,
        active,
    }
}

/// Opaque reference to a persisted review session.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionRef {
    path: PathBuf,
}

impl SessionRef {
    pub fn from_path(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Whether a persisted session tracks a local checkout or a forge PR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionKind {
    Local,
    Pr,
}

impl SessionKind {
    pub fn id(self) -> &'static str {
        match self {
            SessionKind::Local => "local",
            SessionKind::Pr => "pr",
        }
    }
}

/// Lightweight metadata for a persisted session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSummary {
    pub session_ref: SessionRef,
    pub slug: String,
    pub kind: SessionKind,
    pub updated_at: DateTime<Utc>,
    pub comment_count: usize,
    pub reviewed_count: usize,
    pub file_count: usize,
    pub anchor: String,
    pub active: bool,
}

/// Request to add a local draft comment to a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddCommentRequest {
    pub target: CommentTarget,
    pub content: String,
    pub comment_type: CommentType,
    /// Author to stamp on the resulting comment. Caller is responsible for
    /// picking a sensible default (`Comment::DEFAULT_AUTHOR`) when none is
    /// supplied.
    pub author: String,
}

/// Where a new local draft comment should be attached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommentTarget {
    Review,
    File {
        path: PathBuf,
    },
    Line {
        path: PathBuf,
        line: u32,
        side: LineSide,
    },
    LineRange {
        path: PathBuf,
        range: LineRange,
        side: LineSide,
    },
}

/// Add a local draft comment to an in-memory session.
///
/// This is the shared primitive used by the TUI and by [`ReviewStore`].
pub fn add_comment_to_session(
    session: &mut ReviewSession,
    request: AddCommentRequest,
) -> Result<Comment> {
    let content = request.content.trim().to_string();
    if content.is_empty() {
        return Err(TuicrError::InvalidInput(
            "comment cannot be empty".to_string(),
        ));
    }

    let author = request.author;
    let comment = match request.target {
        CommentTarget::Review => {
            let comment = Comment::new(content, request.comment_type, None).with_author(author);
            session.review_comments.push(comment.clone());
            comment
        }
        CommentTarget::File { path } => {
            let review = file_review_mut(session, &path)?;
            let comment = Comment::new(content, request.comment_type, None).with_author(author);
            review.add_file_comment(comment.clone());
            comment
        }
        CommentTarget::Line { path, line, side } => {
            let review = file_review_mut(session, &path)?;
            let comment =
                Comment::new(content, request.comment_type, Some(side)).with_author(author);
            review.add_line_comment(line, comment.clone());
            comment
        }
        CommentTarget::LineRange { path, range, side } => {
            let review = file_review_mut(session, &path)?;
            let comment = Comment::new_with_range(content, request.comment_type, Some(side), range)
                .with_author(author);
            review.add_line_comment(range.end, comment.clone());
            comment
        }
    };

    session.updated_at = Utc::now();
    Ok(comment)
}

fn file_review_mut<'a>(
    session: &'a mut ReviewSession,
    path: &Path,
) -> Result<&'a mut crate::model::review::FileReview> {
    session.get_file_mut(&path.to_path_buf()).ok_or_else(|| {
        TuicrError::InvalidInput(format!("session does not contain file {}", path.display()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{FileStatus, SessionDiffSource};

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
    fn should_add_review_level_comment_to_session() {
        let mut session = test_session(PathBuf::from("/repo"));

        let comment = add_comment_to_session(
            &mut session,
            AddCommentRequest {
                target: CommentTarget::Review,
                content: "looks good".to_string(),
                comment_type: CommentType::Praise,
                author: crate::model::comment::DEFAULT_AUTHOR.to_string(),
            },
        )
        .unwrap();

        assert_eq!(session.review_comments, vec![comment]);
    }

    #[test]
    fn should_add_file_comment_to_session() {
        let mut session = test_session(PathBuf::from("/repo"));

        let comment = add_comment_to_session(
            &mut session,
            AddCommentRequest {
                target: CommentTarget::File {
                    path: PathBuf::from("src/main.rs"),
                },
                content: "file note".to_string(),
                comment_type: CommentType::Note,
                author: crate::model::comment::DEFAULT_AUTHOR.to_string(),
            },
        )
        .unwrap();

        let review = session.files.get(&PathBuf::from("src/main.rs")).unwrap();
        assert_eq!(review.file_comments, vec![comment]);
    }

    #[test]
    fn should_add_line_range_comment_by_range_end() {
        let mut session = test_session(PathBuf::from("/repo"));
        let range = LineRange::new(10, 12);

        let comment = add_comment_to_session(
            &mut session,
            AddCommentRequest {
                target: CommentTarget::LineRange {
                    path: PathBuf::from("src/main.rs"),
                    range,
                    side: LineSide::New,
                },
                content: "range note".to_string(),
                comment_type: CommentType::Suggestion,
                author: crate::model::comment::DEFAULT_AUTHOR.to_string(),
            },
        )
        .unwrap();

        let review = session.files.get(&PathBuf::from("src/main.rs")).unwrap();
        assert_eq!(review.line_comments.get(&12), Some(&vec![comment]));
    }

    #[test]
    fn should_reject_unknown_file() {
        let mut session = test_session(PathBuf::from("/repo"));

        let err = add_comment_to_session(
            &mut session,
            AddCommentRequest {
                target: CommentTarget::File {
                    path: PathBuf::from("missing.rs"),
                },
                content: "note".to_string(),
                comment_type: CommentType::Note,
                author: crate::model::comment::DEFAULT_AUTHOR.to_string(),
            },
        )
        .unwrap_err();

        assert!(matches!(err, TuicrError::InvalidInput(_)));
    }

    #[test]
    fn should_list_and_update_sessions_through_store() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let reviews_dir = temp.path().join("reviews");
        let store = ReviewStore::with_reviews_dir(reviews_dir.clone());
        let session = test_session(repo.clone());
        let session_ref = store.save_review(&session).unwrap();

        let listed = store.list_sessions_for_repo(&repo).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].session_ref, session_ref);
        assert_eq!(listed[0].file_count, 1);
        assert_eq!(listed[0].comment_count, 0);
        assert!(!listed[0].active);

        crate::persistence::storage::mark_session_active_in_dir(
            &session,
            session_ref.path(),
            &reviews_dir,
        )
        .unwrap();
        let listed = store.list_sessions_for_repo(&repo).unwrap();
        assert!(listed[0].active);

        store
            .add_comment(
                &session_ref,
                AddCommentRequest {
                    target: CommentTarget::Line {
                        path: PathBuf::from("src/main.rs"),
                        line: 7,
                        side: LineSide::New,
                    },
                    content: "line note".to_string(),
                    comment_type: CommentType::Note,
                    author: crate::model::comment::DEFAULT_AUTHOR.to_string(),
                },
            )
            .unwrap();

        let loaded = store.get_review(&session_ref).unwrap();
        let review = loaded.files.get(&PathBuf::from("src/main.rs")).unwrap();
        assert_eq!(review.line_comments.get(&7).unwrap().len(), 1);

        let listed = store.list_sessions_for_repo(&repo).unwrap();
        assert_eq!(listed[0].comment_count, 1);
    }
}
