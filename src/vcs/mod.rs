//! VCS abstraction layer for supporting multiple version control systems.
//!
//! Currently supports:
//! - Git
//! - Mercurial
//! - Jujutsu
//!
//! ## Detection Order
//!
//! When auto-detecting the VCS type, Jujutsu is tried first because jj repos
//! are Git-backed and contain a `.git` directory. If jj detection fails, Git
//! is tried next, then Mercurial.

mod diff_parser;
pub mod git;
mod hg;
mod jj;
mod traits;

pub use git::GitBackend;
pub use hg::HgBackend;
pub use jj::JjBackend;
pub use traits::{CommitInfo, VcsBackend, VcsInfo};

use crate::error::{Result, TuicrError};

/// Detect the VCS type and return the appropriate backend.
///
/// Detection order: Jujutsu → Git → Mercurial.
/// Jujutsu is tried first because jj repos are Git-backed.
pub fn detect_vcs() -> Result<Box<dyn VcsBackend>> {
    // Try jj first since jj repos are Git-backed
    if let Ok(backend) = JjBackend::discover() {
        return Ok(Box::new(backend));
    }

    // Try git
    if let Ok(backend) = GitBackend::discover() {
        return Ok(Box::new(backend));
    }

    // Try hg
    if let Ok(backend) = HgBackend::discover() {
        return Ok(Box::new(backend));
    }

    Err(TuicrError::NotARepository)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vcs::traits::VcsType;
    use std::path::PathBuf;

    #[test]
    fn exports_are_accessible() {
        // Verify that public types are properly exported
        let _: fn() -> Result<Box<dyn VcsBackend>> = detect_vcs;

        // VcsInfo can be constructed
        let info = VcsInfo {
            root_path: PathBuf::from("/test"),
            head_commit: "abc".to_string(),
            branch_name: None,
            vcs_type: VcsType::Git,
        };
        assert_eq!(info.head_commit, "abc");

        // CommitInfo can be constructed
        let commit = CommitInfo {
            id: "abc".to_string(),
            short_id: "abc".to_string(),
            branch_name: Some("main".to_string()),
            summary: "test".to_string(),
            author: "author".to_string(),
            time: chrono::Utc::now(),
        };
        assert_eq!(commit.id, "abc");
    }

    #[test]
    fn detect_vcs_outside_repo_returns_error() {
        // When run outside any VCS repo, should return NotARepository
        // Note: This test may pass or fail depending on where tests are run
        // In CI or outside a repo, it should fail with NotARepository
        // Inside the tuicr repo (which is git), it will succeed
        let result = detect_vcs();

        // We just verify the function runs without panic
        // The actual result depends on the environment
        match result {
            Ok(backend) => {
                // If we're in a repo, we should get valid info
                let info = backend.info();
                assert!(!info.head_commit.is_empty());
            }
            Err(TuicrError::NotARepository) => {
                // Expected when outside a repo
            }
            Err(e) => {
                panic!("Unexpected error: {e:?}");
            }
        }
    }
}
