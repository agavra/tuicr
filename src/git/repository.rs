use chrono::{DateTime, TimeZone, Utc};
use git2::Repository;
use std::path::PathBuf;

use crate::error::{Result, TuicrError};

#[derive(Debug, Clone)]
pub struct CommitInfo {
    pub id: String,
    pub short_id: String,
    pub summary: String,
    pub author: String,
    pub time: DateTime<Utc>,
}

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

pub fn get_recent_commits(repo: &Repository, count: usize) -> Result<Vec<CommitInfo>> {
    let mut revwalk = repo.revwalk()?;
    revwalk.push_head()?;

    let mut commits = Vec::new();
    for oid in revwalk.take(count) {
        let oid = oid?;
        let commit = repo.find_commit(oid)?;

        let id = oid.to_string();
        let short_id = id[..7.min(id.len())].to_string();
        let summary = commit.summary().unwrap_or("(no message)").to_string();
        let author = commit.author().name().unwrap_or("Unknown").to_string();
        let time = Utc
            .timestamp_opt(commit.time().seconds(), 0)
            .single()
            .unwrap_or_else(Utc::now);

        commits.push(CommitInfo {
            id,
            short_id,
            summary,
            author,
            time,
        });
    }

    Ok(commits)
}
