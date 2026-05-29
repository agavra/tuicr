//! Bitbucket forge backend via `bkt` CLI.

pub mod bkt;
pub mod models;
pub mod submit;

use std::path::PathBuf;

use crate::error::{Result, TuicrError};
use crate::forge::bitbucket::bkt::{BktCommandRunner, SystemBktRunner, map_bkt_error};
use crate::forge::bitbucket::models::{
    pr_list_entry_to_summary, pr_view_to_details, BktCommentsList, BktPrView,
    BktPrListResponse,
};
use crate::forge::remote_comments::{
    RemoteCommentSide, RemoteReviewComment, RemoteReviewThread,
};
use crate::forge::submit::SubmitEvent;
use crate::forge::traits::{
    CreateReviewRequest, ForgeBackend, ForgeFileLinesRequest, ForgeRepository,
    GhCreateReviewResponse, PagedPullRequests, PullRequestCommit, PullRequestDetails,
    PullRequestListQuery, PullRequestTarget,
};
use crate::model::{DiffLine, LineOrigin};
use chrono::Utc;
use std::path::Path;
use std::process::Command;

// ── Local git helpers ────────────────────────────────────────────────────

/// Run a git command in a specific directory.
fn run_git_in_dir(cwd: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        None
    }
}

/// Compute the merge-base between two refs using local git.
fn git_merge_base(cwd: &Path, ref1: &str, ref2: &str) -> Option<String> {
    // expand short hashes first so merge-base can find them
    let r1 = git_rev_parse(cwd, ref1).unwrap_or_else(|| ref1.to_string());
    let r2 = git_rev_parse(cwd, ref2).unwrap_or_else(|| ref2.to_string());
    run_git_in_dir(cwd, &["merge-base", &r1, &r2])
        .and_then(|s| s.trim().to_string().into_some())
}

/// Expand a short ref/hash to full SHA.
fn git_rev_parse(cwd: &Path, rev: &str) -> Option<String> {
    run_git_in_dir(cwd, &["rev-parse", rev])
        .and_then(|s| s.trim().to_string().into_some())
}

/// Run `git log` to list commits between two refs.
fn git_log_range(cwd: &Path, range: &str) -> Option<Vec<PullRequestCommit>> {
    let output = run_git_in_dir(cwd, &["log", "--format=%H%n%h%n%s%n%an%n%aI", range])?;
    let lines: Vec<&str> = output.lines().collect();
    let mut commits = Vec::new();
    for chunk in lines.chunks(5) {
        if chunk.len() >= 5 {
            commits.push(PullRequestCommit {
                oid: chunk[0].to_string(),
                short_oid: chunk[1].to_string(),
                summary: chunk[2].to_string(),
                author: chunk[3].to_string(),
                timestamp: chrono::DateTime::parse_from_rfc3339(chunk[4])
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc)),
            });
        }
    }
    Some(commits)
}

/// Run `git show <sha>:<path>` to fetch file content for context expansion.
fn git_show_blob(cwd: &Path, sha: &str, path: &Path) -> Option<String> {
    let spec = format!("{}:{}", sha, path.to_string_lossy());
    run_git_in_dir(cwd, &["show", &spec])
}

/// Run `git diff <start>..<end>` for commit range diff.
fn git_range_diff(cwd: &Path, start: &str, end: &str) -> Option<String> {
    run_git_in_dir(cwd, &["diff", &format!("{start}..{end}")])
}

/// Convert a string to Some if non-empty.
trait NonEmptyOption {
    fn into_some(self) -> Option<String>;
}
impl NonEmptyOption for String {
    fn into_some(self) -> Option<String> {
        if self.trim().is_empty() {
            None
        } else {
            Some(self)
        }
    }
}

// ── Backend ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct BitbucketBktBackend<R = SystemBktRunner> {
    default_repository: Option<ForgeRepository>,
    runner: R,
    local_checkout: Option<PathBuf>,
}

impl BitbucketBktBackend<SystemBktRunner> {
    pub fn new(default_repository: Option<ForgeRepository>) -> Self {
        Self {
            default_repository,
            runner: SystemBktRunner,
            local_checkout: None,
        }
    }

    pub fn with_local_checkout(mut self, checkout: Option<PathBuf>) -> Self {
        self.local_checkout = checkout;
        self
    }
}

impl<R: BktCommandRunner> BitbucketBktBackend<R> {
    pub fn with_runner(default_repository: Option<ForgeRepository>, runner: R) -> Self {
        Self {
            default_repository,
            runner,
            local_checkout: None,
        }
    }

    pub fn set_local_checkout(&mut self, checkout: Option<PathBuf>) {
        self.local_checkout = checkout;
    }

    pub fn local_checkout(&self) -> Option<&Path> {
        self.local_checkout.as_deref()
    }

    fn resolve_repository(&self, target: &PullRequestTarget) -> Result<ForgeRepository> {
        target
            .repository
            .clone()
            .or_else(|| self.default_repository.clone())
            .ok_or_else(|| {
                TuicrError::Forge(format!(
                    "Bitbucket pull request target `{}` does not include a repository",
                    target.original
                ))
            })
    }

    fn run_bkt(&self, args: Vec<String>) -> Result<String> {
        self.runner
            .run(&args)
            .map_err(map_bkt_error)
    }

    fn runner(&self) -> &R {
        &self.runner
    }
}

impl<R: BktCommandRunner> ForgeBackend for BitbucketBktBackend<R>
where
    R: BktCommandRunner,
{
    fn local_checkout_path(&self) -> Option<PathBuf> {
        self.local_checkout.clone()
    }

    fn list_pull_requests(&self, query: PullRequestListQuery) -> Result<PagedPullRequests> {
        let requested = query.already_loaded + query.page_size + 1;
        let output = self.run_bkt(vec![
            "pr".to_string(),
            "list".to_string(),
            "--json".to_string(),
            "--limit".to_string(),
            requested.to_string(),
        ])?;
        let rows = {
            let resp: BktPrListResponse = serde_json::from_str(&output)?;
            resp.pull_requests
        };
        let has_more = rows.len() > query.already_loaded + query.page_size;
        let pull_requests: Vec<crate::forge::traits::PullRequestSummary> = rows
            .into_iter()
            .skip(query.already_loaded)
            .take(query.page_size)
            .map(|row| pr_list_entry_to_summary(row, &query.repository))
            .collect();
        let total_loaded = query.already_loaded + pull_requests.len();

        Ok(PagedPullRequests {
            pull_requests,
            has_more,
            total_loaded,
        })
    }

    fn get_pull_request(&self, target: PullRequestTarget) -> Result<PullRequestDetails> {
        let repository = self.resolve_repository(&target)?;
        let output = self.run_bkt(vec![
            "pr".to_string(),
            "view".to_string(),
            target.number.to_string(),
            "--json".to_string(),
        ])?;
        let view: BktPrView = serde_json::from_str(&output)?;
        let pr = view
            .pull_request
            .ok_or_else(|| TuicrError::Forge("Bitbucket response missing pull_request".to_string()))?;

        // Compute base_sha via local git merge-base.
        let dest_branch = pr
            .destination
            .as_ref()
            .and_then(|d| d.branch.as_ref())
            .and_then(|b| b.name.as_ref())
            .ok_or_else(|| {
                TuicrError::Forge("Bitbucket response missing destination branch".to_string())
            })?;
        let head_sha = pr
            .source
            .as_ref()
            .and_then(|s| s.commit.as_ref())
            .map(|c| c.hash.clone())
            .ok_or_else(|| {
                TuicrError::Forge("Bitbucket response missing source commit hash".to_string())
            })?;

        let base_sha = self
            .local_checkout
            .as_deref()
            .and_then(|cwd| {
                let origin_ref = format!("origin/{}", dest_branch);
                git_merge_base(cwd, &origin_ref, &head_sha)
            })
            .unwrap_or_default();

        pr_view_to_details(pr, &repository.host, &repository.owner, &repository.name, target.number, base_sha)
    }

    fn get_pull_request_diff(&self, pr: &PullRequestDetails) -> Result<String> {
        self.run_bkt(vec![
            "pr".to_string(),
            "diff".to_string(),
            pr.number.to_string(),
        ])
    }

    fn fetch_file_lines(&self, request: ForgeFileLinesRequest) -> Result<Vec<DiffLine>> {
        if request.start_line == 0 || request.start_line > request.end_line {
            return Ok(Vec::new());
        }

        // Use local git show when we have a checkout.
        let content = self
            .local_checkout
            .as_deref()
            .and_then(|cwd| git_show_blob(cwd, request.sha(), request.path.as_path()))
            .ok_or_else(|| {
                TuicrError::Forge(
                    "Cannot fetch file lines: no local checkout or blob not found".to_string(),
                )
            })?;

        let lines: Vec<&str> = content.lines().collect();
        let mut result = Vec::new();
        for line_num in request.start_line..=request.end_line {
            let idx = (line_num - 1) as usize;
            if idx < lines.len() {
                result.push(DiffLine {
                    origin: LineOrigin::Context,
                    content: lines[idx].to_string(),
                    old_lineno: Some(line_num),
                    new_lineno: Some(line_num),
                    highlighted_spans: None,
                });
            }
        }
        Ok(result)
    }

    fn list_review_threads(&self, pr: &PullRequestDetails) -> Result<Vec<RemoteReviewThread>> {
        let output = self.run_bkt(vec![
            "pr".to_string(),
            "comments".to_string(),
            pr.number.to_string(),
            "--json".to_string(),
        ])?;
        let comments_list: BktCommentsList = serde_json::from_str(&output)?;

        // Group comments into threads: roots have no parent, replies reference parent.id.
        let mut roots: Vec<u64> = Vec::new();
        let mut by_id: std::collections::HashMap<u64, &crate::forge::bitbucket::models::BktComment> =
            std::collections::HashMap::new();

        for comment in &comments_list.comments {
            if comment.inline.is_some() {
                // Only inline comments become threads.
                if comment.parent.is_none() {
                    roots.push(comment.id);
                }
                by_id.insert(comment.id, comment);
            }
        }

        // Attach replies to their root.
        let mut thread_replies: std::collections::HashMap<u64, Vec<&crate::forge::bitbucket::models::BktComment>> =
            std::collections::HashMap::new();
        for comment in &comments_list.comments {
            if let Some(parent_id) = comment.parent.as_ref().map(|p| p.id) {
                if comment.inline.is_some() {
                    thread_replies
                        .entry(parent_id)
                        .or_default()
                        .push(comment);
                }
            }
        }

        let mut threads = Vec::new();
        for root_id in roots {
            let root = by_id.get(&root_id).unwrap();
            let inline = root.inline.as_ref().unwrap();

            let path = inline.path.clone().unwrap_or_default();
            let line = inline.to.or(inline.from);
            let side = match (inline.from, inline.to) {
                (Some(_), None) => RemoteCommentSide::Left,
                _ => RemoteCommentSide::Right,
            };

            let is_resolved = root.resolution.as_ref().map(|r| r.user.is_some()).unwrap_or(false);

            let mut comments = Vec::new();
            // Root comment first.
            comments.push(RemoteReviewComment {
                id: root.id.to_string(),
                author: root.user.as_ref().and_then(|u| u.display_name.clone()),
                body: root
                    .content
                    .as_ref()
                    .map(|c| c.raw.clone())
                    .unwrap_or_default(),
                created_at: root.created_on,
                in_reply_to: None,
                url: String::new(),
            });
            // Then replies.
            if let Some(replies) = thread_replies.get(&root_id) {
                for reply in replies {
                    comments.push(RemoteReviewComment {
                        id: reply.id.to_string(),
                        author: reply.user.as_ref().and_then(|u| u.display_name.clone()),
                        body: reply
                            .content
                            .as_ref()
                            .map(|c| c.raw.clone())
                            .unwrap_or_default(),
                        created_at: reply.created_on,
                        in_reply_to: Some(reply.parent.as_ref().map(|p| p.id.to_string()).unwrap_or_default()),
                        url: String::new(),
                    });
                }
            }

            threads.push(RemoteReviewThread {
                id: root_id.to_string(),
                path,
                line,
                side,
                is_resolved,
                is_outdated: false, // Bitbucket doesn't track this.
                comments,
            });
        }

        Ok(threads)
    }

    fn list_pull_request_commits(&self, pr: &PullRequestDetails) -> Result<Vec<PullRequestCommit>> {
        // Use local git log between base and head refs.
        let Some(cwd) = self.local_checkout.as_deref() else {
            return Ok(Vec::new());
        };

        // We need the origin branch names. The PR details have ref names but
        // not the full origin/ prefix. Try common patterns.
        let head_ref = &pr.head_ref_name;
        let base_ref = &pr.base_ref_name;

        // Try `git log origin/base..origin/head` first.
        if let Some(commits) = git_log_range(cwd, &format!("origin/{base_ref}..origin/{head_ref}")) {
            return Ok(commits);
        }

        // Fall back to using SHAs directly.
        if let Some(commits) = git_log_range(cwd, &format!("{}..{}", pr.base_sha, pr.head_sha)) {
            return Ok(commits);
        }

        Ok(Vec::new())
    }

    fn get_pull_request_commit_range_diff(
        &self,
        _pr: &PullRequestDetails,
        start_sha: &str,
        end_sha: &str,
    ) -> Result<String> {
        // Fast path: local git diff when we have a checkout.
        if let Some(cwd) = self.local_checkout.as_deref() {
            if let Some(diff) = git_range_diff(cwd, start_sha, end_sha) {
                return Ok(diff);
            }
        }

        // Fall back to `bkt commit diff`.
        self.run_bkt(vec![
            "commit".to_string(),
            "diff".to_string(),
            start_sha.to_string(),
            end_sha.to_string(),
        ])
    }

    fn create_review(
        &self,
        pr: &PullRequestDetails,
        request: CreateReviewRequest<'_>,
    ) -> Result<GhCreateReviewResponse> {
        if matches!(request.event, SubmitEvent::RequestChanges) {
            return Err(TuicrError::Forge(
                "Bitbucket does not support Request Changes. Use Comment or Approve.".to_string(),
            ));
        }

        use crate::forge::bitbucket::submit::{
            create_review as bkt_create_review, build_response,
        };

        let pending = matches!(request.event, SubmitEvent::Draft);
        let result = bkt_create_review(
            self.runner(),
            pr.number,
            request.event,
            request.body,
            request.comments,
            pending,
        )?;

        Ok(build_response(
            pr.number,
            &pr.repository.host,
            &pr.repository.owner,
            &pr.repository.name,
            &result,
        ))
    }
}

// ── URL / remote parsing ─────────────────────────────────────────────────

/// Parse a Bitbucket remote URL into a `ForgeRepository`.
/// Supports:
/// - `git@bitbucket.org:<workspace>/<repo>.git`
/// - `https://bitbucket.org/<workspace>/<repo>.git`
/// - `ssh://git@bitbucket.org/<workspace>/<repo>.git`
pub fn parse_bitbucket_remote_url(remote_url: &str) -> Option<ForgeRepository> {
    let trimmed = trim_url_suffix(remote_url.trim());
    if trimmed.is_empty() {
        return None;
    }

    // SCP-style: git@bitbucket.org:workspace/repo.git
    if let Some((host, path)) = parse_scp_like_remote(trimmed) {
        if host != "bitbucket.org" {
            return None;
        }
        return repository_from_path(host, path);
    }

    // With scheme: https://bitbucket.org/workspace/repo.git
    let without_scheme = strip_scheme(trimmed).unwrap_or(trimmed);
    let without_user = without_scheme
        .rsplit_once('@')
        .map(|(_, rest)| rest)
        .unwrap_or(without_scheme);
    let (host, path) = without_user.split_once('/')?;
    if host != "bitbucket.org" {
        return None;
    }
    repository_from_path(host, path)
}

fn parse_scp_like_remote(remote_url: &str) -> Option<(&str, &str)> {
    if remote_url.contains("://") {
        return None;
    }
    let (host_part, path) = remote_url.split_once(':')?;
    if host_part.contains('/') || path.is_empty() {
        return None;
    }
    let host = host_part
        .rsplit_once('@')
        .map(|(_, h)| h)
        .unwrap_or(host_part);
    Some((host, path))
}

fn repository_from_path(host: &str, path: &str) -> Option<ForgeRepository> {
    let mut parts = path.split('/').filter(|part| !part.is_empty());
    let owner = parts.next()?;
    let repo = parts.next()?;
    Some(ForgeRepository::bitbucket(
        host.to_string(),
        owner.to_string(),
        strip_git_suffix(trim_url_suffix(repo)),
    ))
}

fn strip_scheme(value: &str) -> Option<&str> {
    value
        .strip_prefix("https://")
        .or_else(|| value.strip_prefix("http://"))
        .or_else(|| value.strip_prefix("ssh://"))
}

fn trim_url_suffix(value: &str) -> &str {
    value
        .split(['?', '#'])
        .next()
        .unwrap_or(value)
        .trim_end_matches('/')
}

fn strip_git_suffix(value: &str) -> &str {
    value.strip_suffix(".git").unwrap_or(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_https_remote_url() {
        let repo = parse_bitbucket_remote_url("https://bitbucket.org/workspace/repo.git").unwrap();
        assert_eq!(repo.host, "bitbucket.org");
        assert_eq!(repo.owner, "workspace");
        assert_eq!(repo.name, "repo");
    }

    #[test]
    fn parses_scp_like_ssh_remote_url() {
        let repo = parse_bitbucket_remote_url("git@bitbucket.org:workspace/repo.git").unwrap();
        assert_eq!(repo.host, "bitbucket.org");
        assert_eq!(repo.owner, "workspace");
        assert_eq!(repo.name, "repo");
    }

    #[test]
    fn parses_ssh_scheme_remote_url() {
        let repo = parse_bitbucket_remote_url("ssh://git@bitbucket.org/workspace/repo.git").unwrap();
        assert_eq!(repo.host, "bitbucket.org");
        assert_eq!(repo.owner, "workspace");
        assert_eq!(repo.name, "repo");
    }

    #[test]
    fn rejects_non_bitbucket_host() {
        assert!(parse_bitbucket_remote_url("https://github.com/workspace/repo.git").is_none());
        assert!(parse_bitbucket_remote_url("git@gitlab.com:workspace/repo.git").is_none());
    }

    #[test]
    fn strips_git_suffix() {
        let repo = parse_bitbucket_remote_url("https://bitbucket.org/ws/my-repo.git").unwrap();
        assert_eq!(repo.name, "my-repo");
    }

    #[test]
    fn handles_no_git_suffix() {
        let repo = parse_bitbucket_remote_url("https://bitbucket.org/ws/my-repo").unwrap();
        assert_eq!(repo.name, "my-repo");
    }
}
