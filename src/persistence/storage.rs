//! Slug-addressed storage layer for review sessions.
//!
//! Layout under `~/.local/share/tuicr/reviews/`:
//!
//! ```text
//! reviews/
//!   index.json                          # manifest, source of truth for lookups
//!   local/<fp>/<owner>/<repo>/<ref>/<source>.json
//!   local/<fp>/<repo>/<ref>/<source>.json        # no-owner fallback
//!   gh/<owner>/<repo>/pr/<number>/<head_sha>.json
//! ```
//!
//! Every save writes a session JSON file and updates the manifest. Every load
//! is a manifest lookup keyed by the slug derived from the caller's context.
//! Per-session JSON files are self-describing so the manifest can be rebuilt
//! by walking the tree if it gets lost or corrupted.

use std::fs;
use std::path::{Path, PathBuf};

#[cfg(not(test))]
use directories::ProjectDirs;

use crate::error::{Result, TuicrError};
use crate::forge::traits::PrSessionKey;
use crate::hash::fnv1a_64;
use crate::model::ReviewSession;
use crate::model::review::SessionDiffSource;
use crate::persistence::manifest::{self, MANIFEST_FILENAME, ManifestKind};
use crate::slug::{self, LocalSlug, Slug, SlugAnchor, SlugSource};

const FINGERPRINT_HEX_LEN: usize = 8;

// ---------- Public API ----------

/// Save a session to disk and update the manifest. The on-disk path is
/// derived from the session's slug; the slug is computed from the session's
/// fields at save time.
pub fn save_session(session: &ReviewSession) -> Result<PathBuf> {
    let reviews_dir = get_reviews_dir()?;
    maybe_migrate(&reviews_dir)?;

    let slug = slug_for_session(session)?;
    let relative = relative_path_for_slug(&slug, session)?;
    let full_path = reviews_dir.join(&relative);

    if let Some(parent) = full_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(session)?;
    fs::write(&full_path, json)?;

    let mut manifest = manifest::load_manifest(&reviews_dir).unwrap_or_default();
    let anchor = manifest_anchor_for(&slug);
    let entry = manifest::entry_from_session(session, relative, anchor);
    manifest.upsert(slug.to_string(), entry);
    manifest::save_manifest(&reviews_dir, &manifest)?;

    Ok(full_path)
}

/// Load a session JSON file from an absolute path.
pub fn load_session(path: &Path) -> Result<ReviewSession> {
    let contents = fs::read_to_string(path)?;
    serde_json::from_str(&contents).map_err(|e| TuicrError::CorruptedSession(e.to_string()))
}

/// Look up the persisted local session that matches the requested context.
/// Returns `None` if no matching slug is in the manifest, or if a manifest
/// entry exists but belongs to a different canonical checkout (same slug,
/// different path on disk).
pub fn load_latest_session_for_context(
    repo_path: &Path,
    branch_name: Option<&str>,
    head_commit: &str,
    diff_source: SessionDiffSource,
    commit_range: Option<&[String]>,
) -> Result<Option<(PathBuf, ReviewSession)>> {
    // PR sessions are resolved via `load_pr_session`. Mirror the old behavior
    // so callers that pass `PullRequest` here get `None` rather than an error.
    if matches!(diff_source, SessionDiffSource::PullRequest) {
        return Ok(None);
    }

    let reviews_dir = get_reviews_dir()?;
    maybe_migrate(&reviews_dir)?;

    let owner_repo = slug::resolve_owner_repo(repo_path)
        .map_err(|e| TuicrError::CorruptedSession(format!("slug derive: {e}")))?;
    let local = slug::build_local_slug(
        owner_repo,
        branch_name,
        head_commit,
        diff_source,
        commit_range,
    )
    .map_err(|e| TuicrError::CorruptedSession(format!("slug build: {e}")))?;
    let slug = Slug::Local(local);

    let manifest = manifest::load_manifest(&reviews_dir).unwrap_or_default();
    let canonical = fs::canonicalize(repo_path).unwrap_or_else(|_| repo_path.to_path_buf());
    let Some(entry) = manifest.get_local(&slug.to_string(), &canonical) else {
        return Ok(None);
    };

    let full_path = reviews_dir.join(&entry.path);
    let session = load_session(&full_path)?;
    Ok(Some((full_path, session)))
}

/// Look up the persisted PR session for a key. Returns `None` if no entry
/// exists for the slug, or if the manifest's current head differs from the
/// requested head (the old head's file may still be on disk but is not
/// surfaced).
pub fn load_pr_session(key: &PrSessionKey) -> Result<Option<(PathBuf, ReviewSession)>> {
    let reviews_dir = get_reviews_dir()?;
    maybe_migrate(&reviews_dir)?;

    let slug: Slug = key.into();
    let manifest = manifest::load_manifest(&reviews_dir).unwrap_or_default();
    let Some(entry) = manifest.get_pr(&slug.to_string()) else {
        return Ok(None);
    };

    match &entry.kind {
        ManifestKind::Pr { head_sha, .. } if head_sha == &key.head_sha => {
            let full_path = reviews_dir.join(&entry.path);
            let session = load_session(&full_path)?;
            Ok(Some((full_path, session)))
        }
        _ => Ok(None),
    }
}

/// Derive the slug for a session from its embedded fields. Local sessions
/// require resolving the repo's `origin` remote (I/O); PR sessions are
/// derived purely from the embedded `pr_session_key`.
pub fn slug_for_session(session: &ReviewSession) -> Result<Slug> {
    if let Some(key) = session.pr_session_key.as_ref() {
        return Ok(key.into());
    }
    let owner_repo = slug::resolve_owner_repo(&session.repo_path)
        .map_err(|e| TuicrError::CorruptedSession(format!("slug derive: {e}")))?;
    let local = slug::build_local_slug(
        owner_repo,
        session.branch_name.as_deref(),
        &session.base_commit,
        session.diff_source,
        session.commit_range.as_deref(),
    )
    .map_err(|e| TuicrError::CorruptedSession(format!("slug build: {e}")))?;
    Ok(Slug::Local(local))
}

// ---------- Internals ----------

#[cfg(test)]
thread_local! {
    static TEST_REVIEWS_DIR: std::cell::RefCell<Option<PathBuf>> = const {
        std::cell::RefCell::new(None)
    };
}

#[cfg(test)]
pub(crate) fn set_test_reviews_dir(path: Option<PathBuf>) {
    TEST_REVIEWS_DIR.with(|cell| *cell.borrow_mut() = path);
}

fn get_reviews_dir() -> Result<PathBuf> {
    #[cfg(test)]
    {
        // In tests, the reviews dir is a thread-local so that two parallel
        // tests never share state through it. Tests that touch storage and
        // care about isolation set the thread-local via
        // `set_test_reviews_dir`; tests that hit storage incidentally (e.g.,
        // App tests that toggle save markers) fall back to a per-thread
        // temp dir. The real `~/.local/share/tuicr/reviews` is never used in
        // test mode.
        let configured = TEST_REVIEWS_DIR.with(|cell| cell.borrow().clone());
        if let Some(path) = configured {
            fs::create_dir_all(&path)?;
            return Ok(path);
        }
        let thread_id = std::thread::current().id();
        let path = std::env::temp_dir().join(format!(
            "tuicr-test-thread-{:?}-{}",
            thread_id,
            std::process::id()
        ));
        fs::create_dir_all(&path)?;
        Ok(path)
    }

    #[cfg(not(test))]
    {
        let proj_dirs = ProjectDirs::from("", "", "tuicr").ok_or_else(|| {
            TuicrError::Io(std::io::Error::other("Could not determine data directory"))
        })?;

        let data_dir = proj_dirs.data_dir().join("reviews");
        fs::create_dir_all(&data_dir)?;
        Ok(data_dir)
    }
}

/// On first run under the new layout, move any old flat-layout sessions out
/// of the reviews dir. The new layout is identified by the presence of the
/// manifest file. If the reviews dir contains top-level `.json` session files
/// (old layout), rename it to `<reviews>.v1` and start fresh.
fn maybe_migrate(reviews_dir: &Path) -> Result<()> {
    if !reviews_dir.exists() {
        return Ok(());
    }
    if reviews_dir.join(MANIFEST_FILENAME).exists() {
        return Ok(());
    }

    let mut has_top_level_json = false;
    for entry in fs::read_dir(reviews_dir)? {
        let entry = entry?;
        let ft = entry.file_type()?;
        if !ft.is_file() {
            continue;
        }
        if entry
            .file_name()
            .to_str()
            .is_some_and(|n| n.ends_with(".json"))
        {
            has_top_level_json = true;
            break;
        }
    }

    if !has_top_level_json {
        return Ok(());
    }

    let parent = reviews_dir
        .parent()
        .ok_or_else(|| TuicrError::Io(std::io::Error::other("reviews dir has no parent")))?;
    let stem = reviews_dir
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| TuicrError::Io(std::io::Error::other("reviews dir has no name")))?;
    let mut backup = parent.join(format!("{stem}.v1"));
    let mut suffix = 1u32;
    while backup.exists() {
        suffix += 1;
        backup = parent.join(format!("{stem}.v{suffix}"));
    }

    fs::rename(reviews_dir, &backup)?;
    fs::create_dir_all(reviews_dir)?;
    eprintln!(
        "[tuicr] migrating reviews to new layout; previous reviews moved to {}",
        backup.display()
    );
    Ok(())
}

fn relative_path_for_slug(slug: &Slug, session: &ReviewSession) -> Result<PathBuf> {
    match slug {
        Slug::Local(local) => Ok(local_relative_path(local, &session.repo_path)),
        Slug::Pr(_) => {
            let key = session.pr_session_key.as_ref().ok_or_else(|| {
                TuicrError::CorruptedSession(
                    "PR slug requires session.pr_session_key to be populated".to_string(),
                )
            })?;
            Ok(pr_relative_path(key))
        }
    }
}

fn local_relative_path(local: &LocalSlug, repo_path: &Path) -> PathBuf {
    let fp = repo_path_fingerprint(repo_path);
    let mut path = PathBuf::from("local").join(&fp);
    if let Some(owner) = &local.owner {
        path = path.join(sanitize_path_segment(owner));
    }
    path = path
        .join(sanitize_path_segment(&local.repo))
        .join(sanitize_path_segment(&anchor_segment(&local.anchor)));
    for segment in source_segments(&local.source) {
        path = path.join(sanitize_path_segment(&segment));
    }
    path.with_extension("json")
}

fn pr_relative_path(key: &PrSessionKey) -> PathBuf {
    PathBuf::from("gh")
        .join(sanitize_path_segment(&key.repository.owner))
        .join(sanitize_path_segment(&key.repository.name))
        .join("pr")
        .join(key.number.to_string())
        .join(format!("{}.json", sanitize_path_segment(&key.head_sha)))
}

fn anchor_segment(anchor: &SlugAnchor) -> String {
    match anchor {
        SlugAnchor::Branch(name) => name.clone(),
        SlugAnchor::Anonymous(sha) => format!("~{sha}"),
    }
}

fn source_segments(source: &SlugSource) -> Vec<String> {
    match source {
        SlugSource::Worktree => vec!["worktree".to_string()],
        SlugSource::Staged => vec!["staged".to_string()],
        SlugSource::Unstaged => vec!["unstaged".to_string()],
        SlugSource::StagedAndUnstaged => vec!["staged-and-unstaged".to_string()],
        SlugSource::Pristine => vec!["pristine".to_string()],
        SlugSource::Commits(r) => vec!["commits".to_string(), format!("{}..{}", r.base, r.head)],
        SlugSource::WorktreeAndCommits(r) => vec![
            "worktree-and-commits".to_string(),
            format!("{}..{}", r.base, r.head),
        ],
        SlugSource::StagedUnstagedAndCommits(r) => vec![
            "staged-and-unstaged-and-commits".to_string(),
            format!("{}..{}", r.base, r.head),
        ],
    }
}

/// Filenames must not contain path separators or otherwise-illegal characters.
/// Slugs are already constrained but path segments may receive raw input
/// (e.g., owner names with `.`) — be defensive.
fn sanitize_path_segment(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '~' | '+') {
            out.push(ch);
        } else {
            out.push('-');
        }
    }
    if out.is_empty() {
        "unknown".to_string()
    } else {
        out
    }
}

fn repo_path_fingerprint(repo_path: &Path) -> String {
    let canonical = fs::canonicalize(repo_path).unwrap_or_else(|_| repo_path.to_path_buf());
    let normalized = canonical.to_string_lossy().to_string();
    let normalized = if cfg!(windows) {
        normalized.to_lowercase()
    } else {
        normalized
    };
    let hash = fnv1a_64(normalized.as_bytes());
    let hex = format!("{hash:016x}");
    hex[..FINGERPRINT_HEX_LEN].to_string()
}

fn manifest_anchor_for(slug: &Slug) -> String {
    match slug {
        Slug::Local(local) => anchor_segment(&local.anchor),
        Slug::Pr(pr) => format!("pr/{}", pr.number),
    }
}

#[allow(dead_code)]
pub(crate) fn manifest_path_for(reviews_dir: &Path) -> PathBuf {
    reviews_dir.join(MANIFEST_FILENAME)
}

// ---------- Tests ----------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::traits::{ForgeRepository, PrSessionKey};
    use crate::model::FileStatus;
    use crate::persistence::manifest::Manifest;
    use std::path::PathBuf;

    struct TestReviewsDirGuard {
        path: PathBuf,
    }

    impl Drop for TestReviewsDirGuard {
        fn drop(&mut self) {
            set_test_reviews_dir(None);
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn with_test_reviews_dir() -> TestReviewsDirGuard {
        let path =
            std::env::temp_dir().join(format!("tuicr-reviews-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        set_test_reviews_dir(Some(path.clone()));
        TestReviewsDirGuard { path }
    }

    fn make_repo() -> PathBuf {
        let repo = std::env::temp_dir().join(format!("tuicr-repo-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&repo).unwrap();
        repo
    }

    fn make_local_session(
        repo_path: PathBuf,
        base_commit: &str,
        branch_name: Option<&str>,
        diff_source: SessionDiffSource,
        commit_range: Option<Vec<String>>,
    ) -> ReviewSession {
        let mut s = ReviewSession::new(
            repo_path,
            base_commit.to_string(),
            branch_name.map(|b| b.to_string()),
            diff_source,
        );
        s.commit_range = commit_range;
        s.add_file(PathBuf::from("src/main.rs"), FileStatus::Modified, 0);
        s
    }

    fn make_pr_key(number: u64, head_sha: &str) -> PrSessionKey {
        PrSessionKey::new(
            ForgeRepository::github("github.com", "agavra", "tuicr"),
            number,
            head_sha.to_string(),
        )
    }

    fn make_pr_session(key: &PrSessionKey) -> ReviewSession {
        let mut s = ReviewSession::new(
            PathBuf::from(format!(
                "forge:{}/{}/{}",
                key.repository.host, key.repository.owner, key.repository.name
            )),
            key.head_sha.clone(),
            Some("reviews".to_string()),
            SessionDiffSource::PullRequest,
        );
        s.pr_session_key = Some(key.clone());
        s
    }

    // ---- Save/load round trips ----

    #[test]
    fn should_roundtrip_local_session() {
        let _g = with_test_reviews_dir();
        let repo = make_repo();
        let session = make_local_session(
            repo.clone(),
            "abc1234",
            Some("main"),
            SessionDiffSource::WorkingTree,
            None,
        );
        let path = save_session(&session).unwrap();
        let loaded = load_session(&path).unwrap();
        assert_eq!(session.id, loaded.id);
        assert_eq!(session.base_commit, loaded.base_commit);
    }

    #[test]
    fn should_save_under_new_layout_for_local() {
        let _g = with_test_reviews_dir();
        let repo = make_repo();
        let session = make_local_session(
            repo.clone(),
            "abc1234",
            Some("main"),
            SessionDiffSource::WorkingTree,
            None,
        );
        let path = save_session(&session).unwrap();
        let display = path.display().to_string();
        assert!(display.contains("/local/"), "expected /local/ in {display}");
        assert!(display.ends_with("/worktree.json"));
    }

    #[test]
    fn should_save_under_new_layout_for_pr() {
        let _g = with_test_reviews_dir();
        let key = make_pr_key(125, "abcdef0123456789");
        let session = make_pr_session(&key);
        let path = save_session(&session).unwrap();
        let display = path.display().to_string();
        assert!(
            display.contains("/gh/agavra/tuicr/pr/125/"),
            "expected gh/agavra/tuicr/pr/125 segment in {display}"
        );
        assert!(display.ends_with("/abcdef0123456789.json"));
    }

    #[test]
    fn should_update_manifest_on_save() {
        let _g = with_test_reviews_dir();
        let repo = make_repo();
        let session = make_local_session(
            repo.clone(),
            "abc1234",
            Some("main"),
            SessionDiffSource::WorkingTree,
            None,
        );
        save_session(&session).unwrap();

        let reviews_dir = get_reviews_dir().unwrap();
        let manifest = manifest::load_manifest(&reviews_dir).unwrap();
        let slug = slug_for_session(&session).unwrap();
        let canonical = fs::canonicalize(&repo).unwrap_or(repo);
        let entry = manifest
            .get_local(&slug.to_string(), &canonical)
            .expect("entry exists");
        assert!(matches!(entry.kind, ManifestKind::Local));
        assert_eq!(entry.display.file_count, 1);
    }

    // ---- Lookup ----

    #[test]
    fn should_return_none_for_unknown_context() {
        let _g = with_test_reviews_dir();
        let repo = make_repo();
        let loaded = load_latest_session_for_context(
            &repo,
            Some("main"),
            "head",
            SessionDiffSource::WorkingTree,
            None,
        )
        .unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn should_load_session_by_matching_context() {
        let _g = with_test_reviews_dir();
        let repo = make_repo();
        let session = make_local_session(
            repo.clone(),
            "abc1234",
            Some("main"),
            SessionDiffSource::WorkingTree,
            None,
        );
        save_session(&session).unwrap();

        let (_, loaded) = load_latest_session_for_context(
            &repo,
            Some("main"),
            "new-head",
            SessionDiffSource::WorkingTree,
            None,
        )
        .unwrap()
        .expect("session should be found regardless of head_commit on a branched session");
        assert_eq!(loaded.id, session.id);
    }

    #[test]
    fn should_ignore_sessions_with_different_diff_source() {
        let _g = with_test_reviews_dir();
        let repo = make_repo();
        let session = make_local_session(
            repo.clone(),
            "abc1234",
            Some("main"),
            SessionDiffSource::WorkingTree,
            None,
        );
        save_session(&session).unwrap();

        let loaded = load_latest_session_for_context(
            &repo,
            Some("main"),
            "head",
            SessionDiffSource::Staged,
            None,
        )
        .unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn should_require_commit_range_match() {
        let _g = with_test_reviews_dir();
        let repo = make_repo();
        let range_a = vec!["c1".to_string(), "c0".to_string()];
        let range_b = vec!["c3".to_string(), "c2".to_string()];

        let session_a = make_local_session(
            repo.clone(),
            "c1",
            Some("main"),
            SessionDiffSource::CommitRange,
            Some(range_a.clone()),
        );
        save_session(&session_a).unwrap();

        let session_b = make_local_session(
            repo.clone(),
            "c3",
            Some("main"),
            SessionDiffSource::CommitRange,
            Some(range_b.clone()),
        );
        save_session(&session_b).unwrap();

        let (_, loaded_a) = load_latest_session_for_context(
            &repo,
            Some("main"),
            "c1",
            SessionDiffSource::CommitRange,
            Some(range_a.as_slice()),
        )
        .unwrap()
        .unwrap();
        assert_eq!(loaded_a.id, session_a.id);

        let (_, loaded_b) = load_latest_session_for_context(
            &repo,
            Some("main"),
            "c3",
            SessionDiffSource::CommitRange,
            Some(range_b.as_slice()),
        )
        .unwrap()
        .unwrap();
        assert_eq!(loaded_b.id, session_b.id);
    }

    #[test]
    fn should_disambiguate_two_checkouts_with_same_repo_name() {
        let _g = with_test_reviews_dir();
        let base = std::env::temp_dir().join(format!("tuicr-multi-{}", uuid::Uuid::new_v4()));
        let repo_a = base.join("a").join("same-repo");
        let repo_b = base.join("b").join("same-repo");
        fs::create_dir_all(&repo_a).unwrap();
        fs::create_dir_all(&repo_b).unwrap();

        let session_a = make_local_session(
            repo_a.clone(),
            "head-a",
            Some("main"),
            SessionDiffSource::WorkingTree,
            None,
        );
        let session_b = make_local_session(
            repo_b.clone(),
            "head-b",
            Some("main"),
            SessionDiffSource::WorkingTree,
            None,
        );
        save_session(&session_a).unwrap();
        save_session(&session_b).unwrap();

        // Same slug for both (no remote, fallback to dir name) but
        // canonical_repo_path disambiguates.
        let slug = slug_for_session(&session_a).unwrap();
        let slug_b = slug_for_session(&session_b).unwrap();
        assert_eq!(slug.to_string(), slug_b.to_string());

        let (_, loaded_a) = load_latest_session_for_context(
            &repo_a,
            Some("main"),
            "head-a",
            SessionDiffSource::WorkingTree,
            None,
        )
        .unwrap()
        .expect("repo_a lookup");
        assert_eq!(loaded_a.id, session_a.id);

        let (_, loaded_b) = load_latest_session_for_context(
            &repo_b,
            Some("main"),
            "head-b",
            SessionDiffSource::WorkingTree,
            None,
        )
        .unwrap()
        .expect("repo_b lookup");
        assert_eq!(loaded_b.id, session_b.id);

        let _ = fs::remove_dir_all(&base);
    }

    // ---- PR sessions ----

    #[test]
    fn should_roundtrip_pr_session() {
        let _g = with_test_reviews_dir();
        let key = make_pr_key(125, "abcdef0123456789");
        let session = make_pr_session(&key);
        let path = save_session(&session).unwrap();
        let (loaded_path, loaded) = load_pr_session(&key).unwrap().unwrap();
        assert_eq!(loaded_path, path);
        assert_eq!(loaded.pr_session_key.as_ref(), Some(&key));
    }

    #[test]
    fn should_return_none_when_head_changes_for_pr() {
        let _g = with_test_reviews_dir();
        let old_key = make_pr_key(125, "abcdef0123456789");
        let session = make_pr_session(&old_key);
        save_session(&session).unwrap();

        let new_key = make_pr_key(125, "9999999999999999");
        let loaded = load_pr_session(&new_key).unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn should_separate_pr_sessions_by_number() {
        let _g = with_test_reviews_dir();
        let key_a = make_pr_key(125, "abcdef0123456789");
        let key_b = make_pr_key(148, "abcdef0123456789");
        save_session(&make_pr_session(&key_a)).unwrap();
        save_session(&make_pr_session(&key_b)).unwrap();

        let loaded_a = load_pr_session(&key_a).unwrap().unwrap();
        let loaded_b = load_pr_session(&key_b).unwrap().unwrap();
        assert_eq!(loaded_a.1.pr_session_key.as_ref(), Some(&key_a));
        assert_eq!(loaded_b.1.pr_session_key.as_ref(), Some(&key_b));
    }

    #[test]
    fn should_skip_pr_files_in_local_context_lookup() {
        let _g = with_test_reviews_dir();
        let key = make_pr_key(125, "abcdef0123456789");
        save_session(&make_pr_session(&key)).unwrap();

        let repo = make_repo();
        let loaded = load_latest_session_for_context(
            &repo,
            Some("main"),
            "head",
            SessionDiffSource::WorkingTree,
            None,
        )
        .unwrap();
        assert!(loaded.is_none());
    }

    // ---- Branch sanitization in paths ----

    #[test]
    fn should_sanitize_branch_slashes_in_storage_path() {
        let _g = with_test_reviews_dir();
        let repo = make_repo();
        let session = make_local_session(
            repo.clone(),
            "abc1234",
            Some("feature/login"),
            SessionDiffSource::WorkingTree,
            None,
        );
        let path = save_session(&session).unwrap();
        let display = path.display().to_string();
        assert!(
            display.contains("/feature-login/"),
            "branch slash not sanitized in {display}"
        );
    }

    // ---- Migration ----

    #[test]
    fn should_migrate_old_flat_layout_on_first_run() {
        let _g = with_test_reviews_dir();
        let reviews_dir = get_reviews_dir().unwrap();

        // Drop an old-style flat session file into the reviews dir directly.
        let old_file = reviews_dir.join("old_session.json");
        fs::write(&old_file, "{\"legacy\":true}").unwrap();

        let session = make_local_session(
            make_repo(),
            "abc1234",
            Some("main"),
            SessionDiffSource::WorkingTree,
            None,
        );
        save_session(&session).unwrap();

        assert!(
            !old_file.exists(),
            "old flat session file should have moved during migration"
        );
        let parent = reviews_dir.parent().unwrap();
        let backup_exists = fs::read_dir(parent)
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().contains(".v1"));
        assert!(backup_exists, "expected a .v1 backup dir under {parent:?}");
    }

    #[test]
    fn should_not_migrate_when_manifest_already_present() {
        let _g = with_test_reviews_dir();
        let reviews_dir = get_reviews_dir().unwrap();
        let manifest = Manifest::new();
        manifest::save_manifest(&reviews_dir, &manifest).unwrap();

        let stray = reviews_dir.join("stray.json");
        fs::write(&stray, "{}").unwrap();

        let session = make_local_session(
            make_repo(),
            "abc1234",
            Some("main"),
            SessionDiffSource::WorkingTree,
            None,
        );
        save_session(&session).unwrap();
        assert!(
            stray.exists(),
            "stray .json must survive when manifest exists"
        );
    }
}
