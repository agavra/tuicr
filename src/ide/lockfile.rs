//! Lock file management for Claude Code IDE discovery.
//!
//! Lock files are written to ~/.claude/ide/{port}.lock to allow Claude Code
//! to discover running IDE integrations.

use std::path::PathBuf;

use super::protocol::LockFileContent;

/// Lock file manager that handles creation and cleanup of the lock file.
pub struct LockFile {
    path: PathBuf,
}

impl LockFile {
    /// Get the directory for IDE lock files (~/.claude/ide/)
    pub fn lock_dir() -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join(".claude").join("ide"))
    }

    /// Create a new lock file for the given port.
    pub async fn create(port: u16, workspace_path: &str) -> Result<Self, LockFileError> {
        let lock_dir = Self::lock_dir().ok_or(LockFileError::NoHomeDir)?;

        // Create the directory if it doesn't exist
        tokio::fs::create_dir_all(&lock_dir)
            .await
            .map_err(|e| LockFileError::Io(format!("Failed to create lock directory: {e}")))?;

        let lock_path = lock_dir.join(format!("{port}.lock"));

        let content = LockFileContent {
            pid: std::process::id(),
            workspace_path: workspace_path.to_string(),
            transport: format!("ws://127.0.0.1:{port}"),
            ide_name: "tuicr".to_string(),
            ide_version: Some(env!("CARGO_PKG_VERSION").to_string()),
        };

        let json = serde_json::to_string_pretty(&content)
            .map_err(|e| LockFileError::Serialize(e.to_string()))?;

        tokio::fs::write(&lock_path, json)
            .await
            .map_err(|e| LockFileError::Io(format!("Failed to write lock file: {e}")))?;

        Ok(Self { path: lock_path })
    }

    /// Get the path to the lock file.
    #[allow(dead_code)]
    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    /// Remove the lock file.
    #[allow(dead_code)]
    pub async fn remove(&self) -> Result<(), LockFileError> {
        if self.path.exists() {
            tokio::fs::remove_file(&self.path)
                .await
                .map_err(|e| LockFileError::Io(format!("Failed to remove lock file: {e}")))?;
        }
        Ok(())
    }
}

impl Drop for LockFile {
    fn drop(&mut self) {
        // Best-effort cleanup using blocking I/O since we can't async in Drop
        let _ = std::fs::remove_file(&self.path);
    }
}

#[derive(Debug)]
pub enum LockFileError {
    NoHomeDir,
    Io(String),
    Serialize(String),
}

impl std::fmt::Display for LockFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoHomeDir => write!(f, "Could not determine home directory"),
            Self::Io(msg) => write!(f, "Lock file I/O error: {msg}"),
            Self::Serialize(msg) => write!(f, "Lock file serialization error: {msg}"),
        }
    }
}

impl std::error::Error for LockFileError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn lock_dir_returns_path() {
        let dir = LockFile::lock_dir();
        assert!(dir.is_some());
        let dir = dir.unwrap();
        assert!(dir.ends_with(".claude/ide"));
    }

    #[test]
    fn lock_file_error_display() {
        let err = LockFileError::NoHomeDir;
        assert_eq!(format!("{err}"), "Could not determine home directory");

        let err = LockFileError::Io("test error".to_string());
        assert!(format!("{err}").contains("test error"));

        let err = LockFileError::Serialize("serialize error".to_string());
        assert!(format!("{err}").contains("serialize error"));
    }

    #[tokio::test]
    async fn lock_file_create_and_cleanup() {
        // Use a temporary directory for testing
        let temp_dir = env::temp_dir().join("tuicr_test_lockfile");
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
        tokio::fs::create_dir_all(&temp_dir).await.unwrap();

        // Create a lock file manually in temp dir (since we can't mock lock_dir)
        let lock_path = temp_dir.join("12345.lock");
        let content = super::super::protocol::LockFileContent {
            pid: std::process::id(),
            workspace_path: "/test/path".to_string(),
            transport: "ws://127.0.0.1:12345".to_string(),
            ide_name: "tuicr".to_string(),
            ide_version: Some("0.7.2".to_string()),
        };
        let json = serde_json::to_string_pretty(&content).unwrap();
        tokio::fs::write(&lock_path, &json).await.unwrap();

        // Verify content
        let read_content = tokio::fs::read_to_string(&lock_path).await.unwrap();
        assert!(read_content.contains("tuicr"));
        assert!(read_content.contains("/test/path"));
        assert!(read_content.contains("ws://127.0.0.1:12345"));

        // Parse it back
        let parsed: super::super::protocol::LockFileContent =
            serde_json::from_str(&read_content).unwrap();
        assert_eq!(parsed.ide_name, "tuicr");
        assert_eq!(parsed.workspace_path, "/test/path");

        // Cleanup
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    }

    #[tokio::test]
    async fn lock_file_creates_directory() {
        // This test verifies the lock file creation logic by testing component parts
        // We can't easily test the full create() without mocking the home directory

        let temp_dir = env::temp_dir().join("tuicr_test_lockdir");
        let nested_dir = temp_dir.join("nested").join("path");

        // Clean up first
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;

        // Verify directory doesn't exist
        assert!(!nested_dir.exists());

        // Create nested directory (simulating what create() does)
        tokio::fs::create_dir_all(&nested_dir).await.unwrap();

        // Verify it was created
        assert!(nested_dir.exists());

        // Cleanup
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    }
}
