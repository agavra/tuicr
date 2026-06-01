use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Result, TuicrError};
use crate::forge::remote_comments::{RemoteReviewSummary, RemoteReviewThread};
use crate::forge::traits::{
    ForgeBackend, ForgeFileLinesRequest, ForgeRepository, GhCreateReviewResponse,
    PagedPullRequests, PullRequestCommit, PullRequestDetails, PullRequestListQuery,
    PullRequestListScope, PullRequestTarget,
};
use crate::model::{DiffLine, LineOrigin};
use crate::process::{
    CommandOutputError, CommandOutputErrorKind, run_command_output, run_command_output_with_stdin,
};

use super::models::{GhPrCommit, GhPullRequestDetails, GhPullRequestSummary};
use super::review_summaries::{
    build_query as build_reviews_query, parse_graphql_page as parse_reviews_page,
};
use super::review_threads::{build_query, parse_graphql_page};
use super::submit::build_review_payload;
use crate::forge::traits::CreateReviewRequest;

const DEFAULT_GITHUB_HOST: &str = "github.com";
const PR_LIST_JSON_FIELDS: &str =
    "number,title,author,headRefName,baseRefName,updatedAt,url,state,isDraft";
const PR_VIEW_JSON_FIELDS: &str = concat!(
    "number,title,url,state,isDraft,author,headRefName,baseRefName,",
    "headRefOid,baseRefOid,body,updatedAt,closed,mergedAt"
);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GhCommandError {
    MissingGh,
    Failed { status: Option<i32>, stderr: String },
}

pub type GhCommandResult<T> = std::result::Result<T, GhCommandError>;

pub trait GhCommandRunner {
    fn run(&self, args: &[String]) -> GhCommandResult<String>;

    /// Variant for `gh` invocations that take their payload on stdin (e.g.
    /// `gh api ... --input -`). The default panics; concrete runners that
    /// might be reached by code needing stdin must override.
    fn run_with_stdin(&self, _args: &[String], _stdin: &str) -> GhCommandResult<String> {
        panic!("run_with_stdin not implemented for this GhCommandRunner");
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemGhRunner;

impl GhCommandRunner for SystemGhRunner {
    fn run(&self, args: &[String]) -> GhCommandResult<String> {
        run_command_output("gh", None, args.iter().map(|arg| OsStr::new(arg.as_str())))
            .map_err(GhCommandError::from)
    }

    fn run_with_stdin(&self, args: &[String], stdin: &str) -> GhCommandResult<String> {
        run_command_output_with_stdin(
            "gh",
            None,
            args.iter().map(|arg| OsStr::new(arg.as_str())),
            stdin,
        )
        .map_err(GhCommandError::from)
    }
}

impl From<CommandOutputError> for GhCommandError {
    fn from(error: CommandOutputError) -> Self {
        match error.kind {
            CommandOutputErrorKind::NotFound => Self::MissingGh,
            CommandOutputErrorKind::SpawnFailed | CommandOutputErrorKind::Unsuccessful => {
                Self::Failed {
                    status: error.status,
                    stderr: error.stderr,
                }
            }
        }
    }
}

/// Read a git blob from a checkout at `repo_root` using `git show <sha>:<path>`.
/// Returns `None` if the object is missing or the command fails for any reason.
fn read_blob_with_repo(repo_root: &Path, sha: &str, path: &Path) -> Option<String> {
    let spec = format!("{}:{}", sha, path.to_string_lossy());
    let exists = run_command_output(
        "git",
        Some(repo_root),
        ["cat-file", "-e", spec.as_str()]
            .iter()
            .map(|s| OsStr::new(*s)),
    );
    if exists.is_err() {
        return None;
    }
    run_command_output(
        "git",
        Some(repo_root),
        ["show", spec.as_str()].iter().map(|s| OsStr::new(*s)),
    )
    .ok()
}

/// Return `Some(diff)` when both `start_sha` and `end_sha` are present in
/// the local checkout at `repo_root`, by running `git diff <start>..<end>`.
/// Returns `None` when the checkout is missing either SHA or the command
/// fails — callers fall back to the forge in that case.
fn local_range_diff(repo_root: &Path, start_sha: &str, end_sha: &str) -> Option<String> {
    for sha in [start_sha, end_sha] {
        let exists = run_command_output(
            "git",
            Some(repo_root),
            ["cat-file", "-e", sha].iter().map(|s| OsStr::new(*s)),
        );
        if exists.is_err() {
            return None;
        }
    }
    let range = format!("{start_sha}..{end_sha}");
    run_command_output(
        "git",
        Some(repo_root),
        ["diff", range.as_str()].iter().map(|s| OsStr::new(*s)),
    )
    .ok()
}

#[derive(Debug, Clone)]
pub struct GitHubGhBackend<R = SystemGhRunner> {
    default_repository: Option<ForgeRepository>,
    runner: R,
    /// Optional path to a local checkout. When present, the backend may
    /// satisfy `fetch_file_lines` from local git objects before falling back
    /// to `gh api`. It is **never** used as the source of truth for PR
    /// contents; the source of truth is always GitHub.
    local_checkout: Option<PathBuf>,
}

impl GitHubGhBackend<SystemGhRunner> {
    pub fn new(default_repository: Option<ForgeRepository>) -> Self {
        Self {
            default_repository,
            runner: SystemGhRunner,
            local_checkout: None,
        }
    }

    pub fn with_local_checkout(mut self, checkout: Option<PathBuf>) -> Self {
        self.local_checkout = checkout;
        self
    }
}

impl<R> GitHubGhBackend<R>
where
    R: GhCommandRunner,
{
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
                    "GitHub pull request target `{}` does not include a repository",
                    target.original
                ))
            })
    }

    fn run_gh(&self, args: Vec<String>, host: &str) -> Result<String> {
        self.runner
            .run(&args)
            .map_err(|err| map_gh_error(err, host))
    }
}

impl<R> ForgeBackend for GitHubGhBackend<R>
where
    R: GhCommandRunner,
{
    fn list_pull_requests(&self, query: PullRequestListQuery) -> Result<PagedPullRequests> {
        let page_size = query.page_size.max(1);
        let requested = query.already_loaded + page_size + 1;
        let mut args = vec![
            "pr".to_string(),
            "list".to_string(),
            "--repo".to_string(),
            gh_repo_arg(&query.repository),
            "--state".to_string(),
            "open".to_string(),
            "--limit".to_string(),
            requested.to_string(),
            "--json".to_string(),
            PR_LIST_JSON_FIELDS.to_string(),
        ];
        if query.scope == PullRequestListScope::ReviewRequested {
            args.push("--search".to_string());
            args.push("review-requested:@me".to_string());
        }
        let output = self.run_gh(args, &query.repository.host)?;
        let rows: Vec<GhPullRequestSummary> = serde_json::from_str(&output)?;
        let has_more = rows.len() > query.already_loaded + page_size;
        let pull_requests = rows
            .into_iter()
            .skip(query.already_loaded)
            .take(page_size)
            .map(|row| row.into_summary(&query.repository))
            .collect::<Vec<_>>();
        let total_loaded = query.already_loaded + pull_requests.len();

        Ok(PagedPullRequests {
            pull_requests,
            has_more,
            total_loaded,
        })
    }

    fn get_pull_request(&self, target: PullRequestTarget) -> Result<PullRequestDetails> {
        let repository = self.resolve_repository(&target)?;
        let output = self.run_gh(
            vec![
                "pr".to_string(),
                "view".to_string(),
                target.number.to_string(),
                "--repo".to_string(),
                gh_repo_arg(&repository),
                "--json".to_string(),
                PR_VIEW_JSON_FIELDS.to_string(),
            ],
            &repository.host,
        )?;
        let pr: GhPullRequestDetails = serde_json::from_str(&output)?;
        pr.into_details(&repository)
    }

    fn get_pull_request_diff(&self, pr: &PullRequestDetails) -> Result<String> {
        // We want the *cumulative* diff between base and head for the PR.
        // `gh pr diff --patch` returns mbox-style `git format-patch` output
        // — one patch per commit — so a 7-commit PR yields 7 separate
        // `diff --git` blocks per file, which our parser dutifully turns
        // into duplicate `DiffFile`s. Plain `gh pr diff` (no `--patch`)
        // returns the single cumulative diff. Hard-won lesson; see the
        // duplicate-files-in-list bug.
        self.run_gh(
            vec![
                "pr".to_string(),
                "diff".to_string(),
                pr.number.to_string(),
                "--repo".to_string(),
                gh_repo_arg(&pr.repository),
                "--color".to_string(),
                "never".to_string(),
            ],
            &pr.repository.host,
        )
    }

    fn local_checkout_path(&self) -> Option<PathBuf> {
        self.local_checkout.clone()
    }

    fn list_pull_request_commits(&self, pr: &PullRequestDetails) -> Result<Vec<PullRequestCommit>> {
        // GitHub paginates `pulls/<num>/commits` at 250 commits per page (max
        // per_page=100; PRs are capped at 250 commits via the API). Paginate
        // explicitly so multi-page PRs work; bound the loop defensively.
        let mut commits: Vec<PullRequestCommit> = Vec::new();
        for page in 1..=10 {
            let endpoint = format!(
                "repos/{}/{}/pulls/{}/commits?per_page=100&page={}",
                pr.repository.owner, pr.repository.name, pr.number, page,
            );
            let mut args = vec!["api".to_string()];
            if pr.repository.host != DEFAULT_GITHUB_HOST {
                args.push("--hostname".to_string());
                args.push(pr.repository.host.clone());
            }
            args.push(endpoint);
            let output = self.run_gh(args, &pr.repository.host)?;
            let rows: Vec<GhPrCommit> = serde_json::from_str(&output)?;
            let received = rows.len();
            commits.extend(rows.into_iter().map(GhPrCommit::into_pull_request_commit));
            if received < 100 {
                break;
            }
        }
        Ok(commits)
    }

    fn get_pull_request_commit_range_diff(
        &self,
        pr: &PullRequestDetails,
        start_sha: &str,
        end_sha: &str,
    ) -> Result<String> {
        // Fast path: when both SHAs live in the local checkout, `git diff`
        // gives us the cumulative diff in O(local-IO) without round-tripping
        // through GitHub. The PR diff text is the source of truth, but the
        // forge produces equivalent output for the same two SHAs, so this
        // is a safe optimization.
        if let Some(root) = self.local_checkout.as_deref()
            && let Some(diff) = local_range_diff(root, start_sha, end_sha)
        {
            return Ok(diff);
        }

        // Fall back to GitHub's compare API. `Accept: application/vnd.github.diff`
        // returns plain unified diff text instead of the JSON wrapper.
        let endpoint = format!(
            "repos/{}/{}/compare/{}...{}",
            pr.repository.owner, pr.repository.name, start_sha, end_sha,
        );
        let mut args = vec![
            "api".to_string(),
            "-H".to_string(),
            "Accept: application/vnd.github.diff".to_string(),
        ];
        if pr.repository.host != DEFAULT_GITHUB_HOST {
            args.push("--hostname".to_string());
            args.push(pr.repository.host.clone());
        }
        args.push(endpoint);
        self.run_gh(args, &pr.repository.host)
    }

    fn list_review_threads(&self, pr: &PullRequestDetails) -> Result<Vec<RemoteReviewThread>> {
        let mut all: Vec<RemoteReviewThread> = Vec::new();
        let mut cursor: Option<String> = None;
        // Bound the pagination loop so a buggy/cyclic server can't hang us.
        // 100 threads * 100 pages = 10k threads; well beyond realistic PRs.
        for _ in 0..100 {
            let args = self.build_review_threads_args(pr, cursor.as_deref());
            let output = self.run_gh(args, &pr.repository.host)?;
            let parsed = parse_graphql_page(&output)?;
            all.extend(parsed.threads);
            let Some(page_info) = parsed.page_info else {
                break;
            };
            if !page_info.has_next_page {
                break;
            }
            let Some(end_cursor) = page_info.end_cursor else {
                break;
            };
            cursor = Some(end_cursor);
        }
        Ok(all)
    }

    fn list_review_summaries(&self, pr: &PullRequestDetails) -> Result<Vec<RemoteReviewSummary>> {
        let mut all: Vec<RemoteReviewSummary> = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..100 {
            let args = self.build_review_summaries_args(pr, cursor.as_deref());
            let output = self.run_gh(args, &pr.repository.host)?;
            let parsed = parse_reviews_page(&output)?;
            all.extend(parsed.summaries);
            let Some(page_info) = parsed.page_info else {
                break;
            };
            if !page_info.has_next_page {
                break;
            }
            let Some(end_cursor) = page_info.end_cursor else {
                break;
            };
            cursor = Some(end_cursor);
        }
        Ok(all)
    }

    fn fetch_file_lines(&self, request: ForgeFileLinesRequest) -> Result<Vec<DiffLine>> {
        if request.start_line == 0 || request.start_line > request.end_line {
            return Ok(Vec::new());
        }

        // Local optimization: read the blob from a configured checkout when
        // we have it. The PR's exact SHAs may or may not be present locally;
        // we silently fall back if they aren't.
        let local_content = self
            .local_checkout
            .as_deref()
            .and_then(|root| read_blob_with_repo(root, request.sha(), request.path.as_path()));

        let content = if let Some(content) = local_content {
            content
        } else {
            self.fetch_file_via_api(&request)?
        };

        Ok(slice_to_diff_lines(
            &content,
            request.start_line,
            request.end_line,
        ))
    }

    fn create_review(
        &self,
        pr: &PullRequestDetails,
        request: CreateReviewRequest<'_>,
    ) -> Result<GhCreateReviewResponse> {
        let payload = build_review_payload(
            request.commit_id,
            request.body,
            request.event,
            request.comments,
        );
        let payload_json = serde_json::to_string(&payload)?;

        let endpoint = format!(
            "repos/{}/{}/pulls/{}/reviews",
            pr.repository.owner, pr.repository.name, pr.number,
        );
        let mut args = vec![
            "api".to_string(),
            endpoint,
            "--method".to_string(),
            "POST".to_string(),
            "--input".to_string(),
            "-".to_string(),
        ];
        if pr.repository.host != DEFAULT_GITHUB_HOST {
            args.push("--hostname".to_string());
            args.push(pr.repository.host.clone());
        }

        let output = self
            .runner
            .run_with_stdin(&args, &payload_json)
            .map_err(|err| map_create_review_error(err, &pr.repository.host))?;

        parse_create_review_response(&output)
    }
}

impl<R> GitHubGhBackend<R>
where
    R: GhCommandRunner,
{
    fn build_review_threads_args(
        &self,
        pr: &PullRequestDetails,
        cursor: Option<&str>,
    ) -> Vec<String> {
        self.build_graphql_args(pr, &build_query(cursor), cursor)
    }

    fn build_review_summaries_args(
        &self,
        pr: &PullRequestDetails,
        cursor: Option<&str>,
    ) -> Vec<String> {
        self.build_graphql_args(pr, &build_reviews_query(cursor), cursor)
    }

    fn build_graphql_args(
        &self,
        pr: &PullRequestDetails,
        query: &str,
        cursor: Option<&str>,
    ) -> Vec<String> {
        let mut args = vec![
            "api".to_string(),
            "graphql".to_string(),
            "-f".to_string(),
            format!("query={query}"),
            "-F".to_string(),
            format!("owner={}", pr.repository.owner),
            "-F".to_string(),
            format!("name={}", pr.repository.name),
            "-F".to_string(),
            format!("number={}", pr.number),
        ];
        if let Some(c) = cursor {
            args.push("-F".to_string());
            args.push(format!("after={c}"));
        }
        // Enterprise host routing — `gh api graphql --hostname` picks the
        // correct endpoint when the repo lives on a non-default host.
        if pr.repository.host != "github.com" {
            args.push("--hostname".to_string());
            args.push(pr.repository.host.clone());
        }
        args
    }

    fn fetch_file_via_api(&self, request: &ForgeFileLinesRequest) -> Result<String> {
        // `gh api repos/<owner>/<repo>/contents/<path>?ref=<sha>` returns a
        // JSON object with base64-encoded `content` for text files. The
        // `Accept: application/vnd.github.raw` header skips JSON wrapping
        // and returns raw bytes, which we use here to keep parsing simple
        // and binary-safe (callers already gate binary files out).
        let path_str = request.path.to_string_lossy().replace('\\', "/");
        let endpoint = format!(
            "repos/{}/{}/contents/{}?ref={}",
            request.repository.owner,
            request.repository.name,
            path_str,
            request.sha(),
        );
        let mut args = vec![
            "api".to_string(),
            "-H".to_string(),
            "Accept: application/vnd.github.raw".to_string(),
        ];
        if request.repository.host != "github.com" {
            args.push("--hostname".to_string());
            args.push(request.repository.host.clone());
        }
        args.push(endpoint);
        self.run_gh(args, &request.repository.host)
    }
}

fn slice_to_diff_lines(content: &str, start_line: u32, end_line: u32) -> Vec<DiffLine> {
    let lines: Vec<&str> = content.lines().collect();
    let mut result = Vec::new();
    for line_num in start_line..=end_line {
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
    result
}

pub fn parse_pull_request_target(input: &str) -> Result<PullRequestTarget> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return malformed_target(input);
    }

    if let Some(target) = parse_numeric_target(trimmed) {
        return Ok(target);
    }
    if let Some(target) = parse_url_target(trimmed) {
        return Ok(target);
    }
    if let Some(target) = parse_repo_hash_target(trimmed) {
        return Ok(target);
    }

    malformed_target(input)
}

pub fn parse_github_remote_url(remote_url: &str) -> Option<ForgeRepository> {
    let trimmed = trim_url_suffix(remote_url.trim());
    if trimmed.is_empty() {
        return None;
    }

    if let Some((host, path)) = parse_scp_like_remote(trimmed) {
        let resolved = resolve_ssh_hostname(host);
        return repository_from_path(&resolved, path);
    }

    let without_scheme = strip_scheme(trimmed).unwrap_or(trimmed);
    let without_user = without_scheme
        .rsplit_once('@')
        .map(|(_, rest)| rest)
        .unwrap_or(without_scheme);
    let (host, path) = without_user.split_once('/')?;
    repository_from_path(host, path)
}

fn parse_numeric_target(target: &str) -> Option<PullRequestTarget> {
    if !target.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    let number = target.parse::<u64>().ok()?;
    if number == 0 {
        return None;
    }
    Some(PullRequestTarget::number(number, target))
}

fn parse_url_target(target: &str) -> Option<PullRequestTarget> {
    let without_scheme = strip_scheme(target)?;
    let trimmed = trim_url_suffix(without_scheme);
    let mut parts = trimmed.split('/').filter(|part| !part.is_empty());
    let host = parts.next()?;
    let owner = parts.next()?;
    let repo = parts.next()?;
    let segment = parts.next()?;
    let (number, repository) = if segment == "pull" {
        // GitHub: /<owner>/<repo>/pull/<n>
        (
            parts.next()?.parse::<u64>().ok()?,
            ForgeRepository::github(host, owner, strip_git_suffix(repo)),
        )
    } else if segment == "-" {
        // GitLab: /<owner>/<repo>/-/merge_requests/<n>
        if parts.next()? != "merge_requests" {
            return None;
        }
        (
            parts.next()?.parse::<u64>().ok()?,
            ForgeRepository::gitlab(host, owner, strip_git_suffix(repo)),
        )
    } else {
        return None;
    };
    if number == 0 {
        return None;
    }

    Some(PullRequestTarget::with_repository(
        repository, number, target,
    ))
}

fn parse_repo_hash_target(target: &str) -> Option<PullRequestTarget> {
    let (repo_part, number_part) = target.split_once('#')?;
    let number = number_part.parse::<u64>().ok()?;
    if number == 0 {
        return None;
    }
    let parts = repo_part
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let repository = match parts.as_slice() {
        [owner, repo] => {
            ForgeRepository::github(DEFAULT_GITHUB_HOST, *owner, strip_git_suffix(repo))
        }
        [host, owner, repo] => forge_repo_from_host(host, owner, strip_git_suffix(repo)),
        _ => return None,
    };

    Some(PullRequestTarget::with_repository(
        repository, number, target,
    ))
}

/// Build a `ForgeRepository` from host/owner/repo, picking the forge kind
/// based on the host name.
fn forge_repo_from_host(host: &str, owner: &str, repo: &str) -> ForgeRepository {
    if host.contains("gitlab") {
        ForgeRepository::gitlab(host, owner, repo)
    } else {
        ForgeRepository::github(host, owner, repo)
    }
}

/// Resolve an SSH host alias to its real `HostName` via `~/.ssh/config`.
/// Returns the alias unchanged when the config is missing, unreadable, or
/// contains no matching block.
///
/// Limitations: only exact `Host` patterns are matched. Wildcard (`*`),
/// negation (`!`), `Match`, and `Include` directives are not supported;
/// a `Host`/`HostName` pair that depends on any of those falls back to
/// the alias unchanged.
fn resolve_ssh_hostname(alias: &str) -> String {
    let Ok(home) = std::env::var("HOME") else {
        return alias.to_string();
    };
    let path = PathBuf::from(home).join(".ssh/config");
    let Ok(content) = fs::read_to_string(path) else {
        return alias.to_string();
    };
    resolve_ssh_hostname_from_config(alias, &content)
}

fn resolve_ssh_hostname_from_config(alias: &str, config: &str) -> String {
    let mut in_block = false;
    for raw in config.lines() {
        // Strip inline comments and surrounding whitespace.
        let line = raw.split_once('#').map_or(raw, |(before, _)| before).trim();
        if line.is_empty() {
            continue;
        }
        // SSH config separates keyword from value by whitespace or '='.
        let (key, value) = line
            .split_once(|c: char| c.is_whitespace() || c == '=')
            .unwrap_or((line, ""));
        let value = value
            .trim_start_matches(|c: char| c.is_whitespace() || c == '=')
            .trim();

        if key.eq_ignore_ascii_case("Host") {
            // Exact match only; wildcards and negation are intentionally unsupported.
            in_block = value.split_whitespace().any(|pat| pat == alias);
        } else if key.eq_ignore_ascii_case("Match") {
            // Match blocks aren't supported; exit any Host block we were in.
            in_block = false;
        } else if in_block && key.eq_ignore_ascii_case("HostName") {
            return value.to_string();
        }
    }
    alias.to_string()
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
        .map(|(_, host)| host)
        .unwrap_or(host_part);
    Some((host, path))
}

fn repository_from_path(host: &str, path: &str) -> Option<ForgeRepository> {
    let mut parts = path.split('/').filter(|part| !part.is_empty());
    let owner = parts.next()?;
    let repo = parts.next()?;
    Some(ForgeRepository::github(
        host,
        owner,
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

fn gh_repo_arg(repository: &ForgeRepository) -> String {
    if repository.host == DEFAULT_GITHUB_HOST {
        repository.slug()
    } else {
        format!("{}/{}", repository.host, repository.slug())
    }
}

fn map_gh_error(error: GhCommandError, host: &str) -> TuicrError {
    match error {
        GhCommandError::MissingGh => TuicrError::Forge(
            "GitHub integration requires `gh`.\nInstall GitHub CLI and run `gh auth login`."
                .to_string(),
        ),
        GhCommandError::Failed { stderr, .. } if looks_like_auth_failure(&stderr) => {
            TuicrError::Forge(format!(
                "GitHub authentication failed.\nRun `gh auth login` for {host}."
            ))
        }
        GhCommandError::Failed { stderr, status } => {
            let detail = if stderr.is_empty() {
                status
                    .map(|code| format!("gh exited with status {code}"))
                    .unwrap_or_else(|| "gh command failed".to_string())
            } else {
                stderr
            };
            TuicrError::Forge(format!("GitHub command failed: {detail}"))
        }
    }
}

fn looks_like_auth_failure(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    lower.contains("gh auth login")
        || lower.contains("not logged in")
        || lower.contains("not logged into")
        || lower.contains("authentication failed")
        || lower.contains("requires authentication")
}

fn looks_like_permission_failure(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    lower.contains("resource not accessible by integration")
        || lower.contains("must have pull request write")
        || lower.contains("http 403")
        || lower.contains("status: 403")
        || lower.contains("403 forbidden")
}

fn looks_like_pending_review_conflict(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    lower.contains("only have one pending review per pull request")
}

fn looks_like_unknown_commit(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    lower.contains("commitoid is not part of the pull request")
        || lower.contains("commit_id is not part of the pull request")
}

/// Error mapping specific to create-review. Permission failures get their
/// own message per the spec; pending-review conflicts and unknown-commit
/// 422s get actionable messages too. Other failures fall through to the
/// standard missing-gh / auth / generic mapping.
fn map_create_review_error(error: GhCommandError, host: &str) -> TuicrError {
    if let GhCommandError::Failed { stderr, .. } = &error {
        if looks_like_permission_failure(stderr) {
            return TuicrError::Forge(
                "Cannot submit review: GitHub token lacks pull request write permission."
                    .to_string(),
            );
        }
        if looks_like_pending_review_conflict(stderr) {
            return TuicrError::Forge(
                "You already have a pending review on this PR. Finish or discard it on GitHub, then try again."
                    .to_string(),
            );
        }
        if looks_like_unknown_commit(stderr) {
            return TuicrError::Forge(
                "GitHub rejected the review: the selected commit is not part of this PR (it may have been removed by a force-push). Reload with :e and try again."
                    .to_string(),
            );
        }
    }
    map_gh_error(error, host)
}

/// Parse the GitHub `pulls/<n>/reviews` response into our minimal shape.
/// GitHub returns more fields; we only extract what the App needs.
fn parse_create_review_response(output: &str) -> Result<GhCreateReviewResponse> {
    let value: serde_json::Value = serde_json::from_str(output)?;
    let id = value
        .get("id")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| TuicrError::Forge("Create-review response missing `id`".to_string()))?;
    let html_url = value
        .get("html_url")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let state = value
        .get("state")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Ok(GhCreateReviewResponse {
        id,
        html_url,
        state,
    })
}

fn malformed_target<T>(input: &str) -> Result<T> {
    Err(TuicrError::Forge(format!(
        "Malformed GitHub pull request target: `{input}`"
    )))
}

#[cfg(test)]
pub(crate) mod tests_fixture {
    pub const SIMPLE_PATCH: &str = r##"diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,3 +1,3 @@
 pub fn answer() -> u32 {
-    41
+    42
 }
"##;
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::forge::traits::ForgeBackend;

    const PR_LIST_JSON: &str = r##"
[
  {
    "number": 148,
    "title": "Add forge-backed PR review",
    "author": { "login": "alice" },
    "headRefName": "forge-review",
    "baseRefName": "main",
    "updatedAt": "2026-05-12T18:30:00Z",
    "url": "https://github.com/agavra/tuicr/pull/148",
    "state": "OPEN",
    "isDraft": false
  },
  {
    "number": 125,
    "title": "Support fetching and pushing reviews",
    "author": { "login": "YPares" },
    "headRefName": "reviews",
    "baseRefName": "main",
    "updatedAt": "2026-05-08T10:00:00Z",
    "url": "https://github.com/agavra/tuicr/pull/125",
    "state": "OPEN",
    "isDraft": true
  }
]
"##;

    const PR_VIEW_JSON: &str = r##"
{
  "number": 125,
  "title": "Support fetching and pushing reviews",
  "url": "https://github.com/agavra/tuicr/pull/125",
  "state": "OPEN",
  "isDraft": false,
  "author": { "login": "alice" },
  "headRefName": "reviews",
  "baseRefName": "main",
  "headRefOid": "abcdef1234567890",
  "baseRefOid": "1234567890abcdef",
  "body": "Review workflow",
  "updatedAt": "2026-05-12T18:30:00Z",
  "closed": false,
  "mergedAt": null
}
"##;

    const PR_PATCH: &str = r##"
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,3 +1,3 @@
 pub fn answer() -> u32 {
-    41
+    42
 }
"##;

    #[derive(Default)]
    struct FakeGhRunner {
        calls: RefCell<Vec<Vec<String>>>,
        /// Captured stdin payloads, paired in order with `calls` entries that
        /// went through `run_with_stdin`. Empty for plain `run` invocations.
        stdin_calls: RefCell<Vec<(Vec<String>, String)>>,
        /// When set, `run_with_stdin` returns this error instead of a stub
        /// success response. Lets tests exercise error mapping.
        stdin_error: RefCell<Option<GhCommandError>>,
        /// When set, `run_with_stdin` returns this body as the success output.
        /// Defaults to `CREATE_REVIEW_RESPONSE_JSON` when None.
        stdin_response: RefCell<Option<String>>,
    }

    impl GhCommandRunner for FakeGhRunner {
        fn run(&self, args: &[String]) -> GhCommandResult<String> {
            self.calls.borrow_mut().push(args.to_vec());
            match args.first().map(String::as_str) {
                // gh pr list/view/diff — second arg is the subcommand.
                Some("pr") => match args.get(1).map(String::as_str) {
                    Some("list") => Ok(PR_LIST_JSON.to_string()),
                    Some("view") => Ok(PR_VIEW_JSON.to_string()),
                    Some("diff") => Ok(PR_PATCH.to_string()),
                    _ => Err(GhCommandError::Failed {
                        status: Some(1),
                        stderr: "unexpected pr command".to_string(),
                    }),
                },
                // gh api graphql — dispatch by the query body so the
                // mock returns shape-appropriate fixtures for both the
                // reviewThreads and reviews queries.
                Some("api") if args.get(1).map(String::as_str) == Some("graphql") => {
                    let query = args
                        .iter()
                        .find(|a| a.starts_with("query="))
                        .map(String::as_str)
                        .unwrap_or("");
                    if query.contains("reviewThreads(") {
                        Ok(REVIEW_THREADS_JSON.to_string())
                    } else if query.contains("reviews(") {
                        Ok(REVIEW_SUMMARIES_JSON.to_string())
                    } else {
                        Err(GhCommandError::Failed {
                            status: Some(1),
                            stderr: "unexpected graphql query".to_string(),
                        })
                    }
                }
                // gh api repos/.../pulls/<n>/commits (commit list).
                Some("api")
                    if args
                        .iter()
                        .any(|a| a.contains("/pulls/") && a.contains("/commits")) =>
                {
                    Ok(PR_COMMITS_JSON.to_string())
                }
                // gh api repos/.../compare/<base>...<head> (range diff).
                Some("api") if args.iter().any(|a| a.contains("/compare/")) => {
                    Ok(COMPARE_DIFF.to_string())
                }
                _ => Err(GhCommandError::Failed {
                    status: Some(1),
                    stderr: "unexpected command".to_string(),
                }),
            }
        }

        fn run_with_stdin(&self, args: &[String], stdin: &str) -> GhCommandResult<String> {
            self.calls.borrow_mut().push(args.to_vec());
            self.stdin_calls
                .borrow_mut()
                .push((args.to_vec(), stdin.to_string()));
            if let Some(err) = self.stdin_error.borrow().clone() {
                return Err(err);
            }
            Ok(self
                .stdin_response
                .borrow()
                .clone()
                .unwrap_or_else(|| CREATE_REVIEW_RESPONSE_JSON.to_string()))
        }
    }

    const CREATE_REVIEW_RESPONSE_JSON: &str = r##"{
        "id": 123456,
        "html_url": "https://github.com/agavra/tuicr/pull/125#pullrequestreview-123456",
        "state": "COMMENTED"
    }"##;

    const PR_COMMITS_JSON: &str = r##"[
        {
            "sha": "aaaaaaa1111111111111111111111111111aaaa",
            "commit": {
                "message": "First commit\n\nbody text",
                "author": { "name": "Alice", "email": "a@x", "date": "2026-05-10T10:00:00Z" }
            }
        },
        {
            "sha": "bbbbbbb2222222222222222222222222222bbbb",
            "commit": {
                "message": "Second commit",
                "author": { "name": "Bob", "email": "b@x", "date": "2026-05-11T10:00:00Z" }
            }
        }
    ]"##;

    const COMPARE_DIFF: &str = r##"diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,3 +1,3 @@
 pub fn answer() -> u32 {
-    41
+    42
 }
"##;

    const REVIEW_THREADS_JSON: &str = r##"{
        "data": {
            "repository": {
                "pullRequest": {
                    "reviewThreads": {
                        "pageInfo": { "hasNextPage": false, "endCursor": null },
                        "nodes": [
                            {
                                "id": "PRRT_1",
                                "isResolved": false,
                                "isOutdated": false,
                                "path": "src/lib.rs",
                                "line": 42,
                                "diffSide": "RIGHT",
                                "comments": {
                                    "nodes": [
                                        {
                                            "id": "PRRC_1",
                                            "body": "remote one",
                                            "author": { "login": "alice" },
                                            "url": "https://example.com/1"
                                        }
                                    ]
                                }
                            }
                        ]
                    }
                }
            }
        }
    }"##;

    const REVIEW_SUMMARIES_JSON: &str = r##"{
        "data": {
            "repository": {
                "pullRequest": {
                    "reviews": {
                        "pageInfo": { "hasNextPage": false, "endCursor": null },
                        "nodes": [
                            {
                                "id": "PRR_1",
                                "state": "COMMENTED",
                                "body": "Overall LGTM",
                                "author": { "login": "alice" },
                                "submittedAt": "2026-05-12T18:30:00Z",
                                "url": "https://example.com/r/1"
                            }
                        ]
                    }
                }
            }
        }
    }"##;

    fn repo() -> ForgeRepository {
        ForgeRepository::github("github.com", "agavra", "tuicr")
    }

    #[test]
    fn repository_display_name_omits_github_com() {
        assert_eq!(repo().display_name(), "agavra/tuicr");
        assert_eq!(
            ForgeRepository::github("github.example.com", "agavra", "tuicr").display_name(),
            "github.example.com/agavra/tuicr"
        );
    }

    #[test]
    fn constructs_system_runner_backend() {
        let _backend = GitHubGhBackend::new(Some(repo()));
    }

    #[test]
    fn parses_numeric_pull_request_target() {
        let target = parse_pull_request_target("125").unwrap();
        assert_eq!(target.number, 125);
        assert_eq!(target.repository, None);
    }

    #[test]
    fn parses_owner_repo_pull_request_target() {
        let target = parse_pull_request_target("agavra/tuicr#125").unwrap();
        assert_eq!(target.number, 125);
        assert_eq!(target.repository.unwrap(), repo());
    }

    #[test]
    fn parses_enterprise_owner_repo_pull_request_target() {
        let target = parse_pull_request_target("github.example.com/agavra/tuicr#125").unwrap();
        let repository = target.repository.unwrap();
        assert_eq!(repository.host, "github.example.com");
        assert_eq!(repository.slug(), "agavra/tuicr");
    }

    #[test]
    fn parses_full_pull_request_url() {
        let target = parse_pull_request_target("https://github.com/agavra/tuicr/pull/125").unwrap();
        assert_eq!(target.number, 125);
        assert_eq!(target.repository.unwrap(), repo());
    }

    #[test]
    fn parses_enterprise_pull_url_as_github_even_when_host_contains_gitlab() {
        let target =
            parse_pull_request_target("https://gitlab.ghe.company.com/agavra/tuicr/pull/125")
                .unwrap();
        let repository = target.repository.unwrap();
        assert_eq!(target.number, 125);
        assert_eq!(repository.kind, crate::forge::traits::ForgeKind::GitHub);
        assert_eq!(repository.host, "gitlab.ghe.company.com");
        assert_eq!(repository.slug(), "agavra/tuicr");
    }

    #[test]
    fn rejects_malformed_pull_request_target() {
        let err = parse_pull_request_target("agavra/tuicr/pulls/125").unwrap_err();
        assert!(
            err.to_string()
                .contains("Malformed GitHub pull request target")
        );
    }

    #[test]
    fn parses_https_remote_url() {
        let repository = parse_github_remote_url("https://github.com/agavra/tuicr.git").unwrap();
        assert_eq!(repository, repo());
    }

    #[test]
    fn parses_scp_like_ssh_remote_url() {
        let repository = parse_github_remote_url("git@github.com:agavra/tuicr.git").unwrap();
        assert_eq!(repository, repo());
    }

    #[test]
    fn parses_ssh_remote_url() {
        let repository =
            parse_github_remote_url("ssh://git@github.example.com/agavra/tuicr.git").unwrap();
        assert_eq!(repository.host, "github.example.com");
        assert_eq!(repository.slug(), "agavra/tuicr");
    }

    #[test]
    fn resolve_ssh_hostname_resolves_alias_to_configured_hostname() {
        // (case name, alias, config, expected hostname)
        let cases: &[(&str, &str, &str, &str)] = &[
            (
                "basic alias",
                "github-work",
                "Host github-work\n    HostName github.com\n",
                "github.com",
            ),
            (
                "tab separator",
                "github-work",
                "Host\tgithub-work\n\tHostName\tgithub.com\n",
                "github.com",
            ),
            (
                "equals separator",
                "github-work",
                "Host=github-work\n    HostName=github.com\n",
                "github.com",
            ),
            (
                "case-insensitive keywords",
                "github-work",
                "host github-work\n    hostname github.com\n",
                "github.com",
            ),
            (
                "trailing comment stripped",
                "github-work",
                "Host github-work\n    HostName github.com # inline comment\n",
                "github.com",
            ),
            (
                "blank and comment lines ignored",
                "github-work",
                "\
# leading comment

Host github-work
    # nested comment
    HostName github.com
",
                "github.com",
            ),
            (
                "multi-pattern block: second pattern",
                "github-work",
                "\
Host github.com github-personal
    HostName github.com

Host github-work
    HostName github.mycompany.com
",
                "github.mycompany.com",
            ),
            (
                "multi-pattern block: first pattern",
                "github-personal",
                "\
Host github.com github-personal
    HostName github.com

Host github-work
    HostName github.mycompany.com
",
                "github.com",
            ),
            (
                "duplicate Host blocks: first wins",
                "github-work",
                "\
Host github-work
    HostName first.example.com

Host github-work
    HostName second.example.com
",
                "first.example.com",
            ),
        ];

        for (name, alias, config, expected) in cases {
            assert_eq!(
                resolve_ssh_hostname_from_config(alias, config),
                *expected,
                "case: {name}",
            );
        }
    }

    #[test]
    fn resolve_ssh_hostname_falls_back_to_alias_when_unresolved() {
        // (case name, alias, config). All cases expect the alias back unchanged.
        let cases: &[(&str, &str, &str)] = &[
            (
                "host not in config",
                "github-work",
                "Host github.com\n    HostName github.com\n",
            ),
            (
                "block has no HostName",
                "github-work",
                "Host github-work\n    User git\n    IdentitiesOnly yes\n",
            ),
            (
                "wildcard patterns unsupported",
                "foo.github.com",
                "\
Host *.github.com
    HostName fallback.example.com

Host *
    HostName catchall.example.com
",
            ),
            (
                "Match directive terminates Host block",
                "github-work",
                "\
Host github-work
Match host github-work
    HostName should-not-leak.example.com
",
            ),
        ];

        for (name, alias, config) in cases {
            assert_eq!(
                resolve_ssh_hostname_from_config(alias, config),
                *alias,
                "case: {name}",
            );
        }
    }

    #[test]
    fn parse_scp_url_with_ssh_alias_resolves_via_config() {
        // End-to-end check on the SCP-style path: alias in the URL gets
        // swapped for the configured HostName before repository construction.
        let config = "Host github-work\n    HostName github.com\n";
        let url = "git@github-work:example-org/example-repo.git";
        let trimmed = trim_url_suffix(url.trim());
        let (host, path) = parse_scp_like_remote(trimmed).unwrap();
        let resolved = resolve_ssh_hostname_from_config(host, config);
        let repo = repository_from_path(&resolved, path).unwrap();
        assert_eq!(repo.host, "github.com");
        assert_eq!(repo.slug(), "example-org/example-repo");
    }

    #[test]
    fn list_pull_requests_uses_gh_json_output() {
        let runner = FakeGhRunner::default();
        let backend = GitHubGhBackend::with_runner(None, runner);
        let result = backend
            .list_pull_requests(PullRequestListQuery::first_page(repo(), 1))
            .unwrap();

        assert_eq!(result.pull_requests.len(), 1);
        assert!(result.has_more);
        assert_eq!(result.total_loaded, 1);
        assert_eq!(result.pull_requests[0].number, 148);
        assert_eq!(result.pull_requests[0].author.as_deref(), Some("alice"));

        let calls = backend.runner.calls.borrow();
        assert_eq!(
            calls[0],
            vec![
                "pr",
                "list",
                "--repo",
                "agavra/tuicr",
                "--state",
                "open",
                "--limit",
                "2",
                "--json",
                PR_LIST_JSON_FIELDS,
            ]
        );
    }

    #[test]
    fn list_pull_requests_can_filter_to_review_requested() {
        let runner = FakeGhRunner::default();
        let backend = GitHubGhBackend::with_runner(None, runner);
        backend
            .list_pull_requests(PullRequestListQuery::first_page_with_scope(
                repo(),
                1,
                PullRequestListScope::ReviewRequested,
            ))
            .unwrap();

        let calls = backend.runner.calls.borrow();
        assert_eq!(
            calls[0],
            vec![
                "pr",
                "list",
                "--repo",
                "agavra/tuicr",
                "--state",
                "open",
                "--limit",
                "2",
                "--json",
                PR_LIST_JSON_FIELDS,
                "--search",
                "review-requested:@me",
            ]
        );
    }

    #[test]
    fn list_pull_requests_can_load_next_slice() {
        let runner = FakeGhRunner::default();
        let backend = GitHubGhBackend::with_runner(None, runner);
        let result = backend
            .list_pull_requests(PullRequestListQuery {
                repository: repo(),
                already_loaded: 1,
                page_size: 1,
                scope: PullRequestListScope::Open,
            })
            .unwrap();

        assert_eq!(result.pull_requests.len(), 1);
        assert!(!result.has_more);
        assert_eq!(result.total_loaded, 2);
        assert_eq!(result.pull_requests[0].number, 125);

        let calls = backend.runner.calls.borrow();
        assert_eq!(
            calls[0],
            vec![
                "pr",
                "list",
                "--repo",
                "agavra/tuicr",
                "--state",
                "open",
                "--limit",
                "3",
                "--json",
                PR_LIST_JSON_FIELDS,
            ]
        );
    }

    #[test]
    fn list_pull_requests_uses_host_qualified_repo_for_enterprise() {
        let runner = FakeGhRunner::default();
        let backend = GitHubGhBackend::with_runner(None, runner);
        let repository = ForgeRepository::github("github.example.com", "agavra", "tuicr");
        backend
            .list_pull_requests(PullRequestListQuery::first_page(repository, 1))
            .unwrap();

        let calls = backend.runner.calls.borrow();
        assert_eq!(calls[0][3], "github.example.com/agavra/tuicr");
    }

    #[test]
    fn get_pull_request_uses_default_repository_for_numeric_target() {
        let runner = FakeGhRunner::default();
        let backend = GitHubGhBackend::with_runner(Some(repo()), runner);
        let details = backend
            .get_pull_request(parse_pull_request_target("125").unwrap())
            .unwrap();

        assert_eq!(details.number, 125);
        assert_eq!(details.head_sha, "abcdef1234567890");
        assert!(!details.is_read_only());
    }

    #[test]
    fn get_pull_request_requires_repository() {
        let runner = FakeGhRunner::default();
        let backend = GitHubGhBackend::with_runner(None, runner);
        let err = backend
            .get_pull_request(parse_pull_request_target("125").unwrap())
            .unwrap_err();
        assert!(err.to_string().contains("does not include a repository"));
    }

    #[test]
    fn get_pull_request_diff_returns_patch_text() {
        let runner = FakeGhRunner::default();
        let backend = GitHubGhBackend::with_runner(Some(repo()), runner);
        let details = backend
            .get_pull_request(parse_pull_request_target("125").unwrap())
            .unwrap();
        let patch = backend.get_pull_request_diff(&details).unwrap();

        assert_eq!(patch, PR_PATCH);
    }

    #[test]
    fn should_not_pass_patch_flag_to_gh_pr_diff() {
        // Regression: `gh pr diff --patch` returns mbox-style per-commit
        // patches concatenated, which the diff parser turns into duplicate
        // DiffFile entries (one per commit-touching-the-same-file). We
        // want the cumulative diff. Lock the argv so this can't silently
        // regress.
        let runner = FakeGhRunner::default();
        let backend = GitHubGhBackend::with_runner(Some(repo()), runner);
        let details = backend
            .get_pull_request(parse_pull_request_target("125").unwrap())
            .unwrap();
        let _ = backend.get_pull_request_diff(&details).unwrap();

        let calls = backend.runner.calls.borrow();
        let diff_call = calls
            .iter()
            .find(|args| {
                args.first().map(String::as_str) == Some("pr")
                    && args.get(1).map(String::as_str) == Some("diff")
            })
            .expect("expected a `gh pr diff` call");
        assert!(
            !diff_call.iter().any(|a| a == "--patch"),
            "`gh pr diff` must NOT pass --patch (mbox output duplicates files); got {diff_call:?}"
        );
    }

    #[test]
    fn should_list_review_threads_via_graphql_api_call() {
        // given
        let runner = FakeGhRunner::default();
        let backend = GitHubGhBackend::with_runner(Some(repo()), runner);
        let details = backend
            .get_pull_request(parse_pull_request_target("125").unwrap())
            .unwrap();
        // when
        let threads = backend.list_review_threads(&details).unwrap();
        // then
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].id, "PRRT_1");
        assert_eq!(threads[0].path, "src/lib.rs");
        // and — the API call was placed against `gh api graphql` with the
        // owner/name/number parameters.
        let calls = backend.runner.calls.borrow();
        let graphql_call = calls
            .iter()
            .find(|args| args.first().map(String::as_str) == Some("api"))
            .expect("expected a gh api call");
        assert_eq!(graphql_call[1], "graphql");
        assert!(graphql_call.iter().any(|a| a == "owner=agavra"));
        assert!(graphql_call.iter().any(|a| a == "name=tuicr"));
        assert!(graphql_call.iter().any(|a| a == "number=125"));
    }

    #[test]
    fn should_list_review_summaries_via_graphql_api_call() {
        // given
        let runner = FakeGhRunner::default();
        let backend = GitHubGhBackend::with_runner(Some(repo()), runner);
        let details = backend
            .get_pull_request(parse_pull_request_target("125").unwrap())
            .unwrap();
        // when
        let summaries = backend.list_review_summaries(&details).unwrap();
        // then — the COMMENTED review with body is surfaced.
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].id, "PRR_1");
        assert_eq!(summaries[0].author.as_deref(), Some("alice"));
        assert_eq!(summaries[0].body, "Overall LGTM");
        // and — the call routes the `reviews(` query, not `reviewThreads(`.
        let calls = backend.runner.calls.borrow();
        let summaries_call = calls
            .iter()
            .find(|args| {
                args.first().map(String::as_str) == Some("api")
                    && args.iter().any(|a| a.contains("reviews(first:"))
            })
            .expect("expected a reviews graphql call");
        assert!(summaries_call.iter().any(|a| a == "owner=agavra"));
        assert!(summaries_call.iter().any(|a| a == "number=125"));
    }

    #[test]
    fn should_list_pull_request_commits_via_api_pagination_endpoint() {
        // given
        let runner = FakeGhRunner::default();
        let backend = GitHubGhBackend::with_runner(Some(repo()), runner);
        let details = backend
            .get_pull_request(parse_pull_request_target("125").unwrap())
            .unwrap();
        // when
        let commits = backend.list_pull_request_commits(&details).unwrap();
        // then — both commits parse with summary, short sha, and author.
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].oid, "aaaaaaa1111111111111111111111111111aaaa");
        assert_eq!(commits[0].short_oid, "aaaaaaa");
        assert_eq!(commits[0].summary, "First commit");
        assert_eq!(commits[0].author, "Alice");
        assert!(commits[0].timestamp.is_some());
        assert_eq!(commits[1].summary, "Second commit");
        // and — the call targeted the pulls/<n>/commits endpoint.
        let calls = backend.runner.calls.borrow();
        let commits_call = calls
            .iter()
            .find(|args| {
                args.iter()
                    .any(|a| a.contains("/pulls/125/commits") && a.contains("per_page=100"))
            })
            .expect("expected a pulls commits api call");
        assert_eq!(commits_call[0], "api");
    }

    #[test]
    fn should_request_compare_endpoint_for_range_diff_when_no_local_checkout() {
        // given
        let runner = FakeGhRunner::default();
        let backend = GitHubGhBackend::with_runner(Some(repo()), runner);
        let details = backend
            .get_pull_request(parse_pull_request_target("125").unwrap())
            .unwrap();
        // when
        let diff = backend
            .get_pull_request_commit_range_diff(&details, "baseaaa", "headbbb")
            .unwrap();
        // then
        assert!(diff.contains("diff --git a/src/lib.rs"));
        // and — the call hit the compare endpoint with the Accept diff header.
        let calls = backend.runner.calls.borrow();
        let compare_call = calls
            .iter()
            .find(|args| {
                args.iter()
                    .any(|a| a.contains("/compare/baseaaa...headbbb"))
            })
            .expect("expected a compare api call");
        assert!(compare_call.contains(&"Accept: application/vnd.github.diff".to_string()));
    }

    #[test]
    fn maps_missing_gh_to_install_message() {
        let err = map_gh_error(GhCommandError::MissingGh, "github.com");
        assert!(err.to_string().contains("GitHub integration requires `gh`"));
    }

    #[test]
    fn maps_auth_failure_to_login_message() {
        let err = map_gh_error(
            GhCommandError::Failed {
                status: Some(4),
                stderr: "run `gh auth login` to authenticate".to_string(),
            },
            "github.example.com",
        );
        assert!(err.to_string().contains("GitHub authentication failed"));
        assert!(err.to_string().contains("github.example.com"));
    }

    // create_review tests

    use crate::forge::submit::{GhSide, InlineComment, SubmitEvent};
    use crate::forge::traits::CreateReviewRequest;
    use std::path::PathBuf;

    fn inline(line: u32, body: &str) -> InlineComment {
        InlineComment {
            path: PathBuf::from("src/lib.rs"),
            line,
            side: GhSide::Right,
            counterpart_line: None,
            start_line: None,
            start_side: None,
            old_path: None,
            body: body.to_string(),
            comment_id: format!("cid-{line}"),
        }
    }

    #[test]
    fn should_post_create_review_to_reviews_endpoint_with_stdin_payload() {
        // given
        let runner = FakeGhRunner::default();
        let backend = GitHubGhBackend::with_runner(Some(repo()), runner);
        let details = backend
            .get_pull_request(parse_pull_request_target("125").unwrap())
            .unwrap();
        let comments = vec![inline(42, "[ISSUE] boom")];
        let request = CreateReviewRequest {
            event: SubmitEvent::Comment,
            commit_id: "abcdef1234567890",
            body: "review body",
            comments: &comments,
        };
        // when
        let response = backend.create_review(&details, request).unwrap();
        // then
        assert_eq!(response.id, 123456);
        assert_eq!(response.state, "COMMENTED");
        assert!(response.html_url.contains("pullrequestreview-123456"));

        // and — the call hit the expected endpoint with stdin
        let stdin_calls = backend.runner.stdin_calls.borrow();
        assert_eq!(stdin_calls.len(), 1);
        let (args, stdin) = &stdin_calls[0];
        assert_eq!(args[0], "api");
        assert_eq!(args[1], "repos/agavra/tuicr/pulls/125/reviews");
        assert!(args.iter().any(|a| a == "--method"));
        assert!(args.iter().any(|a| a == "POST"));
        assert!(args.iter().any(|a| a == "--input"));
        assert!(args.iter().any(|a| a == "-"));
        // and — the stdin carries the JSON payload with the expected fields
        let payload: serde_json::Value = serde_json::from_str(stdin).unwrap();
        assert_eq!(payload["commit_id"], "abcdef1234567890");
        assert_eq!(payload["body"], "review body");
        assert_eq!(payload["event"], "COMMENT");
        assert_eq!(payload["comments"][0]["line"], 42);
        assert_eq!(payload["comments"][0]["side"], "RIGHT");
        // and — comment_id stays out of the payload (it's internal state)
        assert!(
            payload["comments"][0].get("comment_id").is_none(),
            "comment_id should not leak into the JSON payload"
        );
    }

    #[test]
    fn should_omit_event_field_for_draft_submission() {
        // given
        let runner = FakeGhRunner::default();
        let backend = GitHubGhBackend::with_runner(Some(repo()), runner);
        let details = backend
            .get_pull_request(parse_pull_request_target("125").unwrap())
            .unwrap();
        // when — draft submission
        let _ = backend
            .create_review(
                &details,
                CreateReviewRequest {
                    event: SubmitEvent::Draft,
                    commit_id: "sha",
                    body: "",
                    comments: &[],
                },
            )
            .unwrap();
        // then — the payload omits `event`
        let stdin_calls = backend.runner.stdin_calls.borrow();
        let (_, stdin) = &stdin_calls[0];
        let payload: serde_json::Value = serde_json::from_str(stdin).unwrap();
        assert!(payload.get("event").is_none());
    }

    #[test]
    fn should_send_hostname_argument_for_enterprise_host() {
        // given — enterprise host
        let runner = FakeGhRunner::default();
        let enterprise = ForgeRepository::github("github.example.com", "agavra", "tuicr");
        let backend = GitHubGhBackend::with_runner(Some(enterprise.clone()), runner);
        let mut details = backend
            .get_pull_request(parse_pull_request_target("125").unwrap())
            .unwrap();
        details.repository = enterprise;
        // when
        let _ = backend
            .create_review(
                &details,
                CreateReviewRequest {
                    event: SubmitEvent::Comment,
                    commit_id: "sha",
                    body: "",
                    comments: &[],
                },
            )
            .unwrap();
        // then
        let stdin_calls = backend.runner.stdin_calls.borrow();
        let (args, _) = &stdin_calls[0];
        assert!(args.iter().any(|a| a == "--hostname"));
        assert!(args.iter().any(|a| a == "github.example.com"));
    }

    #[test]
    fn should_map_permission_failure_to_pull_request_write_message() {
        // given — a runner that fails with a 403-style stderr
        let runner = FakeGhRunner::default();
        *runner.stdin_error.borrow_mut() = Some(GhCommandError::Failed {
            status: Some(1),
            stderr: "HTTP 403: Resource not accessible by integration".to_string(),
        });
        let backend = GitHubGhBackend::with_runner(Some(repo()), runner);
        let details = backend
            .get_pull_request(parse_pull_request_target("125").unwrap())
            .unwrap();
        // when
        let err = backend
            .create_review(
                &details,
                CreateReviewRequest {
                    event: SubmitEvent::Comment,
                    commit_id: "sha",
                    body: "",
                    comments: &[],
                },
            )
            .unwrap_err();
        // then
        let msg = err.to_string();
        assert!(
            msg.contains("pull request write permission"),
            "got: {msg:?}"
        );
    }

    #[test]
    fn should_map_auth_failure_during_create_review() {
        // given — a runner that fails with auth-failure stderr
        let runner = FakeGhRunner::default();
        *runner.stdin_error.borrow_mut() = Some(GhCommandError::Failed {
            status: Some(4),
            stderr: "Run `gh auth login` to authenticate".to_string(),
        });
        let backend = GitHubGhBackend::with_runner(Some(repo()), runner);
        let details = backend
            .get_pull_request(parse_pull_request_target("125").unwrap())
            .unwrap();
        // when
        let err = backend
            .create_review(
                &details,
                CreateReviewRequest {
                    event: SubmitEvent::Comment,
                    commit_id: "sha",
                    body: "",
                    comments: &[],
                },
            )
            .unwrap_err();
        // then
        assert!(err.to_string().contains("GitHub authentication failed"));
    }

    #[test]
    fn should_map_missing_gh_during_create_review() {
        // given — gh is not installed
        let runner = FakeGhRunner::default();
        *runner.stdin_error.borrow_mut() = Some(GhCommandError::MissingGh);
        let backend = GitHubGhBackend::with_runner(Some(repo()), runner);
        let details = backend
            .get_pull_request(parse_pull_request_target("125").unwrap())
            .unwrap();
        // when
        let err = backend
            .create_review(
                &details,
                CreateReviewRequest {
                    event: SubmitEvent::Comment,
                    commit_id: "sha",
                    body: "",
                    comments: &[],
                },
            )
            .unwrap_err();
        // then
        assert!(err.to_string().contains("GitHub integration requires `gh`"));
    }

    #[test]
    fn should_error_when_create_review_response_is_missing_id() {
        // given — malformed response from gh
        let runner = FakeGhRunner::default();
        *runner.stdin_response.borrow_mut() =
            Some(r#"{"html_url": "x", "state": "COMMENTED"}"#.to_string());
        let backend = GitHubGhBackend::with_runner(Some(repo()), runner);
        let details = backend
            .get_pull_request(parse_pull_request_target("125").unwrap())
            .unwrap();
        // when
        let err = backend
            .create_review(
                &details,
                CreateReviewRequest {
                    event: SubmitEvent::Comment,
                    commit_id: "sha",
                    body: "",
                    comments: &[],
                },
            )
            .unwrap_err();
        // then
        assert!(err.to_string().contains("Create-review response missing"));
    }

    #[test]
    fn should_recognize_must_have_pull_request_write_as_permission_failure() {
        // given
        let runner = FakeGhRunner::default();
        *runner.stdin_error.borrow_mut() = Some(GhCommandError::Failed {
            status: Some(1),
            stderr: "must have pull request write permission".to_string(),
        });
        let backend = GitHubGhBackend::with_runner(Some(repo()), runner);
        let details = backend
            .get_pull_request(parse_pull_request_target("125").unwrap())
            .unwrap();
        // when
        let err = backend
            .create_review(
                &details,
                CreateReviewRequest {
                    event: SubmitEvent::Comment,
                    commit_id: "sha",
                    body: "",
                    comments: &[],
                },
            )
            .unwrap_err();
        // then
        assert!(err.to_string().contains("pull request write permission"));
    }

    #[test]
    fn should_map_pending_review_conflict_to_finish_or_discard_message() {
        // 422 from GitHub when a pending review already exists for this user
        // on this PR. We surface a clear actionable message.
        let runner = FakeGhRunner::default();
        *runner.stdin_error.borrow_mut() = Some(GhCommandError::Failed {
            status: Some(22),
            stderr: "gh: Unprocessable Entity (HTTP 422)\n\
                {\"message\":\"Unprocessable Entity\",\"errors\":\
                [\"User can only have one pending review per pull request\"]}"
                .to_string(),
        });
        let backend = GitHubGhBackend::with_runner(Some(repo()), runner);
        let details = backend
            .get_pull_request(parse_pull_request_target("125").unwrap())
            .unwrap();
        let err = backend
            .create_review(
                &details,
                CreateReviewRequest {
                    event: SubmitEvent::Comment,
                    commit_id: "sha",
                    body: "",
                    comments: &[],
                },
            )
            .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("pending review on this PR"),
            "expected pending-review hint, got: {msg:?}"
        );
        assert!(msg.contains("Finish or discard"), "got: {msg:?}");
    }

    #[test]
    fn should_map_unknown_commit_to_force_push_hint() {
        // 422 from GitHub when commit_id isn't part of the PR (e.g., the PR
        // was force-pushed since the session loaded).
        let runner = FakeGhRunner::default();
        *runner.stdin_error.borrow_mut() = Some(GhCommandError::Failed {
            status: Some(22),
            stderr: "gh: Unprocessable Entity (HTTP 422)\n\
                {\"message\":\"Unprocessable Entity\",\"errors\":\
                [\"The commitOID is not part of the pull request\"]}"
                .to_string(),
        });
        let backend = GitHubGhBackend::with_runner(Some(repo()), runner);
        let details = backend
            .get_pull_request(parse_pull_request_target("125").unwrap())
            .unwrap();
        let err = backend
            .create_review(
                &details,
                CreateReviewRequest {
                    event: SubmitEvent::Comment,
                    commit_id: "sha",
                    body: "",
                    comments: &[],
                },
            )
            .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("not part of this PR"),
            "expected unknown-commit hint, got: {msg:?}"
        );
        assert!(msg.contains(":e"), "got: {msg:?}");
    }
}
