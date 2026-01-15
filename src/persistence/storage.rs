use directories::ProjectDirs;
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Result, TuicrError};
use crate::model::{ReviewSession, SessionDiffSource};

fn get_reviews_dir() -> Result<PathBuf> {
    let proj_dirs = ProjectDirs::from("", "", "tuicr").ok_or_else(|| {
        TuicrError::Io(std::io::Error::other("Could not determine data directory"))
    })?;

    let data_dir = proj_dirs.data_dir().join("reviews");
    fs::create_dir_all(&data_dir)?;
    Ok(data_dir)
}

fn session_filename(session: &ReviewSession) -> String {
    let repo_name = session
        .repo_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    let short_commit = if session.base_commit.len() >= 7 {
        &session.base_commit[..7]
    } else {
        &session.base_commit
    };

    let timestamp = session.created_at.format("%Y%m%d_%H%M%S");

    format!("{repo_name}_{short_commit}_{timestamp}.json")
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

pub fn find_session_for_repo(repo_path: &Path) -> Result<Option<PathBuf>> {
    let reviews_dir = get_reviews_dir()?;

    let repo_name = repo_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    let mut matching_sessions: Vec<_> = fs::read_dir(&reviews_dir)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with(repo_name) && name.ends_with(".json"))
        })
        .collect();

    matching_sessions
        .sort_by_key(|e| std::cmp::Reverse(e.metadata().ok().and_then(|m| m.modified().ok())));

    Ok(matching_sessions.first().map(|e| e.path()))
}

/// Find a session that matches the given repo, commit, and diff source
pub fn find_session_for_commit(
    repo_path: &Path,
    base_commit: &str,
    diff_source: SessionDiffSource,
) -> Result<Option<PathBuf>> {
    let reviews_dir = get_reviews_dir()?;

    let repo_name = repo_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    let short_commit = if base_commit.len() >= 7 {
        &base_commit[..7]
    } else {
        base_commit
    };

    // Collect matching files sorted by modification time (most recent first)
    let mut matching_files: Vec<_> = fs::read_dir(&reviews_dir)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            // Filter by filename pattern first (faster than loading files)
            entry.file_name().to_str().is_some_and(|name| {
                name.starts_with(repo_name)
                    && name.contains(short_commit)
                    && name.ends_with(".json")
            })
        })
        .filter_map(|entry| {
            let path = entry.path();
            let modified = fs::metadata(&path).ok()?.modified().ok()?;
            Some((path, modified))
        })
        .collect();

    // Sort by modification time, most recent first
    matching_files.sort_by_key(|(_, modified)| std::cmp::Reverse(*modified));

    // Load sessions in order and return first match
    for (path, _) in matching_files {
        if let Ok(session) = load_session(&path)
            && session.base_commit == base_commit
            && session.diff_source == diff_source
        {
            return Ok(Some(path));
        }
    }

    Ok(None)
}

#[cfg(test)]
fn delete_session(path: &PathBuf) -> Result<()> {
    fs::remove_file(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{FileStatus, SessionDiffSource};
    use std::path::PathBuf;

    fn create_test_session() -> ReviewSession {
        let mut session = ReviewSession::new(
            PathBuf::from("/tmp/test-repo"),
            "abc1234def".to_string(),
            Some("main".to_string()),
            SessionDiffSource::WorkingTree,
        );
        session.add_file(PathBuf::from("src/main.rs"), FileStatus::Modified);
        session
    }

    #[test]
    fn should_generate_correct_filename() {
        // given
        let session = create_test_session();

        // when
        let filename = session_filename(&session);

        // then
        assert!(filename.starts_with("test-repo_abc1234_"));
        assert!(filename.ends_with(".json"));
    }

    #[test]
    fn should_roundtrip_session() {
        // given
        let session = create_test_session();

        // when
        let path = save_session(&session).unwrap();
        let loaded = load_session(&path).unwrap();

        // then
        assert_eq!(session.id, loaded.id);
        assert_eq!(session.base_commit, loaded.base_commit);
        assert_eq!(session.files.len(), loaded.files.len());

        // cleanup
        let _ = delete_session(&path);
    }
}
