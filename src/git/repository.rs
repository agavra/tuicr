use git2::Repository;
use std::path::PathBuf;

use crate::error::{Result, TuicrError};

pub struct RepoInfo {
    pub repo: Repository,
    pub root_path: PathBuf,
    pub head_commit: String,
    pub branch_name: Option<String>,
}

impl RepoInfo {
    pub fn discover() -> Result<Self> {
        let repo = Repository::discover(".").map_err(|_| TuicrError::NotARepository)?;

        let root_path = repo
            .workdir()
            .ok_or(TuicrError::NotARepository)?
            .to_path_buf();

        let head_commit = repo
            .head()
            .ok()
            .and_then(|h| h.peel_to_commit().ok())
            .map(|c| c.id().to_string())
            .unwrap_or_else(|| "HEAD".to_string());

        let branch_name = repo.head().ok().and_then(|h| {
            if h.is_branch() {
                h.shorthand().map(|s| s.to_string())
            } else {
                None
            }
        });

        Ok(Self {
            repo,
            root_path,
            head_commit,
            branch_name,
        })
    }
}
