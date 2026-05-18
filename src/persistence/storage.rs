use directories::ProjectDirs;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::error::{Result, TuicrError};
use crate::forge::traits::PrSessionKey;
use crate::hash::fnv1a_64;
use crate::model::ReviewSession;
use crate::model::review::SessionDiffSource;

const SESSION_MAX_AGE_DAYS: u64 = 7;
const SESSION_FILENAME_MIN_PARTS: usize = 6;
const SESSION_FILENAME_SUFFIX_PARTS: usize = 4;
const SESSION_FILENAME_DATE_LEN: usize = 8;
const SESSION_FILENAME_TIME_LEN: usize = 6;
const FINGERPRINT_HEX_LEN: usize = 8;

struct SessionFilenameParts {
    repo_fingerprints: Vec<String>,
    diff_source: String,
}

fn parse_session_filename(filename: &str) -> Option<SessionFilenameParts> {
    // PR sessions use a different layout; never treat them as local session
    // candidates here.
    if filename.starts_with("pr_github_") {
        return None;
    }
    let stem = filename.strip_suffix(".json")?;
    let parts: Vec<&str> = stem.split('_').collect();

    if parts.len() < SESSION_FILENAME_MIN_PARTS {
        return None;
    }

    let diff_source_idx = parts.len().checked_sub(SESSION_FILENAME_SUFFIX_PARTS)?;
    let date_idx = parts.len().checked_sub(SESSION_FILENAME_SUFFIX_PARTS - 1)?;
    let time_idx = parts.len().checked_sub(SESSION_FILENAME_SUFFIX_PARTS - 2)?;
    let diff_source = parts.get(diff_source_idx)?;
    let date_part = parts.get(date_idx)?;
    let time_part = parts.get(time_idx)?;

    if !matches!(
        *diff_source,
        "worktree" | "commits" | "worktree_and_commits"
    ) {
        return None;
    }

    if !is_timestamp_part(date_part, SESSION_FILENAME_DATE_LEN)
        || !is_timestamp_part(time_part, SESSION_FILENAME_TIME_LEN)
    {
        return None;
    }

    let mut fingerprints = Vec::new();
    for part in &parts[..diff_source_idx] {
        if is_hex_fingerprint(part) && !fingerprints.iter().any(|candidate| candidate == part) {
            fingerprints.push((*part).to_string());
        }
    }

    if fingerprints.is_empty() {
        return None;
    }

    Some(SessionFilenameParts {
        repo_fingerprints: fingerprints,
        diff_source: diff_source.to_string(),
    })
}

fn is_timestamp_part(part: &str, len: usize) -> bool {
    part.len() == len && part.chars().all(|ch| ch.is_ascii_digit())
}

fn is_hex_fingerprint(part: &str) -> bool {
    part.len() == FINGERPRINT_HEX_LEN && part.chars().all(|ch| ch.is_ascii_hexdigit())
}

fn get_reviews_dir() -> Result<PathBuf> {
    #[cfg(test)]
    if let Some(dir) = std::env::var_os("TUICR_REVIEWS_DIR") {
        let path = PathBuf::from(dir);
        fs::create_dir_all(&path)?;
        return Ok(path);
    }

    let proj_dirs = ProjectDirs::from("", "", "tuicr").ok_or_else(|| {
        TuicrError::Io(std::io::Error::other("Could not determine data directory"))
    })?;

    let data_dir = proj_dirs.data_dir().join("reviews");
    fs::create_dir_all(&data_dir)?;
    Ok(data_dir)
}

const MAX_FILENAME_COMPONENT_LEN: usize = 64;

fn sanitize_filename_component(value: &str) -> String {
    let mut sanitized = String::with_capacity(value.len().min(MAX_FILENAME_COMPONENT_LEN));
    for ch in value.chars() {
        if sanitized.len() >= MAX_FILENAME_COMPONENT_LEN {
            break;
        }
        let ok = ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.');
        sanitized.push(if ok { ch } else { '-' });
    }

    let sanitized = sanitized.trim_matches('-');
    if sanitized.is_empty() {
        "unknown".to_string()
    } else {
        sanitized.to_string()
    }
}

fn repo_path_fingerprint(repo_path: &Path) -> String {
    let normalized = normalize_repo_path(repo_path);
    let hash = fnv1a_64(normalized.as_bytes());
    let hex = format!("{hash:016x}");
    hex[..FINGERPRINT_HEX_LEN].to_string()
}

fn normalize_repo_path(repo_path: &Path) -> String {
    let canonical = fs::canonicalize(repo_path).unwrap_or_else(|_| repo_path.to_path_buf());
    let normalized = canonical.to_string_lossy().to_string();

    if cfg!(windows) {
        normalized.to_lowercase()
    } else {
        normalized
    }
}

fn session_filename(session: &ReviewSession) -> String {
    // PR sessions key by forge identity + PR number + head SHA so multiple
    // PR opens of the same repo land in distinct files, and reopening the
    // same PR at the same head reuses the same session.
    if let Some(key) = session.pr_session_key.as_ref() {
        let host = sanitize_filename_component(&key.repository.host);
        let owner = sanitize_filename_component(&key.repository.owner);
        let repo = sanitize_filename_component(&key.repository.name);
        let short = sanitize_filename_component(&key.short_head());
        let id_fragment = session.id.split('-').next().unwrap_or(&session.id);
        return format!(
            "pr_github_{}_{}_{}_{}_{}_{}.json",
            host, owner, repo, key.number, short, id_fragment,
        );
    }

    let repo_name = session
        .repo_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    let repo_name = sanitize_filename_component(repo_name);
    let repo_fingerprint = repo_path_fingerprint(&session.repo_path);

    let branch = session.branch_name.as_deref().unwrap_or("detached");
    let branch = sanitize_filename_component(branch);

    let diff_source = match session.diff_source {
        SessionDiffSource::WorkingTree => "worktree",
        SessionDiffSource::Staged => "staged",
        SessionDiffSource::Unstaged => "unstaged",
        SessionDiffSource::StagedAndUnstaged => "staged_and_unstaged",
        SessionDiffSource::CommitRange => "commits",
        SessionDiffSource::WorkingTreeAndCommits => "worktree_and_commits",
        SessionDiffSource::StagedUnstagedAndCommits => "staged_unstaged_and_commits",
        SessionDiffSource::PullRequest => "pr",
        SessionDiffSource::Pristine => "pristine",
    };

    let timestamp = session.created_at.format("%Y%m%d_%H%M%S");
    let id_fragment = session.id.split('-').next().unwrap_or(&session.id);

    format!(
        "{}_{}_{}_{}_{}_{}.json",
        repo_name, repo_fingerprint, branch, diff_source, timestamp, id_fragment
    )
}

pub fn save_session(session: &ReviewSession) -> Result<PathBuf> {
    let reviews_dir = get_reviews_dir()?;
    let filename = session_filename(session);
    let path = reviews_dir.join(&filename);

    let json = serde_json::to_string_pretty(session)?;
    fs::write(&path, json)?;

    Ok(path)
}

pub fn load_session(path: &PathBuf) -> Result<ReviewSession> {
    let contents = fs::read_to_string(path)?;
    let session: ReviewSession =
        serde_json::from_str(&contents).map_err(|e| TuicrError::CorruptedSession(e.to_string()))?;
    Ok(session)
}

/// Look up the most recent persisted session for a PR keyed by forge identity,
/// PR number, and head SHA. Returns `None` when no matching session exists.
///
/// PR sessions key only by identity; we deliberately do not consult mtime or
/// other filename fields because reopening the same PR at the same head must
/// restore the exact session that was last persisted for it.
pub fn load_pr_session(key: &PrSessionKey) -> Result<Option<(PathBuf, ReviewSession)>> {
    let reviews_dir = get_reviews_dir()?;
    let entries = match fs::read_dir(&reviews_dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(TuicrError::Io(e)),
    };

    let mut best: Option<(PathBuf, ReviewSession)> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(filename) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !filename.starts_with("pr_github_") || !filename.ends_with(".json") {
            continue;
        }
        let Ok(session) = load_session(&path) else {
            continue;
        };
        if session.pr_session_key.as_ref() == Some(key) {
            // Keep the most recently updated entry when more than one exists
            // (e.g. multiple draft saves in flight before cleanup).
            let candidate_updated = session.updated_at;
            let prefer_new = match best.as_ref() {
                Some((_, current)) => candidate_updated > current.updated_at,
                None => true,
            };
            if prefer_new {
                best = Some((path, session));
            }
        }
    }
    Ok(best)
}

pub fn load_latest_session_for_context(
    repo_path: &Path,
    branch_name: Option<&str>,
    head_commit: &str,
    diff_source: SessionDiffSource,
    commit_range: Option<&[String]>,
) -> Result<Option<(PathBuf, ReviewSession)>> {
    let current_repo_path = normalize_repo_path(repo_path);
    let current_fingerprint = repo_path_fingerprint(repo_path);
    let current_diff_source = match diff_source {
        SessionDiffSource::WorkingTree => "worktree",
        SessionDiffSource::Staged => "staged",
        SessionDiffSource::Unstaged => "unstaged",
        SessionDiffSource::StagedAndUnstaged => "staged_and_unstaged",
        SessionDiffSource::CommitRange => "commits",
        SessionDiffSource::WorkingTreeAndCommits => "worktree_and_commits",
        SessionDiffSource::StagedUnstagedAndCommits => "staged_unstaged_and_commits",
        SessionDiffSource::Pristine => "pristine",
        // PR sessions are looked up via `load_pr_session`. If a caller asks
        // for the local-session loader with this diff source, return nothing.
        SessionDiffSource::PullRequest => return Ok(None),
    };

    let reviews_dir = get_reviews_dir()?;
    let now = SystemTime::now();
    let max_age = Duration::from_secs(SESSION_MAX_AGE_DAYS * 24 * 60 * 60);

    let mut session_files: Vec<_> = fs::read_dir(&reviews_dir)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            let path = entry.path();

            if !path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
            {
                return false;
            }

            // Delete sessions older than 7 days
            if let Ok(metadata) = entry.metadata()
                && let Ok(modified) = metadata.modified()
                && let Ok(age) = now.duration_since(modified)
                && age > max_age
            {
                let _ = fs::remove_file(&path);
                return false;
            }

            let Some(filename) = path.file_name().and_then(|f| f.to_str()) else {
                return false;
            };

            let Some(parts) = parse_session_filename(filename) else {
                return true;
            };

            if !parts
                .repo_fingerprints
                .iter()
                .any(|fingerprint| fingerprint == &current_fingerprint)
            {
                return false;
            }

            if parts.diff_source != current_diff_source {
                return false;
            }

            true
        })
        .collect();

    session_files.sort_by(|a, b| {
        let a_modified = a
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let b_modified = b
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .unwrap_or(SystemTime::UNIX_EPOCH);

        b_modified
            .cmp(&a_modified)
            .then_with(|| a.file_name().cmp(&b.file_name()))
    });

    let mut legacy_candidate = None;

    for entry in session_files {
        let path = entry.path();
        let Ok(session) = load_session(&path) else {
            continue;
        };

        if normalize_repo_path(&session.repo_path) != current_repo_path {
            continue;
        }

        if session.diff_source != diff_source {
            continue;
        }

        if matches!(
            diff_source,
            SessionDiffSource::CommitRange
                | SessionDiffSource::WorkingTreeAndCommits
                | SessionDiffSource::StagedUnstagedAndCommits
        ) && let Some(expected_range) = commit_range
            && session.commit_range.as_deref() != Some(expected_range)
        {
            continue;
        }

        let session_branch = session.branch_name.as_deref();
        if session_branch == branch_name {
            if branch_name.is_none() {
                // Pristine sessions key on the path-list hash, not the HEAD
                // sha, so an advancing HEAD after `git pull` still reloads
                // the same comments. Other branch-less sessions fall back to
                // exact head_commit comparison.
                let exact_match = if matches!(diff_source, SessionDiffSource::Pristine) {
                    pristine_keys_match(&session.base_commit, head_commit)
                } else {
                    session.base_commit == head_commit
                };
                if !exact_match {
                    continue;
                }
            }

            return Ok(Some((path, session)));
        }

        let eligible_legacy = branch_name.is_some()
            && legacy_candidate.is_none()
            && commit_range.is_none()
            && session_branch.is_none()
            && session.base_commit == head_commit;
        if eligible_legacy {
            legacy_candidate = Some((path, session));
        }
    }

    Ok(legacy_candidate)
}

/// Match two pristine session keys by their path-hash suffix.
///
/// Pristine `base_commit` values have the shape `"pristine:<head>:<hash>"`.
/// Two sessions belong to the same review if they share the same path-list
/// hash, regardless of HEAD; the prefix and trailing hash must both match.
fn pristine_keys_match(a: &str, b: &str) -> bool {
    let Some(a_hash) = pristine_key_hash(a) else {
        return false;
    };
    let Some(b_hash) = pristine_key_hash(b) else {
        return false;
    };
    a_hash == b_hash
}

fn pristine_key_hash(key: &str) -> Option<&str> {
    let rest = key.strip_prefix("pristine:")?;
    rest.rsplit_once(':').map(|(_, hash)| hash)
}

#[cfg(test)]
fn delete_session(path: &PathBuf) -> Result<()> {
    fs::remove_file(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::FileStatus;
    use std::path::PathBuf;
    use std::sync::{Mutex, OnceLock};
    use std::time::Duration;

    const TEST_MTIME_RETRIES: usize = 40;
    const TEST_MTIME_SLEEP_MS: u64 = 100;

    fn create_test_session() -> ReviewSession {
        let mut session = ReviewSession::new(
            PathBuf::from("/tmp/test-repo"),
            "abc1234def".to_string(),
            Some("main".to_string()),
            SessionDiffSource::WorkingTree,
        );
        session.add_file(PathBuf::from("src/main.rs"), FileStatus::Modified, 0);
        session
    }

    static TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    struct TestReviewsDirGuard<'a> {
        _lock: std::sync::MutexGuard<'a, ()>,
        path: PathBuf,
    }

    impl Drop for TestReviewsDirGuard<'_> {
        fn drop(&mut self) {
            unsafe {
                std::env::remove_var("TUICR_REVIEWS_DIR");
            }
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn with_test_reviews_dir() -> TestReviewsDirGuard<'static> {
        let lock = TEST_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
        let path =
            std::env::temp_dir().join(format!("tuicr-reviews-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        unsafe {
            std::env::set_var("TUICR_REVIEWS_DIR", path.as_os_str());
        }

        TestReviewsDirGuard { _lock: lock, path }
    }

    fn create_session(
        repo_path: PathBuf,
        base_commit: &str,
        branch_name: Option<&str>,
        diff_source: SessionDiffSource,
        commit_range: Option<Vec<String>>,
    ) -> ReviewSession {
        let mut session = ReviewSession::new(
            repo_path,
            base_commit.to_string(),
            branch_name.map(|s| s.to_string()),
            diff_source,
        );
        session.commit_range = commit_range;
        session.add_file(PathBuf::from("src/main.rs"), FileStatus::Modified, 0);
        session
    }

    fn save_legacy_session(reviews_dir: &Path, session: &ReviewSession) -> PathBuf {
        let mut value = serde_json::to_value(session).unwrap();
        let obj = value.as_object_mut().unwrap();
        obj.remove("branch_name");
        obj.remove("diff_source");
        obj.remove("commit_range");
        obj.insert(
            "version".to_string(),
            serde_json::Value::String("1.0".to_string()),
        );

        let id_fragment = session.id.split('-').next().unwrap_or(&session.id);
        let path = reviews_dir.join(format!("legacy_{id_fragment}.json"));
        fs::write(&path, serde_json::to_string_pretty(&value).unwrap()).unwrap();
        path
    }

    fn ensure_newer_mtime(newer: &Path, older: &Path) {
        let older_time = fs::metadata(older)
            .ok()
            .and_then(|m| m.modified().ok())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);

        for _ in 0..TEST_MTIME_RETRIES {
            let newer_time = fs::metadata(newer)
                .ok()
                .and_then(|m| m.modified().ok())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);

            if newer_time > older_time {
                return;
            }

            std::thread::sleep(Duration::from_millis(TEST_MTIME_SLEEP_MS));
            let contents = fs::read_to_string(newer).unwrap();
            fs::write(newer, contents).unwrap();
        }

        let newer_time = fs::metadata(newer)
            .ok()
            .and_then(|m| m.modified().ok())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);

        assert!(
            newer_time > older_time,
            "failed to produce newer mtime for {}",
            newer.display()
        );
    }

    #[test]
    fn should_generate_correct_filename() {
        let session = create_test_session();
        let filename = session_filename(&session);
        assert!(filename.starts_with("test-repo_"));
        assert!(filename.contains("_main_worktree_"));
        assert!(filename.ends_with(".json"));
    }

    #[test]
    fn should_generate_filename_for_staged_unstaged() {
        let session = create_session(
            PathBuf::from("/tmp/test-repo"),
            "abc1234def",
            Some("main"),
            SessionDiffSource::StagedAndUnstaged,
            None,
        );
        let filename = session_filename(&session);
        assert!(filename.contains("_staged_and_unstaged_"));
    }

    #[test]
    fn should_roundtrip_session() {
        let _guard = with_test_reviews_dir();
        let session = create_test_session();
        let path = save_session(&session).unwrap();
        let loaded = load_session(&path).unwrap();
        assert_eq!(session.id, loaded.id);
        assert_eq!(session.base_commit, loaded.base_commit);
        assert_eq!(session.branch_name, loaded.branch_name);
        assert_eq!(session.diff_source, loaded.diff_source);
        assert_eq!(session.files.len(), loaded.files.len());
        let _ = delete_session(&path);
    }

    #[test]
    fn should_sanitize_branch_name_in_filename() {
        let session = create_session(
            PathBuf::from("/tmp/test-repo"),
            "abc1234def",
            Some("feature/login"),
            SessionDiffSource::WorkingTree,
            None,
        );
        let filename = session_filename(&session);
        assert!(!filename.contains('/'));
        assert!(filename.contains("feature-login"));
    }

    #[test]
    fn should_select_latest_session_for_branch() {
        let _guard = with_test_reviews_dir();
        let repo_path = std::env::temp_dir().join(format!("tuicr-repo-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&repo_path).unwrap();

        let session1 = create_session(
            repo_path.clone(),
            "commit-1",
            Some("main"),
            SessionDiffSource::WorkingTree,
            None,
        );
        let path1 = save_session(&session1).unwrap();

        let session2 = create_session(
            repo_path.clone(),
            "commit-2",
            Some("main"),
            SessionDiffSource::WorkingTree,
            None,
        );
        let path2 = save_session(&session2).unwrap();
        ensure_newer_mtime(&path2, &path1);
        let (selected_path, selected) = load_latest_session_for_context(
            &repo_path,
            Some("main"),
            "head-does-not-matter-for-branch",
            SessionDiffSource::WorkingTree,
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(selected_path, path2);
        assert_ne!(selected_path, path1);
        assert_eq!(selected.base_commit, "commit-2");
    }

    #[test]
    fn should_match_branch_even_when_head_commit_differs() {
        let _guard = with_test_reviews_dir();
        let repo_path = std::env::temp_dir().join(format!("tuicr-repo-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&repo_path).unwrap();

        let session = create_session(
            repo_path.clone(),
            "old-head",
            Some("main"),
            SessionDiffSource::WorkingTree,
            None,
        );
        let _ = save_session(&session).unwrap();
        let loaded = load_latest_session_for_context(
            &repo_path,
            Some("main"),
            "new-head",
            SessionDiffSource::WorkingTree,
            None,
        )
        .unwrap();
        assert!(loaded.is_some());
    }

    #[test]
    fn should_load_session_with_underscore_branch_name() {
        let _guard = with_test_reviews_dir();
        let repo_path = std::env::temp_dir().join(format!("tuicr-repo-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&repo_path).unwrap();

        let session = create_session(
            repo_path.clone(),
            "head-commit",
            Some("feature/with_underscores"),
            SessionDiffSource::WorkingTree,
            None,
        );
        let _ = save_session(&session).unwrap();
        let loaded = load_latest_session_for_context(
            &repo_path,
            Some("feature/with_underscores"),
            "new-head",
            SessionDiffSource::WorkingTree,
            None,
        )
        .unwrap();
        assert!(loaded.is_some());
    }

    #[test]
    fn should_load_session_with_hex_like_branch_segment() {
        let _guard = with_test_reviews_dir();
        let repo_path = std::env::temp_dir().join(format!("tuicr-repo-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&repo_path).unwrap();

        let session = create_session(
            repo_path.clone(),
            "head-commit",
            Some("feature/deadbeef_fix"),
            SessionDiffSource::WorkingTree,
            None,
        );
        let _ = save_session(&session).unwrap();
        let loaded = load_latest_session_for_context(
            &repo_path,
            Some("feature/deadbeef_fix"),
            "new-head",
            SessionDiffSource::WorkingTree,
            None,
        )
        .unwrap();
        assert!(loaded.is_some());
    }

    #[test]
    fn should_prefer_branch_match_over_legacy_candidate() {
        let guard = with_test_reviews_dir();
        let repo_path = std::env::temp_dir().join(format!("tuicr-repo-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&repo_path).unwrap();

        let branch_session = create_session(
            repo_path.clone(),
            "branch-base",
            Some("main"),
            SessionDiffSource::WorkingTree,
            None,
        );
        let branch_path = save_session(&branch_session).unwrap();

        let legacy_source = create_session(
            repo_path.clone(),
            "head-commit",
            None,
            SessionDiffSource::WorkingTree,
            None,
        );
        let legacy_path = save_legacy_session(&guard.path, &legacy_source);
        let (selected_path, _selected) = load_latest_session_for_context(
            &repo_path,
            Some("main"),
            "head-commit",
            SessionDiffSource::WorkingTree,
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(selected_path, branch_path);
        assert_ne!(selected_path, legacy_path);
    }

    #[test]
    fn should_fallback_to_legacy_session_when_no_branch_session_exists() {
        let guard = with_test_reviews_dir();
        let repo_path = std::env::temp_dir().join(format!("tuicr-repo-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&repo_path).unwrap();

        let legacy_source = create_session(
            repo_path.clone(),
            "head-commit",
            None,
            SessionDiffSource::WorkingTree,
            None,
        );
        let legacy_path = save_legacy_session(&guard.path, &legacy_source);
        let (selected_path, selected) = load_latest_session_for_context(
            &repo_path,
            Some("main"),
            "head-commit",
            SessionDiffSource::WorkingTree,
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(selected_path, legacy_path);
        assert_eq!(selected.branch_name, None);
        assert_eq!(selected.diff_source, SessionDiffSource::WorkingTree);
    }

    #[test]
    fn should_not_select_legacy_session_when_head_commit_differs() {
        let guard = with_test_reviews_dir();
        let repo_path = std::env::temp_dir().join(format!("tuicr-repo-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&repo_path).unwrap();

        let legacy_source = create_session(
            repo_path.clone(),
            "old-head",
            None,
            SessionDiffSource::WorkingTree,
            None,
        );
        let _legacy_path = save_legacy_session(&guard.path, &legacy_source);
        let loaded = load_latest_session_for_context(
            &repo_path,
            Some("main"),
            "new-head",
            SessionDiffSource::WorkingTree,
            None,
        )
        .unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn should_require_commit_match_in_detached_head() {
        let _guard = with_test_reviews_dir();
        let repo_path = std::env::temp_dir().join(format!("tuicr-repo-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&repo_path).unwrap();

        let session = create_session(
            repo_path.clone(),
            "detached-head",
            None,
            SessionDiffSource::WorkingTree,
            None,
        );
        let _ = save_session(&session).unwrap();
        let mismatch = load_latest_session_for_context(
            &repo_path,
            None,
            "different-head",
            SessionDiffSource::WorkingTree,
            None,
        )
        .unwrap();
        let match_ = load_latest_session_for_context(
            &repo_path,
            None,
            "detached-head",
            SessionDiffSource::WorkingTree,
            None,
        )
        .unwrap();
        assert!(mismatch.is_none());
        assert!(match_.is_some());
    }

    #[test]
    fn should_ignore_sessions_with_different_diff_source() {
        let _guard = with_test_reviews_dir();
        let repo_path = std::env::temp_dir().join(format!("tuicr-repo-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&repo_path).unwrap();

        let commit_range = vec!["commit-2".to_string(), "commit-1".to_string()];
        let commits_session = create_session(
            repo_path.clone(),
            "commit-2",
            Some("main"),
            SessionDiffSource::CommitRange,
            Some(commit_range.clone()),
        );
        let _ = save_session(&commits_session).unwrap();
        let worktree = load_latest_session_for_context(
            &repo_path,
            Some("main"),
            "head",
            SessionDiffSource::WorkingTree,
            None,
        )
        .unwrap();
        let commits = load_latest_session_for_context(
            &repo_path,
            Some("main"),
            "head",
            SessionDiffSource::CommitRange,
            Some(commit_range.as_slice()),
        )
        .unwrap();
        assert!(worktree.is_none());
        assert!(commits.is_some());
    }

    #[test]
    fn should_match_commit_range_session() {
        let _guard = with_test_reviews_dir();
        let repo_path = std::env::temp_dir().join(format!("tuicr-repo-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&repo_path).unwrap();

        let commit_range_a = vec!["commit-a2".to_string(), "commit-a1".to_string()];
        let commit_range_b = vec!["commit-b2".to_string(), "commit-b1".to_string()];

        let session_a = create_session(
            repo_path.clone(),
            "commit-a2",
            Some("main"),
            SessionDiffSource::CommitRange,
            Some(commit_range_a.clone()),
        );
        let path_a = save_session(&session_a).unwrap();

        let session_b = create_session(
            repo_path.clone(),
            "commit-b2",
            Some("main"),
            SessionDiffSource::CommitRange,
            Some(commit_range_b.clone()),
        );
        let path_b = save_session(&session_b).unwrap();
        let (selected_path, selected) = load_latest_session_for_context(
            &repo_path,
            Some("main"),
            "commit-b2",
            SessionDiffSource::CommitRange,
            Some(commit_range_b.as_slice()),
        )
        .unwrap()
        .unwrap();
        assert_eq!(selected_path, path_b);
        assert_ne!(selected_path, path_a);
        assert_eq!(
            selected.commit_range.as_deref(),
            Some(commit_range_b.as_slice())
        );
    }

    #[test]
    fn should_roundtrip_commit_range_session() {
        let _guard = with_test_reviews_dir();
        let repo_path = std::env::temp_dir().join(format!("tuicr-repo-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&repo_path).unwrap();

        let commit_range = vec!["commit-2".to_string(), "commit-1".to_string()];
        let session = create_session(
            repo_path,
            "commit-2",
            Some("main"),
            SessionDiffSource::CommitRange,
            Some(commit_range.clone()),
        );
        let path = save_session(&session).unwrap();
        let loaded = load_session(&path).unwrap();
        assert_eq!(loaded.commit_range, Some(commit_range));
        assert_eq!(loaded.diff_source, SessionDiffSource::CommitRange);
        let _ = delete_session(&path);
    }

    #[test]
    fn should_require_commit_range_order_match() {
        let _guard = with_test_reviews_dir();
        let repo_path = std::env::temp_dir().join(format!("tuicr-repo-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&repo_path).unwrap();

        let commit_range = vec!["commit-2".to_string(), "commit-1".to_string()];
        let reversed_range = vec!["commit-1".to_string(), "commit-2".to_string()];

        let session = create_session(
            repo_path.clone(),
            "commit-2",
            Some("main"),
            SessionDiffSource::CommitRange,
            Some(commit_range),
        );
        let _ = save_session(&session).unwrap();
        let loaded = load_latest_session_for_context(
            &repo_path,
            Some("main"),
            "commit-2",
            SessionDiffSource::CommitRange,
            Some(reversed_range.as_slice()),
        )
        .unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn should_skip_commit_sessions_without_range_match() {
        let _guard = with_test_reviews_dir();
        let repo_path = std::env::temp_dir().join(format!("tuicr-repo-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&repo_path).unwrap();

        let commit_range = vec!["commit-2".to_string(), "commit-1".to_string()];

        let session = create_session(
            repo_path.clone(),
            "commit-2",
            Some("main"),
            SessionDiffSource::CommitRange,
            None,
        );
        let _ = save_session(&session).unwrap();
        let loaded = load_latest_session_for_context(
            &repo_path,
            Some("main"),
            "commit-2",
            SessionDiffSource::CommitRange,
            Some(commit_range.as_slice()),
        )
        .unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn should_disambiguate_repos_with_same_folder_name() {
        let _guard = with_test_reviews_dir();
        let base = std::env::temp_dir().join(format!("tuicr-repos-{}", uuid::Uuid::new_v4()));
        let repo_a = base.join("a").join("same-repo");
        let repo_b = base.join("b").join("same-repo");
        fs::create_dir_all(&repo_a).unwrap();
        fs::create_dir_all(&repo_b).unwrap();

        let session_a = create_session(
            repo_a.clone(),
            "head-a",
            Some("main"),
            SessionDiffSource::WorkingTree,
            None,
        );
        let _ = save_session(&session_a).unwrap();

        let session_b = create_session(
            repo_b.clone(),
            "head-b",
            Some("main"),
            SessionDiffSource::WorkingTree,
            None,
        );
        let _ = save_session(&session_b).unwrap();
        let (_path, selected) = load_latest_session_for_context(
            &repo_a,
            Some("main"),
            "head",
            SessionDiffSource::WorkingTree,
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(selected.base_commit, "head-a");
        assert_eq!(
            normalize_repo_path(&selected.repo_path),
            normalize_repo_path(&repo_a)
        );
    }

    fn pr_key(number: u64, head_sha: &str) -> PrSessionKey {
        PrSessionKey::new(
            crate::forge::traits::ForgeRepository::github("github.com", "agavra", "tuicr"),
            number,
            head_sha.to_string(),
        )
    }

    fn pr_session(key: &PrSessionKey) -> ReviewSession {
        let mut session = ReviewSession::new(
            PathBuf::from(format!(
                "forge:{}/{}/{}",
                key.repository.host, key.repository.owner, key.repository.name
            )),
            key.head_sha.clone(),
            Some("reviews".to_string()),
            SessionDiffSource::PullRequest,
        );
        session.pr_session_key = Some(key.clone());
        session
    }

    #[test]
    fn should_generate_pr_session_filename_keyed_by_identity_and_head() {
        // given a PR session
        let key = pr_key(125, "abcdef0123456789");
        let session = pr_session(&key);
        // when
        let filename = session_filename(&session);
        // then
        assert!(filename.starts_with("pr_github_github.com_agavra_tuicr_125_abcdef01_"));
        assert!(filename.ends_with(".json"));
    }

    #[test]
    fn should_round_trip_pr_session_via_storage() {
        let _guard = with_test_reviews_dir();
        // given
        let key = pr_key(125, "abcdef0123456789");
        let session = pr_session(&key);
        // when
        let path = save_session(&session).unwrap();
        let (loaded_path, loaded) = load_pr_session(&key).unwrap().unwrap();
        // then
        assert_eq!(loaded_path, path);
        assert_eq!(loaded.pr_session_key.as_ref(), Some(&key));
        assert_eq!(loaded.diff_source, SessionDiffSource::PullRequest);
        let _ = delete_session(&path);
    }

    #[test]
    fn should_return_none_when_no_session_matches_head_sha() {
        let _guard = with_test_reviews_dir();
        // given a session at old head
        let old_key = pr_key(125, "abcdef0123456789");
        let _ = save_session(&pr_session(&old_key)).unwrap();
        // when looking up a new head
        let new_key = pr_key(125, "9999999999999999");
        let loaded = load_pr_session(&new_key).unwrap();
        // then
        assert!(loaded.is_none());
    }

    #[test]
    fn should_skip_pr_files_when_loading_local_session() {
        let _guard = with_test_reviews_dir();
        // given a saved PR session and no local sessions
        let key = pr_key(125, "abcdef0123456789");
        let _ = save_session(&pr_session(&key)).unwrap();
        let repo_path = std::env::temp_dir().join(format!("tuicr-repo-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&repo_path).unwrap();
        // when asking for a local working-tree session
        let loaded = load_latest_session_for_context(
            &repo_path,
            Some("main"),
            "head",
            SessionDiffSource::WorkingTree,
            None,
        )
        .unwrap();
        // then PR sessions are not surfaced to the local loader
        assert!(loaded.is_none());
    }

    #[test]
    fn should_keep_pr_sessions_separate_per_pr_number() {
        let _guard = with_test_reviews_dir();
        // given two PR sessions for different numbers
        let key_a = pr_key(125, "abcdef0123456789");
        let key_b = pr_key(148, "abcdef0123456789");
        let _ = save_session(&pr_session(&key_a)).unwrap();
        let _ = save_session(&pr_session(&key_b)).unwrap();
        // when
        let loaded_a = load_pr_session(&key_a).unwrap().unwrap();
        let loaded_b = load_pr_session(&key_b).unwrap().unwrap();
        // then each load returns its matching PR
        assert_eq!(loaded_a.1.pr_session_key.as_ref(), Some(&key_a));
        assert_eq!(loaded_b.1.pr_session_key.as_ref(), Some(&key_b));
    }

    // ---------- Pristine session key tests ----------

    #[test]
    fn pristine_key_hash_extracts_trailing_segment() {
        assert_eq!(pristine_key_hash("pristine:abc1234:9f0a"), Some("9f0a"));
        assert_eq!(pristine_key_hash("pristine:none:0000"), Some("0000"));
        assert_eq!(pristine_key_hash("worktree:abc1234"), None);
        assert_eq!(pristine_key_hash("pristine:no-trailing-hash"), None);
    }

    #[test]
    fn pristine_keys_match_when_path_hash_equal_even_if_head_changed() {
        assert!(pristine_keys_match(
            "pristine:abc1234:9f0a",
            "pristine:def5678:9f0a"
        ));
        assert!(!pristine_keys_match(
            "pristine:abc1234:9f0a",
            "pristine:def5678:beef"
        ));
        assert!(!pristine_keys_match(
            "pristine:abc1234:9f0a",
            "worktree:abc1234"
        ));
    }

    #[test]
    fn pristine_session_reloads_after_head_advance() {
        let _guard = with_test_reviews_dir();
        let repo_path =
            std::env::temp_dir().join(format!("tuicr-pristine-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&repo_path).unwrap();

        let session = create_session(
            repo_path.clone(),
            "pristine:abc1234:9f0a",
            None,
            SessionDiffSource::Pristine,
            None,
        );
        let saved_path = save_session(&session).unwrap();

        // HEAD advanced; the new key shares the same path hash suffix.
        let loaded = load_latest_session_for_context(
            &repo_path,
            None,
            "pristine:def5678:9f0a",
            SessionDiffSource::Pristine,
            None,
        )
        .unwrap();
        assert!(
            loaded.is_some(),
            "advanced HEAD must still find the session"
        );
        assert_eq!(loaded.unwrap().0, saved_path);
    }

    #[test]
    fn pristine_session_does_not_reload_when_path_hash_differs() {
        let _guard = with_test_reviews_dir();
        let repo_path =
            std::env::temp_dir().join(format!("tuicr-pristine-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&repo_path).unwrap();

        let session = create_session(
            repo_path.clone(),
            "pristine:abc1234:aaaa",
            None,
            SessionDiffSource::Pristine,
            None,
        );
        save_session(&session).unwrap();

        let loaded = load_latest_session_for_context(
            &repo_path,
            None,
            "pristine:abc1234:bbbb",
            SessionDiffSource::Pristine,
            None,
        )
        .unwrap();
        assert!(
            loaded.is_none(),
            "different path-hash must NOT match a stored pristine session"
        );
    }

    #[test]
    fn pristine_session_does_not_collide_with_single_file_session() {
        let _guard = with_test_reviews_dir();
        let repo_path =
            std::env::temp_dir().join(format!("tuicr-pristine-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&repo_path).unwrap();

        // Single-file `--file` session at the same root_path uses base_commit "file".
        let single = create_session(
            repo_path.clone(),
            "file",
            None,
            SessionDiffSource::WorkingTree,
            None,
        );
        save_session(&single).unwrap();

        // Pristine session at the same root_path uses base_commit "pristine:...".
        let pristine = create_session(
            repo_path.clone(),
            "pristine:abc1234:9f0a",
            None,
            SessionDiffSource::Pristine,
            None,
        );
        let pristine_path = save_session(&pristine).unwrap();

        let loaded = load_latest_session_for_context(
            &repo_path,
            None,
            "pristine:abc1234:9f0a",
            SessionDiffSource::Pristine,
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            loaded.0, pristine_path,
            "pristine loader must return the pristine session, not the --file one"
        );
        assert_eq!(loaded.1.diff_source, SessionDiffSource::Pristine);
    }
}
