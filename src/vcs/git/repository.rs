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

/// Get commit info for specific commit IDs.
/// Returns CommitInfo in the same order as the input IDs.
pub fn get_commits_info(repo: &Repository, ids: &[String]) -> Result<Vec<CommitInfo>> {
    let branch_tip_names = get_branch_tip_names(repo);
    let mut commits = Vec::new();

    for id_str in ids {
        let oid = Oid::from_str(id_str)
            .map_err(|e| TuicrError::VcsCommand(format!("Invalid commit ID {}: {}", id_str, e)))?;
        let commit = repo
            .find_commit(oid)
            .map_err(|e| TuicrError::VcsCommand(format!("Commit not found {}: {}", id_str, e)))?;

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

/// Resolve commits from merge-base(base, HEAD) through HEAD (oldest first).
/// Returns empty vec when HEAD is already at merge-base with `base`.
pub fn resolve_base_with_head_commits(repo: &Repository, base: &str) -> Result<Vec<String>> {
    let head_commit = repo.head()?.peel_to_commit()?;

    let base_obj = repo.revparse_single(base).map_err(|e| {
        TuicrError::VcsCommand(format!("Failed to resolve base revision '{base}': {e}"))
    })?;
    let base_commit = base_obj.peel_to_commit().map_err(|e| {
        TuicrError::VcsCommand(format!("Base revision '{base}' is not a commit: {e}"))
    })?;

    let merge_base_oid = repo
        .merge_base(base_commit.id(), head_commit.id())
        .map_err(|e| {
            TuicrError::VcsCommand(format!(
                "Failed to compute merge-base between '{base}' and HEAD: {e}"
            ))
        })?;

    if merge_base_oid == head_commit.id() {
        return Ok(Vec::new());
    }

    let mut revwalk = repo.revwalk()?;
    revwalk.push(head_commit.id())?;
    revwalk.hide(merge_base_oid)?;
    revwalk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME)?;

    let mut commit_ids = Vec::new();
    for oid in revwalk {
        commit_ids.push(oid?.to_string());
    }

    // revwalk outputs newest first; reverse so oldest is first
    commit_ids.reverse();
    Ok(commit_ids)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use git2::Repository;
    use tempfile::tempdir;

    use super::*;

    fn commit_file(repo: &Repository, path: &str, content: &str, message: &str) -> Oid {
        let workdir = repo.workdir().expect("repo should have workdir");
        let full_path = workdir.join(path);
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).expect("failed to create parent dir");
        }
        fs::write(&full_path, content).expect("failed to write file");

        let mut index = repo.index().expect("failed to open index");
        index
            .add_path(Path::new(path))
            .expect("failed to add path to index");
        index.write().expect("failed to write index");

        let tree_oid = index.write_tree().expect("failed to write tree");
        let tree = repo.find_tree(tree_oid).expect("failed to find tree");
        let sig = git2::Signature::now("tuicr-test", "test@example.com")
            .expect("failed to create signature");

        let parent_commit = repo
            .head()
            .ok()
            .and_then(|head| head.target())
            .and_then(|oid| repo.find_commit(oid).ok());
        let parents: Vec<&git2::Commit<'_>> = parent_commit.iter().collect();

        repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &parents)
            .expect("failed to create commit")
    }

    #[test]
    fn resolve_base_with_head_commits_returns_commits_after_base() {
        let dir = tempdir().expect("failed to create temp dir");
        let repo = Repository::init(dir.path()).expect("failed to init repo");

        commit_file(&repo, "file.txt", "one\n", "commit 1");
        let second = commit_file(&repo, "file.txt", "two\n", "commit 2");
        let third = commit_file(&repo, "file.txt", "three\n", "commit 3");

        let commits = resolve_base_with_head_commits(&repo, "HEAD~2")
            .expect("should resolve commits from base to HEAD");

        assert_eq!(commits, vec![second.to_string(), third.to_string()]);
    }

    #[test]
    fn resolve_base_with_head_commits_returns_empty_when_base_is_head() {
        let dir = tempdir().expect("failed to create temp dir");
        let repo = Repository::init(dir.path()).expect("failed to init repo");

        commit_file(&repo, "file.txt", "one\n", "commit 1");

        let commits =
            resolve_base_with_head_commits(&repo, "HEAD").expect("should resolve with HEAD base");

        assert!(commits.is_empty());
    }

    #[test]
    fn resolve_base_with_head_commits_errors_for_invalid_base() {
        let dir = tempdir().expect("failed to create temp dir");
        let repo = Repository::init(dir.path()).expect("failed to init repo");

        commit_file(&repo, "file.txt", "one\n", "commit 1");

        let err = resolve_base_with_head_commits(&repo, "does-not-exist")
            .expect_err("invalid base should error");

        assert!(matches!(err, TuicrError::VcsCommand(_)));
    }
}
