use chrono::{DateTime, TimeZone, Utc};
use git2::{BranchType, Oid, Repository};
use std::collections::HashMap;

use crate::error::{Result, TuicrError};
use crate::vcs::{ResolvedRevisionRange, RevisionDiffTarget};

use super::RevisionExpression;

#[derive(Debug, Clone)]
pub struct CommitInfo {
    pub id: String,
    pub short_id: String,
    pub branch_name: Option<String>,
    pub summary: String,
    pub body: Option<String>,
    pub author: String,
    pub time: DateTime<Utc>,
}

/// Parse a full commit message into (summary, optional body).
/// The summary is the first line; the body is everything after the first blank line, trimmed.
fn parse_commit_message(message: &str) -> (String, Option<String>) {
    let mut lines = message.lines();
    let summary = lines.next().unwrap_or("(no message)").to_string();
    // Skip blank separator line(s) between summary and body
    let body_text: String = lines
        .skip_while(|l| l.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    let body = if body_text.trim().is_empty() {
        None
    } else {
        Some(body_text)
    };
    (summary, body)
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
    // Unborn HEAD (fresh `git init` / `git clone` of an empty remote): the
    // branch ref does not point at a commit yet, so there is nothing to walk.
    // Probe with `repo.head()` — `revwalk.push_head()` rewraps the underlying
    // "reference not found" error with a generic code, which would slip past
    // an `ErrorCode::UnbornBranch` check. Treat unborn HEAD as zero commits
    // so App startup falls through to the staged/unstaged paths.
    if matches!(
        repo.head().map_err(|e| e.code()),
        Err(git2::ErrorCode::UnbornBranch | git2::ErrorCode::NotFound)
    ) {
        return Ok(Vec::new());
    }
    let mut revwalk = repo.revwalk()?;
    revwalk.push_head()?;
    let branch_tip_names = get_branch_tip_names(repo);

    let mut commits = Vec::new();
    for oid in revwalk.skip(offset).take(limit) {
        let oid = oid?;
        let commit = repo.find_commit(oid)?;

        let id = oid.to_string();
        let short_id = id[..7.min(id.len())].to_string();
        let full_message = commit.message().unwrap_or("(no message)");
        let (summary, body) = parse_commit_message(full_message);
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
            body,
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
        let full_message = commit.message().unwrap_or("(no message)");
        let (summary, body) = parse_commit_message(full_message);
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
            body,
            author,
            time,
        });
    }

    Ok(commits)
}

/// Resolve a Git revision expression into selected commits and diff endpoints.
///
/// This preserves the distinction between commits selected for review metadata
/// and the old/new trees that Git would compare for the original expression.
pub fn resolve_revision_range(
    repo: &Repository,
    revisions: &str,
) -> Result<ResolvedRevisionRange<'static>> {
    match RevisionExpression::parse(revisions)? {
        RevisionExpression::Single(revision) => {
            // `HEAD`
            let head = resolve_commit_id(repo, revision)?;
            let base = first_parent_id(repo, &head)?;
            Ok(ResolvedRevisionRange::from_owned_commit_ids(
                vec![head.clone()],
                RevisionDiffTarget::Explicit { base, head },
            ))
        }
        RevisionExpression::Range { base, head } => {
            // `A..B`, `A..`, or `..B`
            let base = resolve_commit_id(repo, base)?;
            let head = resolve_commit_id(repo, head)?;
            let commit_ids = revwalk_range(repo, &base, &head)?;
            Ok(ResolvedRevisionRange::from_owned_commit_ids(
                commit_ids,
                RevisionDiffTarget::Explicit {
                    base: Some(base),
                    head,
                },
            ))
        }
        RevisionExpression::MergeBaseRange { left, right } => {
            // `A...B`
            let left = resolve_commit_id(repo, left)?;
            let right = resolve_commit_id(repo, right)?;
            let base = repo
                .merge_base(Oid::from_str(&left)?, Oid::from_str(&right)?)?
                .to_string();
            let commit_ids = revwalk_range(repo, &base, &right)?;
            Ok(ResolvedRevisionRange::from_owned_commit_ids(
                commit_ids,
                RevisionDiffTarget::Explicit {
                    base: Some(base),
                    head: right,
                },
            ))
        }
    }
}

fn resolve_commit_id(repo: &Repository, revision: &str) -> Result<String> {
    let revision = format!("{revision}^{{commit}}");
    let obj = repo.revparse_single(&revision)?;
    let commit = obj
        .peel_to_commit()
        .map_err(|e| TuicrError::VcsCommand(format!("Not a commit: {e}")))?;
    Ok(commit.id().to_string())
}

// Single-revision reviews diff the commit against its first parent.
// Root commits have no parent,
// so callers represent the old side as the empty tree with None.
fn first_parent_id(repo: &Repository, commit_id: &str) -> Result<Option<String>> {
    let commit = repo.find_commit(Oid::from_str(commit_id)?)?;
    if commit.parent_count() == 0 {
        return Ok(None);
    }
    Ok(Some(commit.parent(0)?.id().to_string()))
}

fn revwalk_range(repo: &Repository, base: &str, head: &str) -> Result<Vec<String>> {
    let mut revwalk = repo.revwalk()?;
    revwalk.push(Oid::from_str(head)?)?;
    revwalk.hide(Oid::from_str(base)?)?;
    revwalk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME)?;

    let mut commit_ids = Vec::new();
    for oid in revwalk {
        commit_ids.push(oid?.to_string());
    }

    if commit_ids.is_empty() {
        return Err(TuicrError::NoChanges);
    }

    commit_ids.reverse();
    Ok(commit_ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_return_empty_recent_commits_for_unborn_head() {
        // given a fresh `git init` repo (HEAD points at refs/heads/main but
        // the ref does not exist yet — the "naked clone" state)
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let repo = Repository::init(temp_dir.path()).expect("failed to init repo");

        // when
        let result = get_recent_commits(&repo, 0, 50);

        // then unborn HEAD is treated as zero commits, not an error
        let commits = result.expect("unborn HEAD should yield an empty list");
        assert!(commits.is_empty(), "unborn HEAD has no commits to walk");
    }
}
