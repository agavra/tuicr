use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

use crate::error::{Result, TuicrError};
use crate::model::{Comment, CommentType, LineRange, LineSide, ReviewSession};
use crate::persistence::manifest::{ManifestEntry, ManifestKind};
use crate::persistence::storage;

pub(crate) use crate::persistence::storage::{
    PruneCriterion as ReviewPruneCriterion, RemoveSessionOutcome as RemoveReviewOutcome,
    SessionCleanupEntry as SessionCleanup,
};

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

    /// Remove one persisted session, optionally protecting non-empty or active state.
    pub(crate) fn remove_review(
        &self,
        session_ref: &SessionRef,
        if_empty: bool,
        force: bool,
    ) -> Result<RemoveReviewOutcome> {
        let reviews_dir = self.reviews_dir()?;
        storage::remove_review_session_in_dir(session_ref.path(), &reviews_dir, if_empty, force)
    }

    /// Remove every persisted session in scope that matches `criterion`.
    pub(crate) fn prune_reviews(
        &self,
        selector: Option<&Path>,
        criterion: ReviewPruneCriterion,
        dry_run: bool,
        force: bool,
    ) -> Result<Vec<SessionCleanup>> {
        let reviews_dir = self.reviews_dir()?;
        storage::prune_review_sessions_in_dir(&reviews_dir, selector, criterion, dry_run, force)
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
    /// Commit SHA to stamp on the comment when it was created while the
    /// inline commit selector showed exactly one commit. `None` for
    /// review-level comments and full-range selections. Library callers
    /// (the `review add` CLI) leave this `None`.
    pub commit_id: Option<String>,
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
    let commit_id = request.commit_id;
    let comment = match request.target {
        CommentTarget::Review => {
            let comment = Comment::new(content, request.comment_type, None).with_author(author);
            session.review_comments.push(comment.clone());
            comment
        }
        CommentTarget::File { path } => {
            let review = file_review_mut(session, &path)?;
            let mut comment = Comment::new(content, request.comment_type, None).with_author(author);
            if let Some(sha) = &commit_id {
                comment = comment.with_commit_id(sha.clone());
            }
            review.add_file_comment(comment.clone());
            comment
        }
        CommentTarget::Line { path, line, side } => {
            let review = file_review_mut(session, &path)?;
            let mut comment =
                Comment::new(content, request.comment_type, Some(side)).with_author(author);
            if let Some(sha) = &commit_id {
                comment = comment.with_commit_id(sha.clone());
            }
            review.add_line_comment(line, comment.clone());
            comment
        }
        CommentTarget::LineRange { path, range, side } => {
            let review = file_review_mut(session, &path)?;
            let mut comment =
                Comment::new_with_range(content, request.comment_type, Some(side), range)
                    .with_author(author);
            if let Some(sha) = &commit_id {
                comment = comment.with_commit_id(sha.clone());
            }
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
                comment_type: CommentType::from_id("praise"),
                author: crate::model::comment::DEFAULT_AUTHOR.to_string(),
                commit_id: None,
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
                comment_type: CommentType::from_id("note"),
                author: crate::model::comment::DEFAULT_AUTHOR.to_string(),
                commit_id: None,
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
                comment_type: CommentType::from_id("suggestion"),
                author: crate::model::comment::DEFAULT_AUTHOR.to_string(),
                commit_id: None,
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
                comment_type: CommentType::from_id("note"),
                author: crate::model::comment::DEFAULT_AUTHOR.to_string(),
                commit_id: None,
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
                    comment_type: CommentType::from_id("note"),
                    author: crate::model::comment::DEFAULT_AUTHOR.to_string(),
                    commit_id: None,
                },
            )
            .unwrap();

        let loaded = store.get_review(&session_ref).unwrap();
        let review = loaded.files.get(&PathBuf::from("src/main.rs")).unwrap();
        assert_eq!(review.line_comments.get(&7).unwrap().len(), 1);

        let listed = store.list_sessions_for_repo(&repo).unwrap();
        assert_eq!(listed[0].comment_count, 1);
    }

    #[test]
    fn should_require_force_to_remove_active_session() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let reviews_dir = temp.path().join("reviews");
        let store = ReviewStore::with_reviews_dir(reviews_dir.clone());
        let session = test_session(repo);
        let session_ref = store.save_review(&session).unwrap();
        crate::persistence::storage::mark_session_active_in_dir(
            &session,
            session_ref.path(),
            &reviews_dir,
        )
        .unwrap();

        let blocked = store.remove_review(&session_ref, false, false).unwrap();
        assert!(matches!(blocked, RemoveReviewOutcome::Active(_)));
        assert!(session_ref.path().exists());

        let removed = store.remove_review(&session_ref, false, true).unwrap();
        assert!(matches!(
            removed,
            RemoveReviewOutcome::Removed(SessionCleanup { active: true, .. })
        ));
        assert!(!session_ref.path().exists());
        assert!(
            crate::persistence::storage::active_session_paths_in_dir(&reviews_dir)
                .unwrap()
                .is_empty()
        );
        let active: serde_json::Value = serde_json::from_slice(
            &std::fs::read(reviews_dir.join("active_sessions.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(active["sessions"], serde_json::json!([]));
    }

    #[test]
    fn should_remove_manifestless_session_when_slug_cannot_be_derived() {
        let temp = tempfile::tempdir().unwrap();
        let reviews_dir = temp.path().join("reviews");
        let store = ReviewStore::with_reviews_dir(&reviews_dir);
        let session = test_session(PathBuf::from(std::path::MAIN_SEPARATOR.to_string()));
        let path = temp.path().join("external-session.json");
        std::fs::write(&path, serde_json::to_vec_pretty(&session).unwrap()).unwrap();

        let removed = store
            .remove_review(&SessionRef::from_path(&path), false, false)
            .unwrap();

        assert!(matches!(
            removed,
            RemoveReviewOutcome::Removed(SessionCleanup { slug, .. }) if slug == session.id
        ));
        assert!(!path.exists());
    }

    #[test]
    fn should_remove_only_the_selected_path_when_local_slugs_collide() {
        let temp = tempfile::tempdir().unwrap();
        let repo_a = temp.path().join("a").join("repo");
        let repo_b = temp.path().join("b").join("repo");
        std::fs::create_dir_all(&repo_a).unwrap();
        std::fs::create_dir_all(&repo_b).unwrap();
        let store = ReviewStore::with_reviews_dir(temp.path().join("reviews"));
        let ref_a = store.save_review(&test_session(repo_a.clone())).unwrap();
        let ref_b = store.save_review(&test_session(repo_b.clone())).unwrap();
        let slug_a = store.list_sessions_for_repo(&repo_a).unwrap()[0]
            .slug
            .clone();
        let slug_b = store.list_sessions_for_repo(&repo_b).unwrap()[0]
            .slug
            .clone();
        assert_eq!(slug_a, slug_b, "precondition: the local slugs collide");

        let removed = store.remove_review(&ref_a, false, false).unwrap();

        assert!(matches!(removed, RemoveReviewOutcome::Removed(_)));
        assert!(!ref_a.path().exists());
        assert!(ref_b.path().exists());
        assert!(store.list_sessions_for_repo(&repo_a).unwrap().is_empty());
        assert_eq!(store.list_sessions_for_repo(&repo_b).unwrap().len(), 1);
    }

    #[test]
    fn should_remove_manifest_entry_for_non_normalized_direct_path() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let store = ReviewStore::with_reviews_dir(temp.path().join("reviews"));
        let session_ref = store.save_review(&test_session(repo.clone())).unwrap();
        let sessions_dir = session_ref.path().parent().unwrap();
        let aliased_path = sessions_dir
            .join("..")
            .join("sessions")
            .join(session_ref.path().file_name().unwrap());
        assert!(
            aliased_path.exists(),
            "precondition: the aliased path resolves before deletion"
        );

        let removed = store
            .remove_review(&SessionRef::from_path(aliased_path), false, false)
            .unwrap();

        assert!(matches!(removed, RemoveReviewOutcome::Removed(_)));
        assert!(store.list_sessions_for_repo(&repo).unwrap().is_empty());
    }

    #[test]
    fn should_remove_registered_session_when_file_is_already_missing() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let store = ReviewStore::with_reviews_dir(temp.path().join("reviews"));
        let session_ref = store.save_review(&test_session(repo.clone())).unwrap();
        std::fs::remove_file(session_ref.path()).unwrap();

        let removed = store.remove_review(&session_ref, false, false).unwrap();

        assert!(matches!(removed, RemoveReviewOutcome::Removed(_)));
        assert!(store.list_sessions_for_repo(&repo).unwrap().is_empty());
    }

    #[test]
    fn should_prune_only_sessions_without_comments_or_reviewed_state() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let reviews_dir = temp.path().join("reviews");
        let store = ReviewStore::with_reviews_dir(&reviews_dir);

        let empty = test_session(repo.clone());
        let empty_ref = store.save_review(&empty).unwrap();

        let mut commented = test_session(repo.clone());
        commented.base_commit = "commented".to_string();
        commented.review_comments.push(Comment::new(
            "keep me".to_string(),
            CommentType::from_id("note"),
            None,
        ));
        let commented_ref = store.save_review(&commented).unwrap();

        let mut reviewed_hunk = test_session(repo.clone());
        reviewed_hunk.base_commit = "reviewed".to_string();
        reviewed_hunk
            .get_file_mut(&PathBuf::from("src/main.rs"))
            .unwrap()
            .toggle_hunk_reviewed("hunk".to_string());
        let reviewed_ref = store.save_review(&reviewed_hunk).unwrap();

        let mut missing = test_session(repo.clone());
        missing.base_commit = "missing".to_string();
        let missing_ref = store.save_review(&missing).unwrap();
        std::fs::remove_file(missing_ref.path()).unwrap();

        let preview = store
            .prune_reviews(Some(&repo), ReviewPruneCriterion::Empty, true, false)
            .unwrap();
        assert_eq!(preview.len(), 2);
        assert!(empty_ref.path().exists(), "dry-run must not delete files");

        let removed = store
            .prune_reviews(Some(&repo), ReviewPruneCriterion::Empty, false, false)
            .unwrap();
        assert_eq!(
            removed
                .iter()
                .map(|entry| entry.path.as_path())
                .collect::<std::collections::HashSet<_>>(),
            std::collections::HashSet::from([empty_ref.path(), missing_ref.path()])
        );
        assert!(!empty_ref.path().exists());
        assert!(commented_ref.path().exists());
        assert!(reviewed_ref.path().exists());

        let listed = store.list_sessions_for_repo(&repo).unwrap();
        assert_eq!(listed.len(), 2);
    }

    #[test]
    fn should_skip_active_sessions_during_prune_unless_forced() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let reviews_dir = temp.path().join("reviews");
        let store = ReviewStore::with_reviews_dir(&reviews_dir);

        let active_session = test_session(repo.clone());
        let active_ref = store.save_review(&active_session).unwrap();
        crate::persistence::storage::mark_session_active_in_dir(
            &active_session,
            active_ref.path(),
            &reviews_dir,
        )
        .unwrap();

        let mut inactive_session = test_session(repo.clone());
        inactive_session.base_commit = "inactive".to_string();
        let inactive_ref = store.save_review(&inactive_session).unwrap();

        let removed = store
            .prune_reviews(Some(&repo), ReviewPruneCriterion::Empty, false, false)
            .unwrap();
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].path, inactive_ref.path());
        assert!(active_ref.path().exists());

        let forced = store
            .prune_reviews(Some(&repo), ReviewPruneCriterion::Empty, false, true)
            .unwrap();
        assert_eq!(forced.len(), 1);
        assert!(forced[0].active);
        assert!(!active_ref.path().exists());
        assert!(
            crate::persistence::storage::active_session_paths_in_dir(&reviews_dir)
                .unwrap()
                .is_empty()
        );
        let active: serde_json::Value = serde_json::from_slice(
            &std::fs::read(reviews_dir.join("active_sessions.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(active["sessions"], serde_json::json!([]));
    }

    #[test]
    fn should_require_readable_session_to_prune_as_empty() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let store = ReviewStore::with_reviews_dir(temp.path().join("reviews"));
        let cutoff = Utc::now() - chrono::Duration::days(30);
        let mut session = test_session(repo.clone());
        session.updated_at = cutoff - chrono::Duration::days(1);
        let session_ref = store.save_review(&session).unwrap();
        std::fs::write(session_ref.path(), "{ invalid json").unwrap();

        let err = store
            .prune_reviews(Some(&repo), ReviewPruneCriterion::Empty, false, false)
            .unwrap_err();
        assert!(matches!(err, TuicrError::CorruptedSession(_)));
        assert!(session_ref.path().exists());
        assert_eq!(store.list_sessions_for_repo(&repo).unwrap().len(), 1);

        let removed = store
            .prune_reviews(
                Some(&repo),
                ReviewPruneCriterion::UpdatedBefore(cutoff),
                false,
                false,
            )
            .unwrap();
        assert_eq!(removed.len(), 1);
        assert!(!session_ref.path().exists());
    }

    #[test]
    fn should_not_prune_any_session_when_empty_preflight_fails() {
        let temp = tempfile::tempdir().unwrap();
        let repo_a = temp.path().join("a").join("repo");
        let repo_b = temp.path().join("b").join("repo");
        std::fs::create_dir_all(&repo_a).unwrap();
        std::fs::create_dir_all(&repo_b).unwrap();
        let store = ReviewStore::with_reviews_dir(temp.path().join("reviews"));
        let healthy_ref = store.save_review(&test_session(repo_a.clone())).unwrap();
        let corrupt_ref = store.save_review(&test_session(repo_b.clone())).unwrap();
        std::fs::write(corrupt_ref.path(), "{ invalid json").unwrap();

        let err = store
            .prune_reviews(None, ReviewPruneCriterion::Empty, false, false)
            .unwrap_err();

        assert!(matches!(err, TuicrError::CorruptedSession(_)));
        assert!(healthy_ref.path().exists());
        assert!(corrupt_ref.path().exists());
        assert_eq!(store.list_all_sessions().unwrap().len(), 2);
    }

    #[test]
    fn should_prune_sessions_older_than_cutoff_with_repo_scope() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let other_repo = temp.path().join("other");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::create_dir_all(&other_repo).unwrap();
        let store = ReviewStore::with_reviews_dir(temp.path().join("reviews"));
        let cutoff = Utc::now() - chrono::Duration::days(30);

        let mut old = test_session(repo.clone());
        old.updated_at = cutoff - chrono::Duration::seconds(1);
        let old_ref = store.save_review(&old).unwrap();

        let mut boundary = test_session(repo.clone());
        boundary.base_commit = "boundary".to_string();
        boundary.updated_at = cutoff;
        let boundary_ref = store.save_review(&boundary).unwrap();

        let mut other = test_session(other_repo.clone());
        other.updated_at = cutoff - chrono::Duration::days(1);
        let other_ref = store.save_review(&other).unwrap();

        let removed = store
            .prune_reviews(
                Some(&repo),
                ReviewPruneCriterion::UpdatedBefore(cutoff),
                false,
                false,
            )
            .unwrap();

        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].path, old_ref.path());
        assert!(!old_ref.path().exists());
        assert!(boundary_ref.path().exists());
        assert!(other_ref.path().exists());

        let removed = store
            .prune_reviews(
                None,
                ReviewPruneCriterion::UpdatedBefore(cutoff),
                false,
                false,
            )
            .unwrap();
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].path, other_ref.path());
        assert!(boundary_ref.path().exists());
        assert!(!other_ref.path().exists());
    }
}
