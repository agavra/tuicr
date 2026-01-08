use directories::ProjectDirs;
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Result, TuicrError};
use crate::model::ReviewSession;

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

    format!("{}_{}_{}.json", repo_name, short_commit, timestamp)
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

    fn create_test_session() -> ReviewSession {
        let mut session =
            ReviewSession::new(PathBuf::from("/tmp/test-repo"), "abc1234def".to_string());
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
