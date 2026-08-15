//! Azure DevOps backend: `ForgeBackend` over the Azure DevOps REST API.
//!
//! Transport is pluggable ([`AzHttp`]):
//! - [`PatHttp`] calls the REST API directly over HTTPS with a Personal Access
//!   Token (HTTP Basic auth). Used automatically when a PAT env var is set.
//! - [`AzCliHttp`] shells out to the Azure CLI's `az rest` (reusing `az login`
//!   AAD auth). Used when no PAT is configured.
//!
//! PAT is preferred because many enterprise tenants don't provision the Azure
//! DevOps AAD app, so `az rest` can't mint a token there.
//!
//! Diffs come from a local clone (`git diff base...head`) because Azure DevOps
//! has no single unified-diff REST endpoint — the same approach GitLab uses for
//! its commit-range path. PR metadata, commits, file content, and review
//! submission go through the REST API.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::json;

use crate::error::{Result, TuicrError};
use crate::forge::local_merge_base;
use crate::forge::remote_comments::RemoteReviewThread;
use crate::forge::submit::{GhSide, SubmitEvent};
use crate::forge::traits::{
    CreateReviewRequest, ForgeBackend, ForgeFileLinesRequest, ForgeRepository,
    GhCreateReviewResponse, PagedPullRequests, PullRequestCommit, PullRequestDetails,
    PullRequestListQuery, PullRequestListScope, PullRequestTarget,
};
use crate::model::{DiffLine, FilePatch};
use crate::process::{CommandOutputError, CommandOutputErrorKind, run_command_output};
use crate::vcs::git::raw::run_git_diff;
use crate::vcs::slice_context_lines;

use super::models::{
    AzConnectionData, AzGitCommitRef, AzList, AzPullRequest, AzThread, AzThreadResponse,
};

/// REST api-version pinned across all calls.
const API_VERSION: &str = "7.1";
/// Well-known Azure DevOps AAD application (resource) id. `az rest --resource`
/// uses it to acquire a token for the Azure DevOps audience.
const AZ_RESOURCE: &str = "499b84ac-1321-427f-aa17-267ca6975798";
/// Canonical Azure DevOps cloud host. `*.visualstudio.com` URLs are normalized
/// to this so all API URLs share one shape.
const DEFAULT_AZURE_HOST: &str = "dev.azure.com";
/// Env vars checked (in order) for a Personal Access Token.
const PAT_ENV_VARS: &[&str] = &["AZURE_DEVOPS_EXT_PAT", "AZURE_DEVOPS_PAT"];

// ---------- Transport ----------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AzHttpError {
    /// The transport itself is unavailable (e.g. `az` not installed).
    Missing(String),
    /// 401/403 — authentication or authorization failure.
    Auth(String),
    /// Any other non-2xx status or transport error.
    Failed { status: Option<u16>, body: String },
}

pub type AzHttpResult<T> = std::result::Result<T, AzHttpError>;

/// HTTP transport for the Azure DevOps REST API. `url` already carries the
/// `api-version` query parameter. Returns the raw 2xx response body.
pub trait AzHttp: Send + Sync {
    fn request(&self, method: &str, url: &str, body: Option<&str>) -> AzHttpResult<String>;
}

/// Direct REST transport using a Personal Access Token (HTTP Basic auth:
/// empty username, PAT as password — the Azure DevOps convention).
pub struct PatHttp {
    auth_header: String,
    agent: ureq::Agent,
}

impl PatHttp {
    pub fn new(pat: &str) -> Self {
        let token = BASE64.encode(format!(":{pat}"));
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .http_status_as_error(false)
            .build();
        Self {
            auth_header: format!("Basic {token}"),
            agent: config.into(),
        }
    }
}

impl AzHttp for PatHttp {
    fn request(&self, method: &str, url: &str, body: Option<&str>) -> AzHttpResult<String> {
        let result = match method.to_ascii_uppercase().as_str() {
            "GET" => self
                .agent
                .get(url)
                .header("Authorization", self.auth_header.as_str())
                .call(),
            "POST" => self
                .agent
                .post(url)
                .header("Authorization", self.auth_header.as_str())
                .header("Content-Type", "application/json")
                .send(body.unwrap_or("")),
            "PUT" => self
                .agent
                .put(url)
                .header("Authorization", self.auth_header.as_str())
                .header("Content-Type", "application/json")
                .send(body.unwrap_or("")),
            other => {
                return Err(AzHttpError::Failed {
                    status: None,
                    body: format!("unsupported HTTP method {other}"),
                });
            }
        };

        let response = result.map_err(|err| AzHttpError::Failed {
            status: None,
            body: err.to_string(),
        })?;
        let status = response.status().as_u16();
        let text = response
            .into_body()
            .read_to_string()
            .map_err(|err| AzHttpError::Failed {
                status: Some(status),
                body: err.to_string(),
            })?;

        if (200..300).contains(&status) {
            Ok(text)
        } else if status == 401 || status == 403 {
            Err(AzHttpError::Auth(text))
        } else {
            Err(AzHttpError::Failed {
                status: Some(status),
                body: text,
            })
        }
    }
}

/// Azure CLI transport: shells out to `az rest` (AAD auth from `az login`).
#[derive(Debug, Clone, Copy, Default)]
pub struct AzCliHttp;

impl AzHttp for AzCliHttp {
    fn request(&self, method: &str, url: &str, body: Option<&str>) -> AzHttpResult<String> {
        let mut args = vec![
            "rest".to_string(),
            "--resource".to_string(),
            AZ_RESOURCE.to_string(),
            "--method".to_string(),
            method.to_ascii_lowercase(),
            "--output".to_string(),
            "json".to_string(),
            "--url".to_string(),
            url.to_string(),
        ];
        if let Some(body) = body {
            args.push("--headers".to_string());
            args.push("Content-Type=application/json".to_string());
            args.push("--body".to_string());
            args.push(body.to_string());
        }
        run_az_program(&args).map_err(AzHttpError::from)
    }
}

/// Spawn `az` with `args`. On Windows the launcher is `az.cmd`, which
/// `Command::new("az")` won't resolve, so retry with `az.cmd` on NotFound.
fn run_az_program(args: &[String]) -> std::result::Result<String, CommandOutputError> {
    let osargs = || args.iter().map(|a| OsStr::new(a.as_str()));
    match run_command_output("az", None, osargs()) {
        Err(err) if err.kind == CommandOutputErrorKind::NotFound && cfg!(windows) => {
            run_command_output("az.cmd", None, osargs())
        }
        other => other,
    }
}

impl From<CommandOutputError> for AzHttpError {
    fn from(error: CommandOutputError) -> Self {
        match error.kind {
            CommandOutputErrorKind::NotFound => Self::Missing(
                "Azure DevOps integration needs either a PAT (set AZURE_DEVOPS_EXT_PAT) or the \
                 Azure CLI (`az`)."
                    .to_string(),
            ),
            CommandOutputErrorKind::SpawnFailed | CommandOutputErrorKind::Unsuccessful => {
                if looks_like_az_auth_failure(&error.stderr) {
                    Self::Auth(error.stderr)
                } else {
                    Self::Failed {
                        status: error.status.map(|c| c as u16),
                        body: error.stderr,
                    }
                }
            }
        }
    }
}

fn looks_like_az_auth_failure(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    lower.contains("aadsts")
        || lower.contains("az login")
        || lower.contains("not logged in")
        || lower.contains("please run 'az login'")
        || lower.contains("401")
}

/// Read a PAT from the environment, if configured.
fn pat_from_env() -> Option<String> {
    for name in PAT_ENV_VARS {
        if let Ok(value) = std::env::var(name) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

/// Pick the transport: a PAT from the environment when present, else `az rest`.
fn default_transport() -> Box<dyn AzHttp> {
    match pat_from_env() {
        Some(pat) => Box::new(PatHttp::new(&pat)),
        None => Box::new(AzCliHttp),
    }
}

// ---------- Local git helpers (mirror gh/glab) ----------

/// Read a git blob from a checkout via `git show <sha>:<path>`. `None` on any
/// failure so callers fall back to the REST API.
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

/// `git diff <a><sep><b>` in `repo_root`, returning `None` when either SHA is
/// absent locally or the command fails.
fn local_diff(repo_root: &Path, a: &str, b: &str, sep: &str) -> Option<Vec<FilePatch>> {
    for sha in [a, b] {
        let exists = run_command_output(
            "git",
            Some(repo_root),
            ["cat-file", "-e", sha].iter().map(|s| OsStr::new(*s)),
        );
        if exists.is_err() {
            return None;
        }
    }
    let range = format!("{a}{sep}{b}");
    run_git_diff(repo_root, &[range.as_str()]).ok()
}

// ---------- Coordinate helpers ----------

/// Split an Azure `ForgeRepository`'s packed `owner` into `(organization,
/// project)`. Errors when `owner` isn't `org/project`.
pub fn azure_coords(repo: &ForgeRepository) -> Result<(String, String)> {
    match repo.owner.split_once('/') {
        Some((org, project)) if !org.is_empty() && !project.is_empty() => {
            Ok((org.to_string(), project.to_string()))
        }
        _ => Err(TuicrError::Forge(format!(
            "Azure DevOps repository `{}` is missing an organization/project (expected owner `org/project`)",
            repo.slug()
        ))),
    }
}

fn git_api_base(repo: &ForgeRepository) -> String {
    // owner is `org/project`, so this yields
    // https://dev.azure.com/org/project/_apis/git/repositories/<repo>
    format!(
        "https://{}/{}/_apis/git/repositories/{}",
        repo.host, repo.owner, repo.name
    )
}

fn with_api_version(url: &str) -> String {
    if url.contains('?') {
        format!("{url}&api-version={API_VERSION}")
    } else {
        format!("{url}?api-version={API_VERSION}")
    }
}

/// Percent-encode a query-value while keeping `/` literal (Azure item paths use
/// real slashes). Encodes the characters that would otherwise break the URL.
fn encode_query_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            ' ' => out.push_str("%20"),
            '#' => out.push_str("%23"),
            '?' => out.push_str("%3F"),
            '&' => out.push_str("%26"),
            '+' => out.push_str("%2B"),
            '%' => out.push_str("%25"),
            other => out.push(other),
        }
    }
    out
}

// ---------- Backend ----------

pub struct AzureDevOpsBackend {
    default_repository: Option<ForgeRepository>,
    http: Box<dyn AzHttp>,
    local_checkout: Option<PathBuf>,
}

impl AzureDevOpsBackend {
    /// Build a backend, auto-selecting the transport (PAT env var, else `az`).
    pub fn new(default_repository: Option<ForgeRepository>) -> Self {
        Self {
            default_repository,
            http: default_transport(),
            local_checkout: None,
        }
    }

    /// Build a backend with an explicit transport (used in tests).
    pub fn with_transport(
        default_repository: Option<ForgeRepository>,
        http: Box<dyn AzHttp>,
    ) -> Self {
        Self {
            default_repository,
            http,
            local_checkout: None,
        }
    }

    pub fn with_local_checkout(mut self, checkout: Option<PathBuf>) -> Self {
        self.local_checkout = checkout;
        self
    }

    pub fn set_local_checkout(&mut self, checkout: Option<PathBuf>) {
        self.local_checkout = checkout;
    }

    fn resolve_repository(&self, target: &PullRequestTarget) -> Result<ForgeRepository> {
        target
            .repository
            .clone()
            .or_else(|| self.default_repository.clone())
            .ok_or_else(|| {
                TuicrError::Forge(format!(
                    "Azure DevOps pull request target `{}` does not include a repository",
                    target.original
                ))
            })
    }

    fn get(&self, repo: &ForgeRepository, url: String) -> Result<String> {
        self.http
            .request("GET", &with_api_version(&url), None)
            .map_err(|err| map_http_error(err, &repo.host))
    }

    fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        repo: &ForgeRepository,
        url: String,
    ) -> Result<T> {
        let output = self.get(repo, url)?;
        Ok(serde_json::from_str(&output)?)
    }

    fn send(
        &self,
        repo: &ForgeRepository,
        method: &str,
        url: String,
        body: &str,
    ) -> Result<String> {
        self.http
            .request(method, &with_api_version(&url), Some(body))
            .map_err(|err| map_http_error(err, &repo.host))
    }

    /// GET a JSON endpoint that must NOT carry `?api-version`. `connectionData`
    /// is such an endpoint — it rejects `api-version=7.1` with "the requested
    /// version is under preview", so we call it version-less.
    fn get_json_unversioned<T: serde::de::DeserializeOwned>(
        &self,
        repo: &ForgeRepository,
        url: String,
    ) -> Result<T> {
        let output = self
            .http
            .request("GET", &url, None)
            .map_err(|err| map_http_error(err, &repo.host))?;
        Ok(serde_json::from_str(&output)?)
    }

    /// Resolve the authenticated user's identity id (needed to cast a vote and
    /// to filter the PR list to review-requested).
    fn current_user_id(&self, repo: &ForgeRepository) -> Result<Option<String>> {
        let (org, _project) = azure_coords(repo)?;
        let url = format!("https://{}/{}/_apis/connectionData", repo.host, org);
        let data: AzConnectionData = self.get_json_unversioned(repo, url)?;
        Ok(data
            .authenticated_user
            .map(|u| u.id)
            .filter(|id| !id.is_empty()))
    }

    fn fetch_file_via_api(&self, request: &ForgeFileLinesRequest) -> Result<String> {
        let base = git_api_base(&request.repository);
        let mut path = request.path.to_string_lossy().replace('\\', "/");
        if !path.starts_with('/') {
            path = format!("/{path}");
        }
        let url = format!(
            "{base}/items?path={}&versionDescriptor.version={}&versionDescriptor.versionType=commit&$format=text",
            encode_query_value(&path),
            request.sha(),
        );
        self.get(&request.repository, url)
    }

    /// File content at the request's revision: local blob first, REST fallback.
    fn file_content(&self, request: &ForgeFileLinesRequest) -> Result<String> {
        let local = self
            .local_checkout
            .as_deref()
            .and_then(|root| read_blob_with_repo(root, request.sha(), request.path.as_path()));
        match local {
            Some(content) => Ok(content),
            None => self.fetch_file_via_api(request),
        }
    }
}

impl ForgeBackend for AzureDevOpsBackend {
    fn list_pull_requests(&self, query: PullRequestListQuery) -> Result<PagedPullRequests> {
        let page_size = query.page_size.max(1);
        let base = git_api_base(&query.repository);
        // Fetch one extra to detect a further page.
        let mut url = format!(
            "{base}/pullRequests?searchCriteria.status=active&$top={}&$skip={}",
            page_size + 1,
            query.already_loaded,
        );
        // "Requested" scope → only PRs where the current user is a reviewer.
        // Best-effort: if the user id can't be resolved, fall back to all active.
        if query.scope == PullRequestListScope::ReviewRequested
            && let Some(id) = self.current_user_id(&query.repository).ok().flatten()
        {
            url.push_str(&format!("&searchCriteria.reviewerId={id}"));
        }
        let list: AzList<AzPullRequest> = self.get_json(&query.repository, url)?;
        let has_more = list.value.len() > page_size;
        let pull_requests = list
            .value
            .into_iter()
            .take(page_size)
            .map(|pr| pr.into_summary(&query.repository))
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
        let base = git_api_base(&repository);
        let url = format!("{base}/pullRequests/{}", target.number);
        let pr: AzPullRequest = self.get_json(&repository, url)?;
        Ok(pr.into_details(&repository))
    }

    fn get_pull_request_diff(&self, pr: &PullRequestDetails) -> Result<Vec<FilePatch>> {
        // 3-dot diff (merge-base..head) matches PR review semantics, like the
        // GitHub compare API. Sourced from the local clone.
        let root = self
            .local_checkout
            .as_deref()
            .ok_or_else(missing_checkout)?;
        local_diff(root, &pr.base_sha, &pr.head_sha, "...").ok_or_else(|| {
            TuicrError::Forge(format!(
                "Could not diff {}...{} in the local checkout. Fetch the PR's base and source \
                 commits (e.g. `git fetch origin`) and retry.",
                short(&pr.base_sha),
                short(&pr.head_sha),
            ))
        })
    }

    fn get_pull_request_commit_range_diff(
        &self,
        _pr: &PullRequestDetails,
        start_sha: &str,
        end_sha: &str,
    ) -> Result<Vec<FilePatch>> {
        let root = self
            .local_checkout
            .as_deref()
            .ok_or_else(missing_checkout)?;
        local_diff(root, start_sha, end_sha, "..").ok_or_else(|| {
            TuicrError::UnsupportedOperation(
                "Commit-range diff requires both commits in the local checkout for Azure DevOps"
                    .to_string(),
            )
        })
    }

    fn resolve_diff_base_sha(&self, pr: &PullRequestDetails) -> Option<String> {
        // `base_sha` is `lastMergeTargetCommit` — the target branch tip, which
        // drifts ahead of the branch point as the target moves. The diff above
        // is three-dot, so the old side lives at the merge base. Azure already
        // requires a local checkout for diffing, so this needs no API call.
        let root = self.local_checkout.as_deref()?;
        local_merge_base(root, &pr.base_sha, &pr.head_sha)
    }

    fn local_checkout_path(&self) -> Option<PathBuf> {
        self.local_checkout.clone()
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

    fn list_review_threads(&self, pr: &PullRequestDetails) -> Result<Vec<RemoteReviewThread>> {
        let base = git_api_base(&pr.repository);
        let url = format!("{base}/pullRequests/{}/threads", pr.number);
        let list: AzList<AzThread> = self.get_json(&pr.repository, url)?;
        // Azure emits many `system` threads (pushes, policy, votes); the model
        // mapping drops those and keeps human-authored threads.
        Ok(list
            .value
            .into_iter()
            .filter_map(AzThread::into_review_thread)
            .collect())
    }

    fn list_pull_request_commits(&self, pr: &PullRequestDetails) -> Result<Vec<PullRequestCommit>> {
        let base = git_api_base(&pr.repository);
        let url = format!("{base}/pullRequests/{}/commits", pr.number);
        let list: AzList<AzGitCommitRef> = self.get_json(&pr.repository, url)?;
        Ok(list
            .value
            .into_iter()
            .map(AzGitCommitRef::into_pull_request_commit)
            .collect())
    }

    fn create_review(
        &self,
        pr: &PullRequestDetails,
        request: CreateReviewRequest<'_>,
    ) -> Result<GhCreateReviewResponse> {
        let base = git_api_base(&pr.repository);
        let threads_url = format!("{base}/pullRequests/{}/threads", pr.number);
        let mut first_thread_id = 0u64;

        // Overall review body → a context-less PR comment thread.
        if !request.body.is_empty() {
            let payload = json!({
                "comments": [{ "parentCommentId": 0, "content": request.body, "commentType": "text" }],
                "status": "active",
            });
            let out = self.send(
                &pr.repository,
                "POST",
                threads_url.clone(),
                &serde_json::to_string(&payload)?,
            )?;
            capture_thread_id(&out, &mut first_thread_id);
        }

        // Each inline comment → a thread anchored to a file/line range.
        for comment in request.comments {
            let mut file_path = comment.path.to_string_lossy().replace('\\', "/");
            if !file_path.starts_with('/') {
                file_path = format!("/{file_path}");
            }
            let start_line = comment.start_line.unwrap_or(comment.line);
            // Azure anchors a thread via left/right file line ranges. We only
            // ever produce single-side ranges (the submit mapper rejects
            // mixed-side ranges), so both endpoints use `comment.side`.
            let start = json!({ "line": start_line, "offset": 1 });
            let end = json!({ "line": comment.line, "offset": 1 });
            let thread_context = match comment.side {
                GhSide::Right => json!({
                    "filePath": file_path,
                    "rightFileStart": start,
                    "rightFileEnd": end,
                }),
                GhSide::Left => json!({
                    "filePath": file_path,
                    "leftFileStart": start,
                    "leftFileEnd": end,
                }),
            };
            let payload = json!({
                "comments": [{ "parentCommentId": 0, "content": comment.body, "commentType": "text" }],
                "status": "active",
                "threadContext": thread_context,
            });
            let out = self.send(
                &pr.repository,
                "POST",
                threads_url.clone(),
                &serde_json::to_string(&payload)?,
            )?;
            capture_thread_id(&out, &mut first_thread_id);
        }

        // Approve / reject map to an Azure reviewer vote. Comment and Draft
        // (Azure has no draft-review primitive) post comments without voting.
        let vote = match request.event {
            SubmitEvent::Approve => Some(10),
            SubmitEvent::RequestChanges => Some(-10),
            SubmitEvent::Comment | SubmitEvent::Draft => None,
        };
        if let Some(vote) = vote {
            let user_id = self.current_user_id(&pr.repository)?.ok_or_else(|| {
                TuicrError::Forge(
                    "Could not resolve the current Azure DevOps user to record a vote".to_string(),
                )
            })?;
            let url = format!("{base}/pullRequests/{}/reviewers/{}", pr.number, user_id);
            let payload = json!({ "vote": vote });
            self.send(
                &pr.repository,
                "PUT",
                url,
                &serde_json::to_string(&payload)?,
            )?;
        }

        let state = match request.event {
            SubmitEvent::Approve => "APPROVED",
            SubmitEvent::RequestChanges => "CHANGES_REQUESTED",
            SubmitEvent::Comment | SubmitEvent::Draft => "COMMENTED",
        };
        Ok(GhCreateReviewResponse {
            id: first_thread_id,
            html_url: pr.url.clone(),
            state: state.to_string(),
        })
    }
}

fn capture_thread_id(output: &str, first: &mut u64) {
    if *first == 0
        && let Ok(thread) = serde_json::from_str::<AzThreadResponse>(output)
        && thread.id != 0
    {
        *first = thread.id;
    }
}

fn missing_checkout() -> TuicrError {
    TuicrError::Forge(
        "Reviewing an Azure DevOps PR needs a local clone of the repository (Azure exposes no \
         unified-diff API). Run tuicr from inside a clone of the repo."
            .to_string(),
    )
}

fn short(sha: &str) -> String {
    sha.chars().take(8).collect()
}

fn map_http_error(error: AzHttpError, host: &str) -> TuicrError {
    match error {
        AzHttpError::Missing(msg) => TuicrError::Forge(msg),
        AzHttpError::Auth(detail) => {
            let hint = if pat_from_env().is_some() {
                "Azure DevOps rejected the PAT (check it isn't expired and has Code: Read & Write \
                 scope for this organization)."
            } else {
                "Azure DevOps authentication failed. This org may not allow `az` AAD tokens — set \
                 a PAT in AZURE_DEVOPS_EXT_PAT, or run `az login` for the right tenant."
            };
            TuicrError::Forge(format!("{hint}\n{}", trim_detail(&detail)))
        }
        AzHttpError::Failed { status, body } => {
            let status = status.map(|s| format!(" (HTTP {s})")).unwrap_or_default();
            TuicrError::Forge(format!(
                "Azure DevOps request to {host} failed{status}: {}",
                trim_detail(&body)
            ))
        }
    }
}

/// Keep error detail readable: collapse whitespace and cap the length.
fn trim_detail(detail: &str) -> String {
    let collapsed = detail.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.len() > 400 {
        format!("{}…", &collapsed[..400])
    } else {
        collapsed
    }
}

// ---------- URL & target parsing ----------

/// True when `host` names an Azure DevOps instance.
fn is_azure_host(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    host == "dev.azure.com"
        || host == "ssh.dev.azure.com"
        || host.ends_with(".visualstudio.com")
        || host == "vs-ssh.visualstudio.com"
}

/// Parse an Azure DevOps remote (git) URL into a `ForgeRepository`.
///
/// Accepts the HTTPS forms `https://dev.azure.com/{org}/{project}/_git/{repo}`
/// and `https://{org}.visualstudio.com/[DefaultCollection/]{project}/_git/{repo}`
/// (optionally with an `{org}@` userinfo prefix), and the SSH forms
/// `git@ssh.dev.azure.com:v3/{org}/{project}/{repo}` /
/// `{org}@vs-ssh.visualstudio.com:v3/{org}/{project}/{repo}`. Returns `None`
/// for non-Azure hosts. Always normalizes the stored host to `dev.azure.com`.
pub fn parse_azure_remote_url(remote_url: &str) -> Option<ForgeRepository> {
    let trimmed = trim_url_suffix(remote_url.trim());
    if trimmed.is_empty() {
        return None;
    }

    // SCP-like SSH: `user@host:path`.
    if let Some((host, path)) = parse_scp_like_remote(trimmed) {
        if !is_azure_host(host) {
            return None;
        }
        return azure_from_ssh_path(path);
    }

    let without_scheme = strip_scheme(trimmed).unwrap_or(trimmed);
    let without_user = without_scheme
        .rsplit_once('@')
        .map(|(_, rest)| rest)
        .unwrap_or(without_scheme);
    let (host, path) = without_user.split_once('/')?;
    let host = strip_port(host);
    if !is_azure_host(host) {
        return None;
    }
    azure_from_https_path(host, path)
}

/// Build the repository from an HTTPS path (everything after the host).
fn azure_from_https_path(host: &str, path: &str) -> Option<ForgeRepository> {
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let git_idx = segments
        .iter()
        .position(|&s| s.eq_ignore_ascii_case("_git"))?;
    let repo = strip_git_suffix(segments.get(git_idx + 1)?);
    if repo.is_empty() {
        return None;
    }
    // Segments before `_git`, minus the `DefaultCollection` legacy token.
    let before: Vec<&str> = segments[..git_idx]
        .iter()
        .copied()
        .filter(|&s| !s.eq_ignore_ascii_case("DefaultCollection"))
        .collect();

    let (org, project) = if host.to_ascii_lowercase().ends_with(".visualstudio.com") {
        // org lives in the subdomain; path holds just the project.
        let org = host.split('.').next()?;
        let project = *before.last()?;
        (org.to_string(), project.to_string())
    } else {
        // dev.azure.com: path is org/project.
        if before.len() < 2 {
            return None;
        }
        (before[0].to_string(), before[before.len() - 1].to_string())
    };
    if org.is_empty() || project.is_empty() {
        return None;
    }
    Some(ForgeRepository::azure(
        DEFAULT_AZURE_HOST,
        format!("{org}/{project}"),
        repo,
    ))
}

/// Build the repository from an SSH path (`v3/{org}/{project}/{repo}`).
fn azure_from_ssh_path(path: &str) -> Option<ForgeRepository> {
    let segments: Vec<&str> = path
        .split('/')
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("v3"))
        .collect();
    if segments.len() < 3 {
        return None;
    }
    let org = segments[0];
    let project = segments[1];
    let repo = strip_git_suffix(segments[2]);
    if org.is_empty() || project.is_empty() || repo.is_empty() {
        return None;
    }
    Some(ForgeRepository::azure(
        DEFAULT_AZURE_HOST,
        format!("{org}/{project}"),
        repo,
    ))
}

/// Parse a PR target: a bare number or an Azure PR web URL.
///
/// There is deliberately no `org/project/repo#id` shorthand: a bare `a/b/c#n`
/// is ambiguous with GitHub Enterprise's `host/owner/repo#n` form (which the
/// GitHub parser claims earlier in the chain). Azure PRs are referenced by
/// number or full URL.
pub fn parse_pull_request_target_azure(input: &str) -> Result<PullRequestTarget> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return malformed_target(input);
    }

    if let Some(target) = parse_numeric_target(trimmed) {
        return Ok(target);
    }
    if let Some(target) = parse_azure_url_target(trimmed) {
        return Ok(target);
    }
    malformed_target(input)
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

fn parse_azure_url_target(target: &str) -> Option<PullRequestTarget> {
    // The repo parser already tolerates the trailing `/pullrequest/{id}` (it
    // only reads up to the segment after `_git`).
    let repository = parse_azure_remote_url(target)?;
    let lower = target.to_ascii_lowercase();
    let marker = lower.find("/pullrequest/")?;
    let after = &target[marker + "/pullrequest/".len()..];
    let id_str = after.split('/').next()?;
    let number = id_str.parse::<u64>().ok()?;
    if number == 0 {
        return None;
    }
    Some(PullRequestTarget::with_repository(
        repository, number, target,
    ))
}

fn malformed_target<T>(input: &str) -> Result<T> {
    Err(TuicrError::Forge(format!(
        "Malformed Azure DevOps pull request target: `{input}`"
    )))
}

// ---------- Small URL helpers (mirror gh/glab) ----------

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

fn strip_port(host: &str) -> &str {
    match host.rsplit_once(':') {
        Some((h, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => h,
        _ => host,
    }
}

fn strip_git_suffix(value: &str) -> &str {
    value.strip_suffix(".git").unwrap_or(value)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Mutex;

    use super::*;
    use crate::forge::submit::InlineComment;

    fn azure_repo() -> ForgeRepository {
        ForgeRepository::azure("dev.azure.com", "myorg/myproject", "myrepo")
    }

    // ---- URL parsing ----

    #[test]
    fn parses_https_dev_azure_remote() {
        let repo =
            parse_azure_remote_url("https://dev.azure.com/myorg/myproject/_git/myrepo").unwrap();
        assert_eq!(repo, azure_repo());
    }

    #[test]
    fn parses_https_remote_with_org_userinfo_and_git_suffix() {
        let repo =
            parse_azure_remote_url("https://myorg@dev.azure.com/myorg/myproject/_git/myrepo.git")
                .unwrap();
        assert_eq!(repo, azure_repo());
    }

    #[test]
    fn parses_visualstudio_remote() {
        let repo =
            parse_azure_remote_url("https://myorg.visualstudio.com/myproject/_git/myrepo").unwrap();
        assert_eq!(repo, azure_repo());
    }

    #[test]
    fn parses_visualstudio_remote_with_default_collection() {
        let repo = parse_azure_remote_url(
            "https://myorg.visualstudio.com/DefaultCollection/myproject/_git/myrepo",
        )
        .unwrap();
        assert_eq!(repo, azure_repo());
    }

    #[test]
    fn parses_ssh_remote() {
        let repo =
            parse_azure_remote_url("git@ssh.dev.azure.com:v3/myorg/myproject/myrepo").unwrap();
        assert_eq!(repo, azure_repo());
    }

    #[test]
    fn rejects_non_azure_host() {
        assert!(parse_azure_remote_url("https://github.com/agavra/tuicr").is_none());
        assert!(parse_azure_remote_url("git@github.com:agavra/tuicr.git").is_none());
    }

    // ---- Target parsing ----

    #[test]
    fn parses_numeric_target() {
        let target = parse_pull_request_target_azure("125").unwrap();
        assert_eq!(target.number, 125);
        assert!(target.repository.is_none());
    }

    #[test]
    fn parses_pr_web_url_target() {
        let target = parse_pull_request_target_azure(
            "https://dev.azure.com/myorg/myproject/_git/myrepo/pullrequest/42",
        )
        .unwrap();
        assert_eq!(target.number, 42);
        assert_eq!(target.repository.unwrap(), azure_repo());
    }

    #[test]
    fn rejects_malformed_target() {
        assert!(parse_pull_request_target_azure("not/a/target").is_err());
    }

    // ---- Coordinate helper ----

    #[test]
    fn azure_coords_splits_owner() {
        let (org, project) = azure_coords(&azure_repo()).unwrap();
        assert_eq!(org, "myorg");
        assert_eq!(project, "myproject");
    }

    #[test]
    fn azure_coords_errors_without_project() {
        let repo = ForgeRepository::azure("dev.azure.com", "myorg", "myrepo");
        assert!(azure_coords(&repo).is_err());
    }

    // ---- create_review payload (via a recording transport) ----

    struct RecordingHttp {
        calls: Mutex<Vec<(String, String, Option<String>)>>,
        responses: Mutex<Vec<String>>,
    }

    impl RecordingHttp {
        fn with_responses(responses: Vec<String>) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                responses: Mutex::new(responses),
            }
        }
    }

    impl AzHttp for RecordingHttp {
        fn request(&self, method: &str, url: &str, body: Option<&str>) -> AzHttpResult<String> {
            self.calls.lock().unwrap().push((
                method.to_string(),
                url.to_string(),
                body.map(str::to_string),
            ));
            let mut responses = self.responses.lock().unwrap();
            Ok(if responses.is_empty() {
                "{}".to_string()
            } else {
                responses.remove(0)
            })
        }
    }

    /// A transport the test can inspect after the backend has used it.
    #[derive(Clone)]
    struct SharedHttp(std::sync::Arc<RecordingHttp>);
    impl SharedHttp {
        fn new(responses: Vec<String>) -> Self {
            Self(std::sync::Arc::new(RecordingHttp::with_responses(
                responses,
            )))
        }
    }
    impl AzHttp for SharedHttp {
        fn request(&self, method: &str, url: &str, body: Option<&str>) -> AzHttpResult<String> {
            self.0.request(method, url, body)
        }
    }

    fn pr_details() -> PullRequestDetails {
        PullRequestDetails {
            repository: azure_repo(),
            number: 42,
            title: "t".to_string(),
            url: "https://dev.azure.com/myorg/myproject/_git/myrepo/pullrequest/42".to_string(),
            state: "OPEN".to_string(),
            is_draft: false,
            author: None,
            head_ref_name: "feature".to_string(),
            base_ref_name: "main".to_string(),
            head_sha: "head111".to_string(),
            base_sha: "base000".to_string(),
            body: String::new(),
            updated_at: None,
            closed: false,
            merged_at: None,
            diff_start_sha: None,
        }
    }

    fn inline(line: u32, side: GhSide) -> InlineComment {
        InlineComment {
            path: PathBuf::from("src/lib.rs"),
            line,
            side,
            counterpart_line: None,
            start_line: None,
            start_side: None,
            range_anchors: None,
            old_path: None,
            body: "please fix".to_string(),
            comment_id: "c1".to_string(),
        }
    }

    fn body_json(call: &(String, String, Option<String>)) -> serde_json::Value {
        serde_json::from_str(call.2.as_deref().unwrap()).unwrap()
    }

    #[test]
    fn create_review_posts_inline_thread_with_right_context() {
        let shared = SharedHttp::new(vec![r#"{"id": 555}"#.to_string()]);
        let backend =
            AzureDevOpsBackend::with_transport(Some(azure_repo()), Box::new(shared.clone()));
        let request = CreateReviewRequest {
            event: SubmitEvent::Comment,
            commit_id: "head111",
            body: "",
            comments: &[inline(10, GhSide::Right)],
        };
        let resp = backend.create_review(&pr_details(), request).unwrap();
        assert_eq!(resp.id, 555);
        assert_eq!(resp.state, "COMMENTED");

        let calls = shared.0.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "POST");
        assert!(calls[0].1.contains("/pullRequests/42/threads"));
        assert!(calls[0].1.contains("api-version="));
        let body = body_json(&calls[0]);
        assert_eq!(body["threadContext"]["filePath"], "/src/lib.rs");
        assert_eq!(body["threadContext"]["rightFileStart"]["line"], 10);
        assert_eq!(body["threadContext"]["rightFileEnd"]["line"], 10);
        assert_eq!(body["comments"][0]["content"], "please fix");
        assert_eq!(body["status"], "active");
    }

    #[test]
    fn create_review_left_side_uses_left_context() {
        let shared = SharedHttp::new(vec![r#"{"id": 1}"#.to_string()]);
        let backend =
            AzureDevOpsBackend::with_transport(Some(azure_repo()), Box::new(shared.clone()));
        let request = CreateReviewRequest {
            event: SubmitEvent::Comment,
            commit_id: "head111",
            body: "",
            comments: &[inline(4, GhSide::Left)],
        };
        backend.create_review(&pr_details(), request).unwrap();
        let calls = shared.0.calls.lock().unwrap();
        let body = body_json(&calls[0]);
        assert_eq!(body["threadContext"]["leftFileStart"]["line"], 4);
        assert!(body["threadContext"].get("rightFileStart").is_none());
    }

    #[test]
    fn create_review_approve_casts_positive_vote() {
        // Responses: connectionData GET → vote PUT.
        let shared = SharedHttp::new(vec![
            r#"{"authenticatedUser": {"id": "user-guid"}}"#.to_string(),
            r#"{}"#.to_string(),
        ]);
        let backend =
            AzureDevOpsBackend::with_transport(Some(azure_repo()), Box::new(shared.clone()));
        let request = CreateReviewRequest {
            event: SubmitEvent::Approve,
            commit_id: "head111",
            body: "",
            comments: &[],
        };
        let resp = backend.create_review(&pr_details(), request).unwrap();
        assert_eq!(resp.state, "APPROVED");

        let calls = shared.0.calls.lock().unwrap();
        let put = calls
            .iter()
            .find(|c| c.0 == "PUT")
            .expect("a PUT call for the vote");
        assert!(put.1.contains("/reviewers/user-guid"));
        assert_eq!(body_json(put)["vote"], 10);
    }

    #[test]
    fn backend_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<AzureDevOpsBackend>();
    }

    #[test]
    fn get_pull_request_maps_active_pr() {
        let shared = SharedHttp::new(vec![
            r#"{"pullRequestId":42,"title":"t","status":"active","sourceRefName":"refs/heads/feature","targetRefName":"refs/heads/main","lastMergeSourceCommit":{"commitId":"head111"},"lastMergeTargetCommit":{"commitId":"base000"}}"#.to_string(),
        ]);
        let backend =
            AzureDevOpsBackend::with_transport(Some(azure_repo()), Box::new(shared.clone()));
        let details = backend
            .get_pull_request(PullRequestTarget::number(42, "42"))
            .unwrap();
        assert_eq!(details.head_sha, "head111");
        assert_eq!(details.base_sha, "base000");
        assert_eq!(details.state, "OPEN");
        let calls = shared.0.calls.lock().unwrap();
        assert_eq!(calls[0].0, "GET");
        assert!(calls[0].1.contains("/pullRequests/42?api-version="));
    }

    #[test]
    fn resolve_diff_base_sha_returns_local_merge_base_not_the_target_branch_tip() {
        use std::ffi::OsStr;
        fn git(root: &std::path::Path, args: &[&str]) -> String {
            crate::process::run_command_output(
                "git",
                Some(root),
                args.iter().map(|a| OsStr::new(*a)),
            )
            .unwrap_or_else(|e| panic!("git {args:?} failed: {e:?}"))
            .trim()
            .to_string()
        }
        fn commit(root: &std::path::Path, name: &str) -> String {
            std::fs::write(root.join(name), name).unwrap();
            git(root, &["add", "."]);
            git(root, &["commit", "-m", name, "--no-gpg-sign"]);
            git(root, &["rev-parse", "HEAD"])
        }

        // given — the PR target branch has advanced past the branch point,
        // which is what makes lastMergeTargetCommit the wrong old-side rev.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        git(root, &["init", "--quiet"]);
        git(root, &["config", "user.email", "test@example.com"]);
        git(root, &["config", "user.name", "Test"]);
        git(root, &["config", "commit.gpgsign", "false"]);
        let fork_point = commit(root, "a");
        let pr_head = commit(root, "b");
        git(root, &["checkout", "--quiet", &fork_point]);
        let target_tip = commit(root, "c");
        assert_ne!(target_tip, fork_point);

        let shared = SharedHttp::new(vec![
            r#"{"pullRequestId":42,"title":"t","status":"active","sourceRefName":"refs/heads/feature","targetRefName":"refs/heads/main"}"#.to_string(),
        ]);
        let backend = AzureDevOpsBackend::with_transport(Some(azure_repo()), Box::new(shared))
            .with_local_checkout(Some(root.to_path_buf()));
        let mut details = backend
            .get_pull_request(PullRequestTarget::number(42, "42"))
            .unwrap();
        details.base_sha = target_tip;
        details.head_sha = pr_head;

        // when / then — the three-dot base, resolved without an API call.
        assert_eq!(
            backend.resolve_diff_base_sha(&details).as_deref(),
            Some(fork_point.as_str())
        );
    }

    #[test]
    fn resolve_diff_base_sha_is_none_without_a_local_checkout() {
        let shared = SharedHttp::new(vec![
            r#"{"pullRequestId":42,"title":"t","status":"active","lastMergeSourceCommit":{"commitId":"head111"},"lastMergeTargetCommit":{"commitId":"base000"}}"#.to_string(),
        ]);
        let backend = AzureDevOpsBackend::with_transport(Some(azure_repo()), Box::new(shared));
        let details = backend
            .get_pull_request(PullRequestTarget::number(42, "42"))
            .unwrap();
        // Best-effort: no checkout means base_sha stays as reported.
        assert_eq!(backend.resolve_diff_base_sha(&details), None);
    }

    #[test]
    fn list_open_scope_has_no_reviewer_filter() {
        let shared = SharedHttp::new(vec![
            r#"{"count":1,"value":[{"pullRequestId":1,"title":"t","status":"active"}]}"#
                .to_string(),
        ]);
        let backend =
            AzureDevOpsBackend::with_transport(Some(azure_repo()), Box::new(shared.clone()));
        let query = PullRequestListQuery::first_page_with_scope(
            azure_repo(),
            30,
            PullRequestListScope::Open,
        );
        backend.list_pull_requests(query).unwrap();
        let calls = shared.0.calls.lock().unwrap();
        // Open scope: a single list call, no connectionData lookup, no filter.
        assert_eq!(calls.len(), 1);
        assert!(
            calls[0]
                .1
                .contains("/pullRequests?searchCriteria.status=active")
        );
        assert!(!calls[0].1.contains("reviewerId"));
    }

    #[test]
    fn list_requested_scope_filters_by_reviewer_and_calls_connectiondata_unversioned() {
        let shared = SharedHttp::new(vec![
            r#"{"authenticatedUser":{"id":"user-guid"}}"#.to_string(),
            r#"{"count":1,"value":[{"pullRequestId":1433,"title":"t","status":"active"}]}"#
                .to_string(),
        ]);
        let backend =
            AzureDevOpsBackend::with_transport(Some(azure_repo()), Box::new(shared.clone()));
        let query = PullRequestListQuery::first_page_with_scope(
            azure_repo(),
            30,
            PullRequestListScope::ReviewRequested,
        );
        let page = backend.list_pull_requests(query).unwrap();
        assert_eq!(page.pull_requests.len(), 1);

        let calls = shared.0.calls.lock().unwrap();
        let conn = calls
            .iter()
            .find(|c| c.1.contains("/_apis/connectionData"))
            .expect("connectionData lookup");
        assert!(
            !conn.1.contains("api-version"),
            "connectionData must be version-less, got: {}",
            conn.1
        );
        let list = calls
            .iter()
            .find(|c| c.1.contains("/pullRequests?"))
            .expect("list call");
        assert!(
            list.1.contains("searchCriteria.reviewerId=user-guid"),
            "expected reviewer filter, got: {}",
            list.1
        );
    }
}
