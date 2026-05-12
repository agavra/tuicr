use chrono::{DateTime, TimeZone, Utc};
use git2::{BranchType, Oid, Repository};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::process::Command;

use crate::error::{Result, TuicrError};

use super::GitCapabilities;

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

const COMMIT_FORMAT: &str = "--format=%H%x00%h%x00%an%x00%ct%x00%B%x1e";

fn run_git_command<I, S>(repo: &Repository, args: I) -> Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let workdir = repo.workdir().ok_or(TuicrError::NotARepository)?;
    let output = Command::new("git")
        .current_dir(workdir)
        .args(args)
        .output()
        .map_err(|e| TuicrError::VcsCommand(format!("Failed to run git: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TuicrError::VcsCommand(stderr.trim().to_string()));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
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

fn get_branch_tip_names_cli(repo: &Repository) -> HashMap<String, Vec<String>> {
    let output = run_git_command(
        repo,
        [
            "for-each-ref",
            "--format=%(objectname)%00%(refname:short)",
            "refs/heads",
        ],
    )
    .unwrap_or_default();
    let mut names_by_tip: HashMap<String, Vec<String>> = HashMap::new();

    for line in output.lines() {
        if let Some((oid, name)) = line.split_once('\0') {
            names_by_tip
                .entry(oid.to_string())
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
    capabilities: GitCapabilities,
    offset: usize,
    limit: usize,
) -> Result<Vec<CommitInfo>> {
    if capabilities.requires_git_cli() {
        return get_recent_commits_cli(repo, offset, limit);
    }

    get_recent_commits_libgit2(repo, offset, limit)
}

fn get_recent_commits_cli(
    repo: &Repository,
    offset: usize,
    limit: usize,
) -> Result<Vec<CommitInfo>> {
    let branch_tip_names = get_branch_tip_names_cli(repo);
    let output = run_git_command(
        repo,
        [
            "log".to_string(),
            format!("--skip={offset}"),
            format!("--max-count={limit}"),
            COMMIT_FORMAT.to_string(),
        ],
    )?;

    Ok(parse_commit_records(&output, &branch_tip_names))
}

pub fn get_commits_info(
    repo: &Repository,
    capabilities: GitCapabilities,
    ids: &[String],
) -> Result<Vec<CommitInfo>> {
    if capabilities.requires_git_cli() {
        return get_commits_info_cli(repo, ids);
    }

    get_commits_info_libgit2(repo, ids)
}

fn get_commits_info_cli(repo: &Repository, ids: &[String]) -> Result<Vec<CommitInfo>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    let branch_tip_names = get_branch_tip_names_cli(repo);
    let mut args = vec![
        "show".to_string(),
        "-s".to_string(),
        COMMIT_FORMAT.to_string(),
    ];
    args.extend(ids.iter().cloned());
    let output = run_git_command(repo, args)?;

    Ok(parse_commit_records(&output, &branch_tip_names))
}

/// Resolve a git revision range expression to a list of commit IDs (oldest first).
///
/// Supports both single revisions ("HEAD~3") and ranges ("main..feature").
/// For a range A..B, walks from B back to (but not including) A.
/// For a single revision, returns just that commit.
pub fn resolve_revisions(
    repo: &Repository,
    capabilities: GitCapabilities,
    revisions: &str,
) -> Result<Vec<String>> {
    if capabilities.requires_git_cli() {
        return resolve_revisions_cli(repo, revisions);
    }

    resolve_revisions_libgit2(repo, revisions)
}

fn resolve_revisions_cli(repo: &Repository, revisions: &str) -> Result<Vec<String>> {
    let commit_ids = if revisions.contains("..") {
        let output = run_git_command(repo, ["rev-list", "--reverse", revisions])?;
        output
            .lines()
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect()
    } else {
        let revision = format!("{revisions}^{{commit}}");
        let output = run_git_command(repo, ["rev-parse", "--verify", &revision])?;
        vec![output.trim().to_string()]
    };

    if commit_ids.is_empty() {
        return Err(TuicrError::NoChanges);
    }

    Ok(commit_ids)
}

fn get_recent_commits_libgit2(
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

fn get_commits_info_libgit2(repo: &Repository, ids: &[String]) -> Result<Vec<CommitInfo>> {
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

fn resolve_revisions_libgit2(repo: &Repository, revisions: &str) -> Result<Vec<String>> {
    let revspec = repo.revparse(revisions)?;

    let mut commit_ids = if revspec.mode().contains(git2::RevparseMode::RANGE) {
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

    commit_ids.reverse();
    Ok(commit_ids)
}

fn parse_commit_records(
    output: &str,
    branch_tip_names: &HashMap<String, Vec<String>>,
) -> Vec<CommitInfo> {
    output
        .split('\x1e')
        .filter_map(|record| parse_commit_record(record, branch_tip_names))
        .collect()
}

fn parse_commit_record(
    record: &str,
    branch_tip_names: &HashMap<String, Vec<String>>,
) -> Option<CommitInfo> {
    let record = record.trim_start_matches('\n').trim_end_matches('\n');
    if record.is_empty() {
        return None;
    }

    let mut fields = record.splitn(5, '\0');
    let id = fields.next()?.to_string();
    let short_id = fields.next()?.to_string();
    let author = fields.next().unwrap_or("Unknown").to_string();
    let timestamp = fields
        .next()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or_default();
    let full_message = fields.next().unwrap_or("(no message)");
    let (summary, body) = parse_commit_message(full_message);
    let branch_name = branch_tip_names
        .get(&id)
        .and_then(|names| names.first().cloned());
    let time = Utc
        .timestamp_opt(timestamp, 0)
        .single()
        .unwrap_or_else(Utc::now);

    Some(CommitInfo {
        id,
        short_id,
        branch_name,
        summary,
        body,
        author,
        time,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vcs::git::GitRepoMode;
    use std::fs;
    use std::process::Command;

    fn git(repo: &Repository, args: &[&str]) {
        let workdir = repo.workdir().expect("test repo should have workdir");
        let output = Command::new("git")
            .current_dir(workdir)
            .args(args)
            .output()
            .expect("failed to run git");
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn write_file(repo: &Repository, path: &str, content: &str) {
        let workdir = repo.workdir().expect("test repo should have workdir");
        let full_path = workdir.join(path);
        fs::create_dir_all(full_path.parent().expect("test path should have parent"))
            .expect("failed to create parent");
        fs::write(full_path, content).expect("failed to write file");
    }

    fn setup_sparse_index_repo() -> (tempfile::TempDir, Repository, Vec<String>) {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let repo = Repository::init(temp_dir.path()).expect("failed to init repo");

        git(&repo, &["config", "user.email", "test@example.com"]);
        git(&repo, &["config", "user.name", "Test User"]);
        write_file(&repo, "keep/file.txt", "keep base\n");
        write_file(&repo, "hidden/file.txt", "hidden base\n");
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-m", "initial"]);
        let first_id = run_git_command(&repo, ["rev-parse", "HEAD"])
            .expect("failed to resolve first commit")
            .trim()
            .to_string();

        write_file(&repo, "keep/file.txt", "keep next\n");
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-m", "second"]);
        let second_id = run_git_command(&repo, ["rev-parse", "HEAD"])
            .expect("failed to resolve second commit")
            .trim()
            .to_string();

        git(&repo, &["sparse-checkout", "init", "--cone"]);
        git(&repo, &["sparse-checkout", "set", "keep"]);
        git(&repo, &["sparse-checkout", "reapply", "--sparse-index"]);

        (temp_dir, repo, vec![first_id, second_id])
    }

    fn sparse_index_capabilities() -> GitCapabilities {
        GitCapabilities {
            mode: GitRepoMode::SparseIndex,
            untracked_cache: false,
            fsmonitor: false,
        }
    }

    #[test]
    fn get_recent_commits_supports_sparse_index() {
        let (_temp_dir, repo, _ids) = setup_sparse_index_repo();

        let commits = get_recent_commits(&repo, sparse_index_capabilities(), 0, 10)
            .expect("failed to get commits");

        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].summary, "second");
        assert_eq!(commits[1].summary, "initial");
    }

    #[test]
    fn get_commits_info_supports_sparse_index() {
        let (_temp_dir, repo, ids) = setup_sparse_index_repo();

        let commits = get_commits_info(
            &repo,
            sparse_index_capabilities(),
            &[ids[0].clone(), ids[1].clone()],
        )
        .expect("failed to get commit info");

        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].id, ids[0]);
        assert_eq!(commits[0].summary, "initial");
        assert_eq!(commits[1].id, ids[1]);
        assert_eq!(commits[1].summary, "second");
    }

    #[test]
    fn resolve_revisions_supports_sparse_index() {
        let (_temp_dir, repo, ids) = setup_sparse_index_repo();
        let revset = format!("{}..{}", ids[0], ids[1]);

        let resolved = resolve_revisions(&repo, sparse_index_capabilities(), &revset)
            .expect("failed to resolve revisions");

        assert_eq!(resolved, vec![ids[1].clone()]);
    }
}
