use chrono::{DateTime, TimeZone, Utc};
use git2::{BranchType, Oid, Repository};
use std::collections::HashMap;

use crate::error::{Result, TuicrError};

#[derive(Debug, Clone)]
pub struct CommitInfo {
    pub id: String,
    pub short_id: String,
    pub branch_name: Option<String>,
    pub summary: String,
    pub author: String,
    pub time: DateTime<Utc>,
}

fn get_branch_tip_names(repo: &Repository) -> HashMap<Oid, Vec<String>> {
    let mut names_by_tip: HashMap<Oid, Vec<String>> = HashMap::new();

    if let Ok(branches) = repo.branches(Some(BranchType::Local)) {
        for (branch, _) in branches.flatten() {
            let Some(target) = branch.get().target() else {
                continue;
            };

            let Ok(Some(name)) = branch.name() else {
                continue;
            };

            names_by_tip
                .entry(target)
                .or_default()
                .push(name.to_string());
        }
    }

    for names in names_by_tip.values_mut() {
        names.sort_unstable();
    }

    names_by_tip
}

pub fn get_recent_commits(
    repo: &Repository,
    offset: usize,
    limit: usize,
) -> Result<Vec<CommitInfo>> {
    let mut revwalk = repo.revwalk()?;
    revwalk.push_head()?;
    let branch_tip_names = get_branch_tip_names(repo);

    let mut commits = Vec::new();
    for oid in revwalk.skip(offset).take(limit) {
        let oid = oid?;
        let commit = repo.find_commit(oid)?;

        let id = oid.to_string();
        let short_id = id[..7.min(id.len())].to_string();
        let summary = commit.summary().unwrap_or("(no message)").to_string();
        let author = commit.author().name().unwrap_or("Unknown").to_string();
        let branch_name = branch_tip_names
            .get(&oid)
            .and_then(|names| names.first().cloned());
        let time = Utc
            .timestamp_opt(commit.time().seconds(), 0)
            .single()
            .unwrap_or_else(Utc::now);

        commits.push(CommitInfo {
            id,
            short_id,
            branch_name,
            summary,
            author,
            time,
        });
    }

    Ok(commits)
}

/// Resolve a git revision range expression to a list of commit IDs (oldest first).
///
/// Supports both single revisions ("HEAD~3") and ranges ("main..feature").
/// For a range A..B, walks from B back to (but not including) A.
/// For a single revision, returns just that commit.
pub fn resolve_revisions(repo: &Repository, revisions: &str) -> Result<Vec<String>> {
    // Try parsing as a range first (e.g., "A..B")
    let revspec = repo.revparse(revisions)?;

    let mut commit_ids = if revspec.mode().contains(git2::RevparseMode::RANGE) {
        // Range: walk from `to` back, stopping before `from`
        let from = revspec.from().ok_or_else(|| {
            TuicrError::VcsCommand("Invalid revision range: missing 'from'".into())
        })?;
        let to = revspec
            .to()
            .ok_or_else(|| TuicrError::VcsCommand("Invalid revision range: missing 'to'".into()))?;

        let mut revwalk = repo.revwalk()?;
        revwalk.push(to.id())?;
        revwalk.hide(from.id())?;
        revwalk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME)?;

        let mut ids = Vec::new();
        for oid in revwalk {
            ids.push(oid?.to_string());
        }
        ids
    } else {
        // Single revision
        let obj = revspec
            .from()
            .ok_or_else(|| TuicrError::VcsCommand("Invalid revision expression".into()))?;
        let commit = obj
            .peel_to_commit()
            .map_err(|e| TuicrError::VcsCommand(format!("Not a commit: {}", e)))?;
        vec![commit.id().to_string()]
    };

    if commit_ids.is_empty() {
        return Err(TuicrError::NoChanges);
    }

    // revwalk outputs newest first; reverse so oldest is first
    commit_ids.reverse();
    Ok(commit_ids)
}
