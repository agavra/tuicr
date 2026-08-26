//! Gitea forge backend, driven by the `tea` CLI.
//!
//! Every call goes through `tea api`, which supplies authentication and
//! instance selection; tuicr stores no tokens of its own.
//!
//! Supported forge: Gitea. Gitea forks are deliberately not special-cased —
//! see [`is_gitea_host`].
//!
//! Two `tea` behaviors shape this module:
//!
//! 1. **`tea api` always exits 0.** It writes the response body to stdout for
//!    every HTTP status, so a 404 or 401 looks exactly like a 200 to the exit
//!    code. We pass `-i` and read the status line off stderr instead; see
//!    [`SystemTeaRunner::run`].
//! 2. **`limit` is capped server-side** by `MaxResponseItems` (default 50, and
//!    instance-configurable). Asking for 100 and stopping when fewer come back
//!    silently truncates, so pagination drives off `X-Total-Count` and falls
//!    back to reading until an empty page.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::de::DeserializeOwned;
use serde_json::json;

use crate::error::{Result, TuicrError};
use crate::forge::remote_comments::{
    RemoteCommentSide, RemoteReviewComment, RemoteReviewState, RemoteReviewSummary,
    RemoteReviewThread,
};
use crate::forge::submit::{GhSide, SubmitEvent};
use crate::forge::traits::{
    CreateReviewRequest, ForgeBackend, ForgeFileLinesRequest, ForgeRepository,
    GhCreateReviewResponse, PagedPullRequests, PullRequestCheckStatus, PullRequestCommit,
    PullRequestDetails, PullRequestInfo, PullRequestIssueComment, PullRequestListQuery,
    PullRequestListScope, PullRequestReviewMetadata, PullRequestReviewRecord,
    PullRequestReviewStatus, PullRequestSummary, PullRequestTarget,
};
use crate::model::{DiffLine, FilePatch, FileStatus};
use crate::process::{CommandOutputErrorKind, run_command_output, run_command_streams};
use crate::vcs::git::raw::{FileMetadata, pair_metadata_with_patch, run_git_diff};
use crate::vcs::slice_context_lines;

use super::models::{
    GiteaChangedFile, GiteaCombinedStatus, GiteaCommit, GiteaIssue, GiteaIssueComment,
    GiteaPullRequest, GiteaPullReview, GiteaPullReviewComment, GiteaUser, TeaLogin,
};

/// Page size requested from Gitea. The server clamps this to its own
/// `MaxResponseItems`, so it is an upper bound rather than a promise.
const API_PAGE_SIZE: usize = 50;

/// Defensive bound on pagination loops so a misbehaving instance cannot hang
/// the UI thread.
const MAX_PAGES: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TeaCommandError {
    MissingTea,
    Spawn(String),
    /// A completed request whose HTTP status was not 2xx. `body` is whatever
    /// the server sent, usually Gitea's `{"message": ..., "url": ...}`.
    Http {
        status: u16,
        body: String,
    },
}

pub type TeaCommandResult<T> = std::result::Result<T, TeaCommandError>;

/// A successful `tea api` response.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TeaResponse {
    pub body: String,
    /// `X-Total-Count`, when the endpoint is paginated and sent one.
    pub total_count: Option<usize>,
}

pub trait TeaCommandRunner {
    /// Run `tea` with `args`. `stdin` is the request body for write calls.
    fn run(&self, args: &[String], stdin: Option<&str>) -> TeaCommandResult<TeaResponse>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemTeaRunner;

impl TeaCommandRunner for SystemTeaRunner {
    fn run(&self, args: &[String], stdin: Option<&str>) -> TeaCommandResult<TeaResponse> {
        let streams = run_command_streams(
            "tea",
            None,
            args.iter().map(|arg| OsStr::new(arg.as_str())),
            stdin,
        )
        .map_err(|error| match error.kind {
            CommandOutputErrorKind::NotFound => TeaCommandError::MissingTea,
            _ => TeaCommandError::Spawn(error.stderr),
        })?;

        // `tea api -i` writes the status line and headers to stderr and the
        // body to stdout. A missing status line means tea failed before it
        // issued the request (bad login name, unreachable host), in which case
        // stderr carries tea's own diagnostic.
        let Some(status) = parse_http_status(&streams.stderr) else {
            return Err(TeaCommandError::Spawn(
                if streams.stderr.trim().is_empty() {
                    streams.stdout.trim().to_string()
                } else {
                    streams.stderr.trim().to_string()
                },
            ));
        };

        if !(200..300).contains(&status) {
            return Err(TeaCommandError::Http {
                status,
                body: streams.stdout.trim().to_string(),
            });
        }

        Ok(TeaResponse {
            body: streams.stdout,
            total_count: parse_header(&streams.stderr, "x-total-count")
                .and_then(|value| value.parse().ok()),
        })
    }
}

/// Pull the numeric status out of an HTTP status line (`HTTP/1.1 404 Not
/// Found`). Header blocks from redirects would repeat this; we take the last
/// one so the final response wins.
fn parse_http_status(headers: &str) -> Option<u16> {
    headers
        .lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("HTTP/")?;
            rest.split_whitespace().nth(1)?.parse::<u16>().ok()
        })
        .next_back()
}

fn parse_header<'a>(headers: &'a str, name: &str) -> Option<&'a str> {
    headers.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.trim()
            .eq_ignore_ascii_case(name)
            .then(|| value.trim())
            .filter(|value| !value.is_empty())
    })
}

fn map_tea_error(error: TeaCommandError, host: &str) -> TuicrError {
    match error {
        TeaCommandError::MissingTea => TuicrError::Forge(
            "`tea` CLI not found. Install it from https://gitea.com/gitea/tea and run \
             `tea logins add` for your instance."
                .to_string(),
        ),
        TeaCommandError::Spawn(detail) => TuicrError::Forge(format!(
            "tea failed for Gitea host {host}: {}",
            detail.trim()
        )),
        TeaCommandError::Http { status, body } => {
            let detail = gitea_error_message(&body);
            match status {
                401 => TuicrError::Forge(format!(
                    "Not authenticated to Gitea host {host}. Run `tea logins add` \
                     (or refresh the token) and try again."
                )),
                403 => TuicrError::Forge(format!(
                    "Gitea host {host} refused the request ({detail}). The token may lack \
                     repository or pull-request scope."
                )),
                404 => TuicrError::Forge(format!(
                    "Gitea host {host} returned not found ({detail}). Check the repository \
                     slug and that the token can see it."
                )),
                422 => TuicrError::Forge(format!("Gitea rejected the request: {detail}")),
                _ => TuicrError::Forge(format!(
                    "Gitea host {host} returned HTTP {status}: {detail}"
                )),
            }
        }
    }
}

/// Gitea error bodies are `{"message": "...", "url": "..."}`. Fall back to the
/// raw body when it is not that shape.
fn gitea_error_message(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("message")
                .and_then(|message| message.as_str())
                .map(str::to_string)
        })
        .filter(|message| !message.is_empty())
        .unwrap_or_else(|| {
            let trimmed = body.trim();
            if trimmed.is_empty() {
                "no response body".to_string()
            } else {
                trimmed.to_string()
            }
        })
}

// ----- Host discovery -----

/// Cached `tea logins list --output json`, as `(host, login-name)` pairs.
///
/// Remote detection runs this against every remote of every repo tuicr opens,
/// and the answer cannot change within a process lifetime in any way we care
/// about, so it is resolved once.
fn tea_logins() -> &'static [TeaLoginHost] {
    static LOGINS: OnceLock<Vec<TeaLoginHost>> = OnceLock::new();
    LOGINS.get_or_init(|| {
        let Ok(output) = run_command_output("tea", None, ["logins", "list", "--output", "json"])
        else {
            return Vec::new();
        };
        let Ok(logins) = serde_json::from_str::<Vec<TeaLogin>>(&output) else {
            return Vec::new();
        };
        logins
            .into_iter()
            .filter_map(|login| {
                let host = host_of_url(&login.url)?;
                (!login.name.is_empty()).then_some(TeaLoginHost {
                    host,
                    is_default: login.is_default(),
                    name: login.name,
                })
            })
            .collect()
    })
}

/// A `tea` login reduced to what remote matching needs.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TeaLoginHost {
    host: String,
    name: String,
    is_default: bool,
}

fn host_of_url(url: &str) -> Option<String> {
    let without_scheme = strip_scheme(url).unwrap_or(url);
    let host = without_scheme.split('/').next()?.trim();
    (!host.is_empty()).then(|| strip_port(host).to_ascii_lowercase())
}

/// The `tea` login name configured for `host`, if any.
pub fn tea_login_for_host(host: &str) -> Option<&'static str> {
    select_login(tea_logins(), host)
}

/// Pick the login to use for `host`.
///
/// Nothing stops a user from having two accounts on the same instance. Taking
/// whichever `tea` happened to list first would make the choice depend on
/// config file order, so the one marked default wins and first-listed is only
/// the tiebreak.
fn select_login<'a>(logins: &'a [TeaLoginHost], host: &str) -> Option<&'a str> {
    let mut matching = logins
        .iter()
        .filter(|login| login.host.eq_ignore_ascii_case(host))
        .peekable();
    let first = matching.peek().copied()?;
    matching
        .find(|login| login.is_default)
        .or(Some(first))
        .map(|login| login.name.as_str())
}

/// Whether `host` should route to the Gitea backend.
///
/// Self-hosted instances have arbitrary hostnames, so a name check alone
/// cannot identify them — a configured `tea` login is the reliable signal,
/// mirroring how the GitLab backend trusts `glab config get host`. The name
/// check in front of it keeps `gitea.com` and the common `gitea.*` deployments
/// working before the user has run `tea logins add`.
///
/// Gitea forks are not matched by name on purpose. Forgejo and Codeberg serve
/// a compatible API today and will route here if the user has a `tea` login
/// for them, but recognizing them by hostname would advertise support tuicr
/// does not test and cannot promise as those projects diverge.
pub fn is_gitea_host(host: &str) -> bool {
    let lower = host.to_ascii_lowercase();
    if lower.contains("gitea") {
        return true;
    }
    // Consulting `tea` means spawning it. Skip that for the hosts we already
    // know belong to another forge, so a GitHub-only user never pays for a
    // subprocess they have no use for.
    if matches!(
        lower.as_str(),
        "github.com" | "gitlab.com" | "bitbucket.org" | "dev.azure.com"
    ) {
        return false;
    }
    tea_login_for_host(host).is_some()
}

// ----- Remote / target parsing -----

/// Parse a git remote URL into a Gitea repository, or `None` when the host is
/// not a Gitea instance.
pub fn parse_gitea_remote_url(remote_url: &str) -> Option<ForgeRepository> {
    let trimmed = trim_url_suffix(remote_url.trim());
    if trimmed.is_empty() {
        return None;
    }

    if let Some((host, path)) = parse_scp_like_remote(trimmed) {
        // Check the literal host before resolving the SSH alias: a config that
        // maps a recognizable name onto an opaque bastion would otherwise stop
        // looking like Gitea. Mirrors the GitLab backend's ordering.
        let resolved = resolve_ssh_hostname(host);
        let gitea_host = if is_gitea_host(host) {
            host
        } else if is_gitea_host(&resolved) {
            &resolved
        } else {
            return None;
        };
        return gitea_repository_from_path(gitea_host, path);
    }

    let without_scheme = strip_scheme(trimmed).unwrap_or(trimmed);
    let without_user = without_scheme
        .rsplit_once('@')
        .map(|(_, rest)| rest)
        .unwrap_or(without_scheme);
    let (host, path) = without_user.split_once('/')?;
    let host = strip_port(host);
    if !is_gitea_host(host) {
        return None;
    }
    gitea_repository_from_path(host, path)
}

/// Gitea repositories are always exactly `<owner>/<repo>` — no nested groups,
/// unlike GitLab.
fn gitea_repository_from_path(host: &str, path: &str) -> Option<ForgeRepository> {
    let mut parts = path.split('/').filter(|part| !part.is_empty());
    let owner = parts.next()?;
    let repo = parts.next()?;
    Some(ForgeRepository::gitea(
        host,
        owner,
        strip_git_suffix(trim_url_suffix(repo)),
    ))
}

/// Parse a `tuicr pr` target that names a Gitea pull request.
///
/// Handles the two forms that carry their own repository: a browser URL
/// (`https://host/owner/repo/pulls/123`) and the host-qualified shorthand
/// (`host/owner/repo#123`). Bare numbers are left to the GitHub parser, which
/// runs first and produces a repository-less target that the caller then
/// resolves against the detected remote.
pub fn parse_pull_request_target_gitea(input: &str) -> Result<PullRequestTarget> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return malformed_target(input);
    }
    if let Some(target) = parse_gitea_url_target(trimmed) {
        return Ok(target);
    }
    if let Some(target) = parse_gitea_repo_hash_target(trimmed) {
        return Ok(target);
    }
    malformed_target(input)
}

/// `https://host/owner/repo/pulls/<n>`. Note the plural `pulls` — GitHub uses
/// the singular `pull`, so the two URL shapes never collide.
fn parse_gitea_url_target(target: &str) -> Option<PullRequestTarget> {
    let trimmed = trim_url_suffix(target);
    let without_scheme = strip_scheme(trimmed)?;
    let mut parts = without_scheme.split('/').filter(|part| !part.is_empty());
    let host = strip_port(parts.next()?);
    let owner = parts.next()?;
    let repo = parts.next()?;
    if parts.next()? != "pulls" {
        return None;
    }
    let number = parts.next()?.parse::<u64>().ok()?;
    if number == 0 {
        return None;
    }
    if !is_gitea_host(host) {
        return None;
    }
    Some(PullRequestTarget::with_repository(
        ForgeRepository::gitea(host, owner, strip_git_suffix(repo)),
        number,
        target,
    ))
}

/// `host/owner/repo#<n>` — the bare `owner/repo#<n>` form is ambiguous across
/// forges and stays with GitHub.
fn parse_gitea_repo_hash_target(target: &str) -> Option<PullRequestTarget> {
    let (repo_part, number_part) = target.split_once('#')?;
    let number = number_part.parse::<u64>().ok()?;
    if number == 0 {
        return None;
    }
    let parts: Vec<&str> = repo_part
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    let [host, owner, repo] = parts.as_slice() else {
        return None;
    };
    let host = strip_port(host);
    if !is_gitea_host(host) {
        return None;
    }
    Some(PullRequestTarget::with_repository(
        ForgeRepository::gitea(host, *owner, strip_git_suffix(repo)),
        number,
        target,
    ))
}

fn malformed_target<T>(input: &str) -> Result<T> {
    Err(TuicrError::InvalidInput(format!(
        "'{input}' is not a recognized Gitea pull request target. Expected a number, \
         host/owner/repo#N, or a pull request URL."
    )))
}

// ----- Backend -----

#[derive(Debug, Clone)]
pub struct GiteaTeaBackend<R = SystemTeaRunner> {
    default_repository: Option<ForgeRepository>,
    local_checkout: Option<PathBuf>,
    /// Fetch CI statuses for the PR description panel. Off by default: it is
    /// an extra request per PR open and the panel hides the section anyway.
    show_checks: bool,
    /// Fetch top-level conversation comments for the PR description panel.
    show_comments: bool,
    runner: R,
}

impl GiteaTeaBackend<SystemTeaRunner> {
    pub fn new(default_repository: Option<ForgeRepository>) -> Self {
        Self {
            default_repository,
            local_checkout: None,
            show_checks: false,
            show_comments: true,
            runner: SystemTeaRunner,
        }
    }
}

impl<R> GiteaTeaBackend<R> {
    pub fn with_local_checkout(mut self, checkout: Option<PathBuf>) -> Self {
        self.local_checkout = checkout;
        self
    }

    pub fn with_pr_checks(mut self, show_checks: bool) -> Self {
        self.show_checks = show_checks;
        self
    }

    pub fn with_pr_comments(mut self, show_comments: bool) -> Self {
        self.show_comments = show_comments;
        self
    }

    pub fn with_runner(default_repository: Option<ForgeRepository>, runner: R) -> Self {
        Self {
            default_repository,
            local_checkout: None,
            show_checks: false,
            show_comments: true,
            runner,
        }
    }
}

impl<R> GiteaTeaBackend<R>
where
    R: TeaCommandRunner,
{
    fn resolve_repository(&self, target: &PullRequestTarget) -> Result<ForgeRepository> {
        target
            .repository
            .clone()
            .or_else(|| self.default_repository.clone())
            .ok_or_else(|| {
                TuicrError::Forge(
                    "No Gitea repository configured for this pull request target".to_string(),
                )
            })
    }

    /// Issue one `tea api` call against `repo`'s instance.
    ///
    /// `-i` is mandatory: without it there is no way to distinguish a
    /// successful response from an error body, because `tea api` exits 0
    /// either way.
    fn api(
        &self,
        repo: &ForgeRepository,
        method: &str,
        endpoint: &str,
        body: Option<&str>,
    ) -> Result<TeaResponse> {
        let mut args = vec!["api".to_string(), "-i".to_string()];
        if let Some(login) = tea_login_for_host(&repo.host) {
            args.extend(["--login".to_string(), login.to_string()]);
        }
        if method != "GET" {
            args.extend(["--method".to_string(), method.to_string()]);
        }
        if body.is_some() {
            args.extend(["--data".to_string(), "@-".to_string()]);
        }
        args.push(endpoint.to_string());

        self.runner
            .run(&args, body)
            .map_err(|error| map_tea_error(error, &repo.host))
    }

    fn get_json<T: DeserializeOwned>(&self, repo: &ForgeRepository, endpoint: &str) -> Result<T> {
        let response = self.api(repo, "GET", endpoint, None)?;
        Ok(serde_json::from_str(&response.body)?)
    }

    /// Read every page of a paginated collection.
    ///
    /// `endpoint` must already carry any query parameters other than `page`
    /// and `limit`. Stops on `X-Total-Count` when the server sends it, and on
    /// a short or empty page otherwise — never on "fewer than requested",
    /// which is wrong whenever the instance's `MaxResponseItems` is below
    /// [`API_PAGE_SIZE`].
    fn get_all_pages<T: DeserializeOwned>(
        &self,
        repo: &ForgeRepository,
        endpoint: &str,
    ) -> Result<Vec<T>> {
        let separator = if endpoint.contains('?') { '&' } else { '?' };
        let mut collected: Vec<T> = Vec::new();
        let mut page_size: Option<usize> = None;

        for page in 1..=MAX_PAGES {
            let paged = format!("{endpoint}{separator}page={page}&limit={API_PAGE_SIZE}");
            let response = self.api(repo, "GET", &paged, None)?;
            let rows: Vec<T> = serde_json::from_str(&response.body)?;
            let received = rows.len();
            collected.extend(rows);

            if received == 0 {
                break;
            }
            if let Some(total) = response.total_count
                && collected.len() >= total
            {
                break;
            }
            // Without a total, infer the server's real page size from the
            // first full page and stop once a page comes up short.
            let effective = *page_size.get_or_insert(received);
            if received < effective {
                break;
            }
        }

        Ok(collected)
    }

    fn pulls_endpoint(repo: &ForgeRepository, number: u64, suffix: &str) -> String {
        format!(
            "/repos/{}/{}/pulls/{}{}",
            repo.owner, repo.name, number, suffix
        )
    }

    fn fetch_pull_request(
        &self,
        repo: &ForgeRepository,
        number: u64,
    ) -> Result<PullRequestDetails> {
        let pr: GiteaPullRequest = self.get_json(repo, &Self::pulls_endpoint(repo, number, ""))?;
        if pr.head.sha.is_empty() || pr.base.sha.is_empty() {
            return Err(TuicrError::Forge(format!(
                "Gitea pull request #{number} did not report head and base commit SHAs; \
                 tuicr cannot anchor a review session without them."
            )));
        }
        Ok(into_details(pr, repo))
    }

    /// File metadata for the PR, in the same order as the `.diff` body.
    ///
    /// Both come from one `gitdiff` computation server-side, so positional
    /// pairing is sound — but only if every page is read, hence
    /// [`Self::get_all_pages`].
    fn file_metadata(&self, pr: &PullRequestDetails) -> Result<Vec<FileMetadata>> {
        let rows: Vec<GiteaChangedFile> = self.get_all_pages(
            &pr.repository,
            &Self::pulls_endpoint(&pr.repository, pr.number, "/files"),
        )?;
        rows.into_iter().map(into_file_metadata).collect()
    }

    fn raw_file(&self, repo: &ForgeRepository, sha: &str, path: &Path) -> Result<String> {
        let endpoint = format!(
            "/repos/{}/{}/raw/{}?ref={}",
            repo.owner,
            repo.name,
            percent_encode_path(&path.to_string_lossy()),
            percent_encode_component(sha),
        );
        Ok(self.api(repo, "GET", &endpoint, None)?.body)
    }

    /// File content at `request`'s revision, preferring a local clone.
    fn file_content(&self, request: &ForgeFileLinesRequest) -> Result<String> {
        if let Some(content) = self
            .local_checkout
            .as_deref()
            .and_then(|root| read_blob_with_repo(root, request.sha(), request.path.as_path()))
        {
            return Ok(content);
        }
        self.raw_file(&request.repository, request.sha(), request.path.as_path())
    }

    fn review_comments(
        &self,
        pr: &PullRequestDetails,
        review_id: u64,
    ) -> Result<Vec<GiteaPullReviewComment>> {
        self.get_json(
            &pr.repository,
            &Self::pulls_endpoint(
                &pr.repository,
                pr.number,
                &format!("/reviews/{review_id}/comments"),
            ),
        )
    }

    fn reviews(&self, pr: &PullRequestDetails) -> Result<Vec<GiteaPullReview>> {
        self.get_all_pages(
            &pr.repository,
            &Self::pulls_endpoint(&pr.repository, pr.number, "/reviews"),
        )
    }

    /// Login of the account `tea` is authenticated as, used to attribute the
    /// viewer's own reviews. Best-effort: a failure downgrades the
    /// commits-since-my-last-review hint rather than failing the PR open.
    fn viewer_login(&self, repo: &ForgeRepository) -> Option<String> {
        let user: GiteaUser = self.get_json(repo, "/user").ok()?;
        GiteaUser::login(Some(user))
    }

    fn list_open_pull_requests(&self, query: &PullRequestListQuery) -> Result<PagedPullRequests> {
        let page_size = query.page_size.max(1);
        let repo = &query.repository;
        // `already_loaded` only ever advances by whole pages, so it maps
        // cleanly onto Gitea's 1-based page numbers.
        let page = query.already_loaded / page_size + 1;
        let endpoint = format!(
            "/repos/{}/{}/pulls?state=open&page={}&limit={}",
            repo.owner, repo.name, page, page_size
        );
        let response = self.api(repo, "GET", &endpoint, None)?;
        let rows: Vec<GiteaPullRequest> = serde_json::from_str(&response.body)?;
        let received = rows.len();
        let pull_requests = rows
            .into_iter()
            .map(|row| into_summary(row, repo))
            .collect::<Vec<_>>();
        let total_loaded = query.already_loaded + pull_requests.len();
        let has_more = match response.total_count {
            Some(total) => total_loaded < total,
            None => received == page_size,
        };
        Ok(PagedPullRequests {
            pull_requests,
            has_more,
            total_loaded,
        })
    }

    /// Review-requested scope.
    ///
    /// Gitea exposes this filter only on the cross-repository
    /// `/repos/issues/search` endpoint, which returns the *issue* shape and
    /// cannot be narrowed to a single repository. We pull the (small) result
    /// set, keep the rows belonging to this repo, and page locally. Summaries
    /// built this way have no head/base branch names — the endpoint does not
    /// carry them — which only affects branch-name filtering in the selector.
    fn list_review_requested(&self, query: &PullRequestListQuery) -> Result<PagedPullRequests> {
        let page_size = query.page_size.max(1);
        let repo = &query.repository;
        let endpoint = format!(
            "/repos/issues/search?type=pulls&state=open&review_requested=true&owner={}",
            percent_encode_component(&repo.owner),
        );
        let rows: Vec<GiteaIssue> = self.get_all_pages(repo, &endpoint)?;
        let slug = repo.slug();
        let matching = rows
            .into_iter()
            .filter(|row| {
                row.repository
                    .as_ref()
                    .is_some_and(|meta| meta.full_name.eq_ignore_ascii_case(&slug))
            })
            .map(|row| issue_into_summary(row, repo))
            .collect::<Vec<_>>();

        let has_more = matching.len() > query.already_loaded + page_size;
        let pull_requests = matching
            .into_iter()
            .skip(query.already_loaded)
            .take(page_size)
            .collect::<Vec<_>>();
        let total_loaded = query.already_loaded + pull_requests.len();
        Ok(PagedPullRequests {
            pull_requests,
            has_more,
            total_loaded,
        })
    }
}

impl<R> ForgeBackend for GiteaTeaBackend<R>
where
    R: TeaCommandRunner,
{
    fn list_pull_requests(&self, query: PullRequestListQuery) -> Result<PagedPullRequests> {
        match query.scope {
            PullRequestListScope::Open => self.list_open_pull_requests(&query),
            PullRequestListScope::ReviewRequested => self.list_review_requested(&query),
        }
    }

    fn get_pull_request(&self, target: PullRequestTarget) -> Result<PullRequestDetails> {
        let repository = self.resolve_repository(&target)?;
        self.fetch_pull_request(&repository, target.number)
    }

    fn get_pull_request_info(&self, target: PullRequestTarget) -> Result<PullRequestInfo> {
        let repository = self.resolve_repository(&target)?;
        let details = self.fetch_pull_request(&repository, target.number)?;

        let raw: GiteaPullRequest = self.get_json(
            &repository,
            &Self::pulls_endpoint(&repository, target.number, ""),
        )?;
        let requested_reviewers = raw
            .requested_reviewers
            .unwrap_or_default()
            .into_iter()
            .filter_map(|user| GiteaUser::login(Some(user)))
            .collect::<Vec<_>>();
        let mergeable = raw
            .mergeable
            .map(|ok| if ok { "MERGEABLE" } else { "CONFLICTING" }.to_string());

        // Latest response per reviewer, newest wins. Pending reviews are the
        // viewer's own unsubmitted drafts and review-requests are not
        // responses, so neither belongs in the summary row.
        let mut latest: BTreeMap<String, PullRequestReviewStatus> = BTreeMap::new();
        for review in self.reviews(&details).unwrap_or_default() {
            let state = match review.state.to_ascii_uppercase().as_str() {
                "APPROVED" => "APPROVED",
                "REQUEST_CHANGES" => "CHANGES_REQUESTED",
                "COMMENT" => "COMMENTED",
                _ => continue,
            };
            let Some(author) = GiteaUser::login(review.user) else {
                continue;
            };
            let candidate = PullRequestReviewStatus {
                author: Some(author.clone()),
                state: state.to_string(),
                submitted_at: review.submitted_at,
            };
            latest
                .entry(author)
                .and_modify(|existing| {
                    if candidate.submitted_at >= existing.submitted_at {
                        *existing = candidate.clone();
                    }
                })
                .or_insert(candidate);
        }
        let latest_reviews = latest.into_values().collect::<Vec<_>>();

        let review_decision = if latest_reviews
            .iter()
            .any(|review| review.state == "CHANGES_REQUESTED")
        {
            Some("CHANGES_REQUESTED".to_string())
        } else if latest_reviews
            .iter()
            .any(|review| review.state == "APPROVED")
        {
            Some("APPROVED".to_string())
        } else {
            None
        };

        let checks = if self.show_checks {
            self.get_json::<GiteaCombinedStatus>(
                &repository,
                &format!(
                    "/repos/{}/{}/commits/{}/status",
                    repository.owner, repository.name, details.head_sha
                ),
            )
            .map(|combined| {
                combined
                    .statuses
                    .into_iter()
                    .map(|status| PullRequestCheckStatus {
                        name: status.context,
                        // Gitea has a single state per status rather than
                        // GitHub's status/conclusion pair. Reporting only the
                        // conclusion keeps the rendered label from repeating
                        // itself.
                        status: None,
                        conclusion: Some(status.status.to_ascii_uppercase()),
                        url: status.target_url.filter(|url| !url.is_empty()),
                    })
                    .collect()
            })
            .unwrap_or_default()
        } else {
            Vec::new()
        };

        let issue_comments = if self.show_comments {
            self.get_all_pages::<GiteaIssueComment>(
                &repository,
                &format!(
                    "/repos/{}/{}/issues/{}/comments",
                    repository.owner, repository.name, details.number
                ),
            )
            .unwrap_or_default()
            .into_iter()
            .map(|comment| PullRequestIssueComment {
                author: GiteaUser::login(comment.user),
                body: comment.body,
                url: comment.html_url,
                created_at: comment.created_at,
            })
            .collect()
        } else {
            Vec::new()
        };

        Ok(PullRequestInfo {
            details,
            review_decision,
            mergeable,
            merge_state: None,
            requested_reviewers,
            latest_reviews,
            checks,
            issue_comments,
        })
    }

    fn get_pull_request_diff(&self, pr: &PullRequestDetails) -> Result<Vec<FilePatch>> {
        let metadata = self.file_metadata(pr)?;
        let patch = self
            .api(
                &pr.repository,
                "GET",
                &Self::pulls_endpoint(&pr.repository, pr.number, ".diff"),
                None,
            )?
            .body;
        pair_metadata_with_patch(metadata, patch.as_bytes())
    }

    fn local_checkout_path(&self) -> Option<PathBuf> {
        self.local_checkout.clone()
    }

    fn list_pull_request_commits(&self, pr: &PullRequestDetails) -> Result<Vec<PullRequestCommit>> {
        let commits: Vec<GiteaCommit> = self.get_all_pages(
            &pr.repository,
            &Self::pulls_endpoint(&pr.repository, pr.number, "/commits"),
        )?;
        // Gitea walks the branch backwards from the head, so both the pages
        // and the rows within them arrive newest-first. The trait asks for
        // chronological order, and commit-range scoping depends on it: a
        // reversed list makes `start_sha` the later commit and the compare
        // endpoint answers with an empty diff.
        Ok(commits
            .into_iter()
            .rev()
            .map(into_pull_request_commit)
            .collect())
    }

    fn list_pull_request_review_metadata(
        &self,
        pr: &PullRequestDetails,
    ) -> Result<PullRequestReviewMetadata> {
        let reviews = self
            .reviews(pr)?
            .into_iter()
            // A pending review has not been submitted, so it cannot mark a
            // point the viewer has already reviewed up to.
            .filter(|review| !review.state.eq_ignore_ascii_case("PENDING"))
            .map(|review| PullRequestReviewRecord {
                author: GiteaUser::login(review.user),
                submitted_at: review.submitted_at,
                commit_oid: (!review.commit_id.is_empty()).then_some(review.commit_id),
            })
            .collect();
        Ok(PullRequestReviewMetadata {
            viewer_login: self.viewer_login(&pr.repository),
            reviews,
        })
    }

    fn get_pull_request_commit_range_diff(
        &self,
        pr: &PullRequestDetails,
        start_sha: &str,
        end_sha: &str,
    ) -> Result<Vec<FilePatch>> {
        // Fast path: both SHAs already local means `git diff` answers without
        // a round trip. The forge stays the source of truth — it just produces
        // the same bytes for the same two commits.
        if let Some(root) = self.local_checkout.as_deref()
            && let Some(diff) = local_range_diff(root, start_sha, end_sha)
        {
            return Ok(diff);
        }

        // Gitea's compare endpoint returns only commit metadata as JSON, but
        // `output=diff` switches it to a raw unified diff. There is no
        // matching structured file list, so metadata is recovered from the
        // diff text itself.
        let endpoint = format!(
            "/repos/{}/{}/compare/{}...{}?output=diff",
            pr.repository.owner, pr.repository.name, start_sha, end_sha
        );
        let patch = self.api(&pr.repository, "GET", &endpoint, None)?.body;
        parse_unified_diff(&patch)
    }

    fn list_review_threads(&self, pr: &PullRequestDetails) -> Result<Vec<RemoteReviewThread>> {
        let mut comments: Vec<GiteaPullReviewComment> = Vec::new();
        for review in self.reviews(pr)? {
            if review.comments_count == 0 {
                continue;
            }
            comments.extend(self.review_comments(pr, review.id)?);
        }
        // Posted order across reviews; ties keep the id order the server used.
        comments.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
        Ok(group_comments_into_threads(comments))
    }

    fn list_review_summaries(&self, pr: &PullRequestDetails) -> Result<Vec<RemoteReviewSummary>> {
        Ok(self
            .reviews(pr)?
            .into_iter()
            // Bare approvals carry no prose; the review-summary area only
            // renders bodies.
            .filter(|review| !review.body.trim().is_empty())
            .filter(|review| !review.state.eq_ignore_ascii_case("PENDING"))
            .map(|review| RemoteReviewSummary {
                id: review.id.to_string(),
                author: GiteaUser::login(review.user),
                body: review.body,
                state: parse_review_state(&review.state),
                created_at: review.submitted_at,
                url: review.html_url,
            })
            .collect())
    }

    fn fetch_file_lines(&self, request: ForgeFileLinesRequest) -> Result<Vec<DiffLine>> {
        if request.start_line == 0 || request.start_line > request.end_line {
            return Ok(Vec::new());
        }
        let content = self.file_content(&request)?;
        Ok(slice_context_lines(
            &content,
            request.start_line,
            request.end_line,
        ))
    }

    fn file_line_count(&self, request: ForgeFileLinesRequest) -> Result<u32> {
        let content = self.file_content(&request)?;
        Ok(content.lines().count() as u32)
    }

    fn create_review(
        &self,
        pr: &PullRequestDetails,
        request: CreateReviewRequest<'_>,
    ) -> Result<GhCreateReviewResponse> {
        let body = request.body.trim();
        // Gitea rejects these server-side with a bare 422; catching them here
        // costs a round trip and produces a message that names the fix.
        if request.event == SubmitEvent::RequestChanges && body.is_empty() {
            return Err(TuicrError::Forge(
                "Gitea requires a review summary when requesting changes. Add a review-level \
                 comment and submit again."
                    .to_string(),
            ));
        }
        if request.event == SubmitEvent::Comment && body.is_empty() && request.comments.is_empty() {
            return Err(TuicrError::Forge(
                "Gitea requires a review summary or at least one inline comment.".to_string(),
            ));
        }

        let comments = request
            .comments
            .iter()
            .map(|comment| {
                // Gitea has no multi-line review comments: a comment carries a
                // single line number, and its sign picks the side. Range
                // selections anchor to their last line, matching where GitHub
                // renders the marker.
                let (path, key) = match comment.side {
                    GhSide::Left => (
                        comment.old_path.as_ref().unwrap_or(&comment.path),
                        "old_position",
                    ),
                    GhSide::Right => (&comment.path, "new_position"),
                };
                json!({
                    "path": path.to_string_lossy(),
                    "body": comment.body,
                    key: comment.line,
                })
            })
            .collect::<Vec<_>>();

        let mut payload = json!({
            "commit_id": request.commit_id,
            "body": body,
            "comments": comments,
        });
        // Omitting `event` is what makes Gitea file the review as PENDING.
        if let Some(event) = gitea_review_event(request.event) {
            payload["event"] = json!(event);
        }

        let response = self.api(
            &pr.repository,
            "POST",
            &Self::pulls_endpoint(&pr.repository, pr.number, "/reviews"),
            Some(&serde_json::to_string(&payload)?),
        )?;
        let review: GiteaPullReview = serde_json::from_str(&response.body)?;
        Ok(GhCreateReviewResponse {
            id: review.id,
            html_url: review.html_url,
            state: review.state,
        })
    }
}

// ----- Conversions -----

fn into_summary(pr: GiteaPullRequest, repository: &ForgeRepository) -> PullRequestSummary {
    PullRequestSummary {
        repository: repository.clone(),
        number: pr.number,
        title: pr.title,
        author: GiteaUser::login(pr.user),
        head_ref_name: display_ref(&pr.head.ref_name),
        base_ref_name: display_ref(&pr.base.ref_name),
        updated_at: pr.updated_at,
        url: pr.html_url,
        state: pr.state,
        is_draft: pr.draft,
    }
}

fn into_details(pr: GiteaPullRequest, repository: &ForgeRepository) -> PullRequestDetails {
    let closed = pr.merged || !pr.state.eq_ignore_ascii_case("open");
    PullRequestDetails {
        repository: repository.clone(),
        number: pr.number,
        title: pr.title,
        url: pr.html_url,
        state: pr.state,
        is_draft: pr.draft,
        author: GiteaUser::login(pr.user),
        head_ref_name: display_ref(&pr.head.ref_name),
        base_ref_name: display_ref(&pr.base.ref_name),
        head_sha: pr.head.sha,
        // `merge_base` is where the PR actually forked from; `base.sha` is
        // wherever the base branch has since moved to. Diffs and context
        // lookups want the former.
        base_sha: pr
            .merge_base
            .filter(|sha| !sha.is_empty())
            .unwrap_or(pr.base.sha),
        body: pr.body,
        updated_at: pr.updated_at,
        closed,
        merged_at: pr.merged_at,
        diff_start_sha: None,
    }
}

/// Gitea replaces a deleted head branch with `refs/pull/<n>/head`, which is
/// noise in a branch column.
fn display_ref(value: &str) -> String {
    if value.starts_with("refs/pull/") {
        String::new()
    } else {
        value.to_string()
    }
}

fn issue_into_summary(issue: GiteaIssue, repository: &ForgeRepository) -> PullRequestSummary {
    let meta = issue
        .pull_request
        .unwrap_or(super::models::GiteaPullRequestMeta {
            draft: false,
            merged: false,
            html_url: None,
        });
    PullRequestSummary {
        repository: repository.clone(),
        number: issue.number,
        title: issue.title,
        author: GiteaUser::login(issue.user),
        // The issue search endpoint does not carry branch names.
        head_ref_name: String::new(),
        base_ref_name: String::new(),
        updated_at: issue.updated_at,
        url: meta.html_url.unwrap_or(issue.html_url),
        state: issue.state,
        is_draft: meta.draft,
    }
}

fn into_file_metadata(file: GiteaChangedFile) -> Result<FileMetadata> {
    let new_path = PathBuf::from(&file.filename);
    let previous = file
        .previous_filename
        .filter(|path| !path.is_empty())
        .map(PathBuf::from);

    Ok(match file.status.as_str() {
        "added" => FileMetadata {
            old_path: None,
            new_path: Some(new_path),
            status: FileStatus::Added,
        },
        "deleted" | "removed" => FileMetadata {
            old_path: Some(new_path),
            new_path: None,
            status: FileStatus::Deleted,
        },
        "renamed" => FileMetadata {
            old_path: previous.or_else(|| Some(new_path.clone())),
            new_path: Some(new_path),
            status: FileStatus::Renamed,
        },
        "copied" => FileMetadata {
            old_path: previous.or_else(|| Some(new_path.clone())),
            new_path: Some(new_path),
            status: FileStatus::Copied,
        },
        // Gitea reports a mode-only change as "unchanged"; it still emits a
        // `diff --git` block, so it must stay in the metadata list or the
        // positional pairing with the patch text goes out of step.
        "changed" | "modified" | "unchanged" => FileMetadata {
            old_path: previous.clone().or_else(|| Some(new_path.clone())),
            new_path: Some(new_path),
            status: FileStatus::Modified,
        },
        other => {
            return Err(TuicrError::Forge(format!(
                "Gitea reported unknown file status '{other}' for {}",
                file.filename
            )));
        }
    })
}

fn into_pull_request_commit(commit: GiteaCommit) -> PullRequestCommit {
    let payload = commit.commit.unwrap_or(super::models::GiteaCommitPayload {
        message: String::new(),
        author: None,
    });
    let summary = payload
        .message
        .lines()
        .next()
        .unwrap_or_default()
        .to_string();
    let author = GiteaUser::login(commit.author)
        .or_else(|| payload.author.map(|author| author.name))
        .unwrap_or_default();
    let short_oid = commit.sha.chars().take(7).collect();
    PullRequestCommit {
        oid: commit.sha,
        short_oid,
        summary,
        author,
        timestamp: commit.created,
    }
}

fn parse_review_state(state: &str) -> RemoteReviewState {
    match state.to_ascii_uppercase().as_str() {
        "APPROVED" => RemoteReviewState::Approved,
        // Gitea spells this without the trailing D that GitHub uses, so the
        // shared parser would silently downgrade it to "commented".
        "REQUEST_CHANGES" => RemoteReviewState::ChangesRequested,
        "PENDING" => RemoteReviewState::Pending,
        other => RemoteReviewState::parse(other),
    }
}

/// Gitea's review event names, which differ from GitHub's (`APPROVED` rather
/// than `APPROVE`). `None` means "omit the field", which files the review as
/// a pending draft.
fn gitea_review_event(event: SubmitEvent) -> Option<&'static str> {
    match event {
        SubmitEvent::Comment => Some("COMMENT"),
        SubmitEvent::Approve => Some("APPROVED"),
        SubmitEvent::RequestChanges => Some("REQUEST_CHANGES"),
        SubmitEvent::Draft => None,
    }
}

/// Fold a flat comment list into threads.
///
/// Gitea has no thread object: replies are ordinary comments sharing a file
/// and line, which is exactly how its own UI groups them. Anchor identity is
/// therefore `(path, side, line)`.
fn group_comments_into_threads(comments: Vec<GiteaPullReviewComment>) -> Vec<RemoteReviewThread> {
    let mut threads: Vec<RemoteReviewThread> = Vec::new();
    let mut index: BTreeMap<(String, bool, u32), usize> = BTreeMap::new();

    for comment in comments {
        let (side, line) = if comment.position > 0 {
            (RemoteCommentSide::Right, Some(comment.position))
        } else if comment.original_position > 0 {
            (RemoteCommentSide::Left, Some(comment.original_position))
        } else {
            (RemoteCommentSide::Right, None)
        };

        let resolved = comment.resolver.is_some();
        let entry = RemoteReviewComment {
            id: comment.id.to_string(),
            author: GiteaUser::login(comment.user),
            body: comment.body,
            created_at: comment.created_at,
            in_reply_to: None,
            url: comment.html_url,
        };

        let key = (
            comment.path.clone(),
            matches!(side, RemoteCommentSide::Left),
            line.unwrap_or(0),
        );
        match index.get(&key) {
            Some(&position) => {
                let thread = &mut threads[position];
                // Gitea resolves a whole line conversation at once, so any
                // resolved member marks the thread.
                thread.is_resolved |= resolved;
                let root_id = thread.comments.first().map(|root| root.id.clone());
                thread.comments.push(RemoteReviewComment {
                    in_reply_to: root_id,
                    ..entry
                });
            }
            None => {
                index.insert(key, threads.len());
                threads.push(RemoteReviewThread {
                    id: format!(
                        "{}:{}:{}",
                        comment.path,
                        if matches!(side, RemoteCommentSide::Left) {
                            "LEFT"
                        } else {
                            "RIGHT"
                        },
                        line.unwrap_or(0)
                    ),
                    path: comment.path,
                    line,
                    side,
                    is_resolved: resolved,
                    // Gitea tracks whether a comment was invalidated by a
                    // force-push internally but does not expose it on the API,
                    // so outdated threads are indistinguishable from current
                    // ones here.
                    is_outdated: false,
                    comments: vec![entry],
                });
            }
        }
    }

    threads
}

// ----- Raw diff parsing -----

/// Recover `FilePatch`es from unified diff text alone.
///
/// The cumulative PR diff gets authoritative metadata from `/pulls/{n}/files`,
/// but Gitea's compare endpoint has no structured counterpart, so commit-range
/// diffs have to read status and paths out of the git headers.
fn parse_unified_diff(patch: &str) -> Result<Vec<FilePatch>> {
    let blocks = crate::vcs::git::raw::split_patch_blocks(patch.as_bytes());
    let mut metadata = Vec::with_capacity(blocks.len());
    for block in &blocks {
        metadata.push(metadata_from_patch_block(&String::from_utf8_lossy(block))?);
    }
    crate::vcs::git::raw::pair_metadata_with_patch(metadata, patch.as_bytes())
}

fn metadata_from_patch_block(block: &str) -> Result<FileMetadata> {
    let mut status = FileStatus::Modified;
    let mut rename_from = None;
    let mut rename_to = None;
    let mut minus_path = None;
    let mut plus_path = None;

    for line in block.lines() {
        if let Some(rest) = line.strip_prefix("rename from ") {
            status = FileStatus::Renamed;
            rename_from = Some(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("rename to ") {
            status = FileStatus::Renamed;
            rename_to = Some(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("copy from ") {
            status = FileStatus::Copied;
            rename_from = Some(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("copy to ") {
            status = FileStatus::Copied;
            rename_to = Some(rest.to_string());
        } else if line.starts_with("new file mode ") {
            status = FileStatus::Added;
        } else if line.starts_with("deleted file mode ") {
            status = FileStatus::Deleted;
        } else if let Some(rest) = line.strip_prefix("--- ") {
            minus_path = strip_diff_prefix(rest);
        } else if let Some(rest) = line.strip_prefix("+++ ") {
            plus_path = strip_diff_prefix(rest);
        } else if line.starts_with("@@") {
            break;
        }
    }

    let header = block.lines().next().unwrap_or_default();
    let (header_old, header_new) = parse_diff_git_line(header).unzip();

    let old_path = rename_from.or(minus_path).or(header_old).map(PathBuf::from);
    let new_path = rename_to.or(plus_path).or(header_new).map(PathBuf::from);

    let (old_path, new_path) = match status {
        FileStatus::Added => (None, new_path.or_else(|| old_path.clone())),
        FileStatus::Deleted => (old_path.or_else(|| new_path.clone()), None),
        _ => (old_path, new_path),
    };

    if old_path.is_none() && new_path.is_none() {
        return Err(TuicrError::Forge(format!(
            "could not read a file path from a Gitea diff header: {header}. \
             Run tuicr from a local clone of the repository so the range diff \
             can be computed locally."
        )));
    }

    Ok(FileMetadata {
        old_path,
        new_path,
        status,
    })
}

/// `--- a/path` / `+++ b/path`, minus the side prefix and any trailing tab
/// metadata. `/dev/null` means the file does not exist on that side.
fn strip_diff_prefix(value: &str) -> Option<String> {
    let value = value.split('\t').next().unwrap_or(value).trim_end();
    if value == "/dev/null" {
        return None;
    }
    let stripped = value
        .strip_prefix("a/")
        .or_else(|| value.strip_prefix("b/"))
        .unwrap_or(value);
    (!stripped.is_empty()).then(|| stripped.to_string())
}

/// Split `diff --git a/<old> b/<new>` into its two paths.
///
/// Paths are unprefixed but unquoted, and either may contain spaces, so the
/// split point is found by testing each ` b/` for one that leaves a valid
/// `a/`-prefixed left half. Git quotes paths containing control characters or
/// non-ASCII bytes; those are left to the `---`/`+++` fallback.
fn parse_diff_git_line(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("diff --git ")?;
    if rest.starts_with('"') {
        return None;
    }
    let mut search_from = 0;
    while let Some(offset) = rest[search_from..].find(" b/") {
        let split_at = search_from + offset;
        let left = &rest[..split_at];
        let right = &rest[split_at + 1..];
        if let Some(old) = left.strip_prefix("a/")
            && let Some(new) = right.strip_prefix("b/")
            && !old.is_empty()
            && !new.is_empty()
        {
            return Some((old.to_string(), new.to_string()));
        }
        search_from = split_at + 1;
    }
    None
}

// ----- Local checkout helpers -----

/// Read a blob out of a local clone. `None` for anything that is not a clean
/// hit, so callers fall back to the API.
fn read_blob_with_repo(repo_root: &Path, sha: &str, path: &Path) -> Option<String> {
    let spec = format!("{}:{}", sha, path.to_string_lossy());
    run_command_output(
        "git",
        Some(repo_root),
        ["cat-file", "-e", spec.as_str()]
            .iter()
            .map(|s| OsStr::new(*s)),
    )
    .ok()?;
    run_command_output(
        "git",
        Some(repo_root),
        ["show", spec.as_str()].iter().map(|s| OsStr::new(*s)),
    )
    .ok()
}

/// `Some(diff)` when both SHAs are present locally.
fn local_range_diff(repo_root: &Path, start_sha: &str, end_sha: &str) -> Option<Vec<FilePatch>> {
    for sha in [start_sha, end_sha] {
        run_command_output(
            "git",
            Some(repo_root),
            ["cat-file", "-e", sha].iter().map(|s| OsStr::new(*s)),
        )
        .ok()?;
    }
    run_git_diff(repo_root, &[format!("{start_sha}..{end_sha}").as_str()]).ok()
}

// ----- URL helpers -----

/// Percent-encode one path segment's worth of text, keeping `/` intact so a
/// repository-relative path stays a path.
fn percent_encode_path(value: &str) -> String {
    value
        .split('/')
        .map(percent_encode_component)
        .collect::<Vec<_>>()
        .join("/")
}

fn percent_encode_component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
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

/// Strip a trailing `:<port>`. Self-hosted instances commonly use a non-default
/// SSH port, which is meaningless for the HTTPS API host.
fn strip_port(host: &str) -> &str {
    match host.rsplit_once(':') {
        Some((h, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => h,
        _ => host,
    }
}

fn strip_git_suffix(value: &str) -> &str {
    value.strip_suffix(".git").unwrap_or(value)
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

/// Resolve an SSH host alias through `~/.ssh/config`, so a remote written
/// against an alias still identifies its real instance.
fn resolve_ssh_hostname(alias: &str) -> String {
    let Ok(home) = std::env::var("HOME") else {
        return alias.to_string();
    };
    let Ok(content) = fs::read_to_string(PathBuf::from(home).join(".ssh/config")) else {
        return alias.to_string();
    };
    resolve_ssh_hostname_from_config(alias, &content)
}

/// Only exact `Host` patterns are matched; wildcards, negation, `Match`, and
/// `Include` fall through to the alias unchanged.
fn resolve_ssh_hostname_from_config(alias: &str, config: &str) -> String {
    let mut in_block = false;
    for raw in config.lines() {
        let line = raw.split_once('#').map_or(raw, |(before, _)| before).trim();
        if line.is_empty() {
            continue;
        }
        let (key, value) = line
            .split_once(|c: char| c.is_whitespace() || c == '=')
            .unwrap_or((line, ""));
        let value = value
            .trim_start_matches(|c: char| c.is_whitespace() || c == '=')
            .trim();

        if key.eq_ignore_ascii_case("Host") {
            in_block = value.split_whitespace().any(|pattern| pattern == alias);
        } else if key.eq_ignore_ascii_case("Match") {
            in_block = false;
        } else if in_block && key.eq_ignore_ascii_case("HostName") {
            return value.to_string();
        }
    }
    alias.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::submit::InlineComment;
    use std::cell::RefCell;

    /// Routes calls by matching a substring against the endpoint argument
    /// (always the last one), so tests can register a canned body per request
    /// without caring about flag order.
    #[derive(Default)]
    struct FakeTeaRunner {
        routes: RefCell<Vec<(String, TeaResponse)>>,
        calls: RefCell<Vec<(Vec<String>, Option<String>)>>,
    }

    impl FakeTeaRunner {
        fn route(self, needle: &str, body: &str) -> Self {
            self.routes.borrow_mut().push((
                needle.to_string(),
                TeaResponse {
                    body: body.to_string(),
                    total_count: None,
                },
            ));
            self
        }

        fn route_counted(self, needle: &str, body: &str, total: usize) -> Self {
            self.routes.borrow_mut().push((
                needle.to_string(),
                TeaResponse {
                    body: body.to_string(),
                    total_count: Some(total),
                },
            ));
            self
        }

        fn endpoints(&self) -> Vec<String> {
            self.calls
                .borrow()
                .iter()
                .filter_map(|(args, _)| args.last().cloned())
                .collect()
        }

        fn last_stdin(&self) -> Option<String> {
            self.calls
                .borrow()
                .last()
                .and_then(|(_, body)| body.clone())
        }
    }

    impl TeaCommandRunner for FakeTeaRunner {
        fn run(&self, args: &[String], stdin: Option<&str>) -> TeaCommandResult<TeaResponse> {
            self.calls
                .borrow_mut()
                .push((args.to_vec(), stdin.map(str::to_string)));
            let endpoint = args.last().cloned().unwrap_or_default();
            // Most specific route wins, so a prefix like `/pulls/42` does not
            // shadow `/pulls/42/reviews` just by being registered first.
            self.routes
                .borrow()
                .iter()
                .filter(|(needle, _)| endpoint.contains(needle.as_str()))
                .max_by_key(|(needle, _)| needle.len())
                .map(|(_, response)| response.clone())
                .ok_or_else(|| TeaCommandError::Http {
                    status: 404,
                    body: format!("{{\"message\":\"no route for {endpoint}\"}}"),
                })
        }
    }

    fn repo() -> ForgeRepository {
        ForgeRepository::gitea("gitea.example.com", "team", "service")
    }

    const PR_JSON: &str = r#"{
        "number": 42,
        "title": "Add the thing",
        "state": "open",
        "user": { "login": "author" },
        "html_url": "https://gitea.example.com/team/service/pulls/42",
        "head": { "ref": "feat/thing", "sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" },
        "base": { "ref": "main", "sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb" },
        "merge_base": "cccccccccccccccccccccccccccccccccccccccc",
        "body": "Body text.",
        "updated_at": "2026-08-20T10:00:00Z",
        "draft": false,
        "merged": false,
        "merged_at": null,
        "mergeable": true
    }"#;

    const FILES_JSON: &str = r#"[
        { "filename": "src/lib.rs", "status": "changed" }
    ]"#;

    const PR_DIFF: &str = "diff --git a/src/lib.rs b/src/lib.rs\nindex 111..222 100644\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 +1,2 @@\n-old\n+new\n";

    fn details() -> PullRequestDetails {
        let runner = FakeTeaRunner::default().route("/pulls/42", PR_JSON);
        GiteaTeaBackend::with_runner(Some(repo()), runner)
            .get_pull_request(PullRequestTarget::number(42, "42"))
            .expect("details")
    }

    // ----- response envelope -----

    #[test]
    fn should_read_status_from_the_last_http_status_line() {
        let headers = "HTTP/1.1 301 Moved Permanently\r\nLocation: /x\r\n\r\nHTTP/1.1 200 OK\r\n";
        assert_eq!(parse_http_status(headers), Some(200));
    }

    #[test]
    fn should_return_no_status_when_tea_never_issued_a_request() {
        assert_eq!(parse_http_status("Error: unknown login \"nope\"\n"), None);
    }

    #[test]
    fn should_read_total_count_header_case_insensitively() {
        let headers = "HTTP/1.1 200 OK\r\nX-Total-Count: 710\r\n";
        assert_eq!(parse_header(headers, "x-total-count"), Some("710"));
    }

    #[test]
    fn should_extract_gitea_error_message_from_the_body() {
        assert_eq!(
            gitea_error_message(r#"{"message":"token does not have scope","url":"x"}"#),
            "token does not have scope"
        );
        assert_eq!(gitea_error_message(""), "no response body");
        assert_eq!(gitea_error_message("plain text"), "plain text");
    }

    #[test]
    fn should_map_unauthorized_to_an_actionable_message() {
        let error = map_tea_error(
            TeaCommandError::Http {
                status: 401,
                body: r#"{"message":"unauthorized"}"#.to_string(),
            },
            "gitea.example.com",
        );
        let text = error.to_string();
        assert!(text.contains("tea logins add"), "{text}");
    }

    #[test]
    fn should_map_a_missing_tea_binary_to_install_guidance() {
        let error = map_tea_error(TeaCommandError::MissingTea, "gitea.example.com");
        assert!(error.to_string().contains("`tea` CLI not found"));
    }

    // ----- host + remote parsing -----

    #[test]
    fn should_parse_https_gitea_remote() {
        assert_eq!(
            parse_gitea_remote_url("https://gitea.example.com/team/service.git"),
            Some(repo())
        );
    }

    #[test]
    fn should_parse_scp_style_gitea_remote() {
        assert_eq!(
            parse_gitea_remote_url("git@gitea.example.com:team/service.git"),
            Some(repo())
        );
    }

    #[test]
    fn should_not_claim_a_fork_host_by_name() {
        // Forgejo and Codeberg are compatible today but are separate projects.
        // Routing them requires an explicit `tea` login, not a hostname guess.
        assert_eq!(
            parse_gitea_remote_url("https://codeberg.org/team/service"),
            None
        );
    }

    #[test]
    fn should_strip_ssh_port_from_self_hosted_remote() {
        assert_eq!(
            parse_gitea_remote_url("ssh://git@gitea.example.com:2222/team/service.git"),
            Some(repo())
        );
    }

    #[test]
    fn should_ignore_remotes_belonging_to_other_forges() {
        // These bail before consulting `tea`, so the test never shells out.
        assert_eq!(parse_gitea_remote_url("https://github.com/o/r.git"), None);
        assert_eq!(parse_gitea_remote_url("https://gitlab.com/o/r.git"), None);
        assert_eq!(parse_gitea_remote_url("git@bitbucket.org:o/r.git"), None);
    }

    fn login(host: &str, name: &str, is_default: bool) -> TeaLoginHost {
        TeaLoginHost {
            host: host.to_string(),
            name: name.to_string(),
            is_default,
        }
    }

    #[test]
    fn should_prefer_the_default_login_when_a_host_has_several() {
        let logins = vec![
            login("gitea.example.com", "personal", false),
            login("gitea.example.com", "work", true),
            login("other.example.com", "elsewhere", true),
        ];
        assert_eq!(
            select_login(&logins, "gitea.example.com"),
            Some("work"),
            "the default account must win over config file order"
        );
    }

    #[test]
    fn should_fall_back_to_the_first_login_when_none_is_default() {
        let logins = vec![
            login("gitea.example.com", "first", false),
            login("gitea.example.com", "second", false),
        ];
        assert_eq!(select_login(&logins, "gitea.example.com"), Some("first"));
        assert_eq!(select_login(&logins, "nope.example.com"), None);
    }

    #[test]
    fn should_read_host_out_of_a_tea_login_url() {
        assert_eq!(
            host_of_url("https://gitea.example.com:3000/"),
            Some("gitea.example.com".to_string())
        );
        assert_eq!(host_of_url(""), None);
    }

    // ----- PR target parsing -----

    #[test]
    fn should_parse_a_gitea_pull_request_url() {
        let target =
            parse_pull_request_target_gitea("https://gitea.example.com/team/service/pulls/42")
                .expect("target");
        assert_eq!(target.number, 42);
        assert_eq!(target.repository, Some(repo()));
    }

    #[test]
    fn should_not_claim_a_github_style_singular_pull_url() {
        // GitHub uses `/pull/<n>`; only Gitea uses the plural.
        assert!(
            parse_pull_request_target_gitea("https://gitea.example.com/team/service/pull/42")
                .is_err()
        );
    }

    #[test]
    fn should_parse_a_host_qualified_repo_hash_target() {
        let target =
            parse_pull_request_target_gitea("gitea.example.com/team/service#42").expect("target");
        assert_eq!(target.number, 42);
        assert_eq!(target.repository, Some(repo()));
    }

    #[test]
    fn should_leave_bare_owner_repo_hash_targets_to_github() {
        assert!(parse_pull_request_target_gitea("team/service#42").is_err());
    }

    #[test]
    fn should_reject_zero_and_empty_targets() {
        assert!(parse_pull_request_target_gitea("").is_err());
        assert!(
            parse_pull_request_target_gitea("https://gitea.example.com/team/service/pulls/0")
                .is_err()
        );
    }

    // ----- details -----

    #[test]
    fn should_prefer_merge_base_over_base_branch_tip_for_base_sha() {
        // `base.sha` follows the base branch as it moves; the diff was taken
        // against the fork point.
        let details = details();
        assert_eq!(details.base_sha, "cccccccccccccccccccccccccccccccccccccccc");
        assert_eq!(details.head_sha, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    }

    #[test]
    fn should_fall_back_to_base_sha_when_merge_base_is_absent() {
        let json = PR_JSON.replace(
            "\"merge_base\": \"cccccccccccccccccccccccccccccccccccccccc\",",
            "",
        );
        let runner = FakeTeaRunner::default().route("/pulls/42", &json);
        let details = GiteaTeaBackend::with_runner(Some(repo()), runner)
            .get_pull_request(PullRequestTarget::number(42, "42"))
            .expect("details");
        assert_eq!(details.base_sha, "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
    }

    #[test]
    fn should_blank_a_deleted_head_branch_ref() {
        // Gitea substitutes the pull ref once the branch is gone.
        assert_eq!(display_ref("refs/pull/42/head"), "");
        assert_eq!(display_ref("feat/thing"), "feat/thing");
    }

    #[test]
    fn should_error_when_the_pull_request_reports_no_shas() {
        let json = PR_JSON.replace(
            "\"sha\": \"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"",
            "\"sha\": \"\"",
        );
        let runner = FakeTeaRunner::default().route("/pulls/42", &json);
        let error = GiteaTeaBackend::with_runner(Some(repo()), runner)
            .get_pull_request(PullRequestTarget::number(42, "42"))
            .unwrap_err();
        assert!(error.to_string().contains("head and base commit SHAs"));
    }

    // ----- listing + pagination -----

    #[test]
    fn should_list_open_pull_requests_with_paging_from_total_count() {
        let runner = FakeTeaRunner::default().route_counted(
            "/pulls?state=open&page=1",
            &format!("[{PR_JSON}]"),
            5,
        );
        let backend = GiteaTeaBackend::with_runner(Some(repo()), runner);
        let page = backend
            .list_pull_requests(PullRequestListQuery::first_page(repo(), 1))
            .expect("page");

        assert_eq!(page.pull_requests.len(), 1);
        assert_eq!(page.pull_requests[0].number, 42);
        assert_eq!(page.total_loaded, 1);
        assert!(page.has_more, "5 total with 1 loaded means more remain");
        assert!(backend.runner.endpoints()[0].contains("state=open"));
    }

    #[test]
    fn should_stop_paging_when_total_count_is_reached() {
        let runner =
            FakeTeaRunner::default().route_counted("/pulls?state=open", &format!("[{PR_JSON}]"), 1);
        let page = GiteaTeaBackend::with_runner(Some(repo()), runner)
            .list_pull_requests(PullRequestListQuery::first_page(repo(), 30))
            .expect("page");
        assert!(!page.has_more);
    }

    #[test]
    fn should_keep_paging_when_the_server_caps_the_page_below_the_requested_limit() {
        // Gitea clamps `limit` to MaxResponseItems. Treating "fewer than
        // requested" as the last page would silently drop every file past the
        // cap, so pagination has to infer the real page size instead.
        let runner = FakeTeaRunner::default()
            .route("/files?page=1", r#"[{"filename":"a.rs","status":"added"}]"#)
            .route("/files?page=2", r#"[{"filename":"b.rs","status":"added"}]"#)
            .route("/files?page=3", "[]");
        let backend = GiteaTeaBackend::with_runner(Some(repo()), runner);
        let metadata = backend.file_metadata(&details()).expect("metadata");

        assert_eq!(metadata.len(), 2, "both capped pages must be read");
        assert_eq!(
            backend.runner.endpoints().len(),
            3,
            "two full pages plus the empty one"
        );
    }

    // ----- diff -----

    #[test]
    fn should_pair_file_metadata_with_the_cumulative_diff() {
        let runner = FakeTeaRunner::default()
            .route_counted("/pulls/42/files", FILES_JSON, 1)
            .route("/pulls/42.diff", PR_DIFF);
        let backend = GiteaTeaBackend::with_runner(Some(repo()), runner);
        let patches = backend.get_pull_request_diff(&details()).expect("diff");

        assert_eq!(patches.len(), 1);
        assert_eq!(
            patches[0].new_path.as_deref(),
            Some(Path::new("src/lib.rs"))
        );
        assert_eq!(patches[0].status, FileStatus::Modified);
        assert!(patches[0].patch.contains("+new"));
    }

    #[test]
    fn should_map_every_gitea_file_status() {
        let cases = [
            ("added", FileStatus::Added),
            ("deleted", FileStatus::Deleted),
            ("changed", FileStatus::Modified),
            // A mode-only change. It still produces a `diff --git` block, so
            // dropping it would desynchronize the positional pairing.
            ("unchanged", FileStatus::Modified),
            ("renamed", FileStatus::Renamed),
            ("copied", FileStatus::Copied),
        ];
        for (raw, expected) in cases {
            let metadata = into_file_metadata(GiteaChangedFile {
                filename: "src/lib.rs".to_string(),
                previous_filename: Some("src/old.rs".to_string()),
                status: raw.to_string(),
            })
            .unwrap_or_else(|error| panic!("{raw}: {error}"));
            assert_eq!(metadata.status, expected, "status for {raw}");
        }
    }

    #[test]
    fn should_reject_an_unknown_file_status_rather_than_guess() {
        let error = into_file_metadata(GiteaChangedFile {
            filename: "x".to_string(),
            previous_filename: None,
            status: "teleported".to_string(),
        })
        .unwrap_err();
        assert!(error.to_string().contains("teleported"));
    }

    #[test]
    fn should_fetch_a_commit_range_diff_from_the_compare_endpoint() {
        let runner = FakeTeaRunner::default().route("/compare/", PR_DIFF);
        let backend = GiteaTeaBackend::with_runner(Some(repo()), runner);
        let patches = backend
            .get_pull_request_commit_range_diff(&details(), "start", "end")
            .expect("range diff");

        assert_eq!(patches.len(), 1);
        assert_eq!(
            patches[0].new_path.as_deref(),
            Some(Path::new("src/lib.rs"))
        );
        assert!(
            backend.runner.endpoints().iter().any(
                |endpoint| endpoint.contains("start...end") && endpoint.contains("output=diff")
            )
        );
    }

    // ----- raw diff metadata recovery -----

    #[test]
    fn should_recover_metadata_from_raw_diff_headers() {
        let added = metadata_from_patch_block(
            "diff --git a/new.rs b/new.rs\nnew file mode 100644\n--- /dev/null\n+++ b/new.rs\n@@\n",
        )
        .expect("added");
        assert_eq!(added.status, FileStatus::Added);
        assert_eq!(added.old_path, None);
        assert_eq!(added.new_path.as_deref(), Some(Path::new("new.rs")));

        let deleted = metadata_from_patch_block(
            "diff --git a/gone.rs b/gone.rs\ndeleted file mode 100644\n--- a/gone.rs\n+++ /dev/null\n@@\n",
        )
        .expect("deleted");
        assert_eq!(deleted.status, FileStatus::Deleted);
        assert_eq!(deleted.old_path.as_deref(), Some(Path::new("gone.rs")));
        assert_eq!(deleted.new_path, None);

        let renamed = metadata_from_patch_block(
            "diff --git a/old.rs b/new.rs\nsimilarity index 98%\nrename from old.rs\nrename to new.rs\n",
        )
        .expect("renamed");
        assert_eq!(renamed.status, FileStatus::Renamed);
        assert_eq!(renamed.old_path.as_deref(), Some(Path::new("old.rs")));
        assert_eq!(renamed.new_path.as_deref(), Some(Path::new("new.rs")));
    }

    #[test]
    fn should_recover_metadata_for_a_mode_only_change() {
        // No `---`/`+++` lines at all; the `diff --git` line is the only source.
        let metadata = metadata_from_patch_block(
            "diff --git a/script.sh b/script.sh\nold mode 100644\nnew mode 100755\n",
        )
        .expect("mode change");
        assert_eq!(metadata.status, FileStatus::Modified);
        assert_eq!(metadata.new_path.as_deref(), Some(Path::new("script.sh")));
    }

    #[test]
    fn should_split_a_diff_git_line_whose_paths_contain_spaces() {
        assert_eq!(
            parse_diff_git_line("diff --git a/my dir/file b.rs b/my dir/file b.rs"),
            Some((
                "my dir/file b.rs".to_string(),
                "my dir/file b.rs".to_string()
            ))
        );
    }

    #[test]
    fn should_report_an_actionable_error_for_an_unreadable_diff_header() {
        let error =
            metadata_from_patch_block("diff --git \"a/we\\.rd\" \"b/we\\.rd\"\n").unwrap_err();
        assert!(error.to_string().contains("local clone"), "{error}");
    }

    // ----- review threads -----

    fn comment(
        id: u64,
        path: &str,
        new_line: u32,
        old_line: u32,
        resolved: bool,
    ) -> GiteaPullReviewComment {
        GiteaPullReviewComment {
            id,
            body: format!("comment {id}"),
            user: Some(GiteaUser {
                login: "reviewer".to_string(),
            }),
            resolver: resolved.then(|| GiteaUser {
                login: "resolver".to_string(),
            }),
            pull_request_review_id: 1,
            created_at: None,
            path: path.to_string(),
            commit_id: String::new(),
            html_url: format!("https://gitea.example.com/c/{id}"),
            position: new_line,
            original_position: old_line,
        }
    }

    #[test]
    fn should_group_comments_on_the_same_line_into_one_thread() {
        let threads = group_comments_into_threads(vec![
            comment(1, "src/lib.rs", 10, 0, false),
            comment(2, "src/lib.rs", 10, 0, false),
        ]);

        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].comments.len(), 2);
        assert_eq!(threads[0].line, Some(10));
        assert_eq!(threads[0].side, RemoteCommentSide::Right);
        assert_eq!(threads[0].comments[1].in_reply_to.as_deref(), Some("1"));
    }

    #[test]
    fn should_separate_threads_by_side_even_on_the_same_line_number() {
        let threads = group_comments_into_threads(vec![
            comment(1, "src/lib.rs", 10, 0, false),
            comment(2, "src/lib.rs", 0, 10, false),
        ]);
        assert_eq!(threads.len(), 2);
        assert_eq!(threads[1].side, RemoteCommentSide::Left);
        assert_eq!(threads[1].line, Some(10));
    }

    #[test]
    fn should_mark_a_thread_resolved_when_any_comment_carries_a_resolver() {
        let threads = group_comments_into_threads(vec![
            comment(1, "src/lib.rs", 10, 0, false),
            comment(2, "src/lib.rs", 10, 0, true),
        ]);
        assert_eq!(threads.len(), 1);
        assert!(threads[0].is_resolved);
        // Gitea exposes no invalidation flag, so nothing can be called outdated.
        assert!(!threads[0].is_outdated);
    }

    #[test]
    fn should_keep_review_summaries_with_prose_and_drop_bare_approvals() {
        let reviews = format!(
            "[{},{}]",
            r#"{"id":1,"state":"APPROVED","body":"","user":{"login":"a"},"comments_count":0,"commit_id":"","html_url":"u"}"#,
            r#"{"id":2,"state":"REQUEST_CHANGES","body":"please fix","user":{"login":"b"},"comments_count":0,"commit_id":"","html_url":"u2"}"#
        );
        let runner = FakeTeaRunner::default()
            .route("/pulls/42", PR_JSON)
            .route_counted("/pulls/42/reviews", &reviews, 2);
        let backend = GiteaTeaBackend::with_runner(Some(repo()), runner);
        let summaries = backend
            .list_review_summaries(&details())
            .expect("summaries");

        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].body, "please fix");
        assert_eq!(summaries[0].state, RemoteReviewState::ChangesRequested);
    }

    #[test]
    fn should_parse_gitea_request_changes_state_which_github_spells_differently() {
        // The shared parser expects GitHub's `CHANGES_REQUESTED`.
        assert_eq!(
            parse_review_state("REQUEST_CHANGES"),
            RemoteReviewState::ChangesRequested
        );
        assert_eq!(parse_review_state("APPROVED"), RemoteReviewState::Approved);
        assert_eq!(parse_review_state("COMMENT"), RemoteReviewState::Commented);
    }

    // ----- submit -----

    fn inline(line: u32, side: GhSide, old_path: Option<&str>) -> InlineComment {
        InlineComment {
            path: PathBuf::from("src/lib.rs"),
            line,
            side,
            counterpart_line: None,
            start_line: None,
            start_side: None,
            range_anchors: None,
            old_path: old_path.map(PathBuf::from),
            body: "looks wrong".to_string(),
            comment_id: "local-1".to_string(),
        }
    }

    fn submit(
        event: SubmitEvent,
        body: &str,
        comments: &[InlineComment],
    ) -> (serde_json::Value, GhCreateReviewResponse) {
        let review = r#"{"id":77,"state":"COMMENT","body":"","user":{"login":"me"},"comments_count":0,"commit_id":"","html_url":"https://gitea.example.com/r/77"}"#;
        let runner = FakeTeaRunner::default()
            .route("/pulls/42", PR_JSON)
            .route("/pulls/42/reviews", review);
        let backend = GiteaTeaBackend::with_runner(Some(repo()), runner);
        let response = backend
            .create_review(
                &details(),
                CreateReviewRequest {
                    event,
                    commit_id: "headsha",
                    body,
                    comments,
                },
            )
            .expect("create review");
        let payload: serde_json::Value =
            serde_json::from_str(&backend.runner.last_stdin().expect("stdin payload")).unwrap();
        (payload, response)
    }

    #[test]
    fn should_send_a_right_side_comment_as_new_position() {
        let (payload, response) = submit(
            SubmitEvent::Comment,
            "summary",
            &[inline(12, GhSide::Right, None)],
        );
        let comment = &payload["comments"][0];
        assert_eq!(comment["new_position"], 12);
        assert!(comment.get("old_position").is_none());
        assert_eq!(comment["path"], "src/lib.rs");
        assert_eq!(payload["event"], "COMMENT");
        assert_eq!(response.id, 77);
    }

    #[test]
    fn should_send_a_left_side_comment_as_old_position_on_the_pre_rename_path() {
        let (payload, _) = submit(
            SubmitEvent::Comment,
            "summary",
            &[inline(9, GhSide::Left, Some("src/old.rs"))],
        );
        let comment = &payload["comments"][0];
        assert_eq!(comment["old_position"], 9);
        assert!(comment.get("new_position").is_none());
        assert_eq!(comment["path"], "src/old.rs");
    }

    #[test]
    fn should_use_gitea_event_names_rather_than_githubs() {
        // GitHub says APPROVE; Gitea says APPROVED and rejects the other.
        let (payload, _) = submit(SubmitEvent::Approve, "ship it", &[]);
        assert_eq!(payload["event"], "APPROVED");
        assert_eq!(gitea_review_event(SubmitEvent::Approve), Some("APPROVED"));
    }

    #[test]
    fn should_omit_the_event_field_for_a_draft_so_gitea_files_it_as_pending() {
        let (payload, _) = submit(SubmitEvent::Draft, "wip", &[inline(3, GhSide::Right, None)]);
        assert!(
            payload.get("event").is_none(),
            "a draft must not name an event: {payload}"
        );
        assert_eq!(payload["commit_id"], "headsha");
    }

    #[test]
    fn should_refuse_to_request_changes_without_a_summary() {
        // Gitea answers this with a bare 422; failing locally names the fix.
        let runner = FakeTeaRunner::default().route("/pulls/42", PR_JSON);
        let backend = GiteaTeaBackend::with_runner(Some(repo()), runner);
        let error = backend
            .create_review(
                &details(),
                CreateReviewRequest {
                    event: SubmitEvent::RequestChanges,
                    commit_id: "headsha",
                    body: "   ",
                    comments: &[],
                },
            )
            .unwrap_err();
        assert!(error.to_string().contains("requesting changes"), "{error}");
    }

    #[test]
    fn should_refuse_an_empty_comment_review() {
        let runner = FakeTeaRunner::default().route("/pulls/42", PR_JSON);
        let backend = GiteaTeaBackend::with_runner(Some(repo()), runner);
        let error = backend
            .create_review(
                &details(),
                CreateReviewRequest {
                    event: SubmitEvent::Comment,
                    commit_id: "headsha",
                    body: "",
                    comments: &[],
                },
            )
            .unwrap_err();
        assert!(error.to_string().contains("at least one inline comment"));
    }

    // ----- commits -----

    #[test]
    fn should_convert_commits_using_the_first_message_line_as_the_summary() {
        let commit = into_pull_request_commit(GiteaCommit {
            sha: "abcdef1234567890".to_string(),
            commit: Some(super::super::models::GiteaCommitPayload {
                message: "fix: the thing\n\nlonger body".to_string(),
                author: Some(super::super::models::GiteaCommitAuthor {
                    name: "Fallback Name".to_string(),
                }),
            }),
            author: Some(GiteaUser {
                login: "handle".to_string(),
            }),
            created: None,
        });
        assert_eq!(commit.summary, "fix: the thing");
        assert_eq!(commit.short_oid, "abcdef1");
        assert_eq!(commit.author, "handle");
    }

    #[test]
    fn should_fall_back_to_the_commit_author_name_when_no_account_matches() {
        let commit = into_pull_request_commit(GiteaCommit {
            sha: "abcdef1234567890".to_string(),
            commit: Some(super::super::models::GiteaCommitPayload {
                message: "chore: bump".to_string(),
                author: Some(super::super::models::GiteaCommitAuthor {
                    name: "Unlinked Author".to_string(),
                }),
            }),
            author: None,
            created: None,
        });
        assert_eq!(commit.author, "Unlinked Author");
    }

    #[test]
    fn should_return_pull_request_commits_oldest_first() {
        // Gitea serves them newest-first; the trait and the commit-range
        // scoping both want chronological order.
        let commits = format!(
            "[{},{}]",
            r#"{"sha":"cccc","commit":{"message":"third"},"author":{"login":"a"}}"#,
            r#"{"sha":"aaaa","commit":{"message":"first"},"author":{"login":"a"}}"#
        );
        let runner = FakeTeaRunner::default()
            .route("/pulls/42", PR_JSON)
            .route_counted("/pulls/42/commits", &commits, 2);
        let listed = GiteaTeaBackend::with_runner(Some(repo()), runner)
            .list_pull_request_commits(&details())
            .expect("commits");

        assert_eq!(
            listed
                .iter()
                .map(|c| c.summary.as_str())
                .collect::<Vec<_>>(),
            vec!["first", "third"]
        );
    }

    // ----- url encoding -----

    #[test]
    fn should_percent_encode_path_segments_but_keep_separators() {
        assert_eq!(percent_encode_path("src/a file.rs"), "src/a%20file.rs");
        assert_eq!(percent_encode_component("feat/x"), "feat%2Fx");
    }

    #[test]
    fn should_resolve_an_ssh_alias_to_its_real_hostname() {
        let config = "Host gitea-work\n  HostName gitea.example.com\n  User git\n";
        assert_eq!(
            resolve_ssh_hostname_from_config("gitea-work", config),
            "gitea.example.com"
        );
        assert_eq!(resolve_ssh_hostname_from_config("other", config), "other");
    }
}
